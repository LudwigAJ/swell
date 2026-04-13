# Read-Only Bash Heuristics

## Overview

ClawCode's permission system uses heuristic command classification to gate bash tool execution in read-only mode. Rather than requiring an explicit prompter for every mutating command, the system maintains a conservative allowlist of known-safe read-only commands and denies everything else in `ReadOnly` mode. This document explains how the classification works, its boundaries, and what builders can learn from the design.

**Canonical sources:**
- `references/claw-code/rust/crates/runtime/src/permission_enforcer.rs` — the on-main enforcement layer; primary `check_bash()` implementation using `is_read_only_command()` heuristic
- `references/claw-code/rust/crates/runtime/src/bash_validation.rs` — merged on `main` via Lane 1 (merge commit `1cfd78a`); richer validation submodules available but not the primary enforcement path on `main`
- `references/claw-code/rust/crates/runtime/src/permissions.rs` — PermissionPolicy and mode ladder
- `references/claw-code/rust/crates/rusty-claude-cli/tests/mock_parity_harness.rs` — harness evidence for prompt approve/deny flows
- `references/claw-code/PARITY.md` — parity notes on bash validation lane

---

## The Permission Mode Ladder

Bash execution is governed by a mode ladder defined in `permissions.rs`:

```
Allow > DangerFullAccess > WorkspaceWrite > ReadOnly > Prompt
```

The `PermissionMode` enum carries ordinal values so the policy can compare active mode against required mode via `>=` comparison.

- **`Allow`** — no enforcement
- **`DangerFullAccess`** — unrestricted bash (arbitrary command execution)
- **`WorkspaceWrite`** — restricted to workspace paths; prompts for elevated operations
- **`ReadOnly`** — conservative read-only heuristic; denies mutating commands
- **`Prompt`** — every bash invocation requires interactive confirmation

Evidence: `PermissionMode` ordinals and the `active_mode()` accessor in `permission_enforcer.rs`.

---

## The On-Main Heuristic: `is_read_only_command`

`permission_enforcer.rs` ships a lightweight heuristic function that the main branch uses for read-only bash gating:

```rust
fn is_read_only_command(command: &str) -> bool {
    let first_token = command
        .split_whitespace()
        .next()
        .unwrap_or("")
        .rsplit('/')
        .next()
        .unwrap_or("");

    matches!(
        first_token,
        "cat" | "head" | "tail" | "less" | "more" | "wc" | "ls" | "find"
            | "grep" | "rg" | "awk" | "sed" | "echo" | "printf" | "which"
            | "where" | "whoami" | "pwd" | "env" | "printenv" | "date"
            | "cal" | "df" | "du" | "free" | "uptime" | "uname" | "file"
            | "stat" | "diff" | "sort" | "uniq" | "tr" | "cut" | "paste"
            | "tee" | "xargs" | "test" | "true" | "false" | "type"
            | "readlink" | "realpath" | "basename" | "dirname" | "sha256sum"
            | "md5sum" | "b3sum" | "xxd" | "hexdump" | "od" | "strings"
            | "tree" | "jq" | "yq" | "python3" | "python" | "node"
            | "ruby" | "cargo" | "rustc" | "git" | "gh"
    ) && !command.contains("-i ")
        && !command.contains("--in-place")
        && !command.contains(" > ")
        && !command.contains(" >> ")
}
```

**Design principle: conservative denial.** Any command not explicitly on the allowlist — or containing write redirections (`>`, `>>`) or in-place flags (`-i`, `--in-place`) — is denied in `ReadOnly` mode.

The function handles:
1. **Path stripping** — `/usr/bin/cat` normalizes to `cat`
2. **In-place flags** — `sed -i` or `python -i` are blocked even though the base commands are allowed
3. **Write redirections** — `echo test > file.txt` is blocked

