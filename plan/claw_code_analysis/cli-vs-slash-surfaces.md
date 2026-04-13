# CLI vs Slash Command Surfaces

## Overview

Claw Code exposes two mechanically distinct entry-point surfaces for essentially the same set of commands: **top-level CLI subcommands** (`claw status`, `claw mcp`, `claw skills`) and **REPL slash commands** (`/status`, `/mcp`, `/skills`). The two surfaces coexist intentionally. Conflating them is a common source of confusion about when each form is appropriate and why both exist.

## Why Two Surfaces Coexist

The primary design driver is **entry-path ergonomics for different usage contexts**.

**Top-level CLI subcommands** are designed for:

- **One-shot, non-interactive use** — scripts, CI pipelines, and automation that want a single answer and exit
- **Machine-readable output** — `--output-format json` is natural at the CLI level
- **No session required** — commands like `claw status` or `claw mcp list` run without loading a session file

**REPL slash commands** are designed for:

- **Interactive, session-backed use** — commands that benefit from conversation context, session history, and auto-save
- **Tab-completion with session awareness** — completion injects `/resume <session-id>` and `/session switch <session-id>` from active or recent sessions rather than listing only static slash names
- **Rich session metadata** — `/session list`, `/resume latest`, and the autosave surface (`./claw/sessions/<session-id>.jsonl`) are REPL-native session affordances
- **Post-session inspection** — after a REPL session ends, `claw --resume latest /status` lets you resume a saved session and run slash commands against it

The two surfaces are not aliases for each other. They have independent parsing, independent help rendering, and different execution environments.

## Evidence: Separate Registries and Code Paths

The canonical evidence for this distinction lives in three files:

- `references/claw-code/rust/crates/commands/src/lib.rs` — defines `SlashCommandSpec` and `SLASH_COMMAND_SPECS`, the shared slash-command registry
- `references/claw-code/rust/crates/rusty-claude-cli/src/main.rs` — handles CLI argument parsing, top-level subcommand dispatch, REPL startup, session-aware completion, and session auto-save
- `references/claw-code/rust/README.md` — explicitly lists both surfaces as separate rows in the feature table

### Slash Command Spec Registry

`SlashCommandSpec` in `commands/src/lib.rs` is the canonical registry for REPL slash commands. Each spec carries:

```rust
pub struct SlashCommandSpec {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub summary: &'static str,
    pub argument_hint: Option<&'static str>,
    pub resume_supported: bool,
}
```

The `resume_supported` field is specifically about slash commands — it annotates whether a given slash command can be replayed against a resumed session via `claw --resume SESSION.jsonl /command`. Commands marked `resume_supported: false` (e.g., `/model`, `/permissions`, `/session`) are excluded from `--resume` replay mode.

Notably, **the spec registry does not describe top-level CLI subcommands** — those are handled separately in `main.rs`.

### Top-Level CLI Subcommands

Top-level subcommands are dispatched via `CliAction` parsing in `main.rs`. The `--help` output for these commands is rendered by `print_help_to()` and describes each command with the `claw <command>` form. The test `init_help_mentions_direct_subcommand` confirms that `claw help` mentions `claw status`, `claw sandbox`, `claw mcp`, and similar forms, but does **not** mention `/status`, `/sandbox`, or `/mcp` — intentionally keeping the two surfaces separate in help text.

### REPL Slash Commands

The REPL is initialized in `main.rs` via `LiveCli::new()` which sets up a rustyline editor with `repl_completion_candidates()`. The completions come from `slash_command_completion_candidates_with_sessions()`, which:

1. Iterates `SLASH_COMMAND_SPECS` (the spec registry)
2. Filters out `STUB_COMMANDS` — spec entries that are not yet implemented
3. Dynamically injects session-aware completions: `/resume <active-session-id>` and `/session switch <session-id>` for the active session, plus `/resume <session-id>` and `/session switch <session-id>` for each of the 10 most recent managed sessions

This dynamic session injection is the completion mechanism that VAL-CLI-004 calls out: completion "injects `/resume <session-id>` and `/session switch <session-id>` suggestions from recent or active sessions instead of listing only static slash names."

### Interactive-Only Command Guard

When a user tries to invoke a slash command as a direct CLI subcommand, `main.rs` intercepts this with a specific error message:

```
slash command <name> is interactive-only. Start `claw` and run it there,
or use `claw --resume SESSION.jsonl <name>` when the command is marked [resume] in /help.
```

This error message only appears for commands that are **not** reachable as top-level CLI subcommands — it is the guard that enforces the surface separation at runtime.

### Session Auto-Save

REPL turns auto-save to `.claw/sessions/<session-id>.jsonl`. This is exposed in the REPL help:

```
Auto-save            .claw/sessions/<session-id>.jsonl
Resume latest        /resume latest
Browse sessions      /session list
```

This autosave surface does not exist for top-level CLI subcommands — they are one-shot by design.

## Design Lesson: Surface Parity vs. Surface Identity

A subtle but important distinction: having `claw mcp` **and** `/mcp` does not mean they are the same command. They share a name and often similar output, but they:

- Parse arguments differently (`claw mcp show <server>` vs. `/mcp show <server>`)
- Run in different environments (CLI process vs. REPL session)
- Have different session dependencies
- Are dispatched by different code paths in `main.rs`

The builder lesson is to treat these as **surface parity**, not **surface identity**. When adding a new command, the question of whether it belongs to the CLI surface, the slash-command surface, or both should be answered deliberately — not by accident of name collision.

## Key Files

| File | Role |
|------|------|
| `references/claw-code/rust/crates/commands/src/lib.rs` | `SlashCommandSpec` registry, slash-command parsing and help rendering |
| `references/claw-code/rust/crates/rusty-claude-cli/src/main.rs` | CLI argument parsing, `CliAction` dispatch, REPL setup, session-aware completion, autosave, resume replay |
| `references/claw-code/rust/README.md` | Feature table explicitly separating "Direct CLI subcommands" from "Slash commands" |
