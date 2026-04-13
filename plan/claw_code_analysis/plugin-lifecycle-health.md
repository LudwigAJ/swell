# Plugin Lifecycle Health

Plugin lifecycle in ClawCode is the system that manages how external plugin servers (MCP-compatible) are loaded, health-checked, and torn down. The lifecycle is not a simple binary install/uninstall toggle — it models a **state machine** where plugins move through transitional phases and can end up in one of several health outcomes depending on whether their backing servers are healthy, partially degraded, or completely failed.

This document explains how plugin lifecycle phases relate to health states, what transitional states mean, and what builders can learn from the design.

## Primary Evidence

- `references/claw-code/rust/crates/runtime/src/plugin_lifecycle.rs` — the canonical lifecycle implementation

## PluginState: Transitional Lifecycle Phases

The plugin lifecycle is encoded in the `PluginState` enum. This is not a flat healthy/unhealthy flag — it is a **phased state machine** with distinct transitional states between startup and shutdown.

```rust
pub enum PluginState {
    Unconfigured,    // plugin discovered but not yet validated
    Validated,       // configuration validated, not yet started
    Starting,       // startup in progress
    Healthy,        // all servers operational
    Degraded {      // partial failure: some servers up, some failed
        healthy_servers: Vec<String>,
        failed_servers: Vec<ServerHealth>,
    },
    Failed {        // total failure: no healthy servers remain
        reason: String,
    },
    ShuttingDown,  // shutdown in progress
    Stopped,        // fully stopped
}
```

### Transitional Phases

Each transitional state has a specific meaning:

| Phase | Meaning |
|---|---|
| `Unconfigured` | Plugin is known but no configuration has been validated yet |
| `Validated` | Configuration passes validation; plugin is authorized to start |
| `Starting` | Runtime is attempting to connect to the plugin's backing servers |
| `ShuttingDown` | Runtime is tearing down connections in preparation for stop |
| `Stopped` | Lifecycle complete; plugin is idle and memory is clean |

The transitional phases `Starting` and `ShuttingDown` are meaningful — they tell observers that the plugin is **in flight**, not just done or not-done.

### Steady-State Health Outcomes

Once startup completes, the plugin settles into one of three health outcomes:

- **`Healthy`** — all backing servers are operational; no degraded or failed servers
- **`Degraded`** — some servers failed, but at least one server remains healthy; plugin continues with reduced tool surface
- **`Failed`** — all servers failed; the plugin cannot serve any tools

The distinction between `Degraded` and `Failed` matters: a `Degraded` plugin is still partially usable, and the runtime exposes which tools are available versus unavailable through `DegradedMode`.

## Server-Level Health: ServerStatus and ServerHealth

Plugin health is derived from **per-server health tracking**. Each backing MCP server has its own `ServerStatus`:

```rust
pub enum ServerStatus {
    Healthy,
    Degraded,
    Failed,
}
```

`ServerHealth` records the current status of each server along with its exposed capabilities (tools/resources) and any last-error message:

```rust
pub struct ServerHealth {
    pub server_name: String,
    pub status: ServerStatus,
    pub capabilities: Vec<String>,  // tools + resources this server advertises
    pub last_error: Option<String>,
}
```

The plugin-level `PluginState::Degraded` variant aggregates individual `ServerHealth` records — it carries the list of healthy servers and the list of failed servers, so the runtime can reason about which tools remain available.

## Health Derivation: `PluginState::from_servers`

The `PluginState` is computed from the list of `ServerHealth` records via `PluginState::from_servers()`. The derivation logic in `plugin_lifecycle.rs` is:

1. If no servers are available at all → `Failed { reason: "no servers available" }`
2. If all servers are healthy → `Healthy`
3. If some servers failed but at least one remains healthy → `Degraded { healthy_servers, failed_servers }`
4. If all servers failed → `Failed { reason: "all N servers failed" }`
5. If any server is `Degraded` (not `Failed`) but no servers are `Failed` outright → `Healthy` (Degraded servers are still usable)

Note point 5: a `Degraded` server (e.g., high latency or intermittent errors) does **not** itself trigger `PluginState::Degraded`. Only `Failed` servers count toward the degraded classification. This means a plugin whose servers are all `Degraded` or all `Healthy` is itself `Healthy` — the `Degraded` server status is a warning, not a failure.

## DegradedMode: Partial Functionality Under Degraded Health

When a plugin is in `PluginState::Degraded`, the runtime exposes a `DegradedMode` struct that tells callers exactly which tools are available and which are not:

```rust
pub struct DegradedMode {
    pub available_tools: Vec<String>,
    pub unavailable_tools: Vec<String>,
    pub reason: String,  // e.g., "2 servers healthy, 1 servers failed"
}
```

