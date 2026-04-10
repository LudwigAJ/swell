//! Core types for the Agent Skills system

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// YAML frontmatter extracted from a SKILL.md file
///
/// This is Tier 1 data - lightweight info loaded at startup for catalog.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillFrontmatter {
    /// The skill's unique name (required)
    pub name: String,
    /// A brief description for catalog display and keyword matching (required)
    pub description: String,
    /// Optional additional metadata from frontmatter
    #[serde(flatten)]
    pub extra: std::collections::HashMap<String, serde_yaml::Value>,
}

/// A skill catalog entry - lightweight info for startup catalog
///
/// This represents Tier 1 data (~50-100 tokens) loaded at startup.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillCatalogEntry {
    /// The skill's unique name
    pub name: String,
    /// A brief description for catalog display and keyword matching
    pub description: String,
    /// Absolute path to the skill directory
    pub location: PathBuf,
    /// Relative path from skills root (e.g., "rust-coding")
    pub relative_path: String,
    /// Keywords extracted from description for matching
    #[serde(default)]
    pub keywords: Vec<String>,
}

impl SkillCatalogEntry {
    /// Create a new catalog entry from frontmatter and location
    pub fn new(
        name: String,
        description: String,
        location: PathBuf,
        relative_path: String,
    ) -> Self {
        let keywords = extract_keywords(&description);
        Self {
            name,
            description,
            location,
            relative_path,
            keywords,
        }
    }

    /// Check if this skill matches the given keywords
    pub fn matches_keywords(&self, search_keywords: &[String]) -> bool {
        let search_set: std::collections::HashSet<&str> =
            search_keywords.iter().map(|s| s.as_str()).collect();

        // Check if any keyword from the search matches any keyword in the skill
        for keyword in &self.keywords {
            if search_set.contains(keyword.as_str()) {
                return true;
            }
        }

        // Also check if any search keyword is contained in the description
        let desc_lower = self.description.to_lowercase();
        for keyword in search_keywords {
            if desc_lower.contains(&keyword.to_lowercase()) {
                return true;
            }
        }

        false
    }
}

/// Extract keywords from a description
fn extract_keywords(description: &str) -> Vec<String> {
    let lower = description.to_lowercase();

    // Split on common delimiters and filter short words
    let words: Vec<String> = lower
        .split(|c: char| c.is_whitespace() || c == ',' || c == '-' || c == ':' || c == '/')
        .filter(|s| s.len() >= 3)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    // Remove common stop words
    let stop_words: std::collections::HashSet<&str> = [
        "the", "and", "for", "when", "with", "from", "this", "that", "also", "are", "was", "were",
        "been", "have", "has", "had", "but", "not", "you", "your", "can", "use", "using", "used",
        "etc",
    ]
    .into_iter()
    .collect();

    words
        .into_iter()
        .filter(|w| !stop_words.contains(w.as_str()))
        .collect()
}

/// A fully loaded skill with all content
///
/// This represents Tier 2 data - loaded on demand when a skill is activated.
#[derive(Debug, Clone)]
pub struct Skill {
    /// The skill's unique name
    pub name: String,
    /// A brief description for catalog display
    pub description: String,
    /// Absolute path to the skill directory
    pub location: PathBuf,
    /// Relative path from skills root
    pub relative_path: String,
    /// The full SKILL.md body content (markdown after frontmatter)
    pub body: String,
    /// Keywords extracted from description
    pub keywords: Vec<String>,
    /// Whether scripts/ directory exists
    pub has_scripts: bool,
    /// Whether references/ directory exists
    pub has_references: bool,
    /// Whether assets/ directory exists
    pub has_assets: bool,
}

impl Skill {
    /// Create a new skill from a catalog entry and full body
    pub fn from_catalog_entry(entry: SkillCatalogEntry, body: String) -> Self {
        let scripts_dir = entry.location.join("scripts");
        let references_dir = entry.location.join("references");
        let assets_dir = entry.location.join("assets");

        Self {
            name: entry.name,
            description: entry.description,
            location: entry.location.clone(),
            relative_path: entry.relative_path.clone(),
            body,
            keywords: entry.keywords,
            has_scripts: scripts_dir.exists() && scripts_dir.is_dir(),
            has_references: references_dir.exists() && references_dir.is_dir(),
            has_assets: assets_dir.exists() && assets_dir.is_dir(),
        }
    }

