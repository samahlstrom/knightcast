#!/usr/bin/env python3
"""Explicit native camera/OpenSeeFace sidecar for KnightCast. No frame recording.

Face metrics adapt the user's stream-kit/puppet/tracking/openseeface_server.py;
rotation follows their orientation.mjs. No reference avatar app source used.
Synthetic mode and extraction tests require only the Python standard library.
"""
from __future__ import annotations

import argparse
import json
import math
import os
from pathlib import Path
import socket
import signal
import sys
import threading
import time
import uuid


def clamp(value, low=0.0, high=1.0):
    return float(max(low, min(high, value)))


def quaternion_zyx(pitch, yaw, roll):
    """Active RH qz * qy * qx, degrees in, [x,y,z,w] out."""
    x, y, z = (math.radians(a) / 2 for a in (pitch, yaw, roll))
    sx, cx, sy, cy, sz, cz = math.sin(x), math.cos(x), math.sin(y), math.cos(y), math.sin(z), math.cos(z)
    return [sx*cy*cz-cx*sy*sz, cx*sy*cz+sx*cy*sz,
            cx*cy*sz-sx*sy*cz, cx*cy*cz+sx*sy*sz]


def lost_pose():
    return dict(tracked=False, rotation=[0., 0., 0., 1.], mouth_open=0.,
                blink_left=0., blink_right=0., eye_wide_left=0.,
                eye_wide_right=0., smile=0., gaze=[0., 0.], head_scale=1.)


def extract(face):
    """Inner lips only; no jaw/chin, calibration, or temporal smoothing here."""
    try:
        points = [[float(v) for v in row] for row in face.pts_3d]
        angles = [float(v) for v in face.euler]
        if (not face.success or len(points) < 68 or len(angles) != 3
                or any(len(row) < 3 for row in points)
                or not all(math.isfinite(v) for row in points for v in row)
                or not all(math.isfinite(v) for v in angles)):
            return lost_pose()
        def distance(a, b):
            return math.hypot(points[a][0]-points[b][0], points[a][1]-points[b][1])
        width = max(.02, distance(58, 62))
        mouth = clamp(sum(distance(a, b) for a, b in [(59, 65), (60, 64), (61, 63)]) / (3*width))
        def eye(start):
            return (distance(start+1, start+5)+distance(start+2, start+4))/(2*max(.02, distance(start, start+3)))
        right, left = eye(36), eye(42)
        gaze = []
        for a, b, iris in [(36, 39, 66), (42, 45, 67)]:
            gaze.append([(points[iris][axis]-(points[a][axis]+points[b][axis])/2)
                         / max(.02, distance(a, b)) for axis in (0, 1)])
        wrap = lambda value: (value + 180) % 360 - 180
        pose = dict(tracked=True,
                    rotation=quaternion_zyx(wrap(angles[0]-180), wrap(angles[1]), wrap(angles[2]-90)),
                    mouth_open=mouth, blink_left=clamp(1-left/.23), blink_right=clamp(1-right/.23),
                    eye_wide_left=clamp(left/.60), eye_wide_right=clamp(right/.60),
                    smile=clamp(.5+3*((points[58][1]+points[62][1])/2
                                      -sum(points[i][1] for i in [49, 50, 51])/3)/width),
                    gaze=[clamp(-sum(g[axis] for g in gaze)*1.5, -1, 1) for axis in (0, 1)],
                    head_scale=1.)
        flat = [v for value in pose.values() for v in (value if isinstance(value, list) else [value])]
        return pose if all(math.isfinite(v) for v in flat) else lost_pose()
    except (AttributeError, TypeError, ValueError, IndexError, OverflowError):
        return lost_pose()


class LatestFrame:
    """At most one pending frame, plus a separately owned frame in inference."""
    def __init__(self):
        self.condition = threading.Condition()
        self.pending = None
        self.closed = False
        self.captured = self.consumed = self.superseded = 0
        self.error = None

    def put(self, frame, captured_at):
        with self.condition:
            if self.closed:
                return False
            self.captured += 1
            self.superseded += self.pending is not None
            self.pending = (frame, captured_at)
            self.condition.notify()
            return True

    def take(self, timeout=.1):
        with self.condition:
            self.condition.wait_for(lambda: self.pending is not None or self.closed, timeout)
            item, self.pending = self.pending, None
            self.consumed += item is not None
            return item

    def close(self, error=None):
        with self.condition:
            self.closed = True
            self.error = error
            self.condition.notify_all()

    def counts(self):
        with self.condition:
            return self.captured, self.consumed, self.superseded, int(self.pending is not None)


