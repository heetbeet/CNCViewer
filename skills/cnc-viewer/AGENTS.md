# Inspect G-code with native CNC Viewer

Use the same native executable for the desktop viewer and agent inspection. Run it with `--help agents` to read this guide and its references. An installed or exported skill can point to the executable in `executable.txt`. The packaged checkout build is at `output/desktop/CNC Viewer.exe`.

Start with `inspect FILE` for a compact numeric report. Pass a JSON job to run checks or request evidence. Use `inspect - JOB.json` for source on stdin and set `filename` for source selectors. The command prints JSON. Exit 0 means the requested checks passed, 2 means failed or incomplete, and 1 means the job could not produce an inspection report. Check `validation`, `status`, `execution.scope`, `requirements`, `firstFailure`, and any `artifactError`. A successful prefix covers only the executed blocks.

```text
CNC-Viewer-windows-x64.exe inspect part.nc job.json
```

## Generate and verify a revision

1. From the drawing or user specification, write down the expected cut coordinates, depth, features to preserve, clearance, and tolerances. Establish the work zero and cutter diameter before writing the G-code. State any assumed setup values.
2. Generate a candidate and run the cheapest checks first. Use an explicit setup and check the reported limits, endpoint, execution scope, requirements, and warnings. Test representative parameter cases for macros.
3. Verify the intended geometry. Use a `motion` summary for an operation or execution range. Export selected moves as JSONL when you need vertices. Use `clearance` to measure the swept cutter against boundaries you supply. Use a tight stock box and a short section for remaining thickness or a slope. Check that the selection contains the expected cutting moves.
4. Diagnose failures by source, line, executed step, and XYZ witness. Use a small trace around the finding to inspect variables, or a focused PNG to see the approach. After changing the program, rerun the same checks through the end and export fresh evidence for the revised source.

See [job examples](references/jobs.md) for selection, clearance, and views, and [cost guidance](references/efficiency.md) for which query to use. Call the executable directly from any language. Run `export-skill DIRECTORY` for optional Python helpers `inspect()`, `inspect_text()`, `iter_movements()`, `movements()`, and `state()`. The [Rust API notes](references/api.md) cover in-process inspection.

The desktop opens files by positional path, Ctrl+O or drag and drop. Ctrl+Enter replots. Right-click the Editor header to load a machine setup JSON or add subprograms to the active tab. Expand Variables beside the code, then use the playback number and slider to inspect loop visits. Home resets the camera when the plot has focus. The cube selects top, front and side views.

Coordinates and dimensions in reports are millimetres. Line and step selectors are one-based. The core models MachPro Router syntax, nominal G54 and initial position zero, and explicit machine configuration where required. Undefined macro variables are NIL, displayed as `-`, with zero coercion only where the dialect requires arithmetic. Configure missing tool lengths, offsets, inputs, probe scenarios and custom no-op M-codes through `setup`; never invent a machine response.

Stock is a sampled vertical flat-mill heightfield. Clearance measures the tool-tip sweep and supplied diameter within the Z band. Use the report's completeness and scope when interpreting either. Supply the expected profile, areas to preserve, and tolerances from the design. Treat a report as evidence for the supplied setup and executed scope.
