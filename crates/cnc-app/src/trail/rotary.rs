use cnc_core::geometry::{
    distance, interpolate, rotary_turn, rotate_point, table_bend, Point, RotaryMode,
    ROTARY_CHORD_ERROR_MM,
};
use std::sync::OnceLock;

const RIM: usize = 32;
const MAX_INTERVALS: usize = 512;

pub(super) fn sub(a: Point, b: Point) -> Point {
    std::array::from_fn(|i| a[i] - b[i])
}

pub(super) fn dot(a: Point, b: Point) -> f64 {
    a.into_iter().zip(b).map(|(a, b)| a * b).sum()
}

pub(super) fn cross(a: Point, b: Point) -> Point {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

pub(super) struct Frame {
    pub center: Point,
    basis: [Point; 3],
}

impl Frame {
    pub fn new(tip: Point, angles: Point, mode: RotaryMode, origin: Point) -> Self {
        let basis = [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]].map(|v| rotate_point(v, angles));
        Self {
            center: sub(mode.tip(tip, angles), origin),
            basis,
        }
    }

    pub fn local(&self, offset: Point) -> Point {
        self.basis.map(|axis| dot(axis, offset))
    }

    pub fn point(&self, [x, y, z]: Point) -> Point {
        std::array::from_fn(|i| {
            self.center[i] + x * self.basis[0][i] + y * self.basis[1][i] + z * self.basis[2][i]
        })
    }

    pub fn horizontal(&self) -> bool {
        self.basis[2][0].abs() + self.basis[2][1].abs() < 1e-10
    }

    fn rim(&self, radius: f64) -> [Point; RIM] {
        static CIRCLE: OnceLock<[[f64; 2]; RIM]> = OnceLock::new();
        CIRCLE
            .get_or_init(|| {
                std::array::from_fn(|i| {
                    let (sin, cos) = (std::f64::consts::TAU * i as f64 / RIM as f64).sin_cos();
                    [cos, sin]
                })
            })
            .map(|[x, y]| self.point([x * radius, y * radius, 0.]))
    }
}

pub(super) fn intervals(
    a: Point,
    b: Point,
    angles: [Point; 2],
    radius: f64,
    mode: RotaryMode,
    spin: bool,
) -> Result<usize, String> {
    // Euler angles are interpolated as programmed, including complete turns.
    let turn = rotary_turn(angles[0], angles[1]);
    let tilt = if spin { 0. } else { turn };
    let bend = if mode == RotaryMode::Table {
        table_bend(a, b, turn)
    } else {
        0.
    };
    // Second derivative bound for chord error. The linear bound controls the
    // extra space a convex hull can include while a disc rotates about its tip.
    let chord = ((bend + radius * tilt * tilt) / (8. * ROTARY_CHORD_ERROR_MM)).sqrt();
    let loft = radius * tilt / (2. * ROTARY_CHORD_ERROR_MM);
    let count = chord.max(loft).ceil().max(1.) as usize;
    if count > MAX_INTERVALS {
        return Err("Rotary cutter sweep exceeds the angular resolution budget. Plot a shorter or more finely segmented move.".into());
    }
    Ok(count)
}

#[derive(Clone, Copy)]
struct Face {
    indices: [usize; 3],
    normal: Point,
}

impl Face {
    fn new(points: &[Point], mut indices: [usize; 3], inside: Point) -> Option<Self> {
        let [a, b, c] = indices.map(|i| points[i]);
        let mut normal = cross(sub(b, a), sub(c, a));
        let length = dot(normal, normal).sqrt();
        if length == 0. {
            return None;
        }
        if dot(normal, sub(inside, a)) > 0. {
            indices.swap(1, 2);
            normal = normal.map(|v| -v);
        }
        Some(Self {
            indices,
            normal: normal.map(|v| v / length),
        })
    }

    fn distance(self, points: &[Point], point: Point) -> f64 {
        dot(self.normal, sub(point, points[self.indices[0]]))
    }
}

