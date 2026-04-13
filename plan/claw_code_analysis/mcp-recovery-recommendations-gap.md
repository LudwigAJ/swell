# MCP Recovery Recommendations Gap

## Context

This document is a follow-up companion to [`mcp-degraded-startup.md`](./mcp-degraded-startup.md), which covers the landed degraded-startup semantics in depth. This document specifically addresses the gap between the ROADMAP.md's stated acceptance criteria for MCP degraded startup and what the landed `McpDegradedReport` struct actually delivers.

## ROADMAP.md Intent

ROADMAP.md item 13 (Phase 5 — "First-class plugin/MCP lifecycle contract") states acceptance criteria for MCP degraded-startup reporting:

> "partial-startup and per-server failures are reported structurally"
> "structured `failed_servers` + **`recovery_recommendations`** in tool output"

The `recovery_recommendations` field is listed explicitly alongside `failed_servers` as a required output shape in the tool-level degraded startup report.

## What Landed

`McpDegradedReport` (from `crates/runtime/src/mcp_lifecycle_hardened.rs`) landed with four fields:

```rust
pub struct McpDegradedReport {
    pub working_servers: Vec<String>,
    pub failed_servers: Vec<McpFailedServer>,
    pub available_tools: Vec<String>,
    pub missing_tools: Vec<String>,
}
```

This struct captures:
- Which servers succeeded (`working_servers`)
- Which servers failed with phase classification and error surface (`failed_servers` via `McpFailedServer`)
- Which tools were discovered (`available_tools`)
- Which tools are absent due to server failures (`missing_tools`)

`missing_tools` is computed by comparing expected tool names against actually discovered tool names — giving callers an actionable gap list. However, **no pre-computed recommendation strings are emitted** for any failed server.

## The Gap: No `recovery_recommendations` Field

The ROADMAP.md acceptance criterion calls for `recovery_recommendations` as a distinct output field in the degraded startup report. This field does not exist in the current `McpDegradedReport` struct. The gap manifests in two ways:

### 1. Missing Per-Server Recommendation Strings

`McpFailedServer` carries the structural classification data — `server_name`, `phase`, `error`, and `recoverable` — but does not embed human-readable recovery guidance. A caller inspecting a failed server can derive the phase and recoverability from the struct, but must construct any recovery message manually.

The `recoverable` boolean tells callers whether a retry is worth attempting, but it does not carry instructions like "check that the server process is running" or "verify the `mcpServers` config path resolves." Those guidance strings are the intent of `recovery_recommendations`.

### 2. No Centralized Recommendations Array in `McpDegradedReport`

Even if individual `McpFailedServer` entries were extended with recommendation text, the ROADMAP language implies a top-level `recovery_recommendations` output field on the degraded report itself — a flattened list of actionable items an operator can inspect without iterating and interpreting per-server data.

The current struct does not have such a field. `failed_servers` carries the data from which recommendations could be derived, but does not emit them.

## What Builders Should Learn From This Gap

1. **`McpDegradedReport` is four fields, not five.** The struct does not include `recovery_recommendations`. Any tooling that expects this field will need to derive recommendations from `McpFailedServer.phase` and `McpFailedServer.error` manually, or file a feature request against the runtime crate.

2. **`recoverable` is the closest landed substitute.** Before `recovery_recommendations` is implemented, callers can use the `recoverable: bool` on `McpDiscoveryFailure` (and its propagated form on `McpFailedServer`) to at least distinguish retryable from non-retryable failures without needing to parse error strings.

3. **`missing_tools` is the actionable output for now.** The `missing_tools` field gives callers a precise list of which tools are unavailable due to server failures, which is the most directly useful recovery signal even without recommendation strings.

4. **Roadmap acceptance criteria are not always fully implemented when checked.** The `mcp-degraded-startup.md` document correctly notes this gap as "not yet implemented." Builders interpreting ROADMAP.md should cross-reference against actual struct definitions in `mcp_lifecycle_hardened.rs` rather than assuming the full acceptance criteria text is reflected in the current code.

5. **Phase classification drives recovery guidance.** The `McpLifecyclePhase` on each `McpFailedServer` is the primary driver for what a future `recovery_recommendations` field should say:
   - `ConfigLoad` → "Check MCP config syntax and path resolution"
   - `ServerRegistration` → "Verify transport type (only stdio is supported); check server manifest"
   - `SpawnConnect` → "Confirm server process is installed and `$PATH`-accessible; check stderr for startup errors"
   - `InitializeHandshake` → "Non-recoverable without config change; server rejected protocol parameters"
   - `ToolDiscovery` → "Server responded but `tools/list` returned an error; check server capability"
   - `ResourceDiscovery` → "Server responded but `resources/list` returned an error; check server capability"

## Relationship to `mcp-degraded-startup.md`

The parent document covers the full degraded-startup mechanism in detail and includes a comparison table ("Startup Intent vs. Landed Depth") where the missing `recovery_recommendations` field is already flagged. This document expands on that entry by:

- Explaining what the field was expected to carry
- Showing that the struct ends at four fields, not five
- Deriving what recommendation strings could look like based on phase classification
- Framing this explicitly as documented follow-up tech debt, not silent omission

This separation keeps the scope narrow: degraded-startup recovery recommendation gap only, as specified in the feature brief.

## Key Files

- `crates/runtime/src/mcp_lifecycle_hardened.rs` — `McpDegradedReport` struct definition (four fields, no `recovery_recommendations`)
- `crates/runtime/src/mcp_stdio.rs` — `McpServerManager::discover_tools_best_effort`, `McpDiscoveryFailure` with `recoverable` field
- `ROADMAP.md` — Item 13 Phase 5 acceptance criteria
- [`mcp-degraded-startup.md`](./mcp-degraded-startup.md) — parent document with the "Startup Intent vs. Landed Depth" comparison table
