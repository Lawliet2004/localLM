//! Live interruption belongs to one conversation. Task-local access lets nested
//! foreground tools inherit that ownership without borrowing another run's flags.
use crate::agent_run::RunControl;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

pub struct Signals {
    pub control: Mutex<RunControl>,
    pub cancel: tokio::sync::watch::Sender<bool>,
    phase: AtomicU8,
    instance_id: String,
    visible: Mutex<VisibleState>,
}

#[derive(Default)]
struct VisibleState {
    state: Option<String>,
    activity: Option<String>,
    approvals: HashMap<String, serde_json::Value>,
}

impl Default for Signals {
    fn default() -> Self {
        Self { control: Mutex::new(RunControl::default()), cancel: tokio::sync::watch::channel(false).0, phase: AtomicU8::new(0), instance_id: uuid::Uuid::new_v4().to_string(), visible: Mutex::new(VisibleState::default()) }
    }
}

/// A registry snapshot describes slot ownership, not model or approval state.
/// It is deliberately separate from the persisted run lifecycle.
#[derive(Debug, serde::Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ActiveRun {
    conversation_id: String,
    instance_id: String,
    phase: &'static str,
    cancellation_requested: bool,
    state: Option<String>,
    activity: Option<String>,
    approvals: Vec<serde_json::Value>,
}

impl Signals {
    /// Mirror only current UI state, never response deltas.
    /// Approval payloads are kept in memory only for the life of this run.
    pub fn observe(&self, event: &serde_json::Value) {
        let Ok(mut visible) = self.visible.lock() else { return; };
        if let Some(state) = event.get("state").and_then(|v| v.as_str()) { visible.state = Some(state.into()); }
        if let Some(activity) = event.get("activity").and_then(|v| v.as_str()) { visible.activity = Some(activity.into()); }
        if let Some(approval) = event.get("approval") {
            if approval.is_null() {
                visible.approvals.clear();
            } else if let Some(id) = approval.get("id").and_then(|v| v.as_str()) {
                if approval.get("closed").and_then(|v| v.as_bool()) == Some(true) {
                    visible.approvals.remove(id);
                } else {
                    visible.approvals.insert(id.into(), approval.clone());
                }
            }
        }
    }
    pub fn request_cancel(&self) -> Result<(), String> {
        let mut control = self.control.lock().map_err(|_| "Run control is unavailable.".to_string())?;
        crate::agent_run::request_cancel(&mut control);
        // Set the flag under the same lock used when a queued turn begins.
        // Otherwise a Stop between slot acquisition and begin_generation can
        // be reset before the turn subscribes to cancellation.
        self.cancel.send_replace(true);
        Ok(())
    }
}

pub fn begin_pending_generation(
    control: &mut RunControl,
    cancel: &tokio::sync::watch::Sender<bool>,
) -> Result<(u64, tokio::sync::watch::Receiver<bool>), String> {
    if *cancel.borrow() { return Err("Message cancelled before generation; it was not saved.".into()); }
    Ok(crate::agent_run::begin_generation(control, cancel))
}

tokio::task_local! { static CURRENT: Arc<Signals>; }

pub fn in_session_scope() -> bool { CURRENT.try_with(|_| ()).is_ok() }

pub async fn scope<T>(signals: Arc<Signals>, future: impl std::future::Future<Output = T>) -> T {
    CURRENT.scope(signals, future).await
}

pub fn with_control<T>(fallback: &Mutex<RunControl>, f: impl FnOnce(&mut RunControl) -> T) -> Result<T, String> {
    let signals = CURRENT.try_with(Arc::clone).ok();
    let mut control = signals.as_ref().map(|s| &s.control).unwrap_or(fallback)
        .lock().map_err(|_| "Run control is unavailable.".to_string())?;
    Ok(f(&mut control))
}

pub fn cancellation(fallback: &tokio::sync::watch::Sender<bool>) -> tokio::sync::watch::Sender<bool> {
    CURRENT.try_with(|s| s.cancel.clone()).unwrap_or_else(|_| fallback.clone())
}

const DEFAULT_CONCURRENT_TURNS: usize = 4;

pub struct Registry {
    runs: Mutex<HashMap<String, Arc<Signals>>>,
    slots: Arc<tokio::sync::Semaphore>,
}

