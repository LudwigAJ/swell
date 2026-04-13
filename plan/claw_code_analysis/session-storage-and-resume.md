# Session Storage and Resume

Workspace-local session storage, workspace-hash namespacing, latest/last/recent aliases, managed IDs, explicit paths, legacy compatibility, workspace mismatch protection, and recency ordering semantics.

## Overview

`SessionStore` in `runtime/src/session_control.rs` is the owning type for all managed session persistence. It is constructed either from the current working directory (`.claw/sessions/<workspace_hash>/`) or from an explicit `--data-dir` flag. Both paths partition on a stable FNV-1a fingerprint of the canonical workspace root, so parallel `claw serve` instances on the same machine never collide even when they share a parent directory.

## Workspace-Local Storage Layout

```text
<cwd>/.claw/sessions/<workspace_hash>/
  <session-id>.jsonl        ← primary session format (JSONL)
  <session-id>.json          ← legacy session format
```

The layout is intentionally per-workspace, not global. When a session is created or resumed, the store is rooted at the workspace's `.claw/sessions/` directory. An explicit `--data-dir` flag redirects the store root but still namespaces by workspace fingerprint:

```text
<data_dir>/sessions/<workspace_hash>/
  <session-id>.jsonl
```

Evidence: `SessionStore::from_cwd` and `SessionStore::from_data_dir` constructors in `session_control.rs` (lines 28–55).

## Workspace Hash Namespacing

`workspace_fingerprint` derives a 16-character hex string from the canonical workspace root using FNV-1a (64-bit). This fingerprint is stable across process restarts and is the leaf directory name in the session store path. Two different workspace roots on the same machine produce different fingerprints, guaranteeing isolation.

```rust
pub fn workspace_fingerprint(workspace_root: &Path) -> String {
    let input = workspace_root.to_string_lossy();
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in input.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}")
}
```

Evidence: `workspace_fingerprint` function and test `workspace_fingerprint_is_deterministic_and_differs_per_path` in `session_control.rs` (lines 156–168 and lines 303–316). The test confirms identical paths produce identical fingerprints, different paths produce different fingerprints, and the output is always 16 characters.

The `--data-dir` path supports multi-tenant or CI scenarios where sessions are stored in a shared data directory but still partitioned by workspace.

## Resume Reference Forms

`SessionStore::resolve_reference` accepts three distinct reference shapes:

| Reference form | Example | Resolution strategy |
|---|---|---|
| Alias | `latest`, `last`, `recent` | Delegates to `latest_session()` |
| Explicit path | `/abs/path/session.jsonl` or `relative/path.jsonl` | Used directly if the path exists |
| Managed ID | `session-id-value` (no extension, no path separators) | Passed to `resolve_managed_path` which tries `.jsonl` then `.json` |

Evidence: `resolve_reference` in `session_control.rs` (lines 61–84). The alias check via `is_session_reference_alias` uses a case-insensitive match against `SESSION_REFERENCE_ALIASES` (`&["latest", "last", "recent"]`). Explicit path detection checks for a file extension or multi-component path — if either is present and the file does not exist, an error is returned rather than falling through to managed ID resolution.

## Managed ID Resolution and Legacy Compatibility

`resolve_managed_path` tries the primary extension `.jsonl` first, then the legacy extension `.json`. If neither file exists, it falls back to scanning the legacy sessions root (the parent of `sessions/` in the old layout). Legacy sessions that lack a `workspace_root` binding are loaded only if they are located within the current workspace directory — this prevents accidentally loading a session from a different project.

Evidence: `resolve_managed_path` in `session_control.rs` (lines 86–106), and the tests `session_store_loads_safe_legacy_session_from_same_workspace` and `session_store_loads_unbind_legacy_session_from_same_workspace` (lines 413–447).

## Workspace Mismatch Protection

`validate_loaded_session` checks that a loaded session's stored workspace root matches the current store's workspace root. The comparison uses `fs::canonicalize` on both paths before comparison, so symlinks and path normalization do not cause false positives. If the workspace roots do not match, a `SessionControlError::WorkspaceMismatch` is returned with both paths in the error message.

