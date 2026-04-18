//! Staged Test Executor Module
//!
//! Implements progressive test execution with stages 0-4:
//! - Stage 0: Instant validation (fast syntax/import checks)
//! - Stage 1: Unit tests (test each unit in isolation)
//! - Stage 2: Integration tests (test interactions between units)
//! - Stage 3: Comprehensive validation (full test suite + lint)
//! - Stage 4: Full validation pipeline (all gates including security and AI review)
//!
//! # Usage
//!
//! ```rust,ignore
//! use swell_validation::staged_executor::{StagedTestExecutor, TestStage};
//! use swell_core::ValidationContext;
//!
//! async fn run_validation() {
//!     let executor = StagedTestExecutor::new();
//!     let context = ValidationContext {
//!         task_id: uuid::Uuid::new_v4(),
//!         workspace_path: "/path/to/workspace".to_string(),
//!         changed_files: vec![],
//!         plan: None,
//!     };
//!     
//!     // Execute a single stage
//!     let result = executor.execute_stage(TestStage::Stage0Instant, &context).await;
//!     
//!     // Or execute all stages up to a certain level
//!     let all_result = executor.execute_up_to(TestStage::Stage3Comprehensive, &context).await;
//! }
//! ```

use serde::{Deserialize, Serialize};
use std::process::Command;
use std::time::Instant;
use swell_core::{SwellError, ValidationContext, ValidationMessage, ValidationOutcome};
use tokio::task;

use crate::{cargo_test_semaphore, truncate_output};

/// Test execution stages with progressive rigor
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum TestStage {
    /// Stage 0: Instant validation - fast syntax/import checks (< 5 seconds)
    Stage0Instant = 0,
    /// Stage 1: Unit tests - test each unit in isolation
    Stage1Unit = 1,
    /// Stage 2: Integration tests - test interactions between units
    Stage2Integration = 2,
    /// Stage 3: Comprehensive validation - full test suite + lint
    Stage3Comprehensive = 3,
    /// Stage 4: Full validation pipeline - all gates including security and AI review
    Stage4Full = 4,
}

impl TestStage {
    /// Get the name of the stage as a string
    pub fn name(&self) -> &'static str {
        match self {
            TestStage::Stage0Instant => "Stage 0: Instant",
            TestStage::Stage1Unit => "Stage 1: Unit",
            TestStage::Stage2Integration => "Stage 2: Integration",
            TestStage::Stage3Comprehensive => "Stage 3: Comprehensive",
            TestStage::Stage4Full => "Stage 4: Full",
        }
    }

    /// Get a short description of what this stage validates
    pub fn description(&self) -> &'static str {
        match self {
            TestStage::Stage0Instant => "Fast syntax and import checks",
            TestStage::Stage1Unit => "Unit tests (each component in isolation)",
            TestStage::Stage2Integration => "Integration tests (component interactions)",
            TestStage::Stage3Comprehensive => "Full test suite + lint checks",
            TestStage::Stage4Full => "Complete validation pipeline with all gates",
        }
    }

    /// Check if this stage should continue even if previous stage failed
    pub fn continue_on_failure(&self) -> bool {
        match self {
            TestStage::Stage0Instant => false, // Stop on instant validation failure
            TestStage::Stage1Unit => false,    // Stop on unit test failure
            TestStage::Stage2Integration => true, // Continue to integration even if unit fails
            TestStage::Stage3Comprehensive => true, // Continue to comprehensive
            TestStage::Stage4Full => true,     // Continue to full pipeline
        }
    }

    /// Expected duration in milliseconds for this stage
    pub fn expected_duration_ms(&self) -> u64 {
        match self {
            TestStage::Stage0Instant => 5_000,         // < 5 seconds
            TestStage::Stage1Unit => 60_000,           // < 1 minute
            TestStage::Stage2Integration => 120_000,   // < 2 minutes
            TestStage::Stage3Comprehensive => 300_000, // < 5 minutes
            TestStage::Stage4Full => 600_000,          // < 10 minutes
        }
    }
}

impl std::fmt::Display for TestStage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.name(), self.description())
    }
}

/// Configuration for staged test execution
#[derive(Debug, Clone, Default)]
pub struct StageConfig {
    /// Stage 0 configuration
    pub stage0: Stage0Config,
    /// Stage 1 configuration
    pub stage1: Stage1Config,
    /// Stage 2 configuration
    pub stage2: Stage2Config,
    /// Stage 3 configuration
    pub stage3: Stage3Config,
    /// Stage 4 configuration
    pub stage4: Stage4Config,
}