impl Default for Registry {
    fn default() -> Self {
        Self { runs: Mutex::new(HashMap::new()), slots: Arc::new(tokio::sync::Semaphore::new(DEFAULT_CONCURRENT_TURNS)) }
    }
}

pub struct Registration<'a> {
    registry: &'a Registry,
    conversation_id: String,
    pub signals: Arc<Signals>,
}

impl Registry {
    pub fn snapshots(&self) -> Result<Vec<ActiveRun>, String> {
        let runs = self.runs.lock().map_err(|_| "Session controls are unavailable.".to_string())?;
        let mut snapshots: Vec<_> = runs.iter().map(|(id, signals)| {
          let visible = signals.visible.lock().map_err(|_| "Session activity is unavailable.".to_string())?;
          let mut approvals: Vec<_> = visible.approvals.values().cloned().collect();
          approvals.sort_by(|a, b| a.get("id").and_then(|v| v.as_str()).cmp(&b.get("id").and_then(|v| v.as_str())));
          Ok(ActiveRun {
            conversation_id: id.clone(),
            instance_id: signals.instance_id.clone(),
            phase: match signals.phase.load(Ordering::Acquire) {
                1 => "queued",
                2 => "running",
                _ => "preparing",
            },
            cancellation_requested: *signals.cancel.borrow(),
            state: visible.state.clone(), activity: visible.activity.clone(), approvals,
          })
        }).collect::<Result<Vec<_>, String>>()?;
        snapshots.sort_by(|a, b| a.conversation_id.cmp(&b.conversation_id));
        Ok(snapshots)
    }
    pub fn is_active(&self, conversation_id: &str) -> Result<bool, String> {
        Ok(self.runs.lock().map_err(|_| "Session controls are unavailable.".to_string())?.contains_key(conversation_id))
    }
    pub fn active_count(&self) -> usize {
        self.runs.lock().map(|runs| runs.len()).unwrap_or(0)
    }
    /// Turns holding a slot, excluding queued conversations. Used for local
    /// runtime cache pinning; the waiting queue must not look like inference.
    pub fn running_count(&self) -> usize {
        DEFAULT_CONCURRENT_TURNS.saturating_sub(self.slots.available_permits())
    }
    pub fn cancel_all(&self) {
        if let Ok(runs) = self.runs.lock() {
            for signals in runs.values() {
                if signals.request_cancel().is_err() { signals.cancel.send_replace(true); }
            }
        }
    }
    pub fn register(&self, conversation_id: &str) -> Result<Registration<'_>, String> {
        let mut runs = self.runs.lock().map_err(|_| "Session controls are unavailable.".to_string())?;
        if runs.contains_key(conversation_id) { return Err("Another response is running in this conversation.".into()); }
        let signals = Arc::new(Signals::default());
        runs.insert(conversation_id.into(), signals.clone());
        Ok(Registration { registry: self, conversation_id: conversation_id.into(), signals })
    }

    // Old clients may omit the ID only when the target is unambiguous. With no
    // session, callers retain the separate diagnostic/legacy control path.
    pub fn target(&self, conversation_id: Option<&str>) -> Result<Option<Arc<Signals>>, String> {
        let runs = self.runs.lock().map_err(|_| "Session controls are unavailable.".to_string())?;
        if let Some(id) = conversation_id {
            return runs.get(id).cloned().map(Some).ok_or_else(|| "No active response in this conversation.".into());
        }
        if runs.len() > 1 { return Err("Choose a conversation to interrupt.".into()); }
        Ok(runs.values().next().cloned())
    }
}

impl Registration<'_> {
    /// Wait fairly for a slot, but never start a cancelled queued request.
    /// The callback runs once only when this turn actually has to queue.
    pub async fn wait_for_slot(&self, on_waiting: impl FnOnce()) -> Result<tokio::sync::OwnedSemaphorePermit, String> {
        let mut cancel = self.signals.cancel.subscribe();
        if *cancel.borrow() { return Err("Message cancelled before generation; it was not saved.".into()); }
        let permit = match self.registry.slots.clone().try_acquire_owned() {
            Ok(permit) => permit,
            Err(tokio::sync::TryAcquireError::NoPermits) => {
                self.signals.phase.store(1, Ordering::Release);
                on_waiting();
                tokio::select! {
                    biased;
                    _ = cancel.changed() => return Err("Message cancelled before generation; it was not saved.".into()),
                    result = self.registry.slots.clone().acquire_owned() => result.map_err(|_| "Response slots are unavailable.")?,
                }
            }
            Err(tokio::sync::TryAcquireError::Closed) => return Err("Response slots are unavailable.".into()),
        };
        if *cancel.borrow() { return Err("Message cancelled before generation; it was not saved.".into()); }
        self.signals.phase.store(2, Ordering::Release);
        if let Ok(mut visible) = self.signals.visible.lock() {
            if visible.state.as_deref() == Some("queued") { visible.state = None; }
        }
        Ok(permit)
    }
}

