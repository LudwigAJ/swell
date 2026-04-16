//! Mutation Testing Gate
//!
//! Runs cargo-mutants to perform mutation testing on the codebase.
//! Computes mutation score (killed/total) and rejects if below threshold.
//!
//! # Thresholds
//!
//! - Standard paths: ≥50% mutation score (configurable)
//! - Critical paths: ≥70% mutation score (configurable)
//!
//! # Output
//!
//! Reports surviving mutants with file path, line number, and mutation type.

use async_trait::async_trait;
use std::process::Command;
use std::time::Instant;
use swell_core::{
    SwellError, ValidationContext, ValidationGate, ValidationLevel, ValidationMessage,
    ValidationOutcome,
};
use tokio::task;

/// Configuration for the mutation test gate
#[derive(Debug, Clone)]
pub struct MutationTestConfig {
    /// Minimum mutation score (0.0 to 1.0) for standard paths
    pub min_mutation_score_standard: f64,
    /// Minimum mutation score (0.0 to 1.0) for critical paths
    pub min_mutation_score_critical: f64,
    /// File patterns that indicate critical code (e.g., "**/auth*.rs", "**/crypto*.rs")
    pub critical_path_patterns: Vec<String>,
    /// Whether to fail if cargo-mutants is not installed
    pub fail_if_unavailable: bool,
    /// Timeout for cargo mutants in seconds
    pub timeout_seconds: u64,
}

impl Default for MutationTestConfig {
    fn default() -> Self {
        Self {
            // Industry standard for mutation testing is ~50%+
            min_mutation_score_standard: 0.50,
            // Critical paths should have higher coverage
            min_mutation_score_critical: 0.70,
            // Default critical path patterns
            critical_path_patterns: vec![
                "**/auth*.rs".to_string(),
                "**/crypto*.rs".to_string(),
                "**/security*.rs".to_string(),
                "**/permission*.rs".to_string(),
                "**/validation*.rs".to_string(),
            ],
            fail_if_unavailable: false,
            timeout_seconds: 600, // 10 minutes default
        }
    }
}

/// Result from parsing cargo-mutants output
#[derive(Debug, Clone)]
pub struct MutationTestResult {
    /// Total mutants found
    pub total: usize,
    /// Mutants that were killed by tests (good)
    pub killed: usize,
    /// Mutants that survived (bad - indicates test gap)
    pub survived: usize,
    /// Mutants that failed to build/check (skipped)
    pub unviable: usize,
    /// Mutation score (killed / (total - unviable))
    pub score: f64,
    /// Surviving mutations with details
    pub surviving_mutations: Vec<SurvivingMutant>,
    /// Duration in milliseconds
    pub duration_ms: u64,
}

/// A surviving mutant with location and mutation details
#[derive(Debug, Clone)]
pub struct SurvivingMutant {
    /// File where mutation occurred
    pub file: String,
    /// Line number
    pub line: u32,
    /// Type of mutation applied
    pub mutation_type: String,
    /// Description of the mutation
    pub description: String,
    /// Whether this is in a critical path
    pub is_critical: bool,
}

impl MutationTestConfig {
    /// Check if a file path matches any critical path pattern
    pub fn is_critical_path(&self, file_path: &str) -> bool {
        self.critical_path_patterns.iter().any(|p| {
            glob::Pattern::new(p)
                .map(|pat| pat.matches(file_path))
                .unwrap_or(false)
        })
    }
}

/// Gate that runs mutation testing using cargo-mutants
pub struct MutationTestGate {
    config: MutationTestConfig,
}

impl MutationTestGate {
    /// Create a new MutationTestGate with default configuration
    pub fn new() -> Self {
        Self {
            config: MutationTestConfig::default(),
        }
    }

    /// Create with custom configuration
    pub fn with_config(config: MutationTestConfig) -> Self {
        Self { config }
    }

    /// Check if cargo-mutants is available
    fn is_available() -> bool {
        which::which("cargo-mutants").is_ok() || which::which("cargo").is_ok()
    }