    /// Check if this skill matches the given keywords
    pub fn matches_keywords(&self, search_keywords: &[String]) -> bool {
        let entry = SkillCatalogEntry {
            name: self.name.clone(),
            description: self.description.clone(),
            location: self.location.clone(),
            relative_path: self.relative_path.clone(),
            keywords: self.keywords.clone(),
        };
        entry.matches_keywords(search_keywords)
    }
}

/// Resources associated with a skill
#[derive(Debug, Clone)]
pub struct SkillResources {
    /// Path to the scripts directory
    pub scripts_path: PathBuf,
    /// Path to the references directory
    pub references_path: PathBuf,
    /// Path to the assets directory
    pub assets_path: PathBuf,
}

impl Skill {
    /// Get the paths to skill resources
    pub fn resources(&self) -> SkillResources {
        SkillResources {
            scripts_path: self.location.join("scripts"),
            references_path: self.location.join("references"),
            assets_path: self.location.join("assets"),
        }
    }

    /// List scripts in the skill's scripts/ directory
    pub fn list_scripts(&self) -> Vec<String> {
        self.list_dir_contents("scripts")
    }

    /// List references in the skill's references/ directory
    pub fn list_references(&self) -> Vec<String> {
        self.list_dir_contents("references")
    }

    /// List assets in the skill's assets/ directory
    pub fn list_assets(&self) -> Vec<String> {
        self.list_dir_contents("assets")
    }

    fn list_dir_contents(&self, subdir: &str) -> Vec<String> {
        let path = self.location.join(subdir);
        if !path.exists() || !path.is_dir() {
            return Vec::new();
        }

        std::fs::read_dir(&path)
            .ok()
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .map(|e| e.file_name().to_string_lossy().to_string())
                    .collect()
            })
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_keyword_extraction() {
        let desc = "Write idiomatic Rust code. Use when implementing features in Rust, fixing bugs, or working with async tokio.";
        let keywords = extract_keywords(desc);

        assert!(keywords.contains(&"rust".to_string()));
        assert!(keywords.contains(&"async".to_string()));
        assert!(keywords.contains(&"features".to_string()));
        // Should filter short words
        assert!(!keywords.contains(&"use".to_string()));
        assert!(!keywords.contains(&"when".to_string()));
    }

    #[test]
    fn test_skill_catalog_entry_matches() {
        let entry = SkillCatalogEntry::new(
            "rust-coding".to_string(),
            "Write idiomatic Rust code. Use when implementing features in Rust, fixing bugs, working with async tokio.".to_string(),
            PathBuf::from("/project/.swell/skills/rust-coding"),
            "rust-coding".to_string(),
        );

        // Should match on keywords
        assert!(entry.matches_keywords(&["rust".to_string()]));
        assert!(entry.matches_keywords(&["async".to_string(), "tokio".to_string()]));
        assert!(entry.matches_keywords(&["features".to_string()]));

        // Should not match unrelated
        assert!(!entry.matches_keywords(&["python".to_string()]));
    }

    #[test]
    fn test_skill_catalog_entry_no_keywords() {
        let entry = SkillCatalogEntry::new(
            "test-skill".to_string(),
            "A test skill".to_string(),
            PathBuf::from("/project/.swell/skills/test-skill"),
            "test-skill".to_string(),
        );

        // Short description should still work
        assert!(entry.matches_keywords(&["test".to_string()]));
    }

    #[test]
    fn test_skill_creation() {
        let entry = SkillCatalogEntry::new(
            "rust-coding".to_string(),
            "Write Rust code".to_string(),
            PathBuf::from("/project/.swell/skills/rust-coding"),
            "rust-coding".to_string(),
        );
        let body = "# Rust Coding\n\nSome content".to_string();

        let skill = Skill::from_catalog_entry(entry, body.clone());

        assert_eq!(skill.name, "rust-coding");
        assert_eq!(skill.description, "Write Rust code");
        assert_eq!(skill.body, body);
        assert!(!skill.has_scripts);
        assert!(!skill.has_references);
        assert!(!skill.has_assets);
    }
}
