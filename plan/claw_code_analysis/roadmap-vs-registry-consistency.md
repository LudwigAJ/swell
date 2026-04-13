# Roadmap vs. Registry Consistency: Shipped vs. Aspirational in Task/Team/Cron Coordination

## Context

This document reconciles three sources of truth about ClawCode's coordination surfaces:

- **`ROADMAP.md`** — records intent, planned phases, and product principles for clawable coding harness behavior.
- **`PARITY.md`** — records the verified implementation status of each lane, naming what is merged on `main` and what remains branch-only or aspirational.
- **Landed implementation** (`task_registry.rs`, `team_cron_registry.rs`, `worker_boot.rs`, lane event schemas) — the concrete Rust code that backs coordination tools.

The goal is to establish clear rules for keeping generated analysis documents consistent with each other and with reality: when something is shipped, it should be named as shipped; when it is aspirational, it should be named as such and not conflated with landed behavior.

## The Core Consistency Problem

Generated analysis documents covering task, team, and cron coordination surfaces risk three failure modes:

1. **Overclaiming from roadmap language.** ROADMAP.md describes a full autonomous clawable harness with background schedulers, event-native clawhip integration, and policy-driven execution engines. A document that cites Phase 4 ("Claws-First Task Execution") language while describing current `main` behavior will overstate what is actually implemented.

2. **Underclaiming from implementation-only framing.** A document that only describes `TaskRegistry` as an in-memory map and never connects it to the parity lane that justified its existence (PARITY.md Lane 4: TaskRegistry) misses the coordination story entirely.

3. **Stale parity totals.** PARITY.md reports a fixed scenario count and lane status that can drift as new scenarios land. A document that cites "9 lanes merged" without a timestamp or HEAD commit reference may become inaccurate as the repo advances.

## The Landed Coordination Surfaces

### TaskRegistry

**Source:** `references/claw-code/rust/crates/runtime/src/task_registry.rs` (335 LOC, merged on `main`, Lane 4)

**What it is:** A thread-safe in-memory registry with full task lifecycle operations — `create`, `get`, `list`, `stop`, `update`, `output`, `append_output`, `set_status`, `assign_team`. Task IDs are generated as `task_{timestamp}_{counter}`. `TaskStatus` has five explicit variants: `Created → Running → Completed | Failed | Stopped`.

**What it is not:** An execution engine. It does not spawn subprocesses, fork workers, or consume task packets to drive code changes. `TaskPacket` validation is integrated (`create_from_packet` validates before storing), but the packet's `acceptance_tests`, `commit_policy`, and `escalation_policy` fields are record-keeping — nothing automatically interprets or executes them.

**Parity framing:** PARITY.md Lane 4 (TaskRegistry) explicitly records that the registry replaces stub task state with real in-memory backing. The lane is merged on `main` (commit `21a1e1d`).

### TeamRegistry and CronRegistry

**Source:** `references/claw-code/rust/crates/runtime/src/team_cron_registry.rs` (363 LOC, merged on `main`, Lane 6)

**What they are:** Thread-safe in-memory registries for team and cron record lifecycle. `TeamRegistry` exposes `create`, `get`, `list`, `delete`, `remove`. `CronRegistry` exposes `create`, `get`, `list`, `delete`, `disable`, `record_run`. Both track timestamps and support soft-delete (`status = Deleted` instead of physical removal).

**What they are not:** A background scheduler or worker fleet. The explicit PARITY.md Lane 6 framing states: "team/cron tools now have in-memory lifecycle behavior on `main`; they still stop short of a real background scheduler or worker fleet." `CronRegistry` stores cron expressions and records runs via explicit `record_run()` calls — nothing automatically evaluates the schedule and fires.

**Parity framing:** PARITY.md Lane 6 correctly names the boundary. Documents covering team/cron must reproduce this distinction: registry lifecycle is shipped, scheduler/execution is aspirational.

### WorkerStatus State Machine

**Source:** `references/claw-code/rust/crates/runtime/src/worker_boot.rs`

**Land statuses (shipped on `main`):** `Spawning`, `TrustRequired`, `ReadyForPrompt`, `Running`, `Finished`, `Failed`.

**Roadmap-phase names (aspirational, not landed):** The ROADMAP.md Phase 1 description lists states `spawning`, `trust_required`, `ready_for_prompt`, `prompt_accepted`, `running`, `blocked`, `finished`, `failed`. Note that `prompt_accepted` and `blocked` do not appear in the landed `WorkerStatus` enum on `main`. The landed enum has six states; the roadmap describes eight. Documents must not present the roadmap's extended state list as the current implementation.

