//! Tool-calling compatibility is a fingerprint of the runtime build, the model,
//! the chat template, and the parser. A path-only historical flag is unknown.

pub const PARSER_VERSION: &str = "tool-parser-2";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompatState {
    Unknown,
    Supported,
    Unsupported,
}

impl CompatState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Supported => "supported",
            Self::Unsupported => "unsupported",
        }
    }

    pub fn parse(value: &str) -> Self {
        match value {
            "supported" => Self::Supported,
            "unsupported" => Self::Unsupported,
            _ => Self::Unknown,
        }
    }
}

/// Historical `local_tool_calling_support` booleans came from malformed text
/// calls. Both true and false are unknown until a fingerprint probe runs.
pub fn interpret_historical_flag(_stored: Option<bool>) -> CompatState {
    CompatState::Unknown
}

pub fn fingerprint(runtime_identity: &str, model_identity: &str, template: &str, parser: &str) -> String {
    format!("{runtime_identity}\u{1f}{model_identity}\u{1f}{template}\u{1f}{parser}")
}

/// Identity is the build label plus size and mtime. The directory path is not
/// sufficient: replacing the binary at the same path changes this string.
pub fn runtime_identity(build_label: &str, size: u64, modified_secs: u64) -> String {
    format!("{build_label}|{size}|{modified_secs}")
}

pub fn build_label_from_path(path: &str) -> String {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    if let Some(rest) = name.strip_prefix("llama-") {
        let label = rest.split('-').take(2).collect::<Vec<_>>().join("-");
        if !label.is_empty() {
            return label;
        }
    }
    if let Some(index) = name.find("prism-") {
        return name[index..].split(['/', '\\']).next().unwrap_or(name).to_string();
    }
    "unknown-build".into()
}

/// Tools stay enabled unless this exact fingerprint was probed unsupported.
pub fn tools_enabled(state: CompatState) -> bool {
    !matches!(state, CompatState::Unsupported)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn historical_flags_are_unknown_and_do_not_disable_tools() {
        assert_eq!(interpret_historical_flag(Some(false)), CompatState::Unknown);
        assert_eq!(interpret_historical_flag(Some(true)), CompatState::Unknown);
        assert_eq!(interpret_historical_flag(None), CompatState::Unknown);
        assert!(tools_enabled(CompatState::Unknown));
        assert!(!tools_enabled(CompatState::Unsupported));
    }

    #[test]
    fn replacing_a_runtime_at_the_same_path_invalidates_compatibility() {
        let path = r"C:\runtime\llama-b10855-cuda12.4-aaaa";
        let model = "MiniCPM5-2B.Q6_K.gguf";
        let template = "minicpm";
        let before = fingerprint(
            &runtime_identity(&build_label_from_path(path), 100, 1),
            model,
            template,
            PARSER_VERSION,
        );
        let after = fingerprint(
            &runtime_identity(&build_label_from_path(path), 200, 2),
            model,
            template,
            PARSER_VERSION,
        );
        assert_ne!(before, after);
        assert_eq!(build_label_from_path(path), "b10855-cuda12.4");
        // A stale unsupported result for `before` does not apply to `after`.
        assert!(tools_enabled(interpret_historical_flag(Some(false))));
        assert_eq!(CompatState::parse("nope"), CompatState::Unknown);
    }
}
