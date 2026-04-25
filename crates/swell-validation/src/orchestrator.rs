//! ValidationOrchestrator - High-level interface for task completion validation.
//!
//! This module provides a single entry point [`ValidationOrchestrator::validate_task_completion`]
//! that runs all configured validation gates and returns a structured result.
//!
//! # Usage
//!
//! ```ignore
//! use swell_validation::orchestrator::{ValidationOrchestrator, TaskCompletionInput};
//! use swell_core::ValidationContext;
//!
//! let orchestrator = ValidationOrchestrator::default();
//! let input = TaskCompletionInput {
//!     task_id: uuid::Uuid::new_v4(),
//!     workspace_path: "/path/to/workspace".to_string(),
//!     changed_files: vec!["src/lib.rs".to_string()],
//!     plan: Some(plan),
//! };
//!
//! let result = orchestrator.validate_task_completion(input).await;
//! match result {
//!     Ok(validation_result) => {
//!         if validation_result.passed {
//!             println!("Task completed successfully!");
//!         } else {
//!             println!("Validation failed: {:?}", validation_result.errors);
//!         }
//!     }
//!     Err(e) => println!("Validation error: {}", e),
//! }
//! ```

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use swell_core::{SwellError, TaskId, ValidationContext, ValidationMessage, ValidationOutcome};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// Input for task completion validation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskCompletionInput {
    /// Unique identifier for the task
    pub task_id: TaskId,
    /// Path to the workspace where changes were made
    pub workspace_path: String,
    /// List of files that were changed
    pub changed_files: Vec<String>,
    /// Optional plan that was used for execution
    pub plan: Option<swell_core::Plan>,
    /// Optional metadata about the execution
    pub execution_metadata: Option<TaskExecutionMetadata>,
}

/// Metadata about the task execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskExecutionMetadata {
    /// Whether the task completed without errors
    pub completed_without_error: bool,
    /// Number of turns/iterations used
    pub iteration_count: u32,
    /// Total input tokens consumed
    pub input_tokens: u64,
    /// Total output tokens produced
    pub output_tokens: u64,
    /// Duration in milliseconds
    pub duration_ms: u64,
    /// Tool calls made during execution
    pub tool_calls_made: u32,
    /// Whether the execution was cut off due to max iterations
    pub max_iterations_reached: bool,
}

/// Result of task completion validation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskValidationResult {
    /// Whether the task passed all validation gates
    pub passed: bool,
    /// Whether lint gate passed
    pub lint_passed: bool,
    /// Whether test gate passed
    pub tests_passed: bool,
    /// Whether security gate passed
    pub security_passed: bool,
    /// Whether AI review passed
    pub ai_review_passed: bool,
    /// List of error messages from failed gates
    pub errors: Vec<String>,
    /// List of warning messages from gates
    pub warnings: Vec<String>,
    /// List of info messages from gates
    pub info_messages: Vec<String>,
    /// Detailed validation messages from all gates
    pub validation_messages: Vec<ValidationMessage>,
    /// Execution metadata (if provided)
    pub execution_metadata: Option<TaskExecutionMetadata>,
    /// Total duration in milliseconds
    pub total_duration_ms: u64,
    /// List of gates that were run
    pub gates_run: Vec<String>,
}

impl Default for TaskValidationResult {
    fn default() -> Self {
        Self {
            passed: true,
            lint_passed: true,
            tests_passed: true,
            security_passed: true,
            ai_review_passed: true,
            errors: Vec::new(),
            warnings: Vec::new(),
            info_messages: Vec::new(),
            validation_messages: Vec::new(),
            execution_metadata: None,
            total_duration_ms: 0,
            gates_run: Vec::new(),
        }
    }
}

