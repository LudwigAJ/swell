# Lane Events Schema

## Overview

Lane events are the machine-readable coordination signal surface for ClawCode coding workers. Rather than inferring lane state from log parsing or tmux pane inspection, downstream tooling (clawhip, orchestrators, dashboards) consume typed `LaneEvent` payloads that describe every meaningful state transition in a lane's lifecycle—from spawn through commit creation, PR opening, merge readiness, and eventual closeout.

The canonical implementation lives in `references/claw-code/rust/crates/runtime/src/lane_events.rs`. The schema was designed as part of the Phase 2 roadmap item "Canonical lane event schema" in `references/claw-code/ROADMAP.md`.

## Event Name Taxonomy

`LaneEventName` is a Rust enum with `#[serde(rename)]` annotations that define the wire-format strings. Every event name is namespaced with `lane.` or `branch.` to make filtering unambiguous in log streams.

| Wire value | Enum variant | Lifecycle phase |
|---|---|---|
| `"lane.started"` | `Started` | Spawn |
| `"lane.ready"` | `Ready` | Worker boot |
| `"lane.prompt_misdelivery"` | `PromptMisdelivery` | Trust/prompt |
| `"lane.blocked"` | `Blocked` | Blocker detected |
| `"lane.red"` | `Red` | Test/compile failure |
| `"lane.green"` | `Green` | Verification passed |
| `"lane.commit.created"` | `CommitCreated` | Commit landed |
| `"lane.pr.opened"` | `PrOpened` | PR opened |
| `"lane.merge.ready"` | `MergeReady` | Merge gate satisfied |
| `"lane.finished"` | `Finished` | Clean closeout |
| `"lane.failed"` | `Failed` | Irrecoverable failure |
| `"lane.reconciled"` | `Reconciled` | Branch in sync |
| `"lane.merged"` | `Merged` | Merge completed |
| `"lane.superseded"` | `Superseded` | Replaced by later commit |
| `"lane.closed"` | `Closed` | Lane retired |
| `"branch.stale_against_main"` | `BranchStaleAgainstMain` | Freshness check |
| `"branch.workspace_mismatch"` | `BranchWorkspaceMismatch` | Session/workspace mismatch |

The Rust source (`lane_events.rs`, lines 8–30) derives `Serialize` and `Deserialize` directly, so each variant round-trips through JSON without any wrapper transformation.

## Status Enum

`LaneEventStatus` carries the runtime health signal. It is orthogonal to the event name—a `lane.blocked` carries `status = Blocked`, but a `lane.commit.created` also carries `status = Completed`.

| Wire value | Meaning |
|---|---|
| `"running"` | Active, not yet resolved |
| `"ready"` | Worker boot complete |
| `"blocked"` | Blocker detected, waiting |
| `"red"` | Tests or compile failing |
| `"green"` | Verification passed |
| `"completed"` | Normal closeout |
| `"failed"` | Irrecoverable |
| `"reconciled"` | Branch sync achieved |
| `"merged"` | Merge completed |
| `"superseded"` | Replaced |
| `"closed"` | Retired |

## Failure Taxonomy

`LaneFailureClass` classifies blockers by root cause so retry policies and dashboards can branch on the failure type without parsing freeform error strings.

| Wire value | Trigger |
|---|---|---|
| `"prompt_delivery"` | Prompt landed in wrong shell |
| `"trust_gate"` | Trust prompt not resolved |
| `"branch_divergence"` | Branch is stale vs main |
| `"compile"` | Build failed |
| `"test"` | Tests failed |
| `"plugin_startup"` | Plugin failed to initialize |
| `"mcp_startup"` | MCP server failed startup |
| `"mcp_handshake"` | MCP handshake rejected |
| `"gateway_routing"` | Provider/gateway routing failure |
| `"tool_runtime"` | Tool executor error |
| `"workspace_mismatch"` | Session bound to different workspace |
| `"infra"` | Infrastructure-level failure |

This taxonomy is defined at `lane_events.rs` lines 39–54 and is exercised in tests at `canonical_lane_event_names_serialize_to_expected_wire_values` and `failure_classes_cover_canonical_taxonomy_wire_values`.

## Event Payload Structure

`LaneEvent` is the top-level payload:

```rust
pub struct LaneEvent {
    pub event: LaneEventName,           // which event
    pub status: LaneEventStatus,       // current health
    pub emitted_at: String,            // ISO 8601 timestamp
    pub failure_class: Option<LaneFailureClass>,  // if blocked/failed
    pub detail: Option<String>,        // human-readable note
    pub data: Option<Value>,           // structured extension payload
}
```

