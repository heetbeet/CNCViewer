use cnc_core::engine::{inspect, Options};
use serde_json::{json, Value};
use std::{collections::BTreeMap, fmt::Write};
fn run(text: &str, setup: Value) -> cnc_core::engine::Model {
    inspect(
        text,
        "test.nc",
        &BTreeMap::new(),
        &Options {
            setup,
            ..Options::default()
        },
    )
    .unwrap()
}
#[test]
fn macro_programs_produce_expected_linear_moves() {
    let expected_motion: Value =
        serde_json::from_str(include_str!("fixtures/macro-motion.json")).unwrap();
    for (name, text) in [
        ("loop", include_str!("fixtures/macro-loop.nc")),
        ("call", include_str!("fixtures/macro-call.nc")),
        ("math", include_str!("fixtures/macro-math.nc")),
    ] {
        let model = run(text, Value::Null);
        assert_eq!(model.status, "complete", "{:?}", model.diagnostics);
        let moves = expected_motion[name].as_array().unwrap();
        assert_eq!(model.moves.len(), moves.len(), "{name}");
        for (actual, expected) in model.moves.iter().zip(moves) {
            assert_eq!(actual.line, expected["line"].as_u64().unwrap() as usize);
            assert_eq!(actual.rapid, expected["rapid"]);
            for axis in 0..3 {
                assert!(
                    (actual.end[axis] - expected["end"][axis].as_f64().unwrap()).abs() < 1e-9,
                    "{name} axis {axis}"
                )
            }
        }
    }
}

#[test]
fn large_macro_program_preserves_moves_and_variable_snapshots() {
    let mut text = String::from("G90 G21 G94\n");
    for x in 1..=12_000 {
        writeln!(text, "#100=[{x}+ABS[SIN[30]*2-1]]").unwrap();
        text.push_str("G1 X[#100+COS[0]-1] Y[ABS[FIX[2.9]-4]] F100\n");
    }
    text.push_str("M30\n");
    let model = inspect(
        &text,
        "large-macros.nc",
        &BTreeMap::new(),
        &Options {
            capture_variables: true,
            ..Options::default()
        },
    )
    .unwrap();
    assert_eq!(model.status, "complete", "{:?}", model.diagnostics);
    assert_eq!(model.moves.len(), 12_000);
    assert_eq!(model.moves.last().unwrap().end, [12_000., 2., 0.]);
    assert_eq!(model.executions[2].variables[&100], Some(1.));
    assert_eq!(
        model.executions.last().unwrap().variables[&100],
        Some(12_000.)
    );
}
#[test]
fn goto_and_macro_slots() {
    let m = run(
        include_str!("../../../examples/macro-machining.nc"),
        json!({"tools":{"1":{"length":20,"diameter":8}}}),
    );
    assert_eq!(m.status, "complete", "{:?}", m.diagnostics);
    assert_eq!(m.moves.len(), 52);
    assert_eq!(m.executions.last().unwrap().position, [0., 0., 5.]);
    assert_eq!(m.executions.last().unwrap().variables[&100], Some(4.));
}
#[test]
fn never_continue_missing_state_or_infinite_flow() {
    let m = run("G0 X0\nM777\nG1 X99 F100", Value::Null);
    assert_eq!(m.status, "input-required");
    assert_eq!(m.moves.len(), 1);
    let m = run("N10 GOTO10", json!({"maxExecutions":10}));
    assert_eq!(m.status, "limit");
    assert_eq!(m.executions.len(), 10);
    let m = run("G0 X0\nG2 X100 R1 F100\nG1 X99", Value::Null);
    assert_eq!(m.status, "error");
    assert_eq!(m.moves.len(), 1);
}

#[test]
fn diagnostics_name_the_problem_and_preserve_partial_geometry() {
    for (block, status, code, message) in [
        (
            "G2 X20 R4 F100",
            "error",
            "GEOMETRY",
            "Arc radius R is too small to reach the endpoint",
        ),
        (
            "G3 X20 R-4 F100",
            "error",
            "GEOMETRY",
            "Arc radius R is too small to reach the endpoint",
        ),
        (
            "G2 X0 R5 F100",
            "error",
            "GEOMETRY",
            "Full-circle arcs need an I/J/K center",
        ),
        (
            "G3 X5 I1 F100",
            "error",
            "GEOMETRY",
            "Arc start and end are different distances from the I/J/K center",
        ),
        ("G1 X", "error", "PROGRAM", "Address X is missing a value"),
        ("#100=[12/0]", "error", "ARITHMETIC", "Division by zero"),
        (
            "#100=SQRT[-1]",
            "error",
            "ARITHMETIC",
            "SQRT needs a nonnegative value",
        ),
        (
            "G123 X2",
            "error",
            "UNSUPPORTED",
            "G123 is unsupported in CNC Viewer",
        ),
        (
            "G55 G0 X2",
            "input-required",
            "INPUT_REQUIRED",
            "Work offset G55 is missing from machine setup",
        ),
    ] {
        let model = run(&format!("G0 X0 Y0\n{block}\nG0 X50\nM30"), Value::Null);
        assert_eq!(model.status, status, "{block}");
        assert_eq!(model.moves.len(), 1, "{block}");
        assert_eq!(model.moves[0].end, [0., 0., 0.], "{block}");
        let diagnostic = &model.diagnostics[0];
        assert_eq!(diagnostic.source, "test.nc");
        assert_eq!(diagnostic.line, 2, "{block}");
        assert_eq!(diagnostic.error.code, code, "{block}");
        assert_eq!(diagnostic.error.message, message, "{block}");
    }
}

