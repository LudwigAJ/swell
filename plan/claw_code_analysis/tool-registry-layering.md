# Tool Registry Layering

How built-in tools, plugin-provided tools, and runtime-added tool surfaces are layered, and how structured tool specs describe them.

**Artifact:** `analysis/tool-registry-layering.md`
**Skill:** `clawcode-doc-worker`
**Milestone:** tool-system
**Evidence:** `references/claw-code/rust/crates/tools/src/lib.rs`, `references/claw-code/PARITY.md`, `references/claw-code/rust/crates/rusty-claude-cli/tests/mock_parity_harness.rs`

---

## Overview

The Rust tool system in `references/claw-code/rust/crates/tools/src/lib.rs` layers three distinct tool surfaces into a single unified execution model:

1. **Built-in tools** — statically declared in `mvp_tool_specs()` with `ToolSpec` metadata
2. **Plugin tools** — contributed by external plugin crates at startup via `PluginTool`
3. **Runtime tools** — dynamically added during a session via `RuntimeToolDefinition`

The `GlobalToolRegistry` struct holds all three layers and exposes a single `execute()` dispatch that routes to the correct surface.

```rust
// rust/crates/tools/src/lib.rs
pub struct GlobalToolRegistry {
    plugin_tools: Vec<PluginTool>,
    runtime_tools: Vec<RuntimeToolDefinition>,
    enforcer: Option<PermissionEnforcer>,
}
```

This design means a session's effective tool surface is the union of all three layers, filtered by the session's active `allowed_tools` policy and permission mode.

---

## Layer 1: Built-in Tool Specs

Built-in tools are declared with a `ToolSpec` struct that carries the complete metadata contract:

```rust
// rust/crates/tools/src/lib.rs
pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub input_schema: Value,        // JSON Schema
    pub required_permission: PermissionMode,
}
```

The `mvp_tool_specs()` function returns ~40 tool specs covering these families:

| Family | Tools |
|--------|-------|
| **Shell** | `bash`, `REPL`, `PowerShell` |
| **File** | `read_file`, `write_file`, `edit_file` |
| **Search** | `glob_search`, `grep_search`, `ToolSearch` |
| **Web** | `WebFetch`, `WebSearch` |
| **Agent/Worker** | `Agent`, `WorkerCreate`, `WorkerGet`, `WorkerObserve`, `WorkerResolveTrust`, `WorkerAwaitReady`, `WorkerSendPrompt`, `WorkerRestart`, `WorkerTerminate`, `WorkerObserveCompletion` |
| **Task** | `TaskCreate`, `TaskGet`, `TaskList`, `TaskStop`, `TaskUpdate`, `TaskOutput`, `RunTaskPacket` |
| **Team/Cron** | `TeamCreate`, `TeamDelete`, `CronCreate`, `CronDelete`, `CronList` |
| **Todo** | `TodoWrite` |
| **Skill** | `Skill` |
| **Notebook** | `NotebookEdit` |
| **LSP** | `LSP` |
| **MCP** | `ListMcpResources`, `ReadMcpResource`, `McpAuth`, `MCP` |
| **Misc** | `Sleep`, `SendUserMessage`, `Config`, `EnterPlanMode`, `ExitPlanMode`, `StructuredOutput`, `AskUserQuestion`, `RemoteTrigger`, `TestingPermission` |

Each spec's `required_permission` field ties into the permission enforcement layer. For example, `read_file` requires `PermissionMode::ReadOnly`, while `write_file` requires `PermissionMode::WorkspaceWrite`.

---

## Layer 2: Plugin-Provided Tools

Plugins contribute tools at startup via `GlobalToolRegistry::with_plugin_tools()`:

```rust
// rust/crates/tools/src/lib.rs
pub fn with_plugin_tools(plugin_tools: Vec<PluginTool>) -> Result<Self, String> {
    let builtin_names = mvp_tool_specs()
        .into_iter()
        .map(|spec| spec.name.to_string())
        .collect::<BTreeSet<_>>();
    // ... conflict detection ...
}
```

