// repo_scoping.rs - Repository-level memory scoping tests
//
// Tests that memory entries are properly tagged with repository context
// and that queries are correctly filtered by repository scope to prevent
// cross-repository context leakage.

use swell_core::{MemoryBlockType, MemoryEntry, MemoryQuery, MemoryStore};
use swell_memory::SqliteMemoryStore;
use uuid::Uuid;

/// Helper to create a test memory entry with a specific repository scope
fn create_test_entry(label: &str, content: &str, repository: &str) -> MemoryEntry {
    MemoryEntry {
        id: Uuid::new_v4(),
        block_type: MemoryBlockType::Project,
        label: label.to_string(),
        content: content.to_string(),
        embedding: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        metadata: serde_json::json!({}),
        repository: repository.to_string(),
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
    }
}

/// Test that memory entries are tagged with repository context on store
#[tokio::test]
async fn test_memory_entries_tagged_with_repo_context() {
    let store = SqliteMemoryStore::create("sqlite::memory:").await.unwrap();

    // Store entries for two different repositories
    let entry_repo_a = create_test_entry("project-a-config", "Configuration for repo A", "repo-a");
    let entry_repo_b = create_test_entry("project-b-config", "Configuration for repo B", "repo-b");

    store.store(entry_repo_a.clone()).await.unwrap();
    store.store(entry_repo_b.clone()).await.unwrap();

    // Verify entries are stored with correct repository context
    let retrieved_a = store.get(entry_repo_a.id).await.unwrap();
    assert!(retrieved_a.is_some());
    assert_eq!(retrieved_a.unwrap().repository, "repo-a");

    let retrieved_b = store.get(entry_repo_b.id).await.unwrap();
    assert!(retrieved_b.is_some());
    assert_eq!(retrieved_b.unwrap().repository, "repo-b");
}

/// Test that querying with repo-a scope returns only repo-a memories
#[tokio::test]
async fn test_query_returns_only_same_repo_results() {
    let store = SqliteMemoryStore::create("sqlite::memory:").await.unwrap();

    // Store entries in different repositories
    let entry_a1 = create_test_entry(
        "repo-a-entry-1",
        "This is a memory entry from repository A",
        "repo-a",
    );
    let entry_a2 = create_test_entry(
        "repo-a-entry-2",
        "Another memory entry from repository A",
        "repo-a",
    );
    let entry_b = create_test_entry(
        "repo-b-entry",
        "This is a memory entry from repository B",
        "repo-b",
    );

    store.store(entry_a1.clone()).await.unwrap();
    store.store(entry_a2.clone()).await.unwrap();
    store.store(entry_b.clone()).await.unwrap();

    // Query with repo-a scope - should only return repo-a entries
    let results = store
        .search(MemoryQuery {
            query_text: Some("memory entry".to_string()),
            block_types: None,
            labels: None,
            limit: 10,
            offset: 0,
            repository: "repo-a".to_string(),
            language: None,
            task_type: None,
            org: String::new(),
            workspace: String::new(),
            framework: None,
            environment: None,
            session_id: None,
            cross_scope_override: false,

            source_episode_id: None,
        })
        .await
        .unwrap();

    assert_eq!(results.len(), 2);
    assert!(results.iter().all(|r| r.entry.repository == "repo-a"));

    // Verify we got the correct entries
    let result_ids: Vec<Uuid> = results.iter().map(|r| r.entry.id).collect();
    assert!(result_ids.contains(&entry_a1.id));
    assert!(result_ids.contains(&entry_a2.id));
    assert!(!result_ids.contains(&entry_b.id));
}

