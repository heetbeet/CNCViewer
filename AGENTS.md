# CNC Viewer native Rust

This repository contains the native Rust application. Preserve the established layout, controls, syntax colours, MachPro Router conventions and inspection behavior. Use the saved conformance cases and the bundled demo when changing interpreter behavior. The README records the supported workflows and verification scope.

- `crates/cnc-core/src/engine.rs` and `engine/`: execution, macro variables, machine geometry and probes.
- `crates/cnc-core/src/analysis.rs`: numeric checks, cutter overlaps and sampled stock sections.
- `crates/cnc-app/src/app/view.rs`: desktop layout and syntax highlighting.
- `crates/cnc-app/src/render.rs`, `trail.rs`, `plot.wgsl`: cached native GPU paths, points and capped brush trails.
- `crates/cnc-app/src/cli.rs`: native JSON inspection jobs and PNG evidence.
- `skills/cnc-viewer/AGENTS.md`: agent usage. Keep it consistent with actual implemented behavior.

Use `cargo test --locked --workspace --release`, `cargo clippy --locked --workspace -- -D warnings` and `cargo fmt --all --check`. GPU tests need a Windows graphics adapter. `tools/build.ps1` packages the desktop EXE. Base performance claims on measurements. Keep missing machine inputs explicit and use supplied probe and custom M-code behavior.
