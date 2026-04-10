# SWELL MVP: Core Orchestration Loop

# SWELL MVP: Core Orchestration Loop

## Plan Overview

Implement a working MVP of the SWELL autonomous coding engine in Rust, focused on **Sprint Mode** with a CLI + daemon architecture. This establishes the foundation for all future work.

## Expected Functionality

### Milestone 1: Daemon + CLI Foundation (Day 1)
- [ ] **swell-daemon** as Unix socket server accepting task commands
- [ ] **swell-cli** for task creation, listing, watching, approving
- [ ] Proper error handling and logging throughout

### Milestone 2: Core Orchestration Loop (Days 2-5)
- [ ] **PlannerAgent** - reads codebase, generates execution plan via LLM
- [ ] **GeneratorAgent** - implements plan steps using tools
- [ ] **EvaluatorAgent** - validates code changes
- [ ] **ExecutionController** - coordinates the agent pipeline
- [ ] **TaskStateMachine** - proper state transitions

### Milestone 3: Tool Execution (Days 6-10)
- [ ] **FileTool** (read, write, edit files)
- [ ] **GitTool** (status, diff, commit, branch)
- [ ] **ShellTool** (execute commands in sandbox)
- [ ] **SearchTool** (grep, glob for codebase exploration)

### Milestone 4: LLM Integration (Days 11-15)
- [ ] Connect agents to **AnthropicBackend** for planning
- [ ] Connect agents to **AnthropicBackend** for generation
- [ ] Implement ReAct loop for tool execution
- [ ] Token tracking and CostGuard

### Milestone 5: Validation Pipeline (Days 16-20)
- [ ] **LintGate** - run clippy/format checks
- [ ] **TestGate** - run cargo test
- [ ] **SecurityGate** - stub for MVP
- [ ] Iterative improvement on failures

## Infrastructure

**Services:**
- SQLite database on localhost (state, checkpoints)
- Unix socket at `/tmp/swell-daemon.sock` for CLI communication

**Ports:** 3100-3199 reserved for future HTTP API

**External Services:**
- Anthropic API for LLM calls (requires `ANTHROPIC_API_KEY` env var)
- Local filesystem for workspace operations

## Testing Strategy

- Unit tests per crate (rust in `#[cfg(test)]`)
- Integration tests for daemon CLI commands
- Manual testing with real task execution

## User Testing Strategy

- CLI commands: `swell task "fix bug"`, `swell list`, `swell watch <id>`
- Daemon outputs structured events
- Manual inspection of generated PRs/changes

## Non-Functional Requirements

- 5 minute timeout per task step
- Max 3 retry iterations before escalation
- All operations logged with tracing
- Graceful shutdown on SIGINT/SIGTERM

## Scope Boundaries

**In scope:** CLI, daemon, orchestration, tools, LLM integration, basic validation
**Out of scope:** Sandbox isolation (shell-based for MVP), knowledge graph, vector search, MCP integration, web dashboard