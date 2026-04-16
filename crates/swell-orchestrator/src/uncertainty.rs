//! Uncertainty handling for VAL-ORCH-014: Confidence-based pausing.
//!
//! This module provides the mechanism for agents to pause execution when their
//! confidence drops below a configurable threshold, emit structured clarification
//! requests, and wait for responses before resuming.
//!
//! # Features
//!
//! - **Confidence threshold per agent type**: Each agent role (Generator, Planner, etc.)
//!   can have its own threshold configured via `ConfidenceThresholdConfig`
//! - **Structured clarification events**: Emits `UncertaintyClarificationEvent` with
//!   reason, context, and suggested resolution options
//! - **Blocking pause**: Execution does not resume until clarification response provided
//! - **Configurable thresholds**: Thresholds can be configured per agent type in
//!   `.swell/settings.json`

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

use serde::{Deserialize, Serialize};
use swell_core::{AgentId, AgentRole};

/// Confidence level classification for agent outputs.
///
/// Used to determine when uncertainty pause should be triggered and what
/// kind of response is expected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfidenceLevel {
    /// High confidence - agent is certain about the output
    High,
    /// Medium confidence - agent has some uncertainty but can proceed
    Medium,
    /// Low confidence - agent is uncertain and may need clarification
    Low,
    /// Very low confidence - agent explicitly requests clarification
    VeryLow,
}

impl ConfidenceLevel {
    /// Classify a confidence score into a level.
    ///
    /// Uses the standard thresholds:
    /// - >= 0.8: High
    /// - >= 0.6: Medium
    /// - >= 0.4: Low
    /// - < 0.4: VeryLow
    pub fn from_score(score: f64) -> Self {
        if score >= 0.8 {
            ConfidenceLevel::High
        } else if score >= 0.6 {
            ConfidenceLevel::Medium
        } else if score >= 0.4 {
            ConfidenceLevel::Low
        } else {
            ConfidenceLevel::VeryLow
        }
    }

    /// Returns true if this confidence level requires a pause for clarification.
    pub fn requires_pause(&self) -> bool {
        matches!(self, ConfidenceLevel::Low | ConfidenceLevel::VeryLow)
    }

    /// Returns a human-readable description of this confidence level.
    pub fn description(&self) -> &'static str {
        match self {
            ConfidenceLevel::High => "High confidence - proceeding without pause",
            ConfidenceLevel::Medium => "Medium confidence - acceptable but worth noting",
            ConfidenceLevel::Low => "Low confidence - clarification recommended",
            ConfidenceLevel::VeryLow => "Very low confidence - clarification required",
        }
    }
}

/// A structured clarification request emitted when agent confidence drops below threshold.
///
/// This event captures all the context needed for an operator or automated system
/// to provide a clarifying response that allows the agent to resume execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UncertaintyClarificationEvent {
    /// Unique identifier for this clarification request
    pub request_id: Uuid,
    /// Task requiring clarification
    pub task_id: Uuid,
    /// Agent that generated the uncertainty
    pub agent_id: Option<AgentId>,
    /// Agent role that generated the uncertainty
    pub agent_role: AgentRole,
    /// Agent's self-reported confidence score (0.0 to 1.0)
    pub confidence_score: f64,
    /// Threshold the score needed to be above to avoid pause
    pub confidence_threshold: f64,
    /// Classification level based on the score
    pub confidence_level: ConfidenceLevel,
    /// Why confidence dropped below threshold
    pub reason: String,
    /// Current context/state when uncertainty was detected
    pub current_context: String,
    /// Suggested resolution options for the operator to choose from
    pub suggested_options: Vec<ClarificationOption>,
    /// Timestamp when the request was created
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// Whether this request has been responded to
    pub responded: bool,
    /// The response received (if any)
    pub response: Option<ClarificationResponse>,
}

