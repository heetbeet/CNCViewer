use crate::camera::{dot, Camera};
use eframe::egui::{pos2, Pos2};
use std::sync::LazyLock;

type Point = [f64; 3];
type Polygon = [Point; 4];

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
    let (right, up) = camera.orbit_basis();
    move |v| {
        pos2(
            center.x + dot(v, right) as f32 * 50.,
            center.y - dot(v, up) as f32 * 50.,
        )
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
}
