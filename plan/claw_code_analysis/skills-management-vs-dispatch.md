# Skills Management vs. Invocation Dispatch

This document explains how ClawCode distinguishes between local skill management operations (`/skills list`, `/skills install`, `/skills help`) and skill invocation dispatch (`/skills <skill> [args]`), and how on-disk skill roots are discovered across project and user filesystem locations.

## The Two Dispatch Surfaces

The `/skills` slash command is entered once but branches into two fundamentally different execution paths based on its arguments:

- **Local management** — handled entirely within the CLI process, no skill content is consulted
- **Invocation dispatch** — resolves a named skill on disk, then rewrites the remaining arguments into a `$skill [args]` prompt string that the runtime consumes

The split is controlled by `classify_skills_slash_command` in `commands/src/lib.rs`:

```rust
#[must_use]
pub fn classify_skills_slash_command(args: Option<&str>) -> SkillSlashDispatch {
    match normalize_optional_args(args) {
        None | Some("list" | "help" | "-h" | "--help") => SkillSlashDispatch::Local,
        Some(args) if args == "install" || args.starts_with("install ") => {
            SkillSlashDispatch::Local
        }
        Some(args) => SkillSlashDispatch::Invoke(format!("${}", args.trim_start_matches('/'))),
    }
}
```

`SkillSlashDispatch::Local` is returned for bare `/skills`, `/skills list`, `/skills help`, or any `install` variant. Every other argument shape — including bare skill names like `/skills plan` or arbitrary text like `/skills help overview` — produces `SkillSlashDispatch::Invoke` with a `$`-prefixed prompt string.

The `Invoke` variant carries a rewrite: `/skills help overview` becomes `$help overview`. The leading `$` signals to the runtime that this is a skill invocation prompt, not a plain message. This distinction matters because `Local` operations never consult the filesystem, while `Invoke` operations require the named skill to exist on disk.

## Why the Split Exists

Local listing and help are lightweight and must work even when no skills are installed. They report counts and show usage strings without touching any skill files. Invocation, by contrast, requires that the named skill actually exists — if it does not, `resolve_skill_invocation` produces a structured error that enumerates available skill names from the discovered roots.

From a builder's perspective, this separation means the CLI can report on the skills surface even in an empty or misconfigured environment. It also means that `/skills unknown` is not silently swallowed — the runtime has a chance to explain that the skill was not found and suggest alternatives from the same roots that would have been searched.

## Root Discovery

Skills are not hardcoded in the binary. The CLI discovers them by walking a fixed set of filesystem paths at runtime. The function `discover_skill_roots` in `commands/src/lib.rs` constructs this list in three stages:

### Stage 1 — Project roots via ancestor walk

For each ancestor of the current working directory, the following paths are checked:

| Path | Source | Origin |
|---|---|---|
| `.claw/skills` | `ProjectClaw` | `SkillsDir` |
| `.omc/skills` | `ProjectClaw` | `SkillsDir` |
| `.agents/skills` | `ProjectClaw` | `SkillsDir` |
| `.codex/skills` | `ProjectCodex` | `SkillsDir` |
| `.claude/skills` | `ProjectClaude` | `SkillsDir` |
| `.claw/commands` | `ProjectClaw` | `LegacyCommandsDir` |
| `.codex/commands` | `ProjectCodex` | `LegacyCommandsDir` |
| `.claude/commands` | `ProjectClaude` | `LegacyCommandsDir` |

The ancestor walk means that a project-local skill root in any parent directory of the current working directory is discovered automatically.

### Stage 2 — User config directory roots

If `CLAW_CONFIG_HOME` or `CODEX_HOME` is set, their `skills` and `commands` subdirectories are appended.

### Stage 3 — Home directory roots

From `$HOME` and `$CLAUDE_CONFIG_DIR`, a broader set of compatibility roots is added:

| Path | Source | Origin |
|---|---|---|
| `~/.claw/skills` | `UserClaw` | `SkillsDir` |
| `~/.omc/skills` | `UserClaw` | `SkillsDir` |
| `~/.codex/skills` | `UserCodex` | `SkillsDir` |
| `~/.claude/skills` | `UserClaude` | `SkillsDir` |
| `~/.claude/skills/omc-learned` | `UserClaude` | `SkillsDir` |
| `~/.claw/commands` | `UserClaw` | `LegacyCommandsDir` |
| `~/.codex/commands` | `UserCodex` | `LegacyCommandsDir` |
| `~/.claude/commands` | `UserClaude` | `LegacyCommandsDir` |
| `$CLAUDE_CONFIG_DIR/skills` | `UserClaude` | `SkillsDir` |
| `$CLAUDE_CONFIG_DIR/skills/omc-learned` | `UserClaude` | `SkillsDir` |
| `$CLAUDE_CONFIG_DIR/commands` | `UserClaude` | `LegacyCommandsDir` |

