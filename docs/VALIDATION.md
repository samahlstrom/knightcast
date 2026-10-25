# Validation record — 2026-10-25

This records observed results, not feature-parity or end-to-end latency claims.

## Source and build

- Initial source commit: `afde2ef2d19c07013ecd0196b42f0ec4296c2e9f`.
- Local Linux Rust toolchain: `rustc 1.100.0-nightly (17fd5b8a3 2026-08-28)`.
- `cargo build --release --locked`, `cargo fmt --check`, and `cargo clippy --locked --all-targets -- -D warnings` passed.
- Nine Rust tests passed, including the actual loopback socket, latest-sample storage, invalid-packet preservation, quaternion calibration, time-based smoothing, grace/recovery and config round-trip.
- Fourteen standard-library Python tests passed, including signed pose axes, lips-only extraction, bounded capture storage, letterbox geometry, stale-inference rejection, shutdown and serialization.
- [Linux GitHub Actions run](https://github.com/samahlstrom/knightcast/actions/runs/40236401401) passed on the published source.

## Native GPU / supplied artwork

Device: NVIDIA GeForce RTX 4090, driver 610.57.04, wgpu Vulkan backend. Offscreen output: 1254×1300, 4× MSAA.

The final optimized binary loaded the user's unchanged rounded helmet GLB and original cutout layers: 66 GPU primitives, including the inner jaw sleeves. Neutral and combined turn/nod/tilt/open-mouth images were rendered and visually inspected. Textures, red sculpted plume, outlines, collar layering, full head depth and transparent PNG output were checked. No browser engine or displayed window was used. Personal-art screenshots are deliberately not included in the public repository.

Runtime results:

- Initial native renderer build, before the final jaw-sleeve/transparent-PNG refinements: 36,001 frames in 600.801 seconds at a 60 FPS cap. The test received 120 synthetic poses per second, a five-second loss interval, a 10,000-datagram burst, three malformed datagrams and final sender shutdown. It accepted 75,185 packets, rejected all three malformed packets, and superseded 42,224 unread snapshots rather than replaying a backlog. UDP delivery of every burst packet is not guaranteed or required. It recovered after loss and ended in Lost/neutral after sender shutdown.
- Sampled host RSS during the long run stayed between 325,656 and 325,668 KiB (about 318 MiB), with nine threads. This observation does not prove the absence of every possible leak.
- Final optimized renderer with jaw sleeves: 7,201 frames in 120.013 seconds at the same 60 FPS cap, receiving 30 synthetic poses per second, a five-second loss interval and final sender shutdown. It accepted 3,001 packets, reported no rejected packets or superseded unread snapshots, and returned to Lost/neutral. This run validates the final renderer changes separately from the longer earlier run.

Both tests were offscreen and synthetic. Render FPS is submission cadence with a configured sleep cap, not a measurement of desktop presentation or camera-to-display latency. RSS measures host process memory, not total GPU allocation. Synthetic packet ages do not represent physical camera latency.

## Capture helper

The installed OpenSeeFace model 3 initialized and inferred on generated blank pixels without opening a camera. Real OpenCV letterboxing was checked with generated pixels. Actual localhost delivery was checked with the synthetic helper, including final loss signaling. The shell launcher's child cleanup was tested with fake child processes.

A read-only camera mode query identified the Pocket 3's MJPEG modes as 720p/1080p at 30 or 25 FPS; its listed 60 FPS mode is 4K H.264. The helper requests MJPEG 1280×720 at 30 FPS and uses aspect-preserving 640×360 inference. This is a requested configuration, not a claim that live capture achieved it.

## Not yet verified or implemented

- No live camera was opened during this work. Physical head direction, real face reacquisition, lip sensitivity, camera sharing and end-to-end latency still need a user-controlled live session.
- No visible native window was opened. Wayland desktop presentation, window capture, OBS alpha/transparency and OBS integration remain unverified. Existing OBS scenes, services and windows were left intact.
- The first native body pose is static. Mouse/keyboard arm and finger animation, gaze/smile/surprise rendering, near/far torso movement, VRM import and Windows support are not complete.
- The application/renderer is Rust; camera inference is a separate local Python/OpenSeeFace process. Models and personal artwork are external inputs, not bundled release assets.

## Subsequent live preview and color correction

After the initial record above, the user requested a live trial. The Pocket 3 was released from the old browser tracker and opened by the native capture helper. The device accepted MJPEG 1280×720 at 30 FPS; logs showed measured capture at approximately 30 FPS, valid face poses and approximately 10–14 ms inference durations. The native desktop preview was visually verified receiving approximately 30 packets/second. These timings still exclude sensor exposure, hidden driver buffering and final display presentation. OBS capture integration remains unverified.

The first desktop preview revealed a color-path error that offscreen screenshots did not catch: egui was given an sRGB texture view, decoding already-encoded scene output before treating it as gamma-encoded color. The preview now receives an `Rgba8Unorm` view of the same output texture. Scene rendering, input textures and PNG export retain their existing sRGB handling.

The explicit GPU regression test `preview_preserves_rendered_srgb_colors` renders gray, green, red and silver/blue swatches through the actual egui shader. It verifies the preview against the scene bytes within one 8-bit channel value on both linear-format and sRGB-format output attachments. A negative control using the previous view reproduces the darkening. Run it without a window using `cargo test --locked preview_preserves_rendered_srgb_colors -- --ignored`; it is excluded from ordinary CPU-only CI because it requires a GPU.
