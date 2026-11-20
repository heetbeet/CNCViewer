use super::{append_failure, sweep_for};
use crate::{
    engine::Model,
    error::{NcError, Result},
    geometry::{interpolate, segment_nearest_xy, Point},
    motion::{visits, Selection},
};
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Query {
    id: String,
    #[serde(default)]
    select: Value,
    polygon: Vec<[f64; 2]>,
    z: [f64; 2],
    minimum_clearance: Option<f64>,
}

struct Region {
    vertices: Vec<Point>,
    min: [f64; 2],
    max: [f64; 2],
}

impl Region {
    fn new(mut vertices: Vec<[f64; 2]>) -> Result<Self> {
        if vertices.len() > 1 && vertices.first() == vertices.last() {
            vertices.pop();
        }
        if !(3..=256).contains(&vertices.len())
            || vertices
                .iter()
                .flatten()
                .any(|v| !v.is_finite() || v.abs() > 1e9)
        {
            return Err(NcError::new(
                "Clearance polygon needs 3 to 256 finite XY vertices within ±1e9 mm",
            ));
        }
        let vertices: Vec<_> = vertices.into_iter().map(|[x, y]| [x, y, 0.]).collect();
        let mut region = Self {
            min: [f64::INFINITY; 2],
            max: [f64::NEG_INFINITY; 2],
            vertices,
        };
        let count = region.vertices.len();
        let [x, y, _] = region.vertices[0];
        let mut area = 0.;
        for (i, (a, b)) in region.edges().enumerate() {
            if a == b {
                return Err(NcError::new("Clearance polygon has a zero-length edge"));
            }
            let c = region.vertices[(i + 2) % count];
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let (ex, ey) = (c[0] - b[0], c[1] - b[1]);
            if dx * ey == dy * ex && dx * ex + dy * ey < 0. {
                return Err(NcError::new("Clearance polygon has a backtracking edge"));
            }
            area += (a[0] - x) * (b[1] - y) - (b[0] - x) * (a[1] - y);
            for (j, (c, d)) in region.edges().enumerate().skip(i + 2) {
                if !(i == 0 && j + 1 == count) && segment_nearest_xy(a, b, c, d).0 == 0. {
                    return Err(NcError::new("Clearance polygon must be simple"));
                }
            }
        }
        if area == 0. {
            return Err(NcError::new("Clearance polygon has zero area"));
        }
        for vertex in &region.vertices {
            for (i, coordinate) in vertex[..2].iter().enumerate() {
                region.min[i] = region.min[i].min(*coordinate);
                region.max[i] = region.max[i].max(*coordinate);
            }
        }
        Ok(region)
    }

    fn edges(&self) -> impl Iterator<Item = (Point, Point)> + '_ {
        self.vertices
            .iter()
            .copied()
            .zip(self.vertices.iter().copied().cycle().skip(1))
    }

    fn lower_bound(&self, start: Point, end: Point) -> f64 {
        let gap: [f64; 2] = std::array::from_fn(|i| {
            (self.min[i] - start[i].max(end[i]))
                .max(start[i].min(end[i]) - self.max[i])
                .max(0.)
        });
        gap[0].hypot(gap[1])
    }

    fn nearest(&self, start: Point, end: Point) -> (f64, Point, [f64; 2]) {
        let mut inside = false;
        let mut nearest = (f64::INFINITY, 0., [0.; 3]);
        for (a, b) in self.edges() {
            if (a[1] > start[1]) != (b[1] > start[1])
                && start[0] < a[0] + (b[0] - a[0]) * (start[1] - a[1]) / (b[1] - a[1])
            {
                inside = !inside;
            }
            let (distance, t, u) = segment_nearest_xy(start, end, a, b);
            if distance < nearest.0 || (distance == nearest.0 && t < nearest.1) {
                nearest = (distance, t, interpolate(a, b, u));
            }
        }
        if inside {
            return (0., start, [start[0], start[1]]);
        }
        (
            nearest.0,
            interpolate(start, end, nearest.1),
            [nearest.2[0], nearest.2[1]],
        )
    }
}

#[derive(Clone, Copy)]
struct Witness {
    clearance: f64,
    point: Point,
    region_point: [f64; 2],
    segment: usize,
}

impl Witness {
    fn report(self, model: &Model, visits: &[usize]) -> Value {
        let segment = &model.segments[self.segment];
        let step_index = segment.step - 1;
        json!({"clearance":self.clearance,"point":self.point,"regionPoint":self.region_point,
            "step":segment.step,"line":segment.line,
            "source":model.executions.get(step_index).map(|s| &s.source),
            "visit":visits.get(step_index),"move":segment.movement+1,"segment":self.segment+1,"tool":segment.tool})
    }
}

