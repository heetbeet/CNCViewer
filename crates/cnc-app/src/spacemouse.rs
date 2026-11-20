use crate::camera::Camera;
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct SpaceMouse {
    api: Option<hidapi::HidApi>,
    device: Option<hidapi::HidDevice>,
    last_scan: Option<Instant>,
    last_poll: Option<Instant>,
    last_motion: Option<Instant>,
    axes: [i16; 6],
}

impl SpaceMouse {
    pub fn update(&mut self, camera: &mut Camera) -> Duration {
        let now = Instant::now();
        let elapsed = self.last_poll.replace(now).map_or(0.016, |prior| {
            now.duration_since(prior).as_secs_f64().min(0.1)
        });
        if self.device.is_none()
            && self
                .last_scan
                .is_none_or(|last| now.duration_since(last) >= Duration::from_secs(1))
        {
            self.last_scan = Some(now);
            if self.api.is_none() {
                self.api = hidapi::HidApi::new().ok();
            }
            if let Some(api) = &mut self.api {
                let _ = api.refresh_devices();
                self.device = api
                    .device_list()
                    .filter(|device| {
                        matches!(device.vendor_id(), 0x046d | 0x256f)
                            && device.usage_page() == 1
                            && device.usage() == 8
                    })
                    .find_map(|device| device.open_device(api).ok());
            }
        }
        let Some(device) = &self.device else {
            return Duration::from_secs(1);
        };
        let mut report = [0; 64];
        for _ in 0..16 {
            match device.read_timeout(&mut report, 0) {
                Ok(0) => break,
                Ok(size) => {
                    if decode(&report[..size], &mut self.axes) {
                        self.last_motion = Some(now);
                    }
                }
                Err(_) => {
                    self.device = None;
                    self.axes = [0; 6];
                    return Duration::from_secs(1);
                }
            }
        }
        if self
            .last_motion
            .is_none_or(|time| now.duration_since(time) >= Duration::from_millis(150))
        {
            self.axes = [0; 6];
        }
        let axes = self.axes.map(|axis| {
            if axis.unsigned_abs() > 5 {
                f64::from(axis) / 32767.
            } else {
                0.
            }
        });
        if axes.iter().any(|axis| *axis != 0.) {
            let scale = elapsed * 60.;
            camera.pan(
                axes[0] * scale * 2. / camera.scale,
                axes[2] * scale * 2. / camera.scale,
            );
            camera.zoom((axes[1] * scale * 0.035).exp(), [0.; 2]);
            camera.yaw -= axes[5] * scale * 0.05;
            camera.pitch = (camera.pitch + axes[3] * scale * 0.05).clamp(
                -std::f64::consts::FRAC_PI_2 + 0.001,
                std::f64::consts::FRAC_PI_2 - 0.001,
            );
        }
        Duration::from_millis(16)
    }
}

fn decode(report: &[u8], axes: &mut [i16; 6]) -> bool {
    let (offset, count) = match report.first() {
        Some(1) if report.len() >= 13 => (0, 6),
        Some(1) if report.len() >= 7 => (0, 3),
        Some(2) if report.len() >= 7 => (3, 3),
        _ => return false,
    };
    for (axis, bytes) in axes[offset..offset + count]
        .iter_mut()
        .zip(report[1..].chunks_exact(2))
    {
        *axis = i16::from_le_bytes([bytes[0], bytes[1]]);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_split_and_combined_reports_and_ignores_buttons() {
        let mut axes = [0; 6];
        assert!(decode(&[1, 10, 0, 246, 255, 30, 0], &mut axes));
        assert!(decode(&[2, 5, 0, 6, 0, 7, 0], &mut axes));
        assert!(!decode(&[3, 1], &mut axes));
        assert_eq!(axes, [10, -10, 30, 5, 6, 7]);
        assert!(decode(&[1, 1, 0, 2, 0, 3, 0, 4, 0, 5, 0, 6, 0], &mut axes));
        assert_eq!(axes, [1, 2, 3, 4, 5, 6]);
    }
}
