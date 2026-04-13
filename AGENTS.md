# SWELL Repository Guidelines

## Project Overview

SWELL is an autonomous coding engine built in Rust, designed to handle software engineering tasks end-to-end: planning, execution, testing, and validation. The system uses a multi-agent architecture with specialized agents for different phases of the development workflow.

For the full project specification, see `plan/Autonomous Coding Engine.md`.

## Project Structure

```
swell/
├── Cargo.toml              # Workspace manifest
├── crates/
│   ├── swell-core/         # Core types, traits, state machine
│   ├── swell-orchestrator/ # Task orchestration, scheduling, policy engine
│   ├── swell-llm/          # LLM backends (Anthropic, OpenAI, Mock)
│   ├── swell-tools/        # Tool implementations (file, git, shell, search)
│   ├── swell-validation/   # Validation gates and pipelines
│   ├── swell-memory/       # Memory system with SQLite store
│   ├── swell-state/        # State management and checkpoints
│   ├── swell-sandbox/      # Sandbox isolation (stub for MVP)
│   └── swell-benchmark/    # Benchmark suite for evaluation
├── clients/
│   └── swell-cli/          # CLI client for daemon interaction
├── .swell/                 # Configuration directory
│   ├── settings.json       # Runtime settings (timeouts, limits)
│   ├── policies/default.yaml # Policy rules with deny-first semantics
│   ├── models.json         # LLM model routing
│   ├── crates.json         # Crate dependencies
│   ├── milestones.json     # Milestone definitions
│   └── prompts/            # Agent system prompts
├── plan/
│   ├── Autonomous Coding Engine.md        # Master specification
│   └── research_documents/                # Detailed subsystem specs
└── tests/                 # Integration tests
```

## Core Crates

### swell-core
Core types, traits, error types, and state machine definitions. **All other crates depend on this.**

**Key Modules:**
- `types.rs` - Task, Agent, State enums
- `traits.rs` - Agent, Tool, Validator traits
- `events.rs` - Event emitter for observability
- `error.rs` - SwellError enum with all error variants
- `circuit_breaker.rs` - Resilience pattern for external calls
- `cost_tracking.rs` - Token and cost accounting
- `kill_switch.rs` - Emergency stop mechanism
- `langfuse.rs` - LLM observability integration
- `opentelemetry.rs` - OpenTelemetry tracing setup

### swell-llm
LLM backend implementations with model routing.

**Backends:**
- `anthropic.rs` - Anthropic Claude API
- `openai.rs` - OpenAI API
- `mock.rs` - Mock backend for testing

**Key Types:**
- `LlmBackend` trait - Interface for all LLM backends
- `ModelRouter` - Routes requests to appropriate model

### swell-tools
Tool implementations for code operations and policy enforcement.

**Tools:**
- `file.rs` - Read, write, edit files
- `git.rs` - Git operations (status, diff, commit, branch)
- `shell.rs` - Shell command execution
- `search.rs` - Grep, glob for codebase exploration
- `registry.rs` - Tool registry and permission tiers
- `mcp.rs` - MCP client for external tools
- `vault.rs` - Secret management
- `cedar_policy.rs` - Authorization policy evaluation
- `loop_detection.rs` - Doom loop prevention

### swell-validation
Validation gates and test pipelines.

**Gates:**
- `lint_gate.rs` - Clippy/format checks
- `test_gate.rs` - Cargo test execution
- `security_gate.rs` - Security scanning (stub)
- `ai_review_gate.rs` - LLM-based code review

### swell-memory
Memory system with SQLite persistence.

**Modules:**
- `sqlite_store.rs` - SQLite storage backend
- `memory_blocks.rs` - Memory block management
- `recall.rs` - Memory retrieval
- `skill_extraction.rs` - Skill pattern learning
- `pattern_learning.rs` - Pattern recognition

### swell-state
State management and checkpoint persistence.

**Modules:**
- Checkpoint creation and restoration
- State serialization/deserialization
- Migration support

### swell-sandbox
Sandbox isolation for tool execution.

**Features:**
- Process isolation for shell commands
- Linux: libcia-based containment (stub for MVP)
- macOS/Windows: process isolation via token privileges

