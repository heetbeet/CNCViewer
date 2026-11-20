use super::*;
impl Engine {
    pub(super) fn probe(&mut self, words: &Words, code: f64) -> Result<()> {
        let number = ((code - 31.) * 10.).round() as u32 + 1;
        let config = self.setup["probes"][number.to_string()].clone();
        if config.is_null() {
            return Err(NcError::required(format!("probes.{number}")));
        }
        let start = self.command;
        let target = self.target(words, false)?;
        let mut fraction = 1.;
        let mut triggered = false;
        if !config["plane"].is_null() {
            let axis = config["plane"]["axis"]
                .as_str()
                .and_then(|s| ["X", "Y", "Z"].iter().position(|a| *a == s))
                .ok_or_else(|| NcError::new("Probe plane axis must be X, Y or Z"))?;
            let position = config["plane"]["position"]
                .as_f64()
                .filter(|v| v.is_finite())
                .ok_or_else(|| NcError::new("Probe plane position must be finite"))?;
            let radius = number_value(&config["stylusRadius"], 0.)?;
            if radius < 0. {
                return Err(NcError::new("Probe stylus radius must be nonnegative"));
            }
            let delta = target[axis] - start[axis];
            let crossing = (position - delta.signum() * radius - start[axis]) / delta;
            if delta != 0. && (0. ..=1.).contains(&crossing) {
                fraction = crossing;
                triggered = true;
            }
        } else if let Some(results) = config["results"].as_array() {
            let index = *self.probe_reads.get(&number).unwrap_or(&0);
            let result = results
                .get(index)
                .ok_or_else(|| NcError::required(format!("probes.{number}.results.{index}")))?;
            if result["triggered"] != false {
                if result["position"].as_array().is_none_or(|a| a.len() != 3) {
                    return Err(NcError::new("Probe result needs three program coordinates"));
                }
                let hit = pose(&result["position"])?;
                let vector: [f64; 3] = std::array::from_fn(|i| target[i] - start[i]);
                let squared = vector.iter().map(|v| v * v).sum::<f64>();
                if squared == 0. {
                    return Err(NcError::new("Probe measurement requires nonzero travel"));
                }
                fraction = (0..3).map(|i| vector[i] * (hit[i] - start[i])).sum::<f64>() / squared;
                let projected: [f64; 3] = std::array::from_fn(|i| start[i] + vector[i] * fraction);
                if !(0. ..=1.).contains(&fraction)
                    || distance(&projected, &[hit[0], hit[1], hit[2]]) > 0.001
                {
                    return Err(NcError::new("Probe hit is not on the commanded segment"));
                }
                triggered = true;
            }
            self.probe_reads.insert(number, index + 1);
        } else {
            return Err(NcError::required(format!("probes.{number}.plane")));
        }
        let end = std::array::from_fn(|i| start[i] + (target[i] - start[i]) * fraction);
        self.linear(end, false, false)?;
        self.probe_part = Some(self.part_position()?);
        let mut gauge = self.position;
        gauge[2] += self.length()?;
        self.probe_machine = Some(gauge);
        if let Some(input) = config["input"].as_u64() {
            if self.setup["inputs"].is_null() {
                self.setup["inputs"] = json!({});
            }
            self.setup["inputs"][input.to_string()] = json!(u32::from(triggered));
        }
        self.event(
            "probe",
            json!({"probe":number,"triggered":triggered,"position":end,"fraction":fraction}),
        );
        Ok(())
    }
}
fn number_value(value: &Value, default: f64) -> Result<f64> {
    if value.is_null() {
        Ok(default)
    } else {
        value
            .as_f64()
            .filter(|v| v.is_finite())
            .ok_or_else(|| NcError::new("Probe values must be finite numbers"))
    }
}
