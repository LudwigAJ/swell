//! Retry policy implementation for orchestrator task retry handling.
//!
//! This module implements the retry policy:
//! - First 2 retries: same agent
//! - 3rd retry: switch model
//! - 4+ retries: escalate to human
//!
//! # Architecture
//!
//! The retry policy is evaluated when a task is rejected and needs to be
//! retried. The [`RetryPolicy`] struct tracks retry state and decides what
//! action to take based on the iteration count.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Maximum retries before escalating to human
pub const MAX_RETRIES_BEFORE_ESCALATION: u32 = 3;

/// Retry count threshold for switching models
pub const MODEL_SWITCH_RETRY_COUNT: u32 = 2;

/// Decision made by the retry policy
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RetryDecision {
    /// Retry with the same agent (iteration 1-2)
    RetrySameAgent,
    /// Retry with a different model (iteration 3)
    RetryWithDifferentModel,
    /// Escalate to human for intervention (iteration 4+)
    EscalateToHuman,
    /// Don't retry at all
    NoRetry,
}

impl RetryDecision {
    /// Returns true if this decision allows retrying
    pub fn is_retry(&self) -> bool {
        matches!(
            self,
            RetryDecision::RetrySameAgent | RetryDecision::RetryWithDifferentModel
        )
    }

    /// Returns true if this decision escalates to human
    pub fn is_escalation(&self) -> bool {
        matches!(self, RetryDecision::EscalateToHuman)
    }
}

impl std::fmt::Display for RetryDecision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RetryDecision::RetrySameAgent => write!(f, "RetrySameAgent"),
            RetryDecision::RetryWithDifferentModel => write!(f, "RetryWithDifferentModel"),
            RetryDecision::EscalateToHuman => write!(f, "EscalateToHuman"),
            RetryDecision::NoRetry => write!(f, "NoRetry"),
        }
    }
}

/// Retry policy state for a task
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryState {
    /// Current iteration count (0 = initial attempt)
    pub iteration_count: u32,
    /// Model used in the previous attempt (if any)
    pub previous_model: Option<String>,
    /// Original model when task was first attempted
    pub original_model: Option<String>,
    /// Alternative model to switch to (for 3rd retry)
    pub alternative_model: Option<String>,
    /// Agent ID used in previous attempt
    pub previous_agent_id: Option<Uuid>,
}

impl RetryState {
    /// Create a new retry state for a fresh task
    pub fn new() -> Self {
        Self {
            iteration_count: 0,
            previous_model: None,
            original_model: None,
            alternative_model: None,
            previous_agent_id: None,
        }
    }

    /// Create a retry state with initial model
    pub fn with_model(model: String) -> Self {
        Self {
            iteration_count: 0,
            previous_model: Some(model.clone()),
            original_model: Some(model),
            alternative_model: None,
            previous_agent_id: None,
        }
    }

    /// Record that a retry is about to happen, returning the next decision
    pub fn prepare_retry(&mut self) -> RetryDecision {
        self.iteration_count += 1;
        self.decide_retry()
    }

    /// Decide what action to take based on current iteration count
    pub fn decide_retry(&self) -> RetryDecision {
        match self.iteration_count {
            0 => RetryDecision::NoRetry, // Initial attempt, not a retry yet
            1 | 2 => RetryDecision::RetrySameAgent,
            3 => RetryDecision::RetryWithDifferentModel,
            _ => RetryDecision::EscalateToHuman,
        }
    }

    /// Record the model used for the current attempt
    pub fn record_model(&mut self, model: String) {
        self.previous_model = Some(model);
    }

    /// Record the alternative model to use for switching
    pub fn set_alternative_model(&mut self, model: String) {
        self.alternative_model = Some(model);
    }

    /// Record the agent used for the current attempt
    pub fn record_agent(&mut self, agent_id: Uuid) {
        self.previous_agent_id = Some(agent_id);
    }

    /// Get the model to use for the next attempt based on policy
    pub fn get_next_model(&self) -> Option<String> {
        let decision = self.decide_retry();
        match decision {
            RetryDecision::RetrySameAgent | RetryDecision::NoRetry => {
                // Use the same model as before
                self.previous_model.clone()
            }
            RetryDecision::RetryWithDifferentModel => {
                // Use alternative model if set, otherwise same model
                self.alternative_model
                    .clone()
                    .or(self.previous_model.clone())
            }
            RetryDecision::EscalateToHuman => None,
        }
    }

    /// Get the agent to use for the next attempt based on policy
    pub fn get_next_agent_same(&self) -> bool {
        matches!(self.decide_retry(), RetryDecision::RetrySameAgent)
    }

