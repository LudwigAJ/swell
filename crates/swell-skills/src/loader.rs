//! Skills loader - discovers and parses skills from the filesystem
//!
//! This module implements the skill discovery logic for the Agent Skills
//! standard, scanning directories for SKILL.md files and parsing their
//! YAML frontmatter.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::fs;
use tracing::{debug, info, warn};

use crate::catalog::{AsyncSkillCatalog, SkillCatalog};
use crate::error::SkillsError;
use crate::types::{SkillCatalogEntry, SkillFrontmatter};

/// The standard name of the skill definition file
const SKILL_FILE_NAME: &str = "SKILL.md";

/// Default directories to scan for skills
const DEFAULT_SKILLS_DIRS: &[&str] = &[
    ".swell/skills",   // Project-local skills
    ".swell/skills.d", // Alternative location
];

/// Home directory skills location
const HOME_SKILLS_DIR: &str = ".swell/skills";

/// Loader for discovering and parsing skills
#[derive(Debug, Clone)]
pub struct SkillsLoader {
    /// Directories to scan for skills
    scan_dirs: Vec<PathBuf>,
    /// Cache of parsed frontmatter (name -> frontmatter)
    #[allow(dead_code)]
    frontmatter_cache: HashMap<String, SkillFrontmatter>,
    /// Whether to scan home directory
    scan_home: bool,
}

impl Default for SkillsLoader {
    fn default() -> Self {
        Self::new()
    }
}

impl SkillsLoader {
    /// Create a new skills loader with default settings
    pub fn new() -> Self {
        Self {
            scan_dirs: Vec::new(),
            frontmatter_cache: HashMap::new(),
            scan_home: true,
        }
    }

    /// Create a loader that scans the given directories
    pub fn with_dirs<P: Into<PathBuf>>(dirs: Vec<P>) -> Self {
        Self {
            scan_dirs: dirs.into_iter().map(|p| p.into()).collect(),
            frontmatter_cache: HashMap::new(),
            scan_home: false, // Don't scan home if explicit dirs provided
        }
    }

    /// Add a directory to scan
    pub fn add_scan_dir<P: Into<PathBuf>>(&mut self, dir: P) {
        self.scan_dirs.push(dir.into());
    }

    /// Set whether to scan the home directory
    pub fn set_scan_home(&mut self, scan: bool) {
        self.scan_home = scan;
    }

    /// Discover all skills in the configured directories
    ///
    /// Returns a tuple of (catalog, errors) where errors are non-fatal
    /// (e.g., directories that don't exist, files that can't be parsed)
    pub async fn discover(&self) -> Result<(SkillCatalog, Vec<String>), SkillsError> {
        let mut catalog = SkillCatalog::new();
        let mut errors = Vec::new();
        let mut seen_names: HashMap<String, PathBuf> = HashMap::new();

        // Collect all directories to scan
        let mut all_dirs = self.scan_dirs.clone();

        // Add home directory if enabled
        if self.scan_home {
            if let Some(home) = dirs::home_dir() {
                let home_skills = home.join(HOME_SKILLS_DIR);
                if !all_dirs.iter().any(|d| d == &home_skills) {
                    all_dirs.push(home_skills);
                }
            }
        }

        // Add default project-local directories if no explicit dirs
        if self.scan_dirs.is_empty() {
            for &default_dir in DEFAULT_SKILLS_DIRS {
                all_dirs.push(PathBuf::from(default_dir));
            }
        }

        info!(dirs = ?all_dirs, "Discovering skills");

        // Scan each directory
        for dir in &all_dirs {
            match self.scan_skill_dir(dir, &mut seen_names).await {
                Ok(entries) => {
                    for entry in entries {
                        debug!(skill = %entry.name, location = %entry.location.display(), "Discovered skill");
                        catalog.add_entry(entry);
                    }
                }
                Err(e) => {
                    warn!(dir = %dir.display(), error = %e, "Error scanning skills directory");
                    errors.push(format!("{}: {}", dir.display(), e));
                }
            }
        }

        info!(count = catalog.len(), "Skills discovery complete");
        Ok((catalog, errors))
    }

    /// Scan a single skills directory for skill subdirectories
    async fn scan_skill_dir(
        &self,
        dir: &Path,
        seen_names: &mut HashMap<String, PathBuf>,
    ) -> Result<Vec<SkillCatalogEntry>, SkillsError> {
        let mut entries = Vec::new();

        // Don't fail if directory doesn't exist
        if !dir.exists() {
            return Err(SkillsError::SkillsRootNotFound(dir.display().to_string()));
        }

        if !dir.is_dir() {
            return Err(SkillsError::InvalidSkillDirectory {
                location: dir.display().to_string(),
                reason: "Not a directory".to_string(),
            });
        }

        // Read all entries in the skills directory
        let mut read_dir = fs::read_dir(dir)
            .await
            .map_err(SkillsError::IoError)?;

        while let Some(entry) = read_dir.next_entry().await.map_err(SkillsError::IoError)? {
            let path = entry.path();

            // Skip if not a directory
            if !path.is_dir() {
                continue;
            }

            // Check for SKILL.md in this subdirectory
            let skill_md = path.join(SKILL_FILE_NAME);
            if !skill_md.exists() {
                debug!(dir = %path.display(), "Skipping directory without SKILL.md");
                continue;
            }

            // Parse the skill
            match self.parse_skill(&path, &skill_md).await {
                Ok(catalog_entry) => {
                    // Check for name conflicts
                    if let Some(existing) = seen_names.get(&catalog_entry.name) {
                        warn!(
                            name = %catalog_entry.name,
                            existing = %existing.display(),
                            new = %path.display(),
                            "Skill name conflict - skipping duplicate"
                        );
                        continue;
                    }

                    seen_names.insert(catalog_entry.name.clone(), path.clone());
                    entries.push(catalog_entry);
                }
                Err(e) => {
                    warn!(path = %path.display(), error = %e, "Failed to parse skill");
                }
            }
        }

        Ok(entries)
    }

