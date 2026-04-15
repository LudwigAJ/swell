//! Session management with workspace mismatch protection.
//!
//! This module provides session persistence with workspace fingerprint validation
//! to prevent cross-workspace session contamination.
//!
//! # Session Lifecycle
//!
//! 1. **Session creation**: A new session is created with the current workspace fingerprint.
//! 2. **Session loading**: When loading, the stored fingerprint is compared against the
//!    current workspace fingerprint. A mismatch causes rejection.
//! 3. **Session listing**: Sessions are ordered by `updated_at_ms` (most recent first).
//!
//! # Workspace Mismatch Protection
//!
//! Sessions are tied to a specific workspace through the `workspace_fingerprint` field.
//! When `load_session()` is called, it validates that the current workspace fingerprint
//! matches the stored fingerprint. If not, loading fails with `WorkspaceMismatch` error.

use crate::workspace_fingerprint;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use thiserror::Error;
use uuid::Uuid;

/// Errors that can occur during session operations
#[derive(Debug, Error)]
pub enum SessionError {
    #[error("Session not found: {0}")]
    NotFound(Uuid),

    #[error("Workspace mismatch: session was created in workspace with fingerprint {stored}, but current workspace has fingerprint {current}")]
    WorkspaceMismatch { stored: u64, current: u64 },

    #[error("Failed to compute workspace fingerprint: {0}")]
    FingerprintError(String),

    #[error("Session store error: {0}")]
    StoreError(String),
}

/// Session metadata stored for each session.
///
/// This struct contains all session metadata including the workspace fingerprint
/// used for cross-workspace contamination prevention.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMetadata {
    /// Unique session identifier
    pub id: Uuid,
    /// User-assigned alias for easy resume (e.g., "my-feature")
    pub alias: Option<String>,
    /// Workspace fingerprint (FNV-1a hash of canonical path)
    pub workspace_fingerprint: u64,
    /// Creation timestamp (milliseconds since epoch)
    pub created_at_ms: i64,
    /// Last update timestamp (milliseconds since epoch)
    pub updated_at_ms: i64,
    /// Current task ID (if any)
    pub current_task_id: Option<Uuid>,
    /// Session state (Active, Paused, Completed)
    pub state: SessionState,
}

/// Session state
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SessionState {
    Active,
    Paused,
    Completed,
}

impl std::fmt::Display for SessionState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionState::Active => write!(f, "ACTIVE"),
            SessionState::Paused => write!(f, "PAUSED"),
            SessionState::Completed => write!(f, "COMPLETED"),
        }
    }
}

impl SessionMetadata {
    /// Create a new session metadata with fingerprint from current workspace.
    ///
    /// # Errors
    /// Returns `SessionError::FingerprintError` if workspace fingerprint cannot be computed.
    pub fn new(
        id: Uuid,
        workspace_path: impl AsRef<std::path::Path>,
        alias: Option<String>,
    ) -> Result<Self, SessionError> {
        let fingerprint = workspace_fingerprint(workspace_path)
            .map_err(SessionError::FingerprintError)?;

        let now = Utc::now().timestamp_millis();

        Ok(Self {
            id,
            alias,
            workspace_fingerprint: fingerprint,
            created_at_ms: now,
            updated_at_ms: now,
            current_task_id: None,
            state: SessionState::Active,
        })
    }

    /// Update the `updated_at_ms` timestamp to now.
    pub fn touch(&mut self) {
        self.updated_at_ms = Utc::now().timestamp_millis();
    }

    /// Validate that the current workspace matches this session's workspace fingerprint.
    ///
    /// # Arguments
    /// * `workspace_path` - Path to the current workspace
    ///
    /// # Returns
    /// * `Ok(())` if workspace matches
    /// * `Err(SessionError::WorkspaceMismatch)` if workspace doesn't match
    pub fn validate_workspace(
        &self,
        workspace_path: impl AsRef<std::path::Path>,
    ) -> Result<(), SessionError> {
        let current_fingerprint = workspace_fingerprint(workspace_path)
            .map_err(SessionError::FingerprintError)?;

        if self.workspace_fingerprint != current_fingerprint {
            return Err(SessionError::WorkspaceMismatch {
                stored: self.workspace_fingerprint,
                current: current_fingerprint,
            });
        }

        Ok(())
    }
}

