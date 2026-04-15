//!
//! Tests for section-aware config merge strategies
//!
//! Verifies VAL-SESS-006: Config merge uses section-aware strategies
//! - Scalar values: override (last writer wins)
//! - Nested objects: deep merge (keys merged recursively)
//! - Array settings: unique-extension (union of values, no duplicates)
//! - Designated sections: full replacement (entire section replaced)
//!
//! NOTE: Layers 1-2 (user global and user modern) use actual home directory
//! via dirs::home_dir(), so they cannot be controlled via project_path.
//! These tests only use layers 3-4 (project shared and project modern).

use swell_core::config::{ConfigLoader, LoadedConfig};
use serde_json::Value;
use std::path::PathBuf;
use tempfile::TempDir;

/// Create config file in project .swell directory (layers 3-4 use project_path)
fn create_project_config(dir: &PathBuf, name: &str, content: &str) -> PathBuf {
    let swell_dir = dir.join(".swell");
    std::fs::create_dir_all(&swell_dir).unwrap();
    let path = swell_dir.join(name);
    std::fs::write(&path, content).unwrap();
    path
}

#[test]
fn test_scalar_override() {
    // Scalar values should be overridden by higher layers (last writer wins)
    let temp = TempDir::new().unwrap();

    // Layer 3: project shared - timeout = 10
    create_project_config(
        &temp.path().to_path_buf(),
        "settings.json",
        r#"{"timeout": 10, "debug": false}"#,
    );

    // Layer 4: project modern - timeout = 20 (should override)
    create_project_config(
        &temp.path().to_path_buf(),
        "settings.local.json",
        r#"{"timeout": 20, "debug": true}"#,
    );

    let loader = ConfigLoader::new().with_project_path(temp.path());
    let config = loader.load().unwrap();

    // timeout should be 20 (override from higher layer)
    assert_eq!(config.get("timeout").unwrap().as_i64().unwrap(), 20);
    // debug should be true (override from higher layer)
    assert_eq!(config.get("debug").unwrap().as_bool().unwrap(), true);
}

#[test]
fn test_deep_merge_nested_objects() {
    // Nested objects should be deep merged (keys merged recursively)
    let temp = TempDir::new().unwrap();

    // Layer 3: paths = { a: 1, b: 2 }
    create_project_config(
        &temp.path().to_path_buf(),
        "settings.json",
        r#"{"paths": {"a": 1, "b": 2}}"#,
    );

    // Layer 4: paths = { b: 3, c: 4 } (deep merge)
    create_project_config(
        &temp.path().to_path_buf(),
        "settings.local.json",
        r#"{"paths": {"b": 3, "c": 4}}"#,
    );

    let loader = ConfigLoader::new().with_project_path(temp.path());
    let config = loader.load().unwrap();

    let paths = config.get("paths").unwrap().as_object().unwrap();
    // a is preserved from layer 1
    assert_eq!(paths.get("a").unwrap().as_i64().unwrap(), 1);
    // b is overridden to 3 from layer 2
    assert_eq!(paths.get("b").unwrap().as_i64().unwrap(), 3);
    // c is added from layer 2
    assert_eq!(paths.get("c").unwrap().as_i64().unwrap(), 4);
}

#[test]
fn test_deep_merge_three_levels() {
    // Test deep merge with 3 levels of nesting
    let temp = TempDir::new().unwrap();

    // Layer 3: execution = { max_retries: 3, timeout: 60 }
    create_project_config(
        &temp.path().to_path_buf(),
        "settings.json",
        r#"{"execution": {"max_retries": 3, "timeout": 60}}"#,
    );

    // Layer 4: execution = { max_retries: 5, log_level: "debug" }
    create_project_config(
        &temp.path().to_path_buf(),
        "settings.local.json",
        r#"{"execution": {"max_retries": 5, "log_level": "debug"}}"#,
    );

    let loader = ConfigLoader::new().with_project_path(temp.path());
    let config = loader.load().unwrap();

    let execution = config.get("execution").unwrap().as_object().unwrap();
    assert_eq!(execution.get("max_retries").unwrap().as_i64().unwrap(), 5); // overridden
    assert_eq!(execution.get("timeout").unwrap().as_i64().unwrap(), 60); // preserved
    assert_eq!(execution.get("log_level").unwrap().as_str().unwrap(), "debug"); // added
}