Evidence: tests in `permission_enforcer.rs` confirming `cat`, `grep`, `ls` pass and `rm`, `echo >`, `sed -i` are denied.

---

## The Merged Validation Pipeline

`bash_validation.rs` landed on `main` via Lane 1 (merge commit `1cfd78a`, feature commit `36dac6c`). The module ports the upstream TypeScript validation pipeline and provides richer classification than the lightweight heuristic. Despite being merged, the primary on-main enforcement path for `check_bash()` still routes through `permission_enforcer.rs` and `is_read_only_command()` rather than calling `bash_validation.rs` directly — the validation submodules are available but constitute a secondary surface on `main`.

### Validation Submodules

| Submodule | Purpose |
|-----------|---------|
| `readOnlyValidation` | Block write commands, state-modifying commands, write redirections, sudo-wrapped mutators, and non-read-only git subcommands |
| `destructiveCommandWarning` | Warn on `rm -rf /`, `shred`, fork bombs, and similar catastrophic patterns |
| `modeValidation` | Enforce permission mode constraints; warn in `WorkspaceWrite` for system-path targets |
| `sedValidation` | Block `sed -i` in read-only mode |
| `pathValidation` | Warn on `../` traversal and home-directory references outside workspace |
| `commandSemantics` | Classify intent: `ReadOnly`, `Write`, `Destructive`, `Network`, `ProcessManagement`, `PackageManagement`, `SystemAdmin` |

Evidence: `bash_validation.rs` module-level doc and the `CommandIntent` enum definition.

### Read-Only Validation Details

The `validate_read_only` function checks:

1. **Write command blocklist** — `cp`, `mv`, `rm`, `mkdir`, `touch`, `chmod`, `chown`, `ln`, `tee`, `truncate`, `shred`, `dd`, etc.
2. **State-modifying command blocklist** — `apt`, `npm`, `pip`, `cargo`, `docker`, `systemctl`, `kill`, `reboot`, etc.
3. **Redirection detection** — `>`, `>>`, `>&`
4. **Sudo unwrapping** — `sudo rm -rf /tmp/x` is checked by recursively validating the inner command
5. **Git subcommand allowlist** — `git status`, `log`, `diff`, `show`, `branch`, `tag`, `stash`, `fetch`, `ls-files`, `config`, etc. are allowed; `push`, `commit`, `merge`, `rebase`, etc. are blocked

Evidence: `WRITE_COMMANDS`, `STATE_MODIFYING_COMMANDS`, `WRITE_REDIRECTIONS`, `GIT_READ_ONLY_SUBCOMMANDS` constants and corresponding tests in `bash_validation.rs`.

### `CommandIntent` Classification

The `classify_command` function produces a `CommandIntent` enum:

```rust
pub enum CommandIntent {
    ReadOnly,          // ls, cat, grep, find, etc.
    Write,             // cp, mv, mkdir, tee, etc.
    Destructive,       // rm, shred, wipefs
    Network,           // curl, wget, ssh, nc, etc.
    ProcessManagement, // kill, pkill, ps, top, etc.
    PackageManagement, // apt, npm, pip, cargo, etc.
    SystemAdmin,       // sudo, chmod, mount, systemctl, etc.
    Unknown,
}
```

**Builder lesson:** The intent classification enables future per-intent policy decisions. Rather than a binary read/write split, the system models multiple behavioral categories that can each carry their own permission requirements.

---

## `check_bash`: The Enforcement Entry Point

`PermissionEnforcer::check_bash()` in `permission_enforcer.rs` is the gate:

