//! A goal file is an explicit user input. Its acceptance items persist across
//! segments, independently of the model transcript and provider cache.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::Path;

const MAX_GOAL_BYTES: usize = 128 * 1024;
const MAX_CRITERIA: usize = 64;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Goal {
    pub path: String,
    pub sha256: String,
    criteria: Vec<Criterion>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Criterion {
    id: String,
    title: String,
    command: Option<String>,
    state: String,
    note: String,
    evidence: Option<String>,
}

pub(crate) fn requested_path(content: &str) -> Option<&str> {
    let rest = content.trim().strip_prefix("/goal")?;
    (rest.is_empty() || rest.starts_with(char::is_whitespace)).then(|| rest.trim())
}

pub(crate) fn relative_path<'a>(root: &str, path: &'a str) -> Result<&'a str, String> {
    if path.is_empty() {
        return Err(
            "Use /goal followed by a Markdown or text file in the selected workspace.".into(),
        );
    }
    let input = Path::new(path);
    let relative = if input.is_absolute() {
        input
            .strip_prefix(root)
            .map_err(|_| "The goal file must be inside the selected workspace.")?
    } else {
        input
    };
    let path = relative.to_str().ok_or("The goal path must be UTF-8.")?;
    crate::workspace::valid_path(path, false)?;
    Ok(path)
}

pub(crate) fn read(root: &str, path: &str) -> Result<(Goal, String), String> {
    let path = relative_path(root, path)?;
    let workspace = crate::workspace::Workspace::open(root)?;
    let bytes = workspace.development_bytes(path, MAX_GOAL_BYTES)?;
    let sha256 = format!("{:x}", Sha256::digest(&bytes));
    let text = String::from_utf8(bytes).map_err(|_| "The goal file must be UTF-8 text.")?;
    if text.trim().is_empty() {
        return Err("The goal file is empty.".into());
    }
    let mut criteria = Vec::new();
    let mut verification = false;
    let mut handoff = false;
    let mut fenced = false;
    for line in text.lines().map(str::trim) {
        if let Some(heading) = line.strip_prefix("## ") {
            let lower = heading.to_lowercase();
            verification = lower.starts_with("verification commands");
            handoff = lower.starts_with("definition of done");
            fenced = false;
            if heading
                .split_once(". ")
                .is_some_and(|(number, _)| number.parse::<u32>().is_ok())
            {
                add(&mut criteria, heading, None)?;
            }
        } else if line.starts_with("```") {
            fenced = !fenced;
        } else if verification && fenced && !line.is_empty() && !line.starts_with('#') {
            add(&mut criteria, line, Some(line))?;
        } else if !fenced {
            if let Some(item) = line
                .strip_prefix("- [ ] ")
                .or_else(|| line.strip_prefix("- [x] "))
            {
                add(&mut criteria, item, None)?;
            } else if handoff {
                if let Some(item) = line.strip_prefix("- ") {
                    add(&mut criteria, item, None)?;
                }
            }
        }
    }
    if criteria.is_empty() {
        add(
            &mut criteria,
            "Deliver and verify the work specified in the goal file",
            None,
        )?;
    }
    Ok((
        Goal {
            path: path.into(),
            sha256,
            criteria,
        },
        text,
    ))
}

fn add(criteria: &mut Vec<Criterion>, title: &str, command: Option<&str>) -> Result<(), String> {
    if let Some(item) = criteria.iter_mut().find(|item| item.title == title) {
        if let Some(command) = command {
            item.command = Some(command.into());
        }
        return Ok(());
    }
    if criteria.len() >= MAX_CRITERIA || title.len() > 4096 {
        return Err("Split this goal into smaller files with up to 64 acceptance items.".into());
    }
    criteria.push(Criterion {
        id: format!("G{}", criteria.len() + 1),
        title: title.into(),
        command: command.map(str::to_owned),
        state: "pending".into(),
        note: String::new(),
        evidence: None,
    });
    Ok(())
}

impl Goal {
    pub(crate) fn has_actionable_work(
        &self,
        verification: &crate::verification_progress::Progress,
    ) -> bool {
        self.criteria.iter().any(|item| matches!(item.result(verification).0, "pending" | "failed"))
    }

    pub(crate) fn requires_command(&self, command: &str) -> bool {
        self.criteria
            .iter()
            .any(|item| item.command.as_deref() == Some(command))
    }

