# Prompt Builder Sections

## What This Is

The ClawCode runtime assembles every upstream API request by composing a **system prompt** from a sequence of discrete, ordered sections rather than loading one opaque blob. The builder (`SystemPromptBuilder` in `references/claw-code/rust/crates/runtime/src/prompt.rs`) produces a `Vec<String>` where each entry is a named section. At render time these sections are joined with double newlines into the final string fed to the model.

This section-based design makes three things possible that a monolithic string approach would make awkward:

1. **Static scaffolding** — introductory text, behavioral guidelines, and action caveats — is separated from **dynamic runtime context** injected per-session.
2. Each dynamic section can be independently populated, omitted, or replaced without restructuring the static parts.
3. The boundary between the two is surfaced as a named marker constant so callers can detect or manipulate where dynamic context begins.

## Section Ordering

`SystemPromptBuilder::build()` produces sections in this fixed order:

```
[0] Intro (with or without Output Style header)
[1] Output Style (conditional)
[2] "# System" static guidelines
[3] "# Doing tasks" static guidelines
[4] "# Executing actions with care" static guidelines
[5] SYSTEM_PROMPT_DYNAMIC_BOUNDARY marker ← static/dynamic boundary
[6] Environment context section
[7] Project context section (with git snapshots + instruction files)
[8] Runtime config section
[9] Any appended extra sections
```

The boundary marker (`"__SYSTEM_PROMPT_DYNAMIC_BOUNDARY__"` defined as `SYSTEM_PROMPT_DYNAMIC_BOUNDARY` in `prompt.rs`) lands between the static scaffolding and all dynamic runtime context. This is intentional: static text is the same across sessions; dynamic context changes every time.

## Static Sections

### Intro

`get_simple_intro_section()` renders the opening "You are an interactive agent..." paragraph. When an output style is active, the intro sentence is adjusted to reference that style. This is the **only** section that varies in text based on output-style configuration, but it remains static in structure.

### System, Doing Tasks, Executing Actions

`get_simple_system_section()` emits the `# System` block (tool-result handling, permission mode reminders, prompt-injection warnings, compaction notices). `get_simple_doing_tasks_section()` emits the `# Doing tasks` block (read-before-edit, scope discipline, no speculative abstractions). `get_actions_section()` emits a short `# Executing actions with care` advisory. These three sections are **completely static** — they never change between calls and carry no session-specific data.

## The Dynamic Boundary

```rust
pub const SYSTEM_PROMPT_DYNAMIC_BOUNDARY: &str = "__SYSTEM_PROMPT_DYNAMIC_BOUNDARY__";
```

This constant is inserted as a literal string section. Upstream callers can use it to locate where the static scaffolding ends and runtime-injected context begins. The marker appears verbatim in the rendered prompt, so model-context-aware tooling can also detect it. The boundary is not a comment or metadata — it is part of the prompt itself.

## Environment Context Section

`SystemPromptBuilder::environment_section()` emits a `# Environment context` block with four concrete fields:

| Field | Source |
|---|---|
| Model family | Hardcoded `FRONTIER_MODEL_NAME` constant (`"Claude Opus 4.6"`) |
| Working directory | Resolved from `project_context.cwd` |
| Date | Passed in at build time; injected as-is |
| Platform | `os_name` + `os_version` from builder options |

The section is rendered as bullet points prefixed with ` - `. If no project context is present, `cwd` and `date` fall back to `"unknown"`.

## Project Context Section

`ProjectContext` aggregates everything the runtime discovers about the local workspace:

```rust
pub struct ProjectContext {
    pub cwd: PathBuf,
    pub current_date: String,
    pub git_status: Option<String>,      // git --no-optional-locks status --short --branch
    pub git_diff: Option<String>,        // staged + unstaged diff
    pub git_context: Option<GitContext>, // recent commits + branch metadata
    pub instruction_files: Vec<ContextFile>,
}
```

`ProjectContext::discover()` performs the basic discovery (cwd + date + instruction files). `ProjectContext::discover_with_git()` adds git snapshot collection on top.

### Git Snapshots

- `read_git_status()` runs `git --no-optional-locks status --short --branch` and returns the trimmed output verbatim, or `None` if the command fails or produces no output.
- `read_git_diff()` captures both `git diff --cached` (staged) and `git diff` (unstaged) and joins them with section headers, or returns `None` if everything is clean.
- `GitContext::detect()` (defined in `git_context.rs` within the runtime crate) populates recent commits and branch metadata — these appear separately in the rendered project context.

These snapshots give the model a read-only view of repo state without requiring a tool call.

