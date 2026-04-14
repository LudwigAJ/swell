//! SkillTool adapter - wraps skill definitions as Tool trait objects for execution.
//!
//! This module provides:
//! - [`SkillTool`] - A tool adapter that wraps a skill definition
//! - [`SkillDiscovery`] - Ancestor-walk directory discovery for .swell/skills/
//! - [`register_skills_from_workspace`] - Discovers and registers skills into ToolRegistry

use async_trait::async_trait;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use swell_core::traits::{Tool, ToolBehavioralHints};
use swell_core::{PermissionTier, SwellError, ToolOutput, ToolResultContent, ToolRiskLevel};
use tokio::fs;
use tracing::{debug, info, warn};

/// The standard name of the skill definition file
const SKILL_FILE_NAME: &str = "SKILL.md";

/// The standard skills directory name
const SKILLS_DIR_NAME: &str = ".swell/skills";

/// A tool adapter that wraps a skill definition for execution via ToolRegistry.
///
/// SkillTool allows skills from `.swell/skills/` to be invoked like any other tool.
/// When executed, it loads the skill's SKILL.md body and returns it as tool output.
///
/// # Example
///
/// ```ignore
/// let skill_tool = SkillTool::new("rust-coding", "/project/.swell/skills/rust-coding");
/// let output = skill_tool.execute(json!({})).await?;
/// ```
#[derive(Debug, Clone)]
pub struct SkillTool {
    /// The skill's unique name
    name: String,
    /// The skill's description from frontmatter
    description: String,
    /// Absolute path to the skill directory
    location: PathBuf,
    /// Keywords for matching
    #[allow(dead_code)]
    keywords: Vec<String>,
    /// Whether this skill has scripts/
    #[allow(dead_code)]
    has_scripts: bool,
    /// Whether this skill has references/
    #[allow(dead_code)]
    has_references: bool,
    /// Whether this skill has assets/
    #[allow(dead_code)]
    has_assets: bool,
}

impl SkillTool {
    /// Create a new SkillTool from a catalog entry
    pub fn from_catalog_entry(
        name: String,
        description: String,
        location: PathBuf,
        keywords: Vec<String>,
    ) -> Self {
        let scripts_dir = location.join("scripts");
        let references_dir = location.join("references");
        let assets_dir = location.join("assets");

        Self {
            name,
            description,
            location,
            keywords,
            has_scripts: scripts_dir.exists() && scripts_dir.is_dir(),
            has_references: references_dir.exists() && references_dir.is_dir(),
            has_assets: assets_dir.exists() && assets_dir.is_dir(),
        }
    }

    /// Get the path to the SKILL.md file
    fn skill_md_path(&self) -> PathBuf {
        self.location.join(SKILL_FILE_NAME)
    }

    /// Load the full skill content from disk
    async fn load_skill_body(&self) -> Result<String, SwellError> {
        let skill_md = self.skill_md_path();

        if !skill_md.exists() {
            return Err(SwellError::ToolExecutionFailed(format!(
                "SKILL.md not found at '{}'",
                skill_md.display()
            )));
        }

        let content = tokio::fs::read_to_string(&skill_md).await.map_err(|e| {
            SwellError::ToolExecutionFailed(format!("Failed to read skill file: {}", e))
        })?;

        // Extract body after frontmatter
        let body = extract_skill_body(&content);
        Ok(body)
    }

    /// List available scripts in the skill's scripts/ directory
    pub fn list_scripts(&self) -> Vec<String> {
        self.list_dir_contents("scripts")
    }

    /// List available references in the skill's references/ directory
    pub fn list_references(&self) -> Vec<String> {
        self.list_dir_contents("references")
    }

    /// List available assets in the skill's assets/ directory
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

#[async_trait]
impl Tool for SkillTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> String {
        self.description.clone()
    }

    fn input_schema(&self) -> serde_json::Value {
        // Skills don't take input - they just return their content
        serde_json::json!({
            "type": "object",
            "properties": {},
            "required": []
        })
    }

    fn risk_level(&self) -> ToolRiskLevel {
        // Skills are read-only by default - they just provide guidance
        ToolRiskLevel::Read
    }

    fn permission_tier(&self) -> PermissionTier {
        PermissionTier::Auto
    }

    fn behavioral_hints(&self) -> ToolBehavioralHints {
        ToolBehavioralHints {
            read_only_hint: true,
            destructive_hint: false,
            idempotent_hint: true,
        }
    }

