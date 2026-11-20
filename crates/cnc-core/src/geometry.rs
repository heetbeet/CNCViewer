use std::f64::consts::{PI, TAU};
pub type Point = [f64; 3];

#[derive(Debug, Clone)]
pub struct Arc {
    pub start: Point,
    pub end: Point,
    pub center: Option<Point>,
    pub radius: Option<f64>,
    pub plane: u32,
    pub clockwise: bool,
    pub tolerance: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(i32)]
pub enum ArcError {
    Range = 1,
    Radius = 2,
    Center = 3,
    Circle = 4,
    Tessellation = 5,
    Plane = 6,
    FullCircle = 7,
    ZeroRadius = 8,
}
impl std::fmt::Display for ArcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Range => "Arc coordinates, radius or tolerance are out of range",
            Self::Radius => "Arc radius R is too small to reach the endpoint",
            Self::Center => "Arc needs an R radius or an I/J/K center",
            Self::Circle => "Arc start and end are different distances from the I/J/K center",
            Self::Tessellation => "Arc exceeds the viewer's segment limit at this tolerance",
            Self::Plane => "Arc needs plane G17, G18 or G19",
            Self::FullCircle => "Full-circle arcs need an I/J/K center",
            Self::ZeroRadius => "Arc starts at its I/J/K center",
        })
    }
}
pub fn distance(a: &[f64], b: &[f64]) -> f64 {
    a.iter()
        .zip(b)
        .map(|(a, b)| (b - a).powi(2))
        .sum::<f64>()
        .sqrt()
}
pub fn interpolate(a: Point, b: Point, t: f64) -> Point {
    std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t)
}
pub fn rotate_point(point: Point, angles: Point) -> Point {
    if angles == [0.; 3] {
        return point;
    }
    let [ax, ay, az] = [
        -angles[0] * PI / 180.,
        -angles[1] * PI / 180.,
        angles[2] * PI / 180.,
    ];
    let [mut x, mut y, mut z] = point;
    (y, z) = (y * ax.cos() - z * ax.sin(), y * ax.sin() + z * ax.cos());
    (x, z) = (x * ay.cos() + z * ay.sin(), -x * ay.sin() + z * ay.cos());
    (x, y) = (x * az.cos() - y * az.sin(), x * az.sin() + y * az.cos());
    [x, y, z]
}
pub fn arc_points(a: &Arc) -> Result<Vec<Point>, ArcError> {
    if a.start
        .iter()
        .chain(&a.end)
        .chain(a.center.as_ref().into_iter().flatten())
        .any(|v| !v.is_finite() || v.abs() > 1e9)
        || a.radius.is_some_and(|r| !r.is_finite() || r.abs() > 1e9)
        || !a.tolerance.is_finite()
        || a.tolerance <= 0.
    {
        return Err(ArcError::Range);
    }
    let [h, v, n] = match a.plane {
        17 => [0, 1, 2],
        18 => [2, 0, 1],
        19 => [1, 2, 0],
        _ => return Err(ArcError::Plane),
    };
    let dx = a.end[h] - a.start[h];
    let dy = a.end[v] - a.start[v];
    let chord = dx.hypot(dy);
    let mut center = a.center;
    if let Some(radius) = a.radius {
        let magnitude = radius.abs();
        if chord < 1e-9 {
            return Err(ArcError::FullCircle);
        }
        if magnitude < chord / 2. - 1e-7 {
            return Err(ArcError::Radius);
        }
        let midpoint = interpolate(a.start, a.end, 0.5);
        let offset = (magnitude.powi(2) - (chord / 2.).powi(2)).max(0.).sqrt();
        let direction = if a.clockwise { -1. } else { 1. } * if radius < 0. { -1. } else { 1. };
        let mut c = a.start;
        c[h] = midpoint[h] - dy / chord * offset * direction;
        c[v] = midpoint[v] + dx / chord * offset * direction;
        center = Some(c);
    }
    let center = center.ok_or(ArcError::Center)?;
    let (sx, sy) = (a.start[h] - center[h], a.start[v] - center[v]);
    let (ex, ey) = (a.end[h] - center[h], a.end[v] - center[v]);
    let radius = sx.hypot(sy);
    if radius < 1e-9 {
        return Err(ArcError::ZeroRadius);
    }
    if (radius - ex.hypot(ey)).abs() > a.tolerance.max(radius * 0.001) {
        return Err(ArcError::Circle);
    }
    let angle = sy.atan2(sx);
    let mut sweep = (ey.atan2(ex) - angle) % TAU;
    if chord < 1e-9 {
        sweep = if a.clockwise { -TAU } else { TAU }
    } else if a.clockwise && sweep >= 0. {
        sweep -= TAU
    } else if !a.clockwise && sweep <= 0. {
        sweep += TAU
    }
    let max_step = (PI / 36.).min(2. * (-1f64).max(1. - a.tolerance / radius).acos());
    let count = (sweep.abs() / max_step).ceil().max(2.);
    if !count.is_finite() || count > 16384. {
        return Err(ArcError::Tessellation);
    }
    let count = count as usize;
    let mut result = Vec::with_capacity(count + 1);
    result.push(a.start);
    for step in 1..count {
        let fraction = step as f64 / count as f64;
        let theta = angle + sweep * fraction;
        let mut p = a.start;
        p[h] = center[h] + theta.cos() * radius;
        p[v] = center[v] + theta.sin() * radius;
        p[n] = a.start[n] + (a.end[n] - a.start[n]) * fraction;
        result.push(p);
    }
    result.push(a.end);
    Ok(result)
}

