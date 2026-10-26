//! Pure formatting helpers for telemetry log lines.
//!
//! Optional readings are rendered with fixed sensible precision or the
//! literal word `unavailable` when missing, instead of leaking `Option`
//! internals (`Some(...)`, `None`) or sentinel values such as `-1.0`.

pub const UNAVAILABLE: &str = "unavailable";

/// Formats volts with three decimals, or `unavailable` when missing.
pub fn format_volts(value: Option<f64>) -> String {
    match value {
        Some(v) => format!("{v:.3}"),
        None => UNAVAILABLE.to_owned(),
    }
}

/// Formats amperes with three decimals, or `unavailable` when missing.
pub fn format_amps(value: Option<f64>) -> String {
    match value {
        Some(a) => format!("{a:.3}"),
        None => UNAVAILABLE.to_owned(),
    }
}

/// Formats a temperature in degrees Celsius with one decimal, or
/// `unavailable` when missing.
pub fn format_celsius(value: Option<f64>) -> String {
    match value {
        Some(c) => format!("{c:.1}"),
        None => UNAVAILABLE.to_owned(),
    }
}

/// Formats a percentage as an integer, or `unavailable` when missing.
pub fn format_percent(value: Option<u8>) -> String {
    match value {
        Some(p) => format!("{p}"),
        None => UNAVAILABLE.to_owned(),
    }
}

/// Formats CPU usage as a percent with one decimal, or `unavailable` when
/// missing (instead of a `-1.0` sentinel).
pub fn format_cpu_percent(value: Option<f64>) -> String {
    match value {
        Some(c) => format!("{c:.1}"),
        None => UNAVAILABLE.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volts_use_three_decimals_and_mark_missing() {
        assert_eq!(format_volts(Some(3.801)), "3.801");
        assert_eq!(format_volts(Some(4.0)), "4.000");
        assert_eq!(format_volts(None), UNAVAILABLE);
    }

    #[test]
    fn amps_use_three_decimals_and_mark_missing() {
        assert_eq!(format_amps(Some(0.525)), "0.525");
        assert_eq!(format_amps(Some(-0.1234)), "-0.123");
        assert_eq!(format_amps(None), UNAVAILABLE);
    }

    #[test]
    fn celsius_uses_one_decimal_and_marks_missing() {
        assert_eq!(format_celsius(Some(25.34)), "25.3");
        assert_eq!(format_celsius(Some(25.0)), "25.0");
        assert_eq!(format_celsius(None), UNAVAILABLE);
    }

    #[test]
    fn percent_is_an_integer_and_marks_missing() {
        assert_eq!(format_percent(Some(87)), "87");
        assert_eq!(format_percent(Some(0)), "0");
        assert_eq!(format_percent(None), UNAVAILABLE);
    }

    #[test]
    fn cpu_percent_uses_one_decimal_and_never_prints_sentinel() {
        assert_eq!(format_cpu_percent(Some(80.0)), "80.0");
        assert_eq!(format_cpu_percent(Some(12.34)), "12.3");
        assert_eq!(format_cpu_percent(None), UNAVAILABLE);
    }
}
