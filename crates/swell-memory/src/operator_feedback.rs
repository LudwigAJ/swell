// operator_feedback.rs - Pattern learning from operator-provided guidance files
//
// This module provides functionality to parse and learn from CLAUDE.md and AGENTS.md
// files which contain operator-provided guidance. These patterns have HIGHER trust
// weight than agent self-learned patterns (e.g., from rejections or successful tasks).
//
// Key differences from pattern_learning:
// - ConventionSource::FromOperatorFeedback has base confidence of 0.95
// - Patterns extracted from CLAUDE.md/AGENTS.md are considered authoritative
// - Monitors file modification times for change detection and sync

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

use swell_core::SwellError;

use super::pattern_learning::{
    Convention, ConventionSource, ConventionType,
};

/// Known guidance file names that contain operator-provided patterns
const GUIDANCE_FILES: &[&str] = &["CLAUDE.md", "AGENTS.md"];

/// Base confidence for operator feedback patterns - higher than learned patterns
/// Pattern learning uses 0.4-0.7, operator feedback is authoritative at 0.95
pub const OPERATOR_FEEDBACK_BASE_CONFIDENCE: f64 = 0.95;

/// Minimum confidence for operator patterns (still higher than learned min)
pub const OPERATOR_FEEDBACK_MIN_CONFIDENCE: f64 = 0.85;

/// Represents an extracted pattern from operator guidance files
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperatorGuidancePattern {
    pub id: Uuid,
    /// Name of the pattern (extracted from heading or derived from content)
    pub name: String,
    /// Full description of the pattern
    pub description: String,
    /// Which file this pattern came from
    pub source_file: String,
    /// Source file line number for reference
    pub source_line: Option<u32>,
    /// The pattern type determines how it should be applied
    pub pattern_type: OperatorPatternType,
    /// Confidence score - higher than learned patterns
    pub confidence: f64,
    /// Keywords that indicate when this pattern is applicable
    pub context_keywords: Vec<String>,
    /// Examples extracted from the guidance
    pub examples: Vec<String>,
    /// Tags for categorization
    pub tags: Vec<String>,
    /// When this pattern was first observed
    pub created_at: DateTime<Utc>,
    /// When this pattern was last verified/synced
    pub last_synced_at: DateTime<Utc>,
    /// File modification time when last parsed
    pub source_mtime: Option<i64>,
}

impl OperatorGuidancePattern {
    /// Create a new operator guidance pattern with high default confidence
    pub fn new(name: String, pattern_type: OperatorPatternType) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            name,
            description: String::new(),
            source_file: String::new(),
            source_line: None,
            pattern_type,
            confidence: OPERATOR_FEEDBACK_BASE_CONFIDENCE,
            context_keywords: Vec::new(),
            examples: Vec::new(),
            tags: Vec::new(),
            created_at: now,
            last_synced_at: now,
            source_mtime: None,
        }
    }

    /// Check if this pattern has been modified since last sync
    pub fn is_modified(&self, current_mtime: Option<i64>) -> bool {
        match (self.source_mtime, current_mtime) {
            (Some(stored), Some(current)) => stored != current,
            _ => true, // No mtime tracking = treat as modified
        }
    }
}

/// Types of patterns that can be extracted from operator guidance
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperatorPatternType {
    /// Naming convention (file names, function names, etc.)
    Naming,
    /// Import/dependency convention
    Import,
    /// Error handling pattern
    ErrorHandling,
    /// Testing convention
    Testing,
    /// Documentation requirements
    Documentation,
    /// Code formatting/style
    Formatting,
    /// Git commit conventions
    GitCommit,
    /// Type usage conventions
    TypeUsage,
    /// General coding principle
    Principle,
    /// Project-specific workflow
    Workflow,
    /// Tool usage guidance
    ToolUsage,
    /// Architecture guidance
    Architecture,
    /// Deny-list or anti-pattern (things to avoid)
    Avoid,
}