pub fn segment_distance_xy(a: Point, b: Point, c: Point, d: Point) -> f64 {
    segment_nearest_xy(a, b, c, d).0
}

// Distance and interpolation factors on AB and CD.
pub(crate) fn segment_nearest_xy(a: Point, b: Point, c: Point, d: Point) -> (f64, f64, f64) {
    fn point_segment(p: Point, a: Point, b: Point) -> (f64, f64) {
        let dx = b[0] - a[0];
        let dy = b[1] - a[1];
        let den = dx * dx + dy * dy;
        let t = if den > 0. {
            ((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / den
        } else {
            0.
        }
        .clamp(0., 1.);
        ((p[0] - a[0] - t * dx).hypot(p[1] - a[1] - t * dy), t)
    }
    let ab = [b[0] - a[0], b[1] - a[1]];
    let cd = [d[0] - c[0], d[1] - c[1]];
    let ac = [c[0] - a[0], c[1] - a[1]];
    let cross = |[x, y]: [f64; 2], [u, v]: [f64; 2]| x * v - y * u;
    let denominator = cross(ab, cd);
    if denominator != 0. {
        let t = cross(ac, cd) / denominator;
        let u = cross(ac, ab) / denominator;
        if (0. ..=1.).contains(&t) && (0. ..=1.).contains(&u) {
            return (0., t, u);
        }
    }
    let (da, ua) = point_segment(a, c, d);
    let (db, ub) = point_segment(b, c, d);
    let (dc, tc) = point_segment(c, a, b);
    let (dd, td) = point_segment(d, a, b);
    [(da, 0., ua), (db, 1., ub), (dc, tc, 0.), (dd, td, 1.)]
        .into_iter()
        .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)))
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn planes_and_exact_endpoints() {
        for (plane, start, end) in [
            (17, [1., 0., 0.], [0., 1., 3.]),
            (18, [0., 0., 1.], [1., 3., 0.]),
            (19, [0., 1., 0.], [3., 0., 1.]),
        ] {
            let a = Arc {
                start,
                end,
                center: Some([0.; 3]),
                radius: None,
                plane,
                clockwise: false,
                tolerance: 0.05,
            };
            let points = arc_points(&a).unwrap();
            assert_eq!(points.len(), 19);
            assert_eq!(points[0], start);
            assert_eq!(*points.last().unwrap(), end);
        }
    }
    #[test]
    fn major_radius_and_errors() {
        let mut a = Arc {
            start: [1., 0., 0.],
            end: [0., 1., 0.],
            center: None,
            radius: Some(-1.),
            plane: 17,
            clockwise: false,
            tolerance: 0.05,
        };
        assert!(arc_points(&a).unwrap().len() > 50);
        a.radius = Some(0.1);
        assert_eq!(arc_points(&a), Err(ArcError::Radius));
        a.radius = None;
        assert_eq!(arc_points(&a), Err(ArcError::Center));
    }
    #[test]
    fn rotations_and_intersections() {
        let p = [1., 2., 3.];
        assert!(
            (distance(&rotate_point(p, [23., 41., 99.]), &[0.; 3]) - distance(&p, &[0.; 3])).abs()
                < 1e-12
        );
        assert_eq!(
            segment_distance_xy([0., 0., 0.], [10., 10., 0.], [0., 10., 0.], [10., 0., 0.]),
            0.
        );
        assert_eq!(
            segment_distance_xy([0.; 3], [0.; 3], [3., 4., 0.], [3., 4., 0.]),
            5.
        );
    }
}