#[test]
fn unset_variables_warn_without_changing_execution_or_nil_checks() {
    let model = run(
        "IF [#101 EQ #0] THEN #101=5\nIF [#0 NE #111] GOTO 99\n#103=0\nWHILE [#103 LT 2] DO1\n#100=[#102+#102]\n#103=[#103+1]\nEND1\n#104=#0\nG0 X5 Y6\nG1 X#104 Y#105 Z1 F100\nM30",
        Value::Null,
    );
    assert_eq!(model.status, "complete");
    assert_eq!(model.moves.last().unwrap().end, [5., 6., 1.]);
    let variables = &model.executions.last().unwrap().variables;
    assert_eq!(variables[&100], Some(0.));
    assert_eq!(variables[&101], Some(5.));
    assert_eq!(variables[&104], None);
    assert_eq!(
        model
            .warnings
            .iter()
            .map(|note| (
                note.line,
                note.error.code.as_str(),
                note.error.message.as_str()
            ))
            .collect::<Vec<_>>(),
        [
            (
                5,
                "UNASSIGNED_VARIABLE",
                "Variable #102 used before assignment"
            ),
            (
                10,
                "UNASSIGNED_VARIABLE",
                "Variable #105 used before assignment"
            ),
        ]
    );
}
#[test]
fn offsets_units_and_arcs() {
    let m = run(
        "G21 G0 X0 Y0 Z0\nG1 X1 F100\nG3 X0 Y1 I-1 J0\nG91 G1 Z-1\nG90 G0 Z5",
        json!({"workOffsets":{"54":[10,20,0]}}),
    );
    assert_eq!(m.status, "complete", "{:?}", m.diagnostics);
    assert_eq!(m.executions.last().unwrap().position, [10., 21., 5.]);
    assert!(m.segments.len() > 18);
    let m = run("G20 G0 X1\nG21 G1 X50.8 F100", Value::Null);
    assert_eq!(m.moves[0].end[0], 25.4);
    assert_eq!(m.moves[1].end[0], 50.8);
}

#[test]
fn modal_groups_diameter_and_nil_are_real_state() {
    let m=run("#100=#4009\n#101=#4014\nG0X20\nG81X20Z-2R1F100\n#102=#4009\nG80\n#103=#0\n#104=[#103+3]\nM30",json!({"diameterMode":true}));
    assert_eq!(m.status, "complete", "{:?}", m.diagnostics);
    let vars = &m.executions.last().unwrap().variables;
    assert_eq!(vars[&100], Some(80.));
    assert_eq!(vars[&101], Some(54.));
    assert_eq!(vars[&102], Some(81.));
    assert_eq!(vars[&103], None);
    assert_eq!(vars[&104], Some(3.));
    assert_eq!(m.moves[0].end[0], 10.);
    assert_eq!(run("G7X1", Value::Null).status, "error");
}

#[test]
fn configured_probe_clips_travel_and_publishes_measurement() {
    let text = "G0X0Y0Z5\nG31Z-10F100\n#100=#5063\n#101=#5073\nM30";
    let m = run(
        text,
        json!({"probes":{"1":{"plane":{"axis":"Z","position":0},"stylusRadius":1}}}),
    );
    assert_eq!(m.status, "complete", "{:?}", m.diagnostics);
    assert_eq!(m.moves.last().unwrap().end, [0., 0., 1.]);
    assert_eq!(m.executions.last().unwrap().variables[&100], Some(1.));
    assert!(m
        .events
        .iter()
        .any(|e| e["type"] == "probe" && e["details"]["triggered"] == true));
    assert_eq!(run(text, Value::Null).status, "input-required");
    let m = run(
        text,
        json!({"probes":{"1":{"results":[{"position":[100,0,0]}]}}}),
    );
    assert_eq!(m.status, "error");
    assert_eq!(m.moves.len(), 1);
}