Plugin tools go through a strict conflict check: any plugin tool whose name matches a built-in name is rejected at registration time. This prevents namespace collisions and keeps the tool identity namespace clean.

The harness scenario `plugin_tool_roundtrip` validates the plugin execution path end-to-end (see `references/claw-code/rust/crates/rusty-claude-cli/tests/mock_parity_harness.rs`).

---

## Layer 3: Runtime-Added Tools

Runtime tools are added via `GlobalToolRegistry::with_runtime_tools()`:

```rust
// rust/crates/tools/src/lib.rs
pub fn with_runtime_tools(
    mut self,
    runtime_tools: Vec<RuntimeToolDefinition>,
) -> Result<Self, String>
```

`RuntimeToolDefinition` mirrors `ToolSpec` but uses owned `String` types since these tools are dynamically registered:

```rust
pub struct RuntimeToolDefinition {
    pub name: String,
    pub description: Option<String>,
    pub input_schema: Value,
    pub required_permission: PermissionMode,
}
```

Runtime tools also undergo conflict checking against both built-in and plugin tool names.

---

## Allowed-Tools Normalization and Aliasing

The registry supports a normalization layer for allowed-tool policy:

```rust
// rust/crates/tools/src/lib.rs
pub fn normalize_allowed_tools(
    &self,
    values: &[String],
) -> Result<Option<BTreeSet<String>>, String>
```

This handles:
- **Alias expansion:** `read` → `read_file`, `write` → `write_file`, `edit` → `edit_file`, `glob` → `glob_search`, `grep` → `grep_search`
- **Comma and whitespace splitting** in the input value
- **Case-insensitive matching** via `normalize_tool_name()`

This means configuration and policy operate on normalized canonical names regardless of how the user or config expresses the tool list.

---

## Permission-Aware Execution Path

Built-in tool execution always routes through `execute_tool_with_enforcer()`:

```rust
// rust/crates/tools/src/lib.rs
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

Plugin tools, by contrast, execute directly via their own `execute()` method on `PluginTool`. The enforcer is not injected into plugin execution in the same way, which is a design distinction worth noting for builders extending the system.

---

## Dynamic Command Classification for Shell-Like Tools

Shell executors (`bash`, `PowerShell`) use dynamic permission classification because the actual command determines the required permission level, not just the tool name:

```rust
// rust/crates/tools/src/lib.rs
fn classify_bash_permission(command: &str) -> PermissionMode {
    const READ_ONLY_COMMANDS: &[&str] = &[
        "cat", "head", "tail", "less", "more", "ls", "ll", "dir", "find", "test", "[", "[[",
        "grep", "rg", "awk", "sed", "file", "stat", "readlink", "wc", "sort", "uniq", "cut", "tr",
        "pwd", "echo", "printf",
    ];
    // ...
}
```

If the command matches a known read-only command AND all paths are within the workspace, the effective required permission is `WorkspaceWrite`. Otherwise it is `DangerFullAccess`. This allows read-only mode to permit conservative read commands while denying mutating ones.

The same pattern applies to `PowerShell` via `classify_powershell_permission()`.

---

## Structured Tool Output Shape

Tool results are serialized as structured JSON, not freeform strings. Each `run_*` handler returns a `Result<String, String>` where the `Ok` variant is pretty-printed JSON:

```rust
// rust/crates/tools/src/lib.rs
fn run_read_file(input: ReadFileInput) -> Result<String, String> {
    to_pretty_json(read_file(&input.path, input.offset, input.limit).map_err(io_to_string)?)
}
```

Examples of structured output shapes:

- `read_file` → `{"path": "...", "content": "...", "offset": 0, "limit": 100, "lines": 5}`
- `TaskCreate` → `{"task_id": "...", "status": "running", "prompt": "...", "created_at": "..."}`
- `bash` → `{"stdout": "...", "stderr": "...", "return_code": 0, ...}`

The harness validates this in scenarios like `read_file_roundtrip` and `bash_stdout_roundtrip`.

---

## MCP Tool Bridge

MCP tools are surfaced through the `McpToolRegistry` (defined in `references/claw-code/rust/crates/runtime/src/mcp_tool_bridge.rs`) and exposed via:

- `ListMcpResources` — list available resources from connected MCP servers
- `ReadMcpResource` — read a specific resource by URI
- `McpAuth` — authenticate with an MCP server
- `MCP` — execute a tool provided by a connected MCP server

The `GlobalToolRegistry::search()` method includes deferred tool specs and MCP tools in its search results via `searchable_tool_specs()`.

---

## Multi-Tool Same-Turn Behavior

The harness scenario `multi_tool_turn_roundtrip` validates that a single model turn can include multiple tool calls before returning final assistant output. This is a first-class behavior of the turn loop defined in `references/claw-code/rust/crates/runtime/src/conversation.rs`, not a special-case hack.

---

## Parity Evidence for Tool Execution Paths

The mock parity harness (`references/claw-code/rust/crates/rusty-claude-cli/tests/mock_parity_harness.rs`) exercises:

| Scenario | What It Validates |
|----------|-------------------|
| `read_file_roundtrip` | Read-path execution and final synthesis |
| `grep_chunk_assembly` | Chunked grep tool output handling |
| `write_file_allowed` | Write success under `workspace-write` |
| `write_file_denied` | Write denial under `read-only` |
| `multi_tool_turn_roundtrip` | Multiple tool uses in a single turn |
| `bash_stdout_roundtrip` | Bash stdout capture |
| `bash_permission_prompt_approved` | Bash prompt approval flow |
| `bash_permission_prompt_denied` | Bash prompt denial flow |
| `plugin_tool_roundtrip` | Plugin tool execution surface |
| `auto_compact_triggered` | Auto-compaction after turn |
| `token_cost_reporting` | Usage and cost reporting |

The `PARITY.md` document maps these scenarios to the 9-lane checkpoint and is the canonical source of truth consumed by `rust/scripts/run_mock_parity_diff.py`.

---

## Builder Lessons

1. **Three-layer registry is a composition pattern, not inheritance.** `GlobalToolRegistry` holds three separate `Vec` fields and chains them in `definitions()`, `permission_specs()`, and `searchable_tool_specs()`. This is easy to extend — add a new field and a new chain line.

2. **Conflict detection at registration time is cheap insurance.** Both plugin and runtime tool registration walk the existing name set and reject duplicates eagerly. This prevents subtle runtime failures from naming collisions.

3. **Normalization at the boundary keeps policy simple.** The alias expansion and normalization in `normalize_allowed_tools()` means the rest of the system never needs to handle `read` vs `read_file` — only the canonical form.

4. **Dynamic classification separates tool identity from permission.** `bash` is always `bash` in the tool identity, but its required permission is derived from the command string. This keeps the dispatch table simple while enabling nuanced enforcement.

5. **Structured output is the contract.** Every tool returns `Result<String, String>` where the `Ok` is JSON. This makes the result machine-parseable downstream and avoids freeform string parsing throughout the system.

---

## Key Files

| File | Role |
|------|------|
| `rust/crates/tools/src/lib.rs` | Tool registry, `mvp_tool_specs()`, execution dispatch |
| `rust/crates/runtime/src/mcp_tool_bridge.rs` | MCP tool bridge |
| `rust/crates/runtime/src/permission_enforcer.rs` | Permission enforcement |
| `rust/crates/rusty-claude-cli/tests/mock_parity_harness.rs` | Harness scenarios |
| `PARITY.md` | Parity evidence and scenario map |
| `rust/mock_parity_scenarios.json` | Scenario manifest |
