use chrono::{DateTime, Local, Offset, TimeZone, Utc};
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

/// Resolve an IANA timezone's current offset, a fixed alias, or numeric UTC offset.
pub fn parse_iana_offset(tz_name: &str) -> Option<chrono::FixedOffset> {
    parse_iana_offset_at(tz_name, Utc::now())
}

fn parse_iana_offset_at(tz_name: &str, now: DateTime<Utc>) -> Option<chrono::FixedOffset> {
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
        if !rest.is_ascii() { return None; }
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

    // Legacy abbreviations explicitly denote fixed offsets; IANA regions follow DST.
    let seconds = match normalized.to_ascii_lowercase().as_str() {
        "ist" => 19_800, "cst" => 28_800, "jst" => 32_400, "wwe" => 0,
        "cet" => 3_600, "eet" => 7_200, "est" => -18_000, "mst" => -25_200,
        "pst" => -28_800, "aest" => 36_000, "nzst" => 43_200,
        _ => {
            let timezone = chrono_tz::TZ_VARIANTS.iter().find(|tz| tz.name().eq_ignore_ascii_case(normalized))?;
            return Some(now.with_timezone(timezone).offset().fix());
        }
    };
    chrono::FixedOffset::east_opt(seconds)
}

/// Query the deterministic system clock with optional IANA timezone translation.
pub fn get_system_time(timezone_arg: Option<&str>) -> Result<SystemTimeReport, String> {
    get_system_time_at(timezone_arg, Utc::now())
}

fn get_system_time_at(timezone_arg: Option<&str>, now_utc: DateTime<Utc>) -> Result<SystemTimeReport, String> {
    let epoch_ms = now_utc.timestamp_millis();

    if let Some(tz_str) = timezone_arg {
        let tz_trimmed = tz_str.trim();
        if !tz_trimmed.is_empty() {
            if let Some(offset) = parse_iana_offset_at(tz_trimmed, now_utc) {
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
    let now_local: DateTime<Local> = now_utc.with_timezone(&Local);
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
    fn system_time_rejects_non_ascii_numeric_offsets_without_panicking() {
        for timezone in ["+😀", "-éé", "+1界"] {
            assert!(parse_iana_offset(timezone).is_none());
            assert!(execute_system_time(&json!({"timezone": timezone})).is_err());
        }
    }

    #[test]
    fn iana_regions_follow_winter_and_summer_rules_at_the_reported_instant() {
        let winter = Utc.with_ymd_and_hms(2026, 1, 15, 12, 0, 0).unwrap();
        let summer = Utc.with_ymd_and_hms(2026, 7, 15, 12, 0, 0).unwrap();
        for (zone, january, july) in [
            ("America/New_York", "-05:00", "-04:00"),
            ("europe/london", "+00:00", "+01:00"),
            ("Australia/Sydney", "+11:00", "+10:00"),
            ("Asia/Kathmandu", "+05:45", "+05:45"),
        ] {
            for (instant, expected) in [(winter, january), (summer, july)] {
                let report = get_system_time_at(Some(zone), instant).unwrap();
                assert_eq!(report.utc_offset, expected, "{zone} at {instant}");
                assert!(report.iso.ends_with(expected));
                assert_eq!(report.epoch_ms, instant.timestamp_millis());
            }
        }
    }

    #[test]
    fn fixed_aliases_and_numeric_offsets_do_not_follow_daylight_saving() {
        let summer = Utc.with_ymd_and_hms(2026, 7, 15, 12, 0, 0).unwrap();
        for (zone, expected) in [("EST", "-05:00"), ("CET", "+01:00"), ("+0530", "+05:30"), ("-4", "-04:00")] {
            assert_eq!(get_system_time_at(Some(zone), summer).unwrap().utc_offset, expected);
        }
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
