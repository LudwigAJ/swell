//! Agent Skills loader following agentskills.io standard
//!
//! This crate provides functionality to discover, parse, and load Agent Skills
//! from the standard .swell/skills/ directory structure.
//!
//! # Skill Directory Structure
//!
//! ```text
//! .swell/skills/          # User-extensible - add skills here!
//! ├── rust-coding/
//! │   ├── SKILL.md       # Required: YAML frontmatter + markdown body
//! │   ├── scripts/       # Optional: executable scripts
//! │   ├── references/    # Optional: reference documents
//! │   └── assets/        # Optional: images, etc.
//! └── test-writing/
//!     └── SKILL.md
//! ```
//!
//! # SKILL.md Format
//!
//! ```yaml
//! ---
//! name: my-custom-skill
//! description: What this skill does and when to use it.
//! ---
//! # Instructions in Markdown
//! ```

pub mod catalog;
pub mod error;
pub mod loader;
pub mod types;

pub use catalog::SkillCatalog;
pub use error::SkillsError;
pub use loader::SkillsLoader;
pub use types::{Skill, SkillCatalogEntry, SkillFrontmatter};