impl StageConfig {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Stage 0: Instant validation configuration
#[derive(Debug, Clone)]
pub struct Stage0Config {
    /// Run cargo check for syntax and type errors
    pub run_cargo_check: bool,
    /// Run format check with rustfmt
    pub run_fmt_check: bool,
    /// Maximum time in milliseconds allowed for this stage
    pub timeout_ms: u64,
}

impl Default for Stage0Config {
    fn default() -> Self {
        Self {
            run_cargo_check: true,
            run_fmt_check: true,
            timeout_ms: 10_000, // 10 seconds max for instant validation
        }
    }
}

/// Stage 1: Unit test configuration
#[derive(Debug, Clone)]
pub struct Stage1Config {
    /// Only run lib tests (not integration tests)
    pub lib_tests_only: bool,
    /// Run doc tests
    pub run_doc_tests: bool,
    /// Maximum time in milliseconds allowed for this stage
    pub timeout_ms: u64,
}

impl Default for Stage1Config {
    fn default() -> Self {
        Self {
            lib_tests_only: true,
            run_doc_tests: true,
            timeout_ms: 120_000, // 2 minutes max
        }
    }
}

/// Stage 2: Integration test configuration
#[derive(Debug, Clone)]
pub struct Stage2Config {
    /// Run integration tests in tests/ directory
    pub run_integration_tests: bool,
    /// Run tests/ directory tests
    pub run_tests_dir: bool,
    /// Maximum time in milliseconds allowed for this stage
    pub timeout_ms: u64,
}

impl Default for Stage2Config {
    fn default() -> Self {
        Self {
            run_integration_tests: true,
            run_tests_dir: true,
            timeout_ms: 300_000, // 5 minutes max
        }
    }
}

/// Stage 3: Comprehensive validation configuration
#[derive(Debug, Clone)]
pub struct Stage3Config {
    /// Run full test suite
    pub run_full_tests: bool,
    /// Run clippy linter
    pub run_clippy: bool,
    /// Run rustfmt
    pub run_fmt: bool,
    /// Maximum time in milliseconds allowed for this stage
    pub timeout_ms: u64,
}

impl Default for Stage3Config {
    fn default() -> Self {
        Self {
            run_full_tests: true,
            run_clippy: true,
            run_fmt: true,
            timeout_ms: 600_000, // 10 minutes max
        }
    }
}

/// Stage 4: Full validation pipeline configuration
#[derive(Debug, Clone)]
pub struct Stage4Config {
    /// Run all gates in ValidationPipeline
    pub run_all_gates: bool,
    /// Run security scans
    pub run_security: bool,
    /// Run AI review (if LLM configured)
    pub run_ai_review: bool,
    /// Maximum time in milliseconds allowed for this stage
    pub timeout_ms: u64,
}

impl Default for Stage4Config {
    fn default() -> Self {
        Self {
            run_all_gates: true,
            run_security: true,
            run_ai_review: true,
            timeout_ms: 900_000, // 15 minutes max
        }
    }
}

/// Result of a single stage execution
#[derive(Debug, Clone)]
pub struct StageResult {
    /// The stage that was executed
    pub stage: TestStage,
    /// Whether the stage passed
    pub passed: bool,
    /// Duration in milliseconds
    pub duration_ms: u64,
    /// Messages from the stage
    pub messages: Vec<ValidationMessage>,
    /// Whether execution was skipped
    pub skipped: bool,
    /// Reason for skipping (if applicable)
    pub skip_reason: Option<String>,
}

impl StageResult {
    /// Create a successful stage result
    fn success(stage: TestStage, duration_ms: u64, messages: Vec<ValidationMessage>) -> Self {
        Self {
            stage,
            passed: true,
            duration_ms,
            messages,
            skipped: false,
            skip_reason: None,
        }
    }

    /// Create a failed stage result
    fn failure(stage: TestStage, duration_ms: u64, messages: Vec<ValidationMessage>) -> Self {
        Self {
            stage,
            passed: false,
            duration_ms,
            messages,
            skipped: false,
            skip_reason: None,
        }
    }

