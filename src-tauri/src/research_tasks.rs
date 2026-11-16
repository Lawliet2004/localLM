//! Durable long-running research tasks: checkpoint, pause, resume, cancel,
//! restart recovery, budgets, and partial results.
//!
//! A task checkpoints after every meaningful operation (each completed
//! search/fetch/verify round). Pause/resume/cancel are explicit; restart
//! recovery replays only pending operations from durable state — completed
//! side effects are never replayed. Interrupted operations with uncertain
//! outcomes are marked `unknown` explicitly. Budget exhaustion yields
//! recoverable partial results, never silent success.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};


#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Running,
    Paused,
    Completed,
    Cancelled,
    Failed,
    BudgetExhausted,
}

impl TaskStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            TaskStatus::Running => "running",
            TaskStatus::Paused => "paused",
            TaskStatus::Completed => "completed",
            TaskStatus::Cancelled => "cancelled",
            TaskStatus::Failed => "failed",
            TaskStatus::BudgetExhausted => "budget_exhausted",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "running" => Some(TaskStatus::Running),
            "paused" => Some(TaskStatus::Paused),
            "completed" => Some(TaskStatus::Completed),
            "cancelled" => Some(TaskStatus::Cancelled),
            "failed" => Some(TaskStatus::Failed),
            "budget_exhausted" => Some(TaskStatus::BudgetExhausted),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OperationStatus {
    Pending,
    Running,
    Completed,
    Failed,
    /// Interrupted with an uncertain outcome: must NOT be auto-replayed.
    Unknown,
    Skipped,
}

impl OperationStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            OperationStatus::Pending => "pending",
            OperationStatus::Running => "running",
            OperationStatus::Completed => "completed",
            OperationStatus::Failed => "failed",
            OperationStatus::Unknown => "unknown",
            OperationStatus::Skipped => "skipped",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskBudgets {
    #[serde(default = "default_task_secs")]
    pub task_timeout_secs: u64,
    #[serde(default = "default_max_rounds")]
    pub max_rounds: usize,
    #[serde(default = "default_max_searches")]
    pub max_searches: usize,
    #[serde(default = "default_max_documents")]
    pub max_documents: usize,
    #[serde(default = "default_max_retries")]
    pub max_retries: usize,
    #[serde(default = "default_op_secs")]
    pub operation_timeout_secs: u64,
}

fn default_task_secs() -> u64 {
    1800
}
fn default_max_rounds() -> usize {
    8
}
fn default_max_searches() -> usize {
    12
}
fn default_max_documents() -> usize {
    20
}
fn default_max_retries() -> usize {
    2
}
fn default_op_secs() -> u64 {
    240
}

impl Default for TaskBudgets {
    fn default() -> Self {
        Self {
            task_timeout_secs: default_task_secs(),
            max_rounds: default_max_rounds(),
            max_searches: default_max_searches(),
            max_documents: default_max_documents(),
            max_retries: default_max_retries(),
            operation_timeout_secs: default_op_secs(),
        }
    }
}

