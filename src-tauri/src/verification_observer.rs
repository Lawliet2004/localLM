//! Native event observations; gaps are explicit and never become a clean result.
use notify::{EventKind, RecursiveMode, Watcher};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
};
struct Watch {
    _watcher: notify::RecommendedWatcher,
    state: Arc<Mutex<Value>>,
    owner: String,
    root: String,
}
#[derive(Default)]
pub struct Registry {
    watches: Mutex<BTreeMap<String, Watch>>,
}
pub struct Pending {
    watcher: notify::RecommendedWatcher,
    barrier: String,
    state: Arc<Mutex<Value>>,
}
// Owned by the process supervisor before launch, so even a child that exits
// immediately cannot finish before its observation completion hook is installed.
pub(crate) struct Completion {
    pub pending: Pending,
    pub root: String,
    pub sources: Value,
    pub membership: Option<Value>,
    pub db: std::path::PathBuf,
    pub owner: String,
    pub run: String,
    pub id: String,
}
impl Completion {
    pub async fn finish(self, phase: &str) {
        let ended = crate::store::now();
        // The freshness scan and the source inventory walk the whole
        // workspace synchronously. They run on a blocking thread so the tokio
        // worker stays free: the process supervisor awaits this completion
        // right after it acknowledges a stop.
        let root = self.root.clone();
        let sources = self.sources.clone();
        let before = self.membership.clone();
        let (freshness, membership) = tokio::task::spawn_blocking(move || {
            let freshness = crate::development_tools::freshness(&root, &sources);
            let membership = before.as_ref().map(|before| {
                crate::source_inventory::inventory(&root)
                    .map(|after| {
                        before["paths"] == after["paths"]
                            && before["complete"] == true
                            && after["complete"] == true
                    })
                    .unwrap_or(false)
            });
            (freshness, membership)
        })
        .await
        .unwrap_or_else(|_| {
            (
                json!({"matches":false,"reason":"source freshness could not be computed"}),
                None,
            )
        });
        // Linux inotify uses one ordered queue for the recursive watcher.
        // Seeing our owned marker proves earlier queued events were delivered.
        // Other backends lack this portable guarantee and remain incomplete.
        let marker_path = std::path::Path::new(&self.root).join(&self.pending.barrier);
        let marker = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&marker_path);
        let mut drained = false;
        if marker.is_ok() {
            for _ in 0..100 {
                if self
                    .pending
                    .state
                    .lock()
                    .map(|v| v["barrierObserved"] == true)
                    .unwrap_or(false)
                {
                    drained = cfg!(target_os = "linux");
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        }
        drop(self.pending.watcher);
        if marker.is_ok() {
            let _ = std::fs::remove_file(marker_path);
        }
        let mut value = match self.pending.state.lock() {
            Ok(v) => v.clone(),
            Err(_) => {
                json!({"complete":false,"warnings":["observer_state_unavailable"],"changed":[]})
            }
        };
        if !drained {
            value["complete"] = json!(false);
            value["warnings"] = json!(["watcher_completion_barrier_unavailable"]);
        }
        value["endedAt"] = json!(ended);
        value["sealedAt"] = json!(crate::store::now());
        value["processState"] = json!(phase);
        value["endpointFreshness"] = freshness.clone();
        value["membershipMatches"] = json!(membership);
        value["root"] = json!(self.root);
        value["boundaryPolicy"] = json!("Includes events through an owned queue marker; post-exit events before sealing are conservatively attributed to execution. Non-Linux backends without a certified queue barrier remain incomplete.");
        if freshness["matches"] != true || membership == Some(false) || phase != "exited" {
            value["complete"] = json!(false);
        }
        let content = value.to_string();
        let artifact = crate::artifacts::ArtifactRecord {
            id: self.id,
            conversation_id: self.owner,
            run_id: Some(self.run),
            tool_name: "verification_observation".into(),
            mime_type: "application/json".into(),
            size_bytes: content.len(),
            sha256: crate::development_tools::hash(content.as_bytes()),
            content,
            created_at: crate::store::now(),
        };
        // Failure leaves no completion evidence: readers fail closed, including
        // after restart. Never substitute a fresh observation at report time.
        // The write is blocking, so it never runs on the tokio worker.
        let db = self.db.clone();
        let _ = tokio::task::spawn_blocking(move || {
            if let Ok(store) = crate::store::Store::open(&db) {
                let _ = store.save_artifact(&artifact);
            }
        })
        .await;
    }
}
impl Registry {
    pub fn begin(root: &str) -> Result<Pending, String> {
        let state = Arc::new(Mutex::new(
            json!({"changed":[],"complete":true,"warnings":[],"startedAt":crate::store::now()}),
        ));
        let barrier = format!(".locallm-observer-{}", uuid::Uuid::new_v4().simple());
        let marker = barrier.clone();
        let output = state.clone();
        let prefix = std::path::PathBuf::from(root);
        let mut watcher =
            notify::recommended_watcher(move |event: Result<notify::Event, notify::Error>| {
                if let Ok(mut state) = output.lock() {
                    match event {
                        Ok(event) => {
                            if event.need_rescan() {
                                state["complete"] = json!(false);
                                state["warnings"] = json!(["native_observer_rescan_required"]);
                            }
                            if matches!(event.kind, EventKind::Access(_)) {
                                return;
                            }
                            let mut paths = state["changed"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .filter_map(Value::as_str)
                                .map(str::to_string)
                                .collect::<BTreeSet<_>>();
                            for path in event.paths {
                                if let Ok(relative) = path.strip_prefix(&prefix) {
                                    let path = relative.to_string_lossy().replace('\\', "/");
                                    if path == marker {
                                        state["barrierObserved"] = json!(true);
                                    } else {
                                        paths.insert(path);
                                    }
                                }
                            }
                            if paths.len() > 4096 {
                                state["complete"] = json!(false);
                                state["warnings"] = json!(["event_budget_exceeded"]);
                            }
                            state["changed"] =
                                json!(paths.into_iter().take(4096).collect::<Vec<_>>());
                        }
                        Err(_) => {
                            state["complete"] = json!(false);
                            state["warnings"] = json!(["native_observer_error"]);
                        }
                    }
                }
            })
            .map_err(|e| format!("Cannot initialize source observer: {e}"))?;
        watcher
            .watch(std::path::Path::new(root), RecursiveMode::Recursive)
            .map_err(|e| format!("Cannot watch selected workspace: {e}"))?;
        Ok(Pending {
            watcher,
            barrier,
            state,
        })
    }
    pub fn attach(
        &self,
        id: &str,
        owner: &str,
        root: &str,
        pending: Pending,
    ) -> Result<(), String> {
        let mut watches = self
            .watches
            .lock()
            .map_err(|_| "Source observer registry unavailable.")?;
        if watches.len() >= 64 {
            return Err("Source observation limit reached; finish existing checks.".into());
        }
        watches.insert(
            id.into(),
            Watch {
                _watcher: pending.watcher,
                state: pending.state,
                owner: owner.into(),
                root: root.into(),
            },
        );
        Ok(())
    }
    pub fn read(
        &self,
        id: &str,
        owner: &str,
        root: &str,
        finished: bool,
    ) -> Result<Option<Value>, String> {
        let mut watches = self
            .watches
            .lock()
            .map_err(|_| "Source observer registry unavailable.")?;
        let Some(w) = watches.get(id) else {
            return Ok(None);
        };
        if w.owner != owner || w.root != root {
            return Err("Source observation belongs to another conversation/workspace.".into());
        }
        let mut value = w
            .state
            .lock()
            .map_err(|_| "Source observer unavailable.")?
            .clone();
        if finished {
            value["endedAt"] = json!(crate::store::now());
            watches.remove(id);
        }
        Ok(Some(value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn native_events_detect_modified_then_restored_sources() {
        let t = tempfile::tempdir().unwrap();
        std::fs::write(t.path().join("main.rs"), "original").unwrap();
        let registry = Registry::default();
        let pending = Registry::begin(t.path().to_str().unwrap()).unwrap();
        registry
            .attach("evidence", "owner", t.path().to_str().unwrap(), pending)
            .unwrap();
        std::fs::write(t.path().join("main.rs"), "changed").unwrap();
        std::fs::write(t.path().join("main.rs"), "original").unwrap();
        let mut events = Value::Null;
        for _ in 0..20 {
            events = registry
                .read("evidence", "owner", t.path().to_str().unwrap(), false)
                .unwrap()
                .unwrap();
            if events["changed"]
                .as_array()
                .unwrap()
                .iter()
                .any(|p| p == "main.rs")
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert_eq!(events["complete"], true);
        assert!(
            events["changed"]
                .as_array()
                .unwrap()
                .iter()
                .any(|p| p == "main.rs"),
            "{events}"
        );
        assert!(registry
            .read("evidence", "foreign", t.path().to_str().unwrap(), false)
            .is_err());
    }
}
