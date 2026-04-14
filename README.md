# SWELL - Autonomous Coding Engine

An autonomous coding agent built in Rust that doesn't stop until the job is done.

## Overview

SWELL is an autonomous coding engine designed to handle software engineering tasks end-to-end: planning, execution, testing, and validation. It uses a multi-agent architecture with specialized agents for different phases of the development workflow.

**Key capabilities:**
- **Multi-Agent System**: Planner, Generator, Evaluator, Reviewer, and Refactorer agents
- **Task Orchestration**: DAG-based task management with dependency tracking and state machine
- **Tool Runtime**: File, git, shell, and search tools with permission tiers and sandbox isolation
- **LLM Integration**: Anthropic Claude and OpenAI GPT backends with streaming support
- **Validation Pipeline**: Lint, test, security, and AI review gates
- **MCP Protocol**: Model Context Protocol client for external tool integration
- **Persistent Memory**: SQLite-backed memory system with pattern learning

---

## Installation

### Prerequisites

- **Rust 1.94+**: Install via [rustup](https://rustup.rs/)
  ```bash
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
  ```
- **SQLite**: Included via `sqlx` (no separate installation needed)
- **Git**: Required for repository operations and git-based tools

### Build from Source

```bash
# Clone the repository
git clone https://github.com/factory/swell.git
cd swell

# Build the workspace
cargo build --workspace

# Run all tests
cargo test --workspace -- --test-threads=4
```

### Environment Variables

Set your Anthropic API key for Claude integration:

```bash
export ANTHROPIC_API_KEY="sk-ant-..."
```

For OpenAI integration:

```bash
export OPENAI_API_KEY="sk-..."
```

---

## Quickstart

### 1. Start the Daemon

In one terminal, start the SWELL daemon:

```bash
cargo run --bin swell-daemon
```

The daemon runs as a Unix socket server (default: `/tmp/swell.sock`).

### 2. Create a Task

In another terminal, create a new coding task:

```bash
cargo run --bin swell -- task "Add unit tests for the auth module"
```

The CLI connects to the daemon via Unix socket and returns a task ID.

### 3. Watch Task Progress

Monitor a task's execution in real-time:

```bash
cargo run --bin swell -- watch <task-id>
```

### 4. Approve, Reject, or Cancel

```bash
# Approve a task (allows execution to proceed)
cargo run --bin swell -- approve <task-id>

# Reject a task
cargo run --bin swell -- reject <task-id>

# Cancel a running task
cargo run --bin swell -- cancel <task-id>
```

### 5. List Tasks

```bash
# List all tasks
cargo run --bin swell -- list

# Filter by status
cargo run --bin swell -- list --status executing
```

---

## End-to-End Workflow

This example demonstrates the complete task lifecycle: creation → planning → approval → execution → validation → completion.

### Step 1: Create the Task

```bash
cargo run --bin swell -- task "Refactor the user authentication in crates/swell-core/src/auth.rs"
```

Output:
```
Task created: a1b2c3d4-e5f6-7890-abcd-ef1234567890
Status: CREATED
```

### Step 2: Watch the Planning Phase

```bash
cargo run --bin swell -- watch a1b2c3d4-e5f6-7890-abcd-ef1234567890
```

The PlannerAgent analyzes the codebase, understands the task requirements, and generates a plan. Task transitions to `PLANNING`.

### Step 3: Review and Approve the Plan

Once planning completes, the task enters `AWAITING_APPROVAL`. Review the plan:

```bash
cargo run --bin swell -- list
```

To approve and begin execution:

```bash
cargo run --bin swell -- approve a1b2c3d4-e5f6-7890-abcd-ef1234567890
```

Task transitions to `EXECUTING`. The GeneratorAgent reads the auth module, makes the necessary refactoring changes, and the EvaluatorAgent runs validation gates (lint, tests, AI review).

### Step 4: Validation Gates Run

During `EXECUTING`, the EvaluatorAgent runs validation after each code change:

1. **Lint Gate**: `cargo clippy` and `cargo fmt --check`
2. **Test Gate**: `cargo test -p <crate>`
3. **AI Review Gate**: Claude reviews the code changes for correctness and style

### Step 5: Task Completion

After all validation gates pass, the task transitions to `COMPLETED`:

```
Task completed: a1b2c3d4-e5f6-7890-abcd-ef1234567890
Duration: 45.2s
Cost: $0.0234
```

If validation fails, the task transitions to `FAILED` with detailed error output.

---

## Crate Overview

SWELL is organized as a Rust workspace with 12 crates:

| Crate | Path | Description |
|-------|------|-------------|
| **swell-core** | `crates/swell-core` | Core types, traits, and state machine definitions. All other crates depend on this. Provides `Task`, `Agent`, `State` enums, event emitter, and foundational traits. |
| **swell-orchestrator** | `crates/swell-orchestrator` | Task orchestration, scheduling, policy engine, and autonomy controller. Coordinates the main execution loop with planner, generator, evaluator, reviewer, and refactorer agents. |
| **swell-llm** | `crates/swell-llm` | LLM backend implementations. `AnthropicBackend` for Claude API, `OpenAIBackend` for GPT API, `MockLlm` for testing. Includes SSE streaming, token tracking, and retry logic. |
| **swell-tools** | `crates/swell-tools` | Tool implementations: file operations, git operations, shell execution, search (grep/glob), MCP client, LSP tools, permission system, and tool registry. |
| **swell-validation** | `crates/swell-validation` | Validation gates and pipelines. `LintGate` (clippy/format), `TestGate` (cargo test), `SecurityGate` (stub), `AiReviewGate` (LLM-based review), and `ValidationOrchestrator`. |
| **swell-memory** | `crates/swell-memory` | Persistent memory system with SQLite store. Memory blocks for context assembly, recall for retrieval, skill extraction, and pattern learning from agent trajectories. |
| **swell-state** | `crates/swell-state` | State management and checkpoint persistence. Task state machine transitions, checkpoint store for crash recovery, and file-based state observability (`.swell/task-state.json`). |
| **swell-sandbox** | `crates/swell-sandbox` | Sandbox isolation for tool execution. Stub implementation for MVP with microVM-based isolation planned (Firecracker). |
| **swell-skills** | `crates/swell-skills` | Skill discovery and execution. Loads user-extensible skills from `.swell/skills/` directory. Skills are wrapped as tools via `SkillTool` adapter. |
| **swell-daemon** | `crates/swell-daemon` | Daemon server running as a Unix socket server. Handles `TaskCreate`, `TaskApprove`, `TaskReject`, `TaskCancel`, `TaskList`, `TaskWatch` commands. Emits `DaemonEvent` stream. |
| **swell-benchmark** | `crates/swell-benchmark` | Benchmark suite for evaluation. 50 curated benchmark tasks, metrics aggregation, and progress tracking. |
| **swell-cli** | `clients/swell-cli` | CLI client for daemon interaction. Commands: `swell task`, `swell list`, `swell watch`, `swell approve`, `swell reject`, `swell cancel`. |

---

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                         SWELL                                │
├─────────────────────────────────────────────────────────────┤
│  ┌─────────────┐          ┌─────────────────────┐          │
│  │  swell-cli  │◄────────►│     swell-daemon    │          │
│  │   (client)  │  Unix    │  (Unix socket srv)  │          │
│  └─────────────┘  Socket  └──────────┬──────────┘          │
│                                       │                     │
│                            ┌──────────▼──────────┐          │
│                            │  swell-orchestrator  │          │
│                            │  (task coordination) │          │
│                            └──────────┬──────────┘          │
│                                       │                     │
│         ┌─────────────────────────────┼───────────────────┐│
│         │           agents            │                   ││
│         │  ┌─────────┐ ┌──────────┐ ┌──▼──────┐ ┌────────┐ ││
│         │  │ Planner│ │Generator │ │Evaluator│ │Reviewer│ ││
│         │  └─────────┘ └──────────┘ └─────────┘ └────────┘ ││
│         └─────────────────────────────┼───────────────────┘│
│                                       │                     │
│         ┌─────────────────────────────┼───────────────────┐ │
│         │          tools              │                   │ │
│         │  file │ git │ shell │ search │ MCP │ LSP        │ │
│         └─────────────────────────────┼───────────────────┘ │
│                                       │                     │
│                            ┌──────────▼──────────┐          │
│                            │   swell-validation  │          │
│                            │ (lint│test│review) │          │
│                            └──────────┬──────────┘          │
│                                       │                     │
│         ┌─────────────────────────────┼───────────────────┐ │
│         │         memory             │                   │ │
│         │  SQLite store, recall, patterns, skills        │ │
│         └─────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────────┘
```

### State Machine

Tasks follow this state progression:

```
CREATED → PLANNING → APPROVED → EXECUTING → VALIDATING → COMPLETED
                ↓                                    ↓
            REJECTED                             FAILED
                ↓                                    ↓
            CANCELLED                           PAUSED
```

---

## Troubleshooting

### Issue: "Connection refused" when running CLI commands

**Symptom**: `Error: connection refused` or `No such file or directory` when running `swell task`, `swell list`, etc.

**Cause**: The daemon is not running.

**Solution**: Start the daemon in a separate terminal:
```bash
cargo run --bin swell-daemon
```
The daemon must be running before CLI commands will work.

---

### Issue: Build fails with "unknown feature `rustls-tls`"

**Symptom**: `cargo build` fails with feature not found error.

**Cause**: Rust version is below 1.94.

**Solution**: Update Rust to 1.94 or later:
```bash
rustup update
rustup default stable
rustc --version  # should be 1.94+
```

---

### Issue: "API key not found" errors

**Symptom**: LLM requests fail with authentication errors.

**Cause**: Missing API key environment variable.

**Solution**: Set the appropriate environment variable:

```bash
# For Anthropic/Claude
export ANTHROPIC_API_KEY="sk-ant-..."

# For OpenAI
export OPENAI_API_KEY="sk-..."
```

Alternatively, configure in `.swell/settings.json`:
```json
{
  "llm": {
    "anthropic_api_key": "${ANTHROPIC_API_KEY}"
  }
}
```

---

### Issue: Clippy warnings treated as errors

**Symptom**: `cargo clippy --workspace -- -D warnings` fails due to new warnings.

**Cause**: Code introduced lint warnings.

**Solution**: Fix the warnings. Common patterns:
- Add `#[allow(unused_variables)]` if variable is intentionally unused
- Use `_` prefix for intentionally unused variables: `let _unused = ...`
- Add missing doc comments on public items: `/// Description here`

---

### Issue: Tests hang or timeout

**Symptom**: `cargo test -p swell-orchestrator` exits with code 124 or never produces output.

**Cause**: `swell-orchestrator` is a large crate (~42k lines, heavy deps). Even with a warm
incremental build cache, compiling and linking takes **60–120 seconds** on macOS ARM64.
Exit code 124 means the timeout fired during compilation — it is not a test failure.

**Solution**: Use a sufficient timeout:
```bash
# Single crate (minimum 180 s)
timeout 180 cargo test -p swell-orchestrator -- <filter> --test-threads=4

# Workspace-wide (minimum 300 s)
timeout 300 cargo test --workspace -- --test-threads=4
```

For genuinely hanging async tests (missing `.await`, infinite loop), add `--test-threads=1`
and check for missing `.await` calls.

---

## Development

### Adding a New Tool

1. Implement `Tool` trait in `crates/swell-tools/src/`:
```rust
use swell_core::tools::{Tool, ToolResult, ToolSpec};

pub struct MyTool;

impl Tool for MyTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "my_tool".into(),
            description: "Description of my tool".into(),
            required_permission: PermissionMode::Ask,
            input_schema: serde_json::json!({}),
        }
    }

    async fn execute(&self, params: serde_json::Value) -> ToolResult {
        // Implementation
        ToolResult::ok(serde_json::json!({"result": "done"}))
    }
}
```

2. Register in `swell-tools/src/registry.rs`.

### Running Crate-Specific Tests

```bash
# Test a specific crate
cargo test -p swell-core -- --test-threads=4

# Run clippy on a specific crate
cargo clippy -p swell-llm -- -D warnings

# Check formatting
cargo fmt -p swell-tools -- --check
```

### Code Style

```bash
# Format all code
cargo fmt --all

# Run all lints
cargo clippy --workspace -- -D warnings
```

### Build Cache Management

The `target/` directory holds compiled artifacts and can grow large over time. Each unique
compilation environment (different flags, env vars, or Cargo profile) creates a new incremental
session directory — old ones are never automatically deleted by Cargo.

**Expected sizes after a clean rebuild:**

| Directory | Expected size |
|---|---|
| `target/debug/` | ~2–4 GB |
| `target/debug/incremental/` | ~1 GB |
| `target/release/` | ~2 GB (only after `--release` build) |

**If `target/` grows beyond ~10 GB**, stale incremental sessions have accumulated.
To reset to a minimal state:

```bash
# Wipe everything and rebuild from scratch (~50 s on macOS ARM64)
cargo clean && cargo build --workspace
```

To clean only the stale incremental data (faster, preserves compiled deps):

```bash
# Remove only incremental session data (~1–30 GB when stale)
rm -rf target/debug/incremental target/release/incremental
```

**Do not** set `CARGO_INCREMENTAL=0` or run `cargo clean` before every build — it destroys
the dependency cache that makes incremental rebuilds fast.

---

## Research & Specifications

Detailed specifications are in `plan/research_documents/`:

| Document | Purpose |
|----------|---------|
| `Autonomous Coding Engine.md` | Master specification |
| `Technical Architecture and Roadmap Spec.md` | Overall architecture and roadmap |
| `Memory and Learning Architecture.md` | Memory system design |
| `Orchestrator and Execution Design Spec.md` | Orchestration design |
| `Product definition and UX strategy.md` | Product vision and UX |
| `Testing and Validation Research Spec.md` | Testing strategy |
| `Tools and Runtime Control Spec.md` | Tools and sandbox design |

---

## License

MIT
