# Tool Families in ClawCode

## Overview

ClawCode exposes its capabilities through a layered tool system. Built-in tools are declared in `mvp_tool_specs()` in `rust/crates/tools/src/lib.rs`, plugin tools are registered separately via `GlobalToolRegistry::with_plugin_tools()`, and runtime tools can be added via `GlobalToolRegistry::with_runtime_tools()`. This document enumerates the major tool families a builder works with when extending or reasoning about ClawCode's tool surface.

**Canonical evidence:** `references/claw-code/rust/crates/tools/src/lib.rs` (`GlobalToolRegistry`, `mvp_tool_specs()`, `execute_tool_with_enforcer()`)

---

## Tool Registry Layering

`GlobalToolRegistry` in `rust/crates/tools/src/lib.rs` owns the tool surface. It is composed of three distinct layers:

| Layer | Source | Backing |
|---|---|---|
| Built-in tools | `mvp_tool_specs()` | Hardcoded `ToolSpec` definitions |
| Plugin tools | `with_plugin_tools()` | `PluginTool` trait implementations |
| Runtime tools | `with_runtime_tools()` | `RuntimeToolDefinition` structs added at startup |

Each layer is queried in order when `definitions()` produces the tool list for the API. When `execute()` dispatches a tool call, it first checks built-ins, then iterates plugin tools. This ordering means built-in names are reserved — a plugin cannot override a built-in tool name.

```rust
// GlobalToolRegistry::definitions() chain
let builtin = mvp_tool_specs().into_iter()...;
let runtime = self.runtime_tools.iter()...;
let plugin = self.plugin_tools.iter()...;
builtin.chain(runtime).chain(plugin).collect()
```

**Evidence:** `GlobalToolRegistry::definitions()`, `GlobalToolRegistry::execute()`, `GlobalToolRegistry::with_plugin_tools()` (which rejects duplicate names against `mvp_tool_specs()`)

---

## Built-in Tool Metadata

Every built-in tool is declared as a `ToolSpec` with four fields:

```rust
pub struct ToolSpec {
    pub name: &'static str,           // Unique identifier
    pub description: &'static str,     // Human-readable description
    pub input_schema: Value,           // JSON Schema for tool input
    pub required_permission: PermissionMode, // Minimum permission level
}
```

The `required_permission` field is the per-tool permission floor. Tools like `read_file` require `PermissionMode::ReadOnly`, while `bash` and `Agent` require `PermissionMode::DangerFullAccess`.

**Evidence:** `mvp_tool_specs()` in `rust/crates/tools/src/lib.rs`

---

## Core Tool Families

ClawCode's `mvp_tool_specs()` exposes approximately 40 tool specs organized into the following families. These are visible to the model and can be allowed or restricted via `--allowedTools`.

### Shell Family
Executes code in a subprocess. Classified dynamically at runtime based on the specific command.

| Tool | Required Permission | Notes |
|---|---|---|
| `bash` | `DangerFullAccess` | Dynamic classification via `classify_bash_permission()` |
| `PowerShell` | `DangerFullAccess` | Dynamic classification via `classify_powershell_permission()` |
| `REPL` | `DangerFullAccess` | Code execution in a REPL subprocess |

### File Family
Reads and writes files on the local filesystem.

| Tool | Required Permission | Notes |
|---|---|---|
| `read_file` | `ReadOnly` | Supports offset and limit |
| `write_file` | `WorkspaceWrite` | Full-file overwrite |
| `edit_file` | `WorkspaceWrite` | Supports `replace_all` for multi-edit |
| `NotebookEdit` | `WorkspaceWrite` | Jupyter notebook cell replacement/insertion/deletion |

### Search Family
Locates files and content.

| Tool | Required Permission | Notes |
|---|---|---|
| `glob_search` | `ReadOnly` | Pattern-based file discovery |
| `grep_search` | `ReadOnly` | Regex search with context flags, multiline, type filtering |

### Web Family
Fetches remote content and performs web searches.

| Tool | Required Permission | Notes |
|---|---|---|
| `WebFetch` | `ReadOnly` | URL → readable text, with prompt-based summarization |
| `WebSearch` | `ReadOnly` | DuckDuckGo HTML scraping, domain allow/block lists |

### Agent / Subagent Family
Spawns, coordinates, and manages autonomous worker sessions.