/// Test that querying with repo-b scope returns empty for repo-a memories
#[tokio::test]
async fn test_cross_repo_query_returns_empty() {
    let store = SqliteMemoryStore::create("sqlite::memory:").await.unwrap();

    // Store entry only in repo-a
    let entry_a = create_test_entry(
        "secret-from-repo-a",
        "Sensitive information only for repo A",
        "repo-a",
    );

    store.store(entry_a.clone()).await.unwrap();

    // Query with repo-b scope - should return empty (cross-repo isolation)
    let results = store
        .search(MemoryQuery {
            query_text: Some("Sensitive information".to_string()),
            block_types: None,
            labels: None,
            limit: 10,
            offset: 0,
            repository: "repo-b".to_string(),
            language: None,
            task_type: None,
            org: String::new(),
            workspace: String::new(),
            framework: None,
            environment: None,
            session_id: None,
            cross_scope_override: false,

            source_episode_id: None,
        })
        .await
        .unwrap();

    assert_eq!(
        results.len(),
        0,
        "repo-a memory should not leak to repo-b query"
    );
}

/// Test full cross-repo isolation scenario
#[tokio::test]
async fn test_full_cross_repo_isolation() {
    let store = SqliteMemoryStore::create("sqlite::memory:").await.unwrap();

    // Create multiple entries across different repositories
    let entries = vec![
        ("proj-a-1", "Repo A - Project 1", "repo-a"),
        ("proj-a-2", "Repo A - Project 2", "repo-a"),
        ("proj-b-1", "Repo B - Project 1", "repo-b"),
        ("proj-b-2", "Repo B - Project 2", "repo-b"),
        ("proj-c-1", "Repo C - Project 1", "repo-c"),
    ];

    for (label, content, repo) in &entries {
        let entry = create_test_entry(label, content, repo);
        store.store(entry).await.unwrap();
    }

    // Query each repository and verify isolation
    for (_, _, repo) in &entries {
        let results = store
            .search(MemoryQuery {
                query_text: None, // Get all entries for this repo
                block_types: None,
                labels: None,
                limit: 10,
                offset: 0,
                repository: repo.to_string(),
                language: None,
                task_type: None,
                org: String::new(),
                workspace: String::new(),
                framework: None,
                environment: None,
                session_id: None,
                cross_scope_override: false,

                source_episode_id: None,
            })
            .await
            .unwrap();

        // Each repo should have exactly 2 entries (except repo-c which has 1)
        let expected_count = if *repo == "repo-c" { 1 } else { 2 };
        assert_eq!(
            results.len(),
            expected_count,
            "Repo {} should have {} entries",
            repo,
            expected_count
        );

        // All results should be from the correct repo
        for result in &results {
            assert_eq!(
                result.entry.repository, *repo,
                "Entry should belong to {} but got {}",
                repo, result.entry.repository
            );
        }
    }
}

/// Test get_by_type respects repository scope
#[tokio::test]
async fn test_get_by_type_respects_repo_scope() {
    let store = SqliteMemoryStore::create("sqlite::memory:").await.unwrap();

    // Create entries of the same type in different repos
    let entry_a = create_test_entry("task-a", "Task A content", "repo-a");
    let mut entry_a = entry_a;
    entry_a.block_type = MemoryBlockType::Task;

    let entry_b = create_test_entry("task-b", "Task B content", "repo-b");
    let mut entry_b = entry_b;
    entry_b.block_type = MemoryBlockType::Task;

    store.store(entry_a.clone()).await.unwrap();
    store.store(entry_b.clone()).await.unwrap();

    // Get by type for repo-a only
    let results_a = store
        .get_by_type(MemoryBlockType::Task, "repo-a".to_string())
        .await
        .unwrap();

    assert_eq!(results_a.len(), 1);
    assert_eq!(results_a[0].repository, "repo-a");
    assert_eq!(results_a[0].id, entry_a.id);

    // Get by type for repo-b only
    let results_b = store
        .get_by_type(MemoryBlockType::Task, "repo-b".to_string())
        .await
        .unwrap();

    assert_eq!(results_b.len(), 1);
    assert_eq!(results_b[0].repository, "repo-b");
    assert_eq!(results_b[0].id, entry_b.id);
}

