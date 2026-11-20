# Inspection cost

| Need | Cheapest useful request | Cost |
| --- | --- | --- |
| Execution, feeds, rapid height, endpoint | Numeric checks, no artifacts | Fastest. Compiles and executes once, no GPU or variable history. |
| A particular loop visit | `until` line/step/sequence and visit | Executes a prefix; still compiles the source. |
| Why a check failed | Small trace window around its finding | Small output, but retains variable history for the executed prefix. |
| Bounds, tool use or move counts | Named `motion` summaries | Small report. Batch several selections in one job. |
| Actual coordinates | Selected movements in JSONL or Python `iter_movements()` | The program still executes normally. Output grows with the selected moves. |
| Distance to a boundary | `clearance` with a selection, XY polygon and Z band | Uses cutter segments and polygon edges. Returns two witnesses at most per query. |
| A camera view | One native PNG, then more angles in the same job | Starts a GPU device, not a browser. Reuses the device across views. |
| Overlap | Two tight step ranges | Dense all-pairs work can become quadratic; respect incomplete results. |
| Remaining material | Small stock box, coarse grid, short section | The stock box controls sampling work. A section range trims output. Halving cell size roughly quadruples the grid. |
| Parameter coverage | Numeric cases or matrix | Shared compilation, repeated execution. Add images only to selected cases. |

Keep full traces, fine whole-stock grids, unrestricted overlap and PNGs for investigations that need them. Default clearance budget is 5 million segment-edge tests per case. Default stock budgets are 250,000 cells and 5 million sweep/cell tests. Default overlap budgets are 1,000 pairs and 2 million comparisons. A cutter trail is limited to 50,000 cutting segments. Use the report's `timing` field to measure the cost of a job.