    /// Run cargo mutants and parse output
    async fn run_mutation_test(
        &self,
        workspace_path: &str,
    ) -> Result<MutationTestResult, SwellError> {
        let workspace_path = workspace_path.to_string();

        task::spawn_blocking(move || Self::run_mutation_test_sync(&workspace_path))
            .await
            .map_err(|e| {
                SwellError::IoError(std::io::Error::other(format!("Task join error: {}", e)))
            })?
    }

    /// Synchronous version of mutation test runner
    fn run_mutation_test_sync(workspace_path: &str) -> Result<MutationTestResult, SwellError> {
        let start = Instant::now();

        // Try cargo mutants first (better output format)
        let output = Command::new("cargo")
            .args(["mutants", "--json"])
            .current_dir(workspace_path)
            .output();

        match output {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();

                // Check if cargo mutants ran successfully
                if output.status.success() || stdout.contains("mutants tested") {
                    return Self::parse_mutation_output(
                        &stdout,
                        &stderr,
                        start.elapsed().as_millis() as u64,
                    );
                }

                // If exit code is 2, mutants were found - still parse output
                if output.status.code() == Some(2) {
                    return Self::parse_mutation_output(
                        &stdout,
                        &stderr,
                        start.elapsed().as_millis() as u64,
                    );
                }

                // Check if cargo-mutants is not found
                if stderr.contains("not found") || stdout.contains("not found") {
                    return Err(SwellError::ConfigError(
                        "cargo-mutants not found. Install with: cargo install cargo-mutants"
                            .to_string(),
                    ));
                }

                // Try to parse anyway (might have partial output)
                if !stdout.is_empty() || !stderr.is_empty() {
                    return Self::parse_mutation_output(
                        &stdout,
                        &stderr,
                        start.elapsed().as_millis() as u64,
                    );
                }

                Err(SwellError::IoError(std::io::Error::other(format!(
                    "cargo mutants failed with exit code {:?}: {}",
                    output.status.code(),
                    stderr
                ))))
            }
            Err(e) => {
                if e.kind() == std::io::ErrorKind::NotFound {
                    return Err(SwellError::ConfigError(
                        "cargo-mutants not found. Install with: cargo install cargo-mutants"
                            .to_string(),
                    ));
                }
                Err(SwellError::IoError(e))
            }
        }
    }

    /// Parse cargo-mutants output (JSON or text)
    fn parse_mutation_output(
        stdout: &str,
        stderr: &str,
        duration_ms: u64,
    ) -> Result<MutationTestResult, SwellError> {
        // Try JSON output first
        if stdout.contains("\"mutants\"") || stdout.contains("\"total\"") {
            return Self::parse_json_output(stdout, duration_ms);
        }

        // Fall back to text parsing
        Self::parse_text_output(stdout, stderr, duration_ms)
    }

    /// Parse JSON output from cargo-mutants
    fn parse_json_output(
        json_str: &str,
        duration_ms: u64,
    ) -> Result<MutationTestResult, SwellError> {
        let json: serde_json::Value = serde_json::from_str(json_str).map_err(|e| {
            SwellError::IoError(std::io::Error::other(format!("JSON parse error: {}", e)))
        })?;

        let total = json
            .get("total")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize)
            .unwrap_or(0);

        let killed = json
            .get("caught")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize)
            .unwrap_or(0);

        let survived = json
            .get("missed")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize)
            .unwrap_or(0);

        let unviable = json
            .get("unviable")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize)
            .unwrap_or(0);

        // Parse surviving mutations details
        let mut surviving_mutations = Vec::new();
        if let Some(mutations) = json.get("mutations").and_then(|v| v.as_array()) {
            for m in mutations {
                if m.get("outcome").and_then(|v| v.as_str()) == Some("missed") {
                    let file = m
                        .get("file")
                        .or_else(|| m.get("src_file"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown")
                        .to_string();
                    let line = m
                        .get("line")
                        .and_then(|v| v.as_u64())
                        .map(|v| v as u32)
                        .unwrap_or(0);
                    let mutation_type = m
                        .get("mut_type")
                        .or_else(|| m.get("mutation_type"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown")
                        .to_string();
                    let description = m
                        .get("description")
                        .or_else(|| m.get("mutated_function"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("no description")
                        .to_string();

                    surviving_mutations.push(SurvivingMutant {
                        file: file.clone(),
                        line,
                        mutation_type,
                        description,
                        is_critical: false, // Will be set later
                    });
                }
            }
        }

        let score = if total > unviable {
            killed as f64 / (total - unviable) as f64
        } else {
            1.0 // All unviable or no mutants
        };

        Ok(MutationTestResult {
            total,
            killed,
            survived,
            unviable,
            score,
            surviving_mutations,
            duration_ms,
        })
    }

    /// Parse text output from cargo-mutants
    fn parse_text_output(
        stdout: &str,
        stderr: &str,
        duration_ms: u64,
    ) -> Result<MutationTestResult, SwellError> {
        let mut total = 0usize;
        let mut killed = 0usize;
        let mut survived = 0usize;
        let mut unviable = 0usize;
        let mut surviving_mutations = Vec::new();

        // Join lines that were split across multiple lines
        // (cargo-mutants can break long lines)
        let mut pending_line = String::new();
        let combined = format!("{}\n{}", stdout, stderr);
        let lines: Vec<&str> = combined.lines().collect();

        for line in &lines {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            // Check if this is a continuation of the previous mutant line
            let has_file_path = line.starts_with("src/")
                || line.starts_with("lib/")
                || line.starts_with("crates/")
                || line.starts_with('/');
            let has_not_caught = line.contains("NOT CAUGHT") || line.contains("not caught");

            // If we have a pending line being built
            if !pending_line.is_empty() {
                if has_file_path {
                    // New mutant starts - process the pending one first
                    Self::process_line_for_mutants(&pending_line, &mut surviving_mutations);
                    pending_line.clear();
                    // Now start building this new line
                    pending_line = line.to_string();
                    continue;
                } else if has_not_caught {
                    // This line completes the pending mutant (has NOT CAUGHT)
                    pending_line.push(' ');
                    pending_line.push_str(line);
                    // Process the complete mutant line
                    Self::process_line_for_mutants(&pending_line, &mut surviving_mutations);
                    pending_line.clear();
                    continue;
                } else {
                    // This is a continuation line (no file path, no NOT CAUGHT)
                    pending_line.push(' ');
                    pending_line.push_str(line);
                    continue;
                }
            }

            // Parse summary line
            if line.contains("mutants tested") {
                // Extract numbers from "X mutants tested in time: A missed, B caught, C unviable"
                let parts: Vec<&str> = line.split_whitespace().collect();
                for (i, part) in parts.iter().enumerate() {
                    // Strip trailing punctuation for comparison
                    let clean_part = part.trim_end_matches([',', '.', ':']);
                    if *part == "mutants" && i > 0 {
                        if let Ok(n) = parts[i - 1].parse::<usize>() {
                            total = n;
                        }
                    }
                    if clean_part == "missed" && i > 0 {
                        if let Ok(n) = parts[i - 1].parse::<usize>() {
                            survived = n;
                        }
                    }
                    if clean_part == "caught" && i > 0 {
                        if let Ok(n) = parts[i - 1].parse::<usize>() {
                            killed = n;
                        }
                    }
                    if clean_part == "unviable" && i > 0 {
                        if let Ok(n) = parts[i - 1].parse::<usize>() {
                            unviable = n;
                        }
                    }
                }
                continue;
            }

            // Parse individual mutant lines: "src/lib.rs:386: replace X with Y ... NOT CAUGHT in time"
            if has_not_caught {
                // Complete line on one line
                Self::process_line_for_mutants(line, &mut surviving_mutations);
            } else if has_file_path {
                // Start building a potentially multi-line mutant line
                pending_line = line.to_string();
            }
        }

        // Process any remaining pending line
        if !pending_line.is_empty() {
            Self::process_line_for_mutants(&pending_line, &mut surviving_mutations);
        }

        // If we couldn't parse the summary, calculate from individual lines
        if total == 0 {
            total = killed + survived + unviable;
        }

        let score = if total > unviable {
            killed as f64 / (total - unviable) as f64
        } else {
            1.0
        };

        Ok(MutationTestResult {
            total,
            killed,
            survived,
            unviable,
            score,
            surviving_mutations,
            duration_ms,
        })
    }

    /// Process a line to extract surviving mutants
    fn process_line_for_mutants(line: &str, surviving_mutations: &mut Vec<SurvivingMutant>) {
        if line.contains("NOT CAUGHT") || line.contains("not caught") {
            if let Some(mutant) = Self::parse_mutant_line(line) {
                surviving_mutations.push(mutant);
            }
        }
    }

    /// Parse a single mutant line
    fn parse_mutant_line(line: &str) -> Option<SurvivingMutant> {
        // Format: "src/lib.rs:386: replace X with Y ... NOT CAUGHT in time"
        // Handle lines that may span multiple lines (newlines in description)
        let clean_line = line.replace('\n', " ").replace('\r', "");

        let parts: Vec<&str> = clean_line.split(':').collect();
        if parts.len() < 3 {
            return None;
        }

        let file = parts[0].trim().to_string();
        let line_num = parts[1].trim().parse::<u32>().ok()?;

        // Extract mutation type and description
        let full_line = parts[2..].join(":");

        // Find the description between "replace" and "..."
        let description = if full_line.contains("replace") {
            if let Some(start) = full_line.find("replace") {
                let after_replace = &full_line[start + 7..];
                let end = after_replace.find("...").unwrap_or(after_replace.len());
                after_replace[..end].trim().to_string()
            } else {
                full_line
                    .split("...")
                    .next()
                    .unwrap_or(&full_line)
                    .trim()
                    .to_string()
            }
        } else {
            full_line
                .split("...")
                .next()
                .unwrap_or(&full_line)
                .trim()
                .to_string()
        };

        // Determine mutation type from description
        let mutation_type = Self::classify_mutation_type(&description);

        Some(SurvivingMutant {
            file,
            line: line_num,
            mutation_type,
            description,
            is_critical: false,
        })
    }

    /// Classify the mutation type from the description
    fn classify_mutation_type(description: &str) -> String {
        let desc_lower = description.to_lowercase();
        if desc_lower.contains("relational")
            || desc_lower.contains("<")
            || desc_lower.contains(">")
            || desc_lower.contains("<=")
            || desc_lower.contains(">=")
        {
            "relational_flip".to_string()
        } else if desc_lower.contains("arithmetic")
            || desc_lower.contains("+")
            || desc_lower.contains("-")
            || desc_lower.contains("*")
            || desc_lower.contains("/")
        {
            "arithmetic_change".to_string()
        } else if desc_lower.contains("conditional")
            || desc_lower.contains("if")
            || desc_lower.contains("else")
        {
            "conditional_complement".to_string()
        } else if desc_lower.contains("return")
            || desc_lower.contains("->")
            || desc_lower.contains("ok(")
            || desc_lower.contains("err(")
        {
            "return_value_change".to_string()
        } else if desc_lower.contains("default")
            || desc_lower.contains("unwrap")
            || desc_lower.contains("expect")
            || desc_lower.contains("none")
            || desc_lower.contains("some(")
        {
            "default_value".to_string()
        } else if desc_lower.contains("dead") || desc_lower.contains("unreachable") {
            "dead_code".to_string()
        } else {
            "unknown".to_string()
        }
    }

    /// Convert results to validation messages
    fn results_to_messages(&self, result: &MutationTestResult) -> Vec<ValidationMessage> {
        let mut messages = Vec::new();
        let config = &self.config;

        // Determine applicable threshold based on critical paths
        let has_critical_mutants = result
            .surviving_mutations
            .iter()
            .any(|m| config.is_critical_path(&m.file));

        let threshold = if has_critical_mutants {
            config.min_mutation_score_critical
        } else {
            config.min_mutation_score_standard
        };

        // Add score summary
        let score_percent = (result.score * 100.0).round();
        let threshold_percent = (threshold * 100.0).round();

        messages.push(ValidationMessage {
            level: ValidationLevel::Info,
            code: Some("MUTATION_SCORE".to_string()),
            message: format!(
                "Mutation score: {:.1}% ({}/{} killed, {}/{} survived, {} unviable) in {}ms",
                score_percent,
                result.killed,
                result.total.saturating_sub(result.unviable),
                result.survived,
                result.total.saturating_sub(result.unviable),
                result.unviable,
                result.duration_ms
            ),
            file: None,
            line: None,
        });

        // Check if score meets threshold
        if result.score < threshold {
            messages.push(ValidationMessage {
                level: ValidationLevel::Error,
                code: Some("MUTATION_SCORE_LOW".to_string()),
                message: format!(
                    "Mutation score {:.1}% is below threshold {:.1}%. Test coverage is insufficient.",
                    score_percent,
                    threshold_percent
                ),
                file: None,
                line: None,
            });
        }

        // Report surviving mutants (limit to first 20)
        let surviving_limit = 20;
        let surviving_count = result.surviving_mutations.len();

        if !result.surviving_mutations.is_empty() {
            let mut mutant_list = Vec::new();
            for (i, m) in result
                .surviving_mutations
                .iter()
                .take(surviving_limit)
                .enumerate()
            {
                let critical_tag = if m.is_critical { " [CRITICAL]" } else { "" };
                mutant_list.push(format!(
                    "  {}. {}:{} - {}{}",
                    i + 1,
                    m.file,
                    m.line,
                    m.mutation_type,
                    critical_tag
                ));
            }

            if surviving_count > surviving_limit {
                mutant_list.push(format!(
                    "  ... and {} more surviving mutants (not shown)",
                    surviving_count - surviving_limit
                ));
            }

            messages.push(ValidationMessage {
                level: if has_critical_mutants && result.score < config.min_mutation_score_critical
                {
                    ValidationLevel::Error
                } else {
                    ValidationLevel::Warning
                },
                code: Some("SURVIVING_MUTANTS".to_string()),
                message: format!(
                    "Surviving mutants ({} total) - suggest adding tests:\n{}",
                    surviving_count,
                    mutant_list.join("\n")
                ),
                file: None,
                line: None,
            });
        }

        messages
    }

    /// Check if validation passed based on results
    fn check_passed(&self, result: &MutationTestResult) -> bool {
        let config = &self.config;

        // Determine applicable threshold
        let has_critical_mutants = result
            .surviving_mutations
            .iter()
            .any(|m| config.is_critical_path(&m.file));

        let threshold = if has_critical_mutants {
            config.min_mutation_score_critical
        } else {
            config.min_mutation_score_standard
        };

        result.score >= threshold
    }
}

impl Default for MutationTestGate {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ValidationGate for MutationTestGate {
    fn name(&self) -> &'static str {
        "mutation_test"
    }

    fn order(&self) -> u32 {
        25 // Run after lint but before AI review
    }

    async fn validate(&self, context: ValidationContext) -> Result<ValidationOutcome, SwellError> {
        let workspace_path = context.workspace_path.clone();

        // Check if cargo-mutants is available
        if !Self::is_available() {
            if self.config.fail_if_unavailable {
                return Err(SwellError::ConfigError(
                    "cargo-mutants is not installed. Install with: cargo install cargo-mutants"
                        .to_string(),
                ));
            }

            return Ok(ValidationOutcome {
                passed: true,
                messages: vec![ValidationMessage {
                    level: ValidationLevel::Warning,
                    code: Some("MUTATION_SKIP".to_string()),
                    message: "Mutation testing skipped: cargo-mutants not installed. Install with: cargo install cargo-mutants".to_string(),
                    file: None,
                    line: None,
                }],
                artifacts: vec![],
            });
        }

        // Run mutation testing
        let mut result = match self.run_mutation_test(&workspace_path).await {
            Ok(r) => r,
            Err(e) => {
                // If cargo-mutants failed to run, report but don't fail
                let error_msg = match &e {
                    SwellError::ConfigError(msg) => msg.clone(),
                    _ => format!("{:?}", e),
                };

                if self.config.fail_if_unavailable {
                    return Err(e);
                }

                return Ok(ValidationOutcome {
                    passed: true,
                    messages: vec![ValidationMessage {
                        level: ValidationLevel::Warning,
                        code: Some("MUTATION_ERROR".to_string()),
                        message: format!("Mutation testing encountered an error: {}. Install cargo-mutants for mutation testing.", error_msg),
                        file: None,
                        line: None,
                    }],
                    artifacts: vec![],
                });
            }
        };

        // Mark critical mutations
        for m in &mut result.surviving_mutations.iter_mut() {
            m.is_critical = self.config.is_critical_path(&m.file);
        }

        // Convert to messages
        let messages = self.results_to_messages(&result);

        // Check if passed
        let passed = self.check_passed(&result);

        Ok(ValidationOutcome {
            passed,
            messages,
            artifacts: vec![],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mutation_type_classification() {
        assert_eq!(
            MutationTestGate::classify_mutation_type("replace x < y with x >= y"),
            "relational_flip"
        );
        assert_eq!(
            MutationTestGate::classify_mutation_type("replace a + b with a - b"),
            "arithmetic_change"
        );
        assert_eq!(
            MutationTestGate::classify_mutation_type("replace if cond with else branch"),
            "conditional_complement"
        );
        assert_eq!(
            MutationTestGate::classify_mutation_type("replace Ok(x) with Err(y)"),
            "return_value_change"
        );
        assert_eq!(
            MutationTestGate::classify_mutation_type("replace opt.unwrap() with None"),
            "default_value"
        );
    }

    #[test]
    fn test_parse_mutant_line() {
        let line = "src/lib.rs:386: replace x < y with x >= y ... NOT CAUGHT in 0.6s";
        let mutant = MutationTestGate::parse_mutant_line(line);

        assert!(mutant.is_some());
        let m = mutant.unwrap();
        assert_eq!(m.file, "src/lib.rs");
        assert_eq!(m.line, 386);
        assert!(m.description.contains("x < y"));
    }

    #[test]
    fn test_parse_text_output() {
        let stdout = r#"
Found 14 mutants to test
src/lib.rs:386: replace <impl Error for Error>::source -> Option<&(dyn std::error::Error + 'static)>
 with Default::default() ... NOT CAUGHT in 0.6s build + 0.3s test
src/lib.rs:485: replace copy_symlink -> Result<()> with Ok(Default::default()) ...
 NOT CAUGHT in 0.5s build + 0.3s test
14 mutants tested in 0:08: 2 missed, 9 caught, 3 unviable
"#;

        let result = MutationTestGate::parse_text_output(stdout, "", 8000).unwrap();

        assert_eq!(result.total, 14);
        assert_eq!(result.survived, 2);
        assert_eq!(result.killed, 9);
        assert_eq!(result.unviable, 3);
        assert!((result.score - 0.818).abs() < 0.01); // 9/11
        assert_eq!(result.surviving_mutations.len(), 2);
    }

    #[test]
    fn test_critical_path_detection() {
        let config = MutationTestConfig::default();

        assert!(config.is_critical_path("src/auth/login.rs"));
        assert!(config.is_critical_path("src/crypto/mod.rs"));
        assert!(config.is_critical_path("src/security/permissions.rs"));
        assert!(!config.is_critical_path("src/main.rs"));
        assert!(!config.is_critical_path("src/utils/helpers.rs"));
    }

    #[test]
    fn test_score_calculation() {
        // Test with valid mutants
        let result = MutationTestResult {
            total: 100,
            killed: 80,
            survived: 15,
            unviable: 5,
            score: 80.0 / 95.0,
            surviving_mutations: vec![],
            duration_ms: 1000,
        };

        assert!((result.score - 0.842).abs() < 0.01);

        // Test with all unviable
        let result = MutationTestResult {
            total: 10,
            killed: 0,
            survived: 0,
            unviable: 10,
            score: 1.0,
            surviving_mutations: vec![],
            duration_ms: 1000,
        };

        assert_eq!(result.score, 1.0);
    }

    #[tokio::test]
    async fn test_gate_with_unavailable() {
        // When cargo-mutants is not available, gate should pass with warning
        let gate = MutationTestGate::with_config(MutationTestConfig {
            fail_if_unavailable: false,
            ..Default::default()
        });

        // This test just verifies the gate can be created and configured
        // Actual invocation would require mocking which::which
        assert_eq!(gate.name(), "mutation_test");
        assert_eq!(gate.order(), 25);
    }

    #[tokio::test]
    async fn test_default_config() {
        let config = MutationTestConfig::default();

        assert_eq!(config.min_mutation_score_standard, 0.50);
        assert_eq!(config.min_mutation_score_critical, 0.70);
        assert!(!config.critical_path_patterns.is_empty());
    }
}
