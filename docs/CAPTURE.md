# Native capture helper

KnightCast renders in Rust. `scripts/native_capture.py` is a separate Python/ONNX
inference sidecar, with native OpenCV camera capture and loopback UDP output. It
does not use a browser, HTTP, WebSockets, cloud inference, or frame recording.
It never starts without an explicit mode; `--camera` is required to open a device.
The native app does not automatically launch this helper.

From the repository root, list Linux device metadata without opening devices:

```sh
python scripts/native_capture.py --list-cameras
```

When you choose to switch away from your current camera owner, explicitly stop
that owner's capture yourself. Then run this command with your chosen camera
index or stable `/dev/v4l/by-id/...` path:

```sh
~/.local/share/stream-kit/openseeface-venv/bin/python scripts/native_capture.py --camera 0 --fps 30
```

The helper does not terminate other processes or change OBS, browser, desktop, or
camera service settings. Busy/inaccessible cameras produce an error. Linux
metadata listing may include noncapture nodes; select the working capture node.
Ctrl-C requests capture shutdown, releases the device, and emits tracking loss.
A defective driver that blocks indefinitely in `read()` cannot be interrupted
portably by OpenCV: after a two-second shutdown join, the process exits and the OS
closes the camera descriptor. This exceptional path logs a warning.

The existing OpenSeeFace installation defaults to
`~/.local/share/stream-kit/OpenSeeFace`. Override with `--openseeface-home PATH` or
`OPENSEEFACE_HOME`. Dependencies are OpenCV, NumPy (used by OpenSeeFace), ONNX Runtime,
and the installed OpenSeeFace module/models. No upstream code, models, or package
downloads are bundled or performed. The already installed venv provides them.
ONNX Runtime telemetry is explicitly disabled before tracker construction.
Model 3 is the default highest-accuracy model from the authorized existing
backend; `--model 0` through `--model 4` select alternatives present in the install.

Defaults request `--fps 30 --capture-width 1280 --capture-height 720 --fourcc MJPG`,
matching a Pocket 3 MJPG mode found by read-only device mode inspection. That
inspection found MJPG 1280×720 and 1920×1080 at 30/25 FPS; 60 FPS was advertised
for H264 4K, not MJPG. This is a mode request, not a verified live capture result.
Generic cameras can override all these options; `--width`/`--height` are aliases
for the requested capture dimensions. The Linux backend explicitly requests
MJPG (or your FourCC override) and reports the accepted format and request result.
Startup also reports dimensions/FPS and buffer settings. Every
five seconds it reports measured capture FPS, frame counters, inference time and
frame age. Device-reported FPS is not proof of achieved FPS. Inference separately
defaults to `--inference-width 640 --inference-height 360`; Tracker is initialized
with those exact dimensions. Actual captured frames are uniformly resized to fit
and centered with black letterboxing if the aspect differs. For example 640×480
input becomes 480×360 plus 80-pixel side borders, preserving face proportions.
Capture FPS and render FPS are independent;
inference throughput depends on model, resolution and CPU.

The dedicated capture thread continuously drains camera frames into one pending
slot. A new frame replaces the pending frame, never appends to a queue. Inference
owns at most one additional frame. `captured = consumed + superseded + pending`;
pending is always 0 or 1. The helper requests a one-frame backend buffer, but
drivers may reject or ignore it. `capture_age_ms` measures monotonic time from a
successful OpenCV read to sending, including slot wait, resize, and inference.
It cannot measure sensor exposure delay or hidden driver buffering and is not an
end-to-end latency guarantee. `inference_ms` includes resize and inference.
No intermediate images are saved or included in logs.

The first three successful detections send loss while tracker statistics settle.
No face sends loss; stalled capture emits loss every 100 ms while inference is
idle. Failure to read closes capture and exits with an actionable diagnostic;
there is no silent camera reconnect. A long-running inference can delay all
sends; the receiver's own monotonic grace timer still handles that gap. Results
older than `--max-pose-age-ms 250` emit loss rather than refresh the receiver with
an obsolete pose. This threshold is configurable separately from protocol bounds
and applies to post-read age/inference time, not unknown sensor/driver delay. The
helper sends a final loss packet on normal stop/error. UDP delivery is not
guaranteed, so the receiver must retain its timeout behavior.

## Coordinate and expression contract

Output follows [TRACKING_PROTOCOL.md](TRACKING_PROTOCOL.md), exclusively to
`127.0.0.1:39539` (`--port` changes the port). Each process generates an optional
UUID `session_id`; `seq` starts at zero. The receiver accepts latest valid arrival
and treats sequence/session values as diagnostics, so restarting never requires
the sequence counter to catch up. This is the custom JSON bridge, not raw
OpenSeeFace binary UDP or VMC/OSC.

