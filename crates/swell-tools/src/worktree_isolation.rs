//! Per-worktree environment isolation with separate PATH, env vars, and filesystem view.
//!
//! This module provides isolated execution environments for git worktrees, ensuring that
//! each worktree has its own PATH, environment variables, and filesystem scope.
//!
//! ## Features
//!
//! - **Isolated PATH**: Each worktree can have its own bin directory prepended to PATH
//! - **Scoped environment variables**: Environment variables can be scoped per worktree
//! - **Filesystem view**: Worktree isolation can restrict filesystem access
//!
//! ## Usage
//!
//! ```rust,ignore
//! use swell_tools::worktree_isolation::{WorktreeIsolation, WorktreeIsolationConfig};
//! use std::path::PathBuf;
//!
//! let config = WorktreeIsolationConfig::default()
//!     .with_worktree_path("/path/to/worktree")
//!     .with_local_bin("bin");
//!
//! let isolation = WorktreeIsolation::new(config);
//! let env = isolation.get_isolated_env().await;
//! ```

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::debug;

/// Configuration for worktree isolation
#[derive(Debug, Clone)]
pub struct WorktreeIsolationConfig {
    /// Path to the worktree root
    pub worktree_path: PathBuf,
    /// Local bin directory relative to worktree (e.g., "bin" or ".local/bin")
    pub local_bin: Option<PathBuf>,
    /// Additional environment variables scoped to this worktree
    pub env_vars: HashMap<String, String>,
    /// Whether to inherit parent environment variables
    pub inherit_parent_env: bool,
    /// Filesystem root - commands will be confined to this path and below
    pub filesystem_root: Option<PathBuf>,
    /// Additional paths to expose (read-only by default)
    pub exposed_paths: Vec<PathBuf>,
}

impl Default for WorktreeIsolationConfig {
    fn default() -> Self {
        Self {
            worktree_path: PathBuf::from("."),
            local_bin: None,
            env_vars: HashMap::new(),
            inherit_parent_env: true,
            filesystem_root: None,
            exposed_paths: Vec::new(),
        }
    }
}

impl WorktreeIsolationConfig {
    /// Set the worktree path
    pub fn with_worktree_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.worktree_path = path.into();
        self
    }

    /// Set the local bin directory relative to worktree
    pub fn with_local_bin(mut self, bin: impl Into<PathBuf>) -> Self {
        self.local_bin = Some(bin.into());
        self
    }

    /// Add an isolated environment variable
    pub fn with_env_var(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env_vars.insert(key.into(), value.into());
        self
    }

    /// Add multiple isolated environment variables
    pub fn with_env_vars<I, K, V>(mut self, vars: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        for (key, value) in vars {
            self.env_vars.insert(key.into(), value.into());
        }
        self
    }

    /// Set whether to inherit parent environment variables
    pub fn with_inherit_parent_env(mut self, inherit: bool) -> Self {
        self.inherit_parent_env = inherit;
        self
    }

    /// Set the filesystem root for confinement
    pub fn with_filesystem_root(mut self, root: impl Into<PathBuf>) -> Self {
        self.filesystem_root = Some(root.into());
        self
    }

    /// Add a path to expose to the worktree
    pub fn with_exposed_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.exposed_paths.push(path.into());
        self
    }
}

/// Per-worktree environment isolation
///
/// Provides isolated environment variables, PATH, and filesystem view for each worktree.
/// This ensures that operations in one worktree don't affect others.
#[derive(Debug, Clone)]
pub struct WorktreeIsolation {
    config: WorktreeIsolationConfig,
}

impl WorktreeIsolation {
    /// Create a new WorktreeIsolation with the given configuration
    pub fn new(config: WorktreeIsolationConfig) -> Self {
        Self { config }
    }

    /// Create with default configuration for a worktree path
    pub fn for_worktree(worktree_path: impl Into<PathBuf>) -> Self {
        let config = WorktreeIsolationConfig::default().with_worktree_path(worktree_path);
        Self::new(config)
    }

