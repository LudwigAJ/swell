//! Evidence pack generation for validation results.
//!
//! Bundles all validation signals, test results, and artifacts into
//! an immutable evidence pack suitable for PR review and audit trails.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// An evidence pack containing all validation data for a task
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidencePack {
    /// Unique identifier for this evidence pack
    pub id: Uuid,
    /// Task ID this evidence is for
    pub task_id: Uuid,
    /// When this evidence was created
    pub created_at: DateTime<Utc>,
    /// Validation outcome summary
    pub outcome: EvidenceOutcome,
    /// Individual gate results
    pub gate_results: Vec<GateEvidence>,
    /// Test results summary
    pub test_summary: TestEvidence,
    /// Coverage data if available
    pub coverage: Option<CoverageEvidence>,
    /// Security scan results
    pub security: SecurityEvidence,
    /// AI review results
    pub ai_review: Option<AiReviewEvidence>,
    /// Confidence score
    pub confidence: ConfidenceEvidence,
    /// Flakiness analysis
    pub flakiness: FlakinessEvidence,
    /// Additional metadata
    pub metadata: HashMap<String, String>,
}

/// Overall validation outcome
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvidenceOutcome {
    /// All gates passed
    Passed,
    /// Some gates failed
    Failed,
    /// Validation was skipped or inconclusive
    Skipped,
    /// Validation encountered an error
    Error,
}

/// Evidence from a single validation gate
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GateEvidence {
    /// Gate name
    pub name: String,
    /// Whether the gate passed
    pub passed: bool,
    /// Number of messages by level
    pub message_counts: MessageCounts,
    /// Gate-specific details
    pub details: HashMap<String, String>,
}

/// Count of messages by level
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MessageCounts {
    pub errors: usize,
    pub warnings: usize,
    pub info: usize,
}

impl MessageCounts {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_error(&mut self) {
        self.errors += 1;
    }

    pub fn add_warning(&mut self) {
        self.warnings += 1;
    }

    pub fn add_info(&mut self) {
        self.info += 1;
    }
}

/// Test execution evidence
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestEvidence {
    /// Total tests run
    pub total: usize,
    /// Tests that passed
    pub passed: usize,
    /// Tests that failed
    pub failed: usize,
    /// Tests that were skipped
    pub skipped: usize,
    /// Total duration in milliseconds
    pub duration_ms: u64,
    /// Individual test results
    pub tests: Vec<TestResult>,
}

/// Individual test result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestResult {
    /// Test name (fully qualified)
    pub name: String,
    /// Whether test passed
    pub passed: bool,
    /// Duration in milliseconds
    pub duration_ms: u64,
    /// Failure message if applicable
    pub failure_message: Option<String>,
}

/// Coverage evidence
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoverageEvidence {
    /// Line coverage percentage
    pub line_coverage: f64,
    /// Branch coverage percentage
    pub branch_coverage: f64,
    /// Function coverage percentage
    pub function_coverage: f64,
    /// Coverage report file path
    pub report_path: Option<String>,
}

/// Security scan evidence
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityEvidence {
    /// Whether scan passed
    pub passed: bool,
    /// Critical findings
    pub critical: usize,
    /// High severity findings
    pub high: usize,
    /// Medium severity findings
    pub medium: usize,
    /// Low severity findings
    pub low: usize,
    /// Findings by category
    pub findings: Vec<SecurityFinding>,
}

/// A security finding
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityFinding {
    /// Finding ID
    pub id: String,
    /// CWE category
    pub cwe: Option<String>,
    /// Severity
    pub severity: String,
    /// Title
    pub title: String,
    /// File where found
    pub file: Option<String>,
    /// Line number
    pub line: Option<u32>,
    /// Description
    pub description: String,
}

/// AI review evidence
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiReviewEvidence {
    /// Whether AI review passed
    pub passed: bool,
    /// Overall confidence score (0-1)
    pub confidence_score: f64,
    /// Number of issues found
    pub issues_found: usize,
    /// Issue categories
    pub issue_categories: Vec<String>,
    /// Review comments
    pub comments: Vec<ReviewComment>,
}

