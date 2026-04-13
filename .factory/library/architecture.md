# SWELL System Architecture

## 1. System Overview

SWELL is an autonomous coding engine built in Rust. It accepts a natural-language task description (e.g., "add pagination to the users endpoint"), then **plans**, **executes**, **tests**, and **validates** the implementation end-to-end without human intervention.

The system is structured as a Rust workspace of 12 crates, a Unix-socket daemon, and a CLI client. It uses a multi-agent architecture where specialized agents (planner, coder, reviewer, etc.) collaborate through an orchestrator that drives a task state machine.

**Core capabilities:**
- LLM-powered planning and code generation (Anthropic, OpenAI)
- File, git, shell, and search tool execution with permission tiers
- Multi-gate validation (lint, test, security, AI review)
- SQLite-backed memory with pattern learning and recall
- MCP protocol integration for external tool servers
- User-extensible skill system

---

## 2. Crate Dependency Graph

```
                          ┌──────────────┐
                          │  swell-core  │  ◄── Foundation: traits, types, events
                          └──────┬───────┘
                                 │
            ┌────────────┬───────┼────────┬─────────────┬──────────────┐
            │            │       │        │             │              │
            ▼            ▼       ▼        ▼             ▼              ▼
      ┌──────────┐ ┌─────────┐ ┌───────┐ ┌──────────┐ ┌──────────┐ ┌──────────┐
      │swell-llm │ │swell-   │ │swell- │ │swell-    │ │swell-    │ │swell-    │
      │          │ │tools    │ │state  │ │memory    │ │sandbox   │ │skills    │
      └────┬─────┘ └────┬────┘ └───┬───┘ └──────────┘ └──────────┘ └──────────┘
           │            │          │
           ▼            ▼          ▼
      ┌────────────────────────────────┐
      │       swell-validation         │  ◄── depends on: core, llm
      └────────────────┬───────────────┘
                       │
                       ▼
      ┌────────────────────────────────┐
      │       swell-orchestrator       │  ◄── depends on: core, llm, tools, state, validation
      └────────────────┬───────────────┘
                       │
                       ▼
      ┌────────────────────────────────┐
      │         swell-daemon           │  ◄── depends on: core, orchestrator
      └────────────────────────────────┘
                       ▲
                       │ (IPC over Unix socket)
      ┌────────────────────────────────┐
      │          swell-cli             │  ◄── depends on: core
      └────────────────────────────────┘

      ┌────────────────────────────────┐
      │       swell-benchmark          │  ◄── depends on: core, orchestrator, llm, tools,
      └────────────────────────────────┘      validation, state (evaluation harness)
```

**Dependency summary (each crate → its internal deps):**

| Crate | Depends On |
|-------|-----------|
| `swell-core` | *(none — leaf crate)* |
| `swell-llm` | core |
| `swell-tools` | core |
| `swell-state` | core |
| `swell-memory` | core |
| `swell-sandbox` | core |
| `swell-skills` | core |
| `swell-validation` | core, llm |
| `swell-orchestrator` | core, llm, tools, state, validation |
| `swell-daemon` | core, orchestrator |
| `swell-cli` | core |
| `swell-benchmark` | core, orchestrator, llm, tools, validation, state |

---

## 3. Execution Flow — Task Lifecycle

Every task moves through a state machine managed by the orchestrator:

```
  ┌─────────┐     ┌──────────┐     ┌──────────┐     ┌───────────┐     ┌────────────┐     ┌───────────┐
  │ CREATED  │────►│ PLANNING │────►│ APPROVED │────►│ EXECUTING │────►│ VALIDATING │────►│ COMPLETED │
  └─────────┘     └──────────┘     └──────────┘     └───────────┘     └────────────┘     └───────────┘
                       │                │                  │                 │
                       │                │                  │                 │
                       ▼                ▼                  ▼                 ▼
                   ┌────────┐      ┌──────────┐      ┌────────┐       ┌────────┐
                   │ FAILED │      │ CANCELLED│      │ PAUSED │       │ FAILED │
                   └────────┘      └──────────┘      └────────┘       └────────┘
```

**Phase breakdown:**

| Phase | Agent | What Happens |
|-------|-------|-------------|
| **CREATED** | — | CLI sends `TaskCreate` to daemon; task is registered |
| **PLANNING** | PlannerAgent | LLM generates a structured plan (steps, files, tests) |
| **APPROVED** | — | Plan is accepted (auto or manual depending on autonomy level) |
| **EXECUTING** | GeneratorAgent / CoderAgent | Agents execute plan steps using tools (file write, git, shell) |
| **VALIDATING** | EvaluatorAgent | Validation pipeline runs gates in sequence (lint → test → security → AI review) |
| **COMPLETED** | — | Feature branch with commits; PR description generated |

