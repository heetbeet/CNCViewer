# Native inspection jobs

Pass a JSON file as the second argument to `inspect FILE JOB.json`. Library and output paths resolve relative to that JSON file. Use `inspect - JOB.json` for input on stdin. Without a job the command prints a compact numerical report and creates no artifacts.

```json
{
  "setup": {"tools": {"1": {"length": 20, "diameter": 8}}},
  "cutter": {"diameter": 8},
  "checks": {"maxFeed": 600, "safeZ": 5, "expectedEnd": [0, 0, 5]},
  "output": "./evidence",
  "artifacts": {"movements": true, "trace": {"step": 30, "radius": 3}},
  "views": [{"name": "top", "angle": "top", "cutter": true}]
}
```

Use `"opaqueTrail": true` in a PNG view with a cutter trail to have it occlude geometry.

Set `setup.initialPosition` and `setup.workOffsets` when the program relies on a known start or fixture zero. For example, `"setup":{"initialPosition":[100,40,25],"workOffsets":{"55":[100,40,20]}}` puts the tool tip 5 mm above the G55 origin. Report coordinates and check targets use machine tool-tip XYZ, so convert expected work coordinates using the supplied offset. Use `setup.variables` for initial macro values and `setup.tools` for numbered tool data. Follow the report's `requirements` for any other machine inputs.

`until` accepts `line`, `step`, `sequence`, optional `source` or `program`, and `visit`. For example `{"until":{"line":14,"visit":3}}` pauses after the third executed visit to line 14. Use it to inspect a loop without running the remainder. `libraries` is an array of subprogram file paths.

Checks include `maxFeed`, `maxPlunge`, `safeZ`, `rapidFloor`, `requireSpindle`, `allowProgramStops`, `expectedEnd`, `tolerance`, and `allowedBounds: {min:[x,y,z],max:[x,y,z]}`. Tool-tip bounds check sampled motion vertices. Supplying cutter dimensions adds a check of the projected swept cutter envelope. A motion finding includes the executed visit and the sampled XYZ witness.

`artifacts.movements` writes `movements.json` with sampled motion vertices and source locations; `footprints` writes cutter capsules; `trace` writes JSONL execution states. Trace `true` exports everything and is expensive. A trace window can select `step` or a report `finding` ID and `radius`. Set `{ "end": true, "radius": 0 }` to export the final executed state. These exports require an output directory and are not printed in the compact report.

Views accept `name`, `angle` (`iso`, `top`, `bottom`, `front`, `back`, `left`, `right`), `width`, `height`, `step`, `showRapid`, `showGrid`, `showPoints`, and `cutter`. `camera.bounds` fits a `{min,max}` box. Use `direction`, `center`, `zoom` or `zoomFactor` for more camera control. `fromStep`, `toStep` and `region` restrict display geometry. Each PNG has a JSON camera sidecar, source and executable hashes. `finding: "F001"` selects the executed step of a finding and marks its XYZ witness when present. Use `labels: true` for a few movement landmarks, or `labels: [{id,text,point:[x,y,z]}, {step:12}, {line:14,visit:2}]` for selected coordinate annotations. Exact coordinates are saved beside the PNG. `operations: [{id,from:{line:7},to:{line:11},tool:1}]` checks declared operation order and tool selection.

`stock` accepts `bounds`, `cellSize`, `maxCells`, `maxTests`, `sections:[{id,axis,at}]`, and `target:{z,tolerance,bounds:{min:[x,y],max:[x,y]}}`. Request `artifacts.stock` or `sections` for evidence. Sections produce JSON coordinates and an SVG profile. Smaller cells increase cost quadratically. `overlap` accepts `between:[{fromStep,toStep},{fromStep,toStep}]`, `clearance`, `maxPairs` and `maxComparisons`.

Set the target rectangle from the intended feature. Check `stock.complete`, `stock.target.samples`, and the status before relying on it. A target result covers the sampled cells inside that rectangle. Choose smaller regions or inspect sections and movement coordinates when you need to assess edges or nearby material at a finer scale.

`cases` is an array of `{id,setup,...}` overrides. Alternatively use `matrix`, mapping dotted paths such as `setup.variables.100` to candidate arrays. Compilation is shared, execution is per case. A job with multiple cases and an explicit `output` saves `matrix.json` alongside its case directories. `assignmentOverrides` selects `{source,line,variable,value,visit}`. Job limits include `inputBytes`, `timeMs`, `findings` and `traceBytes`. Execution budgets belong in `setup.maxExecutions`, `maxSegments` and `maxCallDepth`.

The executable preserves numeric results with `validation: incomplete` and `artifactError` if a requested artifact cannot be produced. Do not use stale evidence from a previous job. Unknown checks are rejected.
