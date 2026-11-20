use serde::Serialize;

pub const UNASSIGNED_VARIABLE: &str = "UNASSIGNED_VARIABLE";

#[derive(Debug, Clone, Serialize)]
pub struct SourceLocation {
    pub source: String,
    pub line: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct NcError {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requirement: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alarm: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<Box<SourceLocation>>,
}
pub type Result<T> = std::result::Result<T, NcError>;
impl NcError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            code: "PROGRAM".into(),
            message: message.into(),
            requirement: None,
            column: None,
            alarm: None,
            location: None,
        }
    }
    pub fn code(mut self, code: &str) -> Self {
        self.code = code.into();
        self
    }
    pub fn required(name: impl Into<String>) -> Self {
        let name = name.into();
        let (group, item) = name.split_once('.').unwrap_or((&name, ""));
        let message = match group {
            "workOffsets" => {
                let (code, slot) = item.split_once(':').unwrap_or((item, ""));
                if slot.is_empty() {
                    format!("Work offset G{code} is missing from machine setup")
                } else {
                    format!("Work offset G{code} P{slot} is missing from machine setup")
                }
            }
            "referencePositions" => match item.split_once(':') {
                Some((code, slot)) => {
                    format!("Reference position G{code} P{slot} is missing from machine setup")
                }
                None => format!("Reference position G{item} is missing from machine setup"),
            },
            "tools" => match item.split_once('.') {
                Some((tool, field)) => format!("Tool {tool} {field} is missing from machine setup"),
                None => format!("Tool {item} is missing from machine setup"),
            },
            "subprograms" => format!("Subprogram {item} is not loaded"),
            "customMCodes" => format!("M{item} needs a machine setup mapping"),
            "axisMapping" => format!("Axis {item} needs a machine setup mapping"),
            "signalMapping" => format!("Signal mapping for {item} is missing from machine setup"),
            "inputs" => format!("Input signal {item} is missing from machine setup"),
            "outputs" => format!("Output signal {item} is missing from machine setup"),
            "machineSettings" => format!("Machine setting '{item}' is missing from setup"),
            "systemVariables" => format!("System variable #{item} is missing from machine setup"),
            "systemVariableWrites" => format!("Writing #{item} needs a machine setup mapping"),
            "parameters" => format!("Parameter {item} is missing from machine setup"),
            "probes" => match item.split_once('.') {
                Some((probe, "plane")) => format!("Probe {probe} needs a target plane"),
                Some((probe, result)) => format!(
                    "Probe {probe} result {} is missing",
                    result.trim_start_matches("results.")
                ),
                None => format!("Probe {item} is missing from machine setup"),
            },
            "probeMeasurement" => "No probe measurement is available".into(),
            "initialFeed" => "Cutting move needs a positive feed rate F".into(),
            "clock" => "Program clock is missing from machine setup".into(),
            "manualRetract" => "G88 needs a manual retract to continue".into(),
            "cycleImplementations" => format!("{item} cycle is unsupported in CNC Viewer"),
            _ => format!("Missing machine setup value: {name}"),
        };
        Self {
            code: "INPUT_REQUIRED".into(),
            message,
            requirement: Some(name),
            column: None,
            alarm: None,
            location: None,
        }
    }
    pub(crate) fn at(mut self, source: &str, line: usize) -> Self {
        self.location = Some(Box::new(SourceLocation {
            source: source.into(),
            line,
        }));
        self
    }
}
impl std::fmt::Display for NcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(location) = &self.location {
            write!(f, "{}:{} ", location.source, location.line)?;
        }
        f.write_str(&self.message)
    }
}
impl std::error::Error for NcError {}
pub fn integer(value: f64, label: &str, min: u32, max: u32) -> Result<u32> {
    if value.is_finite() && value.fract() == 0. && value >= min as f64 && value <= max as f64 {
        Ok(value as u32)
    } else {
        Err(NcError::new(format!(
            "{label} must be an integer between {min} and {max}."
        )))
    }
}
