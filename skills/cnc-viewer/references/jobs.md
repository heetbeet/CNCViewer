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

Set `"depthSupport": true` in a PNG view to show halos at depth crossings and mute lines beneath the cutter trail. For a vertical flat-end cutter, the trail shows the volume swept by the end disc on ramps and plunges. Use a stock section to measure the resulting surface.

Set `setup.initialPosition` and `setup.workOffsets` when the program relies on a known start or fixture zero. For example, `"setup":{"initialPosition":[100,40,25],"workOffsets":{"55":[100,40,20]}}` puts the tool tip 5 mm above the G55 origin. Report coordinates and check targets use machine tool-tip XYZ, so convert expected work coordinates using the supplied offset. Use `setup.variables` for initial macro values and `setup.tools` for numbered tool data. Follow the report's `requirements` for any other machine inputs.

`until` accepts `line`, `step`, `sequence`, optional `source` or `program`, and `visit`. For example `{"until":{"line":14,"visit":3}}` pauses after the third executed visit to line 14. Use it to inspect a loop without running the remainder. `libraries` is an array of subprogram file paths.

Checks include `maxFeed`, `maxPlunge`, `safeZ`, `rapidFloor`, `requireSpindle`, `allowProgramStops`, `expectedEnd`, `tolerance`, and `allowedBounds: {min:[x,y,z],max:[x,y,z]}`. Tool-tip bounds check sampled motion vertices. Supplying cutter dimensions adds a check of the projected swept cutter envelope. A motion finding includes the executed visit and the sampled XYZ witness.

`motion` reports counts, lengths, tools, and centerline bounds for named selections. The same `select` works on movement exports, clearance queries, and views. Ranges are inclusive. Copy the selector between them to inspect exactly the same motion.

- `{"operation":"finish"}` uses an operation declared in the job.
- `{"fromStep":12,"toStep":20}` selects an executed step range. Both endpoints must be reached.
- `{"source":"sub.nc","fromLine":7,"toLine":9,"visit":2}` selects the second visit to each of those source lines. Omit `visit` for all visits. Omit `source` for the first executed source. Only matching steps are selected, including when visits are interleaved with other lines.

```json
{
  "operations": [{"id":"finish","from":{"line":12},"to":{"line":30}}],
  "motion": [{"id":"finish","select":{"operation":"finish"}}],
  "artifacts": {"movements":{"select":{"operation":"finish"},"format":"jsonl"}},
  "output": "./finish-evidence"
}
```

Selection filters the result of execution. Use `until` when you want to stop the interpreter early. A movement export includes one record per move with stable IDs, source line, step, visit and sampled vertices. `movements: true` writes the full `movements.json` array. The object accepts `format` (`json` or `jsonl`) and `feedOnly` as well as `select`. Use the report's `artifacts.movements.file` and `records` to find the current export and confirm it has the expected moves. Read JSONL one line at a time.

`footprints` writes cutter capsules. `trace` writes JSONL execution states. A trace window can select `step` or a report `finding` ID and `radius`. Set `{ "end": true, "radius": 0 }` for the final executed state. Use a window for variable inspection.

### Measure cutter clearance

Give `clearance` a list of named queries. Use a simple XY polygon and an inclusive `z` band in machine coordinates. Supply the cutter diameter in the job or the tool table.

```json
{
  "cutter": {"diameter":8},
  "clearance": [{
    "id":"adjacent-part",
    "select":{"fromStep":12,"toStep":70},
    "polygon":[[100,20],[180,20],[180,40],[100,40]],
    "z":[-12,0],
    "minimumClearance":0.5
  }]
}
```

The query measures XY distance from the feed centerline to the filled polygon, minus cutter radius, after clipping each segment to the tool-tip Z band. It returns the minimum and its witness, including `point` (tool tip), `regionPoint` (XY), and source visit. A negative result means overlap. The value reaches minus the radius when the tool center enters the polygon. It measures the sampled toolpath with a vertical flat cutter. Use separate checks for rapids, cutter shaft, or holder.

Omit `minimumClearance` for measurement alone. A threshold adds a finding for the earliest violating segment and keeps the global minimum witness separately. Allow 0.1 mm of overlap with `minimumClearance: -0.1`. Check `complete` and `status`. An empty result has no contributing feed segment in its Z band. A threshold check with no selected feed moves is incomplete. Missing diameter and exhausted query budgets also produce incomplete results. `limits.clearanceTests` sets the shared polygon-edge comparison budget per case.

Views accept `name`, `angle` (`iso`, `top`, `bottom`, `front`, `back`, `left`, `right`), `width`, `height`, `step`, `showRapid`, `showGrid`, `showPoints`, and `cutter`. `camera.bounds` fits a `{min,max}` box. Use `direction`, `center`, `zoom` or `zoomFactor` for more camera control. `select`, `fromStep`, `toStep` and `region` restrict display geometry together. Each PNG has a JSON camera sidecar, source and executable hashes. `finding: "F001"` selects the executed step of a finding and marks its XYZ witness when present. Use `labels: true` for a few movement landmarks, or `labels: [{id,text,point:[x,y,z]}, {step:12}, {line:14,visit:2}]` for selected coordinate annotations. Exact coordinates are saved beside the PNG. `operations: [{id,from:{line:7},to:{line:11},tool:1}]` checks declared operation order and tool selection.

`stock` accepts `bounds`, `cellSize`, `maxCells`, `maxTests`, `sections:[{id,axis,at,range}]`, and `target:{z,tolerance,bounds:{min:[x,y],max:[x,y]}}`. A section at fixed X with `"range":[12,24]` reports only Y sample centers from 12 to 24 mm. The report provides `sampledAt`, `sampleSpacing`, and height samples. Use an explicit stock box to reduce sampling work. Request `artifacts.stock` for the heightfield or `artifacts.sections` for JSON and SVG profiles. Compare short sections to your design profile at the reported sample coordinates. For a height comparison, measured minus expected is negative for overcut and positive for material left.

`overlap` accepts `between:[{fromStep,toStep},{fromStep,toStep}]`, `clearance`, `maxPairs` and `maxComparisons`.

Set the target rectangle from the intended feature. Check `stock.complete`, `stock.target.samples`, and the status before relying on it. A target result covers the sampled cells inside that rectangle. Choose smaller regions or inspect sections and movement coordinates when you need to assess edges or nearby material at a finer scale.

`cases` is an array of `{id,setup,...}` overrides. Alternatively use `matrix`, mapping dotted paths such as `setup.variables.100` to candidate arrays. Compilation is shared, execution is per case. A job with multiple cases and an explicit `output` saves `matrix.json` alongside its case directories. `assignmentOverrides` selects `{source,line,variable,value,visit}`. Job limits include `inputBytes`, `timeMs`, `findings` and `traceBytes`. Execution budgets belong in `setup.maxExecutions`, `maxSegments` and `maxCallDepth`.

The executable preserves numeric results with `validation: incomplete` and `artifactError` if a requested artifact cannot be produced. Do not use stale evidence from a previous job. Unknown checks are rejected.
