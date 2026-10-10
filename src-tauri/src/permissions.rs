use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AccessMode {
    #[default]
    Ask,
    AutoApprove,
    FullAccess,
}
impl AccessMode {
    pub fn automatic_reason(self, trusted_read: bool) -> Option<&'static str> {
        match self {
            Self::Ask => None,
            Self::AutoApprove if trusted_read => Some("auto-approved workspace read"),
            Self::AutoApprove => None,
            Self::FullAccess => Some("full access selected by user"),
        }
    }
}

/// Ask / Plan / Act: the per-conversation interaction mode. Orthogonal to
/// `AccessMode`, which gates auto-approval once a tool is allowed to run —
/// the interaction mode narrows which tools may run at all: Ask answers from
/// read-only context tools, Plan may additionally write todos, and Act is
/// unrestricted. Enforced in the offered-tool filter and again at dispatch.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum InteractionMode {
    Ask,
    Plan,
    #[default]
    Act,
}

fn is_todo_alias(alias: &str) -> bool {
    matches!(alias, "todo_write" | "todo_add" | "todo_update")
}

/// The alias-level twin of `AgentTool::trusted_read`, for paths that only
/// know the emitted call name (dispatch gates, capability listings). A name
/// that lacks the read-only classification is never treated as read-only.
pub fn trusted_read_alias(alias: &str) -> bool {
    crate::harness::is_trusted_read(alias)
        || matches!(alias, "system_time" | "workspace_read_file" | "workspace_list_files")
}

impl InteractionMode {
    /// Command-arg parser: absent or empty is `None` (caller decides the
    /// fallback); unknown strings are rejected instead of silently widening
    /// permissions.
    pub fn parse(value: Option<&str>) -> Result<Option<Self>, String> {
        match value.map(str::trim).filter(|value| !value.is_empty()) {
            None => Ok(None),
            Some("ask") => Ok(Some(Self::Ask)),
            Some("plan") => Ok(Some(Self::Plan)),
            Some("act") => Ok(Some(Self::Act)),
            Some(other) => Err(format!("Unknown interaction mode '{other}'. Choose ask, plan, or act.")),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ask => "ask",
            Self::Plan => "plan",
            Self::Act => "act",
        }
    }

    /// Whether a tool may run under this mode. `trusted_read` is the same
    /// read classification `AccessMode::AutoApprove` uses for auto-approved
    /// reads (`AgentTool::trusted_read`).
    pub fn permits(self, alias: &str, trusted_read: bool) -> bool {
        match self {
            Self::Act => true,
            Self::Plan => trusted_read || is_todo_alias(alias) || alias == "propose_criteria",
            Self::Ask => trusted_read,
        }
    }

    pub fn permits_tool(self, tool: &crate::connectors::AgentTool) -> bool {
        self.permits(&tool.alias, tool.trusted_read())
    }

    /// Dispatch-side check for a model-emitted call name: resolves the read
    /// classification by alias so a write never slips through because it was
    /// never offered a tool record.
    pub fn permits_call(self, alias: &str) -> bool {
        self.permits(alias, trusted_read_alias(alias))
    }

    /// Why an alias is blocked in this mode, for capability listings and
    /// denial results. `None` when the call may run.
    pub fn blocked_reason(self, alias: &str) -> Option<String> {
        match self {
            Self::Act => None,
            Self::Ask => (!self.permits_call(alias)).then(|| {
                "Not available in Ask mode: only read-only context tools can run. Switch to Act to use it.".to_string()
            }),
            Self::Plan => (!self.permits_call(alias)).then(|| {
                "Not available in Plan mode: only read-only context tools and todo updates can run. Switch to Act to use it.".to_string()
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn modes_only_grant_their_declared_scope() {
        assert_eq!(AccessMode::default(), AccessMode::Ask);
        for trusted_read in [true, false] {
            assert!(AccessMode::Ask.automatic_reason(trusted_read).is_none());
            assert!(AccessMode::FullAccess
                .automatic_reason(trusted_read)
                .is_some());
        }
        assert!(AccessMode::AutoApprove.automatic_reason(true).is_some());
        assert!(AccessMode::AutoApprove.automatic_reason(false).is_none());
        assert!(serde_json::from_str::<AccessMode>("\"unknown\"").is_err());
    }
    #[test]
    fn old_conversations_default_to_asking_and_invalid_modes_are_rejected() {
        let settings: crate::store::ConversationTools =
            serde_json::from_str(r#"{"sources":[],"tools":[]}"#).unwrap();
        assert_eq!(settings.access_mode, AccessMode::Ask);
        assert!(serde_json::from_str::<crate::store::ConversationTools>(
            r#"{"sources":[],"tools":[],"accessMode":"skipSecurity"}"#
        )
        .is_err());
    }

    #[test]
    fn interaction_mode_parses_only_the_three_modes() {
        assert_eq!(InteractionMode::parse(None).unwrap(), None);
        assert_eq!(InteractionMode::parse(Some("")).unwrap(), None);
        assert_eq!(InteractionMode::parse(Some("ask")).unwrap(), Some(InteractionMode::Ask));
        assert_eq!(InteractionMode::parse(Some("plan")).unwrap(), Some(InteractionMode::Plan));
        assert_eq!(InteractionMode::parse(Some("act")).unwrap(), Some(InteractionMode::Act));
        assert!(InteractionMode::parse(Some("god")).is_err());
        assert_eq!(InteractionMode::default(), InteractionMode::Act);
    }

    #[test]
    fn ask_and_plan_filter_out_writes_execution_browser_and_orchestration() {
        for mode in [InteractionMode::Ask, InteractionMode::Plan] {
            for alias in [
                "file_write", "run_code", "local_run_code", "local_execute_command",
                "terminal_send", "terminal_create", "browser", "subagent",
                "workflow_run", "ralph_run", "ptc_run", "web_fetch",
            ] {
                assert!(!mode.permits_call(alias), "{mode:?} must not permit {alias}");
            }
            for alias in [
                "file_read", "file_search", "workspace_read_file", "workspace_list_files",
                "memory_recall", "system_time", "finish", "update_context", "preset_guide",
            ] {
                assert!(mode.permits_call(alias), "{mode:?} must permit read-only {alias}");
            }
            // Connector and skill calls are unclassified, so they are not read-only.
            for alias in ["some_connector_tool", "skills_read_file"] {
                assert!(!mode.permits_call(alias), "{mode:?} must not permit unclassified {alias}");
            }
        }
        assert!(!InteractionMode::Ask.permits_call("todo_write"));
        assert!(InteractionMode::Plan.permits_call("todo_write"));
        // Evidence tools: criteria proposals may happen while planning;
        // running a check executes a command, so it stays Act-only.
        for mode in [InteractionMode::Ask, InteractionMode::Plan] {
            assert!(!mode.permits_call("verify_run"), "{mode:?} must not run checks");
        }
        assert!(!InteractionMode::Ask.permits_call("propose_criteria"));
        assert!(InteractionMode::Plan.permits_call("propose_criteria"));
        assert!(InteractionMode::Act.permits_call("verify_run"));
        assert!(InteractionMode::Act.permits_call("file_write"));
        assert!(InteractionMode::Plan.blocked_reason("file_write").is_some());
        assert!(InteractionMode::Plan.blocked_reason("file_read").is_none());
        assert!(InteractionMode::Act.blocked_reason("file_write").is_none());
    }
}
