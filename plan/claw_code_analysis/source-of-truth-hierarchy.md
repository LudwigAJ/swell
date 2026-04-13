# Source-of-Truth Hierarchy in ClawCode

## The canonical surface: `rust/`

The root README.md states this plainly:

> The canonical implementation lives in `rust/`, and the current source of truth for this repository is **ultraworkers/claw-code**.

This is not a stylistic preference — it reflects how the project is organized and maintained. The `rust/` directory is a Cargo workspace containing nine crates:

- **`runtime`** — session, config, permissions, MCP, prompts, auth, and the conversation runtime loop
- **`tools`** — built-in tool specs and execution handlers (Bash, ReadFile, WriteFile, EditFile, GlobSearch, GrepSearch, WebSearch, WebFetch, Agent, TodoWrite, NotebookEdit, Skill, ToolSearch, etc.)
- **`rusty-claude-cli`** — the `claw` binary, REPL, one-shot prompt, CLI subcommands, streaming display
- **`api`** — provider clients, SSE streaming, auth (`ANTHROPIC_API_KEY` + bearer-token support)
- **`commands`** — shared slash-command registry, parsing, help rendering
- **`plugins`** — plugin metadata, install/enable/disable surfaces, hook integration
- **`mock-anthropic-service`** — deterministic Anthropic-compatible mock for CLI parity tests
- **`compat-harness`** — extracts tool/prompt manifests from upstream TS source
- **`telemetry`** — session trace events and usage telemetry types

The workspace ships **~20K lines of Rust** across **48,599 tracked Rust LOC** (per PARITY.md at the 9-lane checkpoint). Every behavioral claim in the analysis documents must trace to a file under `rust/` or to one of the root narrative docs (PARITY.md, ROADMAP.md, PHILOSOPHY.md, USAGE.md).

## Why `rust/` is primary

`rust/` is primary because it is the actively maintained, CI-validated implementation surface:

1. **Verification runs from `rust/`** — `cargo fmt`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo test --workspace` are the canonical health checks. The CLAUDE.md at the repo root instructs agents to run these from `rust/` specifically.

2. **The CLI binary lives there** — `cargo run -p rusty-claude-cli -- --help` is the canonical help surface. Any claim about CLI behavior must be verified against the actual binary, not against the Python reference.

3. **The parity harness is Rust-native** — `mock_parity_scenarios.json`, `mock_parity_harness.rs`, and `run_mock_parity_diff.py` all live under `rust/`. Behavioral coverage is measured against the Rust implementation, not the Python surface.

4. **Crates have explicit ownership** — runtime, tools, api, commands, and plugins each own a distinct part of the system. Cross-crate boundaries are enforced by the Cargo workspace graph.

## What the root docs contribute

The root-level documents are narrative surfaces, not implementation surfaces:

| Document | Role |
|---|---|
| **`USAGE.md`** | Task-oriented guide: build steps, auth setup, CLI usage, session workflows, parity harness usage. Do not use `cargo install claw-code` — that installs a deprecated stub. |
| **`PARITY.md`** | Canonical parity status. The top-level `PARITY.md` (not `rust/PARITY.md`) is consumed by `rust/scripts/run_mock_parity_diff.py`. All behavioral coverage claims must appear here to pass the harness reference check. |
| **`ROADMAP.md`** | Active roadmap and cleanup backlog. Contains done items, in-progress work, and aspirational items. Some roadmap entries describe functionality not yet merged to `main` — these are explicitly marked and must not be treated as current behavior. |
| **`PHILOSOPHY.md`** | Project intent and system-design framing. States that "the code is evidence; the coordination system is the product lesson." Frames the human role as direction-setting, not micromanagement. |
| **`CLAUDE.md`** | AI-agent guidance for working in the repo. Confirms `rust/` is canonical, instructs to update `src/` and `tests/` together when behavior changes, and specifies that `.claude/settings.local.json` is for machine-local overrides. |

None of these root docs are sufficient alone for claiming how something *works now*. They provide context, but implementation in `rust/` is the final authority.

## Why `src/` and `tests/` are secondary

The root README.md describes them explicitly:

> `src/` + `tests/` — companion Python/reference workspace and audit helpers; **not the primary runtime surface**.

The CLAUDE.md clarifies the working agreement for these surfaces:

- `src/` contains source files that should stay consistent with generated guidance and tests.
- `tests/` contains validation surfaces that should be reviewed alongside code changes.
- Both `src/` and `tests/` should be updated together when behavior changes.

This secondary status means several things in practice:

- **`src/` is not the runtime** — it is a Python reference surface. Claims about conversation loop behavior, permission enforcement, or tool execution must be traced to `rust/crates/runtime/` or `rust/crates/tools/`, not to Python files in `src/`.

- **`tests/` is not implementation evidence** — it validates behavior but does not define it. A test passing or failing tells you about the behavior contract, not about whether the Rust implementation is correct.

- **Companion, not canonical** — when a feature changes, both `src/` and `tests/` surfaces are updated together, but the Rust implementation is what ships. The Python surfaces are reference material for auditing parity against the upstream.

## Present-tense behavior lives in `rust/`

When writing analysis documents about ClawCode, the discipline is:

- For **how something works now** → cite `rust/crates/` files, `rust/README.md`, or root docs (PARITY.md, ROADMAP.md, USAGE.md)
- For **what is being built** → cite ROADMAP.md entries explicitly marked as in-progress or not-yet-merged
- For **philosophy and intent** → cite PHILOSOPHY.md
- For **parity evidence chain** → cite PARITY.md, `rust/mock_parity_scenarios.json`, and `rust/scripts/run_mock_parity_diff.py`

The `src/` and `tests/` surfaces are useful for cross-checking behavior against the Python reference, but they are not the implementation of record. A claim like "the permission system denies writes outside the workspace" must be evidenced from `rust/crates/runtime/src/permission_enforcer.rs` or `rust/crates/runtime/src/permissions.rs`, not from Python files in `src/`.

## Evidence from the repo

- `references/claw-code/README.md` — explicitly names `rust/` as canonical: "The canonical implementation lives in `rust/`"
- `references/claw-code/CLAUDE.md` — confirms `rust/` as the active CLI/runtime implementation; specifies `src/` and `tests/` are companion surfaces to be updated together
- `references/claw-code/rust/README.md` — workspace layout, crate responsibilities, feature table, and CLI surface (the binary is `claw`)
- `references/claw-code/PARITY.md` — states "Canonical document: this top-level `PARITY.md` is the file consumed by `rust/scripts/run_mock_parity_diff.py`" — confirming `rust/` as the enforcement surface
- `references/claw-code/PHILOSOPHY.md` — "the code is evidence; the coordination system is the product lesson"

## Builder lessons

**Treat the canonical surface as the only authoritative source.** When a repo has multiple surfaces (Rust implementation + Python reference + root docs), explicit hierarchy prevents analysis documents from drifting into contradictions. ClawCode resolves this by making `rust/` primary and naming `src/`/`tests/` as secondary companion surfaces.

**Narrative docs must be verifiable against implementation.** PARITY.md is consumed by a script, not just read by humans. This means staleness is a CI failure, not a documentation debt. The same pattern applies: if a design document is not checked by automated tooling, its claims about implementation will eventually drift.

**Secondary surfaces still carry obligations.** CLAUDE.md specifies that `src/` and `tests/` must be updated together when behavior changes. This keeps the Python reference from diverging from the Rust implementation in normal development, even though the Rust surface is canonical.