The landed `WorkerStatus` is the only backed enum. The roadmap language represents intended expansion, not current behavior.

### Lane Events

**Source:** `references/claw-code/rust/crates/runtime/src/lane_events.rs` and ROADMAP.md Phase 2

**Shipped (landed on `main`):** Typed `LaneEvent` enum with `Started`, `Blocked`, `Failed`, `Finished` variants, wired into `tools/src/lib.rs`. Lane completion detection (`LaneContext::completed` auto-set from session-finished + tests-green + push-complete) is shipped.

**Aspirational (from roadmap):** Full canonical lane event schema with `lane.started`, `lane.ready`, `lane.prompt_misdelivery`, `lane.red`, `lane.green`, `lane.commit.created`, `lane.pr.opened`, `lane.merge.ready`, `branch.stale_against_main`. The roadmap enumerates a richer event vocabulary than what is currently wired.

Documents must distinguish: the typed `LaneEvent` enum is landed; the full `lane.*` vocabulary from the roadmap is aspirational.

## Rules for Keeping Documents Consistent

### Rule 1: Parity framing is the authoritative shipped/ aspirational filter

When a document describes task, team, or cron behavior, the first question is: does PARITY.md name this as merged on `main`?

- If yes: describe the implementation as shipped and cite the lane commit.
- If no (branch-only, pending, or not mentioned): describe it as aspirational or not-yet-landed and do not cite it as current behavior.

Example: "TaskRegistry provides in-memory task lifecycle on `main` (PARITY.md Lane 4, merged commit `21a1e1d`)" is accurate. "TaskRegistry autonomously executes task packets" overclaims — nothing in the implementation does that.

### Rule 2: Roadmap phase language is aspirational, not implementation description

ROADMAP.md is written as a product planning document. Its language describes intent and future states. Documents that borrow roadmap phrasing to describe current behavior will mislead readers about what is actually implemented.

Specifically:

- "Phase 4 — Claws-First Task Execution" describes intent for structured task packets and autonomous execution. The `TaskPacket` struct exists on `main`, but the execution loop that consumes it is not implemented.
- Phase 1 ready-handshake lifecycle states include `prompt_accepted` and `blocked` that are not in the landed `WorkerStatus` enum.
- Phase 2 lane event schema includes `lane.prompt_misdelivery` and `lane.pr.opened` that are not in the current `LaneEvent` enum.

When citing roadmap language, prefix it explicitly: "ROADMAP.md Phase 1 describes intent for `prompt_accepted` and `blocked` states that are not yet in the landed `WorkerStatus` enum."

### Rule 3: Team and cron are registry-backed, not executor-backed

The explicit PARITY.md Lane 6 framing — "in-memory lifecycle behavior... still stop short of a real background scheduler or worker fleet" — must be reproduced in any document that covers team or cron. This is not a minor caveat; it is the core scope boundary.

A document that describes `CronRegistry` as scheduling cron jobs misrepresents the implementation. The correct description: `CronRegistry` stores cron entries and records when `record_run()` is called, but nothing automatically evaluates the cron expression. The execution trigger is external.

### Rule 4: WorkerStatus names are landed; roadmap lifecycle names are distinct

VAL-COORD-009 requires that documents not present roadmap lifecycle names as current `WorkerStatus` enum values without qualification.

The landed enum: `Spawning`, `TrustRequired`, `ReadyForPrompt`, `Running`, `Finished`, `Failed`.

The roadmap's intended expansion: `spawning`, `trust_required`, `ready_for_prompt`, `prompt_accepted`, `running`, `blocked`, `finished`, `failed`.

These are different sets. Documents must use the landed names for present-tense behavior and explicitly note roadmap intent when the roadmap names are relevant.

### Rule 5: Observability surface is file/CLI, not HTTP

ROADMAP.md Phase 1 describes a "structured session control API" with explicit acceptance criteria around creating workers, awaiting ready, sending tasks, and fetching state. The actual shipped surface is `claw state` reading `.claw/worker-state.json` (file-based), not an HTTP endpoint.

The discrepancy is documented in ROADMAP.md itself under "Observability Transport Decision": the canonical state surface is CLI/file-based, HTTP endpoint is deferred because claw-code is a plugin inside the opencode binary and cannot add HTTP routes to the upstream server.

Documents covering worker observability must describe the file/CLI surface as the shipped state and note that HTTP state endpoints are not implemented.

### Rule 6: Parity totals must include a timestamp or HEAD reference

