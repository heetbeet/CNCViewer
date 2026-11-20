# Inspect G-code with native CNC Viewer

Use the same native executable for the desktop viewer and agent inspection. The command `--help agents` prints this guide and all its Markdown references in one stream, with a file marker before each document. Use the full path to the executable you invoked to read this guide. With an installed or exported skill pointer, `executable.txt` beside `SKILL.md` gives that path. From the source checkout, use `output/desktop/CNC Viewer.exe` or `target/release/cnc-viewer.exe`.

Start with a numeric inspection. Omit the job file for a quick summary, or pass a JSON job with the checks relevant to the cut. Use `inspect - JOB.json` for G-code on standard input and set `filename` in the job if source selectors need a specific name. The command prints a JSON report. Exit 0 means the requested checks passed, 2 means failed or incomplete, and 1 means an input or artifact error. Check `validation`, `status`, `execution.scope`, `requirements` and `firstFailure`. A successful prefix covers only the executed blocks.

```powershell
$viewer = './output/desktop/CNC Viewer.exe'
& $viewer inspect part.nc job.json | Out-String
```

## Generate and verify a revision

1. From the drawing or user specification, write down the expected cut coordinates, depth, features to preserve, clearance, and tolerances. Establish the work zero and cutter diameter before writing the G-code. State any assumed setup values.
2. Generate a candidate and run the cheapest checks first. Use an explicit setup and check the reported limits, endpoint, execution scope, requirements, and warnings. Test representative parameter cases for macros.
3. Verify the intended geometry separately. A program that retracts and ends can pass feed, clearance, and endpoint checks without making a cut. Compare `movements.json` to the expected coordinates, or use `stock.target` for a required floor depth and stock sections to check areas that should remain intact.
4. Diagnose failures by source, line, executed step, and XYZ witness. Use a small trace around the finding to inspect variables, or a focused PNG to see the approach. After changing the program, rerun the same checks through the end and export fresh evidence for the revised source.

See [references/jobs.md](references/jobs.md) for selectors, camera views and artifacts, and [references/efficiency.md](references/efficiency.md) to pick the cheapest useful check. Call the executable directly from any language. For optional Python helpers, run `export-skill DIRECTORY` to extract `scripts/inspect.py` with `inspect(path, job)`, `inspect_text(text, job)`, `movements(path, setup=None)`, and `state(path, selector, setup=None)`. The [Rust API notes](references/api.md) cover in-process inspection from a Rust application.

The desktop opens files by positional path, Ctrl+O or drag and drop. Ctrl+Enter replots. Right-click the Editor header to load a machine setup JSON or add subprograms to the active tab. Expand Variables beside the code, then use the playback number and slider to inspect loop visits. Home resets the camera when the plot has focus. The cube selects top, front and side views.

Coordinates and dimensions in reports are millimetres. Line and step selectors are one-based. The core models MachPro Router syntax, nominal G54 and initial position zero, and explicit machine configuration where required. Undefined macro variables are NIL, displayed as `-`, with zero coercion only where the dialect requires arithmetic. Configure missing tool lengths, offsets, inputs, probe scenarios and custom no-op M-codes through `setup`; never invent a machine response.

Stock is a sampled vertical flat-mill heightfield. Overlap uses projected XY capsules. Check completeness, grid uncertainty and scope when interpreting results. Stock analysis requires a vertical cutter. The viewer reports missing machine inputs and stops at unsupported operations, including nonzero cutter compensation. The report and exported evidence describe the simulated cut for the supplied setup.
