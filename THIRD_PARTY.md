# Third-party software and assets

KnightCast's application code is MIT-licensed. Dependency licenses remain their own; `Cargo.lock` pins the Rust dependency graph.

`docs/DEPENDENCIES.json` records package names, versions, declared license expressions and upstream repositories from the locked Cargo metadata. It covers all resolved targets, not just packages executed on Linux. It is an inventory, not a substitute for the original license texts when distributing binaries.

The initial renderer uses eframe/egui, winit and wgpu, with glTF and image-loading dependencies. A release must carry all notices required by its resolved dependencies. Development source distribution through Cargo does not relicense those packages.

The optional capture helper imports a separately installed [OpenSeeFace](https://github.com/emilianavt/OpenSeeFace), OpenCV, NumPy and ONNX Runtime. OpenSeeFace is distributed under BSD-2-Clause; consult its `LICENSE` and `Licenses/` directory for its bundled components and models. These packages and models are not vendored or redistributed by KnightCast.

VMagicMirror is a behavioral reference only. No VMagicMirror application implementation or packaged artwork is included. Its public documentation and source license are linked in `docs/PROVENANCE.md`.

The user's Knight GLB, textures and cutout images are external local inputs. They are not included in the public repository and are not made MIT-licensed by this project's license.
