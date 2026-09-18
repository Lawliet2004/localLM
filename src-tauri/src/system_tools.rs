use chrono::{DateTime, Local, TimeZone, Utc};
use serde::Serialize;
use serde_json::{json, Value};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemTimeReport {
    pub iso: String,
    pub formatted: String,
    pub timezone: String,
    pub utc_offset: String,
    pub epoch_ms: i64,
    pub source: &'static str,
}

/// Resolve an IANA timezone or offset string to a fixed offset in seconds.
/// Supports common canonical IANA timezones and numeric UTC offsets.
pub fn parse_iana_offset(tz_name: &str) -> Option<chrono::FixedOffset> {
    let normalized = tz_name.trim();
    if normalized.is_empty() {
        return None;
    }

    // Direct UTC / GMT checks
    if normalized.eq_ignore_ascii_case("utc") || normalized.eq_ignore_ascii_case("gmt") || normalized.eq_ignore_ascii_case("z") {
        return Some(chrono::FixedOffset::east_opt(0).unwrap());
    }

    // Explicit numeric offsets like "+05:30", "-04:00", "+0530", "+5"
    if normalized.starts_with('+') || normalized.starts_with('-') {
        let sign = if normalized.starts_with('-') { -1 } else { 1 };
        let rest = &normalized[1..];
        let (hours, mins) = if let Some((h, m)) = rest.split_once(':') {
            (h.parse::<i32>().ok()?, m.parse::<i32>().ok()?)
        } else if rest.len() == 4 {
            (rest[..2].parse::<i32>().ok()?, rest[2..].parse::<i32>().ok()?)
        } else if let Ok(h) = rest.parse::<i32>() {
            (h, 0)
        } else {
            return None;
        };
        if !(0..=23).contains(&hours) || !(0..=59).contains(&mins) {
            return None;
        }
        let total_secs = sign * (hours * 3600 + mins * 60);
        return chrono::FixedOffset::east_opt(total_secs);
    }

    // Common standard IANA timezone mappings
    match normalized.to_ascii_lowercase().as_str() {
        "asia/kolkata" | "asia/calcutta" | "ist" => chrono::FixedOffset::east_opt(5 * 3600 + 30 * 60),
        "asia/shanghai" | "asia/hong_kong" | "asia/singapore" | "cst" => chrono::FixedOffset::east_opt(8 * 3600),
        "asia/tokyo" | "jst" => chrono::FixedOffset::east_opt(9 * 3600),
        "asia/dubai" => chrono::FixedOffset::east_opt(4 * 3600),
        "asia/bangkok" | "asia/jakarta" => chrono::FixedOffset::east_opt(7 * 3600),
        "europe/london" | "europe/belfast" | "wwe" => chrono::FixedOffset::east_opt(0),
        "europe/paris" | "europe/berlin" | "europe/rome" | "europe/madrid" | "europe/amsterdam" | "cet" => chrono::FixedOffset::east_opt(3600),
        "europe/helsinki" | "europe/athens" | "eet" => chrono::FixedOffset::east_opt(2 * 3600),
        "america/new_york" | "est" | "us/eastern" => chrono::FixedOffset::west_opt(5 * 3600),
        "america/chicago" | "us/central" => chrono::FixedOffset::west_opt(6 * 3600),
        "america/denver" | "mst" | "us/mountain" => chrono::FixedOffset::west_opt(7 * 3600),
        "america/los_angeles" | "pst" | "us/pacific" => chrono::FixedOffset::west_opt(8 * 3600),
        "america/sao_paulo" => chrono::FixedOffset::west_opt(3 * 3600),
        "australia/sydney" | "australia/melbourne" | "aest" => chrono::FixedOffset::east_opt(10 * 3600),
        "pacific/auckland" | "nzst" => chrono::FixedOffset::east_opt(12 * 3600),
        _ => None,
    }
}