    async fn execute(&self, _arguments: serde_json::Value) -> Result<ToolOutput, SwellError> {
        // Load and return the skill body
        let body = self.load_skill_body().await?;

        Ok(ToolOutput {
            is_error: false,
            content: vec![ToolResultContent::Text(body)],
        })
    }

    fn is_available(&self) -> bool {
        self.skill_md_path().exists()
    }
}

/// Extract the body (markdown after frontmatter) from SKILL.md content
fn extract_skill_body(content: &str) -> String {
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

/// Discovers skills by walking up the directory tree from a starting path.
///
/// Discovery starts from `start_path` and walks up to the filesystem root,
/// searching for `.swell/skills/` directories at each level. Skills found
/// closer to the start path take precedence over those found higher up.
///
/// # Example
///
/// ```ignore
/// let discovery = SkillDiscovery::new();
/// let skills = discovery.discover_from("/project/src/utils").await?;
/// ```
#[derive(Debug, Clone, Default)]
pub struct SkillDiscovery {
    /// Additional search roots to scan (beyond ancestor walk)
    extra_roots: Vec<PathBuf>,
}

impl SkillDiscovery {
    /// Create a new skill discovery instance
    pub fn new() -> Self {
        Self::default()
    }

    /// Add an additional search root directory
    pub fn add_search_root<P: Into<PathBuf>>(&mut self, path: P) {
        self.extra_roots.push(path.into());
    }

    /// Discover all skills by walking ancestor directories.
    ///
    /// Starts from `start_path` and walks up to the filesystem root,
    /// collecting skills from each `.swell/skills/` directory found.
    /// Skills are deduplicated by name, with earlier discoveries taking precedence.
    pub async fn discover_from(&self, start_path: &Path) -> Result<Vec<SkillInfo>, SwellError> {
        let mut skills = Vec::new();
        let mut seen_names: std::collections::HashSet<String> = std::collections::HashSet::new();

        // Walk up from start_path
        let mut current = start_path.to_path_buf();
        loop {
            let skills_dir = current.join(SKILLS_DIR_NAME);

            if skills_dir.is_dir() {
                match self.scan_skills_dir(&skills_dir).await {
                    Ok(dir_skills) => {
                        for skill in dir_skills {
                            if seen_names.insert(skill.name.clone()) {
                                skills.push(skill);
                            }
                        }
                    }
                    Err(e) => {
                        debug!(dir = %skills_dir.display(), error = %e, "Error scanning skills directory during ancestor walk");
                    }
                }
            }

            // Move to parent or stop if we can't go higher
            if !current.pop() {
                break;
            }
        }

        // Also scan any extra roots (e.g., home directory, workspace root)
        for extra_root in &self.extra_roots {
            if extra_root.is_dir() {
                let skills_dir = extra_root.join(SKILLS_DIR_NAME);
                if skills_dir.is_dir() {
                    match self.scan_skills_dir(&skills_dir).await {
                        Ok(dir_skills) => {
                            for skill in dir_skills {
                                if seen_names.insert(skill.name.clone()) {
                                    skills.push(skill);
                                }
                            }
                        }
                        Err(e) => {
                            debug!(dir = %skills_dir.display(), error = %e, "Error scanning extra skills root");
                        }
                    }
                }
            }
        }

        info!(count = skills.len(), "Skill discovery complete");
        Ok(skills)
    }

    /// Discover skills from the current working directory
    pub async fn discover_from_cwd(&self) -> Result<Vec<SkillInfo>, SwellError> {
        let cwd = std::env::current_dir().map_err(|e| {
            SwellError::ToolExecutionFailed(format!("Failed to get current directory: {}", e))
        })?;
        self.discover_from(&cwd).await
    }

    /// Discover skills from a list of explicit paths
    pub async fn discover_from_paths(
        &self,
        paths: &[PathBuf],
    ) -> Result<Vec<SkillInfo>, SwellError> {
        let mut all_skills = Vec::new();
        let mut seen_names: std::collections::HashSet<String> = std::collections::HashSet::new();

        for path in paths {
            if path.is_dir() {
                let skills_dir = path.join(SKILLS_DIR_NAME);
                if skills_dir.is_dir() {
                    match self.scan_skills_dir(&skills_dir).await {
                        Ok(dir_skills) => {
                            for skill in dir_skills {
                                if seen_names.insert(skill.name.clone()) {
                                    all_skills.push(skill);
                                }
                            }
                        }
                        Err(e) => {
                            debug!(dir = %skills_dir.display(), error = %e, "Error scanning skills directory");
                        }
                    }
                }
            }
        }

        Ok(all_skills)
    }

    /// Scan a single skills directory for skill subdirectories
    async fn scan_skills_dir(&self, dir: &Path) -> Result<Vec<SkillInfo>, SwellError> {
        let mut skills = Vec::new();

        if !dir.exists() {
            return Ok(skills);
        }

        let mut entries = fs::read_dir(dir).await.map_err(|e| {
            SwellError::ToolExecutionFailed(format!("Failed to read skills directory: {}", e))
        })?;

        while let Some(entry) = entries.next_entry().await.map_err(|e| {
            SwellError::ToolExecutionFailed(format!("Failed to read directory entry: {}", e))
        })? {
            let path = entry.path();

            // Skip if not a directory
            if !path.is_dir() {
                continue;
            }

            // Check for SKILL.md in this subdirectory
            let skill_md = path.join(SKILL_FILE_NAME);
            if !skill_md.exists() {
                continue;
            }

            // Parse the skill
            match self.parse_skill(&path).await {
                Ok(skill_info) => {
                    debug!(name = %skill_info.name, location = %skill_info.location.display(), "Discovered skill");
                    skills.push(skill_info);
                }
                Err(e) => {
                    warn!(path = %path.display(), error = %e, "Failed to parse skill");
                }
            }
        }

        Ok(skills)
    }

    /// Parse a single skill's SKILL.md file
    async fn parse_skill(&self, skill_dir: &Path) -> Result<SkillInfo, SwellError> {
        let skill_md = skill_dir.join(SKILL_FILE_NAME);

        let content = tokio::fs::read_to_string(&skill_md).await.map_err(|e| {
            SwellError::ToolExecutionFailed(format!("Failed to read {}: {}", skill_md.display(), e))
        })?;

        let frontmatter = parse_skill_frontmatter(&content, &skill_md)?;

        let relative_path = skill_dir
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| skill_dir.to_string_lossy().to_string());

        Ok(SkillInfo {
            name: frontmatter.name,
            description: frontmatter.description,
            location: skill_dir.to_path_buf(),
            relative_path,
            keywords: frontmatter.keywords,
            has_scripts: skill_dir.join("scripts").is_dir(),
            has_references: skill_dir.join("references").is_dir(),
            has_assets: skill_dir.join("assets").is_dir(),
        })
    }
}

