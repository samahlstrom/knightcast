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

## Initial validation boundaries (superseded where noted below)

- No live camera was opened during this work. Physical head direction, real face reacquisition, lip sensitivity, camera sharing and end-to-end latency still need a user-controlled live session.
- No visible native window was opened. Wayland desktop presentation, window capture, OBS alpha/transparency and OBS integration remain unverified. Existing OBS scenes, services and windows were left intact.
- The first native body pose is static. Mouse/keyboard arm and finger animation, gaze/smile/surprise rendering, near/far torso movement, VRM import and Windows support are not complete.
- The application/renderer is Rust; camera inference is a separate local Python/OpenSeeFace process. Models and personal artwork are external inputs, not bundled release assets.

## Subsequent live preview and color correction

After the initial record above, the user requested a live trial. The Pocket 3 was released from the old browser tracker and opened by the native capture helper. The device accepted MJPEG 1280×720 at 30 FPS; logs showed measured capture at approximately 30 FPS, valid face poses and approximately 10–14 ms inference durations. The native desktop preview was visually verified receiving approximately 30 packets/second. These timings still exclude sensor exposure, hidden driver buffering and final display presentation. OBS capture integration remains unverified.

The first desktop preview revealed a color-path error that offscreen screenshots did not catch: egui was given an sRGB texture view, decoding already-encoded scene output before treating it as gamma-encoded color. The preview now receives an `Rgba8Unorm` view of the same output texture. Scene rendering, input textures and PNG export retain their existing sRGB handling.

The explicit GPU regression test `preview_preserves_rendered_srgb_colors` renders gray, green, red and silver/blue swatches through the actual egui shader. It verifies the preview against the scene bytes within one 8-bit channel value on both linear-format and sRGB-format output attachments. A negative control using the previous view reproduces the darkening. Run it without a window using `cargo test --locked preview_preserves_rendered_srgb_colors -- --ignored`; it is excluded from ordinary CPU-only CI because it requires a GPU.

## Resting eyes and reset controls

The native app now calibrates from two seconds of distinct fresh samples, taking a low percentile for resting eye/mouth values and a sign-aligned quaternion average for orientation. By default both eyes follow the more-open normalized eye, with a resting dead zone and separate 35 ms smoothing. This targets the asymmetric squint reported with the off-center camera without changing head smoothing. Independent winks remain available by disabling eye linking.

Fourteen CPU/loopback Rust tests now pass, including asymmetric-eye jitter, full-blink preservation, calibration with incidental blinks and quaternion sign changes, migration of older settings, and distinct framing/full-reset behavior. Formatting, strict Clippy and the optimized build passed. The app was reopened using the user's saved settings; the corrected colors, new controls and active live tracking were visually verified. Capture remained at approximately 30 FPS with approximately 9–11 ms inference during that check. Perceived blink quality still benefits from the user's live feedback; a screenshot cannot prove the absence of temporal flicker.


## Stream-kit integration

The native renderer now consumes the user's existing loopback WebSocket event hub.
Read-only live hello confirmed keyboard/mouse service health. Isolated synthetic
WebSocket fixtures verified keyboard finger cycling, left-index/right-middle mouse
mapping, bounded pointer accumulation, disconnect release, and timed stars/annoyed
expressions. Ten reference arm poses match the user's original JavaScript skeleton
coordinates/angles. Native screenshots with these messages were inspected; no fake
viewer or input events were injected into the live hub.

The native RGBA output uses background GPU readback, linear-space unpremultiplication,
one latest CPU frame, FFmpeg NUT muxing and loopback TCP. Opt-in tests verified actual
GPU-to-TCP pixel/alpha preservation, fresh streams on reconnect, disconnects for
stalled consumers and bounded child shutdown. An isolated OBS Media Source probe
preserved generated RGBA values with alpha 0,64,128,255 exactly. It created a temporary
unselected scene/source and removed both, verifying existing scene/source names and
program selection unchanged. `scripts/obs_alpha_probe.py` reproduces this explicit
check; it refuses to run during streaming/recording.

The stream-kit-managed service was started on this workstation. Its camera logged
approximately 30 samples/second, valid face tracking and roughly 10–13 ms inference.
The existing OBS scene now contains the native Media Source with the original avatar
position/crop/scale/visibility; the previous browser source is retained hidden for
rollback. The actual native-source screenshot showed the approved art with alpha,
and OBS reported the network Media Source playing. No broadcast or recording was
started. This does not measure total camera-to-stream latency.

The native window and renderer remain local. Personal artwork, live avatar images,
OBS credentials, camera frames and workstation settings are not published here.


The independent OBS frame-worker regression rendered 24 decoded transparent frames
with no UI updates and responded to newly supplied UDP poses. Shutdown completed
within two seconds. Production logs after restart showed 600 new readbacks and 600
written video frames over ten seconds (60 FPS), latest converted-frame age around
14 ms, connected input, active face tracking and no output errors. This frame-age
metric excludes camera and OBS/display latency. OBS automatically reconnected after
the stream-kit-managed avatar restart. Final standard tests: 24 passed, six opt-in
GPU/external-fixture tests excluded from the default run; the integration-specific
opt-in tests were run separately as described above. Formatting, strict Clippy and
the release build passed.

## Restored surprise expression and black eyebrows

The original stream-kit eye-pop motion and trigger were ported to the native path.
The trigger requires sustained fresh tracking, unamplified lip opening and both
calibrated eyes widened; a blink, a brief spike, talking alone or widened eyes alone
cannot trigger it. Face calibration records median resting eye openness. Eyebrows
use the existing curved shape with radius0.055 instead of0.035 and the helmet's
black ink color, as requested.

Geometry and visibility tests verify exactly three white pairs (original first
size, stages1/1.55/2.15), increasing depth along the rotated helmet's forward axis,
replacement of normal/voice eyes without a fourth pair, and thicker ink brows.
Actual CLI synthetic-UDP front, turned, brows-only and neutral screenshots were
visually inspected. The real egui GPU color regression passed after the brow-opacity
uniform change. Final standard tests:31passed,sixopt-inGPU/external tests excluded;
formatting, strict Clippy and the optimized build passed. These are synthetic
expression checks; actual trigger comfort depends on the user's resting-face
calibration and camera view.

## Wrist overlap, keyboard trim and brow placement

The external stream-kit rig now uses an additional rigid wrist-mounted plate from
its existing keyboard-side gauntlet artwork, behind the mouse glove. Native neutral,
maximum inward bend and outward-reach screenshots were inspected. A convex keyboard
outline removes the extracted wood fringe; an existing desk scanline fills the old
rectangular cutout below it. Source PNGs remain unchanged. Brows are 24% wider,
0.16 model units higher and pure black, with a stronger opacity response. The brow
geometry test, strict Clippy and release build passed; isolated synthetic-input
renders verified these changes without injecting events into the live hub.

The subsequent simplification removes the extra mouse wrist plate. Both arms now
have the same eight main arm/hand slots (excluding individual fingers), with one
gauntlet each. They share the keyboard gauntlet artwork; the mouse attachment is
mirrored and fitted once to its forearm length, then moved closer to the wrist for
overlap. Neutral, full inward bend and outward-reach previews were checked. The
keyboard trim and eyebrow improvements remain in place. Source art is unchanged.
