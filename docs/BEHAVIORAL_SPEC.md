# KnightCast: clean-room behavioral specification

Date: 2026-10-25. This is an independently authored product contract, not a description of another application's internals. Source IDs resolve in PROVENANCE.md. Implement independently from this document and standard dependency documentation; do not inspect the reference applications' implementation source or binaries.

## Choice of reference

Use VMagicMirror as the primary behavioral reference: its documented workflow includes keyboard/mouse driven avatar movement, webcam head tracking, calibration, adjustable device layout, and local PNG/GLB attachments [S1–S6]. VirtualMotionCapture's official presentation emphasizes VR equipment and VR-game compositing [S9], so it is less aligned with the immediate seated desktop task. This is a suitability judgment, not a popularity ranking. VSeeFace offers useful public examples of separate tracking and rendering processes, OpenSeeFace network input, and diagnostics [S8].

VMagicMirror and VirtualMotionCapture repository license files are MIT [S7,S10]. This does not make their packaged assets reusable: VMagicMirror documents restricted third-party/built-in assets [S7]. VSeeFace explicitly says its complete application is not open source [S8]. Build a new open-source Rust application; do not fork/copy those applications or redistribute their assets. Preserve the user's supplied art unchanged.

## First vertical slice: M1

Deliver a native Rust executable using a native window/event loop and GPU renderer, running on the user's Linux Wayland desktop. No Chromium, Electron, browser, webview, JS renderer, or local HTTP frontend. Native GPU UI is acceptable. Windows is a later portability target; isolate windowing, config paths, and capture integration accordingly.

1. Load the user's local GLB helmet and RGBA cutout-body image. Render the actual GLB mesh, materials/textures supported by the selected loader, and the supplied body image with alpha. Unsupported GLB features produce actionable diagnostics rather than silent substitutions. Keep the body static in M1; rotate the helmet around its middle, preserving the accepted center pivot and separate collar fit in KNIGHT_ASSETS.md. Provide native controls for model/body scale, offsets, pivot, neutral base orientation, camera framing, and background color. Persist those values. Do not redraw, regenerate, or overwrite artwork. Transparent output is desirable if the Wayland surface supports it; also provide an explicit solid background mode. Do not claim OBS transparency compatibility without testing it.
2. Receive head-pose samples from a separately running local tracker over UDP, binding loopback only by default. Use the versioned JSON bridge contract in TRACKING_PROTOCOL.md. Adapt the user's own existing OpenSeeFace Python backend to direct camera capture and this contract; the Rust application need not parse OpenSeeFace's original binary protocol. Each accepted sample has orientation, validity, sequence, and a locally recorded monotonic receive time. Validate size, field bounds, finite numbers and quaternion norm before publication. Malformed packets must never panic or overwrite a valid pose. Count rejected packets.
3. Use a single latest-sample slot between receiver and renderer. Continuously drain incoming datagrams independently of rendering; overwrite superseded samples. No growing channel, replay backlog, or per-packet render work. Bound socket/worker work and memory. Follow TRACKING_PROTOCOL.md latest-arrival sequence/restart policy; do not reject all samples from a restarted tracker.
4. Neutral calibration is an explicit native UI action, accepted only with a fresh valid pose. Normalize the source quaternion and convert to one documented application basis first. Store that orientation as neutral; compute the relative rotation with a stated multiplication convention (for active column-vector rotations, q_relative = inverse(q_neutral) * q_current). Apply the authored model base pose separately. Include independent yaw/pitch/roll sign fixtures and a nonidentity combined-rotation calibration fixture. Do not subtract Euler angles.
5. Smooth normalized rotations with shortest-arc quaternion interpolation and alpha = 1 - exp(-dt/tau), using elapsed monotonic render time. tau is configurable in milliseconds; zero disables smoothing. q and -q represent the same orientation and must not cause a spin. Avoid multiple smoothing stages with undocumented lag. Track packet age separately from smoothing.
6. Tracking state is visible: Waiting, Tracking, Grace, Lost. Default grace is 250 ms since the last valid tracked sample. Hold the last pose during grace; afterwards ease to neutral using a configurable return time (default 400 ms time constant). Invalid packets and tracked:false packets do not extend grace. Reacquisition uses normal smoothing without snapping. Use a monotonic clock for all freshness decisions.
7. Native controls: asset paths/reload, calibration, smoothing, grace/return duration, source protocol/port, transform controls, and save/reset config. Version the native config, use Linux XDG paths, validate on load, and save atomically. Explain malformed config/assets/occupied UDP port without crashing. Show renderer/backend, render FPS, packet rate, accepted/rejected/superseded counts, age of last valid pose, tracking state, and errors. No raw camera images or biometric recordings in logs by default.
8. Work locally without account, cloud, telemetry, automatic downloads, or online runtime dependency. No launching trackers/camera capture unless explicitly selected. Never move other windows, edit compositor rules, or touch OBS settings/scenes. App-only controls are in scope.

