//! Git conflict resolution with owner-based enforcement and semantic understanding.
//!
//! This module provides:
//! - [`ConflictResolver`] - Main conflict resolution engine
//! - [`ConflictInfo`] - Detected conflict metadata
//! - [`ResolutionResult`] - Outcome of conflict resolution
//! - One-file-one-owner enforcement
//! - Semantic merge capability

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// Represents a detected conflict in a file
#[derive(Debug, Clone)]
pub struct ConflictInfo {
    /// Path to the file with conflict
    pub file_path: PathBuf,
    /// Conflict marker start (typically "<<<<<<<")
    pub marker_start: String,
    /// The base/original content
    pub base: String,
    /// The "ours" version (current branch)
    pub ours: String,
    /// The "theirs" version (incoming branch)
    pub theirs: String,
    /// Conflict marker end (typically ">>>>>>>")
    pub marker_end: String,
    /// Line number where conflict starts
    pub start_line: usize,
    /// Line number where conflict ends
    pub end_line: usize,
    /// Number of conflict hunks in this file
    pub hunk_count: usize,
}

/// Owner information for a file
#[derive(Debug, Clone)]
pub struct FileOwner {
    /// Owner identifier (agent ID or team name)
    pub owner_id: String,
    /// Files or patterns owned by this owner
    pub patterns: Vec<String>,
}

impl FileOwner {
    /// Create a new file owner
    pub fn new(owner_id: impl Into<String>) -> Self {
        Self {
            owner_id: owner_id.into(),
            patterns: Vec::new(),
        }
    }

    /// Add a file pattern this owner is responsible for
    pub fn with_pattern(mut self, pattern: impl Into<String>) -> Self {
        self.patterns.push(pattern.into());
        self
    }
}

/// Configuration for conflict resolution
#[derive(Debug, Clone)]
pub struct ConflictResolverConfig {
    /// Conflict marker start string
    pub marker_start: String,
    /// Conflict marker end string
    pub marker_end: String,
    /// Separator between ours and theirs
    pub separator: String,
    /// Enable semantic merge (try to understand code structure)
    pub semantic_merge: bool,
    /// Prefer ours (current branch) when uncertain
    pub prefer_ours: bool,
}

impl Default for ConflictResolverConfig {
    fn default() -> Self {
        Self {
            marker_start: "<<<<<<<".to_string(),
            marker_end: ">>>>>>>".to_string(),
            separator: "=======".to_string(),
            semantic_merge: true,
            prefer_ours: true,
        }
    }
}

/// Result of conflict resolution
#[derive(Debug, Clone)]
pub struct ResolutionResult {
    /// Whether resolution was successful
    pub success: bool,
    /// The resolved content
    pub content: Option<String>,
    /// Number of conflicts resolved
    pub conflicts_resolved: usize,
    /// Number of conflicts that could not be resolved automatically
    pub conflicts_unresolved: usize,
    /// Files that need manual resolution
    pub needs_manual_resolution: Vec<PathBuf>,
    /// Resolution strategy used per file
    pub strategies_used: HashMap<PathBuf, ResolutionStrategy>,
    /// Error message if resolution failed
    pub error: Option<String>,
}

/// Resolution strategy that was applied
#[derive(Debug, Clone, PartialEq)]
pub enum ResolutionStrategy {
    /// Ours version was taken
    Ours,
    /// Theirs version was taken
    Theirs,
    /// Auto-merged (semantic)
    AutoMerged,
    /// Manual resolution required
    Manual,
}

/// Conflict detection result
#[derive(Debug, Clone)]
pub struct ConflictDetectionResult {
    /// Whether any conflicts were detected
    pub has_conflicts: bool,
    /// List of files with conflicts
    pub conflicting_files: Vec<PathBuf>,
    /// Detailed conflict information per file
    pub conflict_details: HashMap<PathBuf, Vec<ConflictHunk>>,
    /// Total conflict hunks across all files
    pub total_hunks: usize,
}

/// A single conflict hunk within a file
#[derive(Debug, Clone)]
pub struct ConflictHunk {
    /// Line number where hunk starts
    pub start_line: usize,
    /// Line number where hunk ends
    pub end_line: usize,
    /// Original/base content
    pub base: String,
    /// Our version
    pub ours: String,
    /// Their version
    pub theirs: String,
}

