# KnightCast loopback tracking bridge v1

Independently specified for the user's existing OpenSeeFace backend; this is NOT OpenSeeFace's native binary protocol or VMC/OSC. One UTF-8 JSON object per UDP datagram to `127.0.0.1:39539`. Default receiver binding is loopback only. Maximum accepted datagram size: 8192 bytes. No HTTP/WebSocket/browser is involved. Python performs direct camera capture/inference; Rust owns rendering, calibration and smoothing.

Required fields:

```json
{"version":1,"seq":42,"tracked":true,"rotation":[0,0,0,1],"mouth_open":0,"blink_left":0,"blink_right":0,"eye_wide_left":0,"eye_wide_right":0,"smile":0,"gaze":[0,0],"head_scale":1,"capture_age_ms":14,"inference_ms":11}
```

- `version`: integer, exactly 1. `seq`: unsigned integer within the exact JSON integer range 0..9007199254740991; increase for each datagram during a sender run. `tracked`: boolean, true only when the chosen face has a valid current inference.
- `rotation`: finite unit quaternion `[x,y,z,w]`, active rotation in a documented application coordinate basis. The bridge must convert its existing tracker basis explicitly and include independent signed-axis fixtures; do not guess the source basis. Identity is the source's reference orientation, not necessarily the user's calibrated neutral. Receiver rejects norm below 0.000001 and normalizes otherwise.
- `mouth_open`, `blink_left`, `blink_right`, `eye_wide_left`, `eye_wide_right`, `smile`: finite floats in [0,1]. Blink is 0=open, 1=closed. Unsupported channels are zero. These channels may remain diagnostic-only for a rigid helmet in M1.
- `gaze`: two finite normalized values [horizontal, vertical], each in [-1,1]; positive horizontal points avatar-left (+X in the selected camera-facing basis), positive vertical points up. Unsupported gaze is [0,0]. Document the rendering basis and any mirror option consistently.
- `head_scale`: finite float in [0.1,10], neutral value 1; may remain diagnostic-only in M1 to avoid unintended body movement.
- `capture_age_ms`: finite float in [0,60000], sender's monotonic elapsed time from frame capture to send. `inference_ms`: finite float in [0,60000], measured inference duration. These are durations, not absolute wall-clock timestamps.

Unknown fields may be ignored for forward compatibility. Missing required fields, bad types/ranges, nonfinite values, oversized data and unsupported versions reject the whole datagram and increment diagnostics. For `tracked:false`, send identity rotation and safe defaults for expressions; receiver records tracking loss without treating this as a fresh valid pose.

Timestamp/order policy: local monotonic receive time is authoritative for grace and freshness; sender and receiver clocks are not compared. Receiver continuously drains socket on a dedicated worker and publishes exactly one latest-arrival snapshot, overwriting previous snapshots. `seq` is diagnostic (gaps/restart), not a backlog index: never wait for missing sequence values. A lower sequence must not permanently reject a restarted sender. For this loopback bridge, latest valid arrival wins; document that reordered datagrams are theoretically possible and add an optional sender-session field in a future protocol if strict ordering is needed. Malformed packets do not replace the snapshot. Keep last-valid-pose time separately from last-datagram time. `tracked:false` snapshots can immediately change diagnostics to Grace but preserve the previous valid pose until its grace expires.

The Python side also needs bounded freshness: capture uses a single newest frame, inference consumes the latest available frame, and no growing work queue exists. A blocking camera API must be checked for internal buffering; report real capture-age limits honestly. Emit loss updates at a bounded cadence (e.g. 10 Hz) while camera/tracker is unavailable. Native render frame rate is independent of tracker cadence.

Testing: provide a deterministic synthetic sender using this same protocol and loopback receiver path; verify burst/latest semantics, sequence reset, malformed packets, loss/grace, and reconnect. Live compatibility means this JSON bridge plus the user's OpenSeeFace backend was tested together, not that arbitrary raw OpenSeeFace UDP traffic is supported.
