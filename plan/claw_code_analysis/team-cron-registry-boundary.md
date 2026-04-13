# Team and Cron Registries: Lifecycle Backing vs. Full Scheduler Boundary

## Overview

`TeamRegistry` and `CronRegistry` in `references/claw-code/rust/crates/runtime/src/team_cron_registry.rs` provide real in-memory lifecycle backing for team and cron tools. They replace the earlier stub implementations that returned fixed dummy payloads. However, they stop short of a full background scheduler or worker fleet — they manage in-memory state but do not themselves execute periodic work or spawn background processes.

## What the Registries Actually Provide

The registries are in-memory, thread-safe (`Arc<Mutex<...>)`) data stores for team and cron records.

### TeamRegistry

**Location:** `references/claw-code/rust/crates/runtime/src/team_cron_registry.rs`

**Operations:**
- `create(name, task_ids)` — spawns a team record with a generated `team_id` and `TeamStatus::Created`
- `get(team_id)` — returns a clone of the stored `Team` struct
- `list()` — returns all teams as a `Vec<Team>`
- `delete(team_id)` — soft-deletes (sets `TeamStatus::Deleted`, leaves record in the map)
- `remove(team_id)` — hard-removes the record from the map
- `len()` / `is_empty()` — cardinality queries

**Team record shape:**
```rust
pub struct Team {
    pub team_id: String,
    pub name: String,
    pub task_ids: Vec<String>,
    pub status: TeamStatus,   // Created | Running | Completed | Deleted
    pub created_at: u64,
    pub updated_at: u64,
}
```

**Lifecycle states:** `Created → Running → Completed | Deleted`

A `delete` call does not physically remove the team from the registry; it transitions the status to `Deleted`. A separate `remove` call does the physical deletion.

### CronRegistry

**Location:** `references/claw-code/rust/crates/runtime/src/team_cron_registry.rs`

**Operations:**
- `create(schedule, prompt, description)` — creates a `CronEntry` with a generated `cron_id`
- `get(cron_id)` — returns a clone of the stored entry
- `list(enabled_only)` — filters by the `enabled` flag
- `delete(cron_id)` — physical removal from the map
- `disable(cron_id)` — sets `enabled = false` without removing the entry
- `record_run(cron_id)` — increments `run_count` and stamps `last_run_at` with the current Unix timestamp

**CronEntry record shape:**
```rust
pub struct CronEntry {
    pub cron_id: String,
    pub schedule: String,       // cron expression as a string
    pub prompt: String,         // the prompt text to execute
    pub description: Option<String>,
    pub enabled: bool,
    pub created_at: u64,
    pub updated_at: u64,
    pub last_run_at: Option<u64>,
    pub run_count: u64,
}
```

The registry tracks `run_count` and `last_run_at` — but these are record-keeping fields, not execution triggers.

## The Boundary: No Background Scheduler or Worker Fleet

The registries manage lifecycle state for teams and crons, but they do **not** include:

1. **No tick loop** — `CronRegistry` does not run a background timer that fires on cron expression matches. It stores schedules and records run timestamps, but nothing polls or fires.
2. **No worker fleet** — `TeamRegistry` tracks which tasks belong to a team and the team's status, but does not spawn workers, assign work, or track their progress in a subprocess sense.
3. **No execution dispatch** — calling `record_run` updates the entry's `last_run_at` and `run_count` fields, but nothing automatically evaluates the cron expression and triggers a prompt.

In other words, the registries answer the question "what teams and crons exist and what is their state?" — not "when should something run and who does it?"

This matches the PARITY.md framing for Lane 6 (Team+Cron):

> "Current state: team/cron tools now have in-memory lifecycle behavior on `main`; they still stop short of a real background scheduler or worker fleet."
> — `references/claw-code/PARITY.md` ("Lane details / Lane 6 — Team+Cron")

## How the Registries Are Wired

The tool dispatch layer in `references/claw-code/rust/crates/tools/src/lib.rs` maps tool schemas to registry operations:

- `TeamCreate` → `TeamRegistry::create()`
- `TeamDelete` → `TeamRegistry::delete()`
- `CronCreate` → `CronRegistry::create()`
- `CronDelete` → `CronRegistry::delete()`
- `CronList` → `CronRegistry::list(enabled_only: false)`

These form the executable surface for team and cron management. The lifecycle is real and persisted in memory, but the "when and who executes the prompt" question is left to a future component.

## Builder Lessons

1. **Registry-backed vs. executor-backed are separate concerns.** A registry that stores "what should run and when" is architecturally distinct from the component that actually ticks and dispatches work. Confusing them produces a blurred responsibility boundary that is hard to extend.

2. **Soft-delete leaves audit data.** `TeamRegistry::delete()` transitions status rather than removing the record, preserving the history of what existed. Hard removal via `remove()` is a separate, explicit operation. This pattern keeps deleted entries queryable for debugging or recovery without retaining live state.

3. **Thread-safety is a one-liner with `Arc<Mutex<...>>`.** The registry wraps its inner state in `Arc<Mutex<TeamInner>>` and exposes clean API methods — no lock granularization needed at this scale. The lock is held only for the duration of a single operation, not across a multi-step transaction.

4. **Run bookkeeping and execution are separate.** `CronEntry.run_count` and `last_run_at` are updated by explicit `record_run()` calls, meaning the act of recording a run is decoupled from the act of triggering one. This makes testing and audit trails straightforward.

5. **The parity gap is explicitly documented.** PARITY.md clearly states the remaining gap — the registry lifecycle is complete, but the scheduler/execution loop is not yet built. This is the right way to represent partial parity: show what exists, name what doesn't, and do not overclaim.

## Code References

| Symbol | Location |
|--------|----------|
| `TeamRegistry` | `references/claw-code/rust/crates/runtime/src/team_cron_registry.rs` |
| `CronRegistry` | `references/claw-code/rust/crates/runtime/src/team_cron_registry.rs` |
| `Team`, `TeamStatus` | `references/claw-code/rust/crates/runtime/src/team_cron_registry.rs` |
| `CronEntry` | `references/claw-code/rust/crates/runtime/src/team_cron_registry.rs` |
| Tool dispatch wiring | `references/claw-code/rust/crates/tools/src/lib.rs` |
| Parity status (Lane 6) | `references/claw-code/PARITY.md` ("Lane 6 — Team+Cron") |
