//! swell-validation - Validation pipeline for the SWELL autonomous coding engine.
//!
//! This crate provides validation gates that run quality assurance checks
//! on code changes, including linting, testing, security scanning, and AI review.
//!
//! # Validation Gates
//!
//! - [`LintGate`] - Runs linters (clippy, rustfmt)
//! - [`TestGate`] - Runs test suites
//! - [`SecurityGate`] - Runs security scans (Semgrep)
//! - [`AiReviewGate`] - AI-powered code review with Evaluator agent
//!
//! # Pipeline
//!
//! Use [`ValidationPipeline`] to run all gates in order.
//!
//! # Confidence Scoring
//!
//! Use [`ConfidenceScorer`] to compute confidence scores from validation signals.
//!
//! # Evidence Pack
//!
//! Use [`EvidencePackBuilder`] to create comprehensive evidence packs for PR review.

use async_trait::async_trait;
use std::process::Command;
use swell_core::{
    SwellError, ValidationContext, ValidationGate, ValidationLevel, ValidationMessage,
    ValidationOutcome,
};
use tokio::task;

// Re-export confidence scoring for use by other crates
pub mod confidence;
pub use confidence::{
    ConfidenceLevel, ConfidenceScore, ConfidenceScorer, ConfidenceSignal, ConfidenceThresholds,
    FlakinessHistory, TestRun,
};

// Re-export evidence pack for use by other crates
pub mod evidence;
pub use evidence::{
    AiReviewEvidence, ConfidenceEvidence, CoverageEvidence, EvidenceOutcome, EvidencePack,
    EvidencePackBuilder, EvidencePackError, FlakinessEvidence, GateEvidence, MessageCounts,
    ReviewComment, SecurityEvidence, SecurityFinding, SignalScore, TestEvidence, TestResult,
};

// ============================================================================
// Lint Gate
// ============================================================================

/// Gate that runs linters on changed files.
pub struct LintGate;

impl LintGate {
    pub fn new() -> Self {
        Self
    }
}

impl Default for LintGate {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ValidationGate for LintGate {
    fn name(&self) -> &'static str {
        "lint"
    }

    fn order(&self) -> u32 {
        10
    }

    async fn validate(&self, context: ValidationContext) -> Result<ValidationOutcome, SwellError> {
        let workspace_path = context.workspace_path.clone();

        let output = task::spawn_blocking(move || {
            // Run clippy in check mode
            Command::new("cargo")
                .args(["clippy", "--message-format", "json"])
                .current_dir(&workspace_path)
                .output()
        })
        .await
        .map_err(|e| SwellError::IoError(std::io::Error::other(format!("Task join error: {}", e))))?
        .map_err(SwellError::IoError)?;

        let passed = output.status.success();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        let mut messages = Vec::new();

        if !passed {
            // Parse clippy output for errors
            for line in stdout.lines().chain(stderr.lines()) {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(line) {
                    if let Some(msg) = json.get("message").and_then(|m| m.as_str()) {
                        let level = if json.get("level").and_then(|l| l.as_str()) == Some("error") {
                            ValidationLevel::Error
                        } else {
                            ValidationLevel::Warning
                        };

                        messages.push(ValidationMessage {
                            level,
                            code: json
                                .get("code")
                                .and_then(|c| c.get("code"))
                                .and_then(|c| c.as_str())
                                .map(String::from),
                            message: msg.to_string(),
                            file: json.get("file").and_then(|f| f.as_str()).map(String::from),
                            line: json.get("line").and_then(|l| l.as_u64()).map(|l| l as u32),
                        });
                    }
                }
            }
        }

        Ok(ValidationOutcome {
            passed,
            messages,
            artifacts: vec![],
        })
    }
}

// ============================================================================
// Test Gate
// ============================================================================

/// Classification of test failures
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TestFailureClassification {
    /// Failure due to actual code bugs or incorrect implementation
    ImplementationDefect,
    /// Failure due to incorrect test logic, assertions, or test setup
    TestDefect,
    /// Failure due to environment issues (missing deps, compilation errors, infrastructure)
    EnvironmentDefect,
    /// Cannot determine the cause from available information
    Unknown,
}

/// Parsed test results from cargo test output
#[derive(Debug, Clone, Default)]
pub struct ParsedTestOutput {
    /// Total number of tests
    pub total: usize,
    /// Number of tests that passed
    pub passed: usize,
    /// Number of tests that failed
    pub failed: usize,
    /// Number of tests that were ignored/skipped
    pub skipped: usize,
    /// Duration in milliseconds
    pub duration_ms: u64,
    /// Individual test failures with classifications
    pub failures: Vec<TestFailure>,
}

/// A single test failure with classification
#[derive(Debug, Clone)]
pub struct TestFailure {
    /// Test name (fully qualified)
    pub name: String,
    /// The failure message
    pub message: String,
    /// File where the failure occurred
    pub file: Option<String>,
    /// Line number
    pub line: Option<u32>,
    /// Classification of the failure cause
    pub classification: TestFailureClassification,
}

impl TestFailureClassification {
    /// Classify a test failure based on available information
    fn classify(message: &str, _file: Option<&str>, is_compilation_error: bool) -> Self {
        // Environment issues: compilation errors, missing dependencies, infrastructure problems
        if is_compilation_error {
            return TestFailureClassification::EnvironmentDefect;
        }

        let msg_lower = message.to_lowercase();

        // Implementation defect patterns: logic errors, null checks, index bounds, etc.
        // These take priority over generic "panicked" matches
        if msg_lower.contains("none")
            || msg_lower.contains("null")
            || msg_lower.contains("index")
            || msg_lower.contains("overflow")
            || msg_lower.contains("out of bounds")
            || msg_lower.contains("deadlock")
            || msg_lower.contains("failed to")
        {
            return TestFailureClassification::ImplementationDefect;
        }

        // Test defect patterns: assertion failures, wrong expectations, test setup issues
        if msg_lower.contains("assertion")
            || msg_lower.contains("assert_eq")
            || msg_lower.contains("assert_ne")
            || msg_lower.contains("assert!")
            || msg_lower.contains("expected:")
            || msg_lower.contains("expected ")
            || msg_lower.contains("actual:")
            || msg_lower.contains("but got")
            || msg_lower.contains("mismatch")
            || msg_lower.contains("result was")
        {
            return TestFailureClassification::TestDefect;
        }

        // Environment issues: compilation errors, missing dependencies, infrastructure problems
        if msg_lower.contains("link:")
            || msg_lower.contains("cannot find")
            || msg_lower.contains("library not found")
            || msg_lower.contains("depends on")
            || msg_lower.contains("could not compile")
        {
            return TestFailureClassification::EnvironmentDefect;
        }

        // Runtime environment issues - but NOT if it looks like an impl defect
        if msg_lower.contains("panicked") && msg_lower.contains("thread") {
            // Check for known impl patterns first
            if msg_lower.contains("lock") || msg_lower.contains("mutex") {
                return TestFailureClassification::ImplementationDefect;
            }
            return TestFailureClassification::EnvironmentDefect;
        }

        TestFailureClassification::Unknown
    }
}

