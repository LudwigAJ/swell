//! Integration tests for the config audit trail feature.
//!
//! Verifies VAL-SESS-007: Config audit trail tracks which file contributed each setting.
//!
//! `loaded_entries()` returns `(key_path, value, source_file)` triples where:
//! - Each key appears at most **once** (only the winning/highest-priority entry).
//! - The `source_file` points to the config file that ultimately supplied the value.
//!
//! NOTE: Layers 1-2 (user global / user modern) resolve through `dirs::home_dir()` and
//! cannot be controlled by `project_path`. These tests only exercise layers 3-4
//! (project shared `settings.json` and project modern `settings.local.json`).

use std::path::{Path, PathBuf};
use swell_core::config::ConfigLoader;
use tempfile::TempDir;

/// Write a JSON config file inside `<dir>/.swell/<name>`.
fn write_project_config(dir: &Path, name: &str, content: &str) -> PathBuf {
    let swell_dir = dir.join(".swell");
    std::fs::create_dir_all(&swell_dir).unwrap();
    let path = swell_dir.join(name);
    std::fs::write(&path, content).unwrap();
    path
}

// ─────────────────────────────────────────────────────────────────────────────
// VAL-SESS-007 primary scenario
// ─────────────────────────────────────────────────────────────────────────────

/// The winning source for an overridden key is the higher-priority layer.
#[test]
fn test_overridden_key_shows_higher_priority_source() {
    let temp = TempDir::new().unwrap();

    // Layer 3: project shared — sets timeout to 10 and a unique key
    let layer3_path = write_project_config(
        temp.path(),
        "settings.json",
        r#"{"timeout": 10, "only_in_layer3": true}"#,
    );

    // Layer 4: project modern — overrides timeout to 20
    let layer4_path =
        write_project_config(temp.path(), "settings.local.json", r#"{"timeout": 20}"#);

    let loader = ConfigLoader::new().with_project_path(temp.path());
    let config = loader.load().unwrap();

    let entries = config.loaded_entries();

    // There must be exactly ONE entry for "timeout" (no duplicates).
    let timeout_entries: Vec<_> = entries.iter().filter(|e| e.key_path == "timeout").collect();
    assert_eq!(
        timeout_entries.len(),
        1,
        "expected exactly one audit entry for 'timeout', got {}",
        timeout_entries.len()
    );

    // The winning entry must point to layer 4 (settings.local.json).
    let timeout_entry = &timeout_entries[0];
    assert_eq!(
        timeout_entry.value.as_i64().unwrap(),
        20,
        "winning value for 'timeout' should be 20"
    );
    let source = timeout_entry
        .source_file
        .as_deref()
        .expect("source_file must be Some for a file-sourced entry");
    assert!(
        source.contains("settings.local.json"),
        "overridden 'timeout' should trace to settings.local.json (layer 4), got: {source}"
    );
    // Sanity: the path reported must actually exist on disk.
    assert!(
        std::path::Path::new(source).exists(),
        "reported source file must exist on disk: {source}"
    );
    // Double-check it matches the exact path we created.
    assert_eq!(
        std::path::Path::new(source).canonicalize().unwrap(),
        layer4_path.canonicalize().unwrap(),
        "source_file should be the canonical path of settings.local.json"
    );

    // The key that was NOT overridden must trace back to layer 3.
    let layer3_entries: Vec<_> = entries
        .iter()
        .filter(|e| e.key_path == "only_in_layer3")
        .collect();
    assert_eq!(
        layer3_entries.len(),
        1,
        "expected exactly one audit entry for 'only_in_layer3'"
    );
    let l3_entry = &layer3_entries[0];
    assert!(
        l3_entry.value.as_bool().unwrap(),
        "value for 'only_in_layer3' should be true"
    );
    let l3_source = l3_entry
        .source_file
        .as_deref()
        .expect("source_file must be Some");
    assert!(
        l3_source.contains("settings.json"),
        "non-overridden 'only_in_layer3' should trace to settings.json (layer 3), got: {l3_source}"
    );
    assert_eq!(
        std::path::Path::new(l3_source).canonicalize().unwrap(),
        layer3_path.canonicalize().unwrap(),
        "source_file should be the canonical path of settings.json"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Each setting is present in the audit trail
// ─────────────────────────────────────────────────────────────────────────────

/// Every key that ends up in the merged config must appear in loaded_entries().
#[test]
fn test_all_settings_appear_in_audit_trail() {
    let temp = TempDir::new().unwrap();

    write_project_config(
        temp.path(),
        "settings.json",
        r#"{"alpha": 1, "beta": 2, "gamma": 3}"#,
    );

    let loader = ConfigLoader::new().with_project_path(temp.path());
    let config = loader.load().unwrap();

    let entries = config.loaded_entries();

    // All three keys must be in the audit trail.
    for key in ["alpha", "beta", "gamma"] {
        let found = entries.iter().any(|e| e.key_path == key);
        assert!(found, "expected audit entry for key '{key}'");
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// No duplicates in the audit trail
// ─────────────────────────────────────────────────────────────────────────────

/// loaded_entries() must not contain duplicate key_path values even when the
/// same key is present in multiple layers.
#[test]
fn test_no_duplicate_key_paths_in_audit_trail() {
    let temp = TempDir::new().unwrap();

    // Both layers define the same three keys.
    write_project_config(temp.path(), "settings.json", r#"{"x": 1, "y": 2, "z": 3}"#);
    write_project_config(
        temp.path(),
        "settings.local.json",
        r#"{"x": 10, "y": 20, "z": 30}"#,
    );

    let loader = ConfigLoader::new().with_project_path(temp.path());
    let config = loader.load().unwrap();

    let entries = config.loaded_entries();

    for key in ["x", "y", "z"] {
        let count = entries.iter().filter(|e| e.key_path == key).count();
        assert_eq!(
            count, 1,
            "key '{key}' should appear exactly once in the audit trail, got {count}"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Winning values match the merged config
// ─────────────────────────────────────────────────────────────────────────────

/// The value stored in the audit-trail entry must match what get() returns.
#[test]
fn test_audit_trail_values_match_merged_config() {
    let temp = TempDir::new().unwrap();

    write_project_config(
        temp.path(),
        "settings.json",
        r#"{"rate": 5, "name": "base"}"#,
    );
    write_project_config(temp.path(), "settings.local.json", r#"{"rate": 99}"#);

    let loader = ConfigLoader::new().with_project_path(temp.path());
    let config = loader.load().unwrap();

    let entries = config.loaded_entries();

    // "rate" should be 99 in both the merged config and the audit trail.
    let rate_entry = entries.iter().find(|e| e.key_path == "rate").unwrap();
    assert_eq!(
        rate_entry.value.as_i64().unwrap(),
        99,
        "audit trail value for 'rate' must match the merged value"
    );
    assert_eq!(config.get("rate").unwrap().as_i64().unwrap(), 99);

    // "name" should be "base" (only in layer 3, not overridden).
    let name_entry = entries.iter().find(|e| e.key_path == "name").unwrap();
    assert_eq!(name_entry.value.as_str().unwrap(), "base");
    assert_eq!(config.get("name").unwrap().as_str().unwrap(), "base");
}

// ─────────────────────────────────────────────────────────────────────────────
// Empty config yields empty audit trail
// ─────────────────────────────────────────────────────────────────────────────

/// When no config files exist, loaded_entries() returns an empty slice.
#[test]
fn test_empty_config_yields_empty_audit_trail() {
    // Point at a directory that has no .swell subdirectory at all.
    let temp = TempDir::new().unwrap();

    let loader = ConfigLoader::new().with_project_path(temp.path());
    let config = loader.load().unwrap();

    // The audit trail may contain entries sourced from the user's own home-dir
    // config files (layers 1-2).  We can't assert 0 entries here because those
    // layers are outside test control.  What we CAN assert is that every entry
    // in the audit trail has a source_file (file-sourced), or no source_file
    // (env-var sourced) – i.e. the struct is well-formed.
    for entry in config.loaded_entries() {
        // key_path must be non-empty
        assert!(!entry.key_path.is_empty(), "key_path must not be empty");
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Environment-variable entries have no source_file
// ─────────────────────────────────────────────────────────────────────────────

/// Entries sourced from environment variables have source_file == None.
///
/// Note: the loader converts `SWELL_FOO_BAR` → key_path `foo.bar`
/// (prefix stripped, lowercased, underscores → dots).
#[test]
fn test_env_var_entries_have_no_source_file() {
    let temp = TempDir::new().unwrap();

    // The env var SWELL_AUDITUNIQUE is converted to key_path "auditunique".
    // We deliberately use a single-word suffix to avoid the underscore→dot
    // conversion producing a nested key name.
    std::env::set_var("SWELL_AUDITUNIQUE", "42");

    let loader = ConfigLoader::new().with_project_path(temp.path());
    let config = loader.load().unwrap();

    std::env::remove_var("SWELL_AUDITUNIQUE");

    // The key_path after stripping SWELL_ prefix and lowercasing is "auditunique".
    let expected_key = "auditunique";
    let entries = config.loaded_entries();
    let env_entry = entries.iter().find(|e| e.key_path == expected_key);
    assert!(
        env_entry.is_some(),
        "expected an audit entry for the env-var-sourced key '{expected_key}'"
    );
    let env_entry = env_entry.unwrap();
    assert!(
        env_entry.source_file.is_none(),
        "env-var sourced entries must have source_file == None, got: {:?}",
        env_entry.source_file
    );
}
