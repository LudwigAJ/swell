# swell-daemon AGENTS.md

## Purpose

`swell-daemon` provides the daemon server for the SWELL autonomous coding engine. It runs as a Unix socket server, accepting commands from the CLI client and coordinating task execution through the orchestrator.

This crate handles:
- **Unix Socket Server** — Accepts connections from CLI clients
- **Command Processing** — Handles task CRUD operations (create, approve, reject, cancel)
- **Event Streaming** — Streams real-time events to watching clients
- **Orchestrator Coordination** — Delegates task execution to the orchestrator
- **Graceful Shutdown** — Handles SIGTERM with connection draining
- **Dashboard API** — HTTP/WebSocket server for operational dashboard

**Depends on:** `swell-core`, `swell-orchestrator`

## Public API

### Daemon (`server.rs`)

```rust
pub struct Daemon {
    orchestrator: Arc<Mutex<Arc<Orchestrator>>>,
    event_emitter: Arc<EventEmitter>,
    socket_path: String,
    shutdown_flag: Arc<AtomicBool>,
    active_connections: Arc<AtomicUsize>,
    shutdown_tx: watch::Sender<bool>,
    shutdown_rx: watch::Receiver<bool>,
}

impl Daemon {
    pub fn new(socket_path: String, llm: Arc<dyn LlmBackend>) -> Self;
    pub fn event_emitter(&self) -> Arc<EventEmitter>;
    pub fn orchestrator(&self) -> Arc<Mutex<Arc<Orchestrator>>>;
    pub async fn run(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
    fn request_shutdown(&self);
    fn is_shutting_down(&self) -> bool;
    async fn wait_for_active_connections(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
}
```

### Event Emitter (`events.rs`)

```rust
pub struct EventEmitter {
    // Internal event storage and broadcasting
}

impl EventEmitter {
    pub fn new() -> Self;
    pub fn new_correlation_id() -> Uuid;
    pub async fn emit(&self, event: DaemonEvent) -> usize;
    pub async fn get_events_since(&self, sequence: usize) -> Vec<DaemonEvent>;
    pub async fn get_events_since_for_task(&self, task_id: Uuid, since: usize) -> Vec<DaemonEvent>;
    pub async fn current_sequence(&self) -> usize;
}

pub struct ImmutableEventLog {
    // Read-only view of event history
}
```

### Dashboard (`dashboard.rs`)

```rust
pub struct DashboardState {
    pub active_connections: usize,
    pub running_tasks: usize,
    pub total_tasks: usize,
    pub uptime_secs: f64,
}

pub enum DashboardEvent {
    ConnectionOpened { connection_id: Uuid },
    ConnectionClosed { connection_id: Uuid },
    TaskStarted { task_id: Uuid },
    TaskCompleted { task_id: Uuid, duration_secs: f64 },
}
```

### Commands (`commands.rs`)

```rust
// Command handlers exported for use by the daemon
pub async fn handle_command(
    command: CliCommand,
    orchestrator: Arc<Mutex<Arc<Orchestrator>>>,
    event_emitter: Arc<EventEmitter>,
) -> DaemonEvent;
```

### Key Re-exports

```rust
pub use dashboard::{DashboardEvent, DashboardState};
pub use events::{EventEmitter, ImmutableEventLog};
pub use server::Daemon;
```

## Architecture