class Sender:
    def __init__(self, port, max_pose_age_ms=250.):
        self.socket = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        self.destination = ('127.0.0.1', port)
        self.seq = 0
        self.session_id = str(uuid.uuid4())
        self.max_pose_age_ms = max_pose_age_ms

    def send(self, pose, captured_at=None, inference_ms=0.):
        age = 0. if captured_at is None else max(0., (time.monotonic()-captured_at)*1000)
        # Receiver freshness starts at receipt: never revive an old inference as fresh.
        if age > self.max_pose_age_ms or inference_ms > self.max_pose_age_ms:
            pose = lost_pose()
        packet = dict(pose, version=1, seq=self.seq, session_id=self.session_id,
                      capture_age_ms=min(age, 60000.), inference_ms=min(max(inference_ms, 0.), 60000.))
        data = json.dumps(packet, allow_nan=False, separators=(',', ':')).encode('utf-8')
        self.socket.sendto(data, self.destination)
        self.seq += 1
        return packet

    def close(self):
        self.socket.close()


def list_cameras():
    """Read filesystem metadata only: never open or probe a camera."""
    nodes = sorted(Path('/dev').glob('video*'))
    aliases = sorted(Path('/dev/v4l/by-id').glob('*'))
    for node in nodes:
        namefile = Path('/sys/class/video4linux') / node.name / 'name'
        try:
            name = namefile.read_text().strip()
        except OSError:
            name = 'unknown device'
        print(f'{node}: {name}')
        for alias in aliases:
            if alias.resolve() == node.resolve():
                print(f'  {alias}')
    if not nodes:
        print('No /dev/video* entries found. No camera was opened.')


def capture_worker(camera, slot, stop):
    try:
        while not stop.is_set():
            success, frame = camera.read()
            captured_at = time.monotonic()
            if not success:
                raise RuntimeError('Camera stopped returning frames (busy, disconnected, or unsupported mode).')
            slot.put(frame, captured_at)
    except Exception as error:
        slot.close(str(error))
    finally:
        camera.release()
        slot.close(slot.error)


def letterbox_geometry(source_width, source_height, target_width, target_height):
    """Return resized width/height and left/top/right/bottom padding."""
    if min(source_width, source_height, target_width, target_height) <= 0:
        raise ValueError('Frame dimensions must be positive.')
    scale = min(target_width/source_width, target_height/source_height)
    width = min(target_width, max(1, round(source_width*scale)))
    height = min(target_height, max(1, round(source_height*scale)))
    left, top = (target_width-width)//2, (target_height-height)//2
    return width, height, left, top, target_width-width-left, target_height-height-top


def inference_frame(frame, width, height, cv2):
    """Fit actual camera pixels without stretching; Tracker uses target dimensions."""
    source_height, source_width = frame.shape[:2]
    resized_width, resized_height, left, top, right, bottom = letterbox_geometry(source_width, source_height, width, height)
    if (source_width, source_height) != (resized_width, resized_height):
        frame = cv2.resize(frame, (resized_width, resized_height), interpolation=cv2.INTER_AREA)
    if left or top or right or bottom:
        frame = cv2.copyMakeBorder(frame, top, bottom, left, right, cv2.BORDER_CONSTANT, value=(0, 0, 0))
    return frame


def synthetic(args, sender):
    print(f'SYNTHETIC tracking → 127.0.0.1:{args.port}, session {sender.session_id}', file=sys.stderr)
    started = deadline = time.monotonic()
    frame = 0
    while not args.duration or time.monotonic()-started < args.duration:
        elapsed = frame/args.fps
        pose = lost_pose()
        if not (args.loss_after >= 0 and args.loss_after <= elapsed < args.loss_after+args.loss_for):
            pose.update(tracked=True, rotation=quaternion_zyx(12*math.sin(elapsed), 25*math.sin(elapsed*.7), 8*math.sin(elapsed*.4)),
                        mouth_open=.08*(1+math.sin(elapsed*3)))
        sender.send(pose)
        frame += 1
        if args.burst:
            if frame >= args.burst:
                break
        else:
            deadline += 1/args.fps
            time.sleep(max(0., deadline-time.monotonic()))


