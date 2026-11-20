# Rust API

Use the `cnc-core` crate directly when processing multiple programs in one process:

```rust
let model = cnc_core::engine::inspect(
    text, "part.nc", &Default::default(),
    &cnc_core::engine::Options::default(),
)?;
// model.moves, model.segments, model.executions, model.diagnostics
```

For parameter sweeps, call `compiler::compile` once, then `engine::inspect_compiled` for each `Options` value. `analysis::validate` produces a compact report; `analysis::stock` and `analysis::sweeps` perform requested geometry analysis. `Options.capture_variables` retains variable snapshots and should stay false for numeric checks. `Options.cancel` accepts a shared atomic flag.

For agents and other process-based callers, use `inspect FILE|- [JOB.json]` and parse its stdout as JSON. The native executable bundles the job reference under `--help agents`.
