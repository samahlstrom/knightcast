# Contributing

KnightCast is a Linux-first native avatar application. Keep the runtime free of browser engines and cloud requirements. Windows support is a portability target, not a currently verified claim.

The project uses independently authored behavioral specifications. Read `docs/PROVENANCE.md`, `docs/BEHAVIORAL_SPEC.md`, and `docs/TRACKING_PROTOCOL.md` before changing tracking or rendering. Do not contribute proprietary application code, decompiled code, or extracted assets. Reference-app behavior can be described from public user documentation. Credit third-party code and preserve its license when legitimately incorporating it; do not call copied code clean-room work.

Contributions should include focused tests for packet parsing, freshness, pose transforms, or other behavior they change. Camera tests must be explicitly enabled. Do not record camera frames or typed text by default. Never alter compositor configuration, OBS scenes, or unrelated application windows.

Application source is MIT-licensed. Artwork, tracker models, and external dependencies retain their own licenses. Personal Knight artwork is loaded from a local path and is not distributed here.
