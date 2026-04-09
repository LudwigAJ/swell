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

use crate::{Task, TaskState, Plan, AgentId, AgentRole, SwellError};
use async_trait::async_trait;
use std::any::Any;
use uuid::Uuid;

// ============================================================================
// LLM Backend Protocol
// ============================================================================

/// A message in an LLM conversation
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LlmMessage {
    pub role: LlmRole,
    pub content: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum LlmRole {
    System,
    User,
    Assistant,
}

/// A tool call request from an LLM
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LlmToolCall {
    pub id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}

/// Response from an LLM
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LlmResponse {
    pub content: String,
    pub tool_calls: Option<Vec<LlmToolCall>>,
    pub usage: LlmUsage,
}

/// Token usage statistics
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct LlmUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
}

/// Configuration for an LLM call
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LlmConfig {
    pub temperature: f32,
    pub max_tokens: u64,
    pub stop_sequences: Option<Vec<String>>,
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
    pub session_id: Uuid,
    pub workspace_path: Option<String>,
}

/// Result from an agent execution
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AgentResult {
    pub success: bool,
    pub output: String,
    pub tool_calls: Vec<ToolCallResult>,
    pub tokens_used: u64,
    pub error: Option<String>,
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

/// Input for tool execution
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ToolInput {
    pub name: String,
    pub arguments: serde_json::Value,
}

/// Output from tool execution
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ToolOutput {
    pub success: bool,
    pub result: String,
    pub error: Option<String>,
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
}

/// Search query for memory
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MemoryQuery {
    pub query_text: Option<String>,
    pub block_types: Option<Vec<crate::MemoryBlockType>>,
    pub labels: Option<Vec<String>>,
    pub limit: usize,
    pub offset: usize,
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

    /// Get all memories of a specific type
    async fn get_by_type(
        &self,
        block_type: crate::MemoryBlockType,
    ) -> Result<Vec<MemoryEntry>, SwellError>;

    /// Get all memories with a specific label
    async fn get_by_label(&self, label: String) -> Result<Vec<MemoryEntry>, SwellError>;
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum KgNodeType {
    File,
    Function,
    Class,
    Method,
    Module,
    Type,
    Import,
}

/// An edge in the knowledge graph
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct KgEdge {
    pub id: Uuid,
    pub source: Uuid,
    pub target: Uuid,
    pub relation: KgRelation,
}

/// Types of relationships between nodes
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum KgRelation {
    Calls,
    InheritsFrom,
    Imports,
    DependsOn,
    Contains,
    HasType,
}

/// Graph traversal query
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct KgTraversal {
    pub start_node: Uuid,
    pub relation: Option<KgRelation>,
    pub max_depth: usize,
    pub direction: KgDirection,
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
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
    pub task_id: Uuid,
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
    async fn load_latest(&self, task_id: Uuid) -> Result<Option<Checkpoint>, SwellError>;

    /// Load a specific checkpoint by ID
    async fn load(&self, id: Uuid) -> Result<Option<Checkpoint>, SwellError>;

    /// List all checkpoints for a task
    async fn list(&self, task_id: Uuid) -> Result<Vec<Checkpoint>, SwellError>;

    /// Delete old checkpoints, keeping only the latest N
    async fn prune(&self, task_id: Uuid, keep: usize) -> Result<(), SwellError>;
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
    pub task_id: Uuid,
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
    TaskCreated { task_id: Uuid },
    TaskStateChanged { task_id: Uuid, from: TaskState, to: TaskState },
    TaskProgress { task_id: Uuid, message: String },
    TaskCompleted { task_id: Uuid, pr_url: Option<String> },
    TaskFailed { task_id: Uuid, error: String },
    AgentStarted { agent_id: AgentId, role: AgentRole, task_id: Uuid },
    AgentFinished { agent_id: AgentId, task_id: Uuid, success: bool },
    ToolExecuted { tool_name: String, success: bool, duration_ms: u64 },
    ValidationStarted { task_id: Uuid, gate: &'static str },
    ValidationCompleted { task_id: Uuid, gate: &'static str, passed: bool },
    Error { message: String },
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