/// Test get_by_label respects repository scope
#[tokio::test]
async fn test_get_by_label_respects_repo_scope() {
    let store = SqliteMemoryStore::create("sqlite::memory:").await.unwrap();

    // Create entries with the same label in different repos
    let entry_a = create_test_entry("shared-label", "Content in repo A", "repo-a");
    let entry_b = create_test_entry("shared-label", "Content in repo B", "repo-b");

    store.store(entry_a.clone()).await.unwrap();
    store.store(entry_b.clone()).await.unwrap();

    // Get by label for repo-a only
    let results_a = store
        .get_by_label("shared-label".to_string(), "repo-a".to_string())
        .await
        .unwrap();

    assert_eq!(results_a.len(), 1);
    assert_eq!(results_a[0].repository, "repo-a");

    // Get by label for repo-b only
    let results_b = store
        .get_by_label("shared-label".to_string(), "repo-b".to_string())
        .await
        .unwrap();

    assert_eq!(results_b.len(), 1);
    assert_eq!(results_b[0].repository, "repo-b");
}

/// Test that repository scoping prevents cross-repo pattern/skills leakage
/// This simulates the "patterns" and "skills" memory types mentioned in VAL-MEM-007
#[tokio::test]
async fn test_skills_patterns_cross_repo_isolation() {
    let store = SqliteMemoryStore::create("sqlite::memory:").await.unwrap();

    // Create skill entries in different repositories
    let mut skill_a = create_test_entry(
        "skill:rust-error-handling",
        "Use thiserror for error handling in Rust",
        "repo-a",
    );
    skill_a.block_type = MemoryBlockType::Skill;

    let mut skill_b = create_test_entry(
        "skill:python-error-handling",
        "Use exceptions for error handling in Python",
        "repo-b",
    );
    skill_b.block_type = MemoryBlockType::Skill;

    // Create pattern entries in different repositories
    let mut pattern_a = create_test_entry(
        "pattern:commit-msg-format",
        "Follow conventional commits format",
        "repo-a",
    );
    pattern_a.block_type = MemoryBlockType::Convention;

    let mut pattern_b = create_test_entry(
        "pattern:python-style-guide",
        "Follow PEP 8 style guide",
        "repo-b",
    );
    pattern_b.block_type = MemoryBlockType::Convention;

    store.store(skill_a.clone()).await.unwrap();
    store.store(skill_b.clone()).await.unwrap();
    store.store(pattern_a.clone()).await.unwrap();
    store.store(pattern_b.clone()).await.unwrap();

    // Query skills for repo-a - should only get the Rust skill
    let skills_a = store
        .get_by_type(MemoryBlockType::Skill, "repo-a".to_string())
        .await
        .unwrap();

    assert_eq!(skills_a.len(), 1);
    assert_eq!(skills_a[0].id, skill_a.id);
    assert!(skills_a[0].content.contains("Rust"));

    // Query patterns for repo-a - should only get the commit-msg pattern
    let patterns_a = store
        .get_by_type(MemoryBlockType::Convention, "repo-a".to_string())
        .await
        .unwrap();

    assert_eq!(patterns_a.len(), 1);
    assert_eq!(patterns_a[0].id, pattern_a.id);
    assert!(patterns_a[0].content.contains("conventional commits"));

    // Verify repo-b memories are completely isolated
    let skills_b = store
        .get_by_type(MemoryBlockType::Skill, "repo-b".to_string())
        .await
        .unwrap();

    assert_eq!(skills_b.len(), 1);
    assert!(skills_b[0].content.contains("Python"));

    let patterns_b = store
        .get_by_type(MemoryBlockType::Convention, "repo-b".to_string())
        .await
        .unwrap();

    assert_eq!(patterns_b.len(), 1);
    assert!(patterns_b[0].content.contains("PEP 8"));
}

