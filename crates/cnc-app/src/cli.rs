use cnc_core::{
    analysis, compiler,
    engine::{self, Options},
    motion,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::{self, BufWriter, Read, Write},
    path::Path,
    time::Instant,
};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
const MAX_INPUT_BYTES: u64 = 10_000_000;

pub fn input(file: &str) -> Result<String> {
    read_input(file, MAX_INPUT_BYTES)
}
fn read_input(file: &str, max_bytes: u64) -> Result<String> {
    let mut data = Vec::new();
    if file == "-" {
        io::stdin().take(max_bytes + 1).read_to_end(&mut data)?;
    } else {
        fs::File::open(file)?
            .take(max_bytes + 1)
            .read_to_end(&mut data)?;
    }
    if data.len() as u64 > max_bytes {
        return Err("Source and libraries exceed inputBytes".into());
    }
    Ok(String::from_utf8(data)?)
}
fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
fn write(path: &Path, bytes: &[u8]) -> Result<()> {
    write_artifact(path, |mut file| Ok(file.write_all(bytes)?))
}
pub(crate) fn write_artifact(
    path: &Path,
    writer: impl FnOnce(fs::File) -> Result<()>,
) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?
    }
    let mut filename = path
        .file_name()
        .ok_or("Missing evidence filename")?
        .to_owned();
    filename.push(format!(".{}.tmp", std::process::id()));
    let temp = path.with_file_name(filename);
    let file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)?;
    let result = writer(file).and_then(|()| Ok(fs::rename(&temp, path)?));
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}
fn valid_name(name: &str) -> bool {
    let base = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    let device = matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (base.len() == 4
            && matches!(&base.as_bytes()[..3], b"COM" | b"LPT")
            && matches!(base.as_bytes()[3], b'1'..=b'9'));
    !device
        && name
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        && name.len() <= 80
        && !name.ends_with('.')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b'.')
}
fn merge(base: &mut Value, overrides: &Value) {
    if let Some(values) = overrides.as_object() {
        if !base.is_object() {
            *base = json!({});
        }
        for (key, value) in values {
            if value.is_object() {
                merge(&mut base[key], value)
            } else {
                base[key] = value.clone()
            }
        }
    } else {
        *base = overrides.clone()
    }
}
fn set_path(value: &mut Value, path: &[&str], replacement: Value) -> Result<()> {
    if path.is_empty() {
        *value = replacement;
        return Ok(());
    }
    if ["__proto__", "constructor", "prototype"].contains(&path[0]) {
        return Err("Invalid matrix path".into());
    }
    if let Some(values) = value.as_array_mut() {
        let index = path[0].parse::<usize>()?;
        let value = values
            .get_mut(index)
            .ok_or("Matrix array index out of range")?;
        return set_path(value, &path[1..], replacement);
    }
    if value.is_null() {
        *value = json!({})
    }
    if !value.is_object() {
        return Err("Matrix path crosses a scalar value".into());
    }
    set_path(&mut value[path[0]], &path[1..], replacement)
}
fn cases(job: &Value) -> Result<Vec<(String, Value)>> {
    if !job["cases"].is_null() && !job["cases"].is_array() {
        return Err("cases must be an array".into());
    }
    if !job["matrix"].is_null() && !job["matrix"].is_object() {
        return Err("matrix must be an object".into());
    }
    if !job["cases"].is_null() && !job["matrix"].is_null() {
        return Err("Use cases or matrix, not both".into());
    }
    if let Some(cases) = job["cases"].as_array() {
        if cases.is_empty() || cases.len() > 128 {
            return Err("Cases must contain 1 to 128 entries".into());
        }
        let mut ids = std::collections::BTreeSet::new();
        return cases
            .iter()
            .map(|case| {
                let id = case["id"]
                    .as_str()
                    .filter(|id| valid_name(id) && ids.insert(id.to_ascii_lowercase()))
                    .ok_or("Case IDs must be unique filesystem-safe names")?;
                let mut config = job.clone();
                merge(&mut config, case);
                Ok((id.to_owned(), config))
            })
            .collect();
    }
    let mut rows = vec![job.clone()];
    if let Some(matrix) = job["matrix"].as_object() {
        for (key, values) in matrix {
            let path: Vec<_> = key.split('.').collect();
            if !["setup", "cutter", "checks", "assignmentOverrides"].contains(&path[0])
                || path.len() < 2
                || path.iter().any(|key| {
                    key.is_empty() || !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                })
            {
                return Err("Invalid matrix path".into());
            }
            let values = values
                .as_array()
                .filter(|a| !a.is_empty())
                .ok_or("Matrix values must be nonempty arrays")?;
            if values.len() > 128 / rows.len() {
                return Err("Matrix exceeds 128 cases".into());
            }
            let mut next = Vec::new();
            for row in rows {
                for value in values {
                    let mut row = row.clone();
                    set_path(&mut row, &path, value.clone())?;
                    next.push(row)
                }
            }
            rows = next;
        }
    }
    Ok(rows
        .into_iter()
        .enumerate()
        .map(|(i, row)| {
            (
                if job["matrix"].is_object() {
                    format!("case-{:03}", i + 1)
                } else {
                    "nominal".into()
                },
                row,
            )
        })
        .collect())
}
fn limit(job: &Value, key: &str, default: u64, max: u64) -> Result<u64> {
    let value = &job["limits"][key];
    if value.is_null() {
        return Ok(default);
    }
    value
        .as_u64()
        .filter(|n| (1..=max).contains(n))
        .ok_or_else(|| format!("limits.{key} must be an integer in 1..={max}").into())
}