/// Session store trait for persisting sessions.
///
/// Implementations can use any backing store (SQLite, PostgreSQL, in-memory, etc.)
#[async_trait::async_trait]
pub trait SessionStore: Send + Sync {
    /// Save a session.
    async fn save_session(&self, metadata: SessionMetadata) -> Result<(), SessionError>;

    /// Load a session by ID.
    ///
    /// If the session exists but workspace fingerprint doesn't match current workspace,
    /// returns `Err(SessionError::WorkspaceMismatch)`.
    async fn load_session(
        &self,
        id: Uuid,
        workspace_path: impl AsRef<std::path::Path> + Send,
    ) -> Result<Option<SessionMetadata>, SessionError>;

    /// Load a session by alias.
    ///
    /// If the session exists but workspace fingerprint doesn't match current workspace,
    /// returns `Err(SessionError::WorkspaceMismatch)`.
    async fn load_session_by_alias(
        &self,
        alias: &str,
        workspace_path: impl AsRef<std::path::Path> + Send,
    ) -> Result<Option<SessionMetadata>, SessionError>;

    /// List all sessions, ordered by `updated_at_ms` descending (most recent first).
    async fn list_sessions(&self) -> Result<Vec<SessionMetadata>, SessionError>;

    /// Load a session by workspace path.
    ///
    /// Returns the most recent session in the given workspace (by updated_at_ms).
    /// If the session exists but workspace fingerprint doesn't match current workspace,
    /// returns `Err(SessionError::WorkspaceMismatch)`.
    async fn load_session_by_workspace_path(
        &self,
        workspace_path: impl AsRef<std::path::Path> + Send,
    ) -> Result<Option<SessionMetadata>, SessionError>;

    /// Delete a session by ID.
    async fn delete_session(&self, id: Uuid) -> Result<(), SessionError>;
}

// In-memory session store for testing
#[derive(Debug, Clone, Default)]
pub struct InMemorySessionStore {
    sessions: std::sync::Arc<tokio::sync::RwLock<std::collections::HashMap<Uuid, SessionMetadata>>>,
    aliases: std::sync::Arc<tokio::sync::RwLock<std::collections::HashMap<String, Uuid>>>,
}

impl InMemorySessionStore {
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
            aliases: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        }
    }

    pub async fn clear(&self) {
        self.sessions.write().await.clear();
        self.aliases.write().await.clear();
    }
}

#[async_trait::async_trait]
impl SessionStore for InMemorySessionStore {
    async fn save_session(&self, metadata: SessionMetadata) -> Result<(), SessionError> {
        let mut sessions = self.sessions.write().await;
        let mut aliases = self.aliases.write().await;

        // Remove old alias if present
        if let Some(old_metadata) = sessions.get(&metadata.id) {
            if let Some(ref old_alias) = old_metadata.alias {
                aliases.remove(old_alias);
            }
        }

        // Add new alias mapping
        if let Some(ref alias) = metadata.alias {
            aliases.insert(alias.clone(), metadata.id);
        }

        sessions.insert(metadata.id, metadata);
        Ok(())
    }

    async fn load_session(
        &self,
        id: Uuid,
        workspace_path: impl AsRef<std::path::Path> + Send,
    ) -> Result<Option<SessionMetadata>, SessionError> {
        let sessions = self.sessions.read().await;

        if let Some(metadata) = sessions.get(&id) {
            // Validate workspace fingerprint
            metadata.validate_workspace(&workspace_path)?;
            Ok(Some(metadata.clone()))
        } else {
            Ok(None)
        }
    }

    async fn load_session_by_alias(
        &self,
        alias: &str,
        workspace_path: impl AsRef<std::path::Path> + Send,
    ) -> Result<Option<SessionMetadata>, SessionError> {
        let aliases = self.aliases.read().await;
        let sessions = self.sessions.read().await;

        if let Some(id) = aliases.get(alias) {
            if let Some(metadata) = sessions.get(id) {
                // Validate workspace fingerprint
                metadata.validate_workspace(&workspace_path)?;
                Ok(Some(metadata.clone()))
            } else {
                Ok(None)
            }
        } else {
            Ok(None)
        }
    }

