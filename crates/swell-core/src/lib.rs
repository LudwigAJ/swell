pub mod circuit_breaker;
pub mod error;
pub mod kill_switch;
pub mod traits;
pub mod types;

pub use error::SwellError;
pub use kill_switch::{
    EnvVarVerifier, FileVerifier, KillLevel, KillSwitchError, KillSwitchGuard, KillSwitchState,
    KillSwitchVerifier, ScopeBlock, ThrottleConfig,
};
pub use types::*;
// Explicitly re-export traits to avoid ambiguity with types::Agent and types::Tool
// Also include LlmRole, LlmToolCall, LlmUsage from traits
pub use traits::{
    AgentContext, AgentResult, Checkpoint, CheckpointStore, DynServiceContainer, Event,
    EventSubscriber, KgDirection, KgEdge, KgNode, KgNodeType, KgPath, KgRelation, KgTraversal,
    KnowledgeGraph, LlmBackend, LlmConfig, LlmMessage, LlmResponse, LlmRole, LlmToolCall,
    LlmToolDefinition, LlmUsage, MemoryEntry, MemoryQuery, MemorySearchResult, MemoryStore,
    Sandbox, SandboxCommand, SandboxOutput, ServiceContainer, ToolCallResult, ToolInput,
    ToolOutput, ValidationArtifact, ValidationContext, ValidationGate, ValidationLevel,
    ValidationMessage, ValidationOutcome,
};

use tracing_subscriber::{fmt, prelude::*, EnvFilter};

/// Initialize tracing/logging for the crate
pub fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(filter)
        .init();
}