/// Query the deterministic system clock with optional IANA timezone translation.
pub fn get_system_time(timezone_arg: Option<&str>) -> Result<SystemTimeReport, String> {
    let now_utc: DateTime<Utc> = Utc::now();
    let epoch_ms = now_utc.timestamp_millis();

    if let Some(tz_str) = timezone_arg {
        let tz_trimmed = tz_str.trim();
        if !tz_trimmed.is_empty() {
            if let Some(offset) = parse_iana_offset(tz_trimmed) {
                let converted = offset.from_utc_datetime(&now_utc.naive_utc());
                let offset_secs = offset.local_minus_utc();
                let sign = if offset_secs >= 0 { '+' } else { '-' };
                let abs_secs = offset_secs.abs();
                let off_h = abs_secs / 3600;
                let off_m = (abs_secs % 3600) / 60;
                let utc_offset = format!("{sign}{off_h:02}:{off_m:02}");
                return Ok(SystemTimeReport {
                    iso: converted.to_rfc3339(),
                    formatted: converted.format("%A, %B %d, %Y %I:%M:%S %p").to_string(),
                    timezone: tz_trimmed.to_string(),
                    utc_offset,
                    epoch_ms,
                    source: "local_system_clock",
                });
            } else {
                return Err(format!(
                    "Unrecognized timezone '{tz_trimmed}'. Use standard IANA format (e.g. 'Asia/Kolkata', 'UTC', 'America/New_York') or offset (e.g. '+05:30')."
                ));
            }
        }
    }

    // Default to host system local time
    let now_local: DateTime<Local> = Local::now();
    let offset = *now_local.offset();
    let offset_secs = offset.local_minus_utc();
    let sign = if offset_secs >= 0 { '+' } else { '-' };
    let abs_secs = offset_secs.abs();
    let off_h = abs_secs / 3600;
    let off_m = (abs_secs % 3600) / 60;
    let utc_offset = format!("{sign}{off_h:02}:{off_m:02}");

    Ok(SystemTimeReport {
        iso: now_local.to_rfc3339(),
        formatted: now_local.format("%A, %B %d, %Y %I:%M:%S %p").to_string(),
        timezone: "System Local".to_string(),
        utc_offset,
        epoch_ms,
        source: "local_system_clock",
    })
}

pub fn system_time_tool_definition() -> crate::connectors::ToolView {
    crate::connectors::ToolView {
        name: "system_time".into(),
        description: "Get the current date and time from the local system clock, optionally translated to an IANA timezone (e.g. 'Asia/Kolkata', 'UTC', 'America/New_York') or offset (e.g. '+05:30'). Result reflects the operating system clock; it is not independently network-verified.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "timezone": {
                    "type": "string",
                    "description": "Optional IANA timezone name (e.g. 'Asia/Kolkata', 'America/New_York', 'UTC') or numeric offset (e.g. '+05:30'). Omit for host system local time."
                }
            },
            "additionalProperties": false
        }),
    }
}

pub fn execute_system_time(arguments: &Value) -> Result<Value, String> {
    let timezone = arguments.get("timezone").and_then(Value::as_str);
    let report = get_system_time(timezone)?;
    serde_json::to_value(report).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_kolkata_timezone() {
        let offset = parse_iana_offset("Asia/Kolkata").unwrap();
        assert_eq!(offset.local_minus_utc(), 5 * 3600 + 30 * 60);
    }

    #[test]
    fn parse_numeric_offsets() {
        let pos = parse_iana_offset("+05:30").unwrap();
        assert_eq!(pos.local_minus_utc(), 19800);

        let neg = parse_iana_offset("-05:00").unwrap();
        assert_eq!(neg.local_minus_utc(), -18000);
    }

    #[test]
    fn system_time_returns_clock_source_and_kolkata_details() {
        let result = get_system_time(Some("Asia/Kolkata")).unwrap();
        assert_eq!(result.source, "local_system_clock");
        assert_eq!(result.timezone, "Asia/Kolkata");
        assert_eq!(result.utc_offset, "+05:30");
        assert!(result.iso.contains("+05:30"));
        assert!(result.epoch_ms > 1_700_000_000_000);
    }

    #[test]
    fn system_time_rejects_malformed_timezone() {
        let err = get_system_time(Some("Mars/Olympus_Mons")).unwrap_err();
        assert!(err.contains("Unrecognized timezone"));
    }

    #[test]
    fn execute_tool_returns_valid_json() {
        let val = execute_system_time(&json!({"timezone": "Asia/Kolkata"})).unwrap();
        assert_eq!(val["utcOffset"], "+05:30");
        assert_eq!(val["source"], "local_system_clock");
    }
}
