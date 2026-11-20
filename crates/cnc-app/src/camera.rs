#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Camera {
    pub center: [f64; 3],
    pub yaw: f64,
    pub pitch: f64,
    pub scale: f64,
    pub z_up: bool,
}
pub(crate) fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a.into_iter().zip(b).map(|(a, b)| a * b).sum()
}
impl Default for Camera {
    fn default() -> Self {
        Self {
            center: [0.; 3],
            yaw: -45f64.to_radians(),
            pitch: 35.26438968f64.to_radians(),
            scale: 1.,
            z_up: true,
        }
    }
}
impl Camera {
    pub fn orbit_basis(&self) -> ([f64; 3], [f64; 3]) {
        let (s, c) = self.yaw.sin_cos();
        let (p, q) = self.pitch.sin_cos();
        ([-s, c, 0.], [-c * p, -s * p, q])
    }
    pub fn basis(&self) -> ([f64; 3], [f64; 3]) {
        let (r, u) = self.orbit_basis();
        if self.z_up {
            (r, u)
        } else {
            ([r[0], -r[2], r[1]], [u[0], -u[2], u[1]])
        }
    }
    pub fn view(&mut self, name: &str) {
        let (yaw, pitch) = match name {
            "top" => (-90., 90.),
            "bottom" => (-90., -90.),
            "front" => (-90., 0.),
            "back" => (90., 0.),
            "left" => (180., 0.),
            "right" => (0., 0.),
            _ => (-45., 35.26438968),
        };
        self.yaw = f64::to_radians(yaw);
        self.pitch = f64::to_radians(pitch);
    }
    pub fn view_direction(&mut self, [x, y, z]: [f64; 3]) {
        self.yaw = if x == 0. && y == 0. {
            -std::f64::consts::FRAC_PI_2
        } else {
            y.atan2(x)
        };
        self.pitch = z.atan2(x.hypot(y));
    }
    pub fn orbit_direction(&self) -> [f64; 3] {
        let (s, c) = self.yaw.sin_cos();
        let (p, q) = self.pitch.sin_cos();
        [q * c, q * s, p]
    }
    pub fn project(&self, point: [f64; 3]) -> [f64; 2] {
        let (r, u) = self.basis();
        let delta = std::array::from_fn(|i| point[i] - self.center[i]);
        [dot(delta, r) / self.scale, -dot(delta, u) / self.scale]
    }
    pub fn pan(&mut self, dx: f64, dy: f64) {
        let (r, u) = self.basis();
        for i in 0..3 {
            self.center[i] -= r[i] * dx * self.scale;
            self.center[i] += u[i] * dy * self.scale
        }
    }
    pub fn zoom(&mut self, factor: f64, anchor: [f64; 2]) {
        let previous = self.scale;
        self.scale = (self.scale * factor).clamp(1e-8, 1e9);
        let (r, u) = self.basis();
        for i in 0..3 {
            self.center[i] += (r[i] * anchor[0] - u[i] * anchor[1]) * (previous - self.scale)
        }
    }
    pub fn fit(&mut self, bounds: Option<([f64; 3], [f64; 3])>, _width: f64, height: f64) {
        if let Some((min, max)) = bounds {
            self.center = std::array::from_fn(|i| (min[i] + max[i]) / 2.);
            let radius = min
                .into_iter()
                .zip(max)
                .map(|(a, b)| (b - a).powi(2))
                .sum::<f64>()
                .sqrt()
                * 0.65;
            self.scale = 2. * radius.max(5.) / height.max(1.);
        }
    }
    pub fn fit_box(&mut self, min: [f64; 3], max: [f64; 3], width: f64, height: f64) {
        self.center = std::array::from_fn(|i| (min[i] + max[i]) / 2.);
        let size = std::array::from_fn::<_, 3, _>(|i| max[i] - min[i]);
        let projected = |basis: [f64; 3]| {
            size.into_iter()
                .zip(basis)
                .map(|(width, axis)| width * axis.abs())
                .sum::<f64>()
        };
        let (right, up) = self.basis();
        self.scale = (projected(right).max(1.) / width).max(projected(up).max(1.) / height) * 1.15;
    }
    pub fn uniform(&self, width: f64, height: f64, show_rapid: bool) -> [f32; 12] {
        let (r, u) = self.basis();
        let sx = 2. / (width * self.scale);
        let sy = 2. / (height * self.scale);
        [
            (r[0] * sx) as f32,
            (r[1] * sx) as f32,
            (r[2] * sx) as f32,
            (-dot(self.center, r) * sx) as f32,
            (u[0] * sy) as f32,
            (u[1] * sy) as f32,
            (u[2] * sy) as f32,
            (-dot(self.center, u) * sy) as f32,
            if show_rapid { 1. } else { 0. },
            (2. / width) as f32,
            (2. / height) as f32,
            0.,
        ]
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn anchored_zoom() {
        let mut c = Camera::default();
        c.view("top");
        let p = [10., 20., 0.];
        let before = c.project(p);
        c.zoom(0.5, before);
        let after = c.project(p);
        assert!((before[0] - after[0]).abs() < 1e-9);
        assert!((before[1] - after[1]).abs() < 1e-9);
    }
}