/// Information about a discovered skill
#[derive(Debug, Clone)]
pub struct SkillInfo {
    /// The skill's unique name
    pub name: String,
    /// A brief description
    pub description: String,
    /// Absolute path to the skill directory
    pub location: PathBuf,
    /// Relative path from skills root
    pub relative_path: String,
    /// Keywords for matching
    pub keywords: Vec<String>,
    /// Whether this skill has scripts/
    pub has_scripts: bool,
    /// Whether this skill has references/
    pub has_references: bool,
    /// Whether this skill has assets/
    pub has_assets: bool,
}

impl SkillInfo {
    /// Convert this skill info into a SkillTool for registration
    pub fn into_skill_tool(self) -> SkillTool {
        SkillTool::from_catalog_entry(self.name, self.description, self.location, self.keywords)
    }
}

/// Frontmatter parsed from a SKILL.md file
#[derive(Debug)]
struct SkillFrontmatter {
    name: String,
    description: String,
    keywords: Vec<String>,
}

/// Parse YAML frontmatter from SKILL.md content
fn parse_skill_frontmatter(
    content: &str,
    file_path: &Path,
) -> Result<SkillFrontmatter, SwellError> {
    let trimmed = content.trim();

    // Must start with YAML frontmatter delimiter
    if !trimmed.starts_with("---") {
        return Err(SwellError::ToolExecutionFailed(format!(
            "SKILL.md at '{}' missing YAML frontmatter",
            file_path.display()
        )));
    }

    // Find the closing ---
    let after_first_dash = &trimmed[3..];
    let end_pos = after_first_dash.find("---").ok_or_else(|| {
        SwellError::ToolExecutionFailed(format!(
            "SKILL.md at '{}' has unclosed YAML frontmatter",
            file_path.display()
        ))
    })?;

    let frontmatter_str = &after_first_dash[..end_pos];

    // Parse the YAML - we do a simple parse for name and description
    let name = extract_yaml_field(frontmatter_str, "name").ok_or_else(|| {
        SwellError::ToolExecutionFailed(format!(
            "SKILL.md at '{}' missing required 'name' field",
            file_path.display()
        ))
    })?;

    let description = extract_yaml_field(frontmatter_str, "description").unwrap_or_default();

    let keywords = extract_keywords(&description);

    Ok(SkillFrontmatter {
        name,
        description,
        keywords,
    })
}

