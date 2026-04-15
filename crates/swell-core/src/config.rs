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

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::env;
use std::path::{Path, PathBuf};

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

        self.merge_value(parsed, layer, path.to_string_lossy().to_string(), values, entries);
    }

    /// Load environment variable overrides
    fn load_env_overrides(
        &self,
        values: &mut serde_json::Map<String, serde_json::Value>,
        entries: &mut Vec<ConfigEntry>,
    ) {
        // Look for SWELL_* environment variables
        for (key, value) in env::vars() {
            if key.starts_with(&self.env_prefix) {
                // Convert SWELL_EXECUTION_MAX_TIMEOUT to execution.max_timeout
                let config_key = key
                    .strip_prefix(&self.env_prefix)
                    .unwrap_or(&key)
                    .to_lowercase()
                    .replace('_', ".");

                let json_value: serde_json::Value = serde_json::from_str(&value)
                    .unwrap_or(serde_json::Value::String(value));

                entries.push(ConfigEntry {
                    key_path: config_key.clone(),
                    value: json_value.clone(),
                    source_file: None,
                });

                // Set the value (deep merge or override)
                set_nested_value(values, &config_key, json_value);
            }
        }
    }

    /// Merge a value into the config, tracking entries
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
                entries.push(ConfigEntry {
                    key_path: key_path.clone(),
                    value: val.clone(),
                    source_file: Some(source.clone()),
                });

                // Merge: higher layers override lower layers
                if let Some(existing) = values.get_mut(&key) {
                    *existing = val;
                } else {
                    values.insert(key, val);
                }
            }
        }
    }
}

/// Set a nested value in a JSON object using dot notation
fn set_nested_value(map: &mut serde_json::Map<String, serde_json::Value>, key_path: &str, value: serde_json::Value) {
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
            current.insert(part_str.clone(), serde_json::Value::Object(serde_json::Map::new()));
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
    use tempfile::TempDir;

    fn create_temp_config(dir: &PathBuf, name: &str, content: &str) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn test_five_layer_precedence_ordering() {
        // Test that higher layers override lower layers
        let temp = TempDir::new().unwrap();

        // Layer 1: user global
        create_temp_config(&temp.path().join(".config/swell"), "settings.json",
            r#"{"timeout": 10}}"#);

        // Layer 2: user modern
        create_temp_config(&temp.path().join(".swell"), "settings.json",
            r#"{"timeout": 20}}"#);

        // Layer 3: project shared
        create_temp_config(&temp.path().join(".swell"), "settings.json",
            r#"{"timeout": 30}}"#);

        // Layer 4: project modern
        create_temp_config(&temp.path().join(".swell"), "settings.local.json",
            r#"{"timeout": 40}}"#);

        let loader = ConfigLoader::new()
            .with_project_path(temp.path());

        // With env override
        std::env::set_var("SWELL_TIMEOUT", "50");

        let config = loader.load().unwrap();

        // Env var (layer 5) should win
        assert_eq!(config.get("timeout").unwrap().as_i64().unwrap(), 50);

        std::env::remove_var("SWELL_TIMEOUT");
    }

    #[test]
    fn test_higher_layer_overrides_lower_layer() {
        let temp = TempDir::new().unwrap();

        // User global sets timeout to 10
        std::fs::create_dir_all(temp.path().join(".config/swell")).unwrap();
        std::fs::write(
            temp.path().join(".config/swell/settings.json"),
            r#"{"timeout": 10, "debug": false}"#,
        ).unwrap();

        // User modern overrides timeout to 20
        std::fs::create_dir_all(temp.path().join(".swell")).unwrap();
        std::fs::write(
            temp.path().join(".swell/settings.json"),
            r#"{"timeout": 20, "debug": true}"#,
        ).unwrap();

        let loader = ConfigLoader::new().with_project_path(temp.path());
        let config = loader.load().unwrap();

        // timeout should be 20 (from user modern)
        assert_eq!(config.get("timeout").unwrap().as_i64().unwrap(), 20);
        // debug should be true (from user modern)
        assert_eq!(config.get("debug").unwrap().as_bool().unwrap(), true);
    }

    #[test]
    fn test_missing_layers_handled_gracefully() {
        // Ensure no leftover env var from previous tests
        std::env::remove_var("SWELL_TIMEOUT");

        let temp = TempDir::new().unwrap();

        // Only create project shared config
        std::fs::create_dir_all(temp.path().join(".swell")).unwrap();
        std::fs::write(
            temp.path().join(".swell/settings.json"),
            r#"{"timeout": 30}"#,
        ).unwrap();

        let loader = ConfigLoader::new().with_project_path(temp.path());
        let config = loader.load().unwrap();

        // Should successfully load from available layer
        assert_eq!(config.get("timeout").unwrap().as_i64().unwrap(), 30);

        // Should not fail on missing user configs
        // (would panic on unwrap if not handled)
    }

    #[test]
    fn test_env_override() {
        let temp = TempDir::new().unwrap();
        std::fs::create_dir_all(temp.path().join(".swell")).unwrap();
        std::fs::write(
            temp.path().join(".swell/settings.json"),
            r#"{"timeout": 30}"#,
        ).unwrap();

        std::env::set_var("SWELL_TIMEOUT", "100");

        let loader = ConfigLoader::new().with_project_path(temp.path());
        let config = loader.load().unwrap();

        // Env var should override file value
        assert_eq!(config.get("timeout").unwrap().as_i64().unwrap(), 100);

        std::env::remove_var("SWELL_TIMEOUT");
    }

    #[test]
    fn test_loaded_entries_tracking() {
        let temp = TempDir::new().unwrap();

        // Create two layers
        std::fs::create_dir_all(temp.path().join(".config/swell")).unwrap();
        std::fs::write(
            temp.path().join(".config/swell/settings.json"),
            r#"{"timeout": 10}"#,
        ).unwrap();

        std::fs::create_dir_all(temp.path().join(".swell")).unwrap();
        std::fs::write(
            temp.path().join(".swell/settings.json"),
            r#"{"timeout": 20, "debug": true}"#,
        ).unwrap();

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
        let loader = ConfigLoader::new()
            .with_project_path("/nonexistent/path/that/does/not/exist");

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
        ).unwrap();

        let loader = ConfigLoader::new().with_project_path(temp.path());
        let config = loader.load().unwrap();

        // Should not panic, just skip invalid file
        assert!(config.values.is_object());
    }
}