impl OperatorPatternType {
    pub fn as_str(&self) -> &'static str {
        match self {
            OperatorPatternType::Naming => "naming",
            OperatorPatternType::Import => "import",
            OperatorPatternType::ErrorHandling => "error_handling",
            OperatorPatternType::Testing => "testing",
            OperatorPatternType::Documentation => "documentation",
            OperatorPatternType::Formatting => "formatting",
            OperatorPatternType::GitCommit => "git_commit",
            OperatorPatternType::TypeUsage => "type_usage",
            OperatorPatternType::Principle => "principle",
            OperatorPatternType::Workflow => "workflow",
            OperatorPatternType::ToolUsage => "tool_usage",
            OperatorPatternType::Architecture => "architecture",
            OperatorPatternType::Avoid => "avoid",
        }
    }

    /// Infer pattern type from keywords in content
    pub fn infer_from_content(content: &str) -> Self {
        let lower = content.to_lowercase();

        // Check for "avoid" patterns first - these are high priority
        if lower.contains("avoid") || lower.contains("don't") || lower.contains("do not")
            || lower.contains("never") || lower.contains("forbidden") || lower.contains("don't")
        {
            return OperatorPatternType::Avoid;
        }

        if lower.contains("naming") || lower.contains("file name") || lower.contains("function name") {
            OperatorPatternType::Naming
        } else if lower.contains("import") || lower.contains("dependency") {
            OperatorPatternType::Import
        } else if lower.contains("error") || lower.contains("exception") || lower.contains("panic") {
            OperatorPatternType::ErrorHandling
        } else if lower.contains("test") || lower.contains("spec") || lower.contains("mock") {
            OperatorPatternType::Testing
        } else if lower.contains("document") || lower.contains("comment") || lower.contains("readme") {
            OperatorPatternType::Documentation
        } else if lower.contains("format") || lower.contains("style") || lower.contains("fmt") {
            OperatorPatternType::Formatting
        } else if lower.contains("commit") || lower.contains("branch") {
            OperatorPatternType::GitCommit
        } else if lower.contains("type") || lower.contains("trait") || lower.contains("struct") {
            OperatorPatternType::TypeUsage
        } else if lower.contains("workflow") || lower.contains("process") || lower.contains("step") {
            OperatorPatternType::Workflow
        } else if lower.contains("architecture") || lower.contains("design") || lower.contains("structure") {
            OperatorPatternType::Architecture
        } else {
            OperatorPatternType::Principle
        }
    }

    /// Map to ConventionType for unified convention storage
    pub fn to_convention_type(&self) -> ConventionType {
        match self {
            OperatorPatternType::Naming => ConventionType::Naming,
            OperatorPatternType::Import => ConventionType::Import,
            OperatorPatternType::ErrorHandling => ConventionType::ErrorHandling,
            OperatorPatternType::Testing => ConventionType::Testing,
            OperatorPatternType::Documentation => ConventionType::Documentation,
            OperatorPatternType::Formatting => ConventionType::Formatting,
            OperatorPatternType::GitCommit => ConventionType::GitCommit,
            OperatorPatternType::TypeUsage => ConventionType::TypeUsage,
            OperatorPatternType::Principle | OperatorPatternType::Workflow | OperatorPatternType::ToolUsage | OperatorPatternType::Architecture => ConventionType::CodeStructure,
            OperatorPatternType::Avoid => ConventionType::CodeStructure,
        }
    }
}

/// Result of parsing operator guidance files
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperatorFeedbackResult {
    pub patterns_extracted: usize,
    pub files_parsed: usize,
    pub patterns: Vec<OperatorGuidancePattern>,
    pub errors: Vec<String>,
    /// Mtime tracking for change detection
    pub file_mtimes: HashMap<String, Option<i64>>,
}

/// Configuration for operator feedback parsing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperatorFeedbackConfig {
    /// Base confidence for operator patterns (default: 0.95)
    pub base_confidence: f64,
    /// Minimum confidence for operator patterns
    pub min_confidence: f64,
    /// Whether to enable file watching for sync
    pub enable_file_watching: bool,
    /// Paths to scan for guidance files (defaults to project root)
    pub scan_paths: Vec<String>,
    /// Additional guidance files to parse beyond CLAUDE.md/AGENTS.md
    pub additional_files: Vec<String>,
}

impl Default for OperatorFeedbackConfig {
    fn default() -> Self {
        Self {
            base_confidence: OPERATOR_FEEDBACK_BASE_CONFIDENCE,
            min_confidence: OPERATOR_FEEDBACK_MIN_CONFIDENCE,
            enable_file_watching: true,
            scan_paths: vec![String::new()], // Project root by default
            additional_files: Vec::new(),
        }
    }
}

/// Parser for operator guidance files
pub struct OperatorFeedbackParser {
    config: OperatorFeedbackConfig,
}

impl OperatorFeedbackParser {
    pub fn new(config: OperatorFeedbackConfig) -> Self {
        Self { config }
    }