pub(super) fn bridge(first: &Frame, last: &Frame, radius: f64, mut emit: impl FnMut([Point; 3])) {
    let mut points = [first.rim(radius), last.rim(radius)].concat();
    // Use coordinates near the first centre for the hull predicates.
    for point in &mut points {
        *point = sub(*point, first.center);
    }
    let epsilon = 1e-9 * (radius + distance(&first.center, &last.center)).max(1.);
    let a = points[0];
    let furthest = |distance: &dyn Fn(Point) -> f64| {
        (1..points.len())
            .max_by(|&b, &c| distance(points[b]).total_cmp(&distance(points[c])))
            .unwrap()
    };
    let b = furthest(&|p| dot(sub(p, a), sub(p, a)));
    let line = sub(points[b], a);
    let c = furthest(&|p| {
        let out = cross(line, sub(p, a));
        dot(out, out)
    });
    let normal = cross(line, sub(points[c], a));
    let d = furthest(&|p| dot(normal, sub(p, a)).abs());
    let mut output = |indices: [usize; 3]| {
        emit(indices.map(|index| std::array::from_fn(|i| points[index][i] + first.center[i])))
    };
    if dot(normal, sub(points[d], a)).abs() <= epsilon * dot(normal, normal).sqrt() {
        // Coplanar rims still sweep an area when the centre translates.
        let uv: Vec<_> = points.iter().map(|p| first.local(*p)).collect();
        let mut sorted: Vec<_> = (0..points.len()).collect();
        sorted.sort_unstable_by(|&a, &b| {
            uv[a][0]
                .total_cmp(&uv[b][0])
                .then(uv[a][1].total_cmp(&uv[b][1]))
        });
        let mut outline = Vec::with_capacity(points.len());
        for reverse in [false, true] {
            let first = outline.len();
            for i in 0..sorted.len() {
                let p = sorted[if reverse { sorted.len() - 1 - i } else { i }];
                while outline.len() >= first + 2 {
                    let [a, b] = [
                        uv[outline[outline.len() - 2]],
                        uv[outline[outline.len() - 1]],
                    ];
                    if cross(sub(b, a), sub(uv[p], a))[2] > 0. {
                        break;
                    }
                    outline.pop();
                }
                outline.push(p);
            }
            outline.pop();
        }
        for edge in outline.get(1..).unwrap_or_default().windows(2) {
            output([outline[0], edge[0], edge[1]]);
        }
        return;
    }
    let seed = [0, b, c, d];
    let inside = std::array::from_fn(|i| seed.iter().map(|&p| points[p][i]).sum::<f64>() / 4.);
    let mut faces: Vec<_> = [[0, b, c], [0, b, d], [0, c, d], [b, c, d]]
        .into_iter()
        .filter_map(|indices| Face::new(&points, indices, inside))
        .collect();
    let mut boundary = Vec::<(usize, usize)>::new();
    for (index, &point) in points.iter().enumerate() {
        if seed.contains(&index) {
            continue;
        }
        boundary.clear();
        faces.retain(|face| {
            if face.distance(&points, point) <= epsilon {
                return true;
            }
            let [a, b, c] = face.indices;
            for (a, b) in [(a, b), (b, c), (c, a)] {
                if let Some(position) = boundary.iter().position(|edge| *edge == (b, a)) {
                    boundary.swap_remove(position);
                } else {
                    boundary.push((a, b));
                }
            }
            false
        });
        faces.extend(
            boundary
                .iter()
                .filter_map(|&(a, b)| Face::new(&points, [a, b, index], inside)),
        );
    }
    for face in faces {
        output(face.indices);
    }
}

pub(super) fn sample(
    a: Point,
    b: Point,
    angles: [Point; 2],
    mode: RotaryMode,
    origin: Point,
    t: f64,
) -> Frame {
    Frame::new(
        interpolate(a, b, t),
        interpolate(angles[0], angles[1], t),
        mode,
        origin,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn combined_rotary_sweeps_are_closed_and_cover_intermediate_rim_positions() {
        let [a, b] = [[55., 20., 12.], [58., 24., 9.]];
        let angles = [[18., -24., 50.], [24., -21., 43.]];
        let radius = 4.;
        let slack =
            ROTARY_CHORD_ERROR_MM + radius * (1. - (std::f64::consts::PI / RIM as f64).cos());
        for mode in [RotaryMode::Head, RotaryMode::Table] {
            let parts = intervals(a, b, angles, radius, mode, false).unwrap();
            let pose = |t| sample(a, b, angles, mode, [0.; 3], t / parts as f64);
            for i in 0..parts {
                let first = pose(i as f64);
                let last = pose((i + 1) as f64);
                let mut triangles = Vec::new();
                bridge(&first, &last, radius, |face| triangles.push(face));
                assert!(triangles.len() >= 4);
                let mut edges = BTreeMap::new();
                for &triangle in &triangles {
                    for i in 0..3 {
                        let mut key =
                            [triangle[i], triangle[(i + 1) % 3]].map(|p| p.map(f64::to_bits));
                        key.sort_unstable();
                        *edges.entry(key).or_insert(0) += 1;
                    }
                }
                assert!(
                    edges.values().all(|&count| count == 2),
                    "{mode:?} piece {i}"
                );
                for fraction in [0.25, 0.5, 0.75] {
                    let frame = pose(i as f64 + fraction);
                    for j in 0..(RIM * 4) {
                        let angle = std::f64::consts::TAU * j as f64 / (RIM * 4) as f64;
                        let p = frame.point([radius * angle.cos(), radius * angle.sin(), 0.]);
                        for [a, b, c] in &triangles {
                            let n = cross(sub(*b, *a), sub(*c, *a));
                            assert!(dot(n, sub(p, *a)) <= slack * dot(n, n).sqrt() + 1e-9);
                        }
                    }
                }
            }
        }
    }
}