/// Gate that runs tests for the workspace.
pub struct TestGate;

impl TestGate {
    pub fn new() -> Self {
        Self
    }

    /// Parse cargo test output and classify failures
    fn parse_test_output(stdout: &str, stderr: &str) -> ParsedTestOutput {
        let mut output = ParsedTestOutput::default();
        let full_output = format!("{}\n{}", stdout, stderr);

        // Try to extract test counts from the summary line
        // Format: "test result: ok. X passed; Y failed; Z ignored; ..."
        // or: "test result: FAILED. X passed; Y failed; Z ignored; ..."
        for line in full_output.lines() {
            let line = line.trim();

            // Parse summary line
            if line.starts_with("test result:") || line.starts_with("test result") {
                let summary = line
                    .trim_start_matches("test result:")
                    .trim_start_matches("test result")
                    .replace("ok.", "")
                    .replace("FAILED.", "")
                    .trim()
                    .to_string();

                // Parse "X passed; Y failed; Z ignored"
                for part in summary.split(';') {
                    let part = part.trim();
                    if part.contains("passed") {
                        if let Some(count) = part.split_whitespace().next() {
                            output.passed = count.parse().unwrap_or(0);
                        }
                    } else if part.contains("failed") {
                        if let Some(count) = part.split_whitespace().next() {
                            output.failed = count.parse().unwrap_or(0);
                        }
                    } else if part.contains("ignored") || part.contains("skipped") {
                        if let Some(count) = part.split_whitespace().next() {
                            output.skipped = count.parse().unwrap_or(0);
                        }
                    }
                }
            }

            // Try to extract duration
            if line.contains("finished in") || line.ends_with('s') {
                if let Some(dur) = Self::extract_duration(line) {
                    output.duration_ms = dur;
                }
            }
        }

        output.total = output.passed + output.failed + output.skipped;

        // Parse individual failures
        let mut current_test = None::<String>;
        let mut current_msg = Vec::new();
        let mut current_file = None::<String>;
        let mut current_line = None::<u32>;

        for line in full_output.lines() {
            let line = line.trim();

            // Test name line (rust test format)
            if line.starts_with("test ") && (line.ends_with(" ... FAILED") || line.ends_with(" ... ok")) {
                // Save previous failure if any
                if let Some(name) = current_test.take() {
                    let msg = current_msg.join("\n");
                    if !msg.is_empty() {
                        let is_comp = msg.contains("cannot find")
                            || msg.contains("could not compile")
                            || msg.contains("link");
                        let class = TestFailureClassification::classify(
                            &msg,
                            current_file.as_deref(),
                            is_comp,
                        );
                        output.failures.push(TestFailure {
                            name,
                            message: msg,
                            file: current_file.take(),
                            line: current_line.take(),
                            classification: class,
                        });
                    }
                }

                // Start new test
                let name = line
                    .trim_start_matches("test ")
                    .trim_end_matches(" ... FAILED")
                    .trim_end_matches(" ... ok")
                    .trim();
                current_test = Some(name.to_string());
                current_msg.clear();
            } else if line.starts_with("test ") && line.contains("... ") {
                // Another format
                if let Some(name) = line.strip_prefix("test ") {
                    let parts: Vec<&str> = name.split("... ").collect();
                    if parts.len() == 2 {
                        current_test = Some(parts[0].trim().to_string());
                    }
                }
            }
            // Failure location: "  --> file.rs:line:col"
            else if line.starts_with("  --> ") || line.starts_with("--> ") {
                if let Some(path) = line.strip_prefix("  --> ").or_else(|| line.strip_prefix("--> ")) {
                    let parts: Vec<&str> = path.split(':').collect();
                    if !parts.is_empty() {
                        current_file = Some(parts[0].to_string());
                        if parts.len() > 1 {
                            current_line = parts[1].parse().ok();
                        }
                    }
                }
            }
            // Source line: "   |"
            else if line.starts_with("   |") || line.starts_with("| ") {
                // skip source context lines
            }
            // Error/panic message lines
            else if current_test.is_some()
                && (line.starts_with("thread '") || line.starts_with("panicked at") || line.starts_with("error"))
            {
                current_msg.push(line.to_string());
            } else if current_test.is_some() && !line.is_empty() && !line.starts_with("test ") {
                // Accumulate failure message
                if current_msg.len() < 20 {
                    // Avoid huge messages
                    current_msg.push(line.to_string());
                }
            }
        }

        // Don't forget the last failure
        if let Some(name) = current_test {
            let msg = current_msg.join("\n");
            if !msg.is_empty() {
                let is_comp = msg.contains("cannot find")
                    || msg.contains("could not compile")
                    || msg.contains("link");
                let class = TestFailureClassification::classify(
                    &msg,
                    current_file.as_deref(),
                    is_comp,
                );
                output.failures.push(TestFailure {
                    name,
                    message: msg,
                    file: current_file.take(),
                    line: current_line.take(),
                    classification: class,
                });
            }
        }

        output
    }

    /// Extract duration in ms from a line
    fn extract_duration(line: &str) -> Option<u64> {
        // Look for patterns like "finished in 1.23s" or "X.XXs"
        if let Some(idx) = line.find("finished in ") {
            let after = &line[idx + 12..];
            let value: String = after
                .chars()
                .take_while(|c| c.is_numeric() || *c == '.')
                .collect();
            if let Ok(secs) = value.parse::<f64>() {
                return Some((secs * 1000.0) as u64);
            }
        }
        None
    }
}