impl Drop for Registration<'_> {
    fn drop(&mut self) {
        if let Ok(mut runs) = self.registry.runs.lock() { runs.remove(&self.conversation_id); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshots_keep_only_pending_approval_payloads_for_their_own_run() {
        use serde_json::json;
        let registry = Registry::default();
        let a = registry.register("a").unwrap();
        let b = registry.register("b").unwrap();
        a.signals.observe(&json!({"state":"awaiting_approval", "activity":"Waiting", "approval":{"id":"a1","name":"write","arguments":{"path":"x"}}}));
        a.signals.observe(&json!({"approval":{"id":"a2","kind":"mcp_input","form":{"fields":[]}}}));
        b.signals.observe(&json!({"approval":{"id":"b1","name":"other"}}));
        a.signals.observe(&json!({"approval":{"id":"a1","closed":true}}));
        let runs = registry.snapshots().unwrap();
        assert_eq!(runs[0].state.as_deref(), Some("awaiting_approval"));
        assert_eq!(runs[0].approvals, vec![json!({"id":"a2","kind":"mcp_input","form":{"fields":[]}})]);
        assert_eq!(runs[1].approvals, vec![json!({"id":"b1","name":"other"})]);
        a.signals.observe(&json!({"approval":null}));
        assert!(registry.snapshots().unwrap()[0].approvals.is_empty());
        assert_eq!(registry.snapshots().unwrap()[1].approvals.len(), 1);
        let old_id = runs[0].instance_id.clone();
        drop(a);
        let next = registry.register("a").unwrap();
        assert_ne!(next.signals.instance_id, old_id);
        assert!(registry.snapshots().unwrap()[0].approvals.is_empty());
    }

    #[tokio::test]
    async fn stop_and_pause_are_owned_and_nested_calls_inherit_their_session() {
        let registry = Registry::default();
        let a = registry.register("a").unwrap();
        let b = registry.register("b").unwrap();
        assert_eq!(registry.active_count(), 2);
        let fallback = Mutex::new(RunControl::default());
        let legacy = tokio::sync::watch::channel(false).0;
        assert!(registry.target(None).is_err());
        assert!(registry.register("a").is_err());
        scope(a.signals.clone(), async {
            with_control(&fallback, |c| { crate::agent_run::begin_run(c); crate::agent_run::apply_pause(c); }).unwrap();
            cancellation(&legacy).send_replace(true);
            assert!(with_control(&fallback, |c| c.pause).unwrap());
        }).await;
        scope(b.signals.clone(), async {
            assert!(!with_control(&fallback, |c| c.pause).unwrap());
            assert!(!*cancellation(&legacy).borrow());
        }).await;
        assert!(!*legacy.borrow());
        assert!(!fallback.lock().unwrap().pause);
        drop(a);
        assert_eq!(registry.active_count(), 1);
        assert!(registry.target(Some("a")).is_err());
        assert!(Arc::ptr_eq(&registry.target(None).unwrap().unwrap(), &b.signals));
        drop(b);
        let next = registry.register("a").unwrap();
        assert!(!*next.signals.cancel.borrow());
    }

    #[tokio::test]
    async fn queued_turns_wait_for_a_slot_and_cancel_independently() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::time::Duration;
        let registry = Registry::default();
        let active: Vec<_> = (0..4).map(|i| registry.register(&format!("active-{i}")).unwrap()).collect();
        assert!(registry.snapshots().unwrap().iter().all(|run| run.phase == "preparing"));
        let mut permits = Vec::new();
        for turn in &active { permits.push(turn.wait_for_slot(|| panic!("free slot should not queue")).await.unwrap()); }
        assert!(registry.snapshots().unwrap().iter().all(|run| run.phase == "running"));
        assert_eq!(registry.running_count(), 4);
        let waiting = registry.register("waiting").unwrap();
        let other = registry.register("other").unwrap();
        let notices = AtomicUsize::new(0);
        let first = waiting.wait_for_slot(|| { notices.fetch_add(1, Ordering::SeqCst); });
        let second = other.wait_for_slot(|| { notices.fetch_add(1, Ordering::SeqCst); });
        tokio::pin!(first, second);
        assert!(tokio::time::timeout(Duration::from_millis(10), &mut first).await.is_err());
        assert!(tokio::time::timeout(Duration::from_millis(10), &mut second).await.is_err());
        let snapshots = registry.snapshots().unwrap();
        assert_eq!(snapshots.iter().find(|run| run.conversation_id == "waiting").unwrap().phase, "queued");
        assert_eq!(snapshots.iter().find(|run| run.conversation_id == "other").unwrap().phase, "queued");
        let snapshot = serde_json::to_value(snapshots.iter().find(|run| run.conversation_id == "waiting").unwrap()).unwrap();
        assert_eq!(snapshot["conversationId"], "waiting");
        assert_eq!(snapshot["phase"], "queued");
        assert_eq!(snapshot["cancellationRequested"], false);
        assert!(snapshot["instanceId"].as_str().is_some_and(|id| !id.is_empty()));
        assert_eq!(notices.load(Ordering::SeqCst), 2);
        assert_eq!(registry.active_count(), 6);
        registry.target(Some("waiting")).unwrap().unwrap().cancel.send_replace(true);
        assert!(registry.snapshots().unwrap().iter().find(|run| run.conversation_id == "waiting").unwrap().cancellation_requested);
        assert!(first.await.err().unwrap().contains("cancelled before generation"));
        assert!(!*other.signals.cancel.borrow());
        drop(permits.pop());
        let granted = tokio::time::timeout(Duration::from_secs(1), second).await.unwrap().unwrap();
        assert_eq!(registry.snapshots().unwrap().iter().find(|run| run.conversation_id == "other").unwrap().phase, "running");
        assert_eq!(registry.running_count(), 4);
        drop(granted);
        drop(permits);
        assert_eq!(registry.running_count(), 0);
    }

    #[tokio::test]
    async fn cancellation_before_queue_subscription_never_starts_a_turn() {
        let registry = Registry::default();
        let turn = registry.register("cancelled").unwrap();
        turn.signals.cancel.send_replace(true);
        assert!(turn.wait_for_slot(|| panic!("already cancelled")).await.err().unwrap().contains("cancelled before generation"));
        assert_eq!(registry.running_count(), 0);
    }

    #[tokio::test]
    async fn new_arrivals_do_not_jump_a_waiting_turn() {
        use std::time::Duration;
        let registry = Registry::default();
        let active: Vec<_> = (0..4).map(|i| registry.register(&format!("active-{i}")).unwrap()).collect();
        let mut permits = Vec::new();
        for turn in &active { permits.push(turn.wait_for_slot(|| {}).await.unwrap()); }
        let first = registry.register("first").unwrap();
        let waiting = first.wait_for_slot(|| {});
        tokio::pin!(waiting);
        assert!(tokio::time::timeout(Duration::from_millis(10), &mut waiting).await.is_err());
        drop(permits.pop());
        let later = registry.register("later").unwrap();
        let newcomer = later.wait_for_slot(|| {});
        tokio::pin!(newcomer);
        assert!(tokio::time::timeout(Duration::from_millis(10), &mut newcomer).await.is_err());
        let granted = tokio::time::timeout(Duration::from_secs(1), waiting).await.unwrap().unwrap();
        drop(granted);
        let _later_slot = tokio::time::timeout(Duration::from_secs(1), newcomer).await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn stop_at_the_slot_handoff_is_not_cleared_by_beginning_the_turn() {
        let registry = Registry::default();
        let turn = registry.register("handoff").unwrap();
        let _slot = turn.wait_for_slot(|| panic!("free slot")).await.unwrap();
        turn.signals.request_cancel().unwrap();
        {
            let mut control = turn.signals.control.lock().unwrap();
            assert!(begin_pending_generation(&mut control, &turn.signals.cancel).is_err());
        }

        let next = registry.register("next").unwrap();
        let mut cancellation = {
            let mut control = next.signals.control.lock().unwrap();
            begin_pending_generation(&mut control, &next.signals.cancel).unwrap().1
        };
        next.signals.request_cancel().unwrap();
        cancellation.changed().await.unwrap();
        assert!(*cancellation.borrow());
    }
}
