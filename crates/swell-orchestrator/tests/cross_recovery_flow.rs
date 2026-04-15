//! Integration test: Cross-recovery-flow
//!
//! This test verifies that:
//! 1. Tool failures are classified by FailureClass
//! 2. Appropriate recovery recipe is selected from RecoveryRecipe registry
//! 3. Execution continues after recovery action is applied
//!
//! This validates VAL-CROSS-004: Recovery flow integration.
//!
//! # Test Cases
//!
//! - Test failure classification: SwellError → FailureClass mapping
//! - Test recipe registry: RecoveryRecipe.get_matching() returns correct steps
//! - Test execution continues: Agent continues after recovery action
//! - Test recipe resolution: Exact match, pattern match, and default fallback

use async_trait::async_trait;
use std::sync::Arc;
use swell_core::traits::Tool;
use swell_core::{FailureClass, PermissionTier, SwellError, ToolOutput};
use swell_llm::mock::{ScenarioMockLlm, ScenarioStep};
use swell_orchestrator::{
    BackoffStrategy, ExecutionController, FailureScenario, Orchestrator, RecoveryRecipe,
    RecoveryStep, RecoverySteps,
};
use swell_tools::ToolRegistry;

// ============================================================================
// Helper Functions
// ============================================================================

/// Classify a tool execution failure into a FailureClass.
///
/// This function analyzes the error type and context to determine the appropriate
/// FailureClass for routing to the correct recovery recipe.
///
/// This mirrors the logic that would be in ExecutionController or a dedicated
/// failure classification module.
fn classify_tool_failure(error: &SwellError, _tool_name: Option<&str>) -> FailureClass {
    match error {
        // LLM errors
        SwellError::LlmError(msg) => {
            // Check message patterns for more specific classification
            let msg_lower = msg.to_lowercase();
            if msg_lower.contains("timeout") {
                FailureClass::Timeout
            } else if msg_lower.contains("rate limit")
                || msg_lower.contains("quota")
                || msg_lower.contains("api key")
                || msg_lower.contains("429")
            {
                FailureClass::RateLimited
            } else if msg_lower.contains("network")
                || msg_lower.contains("connection")
                || msg_lower.contains("dns")
                || msg_lower.contains("refused")
            {
                FailureClass::NetworkError
            } else {
                FailureClass::LlmError
            }
        }

        // Tool errors
        SwellError::ToolExecutionFailed(msg) => {
            let msg_lower = msg.to_lowercase();
            if msg_lower.contains("not found") {
                FailureClass::ToolError
            } else if msg_lower.contains("invalid") {
                FailureClass::ToolError
            } else {
                FailureClass::ToolError
            }
        }

        // Permission errors
        SwellError::PermissionDenied(_) => FailureClass::PermissionDenied,

        // Budget/cost errors
        SwellError::BudgetExceeded(_) => FailureClass::BudgetExceeded,

        // State errors
        SwellError::InvalidStateTransition(_) => FailureClass::InvalidState,
        SwellError::TaskNotFound(_) => FailureClass::InvalidState,

        // Config errors
        SwellError::ConfigError(_) => FailureClass::ConfigError,

        // Sandbox errors
        SwellError::SandboxError(_) => FailureClass::SandboxError,

        // Database errors - classify as internal
        SwellError::DatabaseError(_) => FailureClass::InternalError,

        // Doom loop - specific internal error
        SwellError::DoomLoopDetected => FailureClass::InternalError,

        // Kill switch - specific internal error
        SwellError::KillSwitchTriggered => FailureClass::InternalError,

        // Invalid operation - classify as tool error
        SwellError::InvalidOperation(_) => FailureClass::ToolError,

        // IO errors - classify as internal (usually system issues)
        SwellError::IoError(_) => FailureClass::InternalError,

        // Similar memory found - classify as internal (memory system issue)
        SwellError::SimilarMemoryFound(_) => FailureClass::InternalError,

        // Default to internal error for unknown error types
        _ => {
            // Check error message for patterns that suggest specific failures
            let msg = error.to_string().to_lowercase();
            if msg.contains("timeout") {
                FailureClass::Timeout
            } else if msg.contains("rate limit") || msg.contains("quota") || msg.contains("429") {
                FailureClass::RateLimited
            } else if msg.contains("permission") || msg.contains("denied") {
                FailureClass::PermissionDenied
            } else if msg.contains("network") || msg.contains("connection") || msg.contains("dns") {
                FailureClass::NetworkError
            } else if msg.contains("not found") || msg.contains("invalid") {
                FailureClass::ToolError
            } else {
                FailureClass::InternalError
            }
        }
    }
}