    async fn list_sessions(&self) -> Result<Vec<SessionMetadata>, SessionError> {
        let sessions = self.sessions.read().await;

        let mut result: Vec<SessionMetadata> = sessions.values().cloned().collect();

        // Sort by updated_at_ms descending (most recent first)
        result.sort_by(|a, b| b.updated_at_ms.cmp(&a.updated_at_ms));

        Ok(result)
    }

    async fn load_session_by_workspace_path(
        &self,
        workspace_path: impl AsRef<std::path::Path> + Send,
    ) -> Result<Option<SessionMetadata>, SessionError> {
        let workspace_fingerprint = workspace_fingerprint(&workspace_path)
            .map_err(SessionError::FingerprintError)?;

        let sessions = self.sessions.read().await;

        // Find all sessions in this workspace, then pick the most recent by updated_at_ms
        let mut candidates: Vec<&SessionMetadata> = sessions
            .values()
            .filter(|s| s.workspace_fingerprint == workspace_fingerprint)
            .collect();

        if candidates.is_empty() {
            return Ok(None);
        }

        // Pick the most recent session
        candidates.sort_by(|a, b| b.updated_at_ms.cmp(&a.updated_at_ms));

        let session = candidates.remove(0);
        Ok(Some(session.clone()))
    }

    async fn delete_session(&self, id: Uuid) -> Result<(), SessionError> {
        let mut sessions = self.sessions.write().await;
        let mut aliases = self.aliases.write().await;

        if let Some(metadata) = sessions.remove(&id) {
            if let Some(ref alias) = metadata.alias {
                aliases.remove(alias);
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // -------------------------------------------------------------------------
    // VAL-SESS-001: Workspace fingerprinting via FNV-1a hash of canonical path
    // -------------------------------------------------------------------------

    #[test]
    fn test_workspace_fingerprint_deterministic() {
        let temp_dir = TempDir::new().unwrap();
        let path = temp_dir.path();

        let fp1 = workspace_fingerprint(path).unwrap();
        let fp2 = workspace_fingerprint(path).unwrap();

        assert_eq!(fp1, fp2, "Same path should always produce same fingerprint");
    }

    #[test]
    fn test_workspace_fingerprint_same_via_symlink() {
        let temp_dir = TempDir::new().unwrap();
        let real_path = temp_dir.path();

        // Create a symlink to the real directory
        let symlink_path = temp_dir.path().join("symlink_to_dir");
        std::os::unix::fs::symlink(real_path, &symlink_path).unwrap();

        let fp_real = workspace_fingerprint(real_path).unwrap();
        let fp_symlink = workspace_fingerprint(&symlink_path).unwrap();

        assert_eq!(
            fp_real, fp_symlink,
            "Symlink and real path should produce the same fingerprint"
        );
    }

    #[test]
    fn test_workspace_fingerprint_different_for_different_dirs() {
        let temp_dir1 = TempDir::new().unwrap();
        let temp_dir2 = TempDir::new().unwrap();

        let fp1 = workspace_fingerprint(temp_dir1.path()).unwrap();
        let fp2 = workspace_fingerprint(temp_dir2.path()).unwrap();

        assert_ne!(
            fp1, fp2,
            "Different directories should produce different fingerprints"
        );
    }

    // -------------------------------------------------------------------------
    // VAL-SESS-002: Session load rejects workspace mismatch
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn test_session_load_rejects_workspace_mismatch() {
        let store = InMemorySessionStore::new();

        // Create two different temp directories
        let dir_a = TempDir::new().unwrap();
        let dir_b = TempDir::new().unwrap();

        // Create session in dir_a
        let session_id = Uuid::new_v4();
        let metadata = SessionMetadata::new(session_id, dir_a.path(), None).unwrap();
        store.save_session(metadata.clone()).await.unwrap();

        // Verify session loads successfully in dir_a
        let loaded = store
            .load_session(session_id, dir_a.path())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(loaded.id, session_id);

        // Attempt to load session in dir_b should fail with workspace mismatch
        let result = store.load_session(session_id, dir_b.path()).await;
        assert!(result.is_err());

        let err = result.unwrap_err();
        match err {
            SessionError::WorkspaceMismatch { stored, current } => {
                assert_eq!(stored, metadata.workspace_fingerprint);
                // current fingerprint will be different for dir_b
                assert_ne!(stored, current);
            }
            _ => panic!("Expected WorkspaceMismatch error"),
        }
    }

    #[tokio::test]
    async fn test_session_load_by_alias_rejects_workspace_mismatch() {
        let store = InMemorySessionStore::new();

        let dir_a = TempDir::new().unwrap();
        let dir_b = TempDir::new().unwrap();

        let session_id = Uuid::new_v4();
        let metadata = SessionMetadata::new(session_id, dir_a.path(), Some("my-session".to_string())).unwrap();
        store.save_session(metadata.clone()).await.unwrap();

        // Load by alias works in dir_a
        let loaded = store
            .load_session_by_alias("my-session", dir_a.path())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(loaded.id, session_id);

        // Load by alias fails in dir_b
        let result = store.load_session_by_alias("my-session", dir_b.path()).await;
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), SessionError::WorkspaceMismatch { .. }));
    }

