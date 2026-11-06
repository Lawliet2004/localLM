//! Parent-turn verification bookkeeping. This is a completion reminder, not a
//! substitute for source-bound evidence or a sandbox for opaque commands.
use serde_json::Value;
use std::collections::{HashMap, HashSet};

#[derive(Default)]
pub struct Progress {
    revision: u64,
    verified_revision: Option<u64>,
    checks: HashMap<String, Check>,
    failures: HashSet<String>,
    last_outcome: Option<String>,
}
struct Check { revision: u64, key: String, pending: bool, source_bound: bool, verdict: Option<String> }

fn payload(result: &Value) -> &Value {
    if crate::tool_envelope::is_envelope(result) { &result["data"] } else { result }
}

impl Progress {
    pub fn needs_verification(&self) -> bool {
        self.revision > 0 && (self.verified_revision != Some(self.revision)
            || !self.failures.is_empty()
            || self.checks.values().any(|c| c.pending && c.revision == self.revision))
    }

    pub fn outcome(&self) -> Value {
        let status = if self.revision == 0 {
            "not_required"
        } else if self.needs_verification() {
            "unverified"
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

    pub fn reminder(&self) -> String {
        format!("{} Latest observed check outcome: {}. Starting a process is not verification. Collect a terminal check_report or verification_report, fix failures, and rerun relevant checks after the final edit. A successful build only verifies that command; do not claim unperformed tests, visual inspection or gameplay. If checks cannot run or are not applicable, explain that limitation and any remaining failures explicitly. Do not bypass disabled tools or declined preview.", crate::continuation::VERIFY_BEFORE_FINISH, self.last_outcome.as_deref().unwrap_or("no completed check for the current edits"))
    }

    pub fn observe(&mut self, name: &str, args: &Value, result: &Value, development_task: bool) {
        let data = payload(result);
        let side_effects = result.pointer("/error/sideEffects").and_then(Value::as_str)
            .or_else(|| result.pointer("/metadata/sideEffects").and_then(Value::as_str));
        let no_effect = matches!(side_effects, Some("none" | "rolled_back"));
        let failed = crate::tool_envelope::failed(result) || result["isError"] == true;
        let write = name.strip_prefix("workspace_").is_some_and(crate::workspace::is_write_call)
            || matches!(name, "file_write" | "format_paths" | "lsp_rename" | "lsp_apply_edit" | "lsp_execute_command" | "worktree_enter" | "worktree_exit");
        let opaque = development_task && matches!(name, "shell_run" | "process_start" | "process_write" | "terminal_send" | "run_code" | "ptc_run" | "subagent" | "workflow_run");
        // Interrupted/partially applied edits invalidate evidence too. Explicit
        // policy denials and rolled-back transactions do not.
        if (write || opaque) && !(failed && no_effect) {
            self.revision = self.revision.saturating_add(1);
        }
        if !matches!(name, "check_run" | "check_report" | "verification_run" | "verification_report") { return; }
        let source_bound = name.starts_with("verification_");
        let check = if source_bound { &data["check"] } else { data };
        let Some(id) = data["processId"].as_str().or_else(|| check["processId"].as_str()) else { return; };
        if matches!(name, "check_run" | "verification_run") {
            // Record before inspecting the report, including failed immediate
            // exits. A later report can settle only a launch owned by this turn.
            self.checks.insert(id.into(), Check { revision: self.revision, key: format!("{}\n{}", args["cwd"].as_str().unwrap_or("."), args["command"].as_str().unwrap_or("")), pending: true, source_bound, verdict: None });
        }
        let Some(launch) = self.checks.get_mut(id) else { return; };
        if launch.source_bound != source_bound { return; }
        // verification_run deliberately does not certify the source snapshot;
        // verification_report supplies the actual freshness/observer result.
        if name == "verification_run" { return; }
        let Some(verdict) = check["report"]["verdict"].as_str() else { return; };
        self.last_outcome = Some(verdict.into());
        launch.pending = verdict == "running";
        launch.verdict = Some(verdict.into());
        let successful = !failed && matches!(verdict, "passed" | "command_succeeded")
            && check["report"]["commandSucceeded"] == true
            && check["report"]["incomplete"] == false
            && check["process"]["state"] == "exited" && check["process"]["exitCode"] == 0;
        let fresh = !source_bound || (data["freshness"]["matches"] == true
            && data["runtimeMatches"] == true && data["configurationMatches"] == true
            && data["coverageComplete"] == true && data["changedDuringObservation"] == false
            && data["observation"]["complete"] == true);
        if successful && fresh && launch.revision == self.revision {
            self.failures.remove(&launch.key);
            self.verified_revision = Some(self.revision);
        } else if verdict != "running" {
            self.failures.insert(launch.key.clone());
        }
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
    fn running_failed_empty_and_incomplete_checks_never_clear_edits() {
        for verdict in ["running","failed","command_failed","no_tests","incomplete","timed_out","stopped","unknown"] {
            let mut p = Progress::default(); write(&mut p); launch(&mut p,"a",verdict,"test");
            assert!(p.needs_verification(), "{verdict}");
        }
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