    pub fn with_default_config() -> Self {
        Self {
            config: OperatorFeedbackConfig::default(),
        }
    }

    /// Parse all guidance files in the given directory
    pub fn parse_directory(&self, dir_path: &Path) -> OperatorFeedbackResult {
        let mut all_patterns = Vec::new();
        let mut file_mtimes: HashMap<String, Option<i64>> = HashMap::new();
        let mut errors = Vec::new();
        let mut files_parsed = 0;

        // Parse standard guidance files
        for &filename in GUIDANCE_FILES {
            let file_path = dir_path.join(filename);
            if file_path.exists() {
                match self.parse_file(&file_path) {
                    Ok(patterns) => {
                        files_parsed += 1;
                        let mtime = fs::metadata(&file_path).ok().and_then(|m| m.modified().ok())
                            .map(|t| t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() as i64);
                        file_mtimes.insert(filename.to_string(), mtime);
                        all_patterns.extend(patterns);
                    }
                    Err(e) => {
                        errors.push(format!("Failed to parse {}: {}", filename, e));
                    }
                }
            }
        }

        // Parse additional files
        for filename in &self.config.additional_files {
            let file_path = dir_path.join(filename);
            if file_path.exists() {
                match self.parse_file(&file_path) {
                    Ok(patterns) => {
                        files_parsed += 1;
                        let mtime = fs::metadata(&file_path).ok().and_then(|m| m.modified().ok())
                            .map(|t| t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() as i64);
                        file_mtimes.insert(filename.clone(), mtime);
                        all_patterns.extend(patterns);
                    }
                    Err(e) => {
                        errors.push(format!("Failed to parse additional file {}: {}", filename, e));
                    }
                }
            }
        }

        // Deduplicate patterns by name (keep first occurrence)
        all_patterns = self.deduplicate_patterns(all_patterns);

        OperatorFeedbackResult {
            patterns_extracted: all_patterns.len(),
            files_parsed,
            patterns: all_patterns,
            errors,
            file_mtimes,
        }
    }

    /// Parse a single guidance file
    pub fn parse_file(&self, file_path: &Path) -> Result<Vec<OperatorGuidancePattern>, SwellError> {
        use std::io::Read;
        
        let mut content = String::new();
        let mut file = std::fs::File::open(file_path)?;
        file.read_to_string(&mut content)?;
        
        let filename = file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();

        self.parse_content(&content, &filename)
    }

    /// Parse content from a guidance file
    pub fn parse_content(&self, content: &str, filename: &str) -> Result<Vec<OperatorGuidancePattern>, SwellError> {
        let mut patterns = Vec::new();
        let mut current_heading: Option<String> = None;
        let mut current_block: Vec<String> = Vec::new();

        for (line_num, line) in content.lines().enumerate() {
            // Detect headings (# ## ### etc.)
            if let Some((_level, heading)) = self.parse_heading(line) {
                // Flush previous block if exists
                if let Some(heading) = current_heading.take() {
                    if !current_block.is_empty() {
                        let block_content = current_block.join("\n");
                        if let Some(pattern) = self.extract_pattern_from_block(
                            &heading,
                            &block_content,
                            filename,
                            line_num as u32 - current_block.len() as u32,
                        ) {
                            patterns.push(pattern);
                        }
                    }
                }
                current_heading = Some(heading);
                current_block.clear();
            } else {
                current_block.push(line.to_string());
            }
        }

        // Flush last block
        if let Some(heading) = current_heading {
            if !current_block.is_empty() {
                let block_content = current_block.join("\n");
                if let Some(pattern) = self.extract_pattern_from_block(
                    &heading,
                    &block_content,
                    filename,
                    content.lines().count() as u32 - current_block.len() as u32,
                ) {
                    patterns.push(pattern);
                }
            }
        }

        Ok(patterns)
    }

    /// Parse a heading line and return (level, text)
    fn parse_heading(&self, line: &str) -> Option<(u32, String)> {
        let trimmed = line.trim_start_matches('#').trim();
        // Only proceed if we have at least one # and content after #
        if line.starts_with('#') && (trimmed.is_empty() || !trimmed.is_empty()) {
            let level = (line.len() - line.trim_start_matches('#').len()) as u32;
            // Normalize heading text - lowercase and replace spaces with underscores
            let text = if trimmed.is_empty() {
                String::new()
            } else {
                trimmed
                    .to_lowercase()
                    .replace(' ', "_")
                    .chars()
                    .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
                    .collect::<String>()
                    .trim_matches('_')
                    .to_string()
            };
            return Some((level, text));
        }
        None
    }

