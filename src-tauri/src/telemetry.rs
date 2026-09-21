//! Per-round inference telemetry (docs/EXTENSIONS.md §1.7).
//!
//! llama-server attaches a `timings` object to the final streamed chunk:
//! `prompt_n` prompt tokens evaluated, `cache_n` prompt tokens reused from the
//! slot's KV cache, their times and rates, and the same for generated tokens.
//! Only values the server actually reported are kept. A provider that sends
//! no timings yields `None`, which the UI shows as unavailable, never as an
//! estimate.

use serde::Serialize;
use serde_json::Value;

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoundTimings {
    /// Prompt tokens reused from the KV cache.
    pub cache_n: Option<u64>,
    /// Prompt tokens evaluated in this request.
    pub prompt_n: Option<u64>,
    pub prompt_ms: Option<f64>,
    pub prompt_per_second: Option<f64>,
    /// Generated tokens.
    pub predicted_n: Option<u64>,
    pub predicted_ms: Option<f64>,
    pub predicted_per_second: Option<f64>,
}

fn count(object: &Value, key: &str) -> Option<u64> {
    object.get(key).and_then(Value::as_u64)
}

fn measure(object: &Value, key: &str) -> Option<f64> {
    object.get(key).and_then(Value::as_f64).filter(|value| value.is_finite() && *value >= 0.0)
}

impl RoundTimings {
    /// Parse the `timings` object of one streamed chunk, if it has one.
    pub fn from_chunk(chunk: &Value) -> Option<Self> {
        let timings = chunk.get("timings").filter(|value| value.is_object())?;
        let parsed = Self {
            cache_n: count(timings, "cache_n"),
            prompt_n: count(timings, "prompt_n"),
            prompt_ms: measure(timings, "prompt_ms"),
            prompt_per_second: measure(timings, "prompt_per_second"),
            predicted_n: count(timings, "predicted_n"),
            predicted_ms: measure(timings, "predicted_ms"),
            predicted_per_second: measure(timings, "predicted_per_second"),
        };
        (parsed != Self::default()).then_some(parsed)
    }

    /// Share of the prompt served from cache, when both counts were reported.
    pub fn cache_hit_ratio(&self) -> Option<f64> {
        let (cached, evaluated) = (self.cache_n?, self.prompt_n?);
        let total = cached + evaluated;
        (total > 0).then(|| cached as f64 / total as f64)
    }

    /// Payload for the `timings` session event and the live chat event.
    pub fn event(&self, round: usize, id_slot: Option<i64>) -> Value {
        let mut value = serde_json::to_value(self).unwrap_or(Value::Null);
        value["round"] = round.into();
        value["idSlot"] = id_slot.into();
        value["cacheHitRatio"] = self.cache_hit_ratio().into();
        value["source"] = "llama-server".into();
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_reported_timings_and_cache_ratio() {
        let chunk = json!({"choices":[{"delta":{},"finish_reason":"stop"}],"timings":{
            "cache_n": 900, "prompt_n": 100, "prompt_ms": 250.0, "prompt_per_second": 400.0,
            "predicted_n": 64, "predicted_ms": 3200.0, "predicted_per_second": 20.0}});
        let timings = RoundTimings::from_chunk(&chunk).unwrap();
        assert_eq!(timings.cache_n, Some(900));
        assert_eq!(timings.predicted_n, Some(64));
        assert_eq!(timings.cache_hit_ratio(), Some(0.9));
        let event = timings.event(2, Some(0));
        assert_eq!(event["cacheN"], 900);
        assert_eq!(event["round"], 2);
        assert_eq!(event["idSlot"], 0);
        assert_eq!(event["source"], "llama-server");
    }

    #[test]
    fn missing_or_invalid_values_stay_unavailable() {
        assert!(RoundTimings::from_chunk(&json!({"choices": []})).is_none());
        assert!(RoundTimings::from_chunk(&json!({"timings": {}})).is_none());
        let partial = RoundTimings::from_chunk(&json!({"timings": {"prompt_n": 12, "prompt_ms": -1.0}})).unwrap();
        assert_eq!(partial.prompt_ms, None);
        assert_eq!(partial.cache_hit_ratio(), None);
        let event = partial.event(0, None);
        assert!(event["cacheHitRatio"].is_null());
        assert!(event["idSlot"].is_null());
    }
}
