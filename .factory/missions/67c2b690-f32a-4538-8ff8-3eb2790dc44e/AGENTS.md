# SWELL MVP - Agent Guidance

## Available Skills

SWELL has two skill directories:
- **`.factory/skills/`** - Core skills for agent operation
- **`.swell/skills/`** - User-extensible skills following Agent Skills standard

### Orchestrator Skills (`.factory/skills/` - Droid/factoryd infrastructure)

**IMPORTANT:** `.factory/` is NOT part of SWELL. It is Droid/factoryd infrastructure. SWELL must NEVER rely on `.factory/`. The only config place for SWELL is `.swell/`.

These skills are for the orchestrator/workers to use when building SWELL, not for SWELL itself.

| Skill | Description | When to Use |
|-------|-------------|-------------|
| **agent-access-control** | Tiered stranger access control for AI agents. Diplomatic deflection, owner approval flow, multi-tier access (owner/trusted/chat-only/blocked). | Contact permissions, unknown senders, approved contacts, stranger deflection on messaging platforms. |
| **agent-audit-trail** | Append-only, hash-chained audit log. Records actions, tool calls, decisions with sha256 chain integrity. EU AI Act Article 12 compliance. | High-risk AI systems requiring automatic event recording, audit trail for agent actions. |
| **agent-autonomy-kit** | Stop waiting for prompts. Keep working. Enables proactive task queues and continuous operation. | Transforming reactive agents into proactive ones, heartbeat-driven work loops, overnight automation. |
| **agent-evaluation** | Testing and benchmarking LLM agents including behavioral testing, capability assessment, reliability metrics. | Agent testing, benchmark design, capability assessment, regression testing. |
| **agent-team-orchestration** | Orchestrate multi-agent teams with defined roles, task lifecycles, handoff protocols, review workflows. | Teams of 2+ agents, task routing (inbox→spec→build→review→done), handoff protocols, review gates. |
| **agentic-coding** | Ship production code through acceptance contracts, micro diffs, red green loops, deterministic handoff checkpoints. | Production features, risky refactors, bug fixes with reproducible failures, merge-ready code. |
| **arc-security-audit** | Comprehensive security audit chaining scanner, differ, trust-verifier, health-monitor with prioritized findings. | Auditing full skill stack security, generating trust attestations, verifying binary integrity. |
| **code** | Coding workflow with planning, implementation, verification, testing. | User requests code implementation, needs planning and verification guidance. |
| **coding** | Coding style memory that adapts to preferences, conventions, patterns. Learns from explicit corrections. | Storing coding style preferences, naming conventions, formatting rules. |
| **git** | Git commits, branches, rebases, merges, conflict resolution, history recovery, team workflows. | Any Git operation: repositories, branches, commits, merges, rebases, PRs. |
| **proactive-agent** | Transform agents from task-followers into proactive partners. WAL Protocol, Working Buffer, Compaction Recovery, autonomous crons. | Building agents that act without being asked, survive context loss, or improve over time. |
| **prompt-engineering-expert** | Advanced prompt engineering, custom instructions design, prompt optimization. | Writing/refining prompts, designing agent system instructions, optimizing for consistency. |
| **rust** | Write idiomatic Rust avoiding ownership pitfalls, lifetime confusion, borrow checker battles. | Writing Rust code, debugging ownership/borrowing errors. |
| **rust-code-review** | Reviews Rust code for ownership, borrowing, lifetime, error handling, trait design, unsafe usage. | Reviewing .rs files, checking borrow checker issues, validating error handling. |
| **rust-patterns** | Production Rust patterns: async Tokio, Axum, SQLx, error handling, CLI tools, WASM, PyO3. | Building production Rust systems—web servers, async services, database access, CLI tools. |
| **rust-testing-code-review** | Reviews Rust test code for unit tests, integration tests, async testing, mocking, property-based testing. | Reviewing _test.rs files, #[cfg(test)] modules, test infrastructure. |
| **self-improving** | Self-reflection + self-criticism + self-learning. Evaluates own work, catches mistakes, improves permanently. | When commands fail, user corrects you, knowledge is outdated, or better approach discovered. |
| **skill-guard** | Scan ClawHub skills for security vulnerabilities before installing. Detects prompt injections, malware, secrets. | Before installing any new skill from ClawHub or external sources. |
| **task-development-workflow** | TDD-first workflow with structured planning, task tracking, PR-based code review. | Software requiring clarification phases, planning approval gates, Trello, TDD, PR feedback loops. |
| **ui-ux-pro-max** | UI/UX design intelligence and implementation guidance for polished interfaces. | UI design, UX flows, design systems, component specs, frontend UI generation. |
| **website** | Build fast, accessible, SEO-friendly websites with modern best practices. | Creating/auditing websites for performance, accessibility, mobile, SEO. |

