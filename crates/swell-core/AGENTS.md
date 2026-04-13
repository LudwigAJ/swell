# swell-core AGENTS.md

## Purpose

`swell-core` is the foundational crate of the SWELL autonomous coding engine. It defines core types, traits, error types, event systems, and the state machine that underpins all other crates. **All other SWELL crates depend on `swell-core`.**

This crate provides:
- **Core types**: `Task`, `Agent`, `TaskState`, `AgentRole`, `Plan`, `PlanStep` enums
- **Traits**: `Agent`, `Tool`, `Validator`, `LlmBackend`, `CheckpointStore`, `Sandbox` traits
- **Event system**: `EventEmitter` for observability across the system
- **Error handling**: `SwellError` enum with all error variants
- **Resilience**: `CircuitBreaker` pattern for external calls
- **Cost tracking**: `CostTracker` for token and cost accounting
- **Kill switch**: `KillSwitch` emergency stop mechanism
- **Observability**: OpenTelemetry tracing and Langfuse integration

## Public API

### Core Types (`types.rs`)

```rust
// Task and state types
pub struct Task { pub id: Uuid, pub state: TaskState, pub description: String, ... }
pub enum TaskState { Created, Enriched, Ready, Assigned, Executing, Validating, Accepted, Rejected, Failed, Cancelled, Escalated, Paused }
pub enum AgentRole { Planner, Generator, Evaluator, Coder, Reviewer, Refactorer }
pub enum RiskLevel { Low, Medium, High, Critical }

// Plan types
pub struct Plan { pub id: Uuid, pub task_id: Uuid, pub steps: Vec<PlanStep>, ... }
pub struct PlanStep { pub id: Uuid, pub description: String, pub affected_files: Vec<String>, ... }

// LLM types
pub struct LlmMessage { pub role: LlmRole, pub content: String }
pub enum LlmRole { System, User, Assistant }
pub struct LlmResponse { pub content: String, pub tool_calls: Vec<LlmToolCall>, ... }
pub struct LlmToolCall { pub id: String, pub name: String, pub input: Value }
```

### Key Traits (`traits.rs`)

```rust
// LLM Backend trait - implemented by swell-llm
#[async_trait]
pub trait LlmBackend: Send + Sync {
    async fn chat(&self, messages: Vec<LlmMessage>, tools: Option<Vec<LlmToolDefinition>>, config: LlmConfig) -> Result<LlmResponse, SwellError>;
    async fn chat_streaming(&self, messages: Vec<LlmMessage>, tools: Option<Vec<LlmToolDefinition>>, config: LlmConfig) -> Result<StreamingResponse, SwellError>;
}

// Tool trait - implemented by tool crates
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn input_schema(&self) -> Value;
    async fn execute(&self, input: Value) -> Result<ToolOutput, SwellError>;
}

// Validator trait - implemented by swell-validation
#[async_trait]
pub trait ValidationGate: Send + Sync {
    async fn validate(&self, context: ValidationContext) -> Result<ValidationResult, SwellError>;
}

// Checkpoint trait - implemented by swell-state
#[async_trait]
pub trait CheckpointStore: Send + Sync {
    async fn save(&self, checkpoint: Checkpoint) -> Result<(), SwellError>;
    async fn load(&self, id: Uuid) -> Result<Checkpoint, SwellError>;
    async fn list(&self, task_id: Uuid) -> Result<Vec<Checkpoint>, SwellError>;
}
```

### Key Re-exports

```rust
pub use cost_tracking::{CostTracker, CostSummary, record_llm_cost, get_total_llm_tokens};
pub use events::{EventStore, ObservableEvent, ToolInvocation};
pub use kill_switch::{KillSwitch, KillSwitchGuard, KillLevel};
pub use error::SwellError;
pub use traits::{LlmBackend, Tool, ValidationGate, Checkpoint, CheckpointStore};
```

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                        swell-core                          │
├─────────────────────────────────────────────────────────────┤
│  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌───────────┐  │
│  │  types   │  │  traits  │  │  events  │  │   error   │  │
│  │  Task    │  │ LlmBackend│  │ Emitter  │  │SwellError │  │
│  │  Agent   │  │   Tool   │  │  Store   │  │           │  │
│  │  Plan    │  │Validator │  │          │  │           │  │
│  └──────────┘  └──────────┘  └──────────┘  └───────────┘  │
│                                                             │
│  ┌──────────────┐  ┌──────────────┐  ┌────────────────┐  │
│  │circuit_breaker│  │cost_tracking │  │  kill_switch   │  │
│  │              │  │ CostTracker  │  │                │  │
│  └──────────────┘  └──────────────┘  └────────────────┘  │
│                                                             │
│  ┌──────────────┐  ┌──────────────┐  ┌────────────────┐  │
│  │ opentelemetry │  │  langfuse    │  │ trace_waterfall│  │
│  │              │  │              │  │                │  │
│  └──────────────┘  └──────────────┘  └────────────────┘  │
└─────────────────────────────────────────────────────────────┘
                           │ depends on
                           ▼
    ┌──────────┬───────────┼───────────┬───────────┐
    │swell-llm │swell-tools│swell-state│swell-validation
```

**Key modules:**
- `types.rs` — Task, Agent, State enums; Plan, PlanStep structs
- `traits.rs` — Agent, Tool, LlmBackend, Validator, CheckpointStore traits
- `events.rs` — Event emitter for observability
- `error.rs` — SwellError enum with all error variants
- `circuit_breaker.rs` — Resilience pattern for external calls
- `cost_tracking.rs` — Token and cost accounting
- `kill_switch.rs` — Emergency stop mechanism
- `opentelemetry.rs` — OpenTelemetry tracing with GenAI conventions
- `langfuse.rs` — LLM observability integration
- `trace_waterfall.rs` — Distributed trace visualization

**Concurrency:** Uses `Arc<RwLock<T>>` for shared mutable state. All traits are `Send + Sync`.

## Testing

```bash
# Run tests for swell-core
cargo test -p swell-core -- --test-threads=4

# Run with logging
RUST_LOG=debug cargo test -p swell-core

# Run specific test
cargo test -p swell-core -- test_task_state_transitions --nocapture
```

**Test structure:**
- Unit tests in `#[cfg(test)]` modules within each source file
- Tests for state machine transitions in `types.rs` tests
- Tests for circuit breaker in `circuit_breaker.rs`
- Tests for cost tracking in `cost_tracking.rs`
- Tests for kill switch in `kill_switch.rs`

**Mock patterns:** Uses `mockall` for trait mocking in tests.

## Dependencies

```toml
# swell-core/Cargo.toml
[dependencies]
tokio.workspace = true
serde.workspace = true
serde_json.workspace = true
thiserror.workspace = true
anyhow.workspace = true
tracing.workspace = true
tracing-subscriber.workspace = true
uuid.workspace = true
chrono.workspace = true
async-trait.workspace = true
futures.workspace = true

# OpenTelemetry
opentelemetry = "0.26"
opentelemetry_sdk = "0.26"
opentelemetry-otlp = "0.26"  # optional
tracing-opentelemetry = "0.28"
tonic = "0.12"  # optional

# Additional
sha2.workspace = true
reqwest.workspace = true
base64.workspace = true
redis = "0.27"
tree-sitter = "0.24"
tree-sitter-rust = "0.23"
tree-sitter-python = "0.23"
tree-sitter-typescript = "0.23"
tree-sitter-go = "0.23"
tree-sitter-java = "0.23"
tree-sitter-c = "0.24"
tree-sitter-cpp = "0.23"
```

**No internal dependencies** — `swell-core` is the base crate with no dependencies on other workspace crates.