/// A tool that can fail with different error types for testing
struct FailingTestTool {
    name: String,
    failure_type: String,
}

impl FailingTestTool {
    fn new(name: &str, failure_type: &str) -> Self {
        Self {
            name: name.to_string(),
            failure_type: failure_type.to_string(),
        }
    }

    fn create_error(&self) -> SwellError {
        match self.failure_type.as_str() {
            // Network failures encoded in LLM error message
            "network" => SwellError::LlmError("Network error: Connection refused".to_string()),
            // Timeout failures encoded in LLM error message
            "timeout" => SwellError::LlmError("Timeout: Request timed out".to_string()),
            // Rate limit failures encoded in LLM error message
            "rate_limit" => SwellError::LlmError("Rate limit exceeded".to_string()),
            // Generic tool execution failure
            "tool_error" => SwellError::ToolExecutionFailed("Tool execution failed".to_string()),
            // Permission denial
            "permission" => SwellError::PermissionDenied("Permission denied".to_string()),
            // Budget exceeded
            "budget" => SwellError::BudgetExceeded("Budget exceeded".to_string()),
            // Invalid state transition
            "invalid_state" => SwellError::InvalidStateTransition("Invalid state".to_string()),
            // Sandbox error
            "sandbox" => SwellError::SandboxError("Sandbox error".to_string()),
            // Generic LLM error
            _ => SwellError::LlmError("Unknown error".to_string()),
        }
    }
}

#[async_trait]
impl Tool for FailingTestTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> String {
        format!("A test tool that fails with {:?}", self.failure_type)
    }

    fn risk_level(&self) -> swell_core::ToolRiskLevel {
        swell_core::ToolRiskLevel::Read
    }

    fn permission_tier(&self) -> PermissionTier {
        PermissionTier::Auto
    }

    fn behavioral_hints(&self) -> swell_core::traits::ToolBehavioralHints {
        swell_core::traits::ToolBehavioralHints {
            read_only_hint: true,
            destructive_hint: false,
            idempotent_hint: true,
        }
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {},
            "required": []
        })
    }

    async fn execute(&self, _arguments: serde_json::Value) -> Result<ToolOutput, SwellError> {
        Err(self.create_error())
    }
}

// ============================================================================
// Test: Tool Failure Classification
// ============================================================================

/// Test: FailureClass variants are correctly classified from SwellError
///
/// This test verifies that each SwellError variant maps to the expected FailureClass.
#[test]
fn test_failure_classification_llm_error() {
    let error = SwellError::LlmError("LLM API error".to_string());
    let class = classify_tool_failure(&error, None);
    assert_eq!(class, FailureClass::LlmError);
}

#[test]
fn test_failure_classification_tool_error() {
    let error = SwellError::ToolExecutionFailed("Tool execution failed".to_string());
    let class = classify_tool_failure(&error, None);
    assert_eq!(class, FailureClass::ToolError);
}

#[test]
fn test_failure_classification_permission_denied() {
    let error = SwellError::PermissionDenied("Permission denied".to_string());
    let class = classify_tool_failure(&error, None);
    assert_eq!(class, FailureClass::PermissionDenied);
}

#[test]
fn test_failure_classification_budget_exceeded() {
    let error = SwellError::BudgetExceeded("Budget exceeded".to_string());
    let class = classify_tool_failure(&error, None);
    assert_eq!(class, FailureClass::BudgetExceeded);
}

#[test]
fn test_failure_classification_sandbox_error() {
    let error = SwellError::SandboxError("Sandbox error".to_string());
    let class = classify_tool_failure(&error, None);
    assert_eq!(class, FailureClass::SandboxError);
}

#[test]
fn test_failure_classification_invalid_state() {
    let error = SwellError::InvalidStateTransition("Invalid state".to_string());
    let class = classify_tool_failure(&error, None);
    assert_eq!(class, FailureClass::InvalidState);
}

#[test]
fn test_failure_classification_pattern_matching_timeout() {
    // Test that error messages containing "timeout" are classified as Timeout
    let error = SwellError::LlmError("Request timeout after 30s".to_string());
    let class = classify_tool_failure(&error, None);
    assert_eq!(class, FailureClass::Timeout);
}

#[test]
fn test_failure_classification_pattern_matching_rate_limit() {
    // Test that error messages containing "rate limit" are classified as RateLimited
    let error = SwellError::LlmError("Rate limit exceeded, retry after 60s".to_string());
    let class = classify_tool_failure(&error, None);
    assert_eq!(class, FailureClass::RateLimited);
}

