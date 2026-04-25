//! Configuration loader with five-layer precedence.
//!
//! Configuration is loaded and merged from five sources in order
//! (lowest to highest priority):
//! 1. User global (`~/.config/swell/settings.json`)
//! 2. User modern (`~/.swell/settings.json`)
//! 3. Project shared (`.swell/settings.json` committed to repo)
//! 4. Project modern (`.swell/settings.local.json`)
//! 5. Local override (environment variables or CLI flags)
//!
//! Higher-priority layers override lower-priority values.
//!
//! # Section-Aware Merge Strategies
//!
//! Different configuration sections use different merge strategies:
//! - **Scalar values**: Override (last writer wins)
//! - **Nested objects**: Deep merge (keys merged recursively)
//! - **Array settings**: Unique-extension (union of values, no duplicates)
//! - **Designated sections**: Full replacement (e.g., `prompts` section)

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::env;
use std::path::{Path, PathBuf};

/// Keys that use unique-extension merge strategy for arrays.
/// These keys represent collections where values should be unioned without duplicates.
const UNIQUE_EXTENSION_KEYS: &[&str] = &[
    "allowed_paths",
    "plugins",
    "tags",
    "items",
    "additional_dependencies",
    "feature_flags",
];

/// Sections that use full replacement strategy (entire section replaced, not merged).
/// By default, this includes the `prompts` section.
const FULL_REPLACEMENT_SECTIONS: &[&str] = &["prompts"];

/// Layer index for precedence (higher = higher priority)
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ConfigLayer {
    /// User global: ~/.config/swell/settings.json
    UserGlobal = 1,
    /// User modern: ~/.swell/settings.json
    UserModern = 2,
    /// Project shared: .swell/settings.json (committed to repo)
    ProjectShared = 3,
    /// Project modern: .swell/settings.local.json
    ProjectModern = 4,
    /// Local override: environment variables or CLI flags
    LocalOverride = 5,
}

impl ConfigLayer {
    pub fn file_name(&self) -> &'static str {
        match self {
            ConfigLayer::UserGlobal => "settings.json",
            ConfigLayer::UserModern => "settings.json",
            ConfigLayer::ProjectShared => "settings.json",
            ConfigLayer::ProjectModern => "settings.local.json",
            ConfigLayer::LocalOverride => "", // Not a file
        }
    }
}

/// A configuration entry with source tracking
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigEntry {
    pub key_path: String,
    pub value: serde_json::Value,
    pub source_file: Option<String>,
}

/// Loaded configuration with audit trail
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LoadedConfig {
    /// The merged configuration values
    pub values: serde_json::Value,
    /// Audit trail of all loaded entries
    pub entries: Vec<ConfigEntry>,
}

impl LoadedConfig {
    /// Returns the audit trail of configuration entries
    pub fn loaded_entries(&self) -> &[ConfigEntry] {
        &self.entries
    }

    /// Get a value by key path (e.g., "execution.max_task_timeout_seconds")
    pub fn get(&self, key_path: &str) -> Option<&serde_json::Value> {
        let mut current = &self.values;
        for part in key_path.split('.') {
            current = current.get(part)?;
        }
        Some(current)
    }
}

/// ConfigLoader with five-layer precedence
#[derive(Debug, Clone)]
pub struct ConfigLoader {
    /// Project path for .swell directory discovery
    project_path: Option<PathBuf>,
    /// Environment prefix for local override
    env_prefix: String,
}

impl Default for ConfigLoader {
    fn default() -> Self {
        Self::new()
    }
}

impl ConfigLoader {
    /// Create a new ConfigLoader with default settings
    pub fn new() -> Self {
        Self {
            project_path: std::env::current_dir().ok(),
            env_prefix: "SWELL_".to_string(),
        }
    }