```
┌─────────────────────────────────────────────────────────────────────┐
│                        swell-daemon                                 │
├─────────────────────────────────────────────────────────────────────┤
│                                                                     │
│  ┌─────────────────────────────────────────────────────────────┐   │
│  │                      Daemon                                  │   │
│  │  ┌─────────────────────────────────────────────────────┐   │   │
│  │  │  UnixListener on /tmp/swell-daemon.sock            │   │   │
│  │  │  - Connection acceptance loop                       │   │   │
│  │  │  - SIGTERM handling                                 │   │   │
│  │  │  - Graceful shutdown with connection draining       │   │   │
│  │  └─────────────────────────────────────────────────────┘   │   │
│  └─────────────────────────────────────────────────────────────┘   │
│                              │                                      │
│          ┌───────────────────┼───────────────────┐                  │
│          ▼                   ▼                   ▼                  │
│  ┌───────────────┐   ┌───────────────┐   ┌───────────────┐         │
│  │ handle_connection │ │handle_watch   │   │ handle_command │         │
│  │  - JSON parse   │   │  - Event stream │   │  - Task CRUD  │         │
│  │  - Single resp  │   │  - Polling     │   │  - Orchestrator│         │
│  └───────────────┘   └───────────────┘   └───────────────┘         │
│                              │                                      │
│                              ▼                                      │
│  ┌─────────────────────────────────────────────────────────────┐   │
│  │                    EventEmitter                             │   │
│  │  - Publish/subscribe model                                 │   │
│  │  - Sequence tracking for replay                            │   │
│  │  - Per-task event filtering                                │   │
│  └─────────────────────────────────────────────────────────────┘   │
│                              │                                      │
│                              ▼                                      │
│  ┌─────────────────────────────────────────────────────────────┐   │
│  │                   Orchestrator                               │   │
│  │  (from swell-orchestrator)                                   │   │
│  └─────────────────────────────────────────────────────────────┘   │
│                                                                     │
│  ┌─────────────────────────────────────────────────────────────┐   │
│  │                    Dashboard (optional)                      │   │
│  │  - HTTP/WS server for operational monitoring                 │   │
│  │  - Real-time metrics                                       │   │
│  └─────────────────────────────────────────────────────────────┘   │
│                                                                     │
└─────────────────────────────────────────────────────────────────────┘
                           │ used by
                           ▼
              ┌────────────────────────┐
              │      swell-cli          │
              │   (Unix socket client) │
              └────────────────────────┘
```

**Key modules:**
- `server.rs` — `Daemon` struct with socket listening and connection handling
- `events.rs` — `EventEmitter` for publish/subscribe event distribution
- `commands.rs` — Command handlers for all CLI commands
- `dashboard.rs` — Optional HTTP/WS dashboard for operational monitoring

**Connection Lifecycle:**
1. Client connects to Unix socket
2. Daemon accepts and spawns connection handler
3. Handler reads JSON command, processes via `handle_command`
4. Response sent back on same socket (or event stream for watch)
5. Connection closed on response completion

**Shutdown Sequence:**
1. SIGTERM received → `shutdown_flag` set to true
2. Daemon stops accepting new connections
3. Waits for active connections to complete (up to 30s)
4. Socket file removed
5. Process exits

**Concurrency:** Uses `tokio::sync::watch` for shutdown signaling, `AtomicUsize` for connection counting.

## Testing

```bash
# Run tests for swell-daemon
cargo test -p swell-daemon -- --test-threads=4

# Run with logging
RUST_LOG=debug cargo test -p swell-daemon

# Run specific test
cargo test -p swell-daemon -- test_command_parsing --nocapture

# Run event emitter tests
cargo test -p swell-daemon -- events --nocapture

# Run integration tests (requires daemon binary)
cargo test -p swell-daemon -- daemon
```

**Test structure:**
- Unit tests for command parsing and validation
- Integration tests with mock Unix socket connections
- Event emitter tests for publish/subscribe
- Dashboard state tests

**Mock patterns:**
```rust
#[tokio::test]
async fn test_daemon_startup() {
    let temp_dir = tempfile::tempdir().unwrap();
    let socket_path = temp_dir.path().join("swell-daemon.sock");
    let daemon = Daemon::new(socket_path.to_string_lossy().to_string());
    // Test initialization...
}

#[test]
fn test_event_correlation() {
    let emitter = EventEmitter::new();
    let correlation_id = EventEmitter::new_correlation_id();
    assert!(correlation_id != Uuid::nil());
}
```

## Dependencies

```toml
# swell-daemon/Cargo.toml
[dependencies]
swell-core = { path = "../swell-core" }
swell-orchestrator = { path = "../swell-orchestrator" }
tokio.workspace = true
serde.workspace = true
serde_json.workspace = true
thiserror.workspace = true
anyhow.workspace = true
tracing.workspace = true
uuid.workspace = true
chrono.workspace = true

# HTTP/WebSocket server for Dashboard API
axum = { version = "0.7", features = ["ws"] }
tokio-tungstenite = { version = "0.24", features = ["tokio-native-tls"] }
futures-util = { version = "0.3", default-features = false, features = ["sink", "async-await"] }
tower = "0.4"
tower-http = { version = "0.5", features = ["cors"] }
hyper = { version = "1.4", features = ["full"] }
http-body-util = "0.1"

[dev-dependencies]
futures = "0.3"
```

**Internal workspace dependencies:** `swell-orchestrator` for task coordination