### swell-skills
Agent Skills loader following the [agentskills.io](https://agentskills.io) standard.

**Features:**
- Skill discovery from `.swell/skills/` directory
- YAML frontmatter parsing for metadata
- User-extensible skill registry
- Skill conflict detection

### swell-orchestrator
Coordinates task planning and execution flow.

**Key Modules:**
- `orchestrator.rs` - Main orchestration loop
- `scheduler.rs` - Task queue and dependency management
- `policy.rs` - Policy evaluation engine
- `autonomy.rs` - Autonomy level controller
- `backlog.rs` - Work backlog aggregation

**Agent Types:**
- `planner_agent.rs` - Plan generation
- `generator_agent.rs` - Code generation
- `evaluator_agent.rs` - Validation evaluation
- `coder_agent.rs` - General coding
- `reviewer_agent.rs` - Code review
- `refactorer_agent.rs` - Refactoring

### swell-daemon
Daemon server running as Unix socket server.

**Commands:**
- `TaskCreate` - Create new task
- `TaskApprove` - Approve task
- `TaskReject` - Reject task
- `TaskCancel` - Cancel task
- `TaskList` - List tasks
- `TaskWatch` - Watch task state

### swell-cli
CLI client for daemon interaction.

**Commands:**
- `swell task <description>` - Create task
- `swell list` - List tasks
- `swell watch <id>` - Watch task
- `swell cancel <id>` - Cancel task
- `swell approve <id>` - Approve task

### swell-benchmark
Benchmark suite for system evaluation.

**Contains:**
- 50 curated benchmark tasks
- Metrics aggregation
- Progress tracking

## Configuration

All configurable values are in `.swell/` folder:

| File | Purpose |
|------|---------|
| `settings.json` | Runtime settings (timeouts, limits, thresholds) |
| `policies/default.yaml` | Policy rules with deny-first semantics |
| `models.json` | LLM model routing and configuration |
| `crates.json` | Workspace crate dependencies |
| `milestones.json` | Milestone definitions and blocking rules |
| `prompts/` | Agent system prompts |

**Never hardcode magic numbers** - always load from `.swell/settings.json` or environment variables.

## Build, Test, and Development

### Default Validation Scope
```bash
cargo check -p <crate>
cargo build -p <crate>
cargo test -p <crate> -- --test-threads=4
cargo clippy -p <crate> -- -D warnings
```

Use crate-scoped validation by default for the directly affected package. Use `-- --test-threads=1` only for stateful, flaky, or explicitly serial tests. Reserve workspace-wide `cargo build/test/clippy` for explicit full-repo validation, cross-crate changes, or final release gates.

### Workspace-wide Validation (Opt-in)
```bash
cargo build --workspace
cargo test --workspace
cargo clippy --workspace -- -D warnings
```

### Format
```bash
cargo fmt --all
```

### Run CLI
```bash
cargo run --bin swell -- <command>
```

### Run Daemon
```bash
cargo run --bin swell-daemon
```

## Architecture Overview

The autonomous coding engine consists of several core subsystems:

### Orchestrator
Coordinates task planning and execution flow using a state machine:
- CREATED → PLANNING → APPROVED → EXECUTING → VALIDATING → COMPLETED
- Or: FAILED, CANCELLED, PAUSED states

### Memory System
Persists context and learned patterns across sessions using SQLite:
- Memory blocks for context assembly
- Recall for retrieval
- Skill extraction from successful trajectories
- Pattern learning from feedback

### Tool Runtime
Executes code, runs tests, and manages subprocesses:
- File tools (read, write, edit)
- Git tools (status, diff, commit, branch)
- Shell execution
- Search (grep, glob)

### Validation Layer
Ensures outputs meet quality and correctness standards:
- Lint gate (clippy, format)
- Test gate (cargo test)
- Security gate (stub)
- AI review gate

### LLM Integration
Multi-backend LLM support:
- Anthropic Claude
- OpenAI GPT
- Mock for testing

## MCP Protocol Integration

SWELL acts as an **MCP Client** - it connects to and consumes tools from external MCP servers. The servers (not SWELL) implement the actual AST/code intelligence logic.

### Architecture

```
SWELL (MCP Client)           External MCP Servers
      │                              │
      ├── connect ──────────────────►│ mcp-server-tree-sitter
      │                               │   └── AST parsing, symbol extraction
      ├── connect ──────────────────►│ eslint/mcp
      │                               │   └── JavaScript/TypeScript linting  
      ├── connect ──────────────────►│ mcp-language-server + rust-analyzer
      │                               │   └── Rust code intelligence
      │                               │
      │◄── capabilities negotiation ──│
      │◄── tools/list ───────────────│
      │◄── tools/call ───────────────│
```

### MCP Client Implementation

The MCP client in `swell-tools/src/mcp.rs` follows the standard protocol:

- **Transport**: JSON-RPC 2.0 over stdio (subprocess)
- **Startup**: `initialize` → capabilities → `notifications/initialized`
- **Discovery**: `tools/list` to discover available tools
- **Invocation**: `tools/call` to execute tools

### Capability Negotiation

At startup, SWELL and each MCP server negotiate:
- Protocol version
- Supported capabilities (tools, resources, prompts, etc.)

SWELL does NOT hardcode specific tools - it discovers what's available at runtime.

### MCP Configuration

Configure external MCP servers in `.swell/mcp_servers.json`:

```json
{
  "servers": [
    {
      "name": "tree-sitter",
      "command": "python3",
      "args": ["-m", "mcp_server_tree_sitter"],
      "env": {}
    },
    {
      "name": "rust-analyzer", 
      "command": "npx",
      "args": ["-y", "mcp-language-server", "--lsp", "rust-analyzer"]
    }
  ]
}
```

SWELL will connect to each server, negotiate capabilities, and expose whatever tools the server provides.

## Agent Skills

SWELL supports the [Agent Skills](https://agentskills.io) standard for defining reusable agent capabilities. Skills are discovered from `.swell/skills/` and can be added by users.

### User-Extensible

Users can add new skills to `.swell/skills/` - no code changes needed. SWELL discovers skills at startup.

### Skill Directory Structure

```
.swell/skills/          # User-extensible - add skills here!
├── rust-coding/
│   └── SKILL.md
├── test-writing/
│   └── SKILL.md
├── code-review/
│   └── SKILL.md
└── refactoring/
    └── SKILL.md
```

### SKILL.md Format

```yaml
---
name: my-custom-skill
description: What this skill does and when to use it.
---
# Instructions in Markdown
```

### Progressive Disclosure

1. **Startup**: Load skill `name` and `description` for all skills
2. **Activation**: Load full `SKILL.md` body when skill is needed
3. **Resources**: Load `scripts/`, `references/`, `assets/` on demand

### Included Skills (Defaults)

These come with SWELL but can be replaced or extended:

| Skill | Purpose |
|-------|---------|
| `rust-coding` | Idiomatic Rust patterns, ownership, async |
| `test-writing` | Unit tests, mocking, async tests |
| `code-review` | Review checklist, clippy, security |
| `refactoring` | Safe refactoring patterns, strangler fig |

## Cross-Crate Dependency Graph

The workspace follows a layered architecture where `swell-core` is the foundation that all other crates depend on.

```
                    ┌─────────────────────┐
                    │     swell-core      │  (Base: types, traits, errors)
                    │   (no dependencies) │
                    └──────────┬──────────┘
                               │
          ┌────────────────────┼────────────────────┐
          │                    │                    │
          ▼                    ▼                    ▼
   ┌─────────────┐      ┌─────────────┐      ┌─────────────┐
   │  swell-llm  │      │ swell-state │      │ swell-tools │
   └──────┬──────┘      └──────┬──────┘      └──────┬──────┘
          │                   │                   │
          │           ┌───────┴───────┐           │
          │           │               │           │
          ▼           ▼               │           │
   ┌─────────────┐ ┌───────────┐      │           │
   │swell-validation│ │swell-memory│     │           │
   └──────┬──────┘ └───────────┘      │           │
          │                            │           │
          └────────────┬───────────────┘           │
                       │                           │
                       ▼                           │
              ┌─────────────────┐                   │
              │swell-orchestrator│                  │
              └────────┬────────┘                   │
                       │                           │
          ┌────────────┼────────────┐              │
          │                         │              │
          ▼                         ▼              │
   ┌─────────────┐           ┌─────────────┐      │
   │swell-daemon│           │swell-benchmark│     │
   └─────────────┘           └─────────────┘      │
                                          │
                                          ▼
                                   ┌─────────────┐
                                   │  swell-cli  │
                                   └─────────────┘

   Independent crate (no internal deps):
   ┌─────────────┐
   │swell-sandbox│
   └─────────────┘
```

### Dependency Summary

| Crate | Dependencies | Role |
|-------|-------------|------|
| `swell-core` | None | Foundation: types, traits, errors, events |
| `swell-llm` | swell-core | LLM backend implementations |
| `swell-state` | swell-core | State management, checkpoints |
| `swell-memory` | swell-core | SQLite-backed memory system |
| `swell-tools` | swell-core | File, git, shell, search, MCP tools |
| `swell-sandbox` | swell-core | Process isolation (cross-platform) |
| `swell-skills` | swell-core | Agent skills loader |
| `swell-validation` | swell-core, swell-llm | Lint, test, security gates |
| `swell-orchestrator` | swell-core, swell-llm, swell-state, swell-tools, swell-validation | Task coordination |
| `swell-daemon` | swell-core, swell-orchestrator | HTTP/WS server for task management |
| `swell-benchmark` | swell-core, swell-orchestrator, swell-llm, swell-tools, swell-validation, swell-state | Evaluation suite |
| `swell-cli` | swell-core | CLI client binary |

## Error Handling Conventions

SWELL uses a two-tier error handling strategy with `thiserror` for domain-specific errors and `anyhow` for context-rich error handling.

### Domain Errors with `thiserror`

Use `thiserror` for errors that represent specific failure modes in your domain:

```rust
use thiserror::Error;

#[derive(Error, Debug)]
pub enum MyError {
    #[error("Resource {0} not found")]
    NotFound(Uuid),

    #[error("Invalid operation: {0}")]
    InvalidOperation(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}
```

**When to use `thiserror`:**
- Define the primary error type for a crate
- When errors need to implement `std::error::Error`
- When you want exhaustive pattern matching on error variants

### Context Errors with `anyhow`

Use `anyhow` for operations that may fail in many ways and where context matters:

```rust
use anyhow::{Context, Result};

fn load_config() -> Result<Config> {
    let contents = std::fs::read_to_string("config.json")
        .context("Failed to read config file")?;
    serde_json::from_str(&contents)
        .context("Failed to parse config JSON")?
}
```

**When to use `anyhow`:**
- In `fn main()` and test functions
- At API boundaries where you want rich error messages
- When errors should be user-friendly and informative

### Central Error Type

`swell-core/src/error.rs` defines `SwellError` - the canonical error type for the system:

```rust
pub enum SwellError {
    TaskNotFound(Uuid),
    AgentNotFound(Uuid),
    InvalidStateTransition(String),
    ToolExecutionFailed(String),
    SandboxError(String),
    LlmError(String),
    DatabaseError(String),
    BudgetExceeded(String),
    PermissionDenied(String),
    DoomLoopDetected,
    KillSwitchTriggered,
    ConfigError(String),
    IoError(#[from] std::io::Error),
}
```

### Error Propagation Patterns

1. **Prefer `?` operator** for automatic error conversion
2. **Use `context()`** from `anyhow` to add context at failure points
3. **Implement `From`** for conversion between error types
4. **Serialize errors** with `serde` when crossing network boundaries

```rust
// Good: Rich context on failure
async fn fetch_data(url: &str) -> Result<Data> {
    reqwest::get(url)
        .await
        .context("Failed to fetch data from {url}")?
        .json::<Data>()
        .await
        .context("Failed to deserialize response")?
}

// Good: Domain error with source
#[derive(Error, Debug)]
pub enum StoreError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Entity {id} not found")]
    NotFound { id: Uuid },
}
```

## Async Patterns

SWELL uses Tokio as the async runtime with consistent patterns across all crates.

### Runtime Configuration

All async code runs on Tokio with full features:

```toml
# Cargo.toml workspace.dependencies
tokio = { version = "1", features = ["full"] }
```

### Entry Points

Use `#[tokio::main]` for binaries and `#[tokio::test]` for tests:

```rust
// Binary entry point
#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt::init();
    run().await
}

// Test entry point
#[tokio::test]
async fn test_something() {
    let result = do_something().await;
    assert!(result.is_ok());
}
```

### Send + Sync Bounds

All types that cross `.await` points or are shared between tasks must be `Send + Sync`:

```rust
// Good: Stateless service
struct MyService {
    // Fields are Send + Sync (built-in types or explicitly bounded)
}

// Good: Shared state with proper bounds
impl State {
    pub async fn update(&self, value: String) -> Result<()> {
        let guard = self.data.lock().await;
        // ...
    }
}

// Caution: Arc<Mutex<T>> for shared mutable state
let shared: Arc<Mutex<HashMap<K, V>>> = Arc::new(Mutex::new(HashMap::new()));
```

### Async Traits

Use `async-trait` for async methods in traits:

```rust
use async_trait::async_trait;

#[async_trait]
pub trait LlmBackend: Send + Sync {
    async fn complete(&self, prompt: &str) -> Result<String>;
    async fn complete_streaming(&self, prompt: &str) -> Result<Vec<String>>;
}
```

### Spawning Tasks

Use `tokio::spawn` for fire-and-forget tasks and `spawn_blocking` for CPU-bound work:

```rust
// I/O-bound work: stay in async context
let handle = tokio::spawn(async move {
    read_file().await
});

// CPU-bound work: move to blocking thread pool
let result = tokio::task::spawn_blocking(|| {
    cpu_intensive_computation()
}).await?;
```

### Channel Patterns

Use Tokio channels for task-to-task communication:

```rust
use tokio::sync::mpsc;
use tokio::sync::watch;

// For multiple producers, single consumer: mpsc
let (tx, mut rx) = mpsc::channel::<Event>(100);

// For broadcast to many consumers: broadcast
use tokio::sync::broadcast;
let (tx, _rx) = broadcast::channel::<State>(10);
```

### Common Pitfalls

1. **Blocking in async context** - Always use `spawn_blocking` for blocking operations
2. **Missing `Send` bounds** - Ensure types shared across tasks implement `Send`
3. **Silent task drops** - Store `JoinHandle` to await task completion
4. **Deadlocks** - Avoid holding locks across `.await` points; use `Mutex` (not `RwLock`) carefully

## v2 Roadmap (Based on Research Documents)

These features are planned based on detailed specifications in `plan/research_documents/`:

### Knowledge Graph
Property graph with typed nodes and edges for code structure.
- Nodes: File, Module, Class, Function, Variable
- Edges: CALLS, INHERITS_FROM, IMPORTS, DEPENDS_ON
- Reference: `Memory and Learning Architecture.md`

### Vector Search
LanceDB integration with code embeddings for semantic search.
- Voyage Code-3 embeddings (97.3% MRR)
- Semantic code chunking via Tree-sitter
- Reference: `Technical Architecture and Roadmap Spec.md`

### Tree-sitter AST Parsing
AST-based code analysis for dependency tracking and chunking.
- Semantic boundaries: functions, classes, methods
- Code-test graph building
- Reference: All research documents

### Firecracker Sandbox
MicroVM-based isolation for production workloads.
- <125ms startup, <5 MiB memory overhead
- Hardware virtualization isolation
- Reference: `Tools and Runtime Control Spec.md`

### OpenTelemetry Integration
Full OTel traces with GenAI semantic conventions.
- `gen_ai.*` attributes
- Langfuse export for observability
- Reference: `Tools and Runtime Control Spec.md`

### Hierarchical Agents
Feature Lead sub-orchestrators for large-scale projects.
- Dynamic spawning (max 3 levels)
- Planner-Worker pattern
- Reference: `Orchestrator and Execution Design Spec.md`

## Testing Guidelines

- Unit tests per crate (`#[cfg(test)]` modules)
- Integration tests in `tests/` directory
- Mock LLM backends for reproducible testing
- Benchmark suite for performance evaluation

## Commit & Pull Request Guidelines

- Conventional commits: `feat:`, `fix:`, `docs:`, `refactor:`, `test:`
- PRs should reference the relevant spec document in `plan/research_documents/`
- All commits must pass lint and type checks before merge
- Test coverage should not decrease

## Research Documents

Detailed specifications for each subsystem:

| Document | Purpose |
|----------|---------|
| `Technical Architecture and Roadmap Spec.md` | Overall architecture and roadmap |
| `Memory and Learning Architecture.md` | Memory system design |
| `Orchestrator and Execution Design Spec.md` | Orchestration design |
| `Product definition and UX strategy.md` | Product vision and UX |
| `Testing and Validation Research Spec.md` | Testing strategy |
| `Tools and Runtime Control Spec.md` | Tools and sandbox design |

Detailed architecture documentation is available in `plan/Autonomous Coding Engine.md`.

## Available Skills (.factory/skills/)

SWELL uses these skills from `~/.factory/skills/`:

| Skill | Purpose | Used By |
|-------|---------|---------|
| `rust` | Idiomatic Rust patterns, ownership, async | Mission workers |
| `rust-patterns` | Production patterns (Tokio, Axum, SQLx) | Advanced features |
| `rust-code-review` | Ownership/borrowing code review | Validation |
| `git` | Branches, commits, rebases, PRs | Git operations |
| `agentic-coding` | PACT protocol, red-green loops | Agent workflow |
| `agent-team-orchestration` | Multi-agent coordination | Orchestrator |
| `task-development-workflow` | TDD, PR workflow | Development |
| `prompt-engineering-expert` | LLM prompt optimization | LLM integration |

See `~/.factory/skills/*/SKILL.md` for full skill documentation.
