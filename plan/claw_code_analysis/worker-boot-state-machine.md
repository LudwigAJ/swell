# Worker Boot State Machine

## Overview

The worker boot subsystem in `references/claw-code/rust/crates/runtime/src/worker_boot.rs` implements an explicit state machine that governs a coding worker's lifecycle from spawn through prompt delivery to completion or failure. The state machine is the primary control plane for reliable worker startup: trust-gate detection, ready-for-prompt handshakes, and prompt-misdelivery detection and recovery all live above raw terminal transport.

## Landed WorkerStatus Enum

The canonical status enum is `WorkerStatus` (snake_case serialization):

```rust
pub enum WorkerStatus {
    Spawning,
    TrustRequired,
    ReadyForPrompt,
    Running,
    Finished,
    Failed,
}
```

These six states cover the full worker lifecycle:

| State | Meaning |
|-------|---------|
| `Spawning` | Worker process created; boot sequence in progress |
| `TrustRequired` | Trust prompt detected; worker blocked until trust is resolved |
| `ReadyForPrompt` | Worker has cleared trust gate and is ready to receive a task prompt |
| `Running` | Prompt dispatched; worker is actively processing |
| `Finished` | Session completed normally (finish reason is known, non-provider-error) |
| `Failed` | Terminal failure — trust gate unresolved, prompt misdelivery, or provider error |

**Important:** `WorkerStatus` is the only backed enum. The states above are the complete set. No other status values exist in the implementation.

## State Machine Transitions

Transitions are driven by three inputs:

1. **`WorkerRegistry::observe()`** — reads screen text from the worker terminal and detects trust prompts, ready-for-prompt cues, running cues, and prompt misdelivery patterns
2. **`WorkerRegistry::send_prompt()`** — explicitly transitions a `ReadyForPrompt` worker to `Running`
3. **`WorkerRegistry::resolve_trust()`** — manually resolves a `TrustRequired` hold
4. **`WorkerRegistry::restart()`** — resets a worker back to `Spawning`
5. **`WorkerRegistry::terminate()`** — forces a worker to `Finished`
6. **`WorkerRegistry::observe_completion()`** — classifies a session finish as `Finished` or `Failed` based on finish reason and token count

### Trust Gate Flow

```
Spawning → TrustRequired → (auto-allowlist or manual resolve) → Spawning → ReadyForPrompt
```

- Trust prompt detection scans screen text for phrases like `"do you trust the files in this folder"`, `"trust this folder"`, `"allow and continue"`, `"yes, proceed"`
- If the worker's `cwd` matches a configured `trusted_root` allowlist, `trust_auto_resolve` is set and the gate clears automatically
- Non-allowlisted workers transition to `TrustRequired` and remain blocked until `resolve_trust()` is called
- `trust_gate_cleared` is a boolean flag on the `Worker` struct; once set it is not reset except by `restart()`

### Prompt Delivery Flow

```
ReadyForPrompt → Running → ReadyForPrompt (on misdelivery + recovery armed)
                → Finished (normal completion)
                → Failed (provider error: finish="unknown" + zero tokens, or finish="error")
```

- `send_prompt()` requires the worker to already be in `ReadyForPrompt`; sending to any other state returns an error
- A `prompt_in_flight` flag tracks whether a prompt has been dispatched but not yet acknowledged
- `prompt_delivery_attempts` counts delivery attempts (used for recovery decisions)
- Running cue detection looks for `"thinking"`, `"working"`, `"running tests"`, `"inspecting"`, `"analyzing"` in screen text

### Prompt Misdelivery Detection

The state machine detects three classes of misdelivery via `detect_prompt_misdelivery()`:

| Target | Detection Trigger |
|--------|-------------------|
| `Shell` | Prompt text visible in screen output AND shell error (e.g., `"command not found"`) |
| `WrongTarget` | Prompt visible AND observed CWD differs from expected `cwd` |
| `WrongTask` | Prompt visible but task receipt tokens missing from screen output |

When misdelivery is detected and `auto_recover_prompt_misdelivery` is `true`, the worker transitions to `ReadyForPrompt` with `replay_prompt` armed for automatic retry.

## Failure Taxonomy

`WorkerFailureKind` categorizes terminal failures:

```rust
pub enum WorkerFailureKind {
    TrustGate,       // Trust prompt unresolved
    PromptDelivery,  // Prompt misdelivered and recovery not armed or failed
    Protocol,        // Unexpected protocol state
    Provider,        // API provider error (finish="unknown" + 0 tokens, or finish="error")
}
```

Each failure carries a human-readable `message` and a `created_at` timestamp. Failures are recorded in the worker's `last_error` field and emitted as part of the `WorkerEvent` sequence.

## Event Sequence

`WorkerEventKind` enumerates every state change:

```rust
pub enum WorkerEventKind {
    Spawning,
    TrustRequired,
    TrustResolved,
    ReadyForPrompt,
    PromptMisdelivery,
    PromptReplayArmed,
    Running,
    Restarted,
    Finished,
    Failed,
}
```

Events are stored in the `Worker::events` vector with a sequential `seq` number, `timestamp`, `status` snapshot, optional `detail` string, and optional typed `payload` (`WorkerEventPayload`). The event sequence provides a full audit trail of every state transition and detection decision.

## Roadmap vs. Landed Naming

The ROADMAP.md Phase 1 describes an intended lifecycle with these names:

> `spawning`, `trust_required`, `ready_for_prompt`, `prompt_accepted`, `running`, `blocked`, `finished`, `failed`

