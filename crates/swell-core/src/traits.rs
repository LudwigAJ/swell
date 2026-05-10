//! Core traits and protocols for the SWELL autonomous coding engine.
//!
//! This module defines the foundational abstractions that allow
//! the system to be modular and testable.
//!
//! # Architecture
//!
//! The engine is built on several key abstractions:
//!
//! - [`LlmBackend`] - LLM provider abstraction (Anthropic, OpenAI, etc.)
//! - [`Agent`] - Agent behavior (Planner, Generator, Evaluator)
//! - [`Tool`] - Tool execution (file ops, git, shell, etc.)
//! - [`MemoryStore`] - Persistent memory (vector, recall, KG)
//! - [`Sandbox`] - Isolated execution environment
//! - [`CheckpointStore`] - State persistence
//! - [`ValidationGate`] - Quality assurance steps

use crate::{
    ids::SessionId, AgentId, AgentRole, Plan, StreamEvent, SwellError, Task, TaskId, TaskState,
    TurnSummaryEvent,
};
use async_trait::async_trait;
use futures::Stream;
use serde::{Deserialize, Serialize};
use std::any::Any;
use std::pin::Pin;
use uuid::Uuid;

// ============================================================================
// LLM Backend Protocol
// ============================================================================

/// A message in an LLM conversation.
///
/// `content` carries the text portion. `tool_calls`, when present on an
/// assistant message, carries the tool_use blocks that need to be echoed
/// back to the API on the next turn (Anthropic requires the prior
/// assistant turn's tool_use blocks when responding with tool_result).
/// `tool_call_id`, when present on a user message, marks that message as
/// a tool_result for the named tool_use id.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LlmMessage {
    pub role: LlmRole,
    pub content: String,
    /// Tool calls emitted by the assistant on this turn. Required when
    /// echoing the assistant turn back to the API in a multi-turn tool
    /// loop — Anthropic rejects tool_result messages whose preceding
    /// assistant turn lacks the matching tool_use block.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<LlmToolCall>>,
    /// When present, this message is a tool_result for the named
    /// tool_use id. `content` carries the result body as text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// Mark a tool_result as an error so the model can react to failure.
    /// Ignored on non-tool_result messages. Routes to the SDK's
    /// `tool_result_error` block on Anthropic.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub tool_result_is_error: bool,
    /// Thinking blocks to echo back as part of an assistant turn. Required
    /// for multi-turn tool-call flows on providers that bind reasoning to
    /// signatures (MiniMax's Anthropic-compatible endpoint). Empty otherwise.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub thinking_blocks: Vec<LlmThinkingBlock>,
}

impl Default for LlmMessage {
    fn default() -> Self {
        Self {
            role: LlmRole::User,
            content: String::new(),
            tool_calls: None,
            tool_call_id: None,
            tool_result_is_error: false,
            thinking_blocks: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum LlmRole {
    System,
    User,
    Assistant,
}

// Re-export LlmToolCall from types for convenience
pub use crate::LlmToolCall;

/// One Anthropic-style thinking block carried alongside a message turn.
///
/// MiniMax's Anthropic-compatible endpoint requires the *full* assistant
/// response — including thinking blocks with their signatures — to be
/// echoed back in the next turn's history when tool calls are involved,
/// or the reasoning chain breaks. We carry the block verbatim so we can
/// round-trip it on the request side via `ContentBlockParam::thinking`
/// or `thinking_with_signature`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LlmThinkingBlock {
    pub thinking: String,
    /// Some providers (Anthropic itself, MiniMax in some setups) attach a
    /// signature; others omit it. Echo back exactly what we received.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
}

/// Why the model stopped generating. Mirrors `anthropic_client::StopReason`
/// with `Other(String)` for forward-compatible round-tripping.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum LlmStopReason {
    EndTurn,
    MaxTokens,
    StopSequence,
    ToolUse,
    PauseTurn,
    Refusal,
    Other(String),
}

impl LlmStopReason {
    pub fn as_str(&self) -> &str {
        match self {
            Self::EndTurn => "end_turn",
            Self::MaxTokens => "max_tokens",
            Self::StopSequence => "stop_sequence",
            Self::ToolUse => "tool_use",
            Self::PauseTurn => "pause_turn",
            Self::Refusal => "refusal",
            Self::Other(s) => s.as_str(),
        }
    }

