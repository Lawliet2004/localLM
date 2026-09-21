use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    Preparing,
    Generating,
    AwaitingApproval,
    ExecutingTools,
    PreparingNextRound,
    Completed,
    Cancelled,
    Failed,
    OutcomeUnknown,
}

impl RunState {
    pub fn as_str(&self) -> &'static str {
        match self {
            RunState::Preparing => "preparing",
            RunState::Generating => "generating",
            RunState::AwaitingApproval => "awaiting_approval",
            RunState::ExecutingTools => "executing_tools",
            RunState::PreparingNextRound => "preparing_next_round",
            RunState::Completed => "completed",
            RunState::Cancelled => "cancelled",
            RunState::Failed => "failed",
            RunState::OutcomeUnknown => "outcome_unknown",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "preparing" => Some(RunState::Preparing),
            "generating" => Some(RunState::Generating),
            "awaiting_approval" => Some(RunState::AwaitingApproval),
            "executing_tools" => Some(RunState::ExecutingTools),
            "preparing_next_round" => Some(RunState::PreparingNextRound),
            "completed" => Some(RunState::Completed),
            "cancelled" => Some(RunState::Cancelled),
            "failed" => Some(RunState::Failed),
            "outcome_unknown" => Some(RunState::OutcomeUnknown),
            _ => None,
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            RunState::Completed | RunState::Cancelled | RunState::Failed | RunState::OutcomeUnknown
        )
    }

    pub fn can_transition_to(&self, next: RunState) -> bool {
        if self.is_terminal() {
            return false;
        }
        match (self, next) {
            // Cancellation or failure can happen from any active state
            (_, RunState::Cancelled) | (_, RunState::Failed) | (_, RunState::OutcomeUnknown) => true,
            (RunState::Preparing, RunState::Generating) => true,
            (RunState::Generating, RunState::Completed) => true,
            (RunState::Generating, RunState::AwaitingApproval) => true,
            (RunState::Generating, RunState::ExecutingTools) => true,
            (RunState::Generating, RunState::PreparingNextRound) => true,
            (RunState::AwaitingApproval, RunState::ExecutingTools) => true,
            (RunState::AwaitingApproval, RunState::PreparingNextRound) => true, // When user denies
            (RunState::ExecutingTools, RunState::ExecutingTools) => true, // Batch: multiple calls in one turn
            (RunState::ExecutingTools, RunState::AwaitingApproval) => true, // Batch: later call needs approval
            (RunState::ExecutingTools, RunState::PreparingNextRound) => true,
            // An in-loop `finish` returns straight to completion while the run
            // is still in ExecutingTools — without this edge the record stays
            // "executing_tools" forever and active_run reports it as live.
            (RunState::ExecutingTools, RunState::Completed) => true,
            (RunState::PreparingNextRound, RunState::Generating) => true,
            (RunState::PreparingNextRound, RunState::Completed) => true,
            _ => false,
        }
    }
}

impl fmt::Display for RunState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RunRecord {
    pub id: String,
    pub conversation_id: String,
    pub status: RunState,
    pub model_provider: Option<String>,
    pub model_id: Option<String>,
    pub checkpoint: Option<String>,
    pub error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl RunRecord {
    pub fn transition_to(&mut self, next: RunState) -> Result<(), String> {
        if !self.status.can_transition_to(next) {
            return Err(format!("Invalid run transition from {} to {}", self.status, next));
        }
        self.status = next;
        self.updated_at = crate::store::now();
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunEvent {
    pub run_id: String,
    pub seq: u64,
    pub step_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    pub event_type: String,
    pub payload: Value,
    pub created_at: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_transitions() {
        assert!(RunState::Preparing.can_transition_to(RunState::Generating));
        assert!(RunState::Generating.can_transition_to(RunState::AwaitingApproval));
        assert!(RunState::AwaitingApproval.can_transition_to(RunState::ExecutingTools));
        assert!(RunState::ExecutingTools.can_transition_to(RunState::ExecutingTools));
        assert!(RunState::ExecutingTools.can_transition_to(RunState::AwaitingApproval));
        assert!(RunState::ExecutingTools.can_transition_to(RunState::PreparingNextRound));
        assert!(RunState::ExecutingTools.can_transition_to(RunState::Completed));
        assert!(RunState::PreparingNextRound.can_transition_to(RunState::Generating));
        assert!(RunState::Generating.can_transition_to(RunState::Completed));
    }

    #[test]
    fn terminal_states_reject_transitions() {
        assert!(!RunState::Completed.can_transition_to(RunState::Generating));
        assert!(!RunState::Cancelled.can_transition_to(RunState::ExecutingTools));
        assert!(!RunState::Failed.can_transition_to(RunState::PreparingNextRound));
        assert!(!RunState::OutcomeUnknown.can_transition_to(RunState::Completed));
    }

    #[test]
    fn any_active_state_can_cancel_or_fail() {
        for state in [
            RunState::Preparing,
            RunState::Generating,
            RunState::AwaitingApproval,
            RunState::ExecutingTools,
            RunState::PreparingNextRound,
        ] {
            assert!(state.can_transition_to(RunState::Cancelled));
            assert!(state.can_transition_to(RunState::Failed));
            assert!(state.can_transition_to(RunState::OutcomeUnknown));
        }
    }

    #[test]
    fn roundtrip_state_strings() {
        for state in [
            RunState::Preparing,
            RunState::Generating,
            RunState::AwaitingApproval,
            RunState::ExecutingTools,
            RunState::PreparingNextRound,
            RunState::Completed,
            RunState::Cancelled,
            RunState::Failed,
            RunState::OutcomeUnknown,
        ] {
            assert_eq!(RunState::from_str(state.as_str()), Some(state));
        }
    }
}