| Tool | Required Permission | Notes |
|---|---|---|
| `Agent` | `DangerFullAccess` | Launches a sub-agent task with handoff metadata |
| `Skill` | `ReadOnly` | Loads a local skill definition from disk |
| `TaskCreate`, `TaskGet`, `TaskList`, `TaskStop`, `TaskUpdate`, `TaskOutput` | `DangerFullAccess` / `ReadOnly` | In-memory `TaskRegistry` lifecycle |
| `TaskPacket` | `DangerFullAccess` | Creates a task from a structured `TaskPacket` |
| `TeamCreate`, `TeamDelete` | `DangerFullAccess` | `TeamRegistry` for parallel task execution |
| `CronCreate`, `CronDelete`, `CronList` | `DangerFullAccess` / `ReadOnly` | `CronRegistry` for scheduled recurring tasks |
| `WorkerCreate`, `WorkerGet`, `WorkerObserve`, `WorkerResolveTrust`, `WorkerAwaitReady`, `WorkerSendPrompt`, `WorkerRestart`, `WorkerTerminate`, `WorkerObserveCompletion` | `DangerFullAccess` / `ReadOnly` | Worker boot state machine with trust gating |

**Builder note:** Task, Team, Cron, and Worker tools are backed by in-memory registries. They stop short of an external subprocess scheduler or worker fleet. See `rust/crates/runtime/src/task_registry.rs` and `rust/crates/runtime/src/team_cron_registry.rs`.

### Todo Family
Manages structured task state.

| Tool | Required Permission | Notes |
|---|---|---|
| `TodoWrite` | `WorkspaceWrite` | Reads/writes `.clawd-todos.json`, emits `verification_nudge_needed` |

### MCP (Model Context Protocol) Family
Discovers and calls tools from connected MCP servers.

| Tool | Required Permission | Notes |
|---|---|---|
| `ListMcpResources` | `ReadOnly` | Lists resources from a named MCP server |
| `ReadMcpResource` | `ReadOnly` | Reads a specific MCP resource by URI |
| `McpAuth` | `DangerFullAccess` | Triggers OAuth or credential auth with an MCP server |
| `MCP` | `DangerFullAccess` | Executes a tool on a connected MCP server |

These tools route through `global_mcp_registry()` (`McpToolRegistry`). The runtime MCP bridge tracks connection state, tools, and resources per server.

**Evidence:** `rust/crates/runtime/src/mcp_tool_bridge.rs`, `rust/crates/tools/src/lib.rs` (`run_list_mcp_resources`, `run_read_mcp_resource`, `run_mcp_auth`, `run_mcp_tool`)

### LSP (Language Server Protocol) Family
Code intelligence via LSP.

| Tool | Required Permission | Notes |
|---|---|---|
| `LSP` | `ReadOnly` | Dispatch to `symbols`, `references`, `diagnostics`, `definition`, `hover` |

Routes through `global_lsp_registry()` (`LspRegistry`).

**Evidence:** `rust/crates/runtime/src/lsp_client.rs`

### Interactive / Messaging Family
Communicates with the human operator.

| Tool | Required Permission | Notes |
|---|---|---|
| `AskUserQuestion` | `ReadOnly` | Interactive prompt with optional choice set |
| `SendUserMessage` / `Brief` | `ReadOnly` | Sends a message to the user with optional attachments |
| `Config` | `WorkspaceWrite` | Reads/writes Claude Code settings |

### Planning / Mode Family
Controls session planning mode.

| Tool | Required Permission | Notes |
|---|---|---|
| `EnterPlanMode` | `WorkspaceWrite` | Worktree-local planning mode override |
| `ExitPlanMode` | `WorkspaceWrite` | Restores previous local planning mode setting |

### Output / Utility Family

| Tool | Required Permission | Notes |
|---|---|---|
| `StructuredOutput` | `ReadOnly` | Returns structured JSON in a requested format |
| `ToolSearch` | `ReadOnly` | Searches deferred or specialized tools by name/keywords |
| `Sleep` | `ReadOnly` | Duration-based wait without holding a shell |
| `RemoteTrigger` | `DangerFullAccess` | HTTP/webhook dispatch (GET/POST/PUT/DELETE/PATCH/HEAD) |
| `TestingPermission` | `DangerFullAccess` | Test-only permission verification |

---

## Allowed-Tools Normalization and Aliasing

`GlobalToolRegistry::normalize_allowed_tools()` accepts a list of tool names and normalizes them before checking against the registry. It supports:

1. **Case-insensitive matching:** `Read_File` → `read_file`
2. **Hyphen-to-underscore:** `read-file` → `read_file`
3. **Short-form aliases:**
   - `read` → `read_file`
   - `write` → `write_file`
   - `edit` → `edit_file`
   - `glob` → `glob_search`
   - `grep` → `grep_search`

