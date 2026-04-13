# Workspace-Write Boundaries

Workspace-write boundaries are the first line of defense for file operations in ClawCode. The system permits writes only inside a designated workspace root and denies operations that escape that boundary — but write safety depends on two distinct enforcement layers working together: the **permission-mode gate** in `PermissionEnforcer` and the **canonical path validation** in `file_ops.rs`. Neither layer is sufficient alone.

## The Two Enforcement Layers

### Permission-mode gate (`PermissionEnforcer::check_file_write`)

`PermissionEnforcer::check_file_write()` in `rust/crates/runtime/src/permission_enforcer.rs` is the first check applied when a file-write tool is invoked. It evaluates the active `PermissionMode` against the requested operation:

| Active mode | `check_file_write` behavior |
|---|---|
| `ReadOnly` | Always denied — file writes are not permitted |
| `WorkspaceWrite` | Allowed only if `is_within_workspace(path, workspace_root)` returns true |
| `Allow` | Always allowed |
| `DangerFullAccess` | Always allowed |
| `Prompt` | Always denied without an interactive prompter present |

When `WorkspaceWrite` mode is active, the gate uses `is_within_workspace()` — a **string-prefix check**:

```rust
fn is_within_workspace(path: &str, workspace_root: &str) -> bool {
    let normalized = if path.starts_with('/') {
        path.to_owned()
    } else {
        format!("{workspace_root}/{path}")
    };
    let root = if workspace_root.ends_with('/') {
        workspace_root.to_owned()
    } else {
        format!("{workspace_root}/")
    };
    normalized.starts_with(&root) || normalized == workspace_root.trim_end_matches('/')
}
```

This resolves relative paths against the workspace root and verifies the result starts with the root prefix. However, it is a purely syntactic check — it does not resolve symlinks or canonicalize paths.

### Canonical path validation (`file_ops.rs`)

`file_ops.rs` in `rust/crates/runtime/src/` performs the second, deeper enforcement. Every write path that passes the permission-mode gate still goes through canonical validation before I/O occurs:

```rust
fn validate_workspace_boundary(resolved: &Path, workspace_root: &Path) -> io::Result<()> {
    if !resolved.starts_with(workspace_root) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "path {} escapes workspace boundary {}",
                resolved.display(),
                workspace_root.display()
            ),
        ));
    }
    Ok(())
}
```

The key difference from the permission-layer check: `file_ops` calls `Path::canonicalize()` before the starts-with comparison. This means `../` traversal and symlinks are resolved to their real paths first, and any path that resolves outside the canonical workspace root is rejected.

Additionally, `is_symlink_escape()` explicitly detects symlinks that point outside the workspace:

```rust
pub fn is_symlink_escape(path: &Path, workspace_root: &Path) -> io::Result<bool> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_symlink() {
        return Ok(false);
    }
    let resolved = path.canonicalize()?;
    let canonical_root = workspace_root.canonicalize().unwrap_or_else(|_| workspace_root.to_path_buf());
    Ok(!resolved.starts_with(&canonical_root))
}
```

The `_in_workspace` variants of read/write/edit all chain canonical boundary validation:

- `read_file_in_workspace()`
- `write_file_in_workspace()`
- `edit_file_in_workspace()`

These are the functions actually called by the tool execution layer.

## Why Two Layers Are Needed

The permission-mode gate decides **whether the session's mode permits writes at all** and provides fast denial without filesystem access. It can reject a write in `ReadOnly` mode immediately, without checking whether the path actually exists or whether it resolves inside the workspace.

The file-ops layer decides **whether the path is safe to operate on** even if the mode permits writes. It catches cases where a path that appears to be inside the workspace actually resolves outside — through `../` traversal, symlinks, or path normalization edge cases.

Together, they cover two distinct failure modes:

1. **Mode insufficient** — the session mode does not permit writes at all (`ReadOnly`, `Prompt` without a prompter). Handled by `PermissionEnforcer::check_file_write()`.
2. **Path escaped** — the path appears to be inside the workspace but resolves outside after canonicalization. Handled by `validate_workspace_boundary()` and `is_symlink_escape()` in `file_ops.rs`.

If only the permission-layer check existed, a symlink like `workspace/link-to-../../etc/passwd` could pass the string-prefix test but actually resolve outside the workspace. If only the file-ops layer existed, every write operation would need to perform canonicalization and symlink checks even when the session is already in `ReadOnly` mode — wasteful and slower at the common-case denial.

## Additional File-Safety Guardrails

Beyond the boundary layers, `file_ops.rs` includes independent safety limits that apply regardless of mode:

- **`MAX_READ_SIZE` (10 MB)** — files larger than this are rejected before content is read
- **`MAX_WRITE_SIZE` (10 MB)** — writes exceeding this are rejected before content is written
- **Binary detection** — files containing NUL bytes in their first 8 KB are rejected as binary, not returned as text

These limits are enforced inside the `read_file()`, `write_file()`, and `edit_file()` functions independently of any boundary checks, and they apply even when called through the `_in_workspace` variants.

## Parity Evidence

The write-denial and write-allow scenarios are captured in the mock parity harness:

- `write_file_allowed` — validates that a write to a path inside the workspace succeeds
- `write_file_denied` — validates that a write to a path outside the workspace is denied

These scenarios are defined in `rust/mock_parity_scenarios.json` and exercised by `rust/crates/rusty-claude-cli/tests/mock_parity_harness.rs`. Lane 3 ("File-tool") in `PARITY.md` documents the corresponding evidence:

> `rust/crates/runtime/src/file_ops.rs` is **744 LOC** and now includes `MAX_READ_SIZE`, `MAX_WRITE_SIZE`, NUL-byte binary detection, and canonical workspace-boundary validation.

The permission enforcement layer (Lane 9) adds on top of this:

> `PermissionEnforcer::check()` delegates to `PermissionPolicy::authorize()` and returns structured allow/deny results. `check_file_write()` enforces workspace boundaries and read-only denial; `check_bash()` denies mutating commands in read-only mode and blocks prompt-mode bash without confirmation.

## Builder Lessons

1. **Two enforcement points are safer than one.** A purely syntactic check (string prefix) is fast but can be fooled by path traversal or symlinks. A canonical check catches those cases but requires filesystem access. Layering them gets fast common-case denials plus safe edge-case detection.

2. **Permission mode and path safety are separate concerns.** `ReadOnly` vs `WorkspaceWrite` is about what the session is allowed to do. Whether a specific path is inside or outside the workspace is a property of the filesystem. Confusing these leads to either slow code (checking canonical paths for every read in read-only mode) or unsafe code (trusting string-prefix checks for write safety).

3. **Symlinks deserve explicit treatment.** A simple canonicalization check covers `../` escapes but does not automatically handle symlinks pointing outside. `is_symlink_escape()` handles this case explicitly by checking `symlink_metadata` before following the link.

4. **Canonical validation must run after path construction.** When a relative path is resolved against the current working directory and then passed to canonical validation, both steps must happen in the right order. `normalize_path()` and `normalize_path_allow_missing()` in `file_ops.rs` handle this for reads and writes respectively.

5. **Denials should be explicit tool-result failures, not silent drops.** When a write is denied at either layer, the result is a structured `EnforcementResult::Denied` payload that includes the tool name, active mode, required mode, and a human-readable reason. This flows back into the conversation transcript so the model can reason about the denial rather than retrying blindly.