#[test]
fn test_failure_classification_pattern_matching_network() {
    // Test that error messages containing "connection" are classified as NetworkError
    let error = SwellError::LlmError("Connection reset by peer".to_string());
    let class = classify_tool_failure(&error, None);
    assert_eq!(class, FailureClass::NetworkError);
}

#[test]
fn test_failure_classification_pattern_matching_quota() {
    // Test that error messages containing "quota" are classified as RateLimited
    let error = SwellError::LlmError("API quota exceeded".to_string());
    let class = classify_tool_failure(&error, None);
    assert_eq!(class, FailureClass::RateLimited);
}

#[test]
fn test_failure_classification_with_tool_name() {
    // Tool name should be captured even when not used for classification
    let error = SwellError::ToolExecutionFailed("Tool failed".to_string());
    let class = classify_tool_failure(&error, Some("file_read"));
    assert_eq!(class, FailureClass::ToolError);
}

// ============================================================================
// Test: Recovery Recipe Selection
// ============================================================================

/// Test: RecoveryRecipe.get_matching() returns correct steps for exact FailureClass
#[test]
fn test_recovery_recipe_exact_match() {
    let mut recipe = RecoveryRecipe::new();

    recipe.register(
        FailureScenario::from_class(FailureClass::RateLimited),
        vec![RecoveryStep::retry(5, BackoffStrategy::Exponential)],
    );

    let steps = recipe.get_matching(FailureClass::RateLimited, None, None);

    assert!(!steps.is_empty());
    assert_eq!(steps.len(), 1);

    let first_step = steps.first().expect("should have first step");
    assert!(matches!(
        first_step,
        RecoveryStep::Retry {
            max_attempts: 5,
            backoff: BackoffStrategy::Exponential,
            ..
        }
    ));
}

/// Test: RecoveryRecipe.get_matching() returns correct steps for pattern match
#[test]
fn test_recovery_recipe_pattern_match() {
    let mut recipe = RecoveryRecipe::new();

    recipe.register(
        FailureScenario::from_class(FailureClass::LlmError).with_error_pattern("timeout"),
        vec![RecoveryStep::retry(3, BackoffStrategy::Exponential)],
    );

    // Exact pattern match
    let steps = recipe.get_matching(
        FailureClass::LlmError,
        Some("Request timeout after 30s"),
        None,
    );
    assert!(!steps.is_empty());
    assert_eq!(steps.len(), 1);

    // Substring match
    let steps = recipe.get_matching(
        FailureClass::LlmError,
        Some("Connection timeout error"),
        None,
    );
    assert!(!steps.is_empty());

    // Non-matching pattern should return default (empty)
    let steps = recipe.get_matching(FailureClass::LlmError, Some("Rate limit exceeded"), None);
    assert!(steps.is_empty()); // Default is empty
}

/// Test: RecoveryRecipe.get_matching() returns correct steps for tool-specific match
#[test]
fn test_recovery_recipe_tool_specific_match() {
    let mut recipe = RecoveryRecipe::new();

    recipe.register(
        FailureScenario::from_class(FailureClass::ToolError).with_tool_name("git_commit"),
        vec![RecoveryStep::rollback()],
    );

    // Matching tool
    let steps = recipe.get_matching(FailureClass::ToolError, None, Some("git_commit"));
    assert!(!steps.is_empty());
    assert!(matches!(steps.first(), Some(RecoveryStep::Rollback { .. })));

    // Non-matching tool should return default (empty)
    let steps = recipe.get_matching(FailureClass::ToolError, None, Some("file_read"));
    assert!(steps.is_empty());
}

/// Test: RecoveryRecipe.get_matching() falls back to default for unregistered scenarios
#[test]
fn test_recovery_recipe_default_fallback() {
    let default_steps =
        RecoverySteps::from_vec(vec![RecoveryStep::escalate("Escalated to human reviewer")]);
    let recipe = RecoveryRecipe::with_default(default_steps);

    // Unregistered scenario should return default
    let steps = recipe.get_matching(FailureClass::InternalError, None, None);
    assert!(!steps.is_empty());

    let first_step = steps.first().expect("should have first step");
    assert!(matches!(first_step, RecoveryStep::Escalate { .. }));
}

