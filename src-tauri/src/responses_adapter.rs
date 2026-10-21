//! Explicit native Responses protocol. No HTTP, credentials, execution or
//! provider guessing lives here; malformed requests/streams fail before tools.
use serde_json::{json, Value};
use std::collections::BTreeMap;
const MAX_BYTES: usize = 4 * 1024 * 1024;
const MAX_ITEMS: usize = 128;
fn string<'a>(value: &'a Value, key: &str) -> Result<&'a str, String> {
    value[key]
        .as_str()
        .ok_or_else(|| format!("Responses {key} must be a string."))
}
fn valid_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 128 && !value.chars().any(char::is_control)
}
fn optional_bool(value: &Value, field: &str, default: bool) -> Result<bool, String> {
    match value.get(field) {
        None => Ok(default),
        Some(value) => value
            .as_bool()
            .ok_or_else(|| format!("Responses {field} must be a boolean.")),
    }
}
fn native_item(item: &Value) -> Result<(), String> {
    if !item.is_object() {
        return Err("Responses output item must be an object.".into());
    }
    match item["type"].as_str(){
        Some("function_call")=>{
            if item.get("status").is_some_and(|status|status!="completed"){return Err("Responses function output is not completed.".into());}
            for key in ["call_id","name"]{if !valid_id(string(item,key)?){return Err(format!("Invalid Responses function {key}."));}}
            let arguments:Value=serde_json::from_str(string(item,"arguments")?).map_err(|_|"Responses function arguments are incomplete JSON.")?;
            if !arguments.is_object(){return Err("Responses function arguments must be an object.".into());}
        },
        Some("message")=>{
            if item["role"]!="assistant"{return Err("Responses output message must have assistant role.".into());}
            for part in item["content"].as_array().ok_or("Responses message content must be an array.")?{
                match part["type"].as_str(){Some("output_text")=>{string(part,"text")?;},Some("refusal")=>{string(part,"refusal")?;},_=>return Err("Unsupported Responses message content.".into())}
            }
        },
        Some("reasoning")=>{
            if let Some(encrypted)=item.get("encrypted_content"){if !encrypted.is_null()&&!encrypted.is_string(){return Err("Invalid encrypted Responses reasoning context.".into());}}
        },
        _=>return Err("Unsupported Responses output item; built-in server tools are not enabled by this harness.".into()),
    }
    Ok(())
}
fn content(message: &Value) -> Result<Value, String> {
    match &message["content"] {
        Value::Null => Ok(json!("")),
        Value::String(text) => Ok(json!(text)),
        Value::Array(parts) => {
            let mut result = Vec::new();
            for part in parts {
                result.push(match part["type"].as_str(){
                    Some("text")=>json!({"type":"input_text","text":string(part,"text")?}),
                    Some("image_url")=>{
                        if message["role"]!="user"{return Err("Image input requires a user message.".into());}
                        let url=string(&part["image_url"],"url")?;
                        if !["data:image/png;base64,","data:image/jpeg;base64,","data:image/webp;base64,"].iter().any(|prefix|url.starts_with(prefix)){return Err("Responses images must be embedded supported images; remote image fetches are not allowed.".into());}
                        let mut image=json!({"type":"input_image","image_url":url});
                        if let Some(detail)=part["image_url"].get("detail"){if !matches!(detail.as_str(),Some("auto"|"low"|"high")){return Err("Invalid Responses image detail.".into());}image["detail"]=detail.clone();}image
                    },_=>return Err("Unsupported Responses input content part.".into()),
                });
            }
            Ok(json!(result))
        }
        _ => Err("Invalid Responses input message content.".into()),
    }
}
pub fn request(internal: &Value, model: &str) -> Result<Value, String> {
    crate::providers::validate_model_id(model)?;
    let messages = internal["messages"]
        .as_array()
        .ok_or("Responses request needs canonical messages.")?;
    let mut input = Vec::new();
    for message in messages {
        let role = string(message, "role")?;
        if role == "tool" {
            let id = string(message, "tool_call_id")?;
            if !valid_id(id) {
                return Err("Invalid Responses tool-result call ID.".into());
            }
            input.push(json!({"type":"function_call_output","call_id":id,"output":string(message,"content")?}));
            continue;
        }
        if !matches!(role, "system" | "developer" | "user" | "assistant") {
            return Err("Unsupported Responses input role.".into());
        }
        if role == "assistant" {
            if let Some(native) = message.get("locallm_responses_output") {
                let items = native.as_array().ok_or("Invalid saved Responses output.")?;
                if items.len() > MAX_ITEMS || native.to_string().len() > MAX_BYTES {
                    return Err("Saved Responses output exceeds its bound.".into());
                }
                let mut native_calls = Vec::new();
                for item in items {
                    native_item(item)?;
                    if item["type"] == "function_call" {
                        native_calls.push(json!({"id":item["call_id"],"name":item["name"],"arguments":serde_json::from_str::<Value>(string(item,"arguments")?).map_err(|_|"Invalid saved Responses arguments.")?}));
                    }
                }
                let mut canonical = Vec::new();
                for call in message["tool_calls"].as_array().into_iter().flatten() {
                    if call["type"] != "function" {
                        return Err("Invalid canonical Responses tool history.".into());
                    }
                    canonical.push(json!({"id":call["id"],"name":call["function"]["name"],"arguments":serde_json::from_str::<Value>(string(&call["function"],"arguments")?).map_err(|_|"Invalid canonical Responses arguments.")?}));
                }
                native_calls.sort_by_cached_key(Value::to_string);
                canonical.sort_by_cached_key(Value::to_string);
                if native_calls != canonical {
                    return Err(
                        "Saved Responses function calls conflict with canonical history.".into(),
                    );
                }
                input.extend(items.iter().cloned());
                continue;
            }
        }
        let text = content(message)?;
        if text.as_str() != Some("") {
            input.push(json!({"role":role,"content":text}));
        }
        if let Some(calls) = message.get("tool_calls").filter(|v| !v.is_null()) {
            if role != "assistant" {
                return Err("Only assistant messages may contain function calls.".into());
            }
            for call in calls
                .as_array()
                .ok_or("Invalid canonical tool-call history.")?
            {
                if call["type"] != "function" {
                    return Err("Unsupported canonical tool-call type.".into());
                }
                let item = json!({"type":"function_call","call_id":call["id"],"name":call["function"]["name"],"arguments":call["function"]["arguments"]});
                native_item(&item)?;
                input.push(item);
            }
        }
    }
    let mut out = json!({"model":model,"input":input,"store":false,"stream":optional_bool(internal,"stream",true)?});
    let mut limit = None;
    for field in ["max_tokens", "max_completion_tokens", "max_output_tokens"] {
        if let Some(value) = internal.get(field) {
            let value = value
                .as_u64()
                .filter(|v| *v > 0 && *v <= u64::from(u32::MAX))
                .ok_or("Invalid Responses output-token limit.")?;
            if limit.is_some_and(|previous| previous != value) {
                return Err("Conflicting Responses output-token limits.".into());
            }
            limit = Some(value);
        }
    }
    if let Some(limit) = limit {
        out["max_output_tokens"] = json!(limit);
    }
    if let Some(reasoning) = internal.get("reasoning") {
        let object = reasoning
            .as_object()
            .ok_or("Responses reasoning must be an object.")?;
        if object
            .keys()
            .any(|key| !matches!(key.as_str(), "effort" | "summary"))
        {
            return Err("Unsupported Responses reasoning option.".into());
        }
        if reasoning.get("effort").is_some_and(|value| {
            !matches!(
                value.as_str(),
                Some("none" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max")
            )
        }) {
            return Err("Invalid Responses reasoning effort.".into());
        }
        if reasoning
            .get("summary")
            .is_some_and(|value| !matches!(value.as_str(), Some("auto" | "concise" | "detailed")))
        {
            return Err("Invalid Responses reasoning summary.".into());
        }
        out["reasoning"] = reasoning.clone();
    }
    for (field, max) in [("temperature", 2.0), ("top_p", 1.0)] {
        if let Some(value) = internal.get(field) {
            if !value
                .as_f64()
                .is_some_and(|n| n.is_finite() && (0.0..=max).contains(&n))
            {
                return Err(format!("Invalid Responses {field}."));
            }
            out[field] = value.clone();
        }
    }
    if let Some(value) = internal.get("parallel_tool_calls") {
        if !value.is_boolean() {
            return Err("Responses parallel_tool_calls must be a boolean.".into());
        }
        out["parallel_tool_calls"] = value.clone();
    }
    if let Some(tools) = internal.get("tools") {
        let tools = tools
            .as_array()
            .ok_or("Responses tools must be an array.")?;
        if tools.len() > 64 {
            return Err("Responses tool schema limit exceeded64.".into());
        }
        let mut converted = Vec::new();
        let mut names = std::collections::HashSet::new();
        for tool in tools {
            if tool["type"] != "function" {
                return Err(
                    "Only client-owned function tools are supported by this Responses adapter."
                        .into(),
                );
            }
            let function = &tool["function"];
            let name = string(function, "name")?;
            if !valid_id(name) || !names.insert(name) {
                return Err("Invalid or duplicate Responses tool name.".into());
            }
            crate::tool_validation::compile(&function["parameters"])?;
            let mut native = json!({"type":"function","name":name,"parameters":function["parameters"],"strict":optional_bool(function,"strict",false)?});
            if let Some(description) = function.get("description") {
                if !description.is_string() {
                    return Err("Invalid Responses tool description.".into());
                }
                native["description"] = description.clone();
            }
            converted.push(native);
        }
        out["tools"] = json!(converted);
    }
    if let Some(choice) = internal.get("tool_choice") {
        if choice == "required"
            && !out["tools"]
                .as_array()
                .is_some_and(|tools| !tools.is_empty())
        {
            return Err("Required Responses tool choice needs an offered tool.".into());
        }
        out["tool_choice"] = match choice.as_str() {
            Some("auto" | "none" | "required") => choice.clone(),
            Some(_) => return Err("Invalid Responses tool choice.".into()),
            None => {
                if choice["type"] != "function" {
                    return Err("Unsupported Responses tool choice.".into());
                }
                let name = string(&choice["function"], "name")?;
                if !out["tools"]
                    .as_array()
                    .is_some_and(|tools| tools.iter().any(|tool| tool["name"] == name))
                {
                    return Err("Forced Responses tool is not offered.".into());
                }
                json!({"type":"function","name":name})
            }
        };
    }
    if let Some(format) = internal.get("response_format") {
        out["text"] = json!({"format":match format["type"].as_str(){
            Some("json_object")=>json!({"type":"json_object"}),Some("text")=>json!({"type":"text"}),
            Some("json_schema")=>{let spec=&format["json_schema"];let name=string(spec,"name")?;if !valid_id(name){return Err("Invalid Responses output-schema name.".into());}crate::tool_validation::compile(&spec["schema"])?;
                let mut result=json!({"type":"json_schema","name":name,"schema":spec["schema"],"strict":optional_bool(spec,"strict",false)?});if let Some(description)=spec.get("description"){if !description.is_string(){return Err("Invalid Responses output-schema description.".into());}result["description"]=description.clone();}result},
            _=>return Err("Unsupported Responses output format.".into()),
        }});
    }
    if out.to_string().len() > 16 * 1024 * 1024 {
        return Err("Responses request exceeds16MiB.".into());
    }
    Ok(out)
}

#[derive(Default)]
struct Item {
    id: String,
    kind: String,
    slot: Option<u64>,
    call_id: String,
    name: String,
    arguments: String,
    parts: BTreeMap<(u64, String), String>,
    done: bool,
}
#[derive(Default)]
pub struct Stream {
    items: BTreeMap<u64, Item>,
    output: BTreeMap<u64, Value>,
    calls: usize,
    bytes: usize,
    pub completed: bool,
}
impl Stream {
    fn account(&mut self, bytes: usize) -> Result<(), String> {
        self.bytes = self.bytes.saturating_add(bytes);
        if self.bytes > MAX_BYTES {
            return Err("Responses stream exceeded4MiB.".into());
        }
        Ok(())
    }
    fn index(value: &Value) -> Result<u64, String> {
        value["output_index"]
            .as_u64()
            .filter(|i| *i < MAX_ITEMS as u64)
            .ok_or("Invalid Responses output index.".into())
    }
    fn start(&mut self, index: u64, item: &Value, delta: &mut Value) -> Result<(), String> {
        let kind = string(item, "type")?;
        let id = string(item, "id")?;
        if !valid_id(id) {
            return Err("Invalid Responses output-item ID.".into());
        }
        if self.items.contains_key(&index) || self.items.values().any(|item| item.id == id) {
            return Err("Duplicate Responses output item.".into());
        }
        if !matches!(kind, "message" | "reasoning" | "function_call") {
            return Err("Unsupported server-owned Responses tool or output item.".into());
        }
        let mut state = Item {
            id: id.into(),
            kind: kind.into(),
            ..Default::default()
        };
        if kind == "function_call" {
            if self
                .items
                .iter()
                .any(|(existing, item)| *existing > index && item.kind == "function_call")
            {
                return Err("Responses function items arrived out of execution order.".into());
            }
            let call_id = string(item, "call_id")?;
            let name = string(item, "name")?;
            if !valid_id(call_id)
                || !valid_id(name)
                || self.calls >= 8
                || self.items.values().any(|i| i.call_id == call_id)
            {
                return Err("Invalid, duplicate or excessive Responses function call.".into());
            }
            state.slot = Some(self.calls as u64);
            state.call_id = call_id.into();
            state.name = name.into();
            self.calls += 1;
            delta["tool_calls"].as_array_mut().unwrap().push(json!({"index":state.slot,"type":"function","id":call_id,"function":{"name":name,"arguments":""}}));
        }
        self.items.insert(index, state);
        Ok(())
    }
    fn argument(
        &mut self,
        index: u64,
        item_id: Option<&str>,
        text: &str,
        final_value: bool,
        delta: &mut Value,
    ) -> Result<(), String> {
        let state = self
            .items
            .get_mut(&index)
            .ok_or("Responses argument arrived before its item.")?;
        if state.kind != "function_call" || state.done || item_id.is_some_and(|id| id != state.id) {
            return Err("Responses argument does not match its active function item.".into());
        }
        let fragment = if final_value {
            text.strip_prefix(&state.arguments)
                .ok_or("Conflicting final Responses function arguments.")?
        } else {
            text
        };
        if state.arguments.len() + fragment.len() > 65536 {
            return Err("Responses function arguments exceed64KiB.".into());
        }
        if !fragment.is_empty() {
            state.arguments.push_str(fragment);
            delta["tool_calls"]
                .as_array_mut()
                .unwrap()
                .push(json!({"index":state.slot,"function":{"arguments":fragment}}));
        }
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    fn part(
        &mut self,
        index: u64,
        part: u64,
        item_id: Option<&str>,
        field: &str,
        text: &str,
        final_value: bool,
        delta: &mut Value,
    ) -> Result<(), String> {
        let state = self
            .items
            .get_mut(&index)
            .ok_or("Responses text arrived before its output item.")?;
        if state.done || item_id.is_some_and(|id| id != state.id) || part >= 128 {
            return Err("Invalid Responses text item or part index.".into());
        }
        if (field == "reasoning_content" && state.kind != "reasoning")
            || (field == "content" && state.kind != "message")
        {
            return Err("Responses text kind does not match its item.".into());
        }
        let accumulated = state.parts.entry((part, field.into())).or_default();
        let fragment = if final_value {
            text.strip_prefix(accumulated.as_str())
                .ok_or("Conflicting final Responses text.")?
        } else {
            text
        };
        accumulated.push_str(fragment);
        let previous = delta[field].as_str().unwrap_or("").to_string();
        delta[field] = json!(previous + fragment);
        Ok(())
    }
    fn finish_item(&mut self, index: u64, item: &Value, delta: &mut Value) -> Result<(), String> {
        native_item(item)?;
        if !self.items.contains_key(&index) {
            self.start(index, item, delta)?;
        }
        let state = &self.items[&index];
        if state.id != string(item, "id")? || state.kind != string(item, "type")? {
            return Err("Conflicting final Responses output identity.".into());
        }
        if state.done {
            if self.output.get(&index) == Some(item) {
                return Ok(());
            }
            return Err("Conflicting repeated Responses output item.".into());
        }
        match item["type"].as_str() {
            Some("function_call") => {
                if state.call_id != string(item, "call_id")? || state.name != string(item, "name")?
                {
                    return Err("Conflicting final Responses function identity.".into());
                }
                self.argument(index, None, string(item, "arguments")?, true, delta)?;
            }
            Some("message") => {
                for (part, value) in item["content"].as_array().unwrap().iter().enumerate() {
                    let text = if value["type"] == "refusal" {
                        string(value, "refusal")?
                    } else {
                        string(value, "text")?
                    };
                    self.part(index, part as u64, None, "content", text, true, delta)?;
                }
            }
            Some("reasoning") => {
                if let Some(parts) = item["summary"].as_array() {
                    for (part, value) in parts.iter().enumerate() {
                        if value["type"] != "summary_text" {
                            return Err("Unsupported Responses reasoning summary.".into());
                        }
                        self.part(
                            index,
                            part as u64,
                            None,
                            "reasoning_content",
                            string(value, "text")?,
                            true,
                            delta,
                        )?;
                    }
                }
            }
            _ => unreachable!(),
        }
        self.items.get_mut(&index).unwrap().done = true;
        self.output.insert(index, item.clone());
        Ok(())
    }
    pub fn push(&mut self, value: &Value) -> Result<Option<Value>, String> {
        if self.completed {
            return Err("Responses event after completion.".into());
        }
        self.account(value.to_string().len())?;
        let kind = string(value, "type")?;
        let mut delta = json!({"tool_calls":[]});
        let mut finish = None;
        let mut usage = Value::Null;
        match kind {
            "error" => return Ok(Some(json!({"error":value}))),
            "response.failed" => return Ok(Some(json!({"error":value["response"]["error"]}))),
            "response.output_item.added" => {
                let index = Self::index(value)?;
                self.start(index, &value["item"], &mut delta)?;
                if value["item"]["type"] == "function_call" {
                    self.argument(
                        index,
                        None,
                        string(&value["item"], "arguments")?,
                        false,
                        &mut delta,
                    )?;
                }
            }
            "response.function_call_arguments.delta" | "response.function_call_arguments.done" => {
                let index = Self::index(value)?;
                let final_value = kind.ends_with(".done");
                self.argument(
                    index,
                    Some(string(value, "item_id")?),
                    string(value, if final_value { "arguments" } else { "delta" })?,
                    final_value,
                    &mut delta,
                )?;
            }
            "response.output_text.delta"
            | "response.output_text.done"
            | "response.refusal.delta"
            | "response.refusal.done"
            | "response.reasoning_summary_text.delta"
            | "response.reasoning_summary_text.done" => {
                let index = Self::index(value)?;
                let reasoning = kind.contains("reasoning_summary");
                let part = value[if reasoning {
                    "summary_index"
                } else {
                    "content_index"
                }]
                .as_u64()
                .ok_or("Missing Responses text part index.")?;
                let final_value = kind.ends_with(".done");
                let text = string(
                    value,
                    if !final_value {
                        "delta"
                    } else if kind == "response.refusal.done" {
                        "refusal"
                    } else {
                        "text"
                    },
                )?;
                self.part(
                    index,
                    part,
                    Some(string(value, "item_id")?),
                    if reasoning {
                        "reasoning_content"
                    } else {
                        "content"
                    },
                    text,
                    final_value,
                    &mut delta,
                )?;
            }
            "response.output_item.done" => {
                self.finish_item(Self::index(value)?, &value["item"], &mut delta)?
            }
            "response.completed" | "response.incomplete" => {
                let response = &value["response"];
                let incomplete = kind == "response.incomplete";
                if response["status"]
                    != if incomplete {
                        "incomplete"
                    } else {
                        "completed"
                    }
                {
                    return Err("Responses terminal event/status mismatch.".into());
                }
                let output = response["output"]
                    .as_array()
                    .ok_or("Responses terminal event is missing output.")?;
                if output.len() > MAX_ITEMS {
                    return Err("Responses output item limit exceeded.".into());
                }
                for (index, item) in output.iter().enumerate() {
                    self.finish_item(index as u64, item, &mut delta)?;
                }
                if self.items.len() != output.len() || self.items.values().any(|item| !item.done) {
                    return Err("Responses terminal output omits streamed items.".into());
                }
                if incomplete && response["incomplete_details"]["reason"] != "max_output_tokens" {
                    return Err("Responses generation was incomplete; no tool was executed.".into());
                }
                finish = Some(if incomplete {
                    "length"
                } else if self.calls > 0 {
                    "tool_calls"
                } else {
                    "stop"
                });
                self.completed = true;
                if let Some(raw) = response.get("usage").filter(|v| !v.is_null()) {
                    usage = json!({"prompt_tokens":raw["input_tokens"],"completion_tokens":raw["output_tokens"],"prompt_tokens_details":{"cached_tokens":raw["input_tokens_details"]["cached_tokens"]}});
                }
            }
            "response.created"
            | "response.in_progress"
            | "response.queued"
            | "response.content_part.added"
            | "response.content_part.done"
            | "response.reasoning_summary_part.added"
            | "response.reasoning_summary_part.done"
            | "response.output_text.annotation.added" => {}
            _ => return Err(format!("Unsupported Responses stream event '{kind}'.")),
        }
        if delta["tool_calls"].as_array().unwrap().is_empty() {
            delta.as_object_mut().unwrap().remove("tool_calls");
        }
        if delta.as_object().unwrap().is_empty() && finish.is_none() && usage.is_null() {
            return Ok(None);
        }
        let mut out = json!({"choices":[{"delta":delta}]});
        if let Some(finish) = finish {
            out["choices"][0]["finish_reason"] = json!(finish);
        }
        if !usage.is_null() {
            out["usage"] = usage;
        }
        Ok(Some(out))
    }
    pub fn replay(&self) -> Option<Value> {
        if self.completed {
            Some(json!(self.output.values().collect::<Vec<_>>()))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn function() -> Value {
        json!({"type":"function","function":{"name":"echo","description":"Echo","parameters":{"type":"object","properties":{"input":{"type":"string"}},"required":["input"],"additionalProperties":false}}})
    }
    fn call() -> Value {
        json!({"type":"function_call","id":"fc-id","call_id":"call-real","name":"echo","arguments":"{\"input\":\"世界\"}","status":"completed"})
    }
    fn complete(output: Value) -> Value {
        json!({"type":"response.completed","response":{"status":"completed","output":output,"usage":{"input_tokens":20,"output_tokens":3,"input_tokens_details":{"cached_tokens":7}}}})
    }
    #[test]
    fn request_preserves_tool_identity_images_named_choice_schema_and_local_only_boundary() {
        let request=request(&json!({"messages":[{"role":"system","content":"Instructions"},{"role":"user","content":[{"type":"text","text":"Look"},{"type":"image_url","image_url":{"url":"data:image/png;base64,aGVsbG8=","detail":"low"}}]},
            {"role":"assistant","content":"","tool_calls":[{"id":"call-real","type":"function","function":{"name":"echo","arguments":"{\"input\":\"hello\"}"}}]},
            {"role":"tool","tool_call_id":"call-real","content":"{\"processed\":true}"}],"tools":[function()],"tool_choice":{"type":"function","function":{"name":"echo"}},"parallel_tool_calls":false,
            "max_tokens":64,"cache_prompt":true,"id_slot":9,"repeat_penalty":1.2,"locallm_reasoning_mode":"think","response_format":{"type":"json_schema","json_schema":{"name":"answer","strict":true,"schema":{"type":"object","properties":{},"additionalProperties":false}}}}),"configured-model").unwrap();
        assert_eq!(request["store"], false);
        assert_eq!(request["max_output_tokens"], 64);
        assert_eq!(request["input"][1]["content"][1]["type"], "input_image");
        assert_eq!(request["input"][2]["call_id"], "call-real");
        assert!(request["input"][2].get("id").is_none());
        assert_eq!(request["input"][3]["type"], "function_call_output");
        assert_eq!(request["tools"][0]["name"], "echo");
        assert_eq!(request["tools"][0]["strict"], false);
        assert_eq!(
            request["tool_choice"],
            json!({"type":"function","name":"echo"})
        );
        assert_eq!(
            request["text"]["format"]["schema"]["additionalProperties"],
            false
        );
        for key in [
            "cache_prompt",
            "id_slot",
            "repeat_penalty",
            "locallm_reasoning_mode",
            "messages",
            "max_tokens",
        ] {
            assert!(request.get(key).is_none(), "{key}");
        }
    }
    #[test]
    fn request_errors_are_explicit_before_send() {
        for value in [
            json!({}),
            json!({"messages":[],"max_tokens":8,"max_output_tokens":9}),
            json!({"messages":[],"tool_choice":"required"}),
            json!({"messages":[],"tools":[{"type":"web_search"}]}),
            json!({"messages":[{"role":"user","content":[{"type":"image_url","image_url":{"url":"https://private.example/image.png"}}]}]}),
            json!({"messages":[],"parallel_tool_calls":"yes"}),
            json!({"messages":[],"stream":null}),
            json!({"messages":[],"reasoning":{"effort":"bogus"}}),
            json!({"messages":[],"reasoning":{"summary":"bogus"}}),
            json!({"messages":[],"reasoning":{"unexpected":true}}),
            json!({"messages":[],"tools":[{"type":"function","function":{"name":"echo","parameters":{"type":"object"},"strict":"yes"}}]}),
            json!({"messages":[],"response_format":{"type":"json_schema","json_schema":{"name":"result","schema":{"type":"object"},"strict":null}}}),
            json!({"messages":[],"response_format":{"type":"json_schema","json_schema":{"name":"result","schema":{"type":"object"},"description":42}}}),
            json!({"messages":[],"temperature":99}),
            json!({"messages":[],"tool_choice":{"type":"function","function":{"name":"not-offered"}}}),
        ] {
            assert!(request(&value, "configured-model").is_err(), "{value}");
        }
    }
    #[test]
    fn completed_stream_keeps_opaque_history_dense_call_ids_usage_and_exact_arguments() {
        let mut stream = Stream::default();
        let mut calls = crate::tool_calls::ToolCalls::default();
        let mut text = String::new();
        let mut usage = crate::inference::InferenceUsage::default();
        let reason = json!({"type":"reasoning","id":"rs-id","summary":[{"type":"summary_text","text":"Brief summary"}],"encrypted_content":"opaque-server-context"});
        let message = json!({"type":"message","id":"msg-id","role":"assistant","phase":"commentary","content":[{"type":"output_text","text":"Working","annotations":[]}]});
        let output = json!([reason, message, call()]);
        let events = vec![
            json!({"type":"response.output_item.added","output_index":0,"item":{"type":"reasoning","id":"rs-id","summary":[]}}),
            json!({"type":"response.reasoning_summary_text.delta","output_index":0,"summary_index":0,"item_id":"rs-id","delta":"Brief summary"}),
            json!({"type":"response.output_item.done","output_index":0,"item":output[0]}),
            json!({"type":"response.output_item.added","output_index":1,"item":{"type":"message","id":"msg-id","role":"assistant","content":[]}}),
            json!({"type":"response.output_text.delta","output_index":1,"content_index":0,"item_id":"msg-id","delta":"Working"}),
            json!({"type":"response.output_item.done","output_index":1,"item":output[1]}),
            json!({"type":"response.output_item.added","output_index":2,"item":{"type":"function_call","id":"fc-id","call_id":"call-real","name":"echo","arguments":""}}),
            json!({"type":"response.function_call_arguments.delta","output_index":2,"item_id":"fc-id","delta":"{\"input\":"}),
            json!({"type":"response.function_call_arguments.delta","output_index":2,"item_id":"fc-id","delta":"\"世界\"}"}),
            json!({"type":"response.function_call_arguments.done","output_index":2,"item_id":"fc-id","arguments":call()["arguments"]}),
            json!({"type":"response.output_item.done","output_index":2,"item":call()}),
            complete(output.clone()),
        ];
        for event in events {
            if let Some(normalized) = stream.push(&event).unwrap() {
                calls.push(&normalized["choices"][0]["delta"]).unwrap();
                text.push_str(
                    normalized["choices"][0]["delta"]["content"]
                        .as_str()
                        .unwrap_or(""),
                );
                usage.observe(&normalized);
            }
        }
        assert!(stream.completed);
        assert_eq!(text, "Working");
        let calls = calls.finish().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "call-real");
        assert_eq!(calls[0].arguments, json!({"input":"世界"}));
        assert_eq!(usage.input_tokens, Some(20));
        assert_eq!(usage.cached_tokens, Some(7));
        assert_eq!(usage.output_tokens, Some(3));
        assert_eq!(stream.replay().unwrap(), output);
        let replay = json!({"messages":[{"role":"assistant","content":text,"tool_calls":calls.iter().map(|c|c.model_value()).collect::<Vec<_>>(),"locallm_responses_output":stream.replay().unwrap()}]});
        assert_eq!(
            request(&replay, "configured-model").unwrap()["input"],
            output
        );
        let mut changed = replay;
        changed["messages"][0]["tool_calls"][0]["function"]["name"] = json!("wrong");
        assert!(request(&changed, "configured-model").is_err());
    }
    #[test]
    fn terminal_only_output_is_complete_and_repeat_does_not_duplicate_calls() {
        let mut stream = Stream::default();
        let normalized = stream.push(&complete(json!([call()]))).unwrap().unwrap();
        let mut calls = crate::tool_calls::ToolCalls::default();
        calls.push(&normalized["choices"][0]["delta"]).unwrap();
        assert_eq!(calls.finish().unwrap().len(), 1);
        assert!(stream.push(&complete(json!([call()]))).is_err());
    }
    #[test]
    fn malformed_changed_and_oversized_function_streams_fail_without_executable_calls() {
        for event in [
            json!({"type":"response.function_call_arguments.delta","output_index":0,"item_id":"unknown","delta":"{}"}),
            json!({"type":"response.output_item.added","output_index":128,"item":call()}),
            json!({"type":"response.future_server_tool"}),
        ] {
            assert!(Stream::default().push(&event).is_err());
        }
        let mut stream = Stream::default();
        let mut item = call();
        item["arguments"] = json!("");
        stream
            .push(&json!({"type":"response.output_item.added","output_index":0,"item":item}))
            .unwrap();
        stream.push(&json!({"type":"response.function_call_arguments.delta","output_index":0,"item_id":"fc-id","delta":"{\"input\":"})).unwrap();
        assert!(stream.push(&json!({"type":"response.function_call_arguments.done","output_index":0,"item_id":"fc-id","arguments":"{\"different\":true}"})).is_err());
        assert!(!stream.completed);
        assert!(stream.replay().is_none());
        assert!(Stream::default().push(&complete(json!([{"type":"function_call","id":"fc-id","call_id":"call-real","name":"echo","arguments":"{"}]))).is_err());
        for status in ["in_progress", "incomplete", "failed"] {
            let mut unfinished = call();
            unfinished["status"] = json!(status);
            assert!(
                Stream::default()
                    .push(&complete(json!([unfinished])))
                    .is_err(),
                "{status} function must never authorize dispatch"
            );
        }
        assert!(Stream::default()
            .push(&json!({"type":"response.created","padding":"x".repeat(MAX_BYTES)}))
            .is_err());
    }
    #[test]
    fn refusals_and_token_exhaustion_are_distinct_from_successful_tools() {
        let mut stream = Stream::default();
        let refusal = json!({"type":"message","id":"refusal-id","role":"assistant","content":[{"type":"refusal","refusal":"Cannot comply"}]});
        let value = stream.push(&complete(json!([refusal]))).unwrap().unwrap();
        assert_eq!(value["choices"][0]["delta"]["content"], "Cannot comply");
        assert_eq!(value["choices"][0]["finish_reason"], "stop");
        let incomplete = json!({"type":"response.incomplete","response":{"status":"incomplete","incomplete_details":{"reason":"max_output_tokens"},"output":[]}});
        assert_eq!(
            Stream::default().push(&incomplete).unwrap().unwrap()["choices"][0]["finish_reason"],
            "length"
        );
        let mut invalid = incomplete;
        invalid["response"]["incomplete_details"]["reason"] = json!("content_filter");
        assert!(Stream::default().push(&invalid).is_err());
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    #[tokio::test]
    async fn native_responses_selected_format_survives_parent_and_child_assembly_without_network() {
        use crate::inference::InferenceProvider;
        let dir = tempfile::tempdir().unwrap();
        let state = crate::test_support::app_state(dir.path());
        crate::test_support::select_fixture_model(&state, dir.path());
        let conversation = {
            let store = state.database().unwrap();
            let mut provider = store.provider("fixture-loopback").unwrap().unwrap();
            provider.api_format = crate::providers::OPENAI_RESPONSES.into();
            provider.base_url = "https://api.example.test/v1".into();
            store.save_provider(&provider).unwrap();
            store.create_conversation().unwrap().id
        };
        let plan = crate::chat::assemble_turn(
            &state,
            &conversation,
            "Echo",
            vec![],
            Some(vec![]),
            None,
            None,
        )
        .await
        .unwrap();
        assert!(plan.backend.is_responses());
        let input = json!({"messages":[{"role":"user","content":"Echo"}],"stream":true});
        let request = plan.backend.payload(&input).unwrap();
        assert_eq!(request["store"], false);
        assert!(request.get("input").is_some());
        let child = crate::subagents::snapshot_for_conversation(&state, &conversation)
            .await
            .unwrap();
        assert_eq!(child.api_format, crate::providers::OPENAI_RESPONSES);
        assert!(child.key.is_empty());
        let backend = child.backend().unwrap();
        assert!(backend.is_responses());
        assert_eq!(backend.payload(&input).unwrap(), request);
    }
    fn event(value: Value) -> String {
        format!("data: {value}\r\n\r\n")
    }
    #[tokio::test]
    async fn native_responses_parent_round_executes_original_tool_and_replays_opaque_context() {
        let dir = tempfile::tempdir().unwrap();
        let state = crate::test_support::app_state(dir.path());
        crate::test_support::select_fixture_model(&state, dir.path());
        let reasoning = json!({"type":"reasoning","id":"rs-first","summary":[],"encrypted_content":"opaque-first"});
        let call = json!({"type":"function_call","id":"fc-item-id","call_id":"call-owned-id","name":"system_time","arguments":"{}"});
        let final_reason = json!({"type":"reasoning","id":"rs-final","summary":[],"encrypted_content":"opaque-final"});
        let message = json!({"type":"message","id":"msg-final","role":"assistant","phase":"final_answer","content":[{"type":"output_text","text":"Processed fixture string.","annotations":[]}]});
        let (base,requests,server)=crate::test_support::scripted_sse(vec![
            event(json!({"type":"response.completed","response":{"status":"completed","output":[reasoning,call],"usage":{"input_tokens":20,"output_tokens":4}}})),
            event(json!({"type":"response.completed","response":{"status":"completed","output":[final_reason,message],"usage":{"input_tokens":35,"output_tokens":5,"input_tokens_details":{"cached_tokens":10}}}})),
        ]).await;
        let conversation = {
            let store = state.database().unwrap();
            let mut provider = store.provider("fixture-loopback").unwrap().unwrap();
            provider.api_format = crate::providers::OPENAI_RESPONSES.into();
            provider.base_url = base;
            store.save_provider(&provider).unwrap();
            let conversation = store.create_conversation().unwrap();
            store
                .save_conversation_tools(
                    &conversation.id,
                    &crate::store::ConversationTools {
                        access_mode: crate::permissions::AccessMode::FullAccess,
                        sources: vec![],
                        tools: vec![],
                    },
                )
                .unwrap();
            conversation.id
        };
        let channel = tauri::ipc::Channel::<crate::chat::ChatEvent>::new(|_| Ok(()));
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            crate::chat::send_message_inner(
                &state,
                conversation.clone(),
                "Process the fixture string.".into(),
                Some(vec![]),
                Some(vec![]),
                None,
                None,
                channel,
            ),
        )
        .await
        .unwrap()
        .unwrap();
        server.await.unwrap();
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        for request in requests.iter() {
            assert!(request.headers.starts_with("POST /v1/responses "));
            assert!(!request.headers.to_lowercase().contains("authorization:"));
            assert_eq!(request.body["store"], false);
            assert!(request.body.get("messages").is_none());
        }
        let second = requests[1].body["input"].as_array().unwrap();
        assert!(second
            .iter()
            .any(|item| item["encrypted_content"] == "opaque-first"));
        let result = second
            .iter()
            .find(|item| item["type"] == "function_call_output")
            .unwrap();
        assert_eq!(result["call_id"], "call-owned-id");
        let result: Value = serde_json::from_str(result["output"].as_str().unwrap()).unwrap();
        assert!(!crate::tool_envelope::failed(&result), "{result}");
        assert!(result["data"].is_object() || result["data"].is_string());
        let store = state.database().unwrap();
        let rows = store.messages(&conversation).unwrap();
        assert_eq!(rows.iter().filter(|row| row.role == "tool").count(), 1);
        assert!(rows.iter().all(|row| row.status == "complete"));
        let cached = store.cached_model_history(&conversation).unwrap().unwrap();
        assert!(cached
            .iter()
            .any(|message| message["locallm_responses_output"]
                .as_array()
                .is_some_and(|items| items
                    .iter()
                    .any(|item| item["encrypted_content"] == "opaque-final"))));
        let rebuilt = crate::history::model_history_with_contents(
            &rows,
            None,
            &store.model_contents(&conversation).unwrap(),
        )
        .unwrap();
        let native = request(&json!({"messages":rebuilt}), "fixture-model").unwrap();
        assert!(native["input"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["encrypted_content"] == "opaque-first"));
        assert!(native["input"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["encrypted_content"] == "opaque-final"));
    }
}
