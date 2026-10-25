# Source and provenance ledger

Reviewed 2026-10-25 by the specification author. Only public user-facing documentation and standalone license files were opened. No application implementation source, decompilation, binaries, internal algorithms, or copied artwork were inspected. The implementation author should receive this specification and ledger, not reference implementation source.

| ID | Public source | Facts used |
| --- | --- | --- |
| S1 | https://malaybaku.github.io/VMagicMirror/en/ | Windows VRM app; keyboard/mouse, webcam head tracking, expression controls. |
| S2 | https://malaybaku.github.io/VMagicMirror/en/docs/face_tracker/ | Face-pose calibration, tracking quality options, horizontal flip, webcam/external tracking options. |
| S3 | https://malaybaku.github.io/VMagicMirror/en/docs/motion/ | Typing/mouse motion toggle, shoulder and arm adjustments, privacy-oriented random typing option. |
| S4 | https://malaybaku.github.io/VMagicMirror/en/docs/layout/ | Camera FOV/viewpoint persistence and visible device layout settings. |
| S5 | https://malaybaku.github.io/VMagicMirror/en/docs/accessory/ | PNG and GLB/glTF attachment support and local transform controls. |
| S6 | https://malaybaku.github.io/VMagicMirror/en/docs/window/ | Background and transparency settings; Spout output documented for that application. No inference that these mechanisms work on Linux. |
| S7 | https://malaybaku.github.io/VMagicMirror/en/license/ and https://raw.githubusercontent.com/malaybaku/VMagicMirror/master/LICENSE | MIT repository license; documentation distinguishes restricted assets in binary distributions. Only the standalone LICENSE was read from the repository. |
| S8 | https://www.vseeface.icu/ (About, Terms, privacy FAQ, open-source FAQ, Network tracking) | Separate tracker process, OpenSeeFace network input, local tracking, application not open source. No byte-level protocol definition found here. No linked implementation files opened. |
| S9 | https://vmc.info/ | Official Japanese landing page describes VR equipment, VRM control, SteamVR/MR compositing, and configurable hand/controller behavior. Suitability assessment derives from these user-facing descriptions. |
| S10 | https://raw.githubusercontent.com/sh-akira/VirtualMotionCapture/master/LICENSE | MIT repository license; standalone LICENSE only, no implementation inspected. |
| S11 | https://malaybaku.github.io/VMagicMirror/en/docs/ | Documentation index used for discovery. |

Unsuccessful lookup: https://sh-akira.github.io/VirtualMotionCapture/ was inaccessible through the browser tool; no claims derive from it. The official https://vmc.info/ site was used instead.

## Independent requirements and design decisions

The following come from the user's requested product scope and this new design, not a claim about reference-app implementation: Rust/native GPU/Wayland-first; user's helmet plus cutout composition; single latest-frame slot; monotonic timestamps; quaternion calibration formula; exponential time-based smoothing; specific grace/return defaults; bounded queues; loopback default; diagnostics/config requirements; acceptance tolerances; offline tests; no Chromium; no OBS/window manipulation; and milestone boundaries.

The new project may choose MIT or MIT/Apache-2.0 for independently authored application code. Audit dependency licenses and preserve required notices before distribution. User artwork remains separately owned/licensed; do not imply that choosing a source-code license grants redistribution rights to it. Existing OpenSeeFace/tracker/model licenses must be reviewed before bundling; running an independently installed tracker is separate from redistributing it.

## Evidence still required

- Provenance of the user's authorized existing OpenSeeFace Python backend and validation of its adaptation to TRACKING_PROTOCOL.md; raw OpenSeeFace binary support is not required.
- Actual supplied-art paths and hashes (the spec author did not inspect user files).
- Implementation dependency/version/license list and hardware-specific validation record.
- Any claim of capture transparency, live tracker compatibility, or Windows support must have its own test evidence.

## Authorized stream-kit integration (2026-10-25)

The user explicitly requested integration with their existing stream-kit. The
implementation therefore read and ported the user's own `overlays/desk-rig/skeleton.js`
and activity/expression behavior from `overlays/knight-puppet.html`, using their
`overlays/original-rig/rig.json` as an external asset contract. This is authorized
reuse of the user's existing rig, not reference Windows-application implementation
access. No VMagicMirror/VSeeFace/VirtualMotionCapture implementation was inspected.
Private artwork and asset files remain external to this source repository.

OBS Media Source integration used the official OBS `obs-ffmpeg-source.c` settings,
OBS media-source documentation and FFmpeg protocol documentation, plus isolated
pixel/alpha tests against the installed OBS. Those are APIs of the integration
dependencies, not behavioral-reference avatar application source. The source links
are in the implementation task's evidence and the validation documents; actual
transparent output was tested rather than inferred from documentation.
