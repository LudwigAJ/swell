# MCP Degraded Startup Semantics

MCP startup in ClawCode is not a binary success-or-failure event. The system is designed to treat partial or degraded startup as a **first-class operational state** — when some MCP servers fail to initialize, the working servers remain fully usable and the failure is reported structurally rather than causing a hard abort. This document explains how that works, what the failure classification taxonomy looks like, and the difference between startup intent (roadmap) and landed depth (what the code actually does).

## The Core Principle: Partial Success Is First-Class

The ROADMAP articulates this design principle directly:

> "Partial success is first-class — e.g. MCP startup can succeed for some servers and fail for others, with structured degraded-mode reporting."

This is not a future aspiration. It is **landed behavior** implemented in `crates/runtime/src/mcp_stdio.rs` via `McpServerManager::discover_tools_best_effort` and the `McpToolDiscoveryReport` struct. When this method runs, it iterates over all configured MCP servers and continues discovery even when individual servers fail, accumulating results into a structured report that separates working servers, failed servers, and unsupported servers.

## `McpToolDiscoveryReport` — The Structured Startup Result

The discovery report (from `mcp_stdio.rs`) is the primary artifact of MCP startup:

```rust
pub struct McpToolDiscoveryReport {
    pub tools: Vec<ManagedMcpTool>,              // successfully discovered tools
    pub failed_servers: Vec<McpDiscoveryFailure>, // per-server failure details
    pub unsupported_servers: Vec<UnsupportedMcpServer>, // transport not supported
    pub degraded_startup: Option<McpDegradedReport>, // populated when mixed
}
```

The presence of a `degraded_startup` field is the key signal: it is `Some` **only when** at least one server succeeded and at least one server failed. If all servers succeed, it is `None`. If all servers fail, it is also `None` — there is no degraded state when nothing is working (that is a hard failure, not a degraded one).

## Failed-Server Classification

Every server that fails during discovery produces a `McpDiscoveryFailure` struct with rich classification metadata:

```rust
pub struct McpDiscoveryFailure {
    pub server_name: String,
    pub phase: McpLifecyclePhase,        // which lifecycle phase failed
    pub error: String,                    // human-readable error
    pub recoverable: bool,                // can a retry succeed?
    pub context: BTreeMap<String, String>, // structured error context
}
```

### Lifecycle Phase Classification

The `phase` field is a `McpLifecyclePhase` enum (from `mcp_lifecycle_hardened.rs`) that tells you **where** in the startup sequence the failure occurred:

```rust
pub enum McpLifecyclePhase {
    ConfigLoad,
    ServerRegistration,
    SpawnConnect,
    InitializeHandshake,
    ToolDiscovery,
    ResourceDiscovery,
    Ready,
    Invocation,
    ErrorSurfacing,
    Shutdown,
    Cleanup,
}
```

The relevant failure phases for degraded startup are:
- **`ConfigLoad`** — server config is malformed or unreadable
- **`ServerRegistration`** — server was recognized but could not be registered (e.g., unsupported transport)
- **`SpawnConnect`** — process spawn or initial connection failed
- **`InitializeHandshake`** — the JSON-RPC `initialize` call failed or timed out
- **`ToolDiscovery`** — `tools/list` returned an error
- **`ResourceDiscovery`** — `resources/list` returned an error

The phase drives two downstream decisions:
1. **Whether the error is recoverable.** From `McpServerManagerError::recoverable()` in `mcp_stdio.rs`:

   ```rust
   fn recoverable(&self) -> bool {
       !matches!(
           self.lifecycle_phase(),
           McpLifecyclePhase::InitializeHandshake
       ) && matches!(self, Self::Transport { .. } | Self::Timeout { .. })
   }
   ```

   `InitializeHandshake` failures are **never recoverable** — if the server rejects the handshake, retrying will not help. Transport and timeout errors during other phases may be recoverable with a retry.

2. **What the failed-server entry looks like in the degraded report.** `discover_tools_best_effort` converts each `McpDiscoveryFailure` into a `McpFailedServer` for the degraded report:

   ```rust
   let degraded_failed_servers = failed_servers
       .iter()
       .map(|failure| McpFailedServer {
           server_name: failure.server_name.clone(),
           phase: failure.phase,
           error: McpErrorSurface::new(
               failure.phase,
               Some(failure.server_name.clone()),
               failure.error.clone(),
               failure.context.clone(),
               failure.recoverable,
           ),
       })
       .chain(
           self.unsupported_servers
               .iter()
               .map(unsupported_server_failed_server),
       )
       .collect::<Vec<_>>();
   ```

### Unsupported Servers vs. Failed Servers

`UnsupportedMcpServer` is a distinct category. A server is **unsupported** when its transport type is not `Stdio` — the `McpServerManager` only handles stdio servers. Non-stdio servers (HTTP, SDK, WebSocket) are classified as unsupported at `ServerRegistration` phase and reported separately:

