use crate::{
    engine::{Model, Movement, Segment},
    error::{NcError, Result},
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;

#[derive(Deserialize)]
#[serde(untagged)]
enum Request {
    Operation(Operation),
    Steps(Steps),
    Lines(Lines),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Operation {
    operation: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Steps {
    from_step: usize,
    to_step: usize,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Lines {
    source: Option<String>,
    from_line: usize,
    to_line: usize,
    visit: Option<usize>,
}

pub struct Selection {
    first: usize,
    last: usize,
    steps: Option<Vec<usize>>,
}

impl Selection {
    pub fn resolve(model: &Model, operations: &Value, request: &Value) -> Result<Self> {
        let count = model.executions.len();
        if request.is_null() {
            return Ok(Self {
                first: 1,
                last: count,
                steps: None,
            });
        }
        let request: Request = serde_json::from_value(request.clone()).map_err(|_| {
            NcError::new("select requires operation, fromStep and toStep, or fromLine and toLine")
        })?;
        let (first, last) = match request {
            Request::Steps(range) => (range.from_step, range.to_step),
            Request::Operation(request) => {
                let operation = operations
                    .as_array()
                    .into_iter()
                    .flatten()
                    .find(|item| item["id"] == request.operation)
                    .ok_or_else(|| {
                        NcError::new(format!("Unknown operation {}", request.operation))
                    })?;
                (
                    operation["fromStep"].as_u64().ok_or_else(|| {
                        NcError::new(format!("Operation {} was not reached", request.operation))
                    })? as usize,
                    operation["toStep"]
                        .as_u64()
                        .ok_or_else(|| NcError::new("Operation end was not reached"))?
                        as usize,
                )
            }
            Request::Lines(range) => {
                if range.from_line == 0 || range.to_line < range.from_line || range.visit == Some(0)
                {
                    return Err(NcError::new(
                        "select requires an ordered positive line range and visit",
                    ));
                }
                let source = range
                    .source
                    .as_deref()
                    .or_else(|| model.executions.first().map(|s| s.source.as_str()));
                let mut visits = HashMap::new();
                let steps: Vec<_> = model
                    .executions
                    .iter()
                    .filter(|s| {
                        Some(s.source.as_str()) == source
                            && (range.from_line..=range.to_line).contains(&s.line)
                    })
                    .filter(|s| {
                        range.visit.is_none_or(|visit| {
                            let seen = visits.entry(s.line).or_insert(0);
                            *seen += 1;
                            *seen == visit
                        })
                    })
                    .map(|s| s.step)
                    .collect();
                return Ok(Self {
                    first: steps.first().copied().unwrap_or(1),
                    last: steps.last().copied().unwrap_or(0),
                    steps: Some(steps),
                });
            }
        };
        if first == 0 || first > last || last > count {
            return Err(NcError::new(
                "Selected step range is outside the executed prefix or reversed",
            ));
        }
        Ok(Self {
            first,
            last,
            steps: None,
        })
    }

    pub fn contains(&self, step: usize) -> bool {
        (self.first..=self.last).contains(&step)
            && self
                .steps
                .as_ref()
                .is_none_or(|steps| steps.binary_search(&step).is_ok())
    }

    pub fn bounds(&self) -> Option<(usize, usize)> {
        (self.first <= self.last).then_some((self.first, self.last))
    }

    pub fn moves<'a>(&'a self, model: &'a Model) -> impl Iterator<Item = (usize, &'a Movement)> {
        let start = model.moves.partition_point(|m| m.step < self.first);
        let end = model
            .moves
            .partition_point(|m| m.step <= self.last)
            .max(start);
        model.moves[start..end]
            .iter()
            .enumerate()
            .filter(move |(_, movement)| self.contains(movement.step))
            .map(move |(index, movement)| (index + start, movement))
    }

    pub fn segments<'a>(&'a self, model: &'a Model) -> impl Iterator<Item = (usize, &'a Segment)> {
        let start = model.segments.partition_point(|s| s.step < self.first);
        let end = model
            .segments
            .partition_point(|s| s.step <= self.last)
            .max(start);
        model.segments[start..end]
            .iter()
            .enumerate()
            .filter(move |(_, segment)| self.contains(segment.step))
            .map(move |(index, segment)| (index + start, segment))
    }

    pub fn summary(&self, model: &Model) -> Value {
        let mut counts = [0; 2];
        let mut lengths = [0.; 2];
        let mut tools = std::collections::BTreeSet::new();
        for (_, movement) in self.moves(model) {
            let kind = usize::from(movement.rapid);
            counts[kind] += 1;
            lengths[kind] += movement.length;
            tools.insert(movement.tool);
        }
        let mut bounds = None;
        for (_, segment) in self.segments(model) {
            let (min, max) = bounds.get_or_insert((segment.start, segment.start));
            for i in 0..3 {
                min[i] = min[i].min(segment.start[i]).min(segment.end[i]);
                max[i] = max[i].max(segment.start[i]).max(segment.end[i]);
            }
        }
        json!({
            "fromStep": self.bounds().map(|(first, _)| first),
            "toStep": self.bounds().map(|(_, last)| last),
            "steps": self.steps.as_ref().map_or(
                self.bounds().map_or(0, |(first, last)| last - first + 1), Vec::len),
            "moves": counts.iter().sum::<usize>(), "feedMoves": counts[0], "rapidMoves": counts[1],
            "feedLength": lengths[0], "rapidLength": lengths[1], "tools": tools,
            "bounds": bounds.map(|(min, max)| json!({"min": min, "max": max})),
        })
    }
}

pub fn visits(model: &Model) -> impl Iterator<Item = usize> + '_ {
    let mut counts = HashMap::new();
    model.executions.iter().map(move |s| {
        let n = counts.entry((s.source.as_str(), s.line)).or_insert(0);
        *n += 1;
        *n
    })
}