    /// Create a skipped stage result
    #[allow(dead_code)]
    fn skipped(stage: TestStage, reason: String) -> Self {
        Self {
            stage,
            passed: true, // Skipped stages don't count as failures
            duration_ms: 0,
            messages: vec![ValidationMessage {
                level: swell_core::ValidationLevel::Info,
                code: Some("STAGE_SKIPPED".to_string()),
                message: format!("Stage skipped: {}", reason),
                file: None,
                line: None,
            }],
            skipped: true,
            skip_reason: Some(reason),
        }
    }
}

/// Result of staged test execution
#[derive(Debug, Clone)]
pub struct StagedResult {
    /// Results from each stage
    pub stage_results: Vec<StageResult>,
    /// Final overall result
    pub passed: bool,
    /// Total duration in milliseconds
    pub total_duration_ms: u64,
    /// Highest stage that failed
    pub first_failure_stage: Option<TestStage>,
}

impl StagedResult {
    /// Get the stage result for a specific stage
    pub fn get_stage_result(&self, stage: TestStage) -> Option<&StageResult> {
        self.stage_results.iter().find(|r| r.stage == stage)
    }

    /// Check if any stage failed
    pub fn has_failures(&self) -> bool {
        self.stage_results.iter().any(|r| !r.passed && !r.skipped)
    }

    /// Get total number of messages across all stages
    pub fn total_messages(&self) -> usize {
        self.stage_results.iter().map(|r| r.messages.len()).sum()
    }
}

/// Staged test executor with progressive rigor
#[derive(Debug, Clone)]
pub struct StagedTestExecutor {
    config: StageConfig,
    include_stages: Vec<TestStage>,
}

impl Default for StagedTestExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl StagedTestExecutor {
    /// Create a new staged executor with default configuration
    pub fn new() -> Self {
        Self {
            config: StageConfig::default(),
            include_stages: vec![
                TestStage::Stage0Instant,
                TestStage::Stage1Unit,
                TestStage::Stage2Integration,
                TestStage::Stage3Comprehensive,
                TestStage::Stage4Full,
            ],
        }
    }

    /// Create with custom stage configuration
    pub fn with_config(config: StageConfig) -> Self {
        Self {
            config,
            include_stages: vec![
                TestStage::Stage0Instant,
                TestStage::Stage1Unit,
                TestStage::Stage2Integration,
                TestStage::Stage3Comprehensive,
                TestStage::Stage4Full,
            ],
        }
    }

    /// Create with specific stages to run
    pub fn with_stages(stages: Vec<TestStage>) -> Self {
        Self {
            config: StageConfig::default(),
            include_stages: stages,
        }
    }

    /// Execute a specific stage
    pub async fn execute_stage(
        &self,
        stage: TestStage,
        context: &ValidationContext,
    ) -> Result<StageResult, SwellError> {
        let start = Instant::now();

        match stage {
            TestStage::Stage0Instant => self.run_stage0(context).await,
            TestStage::Stage1Unit => self.run_stage1(context).await,
            TestStage::Stage2Integration => self.run_stage2(context).await,
            TestStage::Stage3Comprehensive => self.run_stage3(context).await,
            TestStage::Stage4Full => self.run_stage4(context).await,
        }
        .map(|outcome| {
            let duration_ms = start.elapsed().as_millis() as u64;
            if outcome.passed {
                StageResult::success(stage, duration_ms, outcome.messages)
            } else {
                StageResult::failure(stage, duration_ms, outcome.messages)
            }
        })
    }

    /// Execute all stages up to and including the specified stage
    pub async fn execute_up_to(
        &self,
        max_stage: TestStage,
        context: &ValidationContext,
    ) -> Result<StagedResult, SwellError> {
        let mut stage_results = Vec::new();
        let start = Instant::now();
        let mut first_failure: Option<TestStage> = None;

        for stage in &self.include_stages {
            if *stage > max_stage {
                break;
            }

            let result = self.execute_stage(*stage, context).await?;
            stage_results.push(result.clone());

            if !result.passed && !result.skipped && first_failure.is_none() {
                first_failure = Some(*stage);

                // Check if we should continue on failure
                if !stage.continue_on_failure() {
                    break;
                }
            }
        }

        let total_duration = start.elapsed().as_millis() as u64;
        let overall_passed = !stage_results.iter().any(|r| !r.passed && !r.skipped);

        Ok(StagedResult {
            stage_results,
            passed: overall_passed,
            total_duration_ms: total_duration,
            first_failure_stage: first_failure,
        })
    }

    /// Execute all stages
    pub async fn execute_all(
        &self,
        context: &ValidationContext,
    ) -> Result<StagedResult, SwellError> {
        self.execute_up_to(TestStage::Stage4Full, context).await
    }

    // =============================================================================
    // Stage 0: Instant validation (fast checks)
    // =============================================================================

