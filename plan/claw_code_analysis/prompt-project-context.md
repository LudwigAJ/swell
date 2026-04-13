# Project-Context Discovery in Prompt Assembly

Project-local context — working directory, instruction files, git state, and runtime configuration — enters the system prompt through a dedicated discovery and rendering pipeline in the `runtime` crate. This document traces how each category of project context is discovered, assembled, and injected so that builders can understand the boundaries and extension points of the prompt construction system.

## Owning Subsystem

Prompt assembly lives in [`references/claw-code/rust/crates/runtime/src/prompt.rs`](references/claw-code/rust/crates/runtime/src/prompt.rs). Git-aware context collection is extracted into [`references/claw-code/rust/crates/runtime/src/git_context.rs`](references/claw-code/rust/crates/runtime/src/git_context.rs). Both are re-exported from the runtime crate's root lib as part of the public API surface.

The runtime crate's lib doc summarizes the scope:

> This crate owns session persistence, permission evaluation, prompt assembly, MCP plumbing, tool-facing file operations, and the core conversation loop that drives interactive and one-shot turns.

## ProjectContext Discovery

`ProjectContext` is the central struct that bundles all discovered project-local state:

```rust
// references/claw-code/rust/crates/runtime/src/prompt.rs
pub struct ProjectContext {
    pub cwd: PathBuf,
    pub current_date: String,
    pub git_status: Option<String>,
    pub git_diff: Option<String>,
    pub git_context: Option<GitContext>,
    pub instruction_files: Vec<ContextFile>,
}
```

`ProjectContext::discover()` populates the working directory, date, and instruction files. `ProjectContext::discover_with_git()` extends that base with git status, diff, and `GitContext` (branch, recent commits, staged files). The two-phase discovery pattern — base discovery then optional git augmentation — reflects the fact that git calls are more expensive (subprocess invocations) and may be suppressed in some environments.

## cwd Discovery

`cwd` is not auto-detected inside `ProjectContext::discover()`; it is passed in by the caller. This is a deliberate design choice: the runtime did not invent its own working-directory detection logic. Instead, the caller — typically the conversation runtime or CLI bootstrap — supplies the cwd at the call site. This keeps the prompt builder testable and keeps working-directory semantics consistent with how the process was launched.

The environment section of the rendered prompt shows the cwd explicitly:

```rust
fn environment_section(&self) -> String {
    let cwd = self.project_context.as_ref().map_or_else(
        || "unknown".to_string(),
        |context| context.cwd.display().to_string(),
    );
    // ...
    format!("Working directory: {cwd}")
}
```

## Instruction-File Discovery Path Behavior

`discover_instruction_files()` walks the directory tree from the current working directory up to the filesystem root, collecting instruction files along the way. The candidate filenames are fixed:

- `CLAUDE.md` — project-level instructions
- `CLAUDE.local.md` — local/machine-level overrides
- `.claw/CLAUDE.md` — dot-claw namespaced instructions
- `.claw/instructions.md` — alternative dot-claw instruction filename

```rust
// references/claw-code/rust/crates/runtime/src/prompt.rs
fn discover_instruction_files(cwd: &Path) -> std::io::Result<Vec<ContextFile>> {
    let mut directories = Vec::new();
    let mut cursor = Some(cwd);
    while let Some(dir) = cursor {
        directories.push(dir.to_path_buf());
        cursor = dir.parent();
    }
    directories.reverse(); // walk from root downward

    let mut files = Vec::new();
    for dir in directories {
        for candidate in [
            dir.join("CLAUDE.md"),
            dir.join("CLAUDE.local.md"),
            dir.join(".claw").join("CLAUDE.md"),
            dir.join(".claw").join("instructions.md"),
        ] {
            push_context_file(&mut files, candidate)?;
        }
    }
    Ok(dedupe_instruction_files(files))
}
```

Key behavioral properties of this walk:

1. **Ancestor chain walk, root-first**: The walk starts at the filesystem root and descends toward `cwd`. This means root-level instruction files appear before nested ones in the collected list, matching the natural scope-breadth ordering (most-general first).
2. **All four names are checked at every level**: Every directory in the ancestor chain gets all four filename variants checked, not just the nearest directory.
3. **Content deduplication**: `dedupe_instruction_files()` normalizes content (collapse blank lines, trim) and hashes it. Files with identical normalized content across different paths are deduplicated — only the first occurrence is kept. This prevents the same instruction block from appearing twice when a root `CLAUDE.md` is inherited by a nested subdirectory.
4. **Per-file and total budget limits**: Individual files are truncated to `MAX_INSTRUCTION_FILE_CHARS` (4,000 chars). The total instruction budget is `MAX_TOTAL_INSTRUCTION_CHARS` (12,000 chars). When the budget is exhausted, remaining files are omitted with an `_Additional instruction content omitted after reaching the prompt budget._` marker.

