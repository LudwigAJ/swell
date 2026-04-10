# SWELL - Autonomous Coding Engine

An autonomous coding agent built in Rust that doesn't stop until the job is done.

## Overview

SWELL is an autonomous coding engine designed to handle software engineering tasks end-to-end: planning, execution, testing, and validation. It uses a multi-agent architecture with specialized agents for different phases of the development workflow.

### Key Features

- **Multi-Agent Architecture**: Specialized agents for planning, coding, review, and refactoring
- **Task Orchestration**: DAG-based task management with dependency tracking
- **Memory System**: Persistent context across sessions with pattern learning
- **Tool Runtime**: File, git, shell, and search tools with permission tiers
- **Validation Pipeline**: Lint, test, security, and AI review gates
- **MCP Protocol**: Model Context Protocol support for external tool integration
- **CLI + Daemon**: Unix socket communication between CLI and daemon

### Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                         SWELL                                │
├─────────────────────────────────────────────────────────────┤
│  ┌─────────┐  ┌──────────────┐  ┌────────────────────┐     │
│  │  CLI    │  │   Daemon     │  │   Dashboard API    │     │
│  │ (swell) │  │ (swell-dmn)  │  │   (REST + WebSocket)│     │
│  └────┬────┘  └──────┬───────┘  └─────────┬──────────┘     │
│       │              │                     │                 │
│       │         ┌─────┴─────┐              │                 │
│       │         │Orchestrator│◄────────────┘                 │
│       │         └─────┬─────┘                                │
│       │               │                                      │
│       │    ┌──────────┼──────────┐                          │
│       │    │          │          │                          │
│       │ ┌──▼──┐  ┌────▼───┐ ┌────▼────┐                   │
│       │ │Planner│ │ Coder │ │Evaluator│  ...more agents    │
│       │ └──┬──┘  └────┬───┘ └────┬────┘                   │
│       │    │          │          │                          │
│       │    └──────────┼──────────┘                          │
│       │               │                                      │
│       │    ┌──────────▼──────────┐                          │
│       │    │      Tools          │                          │
│       │    │  (file, git, shell) │                          │
│       │    └──────────┬──────────┘                          │
│       │               │                                      │
│       │    ┌──────────▼──────────┐                          │
│       │    │     Validation      │                          │
│       │    │ (lint, test, review)│                          │
│       │    └─────────────────────┘                          │
│       │                                                     │
│       │    ┌─────────────────────┐                          │
│       │    │      Memory         │                          │
│       │    │   (SQLite store)    │                          │
│       │    └─────────────────────┘                          │
└───────┼─────────────────────────────────────────────────────┘
        │
   Unix Socket
```

## Crates

| Crate | Description |
|-------|-------------|
| `swell-core` | Core types, traits, state machine |
| `swell-orchestrator` | Task orchestration, scheduling, policy |
| `swell-llm` | LLM backends (Anthropic, OpenAI, Mock) |
| `swell-tools` | Tool implementations |
| `swell-validation` | Validation gates and pipelines |
| `swell-memory` | Memory system with SQLite |
| `swell-state` | State management |
| `swell-sandbox` | Sandbox isolation (stub) |
| `swell-daemon` | Daemon server |
| `swell-cli` | CLI client |
| `swell-benchmark` | Benchmark suite |

## Quick Start

### Prerequisites

- Rust 1.75+
- SQLite
- Anthropic API key (for Claude integration)

### Build

```bash
# Clone the repository
git clone https://github.com/your-org/swell.git
cd swell

# Build the workspace
cargo build --workspace

# Run tests
cargo test --workspace
```

### Configuration

Create `.swell/settings.json`:

```json
{
  "llm": {
    "anthropic_api_key": "${ANTHROPIC_API_KEY}"
  },
  "timeouts": {
    "task_step_seconds": 300,
    "task_max_seconds": 3600
  },
  "limits": {
    "max_retries": 3,
    "max_concurrent_tasks": 5
  }
}
```

### Usage

```bash
# Start the daemon
cargo run --bin swell-daemon &

# Create a task
cargo run --bin swell -- task "Fix the login bug in auth.rs"

# List tasks
cargo run --bin swell -- list

# Watch a task
cargo run --bin swell -- watch <task-id>

# Cancel a task
cargo run --bin swell -- cancel <task-id>

# Approve a task
cargo run --bin swell -- approve <task-id>
```

### Dashboard API

REST API available at `http://localhost:3100`:

```bash
# Get all tasks
curl http://localhost:3100/api/tasks

# Get task by ID
curl http://localhost:3100/api/tasks/<task-id>

# Get agent info
curl http://localhost:3100/api/agents

# Get cost tracking
curl http://localhost:3100/api/cost
```

WebSocket for real-time events:

```bash
wscat -c ws://localhost:3100/ws
```

## Development

### Adding a New Tool

1. Implement `Tool` trait in `swell-tools/src/`:

```rust
use swell_core::tools::{Tool, ToolResult};

pub struct MyTool;

impl Tool for MyTool {
    fn name(&self) -> &str { "my_tool" }
    fn description(&self) -> &str { "Description of my tool" }
    
    async fn execute(&self, params: Value) -> ToolResult {
        // Implementation
    }
}
```

2. Register in tool registry:

```rust
registry.register(Box::new(MyTool));
```

### Adding a New Agent

1. Implement `Agent` trait in `swell-orchestrator/src/agents/`:

```rust
use swell_core::agents::{Agent, AgentConfig};

pub struct MyAgent;

impl Agent for MyAgent {
    async fn run(&self, task: &Task, ctx: &Context) -> Result<AgentOutput> {
        // Implementation
    }
}
```

2. Register in agent pool:

```rust
agent_pool.register("my_agent", Box::new(MyAgent));
```

### MCP Integration

SWELL supports the Model Context Protocol for external tool integration:

```rust
use swell_tools::mcp::McpClient;

let client = McpClient::new("path/to/mcp-server").await?;
let tools = client.discover_tools().await?;

for tool in tools {
    registry.register(tool);
}
```

## Research & Specifications

Detailed specifications are in `plan/research_documents/`:

- **Technical Architecture** - Overall system design and roadmap
- **Memory Architecture** - Memory and learning system
- **Orchestrator Design** - Task orchestration and execution
- **Product Strategy** - Product vision and user experience
- **Testing Research** - Validation and testing strategy
- **Tools Runtime** - Tools and sandbox design

## Autonomy Levels

SWELL supports configurable autonomy levels:

| Level | Description |
|-------|-------------|
| L1 | Supervised - every action requires approval |
| L2 | Guided (default) - plan approved, execute auto |
| L3 | Autonomous - minimal guidance needed |
| L4 | Full Auto - fully autonomous operation |

## Roadmap

### v1 (Current)
- [x] Core orchestration loop
- [x] Basic agent types (Planner, Coder, Evaluator)
- [x] Tool implementations (file, git, shell, search)
- [x] LLM integration (Anthropic, OpenAI)
- [x] Validation pipeline (lint, test, security)
- [x] Memory system with SQLite
- [x] CLI + Daemon architecture
- [x] Benchmark suite

### v2 (Planned)
- [ ] Knowledge Graph for code structure
- [ ] Vector search with code embeddings
- [ ] Tree-sitter AST parsing
- [ ] Firecracker microVM sandbox
- [ ] OpenTelemetry integration
- [ ] Feature Lead sub-orchestrators
- [ ] MCP protocol full implementation

## License

MIT