/// A review comment from AI
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewComment {
    /// File associated with comment
    pub file: Option<String>,
    /// Line number
    pub line: Option<u32>,
    /// Comment text
    pub text: String,
    /// Severity level
    pub severity: String,
}

/// Confidence evidence
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfidenceEvidence {
    /// Overall confidence score (0-1)
    pub score: f64,
    /// Whether eligible for auto-merge
    pub auto_merge: bool,
    /// Individual signal scores
    pub signals: Vec<SignalScore>,
}

/// Individual signal score
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalScore {
    /// Signal name
    pub name: String,
    /// Score value (0-1)
    pub score: f64,
    /// Weight of this signal
    pub weight: f64,
}

/// Flakiness evidence
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlakinessEvidence {
    /// Whether any flaky tests detected
    pub has_flaky: bool,
    /// Tests marked as flaky
    pub flaky_tests: Vec<String>,
    /// Tests quarantined
    pub quarantined: Vec<String>,
    /// Flakiness score per test
    pub scores: HashMap<String, f64>,
}

/// Builder for creating evidence packs
#[derive(Debug, Default)]
pub struct EvidencePackBuilder {
    task_id: Option<Uuid>,
    gate_results: Vec<GateEvidence>,
    test_summary: Option<TestEvidence>,
    coverage: Option<CoverageEvidence>,
    security: Option<SecurityEvidence>,
    ai_review: Option<AiReviewEvidence>,
    confidence_score: Option<ConfidenceEvidence>,
    flakiness: Option<FlakinessEvidence>,
    metadata: HashMap<String, String>,
}

impl EvidencePackBuilder {
    /// Create a new builder
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the task ID
    pub fn task_id(mut self, id: Uuid) -> Self {
        self.task_id = Some(id);
        self
    }

    /// Add gate evidence
    pub fn add_gate_result(mut self, result: GateEvidence) -> Self {
        self.gate_results.push(result);
        self
    }

    /// Set test evidence
    pub fn test_summary(mut self, summary: TestEvidence) -> Self {
        self.test_summary = Some(summary);
        self
    }

    /// Set coverage evidence
    pub fn coverage(mut self, coverage: CoverageEvidence) -> Self {
        self.coverage = Some(coverage);
        self
    }

    /// Set security evidence
    pub fn security(mut self, security: SecurityEvidence) -> Self {
        self.security = Some(security);
        self
    }

    /// Set AI review evidence
    pub fn ai_review(mut self, review: AiReviewEvidence) -> Self {
        self.ai_review = Some(review);
        self
    }

    /// Set confidence evidence
    pub fn confidence_score(mut self, confidence: ConfidenceEvidence) -> Self {
        self.confidence_score = Some(confidence);
        self
    }

    /// Set flakiness evidence
    pub fn flakiness(mut self, flakiness: FlakinessEvidence) -> Self {
        self.flakiness = Some(flakiness);
        self
    }

    /// Add metadata
    pub fn metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    /// Build the evidence pack
    pub fn build(self) -> Result<EvidencePack, EvidencePackError> {
        let task_id = self.task_id.ok_or(EvidencePackError::MissingTaskId)?;

        let outcome = self.determine_outcome();

        Ok(EvidencePack {
            id: Uuid::new_v4(),
            task_id,
            created_at: Utc::now(),
            outcome,
            gate_results: self.gate_results,
            test_summary: self.test_summary.unwrap_or(TestEvidence {
                total: 0,
                passed: 0,
                failed: 0,
                skipped: 0,
                duration_ms: 0,
                tests: vec![],
            }),
            coverage: self.coverage,
            security: self.security.unwrap_or(SecurityEvidence {
                passed: true,
                critical: 0,
                high: 0,
                medium: 0,
                low: 0,
                findings: vec![],
            }),
            ai_review: self.ai_review,
            confidence: self.confidence_score.unwrap_or(ConfidenceEvidence {
                score: 1.0,
                auto_merge: true,
                signals: vec![],
            }),
            flakiness: self.flakiness.unwrap_or(FlakinessEvidence {
                has_flaky: false,
                flaky_tests: vec![],
                quarantined: vec![],
                scores: HashMap::new(),
            }),
            metadata: self.metadata,
        })
    }

