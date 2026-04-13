# File-Operation Guardrails in ClawCode

File-operation guardrails sit below ClawCode's permission mode system and enforce specific safety guarantees on every file read, write, and edit that passes through the runtime. Where `PermissionEnforcer` answers "is this tool allowed given the current mode?", the `file_ops` module answers "is this specific path and content safe to touch right now?" The guardrails exist because permission modes alone are insufficient — a `WorkspaceWrite` session can still be tricked into writing outside the workspace via `../` traversal, symlink chains, or oversized payloads.

The canonical source is `references/claw-code/rust/crates/runtime/src/file_ops.rs` (744 LOC including tests).

## Canonical Boundary Validation

Before any file operation, the path is resolved to its canonical form via `canonicalize()`. This collapses `..` segments, resolves relative paths against the process CWD, and follows symlinks to their final destination. The resolved path is then checked against the canonical workspace root:

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

The canonical workspace root is itself computed via `canonicalize()` at the point where the workspace is bound. This means that even if a file is reached through a relative path like `../../etc/passwd`, the resolved absolute path is what gets checked — the `../` segments cannot escape because the check happens after resolution, not before.

This validation is applied in `read_file_in_workspace`, `write_file_in_workspace`, and `edit_file_in_workspace`, which are the workspace-aware wrappers around the base operations. Each wrapper first canonicalizes the workspace root, calls `validate_workspace_boundary` on the resolved absolute path, and only then delegates to the base operation.

**Builder lesson:** Canonicalization order matters. The check must happen after `canonicalize()` is applied to the path, not before. A naive string-prefix check on a raw relative path is bypassable via `../`; the guard only works because both sides of the comparison are canonicalized.

## Symlink Escape Prevention

The `is_symlink_escape` function detects symlinks that resolve outside the workspace:

```rust
pub fn is_symlink_escape(path: &Path, workspace_root: &Path) -> io::Result<bool> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_symlink() {
        return Ok(false);
    }
    let resolved = path.canonicalize()?;
    let canonical_root = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.to_path_buf());
    Ok(!resolved.starts_with(&canonical_root))
}
```

The distinction between `symlink_metadata` (reads the symlink itself) and `metadata` (follows the symlink) is deliberate. A symlink that points inside the workspace is fine; a symlink that points outside must be flagged. The function checks `is_symlink()` first, so non-symlink files are cheap — no resolution needed.

`is_symlink_escape` is exported and used in tests but the primary defense is `validate_workspace_boundary` combined with `canonicalize()` on the resolved path. When `canonicalize()` follows a symlink, the final resolved path is what gets checked against the workspace root. If the symlink target is outside the workspace, `canonicalize()` returns a path that fails `starts_with`, so the escape is blocked at the canonicalization step.

The test suite covers this explicitly:

```rust
#[test]
fn detects_symlink_escape() {
    // symlink pointing outside workspace → detected
    // non-symlink file → not an escape
}
```

**Builder lesson:** Symlink checks require `symlink_metadata`, not `metadata`. Using the wrong one will follow the link when you only want to inspect it.

## Binary Detection

Binary files are detected before they are read or returned to the model. The `is_binary_file` function reads the first 8192 bytes and checks for NUL bytes:

```rust
fn is_binary_file(path: &Path) -> io::Result<bool> {
    use std::io::Read;
    let mut file = fs::File::open(path)?;
    let mut buffer = [0u8; 8192];
    let bytes_read = file.read(&mut buffer)?;
    Ok(buffer[..bytes_read].contains(&0))
}
```

Text files in typical source repositories almost never contain NUL bytes. A NUL in the first 8 KB is a reliable signal of a binary format (PNG, ELF, compiled object, etc.). When binary detection triggers, `read_file` returns an `io::Error` with kind `InvalidData` and the message "file appears to be binary".

The detection is a pre-check before the size check in `read_file`:

```rust
// Check file size before reading
let metadata = fs::metadata(&absolute_path)?;
if metadata.len() > MAX_READ_SIZE {
    return Err(io::Error::new(
        io::ErrorKind::InvalidData,
        format!(
            "file is too large ({} bytes, max {} bytes)",
            metadata.len(),
            MAX_READ_SIZE
        ),
    ));
}

// Detect binary files
if is_binary_file(&absolute_path)? {
    return Err(io::Error::new(
        io::ErrorKind::InvalidData,
        "file appears to be binary",
    ));
}
```

The test for this:

```rust
#[test]
fn rejects_binary_files() {
    let path = temp_path("binary-test.bin");
    std::fs::write(&path, b"\x00\x01\x02\x03binary content").expect("write should succeed");
    let result = read_file(path.to_string_lossy().as_ref(), None, None);
    assert!(result.is_err());
    let error = result.unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    assert!(error.to_string().contains("binary"));
}
```

**Builder lesson:** Binary detection by NUL byte sampling is a simple heuristic that avoids content-type sniffing and works for the vast majority of binary formats without needing a magic number registry.

## Read and Write Size Limits

Two constants enforce explicit size caps:

```rust
/// Maximum file size that can be read (10 MB).
const MAX_READ_SIZE: u64 = 10 * 1024 * 1024;

/// Maximum file size that can be written (10 MB).
const MAX_WRITE_SIZE: usize = 10 * 1024 * 1024;
```

The read check uses `fs::metadata().len()` before attempting to read, so oversized files are rejected at the metadata check rather than mid-read. The write check is performed on `content.len()` before any filesystem write is initiated:

```rust
pub fn write_file(path: &str, content: &str) -> io::Result<WriteFileOutput> {
    if content.len() > MAX_WRITE_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "content is too large ({} bytes, max {} bytes)",
                content.len(),
                MAX_WRITE_SIZE
            ),
        ));
    }
    // ... proceeds to write
}
```

The test for write size enforcement:

```rust
#[test]
fn rejects_oversized_writes() {
    let path = temp_path("oversize-write.txt");
    let huge = "x".repeat(MAX_WRITE_SIZE + 1);
    let result = write_file(path.to_string_lossy().as_ref(), &huge);
    assert!(result.is_err());
    let error = result.unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    assert!(error.to_string().contains("too large"));
}
```

**Builder lesson:** Size checks should happen before I/O, not after. Checking `content.len()` before `fs::write` prevents wasted work and provides a fast-fail path with a precise error message.

## Interaction with Permission Modes

The `PermissionEnforcer` in `permission_enforcer.rs` implements `check_file_write`, which enforces workspace boundary checks at the permission layer:

```rust
pub fn check_file_write(&self, path: &str, workspace_root: &str) -> EnforcementResult {
    let mode = self.policy.active_mode();

    match mode {
        PermissionMode::ReadOnly => EnforcementResult::Denied { ... },
        PermissionMode::WorkspaceWrite => {
            if is_within_workspace(path, workspace_root) {
                EnforcementResult::Allowed
            } else {
                EnforcementResult::Denied { ... }
            }
        }
        PermissionMode::Allow | PermissionMode::DangerFullAccess => EnforcementResult::Allowed,
        PermissionMode::Prompt => EnforcementResult::Denied { ... },
    }
}
```

`WorkspaceWrite` mode allows writes only if the path is within the workspace root. The `is_within_workspace` helper does a simple string-prefix check that complements but does not replace the canonical path check in `file_ops.rs`:

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

This string-level check is a fast path for `PermissionEnforcer`. The `file_ops.rs` canonicalization check is the deeper enforcement that handles `..` traversal, symlinks, and edge cases that the prefix check misses.

The two layers are:

1. **Permission layer** (`PermissionEnforcer::check_file_write`) — fast string-prefix check, no I/O required, decides whether to even attempt the operation.
2. **File-ops layer** (`file_ops.rs` validate_workspace_boundary) — canonicalizes the path and validates against the canonical workspace root, handles symlinks and path traversal tricks.

**Builder lesson:** Permission-mode checks and filesystem-safety checks are separate concerns. The permission layer says "this mode only allows writes inside the workspace." The file-ops layer says "this specific path, resolved and canonicalized, is actually inside the workspace." Both must pass.

## Parity Evidence

The file-op guardrails are covered in the parity system:

- **Feature commit:** `284163b` — `feat(file_ops): add edge-case guards — binary detection, size limits, workspace boundary, symlink escape`
- **Merge commit:** `a98f2b6` — `Merge jobdori/file-tool-edge-cases: binary detection, size limits, workspace boundary guards`
- **Evidence:** `rust/crates/runtime/src/file_ops.rs` is 744 LOC and includes `MAX_READ_SIZE`, `MAX_WRITE_SIZE`, NUL-byte binary detection, and canonical workspace-boundary validation.
- **Harness coverage:** `read_file_roundtrip`, `grep_chunk_assembly`, `write_file_allowed`, and `write_file_denied` are in the scenario manifest and exercised by the clean-env harness. (`references/claw-code/PARITY.md`)

The older PARITY checklist (`references/claw-code/rust/PARITY.md`) also records these as completed checkpoints:

- [x] Path traversal prevention (symlink following, `../` escapes)
- [x] Size limits on read/write
- [x] Binary file detection

## Builder Takeaways

1. **Canonicalize before comparing.** String-prefix checks on raw paths are bypassable. Always canonicalize both the target path and the workspace root before comparing.

2. **Use `symlink_metadata` for symlink inspection, `metadata` for following.** Confusing these produces incorrect escape detection.

3. **NUL-byte sampling is a lightweight binary detector** that works without a magic number registry and avoids content-type sniffing.

4. **Size checks should precede I/O.** Checking `content.len()` before `fs::write` avoids wasted work; checking `metadata.len()` before reading avoids reading a large file into memory.

5. **Two enforcement layers are better than one.** The permission layer is a fast path; the canonical path check is the thorough enforcement. A builder implementing a similar system should not rely on the fast path alone.

6. **Test the escape paths.** The test suite has explicit coverage for symlink escapes, oversized writes, and binary file rejection — these are the paths an adversarial caller would try to exploit.
