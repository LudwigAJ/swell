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

    #[error("LLM API error ({status} {kind:?}): {message}")]
    LlmApiError {
        kind: LlmErrorKind,
        status: u16,
        request_id: Option<String>,
        message: String,
    },

    #[error("Database error: {0}")]
    DatabaseError(String),

    #[error("Budget exceeded: {0}")]
    BudgetExceeded(String),

    #[error("Permission denied: {0}")]
    PermissionDenied(String),

    #[error("Doom loop detected")]
    DoomLoopDetected,

    #[error("Loop detected ({pattern}): {reason}")]
    LoopDetected { reason: String, pattern: String },

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

/// Categorical kinds of LLM API errors. Mirrors `anthropic_client::ApiErrorKind`
/// and is used to make routing/retry decisions without inspecting strings.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum LlmErrorKind {
    InvalidRequest,
    Authentication,
    Permission,
    NotFound,
    Conflict,
    UnprocessableEntity,
    RateLimit,
    InternalServer,
    Overloaded,
    Unknown(String),
}

impl LlmErrorKind {
    /// Whether the failure is worth retrying or falling back to another model.
    /// Auth, invalid-request, and not-found are terminal regardless of retries.
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::RateLimit | Self::Overloaded | Self::InternalServer | Self::Conflict
        )
    }

    /// Whether the failure is permanent for this request and should not be
    /// retried by ANY backend in a fallback chain (e.g. wrong API key, bad request).
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Authentication
                | Self::Permission
                | Self::InvalidRequest
                | Self::UnprocessableEntity
                | Self::NotFound
        )
    }

    /// Map an HTTP status code into a kind. Used by non-Anthropic backends
    /// (OpenAI, gateways) that surface raw HTTP rather than typed enums.
    pub fn from_http_status(status: u16) -> Self {
        match status {
            400 => Self::InvalidRequest,
            401 => Self::Authentication,
            403 => Self::Permission,
            404 => Self::NotFound,
            409 => Self::Conflict,
            422 => Self::UnprocessableEntity,
            429 => Self::RateLimit,
            529 => Self::Overloaded,
            500..=599 => Self::InternalServer,
            other => Self::Unknown(format!("http_{other}")),
        }
    }
}

impl serde::Serialize for SwellError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}
