//! Skill catalog implementation with progressive disclosure

use std::collections::HashMap;
use tokio::sync::RwLock;

use crate::error::SkillsError;
use crate::loader::SkillsLoader;
use crate::types::{Skill, SkillCatalogEntry};

/// A catalog of skills with Tier 1 (name+description) data
///
/// This is the lightweight catalog loaded at startup for skill discovery
/// and matching. Full skill content is loaded on demand (Tier 2).
#[derive(Debug, Clone, Default)]
pub struct SkillCatalog {
    /// Map of skill name to catalog entry
    entries: HashMap<String, SkillCatalogEntry>,
    /// The skills loader for loading full skill content on demand
    #[allow(dead_code)]
    loader: Option<SkillsLoader>,
}

impl SkillCatalog {
    /// Create a new empty catalog
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            loader: None,
        }
    }

    /// Create a catalog from a list of entries
    pub fn from_entries(entries: Vec<SkillCatalogEntry>) -> Self {
        let mut catalog = Self::new();
        for entry in entries {
            catalog.add_entry(entry);
        }
        catalog
    }

    /// Add an entry to the catalog
    pub fn add_entry(&mut self, entry: SkillCatalogEntry) {
        self.entries.insert(entry.name.clone(), entry);
    }

    /// Get a catalog entry by name
    pub fn get(&self, name: &str) -> Option<&SkillCatalogEntry> {
        self.entries.get(name)
    }

    /// Get all catalog entries
    pub fn entries(&self) -> Vec<&SkillCatalogEntry> {
        self.entries.values().collect()
    }

    /// Get all skill names
    pub fn skill_names(&self) -> Vec<&str> {
        self.entries.keys().map(|s| s.as_str()).collect()
    }

    /// Number of skills in the catalog
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the catalog is empty
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Find skills matching the given keywords
    ///
    /// This is used for model-driven activation - the LLM can read the catalog
    /// and match skills to tasks using description keywords.
    pub fn find_matching(&self, keywords: &[String]) -> Vec<&SkillCatalogEntry> {
        self.entries
            .values()
            .filter(|entry| entry.matches_keywords(keywords))
            .collect()
    }

    /// Find skills whose description contains any of the given keywords
    pub fn search(&self, query: &str) -> Vec<&SkillCatalogEntry> {
        let query_lower = query.to_lowercase();
        self.entries
            .values()
            .filter(|entry| {
                entry.description.to_lowercase().contains(&query_lower)
                    || entry.name.to_lowercase().contains(&query_lower)
                    || entry.keywords.iter().any(|k| k.contains(&query_lower))
            })
            .collect()
    }
}

/// An async, thread-safe version of SkillCatalog that supports on-demand loading
#[derive(Debug, Default)]
pub struct AsyncSkillCatalog {
    inner: RwLock<SkillCatalog>,
}

impl AsyncSkillCatalog {
    /// Create a new async catalog
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(SkillCatalog::new()),
        }
    }

    /// Initialize from a synchronous catalog
    pub fn from_catalog(catalog: SkillCatalog) -> Self {
        Self {
            inner: RwLock::new(catalog),
        }
    }

    /// Get a catalog entry by name
    pub async fn get(&self, name: &str) -> Option<SkillCatalogEntry> {
        self.inner.read().await.get(name).cloned()
    }

    /// Get all catalog entries
    pub async fn entries(&self) -> Vec<SkillCatalogEntry> {
        self.inner
            .read()
            .await
            .entries()
            .into_iter()
            .cloned()
            .collect()
    }

    /// Get the number of skills
    #[allow(clippy::len_without_is_empty)]
    pub async fn len(&self) -> usize {
        self.inner.read().await.len()
    }

    /// Check if the catalog is empty
    pub async fn is_empty(&self) -> bool {
        self.inner.read().await.is_empty()
    }

    /// Find skills matching keywords
    pub async fn find_matching(&self, keywords: &[String]) -> Vec<SkillCatalogEntry> {
        self.inner
            .read()
            .await
            .find_matching(keywords)
            .into_iter()
            .cloned()
            .collect()
    }

    /// Search catalog by query string
    pub async fn search(&self, query: &str) -> Vec<SkillCatalogEntry> {
        self.inner
            .read()
            .await
            .search(query)
            .into_iter()
            .cloned()
            .collect()
    }

    /// Load the full skill content for a given skill name (Tier 2)
    ///
    /// This is called when a skill is activated and needs its full content.
    pub async fn load_skill(&self, name: &str) -> Result<Option<Skill>, SkillsError> {
        let entry = self.inner.read().await.get(name).cloned();
        match entry {
            Some(entry) => {
                // Get the loader from the inner catalog
                // Note: For now, we reconstruct from entry since loader isn't stored
                let skill = self.load_full_skill(&entry).await?;
                Ok(Some(skill))
            }
            None => Ok(None),
        }
    }

    /// Load full skill content from disk
    async fn load_full_skill(&self, entry: &SkillCatalogEntry) -> Result<Skill, SkillsError> {
        let skill_md_path = entry.location.join("SKILL.md");

        if !skill_md_path.exists() {
            return Err(SkillsError::SkillFileNotFound(entry.location.display().to_string()));
        }

        let content = tokio::fs::read_to_string(&skill_md_path).await?;
        let body = extract_body_after_frontmatter(&content);

        Ok(Skill::from_catalog_entry(entry.clone(), body))
    }

    /// Load scripts for a skill (on-demand)
    pub async fn load_skill_scripts(&self, name: &str) -> Result<Option<Vec<u8>>, SkillsError> {
        self.load_skill_resource(name, "scripts").await
    }

    /// Load references for a skill (on-demand)
    pub async fn load_skill_references(&self, name: &str) -> Result<Option<Vec<u8>>, SkillsError> {
        self.load_skill_resource(name, "references").await
    }

    /// Load assets for a skill (on-demand)
    pub async fn load_skill_assets(&self, name: &str) -> Result<Option<Vec<u8>>, SkillsError> {
        self.load_skill_resource(name, "assets").await
    }

    /// Load a specific resource directory as a tarball/zip
    async fn load_skill_resource(
        &self,
        name: &str,
        resource_type: &str,
    ) -> Result<Option<Vec<u8>>, SkillsError> {
        let entry = self.inner.read().await.get(name).cloned();
        match entry {
            Some(entry) => {
                let resource_path = entry.location.join(resource_type);
                if !resource_path.exists() || !resource_path.is_dir() {
                    return Ok(None);
                }

                // For now, just return a simple listing as bytes
                // In a full implementation, this could create a tarball
                let contents: Vec<String> = std::fs::read_dir(&resource_path)
                    .map(|entries| {
                        entries
                            .filter_map(|e| e.ok())
                            .map(|e| e.file_name().to_string_lossy().to_string())
                            .collect()
                    })
                    .unwrap_or_default();

                Ok(Some(contents.join("\n").into_bytes()))
            }
            None => Ok(None),
        }
    }
}

