//! Integration test: Cross-permission-in-execution
//!
//! This test verifies that when a tool is denied by the permission system,
//! the error output is returned to the agent and the agent continues executing
//! (does not crash). This validates graceful permission denial handling.
//!
//! Expected behavior:
//! - Denied tool returns error output to agent
//! - Agent continues execution after permission denial
//! - No crash or panic on permission denial
//!
//! This test validates VAL-CROSS-002: Agent continues after permission denial.

use async_trait::async_trait;
use std::sync::Arc;
use swell_core::traits::Tool;
use swell_core::{PermissionTier, SwellError, ToolOutput, ToolResultContent};
use swell_llm::mock::{ScenarioMockLlm, ScenarioStep};
use swell_orchestrator::{builder::OrchestratorBuilder, ExecutionController};
use swell_tools::ToolRegistry;

/// A test tool that requires a specific permission tier
struct TieredTestTool {
    name: String,
    permission_tier: PermissionTier,
}

impl TieredTestTool {
    fn new(name: &str, permission_tier: PermissionTier) -> Self {
        Self {
            name: name.to_string(),
            permission_tier,
        }
    }
}

#[async_trait]
impl Tool for TieredTestTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> String {
        format!(
            "A test tool requiring {:?} permission",
            self.permission_tier
        )
    }

    fn risk_level(&self) -> swell_core::ToolRiskLevel {
        swell_core::ToolRiskLevel::Read
    }

    fn permission_tier(&self) -> PermissionTier {
        self.permission_tier
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
        Ok(ToolOutput {
            is_error: false,
            content: vec![ToolResultContent::Text("Tool executed".to_string())],
        })
    }
}

/// Test: Agent continues executing after permission denial
///
/// This test verifies that when the LLM requests a denied tool, the agent:
/// 1. Receives the permission denial as a tool result
/// 2. Continues the execution loop (doesn't crash)
/// 3. Can complete normally with subsequent turns
#[tokio::test]
async fn test_agent_continues_after_permission_denial() {
    // Create a scenario where:
    // - Turn 1: LLM requests a denied tool
    // - Turn 2: LLM gives up and returns text (no more tool calls)
    let scenario = vec![
        // Turn 1: LLM requests the denied tool
        ScenarioStep::tool_use(
            "call_denied",
            "deny_test_tool",
            serde_json::json!({}),
            "Permission denied: tool 'deny_test_tool' requires Deny permission",
            false, // success = false indicates tool was denied
        ),
        // Turn 2: LLM gives up and returns text
        ScenarioStep::text("I understand the tool was denied. I'll proceed with what I can do."),
    ];

    let mock_llm = Arc::new(ScenarioMockLlm::new("claude-sonnet", scenario));

    // Create orchestrator and tool registry
    let orchestrator = OrchestratorBuilder::new().build();
    let tool_registry = Arc::new(ToolRegistry::new());

    // Register a tool that requires Deny permission
    let deny_tool = TieredTestTool::new("deny_test_tool", PermissionTier::Deny);
    tool_registry
        .register(
            deny_tool,
            swell_tools::registry::ToolCategory::Misc,
            swell_tools::registry::ToolLayer::Builtin,
        )
        .await;

    // Create ExecutionController with high max_iterations to allow multiple turns
    let mut controller = ExecutionController::with_max_iterations(
        Arc::downgrade(&orchestrator),
        mock_llm.clone(),
        tool_registry.clone(),
        10,
    );

    // Execute through the turn loop (the public API)
    let messages = vec![swell_llm::LlmMessage {
        role: swell_llm::LlmRole::User,
        content: "Try to use the denied tool".to_string(),
        tool_call_id: None,
    }];

    let result = controller.execute_turn_loop(messages, None).await;

    // Should complete without error (denial is surfaced as ToolOutput with is_error=true,
    // not as an Err that would crash the turn loop)
    assert!(
        result.is_ok(),
        "execute_turn_loop should complete without panic/crash: {:?}",
        result.err()
    );

    let (summaries, final_text) = result.unwrap();

    // Verify that turn summaries were collected
    assert!(
        !summaries.is_empty(),
        "Should have at least one turn summary"
    );

    // Check that at least one turn had the denied tool
    let denied_turns: Vec<_> = summaries
        .iter()
        .filter(|s| {
            s.tool_calls
                .iter()
                .any(|tc| tc.name == "deny_test_tool" && !tc.success)
        })
        .collect();

    assert!(
        !denied_turns.is_empty(),
        "Should have at least one turn with denied tool call"
    );

    // Verify the agent continued after denial and produced output
    assert!(
        !final_text.is_empty(),
        "Agent should have continued after denial and produced text output: {:?}",
        final_text
    );
}

