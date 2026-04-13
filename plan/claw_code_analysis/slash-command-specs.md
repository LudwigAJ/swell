# Slash Command Registry: `SlashCommandSpec` and Shared Metadata

The ClawCode CLI exposes a large surface of slash commands (`/help`, `/status`, `/compact`, etc.) through a shared, declarative registry centered on the `SlashCommandSpec` struct. This document explains how that registry works, what metadata each command carries, and what resume support looks like at the per-command level.

**Canonical file:** `references/claw-code/rust/crates/commands/src/lib.rs`

---

## `SlashCommandSpec`: The Declarative Command Descriptor

Every slash command is declared as a `SlashCommandSpec` value in a static slice called `SLASH_COMMAND_SPECS`. The struct is intentionally plain — no methods, no runtime computation — so the entire command surface is visible at a glance:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SlashCommandSpec {
    pub name: &'static str,           // canonical command name, e.g. "help"
    pub aliases: &'static [&'static str], // alternative names, e.g. ["skill"] for "skills"
    pub summary: &'static str,       // one-line description shown in help
    pub argument_hint: Option<&'static str>, // expected argument shape, e.g. Some("[model]")
    pub resume_supported: bool,       // whether the command works inside --resume sessions
}
```

The registry is a `&'static [SlashCommandSpec]` — no dynamic allocation, no lazy loading. This makes it cheap to query anywhere in the codebase and guarantees that help text and completion logic are always in sync with the actual command surface.

### Exposed Accessors

Two public functions provide read-only access to the registry:

```rust
pub fn slash_command_specs() -> &'static [SlashCommandSpec]
pub fn resume_supported_slash_commands() -> Vec<&'static SlashCommandSpec>
```

`slash_command_specs()` returns the full list. `resume_supported_slash_commands()` filters on `resume_supported == true` and returns only those that can safely run inside a `--resume SESSION.jsonl` invocation.

---

## Command Metadata Fields

### `name`

The canonical, slash-prefixed name used in the REPL. Examples: `"help"`, `"compact"`, `"session"`. The name is case-insensitively matched during parsing, but the canonical form stored in the spec is lowercase.

### `aliases`

Alternative names that resolve to the same command. For example:

```rust
SlashCommandSpec {
    name: "skills",
    aliases: &["skill"],  // /skill is equivalent to /skills
    ...
}
```

Aliases are also matched case-insensitively. The alias list is often empty (`&[]`) for most commands.

### `summary`

A one-line description of the command's purpose. This is the primary text shown in `/help` output. Examples:

- `"Show available slash commands"` for `/help`
- `"Compact local session history"` for `/compact`
- `"Show token usage totals"` for `/cost`

### `argument_hint`

An optional string describing the expected argument shape. This feeds directly into:

1. **Help text** — renders as `/command argument_hint` in help output
2. **Completion** — provides hints to the REPL's tab-completion engine
3. **Error messages** — usage errors include the argument hint when validation fails

Examples:

```rust
// No arguments expected
argument_hint: None,

// Optional single argument
argument_hint: Some("[model]"),

// Required argument with specific literals
argument_hint: Some("[read-only|workspace-write|danger-full-access]"),

// Multi-argument command
argument_hint: Some("[list|switch <session-id>|fork [branch-name]|delete <session-id> [--force]]"),
```

### `resume_supported`

A boolean indicating whether the command can execute inside a resumed session (`--resume SESSION.jsonl`). Commands that are **not** resume-supported either:

- Mutate global session state in ways that conflict with replay semantics (`/model`, `/permissions`)
- Require a live REPL context that does not exist in headless resume invocations (`/exit`, `/resume`)
- Trigger new API calls or tool executions that are inappropriate outside an active conversation (`/commit`, `/pr`)

This field is the **single source of truth** for determining resume eligibility. There is no fallback logic or heuristics — if `resume_supported` is `false`, the command is excluded from `resume_supported_slash_commands()` and the help annotation `[resume]` is omitted.

---

## How the Registry Is Used

### Help Rendering

`render_slash_command_help()` and `render_slash_command_help_filtered()` iterate over `slash_command_specs()`, grouping commands by category and formatting each line with `format_slash_command_help_line()`:

```rust
fn format_slash_command_help_line(spec: &SlashCommandSpec) -> String {
    let name = slash_command_usage(spec);
    let alias_suffix = if spec.aliases.is_empty() {
        String::new()
    } else {
        format!(" (aliases: {})", spec.aliases.iter().map(|a| format!("/{a}")).collect::<Vec<_>>().join(", "))
    };
    let resume = if spec.resume_supported { " [resume]" } else { "" };
    format!("  {name:<66} {}{alias_suffix}{resume}", spec.summary)
}
```

Note the `[resume]` suffix on the help line — this is only rendered when `resume_supported` is `true`. This is the user-visible indicator that a command works inside `--resume` sessions.

### Per-Command Help Detail

`render_slash_command_help_detail(name)` looks up a command by name or alias and renders a structured detail block:

```
/compact
  Summary          Compact local session history
  Usage            /compact
  Category         Session
  Resume           Supported with --resume SESSION.jsonl
```

The `Resume` line is only present when `resume_supported` is `true`.

### Completion Suggestions

`suggest_slash_commands(input, limit)` fuzzy-matches user input against both `name` and all `aliases`, ranking by prefix match, substring match, and Levenshtein distance. This drives REPL tab-completion independently of resume support.

### Resume Command Filtering

When the CLI is invoked with `--resume SESSION.jsonl [commands...]`, the `resume_supported_slash_commands()` function is used to determine which slash commands are allowed in that context. Commands with `resume_supported: false` are not presented as valid candidates in resume mode.

---

## Resume Support: Command-Level Granularity

The registry carries explicit per-command resume support rather than making a global statement about the command surface. This means:

- **Resume-supported commands** include `/help`, `/status`, `/compact`, `/cost`, `/config`, `/memory`, `/diff`, `/session` (list action), `/tasks` (list action), and approximately 39 others.
- **Resume-unsupported commands** include `/model`, `/permissions`, `/resume` itself, `/exit`, `/commit`, `/pr`, `/issue`, `/bughunter`, `/teleport`, and many tool-mutation or session-lifecycle commands.

This granularity matters because generic statements like "most commands work on resumed sessions" would misrepresent the actual surface. A headless `--resume` invocation with `/model` would be nonsensical — there is no interactive model-switching prompt in that context.

The minimum expected count of resume-supported commands is asserted in a test:

```rust
// crates/rusty-claude-cli/src/main.rs
#[test]
fn resume_supported_command_list_matches_expected_surface() {
    let names = resume_supported_slash_commands()
        .into_iter()
        .map(|spec| spec.name)
        .collect::<Vec<_>>();
    // Now with 135+ slash commands, verify minimum resume support
    assert!(
        names.len() >= 39,
        "expected at least 39 resume-supported commands, got {}",
        names.len()
    );
}
```

The registry currently holds **139 total commands**, of which at least **39 are resume-supported** — roughly 28%. This ratio reflects a deliberate design choice: many commands are REPL-only or require live runtime state.

---

## Command Categories

Commands are organized into four categories used in help grouping:

| Category | Representative Commands |
|----------|------------------------|
| **Session** | `/help`, `/status`, `/cost`, `/compact`, `/session`, `/history`, `/export`, `/clear` |
| **Config** | `/model`, `/permissions`, `/theme`, `/vim`, `/fast`, `/config`, `/env` |
| **Tools** | `/commit`, `/pr`, `/issue`, `/diff`, `/init`, `/review`, `/bughunter` |
| **Debug** | `/doctor`, `/sandbox`, `/debug-tool-call`, `/diagnostics` |

Category assignment is a pure function of the command name with no per-command metadata field — see `slash_command_category()` in `crates/commands/src/lib.rs`.

---

## Builder Lessons

1. **Static, declarative registries beat dynamic registration for CLI surfaces.** Because `SlashCommandSpec` values are compile-time constants, the entire command surface is visible to static analysis, and there is no risk of a command being registered at runtime but missing from help or completion.

2. **Per-command resume eligibility must be explicit, not heuristic.** Treating resume support as a boolean on the spec avoids subtle bugs where a command accidentally works in resume mode (or fails silently). The test asserting a minimum count prevents regression toward zero resume-supported commands.

3. **Argument hints as structured strings enable consistent help and error formatting.** Encoding the expected argument shape directly in the spec means help rendering and error messages share the same source of truth. Changing an argument hint updates both help text and validation errors simultaneously.

4. **Alias support at the registry level simplifies parsing.** Rather than duplicating match arms for each alias, the parser calls `find_slash_command_spec()` which iterates over `name` and all `aliases` in one place. New aliases can be added without changing the parse logic.

5. **Category grouping as a pure function keeps the registry data clean.** `slash_command_category()` derives categories from command names rather than storing a `category` field on each spec. This avoids category drift when commands are added but the category field is forgotten.

---

## Evidence

- `SlashCommandSpec` struct definition: `references/claw-code/rust/crates/commands/src/lib.rs`
- `SLASH_COMMAND_SPECS` static array: `references/claw-code/rust/crates/commands/src/lib.rs`
- `slash_command_specs()` and `resume_supported_slash_commands()` accessors: `references/claw-code/rust/crates/commands/src/lib.rs`
- Help rendering (`format_slash_command_help_line`, `render_slash_command_help`, `render_slash_command_help_detail`): `references/claw-code/rust/crates/commands/src/lib.rs`
- Completion suggestions (`suggest_slash_commands`): `references/claw-code/rust/crates/commands/src/lib.rs`
- Resume filter test (`resume_supported_command_list_matches_expected_surface`): `references/claw-code/rust/crates/rusty-claude-cli/src/main.rs`
- Category assignment (`slash_command_category`): `references/claw-code/rust/crates/commands/src/lib.rs`
