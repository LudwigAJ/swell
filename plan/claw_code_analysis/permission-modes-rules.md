# Permission Modes and Rules — Builder Analysis

> **Evidence base:** `references/claw-code/rust/crates/runtime/src/permissions.rs`, `references/claw-code/rust/crates/runtime/src/permission_enforcer.rs`, `references/claw-code/rust/crates/tools/src/lib.rs`, `references/claw-code/rust/README.md`, `references/claw-code/PARITY.md`

## What This Document Covers

This document explains how ClawCode's Rust runtime evaluates whether a tool may execute, focusing on:
- The five permission modes and what each mode authorizes
- Per-tool required permission levels and how they interact with the active mode
- The three static rule layers (allow, deny, ask) and how they are evaluated
- The critical distinction between **static rule-based prompting** (ask rules) and **escalation-triggering prompting** (mode gap)
- Hook-provided override guidance and its interaction with the static rule engine

This document does not cover file-operation guardrails (canonical paths, symlink escapes, binary detection, size limits) — those are addressed in a separate analysis document. Nor does it cover workspace-write boundary enforcement in detail.

## Ownership

The permission system lives in the `runtime` crate:
- `crates/runtime/src/permissions.rs` — `PermissionMode`, `PermissionPolicy`, rule evaluation, prompting, and authorization outcomes
- `crates/runtime/src/permission_enforcer.rs` — `PermissionEnforcer`, workspace boundary checks, and bash command classification
- `crates/tools/src/lib.rs` — `ToolSpec` with `required_permission` and `execute_tool_with_enforcer` dispatch

## Permission Modes

`PermissionMode` is a copy-type enum with five variants ordered from least to most privileged:

```rust
// crates/runtime/src/permissions.rs
pub enum PermissionMode {
    ReadOnly,       // lowest — read-only file access and read-only bash only
    WorkspaceWrite, // writes inside workspace root; no bash mutating commands
    DangerFullAccess, // all tool access including bash; no workspace boundary enforcement
    Prompt,         // interactive confirmation required before any privileged operation
    Allow,          // highest — bypasses all policy checks (used for testing or hooks)
}
```

The ordering `ReadOnly < WorkspaceWrite < DangerFullAccess < Prompt < Allow` is intentional — `Ord` is derived so the policy can compare `active_mode >= required_mode` to determine whether a tool can execute without escalation.

### Mode-by-mode behavior

| Mode | File reads | File writes | Bash read-only | Bash mutating | Interactive prompt |
|------|-----------|-------------|---------------|---------------|-------------------|
| `ReadOnly` | ✅ | ❌ denied | ✅ (heuristic) | ❌ denied | n/a |
| `WorkspaceWrite` | ✅ | ✅ inside workspace | ✅ | ✅ | prompt for danger-* |
| `DangerFullAccess` | ✅ | ✅ any path | ✅ any command | ✅ | prompt for danger-* |
| `Prompt` | ❌ denied without prompter | ❌ denied without prompter | ❌ denied without prompter | ❌ denied without prompter | required |
| `Allow` | ✅ always | ✅ always | ✅ always | ✅ always | never required |

**Builder lesson:** The `Prompt` mode is a guard that requires a `PermissionPrompter` to be registered. When `PermissionEnforcer::check()` is called without a prompter and the active mode is `Prompt`, it auto-denies. This means `Prompt` mode only works in an interactive REPL flow where the runtime has a live prompter.

## Per-Tool Required Permission

Each built-in tool declares a minimum permission level it requires via `required_permission: PermissionMode` in its `ToolSpec`. The tool specs are defined in `mvp_tool_specs()` in `crates/tools/src/lib.rs`:

```rust
// crates/tools/src/lib.rs — representative examples from mvp_tool_specs()
ToolSpec {
    name: "read_file",
    required_permission: PermissionMode::ReadOnly,
    // ...
},
ToolSpec {
    name: "write_file",
    required_permission: PermissionMode::WorkspaceWrite,
    // ...
},
ToolSpec {
    name: "bash",
    required_permission: PermissionMode::DangerFullAccess,
    // ...
},
```

The `PermissionPolicy::required_mode_for(tool_name)` method looks up the per-tool requirement in a `BTreeMap<String, PermissionMode>` and falls back to `DangerFullAccess` if no explicit requirement is registered. This fallback design means any unregistered tool defaults to the most restrictive auto-deny unless explicitly added with a lower requirement.

### Dynamic classification for shell-like tools