This allows policy configuration and `--allowedTools` flags to accept user-friendly aliases.

**Evidence:** `GlobalToolRegistry::normalize_allowed_tools()` in `rust/crates/tools/src/lib.rs`

---

## Permission-Aware Execution Path

Built-in tool execution does not go directly from dispatch to implementation. Instead, `execute_tool_with_enforcer()` wraps each call through `PermissionEnforcer`:

```rust
fn execute_tool_with_enforcer(
    enforcer: Option<&PermissionEnforcer>,
    name: &str,
    input: &Value,
) -> Result<String, String> {
    match name {
        "bash" => {
            let bash_input: BashCommandInput = from_value(input)?;
            let classified_mode = classify_bash_permission(&bash_input.command);
            maybe_enforce_permission_check_with_mode(enforcer, name, input, classified_mode)?;
            run_bash(bash_input)
        }
        "read_file" => {
            maybe_enforce_permission_check(enforcer, name, input)?;
            from_value::<ReadFileInput>(input).and_then(run_read_file)
        }
        // ...
    }
}
```

The `enforcer` is set on the `GlobalToolRegistry` via `with_enforcer()`. When the runtime executes a tool turn, it passes the active `PermissionEnforcer` so permission checks are applied before the tool body runs.

**Evidence:** `execute_tool_with_enforcer()`, `maybe_enforce_permission_check()`, `maybe_enforce_permission_check_with_mode()` in `rust/crates/tools/src/lib.rs`

---

## Dynamic Command Classification for Shell Tools

`bash` and `PowerShell` use **dynamic permission classification** rather than a fixed per-tool permission. The `classify_bash_permission()` function inspects the actual command string and classifies it as either:

- **`PermissionMode::WorkspaceWrite`** — for read-only commands (`cat`, `grep`, `ls`, `find`, etc.) targeting workspace paths
- **`PermissionMode::DangerFullAccess`** — for mutating commands or commands with paths outside the workspace

```rust
fn classify_bash_permission(command: &str) -> PermissionMode {
    const READ_ONLY_COMMANDS: &[&str] = &[
        "cat", "head", "tail", "less", "more", "ls", "ll", "dir",
        "find", "test", "[", "[[", "grep", "rg", "awk", "sed", ...
    ];
    // Check base command, then path safety...
}
```

This allows `bash` to be usable in `WorkspaceWrite` mode for read-heavy workflows while still gating mutating commands.

**Evidence:** `classify_bash_permission()`, `classify_powershell_permission()`, `has_dangerous_paths()` in `rust/crates/tools/src/lib.rs`

---

## Tool Execution Output Shape

Tool results are returned as **structured JSON**, not freeform strings. Each handler serializes its output via `to_pretty_json()`:

```rust
fn run_read_file(input: ReadFileInput) -> Result<String, String> {
    to_pretty_json(read_file(&input.path, input.offset, input.limit).map_err(io_to_string)?)
}
```

The `ToolExecutor` trait in the runtime consumes these serialized strings and wraps them in `ToolResultContentBlock` for injection into the conversation transcript as model input for the next iteration.

Example output shapes:

```json
// read_file output
{ "path": "...", "content": "...", "bytes": 1234 }

// glob_search output
{ "files": ["path/to/file.rs", ...], "count": 42 }

// TaskOutput output
{ "task_id": "...", "output": "...", "has_output": true }
```

**Evidence:** `to_pretty_json()` usage throughout `rust/crates/tools/src/lib.rs`, `ToolExecutor` trait in `rust/crates/runtime/src/lib.rs`

---

## Plugin Tool Execution

Plugin tools are first-class citizens in the tool surface. They:

1. Are registered via `GlobalToolRegistry::with_plugin_tools()`
2. Are checked for name conflicts against built-in `mvp_tool_specs()`
3. Are dispatched in `execute()` after the built-in check fails
4. Carry their own `required_permission` metadata translated via `permission_mode_from_plugin()`

```rust
pub fn with_plugin_tools(plugin_tools: Vec<PluginTool>) -> Result<Self, String> {
    let builtin_names = mvp_tool_specs()...;
    // Rejects duplicate names
}

pub fn execute(&self, name: &str, input: &Value) -> Result<String, String> {
    if mvp_tool_specs().iter().any(|spec| spec.name == name) {
        return execute_tool_with_enforcer(self.enforcer.as_ref(), name, input);
    }
    self.plugin_tools
        .iter()
        .find(|tool| tool.definition().name == name)
        .ok_or_else(|| format!("unsupported tool: {name}"))?
        .execute(input)
        .map_err(|error| error.to_string())
}
```

