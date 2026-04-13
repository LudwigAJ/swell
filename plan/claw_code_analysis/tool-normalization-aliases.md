# Tool Normalization and Aliases

How allowed-tool lists are normalized and how aliases are resolved so that policy and configuration can operate on a single canonical identity namespace.

**Artifact:** `analysis/tool-normalization-aliases.md`
**Skill:** `clawcode-doc-worker`
**Milestone:** tool-system
**Fulfills:** `VAL-TOOLS-004`
**Evidence:** `references/claw-code/rust/crates/tools/src/lib.rs` (`normalize_allowed_tools`, `normalize_tool_name`), `references/claw-code/rust/crates/rusty-claude-cli/src/main.rs`

---

## The Problem: Multiple Names, One Identity

Tool names can reach the registry through several paths:

- `--allowedTools read,glob` from the CLI
- `--allowedTools Read,Glob` with mixed case
- `--allowedTools read-file,glob-search` with hyphens
- `read` as a user-facing shorthand vs `read_file` as the canonical spec name

If policy and permission enforcement had to handle all these variations, the dispatch layer would need a proliferated name map everywhere. Instead, ClawCode normalizes all incoming tool identifiers to a single canonical form **once at the boundary**, and the rest of the system operates only on canonical names.

---

## Normalization Function

`normalize_tool_name()` is the shared primitive:

```rust
// rust/crates/tools/src/lib.rs
fn normalize_tool_name(value: &str) -> String {
    value.trim().replace('-', "_").to_ascii_lowercase()
}
```

This applies three transformations in order:

| Step | Example |
|------|---------|
| Trim leading/trailing whitespace | `" read_file "` → `"read_file"` |
| Replace hyphens with underscores | `"read-file"` → `"read_file"` |
| Fold to lowercase | `"Read_File"` → `"read_file"` |

This function is used both when building the alias lookup map and when resolving each token from user input.

---

## Alias Table

Built-in tools have user-facing aliases registered alongside the canonical name map:

```rust
// rust/crates/tools/src/lib.rs
for (alias, canonical) in [
    ("read", "read_file"),
    ("write", "write_file"),
    ("edit", "edit_file"),
    ("glob", "glob_search"),
    ("grep", "grep_search"),
] {
    name_map.insert(alias.to_string(), canonical.to_string());
}
```

The `name_map` is a `BTreeMap<String, String>` that maps **normalized forms** to **canonical names**. Both the alias and the canonical name are inserted under their normalized keys, so `read` and `Read_File` both resolve to `read_file`.

---

## `normalize_allowed_tools` Flow

`normalize_allowed_tools()` is the entry point for all allowed-tool policy resolution:

```rust
// rust/crates/tools/src/lib.rs
pub fn normalize_allowed_tools(
    &self,
    values: &[String],
) -> Result<Option<BTreeSet<String>>, String> {
    if values.is_empty() {
        return Ok(None);
    }

    // Build canonical name list from all three layers
    let canonical_names = builtin_specs_names
        .chain(plugin_tool_names)
        .chain(runtime_tool_names)
        .collect::<Vec<_>>();

    // Build normalized→canonical map including aliases
    let mut name_map = canonical_names
        .iter()
        .map(|name| (normalize_tool_name(name), name.clone()))
        .collect::<BTreeMap<_, _>>();

    for (alias, canonical) in [
        ("read", "read_file"),
        ("write", "write_file"),
        ("edit", "edit_file"),
        ("glob", "glob_search"),
        ("grep", "grep_search"),
    ] {
        name_map.insert(alias.to_string(), canonical.to_string());
    }

    // Resolve each input token
    let mut allowed = BTreeSet::new();
    for value in values {
        for token in value.split(|ch: char| ch == ',' || ch.is_whitespace()) {
            let normalized = normalize_tool_name(token);
            let canonical = name_map.get(&normalized).ok_or_else(|| {
                format!(
                    "unsupported tool in --allowedTools: {token} (expected one of: {})",
                    canonical_names.join(", ")
                )
            })?;
            allowed.insert(canonical.clone());
        }
    }

    Ok(Some(allowed))
}
```