Bash and PowerShell are classified dynamically: the actual required permission depends on the command string, not just the tool name. The `classify_bash_permission()` function in `permission_enforcer.rs` derives `PermissionMode::ReadOnly` for commands like `cat`, `grep`, `git status`, and `ls`, while any command containing `>`, `>>`, `-i`, or `--in-place` is denied even in `ReadOnly` mode. This means the same `bash` tool can pass in `ReadOnly` for a read-only command but be denied for a mutating one.

## The Three Static Rule Layers

`PermissionPolicy` holds three rule vectors populated from `RuntimePermissionRuleConfig`:

```rust
// crates/runtime/src/permissions.rs
pub struct PermissionPolicy {
    active_mode: PermissionMode,
    tool_requirements: BTreeMap<String, PermissionMode>,
    allow_rules: Vec<PermissionRule>,
    deny_rules: Vec<PermissionRule>,
    ask_rules: Vec<PermissionRule>,
}
```

Each `PermissionRule` is parsed from a config string with the form `tool_name(subject)` or `tool_name`:
- `tool_name` alone — matches any input for that tool (`Any` matcher)
- `tool_name(path/to/file)` — matches only when the tool input contains `path/to/file` as the extracted permission subject (`Exact` matcher)
- `tool_name(path/to:*)` — matches when the subject starts with `path/to/` (`Prefix` matcher)

Permission subjects are extracted from tool input JSON by looking up keys like `command`, `path`, `file_path`, `url`, `pattern`, and `code`.

### Rule evaluation order

Deny rules are evaluated **first** and always short-circuit to a denial result:

```rust
// crates/runtime/src/permissions.rs — authorize_with_context
if let Some(rule) = Self::find_matching_rule(&self.deny_rules, tool_name, input) {
    return PermissionOutcome::Deny {
        reason: format!(
            "Permission to use {tool_name} has been denied by rule '{}'",
            rule.raw
        ),
    };
}
```

If no deny rule matches, the authorization proceeds through the allow, ask, and mode-comparison logic.

## Static Rules vs Escalation-Triggering: The Key Distinction

This is the most important conceptual split in the permission system, and it is required by VAL-SAFETY-012.

### Static ask rules

Ask rules (`ask_rules`) are **static, declarative rules** configured ahead of time. When a deny rule does not fire and the `PermissionContext` has no hook override, the authorization algorithm checks whether an ask rule matches the tool and input. If one does, it **immediately prompts the user**, even if the active mode would formally permit the operation:

```rust
// crates/runtime/src/permissions.rs — ask rule forces prompt even in DangerFullAccess
if let Some(rule) = ask_rule {
    let reason = format!(
        "tool '{tool_name}' requires approval due to ask rule '{}'",
        rule.raw
    );
    return Self::prompt_or_deny(
        tool_name,
        input,
        current_mode,
        required_mode,
        Some(reason),
        prompter,
    );
}
```

This is the pattern used when, for example, you want to force confirmation for all `bash(git:*)` invocations regardless of the session's active mode. The ask rule creates a **mandatory prompt gate** that runs even in `DangerFullAccess` mode.

Test evidence from `permissions.rs`:
```rust
#[test]
fn ask_rules_force_prompt_even_when_mode_allows() {
    let rules = RuntimePermissionRuleConfig::new(
        Vec::new(),
        Vec::new(),
        vec!["bash(git:*)".to_string()], // ask rule
    );
    let policy = PermissionPolicy::new(PermissionMode::DangerFullAccess)
        .with_tool_requirement("bash", PermissionMode::DangerFullAccess);
    // Active mode is DangerFullAccess, which formally >= required_mode.
    // But the ask rule fires and a prompt is raised.
}
```

### Mode-escalation prompting

Escalation-triggering prompting is **not rule-based** — it fires when the active mode is formally insufficient for the tool's required permission level, and the mode itself is `Prompt` or the gap is between `WorkspaceWrite` and `DangerFullAccess`:

```rust
// crates/runtime/src/permissions.rs — escalation path
if current_mode == PermissionMode::Prompt
    || (current_mode == PermissionMode::WorkspaceWrite
        && required_mode == PermissionMode::DangerFullAccess)
{
    let reason = Some(format!(
        "tool '{tool_name}' requires approval to escalate from {} to {}",
        current_mode.as_str(),
        required_mode.as_str()
    ));
    return Self::prompt_or_deny(
        tool_name,
        input,
        current_mode,
        required_mode,
        reason,
        prompter,
    );
}
```

Key differences:

| Dimension | Ask rule (static) | Escalation (dynamic) |
|-----------|-----------------|---------------------|
| Trigger condition | Matches tool + subject in config | Active mode < required mode AND (mode==Prompt OR gap is WS→DangerFullAccess) |
| Always prompts even in high modes | Yes (DangerFullAccess still prompts) | No (only triggers when mode is insufficient) |
| Configured by | `RuntimePermissionRuleConfig::ask` | Session active mode + per-tool `required_permission` |
| Reason field | Named after the rule (`ask rule 'bash(git:*)'`) | Named after the mode gap (`escalate from workspace-write to danger-full-access`) |

Both paths produce the same `PermissionRequest` payload structure (tool_name, input, current_mode, required_mode, reason), but the **reason encoding differs** and the **triggering logic is entirely separate**.

## Hook Overrides and Their Interaction with Static Rules

`PermissionContext` carries an optional `PermissionOverride` set by higher-level orchestration (e.g., pre-tool hooks):

```rust
// crates/runtime/src/permissions.rs
pub enum PermissionOverride {
    Allow,
    Deny,
    Ask,
}
```

Hook overrides are evaluated **before** the static rule engine:

```rust
// crates/runtime/src/permissions.rs — hook override short-circuits
match context.override_decision() {
    Some(PermissionOverride::Deny) => {
        return PermissionOutcome::Deny { reason: ... };
    }
    Some(PermissionOverride::Ask) => {
        // Forces a prompt with the hook's reason
        return Self::prompt_or_deny(...);
    }
    Some(PermissionOverride::Allow) => {
        // Hook says allow — but ask rules still fire
        if let Some(rule) = ask_rule {
            // Ask rules still enforce even with hook Allow
            return Self::prompt_or_deny(...);
        }
        // Otherwise allow
    }
    None => { /* normal static rule evaluation */ }
}
```

Critical subtlety: **`PermissionOverride::Allow` does not bypass ask rules**. The test `hook_allow_still_respects_ask_rules` in `permissions.rs` verifies this — a hook-approved session still triggers prompts for matching ask rules. This ensures that even when a hook has pre-approved a session, the static rule layer can still enforce situational gates.

Hook `Deny` short-circuits everything — no static rules, no escalation check, no mode comparison. Hook `Ask` bypasses static allow and mode checks but preserves ask rule enforcement.

## The PermissionRequest Payload

When prompting is required (either via ask rule or escalation), the runtime constructs a `PermissionRequest`:

```rust
// crates/runtime/src/permissions.rs
pub struct PermissionRequest {
    pub tool_name: String,
    pub input: String,
    pub current_mode: PermissionMode,
    pub required_mode: PermissionMode,
    pub reason: Option<String>,
}
```

The `reason` field carries the triggering explanation — either the matched ask rule's identifier or the mode-gap description — so the prompter can explain to the user why confirmation is needed.

## PermissionEnforcer: Adding Boundary Checks on Top of Policy

`PermissionEnforcer` wraps `PermissionPolicy` and adds two enforcement surfaces that go beyond what `PermissionPolicy` evaluates:

1. **Workspace file-write boundary** (`check_file_write`) — in `WorkspaceWrite` mode, writes are allowed only when the resolved path is under the workspace root; in `ReadOnly`, all writes are denied; in `Prompt`, writes are auto-denied without a prompter.
2. **Bash read-only heuristic** (`check_bash`) — in `ReadOnly` mode, `bash` is permitted only when `is_read_only_command()` classifies the command; all other bash commands are denied.

These checks are not part of `PermissionPolicy`'s authorization algorithm — they are additional gating surfaces that the tool dispatch path applies via `enforce_permission_check()` and `maybe_enforce_permission_check_with_mode()` in `crates/tools/src/lib.rs`.

## Config Wiring

`RuntimePermissionRuleConfig` is populated from the merged runtime config file under `permissions.allow`, `permissions.deny`, and `permissions.ask` arrays. The config is loaded via `ConfigLoader` and merged according to the standard precedence chain (user → project → local override). The `PermissionPolicy::with_permission_rules()` method parses raw string rules into `PermissionRule` structs with parsed matchers.

## Parity Harness Evidence

The permission system is validated by three scenarios in the mock parity harness (`rust/crates/rusty-claude-cli/tests/mock_parity_harness.rs`):
- `write_file_denied` — validates that `PermissionEnforcer::check_file_write()` denies outside-workspace writes in `WorkspaceWrite` mode
- `bash_permission_prompt_approved` — validates the escalation prompt path when a tool requires `DangerFullAccess` in `WorkspaceWrite` mode
- `bash_permission_prompt_denied` — validates that denial is returned when the prompter declines

PARITY.md Lane 9 (Permission enforcement) confirms: *"Harness scenarios validate `write_file_denied`, `bash_permission_prompt_approved`, and `bash_permission_prompt_denied`."*
