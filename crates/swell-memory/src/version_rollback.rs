// Memory Version Rollback Module
//
// Provides version history tracking for memory entries and the ability
// to rollback to previous versions. Maintains a full audit trail of all
// rollback operations.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use uuid::Uuid;

use crate::SqliteMemoryStore;
use swell_core::{MemoryEntry, MemoryStore, SwellError};

/// A historical version of a memory entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryVersion {
    /// Unique version ID
    pub id: Uuid,
    /// ID of the memory entry this version belongs to
    pub memory_id: Uuid,
    /// Version number (increments with each update)
    pub version: u32,
    /// Content at this version
    pub content: String,
    /// Metadata at this version
    pub metadata: serde_json::Value,
    /// When this version was created
    pub created_at: DateTime<Utc>,
    /// Who/what triggered this version (e.g., "update", "rollback", "import")
    pub created_by: String,
    /// Optional reason for the change
    pub reason: Option<String>,
}

/// A rollback audit log entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RollbackAuditEntry {
    /// Unique audit entry ID
    pub id: Uuid,
    /// ID of the memory entry that was rolled back
    pub memory_id: Uuid,
    /// Version we rolled back FROM
    pub from_version: u32,
    /// Version we rolled back TO
    pub to_version: u32,
    /// When the rollback occurred
    pub timestamp: DateTime<Utc>,
    /// Who triggered the rollback
    pub triggered_by: String,
    /// Reason for the rollback (if provided)
    pub reason: Option<String>,
}

/// Result of a rollback operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RollbackResult {
    /// Whether the rollback succeeded
    pub success: bool,
    /// The memory entry after rollback (if successful)
    pub memory: Option<MemoryEntry>,
    /// Error message if failed
    pub error: Option<String>,
}