def real_capture(args, sender):
    # Dependency loading is deliberately after explicit --camera selection.
    try:
        import cv2
        import onnxruntime
    except ImportError as error:
        raise RuntimeError(f'{error}. Run with the installed OpenSeeFace venv; see docs/CAPTURE.md.') from error
    onnxruntime.disable_telemetry_events()
    cv2.setNumThreads(1)
    osf = Path(args.openseeface_home).expanduser().resolve()
    if not (osf/'tracker.py').is_file() or not (osf/'models').is_dir():
        raise RuntimeError(f'OpenSeeFace installation/models missing at {osf}; no automatic downloads are made.')
    sys.path.insert(0, str(osf))
    from tracker import Tracker
    tracker = Tracker(args.inference_width, args.inference_height, model_type=args.model, max_threads=2, max_faces=1,
                      detection_threshold=.5, threshold=.6, discard_after=2, bbox_growth=.20,
                      scan_every=2, silent=True, model_dir=str(osf/'models'), static_model=True,
                      try_hard=True, max_feature_updates=900)
    source = int(args.camera) if args.camera.isdecimal() else args.camera
    print(f'Opening requested camera {args.camera}; model {args.model}; capture '
          f'{args.capture_width}×{args.capture_height}@{args.fps:g} {args.fourcc}; '
          f'inference {args.inference_width}×{args.inference_height} letterboxed; '
          f'max pose age {args.max_pose_age_ms:g}ms.', file=sys.stderr)
    camera = cv2.VideoCapture(source, cv2.CAP_V4L2 if sys.platform.startswith('linux') else cv2.CAP_ANY)
    if not camera.isOpened():
        camera.release()
        raise RuntimeError('Cannot open camera. It may be busy, inaccessible, or not a capture node. Stop its other owner yourself or select another camera; no services were stopped.')
    slot, stop = LatestFrame(), threading.Event()
    worker = None
    try:
        fourcc_ok = camera.set(cv2.CAP_PROP_FOURCC, cv2.VideoWriter_fourcc(*args.fourcc))
        camera.set(cv2.CAP_PROP_FRAME_WIDTH, args.capture_width)
        camera.set(cv2.CAP_PROP_FRAME_HEIGHT, args.capture_height)
        fps_ok = camera.set(cv2.CAP_PROP_FPS, args.fps)
        buffer_ok = camera.set(cv2.CAP_PROP_BUFFERSIZE, 1)
        actual_fourcc = int(camera.get(cv2.CAP_PROP_FOURCC))
        actual_fourcc = ''.join(chr((actual_fourcc >> (8*i)) & 255) for i in range(4))
        print(f'Camera reports {camera.get(cv2.CAP_PROP_FRAME_WIDTH):g}×{camera.get(cv2.CAP_PROP_FRAME_HEIGHT):g}'
              f'@{camera.get(cv2.CAP_PROP_FPS):g} FPS {actual_fourcc!r} '
              f'(FPS request accepted={fps_ok}, FourCC accepted={fourcc_ok}); '
              f'buffer request accepted={buffer_ok}, reports {camera.get(cv2.CAP_PROP_BUFFERSIZE):g}. '
              'Reported FPS is not a measured rate; driver latency is not included in capture_age_ms.', file=sys.stderr)
        worker = threading.Thread(target=capture_worker, args=(camera, slot, stop), name='native-camera', daemon=True)
        worker.start()
        started = report_at = time.monotonic()
        last_count = warmup = 0
        while not args.duration or time.monotonic()-started < args.duration:
            item = slot.take(.1)
            if item is None:
                sender.send(lost_pose())
                if slot.closed:
                    raise RuntimeError(slot.error or 'Camera capture stopped.')
                continue
            frame, captured_at = item
            infer_start = time.monotonic()
            frame = inference_frame(frame, args.inference_width, args.inference_height, cv2)
            faces = tracker.predict(frame)
            pose = extract(max(faces, key=lambda face: face.conf)) if faces else lost_pose()
            if pose['tracked'] and warmup < 3:
                warmup += 1
                pose = lost_pose()
            packet = sender.send(pose, captured_at, (time.monotonic()-infer_start)*1000)
            now = time.monotonic()
            if now-report_at >= 5:
                captured, consumed, superseded, pending = slot.counts()
                print(f'Capture measured {(captured-last_count)/(now-report_at):.1f} FPS; '
                      f'captured={captured} consumed={consumed} superseded={superseded} pending={pending}; '
                      f'capture_age={packet["capture_age_ms"]:.1f}ms inference={packet["inference_ms"]:.1f}ms '
                      f'tracked={packet["tracked"]}', file=sys.stderr)
                last_count, report_at = captured, now
    finally:
        stop.set()
        if worker is None:
            camera.release()
        else:
            worker.join(timeout=2)
            if worker.is_alive():
                # Avoid releasing a VideoCapture concurrently with a blocked read.
                # Process teardown closes its descriptor after this warning.
                print('Camera driver read did not stop within 2s; exiting to release its device handle.', file=sys.stderr)