    /// Check if the current iteration count is at or beyond escalation threshold
    pub fn should_escalate(&self) -> bool {
        self.iteration_count >= MAX_RETRIES_BEFORE_ESCALATION
    }

    /// Reset retry state for a fresh start
    pub fn reset(&mut self) {
        self.iteration_count = 0;
        self.previous_model = self.original_model.clone();
        self.previous_agent_id = None;
    }
}

impl Default for RetryState {
    fn default() -> Self {
        Self::new()
    }
}

/// The main retry policy evaluator
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    /// Threshold for model switch (0-indexed, so 2 = 3rd attempt)
    pub model_switch_threshold: u32,
    /// Maximum retries before escalation
    pub max_retries: u32,
}

impl RetryPolicy {
    /// Create a new retry policy with default thresholds
    pub fn new() -> Self {
        Self {
            model_switch_threshold: MODEL_SWITCH_RETRY_COUNT,
            max_retries: MAX_RETRIES_BEFORE_ESCALATION,
        }
    }

    /// Create a retry policy with custom thresholds
    pub fn with_thresholds(model_switch_threshold: u32, max_retries: u32) -> Self {
        Self {
            model_switch_threshold,
            max_retries,
        }
    }

    /// Evaluate the retry policy for a given task
    ///
    /// Returns the decision for what to do next:
    /// - `RetrySameAgent`: Keep same agent, retry
    /// - `RetryWithDifferentModel`: Switch to different model, retry
    /// - `EscalateToHuman`: Human intervention required
    /// - `NoRetry`: No retry allowed (initial state)
    pub fn evaluate(&self, state: &RetryState) -> RetryDecision {
        self.evaluate_for_iteration(state.iteration_count)
    }

    /// Evaluate the retry policy for a given iteration count
    ///
    /// Returns the decision for what to do next:
    /// - `RetrySameAgent`: Keep same agent, retry (iteration 1-2)
    /// - `RetryWithDifferentModel`: Switch to different model, retry (iteration 3)
    /// - `EscalateToHuman`: Human intervention required (iteration 4+)
    /// - `NoRetry`: No retry allowed (iteration 0)
    pub fn evaluate_for_iteration(&self, iteration_count: u32) -> RetryDecision {
        // iteration_count is 0 for initial, 1 for first retry, etc.
        // So iteration_count = 1 or 2 means first or second retry
        // iteration_count = 3 means third retry (model switch)
        // iteration_count >= 4 means escalate

        let decision = match iteration_count {
            0 => RetryDecision::NoRetry, // Initial attempt
            1 | 2 => RetryDecision::RetrySameAgent,
            3 => RetryDecision::RetryWithDifferentModel,
            _ => RetryDecision::EscalateToHuman,
        };

        tracing::debug!(
            iteration = iteration_count,
            decision = ?decision,
            "Retry policy decision"
        );

        decision
    }

    /// Evaluate and return the model to use for the next attempt
    pub fn get_next_model(&self, state: &RetryState) -> Option<String> {
        match self.evaluate(state) {
            RetryDecision::RetrySameAgent => state.previous_model.clone(),
            RetryDecision::RetryWithDifferentModel => state
                .alternative_model
                .clone()
                .or(state.previous_model.clone()),
            RetryDecision::EscalateToHuman | RetryDecision::NoRetry => None,
        }
    }

    /// Check if escalation is needed
    pub fn should_escalate(&self, state: &RetryState) -> bool {
        state.iteration_count >= self.max_retries
    }

    /// Get the retry count for logging/display
    pub fn get_retry_number(&self, iteration_count: u32) -> u32 {
        // iteration_count 1 = retry #1, 2 = retry #2, etc.
        // iteration_count 0 = initial attempt, not a retry
        iteration_count.saturating_sub(1)
    }
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_retry_decision_is_retry() {
        assert!(RetryDecision::RetrySameAgent.is_retry());
        assert!(RetryDecision::RetryWithDifferentModel.is_retry());
        assert!(!RetryDecision::EscalateToHuman.is_retry());
        assert!(!RetryDecision::NoRetry.is_retry());
    }

    #[test]
    fn test_retry_decision_is_escalation() {
        assert!(!RetryDecision::RetrySameAgent.is_escalation());
        assert!(!RetryDecision::RetryWithDifferentModel.is_escalation());
        assert!(RetryDecision::EscalateToHuman.is_escalation());
        assert!(!RetryDecision::NoRetry.is_escalation());
    }

    #[test]
    fn test_retry_state_initial() {
        let state = RetryState::new();
        assert_eq!(state.iteration_count, 0);
        assert!(state.previous_model.is_none());
        assert!(state.original_model.is_none());
        assert!(state.previous_agent_id.is_none());
    }