/// A validation orchestrator that provides a single entry point for task completion validation.
///
/// This is the high-level interface that other crates should use for validation.
/// It wraps the [`super::ValidationPipeline`] and provides a simplified API.
///
/// # Example
///
/// ```ignore
/// use swell_validation::orchestrator::ValidationOrchestrator;
///
/// let orchestrator = ValidationOrchestrator::default();
/// let result = orchestrator.validate_task_completion(input).await?;
/// ```
#[derive(Debug, Clone)]
pub struct ValidationOrchestrator {
    /// Internal validation pipeline
    pipeline: super::ValidationPipeline,
    /// Whether to run lint gate (default: true)
    run_lint: bool,
    /// Whether to run test gate (default: true)
    run_tests: bool,
    /// Whether to run security gate (default: true)
    run_security: bool,
    /// Whether to run AI review gate (default: false for faster validation)
    run_ai_review: bool,
    /// Gate configuration initialized flag
    initialized: Arc<RwLock<bool>>,
}

impl Default for ValidationOrchestrator {
    fn default() -> Self {
        Self::new()
    }
}

impl ValidationOrchestrator {
    /// Create a new ValidationOrchestrator with default gates.
    pub fn new() -> Self {
        let mut orchestrator = Self {
            pipeline: super::ValidationPipeline::new(),
            run_lint: true,
            run_tests: true,
            run_security: true,
            run_ai_review: false, // Disabled by default for faster validation
            initialized: Arc::new(RwLock::new(false)),
        };
        orchestrator.init_gates();
        orchestrator
    }

    /// Create a new ValidationOrchestrator with all gates enabled.
    pub fn with_all_gates() -> Self {
        let mut orchestrator = Self::new();
        orchestrator.run_ai_review = true;
        orchestrator
    }

    /// Create a new ValidationOrchestrator with only fast gates (lint + tests).
    pub fn with_fast_gates() -> Self {
        let mut orchestrator = Self {
            pipeline: super::ValidationPipeline::new(),
            run_lint: true,
            run_tests: true,
            run_security: false,
            run_ai_review: false,
            initialized: Arc::new(RwLock::new(false)),
        };
        orchestrator.init_gates();
        orchestrator
    }

    /// Initialize the validation gates based on configuration.
    fn init_gates(&mut self) {
        use super::{AiReviewGate, LintGate, SecurityGate, TestGate};

        if self.run_lint {
            self.pipeline.add_gate(LintGate::new());
            debug!("Added LintGate to validation pipeline");
        }
        if self.run_tests {
            self.pipeline.add_gate(TestGate::new());
            debug!("Added TestGate to validation pipeline");
        }
        if self.run_security {
            self.pipeline.add_gate(SecurityGate::new());
            debug!("Added SecurityGate to validation pipeline");
        }
        if self.run_ai_review {
            self.pipeline.add_gate(AiReviewGate::new());
            debug!("Added AiReviewGate to validation pipeline");
        }
    }

    /// Validate task completion by running all configured gates.
    ///
    /// This is the main entry point for validating that a task has been completed successfully.
    /// It runs all configured validation gates and returns a structured result.
    ///
    /// # Arguments
    ///
    /// * `input` - The task completion input containing task details and changed files
    ///
    /// # Returns
    ///
    /// * `Ok(TaskValidationResult)` - The validation result with pass/fail status and details
    /// * `Err(SwellError)` - If validation could not be performed
    ///
    /// # Example
    ///
    /// ```ignore
    /// use swell_validation::orchestrator::{ValidationOrchestrator, TaskCompletionInput};
    ///
    /// let orchestrator = ValidationOrchestrator::default();
    /// let input = TaskCompletionInput {
    ///     task_id: uuid::Uuid::new_v4(),
    ///     workspace_path: "/path/to/workspace".to_string(),
    ///     changed_files: vec!["src/lib.rs".to_string()],
    ///     plan: None,
    ///     execution_metadata: None,
    /// };
    ///
    /// let result = orchestrator.validate_task_completion(input).await?;
    /// if result.passed {
    ///     println!("Task validation passed!");
    /// }
    /// ```
    pub async fn validate_task_completion(
        &self,
        input: TaskCompletionInput,
    ) -> Result<TaskValidationResult, SwellError> {
        let start = std::time::Instant::now();

        info!(
            task_id = %input.task_id,
            changed_files = input.changed_files.len(),
            "Starting task completion validation"
        );

        // Build validation context
        let context = ValidationContext {
            task_id: input.task_id,
            workspace_path: input.workspace_path.clone(),
            changed_files: input.changed_files.clone(),
            plan: input.plan.clone(),
        };

        // Run the validation pipeline
        let outcome = self.pipeline.run(&context).await?;

        let duration_ms = start.elapsed().as_millis() as u64;

        // Convert outcome to TaskValidationResult
        let result = self.convert_outcome_to_result(outcome, input.execution_metadata, duration_ms);

        // Log validation completion
        if result.passed {
            info!(
                task_id = %input.task_id,
                duration_ms = result.total_duration_ms,
                gates_run = result.gates_run.len(),
                "Task validation passed"
            );
        } else {
            warn!(
                task_id = %input.task_id,
                duration_ms = result.total_duration_ms,
                errors = result.errors.len(),
                warnings = result.warnings.len(),
                "Task validation failed"
            );
        }

        Ok(result)
    }