```rust
pub fn check_bash(&self, command: &str) -> EnforcementResult {
    let mode = self.policy.active_mode();

    match mode {
        PermissionMode::ReadOnly => {
            if is_read_only_command(command) {
                EnforcementResult::Allowed
            } else {
                EnforcementResult::Denied {
                    tool: "bash".to_owned(),
                    active_mode: mode.as_str().to_owned(),
                    required_mode: PermissionMode::WorkspaceWrite.as_str().to_owned(),
                    reason: format!(
                        "command may modify state; not allowed in '{}' mode",
                        mode.as_str()
                    ),
                }
            }
        }
        PermissionMode::Prompt => EnforcementResult::Denied {
            tool: "bash".to_owned(),
            active_mode: mode.as_str().to_owned(),
            required_mode: PermissionMode::DangerFullAccess.as_str().to_owned(),
            reason: "bash requires confirmation in prompt mode".to_owned(),
        },
        // WorkspaceWrite, Allow, DangerFullAccess: permit bash
        _ => EnforcementResult::Allowed,
    }
}
```

**Three behaviors by mode:**
- `ReadOnly` — heuristic allowlist check; denies anything not recognized as read-only
- `Prompt` — unconditional denial; requires interactive prompter flow (see Prompt-Mode Caveat)
- `WorkspaceWrite`, `Allow`, `DangerFullAccess` — allow

Evidence: test cases `read_only_allows_read_commands`, `read_only_denies_write_commands`, `prompt_mode_denies_without_prompter`, `danger_full_access_permits_file_writes_and_bash` in `permission_enforcer.rs`.

---

## Prompt-Mode Caveat

`Prompt` mode does not auto-execute any bash command. Instead, it always returns `EnforcementResult::Denied` with a structured payload:

```rust
EnforcementResult::Denied {
    tool: "bash".to_owned(),
    active_mode: "prompt".to_owned(),
    required_mode: "danger-full-access".to_owned(),
    reason: "bash requires confirmation in prompt mode".to_owned(),
}
```

The denial payload carries:
- `tool` — always `"bash"` for bash denials
- `active_mode` — the current session mode (`"prompt"`)
- `required_mode` — what mode would be needed to auto-execute (`"danger-full-access"`)
- `reason` — human-readable explanation

**The interactive flow** is handled by the caller (typically the conversation runtime) which:
1. Receives the `Denied` result
2. Renders an approval prompt to the user
3. Passes a prompter to `PermissionPolicy::authorize()` on retry

Evidence: harness scenarios `bash_permission_prompt_approved` and `bash_permission_prompt_denied` in `mock_parity_harness.rs` confirm the full approve/deny flow with stdin injection.

---

## Harness Evidence

The mock parity harness (`mock_parity_harness.rs`) scripts two bash permission scenarios:

### `bash_permission_prompt_approved`
- Mode: `workspace-write`
- stdin: `"y\n"` (user approves)
- Asserts: `run.stdout` contains `"Permission approval required"` and `"Approve this tool call? [y/N]:"`
- Asserts: `tool_results[0].is_error == false` and output contains `"approved via prompt"`

### `bash_permission_prompt_denied`
- Mode: `workspace-write`
- stdin: `"n\n"` (user denies)
- Asserts: `tool_results[0].is_error == true` and output contains `"denied by user approval prompt"`

**Builder lesson:** The harness proves that prompt-mode enforcement is a first-class behavioral contract — not a soft warning or comment — backed by a structured denial payload that the runtime can surface to users.

Evidence: `assert_bash_permission_prompt_approved` and `assert_bash_permission_prompt_denied` in `mock_parity_harness.rs`.

---

## Parity Notes

`PARITY.md` records the bash validation lane:

> **Lane 1 — Bash validation**
> - Feature commit `36dac6c` adds `bash_validation.rs` with 6 validation submodules
> - Merge commit `1cfd78a` lands the module on `main`
> - On `main`, `PermissionEnforcer::check_bash()` routes through `permission_enforcer.rs` and `is_read_only_command()` as the primary enforcement; `bash_validation.rs` is available but not the primary on-main path

This means:
- **On main:** lightweight heuristic via `is_read_only_command()` is the primary enforcement; the richer `bash_validation.rs` submodules are merged but not called from the primary `check_bash()` path
- **Both paths are on `main`** — `bash_validation.rs` is no longer branch-only; the distinction is architectural rather than branch-vs-main