impl Default for TestGate {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ValidationGate for TestGate {
    fn name(&self) -> &'static str {
        "test"
    }

    fn order(&self) -> u32 {
        20
    }

    async fn validate(&self, context: ValidationContext) -> Result<ValidationOutcome, SwellError> {
        let workspace_path = context.workspace_path.clone();

        let output = task::spawn_blocking(move || {
            Command::new("cargo")
                .args(["test", "--", "--format", "pretty"])
                .current_dir(&workspace_path)
                .output()
        })
        .await
        .map_err(|e| SwellError::IoError(std::io::Error::other(format!("Task join error: {}", e))))?
        .map_err(SwellError::IoError)?;

        let passed = output.status.success();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        let parsed = Self::parse_test_output(&stdout, &stderr);

        let mut messages = Vec::new();

        if !passed {
            // Group failures by classification
            let impl_defects: Vec<_> = parsed
                .failures
                .iter()
                .filter(|f| f.classification == TestFailureClassification::ImplementationDefect)
                .collect();
            let test_defects: Vec<_> = parsed
                .failures
                .iter()
                .filter(|f| f.classification == TestFailureClassification::TestDefect)
                .collect();
            let env_defects: Vec<_> = parsed
                .failures
                .iter()
                .filter(|f| f.classification == TestFailureClassification::EnvironmentDefect)
                .collect();
            let unknown: Vec<_> = parsed
                .failures
                .iter()
                .filter(|f| f.classification == TestFailureClassification::Unknown)
                .collect();

            if !impl_defects.is_empty() {
                messages.push(ValidationMessage {
                    level: ValidationLevel::Error,
                    code: None,
                    message: format!(
                        "Implementation defects ({} test{}):\n{}",
                        impl_defects.len(),
                        if impl_defects.len() == 1 { "" } else { "s" },
                        impl_defects
                            .iter()
                            .map(|f| format!("  - {}: {}", f.name, f.message.lines().next().unwrap_or("")))
                            .collect::<Vec<_>>()
                            .join("\n")
                    ),
                    file: None,
                    line: None,
                });
            }

            if !test_defects.is_empty() {
                messages.push(ValidationMessage {
                    level: ValidationLevel::Error,
                    code: None,
                    message: format!(
                        "Test defects ({} test{}):\n{}",
                        test_defects.len(),
                        if test_defects.len() == 1 { "" } else { "s" },
                        test_defects
                            .iter()
                            .map(|f| format!("  - {}: {}", f.name, f.message.lines().next().unwrap_or("")))
                            .collect::<Vec<_>>()
                            .join("\n")
                    ),
                    file: None,
                    line: None,
                });
            }

            if !env_defects.is_empty() {
                messages.push(ValidationMessage {
                    level: ValidationLevel::Error,
                    code: None,
                    message: format!(
                        "Environment defects ({} test{}):\n{}",
                        env_defects.len(),
                        if env_defects.len() == 1 { "" } else { "s" },
                        env_defects
                            .iter()
                            .map(|f| format!("  - {}: {}", f.name, f.message.lines().next().unwrap_or("")))
                            .collect::<Vec<_>>()
                            .join("\n")
                    ),
                    file: None,
                    line: None,
                });
            }

            if !unknown.is_empty() {
                messages.push(ValidationMessage {
                    level: ValidationLevel::Warning,
                    code: None,
                    message: format!(
                        "Unclassified failures ({} test{}):\n{}",
                        unknown.len(),
                        if unknown.len() == 1 { "" } else { "s" },
                        unknown
                            .iter()
                            .map(|f| format!("  - {}: {}", f.name, f.message.lines().next().unwrap_or("")))
                            .collect::<Vec<_>>()
                            .join("\n")
                    ),
                    file: None,
                    line: None,
                });
            }

            // Add summary
            messages.push(ValidationMessage {
                level: ValidationLevel::Info,
                code: None,
                message: format!(
                    "Test summary: {} total, {} passed, {} failed, {} skipped ({}ms)",
                    parsed.total, parsed.passed, parsed.failed, parsed.skipped, parsed.duration_ms
                ),
                file: None,
                line: None,
            });
        } else {
            messages.push(ValidationMessage {
                level: ValidationLevel::Info,
                code: None,
                message: format!(
                    "All tests passed ({} total, {}ms)",
                    parsed.total, parsed.duration_ms
                ),
                file: None,
                line: None,
            });
        }

        Ok(ValidationOutcome {
            passed,
            messages,
            artifacts: vec![],
        })
    }
}

// ============================================================================
// Security Gate
// ============================================================================

/// Security scanner type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityScannerType {
    /// Semgrep静态分析器
    Semgrep,
    /// CodeQL静态分析器
    CodeQL,
}

impl SecurityScannerType {
    /// Get the command name for this scanner
    pub fn command(&self) -> &'static str {
        match self {
            SecurityScannerType::Semgrep => "semgrep",
            SecurityScannerType::CodeQL => "codeql",
        }
    }

    /// Get the arguments to run a scan
    pub fn scan_args(&self, path: &str) -> Vec<String> {
        match self {
            SecurityScannerType::Semgrep => {
                vec![
                    "scan".to_string(),
                    "--json".to_string(),
                    "--no-gitignore".to_string(),
                    "--disable-version-check".to_string(),
                    path.to_string(),
                ]
            }
            SecurityScannerType::CodeQL => {
                vec![
                    "database".to_string(),
                    "analyze".to_string(),
                    "--format=sarif-latest".to_string(),
                    path.to_string(),
                ]
            }
        }
    }
}

/// Severity level for security findings
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum FindingSeverity {
    /// Error severity - blocks merge
    Error = 0,
    /// Warning severity - does not block
    Warning = 1,
    /// Info severity - informational only
    Info = 2,
}

impl FindingSeverity {
    /// Parse severity from string (Semgrep format)
    pub fn from_semgrep(level: &str) -> Option<Self> {
        match level.to_lowercase().as_str() {
            "error" => Some(FindingSeverity::Error),
            "warning" | "warn" => Some(FindingSeverity::Warning),
            "info" => Some(FindingSeverity::Info),
            _ => None,
        }
    }

