//! MachPro scalar operations. The evaluator uses NaN for NIL when calling scalar operations.
use std::f64::consts::PI;
fn number(value: f64) -> f64 {
    if value.is_nan() {
        0.
    } else {
        value
    }
}
fn int32(value: f64) -> i32 {
    (value.trunc().rem_euclid(4294967296.) as u32) as i32
}
pub fn binary(op: u32, left: f64, right: f64, tolerance: f64) -> f64 {
    let (a, b) = (number(left), number(right));
    let equal = if left.is_nan() || right.is_nan() {
        left.is_nan() && right.is_nan()
    } else {
        (a - b).abs() <= tolerance
    };
    match op {
        0 => a + b,
        1 => a - b,
        2 => a * b,
        3 => {
            if b == 0. {
                f64::NAN
            } else {
                a / b
            }
        }
        4 => {
            if b == 0. {
                f64::NAN
            } else {
                a % b
            }
        }
        5 => a.powf(b),
        6 => (int32(a) & int32(b)) as f64,
        7 => (int32(a) | int32(b)) as f64,
        8 => (int32(a) ^ int32(b)) as f64,
        9 => u8::from(equal) as f64,
        10 => u8::from(!equal) as f64,
        11 => u8::from(a < b) as f64,
        12 => u8::from(a <= b) as f64,
        13 => u8::from(a > b) as f64,
        14 => u8::from(a >= b) as f64,
        _ => f64::NAN,
    }
}
pub fn unary(op: u32, value: f64) -> f64 {
    let v = number(value);
    match op {
        0 => v,
        1 => -v,
        2 => (!int32(v)) as f64,
        _ => f64::NAN,
    }
}
pub fn function(op: u32, x: f64, y: f64, argc: u32) -> f64 {
    let x = number(x);
    let y = number(y);
    match op {
        0 => x.abs(),
        1 => x.signum() * (x.abs() + 0.5).floor(),
        2 => x.floor(),
        3 => x.ceil(),
        4 => (x * PI / 180.).sin(),
        5 => (x * PI / 180.).cos(),
        6 => (x * PI / 180.).tan(),
        7 => x.asin() * 180. / PI,
        8 => x.acos() * 180. / PI,
        9 => {
            if argc == 1 {
                x.atan() * 180. / PI
            } else {
                x.atan2(y) * 180. / PI
            }
        }
        10 => x.sqrt(),
        11 => x.ln(),
        12 => x.exp(),
        13 => packed_bcd(x, true),
        14 => packed_bcd(x, false),
        _ => f64::NAN,
    }
}
fn packed_bcd(value: f64, to_binary: bool) -> f64 {
    if !value.is_finite() || value.fract() != 0. || !(0. ..=4294967295.).contains(&value) {
        return f64::NAN;
    }
    if !to_binary && value > 99999999. {
        return f64::NAN;
    }
    let mut input = value as u32;
    let mut output = 0u32;
    let mut shift = 1u32;
    while input > 0 {
        let base = if to_binary { 16 } else { 10 };
        let digit = input % base;
        if digit > 9 {
            return f64::NAN;
        }
        output += digit * shift;
        input /= base;
        if input > 0 {
            shift *= if to_binary { 10 } else { 16 }
        }
    }
    output as f64
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn nil_bitwise_and_domains() {
        assert_eq!(binary(9, f64::NAN, 0., 0.00001), 0.);
        assert_eq!(binary(9, f64::NAN, f64::NAN, 0.00001), 1.);
        assert_eq!(binary(0, f64::NAN, 3., 0.), 3.);
        assert_eq!(binary(7, 4294967296., 1., 0.), 1.);
        assert_eq!(unary(2, 2147483648.), 2147483647.);
        assert!(function(10, -1., 0., 1).is_nan());
        assert!(binary(3, 2., 0., 0.).is_nan());
        assert!(function(12, 1000., 0., 1).is_infinite());
    }
    #[test]
    fn bcd_rounding_and_degrees() {
        assert_eq!(function(13, 0x12345678 as f64, 0., 1), 12345678.);
        assert_eq!(function(14, 99999999., 0., 1), 0x99999999u32 as f64);
        assert!(function(13, 0x1a as f64, 0., 1).is_nan());
        assert_eq!(function(1, -1.5, 0., 1), -2.);
        assert!((function(9, 1., -1., 2) - 135.).abs() < 1e-12);
    }
}