    /// Get the configuration
    pub fn config(&self) -> &WorktreeIsolationConfig {
        &self.config
    }

    /// Get the worktree path
    pub fn worktree_path(&self) -> &PathBuf {
        &self.config.worktree_path
    }

    /// Get the isolated environment variables map
    ///
    /// If inherit_parent_env is true, merges with current process environment.
    /// Worktree-specific values override parent values.
    pub async fn get_env_vars(&self) -> HashMap<String, String> {
        let mut env = HashMap::new();

        // Inherit parent environment if configured
        if self.config.inherit_parent_env {
            for (key, value) in std::env::vars() {
                env.insert(key, value);
            }
        }

        // Apply worktree-specific overrides
        for (key, value) in &self.config.env_vars {
            env.insert(key.clone(), value.clone());
        }

        // Set worktree-specific variables
        let worktree_str = self.config.worktree_path.to_string_lossy();
        env.insert("SWELL_WORKTREE".to_string(), worktree_str.to_string());
        env.insert(
            "SWELL_WORKTREE_ROOT".to_string(),
            self.config.worktree_path.canonicalize()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| worktree_str.to_string()),
        );

        debug!(
            worktree = %worktree_str,
            env_vars_count = env.len(),
            "WorktreeIsolation: prepared environment variables"
        );

        env
    }

    /// Get the isolated PATH with worktree-local bin prepended
    ///
    /// The worktree-local bin directory is prepended to PATH if configured.
    /// This allows tools installed in the worktree to take precedence.
    pub async fn get_path(&self) -> String {
        use std::env;

        let mut paths: Vec<String> = Vec::new();

        // Add worktree-local bin if configured
        if let Some(local_bin) = &self.config.local_bin {
            let bin_path = self.config.worktree_path.join(local_bin);
            if bin_path.exists() {
                paths.push(bin_path.to_string_lossy().to_string());
                debug!(
                    local_bin = %bin_path.display(),
                    "WorktreeIsolation: added local bin to PATH"
                );
            } else {
                debug!(
                    local_bin = %bin_path.display(),
                    "WorktreeIsolation: local bin does not exist, skipping"
                );
            }
        }

        // Get current PATH
        if let Ok(current_path) = env::var("PATH") {
            // Parse existing PATH
            for p in current_path.split(':') {
                let p_str = p.to_string();
                // Don't duplicate if already added as local_bin
                if !paths.contains(&p_str) {
                    paths.push(p_str);
                }
            }
        }

        let result = paths.join(":");

        debug!(
            path_components = paths.len(),
            "WorktreeIsolation: built isolated PATH"
        );

        result
    }

    /// Get the complete isolated environment for command execution
    ///
    /// Returns environment variables with isolated PATH and worktree-specific vars.
    pub async fn get_isolated_env(&self) -> HashMap<String, String> {
        let mut env = self.get_env_vars().await;
        env.insert("PATH".to_string(), self.get_path().await);
        env
    }

    /// Build command arguments with proper environment
    ///
    /// Returns the environment map and PATH string for use with tokio::process::Command.
    pub async fn build_command_env(&self) -> (HashMap<String, String>, String) {
        let env = self.get_isolated_env().await;
        let path = env.get("PATH").cloned().unwrap_or_default();
        (env, path)
    }

    /// Get the filesystem root for this worktree
    ///
    /// Returns the effective filesystem root (worktree path or configured root).
    pub fn get_filesystem_root(&self) -> PathBuf {
        self.config
            .filesystem_root
            .clone()
            .unwrap_or_else(|| self.config.worktree_path.clone())
    }

    /// Check if a path is within the worktree's filesystem scope
    pub fn is_path_in_scope(&self, path: &Path) -> bool {
        let root = self.get_filesystem_root();

        // Canonicalize both paths for accurate comparison
        if let (Ok(root_canonical), Ok(path_canonical)) = (
            root.canonicalize(),
            path.to_path_buf().canonicalize(),
        ) {
            // Check if path starts with root
            path_canonical.starts_with(&root_canonical)
        } else {
            // Fallback to string comparison
            path.starts_with(&root)
        }
    }

    /// Get list of allowed paths for sandboxing
    pub fn get_allowed_paths(&self) -> Vec<PathBuf> {
        let mut paths = vec![self.get_filesystem_root()];

        for exposed in &self.config.exposed_paths {
            if !paths.contains(exposed) {
                paths.push(exposed.clone());
            }
        }

        paths
    }
}

