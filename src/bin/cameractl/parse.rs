//! Scalars agents pass on the command line.

use std::time::Duration;

use sony_ptp3::object;

use crate::error::CliError;
use crate::names;

pub fn code16(code: u16) -> String {
    format!("{code:#06x}")
}

pub fn code32(code: u32) -> String {
    format!("{code:#010x}")
}

pub fn u32_code(raw: &str) -> Result<u32, CliError> {
    let raw = raw.trim();
    if let Some(hex) = raw.strip_prefix("0x").or_else(|| raw.strip_prefix("0X")) {
        if hex.is_empty() || hex.len() > 8 || !hex.chars().all(|ch| ch.is_ascii_hexdigit()) {
            return Err(bad_code(raw));
        }
        return u32::from_str_radix(hex, 16).map_err(|_| bad_code(raw));
    }
    if !raw.is_empty() && raw.chars().all(|ch| ch.is_ascii_digit()) {
        return raw.parse::<u32>().map_err(|_| bad_code(raw));
    }
    Err(bad_code(raw))
}

pub fn handle(raw: &str) -> Result<u32, CliError> {
    match names::slugify(raw).as_str() {
        "captured" => Ok(object::CAPTURED),
        "live-view" => Ok(object::LIVE_VIEW),
        "shot-with-info" => Ok(object::SHOT_WITH_INFO),
        "camera-setting" => Ok(object::CAMERA_SETTING),
        "ftp-setting" => Ok(object::FTP_SETTING),
        _ => u32_code(raw).map_err(|_| {
            CliError::usage(
                format!("unknown handle {raw}"),
                "Pass captured, live-view, shot-with-info, camera-setting, ftp-setting, a decimal handle, or a 0x handle.",
            )
            .input(raw)
        }),
    }
}

pub fn duration(raw: &str, min: Duration, max: Duration) -> Result<Duration, CliError> {
    let raw = raw.trim();
    let (number, unit) = if let Some(rest) = raw.strip_suffix("ms") {
        (rest, Unit::Millis)
    } else if let Some(rest) = raw.strip_suffix('s') {
        (rest, Unit::Seconds)
    } else if let Some(rest) = raw.strip_suffix('m') {
        (rest, Unit::Minutes)
    } else {
        (raw, Unit::Seconds)
    };
    if number.is_empty() || !number.chars().all(|ch| ch.is_ascii_digit()) {
        return Err(bad_duration(raw));
    }
    let value: u64 = number.parse().map_err(|_| bad_duration(raw))?;
    let parsed = match unit {
        Unit::Millis => Duration::from_millis(value),
        Unit::Seconds => Duration::from_secs(value),
        Unit::Minutes => Duration::from_secs(value.saturating_mul(60)),
    };
    if parsed < min || parsed > max {
        return Err(CliError::usage(
            format!(
                "timeout {raw} is outside {}ms..={}ms",
                min.as_millis(),
                max.as_millis()
            ),
            "Pass a timeout such as 500ms, 5s, or 1m.",
        )
        .input(raw));
    }
    Ok(parsed)
}

fn bad_code(raw: &str) -> CliError {
    CliError::usage(
        format!("code {raw} is not a decimal integer or 0x hex value"),
        "Example: --storage-id 0x00010001",
    )
    .input(raw)
}

fn bad_duration(raw: &str) -> CliError {
    CliError::usage(
        format!("timeout {raw} is not a duration"),
        "Use milliseconds, seconds, or minutes, for example 500ms, 5s, or 1m.",
    )
    .input(raw)
}

enum Unit {
    Millis,
    Seconds,
    Minutes,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_accept_units_and_reject_fractions() {
        assert_eq!(
            duration("5s", Duration::ZERO, Duration::from_secs(120)).unwrap(),
            Duration::from_secs(5)
        );
        assert_eq!(
            duration("500ms", Duration::ZERO, Duration::from_secs(120)).unwrap(),
            Duration::from_millis(500)
        );
        assert_eq!(
            duration("1", Duration::from_millis(1), Duration::from_secs(120)).unwrap(),
            Duration::from_secs(1)
        );
        assert!(duration("1.5s", Duration::ZERO, Duration::from_secs(120)).is_err());
        assert!(duration("10m", Duration::ZERO, Duration::from_secs(120)).is_err());
        assert!(duration("0s", Duration::from_millis(1), Duration::from_secs(120)).is_err());
    }

    #[test]
    fn handles_accept_names_and_hex() {
        assert_eq!(handle("captured").unwrap(), object::CAPTURED);
        assert_eq!(handle("0xffffc002").unwrap(), object::LIVE_VIEW);
        assert_eq!(handle("12").unwrap(), 12);
        assert!(handle("nope").is_err());
    }
}