    pub fn from_wire(s: &str) -> Self {
        match s {
            "end_turn" => Self::EndTurn,
            "max_tokens" => Self::MaxTokens,
            "stop_sequence" => Self::StopSequence,
            "tool_use" => Self::ToolUse,
            "pause_turn" => Self::PauseTurn,
            "refusal" => Self::Refusal,
            other => Self::Other(other.to_string()),
        }
    }
}

/// Response from an LLM
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct LlmResponse {
    pub content: String,
    pub tool_calls: Option<Vec<LlmToolCall>>,
    pub usage: LlmUsage,
    /// Why the model stopped generating, when reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<LlmStopReason>,
    /// Extended-thinking text (Anthropic only). Concatenated across thinking
    /// blocks for cheap inspection. Empty/None when thinking was not enabled
    /// or not returned. For round-tripping back into a follow-up turn, use
    /// `thinking_blocks` (which preserves per-block signatures).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<String>,
    /// Typed thinking blocks with optional signatures. Required to echo the
    /// assistant turn back to providers that bind reasoning to signatures
    /// (notably MiniMax's Anthropic-compatible endpoint).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub thinking_blocks: Vec<LlmThinkingBlock>,
}

/// Token usage statistics (four-dimensional for Anthropic cache support).
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct LlmUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    /// Tokens written to provider-managed cache (Anthropic)
    #[serde(default)]
    pub cache_creation_input_tokens: Option<u64>,
    /// Tokens read from provider-managed cache (Anthropic)
    #[serde(default)]
    pub cache_read_input_tokens: Option<u64>,
    /// Cache writes attributed to the 5-minute ephemeral TTL, when reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_creation_5m_input_tokens: Option<u64>,
    /// Cache writes attributed to the 1-hour TTL, when reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_creation_1h_input_tokens: Option<u64>,
    /// Server-side tool invocations (web search + code exec), when reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_tool_use_count: Option<u64>,
    /// Service tier the request was billed under (e.g. `"standard"`,
    /// `"priority"`, `"batch"`), when reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<String>,
}

/// Cache TTL hint for a system block.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LlmCacheTtl {
    /// 5-minute ephemeral cache (the API default).
    #[default]
    Ephemeral,
    /// 1-hour ephemeral cache; cheaper for long-lived agent sessions.
    OneHour,
}

/// Per-request overrides applied on top of the backend's defaults. All
/// fields are optional. Only Anthropic honours these today; other backends
/// silently ignore unknown overrides.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct LlmRequestOverrides {
    /// Hard request timeout in milliseconds. `None` keeps the backend default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    /// Override the backend's `max_retries` for this request only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_retries: Option<u32>,
    /// Extra `anthropic-beta` opt-ins for this request.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub betas: Vec<String>,
}

/// How the model should choose tools when `tools` is present.
/// Mirrors Anthropic's `tool_choice` field.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LlmToolChoice {
    /// Model decides whether to call a tool.
    #[default]
    Auto,
    /// Model must call some tool (any from the list).
    Any,
    /// Model must call this specific tool.
    Tool { name: String },
    /// Model must NOT call any tool (force text reply).
    None,
}

