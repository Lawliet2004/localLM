//! Parent-turn verification bookkeeping. This is a completion reminder, not a
//! substitute for source-bound evidence or a sandbox for opaque commands.
use serde_json::Value;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Progress {
    revision: u64,
    verified_revision: Option<u64>,
    checks: HashMap<String, Check>,
    failures: HashSet<String>,
    last_outcome: Option<String>,
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct Check {
    revision: u64,
    key: String,
    pending: bool,
    source_bound: bool,
    verdict: Option<String>,
    #[serde(default)]
    confirmed: bool,
    #[serde(default)]
    evidence_id: Option<String>,
    #[serde(default)]
    report_id: Option<String>,
}

fn payload(result: &Value) -> &Value {
    if crate::tool_envelope::is_envelope(result) { &result["data"] } else { result }
}

/// Derive program effects from the same typed dispatch metadata as policy.
/// Shell hooks can mutate files even when the underlying tool is a read.
pub(crate) fn step_may_mutate(
    ctx: &crate::harness::HarnessCtx<'_>, tool: &crate::connectors::AgentTool,
) -> bool {
    !tool.trusted_read()
        || crate::hooks::has_pre_use(ctx.state, &ctx.snapshot.workspace_path, &tool.alias)
        || crate::hooks::has_post_use(ctx.state, &ctx.snapshot.workspace_path, &tool.alias)
}

impl Progress {
    pub fn needs_verification(&self) -> bool {
        (self.revision > 0 && self.verified_revision != Some(self.revision))
            || !self.failures.is_empty()
            || self.checks.values().any(|c| c.pending && c.revision == self.revision)
    }

    pub fn outcome(&self) -> Value {
        let status = if self.needs_verification() {
            "unverified"
        } else if self.revision == 0 && self.verified_revision.is_none() {
            "not_required"
        } else {
            "verified"
        };
        let mut checks = self.checks.iter().map(|(id, check)| serde_json::json!({
            "processId":id, "revision":check.revision, "pending":check.pending,
            "sourceBound":check.source_bound, "verdict":check.verdict,
        })).collect::<Vec<_>>();
        checks.sort_by(|left, right| left["processId"].as_str().cmp(&right["processId"].as_str()));
        serde_json::json!({"status":status,"revision":self.revision,"checks":checks})
    }

    /// A command can meet a named gate without being a test suite. Bind it to
    /// the exact command and source revision that produced the report.
    pub(crate) fn evidence_for(&self, command: &str) -> Option<&str> {
        self.checks.values()
            .filter(|check| check.revision == self.revision && check.confirmed && check.source_bound && check.matches_root_command(command))
            .filter_map(|check| check.report_id.as_deref()).min()
    }

    pub(crate) fn settled_gate(&self, command: &str) -> bool {
        self.checks.values().any(|check| check.source_bound && check.matches_root_command(command) && !check.pending)
            && !self.checks.values().any(|check| check.source_bound && check.matches_root_command(command) && check.pending)
    }

    pub(crate) fn running_gates(&self) -> Vec<Value> {
        let mut checks: Vec<_> = self.checks.iter().filter(|(_, check)| check.source_bound && check.pending)
            .map(|(id, check)| serde_json::json!({"processId":id,"evidenceId":check.evidence_id,
                "command":check.key.split_once('\n').map(|(_, command)| command)})).collect();
        checks.sort_by(|left, right| left["processId"].as_str().cmp(&right["processId"].as_str()));
        checks
    }

    pub(crate) fn pending_processes(&self) -> Vec<&str> {
        self.checks.iter()
            .filter(|(_, check)| check.pending)
            .map(|(id, _)| id.as_str())
            .collect()
    }

    /// Model-observed edits advance the revision. Recheck the recorded source
    /// hashes for external changes between reporting and handing off or resuming.
    pub(crate) fn refresh_goal_evidence(&mut self, store: &crate::store::Store, owner: &str, root: &str) {
        let config = store.execution_config().ok();
        for check in self.checks.values_mut().filter(|check| check.revision == self.revision && check.confirmed && check.source_bound) {
            let current = check.evidence_id.as_deref().and_then(|id| store.artifact(id).ok().flatten())
                .filter(|artifact| artifact.conversation_id == owner && artifact.tool_name == "verification_evidence"
                    && crate::development_tools::hash(artifact.content.as_bytes()) == artifact.sha256)
                .and_then(|artifact| serde_json::from_str::<Value>(&artifact.content).ok())
                .is_some_and(|evidence| {
                    let root_matches = crate::development_tools::canonical(root).is_ok_and(|root| evidence["root"] == root);
                    let config_matches = config.as_ref().is_some_and(|config| {
                        let effective = crate::check_tools::effective_config(config.clone(), evidence["cwd"].as_str());
                        serde_json::to_vec(&effective).is_ok_and(|bytes| evidence["configurationSha256"] == crate::development_tools::hash(&bytes))
                    });
                    root_matches && config_matches && crate::verification_tools::source_freshness(root, &evidence)
                        .is_ok_and(|freshness| freshness["matches"] == true)
                });
            if !current {
                check.confirmed = false;
                self.verified_revision = None;
                self.failures.insert(check.key.clone());
                self.last_outcome = Some("recorded source or execution settings changed".into());
            }
        }
    }

    pub fn reminder(&self) -> String {
        format!("{} Latest observed check outcome: {}. Starting a process is not verification. Collect a terminal check_report or verification_report, fix failures, and rerun relevant checks after the final edit. A successful build only verifies that command; do not claim unperformed tests, visual inspection or gameplay. If checks cannot run or are not applicable, explain that limitation and any remaining failures explicitly. Do not bypass disabled tools or declined preview.", crate::continuation::VERIFY_BEFORE_FINISH, self.last_outcome.as_deref().unwrap_or("no completed check for the current edits"))
    }

    pub fn invalidate(&mut self) {
        self.revision = self.revision.saturating_add(1);
    }

    pub fn observe(&mut self, name: &str, args: &Value, result: &Value, _development_task: bool) {
        let data = payload(result);
        let side_effects = result.pointer("/error/sideEffects").and_then(Value::as_str)
            .or_else(|| result.pointer("/metadata/sideEffects").and_then(Value::as_str));
        let no_effect = matches!(side_effects, Some("none" | "rolled_back"));
        let failed = crate::tool_envelope::failed(result) || result["isError"] == true;
        let write = name.strip_prefix("workspace_").is_some_and(crate::workspace::is_write_call)
            || matches!(name, "file_write" | "format_paths" | "lsp_rename" | "lsp_apply_edit" | "lsp_execute_command" | "worktree_enter" | "worktree_exit");
        // Executable code can mutate the workspace regardless of the wording
        // of the user's request. Include the actual local connector aliases.
        let opaque = matches!(name, "shell_run" | "process_start" | "process_write" | "terminal_send" | "run_code" | "local_run_code" | "local_execute_command" | "subagent" | "workflow_run")
            || (name == "ptc_run" && data["workspaceEffects"] != "none");
        // Interrupted/partially applied edits invalidate evidence too. Explicit
        // policy denials and rolled-back transactions do not.
        if (write || opaque) && !(failed && no_effect) {
            self.invalidate();
        }
        if !matches!(name, "check_run" | "check_report" | "verification_run" | "verification_report") { return; }
        let source_bound = name.starts_with("verification_");
        let check = if source_bound { &data["check"] } else { data };
        let Some(id) = data["processId"].as_str().or_else(|| check["processId"].as_str()) else { return; };
        if matches!(name, "check_run" | "verification_run") {
            // Record before inspecting the report, including failed immediate
            // exits. A later report can settle only a launch owned by this turn.
            let cwd = data["workingDirectory"].as_str().or_else(|| args["cwd"].as_str()).unwrap_or(".");
            let cwd = if cwd.is_empty() { "." } else { cwd };
            self.checks.insert(id.into(), Check { revision: self.revision, key: format!("{}\n{}", cwd, args["command"].as_str().unwrap_or("")), pending: true, source_bound, verdict: None, confirmed: false, evidence_id: data["evidenceId"].as_str().map(str::to_owned), report_id: None });
        }
        let Some(launch) = self.checks.get_mut(id) else { return; };
        if launch.source_bound != source_bound { return; }
        // verification_run deliberately does not certify the source snapshot;
        // verification_report supplies the actual freshness/observer result.
        if name == "verification_run" || data["completionPending"] == true { return; }
        let Some(verdict) = check["report"]["verdict"].as_str() else { return; };
        self.last_outcome = Some(verdict.into());
        launch.pending = verdict == "running";
        launch.verdict = Some(verdict.into());
        launch.report_id = data["reportId"].as_str().map(str::to_owned);
        let successful = !failed && matches!(verdict, "passed" | "command_succeeded")
            && check["report"]["commandSucceeded"] == true
            && check["report"]["incomplete"] == false
            && check["process"]["state"] == "exited" && check["process"]["exitCode"] == 0;
        let fresh = !source_bound || (data["freshness"]["matches"] == true
            && data["runtimeMatches"] == true && data["configurationMatches"] == true
            && data["coverageComplete"] == true && data["changedDuringObservation"] == false
            && data["observation"]["complete"] == true);
        launch.confirmed = successful && fresh;
        if launch.confirmed && launch.revision == self.revision {
            self.failures.remove(&launch.key);
            if verdict == "passed" {
                self.verified_revision = Some(self.revision);
            }
        } else if verdict != "running" && launch.revision == self.revision {
            self.failures.insert(launch.key.clone());
        }
    }
}

impl Check {
    fn matches_root_command(&self, command: &str) -> bool {
        self.key.split_once('\n').is_some_and(|(cwd, recorded)| cwd == "." && recorded == command)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn report(id: &str, verdict: &str) -> Value {
        let pass = matches!(verdict, "passed" | "command_succeeded");
        json!({"processId":id,"process":{"state":if verdict == "running" {"running"} else {"exited"},"exitCode":if pass {0} else {1}},"report":{"verdict":verdict,"commandSucceeded":pass,"incomplete":false}})
    }
    fn write(p: &mut Progress) { p.observe("workspace_apply_patch", &json!({}), &json!({"ok":true}), false); }
    fn launch(p: &mut Progress, id: &str, verdict: &str, command: &str) { p.observe("check_run", &json!({"command":command}), &report(id, verdict), false); }
    #[test]
    fn read_only_programs_preserve_verification_but_uncertain_programs_invalidate_it() {
        let mut reads = Progress::default();
        reads.observe("ptc_run", &json!({}), &json!({"workspaceEffects":"none"}), false);
        assert!(!reads.needs_verification());
        for result in [json!({}), json!({"workspaceEffects":"possible"})] {
            let mut uncertain = Progress::default();
            uncertain.observe("ptc_run", &json!({}), &result, false);
            assert!(uncertain.needs_verification());
        }
        write(&mut reads);
        reads.observe("ptc_run", &json!({}), &json!({"workspaceEffects":"none"}), false);
        assert!(reads.needs_verification(), "a read batch must not certify earlier changes");
    }

    #[test]
    fn running_failed_empty_and_incomplete_checks_never_clear_edits() {
        for verdict in ["running","failed","command_failed","no_tests","incomplete","timed_out","stopped","unknown"] {
            let mut p = Progress::default(); write(&mut p); launch(&mut p,"a",verdict,"test");
            assert!(p.needs_verification(), "{verdict}");
        }
    }
    #[test]
    fn an_ordinary_successful_shell_command_cannot_verify_a_mutation() {
        let mut progress = Progress::default();
        write(&mut progress);
        launch(&mut progress, "echo", "command_succeeded", "echo ok");
        assert_eq!(progress.outcome()["status"], "unverified");
        assert!(progress.evidence_for("echo ok").is_none());
    }
    #[test]
    fn reports_settle_only_current_owned_launches_and_edits_invalidate_a_pass() {
        let mut p = Progress::default(); write(&mut p);
        p.observe("check_report", &json!({"id":"historic"}), &report("historic","passed"), false);
        assert!(p.needs_verification());
        launch(&mut p,"a","running","test"); write(&mut p);
        p.observe("check_report", &json!({"id":"a"}), &report("a","passed"), false);
        assert!(p.needs_verification());
        launch(&mut p,"b","passed","test"); assert!(!p.needs_verification());
        write(&mut p); assert!(p.needs_verification());
    }
    #[test]
    fn failed_suite_is_not_hidden_by_a_different_successful_command() {
        let mut p = Progress::default(); write(&mut p);
        launch(&mut p,"tests","failed","npm test");
        launch(&mut p,"build","command_succeeded","npm run build");
        assert!(p.needs_verification());
        launch(&mut p,"tests2","passed","npm test"); assert!(!p.needs_verification());
        launch(&mut p,"lint","running","npm run lint"); assert!(p.needs_verification());
    }
    #[test]
    fn source_bound_launch_requires_fresh_terminal_report() {
        let mut p = Progress::default(); write(&mut p);
        let mut value = json!({"processId":"s","check":report("s","passed"),"freshness":{"matches":true},"runtimeMatches":true,"configurationMatches":true,"coverageComplete":true,"changedDuringObservation":false,"observation":{"complete":true}});
        p.observe("verification_run", &json!({"command":"cargo test"}), &value, false);
        assert!(p.needs_verification());
        value["freshness"]["matches"] = json!(false);
        p.observe("verification_report", &json!({}), &value, false); assert!(p.needs_verification());
        value["freshness"]["matches"] = json!(true);
        p.observe("verification_report", &json!({}), &value, false); assert!(!p.needs_verification());
    }
    #[test]
    fn normalized_results_and_failed_partial_writes_keep_evidence_honest() {
        let mut p = Progress::default();
        let denied = crate::tool_envelope::failure_notice("denied","No",false,crate::tool_envelope::SIDE_NONE,&json!({}));
        p.observe("file_write", &json!({}), &denied, false); assert!(!p.needs_verification());
        let partial = crate::tool_envelope::failure_notice("timeout","Unknown",false,crate::tool_envelope::SIDE_UNKNOWN,&json!({}));
        p.observe("file_write", &json!({}), &partial, false); assert!(p.needs_verification());
        let normalized = crate::tool_envelope::normalize(report("a","passed"), crate::tool_envelope::Family::Harness);
        p.observe("check_run", &json!({"command":"test"}), &normalized, false); assert!(!p.needs_verification());
        p.observe("ptc_run", &json!({}), &json!({"ok":true}), true); assert!(p.needs_verification());
    }
}