    /// Extract a pattern from a heading + block combination
    fn extract_pattern_from_block(
        &self,
        heading: &str,
        block: &str,
        filename: &str,
        start_line: u32,
    ) -> Option<OperatorGuidancePattern> {
        let trimmed_block = block.trim();
        if trimmed_block.is_empty() || trimmed_block.len() < 10 {
            return None;
        }

        // Skip very short blocks that likely aren't patterns
        let word_count = trimmed_block.split_whitespace().count();
        if word_count < 3 {
            return None;
        }

        let pattern_type = OperatorPatternType::infer_from_content(trimmed_block);

        // Generate name from heading
        let name = self.generate_pattern_name(heading, &pattern_type);

        // Skip if name is too generic
        if name.len() < 3 {
            return None;
        }

        // Extract keywords from content
        let keywords = self.extract_keywords(trimmed_block);

        // Extract examples from block (lines starting with - or numbered lists)
        let examples = self.extract_examples(trimmed_block);

        // Extract tags from heading
        let tags = self.extract_tags(heading);

        Some(OperatorGuidancePattern {
            id: Uuid::new_v4(),
            name,
            description: trimmed_block.to_string(),
            source_file: filename.to_string(),
            source_line: Some(start_line),
            pattern_type,
            confidence: self.config.base_confidence,
            context_keywords: keywords,
            examples,
            tags,
            created_at: Utc::now(),
            last_synced_at: Utc::now(),
            source_mtime: None,
        })
    }

    /// Generate a pattern name from heading
    fn generate_pattern_name(&self, heading: &str, pattern_type: &OperatorPatternType) -> String {
        // Clean up heading into a usable name
        let cleaned = heading
            .replace(|c: char| c.is_whitespace() || (!c.is_alphanumeric() && c != '-'), "_")
            .trim_matches('_')
            .to_string();

        if cleaned.is_empty() {
            format!("pattern_{}", pattern_type.as_str())
        } else {
            cleaned
        }
    }

    /// Extract keywords from content
    fn extract_keywords(&self, content: &str) -> Vec<String> {
        let content_lower = content.to_lowercase();
        let mut keywords = Vec::new();

        // Extract code-related keywords
        let code_terms = [
            "rust", "python", "javascript", "typescript", "cargo", "npm", "git",
            "test", "mock", "function", "struct", "trait", "impl", "async",
            "error", "panic", "result", "option", "vec", "string", "str",
            "clippy", "fmt", "build", "run", "execute", "shell", "file",
        ];

        for term in &code_terms {
            if content_lower.contains(term) {
                keywords.push(term.to_string());
            }
        }

        // Extract words that appear multiple times (likely important)
        let words: Vec<&str> = content_lower.split_whitespace().collect();
        let mut word_freq: HashMap<&str, usize> = HashMap::new();
        for word in &words {
            if word.len() > 4 {
                *word_freq.entry(word).or_insert(0) += 1;
            }
        }

        // Add words that appear 2+ times
        for (word, freq) in word_freq {
            if freq >= 2 && keywords.len() < 10 {
                keywords.push(word.to_string());
            }
        }

        keywords
    }

    /// Extract examples from block content
    fn extract_examples(&self, content: &str) -> Vec<String> {
        let mut examples = Vec::new();

        for line in content.lines() {
            let trimmed = line.trim();
            // Lines starting with - or numbers followed by . are examples
            if trimmed.starts_with('-') || trimmed.starts_with(|c: char| c.is_ascii_digit()) {
                // Extract the example text
                let example = trimmed
                    .trim_start_matches(|c: char| c == '-' || c.is_ascii_digit() || c == '.' || c == ' ')
                    .trim();

                // Skip very short examples
                if example.len() > 5 {
                    examples.push(example.to_string());
                }
            }

            // Also capture inline code examples (between backticks)
            if let Some(start) = trimmed.find('`') {
                if let Some(end) = trimmed[start + 1..].find('`') {
                    let code = &trimmed[start + 1..start + 1 + end];
                    if code.len() > 2 && code.len() < 100 {
                        examples.push(code.to_string());
                    }
                }
            }
        }

        // Limit examples
        examples.truncate(5);
        examples
    }

