# SWELL System Architecture

## 1. System Overview

SWELL is an autonomous coding engine built in Rust. It accepts a natural-language task description, then **plans**, **executes**, **tests**, and **validates** the implementation end-to-end without human intervention.

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
      │          │ │state    │ │memory │ │tools     │ │sandbox   │ │skills    │
      └────┬─────┘ └────┬────┘ └───┬────┘ └────┬─────┘ └──────────┘ └──────────┘
           │            │          │           │
           ▼            ▼          ▼           │
      ┌────────────────────────┐  ┌────────────┴────────────┐
      │   swell-orchestrator  │  │      swell-validation   │
      │  (Task coordination)   │  │   (Lint, test, security)│
      └───────────┬───────────┘  └──────────────────────────┘
                   │
         ┌─────────┴──────────┐
         │                    │
         ▼                    ▼
   ┌──────────┐        ┌──────────┐
   │swell-    │        │swell-    │
   │daemon    │        │benchmark │
   └──────────┘        └──────────┘
```

---

## 3. Orchestrator Architecture

### 3.1 Orchestrator State Machine

The `Orchestrator` drives a task through states:

```
CREATED → PLANNING → APPROVED → EXECUTING → VALIDATING → COMPLETED
                    ↘          ↘           ↘
                     REJECTED   FAILED       PAUSED
```

### 3.2 Orchestrator Constructor Policy (BINDING RULE)

**All production-required subsystems of `Orchestrator` must be constructor parameters of `Orchestrator::new`, not `Option<_>` fields with `with_*` setters.**

The `test-only` `OrchestratorBuilder` (gated behind `#[cfg(any(test, feature = "test-support"))]`) may accept subsystems optionally for fake injection. If a new subsystem is not yet implemented in production, do not add it as a field.

**Anti-patterns that violate this rule:**
- `Arc<Mutex<Option<Foo>>>` — structurally still `Option`
- `impl Default` on a required subsystem — silent stub via `Default::default()`
- A `with_foo` setter on `Orchestrator` (not on `OrchestratorBuilder`)
- A `null` / `disabled` constructor that returns a stub
- A config struct with `Option<Foo>` fields

This rule is enforced by:
- `antipattern-gate` CI job (verifies 11 forbidden shapes)
- `build-no-default-features` CI job (verifies `OrchestratorBuilder` not in production binary)

---

## 4. Structural Refactors (Phase B+C)

### Refactor 02 — Startup Wiring Manifest

At daemon startup, print a machine-parseable manifest of every load-bearing subsystem:
- Trait: `WiringReport { name(), identity(), state() }` in `swell-core`
- `WiringState::{Enabled, Degraded(String), Disabled(String)}`
- `Orchestrator::wiring_manifest() -> Vec<Box<dyn WiringReport>>`
- `Daemon::run` prints full manifest at startup, exactly once
- `SWELL_STRICT=1` causes daemon to refuse startup on degraded/disabled subsystems
- `full_cycle_wiring` test asserts manifest lists all required subsystems

### Refactor 03 — Newtype All Domain IDs

Replace raw `String`/`Uuid` with newtype wrappers to make argument-swap bugs unrepresentable:

| Newtype | Inner type |
|---------|-----------|
| `TaskId` | `Uuid` |
| `AgentId` | `Uuid` |
| `WorktreeId` | `Uuid` or `TaskId` |
| `BranchName` | `String` |
| `CommitSha` | `String` |
| `FeatureLeadId` | `Uuid` |
| `CheckpointId` | `Uuid` |
| `SessionId` | `Uuid` |
| `SocketPath` | `PathBuf` |

All 9 live in `swell-core::ids`. No `From<Uuid>` or `From<String>` conversions.

### Refactor 04 — Structured Daemon Error Enum

Replace `anyhow::Error` at daemon's public boundary with concrete `DaemonError`:

```rust
pub enum DaemonError {
    TaskNotFound(TaskId),
    ValidationFailed { task: TaskId, reason: ValidationReason },
    HookDenied { hook: String, detail: String },
    BudgetExceeded { task: TaskId, class: BudgetClass },
    WorktreeAllocFailed(WorktreeError),
    CommitFailed { task: TaskId, source: GitError },
    Llm(#[from] LlmError),
    Config(ConfigError),
    ShuttingDown,
    Internal(String),
}
```

`thiserror`-derived. `DaemonErrorWire` for JSON serialization. All public daemon fns return `Result<_, DaemonError>`. No `.to_string().contains()` error matching outside tests.

---

## 5. Wiring Guardrail Test Suite

The integration test binary `swell-integration-tests/tests/full_cycle_wiring.rs` is the **contract** that the daemon can reach every load-bearing subsystem through production wiring.

Rules:
- Do not delete tests from this file
- Do not convert wiring tests into mock-only tests
- If a fix invalidates a witness/ignore, update the matching invariant hygiene in the same change

---

## 6. Agent Skills

SWELL supports the Agent Skills standard from agentskills.io. Skills are discovered from `.swell/skills/` and `.factory/skills/`. Skills shipped with SWELL:

| Skill | Purpose |
|-------|---------|
| `rust` | Idiomatic Rust patterns, ownership, async |
| `rust-worker` | TDD, red-green loops, crate verification |
| `rust-code-review` | Ownership/borrowing review |
| `git` | Branches, commits, PRs |
| `agentic-coding` | PACT protocol, micro diffs |
| `agent-team-orchestration` | Multi-agent coordination |

---

## 7. Error Handling Conventions

- `thiserror` for domain-specific errors (define primary error type per crate)
- `anyhow` for context-rich error handling at API boundaries
- `SwellError` in `swell-core` is the canonical system-wide error type

---

## 8. Async Patterns

- `#[tokio::main]` for binaries, `#[tokio::test]` for tests
- `async-trait` for async methods in traits
- `tokio::spawn` for fire-and-forget, `spawn_blocking` for CPU-bound
- `tokio::select!` for racing multiple async operations
- Cancellation-safe futures only in `select!` branches