Builders referencing the design should note which surface they are examining. The primary path is simpler but less comprehensive; `bash_validation.rs` models the full upstream TypeScript validation matrix and is available for deeper inspection or future primary-path adoption.

Evidence: `PARITY.md` Lane 1 with merge commit `1cfd78a` and "Bash tool — 9/9 requested validation submodules complete" checklist.

---

## Builder Lessons

### Conservative denial is the safe default

The read-only heuristic denies anything not explicitly allowlisted. This is the correct security posture: false positives (read commands accidentally denied) are fixable by adding them to the allowlist; false negatives (mutating commands accidentally allowed) are silent data hazards.

### First-token extraction is fragile for complex commands

`is_read_only_command` extracts only the first token. Commands like `python -c "import os; os.system('rm -rf /')"` pass through because `python` is not on the blocklist. The merged `bash_validation.rs` addresses this with pattern-based destructive command detection (`DESTRUCTIVE_PATTERNS`).

**Builder takeaway:** First-token heuristics are a useful fast path, not a complete solution. For production systems, consider deeper command inspection or sandboxing.

### Mode comparison via ordinals is ergonomic but ordinal assumptions must hold

`current_mode >= required_mode` works because `PermissionMode` variants are given sequential ordinal values. If a new mode is inserted in the middle, the comparison semantics break. Keep the ordinal contract explicit in comments and tests.

### Prompt-mode denial is not silent failure

The `EnforcementResult::Denied` payload is structured so callers can render actionable UI. Denials are not dropped or logged-and-ignored; they carry `tool`, `active_mode`, `required_mode`, and `reason` fields that downstream code can present to users or log for audit.

Evidence: `EnforcementResult` enum definition in `permission_enforcer.rs`.

### Harness evidence disciplines behavioral claims

The `bash_permission_prompt_approved` and `bash_permission_prompt_denied` harness scenarios document the exact expected behavior (stdout content, response structure, error flag) as executable assertions. This prevents behavioral drift as the codebase evolves.

---

## Summary

ClawCode's read-only bash heuristics use a two-tier approach:

1. **Primary on-main enforcement** — lightweight `is_read_only_command()` in `permission_enforcer.rs`: first-token allowlist with write-redirection and in-place flag detection, consumed by `PermissionEnforcer::check_bash()` as the main gating function
2. **Merged validation module** — `bash_validation.rs` landed on `main` (Lane 1, merge commit `1cfd78a`) with 6 submodules: `readOnlyValidation`, `destructiveCommandWarning`, `modeValidation`, `sedValidation`, `pathValidation`, `commandSemantics`; `validate_command()` and `classify_command()` are available but not the primary `check_bash()` path on `main`

Both paths feed into `PermissionEnforcer::check_bash()` which returns structured `EnforcementResult` values consumed by the runtime's conversation loop. Prompt mode always denies bash execution and defers to an interactive approval flow confirmed by harness evidence.

Test-backed claims use these function names:
- `is_read_only_command()` heuristic tests: `read_only_command_heuristic()`, `read_only_allows_read_commands()`, `read_only_denies_write_commands()` in `permission_enforcer.rs`
- `check_bash()` integration tests: `read_only_denies_writes()`, `prompt_mode_denies_without_prompter()`, `danger_full_access_permits_file_writes_and_bash()`, `prompt_mode_check_bash_denied_payload_fields()` in `permission_enforcer.rs`
- `bash_validation.rs` tests: `pipeline_blocks_write_in_read_only()`, `pipeline_warns_destructive_in_write_mode()`, `pipeline_allows_safe_read_in_read_only()`, `blocks_sed_inplace_in_read_only()`, `classifies_read_only_commands()`, `classifies_git_push_as_write()` in `bash_validation.rs`