    pub(crate) fn pending(
        &self,
        verification: &crate::verification_progress::Progress,
    ) -> Vec<String> {
        self.criteria
            .iter()
            .filter(|item| item.result(verification).0 != "passed")
            .map(|item| {
                format!(
                    "{} · {}{}",
                    item.id,
                    item.title,
                    if item.note.is_empty() {
                        String::new()
                    } else {
                        format!(" ({})", item.note)
                    }
                )
            })
            .collect()
    }

    pub(crate) fn report(&self, verification: &crate::verification_progress::Progress) -> Value {
        let items = self
            .criteria
            .iter()
            .map(|item| {
                let (state, evidence) = item.result(verification);
                json!({"id":item.id,"title":item.title,"command":item.command,"state":state,
                "evidence":evidence,"note":item.note})
            })
            .collect::<Vec<_>>();
        json!({"path":self.path,"sha256":self.sha256,"requirements":items,"runningChecks":verification.running_gates()})
    }

    pub(crate) fn update(
        &mut self,
        args: &Value,
        verification: &crate::verification_progress::Progress,
        has_evidence: impl FnOnce(&str) -> Result<bool, String>,
    ) -> Result<(), String> {
        let id = args["id"]
            .as_str()
            .ok_or("Provide a goal requirement ID.")?;
        let item = self
            .criteria
            .iter_mut()
            .find(|item| item.id == id)
            .ok_or("Unknown goal requirement. Call goal_status for the current IDs.")?;
        let state = args["state"]
            .as_str()
            .ok_or("Provide a requirement state.")?;
        let note = args["note"]
            .as_str()
            .filter(|note| !note.trim().is_empty() && note.len() <= 2000)
            .ok_or("Provide a note of 1–2000 bytes describing the result or blocker.")?;
        let evidence = if state == "passed" {
            if let Some(command) = &item.command {
                Some(verification.evidence_for(command).ok_or("Run the exact gate and collect its terminal check report on the current revision.")?.to_owned())
            } else {
                let id = args["audit_id"].as_str().ok_or("Provide an audit ID for the successful change or check that satisfies this requirement.")?;
                if !has_evidence(id)? {
                    return Err("This audit ID does not identify a successful change or check for this task.".into());
                }
                Some(id.to_owned())
            }
        } else {
            if !matches!(state, "pending" | "failed" | "blocked") {
                return Err("Use pending, failed, blocked, or passed.".into());
            }
            None
        };
        item.state = state.into();
        item.note = note.into();
        item.evidence = evidence;
        Ok(())
    }
}

impl Criterion {
    fn result<'a>(
        &'a self,
        verification: &'a crate::verification_progress::Progress,
    ) -> (&'a str, Option<&'a str>) {
        if let Some(command) = &self.command {
            if let Some(evidence) = verification.evidence_for(command) {
                return ("passed", Some(evidence));
            }
            if self.state == "passed" {
                return ("pending", None);
            }
        }
        (&self.state, self.evidence.as_deref())
    }
}

pub(crate) fn registry() -> Vec<crate::tool_spec::ToolSpec> {
    let schema = |properties, required: &[&str]| json!({"type":"object","properties":properties,"required":required,"additionalProperties":false});
    vec![
        ("goal_status".into(), "Read the saved goal requirements, pending gates, and evidence.".into(),
            schema(json!({}), &[])),
        ("goal_update".into(), "Update a requirement on the current goal. A named command needs a terminal check on the current source revision. Other passed requirements need a successful change or check audit ID. Use blocked with a concrete prerequisite when needed.".into(),
            schema(json!({"id":{"type":"string","maxLength":32},"state":{"type":"string","enum":["pending","failed","blocked","passed"]},
                "note":{"type":"string","maxLength":2000},"audit_id":{"type":"string","maxLength":128}}),
                &["id","state","note"])),
    ].into_iter().map(crate::tool_spec::ToolSpec::from).collect()
}