The user's own `stream-kit/puppet/tracking/openseeface_server.py` extraction and
`orientation.mjs` matrix convention are the authorized provenance. Their logic
was adapted into this standalone helper; it does not import the browser server,
and no VMagicMirror, VSeeFace, or VirtualMotionCapture source was inspected.
Source neutral Euler `(180,0,90)` is corrected to
`pitch=wrap(e0−180), yaw=wrap(e1), roll=wrap(e2−90)` degrees. Quaternion output is
active right-handed **qz(roll) × qy(yaw) × qx(pitch)** in `[x,y,z,w]` order,
equivalent to the authorized matrix `Rz × Ry × Rx`. There are no additional axis
sign flips, mirror transforms, calibration, or smoothing in the sidecar.

Application basis: +X screen-right/avatar-left, +Y up, +Z toward the viewer.
Positive pitch rotates +Y toward +Z (a forward +Z vector tips down); positive yaw
rotates +Z toward +X; positive roll rotates +X toward +Y, counterclockwise on
screen. Unit tests independently check both signs of all three axes and a
combined rotation against the authorized matrix. Physical movement signs still
require live validation with this particular camera orientation; no live camera
was opened during implementation.

`mouth_open` is the mean of inner-lip landmark gaps (59,65), (60,64), (61,63)
divided by inner mouth width (58,62), clamped to [0,1]. Chin/jaw motion is excluded.
Blink and wide-eye values retain the authorized landmark ratios. Gaze retains
the authorized iris offsets/signs (+X avatar-left, +Y up). Smile retains the
authorized mouth-corner metric. `head_scale` is currently fixed to 1; there is no
camera-distance calibration or body scaling in this helper. Invalid/nonfinite
landmarks or pose angles produce a loss packet with identity and safe defaults.

## Headless checks and synthetic sender

These commands do not open a camera or import inference packages:

```sh
python -m unittest discover -s scripts -p 'test_native_capture.py' -v
python scripts/native_capture.py --synthetic --duration 5
python scripts/native_capture.py --synthetic --duration 8 --loss-after 2 --loss-for 2
python scripts/native_capture.py --synthetic --burst 10000
```

Synthetic mode creates deterministic pose curves at the requested FPS, sends the
same schema to loopback, and prints an explicit SYNTHETIC label. Loss injection
and restarting the command exercise receiver grace/reacquisition and sequence
resets. Burst mode sends without pacing and ends with a loss packet, so a receiver
stalled across the entire burst should observe the final loss as newest state.
Use `--duration` to bound a camera session as well, when you explicitly choose to
perform live validation. Tests use generated numeric landmark fixtures and a
fake camera: they are evidence for math, bounded storage, serialization and
release behavior, not evidence of live tracking, real driver buffering, or camera
FPS. Actual camera/OpenSeeFace integration remains pending an explicit switch.

Implementation checks on 2026-10-25: all 14 unit tests passed on system Python
3.14. A 0.3-second synthetic run delivered 19 consecutive datagrams to an actual
ephemeral loopback UDP socket, including final loss, and exited successfully.
The installed Python 3.12 venv initialized model 3 and inferred once on an
in-memory 640×480 black array (zero faces); startup plus inference took 209 ms.
Installed versions were OpenCV 4.11.0, NumPy 1.26.4 and ONNX Runtime 1.31.0.
That dependency smoke test used no camera and establishes neither live accuracy
nor steady-state inference FPS. After separating capture and inference dimensions,
model 3 also passed blank-array inference at the new 640×360 default; an actual
OpenCV pixel check verified a generated 4:3 white frame becomes a 480×360 white
region with exact 80-pixel black side borders in the 640×360 result.

## Optional local launcher

After building `cargo build --release --locked`, the launcher opens the native
window with external assets. Its default asset directory is the adjacent
`../stream-kit`, overridable with `--asset-root` or `KNIGHTCAST_ASSET_ROOT`:

```sh
scripts/run-local.sh --asset-root /path/to/stream-kit
# Explicitly opts into camera capture, when you choose to switch:
scripts/run-local.sh --asset-root /path/to/stream-kit --camera 0 --fps 30
```

No camera helper starts unless `--camera` is provided. The optional helper uses
the installed OpenSeeFace venv and the defaults described above. `--port` applies
to both processes. Native-only arguments follow `--`. Override executable paths
with `KNIGHTCAST_BIN` and `KNIGHTCAST_PYTHON`; use the helper directly for other
camera formats/resolutions. Closing either child stops the other; Ctrl-C/TERM
shuts down only the children this launcher created. Termination allows five
seconds before forcing those children to exit if stuck. It never stops the
existing browser/OBS/tracking services. The launcher was checked with fake
children for optional-camera behavior and child shutdown; no visible window or
real camera was started for that check.