```rust
pub struct UnsupportedMcpServer {
    pub server_name: String,
    pub transport: McpTransport,
    pub reason: String,
}
```

These are included in the degraded report as failed servers with `McpLifecyclePhase::ServerRegistration` so the caller knows the full picture of what was attempted versus what was available.

## `McpDegradedReport` — Mixed Success/Failure State

When `discover_tools_best_effort` detects mixed outcomes, it produces an `McpDegradedReport` (from `mcp_lifecycle_hardened.rs`):

```rust
pub struct McpDegradedReport {
    pub working_servers: Vec<String>,
    pub failed_servers: Vec<McpFailedServer>,
    pub available_tools: Vec<String>,
    pub missing_tools: Vec<String>,
}
```

The report computes `missing_tools` by comparing the tools that were **expected** (from the degraded startup path, passed as `expected_tools`) against the tools that were **actually discovered**. This gives the caller a precise list of tools that are unavailable due to server failures.

The `failed_servers` list in the degraded report includes both servers that produced `McpDiscoveryFailure` during discovery **and** servers that were classified as `UnsupportedMcpServer`. Both represent servers that are not contributing tools to the runtime.

## `discover_tools_best_effort` — The Best-Effort Discovery Algorithm

This method (from `mcp_stdio.rs`) is the mechanism behind degraded startup:

```rust
pub async fn discover_tools_best_effort(&mut self) -> McpToolDiscoveryReport {
    let server_names = self.server_names();
    let mut discovered_tools = Vec::new();
    let mut working_servers = Vec::new();
    let mut failed_servers = Vec::new();

    for server_name in server_names {
        match self.discover_tools_for_server(&server_name).await {
            Ok(server_tools) => {
                working_servers.push(server_name.clone());
                // ... register tools
            }
            Err(error) => {
                self.clear_routes_for_server(&server_name);
                failed_servers.push(error.discovery_failure(&server_name));
            }
        }
    }

    // ... build degraded report if mixed
    McpToolDiscoveryReport { tools, failed_servers, unsupported_servers, degraded_startup }
}
```

Key behaviors:
1. **Continues iterating** even after a server fails — one server's failure does not abort discovery of other servers.
2. **Clears tool routes** for failed servers so stale routing entries do not persist.
3. **Tracks working vs. failed servers separately** to determine whether degraded state applies.
4. **Only emits `degraded_startup`** when both `!working_servers.is_empty()` and `!degraded_failed_servers.is_empty()` — i.e., partial success.

## Working Servers Remain Usable

A critical design property: **tools from working servers remain fully callable** even when other servers fail. The degraded report is informational — it surfaces the gap — but does not disable working servers. The `tools` field in `McpToolDiscoveryReport` contains only tools from successfully initialized servers, and the routing layer (`tool_index` in `McpServerManager`) only registers routes for those tools.

This is verified by the test `manager_discovery_report_keeps_healthy_servers_when_one_server_fails` in `mcp_stdio.rs`, which asserts that after a partial failure, a tool on a healthy server can still be called successfully.

## The `discover_tools_for_server` Retry Loop

Each individual server's discovery goes through a retry loop with one automatic recovery attempt:

```rust
async fn discover_tools_for_server(&mut self, server_name: &str) -> Result<Vec<ManagedMcpTool>, McpServerManagerError> {
    let mut attempts = 0;
    loop {
        match self.discover_tools_for_server_once(server_name).await {
            Ok(tools) => return Ok(tools),
            Err(error) if attempts == 0 && Self::is_retryable_error(&error) => {
                self.reset_server(server_name).await?;
                attempts += 1;
            }
            Err(error) => {
                if Self::should_reset_server(&error) {
                    self.reset_server(server_name).await?;
                }
                return Err(error);
            }
        }
    }
}
```

`is_retryable_error` returns true only for `Transport` and `Timeout` errors. If a retryable error occurs on the first attempt, the server is reset and one retry is attempted before surfacing the failure. This is the "one automatic recovery attempt before escalation" pattern applied at the per-server discovery level.

## Startup Intent vs. Landed Depth

The ROADMAP discusses MCP startup depth in terms of roadmap items and acceptance criteria. Some items are **landed** and some are **in-flight or aspirational**. The distinction matters for builders who want to rely on specific behavior:

