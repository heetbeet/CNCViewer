use crate::geometry::{distance, segment_distance_xy, Point};
#[derive(Clone, Copy)]
pub struct Sweep {
    pub start: Point,
    pub end: Point,
    pub diameter: f64,
    pub id: usize,
    pub movement: usize,
    pub step: usize,
    pub groups: u32,
}
pub struct Overlap {
    pub pairs: Vec<[f64; 9]>,
    pub comparisons: usize,
    pub complete: bool,
}
pub fn compare(
    sweeps: &[Sweep],
    clearance: f64,
    max_pairs: usize,
    max_comparisons: usize,
    explicit: bool,
) -> Overlap {
    let mut sorted: Vec<_> = sweeps
        .iter()
        .filter(|s| !explicit || s.groups != 0)
        .collect();
    sorted.sort_by(|a, b| {
        (a.start[0].min(a.end[0]) - a.diameter / 2.)
            .total_cmp(&(b.start[0].min(b.end[0]) - b.diameter / 2.))
    });
    let mut result = Overlap {
        pairs: Vec::new(),
        comparisons: 0,
        complete: true,
    };
    'outer: for (i, a) in sorted.iter().enumerate() {
        for b in &sorted[i + 1..] {
            if b.start[0].min(b.end[0]) - b.diameter / 2.
                > a.start[0].max(a.end[0]) + a.diameter / 2. + clearance
            {
                break;
            }
            result.comparisons += 1;
            if result.comparisons > max_comparisons {
                result.complete = false;
                break 'outer;
            }
            if a.movement == b.movement
                || a.start[1].max(a.end[1]) + a.diameter / 2. + clearance
                    < b.start[1].min(b.end[1]) - b.diameter / 2.
                || b.start[1].max(b.end[1]) + b.diameter / 2. + clearance
                    < a.start[1].min(a.end[1]) - a.diameter / 2.
            {
                continue;
            }
            if explicit
                && !((a.groups & 1 != 0 && b.groups & 2 != 0)
                    || (a.groups & 2 != 0 && b.groups & 1 != 0))
            {
                continue;
            }
            if !explicit
                && a.movement.abs_diff(b.movement) == 1
                && (distance(&a.end, &b.start) < 1e-8 || distance(&b.end, &a.start) < 1e-8)
            {
                continue;
            }
            let gap = segment_distance_xy(a.start, a.end, b.start, b.end)
                - (a.diameter + b.diameter) / 2.;
            if gap <= clearance + 1e-9 {
                result.pairs.push([
                    a.id as f64,
                    b.id as f64,
                    a.step as f64,
                    b.step as f64,
                    gap,
                    a.start[2].min(a.end[2]),
                    a.start[2].max(a.end[2]),
                    b.start[2].min(b.end[2]),
                    b.start[2].max(b.end[2]),
                ]);
                if result.pairs.len() >= max_pairs {
                    result.complete = false;
                    break 'outer;
                }
            }
        }
    }
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn passes_and_limits() {
        let a = Sweep {
            start: [0.; 3],
            end: [20., 0., 0.],
            diameter: 8.,
            id: 1,
            movement: 1,
            step: 2,
            groups: 1,
        };
        let b = Sweep {
            start: [0., 6., 0.],
            end: [20., 6., 0.],
            id: 2,
            movement: 3,
            step: 4,
            groups: 2,
            ..a
        };
        let r = compare(&[a, b], 0., 10, 100, true);
        assert!(r.complete);
        assert_eq!(r.pairs[0][4], -2.);
        assert!(!compare(&[a, b], 0., 1, 100, false).complete);
    }
}
