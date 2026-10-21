//! Policy for operations that require local inference or execution, including
//! account subscriptions and Daytona. Explicitly selected custom API providers
//! use their validated endpoint instead. OpenAI-compatible loopback is local.

pub const SEARXNG_IMAGE: &str = "searxng/searxng:2026.9.22-019460e07";

pub fn inference_allowed(base_url: &str) -> Result<(), String> {
    if crate::providers::is_loopback_base_url(base_url) {
        Ok(())
    } else {
        Err("This conversation needs a local model. Its history stays readable; remote inference was not called.".into())
    }
}

pub fn cloud_execution_allowed() -> Result<(), String> {
    Err("Cloud execution is unavailable. Historical cloud records stay readable; this call was not sent.".into())
}

/// Offline mode blocks external retrieval. It does not claim that shell or
/// unknown-network MCP is network-isolated unless a sandbox actually exists.
pub fn retrieval_allowed(offline: bool) -> bool {
    !offline
}

pub fn check_retrieval_action(offline: bool, action: &str) -> Result<(), String> {
    if offline && !matches!(action, "open" | "find" | "clear-cache") {
        return Err("Offline mode: external retrieval is disabled.".into());
    }
    Ok(())
}
pub fn external_web_tool(alias: &str) -> bool {
    matches!(
        alias,
        "web_search" | "web_fetch" | "web_fetch_url" | "search" | "visit"
    )
}
#[tauri::command]
pub fn get_offline_mode(state: tauri::State<'_, crate::AppState>) -> Result<bool, String> {
    state.database()?.setting("offline_mode")
}
#[tauri::command]
pub async fn set_offline_mode(
    state: tauri::State<'_, crate::AppState>,
    enabled: bool,
) -> Result<(), String> {
    let _guard = state
        .operation
        .try_lock()
        .map_err(|_| "Wait for the active operation before changing retrieval permissions.")?;
    state.database()?.save_setting("offline_mode", &enabled)
}

pub fn execution_isolation_label(sandbox_enforced: bool) -> &'static str {
    if sandbox_enforced {
        "sandboxed"
    } else {
        "unsandboxed"
    }
}

pub fn offline_capability_claim(offline: bool, sandbox_enforced: bool) -> &'static str {
    if offline && sandbox_enforced {
        "Offline. External retrieval is disabled. Local execution is sandboxed."
    } else if offline {
        "Offline. External retrieval is disabled. Local shell and MCP are not network-isolated."
    } else if sandbox_enforced {
        "Local execution is sandboxed."
    } else {
        "Local execution is unsandboxed."
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetupFact {
    pub name: &'static str,
    pub ok: bool,
    pub detail: String,
}

/// Actionable lines for a fresh install. Missing pieces name the managed fix
/// instead of asking the user to edit a config file by hand.
pub fn fresh_install_report(facts: &[SetupFact]) -> Vec<String> {
    facts
        .iter()
        .filter(|fact| !fact.ok)
        .map(|fact| format!("{}: {}", fact.name, fact.detail))
        .collect()
}

pub fn managed_node_detail(node_ok: bool) -> String {
    if node_ok {
        "Node is available.".into()
    } else {
        "Node is missing. Install it with the managed runtime path (npm run tauri or the packaged Node next to the app). Do not hand-edit a config path.".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn offline_blocks_all_retrieval_but_preserves_saved_passages() {
        for action in [
            "search",
            "fetch-url",
            "fetch-urls",
            "weather",
            "health",
            "",
            "unknown",
        ] {
            assert!(check_retrieval_action(true, action).is_err(), "{action}");
            assert!(check_retrieval_action(false, action).is_ok(), "{action}");
        }
        for action in ["open", "find", "clear-cache"] {
            assert!(check_retrieval_action(true, action).is_ok());
        }
        for tool in [
            "web_search",
            "web_fetch",
            "web_fetch_url",
            "search",
            "visit",
        ] {
            assert!(external_web_tool(tool));
        }
        for tool in ["web_open", "web_find", "web_read", "get_weather"] {
            assert!(!external_web_tool(tool));
        }
    }

    #[test]
    fn loopback_openai_wire_is_local_and_remote_hosts_are_rejected() {
        assert!(inference_allowed("http://127.0.0.1:8080").is_ok());
        assert!(inference_allowed("http://localhost:1919/v1").is_ok());
        let remote = inference_allowed("https://api.openai.com/v1").unwrap_err();
        assert!(remote.contains("local model"));
        assert!(remote.contains("was not called"));
        assert!(cloud_execution_allowed().is_err());
    }

    #[test]
    fn offline_disables_retrieval_without_claiming_a_sandbox() {
        assert!(!retrieval_allowed(true));
        assert!(retrieval_allowed(false));
        assert_eq!(execution_isolation_label(false), "unsandboxed");
        let claim = offline_capability_claim(true, false);
        assert!(claim.contains("not network-isolated"));
        assert!(!claim.contains("sandboxed local shell"));
    }

    #[test]
    fn fresh_install_names_each_missing_piece() {
        let facts = [
            SetupFact {
                name: "runtime",
                ok: false,
                detail: "Managed llama.cpp is not installed.".into(),
            },
            SetupFact {
                name: "tool_probe",
                ok: false,
                detail: "Tool-call probe has not run.".into(),
            },
            SetupFact {
                name: "node",
                ok: false,
                detail: managed_node_detail(false),
            },
            SetupFact {
                name: "python",
                ok: true,
                detail: "Python is available.".into(),
            },
            SetupFact {
                name: "searxng",
                ok: false,
                detail: format!("SearXNG is an explicit install of {SEARXNG_IMAGE}."),
            },
            SetupFact {
                name: "public_page",
                ok: false,
                detail: "Public-page retrieval failed.".into(),
            },
            SetupFact {
                name: "pdf",
                ok: false,
                detail: "PDF parser failed on the fixture.".into(),
            },
            SetupFact {
                name: "browser",
                ok: false,
                detail: "Optional browser rendering did not start.".into(),
            },
        ];
        let report = fresh_install_report(&facts);
        assert_eq!(report.len(), 7);
        assert!(report.iter().any(|line| line.starts_with("node:")));
        assert!(report
            .iter()
            .all(|line| !line.contains("hand-edit") || line.contains("Do not hand-edit")));
        assert!(!SEARXNG_IMAGE.ends_with(":latest"));
        assert!(report.iter().any(|line| line.contains(SEARXNG_IMAGE)));
    }
}