The `render_instruction_files()` function handles both truncation and budget enforcement. It renders each file with a scope label (the nearest parent directory) so the model can see where each instruction block originated:

```
## CLAUDE.md (scope: /tmp/project)
Project rules content here...

## CLAUDE.local.md (scope: /tmp/project)
Local override content...
```

The test `discovers_instruction_files_from_ancestor_chain` in `prompt.rs` validates this behavior by creating a nested directory structure with instruction files at multiple levels and asserting the correct collection order.

## Git Snapshots as Dynamic Prompt Context

Git context is populated by `discover_with_git()` using three distinct subprocess calls:

| Field | Source | Trigger |
|---|---|---|
| `git_status` | `git --no-optional-locks status --short --branch` | Always attempted |
| `git_diff` | `git diff` (staged + unstaged) | Always attempted |
| `git_context` | `GitContext::detect()` | Always attempted |

`GitContext::detect()` performs a cheap gate first (`git rev-parse --is-inside-work-tree`) and returns `None` if the directory is not inside a git repository. When inside a repo, it collects:

- **Branch name**: `git rev-parse --abbrev-ref HEAD`
- **Recent commits (up to 5)**: `git --no-optional-locks log --oneline -n 5 --no-decorate`
- **Staged files**: `git --no-optional-locks diff --cached --name-only`

```rust
// references/claw-code/rust/crates/runtime/src/git_context.rs
impl GitContext {
    pub fn detect(cwd: &Path) -> Option<Self> {
        let rev_parse = Command::new("git")
            .args(["rev-parse", "--is-inside-work-tree"])
            .current_dir(cwd)
            .output()
            .ok()?;
        if !rev_parse.status.success() {
            return None;
        }
        Some(Self {
            branch: read_branch(cwd),
            recent_commits: read_recent_commits(cwd),
            staged_files: read_staged_files(cwd),
        })
    }
}
```

The `--no-optional-locks` flag is used throughout to avoid touching shared git refs that might be held by another process. If any subprocess fails, that specific field is set to `None` — failures are isolated per-field, not fatal.

`render_project_context()` formats all git fields into the prompt under a `# Project context` section:

```
# Project context
 - Today's date is 2026-03-31.
 - Working directory: /tmp/project.
 - Claude instruction files discovered: 2.

Git status snapshot:
## main
M  src/lib.rs
?? new_file.txt

Recent commits (last 5):
  abc1234 fix: correct permission check ordering
  def5678 feat: add MCP degraded startup reporting
  ...

Git diff snapshot:
Unstaged changes:
diff --git a/src/lib.rs b/src/lib.rs
...
```

The staged-files count also appears inside `GitContext::render()` under `Staged files:`. Note that `git_diff` captures both staged and unstaged changes separately — `git diff --cached` for staged, `git diff` for unstaged — and renders them under a single `Git diff snapshot:` heading.

## Dynamic Boundary Marker

The `SYSTEM_PROMPT_DYNAMIC_BOUNDARY` constant (`"__SYSTEM_PROMPT_DYNAMIC_BOUNDARY__"`) is inserted into the assembled prompt between the static scaffolding (intro, system, doing-tasks, actions sections) and the dynamic runtime context (environment, project context, instructions, config). This lets downstream consumers — or the model itself — distinguish which part of the prompt was generated once at startup versus which part changes per-turn based on the current project state.

The boundary appears as a literal string marker in the rendered prompt output. In `SystemPromptBuilder::build()`:

```rust
sections.push(SYSTEM_PROMPT_DYNAMIC_BOUNDARY.to_string());
sections.push(self.environment_section());
if let Some(project_context) = &self.project_context {
    sections.push(render_project_context(project_context));
    if !project_context.instruction_files.is_empty() {
        sections.push(render_instruction_files(&project_context.instruction_files));
    }
}
if let Some(config) = &self.config {
    sections.push(render_config_section(config));
}
```

## Configuration-Backed Sections

`RuntimeConfig` is loaded via `ConfigLoader::default_for(&cwd)` (called inside `load_system_prompt()`) and rendered into the prompt as a `# Runtime config` section. This section is distinct from instruction files: it contains machine-readable runtime settings rather than human-authored project guidance.

