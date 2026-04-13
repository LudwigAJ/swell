# Permission-Aware Tool Dispatch

How built-in tool invocations route through the permission enforcement layer before execution, and how shell-like tools dynamically classify their required permission from the actual command string.

**Artifact:** `analysis/permission-aware-tool-dispatch.md`
**Skill:** `clawcode-doc-worker`
**Milestone:** tool-system
**Evidence:** `references/claw-code/rust/crates/tools/src/lib.rs`, `references/claw-code/rust/crates/runtime/src/permissions.rs`, `references/claw-code/rust/crates/runtime/src/permission_enforcer.rs`

---

## Overview

When a tool call is dispatched in ClawCode, it does not execute directly. Built-in tools always route through `GlobalToolRegistry::execute()` → `execute_tool_with_enforcer()`, which gates execution behind the `PermissionEnforcer` layer. This mediated path applies permission mode checks, allow/deny/ask rules, and interactive prompting before the tool body runs.

The permission enforcement layer is composed of three interacting types:

- **`PermissionMode`** — the active session permission level (`ReadOnly`, `WorkspaceWrite`, `DangerFullAccess`, `Prompt`, `Allow`)
- **`PermissionPolicy`** — combines an active mode with per-tool requirements and allow/deny/ask rules
- **`PermissionEnforcer`** — the runtime object that runtime passes to `GlobalToolRegistry::execute()`, evaluated by `execute_tool_with_enforcer()` before tool bodies run

```rust
// rust/crates/tools/src/lib.rs
pub struct GlobalToolRegistry {
    plugin_tools: Vec<PluginTool>,
    runtime_tools: Vec<RuntimeToolDefinition>,
    enforcer: Option<PermissionEnforcer>,  // Set by with_enforcer()
}
```

Plugin tools and runtime tools are not gated by the same enforcement path — plugin tools execute via their own `execute()` method on `PluginTool`, and runtime tools bypass the enforcer in the current implementation. This distinction matters for builders extending the tool surface.

---

## Permission Modes

`PermissionMode` in `rust/crates/runtime/src/permissions.rs` defines five levels ordered from most restrictive to least:

```rust
pub enum PermissionMode {
    ReadOnly,        // File reads, glob, grep, web — no writes
    WorkspaceWrite,  // File writes within workspace root only
    DangerFullAccess, // All file operations, all bash — no restrictions
    Prompt,          // Interactive confirmation required before dangerous ops
    Allow,           // Everything permitted, skips all enforcement
}
```

The ordering (`ReadOnly < WorkspaceWrite < DangerFullAccess < Prompt < Allow`) is used in `PartialOrd` comparisons: if the active mode is greater-than-or-equal to the required mode, the operation is permitted.

Each built-in tool declares its required permission in `mvp_tool_specs()`:

```rust
ToolSpec {
    name: "read_file",
    required_permission: PermissionMode::ReadOnly,
    // ...
}
ToolSpec {
    name: "write_file",
    required_permission: PermissionMode::WorkspaceWrite,
    // ...
}
ToolSpec {
    name: "bash",
    required_permission: PermissionMode::DangerFullAccess,  // Base; dynamically escalated
    // ...
}
```

---

## The Mediated Dispatch Path

When the runtime processes a tool call, it calls `GlobalToolRegistry::execute()` with an attached `PermissionEnforcer`:

```rust
// rust/crates/tools/src/lib.rs
pub fn execute(&self, name: &str, input: &Value) -> Result<String, String> {
    if mvp_tool_specs().iter().any(|spec| spec.name == name) {
        return execute_tool_with_enforcer(self.enforcer.as_ref(), name, input);
    }
    // Plugin tools fall through to their own execute() method
}
```