impl UncertaintyClarificationEvent {
    /// Create a new clarification event with the given parameters.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        task_id: Uuid,
        agent_id: Option<AgentId>,
        agent_role: AgentRole,
        confidence_score: f64,
        confidence_threshold: f64,
        reason: String,
        current_context: String,
        suggested_options: Vec<ClarificationOption>,
    ) -> Self {
        Self {
            request_id: Uuid::new_v4(),
            task_id,
            agent_id,
            agent_role,
            confidence_score,
            confidence_threshold,
            confidence_level: ConfidenceLevel::from_score(confidence_score),
            reason,
            current_context,
            suggested_options,
            created_at: chrono::Utc::now(),
            responded: false,
            response: None,
        }
    }

    /// Returns true if this clarification request needs a response.
    pub fn needs_response(&self) -> bool {
        !self.responded
    }

    /// Apply a clarification response to this request.
    pub fn respond(&mut self, response: ClarificationResponse) {
        self.response = Some(response);
        self.responded = true;
    }
}

/// A suggested resolution option for a clarification request.
///
/// These are generated by the agent based on the specific uncertainty and
/// provide the operator with concrete options for how to proceed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClarificationOption {
    /// Unique identifier for this option
    pub option_id: String,
    /// Human-readable label for this option
    pub label: String,
    /// Detailed description of what this option does
    pub description: String,
    /// Whether this option requires human input or is fully automated
    pub requires_human_input: bool,
    /// Priority/weight for sorting options (lower = more recommended)
    pub priority: u32,
}

impl ClarificationOption {
    /// Create a new clarification option.
    pub fn new(
        option_id: &str,
        label: &str,
        description: &str,
        requires_human_input: bool,
        priority: u32,
    ) -> Self {
        Self {
            option_id: option_id.to_string(),
            label: label.to_string(),
            description: description.to_string(),
            requires_human_input,
            priority,
        }
    }

    /// Create the standard set of clarification options for a low-confidence situation.
    ///
    /// These options are designed to handle most uncertainty scenarios:
    /// - Continue with current approach
    /// - Provide more specific guidance
    /// - Retry with additional context
    /// - Escalate to human review
    pub fn standard_options() -> Vec<ClarificationOption> {
        vec![
            ClarificationOption::new(
                "continue",
                "Continue with current approach",
                "Proceed with the agent's current plan despite lower confidence. Use this when the task seems straightforward despite the agent's uncertainty.",
                false,
                1,
            ),
            ClarificationOption::new(
                "provide_guidance",
                "Provide more specific guidance",
                "Give the agent more explicit instructions about what to do. Include examples or references to similar tasks.",
                true,
                2,
            ),
            ClarificationOption::new(
                "retry_context",
                "Retry with expanded context",
                "Ask the agent to retry with additional context from memory, similar tasks, or user-provided information.",
                false,
                3,
            ),
            ClarificationOption::new(
                "escalate",
                "Escalate to human review",
                "Pause the task and require human operator intervention before proceeding. Use for complex or high-risk situations.",
                true,
                4,
            ),
        ]
    }
}

/// Response to a clarification request.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClarificationResponse {
    /// The option that was selected
    pub selected_option: String,
    /// Optional additional context/guidance from the responder
    pub additional_guidance: Option<String>,
    /// Whether the clarification should be injected as system context
    pub inject_as_context: bool,
    /// Timestamp when the response was provided
    pub responded_at: chrono::DateTime<chrono::Utc>,
}

impl ClarificationResponse {
    /// Create a new clarification response with the selected option.
    pub fn new(selected_option: String) -> Self {
        Self {
            selected_option,
            additional_guidance: None,
            inject_as_context: true,
            responded_at: chrono::Utc::now(),
        }
    }

    /// Create a response with additional guidance.
    pub fn with_guidance(mut self, guidance: String) -> Self {
        self.additional_guidance = Some(guidance);
        self
    }

    /// Create a response that should NOT be injected as context.
    pub fn without_injection(mut self) -> Self {
        self.inject_as_context = false;
        self
    }

