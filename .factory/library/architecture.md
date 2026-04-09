# SWELL Architecture

## System Overview

SWELL is a Rust-based autonomous coding engine that plans, writes, tests, and validates code autonomously using multi-agent orchestration.

## Core Components

### swell-core
Foundation crate providing traits and types:
- `LlmBackend` trait - abstraction for LLM providers
- `Agent` trait - base for all agent types
- `Tool` trait - base for all executable tools
- `MemoryStore` trait - persistent memory
- `Sandbox` trait - isolated execution
- `CheckpointStore` trait - state persistence
- `ValidationGate` trait - quality checks

### swell-llm
LLM provider implementations:
- `AnthropicBackend` - Claude via Anthropic API
- `OpenAIBackend` - GPT models via OpenAI API
- `MockLlm` - for testing without API calls

### swell-orchestrator
Main coordination layer:
- `Orchestrator` - task orchestration
- `TaskStateMachine` - 8-state lifecycle
- `AgentPool` - agent instance management
- `ExecutionController` - parallel execution control
- `PlannerAgent`, `GeneratorAgent`, `EvaluatorAgent` - agent implementations

### swell-tools
Tool execution:
- `ToolRegistry` - central tool registration
- `ToolExecutor` - permission-aware execution
- `FileReadTool`, `FileWriteTool` - file operations
- `ShellTool` - command execution
- `GitTool` - git operations
- `McpToolWrapper` - MCP server integration

### swell-validation
Quality gates:
- `LintGate` - clippy/format checks
- `TestGate` - test execution
- `SecurityGate` - security scanning (stub)
- `AiReviewGate` - AI code review (stub)
- `ValidationPipeline` - gate orchestration

### swell-state
State persistence:
- `SqliteCheckpointStore` - SQLite-based checkpointing
- `PostgresCheckpointStore` - PostgreSQL for production
- `InMemoryCheckpointStore` - for tests

### swell-memory
Memory system:
- `SqliteMemoryStore` - SQLite-based memory

### swell-sandbox
Sandbox (stub for MVP):
- `FirecrackerSandbox` - Firecracker microVM (future)

### swell-daemon
Server daemon:
- Unix socket server
- Command handling
- Event emission

### swell-cli
CLI client:
- Task creation (`swell task`)
- Task listing (`swell list`)
- Task watching (`swell watch`)
- Task approval (`swell approve`)

## Data Flow

1. **Task Creation**: CLI sends `TaskCreate` command to daemon
2. **Planning**: PlannerAgent receives task, calls LLM, produces Plan
3. **Execution**: GeneratorAgent implements Plan steps using tools
4. **Validation**: ValidationPipeline runs gates (lint, test)
5. **Completion**: EvaluatorAgent reviews, task marked accepted/rejected
6. **Output**: Feature branch with commits, PR description

## Key Design Patterns

- **Trait-based abstraction** for all major components
- **Async/await** throughout (tokio runtime)
- **Structured logging** with tracing
- **Error handling** via thiserror enums
- **Dynamic dispatch** via `Arc<dyn Trait>` for plugins