The `PluginHealthcheck::degraded_mode()` method derives this from the current `PluginState::Degraded` and a `DiscoveryResult`:

```rust
pub fn degraded_mode(&self, discovery: &DiscoveryResult) -> Option<DegradedMode>
```

This allows the runtime to continue operating with a reduced tool surface rather than failing the entire plugin when one server goes down.

## PluginHealthcheck: Runtime Health Probe Result

The `PluginHealthcheck` struct is the runtime-facing health report for a plugin:

```rust
pub struct PluginHealthcheck {
    pub plugin_name: String,
    pub state: PluginState,
    pub servers: Vec<ServerHealth>,
    pub last_check: u64,  // Unix timestamp of last check
}
```

It is constructed by calling `PluginHealthcheck::new(plugin_name, servers)` which derives the `state` from the server list via `PluginState::from_servers()` and records the current Unix timestamp.

## PluginLifecycleEvent: Lifecycle Event Marker

Transitional events are surfaced via the `PluginLifecycleEvent` enum:

```rust
pub enum PluginLifecycleEvent {
    ConfigValidated,
    StartupHealthy,
    StartupDegraded,
    StartupFailed,
    Shutdown,
}
```

These events track progress through the lifecycle — particularly the startup outcomes. A plugin can start as `StartupHealthy` (fully operational), `StartupDegraded` (partial failure), or `StartupFailed` (total failure). These events let the runtime or observers react to the outcome rather than just the final state.

## The PluginLifecycle Trait

The `PluginLifecycle` trait defines the interface every plugin lifecycle implementation must support:

```rust
pub trait PluginLifecycle {
    fn validate_config(&self, config: &RuntimePluginConfig) -> Result<(), String>;
    fn healthcheck(&self) -> PluginHealthcheck;
    fn discover(&self) -> DiscoveryResult;
    fn shutdown(&mut self) -> Result<(), String>;
}
```

- `validate_config` — checks whether the plugin's configuration is valid before startup
- `healthcheck` — returns the current `PluginHealthcheck` with `PluginState` and `ServerHealth` list
- `discover` — returns the tools and resources currently exposed by the plugin
- `shutdown` — cleanly tears down the plugin's connections and transitions to `Stopped`

## Lifecycle Phase vs. Health Outcome: The Key Distinction

The most important distinction in this system is between **lifecycle phases** and **health outcomes**:

**Lifecycle phases** describe progress through startup and shutdown sequences. They are transitional — a plugin is `Starting` only briefly before it settles into a health outcome. Phases: `Unconfigured → Validated → Starting → [health outcome] → ShuttingDown → Stopped`.

**Health outcomes** describe the steady-state result of startup. Once a plugin has completed startup, it lands in one of `Healthy`, `Degraded`, or `Failed` — and it remains there until shutdown begins. The `Degraded` outcome specifically means partial failure with continued partial operation.

The test `degraded_startup_when_one_of_three_servers_fails` in `plugin_lifecycle.rs` demonstrates this distinction: a plugin with 2 healthy and 1 failed server lands in `PluginState::Degraded`, not `Failed`, because `healthy_servers` is non-empty.

## Builder Lessons

1. **Health is not a boolean.** A plugin is not just "working" or "broken" — it can be partially working. Modeling `Degraded` as a first-class state lets the runtime continue operating with a reduced tool surface instead of hard-cutting on any failure.

2. **Per-server health tracking enables granular failure reasoning.** By tracking `ServerHealth` per backing server, the system can distinguish between "one server of three failed" and "all servers failed." This feeds the `DegradedMode` exposure of available vs. unavailable tools.

3. **Health derivation is computed, not stored.** The `PluginState::from_servers()` method computes the plugin-level state from the current `ServerHealth` list rather than storing it separately. This avoids stale health state if a server recovers between checks.

4. **Degraded server status does not trigger Degraded plugin state.** Only `Failed` servers affect the plugin-level degraded classification. A `Degraded` server (slow, warning-level) keeps the plugin `Healthy`. This prevents over-sensitivity where a noisy-but-functional server triggers a degraded classification.

5. **Lifecycle events capture transitional history.** `PluginLifecycleEvent` records not just the final state but the transitions (`StartupHealthy`, `StartupDegraded`, `StartupFailed`, `Shutdown`). This gives observers the full lifecycle arc, not just the snapshot.

6. **Shutdown is a first-class phase, not a side-effect.** By modeling `ShuttingDown` and `Stopped` explicitly, the runtime can cleanly drain connections and ensure proper teardown rather than relying on drop semantics or implicit cleanup.
