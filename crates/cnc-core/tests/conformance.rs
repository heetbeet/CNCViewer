// Saved conformance expectations cover motion, program state and diagnostics.
use cnc_core::engine::{inspect, Options};
use serde_json::Value;
use std::collections::BTreeMap;

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-7 * a.abs().max(b.abs()).max(1.)
}

#[test]
fn interpreter_matches_conformance_cases() {
    let mut failures = Vec::new();
    for line in include_str!("fixtures/conformance.jsonl").lines() {
        let case: Value = serde_json::from_str(line).unwrap();
        let mut options: Options = serde_json::from_value(case["options"].clone()).unwrap();
        if case["options"]["diameterMode"] == true {
            if options.setup.is_null() {
                options.setup = serde_json::json!({});
            }
            options.setup["diameterMode"] = Value::Bool(true);
        }
        let libraries: BTreeMap<String, String> =
            serde_json::from_value(case["options"]["subprograms"].clone()).unwrap_or_default();
        let source = case["text"].as_str().unwrap();
        let filename = case["options"]["filename"].as_str().unwrap_or("program.nc");
        let mut differences = Vec::new();
        match inspect(source, filename, &libraries, &options) {
            Err(error) if case["status"] != "error" => differences.push(error.to_string()),
            Err(_) => {}
            Ok(model) => {
                if model.status != case["status"].as_str().unwrap() {
                    differences.push(format!("status {} != {}", model.status, case["status"]));
                }
                if model.segments.len() != case["segments"].as_u64().unwrap() as usize {
                    differences.push(format!(
                        "segments {} != {}",
                        model.segments.len(),
                        case["segments"]
                    ));
                }
                if model.executions.len() != case["steps"].as_u64().unwrap() as usize {
                    differences.push(format!(
                        "steps {} != {}",
                        model.executions.len(),
                        case["steps"]
                    ));
                }
                if let Some(elapsed) = case["elapsed"].as_f64() {
                    let actual = model.executions.last().map_or(0., |state| state.elapsed);
                    if !close(actual, elapsed) {
                        differences.push(format!("elapsed {actual} != {elapsed}"));
                    }
                }
                let events: Vec<&Value> = model.events.iter().map(|e| &e["type"]).collect();
                if events
                    .iter()
                    .copied()
                    .ne(case["events"].as_array().unwrap())
                {
                    differences.push(format!("events {events:?} != {}", case["events"]));
                }
                if case["requirements"].is_array()
                    && serde_json::json!(model.requirements) != case["requirements"]
                {
                    differences.push(format!(
                        "requirements {:?} != {}",
                        model.requirements, case["requirements"]
                    ));
                }
                let expected_moves = case["motion"].as_array().unwrap();
                if model.moves.len() != expected_moves.len() {
                    differences.push(format!(
                        "moves {} != {}",
                        model.moves.len(),
                        expected_moves.len()
                    ));
                }
                for (index, (actual, expected)) in
                    model.moves.iter().zip(expected_moves).enumerate()
                {
                    if actual.rapid != expected["rapid"].as_bool().unwrap()
                        || actual.line != expected["line"].as_u64().unwrap() as usize
                        || actual
                            .end
                            .iter()
                            .zip(expected["end"].as_array().unwrap())
                            .any(|(a, b)| !close(*a, b.as_f64().unwrap()))
                    {
                        differences.push(format!(
                            "move {} {:?} != {}",
                            index + 1,
                            actual.end,
                            expected
                        ));
                        break;
                    }
                }
                if let Some(state) = model.executions.last() {
                    if let Some(common) = case["common"].as_object() {
                        for (address, value) in common {
                            let actual = state
                                .variables
                                .get(&address.parse().unwrap())
                                .copied()
                                .flatten();
                            let expected = value.as_f64();
                            if !match (actual, expected) {
                                (Some(a), Some(b)) => close(a, b),
                                (None, None) => true,
                                _ => false,
                            } {
                                differences.push(format!("#{address}: {actual:?} != {expected:?}"));
                            }
                        }
                    }
                }
            }
        }
        if !differences.is_empty() {
            failures.push(format!("{source:?}: {}", differences.join(", ")));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn program_cuts_in_all_three_arc_planes() {
    let model = inspect(
        "G21 G90 G94 G17\nG2 X10 Y0 I5 J0 F200\nG18\nG2 X20 Z0 I5 K0\nG19\nG3 Y10 Z0 J5 K0\nM30",
        "arc-planes.nc",
        &BTreeMap::new(),
        &Options::default(),
    )
    .unwrap();
    assert_eq!(model.status, "complete", "{:?}", model.diagnostics);
    let endpoints: Vec<_> = model.moves.iter().map(|movement| movement.end).collect();
    assert_eq!(endpoints, [[10., 0., 0.], [20., 0., 0.], [20., 10., 0.]]);
    let planes: std::collections::BTreeSet<_> = model
        .executions
        .iter()
        .filter(|s| [2., 3.].contains(&s.motion))
        .map(|s| s.plane)
        .collect();
    assert_eq!(planes, [17, 18, 19].into_iter().collect());
}

#[test]
fn macro_compass_sample_runs_with_four_or_six_spokes() {
    let source = include_str!("../../../examples/CNCViewerDemo.nc");
    for spokes in [4, 6] {
        let source = source.replacen("#100 = 4", &format!("#100 = {spokes}"), 1);
        let model = inspect(
            &source,
            "CNCViewerDemo.nc",
            &BTreeMap::new(),
            &Options::default(),
        )
        .unwrap();
        assert_eq!(model.status, "complete", "{:?}", model.diagnostics);
        let last = model.executions.last().unwrap();
        assert_eq!(last.position, [0., 0., 8.]);
        assert_eq!(last.variables[&107], Some(spokes as f64));
        let holes: Vec<_> = model
            .moves
            .iter()
            .filter(|movement| !movement.rapid && movement.end[2] == -2.)
            .collect();
        assert_eq!(holes.len(), spokes);
        for hole in holes {
            assert!((hole.end[0].hypot(hole.end[1]) - 22.).abs() < 1e-9);
        }
        assert!(model.executions.iter().any(|state| state.motion == 3.));
        assert!(model.events.iter().any(|event| event["type"] == "dwell"));
    }
}
