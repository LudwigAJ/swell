# Session-Aware Completions

How the REPL completion system injects dynamic, session-grounded suggestions alongside static slash-command names.

## The Static Foundation

The ClawCode REPL starts its completion candidate list from a **static registry**: `slash_command_specs()` in `commands/src/lib.rs`. Every registered slash command — `/help`, `/status`, `/resume`, `/session`, `/compact`, `/model`, and 90+ others — appears in completions unconditionally, filtered only by the `STUB_COMMANDS` list that excludes unimplemented stubs.

This static layer is deterministic and cheap. The REPL builds it once per session and refreshes it only when the editor signals a new input line.

## The Dynamic Layer: Session-Aware Completions

The more interesting behavior lives in `slash_command_completion_candidates_with_sessions` in `rusty-claude-cli/src/main.rs`. After seeding completions from the static spec table, this function **injects session-specific suggestions** that would be impossible to enumerate statically:

```rust
// Active session gets direct /resume and /session switch entries
if let Some(active_session_id) = active_session_id.filter(|value| !value.trim().is_empty()) {
    completions.insert(format!("/resume {active_session_id}"));
    completions.insert(format!("/session switch {active_session_id}"));
}

// Recent managed sessions (up to 10) also become completion targets
for session_id in recent_session_ids
    .into_iter()
    .filter(|value| !value.trim().is_empty())
    .take(10)
{
    completions.insert(format!("/resume {session_id}"));
    completions.insert(format!("/session switch {session_id}"));
}
```

### What gets injected

| Source | Completions added |
|--------|-------------------|
| Active session (`self.session.id`) | `/resume <id>`, `/session switch <id>` |
| Managed session list (up to 10) | `/resume <id>`, `/session switch <id>` each |
| Model alias resolution | `/model <resolved>`, `/model <current>` |

The active session completions let a user who is mid-session immediately resume **that same session** or **switch away from it** without needing to recall or paste the session ID manually.

The managed-session completions let a user exploring recent sessions tab-complete directly to any stored session they have previously created in this workspace.

### How the session list is retrieved

The completions pipeline calls `list_managed_sessions()` (defined in `rusty-claude-cli/src/main.rs` as a thin wrapper around `runtime::session_control::list_managed_sessions()`). This reads `.claw/sessions/` in the current workspace and returns summaries ordered by the session store's recency semantics — not merely filesystem mtime, but the `updated_at` field that tracks semantic recency after compaction and fork operations.

## Wiring into the REPL loop

The completion pipeline is wired in `LiveCli::run_repl`:

```rust
let mut editor = input::LineEditor::new("> ", cli.repl_completion_candidates().unwrap_or_default());
loop {
    editor.set_completions(cli.repl_completion_candidates().unwrap_or_default());
    match editor.read_line()? {
        input::ReadOutcome::Submit(input) => { /* ... */ }
    }
}
```

`repl_completion_candidates()` calls `slash_command_completion_candidates_with_sessions` on every iteration, meaning completions are **fresh on every input line** — not snapshot from session start. If a new managed session appears (via fork or external session creation), it will be available as a completion target before the next input line.

## Why this matters for builders

A naive completion system would list only the static slash-command names. That is sufficient for discoverability of commands but misses the most context-sensitive operations in the CLI — **session switching and resumption** — which are inherently dynamic and tied to the user's actual session state.

The session-aware layer demonstrates a pattern for contextual completion: enumerate static candidates once, then layer in dynamic candidates that require live queries. The `BTreeSet<String>` return type naturally deduplicates the merged set without extra logic.

The test `completion_candidates_include_workflow_shortcuts_and_dynamic_sessions` verifies that when `slash_command_completion_candidates_with_sessions` is called with `active_session_id = "session-current"` and `recent_session_ids = ["session-old"]`, both `/session switch session-current` and `/resume session-old` appear in the candidate set alongside the static slash commands like `/model` and `/permissions`.

## Boundary: completions vs. session discovery

Session-aware completions handle **completion** — tab-triggered suggestion of valid token sequences. They are not the session discovery surface; that is `/session list`, which renders a table of all managed sessions. Completions supplement that command by making frequent operations (resume, switch) directly accessible without a separate command round-trip.

## Evidence

- `references/claw-code/rust/crates/rusty-claude-cli/src/main.rs` — `slash_command_completion_candidates_with_sessions`, `repl_completion_candidates`, `LiveCli::run_repl`
- `references/claw-code/rust/crates/rusty-claude-cli/src/main.rs` — `list_managed_sessions` thin wrapper
- `references/claw-code/rust/crates/runtime/src/session_control.rs` — `list_managed_sessions` (workspace-local session store)
- `references/claw-code/rust/crates/commands/src/lib.rs` — `slash_command_specs`, `STUB_COMMANDS`
- Test: `completion_candidates_include_workflow_shortcuts_and_dynamic_sessions`
