# Implementation provenance

The user requested a new native Rust avatar application and explicitly requested separate specification and implementation agents. Linux is the first target.

## Separation

- The specification agent received a fresh conversation without implementation context. It consulted the public documentation and standalone licenses listed in `PROVENANCE.md`, then produced the behavioral specification and tracking contract.
- The renderer implementation agent received a fresh conversation, the completed specification, and a map of the user's own artwork and rig. It was instructed not to inspect VMagicMirror, VSeeFace, or VirtualMotionCapture implementation source or binaries.
- The capture implementation agent received the same restriction. It was allowed to adapt the user's existing OpenSeeFace integration, independently of the reference applications.
- This records the workflow followed; it is not a claim of formal legal certification or a claim that the Rust app is an upstream port.

## User-owned material

The existing stream-kit project supplies the user's own rigid sprite hierarchy, helmet geometry, approved pivot/collar transforms, and OpenSeeFace landmark extraction math. These are authorized migration inputs, not reference-application code. Personal artwork remains outside this repository and outside its MIT source-code license.

## External software

OpenSeeFace, its models, OpenCV and ONNX Runtime are optional separately installed runtime dependencies of the Python capture helper. They are not included in this repository. The native app and renderer are Rust; initial camera inference uses this local Python helper. No browser is needed in that path.

Rust dependencies are pinned in `Cargo.lock`. Their original copyright and license terms continue to apply. See `THIRD_PARTY.md` for dependency information and distribution boundaries.