    #[tokio::test]
    async fn test_session_load_returns_none_for_nonexistent() {
        let store = InMemorySessionStore::new();
        let temp_dir = TempDir::new().unwrap();

        let result = store
            .load_session(Uuid::new_v4(), temp_dir.path())
            .await
            .unwrap();

        assert!(result.is_none(), "Loading nonexistent session should return None");
    }

    // -------------------------------------------------------------------------
    // VAL-SESS-003: Sessions ordered by updated_at_ms timestamp
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn test_sessions_ordered_by_updated_at_ms_descending() {
        let store = InMemorySessionStore::new();
        let temp_dir = TempDir::new().unwrap();

        // Create three sessions with different updated_at_ms values
        let session1_id = Uuid::new_v4();
        let mut session1 = SessionMetadata::new(session1_id, temp_dir.path(), None).unwrap();
        session1.updated_at_ms = 100;

        let session2_id = Uuid::new_v4();
        let mut session2 = SessionMetadata::new(session2_id, temp_dir.path(), None).unwrap();
        session2.updated_at_ms = 300;

        let session3_id = Uuid::new_v4();
        let mut session3 = SessionMetadata::new(session3_id, temp_dir.path(), None).unwrap();
        session3.updated_at_ms = 200;

        // Save in order: 1, 2, 3
        store.save_session(session1).await.unwrap();
        store.save_session(session2).await.unwrap();
        store.save_session(session3).await.unwrap();

        // List should return [300, 200, 100] order (descending by updated_at_ms)
        let sessions = store.list_sessions().await.unwrap();
        assert_eq!(sessions.len(), 3);
        assert_eq!(sessions[0].updated_at_ms, 300);
        assert_eq!(sessions[1].updated_at_ms, 200);
        assert_eq!(sessions[2].updated_at_ms, 100);
    }

    #[tokio::test]
    async fn test_session_update_resets_updated_at_ms() {
        let store = InMemorySessionStore::new();
        let temp_dir = TempDir::new().unwrap();

        let session_id = Uuid::new_v4();
        let mut metadata = SessionMetadata::new(session_id, temp_dir.path(), None).unwrap();
        metadata.updated_at_ms = 100;
        store.save_session(metadata.clone()).await.unwrap();

        // Touch to update timestamp
        metadata.touch();
        let new_timestamp = metadata.updated_at_ms;
        assert!(new_timestamp > 100, "Touched timestamp should be greater than 100");

        // Save updated session
        store.save_session(metadata.clone()).await.unwrap();

        // List should show the updated timestamp
        let sessions = store.list_sessions().await.unwrap();
        assert_eq!(sessions[0].updated_at_ms, new_timestamp);
    }