## Protocol boundary

The reviewed public user documentation establishes OpenSeeFace network support but does not specify its byte layout [S8]. This spec deliberately supplies no guessed offsets. The authorized implementation route is the user's own existing Python OpenSeeFace backend, adapted to direct camera capture and the new JSON bridge. Record its provenance independently of the reference applications. A working synthetic UDP receiver alone is an M1 partial, not verified live OpenSeeFace integration. Keep parser and pose processing independent so protocol validation is testable offline.

## Acceptance evidence

| Test | Required result |
| --- | --- |
| Native/local | Process/dependency inspection shows native Rust/GPU UI without browser engine; application runs with external network unavailable. |
| Supplied art | Load user's actual GLB and body; verify texture/alpha, helmet-center pivot and collar alignment, recognizable unchanged art, and source-file hashes unchanged. Record unsupported material features. |
| Offline parsing | Deterministic fixtures cover valid, short, oversized, invalid UTF/binary as applicable, nonfinite, zero quaternion, sequence resets, and malformed field types. No panic; valid pose preserved on rejection. |
| Latest only | Burst many numbered poses while deliberately stalling rendering; next read is newest accepted pose. Storage remains bounded; rendering does not replay the burst. Test the socket receiver as well as the slot. |
| Calibration | A combined nonidentity pose calibrated as neutral produces identity relative rotation; known subsequent yaw/pitch/roll follow documented signs. |
| Smoothing | The same step response at 30, 60, and 144 Hz differs by less than 0.5 degrees at matching times; sign-flipped equivalent quaternions do not rotate. |
| Loss/recovery | Injected monotonic time proves hold inside grace, return afterwards, no refresh from rejected packets, smooth recovery. No wall-clock sleeps required in unit tests. |
| Configuration | Round-trip preserves values; missing/invalid config and failed writes produce clear errors; last good config is not truncated. |
| Headless GPU | Render supplied assets offscreen to an image without a compositor; verify nonempty visible helmet/body and expected alpha/background. If unavailable, report that limitation separately from CPU tests. |
| Actual tracker | Local OpenSeeFace stream drives helmet with correct directions and calibration; source format/version and measured packet age are recorded. Synthetic validation must be labeled synthetic. |
| Runtime | Run at least 10 minutes at the configured frame cap, including a packet burst and disconnect; no accumulating backlog or continuing memory growth after warm-up. Record hardware, resolution, FPS/frametime and packet age; do not promise a hardware-independent FPS. |

Automated CPU tests must run headlessly, with no camera, network service, or GUI. A synthetic sender should optionally exercise the actual loopback UDP receiver and loss/recovery. Separate offline validation, GPU validation, desktop smoke test, and live tracking results in the handoff.

## Later milestones and honest boundary

M2: seated rigid-art upper/lower arms and hands, configurable shoulder/elbow/wrist pivots and keyboard/mouse anchors, bounded reach, idle/typing/mouse motion, adjustable strength. Preserve supplied art; request genuinely missing art instead of redrawing. Keep keyboard content private: animate activity rather than disclose actual typed characters. Linux Wayland global activity collection needs an explicit supported backend and user enablement; focused-window activity is an honest initial limitation. No silent privileged input hooks.

M3: capture-friendly dedicated output, platform-tested transparency, optional lip-sync/expressions/VRM support, packaging, then Windows support and CI. VMC protocol support is a separate feature from the VirtualMotionCapture application.

M1 is a usable native helmet-over-cutout head-tracking prototype. It is not complete VMagicMirror parity, a fully rigged avatar, seated arm interaction, a virtual camera, automatic OBS integration, or proven Windows support. Report missing acceptance evidence precisely; do not label a source-only build as a validated desktop application.

## User-owned integration precedence

KNIGHT_ASSETS.md is authoritative for the supplied artwork, center-of-helmet rotation pivot, collar fit, rigid body layering, mouth/eye behavior, and session safeguards. The body may be static in M1 but must preserve its multi-part layered composition rather than flattening or replacing it. Do not open a visible window or capture the camera during background implementation; use headless/offscreen/synthetic validation, then report live validation as pending until the user chooses to switch.