    /// Set the project path for .swell directory discovery
    pub fn with_project_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.project_path = Some(path.into());
        self
    }

    /// Set the environment variable prefix for local override
    pub fn with_env_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.env_prefix = prefix.into();
        self
    }

    /// Generate a .gitignore template for the .swell directory.
    ///
    /// This template includes entries for:
    /// - `settings.local.json` (local override file that should not be committed)
    /// - Other common .swell artifacts that should remain local
    ///
    /// Projects should include this template in their `.gitignore` or copy the
    /// relevant entries to their existing `.gitignore`.
    pub fn gitignore_template() -> &'static str {
        r#"# SWELL local configuration
# Local overrides - do not commit
.swell/settings.local.json
.swell/.task-state.json
.swell/checkpoints/
"#
    }

    /// Get the user global config path
    fn user_global_path() -> Option<PathBuf> {
        dirs::home_dir().map(|h| h.join(".config").join("swell").join("settings.json"))
    }

    /// Get the user modern config path
    fn user_modern_path() -> Option<PathBuf> {
        dirs::home_dir().map(|h| h.join(".swell").join("settings.json"))
    }

    /// Get the project shared config path
    fn project_shared_path(project_path: &Path) -> PathBuf {
        project_path.join(".swell").join("settings.json")
    }

    /// Get the project modern (local override) config path
    fn project_modern_path(project_path: &Path) -> PathBuf {
        project_path.join(".swell").join("settings.local.json")
    }

    /// Load configuration from all five layers
    pub fn load(&self) -> Result<LoadedConfig> {
        let mut values = serde_json::Map::new();
        let mut entries = Vec::new();

        // Layer 1: User global (~/.config/swell/settings.json)
        if let Some(path) = Self::user_global_path() {
            self.load_file(&path, ConfigLayer::UserGlobal, &mut values, &mut entries);
        }

        // Layer 2: User modern (~/.swell/settings.json)
        if let Some(path) = Self::user_modern_path() {
            self.load_file(&path, ConfigLayer::UserModern, &mut values, &mut entries);
        }

        // Layer 3: Project shared (.swell/settings.json)
        if let Some(ref project_path) = self.project_path {
            let path = Self::project_shared_path(project_path);
            self.load_file(&path, ConfigLayer::ProjectShared, &mut values, &mut entries);
        }

        // Layer 4: Project modern (.swell/settings.local.json)
        if let Some(ref project_path) = self.project_path {
            let path = Self::project_modern_path(project_path);
            self.load_file(&path, ConfigLayer::ProjectModern, &mut values, &mut entries);
        }

        // Layer 5: Local override (environment variables)
        self.load_env_overrides(&mut values, &mut entries);

        Ok(LoadedConfig {
            values: serde_json::Value::Object(values),
            entries,
        })
    }

    /// Load a config file, silently skipping if it doesn't exist
    fn load_file(
        &self,
        path: &PathBuf,
        layer: ConfigLayer,
        values: &mut serde_json::Map<String, serde_json::Value>,
        entries: &mut Vec<ConfigEntry>,
    ) {
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => return, // Silently skip missing files
        };

        let parsed: serde_json::Value = match serde_json::from_str(&content) {
            Ok(v) => v,
            Err(_) => return, // Silently skip invalid JSON
        };

        self.merge_value(
            parsed,
            layer,
            path.to_string_lossy().to_string(),
            values,
            entries,
        );
    }

    /// Load environment variable overrides.
    ///
    /// Environment variables are the highest-priority layer (layer 5). Each
    /// SWELL_* env var is upserted into the audit trail so that `loaded_entries()`
    /// always reflects the winner, even when the env var overrides a file-sourced key.
    ///
    /// # Naming Convention
    ///
    /// Environment variables use underscores as section separators. For example:
    /// - `SWELL_EXECUTION_TIMEOUT` → `execution.timeout` (section.key format)
    /// - `SWELL_EXECUTION_MAX_ITERATIONS` → `execution.max_iterations`
    ///
    /// The first underscore after the prefix separates the section (e.g., `execution`)
    /// from the remaining path. Subsequent underscores within the path are preserved
    /// to support keys that contain underscores (e.g., `max_iterations`).
    fn load_env_overrides(
        &self,
        values: &mut serde_json::Map<String, serde_json::Value>,
        entries: &mut Vec<ConfigEntry>,
    ) {
        // Look for SWELL_* environment variables
        for (key, value) in env::vars() {
            if key.starts_with(&self.env_prefix) {
                // Strip the prefix and lowercase
                let suffix = key
                    .strip_prefix(&self.env_prefix)
                    .unwrap_or(&key)
                    .to_lowercase();

                // Replace only the FIRST underscore with a dot (section separator)
                // Subsequent underscores in the path are preserved
                let config_key = if let Some(pos) = suffix.find('_') {
                    let (section, rest) = suffix.split_at(pos);
                    format!("{}.{}", section, &rest[1..]) // Skip the underscore we split at
                } else {
                    suffix
                };

                let json_value: serde_json::Value =
                    serde_json::from_str(&value).unwrap_or(serde_json::Value::String(value));

                // Upsert: update the existing entry if one already exists for this key
                // (env vars are layer 5 — highest priority, they always win).
                if let Some(existing) = entries.iter_mut().find(|e| e.key_path == config_key) {
                    existing.value = json_value.clone();
                    existing.source_file = None; // env-var sourced → no file path
                } else {
                    entries.push(ConfigEntry {
                        key_path: config_key.clone(),
                        value: json_value.clone(),
                        source_file: None,
                    });
                }

                // Set the value (deep merge or override)
                set_nested_value(values, &config_key, json_value);
            }
        }
    }

    /// Merge a value into the config, tracking entries.
    ///
    /// The audit trail (`entries`) records all config keys with their source:
    /// - For nested objects, each leaf key is recorded with its full path
    ///   (e.g., `execution.max_iterations` for `{"execution": {"max_iterations": 5}}`)
    /// - If a higher-priority layer provides the same key, the existing entry is
    ///   updated in place so that `loaded_entries()` always shows the source that
    ///   ultimately won.
    fn merge_value(
        &self,
        value: serde_json::Value,
        _layer: ConfigLayer,
        source: String,
        values: &mut serde_json::Map<String, serde_json::Value>,
        entries: &mut Vec<ConfigEntry>,
    ) {
        if let serde_json::Value::Object(obj) = value {
            for (key, val) in obj {
                let key_path = key.clone();

                // Record the entry in the audit trail (including nested keys)
                self.record_entries_recursive(&key_path, &val, source.clone(), entries);

                // Apply section-aware merge strategy
                if let Some(existing) = values.get_mut(&key) {
                    *existing = self.apply_merge_strategy(existing.clone(), val, &key);
                } else {
                    values.insert(key, val);
                }
            }
        }
    }

    /// Recursively record all entries in the audit trail.
    ///
    /// For nested objects, this records each leaf key with its full dot-separated path.
    /// For example, `{"execution": {"max_iterations": 5}}` records:
    /// - `execution` → `{"max_iterations": 5}` (the object itself)
    /// - `execution.max_iterations` → `5` (the leaf value)
    fn record_entries_recursive(
        &self,
        key_path: &str,
        value: &serde_json::Value,
        source: String,
        entries: &mut Vec<ConfigEntry>,
    ) {
        // Record/update the entry at this path
        if let Some(existing_entry) = entries.iter_mut().find(|e| e.key_path == key_path) {
            existing_entry.value = value.clone();
            existing_entry.source_file = Some(source.clone());
        } else {
            entries.push(ConfigEntry {
                key_path: key_path.to_string(),
                value: value.clone(),
                source_file: Some(source.clone()),
            });
        }

        // For objects, recursively record nested entries
        if let serde_json::Value::Object(obj) = value {
            for (nested_key, nested_val) in obj {
                let nested_path = format!("{}.{}", key_path, nested_key);
                self.record_entries_recursive(&nested_path, nested_val, source.clone(), entries);
            }
        }
    }

    /// Apply the appropriate merge strategy based on the key and value types.
    ///
    /// - Scalar values: Override (last writer wins)
    /// - Nested objects: Deep merge (keys merged recursively)
    /// - Array settings in UNIQUE_EXTENSION_KEYS: Unique-extension (union, no duplicates)
    /// - Sections in FULL_REPLACEMENT_SECTIONS: Full replacement
    fn apply_merge_strategy(
        &self,
        existing: serde_json::Value,
        new: serde_json::Value,
        key: &str,
    ) -> serde_json::Value {
        // Full replacement for designated sections - higher layer completely replaces
        if Self::is_full_replacement_section(key) {
            return new;
        }

        // Unique extension for arrays in specific keys
        if Self::is_unique_extension_key(key) {
            if let (serde_json::Value::Array(existing_arr), serde_json::Value::Array(new_arr)) =
                (&existing, &new)
            {
                return Self::merge_arrays_unique_extension(existing_arr.clone(), new_arr.clone());
            }
            // If not both arrays, fall back to override
            return new;
        }

        // Deep merge for nested objects
        if let (serde_json::Value::Object(existing_obj), serde_json::Value::Object(new_obj)) =
            (&existing, &new)
        {
            return Self::deep_merge(existing_obj.clone(), new_obj.clone());
        }

        // Override for all other cases (scalars, different types)
        new
    }

    /// Check if a key uses full replacement strategy
    fn is_full_replacement_section(key: &str) -> bool {
        FULL_REPLACEMENT_SECTIONS.contains(&key)
    }

    /// Check if a key uses unique extension strategy for arrays
    fn is_unique_extension_key(key: &str) -> bool {
        UNIQUE_EXTENSION_KEYS.contains(&key)
    }

    /// Deep merge two JSON objects, recursively merging nested objects.
    /// For overlapping keys, values from new override values from existing.
    fn deep_merge(
        existing: serde_json::Map<String, serde_json::Value>,
        new: serde_json::Map<String, serde_json::Value>,
    ) -> serde_json::Value {
        let mut result = existing;

        for (key, new_value) in new {
            if let Some(existing_value) = result.get_mut(&key) {
                // Recursively merge nested objects
                let merged = match (existing_value.clone(), new_value) {
                    (
                        serde_json::Value::Object(existing_obj),
                        serde_json::Value::Object(new_obj),
                    ) => Self::deep_merge(existing_obj, new_obj),
                    // Override for non-objects (scalars, arrays)
                    (_, new_val) => new_val,
                };
                *existing_value = merged;
            } else {
                result.insert(key, new_value);
            }
        }

        serde_json::Value::Object(result)
    }

    /// Merge two arrays using unique-extension strategy.
    /// Items from both arrays are combined, with duplicates removed.
    fn merge_arrays_unique_extension(
        existing: Vec<serde_json::Value>,
        new: Vec<serde_json::Value>,
    ) -> serde_json::Value {
        let mut result = existing;

        for item in new {
            if !result.iter().any(|e| e == &item) {
                result.push(item);
            }
        }

        serde_json::Value::Array(result)
    }
}