    /// Convert this response to an LLM message for injection into the conversation.
    ///
    /// When `inject_as_context` is true, this returns a system message that will
    /// be prepended to the conversation to guide the agent's next action.
    pub fn to_llm_message(&self) -> Option<swell_core::LlmMessage> {
        let content = match &self.additional_guidance {
            Some(guidance) => format!(
                "Clarification response: {}\n\nAdditional guidance: {}",
                self.selected_option, guidance
            ),
            None => format!("Clarification response: {}", self.selected_option),
        };

        Some(swell_core::LlmMessage {
            role: swell_llm::LlmRole::System,
            content,
            tool_call_id: None,
        })
    }
}

/// Manages uncertainty clarification requests and responses.
///
/// This manager:
/// - Tracks pending clarification requests
/// - Allows injection of clarification responses into ongoing conversations
/// - Provides access to active requests for monitoring/debugging
pub struct UncertaintyManager {
    /// Active clarification requests, keyed by request ID
    pending_requests: Arc<RwLock<HashMap<Uuid, UncertaintyClarificationEvent>>>,
    /// Completed requests for audit/history
    completed_requests: Arc<RwLock<Vec<UncertaintyClarificationEvent>>>,
}

impl UncertaintyManager {
    /// Create a new uncertainty manager.
    pub fn new() -> Self {
        Self {
            pending_requests: Arc::new(RwLock::new(HashMap::new())),
            completed_requests: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Create a new clarification request and register it with the manager.
    ///
    /// Returns the request ID that can be used to check for a response later.
    pub async fn create_request(&self, event: UncertaintyClarificationEvent) -> Uuid {
        let request_id = event.request_id;
        let mut pending = self.pending_requests.write().await;
        pending.insert(request_id, event);
        request_id
    }

    /// Check if a clarification request has been responded to.
    ///
    /// Returns the response if available, or None if still pending.
    pub async fn get_response(&self, request_id: Uuid) -> Option<ClarificationResponse> {
        let pending = self.pending_requests.read().await;
        pending.get(&request_id).and_then(|e| e.response.clone())
    }

    /// Check if a clarification request needs a response.
    pub async fn is_pending(&self, request_id: Uuid) -> bool {
        let pending = self.pending_requests.read().await;
        pending
            .get(&request_id)
            .map(|e| e.needs_response())
            .unwrap_or(false)
    }

    /// Provide a response to a clarification request.
    ///
    /// This moves the request from pending to completed.
    pub async fn respond(&self, request_id: Uuid, response: ClarificationResponse) -> bool {
        let mut pending = self.pending_requests.write().await;

        if let Some(event) = pending.remove(&request_id) {
            let mut completed = self.completed_requests.write().await;

            // Update the event with the response
            let mut event_with_response = event;
            event_with_response.respond(response);

            completed.push(event_with_response);
            return true;
        }

        false
    }

    /// Wait for a clarification response with a timeout.
    ///
    /// Polls every `interval_secs` until `timeout_secs` has elapsed or a response is received.
    /// Checks both pending requests (in-flight) and completed requests (already responded) on each
    /// poll iteration so that a response is detected even when the event has been moved to the
    /// completed list by [`respond`].
    ///
    /// Returns the response if one arrives, or `None` if the timeout is reached.
    pub async fn wait_for_response(
        &self,
        request_id: Uuid,
        timeout_secs: u64,
        interval_secs: u64,
    ) -> Option<ClarificationResponse> {
        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(timeout_secs);
        // Use a minimum 1 ms sleep so the loop yields to the runtime even when interval_secs = 0.
        let interval = if interval_secs == 0 {
            std::time::Duration::from_millis(1)
        } else {
            std::time::Duration::from_secs(interval_secs)
        };

        loop {
            // Check pending requests (event not yet responded)
            {
                let pending = self.pending_requests.read().await;
                if let Some(event) = pending.get(&request_id) {
                    if let Some(response) = event.response.clone() {
                        return Some(response);
                    }
                }
            }
            // Check completed requests (event has been responded to and moved)
            {
                let completed = self.completed_requests.read().await;
                if let Some(event) = completed.iter().find(|e| e.request_id == request_id) {
                    // A completed event always has a response
                    if let Some(response) = event.response.clone() {
                        return Some(response);
                    }
                }
            }

            if start.elapsed() >= timeout {
                return None;
            }

            tokio::time::sleep(interval).await;
        }
    }

    /// Get all pending clarification requests.
    pub async fn get_pending_requests(&self) -> Vec<UncertaintyClarificationEvent> {
        let pending = self.pending_requests.read().await;
        pending.values().cloned().collect()
    }

    /// Get pending requests for a specific task.
    pub async fn get_pending_for_task(&self, task_id: Uuid) -> Vec<UncertaintyClarificationEvent> {
        let pending = self.pending_requests.read().await;
        pending
            .values()
            .filter(|e| e.task_id == task_id)
            .cloned()
            .collect()
    }

    /// Get all completed clarification requests (for audit/history).
    pub async fn get_completed_requests(&self) -> Vec<UncertaintyClarificationEvent> {
        let completed = self.completed_requests.read().await;
        completed.clone()
    }

    /// Clear all pending requests (used when task is cancelled).
    pub async fn clear_task(&self, task_id: Uuid) {
        let mut pending = self.pending_requests.write().await;
        pending.retain(|_, e| e.task_id != task_id);
    }

    /// Get the count of pending and completed requests.
    pub async fn stats(&self) -> UncertaintyStats {
        let pending_count = self.pending_requests.read().await.len();
        let completed_count = self.completed_requests.read().await.len();

        UncertaintyStats {
            pending_count,
            completed_count,
            total_count: pending_count + completed_count,
        }
    }
}

impl Default for UncertaintyManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Statistics about uncertainty handling.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UncertaintyStats {
    pub pending_count: usize,
    pub completed_count: usize,
    pub total_count: usize,
}

/// Generate default suggested options based on the agent role and confidence level.
pub fn generate_suggested_options(
    agent_role: AgentRole,
    _confidence_level: ConfidenceLevel,
) -> Vec<ClarificationOption> {
    let mut options = ClarificationOption::standard_options();

    // Add role-specific options based on the agent role
    match agent_role {
        AgentRole::Generator => {
            // Add generator-specific options
            options.push(ClarificationOption::new(
                "simplify_scope",
                "Simplify the scope",
                "Reduce the complexity of what's being generated. Break down the task into smaller, more manageable pieces.",
                false,
                5,
            ));
        }
        AgentRole::Planner => {
            // Add planner-specific options
            options.push(ClarificationOption::new(
                "replan",
                "Replan with different approach",
                "Ask the planner to create a new plan using a different strategy or approach.",
                false,
                5,
            ));
        }
        AgentRole::Evaluator => {
            // Add evaluator-specific options
            options.push(ClarificationOption::new(
                "relax_criteria",
                "Relax validation criteria",
                "Adjust the validation criteria to be more lenient. Only use when the core requirements are met.",
                true,
                5,
            ));
        }
        AgentRole::Coder | AgentRole::Reviewer | AgentRole::Refactorer => {
            // Add coder/reviewer/refactorer-specific options
            options.push(ClarificationOption::new(
                "use_reference",
                "Use reference implementation",
                "Provide a reference implementation or example for the agent to follow.",
                true,
                5,
            ));
        }
        AgentRole::TestWriter | AgentRole::DocWriter | AgentRole::Researcher => {
            // For these agent types, use the default options without additional customization
            // The standard options cover most scenarios for testing, documentation, and research tasks
        }
    }

    // Sort by priority
    options.sort_by_key(|o| o.priority);

    options
}

/// Check if confidence score is below threshold and return the confidence level if a pause
/// is needed.
///
/// The threshold is the primary control: any score strictly below `threshold` triggers a pause
/// regardless of the absolute confidence level classification. Returns `None` when
/// `confidence_score >= threshold`.
///
/// # Examples
///
/// ```
/// # use swell_orchestrator::uncertainty::{check_confidence_threshold, ConfidenceLevel};
/// assert!(check_confidence_threshold(0.3, 0.5).is_some()); // 0.3 < 0.5 → pause
/// assert!(check_confidence_threshold(0.5, 0.5).is_none()); // not strictly less → no pause
/// assert!(check_confidence_threshold(0.7, 0.5).is_none()); // above threshold → no pause
/// // Threshold > 0.6: a Medium-confidence score can still trigger pause
/// assert!(check_confidence_threshold(0.65, 0.7).is_some());
/// ```
pub fn check_confidence_threshold(
    confidence_score: f64,
    threshold: f64,
) -> Option<ConfidenceLevel> {
    if confidence_score < threshold {
        Some(ConfidenceLevel::from_score(confidence_score))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_confidence_level_classification() {
        assert_eq!(ConfidenceLevel::from_score(0.9), ConfidenceLevel::High);
        assert_eq!(ConfidenceLevel::from_score(0.8), ConfidenceLevel::High);
        assert_eq!(ConfidenceLevel::from_score(0.7), ConfidenceLevel::Medium);
        assert_eq!(ConfidenceLevel::from_score(0.6), ConfidenceLevel::Medium);
        assert_eq!(ConfidenceLevel::from_score(0.5), ConfidenceLevel::Low);
        assert_eq!(ConfidenceLevel::from_score(0.3), ConfidenceLevel::VeryLow);
    }

    #[tokio::test]
    async fn test_confidence_level_requires_pause() {
        assert!(!ConfidenceLevel::High.requires_pause());
        assert!(!ConfidenceLevel::Medium.requires_pause());
        assert!(ConfidenceLevel::Low.requires_pause());
        assert!(ConfidenceLevel::VeryLow.requires_pause());
    }

    #[tokio::test]
    async fn test_uncertainty_manager_create_request() {
        let manager = UncertaintyManager::new();

        let event = UncertaintyClarificationEvent::new(
            Uuid::new_v4(),
            None,
            swell_core::AgentRole::Generator,
            0.3,
            0.5,
            "Low confidence in implementation approach".to_string(),
            "Agent produced code but is uncertain about correctness".to_string(),
            ClarificationOption::standard_options(),
        );

        let request_id = manager.create_request(event).await;
        assert!(manager.is_pending(request_id).await);
        assert!(manager.get_response(request_id).await.is_none());
    }

    #[tokio::test]
    async fn test_uncertainty_manager_respond() {
        let manager = UncertaintyManager::new();

        let event = UncertaintyClarificationEvent::new(
            Uuid::new_v4(),
            None,
            swell_core::AgentRole::Generator,
            0.3,
            0.5,
            "Low confidence".to_string(),
            "Context".to_string(),
            ClarificationOption::standard_options(),
        );

        let request_id = manager.create_request(event).await;

        let response = ClarificationResponse::new("continue".to_string())
            .with_guidance("Proceed with current implementation".to_string());

        let result = manager.respond(request_id, response).await;
        assert!(result);
        assert!(!manager.is_pending(request_id).await);
        assert!(manager.get_response(request_id).await.is_none()); // Already moved to completed
    }

    #[tokio::test]
    async fn test_clarification_response_to_llm_message() {
        let response = ClarificationResponse::new("provide_guidance".to_string())
            .with_guidance("Focus on error handling".to_string());

        let message = response.to_llm_message().unwrap();
        assert_eq!(message.role, swell_llm::LlmRole::System);
        assert!(message.content.contains("provide_guidance"));
        assert!(message.content.contains("Focus on error handling"));
    }

    #[tokio::test]
    async fn test_generate_suggested_options() {
        let options = generate_suggested_options(AgentRole::Generator, ConfidenceLevel::VeryLow);

        // Should have standard options plus generator-specific
        assert!(options.len() >= 5);
        assert!(options.iter().any(|o| o.option_id == "continue"));
        assert!(options.iter().any(|o| o.option_id == "simplify_scope"));
    }

    #[tokio::test]
    async fn test_check_confidence_threshold() {
        // Below threshold
        let result = check_confidence_threshold(0.3, 0.5);
        assert!(result.is_some());
        assert_eq!(result.unwrap(), ConfidenceLevel::VeryLow);

        // At threshold (should not trigger)
        let result = check_confidence_threshold(0.5, 0.5);
        assert!(result.is_none());

        // Above threshold
        let result = check_confidence_threshold(0.7, 0.5);
        assert!(result.is_none());
    }
}
