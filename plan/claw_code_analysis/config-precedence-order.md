# Config Precedence Order

Config discovery in ClawCode is an ordered, layered process. The runtime loads multiple config files in a fixed sequence and merges them field-by-field, so **later entries override earlier ones**. The chain spans user-level legacy files, user-level modern files, project-level legacy files, project-level modern files, and finally machine-local project overrides.

This document covers the exact discovery paths, the merge semantics, and the test that validates the precedence behavior.

## Discovery chain

`ConfigLoader::discover()` (in `runtime/src/config.rs`) enumerates five paths in load order:

| # | Source | File path |
|---|---|---|
| 1 | User (legacy) | `~/.claw.json` |
| 2 | User | `~/.claw/settings.json` |
| 3 | Project (legacy) | `<repo>/.claw.json` |
| 4 | Project | `<repo>/.claw/settings.json` |
| 5 | Local (machine-local override) | `<repo>/.claw/settings.local.json` |

`ConfigSource` carries three variants — `User`, `Project`, `Local` — corresponding to global user settings, shared project settings, and machine-specific overrides respectively.

### Default config home

`default_config_home()` derives the user config directory from:

1. `CLAW_CONFIG_HOME` env var if set, otherwise
2. `$HOME/.claw`, otherwise
3. `.claw` (relative to cwd)

This means `~/.claw.json` resolves to `default_config_home().parent()/.claw.json` — the legacy compat file lives next to the config directory, not inside it.

### Legacy compat behavior

The legacy `~/.claw.json` and `<repo>/.claw.json` files use the top-level `.claw.json` shape rather than the nested `.claw/settings.json` shape. `read_optional_json_object()` silently skips a missing legacy file rather than failing, so the absence of a legacy file is not an error.

## Override semantics

`deep_merge_objects()` in `runtime/src/config.rs` implements field-by-field deep merging: when a key exists in both the accumulated config and the incoming file, and both values are JSON objects, the merge recurses. Otherwise the incoming value replaces the existing one entirely.

The load loop in `ConfigLoader::load()` iterates the discovery list in order, parses each file, and calls `deep_merge_objects()` on the accumulated `merged` map. This means **each subsequent file overrides the preceding ones**, both at the top level and within nested objects.

For example, if `~/.claw/settings.json` sets `"model": "sonnet"` and `<repo>/.claw/settings.local.json` sets `"model": "opus"`, the final effective model is `"opus"`. The test `loads_and_merges_claude_code_config_files_by_precedence` validates exactly this: it writes all five files with different model values and asserts that the last-loaded entry wins.

## Merge affects more than scalar keys

The merged config propagates through `RuntimeFeatureConfig`, which is parsed after the raw merge. Feature sections affected include:

- **`hooks`** (`PreToolUse`, `PostToolUse`, `PostToolUseFailure`) — hook command lists are merged via `RuntimeHookConfig::extend()`, which appends unique entries from later configs without duplicating earlier ones.
- **`mcpServers`** — MCP server definitions are collected per-scope in a `BTreeMap<String, ScopedMcpServerConfig>` using scope-aware insertion, so later configs can add or replace individual servers.
- **`oauth`** — OAuth client configuration.
- **`aliases`** — user-defined model aliases.
- **`permissions`** — permission mode and allow/deny/ask rules.
- **`sandbox`** — filesystem isolation, namespace restrictions, network isolation.
- **`providerFallbacks`** — primary and ordered fallback chain.
- **`trustedRoots`** — TLS trust anchor paths.

The test `loads_and_merges_claude_code_config_files_by_precedence` exercises hooks, permissions, and MCP servers across the precedence chain and asserts that later entries win for each section.

## Machine-local override: `settings.local.json`

`<repo>/.claw/settings.local.json` is the machine-local override layer. It sits above shared project config in the precedence chain and is never intended for sharing. The file is discovered via the same `discover()` mechanism — it is not a special case but simply the last entry in the chain.

The `init.rs` guidance in the CLI crate surfaces this distinction to users so they understand `.claw/settings.json` is a team-shared file while `.claw/settings.local.json` is machine-private.

## Test-backed evidence

The test `loads_and_merges_claude_code_config_files_by_precedence` in `runtime/src/config.rs` is the canonical proof of precedence behavior. It:

1. Writes all five config files with deliberately conflicting values (different models, env vars, hooks, permissions, MCP servers).
2. Loads via `ConfigLoader::new(&cwd, &home).load()`.
3. Asserts `loaded.loaded_entries().len() == 5` — all five files were found and loaded.
4. Asserts the final `model` value is `"opus"` (the local override).
5. Asserts `permission_mode` resolves to `WorkspaceWrite` (from the local override).
6. Asserts `env` has four entries (accumulated from all files, not overwritten entirely).
7. Asserts hooks are accumulated: `PreToolUse` from user settings wins over nothing, `PostToolUse` from project settings wins over nothing, and `PostToolUseFailure` from project settings wins over nothing.
8. Asserts both MCP servers from user and project configs survive the merge.

This test is concrete evidence for **VAL-CONFIG-001** (full chain in order), **VAL-CONFIG-002** (later overrides earlier), and **VAL-CONFIG-005** (test-backed claims).

## Builder lesson

The design pattern here is **layered file discovery with deep object merge**. Instead of picking one config file or shallow-merging top-level keys, the runtime:

1. Enumerates a fixed, ordered list of paths.
2. Parses each file as JSON.
3. Recursively merges objects so nested values accumulate rather than replacing entire sections.
4. Records which source each merged entry came from via `loaded_entries` and scope tagging on per-feature configs.

A builder reusing this pattern should expose the `loaded_entries` list so callers can audit which files contributed to the final config, and should preserve the distinction between `User` (global), `Project` (shared), and `Local` (machine-private) scopes so override intent remains clear.