impl TaskBudgets {
    /// Parent runs persist the same limits that their dispatch loop enforces.
    pub(crate) fn for_parent() -> Self {
        let turn = crate::continuation::Budgets::default();
        Self {
            max_rounds: turn.max_rounds as usize,
            max_searches: turn.max_searches as usize,
            max_documents: turn.max_fetches as usize,
            max_retries: turn.max_retries as usize,
            operation_timeout_secs: 300,
            ..Self::default()
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if !(60..=7200).contains(&self.task_timeout_secs) {
            return Err("task_timeout_secs must be 60-7200.".into());
        }
        if !(1..=64).contains(&self.max_rounds) {
            return Err("max_rounds must be 1-64.".into());
        }
        if !(1..=50).contains(&self.max_searches) {
            return Err("max_searches must be 1-50.".into());
        }
        if !(1..=100).contains(&self.max_documents) {
            return Err("max_documents must be 1-100.".into());
        }
        if self.max_retries > 5 {
            return Err("max_retries must be 0-5.".into());
        }
        if !(10..=600).contains(&self.operation_timeout_secs) {
            return Err("operation_timeout_secs must be 10-600.".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskOperation {
    pub id: String,
    pub kind: String,
    pub status: OperationStatus,
    pub attempts: usize,
    pub result_summary: Option<String>,
    pub error: Option<String>,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResearchTask {
    pub id: String,
    pub conversation_id: String,
    pub question: String,
    pub status: TaskStatus,
    pub budgets: TaskBudgets,
    pub operations: Vec<TaskOperation>,
    pub completed_results: Vec<Value>,
    pub requirements: Vec<Value>,
    pub findings: Vec<Value>,
    pub pending_work: Vec<String>,
    pub created_at: i64,
    pub updated_at: i64,
    #[serde(skip)]
    pub(crate) context: TaskContext,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct TaskContext {
    pub workspace: Option<crate::snapshots::WorkspacePin>,
    pub goal: Option<crate::task_goal::Goal>,
    pub verification: crate::verification_progress::Progress,
}

impl ResearchTask {
    pub fn new(
        conversation_id: &str,
        question: &str,
        budgets: TaskBudgets,
    ) -> Result<Self, String> {
        if question.trim().is_empty() || question.len() > 8000 {
            return Err("Task question must contain 1-8000 bytes.".into());
        }
        budgets.validate()?;
        let now = crate::store::now();
        Ok(Self {
            id: format!("task_{}", uuid::Uuid::new_v4().simple()),
            conversation_id: conversation_id.to_string(),
            question: question.trim().to_string(),
            status: TaskStatus::Running,
            budgets,
            operations: Vec::new(),
            completed_results: Vec::new(),
            requirements: Vec::new(),
            findings: Vec::new(),
            pending_work: vec![
                "plan".into(),
                "search".into(),
                "read".into(),
                "verify".into(),
                "answer".into(),
            ],
            created_at: now,
            updated_at: now,
            context: TaskContext::default(),
        })
    }

    pub fn is_terminal(&self) -> bool {
        !matches!(self.status, TaskStatus::Running | TaskStatus::Paused)
    }

    /// Record a checkpoint after a meaningful operation. Completed results
    /// are appended; pending work advances. Returns false when a budget is
    /// exhausted (task moves to `budget_exhausted` with partial results).
    pub fn checkpoint_operation(
        &mut self,
        kind: &str,
        status: OperationStatus,
        summary: Option<String>,
    ) -> bool {
        let completed = status == OperationStatus::Completed;
        self.operations.push(TaskOperation {
            id: format!("op_{}", uuid::Uuid::new_v4().simple()),
            kind: kind.to_string(),
            status,
            attempts: 1,
            result_summary: summary.clone(),
            error: None,
            updated_at: crate::store::now(),
        });
        if let Some(summary) = summary.filter(|_| completed) {
            self.completed_results
                .push(json!({"kind": kind, "summary": summary}));
        }
        if completed {
            self.pending_work.retain(|work| work != kind);
        }
        self.updated_at = crate::store::now();
        !self.budget_exhausted()
    }

    /// Search budget only. Model rounds are the turn loop's `Budgets`, not the
    /// operation count — counting every tool call as a round exhausted the
    /// ledger while the loop was still running.
    pub fn budget_exhausted(&self) -> bool {
        self.operations
            .iter()
            .filter(|op| op.kind == "search")
            .count()
            >= self.budgets.max_searches
    }

    pub fn elapsed_exhausted(&self) -> bool {
        crate::store::now().saturating_sub(self.created_at)
            > (self.budgets.task_timeout_secs as i64) * 1000
    }

    pub fn pause(&mut self) -> Result<(), String> {
        if self.status != TaskStatus::Running {
            return Err("Only a running task can be paused.".into());
        }
        self.status = TaskStatus::Paused;
        self.updated_at = crate::store::now();
        Ok(())
    }

    /// Pause before dispatch. The next segment can propose this work again.
    pub fn pause_before_new_tool(&mut self, call_name: &str) -> Result<(), String> {
        self.pause()?;
        let pending = format!("Review {call_name}, which was paused before dispatch.");
        if !self.pending_work.contains(&pending) {
            self.pending_work.push(pending);
        }
        Ok(())
    }

    pub fn resume(&mut self) -> Result<(), String> {
        if !matches!(self.status, TaskStatus::Paused | TaskStatus::BudgetExhausted) {
            return Err("Only a paused task can be resumed.".into());
        }
        self.status = TaskStatus::Running;
        self.updated_at = crate::store::now();
        Ok(())
    }

    /// Cancel: running operations become `unknown` (uncertain outcome, never
    /// auto-replayed); pending operations are skipped.
    pub fn cancel(&mut self) -> Result<(), String> {
        if self.is_terminal() {
            return Err("Task already reached a terminal state.".into());
        }
        for op in &mut self.operations {
            if matches!(
                op.status,
                OperationStatus::Running | OperationStatus::Pending
            ) {
                op.status = OperationStatus::Unknown;
                op.error = Some("Cancelled; the action may already have changed data.".into());
            }
        }
        self.status = TaskStatus::Cancelled;
        self.updated_at = crate::store::now();
        Ok(())
    }

    /// Restart recovery: `running` operations from a previous process become
    /// `unknown` (never assumed complete, never auto-replayed); `pending`
    /// operations stay resumable.
    /// Restart uses the shared ledger: completed work stays, in-flight work
    /// becomes outcome-unknown, and nothing is replayed.
    pub fn restart_from_ledger(&mut self) -> crate::agent_run::RestartReport {
        let mut ops: Vec<crate::agent_run::LedgerOp> = self
            .operations
            .iter()
            .map(|op| crate::agent_run::LedgerOp {
                id: op.id.clone(),
                kind: op.kind.clone(),
                status: match op.status {
                    OperationStatus::Completed => "succeeded",
                    OperationStatus::Running => "running",
                    OperationStatus::Pending => "planned",
                    OperationStatus::Failed => "failed",
                    OperationStatus::Unknown => "outcome_unknown",
                    OperationStatus::Skipped => "skipped",
                }
                .into(),
            })
            .collect();
        let report = crate::agent_run::restart_operations(&mut ops);
        for (op, ledger) in self.operations.iter_mut().zip(ops.iter()) {
            if ledger.status == "outcome_unknown" {
                op.status = OperationStatus::Unknown;
                op.error = Some("Interrupted by restart; outcome is unknown. Do not replay side effects automatically.".into());
                op.updated_at = crate::store::now();
            }
        }
        if self.status == TaskStatus::Running {
            self.status = TaskStatus::Paused;
        }
        self.updated_at = crate::store::now();
        report
    }

    pub fn mark_interrupted_for_recovery(&mut self) {
        let _ = self.restart_from_ledger();
    }

    pub fn resumable_operations(&self) -> Vec<&TaskOperation> {
        self.operations
            .iter()
            .filter(|op| matches!(op.status, OperationStatus::Pending))
            .collect()
    }

    pub fn progress_summary(&self) -> Value {
        let completed = self
            .operations
            .iter()
            .filter(|op| op.status == OperationStatus::Completed)
            .count();
        json!({
            "taskId": self.id,
            "status": self.status.as_str(),
            "completedOperations": completed,
            "totalOperations": self.operations.len(),
            "pendingWork": self.pending_work,
            "partialResults": self.completed_results.len(),
        })
    }
}

/// Settle the ledger from the authoritative run outcome, not answer delivery.
pub(crate) fn settle_turn(
    store: &crate::store::Store,
    id: &str,
    state: crate::agent_run::RunState,
    budget_exhausted: bool,
    mut pending: Vec<String>,
) -> Result<(), String> {
    use crate::agent_run::RunState;
    let Some(mut task) = store.research_task(id)? else { return Ok(()); };
    if task.status == TaskStatus::Cancelled { return Ok(()); }
    if state == RunState::Paused && task.status == TaskStatus::Paused {
        for item in &task.pending_work {
            if !pending.contains(item) {
                pending.push(item.clone());
            }
        }
    }
    for operation in &task.operations {
        if matches!(operation.status, OperationStatus::Running | OperationStatus::Unknown) {
            pending.push(format!("Review operation {} ({}) before retrying; its outcome may be unknown.", operation.id, operation.kind));
        }
    }
    task.status = match state {
        RunState::Completed | RunState::Paused if budget_exhausted => TaskStatus::BudgetExhausted,
        RunState::Completed if pending.is_empty() => TaskStatus::Completed,
        RunState::Completed | RunState::Paused => TaskStatus::Paused,
        RunState::Cancelled => TaskStatus::Cancelled,
        RunState::Failed | RunState::OutcomeUnknown if task.context.goal.is_some() => TaskStatus::Paused,
        RunState::Failed | RunState::OutcomeUnknown => TaskStatus::Failed,
        _ => return Err("Cannot settle research from a live run.".into()),
    };
    task.pending_work = pending;
    task.updated_at = crate::store::now();
    store.save_research_task(&task)
}

#[allow(dead_code)]
/// Called from application startup and from `restart_generation`.
pub fn recover_interrupted(
    store: &crate::store::Store,
) -> Result<crate::agent_run::RestartReport, String> {
    let mut replayed = Vec::new();
    for id in store.running_research_task_ids()? {
        let Some(mut task) = store.research_task(&id)? else {
            continue;
        };
        let run = store.research_task_run(&task)?;
        let start = run.as_ref().map(|run| store.run_session_event_payload(&run.id, "turn_start")).transpose()?.flatten();
        let end = run.as_ref().map(|run| store.run_session_event_payload(&run.id, "turn_end")).transpose()?.flatten();
        if let Some(run) = &run {
            use crate::agent_run::RunState;
            if matches!(run.status, RunState::Completed | RunState::Failed) && (start.is_none() || end.is_some()) {
                let mut pending = Vec::new();
                if run.status == RunState::Completed {
                    pending.extend(store.todos(&task.conversation_id)?.into_iter().filter(|todo| todo.status != crate::plans::COMPLETED).map(|todo| todo.text));
                    pending.extend(store.tasks(&task.conversation_id)?.into_iter().filter(|task| !matches!(task.state.as_str(), crate::tasks::COMPLETED | crate::tasks::CANCELLED)).map(|task| task.title));
                    if let Some(items) = end.as_ref().and_then(|event| event["pendingWork"].as_array()) {
                        pending.extend(items.iter().filter_map(Value::as_str).map(str::to_owned));
                    }
                    if end.as_ref().is_some_and(|event| event["partial"] == true) && pending.is_empty() {
                        pending.push("Review the partial result and finish the remaining work.".into());
                    }
                }
                settle_turn(store, &id, run.status, false, pending)?;
                continue;
            }
        }
        let report = task.restart_from_ledger();
        replayed.extend(report.replayed_write_ids);
        let tx = store.connection.unchecked_transaction().map_err(|error| error.to_string())?;
        if let (Some(run), Some(start), None) = (run, start, end) {
            if task.pending_work.is_empty() {
                task.pending_work.push("Review the recorded work and resume from the saved checkpoint.".into());
            }
            let unresolved: Vec<_> = task.operations.iter().filter(|op| op.status == OperationStatus::Unknown).collect();
            let error = "The previous run was interrupted. Review recorded outcomes before continuing.";
            store.update_run_status(&run.id, crate::agent_run::RunState::Paused, Some(error), None)?;
            store.append_session_event(&crate::sessions::SessionEvent {
                id: uuid::Uuid::new_v4().to_string(), conversation_id: task.conversation_id.clone(),
                run_id: Some(run.id), seq: 0, step_id: None, tool_call_id: None,
                event_type: "turn_end".into(), ignorable: false, created_at: crate::store::now(),
                payload: json!({"status":"interrupted","stopReason":"restart","messageId":start["messageId"],"partial":true,
                    "delivered":false,"taskId":task.id,"taskStatus":"paused","resumeTaskId":task.id,"pendingWork":task.pending_work,
                    "unresolvedOperations":unresolved,"error":error}),
            })?;
        }
        store.save_research_task(&task)?;
        tx.commit().map_err(|error| error.to_string())?;
    }
    Ok(crate::agent_run::RestartReport {
        replayed_write_ids: replayed,
        unconditional_success: false,
    })
}

/// The research-task table and its index are created by the store's schema
/// batch, so every connection already has them.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_recovers_a_legacy_run_marked_complete_without_its_terminal_journal() {
        use crate::agent_run::{RunRecord, RunState};
        let store = crate::store::Store::open_memory().unwrap();
        let owner = store.create_conversation().unwrap().id;
        let assistant = store.begin_turn(&owner, "Ship the feature").unwrap();
        let run = RunRecord { id:"legacy-early-finish".into(), conversation_id:owner.clone(),
            status:RunState::Completed, model_provider:None, model_id:None, checkpoint:None,
            error:None, created_at:1, updated_at:2 };
        store.save_run(&run).unwrap();
        let task = ResearchTask::new(&owner, "Ship the feature", TaskBudgets::default()).unwrap();
        store.start_research_task(&task, &run.id).unwrap();
        store.append_session_event(&crate::sessions::SessionEvent {
            id:"start".into(), conversation_id:owner.clone(), run_id:Some(run.id.clone()), seq:0,
            step_id:None, tool_call_id:None, event_type:"turn_start".into(),
            payload:json!({"messageId":assistant.id}), ignorable:false, created_at:1,
        }).unwrap();
        store.recover_interrupted().unwrap();
        recover_interrupted(&store).unwrap();
        recover_interrupted(&store).unwrap();
        assert_eq!(store.latest_run(&owner).unwrap().unwrap().status, RunState::Paused);
        assert_eq!(store.research_task(&task.id).unwrap().unwrap().status, TaskStatus::Paused);
        let events = store.session_events(&owner, None, None).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[1].payload["resumeTaskId"], task.id);
        assert_eq!(events[1].payload["stopReason"], "restart");
    }

    #[test]
    fn checkpoint_pause_resume_cancel_flow() {
        let mut task =
            ResearchTask::new("conv", "Compare A and B", TaskBudgets::default()).unwrap();
        assert!(!task.is_terminal());
        assert!(task.checkpoint_operation(
            "search",
            OperationStatus::Completed,
            Some("3 hits".into())
        ));
        assert_eq!(task.completed_results.len(), 1);
        task.pause().unwrap();
        assert!(task.resume().is_ok());
        task.cancel().unwrap();
        assert_eq!(task.status, TaskStatus::Cancelled);
        assert!(task.is_terminal());
        assert!(task.resume().is_err());
    }

    #[test]
    fn pause_before_dispatch_keeps_pending_work_without_an_uncertain_operation() {
        let store = crate::store::Store::open_memory().unwrap();
        let owner = store.create_conversation().unwrap().id;
        let mut task = ResearchTask::new(&owner, "Save a file", TaskBudgets::default()).unwrap();
        task.pause_before_new_tool("workspace_create_file").unwrap();
        store.save_research_task(&task).unwrap();
        settle_turn(&store, &task.id, crate::agent_run::RunState::Paused, false, vec![]).unwrap();
        recover_interrupted(&store).unwrap();
        let task = store.research_task(&task.id).unwrap().unwrap();
        assert_eq!(task.status, TaskStatus::Paused);
        assert!(task.operations.is_empty());
        assert!(task.pending_work.iter().any(|work| work.contains("workspace_create_file") && work.contains("before dispatch")));
    }

    #[test]
    fn restart_recovery_marks_running_unknown_without_replaying() {
        let mut task = ResearchTask::new("conv", "Long research", TaskBudgets::default()).unwrap();
        task.operations.push(TaskOperation {
            id: "op1".into(),
            kind: "search".into(),
            status: OperationStatus::Running,
            attempts: 1,
            result_summary: None,
            error: None,
            updated_at: 0,
        });
        task.operations.push(TaskOperation {
            id: "op2".into(),
            kind: "read".into(),
            status: OperationStatus::Pending,
            attempts: 0,
            result_summary: None,
            error: None,
            updated_at: 0,
        });
        task.mark_interrupted_for_recovery();
        assert_eq!(task.operations[0].status, OperationStatus::Unknown);
        assert_eq!(task.resumable_operations().len(), 1);
        assert_eq!(task.status, TaskStatus::Paused);
    }

    #[test]
    fn budget_exhaustion_yields_recoverable_partial_results() {
        let budgets = TaskBudgets {
            max_searches: 1,
            max_rounds: 4,
            ..TaskBudgets::default()
        };
        let mut task = ResearchTask::new("conv", "Q", budgets).unwrap();
        assert!(!task.checkpoint_operation(
            "search",
            OperationStatus::Completed,
            Some("partial".into())
        ));
        assert!(task.budget_exhausted());
        let mut reads = ResearchTask::new("conv", "Q", TaskBudgets { max_rounds: 2, ..TaskBudgets::default() }).unwrap();
        assert!(reads.checkpoint_operation("workspace_read_file", OperationStatus::Completed, None));
        assert!(reads.checkpoint_operation("shell_run", OperationStatus::Completed, None));
        assert!(!reads.budget_exhausted(), "tool operations are not model rounds");
        assert_eq!(task.completed_results.len(), 1);
        let summary = task.progress_summary();
        assert_eq!(summary["partialResults"], 1);
        assert_eq!(summary["pendingWork"].as_array().unwrap().len(), 4);
    }

    #[test]
    fn budgets_validate_loudly() {
        assert!(TaskBudgets {
            task_timeout_secs: 10,
            ..TaskBudgets::default()
        }
        .validate()
        .is_err());
        assert!(TaskBudgets {
            max_rounds: 99,
            ..TaskBudgets::default()
        }
        .validate()
        .is_err());
        assert!(TaskBudgets::default().validate().is_ok());
        assert!(ResearchTask::new("c", "", TaskBudgets::default()).is_err());
    }
}