/// Configuration for an LLM call
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LlmConfig {
    pub temperature: f32,
    pub max_tokens: u64,
    pub stop_sequences: Option<Vec<String>>,
    /// Anthropic-only nucleus sampling.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    /// Anthropic-only top-k sampling.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_k: Option<u32>,
    /// Tool selection strategy. Defaults to `auto` when tools are present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<LlmToolChoice>,
    /// Anthropic extended-thinking budget in tokens. `None` disables.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_budget_tokens: Option<u32>,
    /// Anthropic `metadata.user_id` for trace correlation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata_user_id: Option<String>,
    /// Cache TTL applied to system blocks. Defaults to 5-minute ephemeral.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_ttl: Option<LlmCacheTtl>,
    /// Per-request overrides routed via the SDK's `*_with` helpers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_overrides: Option<LlmRequestOverrides>,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            temperature: 1.0,
            max_tokens: 8192,
            stop_sequences: None,
            top_p: None,
            top_k: None,
            tool_choice: None,
            thinking_budget_tokens: None,
            metadata_user_id: None,
            cache_ttl: None,
            request_overrides: None,
        }
    }
}

/// LLM backend abstraction - allows swapping between providers
#[async_trait]
pub trait LlmBackend: Send + Sync {
    /// The model identifier for this backend
    fn model(&self) -> &str;

    /// Generate a chat completion
    async fn chat(
        &self,
        messages: Vec<LlmMessage>,
        tools: Option<Vec<LlmToolDefinition>>,
        config: LlmConfig,
    ) -> Result<LlmResponse, SwellError>;

    /// Check if the backend is healthy
    async fn health_check(&self) -> bool;

    /// Generate a streaming chat completion
    ///
    /// Returns a stream of [`StreamEvent`]s that can be used to process
    /// the response in real-time as tokens are generated.
    async fn stream(
        &self,
        messages: Vec<LlmMessage>,
        tools: Option<Vec<LlmToolDefinition>>,
        config: LlmConfig,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamEvent, SwellError>> + Send>>, SwellError>;
}

/// Definition of a tool the LLM can call
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LlmToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

// ============================================================================
// Agent Protocol
// ============================================================================

/// Context passed to agents during execution
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AgentContext {
    pub task: Task,
    pub memory_blocks: Vec<crate::MemoryBlock>,
    pub session_id: SessionId,
    pub workspace_path: Option<String>,
}

/// Result from an agent execution
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct AgentResult {
    pub success: bool,
    pub output: String,
    pub tool_calls: Vec<ToolCallResult>,
    pub tokens_used: u64,
    pub error: Option<String>,
    /// Agent's self-reported confidence score (0.0 to 1.0) for the current output.
    /// Used by the orchestrator to determine if uncertainty pause is needed.
    /// When None, confidence is assumed to be high enough to proceed without pause.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence_score: Option<f64>,
}

/// Result of calling a tool
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ToolCallResult {
    pub tool_name: String,
    pub arguments: serde_json::Value,
    pub result: Result<String, String>,
    pub duration_ms: u64,
}

/// Agent behavior abstraction - implemented by Planner, Generator, Evaluator
#[async_trait]
pub trait Agent: Send + Sync {
    /// The role this agent fulfills
    fn role(&self) -> AgentRole;

    /// Execute the agent's logic
    async fn execute(&self, context: AgentContext) -> Result<AgentResult, SwellError>;

    /// Get a description of what this agent does
    fn description(&self) -> String;
}

// ============================================================================
// Tool Protocol
// ============================================================================
// Tool Protocol
// ============================================================================

/// Input for tool execution
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ToolInput {
    pub name: String,
    pub arguments: serde_json::Value,
}

/// Content variants for tool execution results.
///
/// This enum provides structured, type-safe tool result handling instead of
/// freeform strings. Each variant carries its data in the appropriate format
/// for downstream processing by agents and validation pipelines.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum ToolResultContent {
    /// Plain text output from the tool
    Text(String),
    /// Structured JSON output - useful for tools that return structured data
    Json(serde_json::Value),
    /// Error message describing what went wrong
    Error(String),
    /// Image data with MIME type specification
    Image { data: String, media_type: String },
}

/// Output from tool execution
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ToolOutput {
    /// Signals failure to the LLM - when true, the tool execution failed
    /// and the agent should handle the error appropriately.
    pub is_error: bool,
    /// Structured content from the tool execution.
    /// Multiple content items can be present (e.g., text + image).
    pub content: Vec<ToolResultContent>,
}

