use crate::{
    annotations,
    camera::Camera,
    render::{Draw, Plot},
};
use cnc_core::{
    analysis::{point, sweeps},
    engine::{Model, Segment},
    geometry::Point,
};
use serde_json::{json, Value};
use std::{borrow::Cow, fmt::Write, path::Path};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
type Bounds = (Point, Point);

fn bounds(value: &Value) -> Result<Option<Bounds>> {
    if value.is_null() {
        return Ok(None);
    }
    let (min, max) = (point(&value["min"])?, point(&value["max"])?);
    if min.iter().zip(max).any(|(min, max)| *min > max)
        || min
            .iter()
            .chain(&max)
            .any(|coordinate| coordinate.abs() > 1e12)
    {
        return Err("View bounds must be ordered and within the display range".into());
    }
    Ok(Some((min, max)))
}

fn positive(value: &Value, fallback: f64) -> Result<f64> {
    if value.is_null() {
        return Ok(fallback);
    }
    value
        .as_f64()
        .filter(|v| *v > 0. && v.is_finite())
        .ok_or_else(|| "View zoom and radius must be positive finite numbers".into())
}

fn step(value: &Value, default: usize, count: usize) -> Result<usize> {
    if value.is_null() {
        return Ok(default);
    }
    value
        .as_u64()
        .and_then(|n| usize::try_from(n).ok())
        .filter(|n| *n > 0 && *n <= count)
        .ok_or_else(|| "View step is outside the executed prefix".into())
}

fn intersects(segment: &Segment, (min, max): Bounds, padding: f64) -> bool {
    (0..3).all(|i| {
        let margin = if i == 2 { 0. } else { padding };
        segment.start[i].max(segment.end[i]) + margin >= min[i]
            && segment.start[i].min(segment.end[i]) - margin <= max[i]
    })
}

