//! Open-Meteo forecast lookup backing the `get_weather` harness tool.
//! The model supplies coordinates (no key, no geocoding); `location_name`
//! only labels the result.

use serde_json::{json, Value};

fn coordinate(args: &Value, key: &str, min: f64, max: f64) -> Result<f64, String> {
    let value = args.get(key).and_then(Value::as_f64).ok_or_else(|| {
        format!("get_weather needs a '{key}' number (e.g. {{\"latitude\": 22.57, \"longitude\": 88.36}}).")
    })?;
    if !(min..=max).contains(&value) {
        return Err(format!("get_weather '{key}' must be between {min} and {max}."));
    }
    Ok(value)
}

/// WMO weather-code -> short plain condition.
fn condition(code: Option<i64>) -> &'static str {
    match code {
        Some(0) => "clear sky",
        Some(1) => "mainly clear",
        Some(2) => "partly cloudy",
        Some(3) => "overcast",
        Some(45) | Some(48) => "fog",
        Some(51..=57) => "drizzle",
        Some(61..=67) => "rain",
        Some(71..=77) => "snow",
        Some(80..=82) => "rain showers",
        Some(85) | Some(86) => "snow showers",
        Some(95..=99) => "thunderstorm",
        _ => "unknown",
    }
}

fn shape_report(data: &Value, latitude: f64, longitude: f64, location_name: Option<&str>, url: &str) -> Result<Value, String> {
    let current = &data["current"];
    if !["temperature_2m", "apparent_temperature", "relative_humidity_2m", "wind_speed_10m"]
        .iter()
        .all(|key| current[key].is_number())
    {
        return Err("Weather response has missing measurements.".into());
    }
    let daily = &data["daily"];
    let forecast: Vec<Value> = daily["time"]
        .as_array()
        .map(|days| {
            days.iter()
                .take(8)
                .enumerate()
                .map(|(i, date)| {
                    json!({
                        "date": date,
                        "condition": condition(daily["weather_code"].get(i).and_then(Value::as_i64)),
                        "minC": daily["temperature_2m_min"].get(i),
                        "maxC": daily["temperature_2m_max"].get(i),
                        "precipitationProbabilityPercent": daily["precipitation_probability_max"].get(i),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(json!({
        "location": {
            "latitude": latitude,
            "longitude": longitude,
            "name": location_name,
            "timezone": data["timezone"],
        },
        "observedAt": current["time"],
        "current": {
            "temperatureC": current["temperature_2m"],
            "apparentTemperatureC": current["apparent_temperature"],
            "humidityPercent": current["relative_humidity_2m"],
            "windKph": current["wind_speed_10m"],
            "condition": condition(current["weather_code"].as_i64()),
        },
        "forecast": forecast,
        "source": "Open-Meteo",
        "sourceUrl": url,
    }))
}

/// Fetch current conditions and an 8-day daily forecast from Open-Meteo.
pub async fn get_weather(arguments: &Value) -> Result<Value, String> {
    let latitude = coordinate(arguments, "latitude", -90.0, 90.0)?;
    let longitude = coordinate(arguments, "longitude", -180.0, 180.0)?;
    let location_name = arguments
        .get("location_name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty());
    if location_name.map(str::len).unwrap_or(0) > 200 {
        return Err("get_weather 'location_name' must be at most 200 characters.".into());
    }
    let url = format!(
        "https://api.open-meteo.com/v1/forecast?latitude={latitude}&longitude={longitude}&current=temperature_2m,relative_humidity_2m,apparent_temperature,weather_code,wind_speed_10m&daily=weather_code,temperature_2m_max,temperature_2m_min,precipitation_probability_max&timezone=auto&forecast_days=8"
    );
    let response = reqwest::Client::builder()
        .https_only(true)
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|error| error.to_string())?
        .get(&url)
        .send()
        .await
        .map_err(|error| format!("Weather request failed: {error}"))?;
    if !response.status().is_success() {
        return Err(format!("Weather unavailable (HTTP {}).", response.status()));
    }
    let data: Value = response
        .json()
        .await
        .map_err(|error| format!("Invalid weather response: {error}"))?;
    shape_report(&data, latitude, longitude, location_name, &url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coordinates_are_bounded() {
        assert_eq!(coordinate(&json!({"longitude": -74}), "longitude", -180.0, 180.0).unwrap(), -74.0);
        assert!(coordinate(&json!({"latitude": 91}), "latitude", -90.0, 90.0).is_err());
        assert!(coordinate(&json!({}), "latitude", -90.0, 90.0).is_err());
        assert!(coordinate(&json!({"latitude": "22.57"}), "latitude", -90.0, 90.0).is_err());
    }

    #[test]
    fn wmo_codes_map_to_plain_conditions() {
        assert_eq!(condition(Some(0)), "clear sky");
        assert_eq!(condition(Some(61)), "rain");
        assert_eq!(condition(Some(95)), "thunderstorm");
        assert_eq!(condition(Some(200)), "unknown");
        assert_eq!(condition(None), "unknown");
    }

    #[test]
    fn shaped_report_keeps_eight_days_and_labels_location() {
        let times: Vec<String> = (1..=8).map(|d| format!("2026-10-{d:02}")).collect();
        let data = json!({
            "timezone": "Asia/Kolkata",
            "current": {"time": "2026-09-27T10:00", "temperature_2m": 31.2, "apparent_temperature": 35.0,
                "relative_humidity_2m": 70, "wind_speed_10m": 12.0, "weather_code": 2},
            "daily": {
                "time": times,
                "weather_code": [0, 1, 2, 3, 61, 71, 80, 95],
                "temperature_2m_max": [34.0, 34.1, 33.5, 33.0, 30.2, 29.8, 31.4, 30.9],
                "temperature_2m_min": [25.0, 25.4, 24.9, 24.5, 24.0, 23.8, 24.4, 24.1],
                "precipitation_probability_max": [0, 0, 5, 10, 60, 20, 30, 45],
            },
        });
        let report = shape_report(&data, 22.57, 88.36, Some("Kolkata"), "https://example.test/forecast").unwrap();
        assert_eq!(report["location"]["name"], "Kolkata");
        assert_eq!(report["location"]["timezone"], "Asia/Kolkata");
        assert_eq!(report["current"]["condition"], "partly cloudy");
        let forecast = report["forecast"].as_array().unwrap();
        assert_eq!(forecast.len(), 8);
        assert_eq!(forecast[4]["condition"], "rain");
        assert_eq!(forecast[4]["precipitationProbabilityPercent"], 60);
    }

    #[test]
    fn shaped_report_rejects_missing_measurements() {
        let data = json!({"current": {"temperature_2m": 31.2}});
        assert!(shape_report(&data, 0.0, 0.0, None, "u").is_err());
    }
}
