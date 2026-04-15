//! Workspace fingerprinting using FNV-1a hash of canonical path.
//!
//! This module provides deterministic workspace identification by computing
//! a hash of the canonical (symlink-resolved, absolute) path. The same
//! physical directory always produces the same fingerprint regardless of
//! how it was referenced (relative path, symlink, etc.).

use fnv::FnvHasher;
use std::hash::{Hash, Hasher};
use std::path::Path;
use std::sync::OnceLock;

static CANONICALIZE_ERROR_MESSAGE: OnceLock<String> = OnceLock::new();

/// Computes a workspace fingerprint using FNV-1a hash of the canonical path.
///
/// The fingerprint is deterministic: the same physical directory always produces
/// the same fingerprint regardless of how it was referenced (relative path,
/// symlink, absolute path, etc.).
///
/// # Arguments
///
/// * `path` - A path to the workspace (can be relative, absolute, or symlink)
///
/// # Returns
///
/// * `Ok(u64)` - The FNV-1a hash of the canonical path
/// * `Err(String)` - If the path cannot be canonicalized
pub fn workspace_fingerprint(path: impl AsRef<Path>) -> Result<u64, String> {
    let canonical = canonicalize_path(path.as_ref())?;
    let hash = compute_fnv1a(canonical.as_bytes());
    Ok(hash)
}

/// Canonicalizes a path, resolving symlinks and converting to absolute path.
///
/// Uses the OS's canonicalize functionality which:
/// - Resolves symlinks
/// - Removes redundant path components (`.`, `..`)
/// - Makes the path absolute
fn canonicalize_path(path: &Path) -> Result<String, String> {
    std::fs::canonicalize(path)
        .map(|p| p.to_string_lossy().to_string())
        .map_err(|e| {
            CANONICALIZE_ERROR_MESSAGE
                .get_or_init(|| {
                    format!(
                        "Failed to canonicalize path: {}. Ensure the path exists and is accessible.",
                        e
                    )
                })
                .clone()
        })
}

/// Computes FNV-1a hash of the given bytes.
///
/// FNV-1a is a fast, non-cryptographic hash function that is well-suited
/// for producing deterministic fingerprints from byte sequences.
fn compute_fnv1a(data: &[u8]) -> u64 {
    let mut hasher = FnvHasher::default();
    Hash::hash(data, &mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_same_directory_via_different_paths_produces_same_fingerprint() {
        let temp_dir = TempDir::new().unwrap();
        let real_path = temp_dir.path();

        // Create a symlink to the real directory
        let symlink_path = temp_dir.path().join("symlink_to_dir");
        std::os::unix::fs::symlink(real_path, &symlink_path).unwrap();

        // Fingerprint via real path
        let fp_real = workspace_fingerprint(real_path).unwrap();

        // Fingerprint via symlink path - should be identical
        let fp_symlink = workspace_fingerprint(&symlink_path).unwrap();

        assert_eq!(
            fp_real, fp_symlink,
            "Symlink and real path should produce the same fingerprint"
        );
    }

    #[test]
    fn test_different_directories_produce_different_fingerprints() {
        let temp_dir1 = TempDir::new().unwrap();
        let temp_dir2 = TempDir::new().unwrap();

        let fp1 = workspace_fingerprint(temp_dir1.path()).unwrap();
        let fp2 = workspace_fingerprint(temp_dir2.path()).unwrap();

        assert_ne!(
            fp1, fp2,
            "Different directories should produce different fingerprints"
        );
    }

    #[test]
    fn test_fingerprint_is_u64() {
        let temp_dir = TempDir::new().unwrap();
        let fp = workspace_fingerprint(temp_dir.path()).unwrap();

        // Verify it's a valid u64 (should always be true for our implementation)
        let _ = fp as u64;
    }

    #[test]
    fn test_relative_path_resolves_correctly() {
        let temp_dir = TempDir::new().unwrap();
        let original_cwd = std::env::current_dir().unwrap();

        std::env::set_current_dir(temp_dir.path()).unwrap();

        // Create a subdirectory
        let subdir = temp_dir.path().join("subdir");
        std::fs::create_dir(&subdir).unwrap();

        // Relative path from cwd
        let fp_relative = workspace_fingerprint("subdir").unwrap();

        // Absolute path
        let fp_absolute = workspace_fingerprint(&subdir).unwrap();

        // Restore original cwd
        std::env::set_current_dir(original_cwd).unwrap();

        assert_eq!(
            fp_relative, fp_absolute,
            "Relative and absolute paths to same dir should produce same fingerprint"
        );
    }

    #[test]
    fn test_nonexistent_path_returns_error() {
        let result = workspace_fingerprint("/nonexistent/path/that/does/not/exist");
        assert!(result.is_err(), "Nonexistent path should return an error");
        assert!(
            result.unwrap_err().contains("canonicalize"),
            "Error message should mention canonicalization failure"
        );
    }

    #[test]
    fn test_fingerprint_deterministic() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path();

        let fp1 = workspace_fingerprint(path).unwrap();
        let fp2 = workspace_fingerprint(path).unwrap();

        assert_eq!(
            fp1, fp2,
            "Same path should always produce same fingerprint"
        );
    }

    #[test]
    fn test_nested_symlinks_resolved() {
        let temp_dir = TempDir::new().unwrap();
        let real_path = temp_dir.path().join("real_dir");
        std::fs::create_dir(&real_path).unwrap();

        // Create first level symlink
        let link1 = temp_dir.path().join("link1");
        std::os::unix::fs::symlink(&real_path, &link1).unwrap();

        // Create second level symlink pointing to first symlink
        let link2 = temp_dir.path().join("link2");
        std::os::unix::fs::symlink(&link1, &link2).unwrap();

        let fp_real = workspace_fingerprint(&real_path).unwrap();
        let fp_link1 = workspace_fingerprint(&link1).unwrap();
        let fp_link2 = workspace_fingerprint(&link2).unwrap();

        assert_eq!(
            fp_real, fp_link1,
            "First level symlink should resolve to real path"
        );
        assert_eq!(
            fp_real, fp_link2,
            "Nested symlinks should all resolve to same real path"
        );
    }
}