#[test]
fn oem_signals_arguments_settings_and_prints() {
    let m=run("M200\n#100=#1100\nM201\nM220S3\nM224S3V101\nM223S2V102\nM243T42 (Data: \"offset\")\nM242V103 (Data: \"offset\")\nDPRNT[VALUE*#103[43]]\nM1000\nM30",json!({"machineSettings":{"offset":1},"inputs":{"2":1}}));
    assert_eq!(m.status, "complete", "{:?}", m.diagnostics);
    let vars = &m.executions.last().unwrap().variables;
    for address in [100, 101, 102] {
        assert_eq!(vars[&address], Some(1.));
    }
    assert_eq!(vars[&103], Some(42.));
    assert_eq!(m.executions.last().unwrap().spindle, [0., 0.]);
    assert!(m
        .events
        .iter()
        .any(|e| e["type"] == "dprnt" && e["details"]["text"] == "VALUE   42.000"));
    assert!(m.events.iter().any(|e| e["type"] == "program-stop"));
}

#[test]
fn malformed_setup_and_extreme_duration_are_bounded() {
    for setup in [
        json!({"tools":3}),
        json!({"tools":{"1":4}}),
        json!({"initialPosition":[1e100]}),
        json!({"rapidRate":-1}),
    ] {
        assert!(inspect(
            "G0X0",
            "x.nc",
            &BTreeMap::new(),
            &Options {
                setup,
                ..Default::default()
            }
        )
        .is_err());
    }
    let m = run("G0X0\nG1X10F[10**-300]", Value::Null);
    assert_ne!(m.status, "complete");
    assert!(m.executions.iter().all(|s| s.elapsed.is_finite()));
}

#[test]
fn inspection_does_not_pass_unreached_targets_operations_or_overrides() {
    let m = run("G0X0\nG1X10F100\nM30", Value::Null);
    let job = json!({"until":{"line":99},"operations":[{"id":"slot","from":{"line":2},"to":{"line":3},"tool":2}],"assignmentOverrides":[{"source":"test.nc","line":2,"variable":100,"value":5}]});
    let report = cnc_core::analysis::validate(&m, &job).unwrap();
    assert_eq!(report["validation"], "failed");
    for check in ["targetNotReached", "operationTool", "overrideNotApplied"] {
        assert!(report["findings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| f["check"] == check));
    }
    let report=cnc_core::analysis::validate(&m,&json!({"cutter":{"diameter":8},"checks":{"allowedBounds":{"min":[0,-5,-1],"max":[10,5,1]}}})).unwrap();
    assert_eq!(report["firstFailure"]["check"], "cutterBounds");
}

#[test]
fn system_signals_tool_table_and_extended_offsets() {
    let m=run("#1132=2147483649\n#100=#1132\n#11001=25\n#101=#11001\n#7001=12\nG54.1P1\nG0X0\n#102=#5021\nM30",json!({"workOffsets":{"54.1:1":[0,0,0]}}));
    assert_eq!(m.status, "complete", "{:?}", m.diagnostics);
    let vars = &m.executions.last().unwrap().variables;
    assert_eq!(vars[&100], Some(2147483649.));
    assert_eq!(vars[&101], Some(25.));
    assert_eq!(vars[&102], Some(12.));
}

#[test]
fn seeded_locals_and_parameter_replacement_are_visible() {
    let m = run(
        "#100=#1\n#1=9\n#3004=3\n#101=#3004\nM30",
        json!({"variables":{"1":7}}),
    );
    assert_eq!(m.status, "complete");
    assert_eq!(m.executions.last().unwrap().variables[&100], Some(7.));
    let report = cnc_core::analysis::validate(&m, &json!({})).unwrap();
    assert_eq!(report["validation"], "passed");
    assert!(report["findings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|f| f["check"] == "initialVariableOverwritten" && f["severity"] == "warning"));
}

#[test]
fn dwell_preserves_decimal_literal_units() {
    let m = run("G4P1.0\nG4P1\nM30", Value::Null);
    assert_eq!(m.status, "complete");
    assert_eq!(m.executions[0].elapsed, 1001.);
    assert_eq!(m.executions[1].elapsed, 1003.);
}

#[test]
fn long_left_associative_expression_stays_within_lexer_budget() {
    let m = run(&format!("#100=2000{}\nM30", "-1".repeat(1500)), Value::Null);
    assert_eq!(m.status, "complete");
    assert_eq!(m.executions.last().unwrap().variables[&100], Some(500.));
}

#[test]
fn explicit_clock_crosses_utc_midnight_and_buffered_nil_is_preserved() {
    let m = run(
        "#100=#4109\n#101=#3011\nG4 P1.0\n#102=#3011\n#103=#3012\nM30",
        json!({"clock":"2026-12-31T21:59:59-02:00"}),
    );
    assert_eq!(m.status, "complete");
    let variables = &m.executions.last().unwrap().variables;
    assert_eq!(variables[&100], None);
    assert_eq!(variables[&101], Some(20261231.));
    assert_eq!(variables[&102], Some(20270101.));
    assert_eq!(variables[&103], Some(0.));
}
