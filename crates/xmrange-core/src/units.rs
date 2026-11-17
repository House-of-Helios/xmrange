/// Celsius → Fahrenheit. Data stays °C; convert only at display.
pub fn c_to_f(c: f64) -> f64 {
    c * 9.0 / 5.0 + 32.0
}

/// Hashes per watt. `None` when either side is unknown, non-finite, or watts ≈ 0.
pub fn hw(hs: Option<f64>, watts: Option<f64>) -> Option<f64> {
    match (hs, watts) {
        (Some(h), Some(w)) if h.is_finite() && w.is_finite() && w > 0.05 => Some(h / w),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_celsius_to_fahrenheit() {
        assert_eq!(c_to_f(100.0), 212.0);
        assert_eq!(c_to_f(0.0), 32.0);
        assert!((c_to_f(55.0) - 131.0).abs() < 1e-9);
    }

    #[test]
    fn hw_divides_and_refuses_zero_or_unknown() {
        let v = hw(Some(1826.0), Some(9.1)).unwrap();
        assert!((v - 200.659).abs() < 0.01);
        assert_eq!(hw(Some(1826.0), Some(0.0)), None);
        assert_eq!(hw(Some(1826.0), None), None);
        assert_eq!(hw(None, Some(9.1)), None);
        assert_eq!(hw(Some(f64::NAN), Some(9.1)), None);
    }
}