`execute_tool_with_enforcer()` is the gate for all built-in tools:

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
        "write_file" => {
            maybe_enforce_permission_check(enforcer, name, input)?;
            from_value::<WriteFileInput>(input).and_then(run_write_file)
        }
        // ... all other built-in tools
    }
}
```

The `maybe_enforce_permission_check()` function applies the check:

```rust
fn maybe_enforce_permission_check(
    enforcer: Option<&PermissionEnforcer>,
    tool_name: &str,
    input: &Value,
) -> Result<(), String> {
    if let Some(enforcer) = enforcer {
        enforce_permission_check(enforcer, tool_name, input)?;
    }
    Ok(())
}
```

Which calls through to `PermissionEnforcer::check()`:

```rust
// rust/crates/runtime/src/permission_enforcer.rs
pub fn check(&self, tool_name: &str, input: &str) -> EnforcementResult {
    if self.policy.active_mode() == PermissionMode::Prompt {
        return EnforcementResult::Allowed;  // Defer to interactive flow
    }
    let outcome = self.policy.authorize(tool_name, input, None);
    match outcome {
        PermissionOutcome::Allow => EnforcementResult::Allowed,
        PermissionOutcome::Deny { reason } => EnforcementResult::Denied { ... },
    }
}
```

`EnforcementResult` carries structured denial information:

```rust
pub enum EnforcementResult {
    Allowed,
    Denied {
        tool: String,
        active_mode: String,
        required_mode: String,
        reason: String,
    },
}
```

If `EnforcementResult::Denied` is returned, `execute_tool_with_enforcer()` propagates the denial as an error string, which the runtime turns into a tool-result error message in the transcript.

---

## PermissionPolicy: Rules and Mode Comparison

`PermissionPolicy` in `rust/crates/runtime/src/permissions.rs` combines three sources of authorization logic:

1. **Active mode** — the session's baseline permission level
2. **Per-tool requirements** — minimum required mode per tool name
3. **Allow/deny/ask rules** — glob-pattern rules evaluated against `tool_name(input_subject)`

```rust
pub struct PermissionPolicy {
    active_mode: PermissionMode,
    tool_requirements: BTreeMap<String, PermissionMode>,
    allow_rules: Vec<PermissionRule>,
    deny_rules: Vec<PermissionRule>,
    ask_rules: Vec<PermissionRule>,
}
```

`authorize()` evaluates in priority order:

```
deny rules → hook deny override → hook ask override → ask rules → allow rules → mode comparison → prompt or deny
```

The `PermissionContext` carries hook-provided overrides (`Allow`, `Deny`, `Ask`) applied before the standard evaluation. This is how pre-tool and post-tool hooks can influence permission decisions without implementing a full prompter.

The `PermissionRequest` structure passed to interactive prompts carries the full context:

```rust
pub struct PermissionRequest {
    pub tool_name: String,
    pub input: String,
    pub current_mode: PermissionMode,
    pub required_mode: PermissionMode,
    pub reason: Option<String>,
}
```

---

## Dynamic Shell-Command Classification

For most tools, the required permission is static — `read_file` always needs `ReadOnly`, `write_file` always needs `WorkspaceWrite`. Shell tools (`bash`, `PowerShell`) are different: the danger level depends on the actual command string, not just the tool name.

`classify_bash_permission()` derives the effective required permission at runtime:

```rust
// rust/crates/tools/src/lib.rs
fn classify_bash_permission(command: &str) -> PermissionMode {
    const READ_ONLY_COMMANDS: &[&str] = &[
        "cat", "head", "tail", "less", "more", "ls", "ll", "dir", "find", "test", "[", "[[",
        "grep", "rg", "awk", "sed", "file", "stat", "readlink", "wc", "sort", "uniq", "cut", "tr",
        "pwd", "echo", "printf",
    ];

    let cmd_name = extract_command_name(command);
    let is_read_only = READ_ONLY_COMMANDS.contains(&cmd_name);

    if !is_read_only {
        return PermissionMode::DangerFullAccess;
    }

    if has_dangerous_paths(command) {
        return PermissionMode::DangerFullAccess;
    }

    PermissionMode::WorkspaceWrite
}
```

`classify_powershell_permission()` applies the same pattern for PowerShell:

```rust
// rust/crates/tools/src/lib.rs
fn classify_powershell_permission(command: &str) -> PermissionMode {
    const READ_ONLY_COMMANDS: &[&str] = &[
        "Get-Content", "Get-ChildItem", "Test-Path", "Get-Item", "Get-ItemProperty",
        "Select-Object", "Where-Object", "ForEach-Object", "Sort-Object", "Measure-Object",
        "Compare-Object", "Group-Object", "ConvertTo-Json", "ConvertFrom-Json",
    ];
    // Similar logic: read-only command + workspace path → WorkspaceWrite
}
```

The `maybe_enforce_permission_check_with_mode()` function uses the dynamically classified mode:

```rust
fn maybe_enforce_permission_check_with_mode(
    enforcer: Option<&PermissionEnforcer>,
    tool_name: &str,
    input: &Value,
    required_mode: PermissionMode,
) -> Result<(), String> {
    if let Some(enforcer) = enforcer {
        let input_str = serde_json::to_string(input).unwrap_or_default();
        let result = enforcer.check_with_required_mode(tool_name, &input_str, required_mode);
        match result {
            EnforcementResult::Allowed => Ok(()),
            EnforcementResult::Denied { reason, .. } => Err(reason),
        }
    } else {
        Ok(())
    }
}
```

`PermissionEnforcer::check_with_required_mode()` compares the active mode against the dynamically determined required mode:

```rust
// rust/crates/runtime/src/permission_enforcer.rs
pub fn check_with_required_mode(
    &self,
    tool_name: &str,
    input: &str,
    required_mode: PermissionMode,
) -> EnforcementResult {
    let active_mode = self.policy.active_mode();
    if active_mode >= required_mode {
        EnforcementResult::Allowed
    } else {
        EnforcementResult::Denied { ... }
    }
}
```

This means `bash cat file.txt` in `ReadOnly` mode works (command is read-only, effective required = `WorkspaceWrite`, but `ReadOnly` is not >= `WorkspaceWrite` — so it would actually be denied). A more precise example: `bash cat file.txt` in `WorkspaceWrite` mode succeeds because `WorkspaceWrite` >= `WorkspaceWrite`. `bash rm file.txt` in any mode below `DangerFullAccess` is denied.

The `PermissionEnforcer::check_bash()` method provides a separate mode-specific check used for heuristic enforcement in lower-level bash execution:

```rust
pub fn check_bash(&self, command: &str) -> EnforcementResult {
    let mode = self.policy.active_mode();
    match mode {
        PermissionMode::ReadOnly => {
            if is_read_only_command(command) {
                EnforcementResult::Allowed
            } else {
                EnforcementResult::Denied { ... }
            }
        }
        PermissionMode::Prompt => EnforcementResult::Denied { ... },
        _ => EnforcementResult::Allowed,
    }
}
```

The `is_read_only_command()` heuristic in `permission_enforcer.rs` overlaps with `classify_bash_permission()` but is used at a different enforcement point — `check_bash()` gates the bash subprocess spawn path, while `classify_bash_permission()` gates the tool-level dispatch.

---

## Workspace Boundary Enforcement

`PermissionEnforcer::check_file_write()` enforces that in `WorkspaceWrite` mode, file writes are restricted to within the configured workspace root:

```rust
pub fn check_file_write(&self, path: &str, workspace_root: &str) -> EnforcementResult {
    match self.policy.active_mode() {
        PermissionMode::ReadOnly => EnforcementResult::Denied { ... },
        PermissionMode::WorkspaceWrite => {
            if is_within_workspace(path, workspace_root) {
                EnforcementResult::Allowed
            } else {
                EnforcementResult::Denied { reason: format!("path '{path}' is outside workspace root") }
            }
        }
        PermissionMode::Allow | PermissionMode::DangerFullAccess => EnforcementResult::Allowed,
        PermissionMode::Prompt => EnforcementResult::Denied { ... },
    }
}
```

The `is_within_workspace()` check canonicalizes paths and compares against the workspace root prefix. This is complemented by lower-level file operation safety in `rust/crates/runtime/src/file_ops.rs` (canonical boundary validation, symlink-escape rejection, binary detection, size limits), which operates below the permission layer.

---

## Denial as Explicit Tool-Result

When a tool is denied, the denial is not silent. `execute_tool_with_enforcer()` returns the denial reason as an `Err`, which the runtime's `ToolExecutor` converts into a structured error tool result:

```rust
EnforcementResult::Denied {
    tool: String,
    active_mode: String,
    required_mode: String,
    reason: String,
}
```

The harness scenarios `write_file_denied` and `bash_permission_prompt_denied` validate this behavior — denied operations produce machine-parseable error content in the transcript rather than disappearing or producing opaque failures.

---

## Builder Lessons

1. **The enforcer is set at registry construction, not at dispatch time.** `GlobalToolRegistry::with_enforcer()` attaches the `PermissionEnforcer` once. All subsequent `execute()` calls through the same registry instance use that enforcer. If you construct a fresh registry without an enforcer, built-in tools bypass enforcement — useful for testing, but a security gap in production paths.

2. **Shell tools use a two-stage permission check.** The base tool spec declares `DangerFullAccess` as the required permission, but `classify_bash_permission()` dynamically downgrades this to `WorkspaceWrite` for read-only commands. The enforcement point is `maybe_enforce_permission_check_with_mode()`, which calls `check_with_required_mode()` instead of `check()`. If you add a new shell-like tool, follow this same pattern — parse the input to extract the command, classify it, then call `maybe_enforce_permission_check_with_mode()`.

3. **Plugin tools bypass the built-in enforcement path.** `GlobalToolRegistry::execute()` checks built-ins first via `execute_tool_with_enforcer()`, then falls through to plugin tools which use their own `execute()` method. Plugin tool permissions are enforced via `permission_mode_from_plugin()` at registration time (which translates `"read-only"` → `PermissionMode::ReadOnly`, etc.), but the runtime enforcement path for plugin tools is different from built-in tools.

4. **`PermissionContext` lets hooks short-circuit the evaluation.** Hooks can inject `PermissionOverride::Deny`, `PermissionOverride::Allow`, or `PermissionOverride::Ask` before the standard policy evaluation runs. This is the pre-tool/post-tool hook integration point for permission influence — the hook outcome is evaluated before allow/deny rules or mode comparison.

5. **`EnforcementResult` is the deny payload, not just a boolean.** The `Denied` variant carries `tool`, `active_mode`, `required_mode`, and `reason`. The runtime uses this to construct structured tool-result error messages that surface to the model in the next iteration. If you add new enforcement points that return errors, preserve this structure rather than returning opaque strings.

6. **`WorkspaceWrite` does not mean "all writes allowed."** `check_file_write()` explicitly gates writes to paths within `workspace_root`. This is distinct from `DangerFullAccess`, which permits writes anywhere. Understanding this distinction is important when reasoning about security boundaries in the permission model.

---

## Key Files

| File | Role |
|------|------|
| `rust/crates/tools/src/lib.rs` | `GlobalToolRegistry`, `execute_tool_with_enforcer()`, `classify_bash_permission()`, `classify_powershell_permission()` |
| `rust/crates/runtime/src/permissions.rs` | `PermissionMode`, `PermissionPolicy`, `PermissionOutcome`, `PermissionRequest`, `PermissionRule` |
| `rust/crates/runtime/src/permission_enforcer.rs` | `PermissionEnforcer`, `EnforcementResult`, `check_with_required_mode()`, `check_bash()`, `check_file_write()`, `is_within_workspace()`, `is_read_only_command()` |
| `rust/crates/rusty-claude-cli/tests/mock_parity_harness.rs` | `write_file_denied`, `bash_permission_prompt_approved`, `bash_permission_prompt_denied` harness scenarios |