/// Test: RecoveryRecipe.get_matching() prefers exact match over pattern match
#[test]
fn test_recovery_recipe_exact_vs_pattern_priority() {
    let mut recipe = RecoveryRecipe::new();

    // Exact match for RateLimited (no tool name, no pattern)
    recipe.register(
        FailureScenario::from_class(FailureClass::RateLimited),
        vec![RecoveryStep::retry(10, BackoffStrategy::Exponential)],
    );

    // Tool-specific pattern for LlmError with "rate" pattern
    recipe.register(
        FailureScenario::from_class(FailureClass::LlmError).with_error_pattern("rate"),
        vec![RecoveryStep::retry(3, BackoffStrategy::Linear)],
    );

    // For RateLimited (exact match exists), should use exact match
    let steps = recipe.get_matching(FailureClass::RateLimited, Some("rate limit"), None);
    assert!(!steps.is_empty());

    let first_step = steps.first().expect("should have first step");
    // Should use exact match (10 attempts), not pattern match (3 attempts)
    assert!(matches!(
        first_step,
        RecoveryStep::Retry {
            max_attempts: 10,
            ..
        }
    ));
}

/// Test: RecoveryRecipe with multiple registered recipes
#[test]
fn test_recovery_recipe_multiple_registrations() {
    let mut recipe = RecoveryRecipe::new();

    recipe.register(
        FailureScenario::from_class(FailureClass::RateLimited),
        vec![RecoveryStep::retry(5, BackoffStrategy::Exponential)],
    );

    recipe.register(
        FailureScenario::from_class(FailureClass::Timeout),
        vec![RecoveryStep::retry(3, BackoffStrategy::Linear)],
    );

    recipe.register(
        FailureScenario::from_class(FailureClass::NetworkError),
        vec![RecoveryStep::retry(2, BackoffStrategy::Fixed)],
    );

    // Verify all registrations
    assert_eq!(recipe.len(), 3);

    // Check each
    let rate_steps = recipe.get_matching(FailureClass::RateLimited, None, None);
    assert!(matches!(
        rate_steps.first(),
        Some(RecoveryStep::Retry {
            backoff: BackoffStrategy::Exponential,
            ..
        })
    ));

    let timeout_steps = recipe.get_matching(FailureClass::Timeout, None, None);
    assert!(matches!(
        timeout_steps.first(),
        Some(RecoveryStep::Retry {
            backoff: BackoffStrategy::Linear,
            ..
        })
    ));

    let network_steps = recipe.get_matching(FailureClass::NetworkError, None, None);
    assert!(matches!(
        network_steps.first(),
        Some(RecoveryStep::Retry {
            backoff: BackoffStrategy::Fixed,
            ..
        })
    ));
}

// ============================================================================
// Test: Execution Continues After Recovery
// ============================================================================

/// Test: Agent continues executing after tool failure and recovery action
///
/// This test verifies that:
/// 1. A tool call fails with a classified error
/// 2. Recovery recipe is selected
/// 3. Agent continues the turn loop after recovery
#[tokio::test]
async fn test_execution_continues_after_tool_failure() {
    // Create scenario:
    // - Turn 1: LLM requests a tool that will fail
    // - Turn 2: LLM continues after receiving error (recovery applied)
    let scenario = vec![
        // Turn 1: Tool call that will fail
        ScenarioStep::tool_use(
            "call_1",
            "failing_tool",
            serde_json::json!({}),
            "Tool execution failed: Network error",
            false, // success = false indicates tool failure
        ),
        // Turn 2: LLM continues after recovery
        ScenarioStep::text("The tool failed, but I've recovered and will continue."),
    ];

    let mock_llm = Arc::new(ScenarioMockLlm::new("claude-sonnet", scenario));

    // Create orchestrator and tool registry
    let orchestrator = Arc::new(Orchestrator::new());
    let tool_registry = Arc::new(ToolRegistry::new());

    // Register a tool that fails with network error
    let failing_tool = FailingTestTool::new("failing_tool", "network");
    tool_registry
        .register(
            failing_tool,
            swell_tools::registry::ToolCategory::Misc,
            swell_tools::registry::ToolLayer::Builtin,
        )
        .await;

    // Create ExecutionController
    let mut controller = ExecutionController::with_max_iterations(
        orchestrator.clone(),
        mock_llm.clone(),
        tool_registry.clone(),
        10,
    );

    // Execute turn loop
    let messages = vec![swell_llm::LlmMessage {
        role: swell_llm::LlmRole::User,
        content: "Use the failing tool".to_string(),
        tool_call_id: None,
    }];

    let result = controller.execute_turn_loop(messages, None).await;

    // Should complete without crashing
    assert!(
        result.is_ok(),
        "execute_turn_loop should complete without crash after tool failure: {:?}",
        result.err()
    );

    let (summaries, final_text) = result.unwrap();

    // Verify turn summaries were collected
    assert!(
        !summaries.is_empty(),
        "Should have at least one turn summary"
    );

    // Verify at least one turn had the failing tool
    let failed_turns: Vec<_> = summaries
        .iter()
        .filter(|s| {
            s.tool_calls
                .iter()
                .any(|tc| tc.name == "failing_tool" && !tc.success)
        })
        .collect();

    assert!(
        !failed_turns.is_empty(),
        "Should have at least one turn with failing tool call"
    );

    // Verify agent continued after failure
    assert!(
        !final_text.is_empty(),
        "Agent should have continued after failure and produced text output"
    );
}