impl Default for WorktreeIsolation {
    fn default() -> Self {
        Self::new(WorktreeIsolationConfig::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_config() -> WorktreeIsolationConfig {
        let temp_dir = tempfile::tempdir().unwrap();
        WorktreeIsolationConfig::default()
            .with_worktree_path(temp_dir.path())
            .with_local_bin("bin")
            .with_env_var("TEST_VAR", "test_value")
            .with_env_var("WORKTREE_ID", "test-worktree-123")
    }

    fn create_test_isolation() -> WorktreeIsolation {
        WorktreeIsolation::new(create_test_config())
    }

    #[tokio::test]
    async fn test_worktree_isolation_config() {
        let config = create_test_config();
        assert_eq!(config.env_vars.len(), 2);
        assert!(config.local_bin.is_some());
        assert!(config.inherit_parent_env);
    }

    #[tokio::test]
    async fn test_worktree_isolation_default() {
        let isolation = WorktreeIsolation::default();
        assert_eq!(
            isolation.config().worktree_path,
            PathBuf::from(".")
        );
    }

    #[tokio::test]
    async fn test_worktree_isolation_for_worktree() {
        let isolation = WorktreeIsolation::for_worktree("/tmp/test-worktree");
        assert_eq!(
            isolation.config().worktree_path,
            PathBuf::from("/tmp/test-worktree")
        );
    }

    #[tokio::test]
    async fn test_get_env_vars_without_inherit() {
        let config = WorktreeIsolationConfig::default()
            .with_worktree_path("/test")
            .with_inherit_parent_env(false)
            .with_env_var("ISOLATED_VAR", "isolated_value");

        let isolation = WorktreeIsolation::new(config);
        let env = isolation.get_env_vars().await;

        // Should only have the explicitly set variables
        assert_eq!(env.get("ISOLATED_VAR"), Some(&"isolated_value".to_string()));
        // SWELL_WORKTREE should always be set
        assert_eq!(env.get("SWELL_WORKTREE"), Some(&"/test".to_string()));
    }

    #[tokio::test]
    async fn test_get_env_vars_with_inherit() {
        let config = WorktreeIsolationConfig::default()
            .with_worktree_path("/test")
            .with_inherit_parent_env(true)
            .with_env_var("TEST_VAR", "overridden_value");

        let isolation = WorktreeIsolation::new(config);
        let env = isolation.get_env_vars().await;

        // Should have inherited HOME (if set in test environment)
        // And should have our override
        assert_eq!(env.get("TEST_VAR"), Some(&"overridden_value".to_string()));
        assert_eq!(env.get("SWELL_WORKTREE"), Some(&"/test".to_string()));
    }

    #[tokio::test]
    async fn test_get_path_without_local_bin() {
        let config = WorktreeIsolationConfig::default()
            .with_worktree_path("/test")
            .with_inherit_parent_env(false);

        let isolation = WorktreeIsolation::new(config);
        let path = isolation.get_path().await;

        // Without local_bin set, should just have existing PATH
        // (might be empty or have system paths)
        if !path.is_empty() {
            assert!(path.contains(':') || std::path::Path::new(&path).exists());
        }
    }

    #[tokio::test]
    async fn test_get_path_with_local_bin() {
        let temp_dir = tempfile::tempdir().unwrap();
        let bin_dir = temp_dir.path().join("bin");
        std::fs::create_dir(&bin_dir).unwrap();

        let config = WorktreeIsolationConfig::default()
            .with_worktree_path(temp_dir.path())
            .with_local_bin("bin")
            .with_inherit_parent_env(false);

        let isolation = WorktreeIsolation::new(config);
        let path = isolation.get_path().await;

        // Should start with the local bin path
        assert!(path.starts_with(&*bin_dir.to_string_lossy()));
    }

    #[tokio::test]
    async fn test_get_isolated_env() {
        let isolation = create_test_isolation();
        let env = isolation.get_isolated_env().await;

        // Should have PATH set
        assert!(env.contains_key("PATH"));
        // Should have worktree variables
        assert!(env.contains_key("SWELL_WORKTREE"));
        // Should have custom vars
        assert_eq!(env.get("TEST_VAR"), Some(&"test_value".to_string()));
    }

    #[tokio::test]
    async fn test_build_command_env() {
        let isolation = create_test_isolation();
        let (env, path) = isolation.build_command_env().await;

        assert!(env.contains_key("PATH"));
        assert!(env.contains_key("PATH"));
        assert_eq!(path.as_str(), env.get("PATH").unwrap().as_str());
    }

    #[tokio::test]
    async fn test_get_filesystem_root() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = WorktreeIsolationConfig::default()
            .with_worktree_path(temp_dir.path())
            .with_filesystem_root("/custom/root");

        let isolation = WorktreeIsolation::new(config);
        let root = isolation.get_filesystem_root();

        assert_eq!(root, PathBuf::from("/custom/root"));
    }

    #[tokio::test]
    async fn test_get_filesystem_root_default() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = WorktreeIsolationConfig::default()
            .with_worktree_path(temp_dir.path());

        let isolation = WorktreeIsolation::new(config);
        let root = isolation.get_filesystem_root();

        assert_eq!(root, temp_dir.path().to_path_buf());
    }