/// Test: Full execution path with permission denial in turn loop
///
/// This test verifies the complete flow:
/// 1. Agent makes a tool call
/// 2. Tool execution is denied by permission system
/// 3. Denial is surfaced as error in tool result
/// 4. Agent receives the error and continues
/// 5. Execution completes without crash
#[tokio::test]
async fn test_turn_loop_handles_permission_denial_gracefully() {
    // Create a scenario:
    // - Turn 1: Request denied tool -> gets denial error
    // - Turn 2: Request a different approach -> gets text
    let scenario = vec![
        // Turn 1: LLM requests denied tool
        ScenarioStep::tool_use(
            "call_1",
            "deny_test_tool",
            serde_json::json!({}),
            "Permission denied: tool 'deny_test_tool' requires Deny permission",
            false,
        ),
        // Turn 2: LLM tries a different approach without the denied tool
        ScenarioStep::text("I'll try a different approach without using that tool."),
    ];

    let mock_llm = Arc::new(ScenarioMockLlm::new("claude-sonnet", scenario));

    // Create orchestrator and tool registry
    let orchestrator = OrchestratorBuilder::new().build();
    let tool_registry = Arc::new(ToolRegistry::new());

    // Register the denied tool
    let deny_tool = TieredTestTool::new("deny_test_tool", PermissionTier::Deny);
    tool_registry
        .register(
            deny_tool,
            swell_tools::registry::ToolCategory::Misc,
            swell_tools::registry::ToolLayer::Builtin,
        )
        .await;

    // Create ExecutionController with high max_iterations to allow multiple turns
    let mut controller = ExecutionController::with_max_iterations(
        Arc::downgrade(&orchestrator),
        mock_llm.clone(),
        tool_registry.clone(),
        10,
    );

    // Create messages for turn loop
    let messages = vec![swell_llm::LlmMessage {
        role: swell_llm::LlmRole::User,
        content: "Try to use the denied tool".to_string(),
        tool_call_id: None,
    }];

    // Execute turn loop - should handle denial gracefully
    let result = controller.execute_turn_loop(messages, None).await;

    // Should complete without error (denial is surfaced as ToolOutput, not as Err)
    assert!(
        result.is_ok(),
        "execute_turn_loop should complete without panic/crash: {:?}",
        result.err()
    );

    let (summaries, final_text) = result.unwrap();

    // Verify that turns happened and denial was handled
    assert!(
        !summaries.is_empty(),
        "Should have at least one turn summary"
    );

    // Check that at least one turn had the denied tool
    let denied_turns: Vec<_> = summaries
        .iter()
        .filter(|s| {
            s.tool_calls
                .iter()
                .any(|tc| tc.name == "deny_test_tool" && !tc.success)
        })
        .collect();

    assert!(
        !denied_turns.is_empty(),
        "Should have at least one turn with denied tool call"
    );

    // Verify the final text was received (agent continued)
    assert!(
        !final_text.is_empty(),
        "Agent should have continued after denial and produced some output, got: {:?}",
        final_text
    );
}

/// Test: Multiple denied tools in sequence do not crash
#[tokio::test]
async fn test_multiple_denied_tools_do_not_crash() {
    // Create a scenario with multiple denied tool calls in sequence
    let scenario = vec![
        ScenarioStep::tool_use(
            "call_1",
            "deny_test_tool",
            serde_json::json!({}),
            "Permission denied: tool 'deny_test_tool' requires Deny permission",
            false,
        ),
        ScenarioStep::tool_use(
            "call_2",
            "another_deny_tool",
            serde_json::json!({}),
            "Permission denied: tool 'another_deny_tool' requires Deny permission",
            false,
        ),
        ScenarioStep::text(
            "I've tried the tools but they were denied. I'll summarize what I can do.",
        ),
    ];

    let mock_llm = Arc::new(ScenarioMockLlm::new("claude-sonnet", scenario));

    let orchestrator = OrchestratorBuilder::new().build();
    let tool_registry = Arc::new(ToolRegistry::new());

    // Register multiple denied tools
    let deny_tool1 = TieredTestTool::new("deny_test_tool", PermissionTier::Deny);
    let deny_tool2 = TieredTestTool::new("another_deny_tool", PermissionTier::Deny);
    tool_registry
        .register(
            deny_tool1,
            swell_tools::registry::ToolCategory::Misc,
            swell_tools::registry::ToolLayer::Builtin,
        )
        .await;
    tool_registry
        .register(
            deny_tool2,
            swell_tools::registry::ToolCategory::Misc,
            swell_tools::registry::ToolLayer::Builtin,
        )
        .await;

    let mut controller = ExecutionController::with_max_iterations(
        Arc::downgrade(&orchestrator),
        mock_llm.clone(),
        tool_registry.clone(),
        10,
    );

    let messages = vec![swell_llm::LlmMessage {
        role: swell_llm::LlmRole::User,
        content: "Try both denied tools".to_string(),
        tool_call_id: None,
    }];

    let result = controller.execute_turn_loop(messages, None).await;

    // Should complete without crash
    assert!(
        result.is_ok(),
        "execute_turn_loop should handle multiple denials without crash: {:?}",
        result.err()
    );

    let (summaries, _) = result.unwrap();

    // Verify multiple denied tool calls were recorded
    let denied_calls: Vec<_> = summaries
        .iter()
        .flat_map(|s| s.tool_calls.iter())
        .filter(|tc| !tc.success)
        .collect();

    assert!(
        denied_calls.len() >= 2,
        "Should have recorded at least 2 denied tool calls, got {}",
        denied_calls.len()
    );
}