/// Test that text search within a repository doesn't leak to other repositories
#[tokio::test]
async fn test_text_search_respects_repo_scope() {
    let store = SqliteMemoryStore::create("sqlite::memory:").await.unwrap();

    // Create entries with overlapping content keywords but different repos
    let entry_a = create_test_entry(
        "doc-a",
        "Important secret: the password is 'secret123'",
        "repo-a",
    );
    let entry_b = create_test_entry(
        "doc-b",
        "Important secret: the password is 'different456'",
        "repo-b",
    );

    store.store(entry_a.clone()).await.unwrap();
    store.store(entry_b.clone()).await.unwrap();

    // Search for "password" in repo-a - should only find entry_a
    let results_a = store
        .search(MemoryQuery {
            query_text: Some("password".to_string()),
            block_types: None,
            labels: None,
            limit: 10,
            offset: 0,
            repository: "repo-a".to_string(),
            language: None,
            task_type: None,
            org: String::new(),
            workspace: String::new(),
            framework: None,
            environment: None,
            session_id: None,
            cross_scope_override: false,

            source_episode_id: None,
        })
        .await
        .unwrap();

    assert_eq!(results_a.len(), 1);
    assert_eq!(results_a[0].entry.id, entry_a.id);
    assert!(results_a[0].entry.content.contains("secret123"));
    assert!(!results_a[0].entry.content.contains("different456"));

    // Search for "password" in repo-b - should only find entry_b
    let results_b = store
        .search(MemoryQuery {
            query_text: Some("password".to_string()),
            block_types: None,
            labels: None,
            limit: 10,
            offset: 0,
            repository: "repo-b".to_string(),
            language: None,
            task_type: None,
            org: String::new(),
            workspace: String::new(),
            framework: None,
            environment: None,
            session_id: None,
            cross_scope_override: false,

            source_episode_id: None,
        })
        .await
        .unwrap();

    assert_eq!(results_b.len(), 1);
    assert_eq!(results_b[0].entry.id, entry_b.id);
    assert!(results_b[0].entry.content.contains("different456"));
    assert!(!results_b[0].entry.content.contains("secret123"));
}

/// Test that repository scope works with empty/non-empty query_text
#[tokio::test]
async fn test_repo_scope_with_and_without_query_text() {
    let store = SqliteMemoryStore::create("sqlite::memory:").await.unwrap();

    // Store entries
    let entry_a = create_test_entry("label-a", "Content A", "repo-a");
    let entry_b = create_test_entry("label-b", "Content B", "repo-b");

    store.store(entry_a.clone()).await.unwrap();
    store.store(entry_b.clone()).await.unwrap();

    // Without query_text - get all entries in repo
    let all_results = store
        .search(MemoryQuery {
            query_text: None,
            block_types: None,
            labels: None,
            limit: 10,
            offset: 0,
            repository: "repo-a".to_string(),
            language: None,
            task_type: None,
            org: String::new(),
            workspace: String::new(),
            framework: None,
            environment: None,
            session_id: None,
            cross_scope_override: false,

            source_episode_id: None,
        })
        .await
        .unwrap();

    assert_eq!(all_results.len(), 1);
    assert_eq!(all_results[0].entry.id, entry_a.id);

    // With query_text - filter within repo
    let filtered_results = store
        .search(MemoryQuery {
            query_text: Some("Content A".to_string()),
            block_types: None,
            labels: None,
            limit: 10,
            offset: 0,
            repository: "repo-a".to_string(),
            language: None,
            task_type: None,
            org: String::new(),
            workspace: String::new(),
            framework: None,
            environment: None,
            session_id: None,
            cross_scope_override: false,

            source_episode_id: None,
        })
        .await
        .unwrap();

    assert_eq!(filtered_results.len(), 1);
    assert_eq!(filtered_results[0].entry.id, entry_a.id);

    // Query text that matches nothing in repo-a but exists in repo-b
    let no_match_results = store
        .search(MemoryQuery {
            query_text: Some("Content B".to_string()),
            block_types: None,
            labels: None,
            limit: 10,
            offset: 0,
            repository: "repo-a".to_string(),
            language: None,
            task_type: None,
            org: String::new(),
            workspace: String::new(),
            framework: None,
            environment: None,
            session_id: None,
            cross_scope_override: false,

            source_episode_id: None,
        })
        .await
        .unwrap();

    assert_eq!(no_match_results.len(), 0);
}