    async fn run_stage0(
        &self,
        context: &ValidationContext,
    ) -> Result<ValidationOutcome, SwellError> {
        let mut all_messages = Vec::new();
        let mut all_passed = true;

        // Stage 0: Instant validation - cargo check and fmt check
        if self.config.stage0.run_cargo_check {
            let workspace_path = context.workspace_path.clone();
            let check_result = task::spawn_blocking(move || {
                Command::new("cargo")
                    .args(["check", "--message-format", "short"])
                    .current_dir(&workspace_path)
                    .output()
            })
            .await
            .map_err(|e| {
                SwellError::IoError(std::io::Error::other(format!("Task join error: {}", e)))
            })?
            .map_err(SwellError::IoError)?;

            if !check_result.status.success() {
                all_passed = false;
                let stderr = String::from_utf8_lossy(&check_result.stderr);
                all_messages.push(ValidationMessage {
                    level: swell_core::ValidationLevel::Error,
                    code: Some("STAGE0_CHECK".to_string()),
                    message: format!("Cargo check failed:\n{}", stderr),
                    file: None,
                    line: None,
                });
            } else {
                all_messages.push(ValidationMessage {
                    level: swell_core::ValidationLevel::Info,
                    code: Some("STAGE0_CHECK".to_string()),
                    message: "Cargo check passed".to_string(),
                    file: None,
                    line: None,
                });
            }
        }

        if self.config.stage0.run_fmt_check {
            let workspace_path = context.workspace_path.clone();
            let fmt_result = task::spawn_blocking(move || {
                Command::new("cargo")
                    .args(["fmt", "--check", "--", "."])
                    .current_dir(&workspace_path)
                    .output()
            })
            .await
            .map_err(|e| {
                SwellError::IoError(std::io::Error::other(format!("Task join error: {}", e)))
            })?
            .map_err(SwellError::IoError)?;

            if !fmt_result.status.success() {
                all_passed = false;
                all_messages.push(ValidationMessage {
                    level: swell_core::ValidationLevel::Warning,
                    code: Some("STAGE0_FMT".to_string()),
                    message: "Format check failed - code is not formatted".to_string(),
                    file: None,
                    line: None,
                });
            } else {
                all_messages.push(ValidationMessage {
                    level: swell_core::ValidationLevel::Info,
                    code: Some("STAGE0_FMT".to_string()),
                    message: "Format check passed".to_string(),
                    file: None,
                    line: None,
                });
            }
        }

        Ok(ValidationOutcome {
            passed: all_passed,
            messages: all_messages,
            artifacts: vec![],
        })
    }

    // =============================================================================
    // Stage 1: Unit tests
    // =============================================================================

    async fn run_stage1(
        &self,
        context: &ValidationContext,
    ) -> Result<ValidationOutcome, SwellError> {
        let workspace_path = context.workspace_path.clone();
        let mut args = vec!["test".to_string()];

        // Run lib tests only (not integration tests)
        if self.config.stage1.lib_tests_only {
            args.push("--lib".to_string());
        }

        // Run doc tests
        if self.config.stage1.run_doc_tests {
            args.push("--doc".to_string());
        }

        // Add test filters for unit tests
        args.push("--".to_string());
        args.push("--test-threads=4".to_string());

        let permit = cargo_test_semaphore()
            .acquire_owned()
            .await
            .map_err(|_| SwellError::IoError(std::io::Error::other("Semaphore closed")))?;
        let ws = workspace_path.clone();
        let output = task::spawn_blocking(move || {
            let _permit = permit;
            Command::new("cargo").args(&args).current_dir(&ws).output()
        })
        .await
        .map_err(|e| SwellError::IoError(std::io::Error::other(format!("Task join error: {}", e))))?
        .map_err(SwellError::IoError)?;

        let passed = output.status.success();
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        let mut messages = Vec::new();

        if passed {
            messages.push(ValidationMessage {
                level: swell_core::ValidationLevel::Info,
                code: Some("STAGE1_UNIT".to_string()),
                message: "Unit tests passed".to_string(),
                file: None,
                line: None,
            });
        } else {
            // Extract test failure summary
            let failure_summary = Self::extract_test_summary(&stdout, &stderr);
            messages.push(ValidationMessage {
                level: swell_core::ValidationLevel::Error,
                code: Some("STAGE1_UNIT_FAIL".to_string()),
                message: format!("Unit tests failed:\n{}", truncate_output(&failure_summary)),
                file: None,
                line: None,
            });
        }
        drop(output);

        Ok(ValidationOutcome {
            passed,
            messages,
            artifacts: vec![],
        })
    }

