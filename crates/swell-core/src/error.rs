use thiserror::Error;

/// Main error type for the SWELL system.
#[derive(Error, Debug)]
pub enum SwellError {
    #[error("Task {0} not found")]
    TaskNotFound(uuid::Uuid),

    #[error("Agent {0} not found")]
    AgentNotFound(uuid::Uuid),

    #[error("Invalid state transition: {0}")]
    InvalidStateTransition(String),

    #[error("Tool execution failed: {0}")]
    ToolExecutionFailed(String),

    #[error("Sandbox error: {0}")]
    SandboxError(String),

    #[error("LLM error: {0}")]
    LlmError(String),

    #[error("Database error: {0}")]
    DatabaseError(String),

    #[error("Budget exceeded: {0}")]
    BudgetExceeded(String),

    #[error("Permission denied: {0}")]
    PermissionDenied(String),

    #[error("Doom loop detected")]
    DoomLoopDetected,

    #[error("Safety kill switch triggered")]
    KillSwitchTriggered,

    #[error("Resource limit exceeded: {0}")]
    ResourceLimitExceeded(String),

    #[error("Configuration error: {0}")]
    ConfigError(String),

    #[error("Invalid operation: {0}")]
    InvalidOperation(String),

    #[error("Task not traced to frozen spec: {0}")]
    TaskNotTracedToSpec(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Similar memory found: {0}")]
    SimilarMemoryFound(uuid::Uuid),

    #[error("Duplicate task: {1} (similarity: {0:.2})")]
    DuplicateTask(f32, uuid::Uuid),

    #[error("Duplicate task by file overlap: {1} ({0:.0}% overlap)")]
    DuplicateTaskByFileOverlap(f32, uuid::Uuid),
}

impl serde::Serialize for SwellError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}
