pub mod types;
pub mod error;
pub mod traits;

pub use types::*;
pub use error::SwellError;
// Explicitly re-export traits to avoid ambiguity with types::Agent and types::Tool
// Also include LlmRole, LlmToolCall, LlmUsage from traits
pub use traits::{
    LlmBackend, LlmMessage, LlmRole, LlmToolCall, LlmResponse, LlmUsage, LlmConfig, LlmToolDefinition,
    AgentContext, AgentResult, ToolCallResult, ToolInput, ToolOutput,
    MemoryEntry, MemoryQuery, MemorySearchResult,
    KgNode, KgNodeType, KgEdge, KgRelation, KgTraversal, KgDirection, KgPath,
    SandboxCommand, SandboxOutput, Checkpoint, CheckpointStore,
    ValidationContext, ValidationOutcome, ValidationMessage, ValidationLevel, ValidationArtifact,
    Event, EventSubscriber, ServiceContainer, DynServiceContainer,
    MemoryStore, KnowledgeGraph, Sandbox, ValidationGate,
};

use tracing_subscriber::{fmt, prelude::*, EnvFilter};

/// Initialize tracing/logging for the crate
pub fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(filter)
        .init();
}
