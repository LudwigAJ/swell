//! Error types for the skills system

use thiserror::Error;

/// Errors that can occur when loading or processing skills
#[derive(Debug, Error)]
pub enum SkillsError {
    /// Failed to read a skill file
    #[error("Failed to read skill file: {0}")]
    IoError(#[from] std::io::Error),

    /// Failed to parse YAML frontmatter
    #[error("Failed to parse YAML frontmatter in {location}: {source}")]
    YamlParseError {
        location: String,
        #[source]
        source: serde_yaml::Error,
    },

    /// SKILL.md file not found in skill directory
    #[error("SKILL.md not found in {0}")]
    SkillFileNotFound(String),

    /// Invalid skill directory structure
    #[error("Invalid skill directory {reason}: {reason}")]
    InvalidSkillDirectory { location: String, reason: String },

    /// Skills root directory not found
    #[error("Skills root directory not found: {0}")]
    SkillsRootNotFound(String),

    /// Skill name conflict (same name in multiple locations)
    #[error("Skill name conflict: '{name}' found in both {loc1} and {loc2}")]
    SkillNameConflict {
        name: String,
        loc1: String,
        loc2: String,
    },

    /// Failed to parse frontmatter - missing required field
    #[error("Missing required field '{field}' in skill at {location}")]
    MissingRequiredField {
        field: &'static str,
        location: String,
    },
}