The `data` field carries event-specific structured metadata. For `lane.commit.created` it holds a `LaneCommitProvenance`:

```rust
pub struct LaneCommitProvenance {
    pub commit: String,
    pub branch: String,
    pub worktree: Option<String>,
    pub canonical_commit: Option<String>,
    pub superseded_by: Option<String>,
    pub lineage: Vec<String>,
}
```

`canonical_commit` enables deduplication: when multiple worktrees or lane retries produce commits on the same branch, only the latest canonical commit is retained. `dedupe_superseded_commit_events()` at `lane_events.rs` lines 110–137 applies this collapsing before any agent manifest or clawhip summary is written.

## Builder-Factory Constructors

`LaneEvent` is not constructed by callers directly setting fields. Instead, a set of named builder methods encode the intended semantics:

- `LaneEvent::started(emitted_at)` → `lane.started` with `status = Running`
- `LaneEvent::finished(emitted_at, detail)` → `lane.finished` with `status = Completed`
- `LaneEvent::commit_created(emitted_at, detail, provenance)` → `lane.commit.created` with `status = Completed` plus provenance in `data`
- `LaneEvent::blocked(emitted_at, blocker)` → `lane.blocked` with `failure_class` and `detail` from the blocker
- `LaneEvent::failed(emitted_at, blocker)` → `lane.failed` with `failure_class` and `detail`
- `LaneEvent::superseded(emitted_at, detail, provenance)` → `lane.superseded` with provenance in `data`

These constructors make it impossible to emit a `lane.commit.created` without provenance data, and enforce that `blocked`/`failed` events always carry a `LaneEventBlocker`:

```rust
pub struct LaneEventBlocker {
    pub failure_class: LaneFailureClass,
    pub detail: String,
}
```

## Typed Event Example

A `branch.workspace_mismatch` event serialized to JSON:

```json
{
  "event": "branch.workspace_mismatch",
  "status": "blocked",
  "emittedAt": "2026-04-04T00:00:02Z",
  "failureClass": "workspace_mismatch",
  "detail": "session belongs to /tmp/repo-a but current workspace is /tmp/repo-b",
  "data": {
    "expectedWorkspaceRoot": "/tmp/repo-a",
    "actualWorkspaceRoot": "/tmp/repo-b",
    "sessionId": "sess-123"
  }
}
```

This is the exact shape tested in `workspace_mismatch_failure_class_round_trips_in_branch_event_payloads` (`lane_events.rs` lines 187–213). Note that the wire key is `"failureClass"` (camelCase per JSON convention) while the Rust struct field is `failure_class` (snake_case)—serde handles the rename transparently.

## Schema Design Lessons

**Events over scraped prose.** The primary lesson from this schema is that log-shaped event descriptions ("Worker is now blocked because the prompt was delivered to the shell instead") require downstream parsing. Typed events with structured `data` payloads allow clawhip to render Discord summaries and lane boards without string inference. Adding a new event variant costs one enum entry and one builder method; no downstream scraper needs updating.

**Status orthogonal to event name.** Separating `status: LaneEventStatus` from `event: LaneEventName` means a dashboard can filter all "currently blocked" lanes regardless of which specific event caused the block, and a retry policy can route on `failure_class` without caring whether the lane ended with `Failed` or `Blocked`.

**Deduplication by canonical commit.** When parallel lane workers or retries produce multiple commits on the same branch, `dedupe_superseded_commit_events()` collapses superseded entries to the latest canonical lineage. This keeps agent manifests and clawhip summaries compact and prevents stale commit events from confusing merge readiness calculations.

**Blocker typed, not strings.** `LaneFailureClass` is an enum, not a string enum pattern. Callers match on the variant; dashboards render human-readable labels. Adding a new failure class requires adding one enum variant and updating the places that handle it—which is the visible cost that prevents failure-mode proliferation.

**Provenance out-of-band.** `LaneCommitProvenance` is stored in the optional `data` field rather than flattening into the top-level event, so lanes that don't need commit metadata (e.g., `lane.blocked` or `lane.red`) remain lean. The provenance schema is versioned independently of the event schema.

## Evidence

- `references/claw-code/rust/crates/runtime/src/lane_events.rs` — full schema definition, builder methods, and tests
- `references/claw-code/ROADMAP.md` Phase 2, item 4 "Canonical lane event schema" — design intent and event list
- `references/claw-code/ROADMAP.md` Phase 2, item 5 "Failure taxonomy" — failure class rationale
