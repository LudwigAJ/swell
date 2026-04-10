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
Core types, traits, and state machine definitions. All other crates depend on this.

**Key Modules:**
- `types.rs` - Task, Agent, State enums
- `traits.rs` - Agent, Tool, Validator traits
- `events.rs` - Event emitter for observability

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

### swell-llm
LLM backend implementations with model routing.

**Backends:**
- `anthropic.rs` - Anthropic Claude API
- `openai.rs` - OpenAI API
- `mock.rs` - Mock backend for testing

### swell-tools
Tool implementations for code operations.

**Tools:**
- `file.rs` - Read, write, edit files
- `git.rs` - Git operations (status, diff, commit, branch)
- `shell.rs` - Shell command execution
- `search.rs` - Grep, glob for codebase exploration
- `registry.rs` - Tool registry and permission tiers
- `mcp.rs` - MCP client for external tools

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

### swell-sandbox
Sandbox isolation for tool execution (stub for MVP).

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

### Build
```bash
cargo build --workspace
```

### Run Tests
```bash
cargo test --workspace
```

### Run Specific Crate Tests
```bash
cargo test -p swell-orchestrator
cargo test -p swell-memory
```

### Lint
```bash
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

SWELL uses the Model Context Protocol (MCP) to integrate with external tools and servers. MCP uses JSON-RPC 2.0 over stdio for communication.

### MCP Client Implementation

The MCP client is in `swell-tools/src/mcp.rs` and implements:

- **Transport**: JSON-RPC 2.0 over stdio (subprocess)
- **Discovery**: `tools/list` for finding available tools
- **Invocation**: `tools/call` for executing tools
- **Deferred Loading**: Lazy tool loading for performance

### MCP Server Integration

SWELL integrates with these MCP servers:

| Server | Purpose | Tools Available |
|--------|---------|----------------|
| `mcp-server-tree-sitter` | AST parsing, code analysis | `get_ast`, `run_query`, `find_usage`, `analyze_project` |
| `mcp-language-server` + `rust-analyzer` | Rust code intelligence | `definition`, `references`, `diagnostics`, `hover` |
| `mcp-language-server` + `clangd` | C/C++ intelligence | `definition`, `references`, `diagnostics` |

### MCP Configuration

Configure MCP servers in `.swell/mcp_servers.json`:

```json
{
  "servers": [
    {
      "name": "tree-sitter",
      "command": "python3",
      "args": ["-m", "mcp_server_tree_sitter"],
      "env": {
        "MCP_TS_CACHE_MAX_SIZE_MB": "256"
      }
    },
    {
      "name": "rust-analyzer",
      "command": "mcp-language-server",
      "args": ["--lsp", "rust-analyzer"]
    }
  ]
}
```

### MCP Tool Schema

MCP tools use JSON Schema for input validation:

```json
{
  "name": "tool_name",
  "description": "What this tool does",
  "inputSchema": {
    "type": "object",
    "properties": {
      "param_name": {
        "type": "string",
        "description": "Parameter description"
      }
    },
    "required": ["param_name"]
  }
}
```

### Using MCP Tools in Agents

```rust
use swell_tools::mcp::{McpClient, McpManager};

// Connect to a server
let manager = McpManager::new();
manager.add_server("tree-sitter".into(), "python3 -m mcp_server_tree_sitter".into()).await?;

// List and call tools
let tools = manager.list_all_tools().await;
let result = client.call_tool("get_ast", json!({"path": "src/main.rs"})).await?;
```

## Critical Missing Features (v2 Roadmap)

Based on the research documents, the following are planned for future versions:

### Knowledge Graph
Property graph with typed nodes and edges for code structure analysis.

### Vector Search
LanceDB integration with code embeddings for semantic search.

### Tree-sitter AST Parsing
AST-based code analysis for dependency tracking and chunking.

### Firecracker Sandbox
MicroVM-based isolation for production workloads.

### OpenTelemetry Integration
Full OTel traces with GenAI semantic conventions.

### Hierarchical Agents
Feature Lead sub-orchestrators for large-scale projects.

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