impl SqliteMemoryStore {
    /// Store a new version before updating a memory entry.
    /// This is called automatically before updates to preserve history.
    pub async fn save_version(
        &self,
        memory_id: Uuid,
        content: String,
        metadata: serde_json::Value,
        created_by: &str,
        reason: Option<&str>,
    ) -> Result<Uuid, SwellError> {
        // Get current version number
        let current_version = self.get_current_version_number(memory_id).await?;
        let new_version = current_version + 1;

        let version_id = Uuid::new_v4();
        let now = chrono::Utc::now();
        let now_str = now.to_rfc3339();

        sqlx::query(
            r#"
            INSERT INTO memory_versions (id, memory_id, version, content, metadata, created_at, created_by, reason)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(version_id.to_string())
        .bind(memory_id.to_string())
        .bind(new_version)
        .bind(&content)
        .bind(serde_json::to_string(&metadata).unwrap())
        .bind(&now_str)
        .bind(created_by)
        .bind(reason)
        .execute(self.pool.as_ref())
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        Ok(version_id)
    }

    /// Get the current version number for a memory entry
    async fn get_current_version_number(&self, memory_id: Uuid) -> Result<u32, SwellError> {
        let row: Option<(i64,)> =
            sqlx::query_as("SELECT MAX(version) FROM memory_versions WHERE memory_id = ?")
                .bind(memory_id.to_string())
                .fetch_optional(self.pool.as_ref())
                .await
                .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        Ok(row.map(|r| r.0 as u32).unwrap_or(0))
    }

    /// Get all versions for a memory entry, ordered by version number descending (newest first)
    pub async fn get_versions(&self, memory_id: Uuid) -> Result<Vec<MemoryVersion>, SwellError> {
        let rows = sqlx::query(
            r#"
            SELECT id, memory_id, version, content, metadata, created_at, created_by, reason
            FROM memory_versions
            WHERE memory_id = ?
            ORDER BY version DESC
            "#,
        )
        .bind(memory_id.to_string())
        .fetch_all(self.pool.as_ref())
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        let mut versions = Vec::new();
        for row in rows {
            let id_str: String = row.get("id");
            let memory_id_str: String = row.get("memory_id");
            let version: i64 = row.get("version");
            let content: String = row.get("content");
            let metadata_str: String = row.get("metadata");
            let created_at_str: String = row.get("created_at");
            let created_by: String = row.get("created_by");
            let reason: Option<String> = row.get("reason");

            let id = Uuid::parse_str(&id_str)
                .map_err(|e| SwellError::DatabaseError(format!("Invalid version UUID: {}", e)))?;
            let mid = Uuid::parse_str(&memory_id_str)
                .map_err(|e| SwellError::DatabaseError(format!("Invalid memory UUID: {}", e)))?;

            let created_at = DateTime::parse_from_rfc3339(&created_at_str)
                .map_err(|e| SwellError::DatabaseError(format!("Invalid timestamp: {}", e)))?
                .with_timezone(&Utc);

            let metadata: serde_json::Value = serde_json::from_str(&metadata_str)
                .map_err(|e| SwellError::DatabaseError(format!("Invalid JSON metadata: {}", e)))?;

            versions.push(MemoryVersion {
                id,
                memory_id: mid,
                version: version as u32,
                content,
                metadata,
                created_at,
                created_by,
                reason,
            });
        }

        Ok(versions)
    }

    /// Get a specific version of a memory entry
    pub async fn get_version(
        &self,
        memory_id: Uuid,
        version: u32,
    ) -> Result<Option<MemoryVersion>, SwellError> {
        let row = sqlx::query(
            r#"
            SELECT id, memory_id, version, content, metadata, created_at, created_by, reason
            FROM memory_versions
            WHERE memory_id = ? AND version = ?
            "#,
        )
        .bind(memory_id.to_string())
        .bind(version as i64)
        .fetch_optional(self.pool.as_ref())
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        match row {
            Some(r) => {
                let id_str: String = r.get("id");
                let memory_id_str: String = r.get("memory_id");
                let ver: i64 = r.get("version");
                let content: String = r.get("content");
                let metadata_str: String = r.get("metadata");
                let created_at_str: String = r.get("created_at");
                let created_by: String = r.get("created_by");
                let reason: Option<String> = r.get("reason");

                let id = Uuid::parse_str(&id_str).map_err(|e| {
                    SwellError::DatabaseError(format!("Invalid version UUID: {}", e))
                })?;
                let mid = Uuid::parse_str(&memory_id_str).map_err(|e| {
                    SwellError::DatabaseError(format!("Invalid memory UUID: {}", e))
                })?;

                let created_at = DateTime::parse_from_rfc3339(&created_at_str)
                    .map_err(|e| SwellError::DatabaseError(format!("Invalid timestamp: {}", e)))?
                    .with_timezone(&Utc);

                let metadata: serde_json::Value =
                    serde_json::from_str(&metadata_str).map_err(|e| {
                        SwellError::DatabaseError(format!("Invalid JSON metadata: {}", e))
                    })?;

                Ok(Some(MemoryVersion {
                    id,
                    memory_id: mid,
                    version: ver as u32,
                    content,
                    metadata,
                    created_at,
                    created_by,
                    reason,
                }))
            }
            None => Ok(None),
        }
    }

    /// Rollback a memory entry to a specific version
    pub async fn rollback_to_version(
        &self,
        memory_id: Uuid,
        target_version: u32,
        triggered_by: &str,
        reason: Option<&str>,
    ) -> Result<RollbackResult, SwellError> {
        // First, get the current memory entry
        let current = match self.get(memory_id).await? {
            Some(entry) => entry,
            None => {
                return Ok(RollbackResult {
                    success: false,
                    memory: None,
                    error: Some(format!("Memory entry {} not found", memory_id)),
                });
            }
        };

        // Get the target version
        let target = match self.get_version(memory_id, target_version).await? {
            Some(v) => v,
            None => {
                return Ok(RollbackResult {
                    success: false,
                    memory: None,
                    error: Some(format!(
                        "Version {} not found for memory {}",
                        target_version, memory_id
                    )),
                });
            }
        };

        let from_version = self.get_current_version_number(memory_id).await?;

        // Create a new version for the current state before overwriting (for recovery)
        self.save_version(
            memory_id,
            current.content.clone(),
            current.metadata.clone(),
            "rollback",
            Some(&format!(
                "Pre-rollback state from version {} to {}",
                from_version, target_version
            )),
        )
        .await?;

        // Update the memory entry with the target version's content and metadata
        let mut updated = current;
        updated.content = target.content.clone();
        updated.metadata = target.metadata.clone();
        updated.updated_at = chrono::Utc::now();

        self.update(updated.clone()).await?;

        // Log the rollback in the audit trail
        self.log_rollback(
            memory_id,
            from_version,
            target_version,
            triggered_by,
            reason,
        )
        .await?;

        Ok(RollbackResult {
            success: true,
            memory: Some(updated),
            error: None,
        })
    }

    /// Log a rollback operation to the audit trail
    async fn log_rollback(
        &self,
        memory_id: Uuid,
        from_version: u32,
        to_version: u32,
        triggered_by: &str,
        reason: Option<&str>,
    ) -> Result<(), SwellError> {
        let audit_id = Uuid::new_v4();
        let now = chrono::Utc::now();
        let now_str = now.to_rfc3339();

        sqlx::query(
            r#"
            INSERT INTO rollback_audit_log (id, memory_id, from_version, to_version, timestamp, triggered_by, reason)
            VALUES (?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(audit_id.to_string())
        .bind(memory_id.to_string())
        .bind(from_version as i64)
        .bind(to_version as i64)
        .bind(&now_str)
        .bind(triggered_by)
        .bind(reason)
        .execute(self.pool.as_ref())
        .await
        .map_err(|e| SwellError::DatabaseError(e.to_string()))?;

        Ok(())
    }

    /// Get rollback audit log for a memory entry
    pub async fn get_rollback_history(
        &self,
        memory_id: Uuid,
    ) -> Result<Vec<RollbackAuditEntry>, SwellError> {
        let rows = sqlx::query(
            r#"
            SELECT id, memory_id, from_version, to_version, timestamp, triggered_by, reason
            FROM rollback_audit_log
            WHERE memory_id = ?
            ORDER BY timestamp DESC
            "#,
        )
        .bind(memory_id.to_string())
        .fetch_all(self.pool.as_ref())
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        let mut entries = Vec::new();
        for row in rows {
            let id_str: String = row.get("id");
            let memory_id_str: String = row.get("memory_id");
            let from_version: i64 = row.get("from_version");
            let to_version: i64 = row.get("to_version");
            let timestamp_str: String = row.get("timestamp");
            let triggered_by: String = row.get("triggered_by");
            let reason: Option<String> = row.get("reason");

            let id = Uuid::parse_str(&id_str)
                .map_err(|e| SwellError::DatabaseError(format!("Invalid audit UUID: {}", e)))?;
            let mid = Uuid::parse_str(&memory_id_str)
                .map_err(|e| SwellError::DatabaseError(format!("Invalid memory UUID: {}", e)))?;

            let timestamp = DateTime::parse_from_rfc3339(&timestamp_str)
                .map_err(|e| SwellError::DatabaseError(format!("Invalid timestamp: {}", e)))?
                .with_timezone(&Utc);

            entries.push(RollbackAuditEntry {
                id,
                memory_id: mid,
                from_version: from_version as u32,
                to_version: to_version as u32,
                timestamp,
                triggered_by,
                reason,
            });
        }

        Ok(entries)
    }

    /// Get the latest version number for a memory entry
    pub async fn get_latest_version(&self, memory_id: Uuid) -> Result<Option<u32>, SwellError> {
        // Use Option<Option<i64>> to properly handle NULL from MAX()
        // - None outer = no rows found
        // - Some(None inner) = NULL (no max, i.e., no versions)
        // - Some(Some(val)) = actual max value
        let row: Option<(Option<i64>,)> =
            sqlx::query_as("SELECT MAX(version) FROM memory_versions WHERE memory_id = ?")
                .bind(memory_id.to_string())
                .fetch_optional(self.pool.as_ref())
                .await
                .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        Ok(row.and_then(|r| r.0).map(|v| v as u32))
    }

    /// Delete all versions older than a given version (housekeeping)
    pub async fn delete_versions_before(
        &self,
        memory_id: Uuid,
        before_version: u32,
    ) -> Result<u64, SwellError> {
        let result = sqlx::query("DELETE FROM memory_versions WHERE memory_id = ? AND version < ?")
            .bind(memory_id.to_string())
            .bind(before_version as i64)
            .execute(self.pool.as_ref())
            .await
            .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        Ok(result.rows_affected())
    }

    /// Check if a memory entry has version history
    pub async fn has_version_history(&self, memory_id: Uuid) -> Result<bool, SwellError> {
        let row: Option<(i64,)> =
            sqlx::query_as("SELECT COUNT(*) FROM memory_versions WHERE memory_id = ?")
                .bind(memory_id.to_string())
                .fetch_optional(self.pool.as_ref())
                .await
                .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        Ok(row.map(|r| r.0 > 0).unwrap_or(false))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use swell_core::MemoryBlockType;

    #[tokio::test]
    async fn test_save_and_get_versions() {
        let store = SqliteMemoryStore::create("sqlite::memory:").await.unwrap();

        // Create and store a memory entry
        let entry = MemoryEntry {
            id: Uuid::new_v4(),
            block_type: MemoryBlockType::Project,
            label: "test-project".to_string(),
            content: "Initial content".to_string(),
            embedding: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            metadata: serde_json::json!({}),
            repository: "test-repo".to_string(),
            language: None,
            task_type: None,
            org: String::new(),
            workspace: String::new(),
            framework: None,
            environment: None,
            session_id: None,

            last_reinforcement: None,
            is_stale: false,
            source_episode_id: None,
            evidence: None,
            provenance_context: None,
        };

        store.store(entry.clone()).await.unwrap();

        // Save a version
        store
            .save_version(
                entry.id,
                entry.content.clone(),
                entry.metadata.clone(),
                "test",
                Some("initial version"),
            )
            .await
            .unwrap();

        // Get versions
        let versions = store.get_versions(entry.id).await.unwrap();
        assert_eq!(versions.len(), 1);
        assert_eq!(versions[0].version, 1);
        assert_eq!(versions[0].content, "Initial content");
        assert_eq!(versions[0].created_by, "test");
    }

    #[tokio::test]
    async fn test_version_increments_on_update() {
        let store = SqliteMemoryStore::create("sqlite::memory:").await.unwrap();

        let entry = MemoryEntry {
            id: Uuid::new_v4(),
            block_type: MemoryBlockType::Project,
            label: "version-test".to_string(),
            content: "v1 content".to_string(),
            embedding: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            metadata: serde_json::json!({}),
            repository: "test-repo".to_string(),
            language: None,
            task_type: None,
            org: String::new(),
            workspace: String::new(),
            framework: None,
            environment: None,
            session_id: None,

            last_reinforcement: None,
            is_stale: false,
            source_episode_id: None,
            evidence: None,
            provenance_context: None,
        };

        store.store(entry.clone()).await.unwrap();

        // Save version 1
        store
            .save_version(
                entry.id,
                "v1 content".to_string(),
                entry.metadata.clone(),
                "test",
                None,
            )
            .await
            .unwrap();

        // Update the entry
        let mut updated = entry.clone();
        updated.content = "v2 content".to_string();
        updated.updated_at = chrono::Utc::now();
        store.update(updated.clone()).await.unwrap();

        // Save version 2
        store
            .save_version(
                entry.id,
                "v2 content".to_string(),
                entry.metadata.clone(),
                "test",
                None,
            )
            .await
            .unwrap();

        // Get all versions
        let versions = store.get_versions(entry.id).await.unwrap();
        assert_eq!(versions.len(), 2);
        assert_eq!(versions[0].version, 2); // Newest first
        assert_eq!(versions[1].version, 1);
    }

    #[tokio::test]
    async fn test_rollback_to_previous_version() {
        let store = SqliteMemoryStore::create("sqlite::memory:").await.unwrap();

        let entry = MemoryEntry {
            id: Uuid::new_v4(),
            block_type: MemoryBlockType::Project,
            label: "rollback-test".to_string(),
            content: "current content".to_string(),
            embedding: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            metadata: serde_json::json!({}),
            repository: "test-repo".to_string(),
            language: None,
            task_type: None,
            org: String::new(),
            workspace: String::new(),
            framework: None,
            environment: None,
            session_id: None,

            last_reinforcement: None,
            is_stale: false,
            source_episode_id: None,
            evidence: None,
            provenance_context: None,
        };

        store.store(entry.clone()).await.unwrap();

        // Save version 1 (current state)
        store
            .save_version(
                entry.id,
                "v1 content".to_string(),
                entry.metadata.clone(),
                "update",
                Some("initial save"),
            )
            .await
            .unwrap();

        // Update entry to v2
        let mut updated = entry.clone();
        updated.content = "v2 content".to_string();
        store.update(updated.clone()).await.unwrap();

        // Save version 2
        store
            .save_version(
                entry.id,
                "v2 content".to_string(),
                entry.metadata.clone(),
                "update",
                Some("update to v2"),
            )
            .await
            .unwrap();

        // Verify current state is v2
        let current = store.get(entry.id).await.unwrap().unwrap();
        assert_eq!(current.content, "v2 content");

        // Rollback to version 1
        let result = store
            .rollback_to_version(entry.id, 1, "test_user", Some("testing rollback"))
            .await
            .unwrap();

        assert!(result.success);
        let rolled_back = result.memory.unwrap();
        assert_eq!(rolled_back.content, "v1 content");

        // Verify rollback is in audit log
        let audit = store.get_rollback_history(entry.id).await.unwrap();
        assert_eq!(audit.len(), 1);
        assert_eq!(audit[0].from_version, 2); // Was version 2 before rollback
        assert_eq!(audit[0].to_version, 1);
        assert_eq!(audit[0].triggered_by, "test_user");
    }

    #[tokio::test]
    async fn test_rollback_nonexistent_version() {
        let store = SqliteMemoryStore::create("sqlite::memory:").await.unwrap();

        let entry = MemoryEntry {
            id: Uuid::new_v4(),
            block_type: MemoryBlockType::Project,
            label: "nonexistent-rollback".to_string(),
            content: "content".to_string(),
            embedding: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            metadata: serde_json::json!({}),
            repository: "test-repo".to_string(),
            language: None,
            task_type: None,
            org: String::new(),
            workspace: String::new(),
            framework: None,
            environment: None,
            session_id: None,

            last_reinforcement: None,
            is_stale: false,
            source_episode_id: None,
            evidence: None,
            provenance_context: None,
        };

        store.store(entry.clone()).await.unwrap();

        // Try to rollback to non-existent version
        let result = store
            .rollback_to_version(entry.id, 99, "test", None)
            .await
            .unwrap();

        assert!(!result.success);
        assert!(result.error.is_some());
        assert!(result.error.unwrap().contains("Version 99 not found"));
    }

    #[tokio::test]
    async fn test_rollback_nonexistent_memory() {
        let store = SqliteMemoryStore::create("sqlite::memory:").await.unwrap();

        let fake_id = Uuid::new_v4();

        let result = store
            .rollback_to_version(fake_id, 1, "test", None)
            .await
            .unwrap();

        assert!(!result.success);
        assert!(result.error.is_some());
    }

    #[tokio::test]
    async fn test_rollback_audit_trail() {
        let store = SqliteMemoryStore::create("sqlite::memory:").await.unwrap();

        let entry = MemoryEntry {
            id: Uuid::new_v4(),
            block_type: MemoryBlockType::Task,
            label: "audit-test".to_string(),
            content: "content".to_string(),
            embedding: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            metadata: serde_json::json!({}),
            repository: "test-repo".to_string(),
            language: None,
            task_type: None,
            org: String::new(),
            workspace: String::new(),
            framework: None,
            environment: None,
            session_id: None,

            last_reinforcement: None,
            is_stale: false,
            source_episode_id: None,
            evidence: None,
            provenance_context: None,
        };

        store.store(entry.clone()).await.unwrap();

        // Save initial version
        store
            .save_version(
                entry.id,
                "v1".to_string(),
                entry.metadata.clone(),
                "init",
                None,
            )
            .await
            .unwrap();

        // Perform rollback
        store
            .rollback_to_version(entry.id, 1, "admin", Some("fixing mistake"))
            .await
            .unwrap();

        // Get audit trail
        let audit = store.get_rollback_history(entry.id).await.unwrap();
        assert_eq!(audit.len(), 1);
        assert_eq!(audit[0].triggered_by, "admin");
        assert_eq!(audit[0].reason, Some("fixing mistake".to_string()));
    }

    #[tokio::test]
    async fn test_has_version_history() {
        let store = SqliteMemoryStore::create("sqlite::memory:").await.unwrap();

        let entry = MemoryEntry {
            id: Uuid::new_v4(),
            block_type: MemoryBlockType::Project,
            label: "history-check".to_string(),
            content: "content".to_string(),
            embedding: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            metadata: serde_json::json!({}),
            repository: "test-repo".to_string(),
            language: None,
            task_type: None,
            org: String::new(),
            workspace: String::new(),
            framework: None,
            environment: None,
            session_id: None,

            last_reinforcement: None,
            is_stale: false,
            source_episode_id: None,
            evidence: None,
            provenance_context: None,
        };

        store.store(entry.clone()).await.unwrap();

        // Initially no version history
        let has_history = store.has_version_history(entry.id).await.unwrap();
        assert!(!has_history);

        // After saving a version
        store
            .save_version(
                entry.id,
                "v1".to_string(),
                entry.metadata.clone(),
                "test",
                None,
            )
            .await
            .unwrap();

        let has_history = store.has_version_history(entry.id).await.unwrap();
        assert!(has_history);
    }

    #[tokio::test]
    async fn test_delete_old_versions() {
        let store = SqliteMemoryStore::create("sqlite::memory:").await.unwrap();

        let entry = MemoryEntry {
            id: Uuid::new_v4(),
            block_type: MemoryBlockType::Project,
            label: "cleanup-test".to_string(),
            content: "content".to_string(),
            embedding: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            metadata: serde_json::json!({}),
            repository: "test-repo".to_string(),
            language: None,
            task_type: None,
            org: String::new(),
            workspace: String::new(),
            framework: None,
            environment: None,
            session_id: None,

            last_reinforcement: None,
            is_stale: false,
            source_episode_id: None,
            evidence: None,
            provenance_context: None,
        };

        store.store(entry.clone()).await.unwrap();

        // Save multiple versions
        for i in 1..=5 {
            store
                .save_version(
                    entry.id,
                    format!("v{} content", i),
                    entry.metadata.clone(),
                    "test",
                    None,
                )
                .await
                .unwrap();

            // Update entry to trigger new version
            let mut updated = entry.clone();
            updated.content = format!("v{} content", i + 1);
            store.update(updated.clone()).await.unwrap();
        }

        // Should have 5 versions
        let versions = store.get_versions(entry.id).await.unwrap();
        assert_eq!(versions.len(), 5);

        // Delete versions before version 3
        let deleted = store.delete_versions_before(entry.id, 3).await.unwrap();
        assert_eq!(deleted, 2);

        // Should have 3 versions remaining (3, 4, 5)
        let versions = store.get_versions(entry.id).await.unwrap();
        assert_eq!(versions.len(), 3);
        assert_eq!(versions[0].version, 5);
        assert_eq!(versions[1].version, 4);
        assert_eq!(versions[2].version, 3);
    }

    #[tokio::test]
    async fn test_get_latest_version() {
        let store = SqliteMemoryStore::create("sqlite::memory:").await.unwrap();

        let entry = MemoryEntry {
            id: Uuid::new_v4(),
            block_type: MemoryBlockType::Project,
            label: "latest-version-test".to_string(),
            content: "content".to_string(),
            embedding: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            metadata: serde_json::json!({}),
            repository: "test-repo".to_string(),
            language: None,
            task_type: None,
            org: String::new(),
            workspace: String::new(),
            framework: None,
            environment: None,
            session_id: None,

            last_reinforcement: None,
            is_stale: false,
            source_episode_id: None,
            evidence: None,
            provenance_context: None,
        };

        store.store(entry.clone()).await.unwrap();

        // No versions yet
        let latest = store.get_latest_version(entry.id).await.unwrap();
        assert!(latest.is_none());

        // Save some versions
        for i in 1..=3 {
            store
                .save_version(
                    entry.id,
                    format!("v{}", i),
                    entry.metadata.clone(),
                    "test",
                    None,
                )
                .await
                .unwrap();
        }

        let latest = store.get_latest_version(entry.id).await.unwrap();
        assert_eq!(latest, Some(3));
    }

    #[tokio::test]
    async fn test_get_specific_version() {
        let store = SqliteMemoryStore::create("sqlite::memory:").await.unwrap();

        let entry = MemoryEntry {
            id: Uuid::new_v4(),
            block_type: MemoryBlockType::Project,
            label: "specific-version-test".to_string(),
            content: "content".to_string(),
            embedding: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            metadata: serde_json::json!({"key": "original"}),
            repository: "test-repo".to_string(),
            language: None,
            task_type: None,
            org: String::new(),
            workspace: String::new(),
            framework: None,
            environment: None,
            session_id: None,

            last_reinforcement: None,
            is_stale: false,
            source_episode_id: None,
            evidence: None,
            provenance_context: None,
        };

        store.store(entry.clone()).await.unwrap();

        // Save version 1
        store
            .save_version(
                entry.id,
                "v1 content".to_string(),
                serde_json::json!({"key": "v1"}),
                "test",
                None,
            )
            .await
            .unwrap();

        // Update and save version 2
        store
            .save_version(
                entry.id,
                "v2 content".to_string(),
                serde_json::json!({"key": "v2"}),
                "test",
                None,
            )
            .await
            .unwrap();

        // Get specific version
        let v1 = store.get_version(entry.id, 1).await.unwrap();
        assert!(v1.is_some());
        assert_eq!(v1.unwrap().content, "v1 content");

        let v2 = store.get_version(entry.id, 2).await.unwrap();
        assert!(v2.is_some());
        assert_eq!(v2.unwrap().content, "v2 content");

        // Non-existent version
        let v99 = store.get_version(entry.id, 99).await.unwrap();
        assert!(v99.is_none());
    }
}