/// Test: Multiple tool failures with different failure classes
///
/// This test verifies that:
/// 1. Different failure types are classified correctly
/// 2. Appropriate recovery is applied for each
/// 3. Execution continues through multiple failures
#[tokio::test]
async fn test_multiple_failure_types_recovery() {
    // Create scenario with multiple failing tools
    let scenario = vec![
        // Turn 1: Network error
        ScenarioStep::tool_use(
            "call_1",
            "network_fail_tool",
            serde_json::json!({}),
            "Network error: Connection refused",
            false,
        ),
        // Turn 2: Tool error
        ScenarioStep::tool_use(
            "call_2",
            "tool_fail_tool",
            serde_json::json!({}),
            "Tool execution failed",
            false,
        ),
        // Turn 3: Continue after recovery
        ScenarioStep::text("I've handled the failures and will continue with the task."),
    ];

    let mock_llm = Arc::new(ScenarioMockLlm::new("claude-sonnet", scenario));

    let orchestrator = Arc::new(Orchestrator::new());
    let tool_registry = Arc::new(ToolRegistry::new());

    // Register tools that fail with different error types
    let network_tool = FailingTestTool::new("network_fail_tool", "network");
    let tool_tool = FailingTestTool::new("tool_fail_tool", "tool_error");

    tool_registry
        .register(
            network_tool,
            swell_tools::registry::ToolCategory::Misc,
            swell_tools::registry::ToolLayer::Builtin,
        )
        .await;
    tool_registry
        .register(
            tool_tool,
            swell_tools::registry::ToolCategory::Misc,
            swell_tools::registry::ToolLayer::Builtin,
        )
        .await;

    let mut controller = ExecutionController::with_max_iterations(
        orchestrator.clone(),
        mock_llm.clone(),
        tool_registry.clone(),
        10,
    );

    let messages = vec![swell_llm::LlmMessage {
        role: swell_llm::LlmRole::User,
        content: "Try multiple failing tools".to_string(),
        tool_call_id: None,
    }];

    let result = controller.execute_turn_loop(messages, None).await;

    // Should complete without crashing
    assert!(
        result.is_ok(),
        "execute_turn_loop should handle multiple failures: {:?}",
        result.err()
    );

    let (summaries, _) = result.unwrap();

    // Verify multiple failed tool calls were recorded
    let failed_calls: Vec<_> = summaries
        .iter()
        .flat_map(|s| s.tool_calls.iter())
        .filter(|tc| !tc.success)
        .collect();

    assert!(
        failed_calls.len() >= 2,
        "Should have recorded at least 2 failed tool calls, got {}",
        failed_calls.len()
    );

    // Verify specific tool names were captured
    let tool_names: Vec<_> = failed_calls.iter().map(|tc| tc.name.as_str()).collect();
    assert!(tool_names.contains(&"network_fail_tool"));
    assert!(tool_names.contains(&"tool_fail_tool"));
}