| Aspect | Roadmap Intent | Landed Implementation |
|--------|---------------|----------------------|
| Degraded startup reporting | "partial-startup and per-server failures are reported structurally" | `discover_tools_best_effort` + `McpToolDiscoveryReport.degraded_startup` |
| Failed server classification | "failed server classification (startup/handshake/config/partial)" | `McpLifecyclePhase` enum covers these phases |
| Structured `failed_servers` in tool output | "structured `failed_servers` + `recovery_recommendations` in tool output" | `McpDegradedReport` with `working_servers`, `failed_servers`, `available_tools`, `missing_tools` |
| Recovery recommendations | "recovery_recommendations" | Not yet present in the landed `McpDegradedReport` struct — `missing_tools` is computed but no recovery hint text is emitted |
| Per-server health state machine | "MCP manager discovery flaky test... degraded-startup coverage" | Landed in `mcp_stdio.rs` tests |
| Plugin/MCP lifecycle contract | "first-class plugin/MCP lifecycle contract" with degraded-mode behavior | Partially landed: `PluginLifecycle` in `plugin_lifecycle.rs` has `Degraded` state, but the per-server granularity is in `mcp_stdio.rs` |

The acceptance criterion mentioning "recovery_recommendations" in tool output is **not yet implemented** in the current code. The `McpDegradedReport` struct has fields for `working_servers`, `failed_servers`, `available_tools`, and `missing_tools`, but does not yet carry per-server recovery recommendation strings. This is a gap between roadmap intent and landed depth that builders should be aware of.

## Relationship to Plugin Lifecycle Degraded State

The plugin lifecycle system (`plugin_lifecycle.rs`) has a parallel degraded state concept with its own `PluginState::Degraded` variant:

```rust
PluginState::Degraded {
    healthy_servers: Vec<String>,
    failed_servers: Vec<ServerHealth>,
}
```

This is structurally similar to `McpDegradedReport` but operates at a different layer — the plugin lifecycle coordinates multiple MCP servers and/or other plugin capabilities, while `McpServerManager::discover_tools_best_effort` operates specifically within the MCP stdio server subset. The plugin lifecycle degraded state surfaces through the `degraded_mode()` method on `PluginHealthcheck`:

```rust
pub fn degraded_mode(&self, discovery: &DiscoveryResult) -> Option<DegradedMode> {
    match &self.state {
        PluginState::Degraded { healthy_servers, failed_servers } => Some(DegradedMode {
            available_tools: discovery.tools.iter().map(|t| t.name.clone()).collect(),
            unavailable_tools: failed_servers.iter().flat_map(|s| s.capabilities.iter().cloned()).collect(),
            reason: format!("{} servers healthy, {} servers failed", healthy_servers.len(), failed_servers.len()),
        }),
        _ => None,
    }
}
```

Both systems describe the same underlying reality (some servers work, some do not) but at different granularity and for different audiences.

## Builder Lessons

1. **Treat degraded startup as expected behavior in multi-server MCP configurations.** When multiple MCP servers are configured, partial failure is a realistic scenario (one server's process is down, one has a config drift, one is fine). The system is designed for this — do not assume all-or-nothing.

2. **Check `degraded_startup` on `McpToolDiscoveryReport`, not just `tools.is_empty()`.** A non-empty `tools` field does not mean all servers are healthy. Always inspect `degraded_startup` to determine whether the startup was clean or mixed.

3. **`McpLifecyclePhase` on `McpDiscoveryFailure` tells you the recovery path.** A `SpawnConnect` failure with `recoverable: true` means the server process might start on retry. An `InitializeHandshake` failure with `recoverable: false` means the server is rejecting the protocol — retries will not help until the server configuration is fixed.

4. **`InitializeHandshake` failures are non-recoverable by design.** The MCP specification requires that if a server rejects the initialization parameters, the session is invalid. ClawCode correctly does not retry these — retrying would produce the same rejection.

5. **Working servers are not affected by other servers' failures.** The tool routing in `McpServerManager` is per-server. A failure on server B does not invalidate tools from server A that were already discovered and routed.

6. **`missing_tools` gives you an actionable gap list.** When a server fails, you can use the `missing_tools` field in `McpDegradedReport` to determine exactly which tools are unavailable and why — rather than having to cross-reference failed server names against expected tool lists manually.

7. **The roadmap recovery_recommendations field is not yet landed.** If you are building tooling that consumes degraded reports and want recovery hints, you will need to derive them from `McpFailedServer.phase` and `McpFailedServer.error` yourself — the struct does not yet carry pre-computed recommendation strings.

## Key Files

- `crates/runtime/src/mcp_stdio.rs` — `McpServerManager::discover_tools_best_effort`, `McpDiscoveryFailure`, `McpToolDiscoveryReport`, `discover_tools_for_server` retry loop
- `crates/runtime/src/mcp_lifecycle_hardened.rs` — `McpLifecyclePhase`, `McpFailedServer`, `McpDegradedReport`, `McpErrorSurface`, `McpPhaseResult`, lifecycle validation
- `crates/runtime/src/plugin_lifecycle.rs` — `PluginState::Degraded`, `ServerHealth`, `degraded_mode()` method
- `crates/runtime/src/lib.rs` — public re-exports of degraded startup types
- `ROADMAP.md` — "Partial success is first-class" principle, MCP degraded-startup acceptance criteria (Item 10, Item 13)
