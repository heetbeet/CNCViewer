# Rust API

Use the `cnc-core` crate directly when processing multiple programs in one process:

```rust
let model = cnc_core::engine::inspect(
    text, "part.nc", &Default::default(),
    &cnc_core::engine::Options::default(),
)?;
// model.moves, model.segments, model.executions, model.diagnostics
```

For parameter sweeps, call `compiler::compile` once, then `engine::inspect_compiled` for each `Options` value. `analysis::validate` returns the report, including requested `motion` and `clearance` queries. Use `motion::Selection::resolve(&model, &report["operations"], &selector)` and iterate `moves(&model)` or `segments(&model)` for your own checks. `analysis::stock` samples material and `analysis::sweeps` derives cutter geometry. Enable `Options.capture_variables` only when you need variable snapshots. `Options.cancel` accepts a shared atomic flag.

For agents and other process-based callers, use `inspect FILE|- [JOB.json]` and parse its stdout as JSON. The native executable bundles the job reference under `--help agents`.