/// Behavioral hints for tool execution classification.
///
/// These hints help the system understand tool behavior for:
/// - Retry logic (idempotent tools are safe to retry)
/// - Risk assessment (destructive tools need extra caution)
/// - State tracking (read-only tools don't modify state)
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ToolBehavioralHints {
    /// Tool does not modify any state (filesystem, git, etc.)
    /// Read-only tools are safe to execute for exploration.
    pub read_only_hint: bool,
    /// Tool permanently destroys or overwrites data.
    /// Destructive tools should require explicit confirmation.
    pub destructive_hint: bool,
    /// Tool is safe to retry if execution fails.
    /// Idempotent tools produce the same result regardless of repetitions.
    pub idempotent_hint: bool,
}

impl Default for ToolBehavioralHints {
    fn default() -> Self {
        Self {
            read_only_hint: false,
            destructive_hint: false,
            idempotent_hint: true, // Default to safe to retry
        }
    }
}

/// A tool that can be executed by agents
#[async_trait]
pub trait Tool: Send + Sync {
    /// Unique name of the tool
    fn name(&self) -> &str;

    /// Description for LLM tool selection
    fn description(&self) -> String;

    /// JSON Schema for input parameters
    fn input_schema(&self) -> serde_json::Value;

    /// Risk level of this tool
    fn risk_level(&self) -> crate::ToolRiskLevel;

    /// Permission tier required
    fn permission_tier(&self) -> crate::PermissionTier;

    /// Behavioral hints for execution classification.
    ///
    /// - `read_only_hint`: Tool doesn't modify state
    /// - `destructive_hint`: Tool permanently changes data
    /// - `idempotent_hint`: Safe to retry on failure
    fn behavioral_hints(&self) -> ToolBehavioralHints {
        ToolBehavioralHints::default()
    }

    /// Execute the tool
    async fn execute(&self, arguments: serde_json::Value) -> Result<ToolOutput, SwellError>;

    /// Check if this tool is available in the current environment
    fn is_available(&self) -> bool {
        true
    }
}

// ============================================================================
// Memory Store Protocol
// ============================================================================

/// A memory block with semantic metadata
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MemoryEntry {
    pub id: Uuid,
    pub block_type: crate::MemoryBlockType,
    pub label: String,
    pub content: String,
    pub embedding: Option<Vec<f32>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub metadata: serde_json::Value,
    // =============================================================================
    // Full 8-level scope hierarchy: org → workspace → repo → language → framework
    // → environment → task_type → session
    // =============================================================================
    /// Organization scope - top level of the hierarchy
    pub org: String,
    /// Workspace scope - second level of the hierarchy
    pub workspace: String,
    /// Repository scope - third level of the hierarchy (memories are isolated by repository)
    pub repository: String,
    /// Language scope (e.g., "rust", "python")
    pub language: Option<String>,
    /// Framework scope (e.g., "axum", "actix", "react")
    pub framework: Option<String>,
    /// Environment scope (e.g., "prod", "dev", "test")
    pub environment: Option<String>,
    /// Task type scope (e.g., "bugfix", "feature", "refactor")
    pub task_type: Option<String>,
    /// Session scope - finest granularity (session ID)
    pub session_id: Option<String>,
    /// Last time this memory was reinforced (accessed, used, or confirmed valid).
    /// Used for staleness detection - memories not reinforced within the staleness
    /// window are considered stale and excluded from retrieval.
    pub last_reinforcement: Option<chrono::DateTime<chrono::Utc>>,
    /// Whether this memory has been invalidated due to staleness.
    /// Stale memories are excluded from retrieval results.
    pub is_stale: bool,
    /// Source episode ID - links this memory to the task/episode that created it.
    /// Enables full traceability of knowledge origin.
    pub source_episode_id: Option<Uuid>,
    /// Evidence from the source episode (raw data, transcript excerpt, etc.)
    pub evidence: Option<String>,
    /// Context from the source episode (metadata about how the fact was learned)
    pub provenance_context: Option<serde_json::Value>,
}