fn in_band(start: Point, end: Point, z: [f64; 2]) -> Option<(Point, Point)> {
    if start[2] == end[2] {
        return (z[0]..=z[1]).contains(&start[2]).then_some((start, end));
    }
    let t = z.map(|level| (level - start[2]) / (end[2] - start[2]));
    let (first, last) = (t[0].min(t[1]).max(0.), t[0].max(t[1]).min(1.));
    (first <= last).then(|| {
        (
            interpolate(start, end, first),
            interpolate(start, end, last),
        )
    })
}

pub(super) fn measure(model: &Model, job: &Value, report: &mut Value) -> Result<()> {
    let Some(requests) = job.get("clearance").filter(|v| !v.is_null()) else {
        return Ok(());
    };
    let requests: Vec<Query> = serde_json::from_value(requests.clone())
        .map_err(|error| NcError::new(format!("Invalid clearance query: {error}")))?;
    let budget = &job["limits"]["clearanceTests"];
    let mut remaining = if budget.is_null() {
        5_000_000
    } else {
        budget
            .as_u64()
            .filter(|n| (1..=100_000_000).contains(n))
            .ok_or_else(|| NcError::new("Invalid limits.clearanceTests"))? as usize
    };
    let visits: Vec<_> = visits(model).collect();
    let mut ids = std::collections::BTreeSet::new();
    let mut results = Vec::new();
    for query in requests {
        if query.id.is_empty()
            || !ids.insert(query.id.clone())
            || query.z[0] > query.z[1]
            || query.z.iter().any(|v| !v.is_finite() || v.abs() > 1e9)
            || query.minimum_clearance.is_some_and(|v| !v.is_finite())
        {
            return Err(NcError::new("Clearance queries need unique IDs, an ordered Z band, and a finite minimum clearance"));
        }
        let region = Region::new(query.polygon)?;
        let selection = Selection::resolve(model, &report["operations"], &query.select)?;
        let (mut closest, mut first) = (None::<Witness>, None);
        let mut segments = 0;
        let mut has_feed = false;
        let mut reason = None;
        for (index, segment) in selection.segments(model).filter(|(_, s)| !s.rapid) {
            has_feed = true;
            let Some((start, end)) = in_band(segment.start, segment.end, query.z) else {
                continue;
            };
            segments += 1;
            let sweep = match sweep_for(segment, index, &job["cutter"]) {
                Ok(sweep) => sweep,
                Err(error) => {
                    reason = Some(error.message);
                    break;
                }
            };
            let radius = sweep.diameter / 2.;
            if closest.is_some_and(|best| region.lower_bound(start, end) - radius >= best.clearance)
            {
                continue;
            }
            if remaining < region.vertices.len() {
                reason = Some("Clearance comparison budget reached".into());
                break;
            }
            remaining -= region.vertices.len();
            let (distance, point, region_point) = region.nearest(start, end);
            let gap = distance - radius;
            let witness = Witness {
                clearance: gap,
                point,
                region_point,
                segment: index,
            };
            if first.is_none() && query.minimum_clearance.is_some_and(|min| gap < min - 1e-9) {
                first = Some(witness);
            }
            if closest.is_none_or(|best| gap < best.clearance) {
                closest = Some(witness);
            }
        }
        if !has_feed && query.minimum_clearance.is_some() {
            reason = Some("Selection contains no executed feed segments".into());
        }
        let first_violation = first.map(|witness| witness.report(model, &visits));
        if let Some(mut finding) = first_violation.clone() {
            finding["check"] = json!("cutterClearance");
            finding["query"] = json!(query.id);
            finding["severity"] = json!("error");
            finding["expected"] = json!(query.minimum_clearance);
            append_failure(report, finding);
            if report["validation"] == "passed" {
                report["validation"] = json!("failed");
            }
        }
        let complete = reason.is_none() && matches!(model.status.as_str(), "complete" | "paused");
        let status = if !complete {
            report["validation"] = json!("incomplete");
            if let Some(message) = &reason {
                append_failure(
                    report,
                    json!({"check":"clearanceIncomplete","query":query.id,"severity":"error","message":message}),
                );
            }
            "incomplete"
        } else if first.is_some() {
            "failed"
        } else if closest.is_none() {
            "empty"
        } else if query.minimum_clearance.is_some() {
            "passed"
        } else {
            "measured"
        };
        results.push(json!({"id":query.id,"status":status,"complete":complete,
            "segmentsInBand":segments,"requiredClearance":query.minimum_clearance,
            "minimumClearance":closest.map(|witness| witness.clearance),
            "closest":closest.map(|witness| witness.report(model, &visits)),
            "firstViolation":first_violation,"reason":reason}));
    }
    report["clearance"] = json!(results);
    Ok(())
}