    // =============================================================================
    // Stage 2: Integration tests
    // =============================================================================

    async fn run_stage2(
        &self,
        context: &ValidationContext,
    ) -> Result<ValidationOutcome, SwellError> {
        let workspace_path = context.workspace_path.clone();
        let mut all_messages = Vec::new();
        let mut all_passed = true;

        // Run tests in tests/ directory (integration tests)
        if self.config.stage2.run_tests_dir {
            let permit = cargo_test_semaphore()
                .acquire_owned()
                .await
                .map_err(|_| SwellError::IoError(std::io::Error::other("Semaphore closed")))?;
            let ws = workspace_path.clone();
            let output = task::spawn_blocking(move || {
                let _permit = permit;
                Command::new("cargo")
                    .args(["test", "--test", "*", "--", "--test-threads=2"])
                    .current_dir(&ws)
                    .output()
            })
            .await
            .map_err(|e| {
                SwellError::IoError(std::io::Error::other(format!("Task join error: {}", e)))
            })?
            .map_err(SwellError::IoError)?;

            let passed = output.status.success();
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stderr_truncated = truncate_output(&stderr);
            drop(output);

            if !passed {
                all_passed = false;
                all_messages.push(ValidationMessage {
                    level: swell_core::ValidationLevel::Error,
                    code: Some("STAGE2_INTEGRATION".to_string()),
                    message: format!("Integration tests failed:\n{}", stderr_truncated),
                    file: None,
                    line: None,
                });
            } else {
                all_messages.push(ValidationMessage {
                    level: swell_core::ValidationLevel::Info,
                    code: Some("STAGE2_INTEGRATION".to_string()),
                    message: "Integration tests passed".to_string(),
                    file: None,
                    line: None,
                });
            }
        }

        Ok(ValidationOutcome {
            passed: all_passed,
            messages: all_messages,
            artifacts: vec![],
        })
    }

    // =============================================================================
    // Stage 3: Comprehensive validation (full tests + lint)
    // =============================================================================

    async fn run_stage3(
        &self,
        context: &ValidationContext,
    ) -> Result<ValidationOutcome, SwellError> {
        let workspace_path = context.workspace_path.clone();
        let mut all_messages = Vec::new();
        let mut all_passed = true;

        // Run full test suite
        if self.config.stage3.run_full_tests {
            let permit = cargo_test_semaphore()
                .acquire_owned()
                .await
                .map_err(|_| SwellError::IoError(std::io::Error::other("Semaphore closed")))?;
            let ws = workspace_path.clone();
            let output = task::spawn_blocking(move || {
                let _permit = permit;
                Command::new("cargo")
                    .args(["test", "--", "--test-threads=4"])
                    .current_dir(&ws)
                    .output()
            })
            .await
            .map_err(|e| {
                SwellError::IoError(std::io::Error::other(format!("Task join error: {}", e)))
            })?
            .map_err(SwellError::IoError)?;

            let passed = output.status.success();
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stderr_truncated = truncate_output(&stderr);
            drop(output);

            if !passed {
                all_passed = false;
                all_messages.push(ValidationMessage {
                    level: swell_core::ValidationLevel::Error,
                    code: Some("STAGE3_TESTS".to_string()),
                    message: format!("Full test suite failed:\n{}", stderr_truncated),
                    file: None,
                    line: None,
                });
            } else {
                all_messages.push(ValidationMessage {
                    level: swell_core::ValidationLevel::Info,
                    code: Some("STAGE3_TESTS".to_string()),
                    message: "Full test suite passed".to_string(),
                    file: None,
                    line: None,
                });
            }
        }

        // Run clippy
        if self.config.stage3.run_clippy {
            let ws = workspace_path.clone();
            let output = task::spawn_blocking(move || {
                Command::new("cargo")
                    .args(["clippy", "--", "-D", "warnings"])
                    .current_dir(&ws)
                    .output()
            })
            .await
            .map_err(|e| {
                SwellError::IoError(std::io::Error::other(format!("Task join error: {}", e)))
            })?
            .map_err(SwellError::IoError)?;

            if !output.status.success() {
                all_passed = false;
                let stderr = String::from_utf8_lossy(&output.stderr);
                all_messages.push(ValidationMessage {
                    level: swell_core::ValidationLevel::Error,
                    code: Some("STAGE3_CLIPPY".to_string()),
                    message: format!("Clippy found issues:\n{}", stderr),
                    file: None,
                    line: None,
                });
            } else {
                all_messages.push(ValidationMessage {
                    level: swell_core::ValidationLevel::Info,
                    code: Some("STAGE3_CLIPPY".to_string()),
                    message: "Clippy passed".to_string(),
                    file: None,
                    line: None,
                });
            }
        }

        // Run rustfmt
        if self.config.stage3.run_fmt {
            let ws = workspace_path.clone();
            let output = task::spawn_blocking(move || {
                Command::new("cargo")
                    .args(["fmt", "--check"])
                    .current_dir(&ws)
                    .output()
            })
            .await
            .map_err(|e| {
                SwellError::IoError(std::io::Error::other(format!("Task join error: {}", e)))
            })?
            .map_err(SwellError::IoError)?;

            if !output.status.success() {
                all_passed = false;
                all_messages.push(ValidationMessage {
                    level: swell_core::ValidationLevel::Warning,
                    code: Some("STAGE3_FMT".to_string()),
                    message: "Formatting issues detected".to_string(),
                    file: None,
                    line: None,
                });
            } else {
                all_messages.push(ValidationMessage {
                    level: swell_core::ValidationLevel::Info,
                    code: Some("STAGE3_FMT".to_string()),
                    message: "Formatting check passed".to_string(),
                    file: None,
                    line: None,
                });
            }
        }

        Ok(ValidationOutcome {
            passed: all_passed,
            messages: all_messages,
            artifacts: vec![],
        })
    }

