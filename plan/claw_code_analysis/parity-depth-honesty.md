# Parity Depth Honesty

## What this document is

This document explains what the four parity depth levels — **strong**, **good**, **moderate**, and **stub** — actually mean when applied to ClawCode's Rust implementation. It is meant to help builders understand which tools behave like their upstream counterparts, which are partially implemented, and which are placeholders that need real work.

The canonical source for which tools fall into which bucket is `references/claw-code/rust/PARITY.md`. This document explains the *criteria* behind those categorizations and names the specific implementation details that justify each rating.

---

## Strong parity

**Definition:** Full behavioral match with upstream, including deep validation submodules, edge case coverage, and integration with permission enforcement and sandboxing.

**Characteristics:**
- Real subprocess execution with timeout, background, and sandbox support
- Multiple independent validation submodules covering distinct failure classes
- Integration with the permission enforcement layer
- Mock parity harness coverage of both happy-path and denial flows

### bash tool — the canonical strong parity example

The bash tool in `rust/crates/runtime/src/bash.rs` (283 LOC) is the reference implementation for strong parity. It is not a single monolithic handler — it is divided into nine validation submodules tracked in `rust/crates/runtime/src/bash_validation.rs` (1004 LOC on the `jobdori/bash-validation-submodules` branch):

| Submodule | Responsibility |
|-----------|---------------|
| `readOnlyValidation` | Block writes in read-only permission mode |
| `destructiveCommandWarning` | Warn on `rm -rf`, `dd`, and similar destructive commands |
| `modeValidation` | Validate command intent against current permission mode |
| `sedValidation` | Validate sed commands before execution |
| `pathValidation` | Validate file paths referenced in commands |
| `commandSemantics` | Classify command intent (read vs. write vs. danger-full) |
| `bashPermissions` | Gate commands by required permission level |
| `bashSecurity` | Security boundary checks |
| `shouldUseSandbox` | Sandbox decision logic |

**Evidence:** `rust/crates/runtime/src/bash.rs` owns subprocess spawning and timeout handling. `rust/crates/runtime/src/bash_validation.rs` (branch-only at `36dac6c`) owns the nine submodules. `rust/crates/runtime/src/permission_enforcer.rs` (340 LOC on `main`) enforces read-only gating and mutating-command denial at runtime.

**Registry-backed caveat:** Strong parity on bash does not mean the bash subprocess runs inside a containerized sandbox on every platform. `rust/crates/runtime/src/sandbox.rs` (385 LOC) probes `unshare` capability and container signals rather than assuming sandbox support from binary presence. On platforms where `unshare` is unavailable, the tool falls back to capability probing — this is correct behavior, but it means the sandbox depth is environment-dependent, not guaranteed.

**Branch-only caveat:** The nine validation submodules are on a feature branch (`jobdori/bash-validation-submodules`) at commit `36dac6c`. On `main`, the sandbox and permission enforcement runtime support is present, but the dedicated validation module is not merged. The PARITY.md table correctly notes this with "bash validation is still branch-only" language.

---

## Good parity

**Definition:** Real behavioral implementation that is not a stub. The tool does actual work — it reads files, writes files, queries registries, or dispatches to real handlers. However, the implementation may lack full upstream depth or use in-memory registries instead of external runtime integrations.

**Characteristics:**
- Concrete handler functions that perform real operations
- Error handling for common failure cases
- Integration with the tool dispatch layer (`rust/crates/tools/src/lib.rs::execute_tool`)
- Registry-backed state for tools that manage long-lived resources

### File tools — good parity with behavioral substance

The file tools (`read_file`, `write_file`, `edit_file`, `grep_search`, `glob_search`) live in `rust/crates/runtime/src/file_ops.rs` (744 LOC on `main`). They are good parity, not strong, because:

- **Real offset/limit read** — `read_file` supports `offset` and `limit` parameters
- **Size limits enforced** — `MAX_READ_SIZE` and `MAX_WRITE_SIZE` guards prevent unbounded I/O
- **Binary detection** — NUL-byte detection rejects binary files in text contexts
- **Workspace boundary validation** — canonical path checking prevents symlink escape and `../` traversal

**Evidence:** `rust/crates/runtime/src/file_ops.rs::read_file()` and `rust/crates/runtime/src/file_ops.rs::write_file()` are concrete implementations, not fixed-payload stubs. The `MAX_READ_SIZE` and `MAX_WRITE_SIZE` constants are defined in the same file.

**What is missing for strong parity:** The file tools do not have a dedicated validation submodule structure the way bash does. Edge cases like circular symlink detection, path normalization edge cases, and very large file handling are guarded but not separately tracked as independent validation concerns.

### Registry-backed tools — good parity with in-memory caveats

Several tool families achieve good parity through in-memory registries rather than external runtime integrations:

| Tool family | Registry | Evidence |
|-------------|----------|----------|
| Task* (Create, Get, List, Stop, Update, Output) | `runtime::task_registry::TaskRegistry` | `rust/crates/runtime/src/task_registry.rs` (335 LOC) |
| Team* (Create, Delete) | `runtime::team_cron_registry::TeamRegistry` | `rust/crates/runtime/src/team_cron_registry.rs` (363 LOC) |
| Cron* (Create, Delete, List) | `runtime::team_cron_registry::CronRegistry` | Same file |
| LSP | `runtime::lsp_client::LspRegistry` | `rust/crates/runtime/src/lsp_client.rs` (438 LOC) |
| MCP (ListResources, ReadResource, Tool) | `runtime::mcp_tool_bridge::McpToolRegistry` | `rust/crates/runtime/src/mcp_tool_bridge.rs` (406 LOC) |

**The registry-backed caveat:** These tools are not stubs — they have real handler functions that create, read, update, and delete records in thread-safe in-memory registries. But they do not integrate with real external runtimes:

- **Task tools** create in-memory task records; they do not spawn background subprocess workers or schedule real jobs
- **Team/Cron tools** manage in-memory team and schedule entries; they do not have a background scheduler or worker fleet
- **LSP tools** model diagnostics, hover, definition, references, completion, symbols, and formatting in a registry; they do not spawn an actual language server process or manage a real LSP connection lifecycle beyond the registry bridge
- **MCP tools** track server connection status, resource listings, and tool dispatch in `McpToolRegistry`; end-to-end MCP transport (`mcp_stdio.rs`, `mcp_client.rs`, `mcp.rs`) is separate infrastructure not covered by the registry

**What good parity means in practice:** A `TaskCreate` call produces a real task record with a unique `task_id`, status, and timestamp. A `CronList` call returns entries from the real `CronRegistry`. This is meaningful behavior, not a stub response. But a builder who expects `TaskCreate` to schedule a background worker or `CronCreate` to register with a cron daemon will be disappointed — that integration does not exist on `main`.

**Evidence:** `rust/crates/tools/src/lib.rs::run_task_create()` calls `global_task_registry().create()`. `rust/crates/tools/src/lib.rs::run_cron_create()` calls `global_cron_registry().create()`. Both registries are `OnceLock` singletons initialized on first use.

---

## Moderate parity

**Definition:** Real implementation exists, but the behavioral surface has known gaps relative to upstream. The tool does not silently return stub responses, but it also does not fully replicate upstream behavior in one or more dimensions.

**Characteristics:**
- Concrete handler with real I/O or network operations
- Known behavioral gaps documented in the tool's implementation comments or PARITY.md
- May depend on external services with different availability characteristics than upstream

### WebFetch and WebSearch

`WebFetch` (URL fetch + content extraction) and `WebSearch` (search query execution) in `rust/crates/tools/src/lib.rs` are moderate parity because:

- **Real HTTP operations** — `WebFetch` uses `reqwest::blocking::Client` to fetch URLs; `WebSearch` makes real search API calls
- **Content truncation** — responses over 8192 bytes are truncated with a note (`run_remote_trigger` at line 1785 of `lib.rs`)
- **Redirect handling** — likely follows HTTP redirects but has not been independently verified against upstream behavior for all redirect patterns
- **Search result parsing** — the fallback parser in tests (`websearch_fallback_parsing`) shows that response shape assumptions exist

**Evidence:** `rust/crates/tools/src/lib.rs::run_web_fetch()` and `rust/crates/tools/src/lib.rs::run_web_search()` call `execute_web_fetch()` and `execute_web_search()` respectively.

**What makes it moderate rather than good:** PARITY.md explicitly notes "need to verify content truncation, redirect handling vs upstream" for WebFetch and "moderate parity" for WebSearch. The truncation limit (8192 bytes) is hardcoded, not configurable.

### TodoWrite

`TodoWrite` in `rust/crates/tools/src/lib.rs` manages todo persistence to `.clawd-todos.json` in the current working directory. It is moderate parity because:

- **Real file I/O** — `execute_todo_write()` reads and writes JSON to disk
- **No cross-agent sync** — todos are stored locally per workspace, not synchronized across multiple agent instances or sessions
- **No integration with upstream task system** — upstream may have a unified task/todo surface; the Rust implementation stores todos in a separate JSON file

**Evidence:** `rust/crates/tools/src/lib.rs::execute_todo_write()` uses `todo_store_path()` to locate `.clawd-todos.json`. The `verificationNudgeNeeded` field is a behavioral extra not present in all upstream implementations.

### Skill and Agent

`Skill` (skill discovery/install) and `Agent` (agent delegation) are moderate parity:

- **Skill** loads skill definitions and legacy command markdown files from `.claude/commands/` — real file-based resolution. But skill installation and the full skill lifecycle (install, enable, disable, uninstall) is not implemented.
- **Agent** spawns a subagent by building a new `ConversationRuntime` with `AgentInput` configuration — real runtime delegation. But the subagent runs synchronously in the same process, not as a truly isolated agent process.

**Evidence:** `rust/crates/tools/src/lib.rs::run_skill()` calls `execute_skill()`. `rust/crates/tools/src/lib.rs::run_agent()` calls `execute_agent()` which calls `build_agent_runtime()` and `runtime.run_turn()`.

---

