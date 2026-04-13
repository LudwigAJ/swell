# REPL Help, Autosave, and Session Browsing

The ClawCode REPL surfaces its session persistence, resume, and browsing affordances directly in the help output and key slash commands. This document explains how those surfaces work at the engineering level, what the builder can learn from the design, and which source files evidence each claim.

## What the REPL Help Surface Exposes

When a user runs `/help` inside the REPL, the rendered output goes beyond listing available slash commands. It includes a **session status header** that makes three things explicit:

- **Auto-save path** — every REPL turn is auto-saved to `.claw/sessions/<session-id>.jsonl`
- **Resume latest shortcut** — `/resume latest` jumps directly to the most recently saved session without needing to know its ID
- **Session browsing** — `/session list` enumerates all managed sessions in the workspace

The session status header that appears above the slash-command list is rendered in `LiveCli::render_session_status()` and contains these three affordances alongside the standard help inventory. The key code is in `rust/crates/rusty-claude-cli/src/main.rs` around the REPL help path:

```rust
// Auto-save line shown in the session status block above /help output:
"  Auto-save            .claw/sessions/<session-id>.jsonl".to_string(),
// Resume-latest affordance:
"  Resume latest        /resume latest".to_string(),
// Session browsing affordance:
"  Browse sessions      /session list".to_string(),
```

This is not buried in documentation — it appears in the interactive output the developer sees every time they run `/help`, which means session persistence is discoverable without reading any docs.

## Autosave Mechanism

Autosave is not a background daemon or a periodic flush task. Each completed REPL turn calls `session.append_turn()` synchronously before the next prompt is displayed. The session file uses the `jsonl` (newline-delimited JSON) format, one JSON object per line, written to `.claw/sessions/<session-id>.jsonl`.

The storage path is **workspace-local**, not home-global. The `SessionManager` in `runtime/src/session_control.rs` resolves the root as `<cwd>/.claw/sessions/<workspace-hash>/`. This means:

- Each workspace gets its own isolated session namespace
- Sessions are not mixed across projects
- A session file is a plain `.jsonl` that can be copied, exported, or committed

The autosave path for each turn is established in `LiveCli::run()` by calling `session.append_turn()` after streaming the assistant response. The turn content is the serialized `ConversationMessage` array, which includes all user/assistant/tool blocks, role annotations, and usage telemetry.

## Resume-Latest Affordance

`/resume latest` is a first-class alias, not a heuristic. When `latest` is resolved, `SessionManager::resolve_session_reference()` maps it to the most recently modified session file under `.claw/sessions/`. The `LATEST_SESSION_REFERENCE` constant is `"latest"` and `SESSION_REFERENCE_ALIASES` also includes `"last"` and `"recent"` as equivalent aliases.

The `resume_supported_slash_commands()` function in `crates/commands/src/lib.rs` explicitly marks commands that can run inside a resumed session. This is how the system knows which slash commands to permit when `--resume` is passed from the CLI. Commands marked `resume_supported: false` (like `/model`, `/permissions`, `/exit`) cannot be chained after `--resume`.

## Session Browsing

`/session list` is handled by `handle_slash_command("/session list", ...)` in the slash-command dispatch path. The handler calls `list_managed_sessions()` which reads the `.claw/sessions/` directory, parses each session file's header metadata, and renders a table with session ID, last-modified timestamp, and branch name if present.

```rust
// main.rs — session list rendering
let sessions = list_managed_sessions()?;
let text = render_session_list(&active_id)?;
println!("{}", text);
```

`render_session_list()` formats the output as a plain text table suitable for terminal display. The function also emits a JSON mode when `--output-format json` is active.

The `/session switch <session-id>` command forks the session state so the developer can branch without losing the original session. The `/session delete` command removes the session file from disk after confirmation, but refuses to delete the active session.

## Builder Lessons

### Lesson 1: Affordances Belong in Interactive Output, Not Just Docs

The session header makes autosave and resume discoverable without any documentation lookup. For a builder, the lesson is to put persistence affordances in the primary interface a developer already uses — the help command — rather than hiding them in a README section that only gets read once.

### Lesson 2: Aliases Reduce Friction for Common Operations

`/resume latest` (and its `last`/`recent` variants) means the developer never has to run `/session list` just to resume the most recent session. The constant `LATEST_SESSION_REFERENCE` is resolved in one step in `SessionManager::resolve_session_reference()`. This is a small quality-of-life win that compounds over many REPL sessions.

### Lesson 3: Session Browsing is a Plain-Text Surface

`/session list` renders a text table, not a proprietary format. This makes it scriptable: `claw --resume $(claw --session latest)` from a shell script works because the session ID is a simple string. The builder lesson is that managed state should be readable with standard tooling whenever possible.

### Lesson 4: Resume-Supported Commands are Explicitly Tagged

The `SlashCommandSpec.resume_supported` bool in `crates/commands/src/lib.rs` is the single source of truth for which commands can run in a resumed session. This makes it impossible to silently forget to handle resume for a new command — the spec table forces an explicit decision per command. Adding a new slash command requires choosing `resume_supported: true` or `false`, and tests in `resume_slash_commands.rs` validate that the behavior matches the spec.

## Evidence Files

| Claim | Evidence File |
|-------|--------------|
| Autosave to `.claw/sessions/<session-id>.jsonl` | `rust/crates/rusty-claude-cli/src/main.rs` — `render_session_list()` output strings |
| `/resume latest` shortcut | `rust/crates/rusty-claude-cli/src/main.rs` — `LATEST_SESSION_REFERENCE` constant and help output; `rust/crates/runtime/src/session_control.rs` — `resolve_session_reference()` |
| `/session list` browsing | `rust/crates/rusty-claude-cli/src/main.rs` — `handle_slash_command` dispatch for session; `render_session_list()` |
| Session workspace-local storage | `rust/crates/runtime/src/session_control.rs` — `SessionManager` and workspace-hash namespacing |
| Resume-supported per-command tagging | `rust/crates/commands/src/lib.rs` — `SlashCommandSpec` struct with `resume_supported` field; `resume_supported_slash_commands()` |
| Session list test coverage | `rust/crates/rusty-claude-cli/tests/resume_slash_commands.rs` — `resume_latest_restores_the_most_recent_managed_session` |