    /// Extract tags from heading
    fn extract_tags(&self, heading: &str) -> Vec<String> {
        let mut tags = Vec::new();

        let heading_lower = heading.to_lowercase();
        let tag_keywords = [
            ("naming", "naming"),
            ("format", "formatting"),
            ("style", "formatting"),
            ("error", "error-handling"),
            ("test", "testing"),
            ("import", "imports"),
            ("commit", "git"),
            ("cargo", "rust"),
            ("rust", "rust"),
            ("build", "build"),
            ("run", "execution"),
            ("execute", "execution"),
            ("shell", "tool-usage"),
        ];

        for (keyword, tag) in &tag_keywords {
            if heading_lower.contains(keyword) {
                tags.push(tag.to_string());
            }
        }

        tags
    }

    /// Deduplicate patterns by name
    fn deduplicate_patterns(&self, mut patterns: Vec<OperatorGuidancePattern>) -> Vec<OperatorGuidancePattern> {
        let mut seen: HashMap<String, usize> = HashMap::new();
        patterns.retain(|p| {
            let entry = seen.entry(p.name.clone()).or_insert(0);
            *entry += 1;
            *entry == 1 // Keep only first occurrence
        });
        patterns
    }

    /// Convert operator patterns to conventions for unified storage
    pub fn patterns_to_conventions(&self, patterns: &[OperatorGuidancePattern]) -> Vec<Convention> {
        patterns
            .iter()
            .map(|p| Convention {
                id: Uuid::new_v4(),
                name: p.name.clone(),
                description: p.description.clone(),
                convention_type: p.pattern_type.to_convention_type(),
                pattern: p.examples.first().cloned().unwrap_or_default(),
                examples: p.examples.clone(),
                source: ConventionSource::FromOperatorFeedback,
                confidence: p.confidence,
                created_at: p.created_at,
            })
            .collect()
    }
}

/// Service for managing operator feedback patterns with file watching
pub struct OperatorFeedbackService {
    parser: OperatorFeedbackParser,
    patterns: Arc<RwLock<Vec<OperatorGuidancePattern>>>,
    file_mtimes: Arc<RwLock<HashMap<String, Option<i64>>>>,
    config: OperatorFeedbackConfig,
}

impl OperatorFeedbackService {
    pub fn new(config: OperatorFeedbackConfig) -> Self {
        let parser = OperatorFeedbackParser::new(config.clone());
        Self {
            parser,
            patterns: Arc::new(RwLock::new(Vec::new())),
            file_mtimes: Arc::new(RwLock::new(HashMap::new())),
            config,
        }
    }

    pub fn with_default_config() -> Self {
        Self {
            parser: OperatorFeedbackParser::with_default_config(),
            patterns: Arc::new(RwLock::new(Vec::new())),
            file_mtimes: Arc::new(RwLock::new(HashMap::new())),
            config: OperatorFeedbackConfig::default(),
        }
    }

    /// Load and parse guidance files from a directory
    pub async fn load_from_directory(&self, dir_path: &Path) -> Result<OperatorFeedbackResult, SwellError> {
        let result = self.parser.parse_directory(dir_path);

        if result.errors.is_empty() || result.patterns_extracted > 0 {
            // Update patterns
            let _conventions = self.parser.patterns_to_conventions(&result.patterns);
            let mut patterns = self.patterns.write().await;
            *patterns = result.patterns.clone();

            // Update mtimes
            let mut mtimes = self.file_mtimes.write().await;
            *mtimes = result.file_mtimes.clone();
        }

        Ok(result)
    }

    /// Get all loaded patterns
    pub async fn get_patterns(&self) -> Vec<OperatorGuidancePattern> {
        self.patterns.read().await.clone()
    }

    /// Get patterns filtered by type
    pub async fn get_patterns_by_type(&self, pattern_type: OperatorPatternType) -> Vec<OperatorGuidancePattern> {
        self.patterns
            .read()
            .await
            .iter()
            .filter(|p| p.pattern_type == pattern_type)
            .cloned()
            .collect()
    }

    /// Get patterns matching keywords
    pub async fn get_patterns_by_keywords(&self, keywords: &[String]) -> Vec<OperatorGuidancePattern> {
        let patterns = self.patterns.read().await;
        keywords
            .iter()
            .flat_map(|kw| {
                let kw_lower = kw.to_lowercase();
                patterns
                    .iter()
                    .filter(move |p| {
                        p.name.to_lowercase().contains(&kw_lower)
                            || p.context_keywords.iter().any(|k| k.to_lowercase().contains(&kw_lower))
                    })
                    .cloned()
            })
            .collect()
    }