```rust
fn render_config_section(config: &RuntimeConfig) -> String {
    let mut lines = vec!["# Runtime config".to_string()];
    if config.loaded_entries().is_empty() {
        lines.extend(prepend_bullets(vec![
            "No Claw Code settings files loaded.".to_string()
        ]));
        return lines.join("\n");
    }
    lines.extend(prepend_bullets(
        config
            .loaded_entries()
            .iter()
            .map(|entry| format!("Loaded {:?}: {}", entry.source, entry.path.display()))
            .collect(),
    ));
    lines.push(String::new());
    lines.push(config.as_json().render());
    lines.join("\n")
}
```

Each loaded config entry is named with its source type and path before the full JSON blob is appended. This gives a human-readable audit trail showing which config files were found and where they came from.

## SystemPromptBuilder Composition Order

`SystemPromptBuilder::build()` assembles the full prompt as an ordered `Vec<String>` of sections:

1. **Intro section** — model identity and output-style framing
2. **Output style section** (optional) — user-selected output style customization
3. **System section** — static rules about tool use, text display, and system reminders
4. **Doing tasks section** — engineering conduct expectations (tight scope, no speculation, etc.)
5. **Actions section** — blast-radius and reversibility guidance
6. **`__SYSTEM_PROMPT_DYNAMIC_BOUNDARY__`** — literal boundary marker
7. **Environment section** — model family, working directory, date, platform
8. **Project context section** — git status, diff, recent commits (if git-enabled)
9. **Claude instructions section** — discovered instruction files with scope labels
10. **Runtime config section** — loaded config entries and JSON
11. **Append sections** — extra sections added via `append_section()` (for test extensibility and ad-hoc injection)

The boundary marker at position 6 is the key structural signal: sections 1–5 are static prompt scaffolding that can be cached or pre-computed, while sections 6–11 are computed fresh from the current runtime state.

## Builder Lessons

1. **Two-phase discovery separates cheap from expensive operations.** `ProjectContext::discover()` is pure filesystem reads (no subprocesses), while the git augmentation in `discover_with_git()` adds subprocess calls. Keeping these separate means callers can use base discovery in contexts where git is unavailable or undesirable.

2. **Instruction files use a breadth-first ancestor walk, not a depth-first search.** Walking from root to `cwd` means more-general (root-level) instructions appear before more-specific (nested) ones. The deduplication pass then eliminates content-level duplicates. This ordering is important for the semantic meaning of the collected files — root instructions define project-wide policy, nested instructions refine it.

3. **Content deduplication uses normalized hashes, not path comparison.** Two instruction files at different paths with identical normalized content (same text, collapsed blank lines) are deduplicated. This is the right behavior because instruction files are about content, not location. But it means builders should not rely on path-based uniqueness guarantees.

4. **Budget enforcement is explicit and dual-layered.** Each file is individually capped at 4,000 chars, and the total instruction section is capped at 12,000 chars. The `[truncated]` suffix is appended inline rather than throwing an error, so the prompt remains well-formed even when budget is exceeded.

5. **`--no-optional-locks` on all git commands prevents shared-git-ref contention.** In developer environments where another git process holds a ref lock, these commands will fail gracefully (returning None/empty) rather than blocking or erroring. This makes the git snapshot feature robust in multi-process git environments.

6. **Config sections are self-describing.** Each loaded config entry is annotated with its source type and path before the JSON blob is rendered. This makes the prompt useful for debugging config loading problems — the model can see exactly which files were found and in what order.

## Evidence Chain

- [`references/claw-code/rust/crates/runtime/src/prompt.rs`](references/claw-code/rust/crates/runtime/src/prompt.rs) — `ProjectContext`, `SystemPromptBuilder`, `discover_instruction_files()`, `render_project_context()`, `render_instruction_files()`, `render_config_section()`, `SYSTEM_PROMPT_DYNAMIC_BOUNDARY`
- [`references/claw-code/rust/crates/runtime/src/git_context.rs`](references/claw-code/rust/crates/runtime/src/git_context.rs) — `GitContext::detect()`, `GitCommitEntry`, `read_branch()`, `read_recent_commits()`, `read_staged_files()`
- [`references/claw-code/rust/crates/runtime/src/lib.rs`](references/claw-code/rust/crates/runtime/src/lib.rs) — public re-exports confirming the owning crate and public API surface
- Unit tests in `prompt.rs` — `discovers_instruction_files_from_ancestor_chain`, `dedupes_identical_instruction_content_across_scopes`, `discover_with_git_includes_status_snapshot`, `discover_with_git_includes_recent_commits_and_renders_them`, `discover_with_git_includes_diff_snapshot_for_tracked_changes`
- Unit tests in `git_context.rs` — `returns_none_for_non_git_directory`, `detects_branch_name_and_commits`, `detects_staged_files`, `limits_to_five_recent_commits`
