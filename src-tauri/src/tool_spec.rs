//! The tool contract.
//!
//! A tool is a name, a description, an input schema, and — when the tool
//! produces a structured result — an output schema. [`ToolSpec`] is that
//! contract in one type, so a registry entry can carry everything the model
//! needs to know about a tool and everything the runtime needs to enforce.
//!
//! Before this type existed the registry was a bare
//! `Vec<(String, String, Value)>`, which structurally could not express an
//! output contract: the shape of a tool's result was documented in prose and
//! checked by hand-written regression tests instead of being declared and
//! enforced at the boundary.

use serde_json::Value;

/// One model-facing tool.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolSpec {
    /// The name the model calls. Must be unique inside a catalog.
    pub name: String,
    /// What the tool does, written for the model.
    pub description: String,
    /// JSON Schema for the arguments.
    pub input: Value,
    /// JSON Schema for the result, when the tool returns a structured value.
    pub output: Option<Value>,
}

impl ToolSpec {
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        input: Value,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            input,
            output: None,
        }
    }

    /// Attach an output contract. The result is validated against this schema
    /// when the tool returns, so a tool that drifts from its documented shape
    /// fails at the boundary instead of confusing the model later.
    pub fn with_output(mut self, output: Value) -> Self {
        self.output = Some(output);
        self
    }

    /// Whether this tool declares a result contract.
    pub fn has_output_contract(&self) -> bool {
        self.output.is_some()
    }

    /// Validate a tool result against the declared output contract.
    ///
    /// A tool without a declared contract always passes: the contract is
    /// opt-in so a tool can be introduced before its result shape is stable.
    pub fn validate_output(&self, value: &Value) -> Result<(), String> {
        let Some(output) = self.output.as_ref() else {
            return Ok(());
        };
        crate::tool_validation::validate_value(output, value).map_err(|error| {
            format!(
                "Tool '{}' returned a result that does not match its declared output contract: {error}",
                self.name
            )
        })
    }
}

impl From<(String, String, Value)> for ToolSpec {
    fn from((name, description, input): (String, String, Value)) -> Self {
        Self::new(name, description, input)
    }
}

/// Build a catalog from the legacy tuple shape.
///
/// Registries that have no structured result keep the compact tuple form;
/// this keeps one conversion point instead of a per-entry rewrite.
pub fn catalog(
    entries: impl IntoIterator<Item = (String, String, Value)>,
) -> Vec<ToolSpec> {
    entries.into_iter().map(ToolSpec::from).collect()
}

/// Find a tool in a catalog.
pub fn find<'a>(catalog: &'a [ToolSpec], name: &str) -> Option<&'a ToolSpec> {
    catalog.iter().find(|spec| spec.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tuple_entries_become_specs_without_a_result_contract() {
        let specs = catalog([(
            "demo".to_string(),
            "A demo tool.".to_string(),
            json!({"type":"object"}),
        )]);
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].name, "demo");
        assert!(!specs[0].has_output_contract());
        assert!(specs[0].validate_output(&json!({"anything":true})).is_ok());
    }

    #[test]
    fn a_declared_result_contract_is_enforced() {
        let spec = ToolSpec::new(
            "demo",
            "A demo tool.",
            json!({"type":"object"}),
        )
        .with_output(json!({
            "type":"object",
            "required":["ok"],
            "properties":{"ok":{"type":"boolean"}},
            "additionalProperties":false,
        }));
        assert!(spec.has_output_contract());
        assert!(spec.validate_output(&json!({"ok":true})).is_ok());
        let error = spec.validate_output(&json!({"ok":"yes"})).unwrap_err();
        assert!(error.contains("demo"), "{error}");
        assert!(error.contains("output contract"), "{error}");
    }

    #[test]
    fn catalogs_find_tools_by_name() {
        let specs = catalog([
            ("a".to_string(), "A.".to_string(), json!({})),
            ("b".to_string(), "B.".to_string(), json!({})),
        ]);
        assert_eq!(find(&specs, "b").map(|spec| spec.name.as_str()), Some("b"));
        assert!(find(&specs, "missing").is_none());
    }
}