    /// Check if any guidance files have changed and resync if needed
    pub async fn check_and_sync(&self, dir_path: &Path) -> Result<bool, SwellError> {
        let current_mtimes = self.get_current_mtimes(dir_path)?;
        let stored_mtimes = self.file_mtimes.read().await.clone();

        // Check if any file has changed
        let mut has_changes = false;
        for (filename, current_mtime) in &current_mtimes {
            if let Some(stored_mtime) = stored_mtimes.get(filename) {
                if stored_mtime != current_mtime {
                    has_changes = true;
                    break;
                }
            } else {
                // New file detected
                has_changes = true;
                break;
            }
        }

        if has_changes {
            self.load_from_directory(dir_path).await?;
            return Ok(true);
        }

        Ok(false)
    }

    /// Get current modification times for guidance files
    fn get_current_mtimes(&self, dir_path: &Path) -> Result<HashMap<String, Option<i64>>, SwellError> {
        let mut mtimes = HashMap::new();

        for &filename in GUIDANCE_FILES {
            let file_path = dir_path.join(filename);
            if file_path.exists() {
                let mtime = fs::metadata(&file_path)
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .map(|t| t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() as i64);
                mtimes.insert(filename.to_string(), mtime);
            }
        }

        for filename in &self.config.additional_files {
            let file_path = dir_path.join(filename);
            if file_path.exists() {
                let mtime = fs::metadata(&file_path)
                    .ok()
                    .and_then(|m| m.modified().ok())
                    .map(|t| t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs() as i64);
                mtimes.insert(filename.clone(), mtime);
            }
        }

        Ok(mtimes)
    }

    /// Get all patterns as conventions with higher trust weight
    pub async fn get_conventions(&self) -> Vec<Convention> {
        let patterns = self.patterns.read().await;
        self.parser.patterns_to_conventions(patterns.as_slice())
    }

    /// Get patterns with higher confidence than threshold
    pub async fn get_high_confidence_patterns(&self, min_confidence: f64) -> Vec<OperatorGuidancePattern> {
        self.patterns
            .read()
            .await
            .iter()
            .filter(|p| p.confidence >= min_confidence)
            .cloned()
            .collect()
    }

    /// Update a single pattern's last synced time
    pub async fn mark_synced(&self, pattern_id: Uuid) -> Result<(), SwellError> {
        let mut patterns = self.patterns.write().await;
        if let Some(pattern) = patterns.iter_mut().find(|p| p.id == pattern_id) {
            pattern.last_synced_at = Utc::now();
            Ok(())
        } else {
            Err(SwellError::InvalidOperation(format!("Pattern not found: {}", pattern_id)))
        }
    }

    /// Clear all loaded patterns (e.g., on project change)
    pub async fn clear(&self) {
        self.patterns.write().await.clear();
        self.file_mtimes.write().await.clear();
    }
}