    fn determine_outcome(&self) -> EvidenceOutcome {
        // If any gate failed, outcome is failed
        if self.gate_results.iter().any(|g| !g.passed) {
            return EvidenceOutcome::Failed;
        }

        // If test summary shows failures
        if let Some(ref test) = self.test_summary {
            if test.failed > 0 {
                return EvidenceOutcome::Failed;
            }
        }

        // If security has critical issues
        if let Some(ref security) = self.security {
            if security.critical > 0 {
                return EvidenceOutcome::Failed;
            }
        }

        EvidenceOutcome::Passed
    }
}

/// Errors creating evidence pack
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EvidencePackError {
    /// Missing required task ID
    MissingTaskId,
    /// Invalid data in builder
    InvalidData(String),
}

impl std::fmt::Display for EvidencePackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EvidencePackError::MissingTaskId => write!(f, "Task ID is required"),
            EvidencePackError::InvalidData(msg) => write!(f, "Invalid data: {}", msg),
        }
    }
}

impl std::error::Error for EvidencePackError {}

impl EvidencePack {
    /// Generate a markdown summary suitable for PR comments
    pub fn to_markdown(&self) -> String {
        let mut md = String::new();

        md.push_str("# Validation Evidence\n\n");

        // Outcome badge
        let (emoji, status) = match self.outcome {
            EvidenceOutcome::Passed => ("✅", "PASSED"),
            EvidenceOutcome::Failed => ("❌", "FAILED"),
            EvidenceOutcome::Skipped => ("⏭️", "SKIPPED"),
            EvidenceOutcome::Error => ("💥", "ERROR"),
        };
        md.push_str(&format!("## {} {}\n\n", emoji, status));

        // Confidence
        md.push_str("## Confidence\n\n");
        md.push_str(&format!(
            "- **Score**: {:.1}%\n- **Auto-merge**: {}\n\n",
            self.confidence.score * 100.0,
            if self.confidence.auto_merge { "Yes" } else { "No" }
        ));

        // Test summary
        md.push_str("## Test Results\n\n");
        let ts = &self.test_summary;
        md.push_str(&format!(
            "- **Total**: {} | **Passed**: {} | **Failed**: {} | **Skipped**: {}\n",
            ts.total, ts.passed, ts.failed, ts.skipped
        ));
        md.push_str(&format!("- **Duration**: {:.2}s\n\n", ts.duration_ms as f64 / 1000.0));

        // Security
        md.push_str("## Security\n\n");
        md.push_str(&format!(
            "- **Critical**: {} | **High**: {} | **Medium**: {} | **Low**: {}\n\n",
            self.security.critical, self.security.high, self.security.medium, self.security.low
        ));

        // Gate results
        md.push_str("## Validation Gates\n\n");
        md.push_str("| Gate | Result | Errors | Warnings |\n");
        md.push_str("|------|--------|--------|----------|\n");
        for gate in &self.gate_results {
            let result = if gate.passed { "✅ PASS" } else { "❌ FAIL" };
            md.push_str(&format!(
                "| {} | {} | {} | {} |\n",
                gate.name, result, gate.message_counts.errors, gate.message_counts.warnings
            ));
        }
        md.push('\n');

        // Flaky tests
        if self.flakiness.has_flaky {
            md.push_str("## ⚠️ Flaky Tests Detected\n\n");
            for test in &self.flakiness.flaky_tests {
                md.push_str(&format!("- `{}`\n", test));
            }
            md.push('\n');
        }

        // Metadata
        if !self.metadata.is_empty() {
            md.push_str("## Metadata\n\n");
            for (key, value) in &self.metadata {
                md.push_str(&format!("- **{}**: {}\n", key, value));
            }
            md.push('\n');
        }

        md.push_str(&format!("\n---\n*Evidence pack ID: {}*\n", self.id));

        md
    }

