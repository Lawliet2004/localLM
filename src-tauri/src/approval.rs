use std::{collections::HashMap, sync::Mutex};
use tokio::sync::oneshot;

#[derive(Default)]
pub struct Approvals(Mutex<HashMap<String, oneshot::Sender<bool>>>);
pub struct PendingApproval<'a> {
    approvals: &'a Approvals,
    id: String,
}
impl Drop for PendingApproval<'_> {
    fn drop(&mut self) {
        self.approvals.remove(&self.id);
    }
}
impl Approvals {
    pub fn guard(&self, id: &str) -> PendingApproval<'_> {
        PendingApproval {
            approvals: self,
            id: id.to_string(),
        }
    }
    pub fn request(&self) -> Result<(String, oneshot::Receiver<bool>), String> {
        let id = uuid::Uuid::new_v4().to_string();
        let (sender, receiver) = oneshot::channel();
        self.0
            .lock()
            .map_err(|_| "Approval state unavailable.")?
            .insert(id.clone(), sender);
        Ok((id, receiver))
    }
    pub fn resolve(&self, id: &str, allow: bool) -> Result<(), String> {
        self.0
            .lock()
            .map_err(|_| "Approval state unavailable.")?
            .remove(id)
            .ok_or("This approval is no longer pending.")?
            .send(allow)
            .map_err(|_| "The tool request has already ended.".into())
    }
    pub fn remove(&self, id: &str) {
        if let Ok(mut pending) = self.0.lock() {
            pending.remove(id);
        }
    }
}

/// Choice channel for `ask_user`: the user picks one of up to 4 options or
/// types a freeform answer. `None` means declined/dismissed.
#[derive(Default)]
pub struct ChoiceApprovals(Mutex<HashMap<String, oneshot::Sender<Option<String>>>>);
impl ChoiceApprovals {
    pub fn request_choice(&self) -> Result<(String, oneshot::Receiver<Option<String>>), String> {
        let id = uuid::Uuid::new_v4().to_string();
        let (sender, receiver) = oneshot::channel();
        self.0
            .lock()
            .map_err(|_| "Approval state unavailable.")?
            .insert(id.clone(), sender);
        Ok((id, receiver))
    }
    pub fn resolve_choice(&self, id: &str, choice: Option<String>) -> Result<(), String> {
        if let Some(choice) = &choice {
            if choice.len() > 4000 {
                return Err("Answer exceeds 4000 characters.".into());
            }
        }
        self.0
            .lock()
            .map_err(|_| "Approval state unavailable.")?
            .remove(id)
            .ok_or("This question is no longer pending.")?
            .send(choice)
            .map_err(|_| "The question has already ended.".into())
    }
    /// Pending request ids, for tests that resolve approvals without a UI.
    #[cfg(test)]
    pub(crate) fn pending_ids(&self) -> Vec<String> {
        self.0
            .lock()
            .map(|pending| pending.keys().cloned().collect())
            .unwrap_or_default()
    }
    pub fn remove(&self, id: &str) {
        if let Ok(mut pending) = self.0.lock() {
            pending.remove(id);
        }
    }
}
#[tauri::command]
pub fn resolve_tool_approval(
    state: tauri::State<'_, crate::AppState>,
    id: String,
    allow: bool,
) -> Result<(), String> {
    state.approvals.resolve(&id, allow)
}

#[tauri::command]
pub fn resolve_ask_user(
    state: tauri::State<'_, crate::AppState>,
    id: String,
    choice: Option<String>,
) -> Result<(), String> {
    state.ask_user.resolve_choice(&id, choice)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dropping_an_approval_scope_removes_the_pending_request() {
        let approvals = Approvals::default();
        let (id, _receiver) = approvals.request().unwrap();
        {
            let _pending = approvals.guard(&id);
        }
        assert!(approvals.resolve(&id, true).is_err());
    }
    #[tokio::test]
    async fn decisions_are_single_use_and_bound_to_request() {
        let approvals = Approvals::default();
        let (id, receiver) = approvals.request().unwrap();
        assert!(approvals.resolve("wrong-id", true).is_err());
        approvals.resolve(&id, false).unwrap();
        assert!(!receiver.await.unwrap());
        assert!(approvals.resolve(&id, true).is_err());
        let (id, receiver) = approvals.request().unwrap();
        approvals.remove(&id);
        assert!(receiver.await.is_err());
        assert!(approvals.resolve(&id, true).is_err());
    }
    #[tokio::test]
    async fn choice_channel_returns_the_picked_option() {
        let choices = ChoiceApprovals::default();
        let (id, receiver) = choices.request_choice().unwrap();
        choices.resolve_choice(&id, Some("second".into())).unwrap();
        assert_eq!(receiver.await.unwrap().as_deref(), Some("second"));
        assert!(choices.resolve_choice(&id, Some("again".into())).is_err());
        let (id, receiver) = choices.request_choice().unwrap();
        choices.resolve_choice(&id, None).unwrap();
        assert_eq!(receiver.await.unwrap(), None);
        assert!(choices.resolve_choice("missing", Some("x".into())).is_err());
    }
}
