mod clearance;

use crate::{
    engine::{Model, Segment, State, Target},
    error::{NcError, Result},
    motion, overlap,
    stock::{sample, Sweep},
};
use serde_json::{json, Value};
pub fn validate(model: &Model, job: &Value) -> Result<Value> {
    let checks = &job["checks"];
    if !checks.is_null() && !checks.is_object() {
        return Err(NcError::new("checks must be an object"));
    }
    let supported = [
        "maxFeed",
        "maxPlunge",
        "rapidFloor",
        "safeZ",
        "tolerance",
        "allowedBounds",
        "expectedEnd",
        "requireSpindle",
        "allowProgramStops",
    ];
    if let Some(checks) = checks.as_object() {
        for key in checks.keys() {
            if !supported.contains(&key.as_str()) {
                return Err(NcError::new(format!("Unsupported check {key}")));
            }
        }
    }
    for key in ["maxFeed", "maxPlunge", "rapidFloor", "safeZ", "tolerance"] {
        if !checks[key].is_null()
            && checks[key].as_f64().is_none_or(|v| {
                !v.is_finite() || (!["safeZ", "rapidFloor"].contains(&key) && v < 0.)
            })
        {
            return Err(NcError::new(format!("Invalid {key} limit")));
        }
    }
    for key in ["requireSpindle", "allowProgramStops"] {
        if !checks[key].is_null() && !checks[key].is_boolean() {
            return Err(NcError::new(format!("{key} must be boolean")));
        }
    }
    let bounds = if checks["allowedBounds"].is_null() {
        None
    } else {
        let min = point(&checks["allowedBounds"]["min"])?;
        let max = point(&checks["allowedBounds"]["max"])?;
        if min.iter().zip(max).any(|(a, b)| *a > b) {
            return Err(NcError::new("Bounds must be ordered"));
        }
        Some((min, max))
    };
    let mut findings = Vec::new();
    for d in &model.diagnostics {
        findings.push(json!({"check":"execution","severity":"error","source":d.source,"line":d.line,"step":d.step,"code":d.error.code,"message":d.error.message}));
    }
    for d in &model.warnings {
        findings.push(json!({"check":"execution","severity":"warning","source":d.source,"line":d.line,"code":d.error.code,"message":d.error.message}));
    }
    let check_moves = bounds.is_some()
        || [
            "maxFeed",
            "maxPlunge",
            "rapidFloor",
            "safeZ",
            "requireSpindle",
        ]
        .iter()
        .any(|key| !checks[*key].is_null());
    let moves = if check_moves {
        model.moves.as_slice()
    } else {
        &[]
    };
    for (index, movement) in moves.iter().enumerate() {
        let state = model.executions.get(movement.step.saturating_sub(1));
        let mut fail = |check: &str, actual: Value, expected: Value, point: [f64; 3]| {
            findings.push(json!({"check":check,"severity":"error","source":state.map(|s|&s.source),"line":movement.line,"step":movement.step,"move":index+1,"tool":movement.tool,"point":point,"actual":actual,"expected":expected}));
        };
        let low = if movement.start[2] <= movement.end[2] {
            movement.start
        } else {
            movement.end
        };
        let rate = if movement.seconds > 0. {
            movement.length / movement.seconds * 60.
        } else {
            0.
        };
        if let Some(limit) = checks["maxFeed"].as_f64() {
            if !movement.rapid && rate > limit + limit.abs().max(1.) * 1e-12 {
                fail("maxFeed", json!(rate), json!(limit), movement.end)
            }
        }
        if let Some(limit) = checks["safeZ"].as_f64() {
            if movement.rapid
                && (movement.end[0] - movement.start[0]).hypot(movement.end[1] - movement.start[1])
                    > 1e-9
                && low[2] < limit
            {
                fail("safeZ", json!(low[2]), json!(limit), low)
            }
        }
        if let Some(limit) = checks["rapidFloor"].as_f64() {
            if movement.rapid && low[2] < limit {
                fail("rapidFloor", json!(low[2]), json!(limit), low)
            }
        }
        if let Some(limit) = checks["maxPlunge"].as_f64() {
            if !movement.rapid {
                let peak = model.segments[movement.segment_start..movement.segment_end]
                    .iter()
                    .filter_map(|s| {
                        let length = crate::geometry::distance(&s.start, &s.end);
                        (length > 0. && s.end[2] < s.start[2])
                            .then(|| ((s.start[2] - s.end[2]) / length * rate, s.end))
                    })
                    .max_by(|a, b| a.0.total_cmp(&b.0));
                if let Some((peak, point)) = peak {
                    if peak > limit + limit.abs().max(1.) * 1e-12 {
                        fail("maxPlunge", json!(peak), json!(limit), point)
                    }
                }
            }
        }
        if checks["requireSpindle"] == true
            && !movement.rapid
            && movement.length > 0.
            && state.is_none_or(|s| s.spindle[0] == 0. || s.spindle[1] <= 0.)
        {
            fail("requireSpindle", Value::Null, json!(true), movement.end)
        }
        if let Some((min, max)) = bounds {
            let outside = |p: &[f64; 3]| {
                p.iter()
                    .enumerate()
                    .any(|(i, v)| *v < min[i] || *v > max[i])
            };
            let witness = std::iter::once(movement.start)
                .chain(
                    model.segments[movement.segment_start..movement.segment_end]
                        .iter()
                        .map(|s| s.end),
                )
                .chain(std::iter::once(movement.end))
                .find(outside);
            if let Some(point) = witness {
                fail(
                    "allowedBounds",
                    json!(point),
                    checks["allowedBounds"].clone(),
                    point,
                )
            }
        }
    }
    if !checks["expectedEnd"].is_null() {
        let expected = point(&checks["expectedEnd"])?;
        let actual = model.executions.last().map(|s| s.position);
        if actual.is_none_or(|a| {
            crate::geometry::distance(&a, &expected) > checks["tolerance"].as_f64().unwrap_or(0.001)
        }) {
            findings.push(json!({"check":"expectedEnd","severity":"error","expected":expected,"actual":actual}));
        }
    }
    for event in &model.events {
        if event["type"] == "program-stop" {
            let mut event = event.clone();
            event["check"] = json!("programStop");
            event["severity"] = json!(if checks["allowProgramStops"] == false {
                "error"
            } else {
                "info"
            });
            findings.push(event)
        }
    }
    if let Some((min, max)) = bounds.filter(|_| !job["cutter"].is_null()) {
        for sweep in sweeps(model, job)? {
            let low: [f64; 3] = std::array::from_fn(|i| {
                sweep.start[i].min(sweep.end[i]) - if i < 2 { sweep.diameter / 2. } else { 0. }
            });
            let high: [f64; 3] = std::array::from_fn(|i| {
                sweep.start[i].max(sweep.end[i]) + if i < 2 { sweep.diameter / 2. } else { 0. }
            });
            if (0..3).any(|i| low[i] < min[i] || high[i] > max[i]) {
                let segment = &model.segments[sweep.witness];
                let axis = (0..3)
                    .find(|i| low[*i] < min[*i] || high[*i] > max[*i])
                    .unwrap();
                let lower = low[axis] < min[axis];
                let mut point = if (sweep.start[axis] <= sweep.end[axis]) == lower {
                    sweep.start
                } else {
                    sweep.end
                };
                point[axis] = if lower { low[axis] } else { high[axis] };
                findings.push(json!({"check":"cutterBounds","severity":"error","step":segment.step,"line":segment.line,"move":segment.movement+1,"tool":segment.tool,"point":point,"actual":{"min":low,"max":high},"expected":checks["allowedBounds"]}));
                break;
            }
        }
    }
    let mut operations = Vec::<Value>::new();
    if let Some(requests) = job["operations"].as_array() {
        let mut ids = std::collections::BTreeSet::new();
        for op in requests {
            let id = op["id"]
                .as_str()
                .filter(|s| !s.is_empty() && ids.insert((*s).to_owned()))
                .ok_or_else(|| NcError::new("Operations require unique nonempty IDs"))?;
            let result = (|| -> Result<Value> {
                let first = select_state(model, &op["from"])?;
                let last = select_state(model, &op["to"])?;
                if first.step > last.step {
                    return Err(NcError::new("Operation endpoints are reversed"));
                }
                let selected: Vec<_> = model
                    .moves
                    .iter()
                    .filter(|m| m.step >= first.step && m.step <= last.step)
                    .collect();
                let tools: std::collections::BTreeSet<_> =
                    selected.iter().map(|m| m.tool).collect();
                let mut failed = false;
                if !op["tool"].is_null() {
                    let tool = op["tool"]
                        .as_u64()
                        .filter(|tool| u32::try_from(*tool).is_ok())
                        .ok_or_else(|| {
                            NcError::new("Operation tool must be a nonnegative integer")
                        })?;
                    if tools.iter().any(|t| *t as u64 != tool) {
                        failed = true;
                        findings.push(json!({"check":"operationTool","operation":id,"step":first.step,"severity":"error","expected":tool,"actual":tools}));
                    }
                }
                if operations
                    .last()
                    .and_then(|o| o["toStep"].as_u64())
                    .is_some_and(|end| end >= first.step as u64)
                {
                    failed = true;
                    findings.push(json!({"check":"operationOrder","operation":id,"step":first.step,"severity":"error"}));
                }
                Ok(
                    json!({"id":id,"fromStep":first.step,"toStep":last.step,"moves":selected.len(),"tools":tools,"end":last.position,"status":if failed{"failed"}else{"passed"}}),
                )
            })();
            operations.push(match result{Ok(item)=>item,Err(error)=>{findings.push(json!({"check":"operationNotReached","operation":id,"severity":"error","message":error.message}));json!({"id":id,"status":"not-run","reason":error.message})}});
        }
    } else if !job["operations"].is_null() {
        return Err(NcError::new("operations must be an array"));
    }
    if !job["until"].is_null() && model.status == "complete" {
        findings.push(json!({"check":"targetNotReached","severity":"error","expected":job["until"],"message":"Program ended before the requested checkpoint"}));
    }
    if let Some(overrides) = job["assignmentOverrides"].as_array() {
        for (index, rule) in overrides.iter().enumerate() {
            if !model.events.iter().any(|e| {
                e["type"] == "assignment-override"
                    && e["details"]["override"].as_u64() == Some(index as u64)
            }) {
                findings.push(json!({"check":"overrideNotApplied","severity":"error","source":rule["source"],"line":rule["line"],"message":"Requested assignment override was not executed"}));
            }
        }
    }
    let mut warned = std::collections::BTreeSet::new();
    for event in model
        .events
        .iter()
        .filter(|e| e["type"] == "initial-variable-overwritten")
    {
        if let Some(variable) = event["details"]["variable"].as_u64() {
            if warned.insert(variable) {
                findings.push(json!({"check":"initialVariableOverwritten","severity":"warning","source":event["source"],"line":event["line"],"step":event["step"],"variable":variable,"expected":event["details"]["initial"],"actual":event["details"]["value"]}));
            }
        }
    }
    for (i, finding) in findings.iter_mut().enumerate() {
        finding["id"] = json!(format!("F{:03}", i + 1));
    }
    let first_failure = findings.iter().find(|f| f["severity"] == "error").cloned();
    let complete = model.status == "complete";
    let intentional = model.status == "paused";
    let passed = (complete || intentional) && first_failure.is_none();
    let mut report = json!({"schema":"cnc-inspection/2","units":"mm","coordinateFrame":"machine tool-tip XYZ","profile":{"interpreter":"machpro-2026.7.14.2"},"status":model.status,"validation":if !(complete||intentional){"incomplete"}else if passed{"passed"}else{"failed"},"execution":{"complete":complete,"intentionalPause":intentional,"scope":if complete{"full"}else{"prefix"}},"summary":{"steps":model.executions.len(),"moves":model.moves.len(),"segments":model.segments.len(),"bounds":model.bounds.map(|(min,max)|json!({"min":min,"max":max})),"end":model.executions.last().map(|s|s.position),"feedLength":model.moves.iter().filter(|m|!m.rapid).map(|m|m.length).sum::<f64>(),"rapidLength":model.moves.iter().filter(|m|m.rapid).map(|m|m.length).sum::<f64>(),"seconds":model.executions.last().map_or(0.,|s|s.elapsed/1000.)},"firstFailure":first_failure,"requirements":model.requirements,"findingCount":findings.len(),"findings":findings,"checks":{"passed":passed}});
    report["operations"] = json!(operations);
    if !job["motion"].is_null() {
        let queries = job["motion"]
            .as_array()
            .ok_or_else(|| NcError::new("motion must be an array of named selections"))?;
        let mut ids = std::collections::BTreeSet::new();
        let summaries = queries
            .iter()
            .map(|query| {
                let id = query["id"]
                    .as_str()
                    .filter(|id| !id.is_empty() && ids.insert(*id))
                    .ok_or_else(|| NcError::new("Motion queries require unique nonempty IDs"))?;
                if query
                    .as_object()
                    .is_none_or(|fields| fields.keys().any(|key| key != "id" && key != "select"))
                {
                    return Err(NcError::new("Motion queries accept id and select"));
                }
                let selection =
                    motion::Selection::resolve(model, &report["operations"], &query["select"])?;
                let mut summary = selection.summary(model);
                summary["id"] = json!(id);
                Ok(summary)
            })
            .collect::<Result<Vec<_>>>()?;
        report["motion"] = json!(summaries);
    }
    clearance::measure(model, job, &mut report)?;
    if !job["overlap"].is_null() {
        let config = &job["overlap"];
        if !config.is_object() {
            return Err(NcError::new("overlap must be an object"));
        }
        for key in ["maxPairs", "maxComparisons"] {
            if !config[key].is_null()
                && config[key]
                    .as_u64()
                    .is_none_or(|n| n == 0 || n > 100_000_000)
            {
                return Err(NcError::new(format!("Invalid overlap {key} budget")));
            }
        }
        if !config["clearance"].is_null()
            && config["clearance"]
                .as_f64()
                .is_none_or(|n| !n.is_finite() || n < 0.)
        {
            return Err(NcError::new(
                "Overlap clearance must be nonnegative and finite",
            ));
        }
        if !config["between"].is_null() {
            let groups = config["between"]
                .as_array()
                .filter(|v| v.len() == 2)
                .ok_or_else(|| NcError::new("Overlap between requires two step ranges"))?;
            for group in groups {
                let from = group["fromStep"].as_u64().unwrap_or(1);
                let to = group["toStep"].as_u64().unwrap_or(u64::MAX);
                if !group.is_object()
                    || ["fromStep", "toStep"]
                        .iter()
                        .any(|key| !group[*key].is_null() && group[*key].as_u64().is_none())
                    || from == 0
                    || to < from
                {
                    return Err(NcError::new("Invalid overlap step range"));
                }
            }
        }

        let sweeps = sweeps(model, job)?;
        let groups = &job["overlap"]["between"];
        let selected: Vec<_> = sweeps
            .iter()
            .map(|s| {
                let segment = &model.segments[s.witness];
                let mut mask = 0;
                for g in 0..2 {
                    let range = &groups[g];
                    if segment.step >= range["fromStep"].as_u64().unwrap_or(1) as usize
                        && segment.step <= range["toStep"].as_u64().unwrap_or(u64::MAX) as usize
                    {
                        mask |= 1 << g
                    }
                }
                overlap::Sweep {
                    start: s.start,
                    end: s.end,
                    diameter: s.diameter,
                    id: s.witness + 1,
                    movement: segment.movement,
                    step: segment.step,
                    groups: mask,
                }
            })
            .collect();
        let result = overlap::compare(
            &selected,
            job["overlap"]["clearance"].as_f64().unwrap_or(0.),
            job["overlap"]["maxPairs"].as_u64().unwrap_or(1000) as usize,
            job["overlap"]["maxComparisons"].as_u64().unwrap_or(2000000) as usize,
            groups.is_array(),
        );
        report["overlap"] = json!({"model":"projected-XY-capsules","complete":result.complete,"comparisons":result.comparisons,"pairs":result.pairs});
        if !result.complete {
            report["validation"] = json!("incomplete");
            append_failure(
                &mut report,
                json!({"check":"overlapLimit","severity":"error","message":"Overlap comparison budget reached"}),
            );
        }
    }
    add_finding_locations(model, &mut report);
    Ok(report)
}
fn add_finding_locations(model: &Model, report: &mut Value) {
    let findings = report["findings"]
        .as_array_mut()
        .expect("inspection findings");
    let Some(last_step) = findings
        .iter()
        .filter_map(|finding| finding["step"].as_u64())
        .max()
        .and_then(|step| usize::try_from(step).ok())
    else {
        return;
    };
    let visits: Vec<_> = motion::visits(model).take(last_step).collect();
    for finding in &mut *findings {
        if let Some(index) = finding["step"]
            .as_u64()
            .and_then(|step| step.checked_sub(1))
            .and_then(|index| usize::try_from(index).ok())
        {
            if let (Some(state), Some(visit)) = (model.executions.get(index), visits.get(index)) {
                if finding["source"].is_null() {
                    finding["source"] = json!(state.source);
                }
                if finding["line"].is_null() {
                    finding["line"] = json!(state.line);
                }
                finding["visit"] = json!(visit);
                if finding["point"].is_null() {
                    finding["point"] = json!(state.position);
                }
            }
        }
    }
    report["firstFailure"] = findings
        .iter()
        .find(|finding| finding["severity"] == "error")
        .cloned()
        .unwrap_or(Value::Null);
}
pub fn append_failure(report: &mut Value, mut finding: Value) {
    let count = report["findings"].as_array().map_or(0, Vec::len) + 1;
    finding["id"] = json!(format!("F{count:03}"));
    if report["firstFailure"].is_null() {
        report["firstFailure"] = finding.clone();
    }
    report["findings"]
        .as_array_mut()
        .expect("inspection findings")
        .push(finding);
    report["findingCount"] = json!(count);
    report["checks"]["passed"] = json!(false);
}
pub fn point(value: &Value) -> Result<[f64; 3]> {
    let values = value
        .as_array()
        .filter(|v| v.len() == 3)
        .ok_or_else(|| NcError::new("Expected three finite coordinates"))?;
    let mut p = [0.; 3];
    for i in 0..3 {
        p[i] = values[i]
            .as_f64()
            .filter(|v| v.is_finite())
            .ok_or_else(|| NcError::new("Expected finite coordinates"))?
    }
    Ok(p)
}
pub fn sweeps(model: &Model, job: &Value) -> Result<Vec<Sweep>> {
    let cutter = &job["cutter"];
    if !cutter.is_object() {
        return Err(NcError::new(
            "Cutter dimensions are required for this check",
        ));
    }
    model
        .segments
        .iter()
        .enumerate()
        .filter(|(_, s)| !s.rapid)
        .map(|(i, s)| sweep_for(s, i, cutter))
        .collect()
}