def parser():
    cli = argparse.ArgumentParser(description=__doc__)
    mode = cli.add_mutually_exclusive_group(required=True)
    mode.add_argument('--camera', help='Explicit device index or /dev/v4l/by-id path. Opens camera only in this mode.')
    mode.add_argument('--list-cameras', action='store_true', help='List Linux device metadata without opening devices.')
    mode.add_argument('--synthetic', action='store_true', help='Send generated pose data; no camera or inference dependencies.')
    cli.add_argument('--port', type=int, default=39539)
    cli.add_argument('--fps', type=float, default=30., help='Requested camera/synthetic FPS; default 30 for Pocket 3 MJPG.')
    cli.add_argument('--capture-width', '--width', type=int, default=1280, help='Requested camera width (default 1280).')
    cli.add_argument('--capture-height', '--height', type=int, default=720, help='Requested camera height (default 720).')
    cli.add_argument('--inference-width', type=int, default=640)
    cli.add_argument('--inference-height', type=int, default=360)
    cli.add_argument('--fourcc', default='MJPG', help='Requested four-character camera pixel format, default MJPG.')
    cli.add_argument('--max-pose-age-ms', type=float, default=250., help='Older inference results send loss; default 250ms.')
    cli.add_argument('--model', type=int, choices=range(5), default=3, help='OpenSeeFace model (default 3, highest accuracy).')
    cli.add_argument('--openseeface-home', default=os.environ.get('OPENSEEFACE_HOME', str(Path.home()/'.local/share/stream-kit/OpenSeeFace')))
    cli.add_argument('--duration', type=float, default=0., help='Stop after this many seconds; 0 runs until Ctrl-C.')
    cli.add_argument('--burst', type=int, default=0, help='Synthetic only: send N datagrams without pacing and exit.')
    cli.add_argument('--loss-after', type=float, default=-1., help='Synthetic only: begin tracking loss at this elapsed second.')
    cli.add_argument('--loss-for', type=float, default=1., help='Synthetic loss duration in seconds.')
    return cli


def main(argv=None):
    cli = parser()
    args = cli.parse_args(argv)
    if (not 1 <= args.port <= 65535 or not math.isfinite(args.fps) or not 0 < args.fps <= 1000
            or any(not 16 <= n <= 4096 for n in (args.capture_width, args.capture_height, args.inference_width, args.inference_height))
            or len(args.fourcc) != 4 or not args.fourcc.isascii() or not args.fourcc.isprintable()
            or not math.isfinite(args.max_pose_age_ms) or not 0 < args.max_pose_age_ms <= 60000
            or not math.isfinite(args.duration) or args.duration < 0 or args.burst < 0
            or not math.isfinite(args.loss_after) or not math.isfinite(args.loss_for) or args.loss_for < 0):
        cli.error('Invalid port, dimensions, FPS, duration, or synthetic loss/burst arguments.')
    if args.list_cameras:
        list_cameras()
        return 0
    def terminate(signum, frame):
        raise KeyboardInterrupt
    signal.signal(signal.SIGTERM, terminate)
    sender = Sender(args.port, args.max_pose_age_ms)
    try:
        (synthetic if args.synthetic else real_capture)(args, sender)
        return 0
    except KeyboardInterrupt:
        print('Capture stopped.', file=sys.stderr)
        return 0
    except Exception as error:
        print(f'Capture error: {error}', file=sys.stderr)
        return 1
    finally:
        try:
            sender.send(lost_pose())
        except OSError:
            pass
        sender.close()


if __name__ == '__main__':
    raise SystemExit(main())
