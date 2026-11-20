use crate::camera::{dot, Camera};
use eframe::egui::{pos2, vec2, Align2, Color32, FontId, Painter, Pos2, Stroke};
use std::sync::LazyLock;

type Point = [f64; 3];
type Polygon = [Point; 4];
const SCALE: f32 = 50.;
const MIN_LINE: f32 = 4.;

pub(super) struct Axis {
    name: &'static str,
    color: Color32,
    start: Pos2,
    tip: Pos2,
    label: Pos2,
    depth: f64,
}

struct Face {
    normal: Point,
    vertices: Polygon,
    len: usize,
}

pub(super) struct ProjectedFace {
    pub normal: Point,
    vertices: [Pos2; 4],
    len: usize,
}

static FACES: LazyLock<Vec<Face>> = LazyLock::new(|| {
    let directions = [-1., 0., 1.];
    let mut vertices = Vec::with_capacity(24);
    for major_axis in 0..3 {
        for x in [-1., 1.] {
            for y in [-1., 1.] {
                for z in [-1., 1.] {
                    let mut point = [x * 0.25, y * 0.25, z * 0.25];
                    point[major_axis] *= 2.;
                    vertices.push(point);
                }
            }
        }
    }

    // Each of the 26 directions defines one supporting plane of the beveled cube.
    let mut faces = Vec::with_capacity(26);
    for x in directions {
        for y in directions {
            for z in directions {
                let normal = [x, y, z];
                if normal == [0.; 3] {
                    continue;
                }
                let support = vertices
                    .iter()
                    .map(|v| dot(*v, normal))
                    .fold(f64::NEG_INFINITY, f64::max);
                let mut face: Vec<_> = vertices
                    .iter()
                    .copied()
                    .filter(|&v| dot(v, normal) == support)
                    .collect();
                let center: Point = std::array::from_fn(|i| {
                    face.iter().map(|v| v[i]).sum::<f64>() / face.len() as f64
                });
                let tangent: Point = std::array::from_fn(|i| face[0][i] - center[i]);
                let [a, b, c] = tangent;
                let cross = [y * c - z * b, z * a - x * c, x * b - y * a];
                let angle = |point: Point| {
                    let delta = std::array::from_fn(|i| point[i] - center[i]);
                    dot(delta, cross).atan2(dot(delta, tangent))
                };
                face.sort_by(|a, b| angle(*b).total_cmp(&angle(*a)));
                let mut polygon = [[0.; 3]; 4];
                polygon[..face.len()].copy_from_slice(&face);
                faces.push(Face {
                    normal,
                    vertices: polygon,
                    len: face.len(),
                });
            }
        }
    }
    faces
});

pub(super) fn projector(camera: &Camera, center: Pos2) -> impl Fn(Point) -> Pos2 {
    let basis = camera.orbit_basis();
    move |v| project(basis, center, v)
}

fn project((right, up): (Point, Point), center: Pos2, v: Point) -> Pos2 {
    pos2(
        center.x + dot(v, right) as f32 * SCALE,
        center.y - dot(v, up) as f32 * SCALE,
    )
}

pub(super) fn axes(camera: &Camera, center: Pos2) -> [Axis; 6] {
    let basis = camera.basis();
    let direction = camera.depth_axis();
    let mut axes = [
        ([1., 0., 0.], "X", [183, 65, 54]),
        ([-1., 0., 0.], "-X", [183, 65, 54]),
        ([0., 1., 0.], "Y", [69, 126, 40]),
        ([0., -1., 0.], "-Y", [69, 126, 40]),
        ([0., 0., 1.], "Z", [46, 98, 178]),
        ([0., 0., -1.], "-Z", [46, 98, 178]),
    ]
    .map(|(axis, name, [r, g, b])| {
        let start = project(basis, center, axis.map(|n| n * 0.5));
        let tip = project(basis, center, axis.map(|n| n * 1.05));
        let offset = tip - center;
        let label = tip + offset.normalized() * (0.9 * offset.length()).min(8.);
        Axis {
            name,
            color: Color32::from_rgb(r, g, b),
            start,
            tip,
            label,
            depth: dot(axis, direction),
        }
    });
    axes.sort_by(|a, b| a.depth.total_cmp(&b.depth));
    for index in 0..axes.len() {
        let axis = &axes[index];
        if axes.iter().enumerate().any(|(other_index, other)| {
            let line = other.tip - other.start;
            let length = line.length();
            if index == other_index || length < MIN_LINE {
                return false;
            }
            let delta = axis.label - other.start;
            let direction = line / length;
            let along = delta.dot(direction);
            (0.0..=length).contains(&along) && (delta - direction * along).length() < 5.
        }) {
            let radial = (axis.tip - center).normalized();
            let offset = vec2(radial.y, -radial.x) * 9.;
            axes[index].label += if offset.y <= 0. { offset } else { -offset };
        }
    }
    axes
}

