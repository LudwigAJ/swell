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
use swell_core::{
    ValidationGate, ValidationContext, ValidationOutcome, ValidationMessage,
    ValidationLevel, SwellError,
};
use std::process::Command;
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
    AiReviewEvidence, ConfidenceEvidence, CoverageEvidence,
    EvidenceOutcome, EvidencePack, EvidencePackBuilder, EvidencePackError, FlakinessEvidence,
    GateEvidence, MessageCounts, ReviewComment, SecurityEvidence, SecurityFinding,
    SignalScore, TestEvidence, TestResult,
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
        .map_err(|e| SwellError::IoError(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("Task join error: {}", e),
        )))?
        .map_err(|e| SwellError::IoError(e))?;

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
                            code: json.get("code").and_then(|c| c.get("code")).and_then(|c| c.as_str()).map(String::from),
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

/// Gate that runs tests for the workspace.
pub struct TestGate;

impl TestGate {
    pub fn new() -> Self {
        Self
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
        .map_err(|e| SwellError::IoError(std::io::Error::new(
            std::io::ErrorKind::Other,
            format!("Task join error: {}", e),
        )))?
        .map_err(|e| SwellError::IoError(e))?;

        let passed = output.status.success();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        let mut messages = Vec::new();

        if !passed {
            messages.push(ValidationMessage {
                level: ValidationLevel::Error,
                code: None,
                message: format!("Test suite failed:\n{}", stderr),
                file: None,
                line: None,
            });
        } else {
            messages.push(ValidationMessage {
                level: ValidationLevel::Info,
                code: None,
                message: format!("All tests passed:\n{}", stdout.lines().last().unwrap_or("Tests passed")),
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
// Security Gate (Stub)
// ============================================================================

/// Gate that runs security scans (stub implementation for MVP).
pub struct SecurityGate;

impl SecurityGate {
    pub fn new() -> Self {
        Self
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

    async fn validate(&self, _context: ValidationContext) -> Result<ValidationOutcome, SwellError> {
        // Stub implementation - security scanning not yet implemented
        Ok(ValidationOutcome {
            passed: true,
            messages: vec![
                ValidationMessage {
                    level: ValidationLevel::Info,
                    code: None,
                    message: "Security gate stub: full security scanning not yet implemented".to_string(),
                    file: None,
                    line: None,
                },
            ],
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
            messages: vec![
                ValidationMessage {
                    level: ValidationLevel::Info,
                    code: None,
                    message: "AI review gate stub: full AI review not yet implemented".to_string(),
                    file: None,
                    line: None,
                },
            ],
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


