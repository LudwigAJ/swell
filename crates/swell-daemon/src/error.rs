//! Error types for the swell-daemon crate.
//!
//! This module defines the structured error enum used across the daemon's public
//! boundary, replacing generic `anyhow::Error` with typed variants for better
//! error dispatch and testability.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use swell_core::TaskId;

// ============================================================================
// Sub-enums
// ============================================================================

/// Reason for validation failure in a task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ValidationReason {
    /// The task's execution spec did not match the frozen spec at validation time.
    FrozenSpecMismatch,
    /// One or more tests failed during validation.
    TestFailure,
    /// Multiple agents disagreed on the task outcome.
    MultiAgentDisagreement,
    /// Validation timed out.
    TimeoutExceeded,
    /// Other validation failure reason.
    Other(String),
}

impl std::fmt::Display for ValidationReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ValidationReason::FrozenSpecMismatch => write!(f, "frozen_spec_mismatch"),
            ValidationReason::TestFailure => write!(f, "test_failure"),
            ValidationReason::MultiAgentDisagreement => write!(f, "multi_agent_disagreement"),
            ValidationReason::TimeoutExceeded => write!(f, "timeout_exceeded"),
            ValidationReason::Other(s) => write!(f, "other:{}", s),
        }
    }
}

impl std::error::Error for ValidationReason {}

/// Budget class that was exceeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BudgetClass {
    /// Token budget exceeded.
    TokensExceeded,
    /// USD cost budget exceeded.
    UsdExceeded,
    /// Wall-clock time budget exceeded.
    WallClockExceeded,
}

impl std::fmt::Display for BudgetClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BudgetClass::TokensExceeded => write!(f, "tokens_exceeded"),
            BudgetClass::UsdExceeded => write!(f, "usd_exceeded"),
            BudgetClass::WallClockExceeded => write!(f, "wall_clock_exceeded"),
        }
    }
}

impl std::error::Error for BudgetClass {}

// ============================================================================
// Local error types (defined in swell-daemon for now)
// ============================================================================

/// Error type for worktree allocation failures.
#[derive(Debug, Error)]
pub enum WorktreeError {
    #[error("Worktree allocation failed: {0}")]
    AllocFailed(String),
    #[error("Worktree not found: {0}")]
    NotFound(String),
    #[error("Worktree already exists: {0}")]
    AlreadyExists(String),
    #[error("Worktree operation failed: {0}")]
    OperationFailed(String),
}

/// Error type for git operations.
#[derive(Debug, Error)]
pub enum GitError {
    #[error("Commit failed: {0}")]
    CommitFailed(String),
    #[error("Branch operation failed: {0}")]
    BranchFailed(String),
    #[error("Merge conflict: {0}")]
    MergeConflict(String),
    #[error("Invalid commit sha: {0}")]
    InvalidSha(String),
}

/// Configuration error type for daemon-level config errors.
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("Configuration error: {0}")]
    Message(String),
}

impl From<swell_core::SwellError> for ConfigError {
    fn from(err: swell_core::SwellError) -> Self {
        match err {
            swell_core::SwellError::ConfigError(msg) => ConfigError::Message(msg),
            other => ConfigError::Message(other.to_string()),
        }
    }
}

/// LLM error type for daemon-level LLM errors.
#[derive(Debug, Error)]
pub enum LlmError {
    #[error("LLM error: {0}")]
    Message(String),
}

impl From<swell_core::SwellError> for LlmError {
    fn from(err: swell_core::SwellError) -> Self {
        LlmError::Message(err.to_string())
    }
}

// ============================================================================
// DaemonError
// ============================================================================

/// Main error enum for the swell-daemon crate.
///
/// This enum represents all possible error conditions that can occur in the
/// daemon. Variants carry structured data to enable precise error dispatch
/// in callers (including wiring tests using `matches!()`).
///
/// # Variants
///
/// - [`TaskNotFound`](DaemonError::TaskNotFound) — Referenced task does not exist.
/// - [`ValidationFailed`](DaemonError::ValidationFailed) — Task failed validation gate.
/// - [`HookDenied`](DaemonError::HookDenied) — A hook rejected the requested operation.
/// - [`BudgetExceeded`](DaemonError::BudgetExceeded) — Task exceeded its resource budget.
/// - [`WorktreeAllocFailed`](DaemonError::WorktreeAllocFailed) — Worktree allocation failed.
/// - [`CommitFailed`](DaemonError::CommitFailed) — Git commit operation failed.
/// - [`Llm`](DaemonError::Llm) — LLM backend returned an error.
/// - [`Config`](DaemonError::Config) — Configuration error.
/// - [`ShuttingDown`](DaemonError::ShuttingDown) — Daemon is shutting down.
/// - [`Internal`](DaemonError::Internal) — Internal error with message.
#[derive(Debug, Error)]
pub enum DaemonError {
    #[error("Task {0} not found")]
    TaskNotFound(TaskId),

    #[error("Validation failed for task {task}: {reason}")]
    ValidationFailed {
        task: TaskId,
        reason: ValidationReason,
    },

    #[error("Hook denied: {hook} — {detail}")]
    HookDenied { hook: String, detail: String },

