// Memory Blocks Module
//
// Provides Project, User, and Task memory blocks with auto-loading
// and context assembly for agents.

use async_trait::async_trait;
use std::sync::Arc;
use uuid::Uuid;

pub use swell_core::{
    AgentContext, MemoryBlock, MemoryBlockType, MemoryEntry, MemoryStore, SwellError,
};

/// Memory block labels for well-known blocks
pub mod labels {
    pub const PROJECT_ARCHITECTURE: &str = "project:architecture";
    pub const PROJECT_CONVENTIONS: &str = "project:conventions";
    pub const USER_PREFERENCES: &str = "user:preferences";
    pub const TASK_CONTEXT: &str = "task:context";
}

/// A loaded set of memory blocks for an agent session
#[derive(Debug, Clone, Default)]
pub struct MemoryBlocks {
    pub project: Option<MemoryEntry>,
    pub user: Option<MemoryEntry>,
    pub task: Option<MemoryEntry>,
}

/// Auto-loader for memory blocks per agent
#[async_trait]
pub trait MemoryBlockLoader: Send + Sync {
    /// Load all memory blocks for a given repository/session context
    async fn load_blocks(
        &self,
        store: &dyn MemoryStore,
        repository_scope: &str,
        user_id: Option<&str>,
        task_id: Option<Uuid>,
    ) -> Result<MemoryBlocks, SwellError>;
}

/// Default implementation of MemoryBlockLoader
#[derive(Default)]
pub struct DefaultBlockLoader;

impl DefaultBlockLoader {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl MemoryBlockLoader for DefaultBlockLoader {
    async fn load_blocks(
        &self,
        store: &dyn MemoryStore,
        repository_scope: &str,
        user_id: Option<&str>,
        task_id: Option<Uuid>,
    ) -> Result<MemoryBlocks, SwellError> {
        let mut blocks = MemoryBlocks::default();

        // Load Project block (architecture, conventions)
        // Find the project block that matches the repository scope
        let project_entries = store.get_by_type(MemoryBlockType::Project, repository_scope.to_string()).await?;

        // Find the project block that matches the repository scope
        for entry in &project_entries {
            if let Some(repo_value) = entry.metadata.get("repository") {
                if let Some(repo_label) = repo_value.as_str() {
                    if repo_label == repository_scope {
                        blocks.project = Some(entry.clone());
                        break;
                    }
                }
            }
        }
        // Note: We don't fallback to first entry if no exact match - requires exact repository match

        // Load User block (preferences)
        if let Some(uid) = user_id {
            let user_entries = store.get_by_label(format!("user:{}", uid), repository_scope.to_string()).await?;
            if !user_entries.is_empty() {
                blocks.user = Some(user_entries.into_iter().next().unwrap());
            }
        }

        // Load Task block (context)
        if let Some(tid) = task_id {
            let task_entries = store.get_by_label(format!("task:{}", tid), repository_scope.to_string()).await?;
            if !task_entries.is_empty() {
                blocks.task = Some(task_entries.into_iter().next().unwrap());
            }
        }

        Ok(blocks)
    }
}

/// Assembles context from memory blocks for agent execution
pub struct ContextAssembler {
    loader: Arc<dyn MemoryBlockLoader>,
}

impl ContextAssembler {
    pub fn new(loader: Arc<dyn MemoryBlockLoader>) -> Self {
        Self { loader }
    }

    /// Create a new ContextAssembler with the default block loader
    pub fn with_default_loader() -> Self {
        Self {
            loader: Arc::new(DefaultBlockLoader::new()),
        }
    }

    /// Assemble AgentContext by loading memory blocks for the given parameters
    pub async fn assemble_context(
        &self,
        store: &dyn MemoryStore,
        task: swell_core::Task,
        session_id: Uuid,
        workspace_path: Option<String>,
        repository_scope: &str,
        user_id: Option<&str>,
    ) -> Result<AgentContext, SwellError> {
        let blocks = self
            .loader
            .load_blocks(store, repository_scope, user_id, Some(task.id))
            .await?;

        let memory_blocks = self.blocks_to_memory_blocks(&blocks);

        Ok(AgentContext {
            task,
            memory_blocks,
            session_id,
            workspace_path,
        })
    }

