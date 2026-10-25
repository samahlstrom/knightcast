"""Headless fixtures: no image files, NumPy, camera, models, or listening service."""
import json
import math
from types import SimpleNamespace
import unittest
from unittest.mock import patch

import native_capture as capture


def face_fixture():
    p = [[0., 0., 0.] for _ in range(68)]
    # Horizontal eye widths 1, eye aperture .23; neutral iris centered.
    for start, offset in [(36, -2), (42, 2)]:
        for i, xy in enumerate([(0, 0), (.25, .115), (.75, .115), (1, 0), (.75, -.115), (.25, -.115)]):
            p[start+i] = [xy[0]+offset, xy[1], 0.]
    p[66], p[67] = [-1.5, 0., 0.], [2.5, 0., 0.]
    p[58], p[62] = [-.5, -1., 0.], [.5, -1., 0.]
    for a, b, x in [(59, 65, -.25), (60, 64, 0), (61, 63, .25)]:
        p[a], p[b] = [x, -.95, 0.], [x, -1.05, 0.]
    for i in [49, 50, 51]:
        p[i] = [0., -1., 0.]
    return SimpleNamespace(success=True, pts_3d=p, euler=[180., 0., 90.])


def rotate(q, v):
    x, y, z, w = q
    # Independent standard quaternion-to-matrix formula.
    m = [[1-2*(y*y+z*z), 2*(x*y-z*w), 2*(x*z+y*w)],
         [2*(x*y+z*w), 1-2*(x*x+z*z), 2*(y*z-x*w)],
         [2*(x*z-y*w), 2*(y*z+x*w), 1-2*(x*x+y*y)]]
    return [sum(a*b for a, b in zip(row, v)) for row in m]


class ExtractionTests(unittest.TestCase):
    def assertVector(self, actual, expected):
        for a, b in zip(actual, expected):
            self.assertAlmostEqual(a, b, places=12)

    def test_neutral_and_raw_inner_lips(self):
        face = face_fixture()
        pose = capture.extract(face)
        self.assertTrue(pose['tracked'])
        self.assertVector(pose['rotation'], [0, 0, 0, 1])
        self.assertAlmostEqual(pose['mouth_open'], .1)
        self.assertAlmostEqual(pose['blink_left'], 0.)
        self.assertAlmostEqual(pose['eye_wide_right'], .23/.6)
        self.assertVector(pose['gaze'], [0, 0])
        self.assertAlmostEqual(pose['smile'], .5)
        # Chin and outer-jaw changes have no effect on mouth input.
        for i in range(17):
            face.pts_3d[i] = [100., -100., 500.]
        self.assertEqual(capture.extract(face)['mouth_open'], pose['mouth_open'])

    def test_independent_signed_axes(self):
        for euler, vector, expected in [
                ([270, 0, 90], [0, 1, 0], [0, 0, 1]),
                ([180, 90, 90], [0, 0, 1], [1, 0, 0]),
                ([180, 0, 180], [1, 0, 0], [0, 1, 0]),
                ([90, 0, 90], [0, 1, 0], [0, 0, -1]),
                ([180, -90, 90], [0, 0, 1], [-1, 0, 0]),
                ([180, 0, 0], [1, 0, 0], [0, -1, 0])]:
            face = face_fixture()
            face.euler = euler
            self.assertVector(rotate(capture.extract(face)['rotation'], vector), expected)

    def test_combined_rotation_matches_authorized_orientation_matrix(self):
        pitch, yaw, roll = [math.radians(v) for v in (23, -37, 61)]
        a, b, c, d, e, f = math.cos(pitch), math.sin(pitch), math.cos(yaw), math.sin(yaw), math.cos(roll), math.sin(roll)
        expected = [[c*e, b*d*e-a*f, a*d*e+b*f], [c*f, b*d*f+a*e, a*d*f-b*e], [-d, b*c, a*c]]
        q = capture.quaternion_zyx(23, -37, 61)
        for i in range(3):
            self.assertVector(rotate(q, [float(i == j) for j in range(3)]), [row[i] for row in expected])

    def test_invalid_face_is_loss(self):
        for mutate in [lambda f: setattr(f, 'success', False),
                       lambda f: f.pts_3d[20].__setitem__(0, float('nan')),
                       lambda f: f.euler.__setitem__(1, float('inf')),
                       lambda f: setattr(f, 'pts_3d', []),
                       lambda f: setattr(f, 'euler', [0])]:
            face = face_fixture()
            mutate(face)
            self.assertEqual(capture.extract(face), capture.lost_pose())

    def test_eye_and_gaze_mapping(self):
        face = face_fixture()
        for i in range(42, 48):
            face.pts_3d[i][1] = 0.
        face.pts_3d[66][0] -= .1
        face.pts_3d[67][0] -= .1
        face.pts_3d[66][1] -= .1
        face.pts_3d[67][1] -= .1
        pose = capture.extract(face)
        self.assertEqual(pose['blink_left'], 1.)
        self.assertEqual(pose['eye_wide_left'], 0.)
        self.assertAlmostEqual(pose['blink_right'], 0.)
        self.assertVector(pose['gaze'], [.3, .3])