fn sweep_for(s: &Segment, i: usize, cutter: &Value) -> Result<Sweep> {
    if s.rotation
        .iter()
        .chain(&s.start_rotation)
        .any(|v| v.abs() > 1e-9)
    {
        return Err(NcError::new("Cutter sweeps require a vertical tool"));
    }
    let tool = &cutter["tools"][s.tool.to_string()]["diameter"];
    let default = &cutter["diameter"];
    let diameter = if !tool.is_null() {
        tool.as_f64()
    } else if !default.is_null() {
        default.as_f64()
    } else {
        s.tool_diameter
    }
    .filter(|d| d.is_finite() && *d > 0. && *d <= 10000.)
    .ok_or_else(|| NcError::new(format!("Supply a positive diameter for tool {}", s.tool)))?;
    Ok(Sweep {
        start: s.start,
        end: s.end,
        diameter,
        witness: i,
    })
}
pub fn stock(model: &Model, job: &Value) -> Result<Value> {
    stock_report(model, job, true)
}
pub fn stock_report(model: &Model, job: &Value, include_heights: bool) -> Result<Value> {
    let config = &job["stock"];
    let min = point(&config["bounds"]["min"])?;
    let max = point(&config["bounds"]["max"])?;
    let cell = if config["cellSize"].is_null() {
        1.
    } else {
        config["cellSize"]
            .as_f64()
            .ok_or_else(|| NcError::new("cellSize must be numeric"))?
    };
    if !cell.is_finite() || cell <= 0. || min.iter().zip(max).any(|(a, b)| *a >= b) {
        return Err(NcError::new(
            "Stock bounds and cell size must be positive and finite",
        ));
    }
    let nx = ((max[0] - min[0]) / cell).ceil();
    let ny = ((max[1] - min[1]) / cell).ceil();
    let budget = |key, default| -> Result<u64> {
        if config[key].is_null() {
            return Ok(default);
        }
        config[key]
            .as_u64()
            .filter(|n| *n > 0)
            .ok_or_else(|| NcError::new(format!("{key} must be a positive integer")))
    };
    let max_cells = budget("maxCells", 250000)?;
    if !nx.is_finite() || !ny.is_finite() || nx * ny > max_cells.min(10000000) as f64 {
        return Err(NcError::new("Stock grid exceeds its cell budget"));
    }
    let nx = nx as usize;
    let ny = ny as usize;
    let sweeps = sweeps(model, job)?;
    let result = sample(
        min,
        max,
        nx,
        ny,
        &sweeps,
        budget("maxTests", 5000000)?.min(100000000) as usize,
    )
    .map_err(NcError::new)?;
    let dx = (max[0] - min[0]) / nx as f64;
    let dy = (max[1] - min[1]) / ny as f64;
    let witness = |i: usize| {
        usize::try_from(result.witnesses[i]).ok().and_then(|j|model.segments.get(j)).map(|s|json!({"line":s.line,"step":s.step,"tool":s.tool,"source":model.executions.get(s.step-1).map(|s|&s.source)}))
    };
    let mut sections = Vec::new();
    if let Some(requests) = config["sections"].as_array() {
        for (i, section) in requests.iter().enumerate() {
            let axis = match section["axis"].as_str() {
                Some("x") => 0,
                Some("y") => 1,
                _ => return Err(NcError::new("Sections need axis x or y")),
            };
            let at = section["at"]
                .as_f64()
                .filter(|v| *v >= min[axis] && *v <= max[axis])
                .ok_or_else(|| NcError::new("Section lies outside stock bounds"))?;
            let count = if axis == 0 { nx } else { ny };
            let spacing = if axis == 0 { dx } else { dy };
            let slot = (((at - min[axis]) / spacing).floor() as usize).min(count - 1);
            let across = 1 - axis;
            let mut range = [min[across], max[across]];
            if !section["range"].is_null() {
                range = serde_json::from_value(section["range"].clone())
                    .ok()
                    .filter(|range: &[f64; 2]| {
                        range[0] >= min[across] && range[1] <= max[across] && range[0] < range[1]
                    })
                    .ok_or_else(|| {
                        NcError::new("Section range must be increasing and inside the stock bounds")
                    })?;
            }
            let sample_spacing = if axis == 0 { dy } else { dx };
            let points: Vec<_> = (0..if axis == 0 { ny } else { nx })
                .filter_map(|n| {
                    let coordinate = min[across] + (n as f64 + 0.5) * sample_spacing;
                    if !(range[0]..=range[1]).contains(&coordinate) {
                        return None;
                    }
                    let cell = if axis == 0 {
                        n * nx + slot
                    } else {
                        slot * nx + n
                    };
                    let height = result.heights[cell];
                    Some(
                        json!({"coordinate":coordinate,"height":height,"remaining":height-min[2],
                    "through":height<=min[2],"witness":witness(cell)}),
                    )
                })
                .collect();
            if points.is_empty() {
                return Err(NcError::new(
                    "Section range contains no sample centers. Use a wider range or a finer grid.",
                ));
            }
            sections.push(json!({"id":section["id"].as_str().map(str::to_owned).unwrap_or_else(||format!("section-{}",i+1)),
                "axis":section["axis"],"requestedAt":at,"sampledAt":min[axis]+(slot as f64+0.5)*spacing,
                "range":range,"sampleSpacing":sample_spacing,"points":points}));
        }
    } else if !config["sections"].is_null() {
        return Err(NcError::new("Stock sections must be an array"));
    }
    let mut target = Value::Null;
    if !config["target"].is_null() {
        let z = config["target"]["z"]
            .as_f64()
            .filter(|v| *v >= min[2] && *v <= max[2])
            .ok_or_else(|| NcError::new("Target floor must be inside stock"))?;
        let tolerance_value = &config["target"]["tolerance"];
        let tolerance = if tolerance_value.is_null() {
            0.1
        } else {
            tolerance_value
                .as_f64()
                .ok_or_else(|| NcError::new("Target tolerance must be numeric"))?
        };
        if tolerance < 0. {
            return Err(NcError::new("Target tolerance cannot be negative"));
        }
        let bounds = &config["target"]["bounds"];
        let (target_min, target_max) = if bounds.is_null() {
            ([min[0], min[1]], [max[0], max[1]])
        } else {
            let xy = |key| -> Result<[f64; 2]> {
                let values = bounds[key]
                    .as_array()
                    .filter(|values| values.len() >= 2)
                    .ok_or_else(|| NcError::new("Stock target requires XY bounds"))?;
                match (values[0].as_f64(), values[1].as_f64()) {
                    (Some(x), Some(y)) => Ok([x, y]),
                    _ => Err(NcError::new("Stock target requires finite XY bounds")),
                }
            };
            let (min, max) = (xy("min")?, xy("max")?);
            if min.into_iter().zip(max).any(|(min, max)| min > max) {
                return Err(NcError::new("Stock target bounds must be ordered"));
            }
            (min, max)
        };
        let mut samples = 0;
        let mut leftover = 0;
        let mut gouged = 0;
        let mut remaining = None::<(f64, [f64; 3], usize)>;
        let mut gouge = None::<(f64, [f64; 3], usize)>;
        for y in 0..ny {
            for x in 0..nx {
                let index = y * nx + x;
                let p = [
                    min[0] + (x as f64 + 0.5) * dx,
                    min[1] + (y as f64 + 0.5) * dy,
                    result.heights[index],
                ];
                if (0..2).any(|i| p[i] < target_min[i] || p[i] > target_max[i]) {
                    continue;
                }
                samples += 1;
                let delta = p[2] - z;
                leftover += usize::from(delta > tolerance);
                gouged += usize::from(delta < -tolerance);
                if remaining.is_none_or(|v| delta > v.0) {
                    remaining = Some((delta, p, index))
                }
                if gouge.is_none_or(|v| delta < v.0) {
                    gouge = Some((delta, p, index))
                }
            }
        }
        let location =
            |(delta, point, index)| json!({"delta":delta,"point":point,"witness":witness(index)});
        target = json!({"z":z,"tolerance":tolerance,"samples":samples,"leftover":leftover,"gouged":gouged,"worstRemaining":remaining.map(location),"worstGouge":gouge.map(location),"status":if !result.complete||samples==0{"not-run"}else if leftover>0||gouged>0{"failed"}else{"passed"}});
    }
    let mut report = json!({"model":"vertical-flat-mill-heightfield","complete":result.complete,"throughStep":model.executions.len(),"bounds":{"min":min,"max":max},"grid":{"nx":nx,"ny":ny,"dx":dx,"dy":dy,"sampling":"cell centers","lateralUncertainty":dx.hypot(dy)/2.},"tests":result.tests,"sections":sections,"target":target,"summary":{"cells":nx*ny,"removedVolumeEstimate":result.heights.iter().map(|h|(max[2]-h)*dx*dy).sum::<f64>(),"throughCells":result.heights.iter().filter(|h|**h<=min[2]).count()}});
    if include_heights {
        report["heights"] = json!(result.heights);
    }
    Ok(report)
}

pub fn select_state<'a>(model: &'a Model, selector: &Value) -> Result<&'a State> {
    let target: Target = serde_json::from_value(selector.clone())
        .map_err(|error| NcError::new(format!("Invalid checkpoint: {error}")))?;
    target.validate()?;
    model
        .executions
        .iter()
        .filter(|s| target.matches(s))
        .nth(target.visit.unwrap_or(1) - 1)
        .ok_or_else(|| NcError::new("Checkpoint was not reached"))
}