Failure at any phase transitions the task to **FAILED** with error context preserved for retry or human intervention.

---

## 4. Key Subsystems

### 4.1 Orchestrator (`swell-orchestrator`)

The central coordinator that drives the entire task lifecycle.

- **State Machine** — Enforces valid state transitions; each task has exactly one active state at any time.
- **Scheduler** — Manages a task queue with dependency tracking; determines execution order.
- **Policy Engine** — Evaluates deny-first policy rules (from `.swell/policies/`) before allowing actions.
- **Autonomy Controller** — Governs how much the system can do without human approval (e.g., auto-approve plans vs. require manual review).
- **Agent Pool** — Houses specialized agents (PlannerAgent, GeneratorAgent, EvaluatorAgent, CoderAgent, ReviewerAgent, RefactorerAgent), each implementing the `Agent` trait from core.
- **Backlog** — Aggregates work items across tasks for prioritization.

### 4.2 LLM Integration (`swell-llm`)

Abstracts all LLM communication behind the `LlmBackend` trait.

- **Backends** — `AnthropicBackend` (Claude), `OpenAIBackend` (GPT), `MockLlm` (deterministic testing).
- **Model Routing** — Configuration in `.swell/models.json` maps agent roles to specific models (e.g., planner → claude-sonnet, coder → claude-opus).
- **Streaming** — Supports streaming responses for real-time output.
- **Cost Tracking** — Every LLM call records token usage for budgeting and observability.

### 4.3 Tool Runtime (`swell-tools`)

Provides the hands and feet for agents to interact with the codebase.

- **Tool Registry** — Central registry where tools are registered with metadata and permission tiers.
- **Built-in Tools** — File (read/write/edit), Git (status/diff/commit/branch), Shell (command execution), Search (grep/glob).
- **Permission Tiers** — Tools are classified by risk level; the policy engine must approve before execution.
- **MCP Client** — Connects to external MCP servers (tree-sitter, rust-analyzer, eslint) via JSON-RPC over stdio; discovers tools dynamically at runtime through capability negotiation.

### 4.4 Validation Layer (`swell-validation`)

Ensures every output meets quality standards before a task is marked complete.

- **Gates** — Each gate implements the `ValidationGate` trait:
  - `LintGate` — Runs `cargo clippy` and `cargo fmt --check`
  - `TestGate` — Runs `cargo test` for affected crates
  - `SecurityGate` — Security scanning (stub for MVP)
  - `AiReviewGate` — LLM-based code review (stub for MVP)
- **Pipeline** — Gates execute in sequence; a failure at any gate halts the pipeline and reports back to the orchestrator.
- **Staged Execution** — Validation can run incrementally during execution, not just at the end.

### 4.5 Memory System (`swell-memory`)

Persists context across sessions so the system learns and improves.

- **SQLite Store** — Primary persistence backend for memory blocks.
- **Memory Blocks** — Structured units of context (code snippets, decisions, errors, patterns).
- **Recall** — Retrieval system that surfaces relevant memories for a given task context.
- **Skill Extraction** — Extracts reusable patterns from successful task trajectories.
- **Pattern Learning** — Recognizes recurring patterns from feedback to improve future performance.

### 4.6 Daemon / CLI (`swell-daemon`, `swell-cli`)

The user-facing interface and background service.

- **Daemon** — Long-running Unix socket server that manages task lifecycle. Accepts commands (`TaskCreate`, `TaskApprove`, `TaskReject`, `TaskCancel`, `TaskList`, `TaskWatch`) and emits events.
- **CLI** — Thin client that serializes commands and sends them to the daemon over Unix socket IPC. Commands: `swell task`, `swell list`, `swell watch`, `swell approve`, `swell cancel`.
- **Event Streaming** — `TaskWatch` provides real-time state change events to the CLI for live progress display.

---

## 5. Data Flow

End-to-end flow from user command to result:

```
User                CLI              Daemon           Orchestrator        Agents              Tools            Validation
 │                   │                 │                  │                  │                  │                  │
 │── swell task ───►│                 │                  │                  │                  │                  │
 │                   │── TaskCreate ──►│                  │                  │                  │                  │
 │                   │                 │── new task ─────►│                  │                  │                  │
 │                   │                 │                  │── plan ─────────►│ PlannerAgent     │                  │
 │                   │                 │                  │                  │── LLM call ──────┤                  │
 │                   │                 │                  │◄── plan ─────────│                  │                  │
 │                   │                 │                  │                  │                  │                  │
 │                   │                 │                  │── (approve) ─────┤                  │                  │
 │                   │                 │                  │                  │                  │                  │
 │                   │                 │                  │── execute ──────►│ GeneratorAgent   │                  │
 │                   │                 │                  │                  │── LLM call ──────┤                  │
 │                   │                 │                  │                  │── file.write ────►│                  │
 │                   │                 │                  │                  │── git.commit ────►│                  │
 │                   │                 │                  │                  │── shell.run ─────►│                  │
 │                   │                 │                  │◄── done ─────────│                  │                  │
 │                   │                 │                  │                  │                  │                  │
 │                   │                 │                  │── validate ──────┤──────────────────┤────────────────►│
 │                   │                 │                  │                  │                  │   lint gate      │
 │                   │                 │                  │                  │                  │   test gate      │
 │                   │                 │                  │                  │                  │   security gate  │
 │                   │                 │                  │                  │                  │   AI review gate │
 │                   │                 │                  │◄─────────────────┤──────────────────┤── pass/fail ────│
 │                   │                 │                  │                  │                  │                  │
 │                   │                 │◄── completed ────│                  │                  │                  │
 │                   │◄── result ──────│                  │                  │                  │                  │
 │◄── output ────────│                 │                  │                  │                  │                  │
```

**Key data artifacts moving through the pipeline:**

1. **Task description** (string) → daemon → orchestrator
2. **Plan** (structured steps) ← PlannerAgent via LLM
3. **Tool calls** (file writes, git ops, shell commands) → Tool Runtime
4. **Validation results** (pass/fail per gate) ← Validation Pipeline
5. **Memory blocks** (context, patterns) ↔ Memory System (read during planning, written after completion)
6. **Events** (state changes) → daemon → CLI for live progress

---

## 6. Key Invariants

These properties must hold at all times:

| Invariant | Enforced By |
|-----------|-------------|
| **Permission check before every tool execution** — No tool runs without passing through the policy engine and permission tier check. | `swell-tools` registry + `swell-orchestrator` policy engine |
| **Cost tracking on every LLM call** — Token usage is recorded for every request/response pair, no exceptions. | `swell-llm` backend implementations |
| **Valid state transitions only** — Tasks can only move through defined state machine transitions (e.g., cannot jump from CREATED to EXECUTING). | `swell-orchestrator` state machine |
| **Deny-first policy evaluation** — Policy rules default to deny; actions require explicit allow rules. | `swell-orchestrator` policy engine + `.swell/policies/` |
| **Configuration from files, never hardcoded** — All magic numbers, thresholds, and limits come from `.swell/settings.json` or environment variables. | Convention enforced project-wide |
| **Validation before completion** — A task cannot reach COMPLETED without passing through the validation pipeline. | `swell-orchestrator` state machine |
| **Checkpoint persistence on state transitions** — State is checkpointed at each transition so recovery is possible after crashes. | `swell-state` checkpoint store |
| **Trait-based abstraction for all major components** — LLM backends, tools, validators, and memory stores are behind traits for testability and extensibility. | `swell-core` trait definitions |

---

## 7. Integration Points — Cross-Crate Boundaries

These are the key boundaries where crates connect:

| Boundary | Producer → Consumer | Interface |
|----------|-------------------|-----------|
| **Core traits** | `swell-core` → all crates | `Agent`, `Tool`, `ValidationGate`, `LlmBackend`, `MemoryStore`, `CheckpointStore`, `Sandbox` traits |
| **LLM calls from agents** | `swell-orchestrator` agents → `swell-llm` | Agents hold `Arc<dyn LlmBackend>` and call `generate()` / `stream()` |
| **Tool execution from agents** | `swell-orchestrator` agents → `swell-tools` | Agents invoke tools via `ToolRegistry` which checks permissions then delegates to tool impls |
| **Validation from orchestrator** | `swell-orchestrator` → `swell-validation` | Orchestrator calls `ValidationPipeline::run()` after execution phase |
| **AI review uses LLM** | `swell-validation` → `swell-llm` | `AiReviewGate` calls `LlmBackend` for code review judgments |
| **State persistence** | `swell-orchestrator` → `swell-state` | Orchestrator checkpoints task state on every transition via `CheckpointStore` |
| **Daemon wraps orchestrator** | `swell-daemon` → `swell-orchestrator` | Daemon creates and drives the orchestrator; translates IPC commands into orchestrator method calls |
| **CLI talks to daemon** | `swell-cli` → `swell-daemon` | Serialized commands over Unix socket; CLI uses `swell-core` types for command/response structures |
| **MCP extends tools** | External MCP servers → `swell-tools` | `McpToolWrapper` in swell-tools connects to external servers and exposes discovered tools through the standard `Tool` trait |
| **Skills extend agents** | `.swell/skills/` → `swell-skills` → orchestrator | Skills are loaded at startup; their instructions are injected into agent prompts when activated |
| **Benchmark harness** | `swell-benchmark` → orchestrator, llm, tools, validation, state | Benchmark drives the full stack with curated tasks and measures outcomes |