**Evidence:** `GlobalToolRegistry::with_plugin_tools()`, `GlobalToolRegistry::execute()` in `rust/crates/tools/src/lib.rs`

---

## MCP-Facing Execution Surfaces

MCP tools are exposed through two distinct surfaces:

1. **Built-in MCP tools** (`ListMcpResources`, `ReadMcpResource`, `McpAuth`, `MCP`) — declared in `mvp_tool_specs()` and dispatched through `global_mcp_registry()` (`McpToolRegistry`). These provide MCP server discovery, resource access, and tool invocation.
2. **Runtime MCP bridge** (`rust/crates/runtime/src/mcp_tool_bridge.rs`) — `McpToolRegistry` tracks connection state, lists tools and resources per server, and handles tool call dispatch.

The runtime also surfaces degraded MCP startup semantics via `McpDegradedReport`, which `ToolSearch` can return to indicate pending or partially-failed MCP servers.

**Evidence:** `rust/crates/runtime/src/mcp_tool_bridge.rs`, `McpToolRegistry` usage in `rust/crates/tools/src/lib.rs`

---

## Multi-Tool Same-Turn Behavior

A single model turn can include **multiple tool calls** before a final assistant message is produced. The runtime loop in `ConversationRuntime` processes tool call blocks iteratively:

1. Model emits one or more `tool_use` content blocks in a single response
2. Runtime dispatches each tool call in sequence through `ToolExecutor::execute()`
3. Tool results are accumulated as `tool_result` content blocks
4. The full block is appended to the conversation transcript
5. Model receives the accumulated results in the next request

The `TurnSummary` emitted after a turn captures all tool results, iteration count, and usage telemetry.

**Evidence:** `ConversationRuntime` turn processing in `rust/crates/runtime/src/conversation.rs`, `rust/crates/runtime/src/lib.rs` (`ToolExecutor`, `TurnSummary`)

---

## Parity Evidence for Execution Paths

ClawCode's mock parity harness validates tool execution behavior through scripted scenarios in `rust/crates/rusty-claude-cli/tests/mock_parity_harness.rs`. Relevant scenarios:

| Scenario | What it validates |
|---|---|
| `read_file_roundtrip` | Read path execution and final synthesis |
| `write_file_allowed`, `write_file_denied` | Workspace-write permission enforcement |
| `grep_chunk_assembly` | Chunked grep tool output handling |
| `multi_tool_turn_roundtrip` | Multiple tool calls within a single turn |
| `bash_stdout_roundtrip` | Bash execution and output capture |
| `bash_permission_prompt_approved`, `bash_permission_prompt_denied` | Interactive permission escalation |
| `plugin_tool_roundtrip` | Plugin tool execution path |

The scenario manifest in `rust/mock_parity_scenarios.json` maps each scenario to its `parity_refs` entry in `PARITY.md`, creating an evidence chain from behavioral claim to test artifact.

**Evidence:** `rust/mock_parity_scenarios.json`, `rust/crates/rusty-claude-cli/tests/mock_parity_harness.rs`, `references/claw-code/PARITY.md`

---

## Builder Lessons

1. **Tool names are flat across layers.** Built-in names are reserved; plugins cannot override `bash`, `read_file`, `grep_search`, etc. When adding a new built-in tool, check for name collisions with `mvp_tool_specs()`.

2. **Permission classification is per-invocation for shell tools.** `bash` and `PowerShell` do not use a fixed `required_permission` from the spec. Instead, `classify_bash_permission()` and `classify_powershell_permission()` inspect the command at runtime to derive the effective required mode. This is the `maybe_enforce_permission_check_with_mode()` pattern.

3. **Output shape is always structured JSON.** All tool handlers return `Result<String, String>` where the `Ok` variant is a JSON string. This contract is what the runtime's `ToolExecutor` trait expects. Adding a new tool must follow this pattern.

4. **Allowed-tools aliasing normalizes user input.** The `normalize_allowed_tools()` function is the place to add new aliases (e.g., `read` → `read_file`). It is called before permission checking and dispatch.

5. **Registry-backed tools (Task, Team, Cron, Worker) are in-memory only.** They do not persist across process restarts and do not interface with an external scheduler. This is a current implementation boundary noted in `PARITY.md`.

6. **MCP tools and LSP tools route through global registries.** `global_mcp_registry()` and `global_lsp_registry()` are `OnceLock` singletons that survive across tool calls within a session. Their state is per-process.
