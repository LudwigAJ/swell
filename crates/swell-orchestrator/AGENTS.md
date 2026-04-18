# swell-orchestrator AGENTS.md

## Purpose

`swell-orchestrator` is the task coordination crate of the SWELL autonomous coding engine. It manages the overall execution flow including task lifecycle, agent coordination, context management, policy evaluation, and validation orchestration.

This crate handles:
- **Task State Machine** — State transitions: Created → Enriched → Ready → Assigned → Executing → Validating → Accepted/Rejected
- **Agent Pool** — Manages agent instances (PlannerAgent, GeneratorAgent, EvaluatorAgent, etc.)
- **Execution Controller** — Coordinates multi-turn agent loops with tool execution
- **Policy Engine** — Evaluates YAML-defined policies against agent actions
- **Context Management** — Context chunking and pipeline assembly
- **Feature Leads** — Hierarchical sub-orchestrators for large projects
- **Merge Queue** — GitHub/Mergify PR stacking integration

**Depends on:** `swell-core`, `swell-llm`, `swell-state`, `swell-tools`, `swell-validation`

## Binding rule — production-required subsystems

> **All production-required subsystems of `Orchestrator` must be
> constructor parameters of `Orchestrator::new`, not `Option<_>` fields
> with `with_*` setters.** The test-only `OrchestratorBuilder` may accept
> them optionally for fake injection. If a new subsystem is not yet
> implemented in production, do not add it as a field. Add it in the
> same PR that wires it end-to-end.

This rule was established by the constructor refactor (`plan/structural-refactors/01_orchestrator_constructor/`) — the full rationale and anti-pattern catalogue live in `02_target_architecture.md`.

### Forbidden shapes (enforced by CI `antipattern-gate` in `.github/workflows/ci.yml`)

- `Arc<Mutex<Option<T>>>` / `Arc<RwLock<Option<T>>>` on a required subsystem — structurally equivalent to an `Option` field.
- `llm_backend: Option<…>` or `execution_controller: Arc<RwLock<Option<…>>>` — flip to required `Arc<T>`.
- `execution_controller() -> Option<…>` accessor — return `Arc<T>` directly.
- `impl Default for (ExecutionController | ValidationOrchestrator | WorktreePool | PostToolHookManager)` — back-door `Option::None`.
- `fn with_llm` / `fn with_checkpoint_manager` on `Orchestrator` — setters live **only** on `OrchestratorBuilder` (cfg-gated `test-support`).
- `*const Orchestrator`, `unsafe impl Sync/Send for ExecutionController`, `Arc::from_raw`, `std::ptr::null` — the parent↔child cycle is resolved via `Arc::new_cyclic` + `Weak<Orchestrator>`. No unsafe escape hatches. (Floor established post-`e770933`.)

### The cycle

`Orchestrator` owns `Arc<ExecutionController>`; `ExecutionController` holds `Weak<Orchestrator>` and upgrades on demand. The `.upgrade()` is guaranteed to succeed because `Orchestrator` outlives every `ExecutionController` — the strong count is ≥ 1 for any reachable controller.

## Test-only `OrchestratorBuilder`

Lives at `src/builder.rs`, gated by `#[cfg(any(test, feature = "test-support"))]`. Production binaries build with `--no-default-features` and CI (`build-no-default-features` job) verifies the `OrchestratorBuilder` symbol is absent from the release artifact.

## Public API

### Orchestrator (`lib.rs`)

```rust
pub struct Orchestrator {
    state_machine: Arc<RwLock<TaskStateMachine>>,
    agent_pool: Arc<RwLock<AgentPool>>,
    checkpoint_manager: Arc<CheckpointManager>,
    event_sender: broadcast::Sender<OrchestratorEvent>,
    feature_lead_manager: Arc<RwLock<FeatureLeadManager>>,
}

impl Orchestrator {
    /// Production constructor. Returns `Arc<Self>` because the
    /// `Orchestrator ↔ ExecutionController` cycle is resolved via
    /// `Arc::new_cyclic` + `Weak<Orchestrator>`. See
    /// `plan/structural-refactors/01_orchestrator_constructor/` for the
    /// rationale; this is the one intentional signature deviation from
    /// the doc (which specifies `-> Self`).
    pub fn new(llm: Arc<dyn LlmBackend>) -> Arc<Self>;
    pub fn subscribe(&self) -> broadcast::Receiver<OrchestratorEvent>;

    // Task lifecycle
    pub async fn create_task(&self, description: String) -> Task;
    pub async fn get_task(&self, id: Uuid) -> Result<Task, SwellError>;
    pub async fn set_plan(&self, task_id: Uuid, plan: Plan) -> Result<(), SwellError>;
    pub async fn start_task(&self, task_id: Uuid) -> Result<(), SwellError>;
    pub async fn start_validation(&self, task_id: Uuid) -> Result<(), SwellError>;
    pub async fn complete_task(&self, task_id: Uuid, result: ValidationResult) -> Result<(), SwellError>;

    // Agent management
    pub async fn register_agent(&self, role: AgentRole, model: String) -> AgentId;
    pub async fn available_agents(&self, role: AgentRole) -> usize;
    pub async fn assign_task(&self, task_id: Uuid, role: AgentRole) -> Result<AgentId, SwellError>;
    pub async fn release_agent(&self, agent_id: AgentId, task_id: Uuid);

    // Checkpointing
    pub async fn restore_task(&self, task_id: Uuid) -> Result<Option<Task>, SwellError>;
    pub async fn has_checkpoint(&self, task_id: Uuid) -> Result<bool, SwellError>;

    // Feature leads
    pub async fn get_active_feature_leads(&self) -> Vec<FeatureLead>;
    pub async fn has_feature_lead(&self, task_id: Uuid) -> bool;
    pub async fn get_feature_lead(&self, task_id: Uuid) -> Option<FeatureLead>;
}
```

