//! Local inference and local execution. Remote providers and Daytona stay
//! unreachable. An OpenAI-compatible wire format on loopback is still local.

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
            SetupFact { name: "runtime", ok: false, detail: "Managed llama.cpp is not installed.".into() },
            SetupFact { name: "tool_probe", ok: false, detail: "Tool-call probe has not run.".into() },
            SetupFact { name: "node", ok: false, detail: managed_node_detail(false) },
            SetupFact { name: "python", ok: true, detail: "Python is available.".into() },
            SetupFact { name: "searxng", ok: false, detail: format!("SearXNG is an explicit install of {SEARXNG_IMAGE}.") },
            SetupFact { name: "public_page", ok: false, detail: "Public-page retrieval failed.".into() },
            SetupFact { name: "pdf", ok: false, detail: "PDF parser failed on the fixture.".into() },
            SetupFact { name: "browser", ok: false, detail: "Optional browser rendering did not start.".into() },
        ];
        let report = fresh_install_report(&facts);
        assert_eq!(report.len(), 7);
        assert!(report.iter().any(|line| line.starts_with("node:")));
        assert!(report.iter().all(|line| !line.contains("hand-edit") || line.contains("Do not hand-edit")));
        assert!(!SEARXNG_IMAGE.ends_with(":latest"));
        assert!(report.iter().any(|line| line.contains(SEARXNG_IMAGE)));
    }
}
