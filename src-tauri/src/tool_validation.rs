//! Bounded, cached JSON Schema validation shared by native, MCP and harness
//! calls. Schemas cannot retrieve files or network resources.
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::sync::{Arc, Mutex, OnceLock};
type Cache = std::collections::VecDeque<([u8; 32], Arc<BoundedValidator>)>;

pub struct BoundedValidator {
    schema: Value,
    inner: jsonschema::Validator,
}
impl BoundedValidator {
    fn validate(&self, value: &Value) -> Result<(), String> {
        validation_work(&self.schema, value, &self.schema, 0, &mut 100_000)?;
        self.inner.validate(value).map_err(|e| invalid("value", &e))
    }
}
// Bound the possible schema/instance evaluation tree before invoking the
// validator. All alternatives are charged, even those a validator may skip.
// Productive references descend into finite instance values; non-descending
// cycles and combinatorial recursive branches fail closed before evaluation.
fn validation_work(
    schema: &Value,
    value: &Value,
    root: &Value,
    depth: usize,
    left: &mut usize,
) -> Result<(), String> {
    fn charge(left: &mut usize, cost: usize) -> Result<(), String> {
        *left = left
            .checked_sub(cost)
            .ok_or("Schema validation work budget exceeded.")?;
        Ok(())
    }
    if depth > 128 {
        return Err("Schema validation recursion budget exceeded (possibly a nonproductive reference cycle).".into());
    }
    charge(left, 1)?;
    if let Some(reference) = schema["$ref"].as_str() {
        let target = if reference == "#" {
            root
        } else {
            root.pointer(
                reference
                    .strip_prefix('#')
                    .ok_or("Validation requires offline bundled local references.")?,
            )
            .ok_or("Validation reference pointer is unsupported or missing.")?
        };
        validation_work(target, value, root, depth + 1, left)?;
    }
    for key in ["allOf", "anyOf", "oneOf"] {
        for child in schema[key].as_array().into_iter().flatten() {
            validation_work(child, value, root, depth + 1, left)?;
        }
    }
    for key in ["not", "if", "then", "else"] {
        if let Some(child) = schema.get(key) {
            validation_work(child, value, root, depth + 1, left)?;
        }
    }
    if let Some(items) = value.as_array() {
        if schema["uniqueItems"] == true {
            charge(left, items.len().saturating_mul(items.len()))?;
        }
        for (index, item) in items.iter().enumerate() {
            for key in ["items", "contains", "unevaluatedItems", "additionalItems"] {
                if let Some(child) = schema.get(key) {
                    if let Some(tuple) = child.as_array() {
                        if let Some(child) = tuple.get(index) {
                            validation_work(child, item, root, depth + 1, left)?;
                        }
                    } else {
                        validation_work(child, item, root, depth + 1, left)?;
                    }
                }
            }
            if let Some(child) = schema["prefixItems"].get(index) {
                validation_work(child, item, root, depth + 1, left)?;
            }
        }
    }
    if let Some(fields) = value.as_object() {
        for (name, item) in fields {
            if let Some(child) = schema["properties"].get(name) {
                validation_work(child, item, root, depth + 1, left)?;
            }
            for (pattern, child) in schema["patternProperties"]
                .as_object()
                .into_iter()
                .flatten()
            {
                charge(left, pattern.len().saturating_mul(name.len()).max(1))?;
                validation_work(child, item, root, depth + 1, left)?;
            }
            for key in ["additionalProperties", "unevaluatedProperties"] {
                if let Some(child) = schema.get(key) {
                    validation_work(child, item, root, depth + 1, left)?;
                }
            }
            if let Some(child) = schema.get("propertyNames") {
                validation_work(child, &Value::String(name.clone()), root, depth + 1, left)?;
            }
        }
        for key in ["dependentSchemas", "dependencies"] {
            for child in schema[key]
                .as_object()
                .into_iter()
                .flatten()
                .map(|(_, v)| v)
                .filter(|v| v.is_object() || v.is_boolean())
            {
                validation_work(child, value, root, depth + 1, left)?;
            }
        }
    }
    if let (Some(pattern), Some(text)) = (schema["pattern"].as_str(), value.as_str()) {
        charge(left, pattern.len().saturating_mul(text.len()).max(1))?;
    }
    Ok(())
}
fn bounded(value: &Value, depth: usize, nodes: &mut usize) -> bool {
    *nodes += 1;
    if depth > 48 || *nodes > 8000 {
        return false;
    }
    match value {
        Value::Array(items) => items.iter().all(|v| bounded(v, depth + 1, nodes)),
        Value::Object(fields) => fields.values().all(|v| bounded(v, depth + 1, nodes)),
        _ => true,
    }
}