#[test]
fn test_unique_extension_arrays() {
    // Array settings should use unique-extension (union of values, no duplicates)
    let temp = TempDir::new().unwrap();

    // Layer 3: allowed_paths = ["x", "y"]
    create_project_config(
        &temp.path().to_path_buf(),
        "settings.json",
        r#"{"allowed_paths": ["x", "y"]}"#,
    );

    // Layer 4: allowed_paths = ["y", "z"] (should extend uniquely)
    create_project_config(
        &temp.path().to_path_buf(),
        "settings.local.json",
        r#"{"allowed_paths": ["y", "z"]}"#,
    );

    let loader = ConfigLoader::new().with_project_path(temp.path());
    let config = loader.load().unwrap();

    let allowed = config.get("allowed_paths").unwrap().as_array().unwrap();
    let items: Vec<&str> = allowed.iter().map(|v: &Value| v.as_str().unwrap()).collect();

    // Should have x, y, z (y is unique, not duplicated)
    assert!(items.contains(&"x"));
    assert!(items.contains(&"y"));
    assert!(items.contains(&"z"));
    assert_eq!(items.len(), 3); // No duplicates
}

#[test]
fn test_unique_extension_preserves_order() {
    // Test that unique-extension preserves order from lower to higher layers
    let temp = TempDir::new().unwrap();

    // Layer 3: plugins = ["alpha", "beta"]
    create_project_config(
        &temp.path().to_path_buf(),
        "settings.json",
        r#"{"plugins": ["alpha", "beta"]}"#,
    );

    // Layer 4: plugins = ["gamma", "beta"] (beta already exists, gamma is new)
    create_project_config(
        &temp.path().to_path_buf(),
        "settings.local.json",
        r#"{"plugins": ["gamma", "beta"]}"#,
    );

    let loader = ConfigLoader::new().with_project_path(temp.path());
    let config = loader.load().unwrap();

    let plugins = config.get("plugins").unwrap().as_array().unwrap();
    let items: Vec<&str> = plugins.iter().map(|v: &Value| v.as_str().unwrap()).collect();

    // Order: alpha, beta (from layer 3), gamma (from layer 4, beta already exists)
    assert_eq!(items, vec!["alpha", "beta", "gamma"]);
}

#[test]
fn test_full_replacement_designated_sections() {
    // Designated sections like "prompts" should use full replacement
    let temp = TempDir::new().unwrap();

    // Layer 3: prompts = { sys: "v1" }
    create_project_config(
        &temp.path().to_path_buf(),
        "settings.json",
        r#"{"prompts": {"sys": "version1", "user": "default_user"}}"#,
    );

    // Layer 4: prompts = { usr: "v2" } (should fully replace, not merge)
    create_project_config(
        &temp.path().to_path_buf(),
        "settings.local.json",
        r#"{"prompts": {"usr": "version2"}}"#,
    );

    let loader = ConfigLoader::new().with_project_path(temp.path());
    let config = loader.load().unwrap();

    let prompts = config.get("prompts").unwrap().as_object().unwrap();
    // Only the layer 4 key should exist (full replacement, not deep merge)
    assert!(prompts.get("sys").is_none());
    assert!(prompts.get("user").is_none());
    assert_eq!(prompts.get("usr").unwrap().as_str().unwrap(), "version2");
}

#[test]
fn test_full_replacement_prompts_section() {
    // The "prompts" section uses full replacement strategy
    let temp = TempDir::new().unwrap();

    create_project_config(
        &temp.path().to_path_buf(),
        "settings.json",
        r#"{"prompts": {"system": "old_system", "assistant": "old_assistant"}}"#,
    );

    create_project_config(
        &temp.path().to_path_buf(),
        "settings.local.json",
        r#"{"prompts": {"system": "new_system"}}"#,
    );

    let loader = ConfigLoader::new().with_project_path(temp.path());
    let config = loader.load().unwrap();

    let prompts = config.get("prompts").unwrap().as_object().unwrap();
    // Full replacement: only new_system exists, old keys are gone
    assert_eq!(prompts.len(), 1);
    assert_eq!(prompts.get("system").unwrap().as_str().unwrap(), "new_system");
}