### Task State Machine (`state_machine.rs`)

```rust
pub struct TaskStateMachine { /* ... */ }

impl TaskStateMachine {
    pub fn new() -> Self;
    pub fn create_task(&mut self, description: String) -> Task;
    pub fn get_task(&self, id: Uuid) -> Result<Task, SwellError>;
    pub fn enrich_task(&mut self, id: Uuid) -> Result<(), SwellError>;
    pub fn ready_task(&mut self, id: Uuid) -> Result<(), SwellError>;
    pub fn assign_task(&mut self, id: Uuid, agent_id: Uuid) -> Result<(), SwellError>;
    pub fn start_execution(&mut self, id: Uuid) -> Result<(), SwellError>;
    pub fn start_validation(&mut self, id: Uuid) -> Result<(), SwellError>;
    pub fn accept_task(&mut self, id: Uuid) -> Result<(), SwellError>;
    pub fn reject_task(&mut self, id: Uuid) -> Result<(), SwellError>;
    pub fn retry_task(&mut self, id: Uuid) -> Result<(), SwellError>;
    pub fn escalate_task(&mut self, id: Uuid) -> Result<(), SwellError>;
    pub fn pause_task(&mut self, id: Uuid, reason: String) -> Result<(), SwellError>;
    pub fn resume_task(&mut self, id: Uuid) -> Result<(), SwellError>;
}
```

### Agent Types (`agents.rs`)

```rust
// Agent handles
pub struct AgentHandle { pub id: AgentId, pub role: AgentRole }
pub struct AgentPool { /* ... */ }

// Agent implementations
pub struct PlannerAgent { /* ... */ }
pub struct GeneratorAgent { /* ... */ }
pub struct EvaluatorAgent { /* ... */ }
pub struct CoderAgent { /* ... */ }
pub struct ReviewerAgent { /* ... */ }
pub struct RefactorerAgent { /* ... */ }
pub struct DocWriterAgent { /* ... */ }
pub struct TestWriterAgent { /* ... */ }

// System prompt builder
pub struct SystemPromptBuilder { /* ... */ }
pub struct SystemPromptConfig { /* ... */ }
```

### Execution Controller (`execution.rs`)

```rust
pub struct ExecutionController { /* ... */ }

impl ExecutionController {
    pub fn new(/* ... */) -> Self;
    pub async fn execute(&self, task: &Task, context: AgentContext) -> Result<ExecutionResult, SwellError>;
}
```

### Policy Engine (`policy.rs`)

```rust
pub struct PolicyEngine { /* ... */ }
pub struct PolicyRule { /* ... */ }
pub enum PolicyEffect { Allow, Deny, Ask }
```

### Context Management (`context_chunking.rs`, `context_pipeline.rs`)

```rust
pub struct ContextChunkingAssembler { /* ... */ }
pub struct ContextPipelineConfig { /* ... */ }
pub enum ContextTier { System, Project, Task, Conversation }
```

### Feature Leads (`feature_leads.rs`)

```rust
pub struct FeatureLeadManager { /* ... */ }
pub struct FeatureLead { /* ... */ }

pub const MAX_ORCHESTRATOR_DEPTH: usize = 3;
pub const FEATURE_LEAD_STEP_THRESHOLD: usize = 10;
```

### Key Re-exports

```rust
pub use agents::{AgentPool, PlannerAgent, GeneratorAgent, EvaluatorAgent, SystemPromptBuilder};
pub use state_machine::TaskStateMachine;
pub use execution::ExecutionController;
pub use policy::{PolicyEngine, PolicyRule, PolicyEffect};
pub use scheduler::{Scheduler, SchedulerConfig};
pub use feature_leads::{FeatureLead, FeatureLeadManager};
```

## Architecture

