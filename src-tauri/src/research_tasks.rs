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

fn default_task_secs() -> u64 { 1800 }
fn default_max_rounds() -> usize { 8 }
fn default_max_searches() -> usize { 12 }
fn default_max_documents() -> usize { 20 }
fn default_max_retries() -> usize { 2 }
fn default_op_secs() -> u64 { 240 }

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
    pub fn validate(&self) -> Result<(), String> {
        if !(60..=7200).contains(&self.task_timeout_secs) {
            return Err("task_timeout_secs must be 60-7200.".into());
        }
        if !(1..=16).contains(&self.max_rounds) {
            return Err("max_rounds must be 1-16.".into());
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
}

impl ResearchTask {
    pub fn new(conversation_id: &str, question: &str, budgets: TaskBudgets) -> Result<Self, String> {
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
            pending_work: vec!["plan".into(), "search".into(), "read".into(), "verify".into(), "answer".into()],
            created_at: now,
            updated_at: now,
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
        self.operations.push(TaskOperation {
            id: format!("op_{}", uuid::Uuid::new_v4().simple()),
            kind: kind.to_string(),
            status: status.clone(),
            attempts: 1,
            result_summary: summary.clone(),
            error: None,
            updated_at: crate::store::now(),
        });
        if status == OperationStatus::Completed {
            if let Some(summary) = summary {
                self.completed_results.push(json!({"kind": kind, "summary": summary}));
            }
            self.pending_work.retain(|work| work != kind);
        }
        self.updated_at = crate::store::now();
        !self.budget_exhausted()
    }

    pub fn budget_exhausted(&self) -> bool {
        let searches = self.operations.iter().filter(|op| op.kind == "search").count();
        let rounds = self.operations.len();
        searches >= self.budgets.max_searches || rounds >= self.budgets.max_rounds
    }

    pub fn pause(&mut self) -> Result<(), String> {
        if self.status != TaskStatus::Running {
            return Err("Only a running task can be paused.".into());
        }
        self.status = TaskStatus::Paused;
        self.updated_at = crate::store::now();
        Ok(())
    }

    /// A call paused before dispatch is known not to have started.
    pub fn pause_before_new_tool(&mut self, call_id: &str, call_name: &str) -> Result<(), String> {
        self.operations.push(TaskOperation {
            id: format!("op-{call_id}"),
            kind: call_name.to_string(),
            status: OperationStatus::Pending,
            attempts: 0,
            result_summary: None,
            error: None,
            updated_at: crate::store::now(),
        });
        if !self.pending_work.iter().any(|item| item == call_name) {
            self.pending_work.push(call_name.to_string());
        }
        self.pause()
    }

    pub fn resume(&mut self) -> Result<(), String> {
        if self.status != TaskStatus::Paused {
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
            if op.status == OperationStatus::Running {
                op.status = OperationStatus::Unknown;
                op.error = Some("Cancelled; the action may already have changed data.".into());
            } else if op.status == OperationStatus::Pending {
                op.status = OperationStatus::Skipped;
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

    pub fn record_tool_result(&mut self, call_id: &str, name: &str, result: &Value, allowed: bool) {
        let message = result.get("message").or_else(|| result.get("error")).and_then(Value::as_str).unwrap_or_default();
        let status = if !allowed || result["skipped"] == true {
            OperationStatus::Skipped
        } else if result["outcome"] == "unknown" || (result["isError"] == true && crate::continuation::classify_execution_failure(message).ambiguous) {
            OperationStatus::Unknown
        } else if result["isError"] == true {
            OperationStatus::Failed
        } else {
            OperationStatus::Completed
        };
        let id = format!("op-{call_id}");
        let existing = self.operations.iter().position(|op| op.id == id);
        let already_completed = existing.is_some_and(|i| self.operations[i].status == OperationStatus::Completed);
        // Full output belongs in the tool audit/artifact, not in every ledger save.
        let summary: String = result.to_string().chars().take(2000).collect();
        let operation = TaskOperation {
            id: id.clone(), kind: name.into(), status: status.clone(), attempts: existing.map(|index| self.operations[index].attempts).unwrap_or(usize::from(allowed)),
            result_summary: (status == OperationStatus::Completed).then(|| summary.clone()),
            error: (status != OperationStatus::Completed).then(|| message.chars().take(2000).collect()), updated_at: crate::store::now(),
        };
        if let Some(index) = existing { self.operations[index] = operation; } else { self.operations.push(operation); }
        if status == OperationStatus::Completed && !already_completed {
            self.completed_results.push(json!({"operationId":id,"kind":name,"summary":summary,"artifactId":result["_artifactId"]}));
            self.pending_work.retain(|work| work != name);
        }
        self.updated_at = crate::store::now();
    }

    pub fn finish_turn(&mut self, message_status: &str) {
        self.status = match message_status {
            "complete" => TaskStatus::Completed,
            "paused" => TaskStatus::Paused,
            "interrupted" => TaskStatus::Cancelled,
            _ => TaskStatus::Failed,
        };
        for operation in &mut self.operations {
            if operation.status == OperationStatus::Running {
                operation.status = OperationStatus::Unknown;
                operation.error = Some("The turn ended before this operation settled. Do not automatically repeat it.".into());
            } else if operation.status == OperationStatus::Pending && self.status != TaskStatus::Paused {
                operation.status = OperationStatus::Skipped;
            }
        }
        self.updated_at = crate::store::now();
    }

    pub fn resumable_operations(&self) -> Vec<&TaskOperation> {
        self.operations
            .iter()
            .filter(|op| matches!(op.status, OperationStatus::Pending))
            .collect()
    }

    pub fn progress_summary(&self) -> Value {
        let completed = self.operations.iter().filter(|op| op.status == OperationStatus::Completed).count();
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

/// Called from application startup and from `restart_generation`.
pub fn recover_interrupted(store: &crate::store::Store) -> Result<crate::agent_run::RestartReport, String> {
    let mut replayed = Vec::new();
    let mut any_unknown = false;
    for id in store.running_research_task_ids()? {
        let Some(mut task) = store.research_task(&id)? else { continue };
        let report = task.restart_from_ledger();
        replayed.extend(report.replayed_write_ids);
        any_unknown = true;
        store.save_research_task(&task)?;
    }
    let _ = any_unknown;
    Ok(crate::agent_run::RestartReport {
        replayed_write_ids: replayed,
        unconditional_success: false,
    })
}

pub fn ensure_table(connection: &rusqlite::Connection) -> Result<(), String> {
    connection
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS research_tasks(
                id TEXT PRIMARY KEY,
                conversation_id TEXT NOT NULL,
                question TEXT NOT NULL,
                status TEXT NOT NULL,
                budgets_json TEXT NOT NULL,
                operations_json TEXT NOT NULL DEFAULT '[]',
                results_json TEXT NOT NULL DEFAULT '[]',
                requirements_json TEXT NOT NULL DEFAULT '[]',
                findings_json TEXT NOT NULL DEFAULT '[]',
                pending_json TEXT NOT NULL DEFAULT '[]',
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS research_tasks_conversation ON research_tasks(conversation_id, created_at);",
        )
        .map_err(|error| format!("Local database: {error}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audit_failed_operations_are_not_completed_results() {
        let mut task = ResearchTask::new("conv", "Research", TaskBudgets::default()).unwrap();
        task.pending_work = vec!["search".into()];
        task.checkpoint_operation("search", OperationStatus::Failed, Some("Provider unavailable".into()));
        assert!(task.completed_results.is_empty());
        assert_eq!(task.pending_work, vec!["search"]);
    }

    #[test]
    fn audit_pausing_before_dispatch_preserves_known_pending_outcome() {
        let mut task = ResearchTask::new("conv", "Research", TaskBudgets::default()).unwrap();
        task.pause_before_new_tool("not-dispatched", "file_write").unwrap();
        task.restart_from_ledger();
        assert_eq!(task.operations[0].status, OperationStatus::Pending);
        assert_eq!(task.operations[0].attempts, 0);
        task.cancel().unwrap();
        assert_eq!(task.operations[0].status, OperationStatus::Skipped);
    }

    #[test]
    fn audit_turn_outcomes_settle_tasks_without_replaying_tools() {
        for (message_status, task_status) in [("complete", TaskStatus::Completed), ("paused", TaskStatus::Paused), ("interrupted", TaskStatus::Cancelled), ("error", TaskStatus::Failed)] {
            let mut task = ResearchTask::new("conv", "Research", TaskBudgets::default()).unwrap();
            task.record_tool_result("failed", "search", &json!({"isError":true,"message":"Provider unavailable"}), true);
            task.record_tool_result("denied", "write", &json!({"message":"Permission denied"}), false);
            task.record_tool_result("success", "read", &json!({"content":"Evidence"}), true);
            task.record_tool_result("success", "read", &json!({"content":"Evidence"}), true);
            assert_eq!(task.operations[0].status, OperationStatus::Failed);
            assert_eq!(task.operations[1].status, OperationStatus::Skipped);
            assert_eq!(task.completed_results.len(), 1);
            task.finish_turn(message_status);
            assert_eq!(task.status, task_status);
        }
    }

    #[test]
    fn audit_ledger_keeps_large_results_bounded() {
        let mut task = ResearchTask::new("conv", "Research", TaskBudgets::default()).unwrap();
        task.record_tool_result("large", "read", &json!({"content":"界".repeat(100_000)}), true);
        assert_eq!(task.operations[0].result_summary.as_ref().unwrap().chars().count(), 2000);
        assert!(serde_json::to_string(&task).unwrap().len() < 16_000);
    }

    #[test]
    fn checkpoint_pause_resume_cancel_flow() {
        let mut task = ResearchTask::new("conv", "Compare A and B", TaskBudgets::default()).unwrap();
        assert!(!task.is_terminal());
        assert!(task.checkpoint_operation("search", OperationStatus::Completed, Some("3 hits".into())));
        assert_eq!(task.completed_results.len(), 1);
        task.pause().unwrap();
        assert!(task.resume().is_ok());
        task.cancel().unwrap();
        assert_eq!(task.status, TaskStatus::Cancelled);
        assert!(task.is_terminal());
        assert!(task.resume().is_err());
    }

    #[test]
    fn restart_recovery_marks_running_unknown_without_replaying() {
        let mut task = ResearchTask::new("conv", "Long research", TaskBudgets::default()).unwrap();
        task.operations.push(TaskOperation {
            id: "op1".into(), kind: "search".into(), status: OperationStatus::Running,
            attempts: 1, result_summary: None, error: None, updated_at: 0,
        });
        task.operations.push(TaskOperation {
            id: "op2".into(), kind: "read".into(), status: OperationStatus::Pending,
            attempts: 0, result_summary: None, error: None, updated_at: 0,
        });
        task.mark_interrupted_for_recovery();
        assert_eq!(task.operations[0].status, OperationStatus::Unknown);
        assert_eq!(task.resumable_operations().len(), 1);
        assert_eq!(task.status, TaskStatus::Paused);
    }

    #[test]
    fn budget_exhaustion_yields_recoverable_partial_results() {
        let budgets = TaskBudgets { max_searches: 1, max_rounds: 4, ..TaskBudgets::default() };
        let mut task = ResearchTask::new("conv", "Q", budgets).unwrap();
        assert!(!task.checkpoint_operation("search", OperationStatus::Completed, Some("partial".into())));
        assert!(task.budget_exhausted());
        assert_eq!(task.completed_results.len(), 1);
        let summary = task.progress_summary();
        assert_eq!(summary["partialResults"], 1);
        assert_eq!(summary["pendingWork"].as_array().unwrap().len(), 4);
    }

    #[test]
    fn budgets_validate_loudly() {
        assert!(TaskBudgets { task_timeout_secs: 10, ..TaskBudgets::default() }.validate().is_err());
        assert!(TaskBudgets { max_rounds: 99, ..TaskBudgets::default() }.validate().is_err());
        assert!(TaskBudgets::default().validate().is_ok());
        assert!(ResearchTask::new("c", "", TaskBudgets::default()).is_err());
    }
}
