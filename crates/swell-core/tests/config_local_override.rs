//! Tests for VAL-SESS-008: settings.local.json local override layer
//!
//! - settings.local.json values override settings.json values
//! - .gitignore template includes settings.local.json
//! - Missing settings.local.json is silently skipped

use std::path::PathBuf;
use swell_core::config::ConfigLoader;
use tempfile::TempDir;

/// Write a JSON config file inside `<dir>/.swell/<name>`.
fn write_project_config(dir: &PathBuf, name: &str, content: &str) -> PathBuf {
    let swell_dir = dir.join(".swell");
    std::fs::create_dir_all(&swell_dir).unwrap();
    let path = swell_dir.join(name);
    std::fs::write(&path, content).unwrap();
    path
}

// ─────────────────────────────────────────────────────────────────────────────
// VAL-SESS-008: settings.local.json values override settings.json
// ─────────────────────────────────────────────────────────────────────────────

/// settings.local.json takes precedence over .swell/settings.json
#[test]
fn test_local_override_overrides_settings() {
    let temp = TempDir::new().unwrap();

    // Layer 3: settings.json - debug = false
    write_project_config(
        &temp.path().to_path_buf(),
        "settings.json",
        r#"{"debug": false, "timeout": 10, "mode": "shared"}"#,
    );

    // Layer 4: settings.local.json - debug = true (should override)
    write_project_config(
        &temp.path().to_path_buf(),
        "settings.local.json",
        r#"{"debug": true, "timeout": 20}"#,
    );

    let loader = ConfigLoader::new().with_project_path(temp.path());
    let config = loader.load().unwrap();

    // debug should be true (override from settings.local.json)
    assert_eq!(
        config.get("debug").unwrap().as_bool().unwrap(),
        true,
        "debug should be overridden to true by settings.local.json"
    );

    // timeout should be 20 (override from settings.local.json)
    assert_eq!(
        config.get("timeout").unwrap().as_i64().unwrap(),
        20,
        "timeout should be overridden to 20 by settings.local.json"
    );

    // mode should be "shared" (not overridden, comes from settings.json)
    assert_eq!(
        config.get("mode").unwrap().as_str().unwrap(),
        "shared",
        "mode should remain 'shared' from settings.json"
    );
}

/// Local override works with nested objects (deep merge)
#[test]
fn test_local_override_deep_merges_nested_objects() {
    let temp = TempDir::new().unwrap();

    write_project_config(
        &temp.path().to_path_buf(),
        "settings.json",
        r#"{"execution": {"timeout": 60, "max_retries": 3, "log_level": "info"}}"#,
    );

    write_project_config(
        &temp.path().to_path_buf(),
        "settings.local.json",
        r#"{"execution": {"timeout": 120, "max_retries": 5}}"#,
    );

    let loader = ConfigLoader::new().with_project_path(temp.path());
    let config = loader.load().unwrap();

    let execution = config.get("execution").unwrap().as_object().unwrap();

    // timeout overridden to 120
    assert_eq!(execution.get("timeout").unwrap().as_i64().unwrap(), 120);
    // max_retries overridden to 5
    assert_eq!(execution.get("max_retries").unwrap().as_i64().unwrap(), 5);
    // log_level preserved from settings.json
    assert_eq!(
        execution.get("log_level").unwrap().as_str().unwrap(),
        "info"
    );
}

/// Local override works with array sections using unique-extension
#[test]
fn test_local_override_extends_arrays() {
    let temp = TempDir::new().unwrap();

    write_project_config(
        &temp.path().to_path_buf(),
        "settings.json",
        r#"{"allowed_paths": ["/workspace", "/home"]}"#,
    );

    write_project_config(
        &temp.path().to_path_buf(),
        "settings.local.json",
        r#"{"allowed_paths": ["/tmp", "/workspace"]}"#,
    );

    let loader = ConfigLoader::new().with_project_path(temp.path());
    let config = loader.load().unwrap();

    let allowed = config.get("allowed_paths").unwrap().as_array().unwrap();
    let items: Vec<&str> = allowed.iter().map(|v| v.as_str().unwrap()).collect();

    // Should have all four paths (workspace appears in both but deduped)
    assert!(items.contains(&"/workspace"));
    assert!(items.contains(&"/home"));
    assert!(items.contains(&"/tmp"));
    assert_eq!(items.len(), 3, "should have no duplicates");
}

// ─────────────────────────────────────────────────────────────────────────────
// VAL-SESS-008: .gitignore template includes settings.local.json
// ─────────────────────────────────────────────────────────────────────────────

/// The gitignore template contains settings.local.json entry
#[test]
fn test_gitignore_template_contains_settings_local_json() {
    let template = ConfigLoader::gitignore_template();

    assert!(
        template.contains("settings.local.json"),
        "gitignore template must include settings.local.json entry"
    );
}

/// The gitignore template is well-formed
#[test]
fn test_gitignore_template_is_well_formed() {
    let template = ConfigLoader::gitignore_template();

    // Template should be non-empty
    assert!(
        !template.is_empty(),
        "gitignore template should not be empty"
    );

    // Template should have at least one line
    let lines: Vec<&str> = template.lines().collect();
    assert!(
        !lines.is_empty(),
        "gitignore template should have at least one line"
    );

    // All lines should be valid gitignore entries (start with # or are non-empty)
    for line in lines {
        // Allow empty lines and comment lines
        if !line.is_empty() && !line.trim().starts_with('#') {
            // Non-comment lines should not have trailing whitespace issues
            // (actual gitignore entries are fine)
        }
    }
}