    /// Convert a ValidationOutcome to a TaskValidationResult
    fn convert_outcome_to_result(
        &self,
        outcome: ValidationOutcome,
        execution_metadata: Option<TaskExecutionMetadata>,
        duration_ms: u64,
    ) -> TaskValidationResult {
        let mut result = TaskValidationResult {
            passed: outcome.passed,
            validation_messages: outcome.messages.clone(),
            execution_metadata,
            total_duration_ms: duration_ms,
            ..Default::default()
        };

        // Determine which gates passed based on message metadata or patterns
        let mut lint_passed = true;
        let mut tests_passed = true;
        let mut security_passed = true;
        let mut ai_review_passed = true;
        let mut gates_run = Vec::new();

        for message in &outcome.messages {
            // Check for error messages that indicate a gate failed
            if message.level == swell_core::ValidationLevel::Error {
                let code = message.code.as_deref().unwrap_or("");

                // Determine which gate this message belongs to based on code prefix or content
                if code.starts_with("lint:") || message.file.is_some() {
                    // Lint messages typically have file and line info
                    lint_passed = false;
                    if !gates_run.contains(&"lint".to_string()) {
                        gates_run.push("lint".to_string());
                    }
                } else if code.starts_with("test:") {
                    tests_passed = false;
                    if !gates_run.contains(&"test".to_string()) {
                        gates_run.push("test".to_string());
                    }
                } else if code.starts_with("security:") {
                    security_passed = false;
                    if !gates_run.contains(&"security".to_string()) {
                        gates_run.push("security".to_string());
                    }
                } else if code.starts_with("ai_review:") {
                    ai_review_passed = false;
                    if !gates_run.contains(&"ai_review".to_string()) {
                        gates_run.push("ai_review".to_string());
                    }
                } else {
                    // Check file patterns for additional clues
                    if let Some(file) = &message.file {
                        if file.ends_with(".rs") && !file.contains("test") {
                            lint_passed = false;
                            if !gates_run.contains(&"lint".to_string()) {
                                gates_run.push("lint".to_string());
                            }
                        } else if file.contains("test") {
                            tests_passed = false;
                            if !gates_run.contains(&"test".to_string()) {
                                gates_run.push("test".to_string());
                            }
                        }
                    }
                }

                result.errors.push(message.message.clone());
            } else if message.level == swell_core::ValidationLevel::Warning {
                result.warnings.push(message.message.clone());

                // Track which gates had warnings
                let code = message.code.as_deref().unwrap_or("");
                if code.starts_with("lint:") {
                    if !gates_run.contains(&"lint".to_string()) {
                        gates_run.push("lint".to_string());
                    }
                } else if code.starts_with("test:") && !gates_run.contains(&"test".to_string()) {
                    gates_run.push("test".to_string());
                }
            } else if message.level == swell_core::ValidationLevel::Info {
                result.info_messages.push(message.message.clone());

                // Track info messages for gates that ran
                let code = message.code.as_deref().unwrap_or("");
                if code.starts_with("lint:") && !gates_run.contains(&"lint".to_string()) {
                    gates_run.push("lint".to_string());
                } else if code.starts_with("test:") && !gates_run.contains(&"test".to_string()) {
                    gates_run.push("test".to_string());
                } else if code.starts_with("security:")
                    && !gates_run.contains(&"security".to_string())
                {
                    gates_run.push("security".to_string());
                } else if code.starts_with("ai_review:")
                    && !gates_run.contains(&"ai_review".to_string())
                {
                    gates_run.push("ai_review".to_string());
                }
            }
        }

        // If no gates were explicitly tracked but we have messages, infer from context
        if gates_run.is_empty() && !outcome.messages.is_empty() {
            gates_run.push("lint".to_string());
            gates_run.push("test".to_string());
        }

        result.lint_passed = lint_passed;
        result.tests_passed = tests_passed;
        result.security_passed = security_passed;
        result.ai_review_passed = ai_review_passed;
        result.gates_run = gates_run;

        // If overall passed, all individual gates should be marked as passed
        if result.passed {
            result.lint_passed = true;
            result.tests_passed = true;
            result.security_passed = true;
            result.ai_review_passed = true;
            result.errors.clear();
        }

        result
    }