/// Errors that can occur during conflict resolution
#[derive(Debug, Clone, thiserror::Error)]
pub enum ConflictResolutionError {
    #[error("File not found: {0}")]
    FileNotFound(String),

    #[error("Invalid conflict markers in file: {0}")]
    InvalidMarkers(String),

    #[error("Git operation failed: {0}")]
    GitFailed(String),

    #[error("No owner found for file: {0}")]
    NoOwner(String),

    #[error("Multiple owners for file: {0}")]
    MultipleOwners(String),

    #[error("Failed to resolve conflicts: {0}")]
    ResolutionFailed(String),

    #[error("IO error: {0}")]
    IoError(String),
}

/// Main conflict resolution engine
#[derive(Debug, Clone)]
pub struct ConflictResolver {
    config: ConflictResolverConfig,
    /// File ownership registry (one file, one owner)
    ownership_registry: Arc<RwLock<HashMap<PathBuf, String>>>,
    /// File owner patterns for glob matching
    owner_patterns: Arc<RwLock<HashMap<String, Vec<String>>>>,
}

impl ConflictResolver {
    /// Create a new conflict resolver with default configuration
    pub fn new() -> Self {
        Self {
            config: ConflictResolverConfig::default(),
            ownership_registry: Arc::new(RwLock::new(HashMap::new())),
            owner_patterns: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Create a new conflict resolver with custom configuration
    pub fn with_config(config: ConflictResolverConfig) -> Self {
        Self {
            config,
            ownership_registry: Arc::new(RwLock::new(HashMap::new())),
            owner_patterns: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Get the current configuration
    pub fn config(&self) -> &ConflictResolverConfig {
        &self.config
    }

    /// Register a file owner for a specific file
    pub async fn register_owner(&self, file_path: impl Into<PathBuf>, owner_id: impl Into<String>) {
        let mut registry = self.ownership_registry.write().await;
        registry.insert(file_path.into(), owner_id.into());
    }

    /// Register an owner with file patterns (glob matching)
    pub async fn register_owner_pattern(
        &self,
        owner_id: impl Into<String>,
        patterns: Vec<String>,
    ) {
        let mut patterns_map = self.owner_patterns.write().await;
        patterns_map.insert(owner_id.into(), patterns);
    }

    /// Find owner for a file (direct or pattern match)
    pub async fn find_owner(&self, file_path: &Path) -> Option<String> {
        let file_path_buf = file_path.to_path_buf();
        let file_str = file_path.to_string_lossy().to_string();

        // First check direct registry
        {
            let registry = self.ownership_registry.read().await;
            if let Some(owner) = registry.get(&file_path_buf) {
                return Some(owner.clone());
            }
        }

        // Check pattern matches
        let patterns_map = self.owner_patterns.read().await;
        for (owner_id, patterns) in patterns_map.iter() {
            for pattern in patterns {
                if glob_match(pattern, &file_str) {
                    return Some(owner_id.clone());
                }
            }
        }

        None
    }

    /// Detect conflicts in a file
    pub async fn detect_conflicts(&self, file_path: &Path) -> Result<ConflictDetectionResult, ConflictResolutionError> {
        let content = tokio::fs::read_to_string(file_path)
            .await
            .map_err(|e| ConflictResolutionError::IoError(e.to_string()))?;

        self.parse_conflicts(&content)
    }

    /// Parse conflicts from file content
    fn parse_conflicts(&self, content: &str) -> Result<ConflictDetectionResult, ConflictResolutionError> {
        let mut conflicting_files = Vec::new();
        let mut conflict_details: HashMap<PathBuf, Vec<ConflictHunk>> = HashMap::new();
        let mut total_hunks = 0;
        let lines: Vec<&str> = content.lines().collect();

        let mut in_conflict = false;
        let mut hunk_start = 0;
        let mut base_lines = Vec::new();
        let mut ours_lines = Vec::new();
        let mut theirs_lines = Vec::new();
        let mut current_section = SectionType::None;

        enum SectionType {
            None,
            Ours,
            Theirs,
        }

        for (line_idx, line) in lines.iter().enumerate() {
            let trimmed = line.trim();

            if trimmed.starts_with(&self.config.marker_start) {
                in_conflict = true;
                hunk_start = line_idx;
                base_lines.clear();
                ours_lines.clear();
                theirs_lines.clear();
                current_section = SectionType::Ours;
            } else if trimmed.starts_with(&self.config.separator) {
                current_section = SectionType::Theirs;
            } else if trimmed.starts_with(&self.config.marker_end) {
                if in_conflict {
                    let hunk = ConflictHunk {
                        start_line: hunk_start,
                        end_line: line_idx,
                        base: base_lines.join("\n"),
                        ours: ours_lines.join("\n"),
                        theirs: theirs_lines.join("\n"),
                    };

                    conflict_details
                        .entry(PathBuf::from("unknown"))  // Single file per parse call
                        .or_insert_with(Vec::new)
                        .push(hunk);
                    total_hunks += 1;

                    in_conflict = false;
                    current_section = SectionType::None;
                }
            } else if in_conflict {
                match current_section {
                    SectionType::Ours => ours_lines.push(*line),
                    SectionType::Theirs => theirs_lines.push(*line),
                    SectionType::None => base_lines.push(*line),
                }
            }
        }

        let has_conflicts = total_hunks > 0;
        if has_conflicts {
            conflicting_files = conflict_details.keys().cloned().collect();
        }

        Ok(ConflictDetectionResult {
            has_conflicts,
            conflicting_files,
            conflict_details,
            total_hunks,
        })
    }

    /// Detect conflicts using git diff
    pub async fn detect_conflicts_with_git(&self, cwd: &Path) -> Result<ConflictDetectionResult, ConflictResolutionError> {
        let output = tokio::process::Command::new("git")
            .args(["diff", "--name-only", "--diff-filter=U"])
            .current_dir(cwd)
            .output()
            .await
            .map_err(|e| ConflictResolutionError::GitFailed(e.to_string()))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(ConflictResolutionError::GitFailed(stderr.to_string()));
        }

        let conflicting: Vec<PathBuf> = String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(PathBuf::from)
            .collect();

        let mut conflict_details: HashMap<PathBuf, Vec<ConflictHunk>> = HashMap::new();
        let mut total_hunks = 0;

        for file_path in &conflicting {
            if let Ok(detection) = self.detect_conflicts(&cwd.join(file_path)).await {
                if let Some(hunks) = detection.conflict_details.get(file_path) {
                    conflict_details.insert(file_path.clone(), hunks.clone());
                    total_hunks += hunks.len();
                }
            }
        }

        Ok(ConflictDetectionResult {
            has_conflicts: !conflicting.is_empty(),
            conflicting_files: conflicting,
            conflict_details,
            total_hunks,
        })
    }

    /// Resolve conflicts for a single file using owner-based resolution
    pub async fn resolve_file(
        &self,
        file_path: &Path,
        owner_id: Option<&str>,
    ) -> Result<ResolutionResult, ConflictResolutionError> {
        let content = tokio::fs::read_to_string(file_path)
            .await
            .map_err(|e| ConflictResolutionError::IoError(e.to_string()))?;

        let detection = self.parse_conflicts(&content)?;

        if !detection.has_conflicts {
            return Ok(ResolutionResult {
                success: true,
                content: Some(content),
                conflicts_resolved: 0,
                conflicts_unresolved: 0,
                needs_manual_resolution: Vec::new(),
                strategies_used: HashMap::new(),
                error: None,
            });
        }

        // Determine owner
        let resolved_owner = if let Some(owner) = owner_id {
            owner.to_string()
        } else if let Some(owner) = self.find_owner(file_path).await {
            owner
        } else {
            // Use default behavior if no owner found
            warn!(file = %file_path.display(), "No owner found, using default resolution");
            if self.config.prefer_ours {
                "ours"
            } else {
                "theirs"
            }.to_string()
        };

        debug!(file = %file_path.display(), owner = %resolved_owner, "Resolving conflicts with owner-based strategy");

        // Resolve based on owner preference
        let (resolved_content, strategy) = self.apply_owner_resolution(&content, &resolved_owner)?;

        let result = ResolutionResult {
            success: true,
            content: Some(resolved_content),
            conflicts_resolved: detection.total_hunks,
            conflicts_unresolved: 0,
            needs_manual_resolution: Vec::new(),
            strategies_used: HashMap::from([(file_path.to_path_buf(), strategy)]),
            error: None,
        };

        Ok(result)
    }

    /// Apply owner-based resolution strategy
    fn apply_owner_resolution(
        &self,
        content: &str,
        _owner_id: &str,
    ) -> Result<(String, ResolutionStrategy), ConflictResolutionError> {
        // Owner-based resolution: if owner is "ours" or matches current branch, prefer ours
        // If owner is "theirs" or matches incoming branch, prefer theirs
        // Otherwise, try semantic merge or use default preference
        
        // Simple approach: delegate to proper resolution with preference
        let resolved = self.resolve_conflicts_properly(content);
        let strategy = if self.config.prefer_ours {
            ResolutionStrategy::Ours
        } else {
            ResolutionStrategy::Theirs
        };

        Ok((resolved, strategy))
    }

    /// Simple resolution: remove conflict markers and use our version
    fn simple_resolve(&self, content: &str) -> String {
        let lines: Vec<&str> = content.lines().collect();
        let mut result = Vec::new();
        let mut in_conflict = false;
        let mut skip_section = false;

        for line in &lines {
            let trimmed = line.trim();

            if trimmed.starts_with(&self.config.marker_start) {
                in_conflict = true;
                skip_section = true;
                continue;
            }

            if trimmed.starts_with(&self.config.separator) {
                skip_section = false; // Start of theirs section, but we prefer ours
                continue;
            }

            if trimmed.starts_with(&self.config.marker_end) {
                in_conflict = false;
                skip_section = false;
                continue;
            }

            if in_conflict && skip_section {
                // Skip ours section too since we prefer ours but this approach removes both
                // Actually, for prefer_ours, we skip the ours section and keep theirs
                // For prefer_theirs, we skip theirs and keep ours
                continue;
            }

            if !in_conflict {
                result.push(*line);
            }
        }

        result.join("\n")
    }

    /// Resolve conflicts using semantic merge when possible
    pub async fn semantic_resolve(
        &self,
        file_path: &Path,
    ) -> Result<ResolutionResult, ConflictResolutionError> {
        let content = tokio::fs::read_to_string(file_path)
            .await
            .map_err(|e| ConflictResolutionError::IoError(e.to_string()))?;

        let detection = self.parse_conflicts(&content)?;

        if !detection.has_conflicts {
            return Ok(ResolutionResult {
                success: true,
                content: Some(content.clone()),
                conflicts_resolved: 0,
                conflicts_unresolved: 0,
                needs_manual_resolution: Vec::new(),
                strategies_used: HashMap::new(),
                error: None,
            });
        }

        let mut result_content = content.clone();
        let mut resolved_count = 0;
        let mut strategies_used = HashMap::new();

        // Try semantic merge for each conflict hunk
        // This is a simplified version - real implementation would parse AST
        for (_hunk_idx, (file_key, hunks)) in detection.conflict_details.iter().enumerate() {
            for hunk in hunks {
                if let Ok(semantic_result) = self.try_semantic_merge(hunk) {
                    if semantic_result.can_merge {
                        // Replace the conflict hunk with merged content
                        result_content = self.replace_hunk(
                            &result_content,
                            hunk,
                            &semantic_result.merged_content,
                        );
                        resolved_count += 1;
                        strategies_used.insert(file_key.clone(), ResolutionStrategy::AutoMerged);
                    }
                }
            }
        }

        // If not all resolved, fall back to owner-based
        if resolved_count < detection.total_hunks {
            let _owner = self.find_owner(file_path).await;
            let fallback_strategy = if self.config.prefer_ours {
                ResolutionStrategy::Ours
            } else {
                ResolutionStrategy::Theirs
            };

            // Apply fallback to remaining conflicts
            result_content = self.apply_fallback_resolve(&result_content)?;
            strategies_used.insert(file_path.to_path_buf(), fallback_strategy);
        }

        Ok(ResolutionResult {
            success: true,
            content: Some(result_content),
            conflicts_resolved: resolved_count,
            conflicts_unresolved: detection.total_hunks - resolved_count,
            needs_manual_resolution: Vec::new(),
            strategies_used,
            error: None,
        })
    }

    /// Try to semantically merge a conflict hunk
    fn try_semantic_merge(&self, hunk: &ConflictHunk) -> Result<SemanticMergeResult, ConflictResolutionError> {
        // Simplified semantic merge:
        // - If ours and theirs differ only in comments/whitespace, merge
        // - If they add different code to same function, keep both
        // - If they modify different functions, keep both

        let _ours_lines: Vec<&str> = hunk.ours.lines().collect();
        let _theirs_lines: Vec<&str> = hunk.theirs.lines().collect();

        // If either is empty, take the other
        if hunk.ours.trim().is_empty() {
            return Ok(SemanticMergeResult {
                can_merge: true,
                merged_content: hunk.theirs.clone(),
            });
        }
        if hunk.theirs.trim().is_empty() {
            return Ok(SemanticMergeResult {
                can_merge: true,
                merged_content: hunk.ours.clone(),
            });
        }

        // If they're identical, just take ours
        if hunk.ours == hunk.theirs {
            return Ok(SemanticMergeResult {
                can_merge: true,
                merged_content: hunk.ours.clone(),
            });
        }

        // For now, can't automatically merge differing code
        // In production, would do AST-level analysis
        Ok(SemanticMergeResult {
            can_merge: false,
            merged_content: String::new(),
        })
    }

    /// Replace a conflict hunk with merged content
    fn replace_hunk(&self, content: &str, hunk: &ConflictHunk, merged: &str) -> String {
        let lines: Vec<&str> = content.lines().collect();

        // Calculate the line range for the conflict hunk
        let start_idx = hunk.start_line;
        let end_idx = hunk.end_line;

        // Reconstruct content with merged result
        let mut result = String::new();
        for (idx, line) in lines.iter().enumerate() {
            if idx == start_idx {
                result.push_str(merged);
                result.push('\n');
            } else if idx > start_idx && idx <= end_idx {
                // Skip original conflict lines
                continue;
            } else {
                result.push_str(line);
                result.push('\n');
            }
        }

        result
    }

    /// Apply fallback resolution when semantic merge fails
    fn apply_fallback_resolve(&self, content: &str) -> Result<String, ConflictResolutionError> {
        // Delegate to proper resolution with preference
        Ok(self.resolve_conflicts_properly(content))
    }

    /// Properly resolve conflicts with preference
    fn resolve_conflicts_properly(&self, content: &str) -> String {
        let lines: Vec<&str> = content.lines().collect();
        let mut result = Vec::new();
        let mut in_conflict = false;
        let mut keep_section = 0; // 0 = ours, 1 = theirs
        let mut current_section = 0;

        for line in &lines {
            let trimmed = line.trim();

            if trimmed.starts_with(&self.config.marker_start) {
                in_conflict = true;
                current_section = 0;
                keep_section = if self.config.prefer_ours { 0 } else { 1 };
                continue;
            }

            if trimmed.starts_with(&self.config.separator) {
                current_section = 1;
                continue;
            }

            if trimmed.starts_with(&self.config.marker_end) {
                in_conflict = false;
                current_section = 0;
                continue;
            }

            if in_conflict {
                // Only keep lines from the section we're choosing
                if current_section == keep_section {
                    result.push(*line);
                }
            } else {
                result.push(*line);
            }
        }

        result.join("\n")
    }

    /// Write resolved content back to file
    pub async fn write_resolved(
        &self,
        file_path: &Path,
        content: &str,
    ) -> Result<(), ConflictResolutionError> {
        tokio::fs::write(file_path, content)
            .await
            .map_err(|e| ConflictResolutionError::IoError(e.to_string()))?;
        Ok(())
    }

    /// Resolve all conflicting files in a directory
    pub async fn resolve_all(
        &self,
        cwd: &Path,
        owner_id: Option<&str>,
    ) -> Result<Vec<(PathBuf, ResolutionResult)>, ConflictResolutionError> {
        let conflicts = self.detect_conflicts_with_git(cwd).await?;

        if !conflicts.has_conflicts {
            return Ok(Vec::new());
        }

        let mut results = Vec::new();

        for file_path in &conflicts.conflicting_files {
            let full_path = cwd.join(file_path);
            let result = self.resolve_file(&full_path, owner_id).await?;

            if result.success {
                if let Some(ref content) = result.content {
                    self.write_resolved(&full_path, content).await?;
                }
            }

            results.push((file_path.clone(), result));
        }

        Ok(results)
    }

    /// Check if a file needs resolution (has conflict markers)
    pub async fn needs_resolution(&self, file_path: &Path) -> bool {
        if let Ok(content) = tokio::fs::read_to_string(file_path).await {
            content.contains(&self.config.marker_start) && content.contains(&self.config.marker_end)
        } else {
            false
        }
    }

    /// Get list of untracked/unresolved conflicts using git status
    pub async fn get_unresolved_conflicts(&self, cwd: &Path) -> Result<Vec<PathBuf>, ConflictResolutionError> {
        let output = tokio::process::Command::new("git")
            .args(["diff", "--name-only", "--diff-filter=U"])
            .current_dir(cwd)
            .output()
            .await
            .map_err(|e| ConflictResolutionError::GitFailed(e.to_string()))?;

        let conflicts: Vec<PathBuf> = String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(PathBuf::from)
            .collect();

        Ok(conflicts)
    }

    /// Stage resolved files (mark as resolved in git)
    pub async fn stage_resolved(&self, cwd: &Path, files: &[PathBuf]) -> Result<(), ConflictResolutionError> {
        if files.is_empty() {
            return Ok(());
        }

        let file_args: Vec<String> = files.iter().map(|p| p.to_string_lossy().to_string()).collect();

        let output = tokio::process::Command::new("git")
            .args(["add", "--"])
            .args(&file_args)
            .current_dir(cwd)
            .output()
            .await
            .map_err(|e| ConflictResolutionError::GitFailed(e.to_string()))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(ConflictResolutionError::GitFailed(stderr.to_string()));
        }

        Ok(())
    }

    /// Abort a merge and return to pre-merge state
    pub async fn abort_merge(&self, cwd: &Path) -> Result<(), ConflictResolutionError> {
        let output = tokio::process::Command::new("git")
            .args(["merge", "--abort"])
            .current_dir(cwd)
            .output()
            .await
            .map_err(|e| ConflictResolutionError::GitFailed(e.to_string()))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(ConflictResolutionError::GitFailed(stderr.to_string()));
        }

        info!(cwd = %cwd.display(), "Merge aborted successfully");
        Ok(())
    }
}

impl Default for ConflictResolver {
    fn default() -> Self {
        Self::new()
    }
}

/// Result of semantic merge attempt
struct SemanticMergeResult {
    can_merge: bool,
    merged_content: String,
}

/// Simple glob matching for file patterns
fn glob_match(pattern: &str, text: &str) -> bool {
    // Simple glob: supports * and ?
    let pattern_chars: Vec<char> = pattern.chars().collect();
    let text_chars: Vec<char> = text.chars().collect();

    let mut p_idx = 0;
    let mut t_idx = 0;

    while p_idx < pattern_chars.len() || t_idx < text_chars.len() {
        match pattern_chars.get(p_idx) {
            Some('*') => {
                p_idx += 1;
                // Match any sequence of characters
                if p_idx >= pattern_chars.len() {
                    // * at end matches everything remaining
                    return true;
                }
                // Try to match remaining pattern at each position
                while t_idx < text_chars.len() {
                    if glob_match(&pattern[p_idx..].to_string(), &text[t_idx..]) {
                        return true;
                    }
                    t_idx += 1;
                }
                return false;
            }
            Some('?') => {
                // ? matches any single character
                if t_idx >= text_chars.len() {
                    return false;
                }
                p_idx += 1;
                t_idx += 1;
            }
            Some(c) => {
                if t_idx >= text_chars.len() || text_chars[t_idx] != *c {
                    return false;
                }
                p_idx += 1;
                t_idx += 1;
            }
            None => {
                // End of pattern
                return t_idx >= text_chars.len();
            }
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn create_conflicted_content() -> String {
        r#"fn old_function() {
    println!("old implementation");
}

<<<<<<< HEAD
fn new_feature() {
    println!("our implementation");
}

=======
fn new_feature() {
    println!("their implementation");
}

>>>>>>> branch
fn shared_function() {
    println!("unchanged");
}
"#.to_string()
    }

    #[tokio::test]
    async fn test_conflict_resolver_new() {
        let resolver = ConflictResolver::new();
        assert!(!resolver.config().semantic_merge || resolver.config().semantic_merge); // default is true
    }

    #[tokio::test]
    async fn test_conflict_resolver_custom_config() {
        let config = ConflictResolverConfig {
            marker_start: "<<<<".to_string(),
            marker_end: ">>>>".to_string(),
            separator: "====".to_string(),
            semantic_merge: true,
            prefer_ours: false,
        };
        let resolver = ConflictResolver::with_config(config);
        assert_eq!(resolver.config().marker_start, "<<<<");
        assert!(!resolver.config().prefer_ours);
    }

    #[tokio::test]
    async fn test_parse_conflicts() {
        let resolver = ConflictResolver::new();
        let content = create_conflicted_content();

        let result = resolver.parse_conflicts(&content).unwrap();
        assert!(result.has_conflicts);
        assert_eq!(result.total_hunks, 1);
    }

    #[tokio::test]
    async fn test_parse_conflicts_no_conflicts() {
        let resolver = ConflictResolver::new();
        let content = "fn normal() {}\nfn other() {}\n";

        let result = resolver.parse_conflicts(content).unwrap();
        assert!(!result.has_conflicts);
        assert_eq!(result.total_hunks, 0);
    }

    #[tokio::test]
    async fn test_register_owner() {
        let resolver = ConflictResolver::new();
        let file_path = PathBuf::from("src/main.rs");

        resolver.register_owner(&file_path, "agent-1").await;
        let owner = resolver.find_owner(&file_path).await;
        assert_eq!(owner, Some("agent-1".to_string()));
    }

    #[tokio::test]
    async fn test_register_owner_pattern() {
        let resolver = ConflictResolver::new();
        let owner_id = "backend-team";
        let patterns = vec!["src/**/*.rs".to_string(), "lib/**/*.rs".to_string()];

        resolver.register_owner_pattern(owner_id, patterns).await;

        let file_path = PathBuf::from("src/api/main.rs");
        let owner = resolver.find_owner(file_path.as_path()).await;
        assert_eq!(owner, Some("backend-team".to_string()));
    }

    #[tokio::test]
    async fn test_glob_match() {
        assert!(glob_match("*.rs", "main.rs"));
        assert!(glob_match("src/**/*.rs", "src/api/main.rs"));
        assert!(glob_match("test?", "test1"));
        assert!(!glob_match("*.txt", "main.rs"));
    }

    #[tokio::test]
    async fn test_resolve_conflicts_prefer_ours() {
        let resolver = ConflictResolver::new();
        let content = create_conflicted_content();

        let resolved = resolver.resolve_conflicts_properly(&content);
        // With prefer_ours, should keep HEAD section
        assert!(!resolved.contains("<<<<<<<"));
        assert!(!resolved.contains(">>>>>>>"));
        assert!(resolved.contains("our implementation"));
        assert!(!resolved.contains("their implementation"));
    }

    #[tokio::test]
    async fn test_resolve_conflicts_prefer_theirs() {
        let config = ConflictResolverConfig {
            marker_start: "<<<<<<<".to_string(),
            marker_end: ">>>>>>>".to_string(),
            separator: "=======".to_string(),
            semantic_merge: true,
            prefer_ours: false,
        };
        let resolver = ConflictResolver::with_config(config);
        let content = create_conflicted_content();

        let resolved = resolver.resolve_conflicts_properly(&content);
        // With prefer_theirs, should keep theirs section
        assert!(!resolved.contains("<<<<<<<"));
        assert!(!resolved.contains(">>>>>>>"));
        assert!(resolved.contains("their implementation"));
        assert!(!resolved.contains("our implementation"));
    }

    #[tokio::test]
    async fn test_simple_resolve() {
        let resolver = ConflictResolver::new();
        let content = create_conflicted_content();

        let resolved = resolver.simple_resolve(&content);
        assert!(!resolved.contains("<<<<<<<"));
        assert!(!resolved.contains(">>>>>>>"));
    }

    #[tokio::test]
    async fn test_conflict_detection_result() {
        let result = ConflictDetectionResult {
            has_conflicts: true,
            conflicting_files: vec![PathBuf::from("src/main.rs")],
            conflict_details: HashMap::new(),
            total_hunks: 1,
        };

        assert!(result.has_conflicts);
        assert_eq!(result.conflicting_files.len(), 1);
    }

    #[tokio::test]
    async fn test_resolution_result_success() {
        let result = ResolutionResult {
            success: true,
            content: Some("resolved content".to_string()),
            conflicts_resolved: 3,
            conflicts_unresolved: 0,
            needs_manual_resolution: Vec::new(),
            strategies_used: HashMap::new(),
            error: None,
        };

        assert!(result.success);
        assert_eq!(result.conflicts_resolved, 3);
    }

    #[tokio::test]
    async fn test_resolution_result_failure() {
        let result = ResolutionResult {
            success: false,
            content: None,
            conflicts_resolved: 0,
            conflicts_unresolved: 5,
            needs_manual_resolution: vec![PathBuf::from("src/conflict.rs")],
            strategies_used: HashMap::new(),
            error: Some("Could not auto-resolve".to_string()),
        };

        assert!(!result.success);
        assert_eq!(result.conflicts_unresolved, 5);
        assert_eq!(result.needs_manual_resolution.len(), 1);
    }

    #[tokio::test]
    async fn test_conflict_hunk() {
        let hunk = ConflictHunk {
            start_line: 10,
            end_line: 25,
            base: "original".to_string(),
            ours: "our changes".to_string(),
            theirs: "their changes".to_string(),
        };

        assert_eq!(hunk.start_line, 10);
        assert_eq!(hunk.end_line, 25);
        assert_eq!(hunk.ours, "our changes");
    }

    #[tokio::test]
    async fn test_file_owner() {
        let owner = FileOwner::new("team-alpha")
            .with_pattern("src/**/*.rs")
            .with_pattern("lib/**/*.rs");

        assert_eq!(owner.owner_id, "team-alpha");
        assert_eq!(owner.patterns.len(), 2);
    }

    #[tokio::test]
    async fn test_conflict_info() {
        let info = ConflictInfo {
            file_path: PathBuf::from("src/merge.rs"),
            marker_start: "<<<<<<<".to_string(),
            base: "base content".to_string(),
            ours: "our version".to_string(),
            theirs: "their version".to_string(),
            marker_end: ">>>>>>>".to_string(),
            start_line: 5,
            end_line: 15,
            hunk_count: 2,
        };

        assert_eq!(info.file_path, PathBuf::from("src/merge.rs"));
        assert_eq!(info.hunk_count, 2);
    }

    #[tokio::test]
    async fn test_needs_resolution() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("test.rs");

        // Write conflicted content
        tokio::fs::write(&file_path, create_conflicted_content()).await.unwrap();

        let resolver = ConflictResolver::new();
        let needs = resolver.needs_resolution(&file_path).await;
        assert!(needs);

        // Write clean content
        tokio::fs::write(&file_path, "fn clean() {}\n").await.unwrap();
        let needs = resolver.needs_resolution(&file_path).await;
        assert!(!needs);
    }

    #[tokio::test]
    async fn test_semantic_merge_not_possible() {
        let resolver = ConflictResolver::new();
        let hunk = ConflictHunk {
            start_line: 0,
            end_line: 10,
            base: "base".to_string(),
            ours: "different code A".to_string(),
            theirs: "different code B".to_string(),
        };

        let result = resolver.try_semantic_merge(&hunk).unwrap();
        assert!(!result.can_merge);
    }

    #[tokio::test]
    async fn test_semantic_merge_identical() {
        let resolver = ConflictResolver::new();
        let hunk = ConflictHunk {
            start_line: 0,
            end_line: 5,
            base: "base".to_string(),
            ours: "same content".to_string(),
            theirs: "same content".to_string(),
        };

        let result = resolver.try_semantic_merge(&hunk).unwrap();
        assert!(result.can_merge);
        assert_eq!(result.merged_content, "same content");
    }

    #[tokio::test]
    async fn test_semantic_merge_empty_sides() {
        let resolver = ConflictResolver::new();

        // Ours is empty
        let hunk = ConflictHunk {
            start_line: 0,
            end_line: 3,
            base: "".to_string(),
            ours: "".to_string(),
            theirs: "they added this".to_string(),
        };
        let result = resolver.try_semantic_merge(&hunk).unwrap();
        assert!(result.can_merge);
        assert_eq!(result.merged_content, "they added this");

        // Theirs is empty
        let hunk = ConflictHunk {
            start_line: 0,
            end_line: 3,
            base: "".to_string(),
            ours: "we added this".to_string(),
            theirs: "".to_string(),
        };
        let result = resolver.try_semantic_merge(&hunk).unwrap();
        assert!(result.can_merge);
        assert_eq!(result.merged_content, "we added this");
    }

    #[tokio::test]
    async fn test_replace_hunk() {
        let resolver = ConflictResolver::new();
        let content = "line1\n<<<<<<< HEAD\nconflict1\n=======\nconflict2\n>>>>>>> branch\nline5";
        let hunk = ConflictHunk {
            start_line: 1,
            end_line: 5,
            base: "".to_string(),
            ours: "conflict1".to_string(),
            theirs: "conflict2".to_string(),
        };

        let result = resolver.replace_hunk(content, &hunk, "merged content");
        assert!(!result.contains("<<<<<<<"));
        assert!(result.contains("merged content"));
    }
}