/// Compile a schema through the bounded, offline, cached pipeline. Used both
/// to prevalidate model-supplied schemas (rejecting invalid or remote
/// `$ref` schemas before any work runs) and to validate instances.
pub fn compile(schema: &Value) -> Result<Arc<BoundedValidator>, String> {
    let bytes = serde_json::to_vec(schema).map_err(|e| e.to_string())?;
    if bytes.len() > 65536 || !bounded(schema, 0, &mut 0) {
        return Err("Tool schema exceeds validation limits.".into());
    }
    let key: [u8; 32] = Sha256::digest(&bytes).into();
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    let mut cache = CACHE
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| "Schema cache unavailable.")?;
    let validator = if let Some(index) = cache.iter().position(|(hash, _)| hash == &key) {
        let item = cache
            .remove(index)
            .ok_or("Schema cache entry disappeared.")?;
        let validator = item.1.clone();
        cache.push_back(item);
        validator
    } else {
        validation_work(schema, &Value::Null, schema, 0, &mut 100_000)?;
        let validator = Arc::new(BoundedValidator {
            schema: schema.clone(),
            inner: jsonschema::options()
                .offline()
                .with_pattern_options(
                    jsonschema::PatternOptions::fancy_regex().backtrack_limit(20000),
                )
                .build(schema)
                .map_err(|e| {
                    format!(
                        "Invalid or unsupported tool schema: {}",
                        e.to_string().chars().take(400).collect::<String>()
                    )
                })?,
        });
        while cache.len() >= 64 {
            cache.pop_front();
        }
        cache.push_back((key, validator.clone()));
        validator
    };
    Ok(validator)
}

fn invalid(what: &str, error: &jsonschema::ValidationError<'_>) -> String {
    format!(
        "Invalid {what} at {}: {}",
        error.instance_path(),
        error.masked()
    )
    .chars()
    .take(600)
    .collect()
}

pub fn validate(schema: &Value, arguments: &Value) -> Result<(), String> {
    if !arguments.is_object() || !bounded(arguments, 0, &mut 0) {
        return Err("Tool arguments must be a bounded JSON object.".into());
    }
    compile(schema)?.validate(arguments)
}

/// Validate any JSON value (scalars included, unlike [`validate`]) against a
/// schema. Used for declared output/result contracts: child output schemas
/// and execution `result_schema`.
pub fn validate_value(schema: &Value, value: &Value) -> Result<(), String> {
    if !bounded(value, 0, &mut 0) {
        return Err("The value exceeds validation size limits.".into());
    }
    compile(schema)?.validate(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn recursive_validation_rejects_cycles_and_exponential_work_before_evaluation() {
        assert!(compile(&json!({"$ref":"#"}))
            .err()
            .unwrap()
            .contains("recursion budget"));
        let schema = json!({"$ref":"#/$defs/node","$defs":{"node":{"allOf":[{"type":"object","properties":{"child":{"$ref":"#/$defs/node"}}},{"type":"object","properties":{"child":{"$ref":"#/$defs/node"}}}]}}});
        let compiled = compile(&schema).unwrap();
        assert!(compiled.validate(&json!({"child":{"child":{}}})).is_ok());
        let mut value = json!({});
        for _ in 0..20 {
            value = json!({"child":value});
        }
        assert!(compiled.validate(&value).unwrap_err().contains("work budget"));
    }
    #[test]
    fn enforces_schema_constraints_refs_and_combinations() {
        let schema = json!({"type":"object", "properties":{"n":{"type":"integer", "minimum":1,"maximum":3}, "names":{"type":"array","maxItems":2,"items":{"$ref":"#/$defs/name"}}},"required":["n"],"additionalProperties":false,"$defs":{"name":{"type":"string","pattern":"^[a-z]+$","maxLength":6}}});
        assert!(validate(&schema, &json!({"n":2,"names":["ok"]})).is_ok());
        for args in [
            json!({}),
            json!({"n":4}),
            json!({"n":1.5}),
            json!({"n":2,"extra":true}),
            json!({"n":2,"names":["invalid name"]}),
            json!({"n":2,"names":["a","b","c"]}),
        ] {
            assert!(validate(&schema, &args).is_err(), "{args}");
        }
        let schema = json!({"oneOf":[{"required":["code"]},{"required":["file"]}]});
        assert!(validate(&schema, &json!({"code":"1+1"})).is_ok());
        assert!(validate(&schema, &json!({"code":"1+1","file":"x.py"})).is_err());
        assert!(validate(&json!({"$ref":"http://127.0.0.1/never-fetch"}), &json!({})).is_err());
        assert!(validate(&json!({"$ref":"file:///etc/passwd"}), &json!({})).is_err());
    }
    #[test]
    fn compile_prevalidates_and_value_validation_covers_scalars() {
        assert!(compile(&json!({"type":"object","properties":{"a":{"type":"string"}}})).is_ok());
        for bad in [
            json!({"type":"nonsense"}),
            json!({"$ref":"https://schemas.example/remote.json"}),
            json!({"properties":{"a":{"minimum":"not-a-number"}}}),
        ] {
            assert!(compile(&bad).is_err(), "{bad}");
        }
        let schema = json!({"type":"object","required":["answer"],"properties":{"answer":{"type":"string"},"kind":{"enum":["a","b"]}},"additionalProperties":false});
        assert!(validate_value(&schema, &json!({"answer":"ok","kind":"a"})).is_ok());
        assert!(validate_value(&schema, &json!({"answer":123})).is_err());
        assert!(validate_value(&schema, &json!({"answer":"ok","kind":"z"})).is_err());
        assert!(validate_value(&schema, &json!({"answer":"ok","extra":1})).is_err());
        assert!(validate_value(&schema, &json!("not an object")).is_err());
        assert!(validate_value(&json!({"type":"integer","minimum":2}), &json!(3)).is_ok());
        assert!(validate_value(&json!({"type":"integer","minimum":2}), &json!(1)).is_err());
    }
}