    // -------------------------------------------------------------------------
    // VAL-SESS-004: Session resume supports alias, ID, and path reference forms
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn test_session_resume_by_id() {
        let store = InMemorySessionStore::new();
        let temp_dir = TempDir::new().unwrap();

        let session_id = Uuid::new_v4();
        let metadata = SessionMetadata::new(session_id, temp_dir.path(), None).unwrap();
        store.save_session(metadata.clone()).await.unwrap();

        // Resume by ID works
        let loaded = store
            .load_session(session_id, temp_dir.path())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(loaded.id, session_id);
    }

    #[tokio::test]
    async fn test_session_resume_by_alias() {
        let store = InMemorySessionStore::new();
        let temp_dir = TempDir::new().unwrap();

        let session_id = Uuid::new_v4();
        let metadata = SessionMetadata::new(
            session_id,
            temp_dir.path(),
            Some("my-feature".to_string()),
        )
        .unwrap();
        store.save_session(metadata.clone()).await.unwrap();

        // Resume by alias works
        let loaded = store
            .load_session_by_alias("my-feature", temp_dir.path())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(loaded.id, session_id);
    }

    #[tokio::test]
    async fn test_session_resume_by_workspace_path() {
        let store = InMemorySessionStore::new();
        let temp_dir = TempDir::new().unwrap();

        let session_id = Uuid::new_v4();
        let metadata = SessionMetadata::new(session_id, temp_dir.path(), None).unwrap();
        store.save_session(metadata.clone()).await.unwrap();

        // Resume by workspace path works
        let loaded = store
            .load_session_by_workspace_path(temp_dir.path())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(loaded.id, session_id);
    }

    #[tokio::test]
    async fn test_session_resume_by_workspace_path_returns_most_recent() {
        let store = InMemorySessionStore::new();
        let temp_dir = TempDir::new().unwrap();

        // Create two sessions in the same workspace
        let session1_id = Uuid::new_v4();
        let mut session1 = SessionMetadata::new(session1_id, temp_dir.path(), None).unwrap();
        session1.updated_at_ms = 100;

        let session2_id = Uuid::new_v4();
        let mut session2 = SessionMetadata::new(session2_id, temp_dir.path(), None).unwrap();
        session2.updated_at_ms = 200; // More recent

        store.save_session(session1).await.unwrap();
        store.save_session(session2).await.unwrap();

        // Resume by workspace path returns the most recent one
        let loaded = store
            .load_session_by_workspace_path(temp_dir.path())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(loaded.id, session2_id);
        assert_eq!(loaded.updated_at_ms, 200);
    }

    #[tokio::test]
    async fn test_session_resume_nonexistent_alias_returns_none() {
        let store = InMemorySessionStore::new();
        let temp_dir = TempDir::new().unwrap();

        let result = store
            .load_session_by_alias("nonexistent", temp_dir.path())
            .await
            .unwrap();

        assert!(result.is_none(), "Nonexistent alias should return None");
    }

    // -------------------------------------------------------------------------
    // Additional tests
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn test_delete_session() {
        let store = InMemorySessionStore::new();
        let temp_dir = TempDir::new().unwrap();

        let session_id = Uuid::new_v4();
        let metadata = SessionMetadata::new(session_id, temp_dir.path(), Some("to-delete".to_string())).unwrap();
        store.save_session(metadata.clone()).await.unwrap();

        // Verify session exists
        let loaded = store
            .load_session(session_id, temp_dir.path())
            .await
            .unwrap();
        assert!(loaded.is_some());

        // Delete session
        store.delete_session(session_id).await.unwrap();

        // Verify session no longer exists
        let loaded = store
            .load_session(session_id, temp_dir.path())
            .await
            .unwrap();
        assert!(loaded.is_none());
    }

    #[test]
    fn test_session_metadata_new_validates_fingerprint() {
        // Nonexistent path should return FingerprintError
        let result = SessionMetadata::new(
            Uuid::new_v4(),
            "/nonexistent/path/that/does/not/exist",
            None,
        );

        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), SessionError::FingerprintError(_)));
    }

    #[test]
    fn test_session_state_display() {
        assert_eq!(format!("{}", SessionState::Active), "ACTIVE");
        assert_eq!(format!("{}", SessionState::Paused), "PAUSED");
        assert_eq!(format!("{}", SessionState::Completed), "COMPLETED");
    }
}
