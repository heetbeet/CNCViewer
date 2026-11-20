# Inspection cost

| Need | Cheapest useful request | Cost |
| --- | --- | --- |
| Execution, feeds, rapid height, endpoint | Numeric checks, no artifacts | Fastest. Compiles and executes once, no GPU or variable history. |
| A particular loop visit | `until` line/step/sequence and visit | Executes a prefix; still compiles the source. |
| Why a check failed | Small trace window around its finding | Small output, but retains variable history for the executed prefix. |
| Actual coordinates | Movements artifact or Python `movements()` | Output grows with the number of moves. |
| A camera view | One native PNG, then more angles in the same job | Starts a GPU device, not a browser. Reuses the device across views. |
| Overlap | Two tight step ranges | Dense all-pairs work can become quadratic; respect incomplete results. |
| Remaining material | Small stock box and coarse grid first | Halving cell size roughly quadruples cells and work. |
| Parameter coverage | Numeric cases or matrix | Shared compilation, repeated execution. Add images only to selected cases. |

Avoid full traces, fine whole-stock sampling, unrestricted overlap and PNGs on every matrix case for rapid checks. Default stock budgets are 250,000 cells and 5 million sweep/cell tests. Default overlap budgets are 1,000 pairs and 2 million comparisons. A cutter trail is limited to 50,000 cutting segments. Use the report's `timing` field to measure the cost of a job.
