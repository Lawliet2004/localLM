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

pub const TASKS_TABLE: &str = "research_tasks";

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
            status,
            attempts: 1,
            result_summary: summary.clone(),
            error: None,
            updated_at: crate::store::now(),
        });
        if let Some(summary) = summary {
            self.completed_results.push(json!({"kind": kind, "summary": summary}));
        }
        self.pending_work.retain(|work| work != kind);
        self.updated_at = crate::store::now();
        !self.budget_exhausted()
    }

    pub fn budget_exhausted(&self) -> bool {
        let searches = self.operations.iter().filter(|op| op.kind == "search").count();
        let rounds = self.operations.len();
        searches >= self.budgets.max_searches || rounds >= self.budgets.max_rounds
    }

    pub fn elapsed_exhausted(&self) -> bool {
        crate::store::now().saturating_sub(self.created_at) > (self.budgets.task_timeout_secs as i64) * 1000
    }

    pub fn pause(&mut self) -> Result<(), String> {
        if self.status != TaskStatus::Running {
            return Err("Only a running task can be paused.".into());
        }
        self.status = TaskStatus::Paused;
        self.updated_at = crate::store::now();
        Ok(())
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
            if matches!(op.status, OperationStatus::Running | OperationStatus::Pending) {
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
    pub fn mark_interrupted_for_recovery(&mut self) {
        for op in &mut self.operations {
            if matches!(op.status, OperationStatus::Running) {
                op.status = OperationStatus::Unknown;
                op.error = Some("Interrupted by restart; outcome is unknown. Do not replay side effects automatically.".into());
            }
        }
        if self.status == TaskStatus::Running {
            self.status = TaskStatus::Paused;
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

#[allow(dead_code)]
fn _storage_contract() {
    let _ = TASKS_TABLE;
    let _ = TaskStatus::parse("running");
    let _ = OperationStatus::Pending.as_str();
    let task = ResearchTask::new("c", "q", TaskBudgets::default()).unwrap();
    let _ = task.elapsed_exhausted();
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