`push_unique_skill_root` deduplicates by path, so if multiple environment variables resolve to the same directory it is added only once.

## Two Skill Origin Types

Every discovered root is tagged with one of two `SkillOrigin` values:

- **`SkillsDir`** — the root is a `skills` directory. Skills are subdirectories containing `SKILL.md`. The directory name is the skill name unless overridden by frontmatter.
- **`LegacyCommandsDir`** — the root is a `commands` directory. Skills are individual `.md` files (or subdirectories with `SKILL.md`). This accommodates the older `/command` convention where each command was a single markdown file.

The origin is surfaced in the JSON report and in error messages, so users can understand whether a skill came from a modern skills directory or a legacy commands directory.

## Shadowing and Priority

When multiple roots define a skill with the same name, only the first-seen definition is kept as "active" — subsequent definitions are marked as shadowed. The `load_skills_from_roots` function maintains a `BTreeMap<String, DefinitionSource>` called `active_sources`. When a skill name (lower-cased for case-insensitive matching) is already present, the new entry is marked with `shadowed_by = Some(existing_source)` rather than replacing it.

The order of roots in the discovery list determines priority. Because project roots are discovered first via the ancestor walk, project-local skills shadow user-global skills with the same name. This lets a team pin a specific skill version in the project while still allowing users to maintain personal overrides for skills the project does not define.

## Installation

`/skills install <path>` copies a skill from the given source path into the default install root. The install root is resolved in this order:

1. `$CLAW_CONFIG_HOME/skills` if `CLAW_CONFIG_HOME` is set
2. `$CODEX_HOME/skills` if `CODEX_HOME` is set
3. `$HOME/.claw/skills` as fallback

After copying, the skill becomes available under the invocation name derived from its frontmatter `name` field or, failing that, its directory name.

## JSON Surface

The `/skills` slash command is also available as a structured JSON surface via `handle_skills_slash_command_json`. This serializes the full skill list including `source`, `origin`, `active` status, and `shadowed_by` fields. The `usage` block in help output enumerates all active root paths.

## Builder Lessons

1. **Two-phase argument classification** — `classify_skills_slash_command` separates parsing from execution. This lets the CLI surface errors early (for example, unknown skill names) without running the full command handler.

2. **Ancestor-walk discovery** — project-local configuration is discovered by walking `cwd.ancestors()`, not by searching upward for a fixed filename. This means any directory in the tree can have its own `.claw/skills` without requiring a configuration file pointing to it.

3. **Shadowing replaces overriding** — instead of deep-merge config semantics, later roots add skills to a list but earlier roots win for duplicate names. This is simpler to reason about than layered config and matches how shell `PATH` works.

4. **Legacy accommodation without special-casing** — `LegacyCommandsDir` is treated as just another `SkillOrigin`. The rendering layer adapts the display format based on origin, but the loading logic does not need an ad hoc branch for legacy commands once the origin is tracked.

5. **Error messages include nearby alternatives** — `resolve_skill_invocation` fetches the full skill list on failure and lists the names of available skills in the error message. This turns a generic "not found" into a actionable hint.

## Key Source Locations

| Symbol | File | Role |
|---|---|---|
| `classify_skills_slash_command` | `commands/src/lib.rs` | Argument → dispatch classification |
| `SkillSlashDispatch` | `commands/src/lib.rs` | Enum: `Local`, `Invoke` |
| `discover_skill_roots` | `commands/src/lib.rs` | Filesystem root discovery |
| `push_unique_skill_root` | `commands/src/lib.rs` | Deduplicated root registration |
| `load_skills_from_roots` | `commands/src/lib.rs` | Skill enumeration with shadowing |
| `SkillOrigin` | `commands/src/lib.rs` | Enum: `SkillsDir`, `LegacyCommandsDir` |
| `DefinitionSource` | `commands/src/lib.rs` | Enum: project vs user, claw vs codex vs claude |
| `install_skill` | `commands/src/lib.rs` | Skill installation to default root |
| `render_skills_usage` | `commands/src/lib.rs` | Help output with root enumeration |

## Evidence Sources

- **USAGE.md** — user-facing reference for skill discovery paths: explains `.codex/` directories as legacy compatibility roots (FAQ → "What about Codex?") and documents config resolution order (`~/.claw.json` → `<repo>/.claw/settings.local.json`) which governs how claw applies layered config across project and user directories.