    #[test]
    fn test_retry_state_with_model() {
        let state = RetryState::with_model("claude-sonnet".to_string());
        assert_eq!(state.iteration_count, 0);
        assert_eq!(state.previous_model, Some("claude-sonnet".to_string()));
        assert_eq!(state.original_model, Some("claude-sonnet".to_string()));
    }

    #[test]
    fn test_retry_state_prepare_retry() {
        let mut state = RetryState::with_model("claude-sonnet".to_string());

        // First retry
        let decision = state.prepare_retry();
        assert_eq!(decision, RetryDecision::RetrySameAgent);
        assert_eq!(state.iteration_count, 1);

        // Second retry
        let decision = state.prepare_retry();
        assert_eq!(decision, RetryDecision::RetrySameAgent);
        assert_eq!(state.iteration_count, 2);

        // Third retry (model switch)
        let decision = state.prepare_retry();
        assert_eq!(decision, RetryDecision::RetryWithDifferentModel);
        assert_eq!(state.iteration_count, 3);

        // Fourth retry (escalation)
        let decision = state.prepare_retry();
        assert_eq!(decision, RetryDecision::EscalateToHuman);
        assert_eq!(state.iteration_count, 4);

        // Fifth retry (escalation)
        let decision = state.prepare_retry();
        assert_eq!(decision, RetryDecision::EscalateToHuman);
        assert_eq!(state.iteration_count, 5);
    }

    #[test]
    fn test_retry_state_decide_retry() {
        let state = RetryState::new();

        // With 0 iterations
        assert_eq!(state.decide_retry(), RetryDecision::NoRetry);
    }

    #[test]
    fn test_retry_state_record_model() {
        let mut state = RetryState::new();
        state.record_model("claude-3-opus".to_string());
        assert_eq!(state.previous_model, Some("claude-3-opus".to_string()));
    }

    #[test]
    fn test_retry_state_set_alternative_model() {
        let mut state = RetryState::with_model("claude-sonnet".to_string());
        state.set_alternative_model("claude-3-opus".to_string());

        assert_eq!(state.previous_model, Some("claude-sonnet".to_string()));
        assert_eq!(state.alternative_model, Some("claude-3-opus".to_string()));
    }

    #[test]
    fn test_retry_state_get_next_model_same_agent() {
        let mut state = RetryState::with_model("claude-sonnet".to_string());
        state.iteration_count = 1; // First retry - same agent

        let model = state.get_next_model();
        assert_eq!(model, Some("claude-sonnet".to_string()));
    }

    #[test]
    fn test_retry_state_get_next_model_with_switch() {
        let mut state = RetryState::with_model("claude-sonnet".to_string());
        state.set_alternative_model("claude-3-opus".to_string());
        state.iteration_count = 3; // Third retry - switch model

        let model = state.get_next_model();
        assert_eq!(model, Some("claude-3-opus".to_string()));
    }

    #[test]
    fn test_retry_state_get_next_model_fallback() {
        let mut state = RetryState::with_model("claude-sonnet".to_string());
        state.iteration_count = 3; // Third retry - switch model
                                   // No alternative model set

        let model = state.get_next_model();
        // Falls back to previous model if alternative not set
        assert_eq!(model, Some("claude-sonnet".to_string()));
    }

    #[test]
    fn test_retry_state_should_escalate() {
        let mut state = RetryState::new();

        // Before threshold
        state.iteration_count = 2;
        assert!(!state.should_escalate());

        // At threshold
        state.iteration_count = 3;
        assert!(state.should_escalate());

        // Beyond threshold
        state.iteration_count = 5;
        assert!(state.should_escalate());
    }

    #[test]
    fn test_retry_policy_default() {
        let policy = RetryPolicy::new();
        assert_eq!(policy.model_switch_threshold, MODEL_SWITCH_RETRY_COUNT);
        assert_eq!(policy.max_retries, MAX_RETRIES_BEFORE_ESCALATION);
    }

    #[test]
    fn test_retry_policy_with_thresholds() {
        let policy = RetryPolicy::with_thresholds(1, 2);
        assert_eq!(policy.model_switch_threshold, 1);
        assert_eq!(policy.max_retries, 2);
    }

