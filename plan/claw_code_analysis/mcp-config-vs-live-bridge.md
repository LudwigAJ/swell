# MCP Config Inspection vs. Live Runtime Bridge

The ClawCode MCP integration surfaces two fundamentally different operational layers that builders must understand to avoid conflating configuration reporting with live runtime behavior. This document explains how `/mcp` command inspection and the live `McpToolRegistry` bridge differ in ownership, capability, and purpose.

## Two Distinct Surfaces

### Config Inspection — `/mcp` Command

The `/mcp` command (also available as `claw mcp` CLI) is a **configuration reporting surface** implemented in `crates/commands/src/lib.rs` (`handle_mcp_slash_command`, `render_mcp_report_for`, `parse_mcp_command`). It reads from the merged config loaded by `ConfigLoader` — sourced from `.claw/settings.json` (project) and `.claw/settings.local.json` (machine-local override) — and renders a human-readable or JSON report.

**Supported command forms** (from `parse_mcp_command`):
- `/mcp` or `/mcp list` — summary of all configured servers with transport type, scope, and command/URL
- `/mcp show <server>` — detailed config for a named server (command, args, env keys, timeout, OAuth)
- `/mcp help` — usage text

The scope labels (`project`, `user`, `local`) tell you where each server entry originates in the config precedence chain. The report does **not** reflect live connection state — a server listed by `/mcp show` may be disconnected or errored at runtime.

### Live Runtime Bridge — `McpToolRegistry`

The live MCP bridge lives in `crates/runtime/src/mcp_tool_bridge.rs` and is owned by the runtime. It maintains a **stateful per-server registry** (`McpToolRegistry`) that tracks actual connection status, advertised tools, and available resources for each MCP server that has been initialized at runtime.

**Core struct: `McpServerState`** (from `mcp_tool_bridge.rs`):
```rust
pub struct McpServerState {
    pub server_name: String,
    pub status: McpConnectionStatus,
    pub tools: Vec<McpToolInfo>,        // name, description, input_schema
    pub resources: Vec<McpResourceInfo>, // uri, name, description, mime_type
    pub server_info: Option<String>,
    pub error_message: Option<String>,
}
```

**Connection status enum: `McpConnectionStatus`**:
```rust
pub enum McpConnectionStatus {
    Disconnected,
    Connecting,
    Connected,
    AuthRequired,
    Error,
}
```

## Operations Each Surface Supports

| Operation | Config Inspection (`/mcp`) | Live Bridge (`McpToolRegistry`) |
|-----------|---------------------------|-------------------------------|
| List configured servers | ✅ | ✅ via `list_servers()` |
| Show config for named server | ✅ | — |
| List tools on a server | — | ✅ via `list_tools(server)` (Connected only) |
| List resources on a server | — | ✅ via `list_resources(server)` (Connected only) |
| Read a specific resource | — | ✅ via `read_resource(server, uri)` (Connected only) |
| Call a tool on a server | — | ✅ via `call_tool(server, tool, args)` (Connected only) |
| Set auth status | — | ✅ via `set_auth_status(server, status)` |
| Disconnect a server | — | ✅ via `disconnect(server)` |
| Reflect live connection state | — | ✅ — status drives gating |

## State Gating in the Live Bridge

A key design difference is that the live bridge **enforces connection state gating** on all server-scoped operations. Operations like `list_tools`, `list_resources`, `read_resource`, and `call_tool` all return errors when the server is not in `Connected` state:

```rust
pub fn list_tools(&self, server_name: &str) -> Result<Vec<McpToolInfo>, String> {
    let inner = self.inner.lock().expect("mcp registry lock poisoned");
    match inner.get(server_name) {
        Some(state) => {
            if state.status != McpConnectionStatus::Connected {
                return Err(format!(
                    "server '{}' is not connected (status: {})",
                    server_name, state.status
                ));
            }
            Ok(state.tools.clone())
        }
        None => Err(format!("server '{}' not found", server_name)),
    }
}
```

This means you cannot call a tool on a server that is `AuthRequired` or `Error` — the registry gates the operation and returns a descriptive error. The config inspection layer has no concept of this gating because it never attempts a live connection.

## Tool Naming — `mcp__<server>__<tool>`

When the runtime registers MCP tools for use in a turn, it uses a qualified naming scheme defined in `crates/runtime/src/mcp.rs`:

```rust
pub fn mcp_tool_prefix(server_name: &str) -> String {
    format!("mcp__{}__", normalize_name_for_mcp(server_name))
}

pub fn mcp_tool_name(server_name: &str, tool_name: &str) -> String {
    format!(
        "{}{}",
        mcp_tool_prefix(server_name),
        normalize_name_for_mcp(tool_name)
    )
}
```

`normalize_name_for_mcp` replaces characters outside `[a-zA-Z0-9_-]` with underscores and collapses runs of underscores. This means a server named `"github.com"` becomes `"mcp__github_com__"` and a tool `"weather tool!"` becomes `"weather_tool_"`.

The slash command surface (`/mcp`) does not expose these qualified tool names — it only reports server-level configuration. The qualified names are used at the **tool dispatch layer** inside `McpToolRegistry::spawn_tool_call`, where a qualified name is passed to `McpServerManager::call_tool`.

## Server vs. Tool Scoping in `call_tool`

`McpToolRegistry::call_tool` takes `server_name`, `tool_name`, and `arguments` separately, thenqualifies the tool name internally via `mcp_tool_name(server_name, tool_name)` before dispatching to `McpServerManager`. This separation means the caller does not need to know the internal qualified form — the registry handles the mapping.

## Builder Lessons

1. **Do not use `/mcp` output to reason about runtime state.** A server can be configured but disconnected, connecting, or errored. The config surface has no visibility into runtime health.

2. **Use `McpToolRegistry` for live operations.** If you need to list a server's available tools, read a resource, or invoke a tool, you must go through the live bridge — and the server must be in `Connected` state.

3. **Distinguish config reporting from runtime discovery.** Config tells you what *should* be running. The registry tells you what *is* running, what it exposes, and whether you can call it.

4. **Tool name qualification is internal to the bridge.** Callers pass `(server_name, tool_name)` pairs. The bridge handles normalization and qualification, so you do not need to pre-encode the `mcp__server__tool` form yourself.

5. **Connection state is a first-class concept.** `McpConnectionStatus` is not an implementation detail — it directly controls which operations are permitted. AuthRequired and Error are real states that block tool and resource access until the condition is resolved.

## Key Files

- `crates/commands/src/lib.rs` — `/mcp` slash command parsing and config-rendering (`parse_mcp_command`, `render_mcp_report_for`)
- `crates/runtime/src/mcp_tool_bridge.rs` — `McpToolRegistry`, `McpServerState`, `McpConnectionStatus`, state-gated operations
- `crates/runtime/src/mcp.rs` — tool name qualification (`mcp_tool_name`, `normalize_name_for_mcp`)
- `crates/runtime/src/mcp_stdio.rs` — `McpServerManager`, stdio process lifecycle, JSON-RPC dispatch
- `crates/runtime/src/mcp_server.rs` — `McpServer` trait and lifecycle entry points