/// Create a convention from an operator guidance pattern
impl From<OperatorGuidancePattern> for Convention {
    fn from(pattern: OperatorGuidancePattern) -> Self {
        Convention {
            id: Uuid::new_v4(),
            name: pattern.name,
            description: pattern.description,
            convention_type: pattern.pattern_type.to_convention_type(),
            pattern: pattern.examples.first().cloned().unwrap_or_default(),
            examples: pattern.examples,
            source: ConventionSource::FromOperatorFeedback,
            confidence: pattern.confidence,
            created_at: pattern.created_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_operator_pattern_type_infer_naming() {
        let pattern = OperatorPatternType::infer_from_content("File naming convention: use snake_case");
        assert_eq!(pattern, OperatorPatternType::Naming);
    }

    #[test]
    fn test_operator_pattern_type_infer_testing() {
        let pattern = OperatorPatternType::infer_from_content("Run tests before submitting: cargo test");
        assert_eq!(pattern, OperatorPatternType::Testing);
    }

    #[test]
    fn test_operator_pattern_type_infer_avoid() {
        let pattern = OperatorPatternType::infer_from_content("Never use unwrap() in production code");
        assert_eq!(pattern, OperatorPatternType::Avoid);
    }

    #[test]
    fn test_operator_pattern_type_as_str() {
        assert_eq!(OperatorPatternType::Naming.as_str(), "naming");
        assert_eq!(OperatorPatternType::ErrorHandling.as_str(), "error_handling");
        assert_eq!(OperatorPatternType::Avoid.as_str(), "avoid");
    }

    #[test]
    fn test_operator_guidance_pattern_creation() {
        let pattern = OperatorGuidancePattern::new(
            "snake_case_naming".to_string(),
            OperatorPatternType::Naming,
        );
        assert_eq!(pattern.name, "snake_case_naming");
        assert_eq!(pattern.confidence, OPERATOR_FEEDBACK_BASE_CONFIDENCE);
        assert!(pattern.description.is_empty());
    }

    #[test]
    fn test_operator_feedback_config_default() {
        let config = OperatorFeedbackConfig::default();
        assert_eq!(config.base_confidence, OPERATOR_FEEDBACK_BASE_CONFIDENCE);
        assert!(config.enable_file_watching);
    }

    #[test]
    fn test_parse_heading() {
        let parser = OperatorFeedbackParser::with_default_config();
        
        assert_eq!(parser.parse_heading("## Naming Conventions"), Some((2, "naming_conventions".to_string())));
        assert_eq!(parser.parse_heading("# Testing Guidelines"), Some((1, "testing_guidelines".to_string())));
        assert_eq!(parser.parse_heading("### Error Handling"), Some((3, "error_handling".to_string())));
        assert_eq!(parser.parse_heading("Not a heading"), None);
        // A bare # returns Some with empty text
        assert_eq!(parser.parse_heading("#"), Some((1, String::new())));
    }

    #[test]
    fn test_parse_content_simple() {
        let parser = OperatorFeedbackParser::with_default_config();
        let content = r#"
# Naming Conventions

Use snake_case for file names and function names.

## Examples

- my_file.rs
- my_function()
"#;
        
        let result = parser.parse_content(content, "CLAUDE.md").unwrap();
        assert!(!result.is_empty());
        
        // Should have at least one pattern from "Naming Conventions"
        let naming_patterns: Vec<_> = result.iter()
            .filter(|p| p.pattern_type == OperatorPatternType::Naming)
            .collect();
        assert!(!naming_patterns.is_empty());
    }

    #[test]
    fn test_extract_keywords() {
        let parser = OperatorFeedbackParser::with_default_config();
        let keywords = parser.extract_keywords("Run cargo test and cargo clippy before submitting");
        
        assert!(keywords.contains(&"cargo".to_string()));
        assert!(keywords.contains(&"test".to_string()));
        assert!(keywords.contains(&"clippy".to_string()));
    }

    #[test]
    fn test_extract_examples() {
        let parser = OperatorFeedbackParser::with_default_config();
        let examples = parser.extract_examples(r#"
- cargo test --workspace
- cargo clippy -- -D warnings
- `rustfmt` for formatting
"#);
        
        assert!(examples.iter().any(|e| e.contains("cargo test")));
        assert!(examples.iter().any(|e| e.contains("cargo clippy")));
        assert!(examples.iter().any(|e| e.contains("rustfmt")));
    }

    #[test]
    fn test_pattern_type_to_convention_type() {
        assert_eq!(OperatorPatternType::Naming.to_convention_type(), ConventionType::Naming);
        assert_eq!(OperatorPatternType::Testing.to_convention_type(), ConventionType::Testing);
        assert_eq!(OperatorPatternType::Avoid.to_convention_type(), ConventionType::CodeStructure);
    }

    #[test]
    fn test_operator_feedback_parser_deduplication() {
        let parser = OperatorFeedbackParser::with_default_config();
        
        let mut patterns = Vec::new();
        for i in 0..3 {
            let mut p = OperatorGuidancePattern::new(
                format!("test_pattern_{}", i % 2), // Duplicate names
                OperatorPatternType::Principle,
            );
            p.description = format!("Description {}", i);
            patterns.push(p);
        }
        
        let deduped = parser.deduplicate_patterns(patterns);
        // Should have 2 unique patterns (test_pattern_0 and test_pattern_1)
        assert_eq!(deduped.len(), 2);
    }

    #[tokio::test]
    async fn test_operator_feedback_service_load_from_directory() {
        // Create temp directory with CLAUDE.md
        let temp_dir = std::env::temp_dir();
        let test_dir = temp_dir.join(format!("swell_test_{}", Uuid::new_v4()));
        std::fs::create_dir_all(&test_dir).unwrap();
        
        let claude_content = r#"
# Test Guidance

## Naming

Use snake_case for file names.

## Testing

Run `cargo test` before submitting.
"#;
        
        std::fs::write(test_dir.join("CLAUDE.md"), claude_content).unwrap();
        
        let service = OperatorFeedbackService::with_default_config();
        let result = service.load_from_directory(&test_dir).await.unwrap();
        
        assert!(result.patterns_extracted > 0);
        assert!(result.files_parsed >= 1);
        assert!(result.errors.is_empty());
        
        // Cleanup
        std::fs::remove_dir_all(test_dir).ok();
    }

    #[tokio::test]
    async fn test_operator_feedback_service_get_patterns() {
        let service = OperatorFeedbackService::with_default_config();
        
        // Initially empty
        let patterns = service.get_patterns().await;
        assert!(patterns.is_empty());
    }

    #[tokio::test]
    async fn test_operator_feedback_service_conventions() {
        let temp_dir = std::env::temp_dir();
        let test_dir = temp_dir.join(format!("swell_test_{}", Uuid::new_v4()));
        std::fs::create_dir_all(&test_dir).unwrap();
        
        let content = r#"
# Rust Conventions

## Naming

Use snake_case for functions.

## Testing

Run `cargo test` before committing.
"#;
        
        std::fs::write(test_dir.join("CLAUDE.md"), content).unwrap();
        
        let service = OperatorFeedbackService::with_default_config();
        service.load_from_directory(&test_dir).await.unwrap();
        
        let conventions = service.get_conventions().await;
        assert!(!conventions.is_empty());
        
        // All conventions should have FromOperatorFeedback source
        for conv in &conventions {
            assert_eq!(conv.source, ConventionSource::FromOperatorFeedback);
            // Operator feedback should have high confidence
            assert!(conv.confidence >= OPERATOR_FEEDBACK_MIN_CONFIDENCE);
        }
        
        // Cleanup
        std::fs::remove_dir_all(test_dir).ok();
    }

    #[test]
    fn test_operator_feedback_result_serialization() {
        let result = OperatorFeedbackResult {
            patterns_extracted: 5,
            files_parsed: 2,
            patterns: Vec::new(),
            errors: vec!["error1".to_string()],
            file_mtimes: HashMap::new(),
        };
        
        let json = serde_json::to_string(&result).unwrap();
        let deserialized: OperatorFeedbackResult = serde_json::from_str(&json).unwrap();
        
        assert_eq!(deserialized.patterns_extracted, 5);
        assert_eq!(deserialized.files_parsed, 2);
        assert_eq!(deserialized.errors.len(), 1);
    }

    #[test]
    fn test_convention_source_operator_feedback() {
        let source = ConventionSource::FromOperatorFeedback;
        let json = serde_json::to_string(&source).unwrap();
        assert_eq!(json, "\"from_operator_feedback\"");
        
        let deserialized: ConventionSource = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, ConventionSource::FromOperatorFeedback);
    }

    #[test]
    fn test_avoid_pattern_high_confidence() {
        let pattern = OperatorGuidancePattern::new(
            "never_use_unwrap".to_string(),
            OperatorPatternType::Avoid,
        );
        assert!(pattern.confidence >= OPERATOR_FEEDBACK_MIN_CONFIDENCE);
    }

    #[tokio::test]
    async fn test_get_patterns_by_type() {
        let temp_dir = std::env::temp_dir();
        let test_dir = temp_dir.join(format!("swell_test_{}", Uuid::new_v4()));
        std::fs::create_dir_all(&test_dir).unwrap();
        
        let content = r#"
# Project Guidelines

## Naming Convention
Use snake_case for file names and function names. This is the standard convention in the project.

## Testing
Always run cargo test before submitting. Make sure all tests pass.

## Error Handling
Use Result types for error handling. Never use unwrap in production code.
"#;
        
        std::fs::write(test_dir.join("CLAUDE.md"), content).unwrap();
        
        let service = OperatorFeedbackService::with_default_config();
        service.load_from_directory(&test_dir).await.unwrap();
        
        let testing_patterns = service.get_patterns_by_type(OperatorPatternType::Testing).await;
        assert!(!testing_patterns.is_empty(), "Expected at least one testing pattern");
        
        let naming_patterns = service.get_patterns_by_type(OperatorPatternType::Naming).await;
        assert!(!naming_patterns.is_empty(), "Expected at least one naming pattern");
        
        // Cleanup
        std::fs::remove_dir_all(test_dir).ok();
    }
}