pub(crate) fn execute(
    ctx: &crate::harness::HarnessCtx<'_>,
    alias: &str,
    args: &Value,
) -> Result<Value, String> {
    let store = ctx.state.database()?;
    let mut task = store
        .latest_research_task(&ctx.conversation_id)?
        .filter(|task| task.status == crate::research_tasks::TaskStatus::Running)
        .ok_or("No active goal for this turn.")?;
    if !store
        .research_task_run(&task)?
        .is_some_and(|run| run.id == ctx.run_id)
    {
        return Err("This goal belongs to a different run.".into());
    }
    let verification = &task.context.verification;
    let goal = task
        .context
        .goal
        .as_mut()
        .ok_or("This turn has no goal file.")?;
    if alias == "goal_update" {
        goal.update(args, verification, |audit_id| {
            let mut valid = false;
            store.visit_research_resume_audits(&ctx.conversation_id, &task.id, |id, text| {
                if id != audit_id {
                    return Ok(());
                }
                let audit: Value = serde_json::from_str(text).map_err(|error| error.to_string())?;
                let result = &audit["result"];
                let name = audit["request"]["name"].as_str().unwrap_or_default();
                valid = audit["request"]["decision"] == "allowed"
                    && crate::tool_envelope::operation_status(result) == Some("success")
                    && (crate::workspace::is_write_call(
                        name.strip_prefix("workspace_").unwrap_or(name),
                    ) || result
                        .pointer("/data/report/verdict")
                        .or_else(|| result.pointer("/data/check/report/verdict"))
                        .is_some_and(|verdict| verdict == "passed"));
                Ok(())
            })?;
            Ok(valid)
        })?;
        task.updated_at = crate::store::now();
    }
    let report = goal.report(verification);
    if alias == "goal_update" {
        store.save_research_task(&task)?;
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scoped_goal_preserves_requirements_and_detects_revisions() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let folder = root.path().to_str().unwrap();
        let content = "# Release goal\n## 1. Patch the feature\n- [ ] cargo test --lib\n## Verification commands\n```sh\ncargo test --lib\ncargo build --release\n```\n## Definition of done\n- Record the smoke-test outcome\n";
        let path = root.path().join("GOAL.md");
        std::fs::write(&path, content).unwrap();
        let (goal, body) = read(folder, path.to_str().unwrap()).unwrap();
        assert_eq!(body, content);
        assert_eq!(goal.path, "GOAL.md");
        assert_eq!(goal.criteria.len(), 4);
        assert_eq!(
            goal.criteria[1].command.as_deref(),
            Some("cargo test --lib")
        );
        assert_eq!(goal.pending(&Default::default()).len(), 4);
        std::fs::write(outside.path().join("other.md"), "Outside").unwrap();
        assert!(read(folder, outside.path().join("other.md").to_str().unwrap()).is_err());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(
                outside.path().join("other.md"),
                root.path().join("link.md"),
            )
            .unwrap();
            assert!(read(folder, "link.md").is_err());
        }
        std::fs::write(&path, "# Updated\n- [ ] A different requirement").unwrap();
        assert_ne!(goal.sha256, read(folder, "GOAL.md").unwrap().0.sha256);
    }

    #[test]
    fn goal_requires_real_audit_and_exact_current_source_bound_evidence() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("GOAL.md"),
            "## 1. Edit\n## Verification commands\n```sh\ncargo build --release\n```\n",
        )
        .unwrap();
        let (mut goal, _) = read(directory.path().to_str().unwrap(), "GOAL.md").unwrap();
        let mut progress = crate::verification_progress::Progress::default();
        let edit =
            json!({"id":"G1","state":"passed","note":"Edited the feature","audit_id":"audit-1"});
        assert!(goal.update(&edit, &progress, |_| Ok(false)).is_err());
        goal.update(&edit, &progress, |id| Ok(id == "audit-1"))
            .unwrap();
        let command = json!({"id":"G2","state":"passed","note":"Release build completed"});
        let args = json!({"command":"cargo build --release"});
        let check = json!({"processId":"proc-1","process":{"state":"exited","exitCode":0},"report":{"verdict":"command_succeeded","commandSucceeded":true,"incomplete":false}});
        progress.observe("check_run", &args, &check, true);
        assert!(goal.update(&command, &progress, |_| Ok(true)).is_err());
        let evidence = json!({"processId":"proc-1","reportId":"dev_fixture_report","check":check,"freshness":{"matches":true},"runtimeMatches":true,
            "configurationMatches":true,"coverageComplete":true,"changedDuringObservation":false,"observation":{"complete":true}});
        progress.observe("verification_run", &args, &evidence, true);
        assert_eq!(goal.pending(&progress).len(), 1);
        progress.observe("verification_report", &json!({}), &evidence, true);
        goal.update(&command, &progress, |_| Ok(false)).unwrap();
        assert!(goal.pending(&progress).is_empty());
        let bytes = serde_json::to_vec(&progress).unwrap();
        let mut restored = serde_json::from_slice(&bytes).unwrap();
        assert!(goal.pending(&restored).is_empty());
        restored.invalidate();
        assert_eq!(goal.pending(&restored).len(), 1);
    }
}
