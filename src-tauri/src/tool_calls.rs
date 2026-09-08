use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

#[derive(Default)]
pub struct ToolCalls(BTreeMap<u64, PartialCall>);
#[derive(Default)]
struct PartialCall {
    id: String,
    name: String,
    arguments: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}
impl ToolCalls {
    pub fn push(&mut self, delta: &Value) -> Result<(), String> {
        let Some(calls) = delta.get("tool_calls") else {
            return Ok(());
        };
        for call in calls.as_array().ok_or("Invalid tool call array.")? {
            let index = call["index"]
                .as_u64()
                .ok_or("Tool call index is missing.")?;
            if index >= 8 {
                return Err("The model requested more than eight tools in one round.".into());
            }
            let entry = self.0.entry(index).or_default();
            for (target, value) in [
                (&mut entry.id, &call["id"]),
                (&mut entry.name, &call["function"]["name"]),
                (&mut entry.arguments, &call["function"]["arguments"]),
            ] {
                if let Some(text) = value.as_str() {
                    target.push_str(text);
                } else if !value.is_null() {
                    return Err("Malformed tool call fragment.".into());
                }
            }
            if entry.id.len() > 128 || entry.name.len() > 128 || entry.arguments.len() > 65_536 {
                return Err("Tool call exceeds its size limit.".into());
            }
        }
        Ok(())
    }
    pub fn finish(self) -> Result<Vec<ToolCall>, String> {
        let mut ids = std::collections::HashSet::new();
        self.0
            .into_values()
            .map(|call| {
                if call.id.is_empty() || call.name.is_empty() || !ids.insert(call.id.clone()) {
                    return Err("Tool call has missing or duplicate identifiers.".into());
                }
                let arguments: Value = serde_json::from_str(&call.arguments)
                    .map_err(|_| "Tool arguments are not valid JSON.")?;
                if !arguments.is_object() {
                    return Err("Tool arguments must be a JSON object.".into());
                }
                Ok(ToolCall {
                    id: call.id,
                    name: call.name,
                    arguments,
                })
            })
            .collect()
    }
}
impl ToolCall {
    pub fn model_value(&self) -> Value {
        json!({"id":self.id,"type":"function","function":{"name":self.name,"arguments":self.arguments.to_string()}})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn assembles_interleaved_calls_in_index_order() {
        let mut calls = ToolCalls::default();
        calls.push(&json!({"tool_calls":[{"index":1,"id":"b","function":{"name":"second","arguments":"{}"}},{"index":0,"id":"a","function":{"name":"first","arguments":"{\"q\":"}}]})).unwrap();
        calls
            .push(&json!({"tool_calls":[{"index":0,"function":{"arguments":"\"café\"}"}}]}))
            .unwrap();
        let calls = calls.finish().unwrap();
        assert_eq!(calls[0].arguments["q"], "café");
        assert_eq!(calls[1].name, "second");
        assert_eq!(
            calls[0].model_value()["function"]["arguments"],
            "{\"q\":\"café\"}"
        );
    }
    #[test]
    fn rejects_malformed_and_excessive_calls() {
        for fragment in [
            json!({"tool_calls":{}}),
            json!({"tool_calls":[{"index":8}]}),
            json!({"tool_calls":[{"index":0,"function":{"arguments":{}}}]}),
        ] {
            assert!(ToolCalls::default().push(&fragment).is_err());
        }
        for arguments in ["[]", "{", "null"] {
            let mut calls = ToolCalls::default();
            calls.push(&json!({"tool_calls":[{"index":0,"id":"a","function":{"name":"tool","arguments":arguments}}]})).unwrap();
            assert!(calls.finish().is_err());
        }
    }
}