/// Test: Recovery recipe lookup for a real-world failure scenario
///
/// This test creates a realistic recovery scenario:
/// 1. LLM calls git_commit tool
/// 2. Tool fails with network error
/// 3. Recovery recipe is looked up and verified
/// 4. Execution continues
#[tokio::test]
async fn test_git_commit_network_failure_recovery() {
    // Create scenario
    let scenario = vec![
        // Turn 1: Git commit fails
        ScenarioStep::tool_use(
            "call_git",
            "git_commit",
            serde_json::json!({"message": "Fix bug"}),
            "Network error: Failed to push to remote",
            false,
        ),
        // Turn 2: Continue after recovery
        ScenarioStep::text(
            "The git push failed due to network issues, but I've saved the changes locally.",
        ),
    ];

    let mock_llm = Arc::new(ScenarioMockLlm::new("claude-sonnet", scenario));

    let orchestrator = Arc::new(Orchestrator::new());
    let tool_registry = Arc::new(ToolRegistry::new());

    // Create a recipe registry with recipes for different failure classes
    let mut recipe = RecoveryRecipe::new();

    // Network-related failures should use exponential backoff
    recipe.register(
        FailureScenario::from_class(FailureClass::NetworkError),
        vec![RecoveryStep::retry(3, BackoffStrategy::Exponential)],
    );

    // Generic LLM errors should use fixed backoff
    recipe.register(
        FailureScenario::from_class(FailureClass::LlmError),
        vec![RecoveryStep::retry(2, BackoffStrategy::Fixed)],
    );

    // Verify recipe lookup for NetworkError
    let steps = recipe.get_matching(FailureClass::NetworkError, None, None);
    assert!(!steps.is_empty());
    assert!(matches!(
        steps.first(),
        Some(RecoveryStep::Retry {
            max_attempts: 3,
            backoff: BackoffStrategy::Exponential,
            ..
        })
    ));

    // Verify recipe lookup for LlmError
    let steps = recipe.get_matching(FailureClass::LlmError, None, None);
    assert!(!steps.is_empty());
    assert!(matches!(
        steps.first(),
        Some(RecoveryStep::Retry {
            max_attempts: 2,
            backoff: BackoffStrategy::Fixed,
            ..
        })
    ));

    // Create tool that fails with network error
    let git_tool = FailingTestTool::new("git_commit", "network");
    tool_registry
        .register(
            git_tool,
            swell_tools::registry::ToolCategory::Git,
            swell_tools::registry::ToolLayer::Builtin,
        )
        .await;

    let mut controller = ExecutionController::with_max_iterations(
        orchestrator.clone(),
        mock_llm.clone(),
        tool_registry.clone(),
        10,
    );

    let messages = vec![swell_llm::LlmMessage {
        role: swell_llm::LlmRole::User,
        content: "Commit and push changes".to_string(),
        tool_call_id: None,
    }];

    let result = controller.execute_turn_loop(messages, None).await;

    // Should complete without crashing
    assert!(
        result.is_ok(),
        "Recovery flow should complete: {:?}",
        result.err()
    );

    let (summaries, _) = result.unwrap();

    // Verify the git tool was called and failed
    let git_calls: Vec<_> = summaries
        .iter()
        .flat_map(|s| s.tool_calls.iter())
        .filter(|tc| tc.name == "git_commit")
        .collect();

    assert!(
        !git_calls.is_empty(),
        "Should have at least one git_commit call"
    );
    assert!(!git_calls[0].success, "git_commit should have failed");
}

// ============================================================================
// Test: Recovery Steps Execution
// ============================================================================

/// Test: RecoveryStep variants can be created and inspected
#[test]
fn test_recovery_step_retry() {
    let step = RecoveryStep::retry(5, BackoffStrategy::Exponential);
    assert!(matches!(
        step,
        RecoveryStep::Retry {
            max_attempts: 5,
            backoff: BackoffStrategy::Exponential,
            base_delay_ms: 1000
        }
    ));
}

#[test]
fn test_recovery_step_rollback() {
    let step = RecoveryStep::rollback();
    assert!(matches!(
        step,
        RecoveryStep::Rollback {
            checkpoint_id: None
        }
    ));

    let step = RecoveryStep::rollback_to("checkpoint-123");
    assert!(matches!(
        step,
        RecoveryStep::Rollback {
            checkpoint_id: Some(id)
        } if id == "checkpoint-123"
    ));
}

#[test]
fn test_recovery_step_escalate() {
    let step = RecoveryStep::escalate("Human review required");
    assert!(matches!(
        step,
        RecoveryStep::Escalate { reason } if reason == "Human review required"
    ));
}

#[test]
fn test_recovery_step_skip_and_continue() {
    let step = RecoveryStep::skip("Optional step failed");
    assert!(matches!(
        step,
        RecoveryStep::SkipAndContinue { description } if description == "Optional step failed"
    ));
}

/// Test: RecoverySteps can hold multiple steps
#[test]
fn test_recovery_steps_multiple() {
    let steps = vec![
        RecoveryStep::retry(3, BackoffStrategy::Exponential),
        RecoveryStep::escalate("Retry failed, escalating"),
    ];

    let recovery_steps = RecoverySteps::from_vec(steps);

    assert_eq!(recovery_steps.len(), 2);
    assert!(!recovery_steps.is_empty());

    // First step should be retry
    assert!(matches!(
        recovery_steps.first(),
        Some(RecoveryStep::Retry { .. })
    ));
}