pub fn capture(
    plot: &mut Plot,
    model: &Model,
    job: &Value,
    view: &Value,
    finding: Option<&Value>,
    path: &Path,
) -> Result<Value> {
    if !view.is_object() {
        return Err("Each view must be an object".into());
    }
    for key in ["showRapid", "showGrid", "showPoints"] {
        if !view[key].is_null() && !view[key].is_boolean() {
            return Err(format!("View {key} must be boolean").into());
        }
    }
    let dimension = |key, default| -> Result<u32> {
        if view[key].is_null() {
            return Ok(default);
        }
        view[key]
            .as_u64()
            .filter(|v| (1..=8192).contains(v))
            .map(|v| v as u32)
            .ok_or_else(|| "PNG dimensions must be positive integers up to 8192".into())
    };
    let width = dimension("width", 1280)?;
    let height = dimension("height", 900)?;
    if width * height > 16_000_000 {
        return Err("PNG dimensions exceed render budget".into());
    }
    let settings = &view["camera"];
    if !settings.is_null() && !settings.is_object() {
        return Err("View camera must be an object".into());
    }
    for value in [&view["angle"], &settings["angle"]] {
        if !value.is_null() && !value.is_string() {
            return Err("View angle must be a name".into());
        }
    }
    let angle = settings["angle"]
        .as_str()
        .or_else(|| view["angle"].as_str())
        .unwrap_or("iso");
    if !["iso", "top", "bottom", "front", "back", "left", "right"].contains(&angle) {
        return Err("Invalid camera angle".into());
    }
    let mut camera = Camera::default();
    camera.view(angle);
    if !settings["direction"].is_null() {
        let [x, y, z] = point(&settings["direction"])?;
        if [x, y, z] == [0.; 3] {
            return Err("Camera direction cannot be zero".into());
        }
        camera.view_direction([x, y, z]);
    }

    let witness = finding.and_then(|finding| point(&finding["point"]).ok());
    let radius = positive(&view["radius"], 10.)?;
    let focus = bounds(&settings["bounds"])?
        .or_else(|| witness.map(|p| (p.map(|v| v - radius), p.map(|v| v + radius))));
    let region = bounds(&view["region"])?.or(focus);
    let count = model.executions.len();
    let requested_step = step(
        &view["step"],
        finding
            .and_then(|f| f["step"].as_u64())
            .map_or(count, |step| step as usize),
        count,
    )?;
    let from = step(&view["fromStep"], 1, count)?;
    let to = step(&view["toStep"], requested_step, count)?.min(requested_step);
    if count > 0 && from > to {
        return Err("View step range is reversed".into());
    }
    let cutter = if view["cutter"] == false {
        Cow::Owned(Value::Null)
    } else if view["cutter"].is_object() {
        Cow::Borrowed(&view["cutter"])
    } else if job["cutter"].is_object() {
        Cow::Borrowed(&job["cutter"])
    } else if view["cutter"] == true {
        Cow::Owned(json!({"diameter":8.}))
    } else {
        Cow::Owned(Value::Null)
    };
    let show_trail = !cutter.is_null();
    let padding = if show_trail {
        cutter["tools"]
            .as_object()
            .into_iter()
            .flat_map(|tools| tools.values())
            .filter_map(|tool| tool["diameter"].as_f64())
            .chain(
                model
                    .segments
                    .iter()
                    .filter_map(|segment| segment.tool_diameter),
            )
            .fold(cutter["diameter"].as_f64().unwrap_or(0.), f64::max)
            / 2.
    } else {
        0.
    };
    let mut filtered = Model::default();
    filtered.segments.extend(
        model
            .segments
            .iter()
            .filter(|segment| {
                (from..=to).contains(&segment.step)
                    && region.is_none_or(|region| intersects(segment, region, padding))
            })
            .cloned(),
    );
    for p in filtered
        .segments
        .iter()
        .flat_map(|segment| [segment.start, segment.end])
    {
        let (min, max) = filtered.bounds.get_or_insert((p, p));
        for i in 0..3 {
            min[i] = min[i].min(p[i]);
            max[i] = max[i].max(p[i]);
        }
    }
    if show_trail {
        let resolved = sweeps(&filtered, &json!({"cutter":cutter}))?;
        for sweep in resolved {
            filtered.segments[sweep.witness].tool_diameter = Some(sweep.diameter);
        }
    }
    if let Some((min, max)) = focus {
        camera.fit_box(min, max, width.into(), height.into());
    } else {
        camera.fit(
            filtered.bounds.or(model.bounds),
            width.into(),
            height.into(),
        );
    }
    if !settings["center"].is_null() {
        camera.center = point(&settings["center"])?;
    }
    if !settings["zoom"].is_null() {
        camera.scale = 200. / (f64::from(height) * positive(&settings["zoom"], 1.)?);
    }
    camera.scale /= positive(&settings["zoomFactor"], 1.)?;
    if !(1e-8..=1e12).contains(&camera.scale) || camera.center.iter().any(|v| v.abs() > 1e12) {
        return Err("Camera scale or center exceeds the display range".into());
    }
    plot.load(&filtered)?;
    if show_trail {
        plot.trail(&filtered, 8.)?;
    }
    let size = [width, height];
    plot.render(
        size,
        1.,
        &camera,
        Draw {
            rapid: view["showRapid"] != false,
            grid: view["showGrid"] != false,
            trail: crate::trail::Mode::from_options(show_trail, view["opaqueTrail"] == true),
            points: view["showPoints"] == true,
            segments: filtered.segments.len(),
            live: Default::default(),
        },
    );
    let default_label = witness
        .and_then(|point| finding.map(|finding| json!([{"id":finding["id"],"point":point}])));
    let labels = annotations::labels(
        model,
        if view["labels"].is_null() {
            default_label.as_ref().unwrap_or(&Value::Null)
        } else {
            &view["labels"]
        },
        from,
        to,
    )?;
    plot.annotate(&camera, size, &labels)?;
    plot.png(path)?;
    Ok(json!({
        "schema":"cnc-evidence/1", "image":path, "step": if to==0 {None} else {Some(to)},
        "fromStep":from,"toStep":to,"segments":filtered.segments.len(),
        "scope":if from==1 && to==count && model.status=="complete" {"full"} else {"prefix or range"},
        "region":region.map(|(min,max)|json!({"min":min,"max":max})),
        "labels":labels,"camera":{"angle":angle,"direction":camera.orbit_direction(),"center":camera.center,
            "millimetersPerPixel":camera.scale,"width":width,"height":height}
    }))
}

pub fn section_svg(stock: &Value, section: &Value) -> Result<String> {
    let (min, max) = bounds(&stock["bounds"])?.ok_or("Stock section needs bounds")?;
    let axis = if section["axis"] == "x" { 1 } else { 0 };
    let mut line = String::new();
    for sample in section["points"]
        .as_array()
        .ok_or("Stock section needs samples")?
    {
        let coordinate = sample["coordinate"]
            .as_f64()
            .ok_or("Invalid section coordinate")?;
        let height = sample["height"].as_f64().ok_or("Invalid section height")?;
        let x = 50. + (coordinate - min[axis]) / (max[axis] - min[axis]) * 900.;
        let y = 250. - (height - min[2]) / (max[2] - min[2]) * 210.;
        write!(line, "{x:.3},{y:.3} ")?;
    }
    let sampled = section["sampledAt"]
        .as_f64()
        .ok_or("Invalid section plane")?;
    let name = if axis == 1 { "X" } else { "Y" };
    let left = min[axis];
    let right = max[axis];
    let top = max[2];
    let bottom = min[2];
    Ok(format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1000 300">
<rect width="1000" height="300" fill="white"/>
<path d="M50 250H950M50 40V250" fill="none" stroke="#555"/>
<polyline points="{line}" fill="none" stroke="#287c63" stroke-width="3"/>
<g font-family="sans-serif" font-size="14" fill="#222">
<text x="50" y="25">{name} = {sampled:.3} mm</text><text x="50" y="278">{left} mm</text>
<text x="890" y="278">{right} mm</text><text x="5" y="45">{top}</text><text x="5" y="250">{bottom}</text>
</g></svg>"##
    ))
}