/// Set a nested value in a JSON object using dot notation
fn set_nested_value(
    map: &mut serde_json::Map<String, serde_json::Value>,
    key_path: &str,
    value: serde_json::Value,
) {
    let parts: Vec<&str> = key_path.split('.').collect();
    if parts.is_empty() {
        return;
    }

    if parts.len() == 1 {
        map.insert(parts[0].to_string(), value);
        return;
    }

    // Navigate to the parent of the final key, building path if needed
    let final_key = parts[parts.len() - 1].to_string();

    // Build the nested structure along the path
    let mut current = map;
    for part in parts.iter().take(parts.len() - 1) {
        let part_str = part.to_string();
        if !current.contains_key(&part_str) {
            current.insert(
                part_str.clone(),
                serde_json::Value::Object(serde_json::Map::new()),
            );
        }
        if let Some(serde_json::Value::Object(ref mut obj)) = current.get_mut(&part_str) {
            current = obj;
        } else {
            // Path leads through a non-object, can't set nested value
            return;
        }
    }
    // Now insert at the final key
    current.insert(final_key, value);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use tempfile::TempDir;

    fn create_temp_config(dir: &PathBuf, name: &str, content: &str) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, content).unwrap();
        path
    }

    #[test]
    #[serial]
    fn test_five_layer_precedence_ordering() {
        // Test that higher layers override lower layers
        let temp = TempDir::new().unwrap();

        // Layer 1: user global
        create_temp_config(
            &temp.path().join(".config/swell"),
            "settings.json",
            r#"{"timeout": 10}}"#,
        );

        // Layer 2: user modern
        create_temp_config(
            &temp.path().join(".swell"),
            "settings.json",
            r#"{"timeout": 20}}"#,
        );

        // Layer 3: project shared
        create_temp_config(
            &temp.path().join(".swell"),
            "settings.json",
            r#"{"timeout": 30}}"#,
        );

        // Layer 4: project modern
        create_temp_config(
            &temp.path().join(".swell"),
            "settings.local.json",
            r#"{"timeout": 40}}"#,
        );

        let loader = ConfigLoader::new().with_project_path(temp.path());

        // With env override
        std::env::set_var("SWELL_TIMEOUT", "50");

        let config = loader.load().unwrap();

        // Env var (layer 5) should win
        assert_eq!(config.get("timeout").unwrap().as_i64().unwrap(), 50);

        std::env::remove_var("SWELL_TIMEOUT");
    }

    #[test]
    #[serial]
    fn test_higher_layer_overrides_lower_layer() {
        // Ensure no env var pollution from previous tests
        std::env::remove_var("SWELL_TIMEOUT");

        let temp = TempDir::new().unwrap();

        // User global sets timeout to 10
        std::fs::create_dir_all(temp.path().join(".config/swell")).unwrap();
        std::fs::write(
            temp.path().join(".config/swell/settings.json"),
            r#"{"timeout": 10, "debug": false}"#,
        )
        .unwrap();

        // User modern overrides timeout to 20
        std::fs::create_dir_all(temp.path().join(".swell")).unwrap();
        std::fs::write(
            temp.path().join(".swell/settings.json"),
            r#"{"timeout": 20, "debug": true}"#,
        )
        .unwrap();

        let loader = ConfigLoader::new().with_project_path(temp.path());
        let config = loader.load().unwrap();

        // timeout should be 20 (from user modern)
        assert_eq!(config.get("timeout").unwrap().as_i64().unwrap(), 20);
        // debug should be true (from user modern)
        assert!(config.get("debug").unwrap().as_bool().unwrap());
    }

    #[test]
    #[serial]
    fn test_missing_layers_handled_gracefully() {
        // Ensure no leftover env var from previous tests
        std::env::remove_var("SWELL_TIMEOUT");

        let temp = TempDir::new().unwrap();

        // Only create project shared config
        std::fs::create_dir_all(temp.path().join(".swell")).unwrap();
        std::fs::write(
            temp.path().join(".swell/settings.json"),
            r#"{"timeout": 30}"#,
        )
        .unwrap();

        let loader = ConfigLoader::new().with_project_path(temp.path());
        let config = loader.load().unwrap();

        // Should successfully load from available layer
        assert_eq!(config.get("timeout").unwrap().as_i64().unwrap(), 30);

        // Should not fail on missing user configs
        // (would panic on unwrap if not handled)
    }

    #[test]
    #[serial]
    fn test_env_override() {
        let temp = TempDir::new().unwrap();
        std::fs::create_dir_all(temp.path().join(".swell")).unwrap();
        std::fs::write(
            temp.path().join(".swell/settings.json"),
            r#"{"timeout": 30}"#,
        )
        .unwrap();

        std::env::set_var("SWELL_TIMEOUT", "100");

        let loader = ConfigLoader::new().with_project_path(temp.path());
        let config = loader.load().unwrap();

        // Env var should override file value
        assert_eq!(config.get("timeout").unwrap().as_i64().unwrap(), 100);

        std::env::remove_var("SWELL_TIMEOUT");
    }

    #[test]
    #[serial]
    fn test_loaded_entries_tracking() {
        // Defensive: ensure no stale SWELL_TIMEOUT from a prior test slips in
        // via env-var layer 5 and overwrites the file source on the "timeout"
        // entry. Belt-and-braces; #[serial] already prevents concurrent
        // set_var/remove_var from the sibling env-var tests in this file.
        std::env::remove_var("SWELL_TIMEOUT");

        let temp = TempDir::new().unwrap();

        // Create two layers
        std::fs::create_dir_all(temp.path().join(".config/swell")).unwrap();
        std::fs::write(
            temp.path().join(".config/swell/settings.json"),
            r#"{"timeout": 10}"#,
        )
        .unwrap();

        std::fs::create_dir_all(temp.path().join(".swell")).unwrap();
        std::fs::write(
            temp.path().join(".swell/settings.json"),
            r#"{"timeout": 20, "debug": true}"#,
        )
        .unwrap();

        let loader = ConfigLoader::new().with_project_path(temp.path());
        let config = loader.load().unwrap();

        let entries = config.loaded_entries();

        // Should have entries from both layers
        assert!(entries.len() >= 2);

        // The "timeout" entry should point to the highest-precedence source
        let timeout_entry = entries.iter().find(|e| e.key_path == "timeout").unwrap();
        assert!(timeout_entry.source_file.is_some());
    }

    #[test]
    fn test_nonexistent_project_path() {
        let loader = ConfigLoader::new().with_project_path("/nonexistent/path/that/does/not/exist");

        // Should not panic, just skip the layer
        let config = loader.load().unwrap();
        // Should have empty config (no files exist)
        assert!(config.values.is_object());
    }

    #[test]
    fn test_invalid_json_ignored() {
        let temp = TempDir::new().unwrap();

        std::fs::create_dir_all(temp.path().join(".swell")).unwrap();
        std::fs::write(
            temp.path().join(".swell/settings.json"),
            "not valid json {{{",
        )
        .unwrap();

        let loader = ConfigLoader::new().with_project_path(temp.path());
        let config = loader.load().unwrap();

        // Should not panic, just skip invalid file
        assert!(config.values.is_object());
    }

    #[test]
    #[serial]
    fn test_nested_env_var_override() {
        // Clean up any leftover env vars
        std::env::remove_var("SWELL_EXECUTION_MAX_ITERATIONS");

        let temp = TempDir::new().unwrap();
        std::fs::create_dir_all(temp.path().join(".swell")).unwrap();
        std::fs::write(
            temp.path().join(".swell/settings.json"),
            r#"{"execution": {"max_iterations": 50}}"#,
        )
        .unwrap();

        // Set nested env var: SWELL_EXECUTION_MAX_ITERATIONS -> execution.max_iterations
        std::env::set_var("SWELL_EXECUTION_MAX_ITERATIONS", "100");

        let loader = ConfigLoader::new().with_project_path(temp.path());
        let config = loader.load().unwrap();

        // Env var should override the file value
        let max_iter = config
            .get("execution.max_iterations")
            .expect("execution.max_iterations should be present")
            .as_u64()
            .expect("should be a number");

        assert_eq!(max_iter, 100, "Env var should override file value");

        std::env::remove_var("SWELL_EXECUTION_MAX_ITERATIONS");
    }
}