impl Default for MemoryEntry {
    fn default() -> Self {
        Self {
            id: Uuid::new_v4(),
            block_type: crate::MemoryBlockType::Project,
            label: String::new(),
            content: String::new(),
            embedding: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            metadata: serde_json::json!({}),
            // Full scope hierarchy
            org: String::new(),
            workspace: String::new(),
            repository: String::new(),
            language: None,
            framework: None,
            environment: None,
            task_type: None,
            session_id: None,
            last_reinforcement: None,
            is_stale: false,
            source_episode_id: None,
            evidence: None,
            provenance_context: None,
        }
    }
}

/// Search query for memory
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MemoryQuery {
    pub query_text: Option<String>,
    pub block_types: Option<Vec<crate::MemoryBlockType>>,
    pub labels: Option<Vec<String>>,
    pub limit: usize,
    pub offset: usize,
    // =============================================================================
    // Full 8-level scope hierarchy: org → workspace → repo → language → framework
    // → environment → task_type → session
    // At least ONE scope level must be specified. Cross-scope queries require
    // cross_scope_override = true.
    // =============================================================================
    /// Organization scope - top level of the hierarchy
    pub org: String,
    /// Workspace scope - second level of the hierarchy
    pub workspace: String,
    /// Repository scope - third level of the hierarchy (REQUIRED for all operations)
    pub repository: String,
    /// Language scope (e.g., "rust", "python")
    pub language: Option<String>,
    /// Framework scope (e.g., "axum", "actix", "react")
    pub framework: Option<String>,
    /// Environment scope (e.g., "prod", "dev", "test")
    pub environment: Option<String>,
    /// Task type scope (e.g., "bugfix", "feature", "refactor")
    pub task_type: Option<String>,
    /// Session scope - finest granularity
    pub session_id: Option<String>,
    /// Optional source episode ID filter - find memories from a specific episode
    pub source_episode_id: Option<Uuid>,
    /// Override flag for cross-scope queries.
    /// When true, allows accessing data from different org/workspace/repo.
    /// When false (default), cross-scope access is denied.
    #[serde(default)]
    pub cross_scope_override: bool,
}

impl Default for MemoryQuery {
    fn default() -> Self {
        Self {
            query_text: None,
            block_types: None,
            labels: None,
            limit: 10,
            offset: 0,
            org: String::new(),
            workspace: String::new(),
            repository: String::new(),
            language: None,
            framework: None,
            environment: None,
            task_type: None,
            session_id: None,
            source_episode_id: None,
            cross_scope_override: false,
        }
    }
}

/// Search result with relevance score
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MemorySearchResult {
    pub entry: MemoryEntry,
    pub score: f32,
}

/// Memory store abstraction - vector, recall, and knowledge graph
#[async_trait]
pub trait MemoryStore: Send + Sync {
    /// Store a memory entry
    async fn store(&self, entry: MemoryEntry) -> Result<Uuid, SwellError>;

    /// Retrieve a memory entry by ID
    async fn get(&self, id: Uuid) -> Result<Option<MemoryEntry>, SwellError>;

    /// Update a memory entry
    async fn update(&self, entry: MemoryEntry) -> Result<(), SwellError>;

    /// Delete a memory entry
    async fn delete(&self, id: Uuid) -> Result<(), SwellError>;

    /// Search memories by text query (hybrid: vector + keyword)
    async fn search(&self, query: MemoryQuery) -> Result<Vec<MemorySearchResult>, SwellError>;

    /// Get all memories of a specific type within repository scope
    async fn get_by_type(
        &self,
        block_type: crate::MemoryBlockType,
        repository: String,
    ) -> Result<Vec<MemoryEntry>, SwellError>;

    /// Get all memories with a specific label within repository scope
    async fn get_by_label(
        &self,
        label: String,
        repository: String,
    ) -> Result<Vec<MemoryEntry>, SwellError>;