PARITY.md reports "9-lane checkpoint: All 9 lanes merged on `main`" with a date ("Last updated: 2026-04-03") and HEAD (`ee31e00`). A document that says "all 9 lanes are merged" without those qualifiers will drift as the repo advances.

When citing parity totals, include the date or HEAD reference so readers can verify against the current repo state.

## Cross-Document Consistency Check

The following table maps coordination surface → shipped behavior → aspirational behavior → which documents cover each:

| Surface | Shipped (on `main`) | Aspirational (from ROADMAP) | Key docs |
|---|---|---|---|
| `TaskRegistry` | In-memory lifecycle, `TaskStatus` state machine, wired tool dispatch | Autonomous task execution from `TaskPacket` | `task-registry-lifecycle.md`, this document |
| `TeamRegistry` | In-memory team lifecycle, soft-delete, status transitions | Team-level execution and work distribution | `team-cron-registry-boundary.md`, this document |
| `CronRegistry` | In-memory cron lifecycle, `record_run` bookkeeping | Background tick loop that fires on cron expression | `team-cron-registry-boundary.md`, this document |
| `WorkerStatus` | 6-state enum: `Spawning`, `TrustRequired`, `ReadyForPrompt`, `Running`, `Finished`, `Failed` | 8-state expansion including `prompt_accepted`, `blocked` | `worker-boot-state-machine.md` |
| Lane events | `LaneEvent::Started/Blocked/Failed/Finished`, lane completion detection | Full `lane.*` schema including `prompt_misdelivery`, `pr.opened`, `stale_against_main` | `lane-events-schema.md` |
| Worker observability | `.claw/worker-state.json` file emission, `claw state` CLI | HTTP `/state` endpoint (deferred — opencode plugin constraint) | `worker-boot-state-machine.md` |

## Builder Lessons

**Lesson 1: Implementation is the floor, not the ceiling.** The fact that `TaskRegistry` and `CronRegistry` exist and are wired does not mean the full coordination story is implemented. Treat each registry as a foundation that enables higher-level behavior, not as proof that higher-level behavior exists.

**Lesson 2: Parity framing prevents both overclaim and underclaim.** PARITY.md is the joint constraint: it prevents documents from ignoring what exists (underclaim) and prevents them from borrowing roadmap language as if it were current behavior (overclaim). Always check the parity status before describing a coordination surface as shipped.

**Lesson 3: Explicit shipped/aspirational framing is a correctness requirement, not a style preference.** In a coordination system where roadmap language describes a future state machine and the current implementation is a subset of that, conflating the two produces documents that tell readers the wrong thing about what is actually available to build with today.

**Lesson 4: Registry backing and execution are separate architectural concerns.** Both `TaskRegistry` and `CronRegistry` demonstrate this: the registry stores state and exposes operations, but the execution loop that would consume that state to drive autonomous behavior is a separate component that does not yet exist. Keeping them separate makes it clear what is available to extend.

**Lesson 5: File/CLI observability is the right default for a plugin architecture.** Because claw-code runs inside an upstream binary, it cannot add HTTP routes. File-based state emission (`worker_boot.rs` writes `.claw/worker-state.json`) is the correct pattern. The roadmap describes HTTP endpoints as desired future state; the current implementation uses files. Documents covering observability must make this distinction explicit.

## Evidence

| Source | What it contributes |
|---|---|
| `references/claw-code/ROADMAP.md` | Intent and product principles; phase-based roadmap language; explicit deferral of HTTP observability |
| `references/claw-code/PARITY.md` | Lane-by-lane implementation status; shipped/branch-only distinction; explicit "still stop short" language for team/cron gap |
| `references/claw-code/rust/crates/runtime/src/task_registry.rs` | Landed task lifecycle (335 LOC, merged Lane 4) |
| `references/claw-code/rust/crates/runtime/src/team_cron_registry.rs` | Landed team/cron lifecycle (363 LOC, merged Lane 6); explicit `record_run` bookkeeping without automatic firing |
| `references/claw-code/rust/crates/runtime/src/worker_boot.rs` | Landed `WorkerStatus` six-state enum; file-based observability (`emit_state_file`) |
| `references/claw-code/rust/crates/runtime/src/lane_events.rs` | Landed typed `LaneEvent` enum |
| `analysis/task-registry-lifecycle.md` | Documents the landed task registry scope |
| `analysis/team-cron-registry-boundary.md` | Documents the registry vs. scheduler boundary with explicit "no tick loop" language |
| `analysis/worker-boot-state-machine.md` | Documents the landed `WorkerStatus` states vs. roadmap expansion intent |