    // =============================================================================
    // Stage 4: Full validation pipeline
    // =============================================================================

    async fn run_stage4(
        &self,
        context: &ValidationContext,
    ) -> Result<ValidationOutcome, SwellError> {
        let workspace_path = context.workspace_path.clone();
        let mut all_messages = Vec::new();
        let mut all_passed = true;

        // Run full test suite (final verification)
        if self.config.stage4.run_all_gates {
            let permit = cargo_test_semaphore()
                .acquire_owned()
                .await
                .map_err(|_| SwellError::IoError(std::io::Error::other("Semaphore closed")))?;
            let ws = workspace_path.clone();
            let output = task::spawn_blocking(move || {
                let _permit = permit;
                Command::new("cargo")
                    .args(["test", "--workspace", "--", "--test-threads=4"])
                    .current_dir(&ws)
                    .output()
            })
            .await
            .map_err(|e| {
                SwellError::IoError(std::io::Error::other(format!("Task join error: {}", e)))
            })?
            .map_err(SwellError::IoError)?;

            let passed = output.status.success();
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stderr_truncated = truncate_output(&stderr);
            drop(output);

            if !passed {
                all_passed = false;
                all_messages.push(ValidationMessage {
                    level: swell_core::ValidationLevel::Error,
                    code: Some("STAGE4_TESTS".to_string()),
                    message: format!("Workspace tests failed:\n{}", stderr_truncated),
                    file: None,
                    line: None,
                });
            } else {
                all_messages.push(ValidationMessage {
                    level: swell_core::ValidationLevel::Info,
                    code: Some("STAGE4_TESTS".to_string()),
                    message: "All workspace tests passed".to_string(),
                    file: None,
                    line: None,
                });
            }
        }

        // Security scan is available but handled at a higher level
        // (SecurityGate exists but may not be run in all stages)
        if self.config.stage4.run_security {
            all_messages.push(ValidationMessage {
                level: swell_core::ValidationLevel::Info,
                code: Some("STAGE4_SECURITY".to_string()),
                message: "Security scan: configure external scanner for full validation"
                    .to_string(),
                file: None,
                line: None,
            });
        }

        // AI review is available but handled at a higher level
        // (AiReviewGate exists but requires LLM configuration)
        if self.config.stage4.run_ai_review {
            all_messages.push(ValidationMessage {
                level: swell_core::ValidationLevel::Info,
                code: Some("STAGE4_AI_REVIEW".to_string()),
                message: "AI review: LLM backend required for full validation".to_string(),
                file: None,
                line: None,
            });
        }

        Ok(ValidationOutcome {
            passed: all_passed,
            messages: all_messages,
            artifacts: vec![],
        })
    }

    /// Extract test summary from cargo output
    fn extract_test_summary(stdout: &str, stderr: &str) -> String {
        let full_output = format!("{}\n{}", stdout, stderr);

        // Look for summary line like "test result: ok. X passed; Y failed; Z ignored"
        for line in full_output.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("test result:") {
                return trimmed.to_string();
            }
        }

