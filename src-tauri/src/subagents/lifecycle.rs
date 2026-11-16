//! Admission and durable settlement owned by the lifetime of a child future.
use super::{
    BackendSnapshot, ChildCancel, ChildRequest, SubagentRegistry, SubagentRun, INTERRUPTED,
};
use crate::agent_run::{RunRecord, RunState};
use serde_json::json;
use std::path::PathBuf;
use std::sync::Arc;

pub(super) struct ChildLease {
    pub id: String,
    registry: Arc<SubagentRegistry>,
    db: PathBuf,
    owner: String,
    background: bool,
    parent: String,
    settled: bool,
    _permit: tokio::sync::OwnedSemaphorePermit,
}

impl ChildLease {
    pub fn create(
        registry: Arc<SubagentRegistry>,
        snapshot: &BackendSnapshot,
        request: &ChildRequest,
        background: bool,
    ) -> Result<Self, String> {
        let permit = registry.slots_for(&snapshot.db_path)?.try_acquire_owned()
            .map_err(|_| "At most eight child agents may run across the application. Wait for a child to finish or interrupt one first.".to_string())?;
        let lease = Self {
            id: format!("child-{}", uuid::Uuid::new_v4()),
            registry,
            db: snapshot.db_path.clone(),
            owner: request.conversation_id.clone(),
            background,
            parent: request.parent_run_id.clone(),
            settled: false,
            _permit: permit,
        };
        let store = crate::store::Store::open(&lease.db)?;
        let tx = store
            .connection
            .unchecked_transaction()
            .map_err(|e| e.to_string())?;
        let now = crate::store::now();
        store.save_run(&RunRecord {
            id: lease.id.clone(),
            conversation_id: lease.owner.clone(),
            status: RunState::Preparing,
            model_provider: None,
            model_id: Some(snapshot.model_id.clone()),
            checkpoint: Some("subagent".into()),
            error: None,
            created_at: now,
            updated_at: now,
        })?;
        store.insert_subagent_run(&SubagentRun {
            id: format!("sub-{}", uuid::Uuid::new_v4()),
            conversation_id: lease.owner.clone(),
            parent_run_id: request.parent_run_id.clone(),
            child_run_id: lease.id.clone(),
            depth: request.depth + 1,
            status: "running".into(),
            label: request.label.clone(),
            prompt: request.prompt.clone(),
            error: None,
            created_at: now,
            updated_at: now,
        })?;
        store.append_run_event(&crate::agent_run::RunEvent {
            run_id: lease.id.clone(), seq: 0, step_id: "start".into(), tool_call_id: None,
            event_type: "subagent_start".into(),
            payload: json!({"parentRun":request.parent_run_id,"depth":request.depth+1,"label":request.label}),
            created_at: now,
        })?;
        tx.commit().map_err(|e| e.to_string())?;
        lease.registry.register(&lease.id);
        Ok(lease)
    }

    pub fn cancellation(&self, parent: Option<tokio::sync::watch::Receiver<bool>>) -> ChildCancel {
        let flag = self
            .registry
            .flags
            .lock()
            .unwrap_or_else(|e| e.into_inner())[&self.id]
            .clone();
        ChildCancel::new(flag, self.registry.cancellation(&self.id), parent)
    }

    pub fn finish(&mut self, result: &Result<String, String>) -> Result<(), String> {
        let (status, run_status, error) = match result {
            Ok(_) => ("completed", RunState::Completed, None),
            Err(error) if error == INTERRUPTED || error.starts_with("Subagent was interrupted") => {
                ("interrupted", RunState::Cancelled, Some(error.as_str()))
            }
            Err(error) => ("failed", RunState::Failed, Some(error.as_str())),
        };
        let store = crate::store::Store::open(&self.db)?;
        // Persist the actual answer before certifying completion; the small
        // background notice is only a projection of this durable result.
        store.append_run_event(&crate::agent_run::RunEvent {
            run_id: self.id.clone(),
            seq: 0,
            step_id: "settled".into(),
            tool_call_id: None,
            event_type: "subagent_result".into(),
            payload: match result {
                Ok(answer) => json!({"status":status,"answer":answer}),
                Err(error) => json!({"status":status,"error":error}),
            },
            created_at: crate::store::now(),
        })?;
        store.settle_subagent(&self.id, status, run_status, error)?;
        self.settled = true;
        self.registry.retire(&self.id);
        if self.background {
            let result = match result {
                Ok(answer) => {
                    json!({"childRunId":self.id,"status":status,"summary":answer.chars().take(1500).collect::<String>()})
                }
                Err(error) => json!({"childRunId":self.id,"status":status,"error":error}),
            };
            store.append_message(&self.owner, "tool", &json!({
                "request":{"connector":"Subagents","name":"subagent","arguments":{},"decision":"allowed","authorization":"background child settled"},
                "result":result,
            }).to_string(), "complete")?;
        }
        let mut parent = self.parent.clone();
        let rows = store.subagent_runs_for_conversation(&self.owner)?;
        let mut seen = std::collections::HashSet::new();
        while seen.insert(parent.clone()) {
            store.refresh_delegated_metrics(&self.owner, &parent)?;
            let Some(ancestor) = rows.iter().find(|row| row.child_run_id == parent) else {
                break;
            };
            parent = ancestor.parent_run_id.clone();
        }
        crate::chat::emit_session_event(
            &store,
            &self.owner,
            Some(&self.id),
            None,
            None,
            "subagent_end",
            json!({"status":status,"parentRunId":self.parent}),
            false,
        );
        Ok(())
    }
}

impl Drop for ChildLease {
    fn drop(&mut self) {
        self.registry.retire(&self.id);
        if !self.settled {
            let result = crate::store::Store::open(&self.db).and_then(|store| {
                store.settle_subagent(&self.id, "interrupted", RunState::OutcomeUnknown,
                    Some("The child worker ended before terminal settlement. Review recorded operations; do not replay uncertain side effects."))
            });
            if let Err(error) = result {
                eprintln!("Could not settle an interrupted child: {error}");
            }
        }
    }
}