    /// Get all memories from a specific source episode (provenance tracking) within repository scope
    async fn get_by_provenance(
        &self,
        source_episode_id: Uuid,
        repository: String,
    ) -> Result<Vec<MemoryEntry>, SwellError>;
}

/// Knowledge graph operations
#[async_trait]
pub trait KnowledgeGraph: Send + Sync {
    /// Add a node to the graph
    async fn add_node(&self, node: KgNode) -> Result<Uuid, SwellError>;

    /// Add an edge between nodes
    async fn add_edge(&self, edge: KgEdge) -> Result<(), SwellError>;

    /// Get a node by ID
    async fn get_node(&self, id: Uuid) -> Result<Option<KgNode>, SwellError>;

    /// Query nodes by label
    async fn query_nodes(&self, label: String) -> Result<Vec<KgNode>, SwellError>;

    /// Traverse the graph from a starting node
    async fn traverse(&self, traversal: KgTraversal) -> Result<Vec<KgPath>, SwellError>;
}

/// A node in the knowledge graph
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct KgNode {
    pub id: Uuid,
    pub node_type: KgNodeType,
    pub name: String,
    pub properties: serde_json::Value,
}

/// Types of nodes in the knowledge graph
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KgNodeType {
    File,
    Function,
    Class,
    Method,
    Module,
    Type,
    Import,
    Variable,
    Test,
}

/// An edge in the knowledge graph
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KgEdge {
    pub id: Uuid,
    pub source: Uuid,
    pub target: Uuid,
    pub relation: KgRelation,
}

/// Types of relationships between nodes
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KgRelation {
    Calls,
    InheritsFrom,
    Imports,
    DependsOn,
    Contains,
    HasType,
    Tests,
}

