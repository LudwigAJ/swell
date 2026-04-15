pub mod audit;
pub mod circuit_breaker;
pub mod config;
pub mod cost_tracking;
pub mod dependency_graph;
pub mod error;
pub mod events;
pub mod kill_switch;
pub mod langfuse;
pub mod opentelemetry;
pub mod trace_waterfall;
pub mod tracing_json;
pub mod traits;
pub mod treesitter;
pub mod types;

pub use audit::{
    verify_audit_chain, AuditEntry, AuditEventKind, AuditGate, AuditLog, AuditPlane,
    ChainVerificationResult, GENESIS_HASH,
};
pub use cost_tracking::{
    get_last_llm_model, get_total_llm_tokens, record_llm_cost, BudgetAlert, BudgetAlertType,
    CostBudget, CostRecord, CostSummary, CostTracker, CostTrackerError, ModelBreakdown,
    ModelCostInfo, TaskCostSummary, TaskOutcome,
};
pub use dependency_graph::{DependencyGraph, DependencyQuery, GraphStats, ImpactResult};
pub use error::SwellError;
pub use events::{
    AgentSessionId, CrossTaskCorrelationId, EventStore, ObservableEvent, Outcome, RequestId,
    SpanId, ToolInvocation, TraceId, TurnSummaryEvent,
};
pub use kill_switch::{
    EnvVarVerifier, FileVerifier, KillLevel, KillSwitchError, KillSwitchGuard, KillSwitchState,
    KillSwitchVerifier, RedisVerifier, ScopeBlock, ThrottleConfig,
};
pub use trace_waterfall::{
    SpanAttribute, SpanAttributeValue, SpanKind, ToTraceSpan, ToolSpanDetails, TraceSpan,
    TraceSummary, TraceWaterfall, TraceWaterfallBuilder,
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
    ToolOutput, ToolResultContent, ValidationArtifact, ValidationContext, ValidationGate,
    ValidationLevel, ValidationMessage, ValidationOutcome,
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
