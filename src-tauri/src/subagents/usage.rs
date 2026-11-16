//! Reported child inference usage survives successful, failed and dropped reads.
use super::{BackendSnapshot, ChildRequest};
use crate::inference::InferenceRound;
use serde_json::json;
use std::time::Instant;

pub(super) struct ChildRound {
    db: std::path::PathBuf,
    owner: String,
    child: String,
    model: String,
    index: usize,
    started: Instant,
    pub measurement: InferenceRound,
    saved: bool,
}

impl ChildRound {
    pub fn new(
        snapshot: &BackendSnapshot,
        request: &ChildRequest,
        child: &str,
        index: usize,
    ) -> Self {
        Self {
            db: snapshot.db_path.clone(),
            owner: request.conversation_id.clone(),
            child: child.into(),
            model: snapshot.model_id.clone(),
            index,
            started: Instant::now(),
            measurement: InferenceRound::default(),
            saved: false,
        }
    }
    pub fn elapsed_ms(&self) -> f64 {
        self.started.elapsed().as_secs_f64() * 1000.0
    }

    pub fn finish(&mut self, complete: bool) -> Result<(), String> {
        self.measurement.duration_ms = self.elapsed_ms();
        self.measurement.complete = complete;
        let store = crate::store::Store::open(&self.db)?;
        let payload = json!({"round":self.index,"model":self.model,"usage":self.measurement.usage,"measurement":self.measurement});
        store.append_run_event(&crate::agent_run::RunEvent {
            run_id: self.child.clone(),
            seq: 0,
            step_id: format!("round-{}", self.index),
            tool_call_id: None,
            event_type: "inference_usage".into(),
            payload: payload.clone(),
            created_at: crate::store::now(),
        })?;
        crate::chat::emit_session_event(
            &store,
            &self.owner,
            Some(&self.child),
            Some(&format!("round-{}", self.index)),
            None,
            "inference_usage",
            payload,
            false,
        );
        self.saved = true;
        Ok(())
    }
}

impl Drop for ChildRound {
    fn drop(&mut self) {
        if !self.saved {
            if let Err(error) = self.finish(false) {
                eprintln!("Could not persist interrupted child usage: {error}");
            }
        }
    }
}

pub(crate) fn for_parent(
    store: &crate::store::Store,
    owner: &str,
    parent: &str,
) -> Result<Vec<crate::inference::ChildMetrics>, String> {
    let rows = store.subagent_runs_for_conversation(owner)?;
    let mut included = std::collections::HashSet::from([parent.to_string()]);
    loop {
        let count = included.len();
        for row in &rows {
            if included.contains(&row.parent_run_id) {
                included.insert(row.child_run_id.clone());
            }
        }
        if count == included.len() {
            break;
        }
    }
    rows.into_iter()
        .filter(|row| row.child_run_id != parent && included.contains(&row.child_run_id))
        .map(|row| {
            let rounds = store
                .run_events(&row.child_run_id)?
                .into_iter()
                .filter(|event| event.event_type == "inference_usage")
                .filter_map(|event| {
                    serde_json::from_value(event.payload["measurement"].clone()).ok()
                })
                .collect();
            Ok(crate::inference::ChildMetrics {
                run_id: row.child_run_id,
                status: row.status,
                rounds,
            })
        })
        .collect()
}