class LatestFrameTests(unittest.TestCase):
    def test_burst_and_stalled_consumer_have_one_pending_frame(self):
        slot = capture.LatestFrame()
        for i in range(10000):
            slot.put(i, float(i))
        self.assertEqual(slot.counts(), (10000, 0, 9999, 1))
        self.assertEqual(slot.take(0), (9999, 9999.))
        self.assertIsNone(slot.take(0))
        slot.put('next', 10000.)
        self.assertEqual(slot.take(0), ('next', 10000.))
        self.assertEqual(slot.counts(), (10001, 2, 9999, 0))

    def test_close_and_accounting(self):
        slot = capture.LatestFrame()
        slot.put('frame', 1.)
        slot.close('disconnected')
        self.assertFalse(slot.put('late', 2.))
        self.assertEqual(slot.take(0), ('frame', 1.))
        self.assertIsNone(slot.take(0))
        self.assertEqual(slot.error, 'disconnected')
        self.assertEqual(slot.counts(), (1, 1, 0, 0))

    def test_worker_releases_camera_on_read_failure(self):
        class FakeCamera:
            released = False
            def read(self):
                return False, None
            def release(self):
                self.released = True
        camera = FakeCamera()
        slot = capture.LatestFrame()
        capture.capture_worker(camera, slot, capture.threading.Event())
        self.assertTrue(camera.released)
        self.assertTrue(slot.closed)
        self.assertIn('stopped returning', slot.error)


class SenderTests(unittest.TestCase):
    @patch('native_capture.socket.socket')
    def test_packets_are_loopback_finite_sequenced_and_restart_identified(self, socket):
        sender = capture.Sender(39539)
        first = sender.send(capture.extract(face_fixture()))
        second = sender.send(capture.lost_pose())
        self.assertEqual((first['seq'], second['seq']), (0, 1))
        data, destination = socket.return_value.sendto.call_args.args
        self.assertEqual(destination, ('127.0.0.1', 39539))
        self.assertFalse(json.loads(data)['tracked'])
        restarted = capture.Sender(39539)
        self.assertEqual(restarted.seq, 0)
        self.assertNotEqual(sender.session_id, restarted.session_id)
        sender.close()
        socket.return_value.close.assert_called_once()

    @patch('native_capture.socket.socket')
    def test_extremely_stale_inference_emits_loss(self, socket):
        sender = capture.Sender(39539)
        packet = sender.send(capture.extract(face_fixture()), capture.time.monotonic()-61, 61000)
        self.assertFalse(packet['tracked'])
        self.assertEqual(packet['capture_age_ms'], 60000)
        self.assertEqual(packet['inference_ms'], 60000)

    @patch('native_capture.socket.socket')
    @patch('native_capture.time.monotonic', return_value=10.)
    def test_practical_age_cutoff_and_override(self, monotonic, socket):
        sender = capture.Sender(39539)
        pose = capture.extract(face_fixture())
        self.assertFalse(sender.send(pose, captured_at=9.)['tracked'])
        self.assertFalse(sender.send(pose, inference_ms=1000.)['tracked'])
        self.assertTrue(sender.send(pose, captured_at=9.9)['tracked'])
        self.assertTrue(capture.Sender(39539, max_pose_age_ms=1500).send(pose, captured_at=9.)['tracked'])


class GeometryTests(unittest.TestCase):
    def test_capture_and_inference_defaults_are_separate(self):
        args = capture.parser().parse_args(['--camera', '0'])
        self.assertEqual((args.capture_width, args.capture_height, args.fps, args.fourcc), (1280, 720, 30, 'MJPG'))
        self.assertEqual((args.inference_width, args.inference_height), (640, 360))

    def test_known_aspects_and_exact_target_size(self):
        for source, expected in [((1280, 720), (640, 360, 0, 0, 0, 0)),
                                 ((640, 480), (480, 360, 80, 0, 80, 0)),
                                 ((360, 640), (202, 360, 219, 0, 219, 0)),
                                 ((1920, 800), (640, 267, 0, 46, 0, 47))]:
            result = capture.letterbox_geometry(*source, 640, 360)
            self.assertEqual(result, expected)
            width, height, left, top, right, bottom = result
            self.assertEqual(width+left+right, 640)
            self.assertEqual(height+top+bottom, 360)
            self.assertLessEqual(abs(width/source[0]-height/source[1]), 1/min(source))

    def test_resize_and_padding_calls_preserve_aspect(self):
        class FakeCv:
            INTER_AREA, BORDER_CONSTANT = 1, 2
            def resize(self, frame, size, interpolation):
                self.resized = size
                return SimpleNamespace(shape=(size[1], size[0], 3))
            def copyMakeBorder(self, frame, top, bottom, left, right, mode, value):
                self.padding = (top, bottom, left, right)
                return SimpleNamespace(shape=(frame.shape[0]+top+bottom, frame.shape[1]+left+right, 3))
        cv = FakeCv()
        result = capture.inference_frame(SimpleNamespace(shape=(480, 640, 3)), 640, 360, cv)
        self.assertEqual(cv.resized, (480, 360))
        self.assertEqual(cv.padding, (0, 0, 80, 80))
        self.assertEqual(result.shape, (360, 640, 3))


if __name__ == '__main__':
    unittest.main()