    /// Parse a single skill's SKILL.md file
    async fn parse_skill(
        &self,
        skill_dir: &Path,
        skill_md: &Path,
    ) -> Result<SkillCatalogEntry, SkillsError> {
        let content = tokio::fs::read_to_string(skill_md)
            .await
            .map_err(SkillsError::IoError)?;

        let frontmatter = parse_frontmatter(&content, skill_md)?;

        // Compute relative path from the skills root
        let relative_path = if let Some(parent) = skill_dir.file_name() {
            parent.to_string_lossy().to_string()
        } else {
            skill_dir.to_string_lossy().to_string()
        };

        Ok(SkillCatalogEntry::new(
            frontmatter.name.clone(),
            frontmatter.description.clone(),
            skill_dir.to_path_buf(),
            relative_path,
        ))
    }

    /// Build an async skill catalog from discovered skills
    pub async fn build_async_catalog(&self) -> Result<AsyncSkillCatalog, SkillsError> {
        let (catalog, _errors) = self.discover().await?;
        Ok(AsyncSkillCatalog::from_catalog(catalog))
    }
}

/// Parse YAML frontmatter from SKILL.md content
///
/// This implements lenient validation - missing optional fields are allowed.
fn parse_frontmatter(content: &str, file_path: &Path) -> Result<SkillFrontmatter, SkillsError> {
    let trimmed = content.trim();

    // Must start with YAML frontmatter delimiter
    if !trimmed.starts_with("---") {
        // Use invalid YAML to ensure error
        return Err(SkillsError::YamlParseError {
            location: file_path.display().to_string(),
            source: serde_yaml::from_str::<SkillFrontmatter>("  [invalid").unwrap_err(),
        });
    }

    // Find the closing ---
    let after_first_dash = &trimmed[3..];
    match after_first_dash.find("---") {
        Some(end_pos) => {
            let frontmatter_str = &after_first_dash[..end_pos];

            // Parse the YAML
            let frontmatter: SkillFrontmatter =
                serde_yaml::from_str(frontmatter_str).map_err(|e| SkillsError::YamlParseError {
                    location: file_path.display().to_string(),
                    source: e,
                })?;

            // Validate required fields (lenient - accept missing optional fields)
            if frontmatter.name.is_empty() {
                return Err(SkillsError::MissingRequiredField {
                    field: "name",
                    location: file_path.display().to_string(),
                });
            }

            if frontmatter.description.is_empty() {
                return Err(SkillsError::MissingRequiredField {
                    field: "description",
                    location: file_path.display().to_string(),
                });
            }

            Ok(frontmatter)
        }
        None => Err(SkillsError::YamlParseError {
            location: file_path.display().to_string(),
            source: serde_yaml::from_str::<SkillFrontmatter>("  [invalid").unwrap_err(),
        }),
    }
}

/// Parse frontmatter from a string (for testing)
#[cfg(test)]
pub fn parse_frontmatter_str(content: &str) -> Result<SkillFrontmatter, SkillsError> {
    parse_frontmatter(content, Path::new("test://SKILL.md"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_valid_frontmatter() {
        let content = r#"---
name: rust-coding
description: Write idiomatic Rust code.
---
# Rust Coding
"#;

        let result = parse_frontmatter_str(content).unwrap();
        assert_eq!(result.name, "rust-coding");
        assert_eq!(result.description, "Write idiomatic Rust code.");
    }

    #[test]
    fn test_parse_frontmatter_with_extra_fields() {
        let content = r#"---
name: test-skill
description: A test skill.
version: 1.0.0
custom_field: value
---
# Test
"#;

        let result = parse_frontmatter_str(content).unwrap();
        assert_eq!(result.name, "test-skill");
        assert_eq!(result.description, "A test skill.");
        assert!(result.extra.contains_key("version"));
        assert!(result.extra.contains_key("custom_field"));
    }

    #[test]
    fn test_parse_frontmatter_missing_name() {
        let content = r#"---
description: No name here.
---
# Test
"#;

        let result = parse_frontmatter_str(content);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_frontmatter_missing_description() {
        let content = r#"---
name: no-description
---
# Test
"#;

        let result = parse_frontmatter_str(content);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_frontmatter_empty_name() {
        let content = r#"---
name: ""
description: Has description.
---
# Test
"#;

        let result = parse_frontmatter_str(content);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_frontmatter_no_frontmatter() {
        let content = "# Just a header\n\nNo frontmatter here.";

        let result = parse_frontmatter_str(content);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_frontmatter_missing_closing() {
        let content = r#"---
name: incomplete
description: Missing closing
"#;

        let result = parse_frontmatter_str(content);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_discover_empty_directory() {
        let temp_dir = tempfile::tempdir().unwrap();
        let loader = SkillsLoader::with_dirs(vec![temp_dir.path()]);

        let (catalog, errors) = loader.discover().await.unwrap();
        assert_eq!(catalog.len(), 0);
        // Empty directory should not produce errors
        assert!(errors.is_empty());
    }

    #[tokio::test]
    async fn test_discover_nonexistent_directory() {
        let loader = SkillsLoader::with_dirs(vec!["/nonexistent/path"]);

        let (catalog, errors) = loader.discover().await.unwrap();
        assert_eq!(catalog.len(), 0);
        // Should have an error about the missing directory
        assert!(!errors.is_empty());
    }
}