### Instruction File Discovery

`discover_instruction_files()` walks the **ancestor directory chain** from the current working directory up to the filesystem root, collecting candidate files at each level:

```
CLAUDE.md
CLAUDE.local.md
.claw/CLAUDE.md
.claw/instructions.md
```

The search is bottom-up: the most-specific (nested) files appear first in the collected list. After collection, `dedupe_instruction_files()` normalizes each file's content (collapses blank lines, trims) and deduplicates by stable content hash so identical rules appearing at multiple levels are not repeated.

File content is rendered in `render_instruction_files()` with two budget guards:

- `MAX_INSTRUCTION_FILE_CHARS = 4_000` per individual file
- `MAX_TOTAL_INSTRUCTION_CHARS = 12_000` total across all files

Files exceeding the running budget are stopped with `"[truncated]"` or `"_Additional instruction content omitted after reaching the prompt budget._"`. This prevents instruction files from inflating the prompt beyond what the model can reasonably consume.

## Runtime Config Section

`render_config_section()` emits a `# Runtime config` block listing every successfully loaded settings file (source name + path) followed by the JSON representation of the merged config. If no settings files were loaded, it renders `"No Claw Code settings files loaded."` This section is the only place runtime configuration enters the prompt — hooks, permission modes, aliases, and provider settings are all visible here as a config dump.

## Output Style Section

`SystemPromptBuilder::with_output_style(name, prompt)` adds a `# Output Style: {name}` section with the custom prompt text. This is the **only** section that is opt-in per-session — all others are always present (with null-guard fallbacks). Output style customization is layered into the system prompt as an additive section rather than a modifier applied to existing sections, which keeps the composability clean.

## Builder API Shape

The builder uses a fluent builder pattern:

```rust
let sections = SystemPromptBuilder::new()
    .with_output_style("Concise", "Prefer short answers.")
    .with_os("linux", "6.8")
    .with_project_context(project_context)
    .with_runtime_config(config)
    .append_section("custom section")
    .build(); // -> Vec<String>
```

`.build()` returns the ordered vector of sections. `.render()` joins them with `\n\n` into a single string. Additional sections can be appended via `.append_section()`, which is used for any extension or integration-added content.

## Design Lessons for Builders

1. **Boundary markers make downstream processing tractable.** Having `SYSTEM_PROMPT_DYNAMIC_BOUNDARY` as a literal string in the output makes it trivial for tooling to split or patch the prompt without parsing structural metadata.

2. **Section ordering is a contract, not an implementation detail.** Because callers know the exact ordering, they can insert at position `[5]` (before dynamic context) or position `[8]` (after config) without understanding the contents of other sections.

3. **Budget guards prevent prompt bloat.** Instruction file rendering enforces per-file and total character caps rather than silently letting unbounded content grow. This is the only practical way to keep prompt size predictable as workspace complexity increases.

4. **Git snapshots are optional and guarded.** `read_git_status()` and `read_git_diff()` return `None` on failure rather than propagating errors. This prevents a broken git installation from blocking prompt assembly entirely.

5. **Static scaffolding is loaded once; dynamic context is loaded per-session.** The three static sections (`# System`, `# Doing tasks`, `# Executing actions`) are pure functions with no I/O. Instruction file discovery, git snapshots, and config loading are all deferred to the dynamic pass — which is the only pass that actually touches the filesystem.

## Code References

| Symbol | File | Role |
|---|---|---|
| `SystemPromptBuilder` | `prompt.rs` | Main builder type |
| `SYSTEM_PROMPT_DYNAMIC_BOUNDARY` | `prompt.rs` | Static/dynamic boundary constant |
| `ProjectContext` | `prompt.rs` | Workspace context aggregator |
| `ProjectContext::discover` | `prompt.rs` | Basic discovery (cwd + date + instructions) |
| `ProjectContext::discover_with_git` | `prompt.rs` | Full discovery with git snapshots |
| `discover_instruction_files` | `prompt.rs` | Ancestor-chain instruction file search |
| `render_instruction_files` | `prompt.rs` | Instruction file rendering with budget guards |
| `render_config_section` | `prompt.rs` | Runtime config → prompt section |
| `read_git_status` / `read_git_diff` | `prompt.rs` | Git snapshot collection |
| `FRONTIER_MODEL_NAME` | `prompt.rs` | Default model name embedded in environment section |
| `MAX_INSTRUCTION_FILE_CHARS` / `MAX_TOTAL_INSTRUCTION_CHARS` | `prompt.rs` | Instruction file budget caps |