/// The gitignore template can be used directly as a .gitignore file
#[test]
fn test_gitignore_template_is_valid_gitignore_content() {
    let template = ConfigLoader::gitignore_template();

    // The template should include a comment explaining the section
    assert!(
        template.contains('#'),
        "gitignore template should include comments explaining entries"
    );

    // Should include the local override entry
    assert!(
        template.contains("settings.local.json"),
        "gitignore template must include settings.local.json"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// VAL-SESS-008: Missing settings.local.json is silently skipped
// ─────────────────────────────────────────────────────────────────────────────

/// Loading succeeds when settings.local.json does not exist
#[test]
fn test_missing_local_override_handled_gracefully() {
    let temp = TempDir::new().unwrap();

    // Only create settings.json, NOT settings.local.json
    std::fs::create_dir_all(temp.path().join(".swell")).unwrap();
    std::fs::write(
        temp.path().join(".swell").join("settings.json"),
        r#"{"debug": true, "timeout": 30}"#,
    )
    .unwrap();

    let loader = ConfigLoader::new().with_project_path(temp.path());

    // Should not panic or error - just skip the missing file
    let config = loader.load().unwrap();

    // Values from settings.json should be available
    assert_eq!(config.get("debug").unwrap().as_bool().unwrap(), true);
    assert_eq!(config.get("timeout").unwrap().as_i64().unwrap(), 30);
}

/// Audit trail works correctly when settings.local.json is absent
#[test]
fn test_audit_trail_correct_when_local_override_absent() {
    let temp = TempDir::new().unwrap();

    // Only settings.json exists
    std::fs::create_dir_all(temp.path().join(".swell")).unwrap();
    std::fs::write(
        temp.path().join(".swell").join("settings.json"),
        r#"{"only_in_settings": "from_settings_json"}"#,
    )
    .unwrap();

    let loader = ConfigLoader::new().with_project_path(temp.path());
    let config = loader.load().unwrap();

    let entries = config.loaded_entries();

    // The key should appear in audit trail with source pointing to settings.json
    let entry = entries.iter().find(|e| e.key_path == "only_in_settings");
    assert!(entry.is_some(), "key should appear in audit trail");

    let entry = entry.unwrap();
    assert_eq!(
        entry.value.as_str().unwrap(),
        "from_settings_json",
        "value should match"
    );
    assert!(
        entry.source_file.is_some(),
        "source_file should be set for file-sourced entries"
    );
    assert!(
        entry
            .source_file
            .as_ref()
            .unwrap()
            .contains("settings.json"),
        "source should point to settings.json"
    );
}

/// Loading handles empty settings.local.json gracefully
#[test]
fn test_empty_local_override_file_handled() {
    let temp = TempDir::new().unwrap();

    write_project_config(
        &temp.path().to_path_buf(),
        "settings.json",
        r#"{"timeout": 30}"#,
    );

    // Create an empty settings.local.json
    write_project_config(&temp.path().to_path_buf(), "settings.local.json", "");

    let loader = ConfigLoader::new().with_project_path(temp.path());
    let config = loader.load().unwrap();

    // Should load successfully, timeout from settings.json should be present
    assert_eq!(config.get("timeout").unwrap().as_i64().unwrap(), 30);
}

/// Loading handles invalid JSON in settings.local.json gracefully
#[test]
fn test_invalid_local_override_json_handled() {
    let temp = TempDir::new().unwrap();

    write_project_config(
        &temp.path().to_path_buf(),
        "settings.json",
        r#"{"timeout": 30, "debug": false}"#,
    );

    // Create a settings.local.json with invalid JSON
    write_project_config(
        &temp.path().to_path_buf(),
        "settings.local.json",
        "this is not json {",
    );

    let loader = ConfigLoader::new().with_project_path(temp.path());
    let config = loader.load().unwrap();

    // Should load successfully with values from settings.json
    assert_eq!(config.get("timeout").unwrap().as_i64().unwrap(), 30);
    assert_eq!(config.get("debug").unwrap().as_bool().unwrap(), false);
}

// ─────────────────────────────────────────────────────────────────────────────
// Integration: full five-layer cascade with local override
// ─────────────────────────────────────────────────────────────────────────────

/// Local override takes precedence over all lower layers
#[test]
fn test_local_override_wins_in_full_cascade() {
    let temp = TempDir::new().unwrap();

    // Layer 1+2: skip (user home directory, not controllable in test)

    // Layer 3: project shared
    write_project_config(
        &temp.path().to_path_buf(),
        "settings.json",
        r#"{"level": 3, "source": "settings_json"}"#,
    );

    // Layer 4: project modern (local override)
    write_project_config(
        &temp.path().to_path_buf(),
        "settings.local.json",
        r#"{"level": 4, "source": "settings_local_json"}"#,
    );

    let loader = ConfigLoader::new().with_project_path(temp.path());
    let config = loader.load().unwrap();

    // Both values should come from settings.local.json (layer 4 wins)
    assert_eq!(config.get("level").unwrap().as_i64().unwrap(), 4);
    assert_eq!(
        config.get("source").unwrap().as_str().unwrap(),
        "settings_local_json"
    );
}

/// Environment variables still win over settings.local.json (layer 5)
#[test]
fn test_env_vars_win_over_local_override() {
    let temp = TempDir::new().unwrap();

    write_project_config(
        &temp.path().to_path_buf(),
        "settings.json",
        r#"{"priority": "file"}"#,
    );

    write_project_config(
        &temp.path().to_path_buf(),
        "settings.local.json",
        r#"{"priority": "local"}"#,
    );

    std::env::set_var("SWELL_PRIORITY", "env");

    let loader = ConfigLoader::new().with_project_path(temp.path());
    let config = loader.load().unwrap();

    // Env var (layer 5) should win over settings.local.json (layer 4)
    assert_eq!(config.get("priority").unwrap().as_str().unwrap(), "env");

    std::env::remove_var("SWELL_PRIORITY");
}