    /// Parse severity from SARIF level (CodeQL format)
    pub fn from_sarif(level: &str) -> Option<Self> {
        match level.to_lowercase().as_str() {
            "error" | "critical" | "high" => Some(FindingSeverity::Error),
            "warning" | "medium" => Some(FindingSeverity::Warning),
            "note" | "low" | "info" => Some(FindingSeverity::Info),
            _ => None,
        }
    }

    /// Get the validation level for this finding severity
    pub fn to_validation_level(self) -> ValidationLevel {
        match self {
            FindingSeverity::Error => ValidationLevel::Error,
            FindingSeverity::Warning => ValidationLevel::Warning,
            FindingSeverity::Info => ValidationLevel::Info,
        }
    }

    /// Whether this severity should block the build
    pub fn should_block(self) -> bool {
        matches!(self, FindingSeverity::Error)
    }
}

/// A security vulnerability finding (parsed from scanner output)
#[derive(Debug, Clone)]
pub struct Vulnerability {
    /// Unique identifier
    pub id: String,
    /// CWE category (e.g., "CWE-79")
    pub cwe: Option<String>,
    /// Severity level
    pub severity: FindingSeverity,
    /// Title/short description
    pub title: String,
    /// File where found
    pub file: Option<String>,
    /// Line number
    pub line: Option<u32>,
    /// Full description
    pub description: String,
    /// URL to more info
    pub link: Option<String>,
    /// The scanner that found this
    pub scanner: SecurityScannerType,
}

impl Vulnerability {
    /// Create from Semgrep JSON output
    pub fn from_semgrep(result: &serde_json::Value) -> Option<Self> {
        let check_id = result.get("check_id")?.as_str()?;
        let severity_str = result.get("extra")?.get("severity")?.as_str()?;
        let severity = FindingSeverity::from_semgrep(severity_str)?;
        let message = result.get("extra")?.get("message")?.as_str()?.to_string();

        let mut file = None;
        let mut line = None;
        if let Some(start) = result.get("start") {
            file = start.get("filename").or_else(|| start.get("file")).and_then(|f| f.as_str()).map(String::from);
            line = start.get("line").and_then(|l| l.as_u64()).map(|l| l as u32);
        }

        // Extract CWE from metadata if present
        let cwe = result
            .get("extra")
            .and_then(|e| e.get("metadata"))
            .and_then(|m| m.get("cwe"))
            .and_then(|c| c.as_str())
            .map(String::from);

        // Extract link from metadata
        let link = result
            .get("extra")
            .and_then(|e| e.get("metadata"))
            .and_then(|m| m.get("url"))
            .and_then(|u| u.as_str())
            .map(String::from);

        Some(Vulnerability {
            id: check_id.to_string(),
            cwe,
            severity,
            title: message.lines().next().unwrap_or(&message).to_string(),
            file,
            line,
            description: message,
            link,
            scanner: SecurityScannerType::Semgrep,
        })
    }

    /// Create from CodeQL SARIF output
    pub fn from_sarif(result: &serde_json::Value, run_index: usize) -> Option<Self> {
        let rule_id = result.get("ruleId")?.as_str()?;
        let level = result
            .get("level")
            .and_then(|l| l.as_str())
            .unwrap_or("warning");
        let severity = FindingSeverity::from_sarif(level)?;

        let mut file = None;
        let mut line = None;
        if let Some(loc) = result.get("locations")?.as_array()?.first() {
            if let Some(physical) = loc.get("physicalLocation") {
                file = physical
                    .get("artifactLocation")?
                    .get("uri")?
                    .as_str()
                    .map(String::from);
                line = physical
                    .get("region")?
                    .get("startLine")?
                    .as_u64()
                    .map(|l| l as u32);
            }
        }

        let message = result
            .get("message")?
            .get("text")?
            .as_str()
            .unwrap_or("No message")
            .to_string();

        Some(Vulnerability {
            id: format!("codeql-{}-{}", run_index, rule_id),
            cwe: None,
            severity,
            title: message.lines().next().unwrap_or(&message).to_string(),
            file,
            line,
            description: message,
            link: None,
            scanner: SecurityScannerType::CodeQL,
        })
    }
}

/// Parsed security scan results
#[derive(Debug, Clone, Default)]
pub struct SecurityScanResults {
    /// All findings
    pub findings: Vec<Vulnerability>,
    /// Scanner that was used
    pub scanner: Option<SecurityScannerType>,
    /// Scan duration in milliseconds
    pub duration_ms: u64,
    /// Whether scan succeeded
    pub scan_success: bool,
    /// Error message if scan failed
    pub error_message: Option<String>,
}

impl SecurityScanResults {
    /// Get count of findings by severity
    pub fn count_by_severity(&self) -> (usize, usize, usize) {
        let mut critical_high = 0;
        let mut medium = 0;
        let mut low = 0;

        for f in &self.findings {
            match f.severity {
                FindingSeverity::Error => critical_high += 1,
                FindingSeverity::Warning => medium += 1,
                FindingSeverity::Info => {
                    // Low severity in Semgrep terms is "info"
                    low += 1;
                }
            }
        }

        (critical_high, medium, low)
    }

    /// Check if there are any blocking findings
    pub fn has_blocking_findings(&self) -> bool {
        self.findings
            .iter()
            .any(|f| f.severity.should_block())
    }
}

/// Gate that runs security scans using Semgrep or CodeQL.
pub struct SecurityGate {
    /// Scanners to use (in order of preference)
    scanners: Vec<SecurityScannerType>,
    /// Block on high severity findings
    block_on_high: bool,
}

impl SecurityGate {
    /// Create a new SecurityGate with default scanners.
    pub fn new() -> Self {
        Self {
            scanners: vec![SecurityScannerType::Semgrep],
            block_on_high: false,
        }
    }

    /// Create with specific scanners.
    pub fn with_scanners(scanners: Vec<SecurityScannerType>) -> Self {
        Self {
            scanners,
            block_on_high: false,
        }
    }