    #[tokio::test]
    async fn test_is_path_in_scope() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = WorktreeIsolationConfig::default()
            .with_worktree_path(temp_dir.path());

        let isolation = WorktreeIsolation::new(config);

        // Path within worktree should be in scope
        let inner_path = temp_dir.path().join("src").join("main.rs");
        assert!(isolation.is_path_in_scope(&inner_path));

        // Path outside worktree should not be in scope
        let outer_path = PathBuf::from("/usr/bin/ls");
        assert!(!isolation.is_path_in_scope(&outer_path));
    }

    #[tokio::test]
    async fn test_get_allowed_paths() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = WorktreeIsolationConfig::default()
            .with_worktree_path(temp_dir.path())
            .with_exposed_path("/tmp/shared")
            .with_exposed_path("/home/user");

        let isolation = WorktreeIsolation::new(config);
        let paths = isolation.get_allowed_paths();

        // Should have worktree root plus exposed paths
        assert!(paths.contains(&temp_dir.path().to_path_buf()));
        assert!(paths.contains(&PathBuf::from("/tmp/shared")));
        assert!(paths.contains(&PathBuf::from("/home/user")));
    }

    #[tokio::test]
    async fn test_worktree_isolation_clone() {
        let isolation = create_test_isolation();
        let cloned = isolation.clone();

        assert_eq!(isolation.config().worktree_path, cloned.config().worktree_path);
    }

    #[tokio::test]
    async fn test_multiple_env_vars() {
        let config = WorktreeIsolationConfig::default()
            .with_worktree_path("/test")
            .with_env_vars([
                ("VAR1", "value1"),
                ("VAR2", "value2"),
                ("VAR3", "value3"),
            ]);

        let isolation = WorktreeIsolation::new(config);
        let env = isolation.get_env_vars().await;

        assert_eq!(env.get("VAR1"), Some(&"value1".to_string()));
        assert_eq!(env.get("VAR2"), Some(&"value2".to_string()));
        assert_eq!(env.get("VAR3"), Some(&"value3".to_string()));
    }
}