## Stub

**Definition:** Surface parity only — the tool appears in `mvp_tool_specs()` and accepts well-formed input, but returns a fixed-payload response or performs no meaningful operation. The response structure may be correct, but the behavior does not replicate upstream.

**Characteristics:**
- Appears in the tool spec list (so it is discoverable)
- Has a concrete handler function, but the handler returns a fixed response or performs only trivial operations
- Cannot be used for meaningful work without further implementation

### AskUserQuestion — synchronous blocking stub

`AskUserQuestion` in `rust/crates/tools/src/lib.rs` reads from `stdin` and returns the user's response. It is a stub not because it does nothing, but because its synchronous stdin read cannot interleave with the runtime turn loop:

**Evidence:** `rust/crates/tools/src/lib.rs::run_ask_user_question()` (line 1327) reads from `io::stdin().lock().read_line()` synchronously. This blocks the tool executor, not just the turn — the entire runtime thread waits for stdin input.

**Why it is a stub:** The response structure (`{"question": ..., "answer": ..., "status": "answered"}`) is correct, but the execution model cannot pause and resume a turn in a streaming async runtime. The PARITY.md correctly classifies it as "needs live user I/O integration."

### RemoteTrigger — moderate parity with real HTTP but known gaps

`RemoteTrigger` in `rust/crates/tools/src/lib.rs` makes real HTTP requests using `reqwest::blocking::Client`. It is moderate parity, not stub — the HTTP client implementation is real and concrete:

**Evidence:** `rust/crates/tools/src/lib.rs::run_remote_trigger()` (line 1746) supports GET, POST, PUT, DELETE, PATCH, and HEAD methods, applies custom headers, sends a request body, and has a 30-second timeout. The response body is truncated at 8192 bytes (line 1785).

**What makes it moderate rather than good or strong:** The response truncation is a hardcoded behavioral ceiling — large webhook responses are silently truncated rather than returned in full or returning an error. The call is synchronous and blocking, not integrated with the async runtime event loop. No retry logic, no async webhook delivery, and no event subscription pattern.

### TestingPermission — fixed-payload stub

`TestingPermission` always returns `{"action": ..., "permitted": true, "message": "Testing permission tool stub"}`.

**Evidence:** `rust/crates/tools/src/lib.rs::run_testing_permission()` (line 1831) returns a fixed JSON payload with `permitted: true` unconditionally.

**Why it is a stub:** It performs no permission check whatsoever. It is explicitly test-only (`description: "Test-only tool for verifying permission enforcement behavior."`) and low priority, so this is intentional. A builder should not use this tool in production.

### McpAuth — registry-read stub

`McpAuth` reads connection state from `McpToolRegistry` and returns it as JSON.

**Evidence:** `rust/crates/tools/src/lib.rs::run_mcp_auth()` (line 1727) calls `global_mcp_registry().get_server()` and returns the server state.

**Why it is a stub:** It does not perform any authentication flow — it only reports whether a server is registered and its current connection status. The PARITY.md correctly notes "needs full auth UX beyond the MCP lifecycle bridge."

---

## How to use this document

When evaluating whether to build on a tool or report a gap, check the tool's parity level and its specific caveats:

1. **Strong parity tools** are safe for production use. Watch for branch-only submodules — check whether the feature branch has been merged before relying on validation submodule depth.

2. **Good parity tools** are safe for production use but be aware of what is not covered. Task tools manage in-memory state, not background workers. LSP tools model state in a registry, not a real language server process.

3. **Moderate parity tools** are usable but have known gaps. WebFetch has a hardcoded truncation limit. TodoWrite does not sync across agents. Skill does not implement the full install/enable/disable lifecycle.

4. **Stub tools** are not usable for meaningful work without additional implementation. They may have correct response structures but no meaningful behavioral implementation behind them.

---

## Builder lessons

**Parity depth is a property of the implementation, not the spec.** Two tools can have identical `ToolSpec` definitions in `mvp_tool_specs()` but very different implementation depth. Always check the handler function, not just the spec.

**Registry-backed does not mean stub.** `TaskRegistry`, `TeamRegistry`, `CronRegistry`, `LspRegistry`, and `McpToolRegistry` are real thread-safe data structures with real operations. A handler that uses a registry is doing real work. The caveat is that the registry is in-memory, not an external service.

**Synchronous blocking I/O in a tool handler blocks the entire runtime.** `AskUserQuestion` is the clearest example — it reads stdin synchronously and returns, which works for a batch program but breaks the interleaving model of a streaming runtime. Any tool that must pause and resume a turn needs a different integration pattern, not a synchronous handler.

**The truncation limit in `run_remote_trigger` (8192 bytes) is a behavioral fact, not an implementation detail.** Builders who need full webhook response bodies will hit this ceiling. The truncation is applied unconditionally, so large responses are silently truncated rather than returning an error.

**When PARITY.md and implementation differ, implementation wins for present-tense behavior.** The PARITY.md table may lag behind a recent merge. Always verify claims against the actual handler functions in `rust/crates/tools/src/lib.rs` and the underlying runtime modules.