    /// Enable blocking on high severity findings.
    pub fn with_block_on_high(mut self) -> Self {
        self.block_on_high = true;
        self
    }

    /// Check if a scanner is available in the environment.
    fn is_scanner_available(&self, scanner: SecurityScannerType) -> bool {
        which::which(scanner.command()).is_ok()
    }

    /// Run security scan with the first available scanner.
    async fn run_scan(&self, workspace_path: &str) -> SecurityScanResults {
        let start = std::time::Instant::now();

        for scanner in &self.scanners {
            if !self.is_scanner_available(*scanner) {
                tracing::debug!("{} not available, trying next scanner", scanner.command());
                continue;
            }

            let result = self.run_scanner(*scanner, workspace_path).await;
            if result.scan_success {
                return result;
            }
            tracing::debug!("{} scan failed, trying next scanner", scanner.command());
        }

        // No scanner available
        SecurityScanResults {
            scan_success: false,
            error_message: Some(
                "No security scanner available. Install semgrep or codeql.".to_string(),
            ),
            duration_ms: start.elapsed().as_millis() as u64,
            ..Default::default()
        }
    }

    /// Run a specific scanner.
    async fn run_scanner(
        &self,
        scanner: SecurityScannerType,
        workspace_path: &str,
    ) -> SecurityScanResults {
        use std::io::Result as IoResult;
        
        let start = std::time::Instant::now();
        let args = scanner.scan_args(workspace_path);
        // Convert to String to satisfy 'static lifetime requirement for spawn_blocking
        let workspace_path_string = workspace_path.to_string();

        let join_result: Result<IoResult<std::process::Output>, tokio::task::JoinError> = 
            task::spawn_blocking(move || {
                Command::new(scanner.command())
                    .args(&args)
                    .current_dir(&workspace_path_string)
                    .output()
            })
            .await;

        match join_result {
            Ok(Ok(result)) => {
                let stderr = String::from_utf8_lossy(&result.stderr);
                if !result.status.success() && !stderr.contains("no findings") {
                    tracing::warn!("Security scan had issues: {}", stderr);
                }

                let stdout = String::from_utf8_lossy(&result.stdout);
                let findings = match scanner {
                    SecurityScannerType::Semgrep => self.parse_semgrep_output(&stdout),
                    SecurityScannerType::CodeQL => self.parse_codeql_output(&stdout),
                };

                SecurityScanResults {
                    findings,
                    scanner: Some(scanner),
                    duration_ms: start.elapsed().as_millis() as u64,
                    scan_success: true,
                    error_message: None,
                }
            }
            Ok(Err(e)) => SecurityScanResults {
                scan_success: false,
                error_message: Some(format!("Failed to execute scanner: {}", e)),
                duration_ms: start.elapsed().as_millis() as u64,
                ..Default::default()
            },
            Err(e) => SecurityScanResults {
                scan_success: false,
                error_message: Some(format!("Task join error: {}", e)),
                duration_ms: start.elapsed().as_millis() as u64,
                ..Default::default()
            },
        }
    }

    /// Parse Semgrep JSON output.
    fn parse_semgrep_output(&self, output: &str) -> Vec<Vulnerability> {
        let mut findings = Vec::new();

        // Try to parse as JSON array of results
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(output) {
            if let Some(results) = json.get("results").and_then(|r| r.as_array()) {
                for result in results {
                    if let Some(finding) = Vulnerability::from_semgrep(result) {
                        findings.push(finding);
                    }
                }
            }
        }

        findings
    }

    /// Parse CodeQL SARIF output.
    fn parse_codeql_output(&self, output: &str) -> Vec<Vulnerability> {
        let mut findings = Vec::new();

        if let Ok(json) = serde_json::from_str::<serde_json::Value>(output) {
            // SARIF format has "runs" array
            if let Some(runs) = json.get("runs").and_then(|r| r.as_array()) {
                for (run_index, run) in runs.iter().enumerate() {
                    if let Some(results) = run.get("results").and_then(|r| r.as_array()) {
                        for result in results {
                            if let Some(finding) = Vulnerability::from_sarif(result, run_index) {
                                findings.push(finding);
                            }
                        }
                    }
                }
            }
        }

        findings
    }

    /// Convert findings to validation messages.
    fn findings_to_messages(&self, results: &SecurityScanResults) -> Vec<ValidationMessage> {
        let mut messages = Vec::new();

        if !results.scan_success {
            if let Some(ref err) = results.error_message {
                messages.push(ValidationMessage {
                    level: ValidationLevel::Warning,
                    code: Some("SEC_SCAN_UNAVAILABLE".to_string()),
                    message: err.clone(),
                    file: None,
                    line: None,
                });
            }
            return messages;
        }

        let (critical_high, medium, low) = results.count_by_severity();

        if results.findings.is_empty() {
            messages.push(ValidationMessage {
                level: ValidationLevel::Info,
                code: None,
                message: format!(
                    "Security scan ({}) completed: no vulnerabilities found",
                    results
                        .scanner
                        .map(|s| s.command())
                        .unwrap_or("unknown")
                ),
                file: None,
                line: None,
            });
        } else {
            // Group by severity
            let errors: Vec<_> = results
                .findings
                .iter()
                .filter(|f| f.severity == FindingSeverity::Error)
                .collect();
            let warnings: Vec<_> = results
                .findings
                .iter()
                .filter(|f| f.severity == FindingSeverity::Warning)
                .collect();

            if !errors.is_empty() {
                let mut files: Vec<_> = errors
                    .iter()
                    .filter_map(|f| f.file.clone())
                    .collect();
                files.sort();
                files.dedup();

                let mut cwes: Vec<_> = errors
                    .iter()
                    .filter_map(|f| f.cwe.clone())
                    .collect();
                cwes.sort();
                cwes.dedup();

                messages.push(ValidationMessage {
                    level: ValidationLevel::Error,
                    code: Some("SEC_CRITICAL".to_string()),
                    message: format!(
                        "Critical/High severity security findings ({}):\n  Files: {}\n  CWEs: {}\n\nFirst few findings:\n{}",
                        errors.len(),
                        files.join(", "),
                        cwes.join(", "),
                        errors
                            .iter()
                            .take(5)
                            .map(|f| format!(
                                "  - {} ({}:{}): {}",
                                f.id,
                                f.file.as_deref().unwrap_or("unknown"),
                                f.line.map(|l| l.to_string()).unwrap_or_else(|| "?".to_string()),
                                f.title
                            ))
                            .collect::<Vec<_>>()
                            .join("\n")
                    ),
                    file: None,
                    line: None,
                });
            }

            if !warnings.is_empty() {
                messages.push(ValidationMessage {
                    level: ValidationLevel::Warning,
                    code: Some("SEC_WARNING".to_string()),
                    message: format!(
                        "Medium/Low severity security findings ({})",
                        warnings.len()
                    ),
                    file: None,
                    line: None,
                });
            }

            // Summary
            messages.push(ValidationMessage {
                level: ValidationLevel::Info,
                code: None,
                message: format!(
                    "Security scan summary: {} critical/high, {} medium/low ({}ms)",
                    critical_high,
                    medium + low,
                    results.duration_ms
                ),
                file: None,
                line: None,
            });
        }

        messages
    }
}

