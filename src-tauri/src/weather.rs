use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct WeatherConfig {
    pub enabled: bool,
    pub temperature_unit: String,
    pub location: String,
}
impl Default for WeatherConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            temperature_unit: "celsius".into(),
            location: String::new(),
        }
    }
}
impl WeatherConfig {
    pub fn validate(&self) -> Result<(), String> {
        if !["celsius", "fahrenheit"].contains(&self.temperature_unit.as_str())
            || self.location.chars().count() > 200
        {
            return Err(
                "Choose Celsius or Fahrenheit and a location of at most 200 characters.".into(),
            );
        }
        Ok(())
    }
}
#[tauri::command]
pub fn get_weather_config(
    state: tauri::State<'_, crate::AppState>,
) -> Result<WeatherConfig, String> {
    state.database()?.setting("weather_config")
}
#[tauri::command]
pub async fn save_weather_config(
    state: tauri::State<'_, crate::AppState>,
    config: WeatherConfig,
) -> Result<(), String> {
    let _guard = state
        .operation
        .try_lock()
        .map_err(|_| "Wait for the active operation.")?;
    config.validate()?;
    state.database()?.save_setting("weather_config", &config)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preferences_persist_without_guessing_a_location() {
        let store = crate::store::Store::open_memory().unwrap();
        let mut config: WeatherConfig = store.setting("weather_config").unwrap();
        assert!(config.enabled);
        assert!(config.location.is_empty());
        config.temperature_unit = "fahrenheit".into();
        config.location = "Tokyo, Japan".into();
        config.enabled = false;
        config.validate().unwrap();
        store.save_setting("weather_config", &config).unwrap();
        let loaded: WeatherConfig = store.setting("weather_config").unwrap();
        assert!(!loaded.enabled);
        assert_eq!(loaded.location, "Tokyo, Japan");
        assert_eq!(loaded.temperature_unit, "fahrenheit");
        config.temperature_unit = "kelvin".into();
        assert!(config.validate().is_err());
    }
}
