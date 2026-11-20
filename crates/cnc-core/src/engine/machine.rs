use super::*;
pub(super) fn group(code: f64) -> u32 {
    let groups: &[&[f64]] = &[
        &[0., 1., 2., 3., 12., 13., 31., 31.1, 31.2, 31.3, 32.],
        &[17., 18., 19.],
        &[90., 91.],
        &[90.1, 91.1],
        &[93., 94., 95.],
        &[20., 21.],
        &[40., 41., 42.],
        &[43., 44., 49.],
        &[
            80., 73., 74., 76., 81., 82., 83., 84., 84.2, 84.3, 85., 86., 87., 88., 89.,
        ],
        &[98., 99.],
        &[50., 51.],
        &[67., 66.],
        &[97., 96.],
        &[54., 54.1, 55., 56., 57., 58., 59.],
        &[64., 61.],
        &[69., 68.],
        &[15., 16.],
        &[40.1, 40.2],
    ];
    groups
        .iter()
        .position(|values| values.contains(&code))
        .map_or(0, |i| i as u32 + 1)
}
fn plane_axes(plane: u32) -> [usize; 3] {
    match plane {
        18 => [2, 0, 1],
        19 => [1, 2, 0],
        _ => [0, 1, 2],
    }
}
impl Engine {
    pub(super) fn length(&self) -> Result<f64> {
        if self.length_mode == 49 || self.length_index == 0 {
            return Ok(0.);
        }
        let tool = &self.setup["tools"][self.length_index.to_string()];
        let length = tool["length"]
            .as_f64()
            .ok_or_else(|| NcError::required(format!("tools.{}.length", self.length_index)))?;
        Ok((length + number(&tool["lengthWear"], 0.))
            * if self.length_mode == 44 { -1. } else { 1. })
    }
    pub(super) fn target(&mut self, words: &Words, machine: bool) -> Result<[f64; 6]> {
        let mut target = if machine { self.position } else { self.command };
        if machine {
            target[2] += self.length()?
        }
        for i in 0..6 {
            if let Some(word) = words.get(&AXES[i]) {
                let role = self.setup["axisRoles"][AXES[i].to_string()]
                    .as_str()
                    .unwrap_or("");
                if role == "disabled" {
                    if self.param(1806, 0.) != 0. {
                        return Err(NcError::new(format!("Axis {} is disabled", AXES[i])));
                    }
                    self.event("ignored-axis", json!({"axis": AXES[i], "value": word}));
                    continue;
                }
                let scale = if machine && self.param(1808, 1.) != 0. && (i < 3 || role == "linear")
                {
                    self.initial_unit
                } else {
                    self.axis_scale(i)
                };
                let word = if i == 0 && !machine && self.setup["diameterMode"] == true {
                    word / 2.
                } else {
                    *word
                };
                let next = word * scale
                    + if self.incremental && !machine {
                        target[i]
                    } else {
                        0.
                    };
                if role == "linear" && (next - target[i]).abs() > 0. {
                    return Err(NcError::required(format!("axisMapping.{}", AXES[i])));
                }
                target[i] = next;
            }
        }
        if self.polar && !machine {
            let [u, v, _] = plane_axes(self.plane);
            let radius = *words
                .get(&AXES[u])
                .ok_or_else(|| NcError::new("Polar blocks require radius and angle"))?
                * self.unit;
            let angle = *words
                .get(&AXES[v])
                .ok_or_else(|| NcError::new("Polar blocks require radius and angle"))?
                * std::f64::consts::PI
                / 180.;
            target[u] = radius * angle.cos()
                + if self.incremental {
                    self.command[u]
                } else {
                    0.
                };
            target[v] = radius * angle.sin()
                + if self.incremental {
                    self.command[v]
                } else {
                    0.
                };
        }
        Ok(target)
    }
    pub(super) fn rotation_for(&self, axes: [f64; 6]) -> Point {
        std::array::from_fn(|i| {
            if self.setup["axisRoles"][AXES[i + 3].to_string()] == "rotary" {
                axes[i + 3]
            } else {
                0.
            }
        })
    }
    fn emit(
        &mut self,
        mut points: Vec<Point>,
        end: [f64; 6],
        rapid: bool,
        command: [f64; 6],
    ) -> Result<()> {
        if end.iter().any(|v| !v.is_finite() || v.abs() > 1e9)
            || points
                .iter()
                .flatten()
                .any(|v| !v.is_finite() || v.abs() > 1e9)
        {
            return Err(NcError::new("Coordinates exceed the numeric range").code("GEOMETRY"));
        }
        let start_rotation = self.rotation_for(self.position);
        let end_rotation = self.rotation_for(end);
        let rotary_distance = distance(&start_rotation, &end_rotation);
        if points.len() == 2 && rotary_distance > 0. {
            let count = (rotary_distance / 2.).ceil().clamp(16., 4096.) as usize;
            let [start, end] = [points[0], points[1]];
            points = (0..=count)
                .map(|i| interpolate(start, end, i as f64 / count as f64))
                .collect();
        }
        if self
            .model
            .segments
            .len()
            .checked_add(points.len().saturating_sub(1))
            .is_none_or(|n| n > self.max_segments)
        {
            return Err(NcError::new("Geometry segment limit reached").code("LIMIT"));
        }
        let feed = if rapid {
            number(&self.setup["rapidRate"], 3000.)
        } else if self.feed_mode == 95 {
            if self.spindle[1] <= 0. {
                return Err(NcError::new("G95 requires spindle speed S"));
            }
            self.feed * self.spindle[1]
        } else {
            self.feed
        };
        if !feed.is_finite() || feed <= 0. {
            return Err(NcError::required("initialFeed"));
        }
        let start = [self.position[0], self.position[1], self.position[2]];
        let endpoint = [end[0], end[1], end[2]];
        let segment_start = self.model.segments.len();
        let entry = self.first && rapid;
        let length = if entry {
            0.
        } else {
            points
                .windows(2)
                .map(|p| distance(&p[0], &p[1]))
                .sum::<f64>()
        };
        let seconds = if self.feed_mode == 93 && !rapid {
            60. / self.feed
        } else {
            length / feed * 60.
        };
        let end_time = self.elapsed + seconds * 1000.;
        if !seconds.is_finite() || !end_time.is_finite() || end_time > 9_007_199_254_740_991. {
            return Err(NcError::new("Program duration exceeds the numeric range").code("LIMIT"));
        }

        let tool = &self.setup["tools"][self.tool.to_string()];
        let tool_diameter = tool["diameter"]
            .as_f64()
            .map(|d| d + number(&tool["diameterWear"], 0.));
        if !entry {
            for (point_index, pair) in points.windows(2).enumerate() {
                self.model.segments.push(Segment {
                    start: pair[0],
                    end: pair[1],
                    rapid,
                    line: self.line,
                    step: self.step,
                    tool: self.tool,
                    movement: self.model.moves.len(),
                    start_rotation: interpolate(
                        start_rotation,
                        end_rotation,
                        point_index as f64 / (points.len() - 1) as f64,
                    ),
                    rotation: interpolate(
                        start_rotation,
                        end_rotation,
                        (point_index + 1) as f64 / (points.len() - 1) as f64,
                    ),
                    tool_diameter,
                });
            }
        }
        for point in if entry { vec![endpoint] } else { points } {
            let bounds = self.model.bounds.get_or_insert((point, point));
            for (i, value) in point.iter().enumerate() {
                bounds.0[i] = bounds.0[i].min(*value);
                bounds.1[i] = bounds.1[i].max(*value);
            }
        }
        self.model.moves.push(Movement {
            start,
            end: endpoint,
            start_axes: self.position,
            end_axes: end,
            rapid,
            line: self.line,
            step: self.step,
            tool: self.tool,
            feed,
            length,
            seconds,
            start_time: self.elapsed / 1000.,
            segment_start,
            segment_end: self.model.segments.len(),
        });
        self.add_time(seconds * 1000.)?;
        for (id, value) in std::mem::take(&mut self.pending_outputs) {
            self.setup["outputs"][id.to_string()] = json!(value);
            self.event("output", json!({"id":id,"value":value}));
        }
        self.position = end;
        self.command = command;
        self.first = false;
        Ok(())
    }
    pub(super) fn linear(&mut self, target: [f64; 6], rapid: bool, machine: bool) -> Result<()> {
        let mut end = if machine {
            target
        } else {
            self.transform(target, false)?
        };
        if machine {
            end[2] -= self.length()?
        }
        let command = if machine {
            self.transform(end, true)?
        } else {
            target
        };
        self.emit(
            vec![
                [self.position[0], self.position[1], self.position[2]],
                [end[0], end[1], end[2]],
            ],
            end,
            rapid,
            command,
        )
    }
    fn arc(&mut self, target: [f64; 6], words: &Words, clockwise: bool) -> Result<()> {
        let [u, v, _] = plane_axes(self.plane);
        if (self.scale[u].abs() - self.scale[v].abs()).abs() > 1e-9 {
            return Err(NcError::new(
                "Arc needs equal scale magnitudes in its plane",
            ));
        }
        let center = if words.contains_key(&'R') {
            None
        } else {
            if ![u, v]
                .iter()
                .any(|i| words.contains_key(&['I', 'J', 'K'][*i]))
            {
                return Err(NcError::new(
                    "Arc needs R or an I/J/K center in the active plane",
                ));
            }
            Some(std::array::from_fn(|i| {
                words.get(&['I', 'J', 'K'][i]).map_or(self.command[i], |w| {
                    w * self.unit
                        + if self.absolute_center {
                            0.
                        } else {
                            self.command[i]
                        }
                })
            }))
        };
        let points = arc_points(&Circle {
            start: [self.command[0], self.command[1], self.command[2]],
            end: [target[0], target[1], target[2]],
            center,
            radius: words.get(&'R').map(|r| r * self.unit),
            plane: self.plane,
            clockwise,
            tolerance: 0.05,
        })
        .map_err(|e| NcError::new(e.to_string()).code("GEOMETRY"))?;
        let mut transformed = Vec::with_capacity(points.len());
        for point in points {
            let mut p = self.command;
            p[..3].copy_from_slice(&point);
            let p = self.transform(p, false)?;
            transformed.push([p[0], p[1], p[2]])
        }
        self.emit(transformed, self.transform(target, false)?, false, target)
    }
    pub(super) fn execute(&mut self, words: &Words, codes: &[f64], items: &[Value]) -> Result<()> {
        self.buffered.extend(words.clone());
        for code in codes {
            if group(*code) == 0
                && ![4., 9., 10., 11., 28., 30., 52., 53., 60., 65., 92., 92.1].contains(code)
            {
                return Err(
                    NcError::new(format!("G{code} is unsupported in CNC Viewer"))
                        .code("UNSUPPORTED"),
                );
            }
        }
        if let Some(m) = words.get(&'M') {
            let m = integer(*m, "M code", 0, 99999999)?;
            if self.setup["mCodeOverrides"][m.to_string()].is_null()
                && ![
                    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 19, 30, 48, 49, 62, 63, 64, 65, 98, 99, 220, 221,
                    222, 223, 224, 242, 243, 1000, 1001,
                ]
                .contains(&m)
                && !(200..=219).contains(&m)
            {
                return Err(NcError::required(format!("customMCodes.{m}")));
            }
        }
        let effective: Vec<_> = codes
            .iter()
            .enumerate()
            .filter_map(|(i, c)| {
                let g = group(*c);
                if g == 0 || !codes[i + 1..].iter().any(|n| group(*n) == g) {
                    Some(*c)
                } else {
                    None
                }
            })
            .collect();
        for code in &effective {
            let g = group(*code);
            if g > 0 {
                self.modal.insert(g, *code);
            }
            if *code == 20. {
                self.unit = 25.4
            } else if *code == 21. {
                self.unit = 1.
            }
        }
        if self.rotation.is_some()
            && !effective.contains(&69.)
            && effective.iter().any(|c| {
                [
                    28., 30., 52., 53., 54., 54.1, 55., 56., 57., 58., 59., 92., 92.1,
                ]
                .contains(c)
            })
        {
            return Err(NcError::new(
                "Cancel G68 before reference returns or coordinate changes",
            ));
        }
        {
            if let Some(f) = words.get(&'F') {
                if *f < 0. {
                    return Err(NcError::new("Feed cannot be negative"));
                }
                self.feed = f * if effective.contains(&93.)
                    || (self.feed_mode == 93 && !effective.iter().any(|c| [94., 95.].contains(c)))
                {
                    1.
                } else {
                    self.unit
                };
            }
            if let Some(s) = words.get(&'S').filter(|_| words.get(&'M') != Some(&19.)) {
                if *s < 0. {
                    return Err(NcError::new("Spindle speed cannot be negative"));
                }
                self.spindle[1] = *s;
            }
            if let Some(t) = words.get(&'T') {
                self.selected_tool = integer(*t, "Tool number", 0, 99999999)?;
            }
        }
        let mut special = false;
        if let Some(mode) = self
            .data_mode
            .filter(|_| !effective.iter().any(|c| [10., 11.].contains(c)))
        {
            let mut data = words.clone();
            data.insert('L', mode as f64);
            self.set_data(&data, items)?;
            self.add_time(number(&self.setup["instructionTimeMs"], 1.))?;
            return Ok(());
        }
        for code in &effective {
            match *code {
                0. | 1. | 2. | 3. | 73. | 74. | 76. | 81. | 82. | 83. | 84. | 84.2 | 84.3 | 85.
                | 86. | 87. | 88. | 89. => {
                    self.motion = *code as u32;
                    self.motion_variant = if code.fract() != 0. {
                        Some(*code)
                    } else {
                        None
                    };
                    if ![
                        73., 74., 76., 81., 82., 83., 84., 84.2, 84.3, 85., 86., 87., 88., 89.,
                    ]
                    .contains(code)
                    {
                        self.cycle = None
                    }
                }
                17. | 18. | 19. => self.plane = *code as u32,
                90. | 91. => self.incremental = *code == 91.,
                90.1 | 91.1 => self.absolute_center = *code == 90.1,
                93. | 94. | 95. => self.feed_mode = *code as u32,
                98. | 99. => self.return_initial = *code == 98.,
                80. => {
                    self.cycle = None;
                    self.motion = 80;
                    self.motion_variant = None;
                    special = true
                }
                43. | 44. | 49. => {
                    let old = self.length()?;
                    self.length_mode = *code as u32;
                    if let Some(h) = words.get(&'H') {
                        self.length_index = integer(*h, "H offset", 0, 99999999)?
                    }
                    self.position[2] += old - self.length()?;
                    self.resync()?;
                    self.event(
                        "tool-length",
                        json!({"mode":code,"index": self.length_index,"known":true}),
                    );
                }
                54. | 54.1 | 55. | 56. | 57. | 58. | 59. => {
                    self.work = if *code == 54.1
                        || (*code == 59. && words.get(&'P').is_some_and(|p| *p >= 7.))
                    {
                        let p = words
                            .get(&'P')
                            .copied()
                            .ok_or_else(|| NcError::new("Extended work offset requires P"))?
                            - if *code == 59. { 6. } else { 0. };
                        format!("54.1:{}", integer(p, "Extended work offset", 1, 248)?)
                    } else {
                        format!("{code}")
                    };
                    self.resync()?;
                }
                52. => {
                    for (i, axis) in AXES.iter().enumerate() {
                        if let Some(w) = words.get(axis) {
                            self.local[i] = w * self.axis_scale(i)
                        }
                    }
                    self.resync()?;
                    special = true
                }
                92. => {
                    for (i, axis) in AXES.iter().enumerate() {
                        if let Some(w) = words.get(axis) {
                            self.shift[i] +=
                                (self.command[i] - w * self.axis_scale(i)) * self.scale[i]
                        }
                    }
                    self.resync()?;
                    special = true
                }
                92.1 => {
                    self.shift = [0.; 6];
                    self.resync()?;
                    special = true
                }
                50. => {
                    self.scale = [1.; 6];
                    self.resync()?;
                }
                51. => {
                    for (i, axis) in AXES.iter().enumerate() {
                        if let Some(w) = words.get(axis) {
                            if *w == 0. {
                                return Err(NcError::new("G51 scale factors must be nonzero"));
                            }
                            self.scale[i] = *w;
                        }
                    }
                    self.resync()?;
                    special = true
                }
                68. => {
                    if self.plane != 17 {
                        return Err(NcError::new("G68 requires G17"));
                    }
                    let angle = *words
                        .get(&'R')
                        .ok_or_else(|| NcError::new("G68 requires R"))?;
                    self.rotation = Some([
                        words.get(&'X').copied().unwrap_or(0.) * self.unit,
                        words.get(&'Y').copied().unwrap_or(0.) * self.unit,
                        angle,
                    ]);
                    self.resync()?;
                    special = true
                }
                69. => {
                    self.rotation = None;
                    self.resync()?;
                }
                15. | 16. => self.polar = *code == 16.,
                40. | 41. | 42. => {
                    self.compensation = *code as u32;
                    if *code != 40. {
                        let d = words.get(&'D').copied().unwrap_or(self.tool as f64);
                        let radius = if let Some(p) = words.get(&'P') {
                            p * self.unit
                        } else {
                            let tool = &self.setup["tools"][(d as u32).to_string()];
                            (tool["diameter"]
                                .as_f64()
                                .ok_or_else(|| NcError::required(format!("tools.{d}.diameter")))?
                                + number(&tool["diameterWear"], 0.))
                                / 2.
                        };
                        if radius < 0. {
                            return Err(NcError::new("Cutter radius must be nonnegative"));
                        }
                        if radius > 0. {
                            return Err(NcError::new("Nonzero cutter compensation is unsupported in the reference profile").code("UNSUPPORTED"));
                        }
                    }
                }
                10. => {
                    self.set_data(words, items)?;
                    special = true
                }
                11. => {
                    self.data_mode = None;
                    special = true
                }
                28. | 30. => {
                    let key = if *code == 28. {
                        "28".into()
                    } else {
                        format!("30:{}", words.get(&'P').copied().unwrap_or(1.))
                    };
                    let references = &self.setup["referencePositions"];
                    let reference = if *code == 30. && references[&key].is_null() {
                        references["30"].clone()
                    } else {
                        references[&key].clone()
                    };
                    if reference.is_null() {
                        return Err(NcError::required(format!("referencePositions.{key}")));
                    }
                    if AXES.iter().any(|a| words.contains_key(a)) {
                        let target = self.target(words, false)?;
                        self.linear(target, true, false)?
                    }
                    let reference = pose(&reference)?;
                    let mut target = self.position;
                    target[2] += self.length()?;
                    let all = !AXES.iter().any(|a| words.contains_key(a));
                    for i in 0..6 {
                        if all || words.contains_key(&AXES[i]) {
                            let role = self.setup["axisRoles"][AXES[i].to_string()]
                                .as_str()
                                .unwrap_or("");
                            if role == "disabled" {
                                continue;
                            }
                            if role == "linear" && target[i] != reference[i] {
                                return Err(NcError::required(format!("axisMapping.{}", AXES[i])));
                            }
                            target[i] = reference[i]
                        }
                    }
                    self.linear(target, true, true)?;
                    special = true
                }
                4. => {
                    let p = words
                        .get(&'P')
                        .or_else(|| words.get(&'X'))
                        .copied()
                        .filter(|p| *p >= 0.)
                        .ok_or_else(|| NcError::new("G4 requires a nonnegative P or X dwell"))?;
                    let decimal = items.iter().any(|i| {
                        i["type"] == "word" && i["letter"] == "P" && i["decimalLiteral"] == true
                    });
                    let seconds = p / if !words.contains_key(&'P') || decimal {
                        1.
                    } else {
                        1000.
                    };
                    self.event("dwell", json!({"seconds":seconds}));
                    self.add_time(seconds * 1000.)?;
                    special = true
                }
                31. | 31.1 | 31.2 | 31.3 => {
                    if effective.contains(&53.) {
                        return Err(NcError::new("G53 requires G0 or G1, not a probing cycle"));
                    }
                    self.probe(words, *code)?;
                    special = true;
                }
                12. | 13. | 32. | 60. | 96. => {
                    return Err(
                        NcError::new(format!("G{code} is unsupported in CNC Viewer"))
                            .code("UNSUPPORTED"),
                    )
                }
                _ => {}
            }
        }
        let axes = AXES.iter().any(|a| words.contains_key(a));
        if !special
            && (axes
                || ([2, 3].contains(&self.motion)
                    && ['I', 'J', 'K', 'R'].iter().any(|a| words.contains_key(a))))
        {
            if [73, 74, 76, 81, 82, 83, 84, 85, 86, 87, 88, 89].contains(&self.motion) {
                self.drill(words, self.motion)?
            } else if self.motion == 80 {
                return Err(NcError::new("Axis words require a motion mode after G80"));
            } else {
                let machine = effective.contains(&53.);
                if machine && ![0, 1].contains(&self.motion) {
                    return Err(NcError::new("G53 requires G0 or G1"));
                }
                let target = self.target(words, machine)?;
                if [2, 3].contains(&self.motion) {
                    self.arc(target, words, self.motion == 2)?
                } else {
                    self.linear(target, self.motion == 0, machine)?
                }
            }
        }
        if let Some(m) = words.get(&'M') {
            let arguments = self.m_words.clone().unwrap_or_else(|| words.clone());
            self.execute_m(*m as u32, &arguments)?
        }
        self.add_time(number(&self.setup["instructionTimeMs"], 1.))?;
        Ok(())
    }
    fn set_data(&mut self, words: &Words, items: &[Value]) -> Result<()> {
        let mode = integer(
            *words
                .get(&'L')
                .ok_or_else(|| NcError::new("G10 requires L"))?,
            "G10 L mode",
            0,
            99999999,
        )?;
        if mode == 50 {
            self.data_mode = Some(50);
            if let (Some(&parameter), Some(&value)) = (words.get(&'N'), words.get(&'R')) {
                let parameter = integer(parameter, "Parameter", 0, 99999999)?;
                let literal = items
                    .iter()
                    .find(|item| item["letter"] == "R")
                    .and_then(|item| item["rawLiteral"].as_str())
                    .filter(|raw| raw.len() >= 8 && raw.bytes().all(|b| b == b'0' || b == b'1'));
                let value = if let Some(bits) = literal {
                    bits.bytes()
                        .fold(0., |number, bit| number * 2. + f64::from(bit - b'0'))
                } else {
                    value
                };
                if !value.is_finite() {
                    return Err(NcError::new("Parameter value exceeds the numeric range"));
                }
                self.setup["parameters"][parameter.to_string()] = json!(value);
            }
            return Ok(());
        }
        if !words.contains_key(&'P') && [1, 2, 10, 11, 12, 13, 20].contains(&mode) {
            self.data_mode = Some(mode);
            return Ok(());
        }
        let index = integer(*words.get(&'P').unwrap_or(&0.), "G10 P index", 0, 99999999)?;
        if [1, 10, 11, 12, 13].contains(&mode) {
            let fields: Vec<(char, &str)> = if mode == 1 {
                vec![
                    ('Z', "length"),
                    ('W', "lengthWear"),
                    ('D', "diameter"),
                    ('R', "diameterWear"),
                    ('Q', "pocket"),
                ]
            } else {
                vec![(
                    'R',
                    match mode {
                        10 => "length",
                        11 => "lengthWear",
                        12 => "diameter",
                        _ => "diameterWear",
                    },
                )]
            };
            for (letter, field) in fields {
                if let Some(value) = words.get(&letter) {
                    let previous = number(&self.setup["tools"][index.to_string()][field], 0.);
                    let value = value * if letter == 'Q' { 1. } else { self.unit }
                        + if self.incremental && mode != 1 {
                            previous
                        } else {
                            0.
                        };
                    self.setup["tools"][index.to_string()][field] = json!(value)
                } else if mode != 1 {
                    return Err(NcError::new("G10 tool update requires R"));
                }
            }
            return Ok(());
        }
        if ![2, 20].contains(&mode) {
            return Err(
                NcError::new(format!("G10 L{mode} is unsupported in CNC Viewer"))
                    .code("UNSUPPORTED"),
            );
        }
        if (mode == 2 && index > 6) || (mode == 20 && index > 248) {
            return Err(NcError::new("G10 work offset P is out of range"));
        }
        let key = if mode == 2 {
            (53 + index).to_string()
        } else {
            format!("54.1:{index}")
        };
        let mut target = if index == 0 {
            if mode == 2 {
                self.work_shift
            } else {
                self.head_shift
            }
        } else {
            pose(&self.setup["workOffsets"][&key])?
        };
        for i in 0..6 {
            if let Some(w) = words.get(&AXES[i]) {
                target[i] = w * self.axis_scale(i) + if self.incremental { target[i] } else { 0. }
            }
        }
        if index == 0 {
            if mode == 2 {
                self.work_shift = target
            } else {
                self.head_shift = target
            }
        } else {
            self.setup["workOffsets"][&key] = json!(target)
        }
        self.resync()
    }
    fn execute_m(&mut self, code: u32, words: &Words) -> Result<()> {
        let override_value = &self.setup["mCodeOverrides"][code.to_string()];
        if !override_value.is_null() {
            if override_value["effect"] != "none" {
                return Err(NcError::new(
                    "An M-code override must explicitly specify effect none",
                ));
            }
            self.event(
                "auxiliary-override",
                json!({"code":code,"label":override_value["label"]}),
            );
            return Ok(());
        }
        match code {
            3..=5 => {
                self.spindle[0] = if code == 3 {
                    1.
                } else if code == 4 {
                    -1.
                } else {
                    0.
                };
                self.event(
                    "spindle",
                    json!({"direction":self.spindle[0],"speed":self.spindle[1]}),
                );
            }
            6 => {
                self.tool = self.selected_tool;
                self.event("tool-change", json!({"tool":self.tool}));
            }
            0 | 1 | 1000 | 1001 => {
                if code == 0 || code == 1000 || self.setup["optionalStop"] == true {
                    self.event("program-stop", json!({"code":code}));
                }
            }
            7..=9 => self.event("coolant", json!({"code":code})),
            19 => self.event("spindle-orient", json!({"angle":words.get(&'S')})),
            48 | 49 => self.event("feed-override", json!({"enabled":code==48})),
            62..=65 | 200..=221 => {
                if (220..=224).contains(&code) && words.contains_key(&'A') {
                    return Err(NcError::required(format!(
                        "signalMapping.M{code}.A{}",
                        words[&'A']
                    )));
                }
                let id = if (200..=219).contains(&code) {
                    (code - 200) / 2
                } else {
                    integer(
                        *words
                            .get(&'P')
                            .or_else(|| words.get(&'A'))
                            .or_else(|| words.get(&'S'))
                            .ok_or_else(|| NcError::new("Output code requires a signal ID"))?,
                        "Output signal",
                        0,
                        9999,
                    )?
                };
                let value = u32::from(
                    [62, 64, 220].contains(&code)
                        || ((200..=219).contains(&code) && code.is_multiple_of(2)),
                );
                if code == 62 || code == 63 {
                    self.pending_outputs.push((id, value));
                } else {
                    self.setup["outputs"][id.to_string()] = json!(value);
                    self.event("output", json!({"id":id,"value":value}));
                }
                if let Some(dwell) = words.get(&'D').filter(|d| **d > 0.) {
                    self.add_time(*dwell)?;
                }
            }
            222..=224 => {
                if let Some(a) = words.get(&'A') {
                    return Err(NcError::required(format!("signalMapping.M{code}.A{a}")));
                }
                let id = integer(
                    *words
                        .get(&'S')
                        .ok_or_else(|| NcError::new("Signal read requires S"))?,
                    "Signal",
                    0,
                    9999,
                )?;
                let key = if code == 224 { "outputs" } else { "inputs" };
                let signal = &self.setup[key][id.to_string()];
                if signal.is_null() {
                    return Err(NcError::required(format!("{key}.{id}")));
                }
                let value = u32::from(
                    signal == &Value::Bool(true) || signal.as_f64().is_some_and(|v| v != 0.),
                ) ^ u32::from(words.get(&'I').is_some_and(|v| *v != 0.));
                if code == 222 {
                    if value == 0 {
                        return Err(NcError::required(format!("inputs.{id}")));
                    }
                    self.event("input-wait", json!({"id":id,"value":value}));
                } else {
                    let address = integer(
                        *words
                            .get(&'V')
                            .ok_or_else(|| NcError::new("Signal read requires V destination"))?,
                        "Destination variable",
                        0,
                        99999,
                    )?;
                    self.set_variable(address, Some(value as f64))?;
                }
            }
            242 | 243 => {
                let text = self.block_text.to_ascii_lowercase();
                let start = text
                    .find("data")
                    .and_then(|i| self.block_text[i + 4..].find(':').map(|n| i + 5 + n))
                    .ok_or_else(|| {
                        NcError::new("M242/M243 require Data: followed by a quoted setting name")
                    })?;
                let name: Value =
                    serde_json::Deserializer::from_str(self.block_text[start..].trim_start())
                        .into_iter::<Value>()
                        .next()
                        .ok_or_else(|| NcError::new("Missing quoted machine setting"))?
                        .map_err(|_| NcError::new("Invalid quoted machine setting"))?;
                let name = name
                    .as_str()
                    .ok_or_else(|| NcError::new("Machine setting name must be quoted"))?
                    .to_owned();
                let value = self.setup["machineSettings"][&name]
                    .as_f64()
                    .ok_or_else(|| NcError::required(format!("machineSettings.{name}")))?;
                if code == 242 {
                    let address = integer(
                        *words
                            .get(&'V')
                            .ok_or_else(|| NcError::new("M242 requires V destination"))?,
                        "Destination variable",
                        0,
                        99999,
                    )?;
                    self.set_variable(address, Some(value))?;
                } else {
                    let next = words
                        .get(&'T')
                        .copied()
                        .or_else(|| words.get(&'I').map(|v| value + v))
                        .ok_or_else(|| NcError::new("M243 requires T value or I increment"))?;
                    if !next.is_finite() {
                        return Err(NcError::new("Machine setting exceeds numeric range"));
                    }
                    self.setup["machineSettings"][&name] = json!(next);
                }
                self.event("machine-setting",json!({"name":name,"value":self.setup["machineSettings"][&name],"write":code==243}));
            }
            2 | 30 | 98 | 99 => {}
            _ => return Err(NcError::required(format!("customMCodes.{code}"))),
        }
        Ok(())
    }
    fn cycle_leg(&mut self, axis: usize, value: f64, rapid: bool) -> Result<()> {
        let mut target = self.command;
        target[axis] = value;
        self.linear(target, rapid, false)
    }
    fn drill(&mut self, words: &Words, code: u32) -> Result<()> {
        let [u, v, n] = plane_axes(self.plane);
        if self.cycle.as_ref().is_none_or(|c| c.code != code) {
            self.cycle = Some(Cycle {
                code,
                initial: self.command[n],
                words: Words::new(),
            })
        }
        let cycle = self
            .cycle
            .as_mut()
            .ok_or_else(|| NcError::new("Missing cycle"))?;
        for letter in [AXES[n], 'R', 'Q', 'P'] {
            if let Some(value) = words.get(&letter) {
                cycle.words.insert(letter, *value);
            }
        }
        let cycle = cycle.clone();
        let r = *cycle
            .words
            .get(&'R')
            .ok_or_else(|| NcError::new("A canned cycle requires R"))?;
        let z = *cycle
            .words
            .get(&AXES[n])
            .ok_or_else(|| NcError::new("Drilling cycle needs a Z depth"))?;
        let retract = r * self.unit + if self.incremental { cycle.initial } else { 0. };
        let bottom = z * self.unit + if self.incremental { retract } else { 0. };
        if bottom >= retract {
            return Err(NcError::new("Hole bottom must be below the R plane"));
        }
        let peck = cycle.words.get(&'Q').copied().unwrap_or(0.) * self.unit;
        if [73, 83].contains(&code) && peck <= 0. {
            return Err(NcError::new("Peck cycle requires positive Q"));
        }
        if code == 88 {
            return Err(NcError::required("manualRetract"));
        }
        if [76, 87].contains(&code) {
            return Err(NcError::required(format!("cycleImplementations.G{code}")));
        }
        if self.compensation != 40 {
            return Err(NcError::new("Cancel compensation before a canned cycle"));
        }
        if [74, 84].contains(&code) && self.spindle[1] <= 0. {
            return Err(NcError::new("Tapping requires spindle speed S"));
        }
        let repeats =
            integer(*words.get(&'L').unwrap_or(&1.), "Cycle repeats", 1, 100000)? as usize;
        let estimated = repeats as f64
            * if [73, 83].contains(&code) {
                3. * ((retract - bottom) / peck).ceil() + 4.
            } else {
                5.
            };
        if !estimated.is_finite() || estimated > self.max_segments as f64 {
            return Err(NcError::new("Canned cycle exceeds geometry limit").code("LIMIT"));
        }
        let clearance = self.param(if code == 73 { 5114 } else { 5115 }, 250.)
            * if self.initial_unit == 25.4 {
                0.00254
            } else {
                0.001
            };
        if clearance < 0. {
            return Err(NcError::new("Peck clearance must be nonnegative"));
        }
        let dwell = cycle.words.get(&'P').copied().unwrap_or(0.) / 1000.;
        if dwell < 0. {
            return Err(NcError::new("Cycle dwell must be nonnegative"));
        }
        let mut target = self.command;
        for _ in 0..repeats {
            for axis in [u, v] {
                if let Some(w) = words.get(&AXES[axis]) {
                    target[axis] = w * self.unit + if self.incremental { target[axis] } else { 0. }
                }
            }
            if self.command[n] < retract {
                self.cycle_leg(n, retract, true)?
            }
            target[n] = self.command[n];
            self.linear(target, true, false)?;
            self.cycle_leg(n, retract, true)?;
            if [73, 83].contains(&code) {
                let mut depth = retract;
                while depth > bottom + 1e-9 {
                    let next = bottom.max(depth - peck);
                    if code == 83 && depth < retract {
                        self.cycle_leg(n, retract.min(depth + clearance), true)?
                    }
                    self.cycle_leg(n, next, false)?;
                    depth = next;
                    if depth > bottom {
                        self.cycle_leg(
                            n,
                            if code == 83 {
                                retract
                            } else {
                                retract.min(depth + clearance)
                            },
                            true,
                        )?
                    }
                }
            } else {
                self.cycle_leg(n, bottom, false)?
            }
            if [82, 86, 89].contains(&code) && dwell > 0. {
                self.event("dwell", json!({"seconds":dwell}));
                self.add_time(dwell * 1000.)?;
            }
            let finish = if self.return_initial {
                cycle.initial.max(retract)
            } else {
                retract
            };
            let tapping = [74, 84].contains(&code);
            let variant = self.motion_variant.unwrap_or(f64::from(code));
            if tapping {
                self.event("spindle-reverse", json!({"cycle": variant}));
            } else if code == 86 {
                self.event("spindle-stop", json!({"cycle": variant}));
            }
            self.cycle_leg(n, finish, ![74, 84, 85, 89].contains(&code))?;
            if tapping || code == 86 {
                self.event("spindle-restore", json!({"cycle": variant}));
            }
            target[n] = finish;
        }
        Ok(())
    }
}