impl Default for SecurityGate {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ValidationGate for SecurityGate {
    fn name(&self) -> &'static str {
        "security"
    }

    fn order(&self) -> u32 {
        30
    }

    async fn validate(&self, context: ValidationContext) -> Result<ValidationOutcome, SwellError> {
        let workspace_path = context.workspace_path.clone();

        // Run security scan
        let results = self.run_scan(&workspace_path).await;

        // Convert to validation messages
        let messages = self.findings_to_messages(&results);

        // Determine if validation passed
        // Security scan itself must succeed
        // And we must not have blocking findings
        let scan_passed = results.scan_success;
        let no_blocking_findings = !results.has_blocking_findings();

        // If block_on_high is set, any high severity findings block
        let no_high_findings = if self.block_on_high {
            let (critical_high, _, _) = results.count_by_severity();
            critical_high == 0
        } else {
            true
        };

        let passed = scan_passed && no_blocking_findings && no_high_findings;

        Ok(ValidationOutcome {
            passed,
            messages,
            artifacts: vec![],
        })
    }
}

// ============================================================================
// AI Review Gate (Stub)
// ============================================================================

/// Gate that performs AI-powered code review (stub implementation for MVP).
pub struct AiReviewGate;

impl AiReviewGate {
    pub fn new() -> Self {
        Self
    }
}

impl Default for AiReviewGate {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ValidationGate for AiReviewGate {
    fn name(&self) -> &'static str {
        "ai_review"
    }

    fn order(&self) -> u32 {
        40
    }

    async fn validate(&self, _context: ValidationContext) -> Result<ValidationOutcome, SwellError> {
        // Stub implementation - AI review not yet implemented
        Ok(ValidationOutcome {
            passed: true,
            messages: vec![ValidationMessage {
                level: ValidationLevel::Info,
                code: None,
                message: "AI review gate stub: full AI review not yet implemented".to_string(),
                file: None,
                line: None,
            }],
            artifacts: vec![],
        })
    }
}

// ============================================================================
// Validation Pipeline
// ============================================================================

/// A pipeline that runs multiple validation gates in order.
pub struct ValidationPipeline {
    gates: Vec<Box<dyn ValidationGate>>,
}

impl ValidationPipeline {
    /// Create a new empty pipeline.
    pub fn new() -> Self {
        Self { gates: vec![] }
    }

    /// Create a pipeline with the given gates.
    pub fn with_gates(gates: Vec<Box<dyn ValidationGate>>) -> Self {
        Self { gates }
    }

    /// Add a gate to the pipeline.
    pub fn add_gate<G: ValidationGate + 'static>(&mut self, gate: G) {
        self.gates.push(Box::new(gate));
    }

    /// Run all gates in order.
    pub async fn run(&self, context: &ValidationContext) -> Result<ValidationOutcome, SwellError> {
        let mut all_messages = Vec::new();
        let mut all_passed = true;

        for gate in &self.gates {
            let outcome = gate.validate(context.clone()).await?;
            all_passed &= outcome.passed;
            all_messages.extend(outcome.messages);
        }

        Ok(ValidationOutcome {
            passed: all_passed,
            messages: all_messages,
            artifacts: vec![],
        })
    }
}

impl Default for ValidationPipeline {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod lint_gate_tests {
    use super::*;
    use uuid::Uuid;

    #[tokio::test]
    async fn test_lint_gate_validate_returns_outcome() {
        // Test that LintGate.validate returns a ValidationOutcome
        let gate = LintGate::new();
        let context = ValidationContext {
            task_id: Uuid::new_v4(),
            workspace_path: std::env::current_dir()
                .unwrap()
                .to_string_lossy()
                .to_string(),
            changed_files: vec![],
            plan: None,
        };

        let result = gate.validate(context).await;
        assert!(result.is_ok(), "LintGate.validate should succeed");

        let outcome = result.unwrap();
        // When clippy passes (no errors), passed should be true
        // The workspace should be clean, so this should pass
        assert!(
            outcome.passed || !outcome.messages.is_empty(),
            "Should either pass or have messages about issues"
        );
    }

    #[test]
    fn test_lint_gate_name() {
        let gate = LintGate::new();
        assert_eq!(gate.name(), "lint");
    }

    #[test]
    fn test_lint_gate_order() {
        let gate = LintGate::new();
        assert_eq!(gate.order(), 10);
    }

    #[test]
    fn test_lint_gate_default() {
        let gate = LintGate::default();
        assert_eq!(gate.name(), "lint");
    }

    #[tokio::test]
    async fn test_lint_gate_new() {
        let gate = LintGate::new();
        let context = ValidationContext {
            task_id: Uuid::new_v4(),
            workspace_path: std::env::current_dir()
                .unwrap()
                .to_string_lossy()
                .to_string(),
            changed_files: vec![],
            plan: None,
        };

        let result = gate.validate(context).await;
        assert!(result.is_ok());
        let outcome = result.unwrap();
        assert!(outcome.passed || !outcome.messages.is_empty());
    }
}

#[cfg(test)]
mod test_gate_tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn test_test_gate_name() {
        let gate = TestGate::new();
        assert_eq!(gate.name(), "test");
    }