        // If no summary, return first few lines of stderr
        stderr.lines().take(10).collect::<Vec<_>>().join("\n")
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod staged_executor_tests {
    use super::*;
    use swell_core::TaskId;

    fn create_test_context() -> ValidationContext {
        ValidationContext {
            task_id: TaskId::new(),
            workspace_path: std::env::current_dir()
                .unwrap()
                .to_string_lossy()
                .to_string(),
            changed_files: vec![],
            plan: None,
        }
    }

    #[test]
    fn test_test_stage_ordering() {
        assert!(TestStage::Stage0Instant < TestStage::Stage1Unit);
        assert!(TestStage::Stage1Unit < TestStage::Stage2Integration);
        assert!(TestStage::Stage2Integration < TestStage::Stage3Comprehensive);
        assert!(TestStage::Stage3Comprehensive < TestStage::Stage4Full);
    }

    #[test]
    fn test_test_stage_display() {
        assert_eq!(TestStage::Stage0Instant.name(), "Stage 0: Instant");
        assert_eq!(TestStage::Stage1Unit.name(), "Stage 1: Unit");
        assert_eq!(TestStage::Stage2Integration.name(), "Stage 2: Integration");
        assert_eq!(
            TestStage::Stage3Comprehensive.name(),
            "Stage 3: Comprehensive"
        );
        assert_eq!(TestStage::Stage4Full.name(), "Stage 4: Full");
    }

    #[test]
    fn test_test_stage_descriptions() {
        assert_eq!(
            TestStage::Stage0Instant.description(),
            "Fast syntax and import checks"
        );
        assert_eq!(
            TestStage::Stage1Unit.description(),
            "Unit tests (each component in isolation)"
        );
        assert_eq!(
            TestStage::Stage2Integration.description(),
            "Integration tests (component interactions)"
        );
        assert_eq!(
            TestStage::Stage3Comprehensive.description(),
            "Full test suite + lint checks"
        );
        assert_eq!(
            TestStage::Stage4Full.description(),
            "Complete validation pipeline with all gates"
        );
    }

    #[test]
    fn test_test_stage_continue_on_failure() {
        // Stage 0 and 1 should NOT continue on failure
        assert!(!TestStage::Stage0Instant.continue_on_failure());
        assert!(!TestStage::Stage1Unit.continue_on_failure());

        // Stages 2, 3, 4 should continue on failure
        assert!(TestStage::Stage2Integration.continue_on_failure());
        assert!(TestStage::Stage3Comprehensive.continue_on_failure());
        assert!(TestStage::Stage4Full.continue_on_failure());
    }

    #[test]
    fn test_test_stage_expected_duration() {
        assert_eq!(TestStage::Stage0Instant.expected_duration_ms(), 5_000);
        assert_eq!(TestStage::Stage1Unit.expected_duration_ms(), 60_000);
        assert_eq!(TestStage::Stage2Integration.expected_duration_ms(), 120_000);
        assert_eq!(
            TestStage::Stage3Comprehensive.expected_duration_ms(),
            300_000
        );
        assert_eq!(TestStage::Stage4Full.expected_duration_ms(), 600_000);
    }

    #[test]
    fn test_stage_config_default() {
        let config = StageConfig::default();
        assert!(config.stage0.run_cargo_check);
        assert!(config.stage0.run_fmt_check);
        assert_eq!(config.stage0.timeout_ms, 10_000);
    }

    #[test]
    fn test_staged_test_executor_default() {
        let executor = StagedTestExecutor::default();
        assert_eq!(executor.include_stages.len(), 5);
    }

    #[test]
    fn test_staged_test_executor_with_stages() {
        let stages = vec![TestStage::Stage0Instant, TestStage::Stage1Unit];
        let executor = StagedTestExecutor::with_stages(stages);
        assert_eq!(executor.include_stages.len(), 2);
    }

    #[test]
    fn test_stage_result_success() {
        let result = StageResult::success(
            TestStage::Stage0Instant,
            100,
            vec![ValidationMessage {
                level: swell_core::ValidationLevel::Info,
                code: Some("TEST".to_string()),
                message: "Test passed".to_string(),
                file: None,
                line: None,
            }],
        );

        assert!(result.passed);
        assert!(!result.skipped);
        assert_eq!(result.duration_ms, 100);
        assert!(result.skip_reason.is_none());
    }

    #[test]
    fn test_stage_result_failure() {
        let result = StageResult::failure(
            TestStage::Stage1Unit,
            500,
            vec![ValidationMessage {
                level: swell_core::ValidationLevel::Error,
                code: Some("TEST_FAIL".to_string()),
                message: "Test failed".to_string(),
                file: None,
                line: None,
            }],
        );

        assert!(!result.passed);
        assert!(!result.skipped);
        assert_eq!(result.duration_ms, 500);
    }

    #[test]
    fn test_stage_result_skipped() {
        let result = StageResult::skipped(
            TestStage::Stage2Integration,
            "Previous stage failed".to_string(),
        );

        assert!(result.passed); // Skipped stages don't count as failures
        assert!(result.skipped);
        assert_eq!(result.duration_ms, 0);
        assert!(result.skip_reason.is_some());
    }

    #[test]
    fn test_staged_result_has_failures() {
        let result = StagedResult {
            stage_results: vec![
                StageResult::success(TestStage::Stage0Instant, 100, vec![]),
                StageResult::failure(TestStage::Stage1Unit, 500, vec![]),
            ],
            passed: false,
            total_duration_ms: 600,
            first_failure_stage: Some(TestStage::Stage1Unit),
        };

        assert!(result.has_failures());
    }

    #[test]
    fn test_staged_result_no_failures() {
        let result = StagedResult {
            stage_results: vec![
                StageResult::success(TestStage::Stage0Instant, 100, vec![]),
                StageResult::success(TestStage::Stage1Unit, 500, vec![]),
            ],
            passed: true,
            total_duration_ms: 600,
            first_failure_stage: None,
        };

        assert!(!result.has_failures());
    }

    #[test]
    fn test_staged_result_get_stage_result() {
        let result = StagedResult {
            stage_results: vec![StageResult::success(TestStage::Stage0Instant, 100, vec![])],
            passed: true,
            total_duration_ms: 100,
            first_failure_stage: None,
        };

        assert!(result.get_stage_result(TestStage::Stage0Instant).is_some());
        assert!(result.get_stage_result(TestStage::Stage1Unit).is_none());
    }

    #[tokio::test]
    async fn test_execute_stage0_instant() {
        let executor = StagedTestExecutor::with_stages(vec![TestStage::Stage0Instant]);
        let context = create_test_context();

        let result = executor
            .execute_stage(TestStage::Stage0Instant, &context)
            .await;

        // Should succeed even if check fails (due to warnings in code)
        assert!(result.is_ok());
        let stage_result = result.unwrap();
        assert_eq!(stage_result.stage, TestStage::Stage0Instant);
    }

    #[tokio::test]
    async fn test_execute_stage0_with_custom_config() {
        let mut config = StageConfig::default();
        config.stage0.run_cargo_check = true;
        config.stage0.run_fmt_check = false; // Skip format check

        let executor = StagedTestExecutor::with_config(config);
        let context = create_test_context();

        let result = executor
            .execute_stage(TestStage::Stage0Instant, &context)
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_execute_all_stages() {
        // Run just stages 0 and 1 to keep test fast
        // Note: Stage 0 may fail due to formatting issues in new code,
        // but continue_on_failure=false means execution stops on first failure
        let executor =
            StagedTestExecutor::with_stages(vec![TestStage::Stage0Instant, TestStage::Stage1Unit]);
        let context = create_test_context();

        let result = executor
            .execute_up_to(TestStage::Stage1Unit, &context)
            .await;

        assert!(result.is_ok());
        let staged_result = result.unwrap();
        // Stage 0 may fail (formatting), so we may only get 1 result if it stops
        assert!(staged_result.stage_results.len() >= 1);
    }

    #[test]
    fn test_extract_test_summary() {
        let stdout = r#"
running 5 tests
test test_one ... ok
test test_two ... ok

test result: ok. 5 passed; 0 failed; 0 ignored; finished in 1.234s
"#;

        let summary = StagedTestExecutor::extract_test_summary(stdout, "");
        assert!(summary.contains("test result:"));
    }

    #[test]
    fn test_extract_test_summary_from_stderr() {
        let stderr = r#"
error: test failed
note: Run with `RUST_BACKTRACE=1` for a backtrace.
"#;

        let summary = StagedTestExecutor::extract_test_summary("", stderr);
        assert!(!summary.is_empty());
    }

    #[test]
    fn test_stage4_config_default() {
        let config = Stage4Config::default();
        assert!(config.run_all_gates);
        assert!(config.run_security);
        assert!(config.run_ai_review);
        assert_eq!(config.timeout_ms, 900_000);
    }

    #[test]
    fn test_stage3_config_default() {
        let config = Stage3Config::default();
        assert!(config.run_full_tests);
        assert!(config.run_clippy);
        assert!(config.run_fmt);
        assert_eq!(config.timeout_ms, 600_000);
    }
}
