use crate::{
    arithmetic,
    compiler::{compile_cancellable, Library},
    error::{integer, NcError, Result, UNASSIGNED_VARIABLE},
    geometry::{arc_points, distance, interpolate, Arc as Circle, Point},
};
use chrono::{DateTime, Datelike, Timelike, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};
mod machine;
mod output;
mod probe;
const AXES: [char; 6] = ['X', 'Y', 'Z', 'A', 'B', 'C'];
type Words = BTreeMap<char, f64>;
#[derive(Clone, Debug, Serialize)]
pub struct Segment {
    pub start: Point,
    pub end: Point,
    pub rapid: bool,
    pub line: usize,
    pub step: usize,
    pub tool: u32,
    pub movement: usize,
    pub start_rotation: Point,
    pub rotation: Point,
    pub tool_diameter: Option<f64>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Movement {
    pub start: Point,
    pub end: Point,
    pub start_axes: [f64; 6],
    pub end_axes: [f64; 6],
    pub rapid: bool,
    pub line: usize,
    pub step: usize,
    pub tool: u32,
    pub feed: f64,
    pub length: f64,
    pub seconds: f64,
    pub start_time: f64,
    pub segment_start: usize,
    pub segment_end: usize,
}
#[derive(Clone, Debug, Serialize)]
pub struct State {
    pub source: String,
    pub line: usize,
    pub step: usize,
    pub sequence: Option<u32>,
    pub program: Option<u32>,
    pub position: Point,
    pub part_position: Option<[f64; 6]>,
    pub dro_angle: f64,
    pub rotation: Point,
    pub units: &'static str,
    pub skipped: bool,
    pub stack: Vec<Option<u32>>,
    pub axes: [f64; 6],
    pub feed: f64,
    pub tool: u32,
    pub plane: u32,
    pub motion: f64,
    pub elapsed: f64,
    pub segment_count: usize,
    pub variables: Arc<BTreeMap<u32, Option<f64>>>,
    pub spindle: [f64; 2],
}
#[derive(Clone, Debug, Serialize)]
pub struct Diagnostic {
    pub source: String,
    pub line: usize,
    pub step: usize,
    #[serde(flatten)]
    pub error: NcError,
}
#[derive(Clone, Debug, Serialize, Default)]
pub struct Model {
    pub status: String,
    pub executions: Vec<State>,
    pub moves: Vec<Movement>,
    pub segments: Vec<Segment>,
    pub diagnostics: Vec<Diagnostic>,
    pub requirements: Vec<String>,
    pub warnings: Vec<Diagnostic>,
    pub events: Vec<Value>,
    pub variable_numbers: BTreeSet<u32>,
    pub bounds: Option<(Point, Point)>,
}
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Options {
    pub setup: Value,
    pub capture_variables: bool,
    #[serde(skip)]
    pub cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
    pub until: Option<Target>,
    pub time_limit_ms: Option<u64>,
    pub assignment_overrides: Vec<AssignmentOverride>,
}
#[derive(Clone, Debug, Deserialize)]
pub struct Target {
    pub line: Option<usize>,
    pub step: Option<usize>,
    pub sequence: Option<u32>,
    pub program: Option<u32>,
    pub source: Option<String>,
    pub visit: Option<usize>,
}
impl Target {
    pub fn validate(&self) -> Result<()> {
        if [
            self.line.is_some(),
            self.step.is_some(),
            self.sequence.is_some(),
        ]
        .into_iter()
        .filter(|present| *present)
        .count()
            != 1
            || self.line == Some(0)
            || self.step == Some(0)
            || self.visit == Some(0)
        {
            return Err(NcError::new("Checkpoint requires one positive line or step, or a sequence, and a positive visit"));
        }
        Ok(())
    }
    pub fn matches(&self, state: &State) -> bool {
        self.step.is_none_or(|n| n == state.step)
            && self.line.is_none_or(|n| n == state.line)
            && self.sequence.is_none_or(|n| Some(n) == state.sequence)
            && self.program.is_none_or(|n| Some(n) == state.program)
            && self.source.as_ref().is_none_or(|s| s == &state.source)
    }
}
#[derive(Clone, Debug, Deserialize)]
pub struct AssignmentOverride {
    pub source: String,
    pub line: usize,
    pub variable: u32,
    pub value: f64,
    pub visit: Option<usize>,
}
struct Frame {
    program: usize,
    pc: usize,
    remaining: u32,
    start: usize,
    macro_call: bool,
    arguments: BTreeMap<u32, Option<f64>>,
    modal: Option<Words>,
}
#[derive(Clone)]
struct Cycle {
    code: u32,
    initial: f64,
    words: Words,
}
pub fn validate_setup(setup: Value) -> Result<()> {
    Engine::new(&Options {
        setup,
        ..Options::default()
    })
    .map(|_| ())
}

pub fn inspect(
    text: &str,
    filename: &str,
    libraries: &BTreeMap<String, String>,
    options: &Options,
) -> Result<Model> {
    let library = compile_cancellable(text, filename, libraries, options.cancel.as_deref())?;
    inspect_compiled(&library, filename, options)
}
pub fn inspect_compiled(library: &Library, filename: &str, options: &Options) -> Result<Model> {
    if let Some(target) = &options.until {
        target.validate()?;
        if target
            .source
            .as_ref()
            .is_some_and(|s| !library.sources.contains_key(s))
        {
            return Err(NcError::new("until source is not in the loaded program"));
        }
    }
    let mut engine = Engine::new(options)?;
    engine.model.variable_numbers = library.variable_numbers.clone();
    let main = *library
        .sources
        .get(filename)
        .ok_or_else(|| NcError::new("Main source is missing"))?;
    engine.frames.push(Frame {
        program: main,
        pc: 0,
        remaining: 1,
        start: 0,
        macro_call: false,
        arguments: BTreeMap::new(),
        modal: None,
    });
    engine
        .model
        .variable_numbers
        .extend(engine.common.keys().copied());
    engine.run(library, options);
    if !options.capture_variables {
        let mut variables = engine.common.clone();
        variables.extend(engine.system_reads.clone());
        if let Some(locals) = engine.locals.last() {
            variables.extend(locals.clone());
        }
        if let Some(last) = engine.model.executions.last_mut() {
            last.variables = Arc::new(variables);
        }
    }
    Ok(engine.model)
}
struct Engine {
    setup: Value,
    model: Model,
    frames: Vec<Frame>,
    locals: Vec<BTreeMap<u32, Option<f64>>>,
    common: BTreeMap<u32, Option<f64>>,
    names: BTreeMap<String, u32>,
    unassigned_reads: BTreeSet<(String, usize, u32)>,
    snapshot: Arc<BTreeMap<u32, Option<f64>>>,
    dirty: bool,
    position: [f64; 6],
    command: [f64; 6],
    scale: [f64; 6],
    local: [f64; 6],
    shift: [f64; 6],
    work_shift: [f64; 6],
    head_shift: [f64; 6],
    rotation: Option<[f64; 3]>,
    work: String,
    unit: f64,
    initial_unit: f64,
    incremental: bool,
    absolute_center: bool,
    plane: u32,
    motion: u32,
    motion_variant: Option<f64>,
    feed: f64,
    feed_mode: u32,
    tool: u32,
    selected_tool: u32,
    length_mode: u32,
    length_index: u32,
    spindle: [f64; 2],
    return_initial: bool,
    cycle: Option<Cycle>,
    polar: bool,
    compensation: u32,
    data_mode: Option<u32>,
    elapsed: f64,
    timers: [f64; 2],
    line: usize,
    step: usize,
    source: String,
    first: bool,
    max_steps: usize,
    max_segments: usize,
    max_depth: usize,
    override_visits: Vec<usize>,
    modal: BTreeMap<u32, f64>,
    buffered: Words,
    probe_reads: BTreeMap<u32, usize>,
    probe_part: Option<[f64; 6]>,
    probe_machine: Option<[f64; 6]>,
    block_text: String,
    comment: String,
    pending_outputs: Vec<(u32, u32)>,
    m_words: Option<Words>,
    clock: Option<DateTime<Utc>>,
    system_reads: BTreeMap<u32, Option<f64>>,
    initial_variables: BTreeSet<u32>,
    control_flags: u32,
}
fn number(value: &Value, default: f64) -> f64 {
    value.as_f64().unwrap_or(default)
}
fn pose(value: &Value) -> Result<[f64; 6]> {
    if !value.is_null() && !value.is_array() {
        return Err(NcError::new("A pose must be an array"));
    }
    let mut result = [0.; 6];
    if let Some(values) = value.as_array() {
        if values.len() > 6 {
            return Err(NcError::new("A pose has at most six axes"));
        }
        for (i, v) in values.iter().enumerate() {
            result[i] = v
                .as_f64()
                .filter(|v| v.is_finite())
                .ok_or_else(|| NcError::new("Pose coordinates must be finite numbers"))?
        }
    }
    Ok(result)
}
impl Engine {
    fn new(options: &Options) -> Result<Self> {
        let mut setup = options.setup.clone();
        if setup.is_null() {
            setup = json!({})
        }
        if !setup.is_object() {
            return Err(NcError::new("Machine setup must be a JSON object"));
        }
        for key in [
            "tools",
            "inputs",
            "outputs",
            "variables",
            "machineSettings",
            "systemVariables",
            "mCodeOverrides",
            "referencePositions",
            "probes",
        ] {
            if setup[key].is_null() {
                setup[key] = json!({});
            }
            if !setup[key].is_object() {
                return Err(NcError::new(format!("{key} must be an object")));
            }
        }
        for key in ["tools", "probes", "mCodeOverrides"] {
            for value in setup[key].as_object().unwrap().values() {
                if !value.is_object() {
                    return Err(NcError::new(format!("Entries in {key} must be objects")));
                }
            }
        }
        for key in [
            "rapidRate",
            "initialFeed",
            "instructionTimeMs",
            "maxExecutions",
            "maxSegments",
            "maxCallDepth",
        ] {
            if !setup[key].is_null() && setup[key].as_f64().is_none_or(|v| !v.is_finite() || v < 0.)
            {
                return Err(NcError::new(format!(
                    "{key} must be a finite nonnegative number"
                )));
            }
        }
        let nominal = json!({"parameters":{"1800":0,"1804":0.00001,"1806":0,"1808":1,"5003":3,"5114":250,"5115":250},"workOffsets":{"54":[0,0,0,0,0,0]},"axisRoles":{"A":"linear","B":"disabled","C":"disabled"}});
        for key in ["parameters", "workOffsets", "axisRoles"] {
            let mut merged = nominal[key].as_object().cloned().unwrap_or_default();
            if let Some(values) = setup[key].as_object() {
                merged.extend(values.clone())
            } else if !setup[key].is_null() {
                return Err(NcError::new(format!("{key} must be an object")));
            }
            setup[key] = Value::Object(merged);
        }
        let clock = setup
            .get("clock")
            .map(|value| {
                value
                    .as_str()
                    .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
                    .map(|date| date.to_utc())
                    .ok_or_else(|| {
                        NcError::new("clock must be an ISO date/time with an explicit timezone")
                    })
            })
            .transpose()?;
        if setup.get("units").is_some_and(|v| v != "mm" && v != "inch") {
            return Err(NcError::new("Machine units must be mm or inch"));
        }
        if setup.get("optionalStop").is_some_and(|v| !v.is_boolean())
            || setup.get("blockDelete").is_some_and(|v| {
                v.as_array()
                    .is_none_or(|a| a.iter().any(|n| n.as_u64().is_none_or(|n| n > 9)))
            })
        {
            return Err(NcError::new(
                "optionalStop must be boolean and blockDelete must list numbers 0 to 9",
            ));
        }
        if setup
            .get("dialect")
            .is_some_and(|v| v != "machpro-2026.7.14.2")
        {
            return Err(NcError::new(
                "The implemented dialect is machpro-2026.7.14.2",
            ));
        }
        for (axis, role) in setup["axisRoles"].as_object().unwrap() {
            if !["A", "B", "C"].contains(&axis.as_str())
                || !matches!(
                    role.as_str(),
                    Some("linear" | "rotary" | "auxiliary" | "disabled")
                )
            {
                return Err(NcError::new(
                    "axisRoles must describe A/B/C as linear, rotary, auxiliary or disabled",
                ));
            }
        }
        let position = pose(&setup["initialPosition"])?;
        if position.iter().any(|v| v.abs() > 1e9) {
            return Err(NcError::new("Initial coordinates exceed the numeric range"));
        }
        for key in ["referencePositions", "workOffsets"] {
            if let Some(offsets) = setup[key].as_object() {
                for value in offsets.values() {
                    pose(value)?;
                }
            }
        }

        let unit = if setup["units"] == "inch" { 25.4 } else { 1. };
        let mut common = BTreeMap::new();
        if let Some(values) = setup["variables"].as_object() {
            for (k, v) in values {
                let n = k
                    .parse()
                    .map_err(|_| NcError::new("Invalid initial variable address"))?;
                if !v.is_null() && v.as_f64().is_none() {
                    return Err(NcError::new("Variables must be finite numbers or NIL"));
                }
                common.insert(n, v.as_f64());
            }
        }
        let max_steps = integer(
            number(&setup["maxExecutions"], 250000.),
            "maxExecutions",
            1,
            10000000,
        )? as usize;
        let max_segments = integer(
            number(&setup["maxSegments"], 2000000.),
            "maxSegments",
            1,
            20000000,
        )? as usize;
        let max_depth =
            integer(number(&setup["maxCallDepth"], 32.), "maxCallDepth", 1, 256)? as usize;
        let feed = number(&setup["initialFeed"], 0.) * unit;
        let initial_variables = common.keys().copied().collect();
        let initial_locals = common.range(1..=33).map(|(k, v)| (*k, *v)).collect();
        common.retain(|number, _| *number > 33);
        let mut engine = Self {
            setup,
            model: Model {
                status: "complete".into(),
                ..Model::default()
            },
            frames: Vec::new(),
            locals: vec![initial_locals],
            common,
            names: BTreeMap::new(),
            unassigned_reads: BTreeSet::new(),
            snapshot: Arc::new(BTreeMap::new()),
            dirty: true,
            position,
            command: position,
            scale: [1.; 6],
            local: [0.; 6],
            shift: [0.; 6],
            work_shift: [0.; 6],
            head_shift: [0.; 6],
            rotation: None,
            work: "54".into(),
            unit,
            initial_unit: unit,
            incremental: false,
            absolute_center: false,
            plane: 17,
            motion: 0,
            motion_variant: None,
            feed,
            feed_mode: 94,
            tool: 0,
            selected_tool: 0,
            length_mode: 49,
            length_index: 0,
            spindle: [0.; 2],
            return_initial: true,
            cycle: None,
            polar: false,
            compensation: 40,
            data_mode: None,
            elapsed: 0.,
            timers: [0.; 2],
            line: 0,
            step: 0,
            source: String::new(),
            first: true,
            max_steps,
            max_segments,
            max_depth,
            override_visits: vec![0; options.assignment_overrides.len()],
            modal: [
                0.,
                17.,
                90.,
                91.1,
                94.,
                if unit == 1. { 21. } else { 20. },
                40.,
                49.,
                80.,
                98.,
                50.,
                67.,
                97.,
                54.,
                64.,
                69.,
                15.,
                40.1,
            ]
            .into_iter()
            .enumerate()
            .map(|(i, g)| (i as u32 + 1, g))
            .collect(),
            buffered: BTreeMap::new(),
            probe_reads: BTreeMap::new(),
            probe_part: None,
            probe_machine: None,
            block_text: String::new(),
            comment: String::new(),
            pending_outputs: Vec::new(),
            m_words: None,
            clock,
            system_reads: BTreeMap::new(),
            initial_variables,
            control_flags: 0,
        };
        engine.command = engine.transform(position, true)?;
        Ok(engine)
    }
    fn param(&self, id: u32, default: f64) -> f64 {
        number(&self.setup["parameters"][id.to_string()], default)
    }
    fn axis_scale(&self, axis: usize) -> f64 {
        if axis < 3 || self.setup["axisRoles"][AXES[axis].to_string()] == "linear" {
            self.unit
        } else {
            1.
        }
    }
    fn offset(&self) -> Result<[f64; 6]> {
        let value = &self.setup["workOffsets"][&self.work];
        if value.is_null() {
            return Err(NcError::required(format!("workOffsets.{}", self.work)));
        }
        let mut result = pose(value)?;
        for (i, item) in result.iter_mut().enumerate() {
            *item += self.local[i] + self.shift[i] + self.work_shift[i] + self.head_shift[i]
        }
        Ok(result)
    }
    fn transform(&self, point: [f64; 6], inverse: bool) -> Result<[f64; 6]> {
        let offset = self.offset()?;
        let mut p = point;
        for i in 0..6 {
            p[i] = if inverse {
                p[i] - offset[i]
            } else {
                p[i] * self.scale[i]
            }
        }
        if let Some([x, y, angle]) = self.rotation {
            let a = (angle * std::f64::consts::PI / 180.) * if inverse { -1. } else { 1. };
            let dx = p[0] - x;
            let dy = p[1] - y;
            p[0] = x + dx * a.cos() - dy * a.sin();
            p[1] = y + dx * a.sin() + dy * a.cos()
        }
        for i in 0..6 {
            p[i] = if inverse {
                p[i] / self.scale[i]
            } else {
                p[i] + offset[i]
            }
        }
        Ok(p)
    }
    fn part_position(&self) -> Result<[f64; 6]> {
        let offset = self.offset()?;
        let mut part: [f64; 6] = std::array::from_fn(|axis| self.position[axis] - offset[axis]);
        if let Some([x, y, angle]) = self.rotation.filter(|_| self.param(1800, 0.) != 0.) {
            let (sin, cos) = (-angle).to_radians().sin_cos();
            let (dx, dy) = (part[0] - x, part[1] - y);
            part[0] = x + dx * cos - dy * sin;
            part[1] = y + dx * sin + dy * cos;
        }
        Ok(part)
    }
    fn resync(&mut self) -> Result<()> {
        self.command = self.transform(self.position, true)?;
        Ok(())
    }
    fn variable(&mut self, address: u32) -> Result<Option<f64>> {
        let value = self.read_variable(address)?;
        if address >= 1000 && self.system_reads.get(&address) != Some(&value) {
            self.system_reads.insert(address, value);
            self.dirty = true;
        }
        Ok(value)
    }
    fn read_variable(&mut self, address: u32) -> Result<Option<f64>> {
        self.model.variable_numbers.insert(address);
        if address == 0 || address == 3100 {
            return Ok(None);
        }
        if address == 3101 {
            return Ok(Some(std::f64::consts::PI));
        }
        if address == 3102 {
            return Ok(Some(std::f64::consts::E));
        }
        if address <= 33 {
            return Ok(self
                .locals
                .last()
                .and_then(|v| v.get(&address).copied())
                .flatten());
        }
        if (100..=199).contains(&address) || (500..=999).contains(&address) {
            return Ok(self.common.get(&address).copied().flatten());
        }
        if address == 1229 {
            return Ok(Some(self.tool as f64));
        }
        if address == 1242 {
            return Ok(Some(self.spindle[1]));
        }
        if address == 3004 {
            return Ok(Some(self.control_flags as f64));
        }
        if address == 3005 {
            return Ok(Some(if self.unit == 1. { 1. } else { 0. }));
        }
        if address == 3007 {
            return Ok(Some(
                self.scale[..3]
                    .iter()
                    .enumerate()
                    .fold(0, |bits, (i, v)| bits | if *v < 0. { 1 << i } else { 0 })
                    as f64,
            ));
        }
        for (base, field) in [
            (10000, "lengthWear"),
            (11000, "length"),
            (12000, "diameterWear"),
            (13000, "diameter"),
        ] {
            if address > base && address <= base + 999 {
                let tool = address - base;
                return self.setup["tools"][tool.to_string()][field]
                    .as_f64()
                    .map(|v| Some(v / self.unit))
                    .ok_or_else(|| NcError::required(format!("tools.{tool}.{field}")));
            }
        }
        for (base, value) in [(5061, self.probe_part), (5071, self.probe_machine)] {
            if (base..base + 6).contains(&address) {
                let i = (address - base) as usize;
                return value
                    .map(|p| Some(p[i] / self.axis_scale(i)))
                    .ok_or_else(|| NcError::required("probeMeasurement"));
            }
        }
        if address == 1032 || address == 1132 {
            let key = if address == 1032 { "inputs" } else { "outputs" };
            let mut bits = 0u32;
            for id in 0..32 {
                let v = self.setup[key][id.to_string()].clone();
                if v.is_null() {
                    return Err(NcError::required(format!("{key}.{id}")));
                }
                if v == true || v.as_f64().is_some_and(|v| v != 0.) {
                    bits |= 1 << id;
                }
            }
            return Ok(Some(bits as f64));
        }
        let buffered = match address {
            4102 => Some('B'),
            4107 => Some('D'),
            4108 => Some('E'),
            4109 => Some('F'),
            4111 => Some('H'),
            4113 => Some('M'),
            4114 => Some('N'),
            4115 => Some('O'),
            4119 => Some('S'),
            4120 => Some('T'),
            _ => None,
        };
        if let Some(letter) = buffered {
            return Ok(self.buffered.get(&letter).copied());
        }
        if address == 3001 {
            return Ok(Some((self.elapsed - self.timers[0]) % 2147483648.));
        }
        if address == 3002 {
            return Ok(Some((self.elapsed - self.timers[1]) / 3600000.));
        }
        if matches!(address, 3011 | 3012) {
            let clock = self.clock.ok_or_else(|| NcError::required("clock"))?;
            let instant = clock
                .timestamp_millis()
                .checked_add(self.elapsed.trunc() as i64)
                .and_then(DateTime::from_timestamp_millis)
                .ok_or_else(|| {
                    NcError::new("Program clock exceeds the supported date range").code("LIMIT")
                })?;
            let value = if address == 3011 {
                i64::from(instant.year()) * 10000 + i64::from(instant.month() * 100 + instant.day())
            } else {
                i64::from(instant.hour() * 10000 + instant.minute() * 100 + instant.second())
            };
            return Ok(Some(value as f64));
        }
        if (4001..=4018).contains(&address) {
            return Ok(Some(*self.modal.get(&(address - 4000)).unwrap_or(&0.)));
        }
        for (base, pose) in [
            (5001, self.command),
            (5021, {
                let mut p = self.position;
                p[2] += self.length()?;
                p
            }),
            (5030, self.offset()?),
            (4140, pose(&self.setup["workOffsets"]["54"])?),
            (5157, self.command),
            (5041, self.part_position()?),
            (5050, self.shift),
            (5081, self.head_shift),
            (5201, self.work_shift),
        ] {
            if (base..base + 6).contains(&address) {
                let i = (address - base) as usize;
                return Ok(Some(pose[i] / self.axis_scale(i)));
            }
        }
        for i in 0..6 {
            let base = 5221 + i * 20;
            if (base..base + 6).contains(&address) {
                let value = &self.setup["workOffsets"][(54 + i).to_string()];
                if value.is_null() {
                    return Err(NcError::required(format!("workOffsets.{}", 54 + i)));
                }
                let axis = (address - base) as usize;
                return Ok(Some(pose(value)?[axis] / self.axis_scale(axis)));
            }
        }
        if (7001..10000).contains(&address) && (address - 7001) % 20 < 6 {
            let index = (address - 7001) / 20 + 1;
            let key = format!("54.1:{index}");
            let value = &self.setup["workOffsets"][&key];
            if value.is_null() {
                return Err(NcError::required(format!("workOffsets.{key}")));
            }
            let axis = ((address - 7001) % 20) as usize;
            return Ok(Some(pose(value)?[axis] / self.axis_scale(axis)));
        }
        for (base, key) in [
            (5181, "30:1"),
            (5351, "30:2"),
            (5361, "30:3"),
            (5371, "30:4"),
        ] {
            if (base..base + 6).contains(&address) {
                let value = &self.setup["referencePositions"][key];
                if value.is_null() {
                    return Err(NcError::required(format!("referencePositions.{key}")));
                }
                let axis = (address - base) as usize;
                return Ok(Some(pose(value)?[axis] / self.axis_scale(axis)));
            }
        }
        if (1000..1032).contains(&address) || (1100..1132).contains(&address) {
            let input = address < 1100;
            let key = if input { "inputs" } else { "outputs" };
            let id = address - if input { 1000 } else { 1100 };
            let value = &self.setup[key][id.to_string()];
            if value.is_null() {
                return Err(NcError::required(format!("{key}.{id}")));
            }
            return Ok(Some(value.as_f64().unwrap_or(
                if value.as_bool().unwrap_or(false) {
                    1.
                } else {
                    0.
                },
            )));
        }
        self.setup["systemVariables"][address.to_string()]
            .as_f64()
            .map(Some)
            .ok_or_else(|| NcError::required(format!("systemVariables.{address}")))
    }
    fn set_variable(&mut self, address: u32, value: Option<f64>) -> Result<()> {
        if value.is_some_and(|v| !v.is_finite()) {
            return Err(NcError::new("Variable assignment must be finite or NIL"));
        }
        if [0, 3100, 3101, 3102].contains(&address) {
            return Err(NcError::new(format!("#{address} is read-only.")));
        }
        self.model.variable_numbers.insert(address);
        self.dirty = true;
        if address <= 33 {
            if let Some(locals) = self.locals.last_mut() {
                locals.insert(address, value);
            }
            return Ok(());
        }
        if (100..=199).contains(&address) || (500..=999).contains(&address) {
            self.common.insert(address, value);
            return Ok(());
        }
        let value = value.ok_or_else(|| NcError::new("System variables cannot be NIL"))?;
        for (base, field) in [
            (10000, "lengthWear"),
            (11000, "length"),
            (12000, "diameterWear"),
            (13000, "diameter"),
        ] {
            if address > base && address <= base + 999 {
                self.setup["tools"][(address - base).to_string()][field] = json!(value * self.unit);
                return Ok(());
            }
        }
        if (4001..=4018).contains(&address) {
            let code = value / 10.;
            if machine::group(code) != address - 4000 {
                return Err(NcError::new(
                    "Modal system variable does not match its G-code group",
                ));
            }
            return self.execute(&Words::new(), &[code], &[]);
        }
        if address == 3004 {
            self.control_flags = integer(value, "Control flags", 0, 999)?;
            return Ok(());
        }
        match address {
            3000 => {
                let number = integer(value, "Alarm number", 0, 999999)?;
                let message = if self.comment.is_empty() {
                    format!("Program alarm {number}")
                } else {
                    format!("Program alarm {number} · {}", self.comment)
                };
                let mut error = NcError::new(message).code("ALARM");
                error.alarm = Some(number);
                return Err(error);
            }
            3006 => {
                let number = integer(value, "Stop number", 0, 99999999)?;
                self.event(
                    "program-stop",
                    json!({"number":number,"message":self.comment,"optional":false}),
                );
                return Ok(());
            }
            3001 => {
                self.timers[0] = self.elapsed - value;
                return Ok(());
            }
            3002 => {
                if !(value * 3600000.).is_finite() {
                    return Err(NcError::new("Timer value exceeds numeric range"));
                }
                self.timers[1] = self.elapsed - value * 3600000.;
                return Ok(());
            }
            _ => {}
        }
        for i in 0..6 {
            let base = 5221 + i * 20;
            if (base..base + 6).contains(&address) {
                let axis = (address - base) as usize;
                let mut offset = pose(&self.setup["workOffsets"][(54 + i).to_string()])?;
                offset[axis] = value * self.axis_scale(axis);
                self.setup["workOffsets"][(54 + i).to_string()] = json!(offset);
                return self.resync();
            }
        }
        for (base, kind) in [(5050, 0), (5081, 1), (5201, 2)] {
            if (base..base + 6).contains(&address) {
                let axis = (address - base) as usize;
                let next = value * self.axis_scale(axis);
                match kind {
                    0 => self.shift[axis] = next,
                    1 => self.head_shift[axis] = next,
                    _ => self.work_shift[axis] = next,
                }
                return self.resync();
            }
        }
        if (7001..10000).contains(&address) && (address - 7001) % 20 < 6 {
            let key = format!("54.1:{}", (address - 7001) / 20 + 1);
            let axis = ((address - 7001) % 20) as usize;
            let mut offset = pose(&self.setup["workOffsets"][&key])?;
            offset[axis] = value * self.axis_scale(axis);
            self.setup["workOffsets"][&key] = json!(offset);
            return self.resync();
        }
        for (base, key) in [
            (5181, "30:1"),
            (5351, "30:2"),
            (5361, "30:3"),
            (5371, "30:4"),
        ] {
            if (base..base + 6).contains(&address) {
                let axis = (address - base) as usize;
                let mut point = pose(&self.setup["referencePositions"][key])?;
                point[axis] = value * self.axis_scale(axis);
                self.setup["referencePositions"][key] = json!(point);
                return self.resync();
            }
        }
        if address == 1132 {
            let bits = integer(value, "Output bit mask", 0, u32::MAX)?;
            for id in 0..32 {
                self.setup["outputs"][id.to_string()] = json!((bits >> id) & 1);
            }
            self.event("output-mask", json!({"mask":bits}));
            return Ok(());
        }
        if address == 3005 {
            if value != 0. && value != 1. {
                return Err(NcError::new("#3005 is 0 for inch or 1 for metric"));
            }
            self.unit = if value == 1. { 1. } else { 25.4 };
            self.modal.insert(6, if value == 1. { 21. } else { 20. });
            return Ok(());
        }
        if (1100..1132).contains(&address) {
            if self.setup["outputs"].is_null() {
                self.setup["outputs"] = json!({})
            }
            self.setup["outputs"][(address - 1100).to_string()] = json!(value);
            return Ok(());
        }
        Err(NcError::required(format!("systemVariableWrites.{address}")))
    }
    fn address(&mut self, node: &Value) -> Result<u32> {
        if node["type"] == "named" {
            return self
                .names
                .get(node["name"].as_str().unwrap_or(""))
                .copied()
                .ok_or_else(|| NcError::new("Named variable has not been defined by SETVN"));
        }
        integer(
            self.evaluate(&node["address"])?.unwrap_or(0.),
            "Variable address",
            0,
            99999,
        )
    }
    fn evaluate(&mut self, node: &Value) -> Result<Option<f64>> {
        self.evaluate_operand(node, false)
    }
    fn evaluate_operand(&mut self, node: &Value, nil_test: bool) -> Result<Option<f64>> {
        let kind = node["type"].as_str().unwrap_or("");
        let value = match kind {
            "number" => {
                return node["value"]
                    .as_f64()
                    .map(Some)
                    .ok_or_else(|| NcError::new("Invalid numeric expression"))
            }
            "variable" | "named" => {
                let address = self.address(node)?;
                let value = self.variable(address)?;
                if !nil_test && value.is_none() {
                    let unassigned = match address {
                        1..=33 => !self
                            .locals
                            .last()
                            .is_some_and(|vars| vars.contains_key(&address)),
                        100..=199 | 500..=999 => !self.common.contains_key(&address),
                        _ => false,
                    };
                    if unassigned
                        && self
                            .unassigned_reads
                            .insert((self.source.clone(), self.line, address))
                    {
                        self.model.warnings.push(Diagnostic {
                            source: self.source.clone(),
                            line: self.line,
                            step: self.step,
                            error: NcError::new(format!(
                                "Variable #{address} used before assignment"
                            ))
                            .code(UNASSIGNED_VARIABLE),
                        });
                    }
                }
                return Ok(value);
            }
            "unary" => {
                let v = self.evaluate(&node["operand"])?.unwrap_or(f64::NAN);
                arithmetic::unary(
                    match node["operator"].as_str() {
                        Some("-") => 1,
                        Some("NOT") => 2,
                        _ => 0,
                    },
                    v,
                )
            }
            "chain" => {
                let operations = node["operations"]
                    .as_array()
                    .ok_or_else(|| NcError::new("Invalid expression operation list"))?;
                let first = &node["first"];
                let nil_comparison = operations.first().is_some_and(|operation| {
                    matches!(
                        operation["operator"].as_str(),
                        Some("EQ" | "NE" | "=" | "==" | "<>")
                    )
                });
                let nil_literal = |node: &Value| {
                    node["type"] == "variable"
                        && node["address"]["type"] == "number"
                        && matches!(node["address"]["value"].as_f64(), Some(0. | 3100.))
                };
                let mut accumulator = self
                    .evaluate_operand(
                        first,
                        nil_comparison && nil_literal(&operations[0]["operand"]),
                    )?
                    .unwrap_or(f64::NAN);
                let tolerance = self.param(1804, 0.00001);
                for (index, operation) in operations.iter().enumerate() {
                    let operand = self
                        .evaluate_operand(
                            &operation["operand"],
                            index == 0 && nil_comparison && nil_literal(first),
                        )?
                        .unwrap_or(f64::NAN);
                    let operator = operation["operator"].as_str().unwrap_or("");
                    let op = match operator {
                        "+" => 0,
                        "-" => 1,
                        "*" => 2,
                        "/" => 3,
                        "MOD" => 4,
                        "**" => 5,
                        "AND" => 6,
                        "OR" => 7,
                        "XOR" => 8,
                        "EQ" | "=" | "==" => 9,
                        "NE" | "<>" => 10,
                        "LT" | "<" => 11,
                        "LE" | "<=" => 12,
                        "GT" | ">" => 13,
                        "GE" | ">=" => 14,
                        _ => return Err(NcError::new("Unknown binary operator")),
                    };
                    accumulator = arithmetic::binary(op, accumulator, operand, tolerance);
                    if !accumulator.is_finite() {
                        let message = match operator {
                            "/" if operand == 0. || operand.is_nan() => {
                                "Division by zero".to_owned()
                            }
                            "MOD" if operand == 0. || operand.is_nan() => "MOD by zero".to_owned(),
                            _ => format!(
                                "Operator {operator} produced an invalid or out-of-range value"
                            ),
                        };
                        return Err(NcError::new(message).code("ARITHMETIC"));
                    }
                }
                accumulator
            }
            "function" => {
                let args = node["args"]
                    .as_array()
                    .ok_or_else(|| NcError::new("Missing function arguments"))?;
                let name = node["name"].as_str().unwrap_or("");
                if args.is_empty()
                    || args.len() > 2
                    || (!["ATAN", "PRM"].contains(&name) && args.len() != 1)
                {
                    return Err(NcError::new(format!(
                        "Wrong number of arguments for {name}"
                    )));
                }
                let first = self.evaluate(&args[0])?.unwrap_or(0.);
                let second = if args.len() == 2 {
                    Some(self.evaluate(&args[1])?.unwrap_or(0.))
                } else {
                    None
                };
                if name == "PRM" {
                    let id = integer(first, "Parameter number", 0, 99999999)?;
                    let value = self.setup["parameters"][id.to_string()]
                        .as_f64()
                        .ok_or_else(|| NcError::required(format!("parameters.{id}")))?;
                    return Ok(Some(if let Some(bit) = second {
                        let bit = integer(bit, "Parameter bit", 0, 31)?;
                        ((value as u32 >> bit) & 1) as f64
                    } else {
                        value
                    }));
                }
                let op = match name {
                    "ABS" => 0,
                    "ROUND" => 1,
                    "FIX" => 2,
                    "FUP" => 3,
                    "SIN" => 4,
                    "COS" => 5,
                    "TAN" => 6,
                    "ASIN" => 7,
                    "ACOS" => 8,
                    "ATAN" => 9,
                    "SQRT" => 10,
                    "LN" => 11,
                    "EXP" => 12,
                    "BIN" => 13,
                    "BCD" => 14,
                    _ => return Err(NcError::new(format!("Unknown function {name}"))),
                };
                let value =
                    arithmetic::function(op, first, second.unwrap_or(0.), args.len() as u32);
                if !value.is_finite() {
                    let message = match name {
                        "SQRT" => "SQRT needs a nonnegative value".into(),
                        "LN" => "LN needs a positive value".into(),
                        "ASIN" | "ACOS" => format!("{name} needs a value from -1 to 1"),
                        "BIN" => "BIN needs a valid packed BCD value".into(),
                        "BCD" => "BCD needs an integer from 0 to 99999999".into(),
                        _ => format!("{name} produced an out-of-range value"),
                    };
                    return Err(NcError::new(message).code("ARITHMETIC"));
                }
                value
            }
            _ => return Err(NcError::new("Invalid expression syntax tree")),
        };
        if !value.is_finite() {
            return Err(NcError::new(
                "Mathematical result is outside its domain or numeric range.",
            )
            .code("ARITHMETIC"));
        }
        Ok(Some(if value == 0. { 0. } else { value }))
    }
    fn add_time(&mut self, milliseconds: f64) -> Result<()> {
        let end = self.elapsed + milliseconds;
        if !milliseconds.is_finite()
            || milliseconds < 0.
            || !end.is_finite()
            || end > 9_007_199_254_740_991.
        {
            return Err(NcError::new("Program duration exceeds the numeric range").code("LIMIT"));
        }
        self.elapsed = end;
        Ok(())
    }
    fn event(&mut self, kind: &str, details: Value) {
        self.model.events.push(json!({"type":kind,"source":self.source,"line":self.line,"step":self.step,"time":self.elapsed,"details":details}))
    }
    fn items(
        &mut self,
        items: &[Value],
        words: &mut Words,
        codes: &mut Vec<f64>,
        flow: &mut Option<(bool, usize)>,
        options: &Options,
    ) -> Result<()> {
        for item in items {
            if flow.is_some() {
                return Err(NcError::new(
                    "A control-flow command must be the final statement in its block",
                ));
            }
            match item["type"].as_str().unwrap_or("") {
                "word" => {
                    if let Some(value) = self.evaluate(&item["value"])? {
                        let letter = item["letter"]
                            .as_str()
                            .and_then(|s| s.chars().next())
                            .ok_or_else(|| NcError::new("Invalid address letter"))?;
                        if letter == 'G' {
                            codes.push(value)
                        } else if letter != 'M' && self.m_words.is_some() {
                            if self
                                .m_words
                                .as_mut()
                                .unwrap()
                                .insert(letter, value)
                                .is_some()
                            {
                                return Err(NcError::new(format!("Address {letter} appears more than once in an M-code argument list")));
                            }
                        } else if words.insert(letter, value).is_some() {
                            return Err(NcError::new(format!(
                                "Address {letter} appears more than once in a block"
                            )));
                        }
                        if letter == 'M' && value >= 200. {
                            self.m_words = Some(Words::new());
                        }
                    }
                }
                "assign" => {
                    let value = self.evaluate(&item["value"])?;
                    let address = self.address(&item["target"])?;
                    let mut effective = value;
                    for (i, rule) in options.assignment_overrides.iter().enumerate() {
                        if rule.source == self.source
                            && rule.line == self.line
                            && rule.variable == address
                        {
                            self.override_visits[i] += 1;
                            if self.override_visits[i] == rule.visit.unwrap_or(1) {
                                if !rule.value.is_finite() {
                                    return Err(NcError::new(
                                        "Assignment overrides must be finite",
                                    ));
                                }
                                effective = Some(rule.value);
                                self.event("assignment-override",json!({"variable":address,"original":value,"value":effective,"override":i}));
                            }
                        }
                    }
                    self.set_variable(address, effective)?;
                    if self.initial_variables.contains(&address) {
                        self.event("initial-variable-overwritten",json!({"variable":address,"initial":self.setup["variables"][address.to_string()],"value":effective}));
                    }
                }
                "if" => {
                    if self.evaluate(&item["condition"])?.unwrap_or(0.) != 0. {
                        self.items(
                            item["consequent"]
                                .as_array()
                                .ok_or_else(|| NcError::new("Missing IF consequent"))?,
                            words,
                            codes,
                            flow,
                            options,
                        )?;
                    }
                }
                "goto" => {
                    let value = self.evaluate(&item["target"])?.unwrap_or(0.);
                    *flow = Some((
                        true,
                        integer(value, "GOTO sequence number", 0, 99999999)? as usize,
                    ));
                }
                "while" => {
                    if self.evaluate(&item["condition"])?.unwrap_or(0.) == 0. {
                        *flow = Some((
                            false,
                            item["end"]
                                .as_u64()
                                .ok_or_else(|| NcError::new("WHILE has no matching END"))?
                                as usize
                                + 1,
                        ));
                    }
                }
                "end" => {
                    *flow = Some((
                        false,
                        item["start"]
                            .as_u64()
                            .ok_or_else(|| NcError::new("END has no matching WHILE"))?
                            as usize,
                    ));
                }
                "names" => {
                    let start = item["start"].as_f64().unwrap_or(500.) as u32;
                    let names = item["names"]
                        .as_array()
                        .ok_or_else(|| NcError::new("Invalid SETVN names"))?;
                    if start + names.len() as u32 > 1000 {
                        return Err(NcError::new("SETVN extends beyond variable #999"));
                    }
                    for (i, name) in names.iter().enumerate() {
                        let name = name.as_str().unwrap_or("");
                        if name.is_empty() {
                            return Err(NcError::new("SETVN contains an empty name"));
                        }
                        self.names.insert(name.into(), start + i as u32);
                        self.model.variable_numbers.insert(start + i as u32);
                    }
                }
                "print" => self.print(
                    item["command"].as_str().unwrap_or(""),
                    item["text"].as_str().unwrap_or(""),
                )?,
                _ => return Err(NcError::new("Unsupported statement")),
            }
        }
        Ok(())
    }
    fn run(&mut self, library: &Library, options: &Options) {
        let mut visits = 0usize;
        let started = std::time::Instant::now();
        let mut visited = BTreeSet::new();
        while let Some(frame) = self.frames.last_mut() {
            let program = &library.programs[frame.program];
            if frame.pc >= program.blocks.len() {
                if self.frames.len() > 1 {
                    self.fail(NcError::new("Subprogram reached its end without M99").code("RETURN"))
                }
                break;
            }
            let block = Arc::clone(&program.blocks[frame.pc]);
            frame.pc += 1;
            self.block_text.clone_from(&block.text);
            self.comment.clear();
            if let Some(comments) = block.syntax["comments"].as_array() {
                for comment in comments.iter().filter_map(Value::as_str) {
                    if !self.comment.is_empty() {
                        self.comment.push(' ');
                    }
                    self.comment.push_str(comment.trim());
                }
            }
            self.line = block.line;
            self.source = block.source.clone();
            self.step = self.model.executions.len() + 1;
            if options
                .cancel
                .as_ref()
                .is_some_and(|c| c.load(std::sync::atomic::Ordering::Relaxed))
            {
                self.model.status = "cancelled".into();
                break;
            }
            if options
                .time_limit_ms
                .is_some_and(|limit| started.elapsed().as_millis() >= u128::from(limit))
            {
                self.fail(NcError::new("Execution time limit reached").code("LIMIT"));
                break;
            }
            if self.step > self.max_steps {
                self.fail(
                    NcError::new(format!(
                        "Plot stopped after {} executed blocks",
                        self.max_steps
                    ))
                    .code("LIMIT"),
                );
                break;
            }
            self.m_words = None;
            let mut words = Words::new();
            let mut codes = Vec::new();
            let mut flow = None;
            let skipped = block.syntax["blockDelete"].as_f64().is_some_and(|n| {
                self.setup["blockDelete"]
                    .as_array()
                    .is_some_and(|a| a.iter().any(|v| v.as_f64() == Some(n)))
            });
            let result = (|| -> Result<()> {
                if skipped {
                    return Ok(());
                }
                if let Some(error) = &block.error {
                    visited.insert((block.source.clone(), block.line));
                    return Err(error.clone());
                }
                if let Some(linear) = &block.linear {
                    return self.execute(&linear.words, &linear.code, &[]);
                }
                self.items(
                    block.syntax["items"]
                        .as_array()
                        .ok_or_else(|| NcError::new("Invalid block"))?,
                    &mut words,
                    &mut codes,
                    &mut flow,
                    options,
                )?;
                if let Some((label, target)) = flow {
                    if !codes.is_empty() || words.keys().any(|k| !['N', 'O'].contains(k)) {
                        return Err(NcError::new(
                            "Do not combine branches or loops with motion words",
                        ));
                    }
                    let frame = self
                        .frames
                        .last_mut()
                        .ok_or_else(|| NcError::new("Missing execution frame"))?;
                    frame.pc = if label {
                        *library.programs[frame.program]
                            .labels
                            .get(&(target as u32))
                            .ok_or_else(|| {
                                NcError::new(format!("GOTO target N{target} does not exist"))
                            })?
                    } else {
                        target
                    };
                    return Ok(());
                }
                if codes.iter().any(|c| [65., 66., 67.].contains(c)) {
                    return Ok(());
                }
                let modal_arguments = self.frames.last().and_then(|f| f.modal.as_ref()).is_some()
                    && codes.is_empty()
                    && !words.contains_key(&'M')
                    && words.keys().any(|k| argument(*k).is_some());
                if !modal_arguments {
                    self.execute(
                        &words,
                        &codes,
                        block.syntax["items"]
                            .as_array()
                            .map_or(&[][..], Vec::as_slice),
                    )?
                }
                Ok(())
            })();
            if let Err(error) = result {
                self.fail(error)
            }
            let program_id = self
                .frames
                .last()
                .and_then(|f| library.programs[f.program].id);
            if self.dirty && options.capture_variables {
                let mut vars = self.common.clone();
                vars.extend(self.system_reads.clone());
                if let Some(locals) = self.locals.last() {
                    vars.extend(locals.iter().map(|(k, v)| (*k, *v)))
                }
                self.snapshot = Arc::new(vars);
                self.dirty = false
            }
            self.model.executions.push(State {
                source: self.source.clone(),
                line: self.line,
                step: self.step,
                sequence: block.syntax["label"].as_f64().map(|n| n as u32),
                program: program_id,
                position: [self.position[0], self.position[1], self.position[2]],
                part_position: self.part_position().ok(),
                dro_angle: self
                    .rotation
                    .filter(|_| self.param(1800, 0.) != 0.)
                    .map_or(0., |rotation| rotation[2]),
                rotation: self.rotation_for(self.position),
                units: if self.unit == 1. { "mm" } else { "inch" },
                skipped,
                stack: self
                    .frames
                    .iter()
                    .map(|frame| library.programs[frame.program].id)
                    .collect(),
                axes: self.position,
                feed: self.feed,
                tool: self.tool,
                plane: self.plane,
                motion: self.motion_variant.unwrap_or(f64::from(self.motion)),
                elapsed: self.elapsed,
                segment_count: self.model.segments.len(),
                variables: self.snapshot.clone(),
                spindle: self.spindle,
            });
            if self.model.status != "complete" {
                break;
            }
            if let Some(target) = &options.until {
                let state = self.model.executions.last().unwrap();
                if target.matches(state) {
                    visits += 1;
                    if visits == target.visit.unwrap_or(1) {
                        self.model.status = "paused".into();
                        break;
                    }
                }
            }
            if skipped || flow.is_some() {
                continue;
            }
            let action = (|| -> Result<bool> {
                let macro_code = codes
                    .iter()
                    .rev()
                    .find(|c| [65., 66., 67.].contains(c))
                    .copied();
                let modal_arguments = self.frames.last().and_then(|f| f.modal.as_ref()).is_some()
                    && codes.is_empty()
                    && !words.contains_key(&'M')
                    && words.keys().any(|k| argument(*k).is_some());
                if matches!(macro_code, Some(66. | 67.)) {
                    self.modal.insert(12, macro_code.unwrap());
                }
                if macro_code == Some(67.) {
                    if let Some(f) = self.frames.last_mut() {
                        f.modal = None
                    }
                    return Ok(false);
                }
                if matches!(macro_code, Some(65. | 66.)) {
                    if macro_code == Some(66.) {
                        if let Some(f) = self.frames.last_mut() {
                            f.modal = Some(words.clone())
                        }
                    }
                    self.call(library, &words, true)?;
                    return Ok(false);
                }
                if modal_arguments {
                    let mut args = self
                        .frames
                        .last()
                        .and_then(|f| f.modal.clone())
                        .ok_or_else(|| NcError::new("Missing modal macro"))?;
                    args.extend(words.clone());
                    if let Some(f) = self.frames.last_mut() {
                        f.modal = Some(args.clone())
                    }
                    self.call(library, &args, true)?;
                    return Ok(false);
                }
                match words.get(&'M').copied() {
                    Some(2. | 30.) => return Ok(true),
                    Some(98.) => self.call(library, &words, false)?,
                    Some(99.) => self.return_call(library, words.get(&'P').copied())?,
                    _ => {}
                }
                Ok(false)
            })();
            match action {
                Ok(true) => break,
                Ok(false) => {}
                Err(e) => {
                    self.fail(e);
                    break;
                }
            }
        }
        for block in library
            .programs
            .iter()
            .flat_map(|program| program.blocks.iter())
        {
            if let Some(error) = &block.error {
                if !visited.contains(&(block.source.clone(), block.line)) {
                    self.model.warnings.push(Diagnostic {
                        source: block.source.clone(),
                        line: block.line,
                        step: 0,
                        error: error.clone(),
                    });
                }
            }
        }
    }
    fn fail(&mut self, error: NcError) {
        self.model.status = match error.code.as_str() {
            "LIMIT" => "limit",
            "ALARM" => "alarm",
            "INPUT_REQUIRED" => "input-required",
            _ => "error",
        }
        .into();
        if let Some(r) = &error.requirement {
            self.model.requirements.push(r.clone())
        }
        self.model.diagnostics.push(Diagnostic {
            source: self.source.clone(),
            line: self.line,
            step: self.step,
            error,
        })
    }
    fn call(&mut self, library: &Library, words: &Words, macro_call: bool) -> Result<()> {
        if self.frames.len() >= self.max_depth {
            return Err(NcError::new("Maximum subprogram call depth reached").code("LIMIT"));
        }
        let id = integer(
            *words
                .get(&'P')
                .ok_or_else(|| NcError::new("A call requires P"))?,
            "Subprogram P",
            0,
            99999999,
        )?;
        let program = *library
            .ids
            .get(&id)
            .ok_or_else(|| NcError::required(format!("subprograms.O{id}")))?;
        let remaining = integer(
            *words.get(&'L').unwrap_or(&1.),
            "Call repeat count",
            1,
            1000000,
        )?;
        let arguments = words
            .iter()
            .filter_map(|(k, v)| argument(*k).map(|n| (n, Some(*v))))
            .collect::<BTreeMap<_, _>>();
        let start = if let Some(q) = words.get(&'Q').filter(|_| !macro_call) {
            let q = integer(*q, "M98 Q entry", 0, 99999999)?;
            *library.programs[program]
                .labels
                .get(&q)
                .ok_or_else(|| NcError::new("M98 Q entry does not exist"))?
        } else {
            0
        };
        if macro_call {
            self.locals.push(arguments.clone());
            self.dirty = true
        }
        self.frames.push(Frame {
            program,
            pc: start,
            start,
            remaining,
            macro_call,
            arguments,
            modal: None,
        });
        self.event(
            "call",
            json!({"program":id,"macro":macro_call,"repeat":remaining}),
        );
        Ok(())
    }
    fn return_call(&mut self, library: &Library, target: Option<f64>) -> Result<()> {
        if self.frames.len() == 1 {
            let frame = self
                .frames
                .last_mut()
                .ok_or_else(|| NcError::new("No caller"))?;
            frame.pc = if let Some(target) = target {
                let n = integer(target, "Return N label", 0, 99999999)?;
                *library.programs[frame.program]
                    .labels
                    .get(&n)
                    .ok_or_else(|| NcError::new("Return label missing"))?
            } else {
                0
            };
            return Ok(());
        }
        let mut frame = self
            .frames
            .pop()
            .ok_or_else(|| NcError::new("No call frame"))?;
        if frame.macro_call {
            self.locals.pop();
            self.dirty = true
        }
        if target.is_none() && frame.remaining > 1 {
            frame.remaining -= 1;
            frame.pc = frame.start;
            if frame.macro_call {
                self.locals.push(frame.arguments.clone());
                self.dirty = true
            }
            self.frames.push(frame)
        }
        if let Some(target) = target {
            let frame = self
                .frames
                .last_mut()
                .ok_or_else(|| NcError::new("No caller"))?;
            let n = integer(target, "Return N label", 0, 99999999)?;
            frame.pc = *library.programs[frame.program]
                .labels
                .get(&n)
                .ok_or_else(|| NcError::new("Return label missing"))?;
        }
        self.event("return", Value::Null);
        Ok(())
    }
}
fn argument(letter: char) -> Option<u32> {
    match letter {
        'A' => Some(1),
        'B' => Some(2),
        'C' => Some(3),
        'I' => Some(4),
        'J' => Some(5),
        'K' => Some(6),
        'D' => Some(7),
        'E' => Some(8),
        'F' => Some(9),
        'H' => Some(11),
        'M' => Some(13),
        'Q' => Some(17),
        'R' => Some(18),
        'S' => Some(19),
        'T' => Some(20),
        'U' => Some(21),
        'V' => Some(22),
        'W' => Some(23),
        'X' => Some(24),
        'Y' => Some(25),
        'Z' => Some(26),
        _ => None,
    }
}
