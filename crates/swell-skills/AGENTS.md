# swell-skills AGENTS.md

## Purpose

`swell-skills` provides the Agent Skills loader following the [agentskills.io](https://agentskills.io) standard. It enables discovery, parsing, and loading of reusable agent capabilities defined in `.swell/skills/` directories.

This crate handles:
- **Skill Discovery** — Scan directories for skill subdirectories containing SKILL.md
- **YAML Frontmatter Parsing** — Parse skill metadata from YAML frontmatter
- **Skill Catalog** — Manage discovered skills with conflict detection
- **Async Skill Loading** — Async-compatible skill loading for runtime use
- **User-Extensible Registry** — Users can add skills without code changes

**Depends on:** `swell-core` (for error types)

## Public API

### Skill Types (`types.rs`)

```rust
pub struct SkillFrontmatter {
    pub name: String,
    pub description: String,
    pub extra: HashMap<String, serde_yaml::Value>,  // Additional fields
}

pub struct Skill {
    pub frontmatter: SkillFrontmatter,
    pub content: String,  // Full SKILL.md content
    pub location: PathBuf,
}

pub struct SkillCatalogEntry {
    pub name: String,
    pub description: String,
    pub location: PathBuf,
    pub relative_path: String,
}

impl SkillCatalogEntry {
    pub fn new(name: String, description: String, location: PathBuf, relative_path: String) -> Self;
}
```

### Skills Loader (`loader.rs`)

```rust
pub struct SkillsLoader {
    scan_dirs: Vec<PathBuf>,
    frontmatter_cache: HashMap<String, SkillFrontmatter>,
    scan_home: bool,
}

impl SkillsLoader {
    pub fn new() -> Self;
    pub fn with_dirs(dirs: Vec<P>) -> Self;
    pub fn add_scan_dir<P: Into<PathBuf>>(&mut self, dir: P);
    pub fn set_scan_home(&mut self, scan: bool);
    pub async fn discover(&self) -> Result<(SkillCatalog, Vec<String>), SkillsError>;
    pub async fn build_async_catalog(&self) -> Result<AsyncSkillCatalog, SkillsError>;
}
```

### Skill Catalog (`catalog.rs`)

```rust
pub struct SkillCatalog {
    entries: HashMap<String, SkillCatalogEntry>,
}

impl SkillCatalog {
    pub fn new() -> Self;
    pub fn add_entry(&mut self, entry: SkillCatalogEntry);
    pub fn get(&self, name: &str) -> Option<&SkillCatalogEntry>;
    pub fn len(&self) -> usize;
    pub fn is_empty(&self) -> bool;
    pub fn entries(&self) -> Vec<&SkillCatalogEntry>;
}

pub struct AsyncSkillCatalog {
    // Async-compatible wrapper around SkillCatalog
}

impl AsyncSkillCatalog {
    pub fn from_catalog(catalog: SkillCatalog) -> Self;
    pub async fn get_skill(&self, name: &str) -> Option<Skill>;
}
```

### Error Types (`error.rs`)

```rust
#[derive(Error, Debug)]
pub enum SkillsError {
    #[error("Skills root directory not found: {0}")]
    SkillsRootNotFound(String),

    #[error("Invalid skill directory: {location} - {reason}")]
    InvalidSkillDirectory { location: String, reason: String },

    #[error("Missing required field '{field}' in {location}")]
    MissingRequiredField { field: String, location: String },

    #[error("YAML parse error in {location}: {source}")]
    YamlParseError { location: String, source: serde_yaml::Error },

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}
```

### Key Re-exports

```rust
pub use catalog::{SkillCatalog, AsyncSkillCatalog, SkillCatalogEntry};
pub use error::SkillsError;
pub use loader::SkillsLoader;
pub use types::{Skill, SkillCatalogEntry, SkillFrontmatter};
```

## Architecture

```
┌─────────────────────────────────────────────────────────────────────┐
│                        swell-skills                                 │
├─────────────────────────────────────────────────────────────────────┤
│                                                                     │
│  ┌─────────────────────────────────────────────────────────────┐   │
│  │                    SkillsLoader                             │   │
│  │  ┌─────────────────────────────────────────────────────┐   │   │
│  │  │  scan_dirs: [.swell/skills, ~/.swell/skills]       │   │   │
│  │  │  frontmatter_cache: HashMap<String, SkillFrontmatter│   │   │
│  │  │  scan_home: bool                                    │   │   │
│  │  └─────────────────────────────────────────────────────┘   │   │
│  │                          │                                  │   │
│  │                          ▼                                  │   │
│  │  ┌─────────────────────────────────────────────────────┐   │   │
│  │  │  scan_skill_dir() → parse_skill() → discover()     │   │   │
│  │  │  - Recursive directory scan                         │   │   │
│  │  │  - SKILL.md parsing                                │   │   │
│  │  │  - Conflict detection                              │   │   │
│  │  └─────────────────────────────────────────────────────┘   │   │
│  └─────────────────────────────────────────────────────────────┘   │
│                              │                                      │
│                              ▼                                      │
│  ┌─────────────────────────────────────────────────────────────┐   │
│  │                     SkillCatalog                             │   │
│  │  ┌─────────────────────────────────────────────────────┐   │   │
│  │  │  entries: HashMap<String, SkillCatalogEntry>        │   │   │
│  │  │  - name → SkillCatalogEntry mapping                │   │   │
│  │  │  - Conflict detection on add                        │   │   │
│  │  └─────────────────────────────────────────────────────┘   │   │
│  └─────────────────────────────────────────────────────────────┘   │
│                              │                                      │
│                              ▼                                      │
│  ┌─────────────────────────────────────────────────────────────┐   │
│  │                   AsyncSkillCatalog                          │   │
│  │  - Async wrapper for runtime skill loading                 │   │
│  └─────────────────────────────────────────────────────────────┘   │
│                                                                     │
└─────────────────────────────────────────────────────────────────────┘
                           │ used by
                           ▼
              ┌────────────────────────┐
              │   swell-orchestrator    │
              │   swell-tools (SkillTool)│
              └────────────────────────┘
```

**Skill Directory Structure:**
```
.swell/skills/          # User-extensible - add skills here!
├── rust-coding/
│   ├── SKILL.md       # Required: YAML frontmatter + markdown body
│   ├── scripts/       # Optional: executable scripts
│   ├── references/    # Optional: reference documents
│   └── assets/        # Optional: images, etc.
└── test-writing/
    └── SKILL.md
```

**SKILL.md Format:**
```yaml
---
name: my-custom-skill
description: What this skill does and when to use it.
---
# Instructions in Markdown
```

**Key modules:**
- `types.rs` — `Skill`, `SkillFrontmatter`, `SkillCatalogEntry`
- `loader.rs` — `SkillsLoader` for discovering skills from filesystem
- `catalog.rs` — `SkillCatalog`, `AsyncSkillCatalog` for managing discovered skills
- `error.rs` — `SkillsError` enum with all error variants

**Concurrency:** Loader uses async I/O for filesystem operations. All types are `Send + Sync`.

## Testing

```bash
# Run tests for swell-skills
cargo test -p swell-skills -- --test-threads=4

# Run with logging
RUST_LOG=debug cargo test -p swell-skills

# Run specific test
cargo test -p swell-skills -- test_parse_valid_frontmatter --nocapture

# Run discovery tests
cargo test -p swell-skills -- discover --nocapture

# Run parser tests
cargo test -p swell-skills -- parse_frontmatter --nocapture
```

**Test structure:**
- Unit tests in `#[cfg(test)]` modules within each source file
- Tests for frontmatter parsing in `loader.rs`
- Tests for skill discovery with temp directories
- Tests for catalog operations

**Mock patterns:**
```rust
#[tokio::test]
async fn test_discover_empty_directory() {
    let temp_dir = tempfile::tempdir().unwrap();
    let loader = SkillsLoader::with_dirs(vec![temp_dir.path()]);
    let (catalog, errors) = loader.discover().await.unwrap();
    assert_eq!(catalog.len(), 0);
    assert!(errors.is_empty());
}

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
}
```

## Dependencies

```toml
# swell-skills/Cargo.toml
[dependencies]
swell-core = { path = "../swell-core", version = "0.1.0" }

# Serialization
serde = { version = "1", features = ["derive"] }
serde_yaml = "0.9"

# Async
tokio = { version = "1", features = ["fs", "sync"] }

# Logging
tracing = "0.1"

# Error handling
thiserror = "1"

# Glob for file pattern matching
glob = "0.3"

# Home directory access
dirs = "6"

[dev-dependencies]
tempfile = "3"
tokio-test = "0.4"

[features]
default = []
```

**Note:** `swell-skills` has minimal dependencies to keep it lightweight and avoid circular dependencies.