    /// Generate a JSON summary for API responses
    pub fn to_summary_json(&self) -> serde_json::Value {
        serde_json::json!({
            "id": self.id.to_string(),
            "task_id": self.task_id.to_string(),
            "created_at": self.created_at.to_rfc3339(),
            "outcome": match self.outcome {
                EvidenceOutcome::Passed => "passed",
                EvidenceOutcome::Failed => "failed",
                EvidenceOutcome::Skipped => "skipped",
                EvidenceOutcome::Error => "error",
            },
            "confidence": {
                "score": self.confidence.score,
                "auto_merge": self.confidence.auto_merge,
            },
            "test_summary": {
                "total": self.test_summary.total,
                "passed": self.test_summary.passed,
                "failed": self.test_summary.failed,
                "skipped": self.test_summary.skipped,
                "duration_ms": self.test_summary.duration_ms,
            },
            "security": {
                "passed": self.security.passed,
                "critical": self.security.critical,
                "high": self.security.high,
                "medium": self.security.medium,
                "low": self.security.low,
            },
            "flaky_tests": self.flakiness.flaky_tests,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_evidence_pack_builder() {
        let pack = EvidencePackBuilder::new()
            .task_id(Uuid::new_v4())
            .add_gate_result(GateEvidence {
                name: "lint".to_string(),
                passed: true,
                message_counts: MessageCounts::new(),
                details: HashMap::new(),
            })
            .add_gate_result(GateEvidence {
                name: "test".to_string(),
                passed: true,
                message_counts: MessageCounts::new(),
                details: HashMap::new(),
            })
            .test_summary(TestEvidence {
                total: 10,
                passed: 10,
                failed: 0,
                skipped: 0,
                duration_ms: 500,
                tests: vec![],
            })
            .security(SecurityEvidence {
                passed: true,
                critical: 0,
                high: 0,
                medium: 0,
                low: 0,
                findings: vec![],
            })
            .confidence_score(ConfidenceEvidence {
                score: 0.9,
                auto_merge: true,
                signals: vec![],
            })
            .build()
            .unwrap();

        assert_eq!(pack.outcome, EvidenceOutcome::Passed);
        assert!(pack.confidence.auto_merge);
    }

    #[test]
    fn test_evidence_pack_failed_outcome() {
        let pack = EvidencePackBuilder::new()
            .task_id(Uuid::new_v4())
            .add_gate_result(GateEvidence {
                name: "lint".to_string(),
                passed: false,
                message_counts: {
                    let mut c = MessageCounts::new();
                    c.add_error();
                    c
                },
                details: HashMap::new(),
            })
            .build()
            .unwrap();

        assert_eq!(pack.outcome, EvidenceOutcome::Failed);
    }

    #[test]
    fn test_to_markdown() {
        let pack = EvidencePackBuilder::new()
            .task_id(Uuid::new_v4())
            .test_summary(TestEvidence {
                total: 5,
                passed: 4,
                failed: 1,
                skipped: 0,
                duration_ms: 1000,
                tests: vec![],
            })
            .confidence_score(ConfidenceEvidence {
                score: 0.75,
                auto_merge: false,
                signals: vec![],
            })
            .build()
            .unwrap();

        let md = pack.to_markdown();
        assert!(md.contains("FAILED"));
        assert!(md.contains("75"));
    }

    #[test]
    fn test_to_summary_json() {
        let pack = EvidencePackBuilder::new()
            .task_id(Uuid::new_v4())
            .test_summary(TestEvidence {
                total: 3,
                passed: 3,
                failed: 0,
                skipped: 0,
                duration_ms: 200,
                tests: vec![],
            })
            .build()
            .unwrap();

        let json = pack.to_summary_json();
        assert_eq!(json["outcome"], "passed");
        assert_eq!(json["test_summary"]["total"], 3);
    }

    #[test]
    fn test_builder_error_no_task_id() {
        let result = EvidencePackBuilder::new().build();
        assert!(result.is_err());
    }
}