Evidence: `validate_loaded_session` in `session_control.rs` (lines 128–145) and the dedicated test `session_store_rejects_legacy_session_from_other_workspace` (lines 395–412). The test constructs a session in workspace A and attempts to load it through a store rooted in workspace B, asserting a `WorkspaceMismatch` error with `expected == workspace_b` and `actual == workspace_a`.

```rust
fn validate_loaded_session(
    &self,
    session_path: &Path,
    session: &Session,
) -> Result<(), SessionControlError> {
    let Some(actual) = session.workspace_root() else {
        // Unbound legacy sessions load only if they live inside the workspace
        if path_is_within_workspace(session_path, &self.workspace_root) {
            return Ok(());
        }
        return Err(SessionControlError::Format(
            format_legacy_session_missing_workspace_root(session_path, &self.workspace_root),
        ));
    };
    if workspace_roots_match(actual, &self.workspace_root) {
        return Ok(());
    }
    Err(SessionControlError::WorkspaceMismatch {
        expected: self.workspace_root.clone(),
        actual: actual.to_path_buf(),
    })
}
```

## Recency Ordering Semantics

`sort_managed_sessions` defines the ordering used by `latest_session()` and `list_sessions()`. It sorts by three keys in priority order:

1. **`updated_at_ms`** (session-level semantic timestamp) — descending
2. **`modified_epoch_millis`** (filesystem mtime) — descending, as a tiebreaker when `updated_at_ms` is equal
3. **`id`** (lexicographic session ID) — descending, final tiebreaker for deterministic output

This means that even if a session file is touched on disk after another, the session that was last updated semantically (via `updated_at_ms` written by the runtime during message appends) wins for the purposes of `latest` alias resolution. Filesystem ordering alone is not the primary signal.

Evidence: `sort_managed_sessions` in `session_control.rs` (lines 180–185), and the test `latest_session_prefers_semantic_updated_at_over_file_mtime` (lines 318–340) which constructs two summaries with `updated_at_ms: 200` (older-file-newer-session) and `updated_at_ms: 100` (newer-file-older-session), asserts the one with higher semantic timestamp sorts first regardless of filesystem mtime.

## Autosave and Session Browsing

The REPL autosaves every turn to `.claw/sessions/<session-id>.jsonl`. Users can inspect saved sessions via `/session list`, which calls `SessionStore::list_sessions()` and renders the returned `ManagedSessionSummary` entries. `/resume latest` and `/resume <session-id>` are the primary resume entry points.

Evidence: `render_resume_usage` in `main.rs` (lines 1266–1271) which states: `Auto-save .claw/sessions/<session-id>.jsonl` and `Tip use /session list to inspect saved sessions`. The `LATEST_SESSION_REFERENCE` and `SESSION_REFERENCE_ALIASES` constants are defined in `main.rs` and used throughout the CLI argument parser for `--resume` handling.

## Builder Lessons

**Partition on workspace fingerprint for multi-tenant safety.** Using FNV-1a over the canonical workspace path is a lightweight, zero-configuration way to prevent session collisions. No centralized registry is needed; the filesystem layout itself enforces isolation.

**Aliases + managed IDs + explicit paths are three separate resolution paths, not one.** Blurring these creates UX confusion. `resolve_reference` checks for an alias first, then checks whether the input looks like a path, then falls back to managed ID — this ordering is intentional so that `/resume latest` resolves to the latest session rather than a file named `latest.jsonl` if one happened to exist.

**Semantic timestamps beat filesystem metadata for "most recent" semantics.** `updated_at_ms` reflects when the session was last modified by the runtime, not when the file was last touched by the OS. This distinction matters for correctness in CI environments where filesystem clocks can be unreliable or shared NFS volumes can introduce metadata skew.

**Workspace mismatch errors carry both the expected and actual path.** This makes debugging much faster when a session is accidentally loaded from the wrong workspace — the error message names both sides directly rather than requiring the user to trace through logs.

**Legacy compatibility is opt-in via extension fallback and unbound session loading.** The `.json` extension fallback and the unbound-legacy-session loading path (`path_is_within_workspace`) ensure that sessions created with older versions of the tool can still be loaded, but only when they are within the current workspace boundary. This prevents legacy sessions from silently leaking context from a different project.