pub(super) fn paint_axes(painter: &Painter, axes: &[Axis; 6], front: bool) {
    for (index, axis) in axes.iter().enumerate() {
        if (axis.depth >= 0.) != front {
            continue;
        }
        if axis.start.distance(axis.tip) >= MIN_LINE {
            painter.line_segment([axis.start, axis.tip], Stroke::new(1.8, axis.color));
        }
        // An exactly aligned pair can project to the same tip. Give the nearer label room.
        if axes[index + 1..].iter().any(|near| {
            let delta = near.label - axis.label;
            delta.x.abs() < 13. && delta.y.abs() < 11.
        }) {
            continue;
        }
        painter.text(
            axis.label,
            Align2::CENTER_CENTER,
            axis.name,
            FontId::proportional(11.),
            axis.color,
        );
    }
}

pub(super) fn visible(camera: &Camera, project: impl Fn(Point) -> Pos2) -> Vec<ProjectedFace> {
    let direction = camera.orbit_direction();
    FACES
        .iter()
        .filter(|face| dot(face.normal, direction) > 1e-8)
        .map(|face| ProjectedFace {
            normal: face.normal,
            vertices: face.vertices.map(&project),
            len: face.len,
        })
        .collect()
}

impl ProjectedFace {
    pub fn points(&self) -> &[Pos2] {
        &self.vertices[..self.len]
    }
    pub fn contains(&self, p: Pos2) -> bool {
        (0..self.len).all(|index| {
            let origin = self.vertices[index];
            let edge = self.vertices[(index + 1) % self.len] - origin;
            let delta = p - origin;
            edge.x * delta.y - edge.y * delta.x >= -0.01
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_face_edge_and_corner_is_clickable() {
        assert_eq!(FACES.len(), 26);
        assert_eq!(FACES.iter().filter(|face| face.len == 3).count(), 8);
        for face in &*FACES {
            let mut camera = Camera::default();
            camera.view_direction(face.normal);
            let center = pos2(50., 50.);
            let visible = visible(&camera, projector(&camera, center));
            let hits: Vec<_> = visible
                .iter()
                .filter(|face| face.contains(center))
                .collect();
            assert_eq!(hits.len(), 1, "direction {:?}", face.normal);
            assert_eq!(hits[0].normal, face.normal);
        }
    }

    #[test]
    fn signed_guides_follow_machine_axes_in_both_orientations() {
        let center = pos2(100., 100.);
        for (z_up, front) in [(true, ["X", "-Y", "Z"]), (false, ["X", "-Z", "-Y"])] {
            let mut camera = Camera {
                z_up,
                ..Camera::default()
            };
            camera.view_direction([1., -2., 3.]);
            let guides = axes(&camera, center);
            let actual: Vec<_> = guides
                .iter()
                .filter(|axis| axis.depth > 0.)
                .map(|axis| axis.name)
                .collect();
            assert_eq!(actual, front);
            for (name, direction) in [
                ("X", [1., 0., 0.]),
                ("-X", [-1., 0., 0.]),
                ("Y", [0., 1., 0.]),
                ("-Y", [0., -1., 0.]),
                ("Z", [0., 0., 1.]),
                ("-Z", [0., 0., -1.]),
            ] {
                let guide = guides.iter().find(|axis| axis.name == name).unwrap();
                let [x, y] = camera.project(direction);
                let expected = center + eframe::egui::vec2(x as f32, y as f32) * (1.05 * SCALE);
                assert!(guide.tip.distance(expected) < 0.001, "{name}, z_up={z_up}");
            }
        }
    }
}
