# KnightCast

A native Rust avatar prototype for Linux: the user's rounded 3D knight helmet over their original rigid cutout body. GPU rendering uses wgpu; the settings window uses egui/winit. There is no browser engine, webview, Electron, Tauri, HTTP frontend, account or telemetry. Initial camera inference runs in a separate local Python/OpenSeeFace process.

The source repository contains no character artwork or tracking model weights. Supply your own authorized stream-kit directory. KnightCast reads it without modifying it.

## Build and run

Requires a current Rust toolchain, a Vulkan/OpenGL-capable GPU driver, and Linux Wayland or X11 development/runtime libraries. Rust dependencies download during the build; the app runs locally afterward.

```sh
cargo build --release --locked
./target/release/knightcast --asset-root /path/to/stream-kit
```

The locked dependency graph requires Rust 1.90 or newer. For the existing local stream-kit/OpenSeeFace setup, the launcher can start both processes and close its camera helper when the app exits:

```sh
scripts/run-local.sh --asset-root /path/to/stream-kit --camera 0
```

Release the camera from its previous owner before using `--camera`. Omitting that option opens only the native app. The helper defaults to MJPEG 1280×720 at 30 FPS, with aspect-preserving 640×360 inference; these match the Pocket 3's enumerated MJPEG modes. Other devices can use the capture helper's explicit format options.

Invoking the Rust binary directly opens only the native settings/preview window. The launcher starts the separate camera process only with `--camera`; see [capture documentation](docs/CAPTURE.md). The app binds only `127.0.0.1:39539` by default. It consumes the [KnightCast JSON bridge](docs/TRACKING_PROTOCOL.md), not raw OpenSeeFace/VMC packets. Choose the tracked face in the adapter.

Click **Calibrate face (2 seconds)** while looking normally at your screen with eyes comfortably open and lips together. Calibration samples your resting head, eyes and mouth, ignoring incidental blinks. **Keep both eyes in sync** is enabled by default for off-center cameras; disable it for independent winks. Eye smoothing is separate from head smoothing.

**Reset avatar size and position** restores framing without clearing face calibration. **Reload saved settings** restores the last saved configuration. **Reset all settings to defaults** clears tuning and face calibration while keeping the asset folder and tracking port; click **Save settings** to persist changes. Connection settings and technical diagnostics are collapsed below the everyday controls. F1 hides/shows controls.

Settings live at `$XDG_CONFIG_HOME/knightcast/config.json` or `~/.config/knightcast/config.json`; `--config PATH` overrides this. Saving uses a temporary file followed by atomic rename. Errors appear in the native UI; an invalid saved configuration falls back to defaults. An asset reload failure retains the previous rendered assets.

## Offscreen verification

No window, camera, OBS access or compositor interaction is needed:

```sh
cargo test --locked
./target/release/knightcast --asset-root /path/to/stream-kit \
  --screenshot /tmp/knightcast.png
./target/release/knightcast --asset-root /path/to/stream-kit \
  --screenshot /tmp/knightcast-turn.png --yaw 30 --pitch 10 --mouth 0.6
./target/release/knightcast --asset-root /path/to/stream-kit \
  --screenshot /tmp/knightcast-runtime.png --seconds 600 --fps 60
```

The timed run continuously renders and receives UDP; it reports render submission FPS, accepted/rejected/superseded packets, tracking state and packet age every ten seconds. The FPS includes the configured sleep cap, not isolated GPU execution time. `--width`/`--height` set offscreen resolution, `--port` selects the local UDP port, and `--transparent` exports alpha. Transparent PNGs are supported; transparent desktop capture/OBS integration is not verified. GPU access may require running outside a restrictive development sandbox.

## Implemented behavior

- Actual glTF triangles, UV textures, linear vertex/material colors, sRGB textures, unlit materials, authored outline meshes and 4× MSAA. Unsupported glTF extensions, non-unlit materials, skins and animations are rejected with a diagnostic. This is a rigid-art loader, not a general-purpose glTF viewer.
- Original sprite slot order, bind/source positions, bone transforms and alpha. The initial body pose is static; the 2D head is omitted. External art files are unchanged.
- Full rounded helmet rotates around its center, with the accepted lifted pivot and fixed extended collar. Jaw slides vertically from inner-lip tracking with a neutral baseline and adjustable sensitivity (default 3.8); original eye geometry compresses for blinking. The original fixed inner sleeve and cavity extension keep the sliding jaw enclosed. Helmet yaw/pitch/roll are bounded to 45/25/24 degrees. The model base rotation is independent of calibration.
- A dedicated UDP worker stores one latest packet and one last-valid pose; it does not queue samples. Maximum datagram size is 8192 bytes. Malformed packets preserve the last valid pose. Sequence decreases are diagnostic so restarted senders recover immediately; latest valid arrival wins, including theoretically reordered UDP packets.
- Right-handed active quaternions `[x,y,z,w]`, +X right on screen, +Y up, +Z toward the camera. Relative orientation is `inverse(neutral) * current`; positive yaw rotates the front toward +X, positive pitch tilts the front downward, positive roll is counterclockwise on screen. Shortest-arc quaternion smoothing uses `1-exp(-dt/tau)` with a 25 ms default and no additional rotational filter. This is a smoothing setting, not a measured end-to-end latency promise.
- Monotonic freshness, 250 ms grace and 400 ms neutral return time constant. Loss packets do not refresh the valid-pose timestamp. Recovery uses ordinary smoothing. Capture age and inference duration are displayed separately from local arrival age.

## Current limits

This is an independently implemented native prototype, not full VMagicMirror parity. Live camera/desktop validation must be recorded separately from synthetic/offscreen tests. The body does not yet react to keyboard/mouse activity. Native smile/gaze/wide-eye/surprise rendering, coordinated near/far body motion, output-window capture integration, virtual camera, VRM, packaging and Windows validation are pending. Head-scale and unsupported expression channels are validated but not rendered. The source channel is selected in the adapter; the native app has one custom JSON protocol.

The [behavioral specification](docs/BEHAVIORAL_SPEC.md), [provenance ledger](docs/PROVENANCE.md), [asset contract](docs/KNIGHT_ASSETS.md) and [protocol](docs/TRACKING_PROTOCOL.md) describe the design and evidence boundaries. No reference application's implementation code or binary was inspected for this implementation. Only independently written requirements, standard dependency APIs, and the user's own authorized rig/art were used.

See the [validation record](docs/VALIDATION.md) for build, CI, offscreen GPU, synthetic tracking and stability results, along with the live-camera and desktop checks still outstanding.