```
┌─────────────────────────────────────────────────────────────────────┐
│                    swell-orchestrator                               │
├─────────────────────────────────────────────────────────────────────┤
│                                                                     │
│  ┌──────────────────────────────────────────────────────────────┐  │
│  │                      Orchestrator                             │  │
│  │  ┌──────────────┐  ┌──────────────┐  ┌──────────────────┐  │  │
│  │  │TaskStateMachine│  │ AgentPool    │  │CheckpointManager │  │  │
│  │  └──────────────┘  └──────────────┘  └──────────────────┘  │  │
│  │  ┌──────────────────────────────────────────────────────┐   │  │
│  │  │               FeatureLeadManager                      │   │  │
│  │  │  (hierarchical sub-orchestrators, max depth = 3)     │   │  │
│  │  └──────────────────────────────────────────────────────┘   │  │
│  └──────────────────────────────────────────────────────────────┘  │
│                              │                                      │
│  ┌──────────────────────────┼──────────────────────────────────┐  │
│  │                    Agent Types                               │  │
│  │  ┌─────────┐  ┌─────────┐  ┌─────────┐  ┌────────────────┐  │  │
│  │  │ Planner │  │Generator│  │Evaluator│  │    Others      │  │  │
│  │  │ Agent   │  │ Agent   │  │ Agent   │  │ (Coder,Review, │  │  │
│  │  └─────────┘  └─────────┘  └─────────┘  │  Refactorer)   │  │  │
│  │                                          └────────────────┘  │  │
│  └───────────────────────────────────────────────────────────────┘  │
│                              │                                      │
│  ┌──────────────────────────┼──────────────────────────────────┐  │
│  │              Supporting Systems                               │  │
│  │  ┌──────────────┐  ┌──────────────┐  ┌────────────────┐   │  │
│  │  │ PolicyEngine  │  │ ContextMgr    │  │ ExecutionCtrl  │   │  │
│  │  └──────────────┘  └──────────────┘  └────────────────┘   │  │
│  │  ┌──────────────┐  ┌──────────────┐  ┌────────────────┐   │  │
│  │  │  Scheduler   │  │  Metrics     │  │ MergeQueue     │   │  │
│  │  └──────────────┘  └──────────────┘  └────────────────┘   │  │
│  └───────────────────────────────────────────────────────────────┘  │
│                                                                     │
└─────────────────────────────────────────────────────────────────────┘
                           │ orchestrates
                           ▼
    ┌──────────┬───────────┼───────────┬───────────┐
    │swell-llm │swell-tools│swell-state│swell-validation
```

**Key modules:**
- `lib.rs` — Main `Orchestrator` struct and entry points
- `state_machine.rs` — Task state machine with all transitions
- `agents.rs` — All agent types (Planner, Generator, Evaluator, etc.)
- `execution.rs` — Execution controller for multi-turn loops
- `policy.rs` — Policy engine for action authorization
- `scheduler.rs` — Task scheduling and priority
- `context_chunking.rs` — Context chunking for token limits
- `context_pipeline.rs` — Context assembly pipeline
- `feature_leads.rs` — FeatureLead sub-orchestrators
- `merge_queue.rs` — PR stacking for GitHub/Mergify

**Task State Machine:**
```
Created → Enriched → Ready → Assigned → Executing → Validating
                                                      ↓
                              Accepted ←────────────────┼────────────────→ Rejected
                                   (pass)              │ (fail)          ↓
                                                      │            Retry/Escalate
```

## Testing

```bash
# Run tests for swell-orchestrator
cargo test -p swell-orchestrator -- --test-threads=4

# Run with logging
RUST_LOG=debug cargo test -p swell-orchestrator

# Run specific test
cargo test -p swell-orchestrator -- test_create_task --nocapture

# Run integration tests
cargo test -p swell-orchestrator -- test_full_task_lifecycle

# Run agent tests
cargo test -p swell-orchestrator -- agents
```

**Test patterns:**
- Unit tests for state machine transitions
- Integration tests for full task lifecycle
- Agent pool concurrency tests
- Policy evaluation tests
- Context chunking tests

**Mock patterns:**
```rust
#[tokio::test]
async fn test_create_task_returns_task_with_created_state() {
    let orchestrator = Orchestrator::new();
    let task = orchestrator.create_task("Test task".to_string()).await;
    assert_eq!(task.state, TaskState::Created);
}

#[tokio::test]
async fn test_full_task_lifecycle() {
    let orchestrator = Orchestrator::new();
    let task = orchestrator.create_task("Implement feature X".to_string()).await;
    // ... full lifecycle test
}
```

## Dependencies

```toml
# swell-orchestrator/Cargo.toml
[dependencies]
swell-core = { path = "../swell-core" }
swell-llm = { path = "../swell-llm" }
swell-state = { path = "../swell-state" }
swell-tools = { path = "../swell-tools" }
swell-validation = { path = "../swell-validation" }
tokio.workspace = true
serde.workspace = true
serde_json.workspace = true
serde_yaml.workspace = true
thiserror.workspace = true
anyhow.workspace = true
tracing.workspace = true
uuid.workspace = true
chrono.workspace = true
async-trait.workspace = true
futures.workspace = true
regex.workspace = true

[dev-dependencies]
tokio-test.workspace = true
mockall.workspace = true
```

**Internal dependencies:**
- `swell-core` — Types, traits, errors
- `swell-llm` — LLM backends for agents
- `swell-state` — Checkpoint management
- `swell-tools` — Tool registry and execution
- `swell-validation` — Validation gates
