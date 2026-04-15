# Swell Daemon API Reference

The Swell daemon (`swell-daemon`) exposes a JSON-based command/response API over a Unix domain socket. Clients connect to the socket, send JSON-encoded commands, and receive JSON-encoded responses.

**Socket Path:** `/tmp/swell-daemon.sock` (configurable via `SWELL_SOCKET` environment variable)

**Protocol:** All messages are JSON objects, one per line (newline-delimited JSON for streaming responses).

**Request/Response Pattern:**
- **One-shot commands**: Client sends one JSON object, daemon responds with one JSON object
- **Watch commands**: Client sends one JSON object, daemon responds with multiple newline-delimited JSON objects (stream)

---

## Table of Contents

1. [Connection](#connection)
2. [Command Reference](#command-reference)
   - [TaskCreate](#taskcreate)
   - [TaskApprove](#taskapprove)
   - [TaskReject](#taskreject)
   - [TaskCancel](#taskcancel)
   - [TaskList](#tasklist)
   - [TaskWatch](#taskwatch)
   - [TaskPause](#taskpause)
   - [TaskResume](#taskresume)
   - [TaskInjectInstruction](#taskinjectinstruction)
   - [TaskModifyScope](#taskmodifyscope)
   - [TaskGet](#taskget)
   - [DaemonStatus](#daemonstatus)
   - [ConfigGet](#configget)
   - [ConfigSet](#configset)
   - [MemoryQuery](#memoryquery)
   - [CostQuery](#costquery)
3. [Event Reference](#event-reference)
4. [Error Handling](#error-handling)
5. [Streaming Protocol](#streaming-protocol)
6. [Integration Guide](#integration-guide)
7. [Example Client](#example-client)

---

## Connection

### Establishing a Connection

Connect to the Unix socket at the configured path. The connection timeout is 5 seconds.

```bash
nc -U /tmp/swell-daemon.sock
```

### Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `SWELL_SOCKET` | `/tmp/swell-daemon.sock` | Path to the daemon socket |

### Timeouts

- **Connect timeout:** 5 seconds
- **Request timeout:** 30 seconds

---

## Command Reference

All commands are JSON objects with a `type` field identifying the command variant and a `payload` field containing the command-specific parameters.

### TaskCreate

Creates a new task with the given description.

**Request:**
```json
{
  "type": "TaskCreate",
  "payload": {
    "description": "Implement user authentication"
  }
}
```

| Field | Type | Description |
|-------|------|-------------|
| `description` | string | Natural language description of the task |

**Response:**
```json
{
  "type": "TaskCreated",
  "payload": {
    "id": "550e8400-e29b-41d4-a716-446655440000",
    "correlation_id": "7c9e6679-7425-40de-944b-e07fc1f90ae7"
  }
}
```

| Field | Type | Description |
|-------|------|-------------|
| `id` | UUID | Unique identifier for the created task |
| `correlation_id` | UUID | Correlation ID for tracking related events |

---

### TaskApprove

Approves a task that is awaiting approval, transitioning it from `AwaitingApproval` to `Ready` and then to `Executing`.

**Request:**
```json
{
  "type": "TaskApprove",
  "payload": {
    "task_id": "550e8400-e29b-41d4-a716-446655440000"
  }
}
```

| Field | Type | Description |
|-------|------|-------------|
| `task_id` | UUID | The task to approve |

**Response:**
```json
{
  "type": "TaskStateChanged",
  "payload": {
    "id": "550e8400-e29b-41d4-a716-446655440000",
    "state": "EXECUTING",
    "correlation_id": "7c9e6679-7425-40de-944b-e07fc1f90ae7"
  }
}
```

**Error Response:**
```json
{
  "type": "Error",
  "payload": {
    "message": "Task not found: TaskNotFound(550e8400-e29b-41d4-a716-446655440000)",
    "failure_class": null,
    "correlation_id": "7c9e6679-7425-40de-944b-e07fc1f90ae7"
  }
}
```

---

### TaskReject

Rejects a task that is awaiting approval, transitioning it to `Rejected` state.

**Request:**
```json
{
  "type": "TaskReject",
  "payload": {
    "task_id": "550e8400-e29b-41d4-a716-446655440000",
    "reason": "The plan scope is too broad"
  }
}
```

| Field | Type | Description |
|-------|------|-------------|
| `task_id` | UUID | The task to reject |
| `reason` | string | Reason for rejection |

**Response:**
```json
{
  "type": "TaskStateChanged",
  "payload": {
    "id": "550e8400-e29b-41d4-a716-446655440000",
    "state": "REJECTED",
    "correlation_id": "7c9e6679-7425-40de-944b-e07fc1f90ae7"
  }
}
```

---

### TaskCancel

Cancels a task, transitioning it to `Failed` state.

**Request:**
```json
{
  "type": "TaskCancel",
  "payload": {
    "task_id": "550e8400-e29b-41d4-a716-446655440000"
  }
}
```

**Response:**
```json
{
  "type": "TaskStateChanged",
  "payload": {
    "id": "550e8400-e29b-41d4-a716-446655440000",
    "state": "FAILED",
    "correlation_id": "7c9e6679-7425-40de-944b-e07fc1f90ae7"
  }
}
```

---

### TaskList

Returns a list of all tasks.

**Request:**
```json
{
  "type": "TaskList"
}
```

**Response:**
```json
{
  "type": "DataResponse",
  "data": {
    "type": "TaskList",
    "data": {
      "tasks": [
        {
          "id": "550e8400-e29b-41d4-a716-446655440000",
          "description": "Implement user authentication",
          "state": "EXECUTING",
          "created_at": "2026-04-15T10:30:00Z",
          "updated_at": "2026-04-15T10:35:00Z"
        }
      ],
      "correlation_id": "7c9e6679-7425-40de-944b-e07fc1f90ae7"
    }
  }
}
```

---

### TaskWatch

Watches a specific task for state changes and streams events. Uses the streaming protocol (newline-delimited JSON).

**Request:**
```json
{
  "type": "TaskWatch",
  "payload": {
    "task_id": "550e8400-e29b-41d4-a716-446655440000"
  }
}
```

**Response (stream):**
```
{"type":"TaskStateChanged","payload":{"id":"550e8400-e29b-41d4-a716-446655440000","state":"EXECUTING","correlation_id":"..."}}
{"type":"AgentTurnStarted","payload":{"id":"550e8400-e29b-41d4-a716-446655440000","agent_role":"generator","turn_number":1,"correlation_id":"..."}}
{"type":"ToolInvocationStarted","payload":{"id":"550e8400-e29b-41d4-a716-446655440000","tool_name":"file_read","arguments":{"file_path":"src/main.rs"},"turn_number":1,"correlation_id":"..."}}
{"type":"ToolInvocationCompleted","payload":{"id":"550e8400-e29b-41d4-a716-446655440000","tool_name":"file_read","success":true,"duration_ms":15,"turn_number":1,"correlation_id":"..."}}
{"type":"TaskStateChanged","payload":{"id":"550e8400-e29b-41d4-a716-446655440000","state":"VALIDATING","correlation_id":"..."}}
{"type":"TaskStateChanged","payload":{"id":"550e8400-e29b-41d4-a716-446655440000","state":"ACCEPTED","correlation_id":"..."}}
```

The stream terminates when the task reaches a terminal state (`Accepted`, `Rejected`, `Failed`, `Escalated`).

---

### TaskPause

Pauses a running task (operator intervention). Task transitions to `Paused` state.

**Request:**
```json
{
  "type": "TaskPause",
  "payload": {
    "task_id": "550e8400-e29b-41d4-a716-446655440000",
    "reason": "Operator requested pause for review"
  }
}
```

| Field | Type | Description |
|-------|------|-------------|
| `task_id` | UUID | The task to pause |
| `reason` | string | Reason for pausing |

**Response:**
```json
{
  "type": "TaskStateChanged",
  "payload": {
    "id": "550e8400-e29b-41d4-a716-446655440000",
    "state": "PAUSED",
    "correlation_id": "7c9e6679-7425-40de-944b-e07fc1f90ae7"
  }
}
```

---

### TaskResume

Resumes a paused task, transitioning it back to `Executing` state.

**Request:**
```json
{
  "type": "TaskResume",
  "payload": {
    "task_id": "550e8400-e29b-41d4-a716-446655440000"
  }
}
```

**Response:**
```json
{
  "type": "TaskStateChanged",
  "payload": {
    "id": "550e8400-e29b-41d4-a716-446655440000",
    "state": "EXECUTING",
    "correlation_id": "7c9e6679-7425-40de-944b-e07fc1f90ae7"
  }
}
```

---

### TaskInjectInstruction

Injects instructions into a running task (operator intervention).

**Request:**
```json
{
  "type": "TaskInjectInstruction",
  "payload": {
    "task_id": "550e8400-e29b-41d4-a716-446655440000",
    "instruction": "Also update the README.md file with the new API"
  }
}
```

| Field | Type | Description |
|-------|------|-------------|
| `task_id` | UUID | The target task |
| `instruction` | string | Instruction text to inject |

**Response:**
```json
{
  "type": "TaskProgress",
  "payload": {
    "id": "550e8400-e29b-41d4-a716-446655440000",
    "message": "Instruction injected: Also update the README.md file with the new API",
    "correlation_id": "7c9e6679-7425-40de-944b-e07fc1f90ae7"
  }
}
```

---

### TaskModifyScope

Modifies the scope boundaries of a task.

**Request:**
```json
{
  "type": "TaskModifyScope",
  "payload": {
    "task_id": "550e8400-e29b-41d4-a716-446655440000",
    "scope": {
      "files": ["src/auth.rs", "src/handlers.rs"],
      "directories": ["src/", "tests/"],
      "allowed_operations": ["read", "write", "execute"]
    }
  }
}
```

| Field | Type | Description |
|-------|------|-------------|
| `task_id` | UUID | The target task |
| `scope.files` | string[] | Files in scope |
| `scope.directories` | string[] | Directories in scope |
| `scope.allowed_operations` | string[] | Allowed operations |

**Response:**
```json
{
  "type": "TaskProgress",
  "payload": {
    "id": "550e8400-e29b-41d4-a716-446655440000",
    "message": "Scope modified: 2 files, 2 directories",
    "correlation_id": "7c9e6679-7425-40de-944b-e07fc1f90ae7"
  }
}
```

---

### TaskGet

Returns full task details as JSON.

**Request:**
```json
{
  "type": "TaskGet",
  "payload": {
    "task_id": "550e8400-e29b-41d4-a716-446655440000"
  }
}
```

**Response:**
```json
{
  "type": "TaskDetails",
  "payload": {
    "id": "550e8400-e29b-41d4-a716-446655440000",
    "task_json": "{...full Task object as JSON...}",
    "correlation_id": "7c9e6679-7425-40de-944b-e07fc1f90ae7"
  }
}
```

The `task_json` field contains the full serialized `Task` object.

---

### DaemonStatus

Returns daemon health status including connections, tasks, cost, and MCP health.

**Request:**
```json
{
  "type": "DaemonStatus"
}
```

**Response:**
```json
{
  "type": "DaemonHealth",
  "payload": {
    "active_connections": 2,
    "total_tasks": 5,
    "tasks_by_state": {
      "CREATED": 1,
      "EXECUTING": 2,
      "ACCEPTED": 2
    },
    "total_tokens": 150000,
    "last_model": "claude-sonnet-4-20250514",
    "mcp_health": {},
    "uptime_seconds": 3600,
    "version": "0.1.0",
    "total_budget": 1000000,
    "total_spent": 150000,
    "remaining_budget": 850000,
    "correlation_id": "7c9e6679-7425-40de-944b-e07fc1f90ae7"
  }
}
```

---

### ConfigGet

Gets a configuration value by key.

**Request:**
```json
{
  "type": "ConfigGet",
  "payload": {
    "key": "autonomy.level"
  }
}
```

| Field | Type | Description |
|-------|------|-------------|
| `key` | string | Configuration key (supports dot notation for nested keys) |

**Response:**
```json
{
  "type": "ConfigValue",
  "payload": {
    "key": "autonomy.level",
    "value": "guided",
    "source_file": "/Users/project/.swell/settings.json",
    "correlation_id": "7c9e6679-7425-40de-944b-e07fc1f90ae7"
  }
}
```

---

### ConfigSet

Sets a configuration value (writes to `settings.local.json`).

**Request:**
```json
{
  "type": "ConfigSet",
  "payload": {
    "key": "autonomy.level",
    "value": "autonomous"
  }
}
```

**Response:**
```json
{
  "type": "ConfigValue",
  "payload": {
    "key": "autonomy.level",
    "value": "autonomous",
    "source_file": "/Users/project/.swell/settings.local.json",
    "correlation_id": "7c9e6679-7425-40de-944b-e07fc1f90ae7"
  }
}
```

---

### MemoryQuery

Queries memory with BM25 search and temporal filters.

**Request:**
```json
{
  "type": "MemoryQuery",
  "payload": {
    "query": "authentication jwt token",
    "scope": {
      "session_id": null,
      "task_id": null,
      "agent_role": null
    },
    "limit": 10
  }
}
```

| Field | Type | Description |
|-------|------|-------------|
| `query` | string | Keywords to search for (space-separated) |
| `scope.session_id` | UUID? | Filter by session ID |
| `scope.task_id` | UUID? | Filter by task ID |
| `scope.agent_role` | string? | Filter by agent role |
| `limit` | integer | Maximum results to return |

**Response:**
```json
{
  "type": "MemoryResults",
  "payload": {
    "results": "[{\"id\":\"...\",\"content\":\"...\",\"label\":\"...\"}]",
    "count": 3,
    "correlation_id": "7c9e6679-7425-40de-944b-e07fc1f90ae7"
  }
}
```

---

### CostQuery

Queries cost data for a specific task or aggregate across all tasks.

**Request (aggregate):**
```json
{
  "type": "CostQuery",
  "payload": {
    "task_id": null
  }
}
```

**Request (per-task):**
```json
{
  "type": "CostQuery",
  "payload": {
    "task_id": "550e8400-e29b-41d4-a716-446655440000"
  }
}
```

| Field | Type | Description |
|-------|------|-------------|
| `task_id` | UUID? | Task ID to query (null for aggregate) |

**Response:**
```json
{
  "type": "CostQueryResult",
  "payload": {
    "task_id": null,
    "total_input_tokens": 75000,
    "total_output_tokens": 45000,
    "total_cost_usd": 0.2345,
    "model_breakdown": [
      {
        "model": "claude-sonnet-4-20250514",
        "call_count": 15,
        "total_input_tokens": 75000,
        "total_output_tokens": 45000,
        "total_cost_usd": 0.2345
      }
    ],
    "correlation_id": "7c9e6679-7425-40de-944b-e07fc1f90ae7"
  }
}
```

---

## Event Reference

Events are emitted by the daemon during task execution. They flow through the event emitter system and are visible in watch streams.

### Task Lifecycle Events

| Event | Description |
|-------|-------------|
| `TaskCreated` | A new task was created |
| `TaskStateChanged` | Task state transitioned |
| `TaskProgress` | Task reported progress |
| `TaskCompleted` | Task completed successfully |
| `TaskFailed` | Task failed with error |

### Execution Events

| Event | Description |
|-------|-------------|
| `AgentTurnStarted` | An agent turn began |
| `AgentTurnCompleted` | An agent turn finished |
| `ToolInvocationStarted` | A tool was invoked |
| `ToolInvocationCompleted` | A tool completed |
| `ValidationStepStarted` | A validation step began |
| `ValidationStepCompleted` | A validation step finished |

### Error Events

| Event | Description |
|-------|-------------|
| `Error` | General error occurred |

---

## Error Handling

### Error Response Format

All errors return the `Error` event:

```json
{
  "type": "Error",
  "payload": {
    "message": "Human-readable error message",
    "failure_class": "NetworkError",
    "correlation_id": "7c9e6679-7425-40de-944b-e07fc1f90ae7"
  }
}
```

### Failure Classes

| Class | Description |
|-------|-------------|
| `NetworkError` | Network connectivity issues |
| `LlmError` | LLM API errors |
| `ToolError` | Tool execution failures |
| `PermissionDenied` | Permission denied |
| `BudgetExceeded` | Budget or cost limit exceeded |
| `Timeout` | Operation timed out |
| `RateLimited` | Rate limited by external service |
| `InvalidState` | Invalid state transition |
| `ParseError` | Parse error |
| `ConfigError` | Configuration error |
| `SandboxError` | Sandbox isolation error |
| `InternalError` | Internal error |

### Task States

| State | Description |
|-------|-------------|
| `CREATED` | Task created, not yet started |
| `ENRICHED` | Task enriched with context |
| `AWAITING_APPROVAL` | Waiting for user approval |
| `READY` | Approved, ready to execute |
| `ASSIGNED` | Assigned to an agent |
| `EXECUTING` | Actively executing |
| `PAUSED` | Paused by operator |
| `VALIDATING` | Running validation |
| `ACCEPTED` | Task completed successfully |
| `REJECTED` | Task rejected |
| `FAILED` | Task failed |
| `ESCALATED` | Task escalated to human |

---

## Streaming Protocol

The `TaskWatch` command uses a streaming protocol:

1. Client sends a `TaskWatch` command
2. Daemon immediately sends the current task state as the first event
3. Daemon polls for new events every 500ms
4. Each new event is sent as a newline-delimited JSON object
5. Stream terminates when task reaches a terminal state (`Accepted`, `Rejected`, `Failed`, `Escalated`)

### Stream Format

Each line is a complete JSON object:

```
{"type":"TaskStateChanged","payload":{...}}
{"type":"AgentTurnStarted","payload":{...}}
{"type":"ToolInvocationStarted","payload":{...}}
```

### Detecting Terminal State

Check for these event types to detect stream end:

- `TaskStateChanged` with state `ACCEPTED`, `REJECTED`, `FAILED`, or `ESCALATED`
- `TaskCompleted`
- `TaskFailed`

---

## Integration Guide

### TUI Client

A TUI client should:

1. **Connect to socket** with 5-second timeout
2. **Send commands** as single-line JSON
3. **Handle responses** based on command type
4. **For watch streams**:
   - Read lines continuously
   - Parse each line as JSON
   - Update UI based on event type
   - Check for terminal state to close connection

### GUI Client

A GUI client should:

1. **Run daemon connection in background thread**
2. **Use async/await or callbacks** for response handling
3. **Maintain connection state** (connected, watching, error)
4. **Handle reconnection** on socket errors
5. **Parse events** and dispatch to UI update handlers

### Connection Management

```rust
// Example connection handling
async fn connect(socket_path: &str) -> Result<UnixStream, Error> {
    let stream = tokio::net::UnixStream::connect(socket_path).await?;
    Ok(stream)
}

async fn send_command(stream: &mut UnixStream, cmd: &CliCommand) -> Result<DaemonEvent, Error> {
    let json = serde_json::to_string(cmd)?;
    stream.write_all(json.as_bytes()).await?;
    stream.flush().await?;

    let mut buf = Vec::new();
    stream.read_buf(&mut buf).await?;
    let response: DaemonEvent = serde_json::from_slice(&buf)?;
    Ok(response)
}
```

### Watch Stream Handling

```rust
async fn watch_task(stream: &mut UnixStream, task_id: Uuid) -> Result<(), Error> {
    let cmd = CliCommand::TaskWatch { task_id };
    let json = serde_json::to_string(&cmd)?;
    stream.write_all(json.as_bytes()).await?;
    stream.flush().await?;

    let mut reader = tokio::io::BufReader::new(stream);
    let mut line = String::new();

    loop {
        reader.read_line(&mut line).await?;
        if line.is_empty() {
            break;
        }

        let event: DaemonEvent = serde_json::from_str(&line)?;

        // Check for terminal state
        if is_terminal_event(&event) {
            break;
        }

        // Process event...
        line.clear();
    }

    Ok(())
}
```

### Error Recovery

| Error | Recovery Strategy |
|-------|------------------|
| Socket not found | Retry with backoff, show "daemon not running" |
| Connection timeout | Retry once, then show timeout error |
| Request timeout | Retry command, increase timeout |
| Parse error | Log error, close connection |
| Server error | Display error message to user |

---

## Example Client

### Python Example

```python
import socket
import json
import uuid

SOCKET_PATH = "/tmp/swell-daemon.sock"

def send_command(command_type: str, payload: dict = None) -> dict:
    """Send a command and return the response."""
    payload = payload or {}
    command = {"type": command_type, "payload": payload}

    sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    sock.connect(SOCKET_PATH)
    sock.settimeout(30)

    sock.sendall((json.dumps(command) + "\n").encode())

    response = sock.recv(65536).decode()
    sock.close()

    return json.loads(response)

def watch_task(task_id: str):
    """Watch a task and yield events."""
    command = {
        "type": "TaskWatch",
        "payload": {"task_id": task_id}
    }

    sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    sock.connect(SOCKET_PATH)
    sock.settimeout(30)

    sock.sendall((json.dumps(command) + "\n").encode())

    terminal_states = {"ACCEPTED", "REJECTED", "FAILED", "ESCALATED"}

    while True:
        line = sock.recv(65536).decode()
        if not line:
            break

        event = json.loads(line)
        yield event

        if event["type"] == "TaskStateChanged":
            if event["payload"]["state"] in terminal_states:
                break

    sock.close()

# Usage
task = send_command("TaskCreate", {"description": "Hello world"})
print(f"Created task: {task['payload']['id']}")

events = watch_task(task['payload']['id'])
for event in events:
    print(event)
```

### Rust Example

```rust
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
struct Command {
    #[serde(rename = "type")]
    cmd_type: String,
    payload: serde_json::Value,
}

fn send_command(socket_path: &str, cmd: &Command) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let mut stream = UnixStream::connect(socket_path)?;

    let json = serde_json::to_string(cmd)?;
    stream.write_all(json.as_bytes())?;
    stream.write_all(b"\n")?;
    stream.flush()?;

    let mut response = Vec::new();
    stream.read_to_end(&mut response)?;

    let response_str = String::from_utf8_lossy(&response);
    let value: serde_json::Value = serde_json::from_str(&response_str)?;
    Ok(value)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cmd = Command {
        cmd_type: "TaskCreate".to_string(),
        payload: serde_json::json!({"description": "Hello world"}),
    };

    let response = send_command("/tmp/swell-daemon.sock", &cmd)?;
    println!("Response: {:?}", response);

    Ok(())
}
```

---

## Exit Codes

When using the `swell` CLI binary:

| Code | Meaning |
|------|---------|
| 0 | Success |
| 1 | Server error or internal error |
| 2 | Invalid command or arguments |
| 10 | Connection failed (daemon not running) |
| 11 | Timeout |

---

## See Also

- [swell-cli](../clients/swell-cli/AGENTS.md) - CLI client documentation
- [swell-daemon](../crates/swell-daemon/AGENTS.md) - Daemon internals
- [swell-core](../crates/swell-core/AGENTS.md) - Core types and traits