    #[error("Budget exceeded for task {task}: {class:?}")]
    BudgetExceeded { task: TaskId, class: BudgetClass },

    #[error("Worktree allocation failed: {0}")]
    WorktreeAllocFailed(#[from] WorktreeError),

    #[error("Commit failed for task {task}: {source}")]
    CommitFailed { task: TaskId, source: GitError },

    #[error("LLM error: {0}")]
    Llm(#[from] LlmError),

    #[error("Configuration error: {0}")]
    Config(#[from] ConfigError),

    #[error("Daemon is shutting down")]
    ShuttingDown,

    #[error("Internal error: {0}")]
    Internal(String),
}

// ============================================================================
// DaemonErrorWire
// ============================================================================

/// Wire-format error type for JSON serialization across the daemon boundary.
///
/// This enum mirrors [`DaemonError`] but serializes with a `kind` tag so clients
/// can dispatch on error type without parsing strings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DaemonErrorWire {
    TaskNotFound { task_id: String },
    ValidationFailed { task_id: String, reason: String },
    HookDenied { hook: String, detail: String },
    BudgetExceeded { task_id: String, class: String },
    WorktreeAllocFailed { message: String },
    CommitFailed { task_id: String, message: String },
    Llm { message: String },
    Config { message: String },
    ShuttingDown,
    Internal { message: String },
}

impl From<&DaemonError> for DaemonErrorWire {
    fn from(err: &DaemonError) -> Self {
        match err {
            DaemonError::TaskNotFound(task_id) => DaemonErrorWire::TaskNotFound {
                task_id: task_id.to_string(),
            },
            DaemonError::ValidationFailed { task, reason } => DaemonErrorWire::ValidationFailed {
                task_id: task.to_string(),
                reason: reason_to_string(reason),
            },
            DaemonError::HookDenied { hook, detail } => DaemonErrorWire::HookDenied {
                hook: hook.clone(),
                detail: detail.clone(),
            },
            DaemonError::BudgetExceeded { task, class } => DaemonErrorWire::BudgetExceeded {
                task_id: task.to_string(),
                class: budget_class_to_string(class),
            },
            DaemonError::WorktreeAllocFailed(err) => DaemonErrorWire::WorktreeAllocFailed {
                message: err.to_string(),
            },
            DaemonError::CommitFailed { task, source } => DaemonErrorWire::CommitFailed {
                task_id: task.to_string(),
                message: source.to_string(),
            },
            DaemonError::Llm(err) => DaemonErrorWire::Llm {
                message: err.to_string(),
            },
            DaemonError::Config(err) => DaemonErrorWire::Config {
                message: err.to_string(),
            },
            DaemonError::ShuttingDown => DaemonErrorWire::ShuttingDown,
            DaemonError::Internal(msg) => DaemonErrorWire::Internal {
                message: msg.clone(),
            },
        }
    }
}

fn reason_to_string(reason: &ValidationReason) -> String {
    match reason {
        ValidationReason::FrozenSpecMismatch => "frozen_spec_mismatch".to_string(),
        ValidationReason::TestFailure => "test_failure".to_string(),
        ValidationReason::MultiAgentDisagreement => "multi_agent_disagreement".to_string(),
        ValidationReason::TimeoutExceeded => "timeout_exceeded".to_string(),
        ValidationReason::Other(s) => format!("other:{}", s),
    }
}

fn budget_class_to_string(class: &BudgetClass) -> String {
    match class {
        BudgetClass::TokensExceeded => "tokens_exceeded".to_string(),
        BudgetClass::UsdExceeded => "usd_exceeded".to_string(),
        BudgetClass::WallClockExceeded => "wall_clock_exceeded".to_string(),
    }
}

// ============================================================================
// From conversions
// ============================================================================

// Note: We cannot implement `From<DaemonError> for anyhow::Error` because anyhow
// has a blanket impl `impl<E> From<E> for anyhow::Error where E: StdError`.
// However, DaemonError implements std::error::Error (via thiserror), so the blanket
// impl already provides From<DaemonError> for anyhow::Error. Users can use:
// `Err(daemon_error).map_err(|e| anyhow::Error::new(e))` or let the ? operator use the blanket impl.

impl From<anyhow::Error> for DaemonError {
    fn from(err: anyhow::Error) -> Self {
        DaemonError::Internal(err.to_string())
    }
}

impl From<swell_core::SwellError> for DaemonError {
    fn from(err: swell_core::SwellError) -> Self {
        match err {
            swell_core::SwellError::TaskNotFound(uuid) => {
                DaemonError::TaskNotFound(TaskId::from_uuid(uuid))
            }
            swell_core::SwellError::ConfigError(msg) => {
                DaemonError::Config(ConfigError::Message(msg))
            }
            swell_core::SwellError::LlmError(msg) => DaemonError::Llm(LlmError::Message(msg)),
            swell_core::SwellError::BudgetExceeded(msg) => {
                // Parse budget class from message — use Internal as fallback
                DaemonError::Internal(format!("Budget exceeded: {}", msg))
            }
            other => DaemonError::Internal(other.to_string()),
        }
    }
}