The processing steps are:

1. **Empty check** — returns `Ok(None)` if no values provided, signaling no restrictions
2. **Canonical name collection** — gathers names from built-in specs, plugin tools, and runtime tools
3. **Normalized lookup map construction** — inserts all canonical names + aliases, keyed by their normalized form
4. **Token splitting** — each input string is split on commas and whitespace, so `"read,glob"` and `["read", "glob"]` are equivalent
5. **Canonical resolution** — each token is normalized, looked up, and the canonical name is inserted into the result set
6. **Error on unknown** — if a normalized token has no entry in `name_map`, an error listing all valid canonical names is returned

The result is a `BTreeSet<String>` of **canonical names only** — no aliases, no mixed case, no hyphens. This set is what policy checking and dispatch use throughout the session.

---

## Where Normalization Is Called

In the CLI entry point, `normalize_allowed_tools` is called once when processing `--allowedTools`:

```rust
// rust/crates/rusty-claude-cli/src/main.rs
fn normalize_allowed_tools(values: &[String]) -> Result<Option<AllowedToolSet>, String> {
    if values.is_empty() {
        return Ok(None);
    }
    current_tool_registry()?.normalize_allowed_tools(values)
}
```

The returned `AllowedToolSet` (a `BTreeSet<String>`) is stored in the runtime config and used to filter `GlobalToolRegistry::definitions()` when building the API request. The dispatch layer in `execute()` never sees aliases — it only receives canonical names like `read_file` and `write_file`.

---

## Why Policy Operates on Normalized Identities

Three practical reasons:

1. **Single dispatch table.** `execute()` uses a `match` on canonical names. Without normalization, every arm would need to handle `read`, `Read`, `READ`, `read-file`, `read_file` — doubling the code and the risk of missed cases.

2. **Permission metadata is per-canonical-name.** `mvp_tool_specs()` and `RuntimeToolDefinition` both key off canonical names. The permission lookup in `permission_specs()` would need a reverse-alias map if inputs were not normalized first.

3. **Error messages are deterministic.** When a user types an invalid tool name, the error cites canonical names only. This keeps error messages stable regardless of which alias a user happened to try.

---

## Error Case: Unknown Tool

If a normalized token has no entry in the lookup map, normalization fails with a descriptive error:

```
unsupported tool in --allowedTools: rip (expected one of: bash, read_file, write_file, ...)
```

The error lists all valid canonical names so the user can correct their input. The error does not list aliases — only canonical names appear in error messages.

---

## Builder Lessons

1. **Normalize once at the boundary.** `normalize_tool_name()` is a pure function with no side effects. Calling it at input ingestion time (CLI argument parsing, config loading) means every downstream function receives a consistent format and never needs to re-normalize.

2. **Aliases map to one canonical name.** The alias table is intentionally one-directional: `read` → `read_file`, not the reverse. This prevents ambiguity in dispatch and keeps the lookup map simple.

3. **Token splitting is forgiving.** Splitting on both commas and whitespace means `read,glob` and `read glob` are equivalent. This is a usability win — users do not need to remember the exact separator style.

4. **The normalized set is the policy surface.** `normalize_allowed_tools()` returns a `BTreeSet` of canonical names. This set is what gets stored in session config and checked by `definitions()`. Adding a new alias means updating one table in `normalize_allowed_tools()` and nowhere else.

5. **Unknown tools fail fast.** The `ok_or_else` in the resolution loop means a single unknown token causes the whole call to fail, rather than silently ignoring it. This prevents misconfiguration where a user thinks they allowed a tool but spelled it incorrectly.

---

## Key Files

| File | Role |
|------|------|
| `rust/crates/tools/src/lib.rs` | `normalize_tool_name()`, `normalize_allowed_tools()`, alias table |
| `rust/crates/rusty-claude-cli/src/main.rs` | CLI entry point calling `normalize_allowed_tools()` |