fn artifact_directory(
    output: &Path,
    protected: &[std::path::PathBuf],
) -> Result<std::path::PathBuf> {
    fs::create_dir_all(output)?;
    let output = fs::canonicalize(output)?;
    if protected.iter().any(|file| file.starts_with(&output)) {
        return Err("Artifact directory contains a job input".into());
    }
    Ok(output)
}
fn export_movements(
    output: &Path,
    model: &engine::Model,
    operations: &Value,
    request: &Value,
) -> Result<Value> {
    let selection = motion::Selection::resolve(model, operations, &request["select"])?;
    let jsonl = request["format"] == "jsonl";
    let feed_only = request["feedOnly"] == true;
    let name = if jsonl {
        "movements.jsonl"
    } else {
        "movements.json"
    };
    let last = selection.bounds().map_or(0, |(_, last)| last);
    let visits: Vec<_> = motion::visits(model).take(last).collect();
    let mut written = 0;
    write_artifact(&output.join(name), |file| {
        let mut writer = BufWriter::new(file);
        if !jsonl {
            writer.write_all(b"[")?;
        }
        for (index, movement) in selection
            .moves(model)
            .filter(|(_, movement)| !feed_only || !movement.rapid)
        {
            let state = model.executions.get(movement.step.saturating_sub(1));
            let mut record = serde_json::to_value(movement)?;
            record["id"] = json!(index + 1);
            record["source"] = json!(state.map(|state| &state.source));
            record["program"] = json!(state.and_then(|state| state.program));
            record["sequence"] = json!(state.and_then(|state| state.sequence));
            record["visit"] = json!(visits.get(movement.step.saturating_sub(1)));
            let segments = &model.segments[movement.segment_start..movement.segment_end];
            let mut vertices = Vec::with_capacity(segments.len() + 1);
            vertices.push(movement.start);
            if segments.is_empty() {
                vertices.push(movement.end);
            } else {
                vertices.extend(segments.iter().map(|s| s.end));
            }
            record["vertices"] = json!(vertices);
            if !jsonl && written > 0 {
                writer.write_all(b",")?;
            }
            serde_json::to_writer(&mut writer, &record)?;
            if jsonl {
                writer.write_all(b"\n")?;
            }
            written += 1;
        }
        if !jsonl {
            writer.write_all(b"]")?;
        }
        writer.flush()?;
        Ok(())
    })?;
    Ok(json!({"file":name,"records":written,"format":if jsonl { "jsonl" } else { "json" }}))
}
pub fn inspect(file: &str, config: Option<&str>) -> Result<(Value, i32)> {
    let started = Instant::now();
    let job: Value = if let Some(config) = config {
        serde_json::from_str(&fs::read_to_string(config)?)?
    } else {
        json!({})
    };
    if !job.is_object() {
        return Err("Job must be a JSON object".into());
    }
    if !job["libraries"].is_null() && !job["libraries"].is_array() {
        return Err("libraries must be an array".into());
    }
    if !job["limits"].is_null() && !job["limits"].is_object() {
        return Err("limits must be an object".into());
    }
    let input_limit = limit(&job, "inputBytes", MAX_INPUT_BYTES, MAX_INPUT_BYTES)?;
    let text = read_input(file, input_limit)?;
    let mut remaining_bytes = input_limit - text.len() as u64;
    let base = if let Some(config) = config {
        Path::new(config).parent().unwrap_or(Path::new("."))
    } else {
        Path::new(file).parent().unwrap_or(Path::new("."))
    };
    let base = if base.as_os_str().is_empty() {
        Path::new(".")
    } else {
        base
    };
    let filename = if file == "-" {
        job["filename"].as_str().unwrap_or("stdin.nc").to_owned()
    } else {
        Path::new(file)
            .file_name()
            .ok_or("Missing filename")?
            .to_string_lossy()
            .into_owned()
    };
    let mut libraries = BTreeMap::new();
    let mut protected = Vec::new();
    if file != "-" {
        protected.push(fs::canonicalize(file)?)
    }
    if let Some(config) = config {
        protected.push(fs::canonicalize(config)?)
    }
    if let Some(files) = job["libraries"].as_array() {
        for name in files {
            let name = name.as_str().ok_or("Library filenames must be strings")?;
            let path = base.join(name);
            protected.push(fs::canonicalize(&path)?);
            let filename = path
                .file_name()
                .ok_or("Missing library filename")?
                .to_string_lossy()
                .into_owned();
            let source = read_input(
                path.to_str().ok_or("Invalid library path")?,
                remaining_bytes,
            )?;
            remaining_bytes -= source.len() as u64;
            if libraries.insert(filename, source).is_some() {
                return Err("Duplicate library filename".into());
            }
        }
    }
    let scenarios = cases(&job)?;
    let one_case = scenarios.len() == 1;
    let compiled_at = Instant::now();
    let compiled = compiler::compile(&text, &filename, &libraries)?;
    let compile_ms = compiled_at.elapsed().as_secs_f64() * 1000.;
    let engine_hash = digest(&fs::read(std::env::current_exe()?)?);
    let source_hash = digest(text.as_bytes());
    let library_hashes: BTreeMap<_, _> = libraries
        .iter()
        .map(|(name, text)| (name, digest(text.as_bytes())))
        .collect();
    let mut results = Vec::new();
    let mut gpu = None;
    for (case_index, (id, mut job)) in scenarios.into_iter().enumerate() {
        job.as_object_mut()
            .ok_or("Job must be a JSON object")?
            .remove("matrix");
        job.as_object_mut().ok_or("Invalid job")?.remove("cases");
        for key in ["artifacts", "limits", "stock", "cutter"] {
            if !job[key].is_null() && !job[key].is_object() {
                return Err(format!("{key} must be an object").into());
            }
        }
        if !job["views"].is_null() && !job["views"].is_array() {
            return Err("views must be an array".into());
        }
        if !job["output"].is_null() && !job["output"].is_string() {
            return Err("output must be a directory path".into());
        }
        if let Some(artifacts) = job["artifacts"].as_object() {
            for (key, value) in artifacts {
                match key.as_str() {
                    "movements" if value.is_object() => {
                        if value
                            .as_object()
                            .unwrap()
                            .keys()
                            .any(|k| !["select", "format", "feedOnly"].contains(&k.as_str()))
                            || !(value["format"].is_null()
                                || matches!(value["format"].as_str(), Some("json" | "jsonl")))
                            || !(value["feedOnly"].is_null() || value["feedOnly"].is_boolean())
                        {
                            return Err("artifacts.movements accepts select, format (json or jsonl), and boolean feedOnly".into());
                        }
                    }
                    "movements" | "footprints" | "stock" | "sections" => {
                        if !value.is_null() && !value.is_boolean() {
                            return Err(format!("artifacts.{key} must be boolean").into());
                        }
                    }
                    "trace" => {
                        if !value.is_null() && !value.is_boolean() && !value.is_object() {
                            return Err("artifacts.trace must be boolean or a window object".into());
                        }
                    }
                    _ => return Err(format!("Unknown artifact: {key}").into()),
                }
            }
        }
        if job["stock"].is_null()
            && (job["artifacts"]["stock"] == true || job["artifacts"]["sections"] == true)
        {
            return Err("Stock evidence requires a stock configuration".into());
        }
        if input_limit - remaining_bytes > limit(&job, "inputBytes", input_limit, MAX_INPUT_BYTES)?
        {
            return Err("Source and libraries exceed case limits.inputBytes".into());
        }
        let finding_limit = limit(&job, "findings", 100, 10_000)? as usize;
        let trace_limit = limit(&job, "traceBytes", 2_000_000, usize::MAX as u64)? as usize;
        let mut options: Options = serde_json::from_value(job.clone())?;
        options.capture_variables =
            job["artifacts"]["trace"] != Value::Null && job["artifacts"]["trace"] != false;
        options.time_limit_ms = Some(if job["limits"]["timeMs"].is_null() {
            30000
        } else {
            job["limits"]["timeMs"]
                .as_u64()
                .filter(|n| *n > 0)
                .ok_or("limits.timeMs must be a positive integer")?
        });
        let parsed_at = Instant::now();
        let model = engine::inspect_compiled(&compiled, &filename, &options)?;
        let interpret_ms = parsed_at.elapsed().as_secs_f64() * 1000.;
        let analyzed_at = Instant::now();
        let mut report = analysis::validate(&model, &job)?;
        report["caseId"] = json!(id);
        report["reuse"] = json!({"compiledSource":case_index>0,"machineState":"fresh"});
        if !job["post"].is_null() {
            report["post"] = job["post"].clone();
        }
        report["identity"] = json!({"sourceSha256":source_hash,"setupSha256":digest(serde_json::to_string(&options.setup)?.as_bytes()),"librariesSha256":digest(serde_json::to_string(&libraries)?.as_bytes()),"libraries":library_hashes,"optionsSha256":digest(&serde_json::to_vec(&job)?),"engine":{"version":env!("CARGO_PKG_VERSION"),"sha256":engine_hash}});
        let stock = if !job["stock"].is_null() {
            let mut stock =
                analysis::stock_report(&model, &job, job["artifacts"]["stock"] == true)?;
            let heights = stock
                .as_object_mut()
                .expect("stock report")
                .remove("heights");
            report["stock"] = stock.clone();
            if let Some(heights) = heights {
                stock["heights"] = heights;
            }
            if stock["complete"] != true || stock["target"]["status"] == "not-run" {
                report["validation"] = json!("incomplete")
            } else if stock["target"]["status"] == "failed" && report["validation"] == "passed" {
                report["validation"] = json!("failed")
            }
            if stock["target"]["status"] == "failed"
                || stock["target"]["status"] == "not-run"
                || stock["complete"] != true
            {
                let target = &stock["target"];
                if stock["complete"] != true {
                    analysis::append_failure(
                        &mut report,
                        json!({"check":"stockLimit","severity":"error","message":"Stock sampling budget reached"}),
                    );
                } else if target["status"] == "not-run" {
                    analysis::append_failure(
                        &mut report,
                        json!({"check":"stockTargetNotSampled","severity":"error","message":"Stock target contains no grid samples"}),
                    );
                } else {
                    let tolerance = target["tolerance"]
                        .as_f64()
                        .ok_or("Missing stock target tolerance")?;
                    for (check, kind, direction) in [
                        ("stockRemaining", "worstRemaining", 1.),
                        ("stockGouge", "worstGouge", -1.),
                    ] {
                        let sample = &target[kind];
                        if sample["delta"]
                            .as_f64()
                            .is_some_and(|delta| delta * direction > tolerance)
                        {
                            let witness = &sample["witness"];
                            analysis::append_failure(
                                &mut report,
                                json!({"check":check,"severity":"error","source":witness["source"],"line":witness["line"],"step":witness["step"],"tool":witness["tool"],"point":sample["point"],"actual":sample["delta"],"expected":tolerance}),
                            );
                        }
                    }
                }
            }
            Some(stock)
        } else {
            None
        };
        report["timing"] = json!({"compileMs":if case_index==0{compile_ms}else{0.},"interpretMs":interpret_ms,"analysisMs":analyzed_at.elapsed().as_secs_f64()*1000.});
        let artifacts = &job["artifacts"];
        let mut save_report = None;
        if !job["output"].is_null()
            || artifacts
                .as_object()
                .is_some_and(|a| a.values().any(|v| *v != false && !v.is_null()))
            || job["views"].as_array().is_some_and(|v| !v.is_empty())
        {
            let root = base.join(job["output"].as_str().unwrap_or(".cnc/inspection"));
            let root = artifact_directory(&root, &protected)?;
            let output = if one_case {
                root
            } else {
                artifact_directory(&root.join(&id), &protected)?
            };
            report["artifacts"] = json!({"directory":output});
            let artifact_result = (|| -> Result<()> {
                if artifacts["movements"] == true || artifacts["movements"].is_object() {
                    report["artifacts"]["movements"] = export_movements(
                        &output,
                        &model,
                        &report["operations"],
                        &artifacts["movements"],
                    )?;
                }
                if artifacts["footprints"] == true {
                    let sweeps = analysis::sweeps(&model, &job)?;
                    let data:Vec<_>=sweeps.into_iter().map(|s|{
                        let segment=&model.segments[s.witness];
                        let radius=s.diameter/2.;
                        let min:[f64;3]=std::array::from_fn(|i|s.start[i].min(s.end[i])-if i<2 {radius}else{0.});
                        let max:[f64;3]=std::array::from_fn(|i|s.start[i].max(s.end[i])+if i<2 {radius}else{0.});
                        json!({"start":s.start,"end":s.end,"diameter":s.diameter,"id":s.witness+1,
                            "step":segment.step,"line":segment.line,"move":segment.movement+1,"tool":segment.tool,
                            "source":model.executions.get(segment.step-1).map(|state|&state.source),
                            "bounds":{"min":min,"max":max}})
                    }).collect();
                    write(&output.join("footprints.json"), &serde_json::to_vec(&data)?)?
                }
                if artifacts["trace"] != Value::Null && artifacts["trace"] != false {
                    let request = &artifacts["trace"];
                    let (start, end) = if *request == true {
                        (0, model.executions.len())
                    } else {
                        let step = if request["end"] == true {
                            Some(model.executions.len() as u64)
                        } else if !request["step"].is_null() {
                            request["step"].as_u64()
                        } else if let Some(finding) = request["finding"].as_str() {
                            report["findings"]
                                .as_array()
                                .and_then(|items| items.iter().find(|f| f["id"] == finding))
                                .and_then(|f| f["step"].as_u64())
                        } else {
                            report["firstFailure"]["step"].as_u64()
                        }
                        .ok_or("Trace window needs an executed step or finding")?
                            as usize;
                        let radius = if request["radius"].is_null() {
                            5
                        } else {
                            request["radius"]
                                .as_u64()
                                .ok_or("Trace radius must be a nonnegative integer")?
                        };
                        if radius > 500 || step == 0 || step > model.executions.len() {
                            return Err("Trace window is outside the execution".into());
                        }
                        (
                            step.saturating_sub(radius as usize + 1),
                            (step + radius as usize).min(model.executions.len()),
                        )
                    };
                    let mut bytes = Vec::new();
                    for state in &model.executions[start..end] {
                        serde_json::to_writer(&mut bytes, state)?;
                        bytes.push(b'\n');
                        if bytes.len() > trace_limit {
                            return Err("Trace exceeds byte budget".into());
                        }
                    }
                    write(&output.join("trace.jsonl"), &bytes)?;
                }
                let mut names = std::collections::BTreeSet::from([
                    "report".to_owned(),
                    "movements".to_owned(),
                    "stock".to_owned(),
                    "footprints".to_owned(),
                ]);
                if let Some(stock) = &stock {
                    if artifacts["stock"] == true {
                        write(&output.join("stock.json"), &serde_json::to_vec(stock)?)?
                    }
                    if artifacts["sections"] == true {
                        if let Some(sections) = stock["sections"].as_array() {
                            for section in sections {
                                let id = section["id"]
                                    .as_str()
                                    .filter(|s| valid_name(s))
                                    .ok_or("Invalid section ID")?;
                                if !names.insert(id.to_ascii_lowercase()) {
                                    return Err("Duplicate evidence filename".into());
                                }
                                write(
                                    &output.join(format!("{id}.svg")),
                                    crate::evidence::section_svg(stock, section)?.as_bytes(),
                                )?;
                                write(
                                    &output.join(format!("{id}.json")),
                                    &serde_json::to_vec_pretty(section)?,
                                )?
                            }
                        }
                    }
                }
                if let Some(views) = job["views"].as_array() {
                    if !views.is_empty() && gpu.is_none() {
                        gpu = Some(crate::render::headless()?)
                    }
                    for (i, view) in views.iter().enumerate() {
                        let finding = if let Some(id) = view["finding"].as_str() {
                            Some(
                                report["findings"]
                                    .as_array()
                                    .and_then(|items| items.iter().find(|f| f["id"] == id))
                                    .ok_or("Requested finding is absent")?,
                            )
                        } else {
                            None
                        };
                        let name = view["name"]
                            .as_str()
                            .or_else(|| finding.and_then(|f| f["id"].as_str()))
                            .map(str::to_owned)
                            .unwrap_or_else(|| format!("view-{}", i + 1));
                        if !valid_name(&name) || !names.insert(name.to_ascii_lowercase()) {
                            return Err("Evidence names must be unique and filesystem-safe".into());
                        }
                        let path = output.join(format!("{name}.png"));
                        let plot = gpu.as_mut().ok_or("Renderer unavailable")?;
                        let mut evidence = crate::evidence::capture(
                            plot,
                            &model,
                            &job,
                            &report["operations"],
                            view,
                            finding,
                            &path,
                        )?;
                        evidence["caseId"] = json!(id);
                        evidence["identity"] = report["identity"].clone();
                        evidence["coordinateFrame"] = report["coordinateFrame"].clone();
                        write(
                            &output.join(format!("{name}.json")),
                            &serde_json::to_vec_pretty(&evidence)?,
                        )?;
                        if report["artifacts"]["views"].is_null() {
                            report["artifacts"]["views"] = json!([]);
                        }
                        report["artifacts"]["views"].as_array_mut().expect("view list")
                            .push(json!({"image":path,"metadata":output.join(format!("{name}.json"))}));
                    }
                }
                Ok(())
            })();
            if let Err(error) = artifact_result {
                report["artifactError"] = json!(error.to_string());
                report["validation"] = json!("incomplete");
                report["checks"]["passed"] = json!(false);
            }
            save_report = Some(output.join("report.json"));
        }
        let count = report["findings"].as_array().map_or(0, Vec::len);
        if let Some(findings) = report["findings"].as_array_mut() {
            findings.truncate(finding_limit)
        }
        report["findingCount"] = json!(count);
        report["findingsTruncated"] = json!(count > finding_limit);
        if let Some(path) = save_report {
            write(&path, &serde_json::to_vec_pretty(&report)?)?;
        }
        results.push(report);
    }
    let code = if results.iter().all(|r| r["validation"] == "passed") {
        0
    } else {
        2
    };
    if results.len() == 1 {
        Ok((results.remove(0), code))
    } else {
        let first_failure = results
            .iter()
            .find(|report| report["validation"] != "passed")
            .map(|report| &report["caseId"]);
        let summary = json!({"schema":"cnc-inspection-matrix/2","validation":if code==0{"passed"}else{"failed"},"cases":results,"firstFailure":first_failure,"timing":{"totalMs":started.elapsed().as_secs_f64()*1000.}});
        if let Some(directory) = job["output"].as_str() {
            let output = artifact_directory(&base.join(directory), &protected)?;
            write(
                &output.join("matrix.json"),
                &serde_json::to_vec_pretty(&summary)?,
            )?;
        }
        Ok((summary, code))
    }
}