### SWELL Skills (`.swell/skills/` - User Extensible)

These follow the Agent Skills standard with YAML frontmatter and progressive disclosure.

| Skill | Description | When to Use |
|-------|-------------|-------------|
| **rust-coding** | Write idiomatic Rust following best practices. Ownership, lifetimes, error handling, async Tokio. | Implementing Rust features, fixing bugs, working with ownership/lifetimes. Keywords: rust, cargo, async, tokio, ownership |
| **test-writing** | Write comprehensive tests for Rust code. Unit tests, mocking, async tests, property-based. | Adding tests, improving coverage, fixing failing tests. Keywords: test, mock, unit test, integration test |
| **code-review** | Review Rust code for correctness, style, performance, security. Clippy, ownership checks. | Reviewing PRs, pre-commit checks, security review. Keywords: review, clippy, lint, security |
| **refactoring** | Refactor Rust code while preserving behavior. Martin Fowler patterns, strangler fig. | Improving code structure, extracting functions, reducing duplication. Keywords: refactor, extract, builder |

## Mission Overview

This mission implements the core orchestration loop for SWELL - an autonomous coding engine in Rust. The goal is a working daemon + CLI that can accept tasks, plan them, generate code, and validate output.

## Mission Boundaries (NEVER VIOLATE)

**Port Range:** 3100-3199 reserved for future HTTP API. Do not use outside this range.

**Off-Limits:**
- `/data` directory - do not read or modify
- Port 3000 - user's main dev servers
- Production systems or real user data

**External Services:**
- Anthropic API for LLM calls (requires `ANTHROPIC_API_KEY` env var)
- SQLite for local state (file-based, no external DB)

**Workspace:** The workspace is at `/Users/ludwigjonsson/Projects/swell`

## Coding Conventions

### Rust Patterns
- Use `async_trait::async_trait` for async trait methods
- Use `thiserror::Error` for errors
- Use `#[tokio::test]` for async tests
- Use `Arc<dyn Trait>` for dynamic dispatch
- Import from `swell_core` using relative path `../swell-core`

### Error Handling
- All fallible operations return `Result<T, SwellError>`
- Use `?` operator for error propagation
- Never panic in production code

### Logging
- Use `tracing` for structured logging
- Log state transitions: `info!(task_id = %id, from = %old, to = %new, "Transition")`
- Log errors with context: `error!(error = %err, "Description")`

### Testing
- Unit tests in `#[cfg(test)]` modules
- Integration tests in `tests/` directory
- Mock external dependencies (LLM backends)

## Project Structure

```
swell/
├── crates/
│   ├── swell-core/       # Traits, types, errors
│   ├── swell-llm/        # LLM backends (Anthropic, OpenAI, Mock)
│   ├── swell-state/      # Checkpointing, state management
│   ├── swell-memory/     # Memory store
│   ├── swell-tools/      # Tool registry, executors
│   ├── swell-sandbox/    # Sandbox (stub for MVP)
│   ├── swell-validation/ # Validation gates
│   ├── swell-orchestrator/ # Main orchestration
│   └── swell-daemon/    # Daemon server
└── clients/
    └── swell-cli/        # CLI client
```

## Commands

```bash
# Build
cargo build --workspace

# Test
cargo test --workspace

# Lint
cargo clippy --workspace

# Run daemon
cargo run --bin swell-daemon

# Run CLI
cargo run --bin swell
```

## Testing & Validation Guidance

### Unit Tests
- Run `cargo test -p <crate>` to test specific crate
- Focus on state machine transitions, agent behavior, tool execution

### Integration Tests
- Require daemon running with `cargo run --bin swell-daemon`
- Test CLI commands against running daemon

### Manual Testing
1. Start daemon: `cargo run --bin swell-daemon`
2. In another terminal, run CLI commands:
   - `swell task "implement hello world"`
   - `swell list`
   - `swell watch <task-id>`

## Known Pre-Existing Issues

None currently identified.

## Implementation Priority

1. **Foundation**: Fix any remaining build errors
2. **Orchestration**: Complete state machine, agent pool, execution controller
3. **LLM Integration**: Connect agents to Anthropic backend
4. **Tools**: File, Git, Shell tools with proper error handling
5. **Validation**: LintGate, TestGate with actual command execution
6. **CLI/Daemon**: Clean socket communication, proper error responses
