# SWELL MVP - Agent Guidance

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
