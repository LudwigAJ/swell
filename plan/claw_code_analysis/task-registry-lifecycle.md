# TaskRegistry Lifecycle

The `TaskRegistry` in `references/claw-code/rust/crates/runtime/src/task_registry.rs` is an in-memory, thread-safe task registry that backs the task coordination surface. It is not a stub — it is a full lifecycle manager with explicit state transitions, structured task records, and wired tool dispatch.

## What makes TaskRegistry more than a stub

A stub would return fixed payloads or empty responses without tracking state. `TaskRegistry` does the opposite:

- **Thread-safe interior**: `Arc<Mutex<RegistryInner>>` where `RegistryInner` holds a `HashMap<String, Task>`. The registry is a shared, lock-protected map — safe for concurrent access from any number of tool handlers.
- **Generated task IDs**: IDs are not passed in; they are minted as `task_{timestamp}_{counter}` on creation.
- **Full task record**: Each `Task` carries prompt, description, optional `TaskPacket`, status, timestamps, message history, accumulated output, and optional team assignment.
- **Structured state machine**: `TaskStatus` has five explicit variants — `Created`, `Running`, `Completed`, `Failed`, `Stopped` — with enforcement that terminal states reject further transitions.
- **TaskPacket validation**: `create_from_packet` validates the packet schema before minting a task, integrating with `TaskPacketValidationError`.

## Concrete lifecycle operations

`TaskRegistry` exposes these public operations (all in `task_registry.rs`):

| Operation | Signature | Behavior |
|---|---|---|
| `create` | `(&self, prompt: &str, description: Option<&str>) -> Task` | Mints a task with `Created` status |
| `create_from_packet` | `(&self, packet: TaskPacket) -> Result<Task, TaskPacketValidationError>` | Validates packet, creates task with packet attached |
| `get` | `(&self, task_id: &str) -> Option<Task>` | Fetches a task by ID |
| `list` | `(&self, status_filter: Option<TaskStatus>) -> Vec<Task>` | Lists all tasks or filters by status |
| `stop` | `(&self, task_id: &str) -> Result<Task, String>` | Transitions task to `Stopped`; rejects terminal-state tasks |
| `update` | `(&self, task_id: &str, message: &str) -> Result<Task, String>` | Appends a `TaskMessage` to the task's message history |
| `output` | `(&self, task_id: &str) -> Result<String, String>` | Returns accumulated output string |
| `append_output` | `(&self, task_id: &str, output: &str) -> Result<(), String>` | Appends to the output string |
| `set_status` | `(&self, task_id: &str, status: TaskStatus) -> Result<(), String>` | Sets status directly; no guard on terminal transitions |
| `assign_team` | `(&self, task_id: &str, team_id: &str) -> Result<(), String>` | Assigns a team ID to the task |
| `remove` | `(&self, task_id: &str) -> Option<Task>` | Hard-deletes the task record |
| `len` | `(&self) -> usize` | Returns task count |
| `is_empty` | `(&self) -> bool` | Returns whether count is zero |

## TaskStatus state machine

The `TaskStatus` enum (`task_registry.rs`) defines five states:

```rust
pub enum TaskStatus {
    Created,
    Running,
    Completed,
    Failed,
    Stopped,
}
```

`stop` enforces that `Completed`, `Failed`, and `Stopped` are terminal — calling `stop` on a task in any of these states returns `Err("task {id} is already in terminal state: {status}")`. `set_status` does not enforce this, allowing `Running` or other transitions to be set explicitly by callers.

## TaskPacket integration

`TaskPacket` is a structured contract that can back task creation:

```rust
pub struct TaskPacket {
    pub objective: String,
    pub scope: String,
    pub repo: String,
    pub branch_policy: String,
    pub acceptance_tests: Vec<String>,
    pub commit_policy: String,
    pub reporting_contract: String,
    pub escalation_policy: String,
}
```

`create_from_packet` calls `validate_packet` before inserting, ensuring malformed packets are rejected rather than stored. The resulting task's `description` field is set to the packet's `scope`, and the full packet is stored in `task.task_packet`.

## Tool wiring

`task_registry.rs` is not self-serving — it is wired into the tool system via `tools/src/lib.rs`. The `global_task_registry()` singleton (a `OnceLock`-initialized `TaskRegistry`) is called by six tool handlers:

- `run_task_create` → `registry.create()`
- `run_task_packet` → `registry.create_from_packet()`
- `run_task_get` → `registry.get()`
- `run_task_list` → `registry.list(None)`
- `run_task_stop` → `registry.stop()`
- `run_task_update` → `registry.update()`
- `run_task_output` → `registry.output()`

Team assignment also writes back to the task registry: when `TeamCreate` runs, it calls `global_task_registry().assign_team(task_id, &team.team_id)` for each member task.

## Design lessons

**Lesson 1: Registry-backed over stub-payload.** The previous task tools returned fixed or empty payloads. `TaskRegistry` replaced those with real in-memory state. If you are building a coordination system, prefer a shared registry with real operations over ad hoc response functions.

**Lesson 2: Thread-safe shared state is cheap to add.** The registry uses `Arc<Mutex<...>>` — six lines of interior pattern. This makes the registry safe to use as a global singleton without external synchronization primitives.

**Lesson 3: Structured state machines over unstructured strings.** `TaskStatus` as an enum with `Display` impl and terminal-state enforcement makes invalid transitions a programming error rather than a runtime surprise. The same pattern in `TeamStatus` and `CronEntry` shows the idiom reused consistently.

**Lesson 4: Validation at creation time, not at use time.** `create_from_packet` validates the `TaskPacket` before storing it. This prevents malformed contracts from entering the system and surfaces errors to the caller immediately.

**Lesson 5: Soft delete is not the same as hard delete.** `stop` transitions to `Stopped` but keeps the record; `remove` hard-deletes. The distinction matters for auditability and recovery. `TeamRegistry::delete` follows the same pattern — `status = Deleted` preserves the record.

## Scope boundary

`TaskRegistry` manages in-memory task lifecycle only. It does not execute subprocesses or schedule background work. External subprocess execution and worker fleet management are handled by separate subsystems (see `worker_boot.rs` and `team_cron_registry.rs`).

## Evidence

- `references/claw-code/rust/crates/runtime/src/task_registry.rs` — 335 LOC, full registry implementation
- `references/claw-code/rust/crates/tools/src/lib.rs` — task tool wiring (Lane 5, merged commit `d994be6`)
- `references/claw-code/PARITY.md` — Lane 4 status: merged on `main` (commit `21a1e1d`), replaces stub task state with real registry