    /// Convert MemoryBlocks to Vec<MemoryBlock> for AgentContext
    fn blocks_to_memory_blocks(&self, blocks: &MemoryBlocks) -> Vec<MemoryBlock> {
        let mut result = Vec::new();

        if let Some(ref project) = blocks.project {
            if let Some(mb) = self.entry_to_memory_block(project) {
                result.push(mb);
            }
        }

        if let Some(ref user) = blocks.user {
            if let Some(mb) = self.entry_to_memory_block(user) {
                result.push(mb);
            }
        }

        if let Some(ref task) = blocks.task {
            if let Some(mb) = self.entry_to_memory_block(task) {
                result.push(mb);
            }
        }

        result
    }

    /// Convert a MemoryEntry to MemoryBlock
    fn entry_to_memory_block(&self, entry: &MemoryEntry) -> Option<MemoryBlock> {
        Some(MemoryBlock {
            id: entry.id,
            label: entry.label.clone(),
            description: entry
                .metadata
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            content: entry.content.clone(),
            block_type: entry.block_type,
            created_at: entry.created_at,
            updated_at: entry.updated_at,
        })
    }
}

impl Default for ContextAssembler {
    fn default() -> Self {
        Self::with_default_loader()
    }
}

/// Helper to create a Project memory entry
pub fn create_project_block(
    repository: &str,
    architecture: &str,
    conventions: &str,
) -> MemoryEntry {
    let now = chrono::Utc::now();
    MemoryEntry {
        id: Uuid::new_v4(),
        block_type: MemoryBlockType::Project,
        label: labels::PROJECT_ARCHITECTURE.to_string(),
        content: format!(
            "# Project Architecture\n{}\n\n# Conventions\n{}",
            architecture, conventions
        ),
        embedding: None,
        created_at: now,
        updated_at: now,
        metadata: serde_json::json!({
            "repository": repository,
            "description": "Project architecture and conventions"
        }),
        repository: repository.to_string(),
        language: None,
        task_type: None,
        last_reinforcement: Some(now),
        is_stale: false,
        source_episode_id: None,
        evidence: None,
        provenance_context: None,
    }
}

/// Helper to create a User memory entry
pub fn create_user_block(user_id: &str, preferences: &str) -> MemoryEntry {
    create_user_block_with_repo(user_id, preferences, "")
}

/// Helper to create a User memory entry with repository scope
pub fn create_user_block_with_repo(user_id: &str, preferences: &str, repository: &str) -> MemoryEntry {
    let now = chrono::Utc::now();
    MemoryEntry {
        id: Uuid::new_v4(),
        block_type: MemoryBlockType::User,
        label: format!("user:{}", user_id),
        content: format!("# User Preferences\n{}", preferences),
        embedding: None,
        created_at: now,
        updated_at: now,
        metadata: serde_json::json!({
            "user_id": user_id,
            "description": "User preferences and settings"
        }),
        repository: repository.to_string(),
        language: None,
        task_type: None,
        last_reinforcement: Some(now),
        is_stale: false,
        source_episode_id: None,
        evidence: None,
        provenance_context: None,
    }
}

/// Helper to create a Task memory entry
pub fn create_task_block(task_id: Uuid, context: &str) -> MemoryEntry {
    create_task_block_with_repo(task_id, context, "")
}

/// Helper to create a Task memory entry with repository scope
pub fn create_task_block_with_repo(task_id: Uuid, context: &str, repository: &str) -> MemoryEntry {
    let now = chrono::Utc::now();
    MemoryEntry {
        id: Uuid::new_v4(),
        block_type: MemoryBlockType::Task,
        label: format!("task:{}", task_id),
        content: format!("# Task Context\n{}", context),
        embedding: None,
        created_at: now,
        updated_at: now,
        metadata: serde_json::json!({
            "task_id": task_id.to_string(),
            "description": "Task-specific context and background"
        }),
        repository: repository.to_string(),
        language: None,
        task_type: None,
        last_reinforcement: Some(now),
        is_stale: false,
        source_episode_id: None,
        evidence: None,
        provenance_context: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_create_project_block() {
        let block = create_project_block(
            "my-repo",
            "Microservices architecture with REST APIs",
            "Conventional commits, Rust formatting",
        );

        assert_eq!(block.block_type, MemoryBlockType::Project);
        assert_eq!(block.label, labels::PROJECT_ARCHITECTURE);
        assert!(block.content.contains("Microservices architecture"));
        assert!(block.content.contains("Conventional commits"));
        assert_eq!(
            block.metadata.get("repository").and_then(|v| v.as_str()),
            Some("my-repo")
        );
    }

    #[tokio::test]
    async fn test_create_user_block() {
        let block = create_user_block("user123", "Prefers verbose logging");

        assert_eq!(block.block_type, MemoryBlockType::User);
        assert_eq!(block.label, "user:user123");
        assert!(block.content.contains("verbose logging"));
    }

    #[tokio::test]
    async fn test_create_task_block() {
        let task_id = Uuid::new_v4();
        let block = create_task_block(task_id, "Fix authentication bug");

        assert_eq!(block.block_type, MemoryBlockType::Task);
        assert_eq!(block.label, format!("task:{}", task_id));
        assert!(block.content.contains("authentication bug"));
    }

    #[tokio::test]
    async fn test_memory_blocks_default() {
        let blocks = MemoryBlocks::default();
        assert!(blocks.project.is_none());
        assert!(blocks.user.is_none());
        assert!(blocks.task.is_none());
    }

    #[tokio::test]
    async fn test_context_assembler_with_empty_store() {
        use crate::SqliteMemoryStore;

        let store: SqliteMemoryStore = SqliteMemoryStore::create("sqlite::memory:").await.unwrap();
        let assembler = ContextAssembler::with_default_loader();

        let task = swell_core::Task::new("Test task".to_string());
        let context = assembler
            .assemble_context(
                &store,
                task.clone(),
                Uuid::new_v4(),
                Some("/tmp".to_string()),
                "test-repo",
                Some("testuser"),
            )
            .await
            .unwrap();

        assert_eq!(context.task.id, task.id);
        assert!(context.memory_blocks.is_empty()); // No blocks stored yet
    }

    #[tokio::test]
    async fn test_load_blocks_with_stored_entries() {
        use crate::SqliteMemoryStore;

        let store: SqliteMemoryStore = SqliteMemoryStore::create("sqlite::memory:").await.unwrap();

        // Store a project block
        let project = create_project_block("test-repo", "Simple architecture", "Rust conventions");
        store.store(project.clone()).await.unwrap();

        // Store a user block (user blocks are stored with the repository scope)
        let user = create_user_block_with_repo("testuser", "Test preferences", "test-repo");
        store.store(user.clone()).await.unwrap();

        // Store a task block
        let task_id = Uuid::new_v4();
        let task_block = create_task_block_with_repo(task_id, "Test task context", "test-repo");
        store.store(task_block.clone()).await.unwrap();

        // Load blocks
        let loader = DefaultBlockLoader::new();
        let blocks = loader
            .load_blocks(&store, "test-repo", Some("testuser"), Some(task_id))
            .await
            .unwrap();

        assert!(blocks.project.is_some());
        assert!(blocks.user.is_some());
        assert!(blocks.task.is_some());

        let loaded_project = blocks.project.unwrap();
        assert_eq!(loaded_project.id, project.id);

        let loaded_user = blocks.user.unwrap();
        assert_eq!(loaded_user.id, user.id);

        let loaded_task = blocks.task.unwrap();
        assert_eq!(loaded_task.id, task_block.id);
    }

    #[tokio::test]
    async fn test_assemble_context_with_all_blocks() {
        use crate::SqliteMemoryStore;

        // Use a temp file database for isolation
        let temp_dir = std::env::temp_dir();
        let db_path = temp_dir.join(format!("test_blocks_{}.db", Uuid::new_v4()));
        let db_url = format!("sqlite:{}?mode=rwc", db_path.display());

        let store: SqliteMemoryStore = SqliteMemoryStore::create(&db_url).await.unwrap();

        // Create the task first to get its actual ID
        let task = swell_core::Task::new("Test".to_string());
        let task_id = task.id;

        // Store all block types - use consistent repository "my-repo" and user_id "testuser"
        let project = create_project_block("my-repo", "arch", "conv");
        store.store(project).await.unwrap();

        let user = create_user_block_with_repo("testuser", "prefs", "my-repo");
        store.store(user).await.unwrap();

        let task_block = create_task_block_with_repo(task_id, "context", "my-repo");
        store.store(task_block).await.unwrap();

        let assembler = ContextAssembler::with_default_loader();
        let session_id = Uuid::new_v4();

        let context = assembler
            .assemble_context(
                &store,
                task,
                session_id,
                Some("/workspace".to_string()),
                "my-repo",
                Some("testuser"),
            )
            .await
            .unwrap();

        // Should have 3 memory blocks assembled
        assert_eq!(
            context.memory_blocks.len(),
            3,
            "Expected 3 blocks, got {}",
            context.memory_blocks.len()
        );
        assert_eq!(context.session_id, session_id);
        assert_eq!(context.workspace_path, Some("/workspace".to_string()));

        // Cleanup
        drop(store);
        let _ = std::fs::remove_file(db_path);
    }
}