/// Graph traversal query
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct KgTraversal {
    pub start_node: Uuid,
    pub relation: Option<KgRelation>,
    pub max_depth: usize,
    pub direction: KgDirection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum KgDirection {
    Outgoing,
    Incoming,
    Both,
}

/// A path through the knowledge graph
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct KgPath {
    pub nodes: Vec<KgNode>,
    pub edges: Vec<KgEdge>,
}

// ============================================================================
// Sandbox Protocol
// ============================================================================

/// A sandboxed execution environment
#[async_trait]
pub trait Sandbox: Send + Sync {
    /// Unique identifier for this sandbox instance
    fn id(&self) -> &str;

    /// Start the sandbox
    async fn start(&self) -> Result<(), SwellError>;

    /// Stop the sandbox
    async fn stop(&self) -> Result<(), SwellError>;

    /// Execute a command in the sandbox
    async fn execute(&self, cmd: SandboxCommand) -> Result<SandboxOutput, SwellError>;

    /// Write a file to the sandbox
    async fn write_file(&self, path: &str, content: &[u8]) -> Result<(), SwellError>;

    /// Read a file from the sandbox
    async fn read_file(&self, path: &str) -> Result<Vec<u8>, SwellError>;

    /// Check if the sandbox is running
    async fn is_running(&self) -> bool;
}

/// Command to execute in a sandbox
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SandboxCommand {
    pub command: String,
    pub args: Vec<String>,
    pub env: std::collections::HashMap<String, String>,
    pub working_dir: Option<String>,
    pub timeout_secs: u64,
}

/// Output from a sandbox command
#[derive(Debug, Clone, serde::Serialize, serde:: Deserialize)]
pub struct SandboxOutput {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
}

// ============================================================================
// Checkpoint/State Protocol
// ============================================================================

/// State checkpoint for task persistence
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Checkpoint {
    pub id: Uuid,
    pub task_id: TaskId,
    pub state: TaskState,
    pub snapshot: serde_json::Value,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub metadata: serde_json::Value,
}

/// Checkpoint store for persisting orchestrator state
#[async_trait]
pub trait CheckpointStore: Send + Sync {
    /// Save a checkpoint
    async fn save(&self, checkpoint: Checkpoint) -> Result<Uuid, SwellError>;

    /// Load the latest checkpoint for a task
    async fn load_latest(&self, task_id: TaskId) -> Result<Option<Checkpoint>, SwellError>;

    /// Load a specific checkpoint by ID
    async fn load(&self, id: Uuid) -> Result<Option<Checkpoint>, SwellError>;

    /// List all checkpoints for a task
    async fn list(&self, task_id: TaskId) -> Result<Vec<Checkpoint>, SwellError>;

    /// Delete old checkpoints, keeping only the latest N
    async fn prune(&self, task_id: TaskId, keep: usize) -> Result<(), SwellError>;
}

// ============================================================================
// Validation Gate Protocol
// ============================================================================

/// A validation gate in the pipeline
#[async_trait]
pub trait ValidationGate: Send + Sync {
    /// Name of this validation gate
    fn name(&self) -> &'static str;

    /// Run the validation
    async fn validate(&self, context: ValidationContext) -> Result<ValidationOutcome, SwellError>;

    /// Priority order (lower runs first)
    fn order(&self) -> u32 {
        100
    }
}

/// Context for validation
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ValidationContext {
    pub task_id: TaskId,
    pub workspace_path: String,
    pub changed_files: Vec<String>,
    pub plan: Option<Plan>,
}

/// Outcome of a validation check
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ValidationOutcome {
    pub passed: bool,
    pub messages: Vec<ValidationMessage>,
    pub artifacts: Vec<ValidationArtifact>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ValidationMessage {
    pub level: ValidationLevel,
    pub code: Option<String>,
    pub message: String,
    pub file: Option<String>,
    pub line: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ValidationLevel {
    Error,
    Warning,
    Info,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ValidationArtifact {
    pub name: String,
    pub path: String,
    pub content_type: String,
}

// ============================================================================
// Event/Notification Protocol
// ============================================================================

/// Events emitted by the system
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum Event {
    TaskCreated {
        task_id: TaskId,
    },
    TaskStateChanged {
        task_id: TaskId,
        from: TaskState,
        to: TaskState,
    },
    TaskProgress {
        task_id: TaskId,
        message: String,
    },
    TaskCompleted {
        task_id: TaskId,
        pr_url: Option<String>,
    },
    TaskFailed {
        task_id: TaskId,
        error: String,
    },
    AgentStarted {
        agent_id: AgentId,
        role: AgentRole,
        task_id: TaskId,
    },
    AgentFinished {
        agent_id: AgentId,
        task_id: TaskId,
        success: bool,
    },
    ToolExecuted {
        tool_name: String,
        success: bool,
        duration_ms: u64,
    },
    ValidationStarted {
        task_id: TaskId,
        gate: &'static str,
    },
    ValidationCompleted {
        task_id: TaskId,
        gate: &'static str,
        passed: bool,
    },
    Error {
        message: String,
    },
    /// Turn summary event emitted after each agent turn
    TurnSummary {
        summary: TurnSummaryEvent,
    },
}

/// Event subscriber
#[async_trait]
pub trait EventSubscriber: Send + Sync {
    /// Subscribe to events
    async fn on_event(&self, event: Event) -> Result<(), SwellError>;

    /// Filter - return true to receive this event
    fn filter(&self, _event_type: &str) -> bool {
        true
    }
}

// ============================================================================
// Service Container / DI
// ============================================================================

/// A service container for dependency injection
pub trait ServiceContainer: Send + Sync {
    /// Get a service by type
    fn get<T: 'static>(&self) -> Option<&T>;

    /// Get a service by type, returning a clone for Clone types
    fn get_clone<T: Clone + 'static>(&self) -> Option<T> {
        self.get::<T>().cloned()
    }

    /// Check if a service is registered
    fn has<T: 'static>(&self) -> bool;
}

/// Extension trait for dynamic service retrieval
pub trait DynServiceContainer: Send + Sync {
    /// Get a service by name (for dynamic dispatch)
    fn get_dyn(&self, name: &str) -> Option<&dyn Any>;
}