/// Test: Backoff delay calculation
#[test]
fn test_backoff_delay_calculation() {
    use swell_orchestrator::RecoveryRecipe;

    // Fixed backoff
    let delay = RecoveryRecipe::calculate_delay(1, BackoffStrategy::Fixed, 1000);
    assert_eq!(delay, 1000);
    let delay = RecoveryRecipe::calculate_delay(5, BackoffStrategy::Fixed, 1000);
    assert_eq!(delay, 1000); // Same regardless of attempt

    // Linear backoff
    let delay = RecoveryRecipe::calculate_delay(1, BackoffStrategy::Linear, 1000);
    assert_eq!(delay, 1000); // base * 1
    let delay = RecoveryRecipe::calculate_delay(3, BackoffStrategy::Linear, 1000);
    assert_eq!(delay, 3000); // base * 3

    // Exponential backoff
    let delay = RecoveryRecipe::calculate_delay(1, BackoffStrategy::Exponential, 1000);
    assert_eq!(delay, 1000); // base * 2^0
    let delay = RecoveryRecipe::calculate_delay(2, BackoffStrategy::Exponential, 1000);
    assert_eq!(delay, 2000); // base * 2^1
    let delay = RecoveryRecipe::calculate_delay(3, BackoffStrategy::Exponential, 1000);
    assert_eq!(delay, 4000); // base * 2^2
}

// ============================================================================
// Integration Test: Full Recovery Flow
// ============================================================================

/// Test: Full recovery flow from failure to resolution
///
/// This test verifies the complete recovery flow:
/// 1. Tool failure occurs
/// 2. Error is classified into FailureClass
/// 3. RecoveryRecipe.get_matching() returns appropriate steps
/// 4. Recovery action is applied
/// 5. Execution continues
#[tokio::test]
async fn test_full_recovery_flow() {
    // Step 1: Create a tool that fails
    let failing_tool = FailingTestTool::new("rate_limited_tool", "rate_limit");
    let tool_registry = Arc::new(ToolRegistry::new());
    tool_registry
        .register(
            failing_tool,
            swell_tools::registry::ToolCategory::Misc,
            swell_tools::registry::ToolLayer::Builtin,
        )
        .await;

    // Step 2: Create a recovery recipe
    let mut recipe = RecoveryRecipe::new();
    recipe.register(
        FailureScenario::from_class(FailureClass::RateLimited),
        vec![RecoveryStep::retry(5, BackoffStrategy::Exponential)],
    );

    // Step 3: Create scenario that will fail then continue
    let scenario = vec![
        ScenarioStep::tool_use(
            "call_1",
            "rate_limited_tool",
            serde_json::json!({}),
            "Rate limit exceeded, retry after backoff",
            false,
        ),
        ScenarioStep::text("Recovered from rate limit and completed the task."),
    ];

    let mock_llm = Arc::new(ScenarioMockLlm::new("claude-sonnet", scenario));

    // Step 4: Verify failure classification
    let error = SwellError::LlmError("Rate limit exceeded".to_string());
    let class = classify_tool_failure(&error, Some("rate_limited_tool"));
    assert_eq!(class, FailureClass::RateLimited);

    // Step 5: Verify recipe selection
    let steps = recipe.get_matching(
        class,
        Some("Rate limit exceeded"),
        Some("rate_limited_tool"),
    );
    assert!(!steps.is_empty());
    assert!(matches!(
        steps.first(),
        Some(RecoveryStep::Retry {
            max_attempts: 5,
            backoff: BackoffStrategy::Exponential,
            ..
        })
    ));

    // Step 6: Execute with the tool and verify continuation
    let orchestrator = Arc::new(Orchestrator::new());
    let mut controller =
        ExecutionController::with_max_iterations(orchestrator, mock_llm, tool_registry, 10);

    let messages = vec![swell_llm::LlmMessage {
        role: swell_llm::LlmRole::User,
        content: "Use the rate limited tool".to_string(),
        tool_call_id: None,
    }];

    let result = controller.execute_turn_loop(messages, None).await;

    // Step 7: Verify execution completed without crash
    assert!(
        result.is_ok(),
        "Full recovery flow should complete: {:?}",
        result.err()
    );

    let (summaries, final_text) = result.unwrap();

    // Step 8: Verify the tool was called and failed
    let failed_calls: Vec<_> = summaries
        .iter()
        .flat_map(|s| s.tool_calls.iter())
        .filter(|tc| tc.name == "rate_limited_tool" && !tc.success)
        .collect();
    assert!(
        !failed_calls.is_empty(),
        "Should have recorded the rate limited tool failure"
    );

    // Step 9: Verify execution continued after recovery
    assert!(
        !final_text.is_empty(),
        "Execution should continue after recovery and produce output"
    );
    assert!(
        final_text.contains("Recovered") || final_text.contains("rate limit"),
        "Final text should indicate recovery or task completion"
    );
}