Two roadmap names do **not** appear in the landed `WorkerStatus` enum:

- **`prompt_accepted`** — The roadmap intended a distinct state between `ready_for_prompt` and `running`. In the implementation, `send_prompt()` transitions directly from `ReadyForPrompt` to `Running` without an intermediate enum variant. The `prompt_in_flight` boolean field tracks the in-flight state internally but it is not a separate `WorkerStatus` variant.
- **`blocked`** — The roadmap anticipated a generic blocked state. In the implementation, the only blocked state is `TrustRequired` (explicitly named). The roadmap's `blocked` is not a landed variant.

Builders reading ROADMAP.md should not use `prompt_accepted` or `blocked` as `WorkerStatus` values — they do not exist in the enum and will not serialize or match correctly.

## Observability Surface

The canonical observability surface is file-based, not HTTP-based. This is a deliberate design choice: claw-code runs as a plugin inside the `opencode` binary, which owns its HTTP server. claw-code cannot add routes to the opencode server, so state is written to a well-known file path that external observers (clawhip, orchestrators) can poll.

### `.claw/worker-state.json`

`emit_state_file()` in `worker_boot.rs` writes a JSON snapshot to `.claw/worker-state.json` under the worker's `cwd` on **every state transition**. The write is atomic (write to temp file, then rename). The snapshot schema:

```json
{
  "worker_id": "worker_abc123",
  "status": "ready_for_prompt",
  "is_ready": true,
  "trust_gate_cleared": true,
  "prompt_in_flight": false,
  "last_event": { /* WorkerEvent */ },
  "updated_at": 1713000000,
  "seconds_since_update": 5
}
```

Key fields:
- `status` — the current `WorkerStatus` as a snake_case string
- `is_ready` — `true` when `status == "ready_for_prompt"`
- `trust_gate_cleared` — whether the trust gate has been resolved
- `prompt_in_flight` — whether a prompt has been dispatched and not yet acknowledged
- `seconds_since_update` — seconds elapsed since the last state transition; clawhip uses this to detect stalled workers without computing epoch deltas

### `claw state` CLI Command

`run_worker_state()` in `rust/crates/rusty-claude-cli/src/main.rs` reads `.claw/worker-state.json` from the current working directory and prints it. Supports both text and JSON output (`--output-format json`).

```bash
claw state
claw state --output-format json
```

Exit code is 0 on success, 1 if the file does not exist (no worker has been created in this directory yet).

## Control API

`WorkerRegistry` exposes a structured control API above raw terminal send-keys:

| Method | Precondition | Effect |
|--------|-------------|--------|
| `create(cwd, trusted_roots, auto_recover_prompt_misdelivery)` | — | Creates worker in `Spawning` |
| `get(worker_id)` | Worker exists | Returns full `Worker` snapshot |
| `observe(worker_id, screen_text)` | Worker exists | Detects trust prompts, ready cues, misdelivery; updates status |
| `resolve_trust(worker_id)` | `TrustRequired` | Clears trust gate; transitions to `Spawning` |
| `send_prompt(worker_id, prompt, task_receipt)` | `ReadyForPrompt` | Dispatches prompt; transitions to `Running` |
| `await_ready(worker_id)` | Worker exists | Returns `WorkerReadySnapshot` with `ready`/`blocked`/`replay_prompt_ready` |
| `restart(worker_id)` | Worker exists | Resets to `Spawning` |
| `terminate(worker_id)` | Worker exists | Forces `Finished` |
| `observe_completion(worker_id, finish_reason, tokens_output)` | Worker exists | Classifies as `Finished` or `Failed` |

The `await_ready()` method is the primary mechanism for a claw to wait for a worker to become ready without polling `observe()` directly. It returns immediately with `ready: false` if the worker is not yet in `ReadyForPrompt`.

## Design Lessons for Builders

1. **File-based observability is sufficient for local workers.** Writing state to `.claw/worker-state.json` avoids the fragility of HTTP sidecar processes or requiring upstream patches to the host binary. Polling a local file has lower latency and fewer failure modes than an HTTP round-trip.

2. **Typed events over prose.** `WorkerEvent` sequences encode every state change with a typed `kind`, a status snapshot, and optional structured payload. This gives clawhip a machine-readable audit trail instead of requiring it to scrape terminal output.

3. **Explicit state beats implicit flags.** Rather than storing `is_bootstrapped`, `trust_resolved`, and `prompt_sent` as independent booleans, the `WorkerStatus` enum makes the valid transitions explicit and prevents illegal states (e.g., calling `send_prompt()` when not `ReadyForPrompt`).

4. **Recovery arming pattern.** `auto_recover_prompt_misdelivery` and `replay_prompt` demonstrate a pattern where the system detects a failure, records the recovery data, and stays in a state where the next action can retry without requiring a full restart.

5. **Auto-trust via path allowlist.** `trust_auto_resolve` shows how to combine explicit human approval with automated allowlisting for known-good paths, avoiding manual intervention on every boot in trusted environments.

## Evidence

- `references/claw-code/rust/crates/runtime/src/worker_boot.rs` — `WorkerStatus` enum, `WorkerRegistry`, `emit_state_file()`, state machine logic, and tests
- `references/claw-code/rust/crates/rusty-claude-cli/src/main.rs` — `run_worker_state()` CLI command
- `references/claw-code/ROADMAP.md` — Phase 1 lifecycle naming (used to identify roadmap vs. landed naming drift)
- `references/claw-code/PARITY.md` — parity framing noting team/cron as registry-backed with scheduler gap