/// Extract the body (markdown after frontmatter) from SKILL.md content
fn extract_body_after_frontmatter(content: &str) -> String {
    let trimmed = content.trim();

    // Check for YAML frontmatter delimiters
    if let Some(stripped) = trimmed.strip_prefix("---") {
        if let Some(end_pos) = stripped.find("---") {
            // Skip past the closing ---
            let after_frontmatter = &stripped[end_pos + 3..];
            return after_frontmatter.trim().to_string();
        }
    }

    // No frontmatter found, return the whole content
    trimmed.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_extract_body_after_frontmatter() {
        let content = r#"---
name: test-skill
description: A test skill
---
# Test Skill

Some content here.
"#;

        let body = extract_body_after_frontmatter(content);
        assert!(body.contains("# Test Skill"));
        assert!(body.contains("Some content here"));
    }

    #[test]
    fn test_extract_body_no_frontmatter() {
        let content = "# Just a header\n\nSome content.";
        let body = extract_body_after_frontmatter(content);
        assert_eq!(body, "# Just a header\n\nSome content.");
    }

    #[test]
    fn test_skill_catalog_find_matching() {
        let mut catalog = SkillCatalog::new();

        catalog.add_entry(SkillCatalogEntry::new(
            "rust-coding".to_string(),
            "Write idiomatic Rust code with async tokio.".to_string(),
            PathBuf::from("/project/.swell/skills/rust-coding"),
            "rust-coding".to_string(),
        ));

        catalog.add_entry(SkillCatalogEntry::new(
            "test-writing".to_string(),
            "Write tests for Rust code using tokio::test.".to_string(),
            PathBuf::from("/project/.swell/skills/test-writing"),
            "test-writing".to_string(),
        ));

        catalog.add_entry(SkillCatalogEntry::new(
            "code-review".to_string(),
            "Review code for correctness and style.".to_string(),
            PathBuf::from("/project/.swell/skills/code-review"),
            "code-review".to_string(),
        ));

        // Match on rust
        let matches = catalog.find_matching(&["rust".to_string()]);
        assert_eq!(matches.len(), 2); // rust-coding and test-writing

        // Match on async (only rust-coding has "async" in description)
        let matches = catalog.find_matching(&["async".to_string()]);
        assert_eq!(matches.len(), 1); // only rust-coding

        // Match on tokio
        let matches = catalog.find_matching(&["tokio".to_string()]);
        assert_eq!(matches.len(), 2); // rust-coding and test-writing

        // Match on code (all three entries contain "code")
        let matches = catalog.find_matching(&["code".to_string()]);
        assert_eq!(matches.len(), 3);

        // Match on multiple
        let matches = catalog.find_matching(&["rust".to_string(), "code".to_string()]);
        assert_eq!(matches.len(), 3); // all mention code or rust
    }

    #[test]
    fn test_skill_catalog_search() {
        let mut catalog = SkillCatalog::new();

        catalog.add_entry(SkillCatalogEntry::new(
            "rust-coding".to_string(),
            "Write idiomatic Rust code.".to_string(),
            PathBuf::from("/path"),
            "rust-coding".to_string(),
        ));

        catalog.add_entry(SkillCatalogEntry::new(
            "test-writing".to_string(),
            "Write tests.".to_string(),
            PathBuf::from("/path"),
            "test-writing".to_string(),
        ));

        // Search for "rust"
        let results = catalog.search("rust");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "rust-coding");

        // Search for "write"
        let results = catalog.search("write");
        assert_eq!(results.len(), 2);
    }

    #[tokio::test]
    async fn test_async_skill_catalog() {
        let catalog = SkillCatalog::new();
        let async_catalog = AsyncSkillCatalog::from_catalog(catalog);

        // Initially empty
        assert_eq!(async_catalog.len().await, 0);
    }
}