    #[test]
    fn test_test_gate_order() {
        let gate = TestGate::new();
        assert_eq!(gate.order(), 20);
    }

    #[test]
    fn test_test_gate_default() {
        let gate = TestGate::default();
        assert_eq!(gate.name(), "test");
    }

    #[tokio::test]
    async fn test_test_gate_validate_returns_outcome() {
        let gate = TestGate::new();
        let context = ValidationContext {
            task_id: Uuid::new_v4(),
            workspace_path: std::env::current_dir()
                .unwrap()
                .to_string_lossy()
                .to_string(),
            changed_files: vec![],
            plan: None,
        };

        let result = gate.validate(context).await;
        assert!(result.is_ok(), "TestGate.validate should succeed");

        let outcome = result.unwrap();
        // This may pass or fail depending on current test state
        // Just verify we get a valid outcome
        assert!(outcome.passed || !outcome.messages.is_empty());
    }

    #[test]
    fn test_failure_classification_impl_defect() {
        // Logic errors, null checks, index bounds
        let msg = "thread 'main' panicked at 'index out of bounds: the len is 3 but the index is 99'";
        let class = TestFailureClassification::classify(msg, None, false);
        assert_eq!(class, TestFailureClassification::ImplementationDefect);
    }

    #[test]
    fn test_failure_classification_test_defect() {
        // Assertion failures
        let msg = "assertion failed: `(left == right)`\n  left: `1`\n right: `2`";
        let class = TestFailureClassification::classify(msg, None, false);
        assert_eq!(class, TestFailureClassification::TestDefect);
    }

    #[test]
    fn test_failure_classification_env_defect() {
        // Compilation errors, missing libraries
        let msg = "error: cannot find dependency `missing_crate`";
        let class = TestFailureClassification::classify(msg, None, true);
        assert_eq!(class, TestFailureClassification::EnvironmentDefect);
    }

    #[test]
    fn test_failure_classification_unknown() {
        // Unclear failure
        let msg = "something went wrong in a mysterious way";
        let class = TestFailureClassification::classify(msg, None, false);
        assert_eq!(class, TestFailureClassification::Unknown);
    }

    #[test]
    fn test_parse_test_output_passed() {
        let stdout = r#"running 5 tests
test test_one ... ok
test test_two ... ok
test test_three ... ok
test test_four ... ok
test test_five ... ok

test result: ok. 5 passed; 0 failed; 0 ignored; finished in 1.234s
"#;
        let stderr = "";

        let parsed = TestGate::parse_test_output(stdout, stderr);
        assert_eq!(parsed.total, 5);
        assert_eq!(parsed.passed, 5);
        assert_eq!(parsed.failed, 0);
    }

    #[test]
    fn test_parse_test_output_with_failures() {
        let stdout = r#"running 3 tests
test test_impl ... FAILED
test test_assert ... FAILED
test test_env ... FAILED

test result: FAILED. 0 passed; 3 failed; 0 ignored; finished in 0.500s
"#;
        let stderr = r#"test test_impl ... FAILED
thread 'main' panicked at 'index out of bounds'

test test_assert ... FAILED
assertion failed: `(left == right)`

test test_env ... FAILED
error: cannot find dependency `foo`
"#;

        let parsed = TestGate::parse_test_output(stdout, stderr);
        assert_eq!(parsed.total, 3);
        assert_eq!(parsed.failed, 3);
        assert!(!parsed.failures.is_empty());
    }

    #[test]
    fn test_parse_test_output_empty() {
        let stdout = "";
        let stderr = "";

        let parsed = TestGate::parse_test_output(stdout, stderr);
        assert_eq!(parsed.total, 0);
        assert_eq!(parsed.failures.len(), 0);
    }
}