#[test]
fn test_mixed_merge_strategies() {
    // Test the example from VAL-SESS-006 specification
    let temp = TempDir::new().unwrap();

    // Layer 3: timeout=10, paths={a:1, b:2}, allowed_paths=["x"], prompts={sys:"v1"}
    create_project_config(
        &temp.path().to_path_buf(),
        "settings.json",
        r#"{"timeout": 10, "paths": {"a": 1, "b": 2}, "allowed_paths": ["x"], "prompts": {"sys": "v1"}}"#,
    );

    // Layer 4: timeout=20, paths={b:3, c:4}, allowed_paths=["y"], prompts={usr:"v2"}
    create_project_config(
        &temp.path().to_path_buf(),
        "settings.local.json",
        r#"{"timeout": 20, "paths": {"b": 3, "c": 4}, "allowed_paths": ["y"], "prompts": {"usr": "v2"}}"#,
    );

    let loader = ConfigLoader::new().with_project_path(temp.path());
    let config = loader.load().unwrap();

    // timeout: scalar override → 20
    assert_eq!(config.get("timeout").unwrap().as_i64().unwrap(), 20);

    // paths: deep merge → {a:1, b:3, c:4}
    let paths = config.get("paths").unwrap().as_object().unwrap();
    assert_eq!(paths.get("a").unwrap().as_i64().unwrap(), 1);
    assert_eq!(paths.get("b").unwrap().as_i64().unwrap(), 3);
    assert_eq!(paths.get("c").unwrap().as_i64().unwrap(), 4);

    // allowed_paths: unique-extension → ["x", "y"]
    let allowed = config.get("allowed_paths").unwrap().as_array().unwrap();
    let items: Vec<&str> = allowed.iter().map(|v: &Value| v.as_str().unwrap()).collect();
    assert!(items.contains(&"x"));
    assert!(items.contains(&"y"));
    assert_eq!(items.len(), 2);

    // prompts: full replacement → {usr: "v2"}
    let prompts = config.get("prompts").unwrap().as_object().unwrap();
    assert!(prompts.get("sys").is_none());
    assert_eq!(prompts.get("usr").unwrap().as_str().unwrap(), "v2");
}

#[test]
fn test_designated_sections_configurable() {
    // The set of sections using full replacement should be configurable
    // Default includes "prompts" section
    let temp = TempDir::new().unwrap();

    create_project_config(
        &temp.path().to_path_buf(),
        "settings.json",
        r#"{"prompts": {"a": "1"}, "custom_section": {"a": "1"}}"#,
    );

    create_project_config(
        &temp.path().to_path_buf(),
        "settings.local.json",
        r#"{"prompts": {"b": "2"}, "custom_section": {"b": "2"}}"#,
    );

    let loader = ConfigLoader::new().with_project_path(temp.path());
    let config = loader.load().unwrap();

    // prompts uses full replacement
    let prompts = config.get("prompts").unwrap().as_object().unwrap();
    assert_eq!(prompts.len(), 1);
    assert!(prompts.get("b").is_some());

    // custom_section (not designated) uses deep merge
    let custom = config.get("custom_section").unwrap().as_object().unwrap();
    assert_eq!(custom.len(), 2); // Both a and b exist
    assert!(custom.get("a").is_some());
    assert!(custom.get("b").is_some());
}

#[test]
fn test_deep_merge_does_not_concatenate_strings() {
    // Deep merge should not concatenate string values
    let temp = TempDir::new().unwrap();

    create_project_config(
        &temp.path().to_path_buf(),
        "settings.json",
        r#"{"connection": {"url": "http://old"}}"#,
    );

    create_project_config(
        &temp.path().to_path_buf(),
        "settings.local.json",
        r#"{"connection": {"url": "http://new"}}"#,
    );

    let loader = ConfigLoader::new().with_project_path(temp.path());
    let config = loader.load().unwrap();

    let url = config.get("connection").unwrap().as_object().unwrap()
        .get("url").unwrap().as_str().unwrap();
    assert_eq!(url, "http://new"); // Override, not merge
}

#[test]
fn test_empty_arrays_unique_extension() {
    // Empty arrays should not cause issues in unique-extension
    let temp = TempDir::new().unwrap();

    create_project_config(
        &temp.path().to_path_buf(),
        "settings.json",
        r#"{"items": []}"#,
    );

    create_project_config(
        &temp.path().to_path_buf(),
        "settings.local.json",
        r#"{"items": ["a"]}"#,
    );

    let loader = ConfigLoader::new().with_project_path(temp.path());
    let config = loader.load().unwrap();

    let items = config.get("items").unwrap().as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].as_str().unwrap(), "a");
}

#[test]
fn test_array_with_primitives_only() {
    // Only arrays of primitives use unique-extension
    // Arrays containing objects might need different handling
    let temp = TempDir::new().unwrap();

    create_project_config(
        &temp.path().to_path_buf(),
        "settings.json",
        r#"{"tags": ["a", "b"]}"#,
    );

    create_project_config(
        &temp.path().to_path_buf(),
        "settings.local.json",
        r#"{"tags": ["b", "c"]}"#,
    );

    let loader = ConfigLoader::new().with_project_path(temp.path());
    let config = loader.load().unwrap();

    let tags = config.get("tags").unwrap().as_array().unwrap();
    let items: Vec<&str> = tags.iter().map(|v: &Value| v.as_str().unwrap()).collect();
    assert_eq!(items, vec!["a", "b", "c"]);
}