/// Extract a field value from YAML frontmatter (simple parser)
fn extract_yaml_field(frontmatter: &str, field: &str) -> Option<String> {
    let line_prefix = format!("{}:", field);

    for line in frontmatter.lines() {
        let line = line.trim();
        if line.starts_with(&line_prefix) {
            let value = line[line_prefix.len()..].trim();
            // Remove quotes if present
            let value = value.trim_matches('"').trim_matches('\'');
            if !value.is_empty() {
                return Some(value.to_string());
            }
        }
    }

    None
}

/// Extract keywords from a description
fn extract_keywords(description: &str) -> Vec<String> {
    let lower = description.to_lowercase();

    // Split on common delimiters and filter short words
    let words: Vec<String> = lower
        .split(|c: char| {
            c.is_whitespace() || c == ',' || c == '-' || c == ':' || c == '/' || c == '.'
        })
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

/// Register all discovered skills into a ToolRegistry at the Plugin layer.
///
/// This is the main entry point for wiring skills into the tool system.
/// It discovers skills via ancestor walk from the workspace root and registers
/// each one as a Plugin-layer tool in the registry.
///
/// # Arguments
///
/// * `registry` - The ToolRegistry to register skills into
/// * `workspace_root` - The workspace root to discover skills from
///
/// # Example
///
/// ```ignore
/// use swell_tools::registry::ToolRegistry;
/// use swell_tools::skill::register_skills_from_workspace;
///
/// let registry = ToolRegistry::new();
/// register_skills_from_workspace(&registry, "/project").await;
/// ```
pub async fn register_skills_from_workspace(
    registry: &Arc<ToolRegistry>,
    workspace_root: &Path,
) -> Result<usize, SwellError> {
    let discovery = SkillDiscovery::new();
    let skills = discovery.discover_from(workspace_root).await?;

    let count = skills.len();

    for skill in skills {
        let tool = skill.into_skill_tool();
        let name = tool.name().to_string();

        registry.register_plugin(tool, ToolCategory::Misc).await;

        debug!(name = %name, "Registered skill as plugin tool");
    }

    info!(count = count, "Registered skills into ToolRegistry");
    Ok(count)
}

// Re-export ToolRegistry and ToolCategory for convenience
use crate::registry::{ToolCategory, ToolRegistry};

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    fn create_test_skill(
        dir: &Path,
        name: &str,
        description: &str,
        body: &str,
    ) -> std::io::Result<()> {
        let skill_dir = dir.join(name);
        std::fs::create_dir_all(&skill_dir)?;

        let content = format!(
            "---\nname: {}\ndescription: {}\n---\n{}",
            name, description, body
        );

        std::fs::write(skill_dir.join("SKILL.md"), content)?;
        Ok(())
    }

    #[tokio::test]
    async fn test_skill_tool_name_and_description() {
        let temp_dir = TempDir::new().unwrap();
        create_test_skill(
            temp_dir.path(),
            "test-skill",
            "A test skill",
            "# Test\n\nContent",
        )
        .unwrap();

        let tool = SkillTool::from_catalog_entry(
            "test-skill".to_string(),
            "A test skill".to_string(),
            temp_dir.path().join("test-skill"),
            vec!["test".to_string()],
        );

        assert_eq!(tool.name(), "test-skill");
        assert_eq!(tool.description(), "A test skill");
    }

    #[tokio::test]
    async fn test_skill_tool_execute() {
        let temp_dir = TempDir::new().unwrap();
        create_test_skill(
            temp_dir.path(),
            "rust-coding",
            "Write Rust code",
            "# Rust Coding\n\nUse ownership and borrow checker.",
        )
        .unwrap();

        let tool = SkillTool::from_catalog_entry(
            "rust-coding".to_string(),
            "Write Rust code".to_string(),
            temp_dir.path().join("rust-coding"),
            vec!["rust".to_string(), "code".to_string()],
        );

        let output = tool.execute(json!({})).await.unwrap();
        assert!(!output.is_error);
        match &output.content[0] {
            swell_core::ToolResultContent::Text(s) => {
                assert!(s.contains("Rust Coding"));
            }
            _ => panic!("Expected Text variant"),
        }
    }

    #[tokio::test]
    async fn test_skill_discovery_ancestor_walk() {
        // Create nested directory structure
        let temp_dir = TempDir::new().unwrap();
        let project_dir = temp_dir.path().join("project");
        let src_dir = project_dir.join("src");
        std::fs::create_dir_all(&src_dir).unwrap();

        // Create skills at different levels
        let skills_dir = project_dir.join(".swell/skills");
        std::fs::create_dir_all(&skills_dir).unwrap();
        create_test_skill(
            &skills_dir,
            "project-skill",
            "A project skill",
            "# Project Skill",
        )
        .unwrap();

        let deeper_skills_dir = src_dir.join(".swell/skills");
        std::fs::create_dir_all(&deeper_skills_dir).unwrap();
        create_test_skill(
            &deeper_skills_dir,
            "src-skill",
            "A src skill",
            "# Src Skill",
        )
        .unwrap();

        let discovery = SkillDiscovery::new();
        let skills = discovery.discover_from(&src_dir).await.unwrap();

        // Should find both skills
        assert_eq!(skills.len(), 2);

        // Skills should include names
        let names: Vec<_> = skills.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"project-skill"));
        assert!(names.contains(&"src-skill"));
    }

    #[tokio::test]
    async fn test_skill_discovery_precedence() {
        // Create nested structure with same skill name at different levels
        let temp_dir = TempDir::new().unwrap();
        let project_dir = temp_dir.path().join("project");
        let src_dir = project_dir.join("src");
        std::fs::create_dir_all(&src_dir).unwrap();

        // Create skill at project level
        let skills_dir = project_dir.join(".swell/skills");
        std::fs::create_dir_all(&skills_dir).unwrap();
        create_test_skill(&skills_dir, "duplicate", "Project level skill", "# Project").unwrap();

        // Create same skill at src level (should take precedence)
        let deeper_skills_dir = src_dir.join(".swell/skills");
        std::fs::create_dir_all(&deeper_skills_dir).unwrap();
        create_test_skill(&deeper_skills_dir, "duplicate", "Src level skill", "# Src").unwrap();

        let discovery = SkillDiscovery::new();
        let skills = discovery.discover_from(&src_dir).await.unwrap();

        // Should find only one (deeper takes precedence)
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].description, "Src level skill");
    }

    #[tokio::test]
    async fn test_extract_skill_body() {
        let content = r#"---
name: test-skill
description: A test skill
---
# Test Skill

Some content here with **markdown**.
"#;

        let body = extract_skill_body(content);
        assert!(body.contains("# Test Skill"));
        assert!(body.contains("Some content here"));
    }

    #[tokio::test]
    async fn test_extract_skill_body_no_frontmatter() {
        let content = "# Just a header\n\nSome content without frontmatter.";
        let body = extract_skill_body(content);
        assert!(body.contains("Just a header"));
    }

    #[tokio::test]
    async fn test_keyword_extraction() {
        let desc = "Write idiomatic Rust code with async tokio and ownership.";
        let keywords = extract_keywords(desc);

        assert!(keywords.contains(&"rust".to_string()));
        assert!(keywords.contains(&"async".to_string()));
        assert!(keywords.contains(&"ownership".to_string()));
        // Should filter short words
        assert!(!keywords.contains(&"with".to_string()));
        assert!(!keywords.contains(&"and".to_string()));
    }

    #[tokio::test]
    async fn test_skill_info_into_skill_tool() {
        let skill_info = SkillInfo {
            name: "rust-coding".to_string(),
            description: "Write Rust code".to_string(),
            location: PathBuf::from("/project/.swell/skills/rust-coding"),
            relative_path: "rust-coding".to_string(),
            keywords: vec!["rust".to_string()],
            has_scripts: false,
            has_references: false,
            has_assets: false,
        };

        let tool = skill_info.into_skill_tool();
        assert_eq!(tool.name(), "rust-coding");
    }
}