    #[test]
    fn test_retry_policy_evaluate() {
        let policy = RetryPolicy::new();
        let mut state = RetryState::with_model("claude-sonnet".to_string());

        // Iteration 0 - initial
        assert_eq!(policy.evaluate(&state), RetryDecision::NoRetry);

        // Iteration 1 - first retry, same agent
        state.iteration_count = 1;
        assert_eq!(policy.evaluate(&state), RetryDecision::RetrySameAgent);

        // Iteration 2 - second retry, same agent
        state.iteration_count = 2;
        assert_eq!(policy.evaluate(&state), RetryDecision::RetrySameAgent);

        // Iteration 3 - third retry, switch model
        state.iteration_count = 3;
        assert_eq!(
            policy.evaluate(&state),
            RetryDecision::RetryWithDifferentModel
        );

        // Iteration 4 - escalate
        state.iteration_count = 4;
        assert_eq!(policy.evaluate(&state), RetryDecision::EscalateToHuman);

        // Iteration 5 - escalate
        state.iteration_count = 5;
        assert_eq!(policy.evaluate(&state), RetryDecision::EscalateToHuman);
    }

    #[test]
    fn test_retry_policy_get_next_model() {
        let policy = RetryPolicy::new();

        // Same agent scenario
        let mut state = RetryState::with_model("claude-sonnet".to_string());
        state.iteration_count = 1;

        let model = policy.get_next_model(&state);
        assert_eq!(model, Some("claude-sonnet".to_string()));

        // Model switch scenario with alternative
        let mut state = RetryState::with_model("claude-sonnet".to_string());
        state.set_alternative_model("claude-3-opus".to_string());
        state.iteration_count = 3;

        let model = policy.get_next_model(&state);
        assert_eq!(model, Some("claude-3-opus".to_string()));

        // Model switch scenario without alternative
        let mut state = RetryState::with_model("claude-sonnet".to_string());
        state.iteration_count = 3;
        // No alternative set

        let model = policy.get_next_model(&state);
        assert_eq!(model, Some("claude-sonnet".to_string()));

        // Escalation scenario
        let mut state = RetryState::with_model("claude-sonnet".to_string());
        state.iteration_count = 4;

        let model = policy.get_next_model(&state);
        assert!(model.is_none());
    }

    #[test]
    fn test_retry_policy_should_escalate() {
        let policy = RetryPolicy::new();
        let mut state = RetryState::with_model("claude-sonnet".to_string());

        // Below threshold
        state.iteration_count = 2;
        assert!(!policy.should_escalate(&state));

        // At threshold
        state.iteration_count = 3;
        assert!(policy.should_escalate(&state));

        // Above threshold
        state.iteration_count = 10;
        assert!(policy.should_escalate(&state));
    }

    #[test]
    fn test_retry_policy_get_retry_number() {
        let policy = RetryPolicy::new();

        assert_eq!(policy.get_retry_number(0), 0); // Initial
        assert_eq!(policy.get_retry_number(1), 0); // First retry
        assert_eq!(policy.get_retry_number(2), 1); // Second retry
        assert_eq!(policy.get_retry_number(3), 2); // Third retry
        assert_eq!(policy.get_retry_number(4), 3); // Fourth retry (escalation)
    }

    #[test]
    fn test_retry_state_reset() {
        let mut state = RetryState::with_model("claude-sonnet".to_string());
        state.set_alternative_model("claude-3-opus".to_string());
        state.record_agent(Uuid::new_v4());
        state.iteration_count = 3;

        state.reset();

        assert_eq!(state.iteration_count, 0);
        assert_eq!(state.previous_model, Some("claude-sonnet".to_string()));
        assert!(state.previous_agent_id.is_none());
    }

    #[test]
    fn test_retry_decision_display() {
        assert_eq!(
            format!("{}", RetryDecision::RetrySameAgent),
            "RetrySameAgent"
        );
        assert_eq!(
            format!("{}", RetryDecision::RetryWithDifferentModel),
            "RetryWithDifferentModel"
        );
        assert_eq!(
            format!("{}", RetryDecision::EscalateToHuman),
            "EscalateToHuman"
        );
        assert_eq!(format!("{}", RetryDecision::NoRetry), "NoRetry");
    }

    #[test]
    fn test_retry_state_record_agent() {
        let mut state = RetryState::new();
        let agent_id = Uuid::new_v4();

        state.record_agent(agent_id);

        assert_eq!(state.previous_agent_id, Some(agent_id));
    }

    #[test]
    fn test_retry_state_get_next_agent_same() {
        let mut state = RetryState::with_model("claude-sonnet".to_string());

        // Iteration 1 - same agent
        state.iteration_count = 1;
        assert!(state.get_next_agent_same());

        // Iteration 2 - same agent
        state.iteration_count = 2;
        assert!(state.get_next_agent_same());

        // Iteration 3 - model switch, different agent
        state.iteration_count = 3;
        assert!(!state.get_next_agent_same());

        // Iteration 4 - escalate
        state.iteration_count = 4;
        assert!(!state.get_next_agent_same());
    }
}