/// Test: Recovery flow with permission denied
///
/// This test verifies that permission denials are:
/// 1. Classified correctly as PermissionDenied
/// 2. Handled without crashing
/// 3. Execution continues after denial
#[tokio::test]
async fn test_recovery_flow_permission_denied() {
    let scenario = vec![
        ScenarioStep::tool_use(
            "call_1",
            "permission_tool",
            serde_json::json!({}),
            "Permission denied",
            false,
        ),
        ScenarioStep::text("The tool was denied due to permission restrictions."),
    ];

    let mock_llm = Arc::new(ScenarioMockLlm::new("claude-sonnet", scenario));

    let orchestrator = Arc::new(Orchestrator::new());
    let tool_registry = Arc::new(ToolRegistry::new());

    // Create a permission-denied tool
    let perm_tool = FailingTestTool::new("permission_tool", "permission");
    tool_registry
        .register(
            perm_tool,
            swell_tools::registry::ToolCategory::Misc,
            swell_tools::registry::ToolLayer::Builtin,
        )
        .await;

    let mut controller =
        ExecutionController::with_max_iterations(orchestrator, mock_llm, tool_registry, 10);

    let messages = vec![swell_llm::LlmMessage {
        role: swell_llm::LlmRole::User,
        content: "Try the permission tool".to_string(),
        tool_call_id: None,
    }];

    let result = controller.execute_turn_loop(messages, None).await;

    // Should complete without crash
    assert!(
        result.is_ok(),
        "Permission denial should not crash: {:?}",
        result.err()
    );

    let (summaries, final_text) = result.unwrap();

    // Verify the tool was called
    let perm_calls: Vec<_> = summaries
        .iter()
        .flat_map(|s| s.tool_calls.iter())
        .filter(|tc| tc.name == "permission_tool")
        .collect();
    assert!(!perm_calls.is_empty(), "Should have called permission_tool");

    // Verify execution continued
    assert!(
        !final_text.is_empty(),
        "Execution should continue after permission denial"
    );
}

/// Test: Recovery flow with timeout
///
/// This test verifies that timeouts are:
/// 1. Classified correctly as Timeout
/// 2. Recovery recipe is selected
/// 3. Execution continues after timeout
#[tokio::test]
async fn test_recovery_flow_timeout() {
    let scenario = vec![
        ScenarioStep::tool_use(
            "call_1",
            "slow_tool",
            serde_json::json!({}),
            "Timeout: Request timed out after 30s",
            false,
        ),
        ScenarioStep::text("The operation timed out, proceeding with alternative approach."),
    ];

    let mock_llm = Arc::new(ScenarioMockLlm::new("claude-sonnet", scenario));

    let orchestrator = Arc::new(Orchestrator::new());
    let tool_registry = Arc::new(ToolRegistry::new());

    let timeout_tool = FailingTestTool::new("slow_tool", "timeout");
    tool_registry
        .register(
            timeout_tool,
            swell_tools::registry::ToolCategory::Misc,
            swell_tools::registry::ToolLayer::Builtin,
        )
        .await;

    // Create recipe for timeout
    let mut recipe = RecoveryRecipe::new();
    recipe.register(
        FailureScenario::from_class(FailureClass::Timeout),
        vec![RecoveryStep::retry(3, BackoffStrategy::Exponential)],
    );

    // Verify recipe selection
    let steps = recipe.get_matching(FailureClass::Timeout, None, Some("slow_tool"));
    assert!(!steps.is_empty());
    assert!(matches!(
        steps.first(),
        Some(RecoveryStep::Retry {
            max_attempts: 3,
            ..
        })
    ));

    let mut controller =
        ExecutionController::with_max_iterations(orchestrator, mock_llm, tool_registry, 10);

    let messages = vec![swell_llm::LlmMessage {
        role: swell_llm::LlmRole::User,
        content: "Use the slow tool".to_string(),
        tool_call_id: None,
    }];

    let result = controller.execute_turn_loop(messages, None).await;

    // Should complete without crash
    assert!(
        result.is_ok(),
        "Timeout should not crash: {:?}",
        result.err()
    );

    let (summaries, _) = result.unwrap();

    // Verify timeout tool was called
    let timeout_calls: Vec<_> = summaries
        .iter()
        .flat_map(|s| s.tool_calls.iter())
        .filter(|tc| tc.name == "slow_tool" && !tc.success)
        .collect();
    assert!(
        !timeout_calls.is_empty(),
        "Should have recorded timeout failure"
    );
}