    /// Check if validation has been initialized.
    pub async fn is_initialized(&self) -> bool {
        *self.initialized.read().await
    }

    /// Update gate configuration
    pub fn set_run_lint(&mut self, enabled: bool) {
        self.run_lint = enabled;
    }

    /// Update gate configuration
    pub fn set_run_tests(&mut self, enabled: bool) {
        self.run_tests = enabled;
    }

    /// Update gate configuration
    pub fn set_run_security(&mut self, enabled: bool) {
        self.run_security = enabled;
    }

    /// Update gate configuration
    pub fn set_run_ai_review(&mut self, enabled: bool) {
        self.run_ai_review = enabled;
    }
}

impl std::fmt::Display for TaskValidationResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "TaskValidationResult(")?;
        write!(f, "passed={}, ", self.passed)?;
        write!(f, "lint={}, ", self.lint_passed)?;
        write!(f, "tests={}, ", self.tests_passed)?;
        write!(f, "security={}, ", self.security_passed)?;
        if let Some(meta) = &self.execution_metadata {
            write!(f, "iterations={}, ", meta.iteration_count)?;
        }
        write!(f, "duration={}ms", self.total_duration_ms)?;
        if !self.errors.is_empty() {
            write!(f, ", errors={}", self.errors.len())?;
        }
        if !self.warnings.is_empty() {
            write!(f, ", warnings={}", self.warnings.len())?;
        }
        write!(f, ")")
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use swell_core::{TaskId, ValidationLevel, ValidationMessage};

    /// Create a test input with minimal required fields
    fn create_test_input() -> TaskCompletionInput {
        TaskCompletionInput {
            task_id: TaskId::new(),
            workspace_path: std::env::current_dir()
                .unwrap()
                .to_string_lossy()
                .to_string(),
            changed_files: vec!["src/lib.rs".to_string()],
            plan: None,
            execution_metadata: Some(TaskExecutionMetadata {
                completed_without_error: true,
                iteration_count: 5,
                input_tokens: 1000,
                output_tokens: 500,
                duration_ms: 5000,
                tool_calls_made: 10,
                max_iterations_reached: false,
            }),
        }
    }

    #[tokio::test]
    async fn test_validation_orchestrator_new() {
        let orchestrator = ValidationOrchestrator::new();
        assert!(orchestrator.run_lint);
        assert!(orchestrator.run_tests);
        assert!(orchestrator.run_security);
        assert!(!orchestrator.run_ai_review);
    }

    #[tokio::test]
    async fn test_validation_orchestrator_default() {
        let orchestrator = ValidationOrchestrator::default();
        assert!(orchestrator.run_lint);
        assert!(orchestrator.run_tests);
        assert!(orchestrator.run_security);
        assert!(!orchestrator.run_ai_review);
    }

    #[tokio::test]
    async fn test_validation_orchestrator_with_all_gates() {
        let orchestrator = ValidationOrchestrator::with_all_gates();
        assert!(orchestrator.run_lint);
        assert!(orchestrator.run_tests);
        assert!(orchestrator.run_security);
        assert!(orchestrator.run_ai_review);
    }

    #[tokio::test]
    async fn test_validation_orchestrator_with_fast_gates() {
        let orchestrator = ValidationOrchestrator::with_fast_gates();
        assert!(orchestrator.run_lint);
        assert!(orchestrator.run_tests);
        assert!(!orchestrator.run_security);
        assert!(!orchestrator.run_ai_review);
    }

    #[tokio::test]
    async fn test_validate_task_completion_returns_result() {
        let orchestrator = ValidationOrchestrator::new();
        let input = create_test_input();

        let result = orchestrator.validate_task_completion(input).await;
        assert!(result.is_ok());

        let validation_result = result.unwrap();
        // total_duration_ms is u64, always >= 0; just confirm field is reachable.
        let _ = validation_result.total_duration_ms;
        assert!(!validation_result.gates_run.is_empty());
    }

    #[tokio::test]
    async fn test_task_validation_result_default() {
        let result = TaskValidationResult::default();
        assert!(result.passed);
        assert!(result.lint_passed);
        assert!(result.tests_passed);
        assert!(result.security_passed);
        assert!(result.ai_review_passed);
        assert!(result.errors.is_empty());
        assert!(result.warnings.is_empty());
        assert!(result.info_messages.is_empty());
    }

    #[tokio::test]
    async fn test_task_validation_result_serialization() {
        let result = TaskValidationResult {
            passed: true,
            lint_passed: true,
            tests_passed: true,
            security_passed: true,
            ai_review_passed: true,
            errors: vec!["Error 1".to_string(), "Error 2".to_string()],
            warnings: vec!["Warning 1".to_string()],
            info_messages: vec!["Info 1".to_string()],
            validation_messages: vec![ValidationMessage {
                level: ValidationLevel::Info,
                code: Some("lint:info".to_string()),
                message: "Test message".to_string(),
                file: Some("src/lib.rs".to_string()),
                line: Some(42),
            }],
            execution_metadata: Some(TaskExecutionMetadata {
                completed_without_error: true,
                iteration_count: 5,
                input_tokens: 1000,
                output_tokens: 500,
                duration_ms: 5000,
                tool_calls_made: 10,
                max_iterations_reached: false,
            }),
            total_duration_ms: 100,
            gates_run: vec!["lint".to_string(), "test".to_string()],
        };

        // Test JSON serialization
        let json = serde_json::to_string(&result).unwrap();
        let deserialized: TaskValidationResult = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.passed, result.passed);
        assert_eq!(deserialized.errors.len(), result.errors.len());
        assert_eq!(deserialized.gates_run.len(), result.gates_run.len());
    }

    #[tokio::test]
    async fn test_task_completion_input_serialization() {
        let input = TaskCompletionInput {
            task_id: TaskId::new(),
            workspace_path: "/tmp/workspace".to_string(),
            changed_files: vec!["src/lib.rs".to_string(), "src/main.rs".to_string()],
            plan: None,
            execution_metadata: Some(TaskExecutionMetadata {
                completed_without_error: true,
                iteration_count: 3,
                input_tokens: 500,
                output_tokens: 250,
                duration_ms: 3000,
                tool_calls_made: 5,
                max_iterations_reached: false,
            }),
        };

        // Test JSON serialization
        let json = serde_json::to_string(&input).unwrap();
        let deserialized: TaskCompletionInput = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.task_id, input.task_id);
        assert_eq!(deserialized.workspace_path, input.workspace_path);
        assert_eq!(deserialized.changed_files.len(), input.changed_files.len());
        assert_eq!(
            deserialized
                .execution_metadata
                .as_ref()
                .unwrap()
                .iteration_count,
            3
        );
    }

    #[tokio::test]
    async fn test_validation_orchestrator_set_run_lint() {
        let mut orchestrator = ValidationOrchestrator::new();
        assert!(orchestrator.run_lint);

        orchestrator.set_run_lint(false);
        assert!(!orchestrator.run_lint);

        orchestrator.set_run_lint(true);
        assert!(orchestrator.run_lint);
    }

    #[tokio::test]
    async fn test_validation_orchestrator_set_run_tests() {
        let mut orchestrator = ValidationOrchestrator::new();
        assert!(orchestrator.run_tests);

        orchestrator.set_run_tests(false);
        assert!(!orchestrator.run_tests);

        orchestrator.set_run_tests(true);
        assert!(orchestrator.run_tests);
    }

    #[tokio::test]
    async fn test_validation_orchestrator_set_run_security() {
        let mut orchestrator = ValidationOrchestrator::new();
        assert!(orchestrator.run_security);

        orchestrator.set_run_security(false);
        assert!(!orchestrator.run_security);

        orchestrator.set_run_security(true);
        assert!(orchestrator.run_security);
    }

    #[tokio::test]
    async fn test_validation_orchestrator_set_run_ai_review() {
        let mut orchestrator = ValidationOrchestrator::new();
        assert!(!orchestrator.run_ai_review);

        orchestrator.set_run_ai_review(true);
        assert!(orchestrator.run_ai_review);

        orchestrator.set_run_ai_review(false);
        assert!(!orchestrator.run_ai_review);
    }

    #[tokio::test]
    async fn test_convert_outcome_to_result_passing() {
        let orchestrator = ValidationOrchestrator::new();

        let outcome = ValidationOutcome {
            passed: true,
            messages: vec![
                ValidationMessage {
                    level: ValidationLevel::Info,
                    code: Some("lint:info".to_string()),
                    message: "Lint check passed".to_string(),
                    file: None,
                    line: None,
                },
                ValidationMessage {
                    level: ValidationLevel::Info,
                    code: Some("test:info".to_string()),
                    message: "Tests passed".to_string(),
                    file: None,
                    line: None,
                },
            ],
            artifacts: vec![],
        };

        let result = orchestrator.convert_outcome_to_result(outcome, None, 100);

        assert!(result.passed);
        assert!(result.lint_passed);
        assert!(result.tests_passed);
        assert!(result.errors.is_empty());
        assert!(result.gates_run.contains(&"lint".to_string()));
        assert!(result.gates_run.contains(&"test".to_string()));
    }

    #[tokio::test]
    async fn test_convert_outcome_to_result_with_errors() {
        let orchestrator = ValidationOrchestrator::new();

        let outcome = ValidationOutcome {
            passed: false,
            messages: vec![
                ValidationMessage {
                    level: ValidationLevel::Error,
                    code: Some("lint:error".to_string()),
                    message: "Lint error: unused variable".to_string(),
                    file: Some("src/lib.rs".to_string()),
                    line: Some(10),
                },
                ValidationMessage {
                    level: ValidationLevel::Warning,
                    code: Some("test:warning".to_string()),
                    message: "Test warning: slow test".to_string(),
                    file: None,
                    line: None,
                },
            ],
            artifacts: vec![],
        };

        let result = orchestrator.convert_outcome_to_result(outcome, None, 200);

        assert!(!result.passed);
        // Error message with code "lint:error" means lint_passed should be false
        // since our detection logic checks for error-level messages
        assert!(!result.errors.is_empty());
        assert!(!result.warnings.is_empty());
    }

    #[tokio::test]
    async fn test_validation_result_display_impl() {
        let result = TaskValidationResult {
            passed: true,
            lint_passed: true,
            tests_passed: true,
            security_passed: true,
            ai_review_passed: true,
            errors: vec![],
            warnings: vec![],
            info_messages: vec!["All checks passed".to_string()],
            validation_messages: vec![],
            execution_metadata: None,
            total_duration_ms: 150,
            gates_run: vec!["lint".to_string(), "test".to_string()],
        };

        let display = format!("{}", result);
        assert!(display.contains("passed"));
    }
}