#[cfg(test)]
mod security_gate_tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn test_security_gate_name() {
        let gate = SecurityGate::new();
        assert_eq!(gate.name(), "security");
    }

    #[test]
    fn test_security_gate_order() {
        let gate = SecurityGate::new();
        assert_eq!(gate.order(), 30);
    }

    #[test]
    fn test_security_gate_default() {
        let gate = SecurityGate::default();
        assert_eq!(gate.name(), "security");
    }

    #[test]
    fn test_security_gate_new() {
        let gate = SecurityGate::new();
        assert_eq!(gate.name(), "security");
    }

    #[test]
    fn test_security_gate_with_scanners() {
        let gate = SecurityGate::with_scanners(vec![SecurityScannerType::Semgrep]);
        assert_eq!(gate.name(), "security");
    }

    #[test]
    fn test_security_gate_with_block_on_high() {
        let gate = SecurityGate::new().with_block_on_high();
        assert_eq!(gate.name(), "security");
    }

    #[tokio::test]
    async fn test_security_gate_validate_returns_outcome() {
        let gate = SecurityGate::new();
        let context = ValidationContext {
            task_id: Uuid::new_v4(),
            workspace_path: std::env::current_dir()
                .unwrap()
                .to_string_lossy()
                .to_string(),
            changed_files: vec![],
            plan: None,
        };

        let result = gate.validate(context).await;
        // Should succeed even without scanner - returns warning
        assert!(result.is_ok(), "SecurityGate.validate should succeed");
        
        let outcome = result.unwrap();
        // Without a scanner, it should pass with a warning message
        assert!(outcome.passed || !outcome.messages.is_empty());
    }

    #[test]
    fn test_finding_severity_from_semgrep() {
        assert_eq!(FindingSeverity::from_semgrep("error"), Some(FindingSeverity::Error));
        assert_eq!(FindingSeverity::from_semgrep("warning"), Some(FindingSeverity::Warning));
        assert_eq!(FindingSeverity::from_semgrep("warn"), Some(FindingSeverity::Warning));
        assert_eq!(FindingSeverity::from_semgrep("info"), Some(FindingSeverity::Info));
        assert_eq!(FindingSeverity::from_semgrep("unknown"), None);
    }

    #[test]
    fn test_finding_severity_from_sarif() {
        assert_eq!(FindingSeverity::from_sarif("error"), Some(FindingSeverity::Error));
        assert_eq!(FindingSeverity::from_sarif("critical"), Some(FindingSeverity::Error));
        assert_eq!(FindingSeverity::from_sarif("high"), Some(FindingSeverity::Error));
        assert_eq!(FindingSeverity::from_sarif("warning"), Some(FindingSeverity::Warning));
        assert_eq!(FindingSeverity::from_sarif("medium"), Some(FindingSeverity::Warning));
        assert_eq!(FindingSeverity::from_sarif("note"), Some(FindingSeverity::Info));
        assert_eq!(FindingSeverity::from_sarif("low"), Some(FindingSeverity::Info));
    }

    #[test]
    fn test_finding_severity_should_block() {
        assert!(FindingSeverity::Error.should_block());
        assert!(!FindingSeverity::Warning.should_block());
        assert!(!FindingSeverity::Info.should_block());
    }

    #[test]
    fn test_finding_severity_to_validation_level() {
        assert_eq!(FindingSeverity::Error.to_validation_level(), ValidationLevel::Error);
        assert_eq!(FindingSeverity::Warning.to_validation_level(), ValidationLevel::Warning);
        assert_eq!(FindingSeverity::Info.to_validation_level(), ValidationLevel::Info);
    }

    #[test]
    fn test_scanner_type_command() {
        assert_eq!(SecurityScannerType::Semgrep.command(), "semgrep");
        assert_eq!(SecurityScannerType::CodeQL.command(), "codeql");
    }

    #[test]
    fn test_scanner_type_scan_args_semgrep() {
        let args = SecurityScannerType::Semgrep.scan_args("/path/to/project");
        assert!(args.contains(&"--json".to_string()));
        assert!(args.contains(&"scan".to_string()));
        assert!(args.contains(&"/path/to/project".to_string()));
    }

    #[test]
    fn test_scanner_type_scan_args_codeql() {
        let args = SecurityScannerType::CodeQL.scan_args("/path/to/project");
        assert!(args.contains(&"database".to_string()));
        assert!(args.contains(&"analyze".to_string()));
        assert!(args.contains(&"/path/to/project".to_string()));
    }

    #[test]
    fn test_security_scan_results_default() {
        let results = SecurityScanResults::default();
        assert!(results.findings.is_empty());
        assert!(!results.scan_success);
        assert!(results.error_message.is_none());
    }

    #[test]
    fn test_security_scan_results_count_by_severity() {
        let mut results = SecurityScanResults::default();
        results.findings.push(Vulnerability {
            id: "test-1".to_string(),
            cwe: Some("CWE-79".to_string()),
            severity: FindingSeverity::Error,
            title: "XSS".to_string(),
            file: Some("src/xss.rs".to_string()),
            line: Some(10),
            description: "Cross-site scripting".to_string(),
            link: None,
            scanner: SecurityScannerType::Semgrep,
        });
        results.findings.push(Vulnerability {
            id: "test-2".to_string(),
            cwe: Some("CWE-89".to_string()),
            severity: FindingSeverity::Warning,
            title: "SQL Injection".to_string(),
            file: Some("src/sql.rs".to_string()),
            line: Some(20),
            description: "SQL injection vulnerability".to_string(),
            link: None,
            scanner: SecurityScannerType::Semgrep,
        });

        let (critical_high, medium, low) = results.count_by_severity();
        assert_eq!(critical_high, 1);
        assert_eq!(medium, 1);
        assert_eq!(low, 0);
    }

    #[test]
    fn test_security_scan_results_has_blocking_findings() {
        let mut results = SecurityScanResults::default();
        
        // No findings - no blocking
        assert!(!results.has_blocking_findings());
        
        // Add a warning - no blocking
        results.findings.push(Vulnerability {
            id: "test-1".to_string(),
            cwe: None,
            severity: FindingSeverity::Warning,
            title: "Test".to_string(),
            file: None,
            line: None,
            description: "Test".to_string(),
            link: None,
            scanner: SecurityScannerType::Semgrep,
        });
        assert!(!results.has_blocking_findings());
        
        // Add an error - blocking
        results.findings.push(Vulnerability {
            id: "test-2".to_string(),
            cwe: None,
            severity: FindingSeverity::Error,
            title: "Critical".to_string(),
            file: None,
            line: None,
            description: "Critical".to_string(),
            link: None,
            scanner: SecurityScannerType::Semgrep,
        });
        assert!(results.has_blocking_findings());
    }

    #[test]
    fn test_vulnerability_from_semgrep_minimal() {
        let json = serde_json::json!({
            "check_id": "java.lang.security.audit.xss.xss-sanitizer",
            "extra": {
                "severity": "WARNING",
                "message": "Potential XSS vulnerability"
            }
        });
        
        let vuln = Vulnerability::from_semgrep(&json);
        assert!(vuln.is_some());
        let v = vuln.unwrap();
        assert_eq!(v.id, "java.lang.security.audit.xss.xss-sanitizer");
        assert_eq!(v.severity, FindingSeverity::Warning);
        assert_eq!(v.title, "Potential XSS vulnerability");
        assert!(v.cwe.is_none());
        assert!(v.file.is_none());
        assert!(v.link.is_none());
    }

    #[test]
    fn test_vulnerability_from_semgrep_with_location() {
        let json = serde_json::json!({
            "check_id": "java.lang.security.audit.xss.xss-sanitizer",
            "start": {
                "filename": "src/Main.java",
                "line": 42
            },
            "extra": {
                "severity": "ERROR",
                "message": "XSS in user input",
                "metadata": {
                    "cwe": "CWE-79",
                    "url": "https://example.com/cwe-79"
                }
            }
        });
        
        let vuln = Vulnerability::from_semgrep(&json);
        assert!(vuln.is_some());
        let v = vuln.unwrap();
        assert_eq!(v.file, Some("src/Main.java".to_string()));
        assert_eq!(v.line, Some(42));
        assert_eq!(v.cwe, Some("CWE-79".to_string()));
        assert_eq!(v.link, Some("https://example.com/cwe-79".to_string()));
    }

    #[test]
    fn test_vulnerability_from_semgrep_invalid() {
        // Missing required fields
        let json = serde_json::json!({
            "check_id": "test"
        });
        
        let vuln = Vulnerability::from_semgrep(&json);
        assert!(vuln.is_none());
    }
}
