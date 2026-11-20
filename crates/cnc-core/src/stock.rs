use crate::geometry::Point;

#[derive(Clone, Copy)]
pub struct Sweep {
    pub start: Point,
    pub end: Point,
    pub diameter: f64,
    pub witness: usize,
}
pub struct Stock {
    pub heights: Vec<f64>,
    pub witnesses: Vec<i32>,
    pub tests: usize,
    pub complete: bool,
}
pub fn sample(
    min: Point,
    max: Point,
    nx: usize,
    ny: usize,
    sweeps: &[Sweep],
    max_tests: usize,
) -> Result<Stock, &'static str> {
    if nx == 0
        || ny == 0
        || nx.checked_mul(ny).is_none_or(|n| n > 250_000_000)
        || min.iter().chain(&max).any(|v| !v.is_finite())
        || min.iter().zip(max).any(|(a, b)| *a >= b)
    {
        return Err("Invalid stock grid");
    }
    if sweeps.iter().any(|s| {
        s.start.iter().chain(&s.end).any(|v| !v.is_finite())
            || !s.diameter.is_finite()
            || s.diameter <= 0.
            || s.witness > i32::MAX as usize
    }) {
        return Err("Invalid stock sweep");
    }
    let dx = (max[0] - min[0]) / nx as f64;
    let dy = (max[1] - min[1]) / ny as f64;
    let volume = dx * dy * (max[2] - min[2]) * (nx * ny) as f64;
    if !(dx > 0. && dy > 0. && dx.is_finite() && dy.is_finite() && volume.is_finite()) {
        return Err("Stock grid dimensions exceed the numeric range");
    }
    let mut result = Stock {
        heights: vec![max[2]; nx * ny],
        witnesses: vec![-1; nx * ny],
        tests: 0,
        complete: true,
    };
    'sweeps: for sweep in sweeps {
        let (a, b, r) = (sweep.start, sweep.end, sweep.diameter / 2.);
        let vx = b[0] - a[0];
        let vy = b[1] - a[1];
        let length = vx.hypot(vy);
        if !length.is_finite() || !(b[2] - a[2]).is_finite() {
            return Err("Stock sweep exceeds the numeric range");
        }
        let (ux, uy) = if length > 0. {
            (vx / length, vy / length)
        } else {
            (0., 0.)
        };
        let x0 = (((a[0].min(b[0]) - r - min[0]) / dx - 0.5).ceil() as i64).max(0);
        let x1 = (((a[0].max(b[0]) + r - min[0]) / dx - 0.5).floor() as i64).min(nx as i64 - 1);
        let y0 = (((a[1].min(b[1]) - r - min[1]) / dy - 0.5).ceil() as i64).max(0);
        let y1 = (((a[1].max(b[1]) + r - min[1]) / dy - 0.5).floor() as i64).min(ny as i64 - 1);
        for y in y0..=y1 {
            for x in x0..=x1 {
                result.tests += 1;
                if result.tests > max_tests {
                    result.complete = false;
                    break 'sweeps;
                }
                let px = min[0] + (x as f64 + 0.5) * dx - a[0];
                let py = min[1] + (y as f64 + 0.5) * dy - a[1];
                let (mut lo, mut hi) = (0f64, 1f64);
                if length > 0. {
                    let perpendicular = px.mul_add(uy, -py * ux).abs();
                    if perpendicular > r {
                        continue;
                    }
                    let center = px.mul_add(ux, py * uy) / length;
                    let half = ((r - perpendicular) * (r + perpendicular)).sqrt() / length;
                    lo = lo.max(center - half);
                    hi = hi.min(center + half);
                    if lo > hi {
                        continue;
                    }
                } else if px.hypot(py) > r {
                    continue;
                }
                let z = a[2] + (b[2] - a[2]) * if b[2] >= a[2] { lo } else { hi };
                let i = y as usize * nx + x as usize;
                let height = z.clamp(min[2], max[2]);
                if height < result.heights[i] {
                    result.heights[i] = height;
                    result.witnesses[i] = sweep.witness as i32
                }
            }
        }
    }
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn blind_through_ramp_and_budget() {
        let mut sweep = Sweep {
            start: [0., 0., 0.],
            end: [10., 0., -10.],
            diameter: 2.,
            witness: 7,
        };
        let stock = sample([0., 0., -20.], [10., 2., 0.], 10, 2, &[sweep], 1000).unwrap();
        assert!((stock.heights[5] + 5.5 + 0.75f64.sqrt()).abs() < 1e-12);
        assert_eq!(stock.witnesses[5], 7);
        assert_eq!(stock.heights[15], 0.);
        sweep.start[2] = -30.;
        sweep.end[2] = -30.;
        let through = sample([0., 0., -20.], [10., 2., 0.], 10, 2, &[sweep], 1000).unwrap();
        assert_eq!(through.heights[5], -20.);
        assert!(
            !sample([0., 0., -20.], [10., 2., 0.], 10, 2, &[sweep], 1)
                .unwrap()
                .complete
        );
    }
    #[test]
    fn long_cut_preserves_cutter_radius_in_a_small_stock_region() {
        let sweep = Sweep {
            start: [0., 0., -1.],
            end: [900_000_000., 900_000_000., -1.],
            diameter: 2.,
            witness: 3,
        };
        let stock = sample(
            [500_000_000., 500_000_001.7, -2.],
            [500_000_001., 500_000_002.1, 0.],
            1,
            2,
            &[sweep],
            10,
        )
        .unwrap();
        assert_eq!(stock.heights, [-1., 0.]);
        assert_eq!(stock.witnesses, [3, -1]);
    }
}
