# swell-cli AGENTS.md

## Purpose

`swell-cli` provides the command-line interface client for the SWELL autonomous coding engine daemon. It communicates with the daemon via Unix socket, providing commands for task creation, management, and monitoring.

This crate handles:
- **Task Management** — Create, approve, reject, cancel, pause, resume tasks
- **Task Monitoring** — Watch task progress with real-time event streaming
- **Task Listing** — List all tasks with state and description
- **Instruction Injection** — Inject instructions into running tasks
- **Scope Modification** — Modify task scope (files, directories)
- **Connection Management** — Unix socket connection with timeout handling

**Depends on:** `swell-core` (for `CliCommand`, `DaemonEvent`, `Task` types)

## Public API

### CLI Commands

The CLI accepts these commands (called as `swell <command>`):

| Command | Description |
|---------|-------------|
| `swell task <description>` | Create a new task |
| `swell list [--json]` | List all tasks |
| `swell watch <task-id>` | Watch task status |
| `swell approve <task-id>` | Approve task plan |
| `swell cancel <task-id>` | Cancel a task |
| `swell pause <task-id> [--reason <reason>]` | Pause a running task |
| `swell resume <task-id>` | Resume a paused task |
| `swell inject <task-id> <instruction>` | Inject instructions into a task |
| `swell scope <task-id> [--files <files>] [--dirs <dirs>]` | Modify task scope |

### Error Types

```rust
#[derive(Error, Debug)]
pub enum CliError {
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),

    #[error("Daemon not running. Start with: swell-daemon")]
    DaemonNotRunning,

    #[error("Socket not found at {0}")]
    SocketNotFound(String),

    #[error("Connection timeout after {0:?}")]
    ConnectionTimeout(Duration),

    #[error("Request timeout after {0:?}")]
    RequestTimeout(Duration),

    #[error("Invalid UUID format: {0}")]
    InvalidUuid(String),

    #[error("Invalid command: {0}")]
    InvalidCommand(String),

    #[error("Missing required argument: {0}")]
    MissingArgument(String),

    #[error("Server error: {0}")]
    ServerError(String),

    #[error("Unexpected response format")]
    UnexpectedResponse,

    #[error("JSON parse error: {0}")]
    JsonParseError(String),
}

impl CliError {
    pub fn exit_code(&self) -> i32;
    pub fn error_code(&self) -> &'static str;
}
```

### Internal Functions

```rust
async fn send_command(socket_path: &str, cmd: CliCommand) -> Result<(), CliError>;
async fn watch_task(socket_path: &str, task_id: Uuid) -> Result<(), CliError>;
async fn list_tasks(socket_path: &str, json_output: bool) -> Result<(), CliError>;
fn confirm(prompt: &str) -> bool;
fn print_error(error: &CliError);
fn handle_event(event: &DaemonEvent);
fn is_terminal_event(event: &DaemonEvent) -> bool;
fn print_task_table(tasks: &[Task]);
fn usage();
```

## Architecture

```
┌─────────────────────────────────────────────────────────────────────┐
│                         swell-cli                                   │
├─────────────────────────────────────────────────────────────────────┤
│                                                                     │
│  ┌─────────────────────────────────────────────────────────────┐   │
│  │  main() - Command dispatcher                                │   │
│  │  ┌─────────────────────────────────────────────────────┐   │   │
│  │  │  match args[1] {                                    │   │   │
│  │  │    "task"    → send_command(TaskCreate)              │   │   │
│  │  │    "list"    → list_tasks()                          │   │   │
│  │  │    "watch"   → watch_task()                          │   │   │
│  │  │    "approve" → send_command(TaskApprove)             │   │   │
│  │  │    "cancel"  → send_command(TaskCancel)              │   │   │
│  │  │    "pause"   → send_command(TaskPause)               │   │   │
│  │  │    "resume"  → send_command(TaskResume)               │   │   │
│  │  │    "inject"  → send_command(TaskInjectInstruction)   │   │   │
│  │  │    "scope"   → send_command(TaskModifyScope)         │   │   │
│  │  │  }                                                    │   │   │
│  │  └─────────────────────────────────────────────────────┘   │   │
│  └─────────────────────────────────────────────────────────────┘   │
│                              │                                      │
│          ┌───────────────────┼───────────────────┐                  │
│          ▼                   ▼                   ▼                  │
│  ┌───────────────┐   ┌───────────────┐   ┌───────────────┐         │
│  │ send_command  │   │  watch_task  │   │  list_tasks  │         │
│  │  - Connect     │   │  - Stream    │   │  - Query     │         │
│  │  - Send JSON   │   │  - Display   │   │  - Table     │         │
│  │  - Read resp   │   │  - Terminal  │   │  - JSON opt  │         │
│  └───────────────┘   └───────────────┘   └───────────────┘         │
│                              │                                      │
│                              ▼                                      │
│  ┌─────────────────────────────────────────────────────────────┐   │
│  │  UnixStream connection to /tmp/swell-daemon.sock              │   │
│  │  - Timeout: 5s connect, 30s request                          │   │
│  │  - JSON serialization via serde_json                         │   │
│  └─────────────────────────────────────────────────────────────┘   │
│                                                                     │
└─────────────────────────────────────────────────────────────────────┘
                           │ communicates via
                           ▼
              ┌────────────────────────┐
              │     swell-daemon       │
              │   (Unix socket server) │
              └────────────────────────┘
```

**Key modules:**
- `main.rs` — CLI entry point, command parsing, socket communication

**Connection Parameters:**
- Socket path: `/tmp/swell-daemon.sock` (configurable via `SWELL_SOCKET` env var)
- Connect timeout: 5 seconds
- Request timeout: 30 seconds

**Output Formats:**
- Default: Human-readable text output
- `--json`: Raw JSON for scripting
- Watch mode: Line-delimited JSON events

**Exit Codes:**
| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | Server error or internal error |
| 2 | Invalid command or arguments |
| 10 | Connection failed (daemon not running) |
| 11 | Timeout |

**Concurrency:** Async I/O via Tokio for non-blocking socket operations.

## Testing

```bash
# Build the CLI
cargo build -p swell-cli

# Run the CLI (requires daemon)
cargo run -p swell-cli -- task "Hello world"

# Run tests (if any)
cargo test -p swell-cli -- --test-threads=4
```

**Test patterns:**
- CLI argument parsing tests
- Error message format tests
- Exit code tests

**Manual testing:**
```bash
# Start daemon
cargo run --bin swell-daemon &

# Create task
cargo run -p swell-cli -- task "Implement hello world"

# List tasks
cargo run -p swell-cli -- list

# Watch task
cargo run -p swell-cli -- watch <task-id>

# Approve task
cargo run -p swell-cli -- approve <task-id>
```

## Dependencies

```toml
# clients/swell-cli/Cargo.toml
[package]
name = "swell-cli"
version.workspace = true
edition.workspace = true

[[bin]]
name = "swell"
path = "src/main.rs"

[dependencies]
swell-core = { path = "../../crates/swell-core" }
tokio.workspace = true
serde.workspace = true
serde_json.workspace = true
thiserror.workspace = true
anyhow.workspace = true
tracing.workspace = true
uuid.workspace = true
chrono.workspace = true
reqwest.workspace = true
```

**Internal workspace dependencies:** `swell-core` for type definitions (`CliCommand`, `DaemonEvent`, `Task`)
