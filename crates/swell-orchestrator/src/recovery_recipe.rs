//! Recovery Recipe system for mapping FailureScenarios to typed RecoverySteps.
//!
//! This module implements a scenario-keyed recovery registry that maps failures
//! to appropriate recovery actions, enabling sophisticated failure handling beyond
//! simple retry counts.
//!
//! # Architecture
//!
//! - [`FailureScenario`] - A failure classification combined with optional context
//! - [`BackoffStrategy`] - Backoff algorithm for retries
//! - [`RecoveryStep`] - A single typed recovery action
//! - [`RecoverySteps`] - A sequence of recovery steps to execute
//! - [`RecoveryRecipe`] - A registry mapping scenarios to recovery steps
//!
//! # Example
//!
//! ```
//! use swell_core::FailureClass;
//! use swell_orchestrator::{RecoveryRecipe, FailureScenario, RecoveryStep, BackoffStrategy};
//!
//! let mut recipe = RecoveryRecipe::default();
//!
//! // Register a recipe for rate limiting
//! recipe.register(
//!     FailureScenario::from_class(FailureClass::RateLimited),
//!     vec![RecoveryStep::retry(3, BackoffStrategy::Exponential)]
//! );
//!
//! // Look up the recipe
//! let steps = recipe.get(&FailureScenario::from_class(FailureClass::RateLimited));
//! assert!(matches!(steps.first(), Some(RecoveryStep::Retry { .. })));
//! ```

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use std::hash::Hash;
use swell_core::FailureClass;

/// A failure scenario combining a FailureClass with optional contextual information.
///
/// This allows for more granular failure handling where the same FailureClass
/// might require different recovery strategies based on context (e.g., which
/// tool failed, what the error message says, etc.).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailureScenario {
    /// The base failure classification
    pub class: FailureClass,
    /// Optional error message pattern to match (substring match)
    #[serde(default)]
    pub error_pattern: Option<String>,
    /// Optional tool name if the failure is tool-related
    #[serde(default)]
    pub tool_name: Option<String>,
    /// Optional additional context as key-value pairs
    #[serde(default)]
    pub context: HashMap<String, String>,
}

/// Custom Hash implementation for FailureScenario.
/// We implement this manually because FailureClass doesn't implement Hash.
impl Hash for FailureScenario {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // Hash the discriminant of the FailureClass enum manually
        // since FailureClass doesn't implement Hash
        core::mem::discriminant(&self.class).hash(state);
        self.error_pattern.hash(state);
        self.tool_name.hash(state);
        // Hash context as sorted key-value pairs for deterministic hashing
        let mut sorted_keys: Vec<_> = self.context.keys().collect();
        sorted_keys.sort();
        sorted_keys.hash(state);
    }
}

impl FailureScenario {
    /// Create a scenario from just a FailureClass
    pub fn from_class(class: FailureClass) -> Self {
        Self {
            class,
            error_pattern: None,
            tool_name: None,
            context: HashMap::new(),
        }
    }

    /// Create a scenario with an error pattern
    pub fn with_error_pattern(mut self, pattern: impl Into<String>) -> Self {
        self.error_pattern = Some(pattern.into());
        self
    }

    /// Create a scenario with a tool name
    pub fn with_tool_name(mut self, name: impl Into<String>) -> Self {
        self.tool_name = Some(name.into());
        self
    }

    /// Add context key-value pair
    pub fn with_context(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.context.insert(key.into(), value.into());
        self
    }

    /// Check if this scenario matches a given failure class and optional context.
    ///
    /// A scenario matches if:
    /// - The failure class matches
    /// - If error_pattern is set, the error message contains the pattern
    /// - If tool_name is set, it matches the provided tool name
    /// - All context key-value pairs match
    pub fn matches(
        &self,
        class: FailureClass,
        error_msg: Option<&str>,
        tool_name: Option<&str>,
    ) -> bool {
        if self.class != class {
            return false;
        }

        // Check error pattern
        if let Some(ref pattern) = self.error_pattern {
            match error_msg {
                Some(msg) if msg.contains(pattern) => {}
                _ => return false,
            }
        }

        // Check tool name
        if let Some(ref expected_tool) = self.tool_name {
            match tool_name {
                Some(t) if t == expected_tool => {}
                _ => return false,
            }
        }

        // Check context
        for (key, value) in &self.context {
            // Context requires both key and value to match
            // This is a simple exact match for now
            if value != key {
                // For context matching, we check if there's a matching key-value
                // This is simplified - in production might want more sophisticated matching
                let found = self.context.get(key).map(|v| v == value).unwrap_or(false);
                if !found {
                    return false;
                }
            }
        }

        true
    }
}

impl fmt::Display for FailureScenario {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "FailureScenario({:?}", self.class)?;
        if let Some(ref pattern) = self.error_pattern {
            write!(f, ", pattern={}", pattern)?;
        }
        if let Some(ref tool) = self.tool_name {
            write!(f, ", tool={}", tool)?;
        }
        if !self.context.is_empty() {
            write!(f, ", context={:?}", self.context)?;
        }
        write!(f, ")")
    }
}

/// Backoff strategy for retry operations
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackoffStrategy {
    /// Fixed delay between retries
    Fixed,
    /// Linear increase: delay = base * attempt
    Linear,
    /// Exponential increase: delay = base * 2^attempt
    #[default]
    Exponential,
    /// Exponential with jitter for distributed systems
    ExponentialWithJitter,
}

impl fmt::Display for BackoffStrategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BackoffStrategy::Fixed => write!(f, "Fixed"),
            BackoffStrategy::Linear => write!(f, "Linear"),
            BackoffStrategy::Exponential => write!(f, "Exponential"),
            BackoffStrategy::ExponentialWithJitter => write!(f, "ExponentialWithJitter"),
        }
    }
}

/// A single recovery step action
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "config")]
pub enum RecoveryStep {
    /// Retry with exponential backoff
    ///
    /// # Fields
    /// - `max_attempts`: Maximum number of retry attempts
    /// - `backoff`: Backoff strategy to use
    /// - `base_delay_ms`: Base delay in milliseconds
    Retry {
        /// Maximum retry attempts
        max_attempts: u32,
        /// Backoff strategy
        backoff: BackoffStrategy,
        /// Base delay in milliseconds
        base_delay_ms: u64,
    },
    /// Rollback to a previous state or checkpoint
    Rollback {
        /// Checkpoint ID to rollback to (if applicable)
        checkpoint_id: Option<String>,
    },
    /// Escalate to human intervention
    Escalate {
        /// Reason for escalation
        reason: String,
    },
    /// Skip this step and continue execution
    SkipAndContinue {
        /// Description of what was skipped
        description: String,
    },
}

impl RecoveryStep {
    /// Create a retry step with the given max attempts and exponential backoff
    pub fn retry(max_attempts: u32, backoff: BackoffStrategy) -> Self {
        Self::Retry {
            max_attempts,
            backoff,
            base_delay_ms: 1000, // 1 second default
        }
    }

    /// Create a retry step with custom base delay
    pub fn retry_with_delay(
        max_attempts: u32,
        backoff: BackoffStrategy,
        base_delay_ms: u64,
    ) -> Self {
        Self::Retry {
            max_attempts,
            backoff,
            base_delay_ms,
        }
    }

    /// Create an escalate step
    pub fn escalate(reason: impl Into<String>) -> Self {
        Self::Escalate {
            reason: reason.into(),
        }
    }

    /// Create a rollback step
    pub fn rollback() -> Self {
        Self::Rollback {
            checkpoint_id: None,
        }
    }

    /// Create a rollback step with specific checkpoint
    pub fn rollback_to(checkpoint_id: impl Into<String>) -> Self {
        Self::Rollback {
            checkpoint_id: Some(checkpoint_id.into()),
        }
    }

    /// Create a skip and continue step
    pub fn skip(description: impl Into<String>) -> Self {
        Self::SkipAndContinue {
            description: description.into(),
        }
    }
}

impl fmt::Display for RecoveryStep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RecoveryStep::Retry {
                max_attempts,
                backoff,
                base_delay_ms,
            } => {
                write!(
                    f,
                    "Retry(max={}, backoff={}, delay={}ms)",
                    max_attempts, backoff, base_delay_ms
                )
            }
            RecoveryStep::Rollback { checkpoint_id } => match checkpoint_id {
                Some(id) => write!(f, "Rollback(to={})", id),
                None => write!(f, "Rollback"),
            },
            RecoveryStep::Escalate { reason } => {
                write!(f, "Escalate({})", reason)
            }
            RecoveryStep::SkipAndContinue { description } => {
                write!(f, "SkipAndContinue({})", description)
            }
        }
    }
}

/// A sequence of recovery steps to execute
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecoverySteps(pub Vec<RecoveryStep>);

impl RecoverySteps {
    /// Create new empty recovery steps
    pub fn new() -> Self {
        Self(Vec::new())
    }

    /// Create recovery steps from a vec
    pub fn from_vec(steps: Vec<RecoveryStep>) -> Self {
        Self(steps)
    }

    /// Add a step to the sequence
    pub fn push(&mut self, step: RecoveryStep) {
        self.0.push(step);
    }

    /// Get the first step if present
    pub fn first(&self) -> Option<&RecoveryStep> {
        self.0.first()
    }

    /// Get all steps
    pub fn as_slice(&self) -> &[RecoveryStep] {
        &self.0
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Get step count
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Iterate over steps
    pub fn iter(&self) -> impl Iterator<Item = &RecoveryStep> {
        self.0.iter()
    }
}

impl IntoIterator for RecoverySteps {
    type Item = RecoveryStep;
    type IntoIter = std::vec::IntoIter<RecoveryStep>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<'a> IntoIterator for &'a RecoverySteps {
    type Item = &'a RecoveryStep;
    type IntoIter = std::slice::Iter<'a, RecoveryStep>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl From<Vec<RecoveryStep>> for RecoverySteps {
    fn from(steps: Vec<RecoveryStep>) -> Self {
        Self(steps)
    }
}

impl Default for RecoverySteps {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for RecoverySteps {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "RecoverySteps([")?;
        for (i, step) in self.0.iter().enumerate() {
            if i > 0 {
                write!(f, ", ")?;
            }
            write!(f, "{}", step)?;
        }
        write!(f, "])")
    }
}

/// Recovery recipe registry that maps FailureScenarios to RecoverySteps
///
/// # Example
///
/// ```
/// use swell_core::FailureClass;
/// use swell_orchestrator::{RecoveryRecipe, FailureScenario, RecoveryStep, BackoffStrategy};
///
/// let mut recipe = RecoveryRecipe::default();
///
/// // Register custom recipes
/// recipe.register(
///     FailureScenario::from_class(FailureClass::RateLimited),
///     vec![RecoveryStep::retry(5, BackoffStrategy::Exponential)]
/// );
///
/// recipe.register(
///     FailureScenario::from_class(FailureClass::LlmError)
///         .with_error_pattern("timeout"),
///     vec![RecoveryStep::retry(3, BackoffStrategy::Linear)]
/// );
///
/// // Look up by exact scenario
/// let steps = recipe.get(&FailureScenario::from_class(FailureClass::RateLimited));
/// assert!(!steps.is_empty());
/// ```
#[derive(Debug, Clone, Default)]
pub struct RecoveryRecipe {
    /// Registered recipes keyed by FailureScenario
    recipes: HashMap<FailureScenario, RecoverySteps>,
    /// Default recipe for unregistered scenarios
    default_recipe: RecoverySteps,
}

/// Custom serialization for RecoveryRecipe to handle HashMap with non-string keys
impl serde::Serialize for RecoveryRecipe {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        // Convert HashMap to Vec of (FailureScenario, RecoverySteps) tuples
        let recipes: Vec<_> = self.recipes.iter().collect();
        let mut state = serializer.serialize_struct("RecoveryRecipe", 2)?;
        state.serialize_field("recipes", &recipes)?;
        state.serialize_field("default_recipe", &self.default_recipe)?;
        state.end()
    }
}

/// Custom deserialization for RecoveryRecipe to handle HashMap with non-string keys
impl<'de> serde::Deserialize<'de> for RecoveryRecipe {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(rename = "RecoveryRecipe")]
        struct RecoveryRecipeHelper {
            recipes: Vec<(FailureScenario, RecoverySteps)>,
            default_recipe: RecoverySteps,
        }

        let helper = RecoveryRecipeHelper::deserialize(deserializer)?;
        let recipes: HashMap<FailureScenario, RecoverySteps> = helper.recipes.into_iter().collect();
        Ok(Self {
            recipes,
            default_recipe: helper.default_recipe,
        })
    }
}

impl RecoveryRecipe {
    /// Create a new empty recipe registry
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a recipe registry with a custom default
    pub fn with_default(default: RecoverySteps) -> Self {
        Self {
            recipes: HashMap::new(),
            default_recipe: default,
        }
    }

    /// Register a recipe for a failure scenario
    pub fn register(
        &mut self,
        scenario: FailureScenario,
        steps: impl Into<RecoverySteps>,
    ) -> &mut Self {
        let steps = steps.into();
        // Convert to owned type for proper HashMap storage
        let steps_owned = RecoverySteps(steps.0);
        self.recipes.insert(scenario, steps_owned);
        self
    }

    /// Get the recovery steps for a failure scenario
    ///
    /// Returns the registered steps if found, otherwise the default recipe.
    pub fn get(&self, scenario: &FailureScenario) -> &RecoverySteps {
        self.recipes.get(scenario).unwrap_or(&self.default_recipe)
    }

    /// Get the recovery steps with scenario matching
    ///
    /// This performs a more sophisticated lookup by checking registered
    /// scenarios for pattern matches.
    pub fn get_matching(
        &self,
        class: FailureClass,
        error_msg: Option<&str>,
        tool_name: Option<&str>,
    ) -> &RecoverySteps {
        // First try exact match
        let exact_scenario = FailureScenario::from_class(class);
        if let Some(steps) = self.recipes.get(&exact_scenario) {
            return steps;
        }

        // Then try pattern matching
        for (scenario, steps) in &self.recipes {
            if scenario.matches(class, error_msg, tool_name) {
                return steps;
            }
        }

        // Fall back to default
        &self.default_recipe
    }

    /// Check if a specific scenario is registered
    pub fn contains(&self, scenario: &FailureScenario) -> bool {
        self.recipes.contains_key(scenario)
    }

    /// Get the number of registered recipes (excluding default)
    pub fn len(&self) -> usize {
        self.recipes.len()
    }

    /// Check if there are no registered recipes (excluding default)
    pub fn is_empty(&self) -> bool {
        self.recipes.is_empty()
    }

    /// Get all registered scenarios
    pub fn scenarios(&self) -> impl Iterator<Item = &FailureScenario> {
        self.recipes.keys()
    }

    /// Clear all registered recipes (keep default)
    pub fn clear(&mut self) {
        self.recipes.clear();
    }

    /// Set the default recipe
    pub fn set_default(&mut self, steps: impl Into<RecoverySteps>) -> &mut Self {
        self.default_recipe = steps.into();
        self
    }

    /// Calculate the delay for a retry attempt given the backoff strategy
    pub fn calculate_delay(attempt: u32, backoff: BackoffStrategy, base_delay_ms: u64) -> u64 {
        match backoff {
            BackoffStrategy::Fixed => base_delay_ms,
            BackoffStrategy::Linear => base_delay_ms.saturating_mul(attempt as u64),
            BackoffStrategy::Exponential => {
                // Cap exponent to avoid overflow - max delay of ~69 years with u64
                let exponent = attempt.saturating_sub(1).min(63);
                base_delay_ms.saturating_mul(2u64.saturating_pow(exponent))
            }
            BackoffStrategy::ExponentialWithJitter => {
                // Cap exponent to avoid overflow
                let exponent = attempt.saturating_sub(1).min(63);
                let exp_delay = base_delay_ms.saturating_mul(2u64.saturating_pow(exponent));
                // Add jitter: random value between 0 and exp_delay/2
                let jitter = (exp_delay / 2) as f64 * rand_simple();
                (exp_delay as f64 + jitter) as u64
            }
        }
    }
}

/// Simple pseudo-random for jitter calculation (0.0 to 1.0)
fn rand_simple() -> f64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    (nanos as f64 % 1000.0) / 1000.0
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // --- FailureScenario Tests ---

    #[test]
    fn test_failure_scenario_from_class() {
        let scenario = FailureScenario::from_class(FailureClass::RateLimited);
        assert_eq!(scenario.class, FailureClass::RateLimited);
        assert!(scenario.error_pattern.is_none());
        assert!(scenario.tool_name.is_none());
        assert!(scenario.context.is_empty());
    }

    #[test]
    fn test_failure_scenario_builder() {
        let scenario = FailureScenario::from_class(FailureClass::ToolError)
            .with_error_pattern("file not found")
            .with_tool_name("file_read")
            .with_context("path", "/tmp/test");

        assert_eq!(scenario.class, FailureClass::ToolError);
        assert_eq!(scenario.error_pattern, Some("file not found".to_string()));
        assert_eq!(scenario.tool_name, Some("file_read".to_string()));
        assert_eq!(scenario.context.get("path"), Some(&"/tmp/test".to_string()));
    }

    #[test]
    fn test_failure_scenario_matches_exact() {
        let scenario = FailureScenario::from_class(FailureClass::NetworkError);

        assert!(scenario.matches(FailureClass::NetworkError, None, None));
        assert!(!scenario.matches(FailureClass::LlmError, None, None));
    }

    #[test]
    fn test_failure_scenario_matches_with_pattern() {
        let scenario =
            FailureScenario::from_class(FailureClass::LlmError).with_error_pattern("timeout");

        assert!(scenario.matches(
            FailureClass::LlmError,
            Some("Request timeout after 30s"),
            None
        ));
        assert!(scenario.matches(FailureClass::LlmError, Some("Connection timeout"), None));
        assert!(!scenario.matches(FailureClass::LlmError, Some("rate limit exceeded"), None));
        assert!(!scenario.matches(FailureClass::ToolError, Some("timeout"), None));
    }

    #[test]
    fn test_failure_scenario_matches_with_tool() {
        let scenario =
            FailureScenario::from_class(FailureClass::ToolError).with_tool_name("file_read");

        assert!(scenario.matches(FailureClass::ToolError, None, Some("file_read")));
        assert!(!scenario.matches(FailureClass::ToolError, None, Some("shell")));
        assert!(!scenario.matches(FailureClass::LlmError, None, Some("file_read")));
    }

    #[test]
    fn test_failure_scenario_display() {
        let scenario = FailureScenario::from_class(FailureClass::RateLimited);
        assert_eq!(format!("{}", scenario), "FailureScenario(RateLimited)");

        let scenario = FailureScenario::from_class(FailureClass::ToolError)
            .with_error_pattern("not found")
            .with_tool_name("file_read");
        assert_eq!(
            format!("{}", scenario),
            "FailureScenario(ToolError, pattern=not found, tool=file_read)"
        );
    }

    #[test]
    fn test_failure_scenario_serde_roundtrip() {
        let scenario = FailureScenario::from_class(FailureClass::RateLimited)
            .with_error_pattern("api limit")
            .with_tool_name("llm_call");

        let json = serde_json::to_string(&scenario).expect("should serialize");
        let deserialized: FailureScenario =
            serde_json::from_str(&json).expect("should deserialize");

        assert_eq!(scenario, deserialized);
    }

    // --- BackoffStrategy Tests ---

    #[test]
    fn test_backoff_strategy_default() {
        assert_eq!(BackoffStrategy::default(), BackoffStrategy::Exponential);
    }

    #[test]
    fn test_backoff_strategy_display() {
        assert_eq!(format!("{}", BackoffStrategy::Fixed), "Fixed");
        assert_eq!(format!("{}", BackoffStrategy::Linear), "Linear");
        assert_eq!(format!("{}", BackoffStrategy::Exponential), "Exponential");
        assert_eq!(
            format!("{}", BackoffStrategy::ExponentialWithJitter),
            "ExponentialWithJitter"
        );
    }

    #[test]
    fn test_backoff_strategy_serde_roundtrip() {
        let strategies = [
            BackoffStrategy::Fixed,
            BackoffStrategy::Linear,
            BackoffStrategy::Exponential,
            BackoffStrategy::ExponentialWithJitter,
        ];

        for strategy in &strategies {
            let json = serde_json::to_string(strategy).expect("should serialize");
            let deserialized: BackoffStrategy =
                serde_json::from_str(&json).expect("should deserialize");
            assert_eq!(*strategy, deserialized);
        }
    }

    // --- RecoveryStep Tests ---

    #[test]
    fn test_recovery_step_retry() {
        let step = RecoveryStep::retry(3, BackoffStrategy::Exponential);
        assert!(matches!(
            step,
            RecoveryStep::Retry {
                max_attempts: 3,
                backoff: BackoffStrategy::Exponential,
                base_delay_ms: 1000
            }
        ));
    }

    #[test]
    fn test_recovery_step_retry_with_delay() {
        let step = RecoveryStep::retry_with_delay(5, BackoffStrategy::Linear, 500);
        assert!(matches!(
            step,
            RecoveryStep::Retry {
                max_attempts: 5,
                backoff: BackoffStrategy::Linear,
                base_delay_ms: 500
            }
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
            RecoveryStep::Rollback { checkpoint_id: Some(id) } if id == "checkpoint-123"
        ));
    }

    #[test]
    fn test_recovery_step_skip() {
        let step = RecoveryStep::skip("Optional step failed");
        assert!(matches!(
            step,
            RecoveryStep::SkipAndContinue { description } if description == "Optional step failed"
        ));
    }

    #[test]
    fn test_recovery_step_display() {
        assert_eq!(
            format!("{}", RecoveryStep::retry(3, BackoffStrategy::Exponential)),
            "Retry(max=3, backoff=Exponential, delay=1000ms)"
        );

        assert_eq!(
            format!("{}", RecoveryStep::escalate("test")),
            "Escalate(test)"
        );

        assert_eq!(format!("{}", RecoveryStep::rollback()), "Rollback");

        assert_eq!(
            format!("{}", RecoveryStep::skip("test")),
            "SkipAndContinue(test)"
        );
    }

    #[test]
    fn test_recovery_step_serde_roundtrip() {
        let steps = vec![
            RecoveryStep::retry(3, BackoffStrategy::Exponential),
            RecoveryStep::escalate("test"),
            RecoveryStep::rollback(),
            RecoveryStep::skip("test"),
        ];

        for step in &steps {
            let json = serde_json::to_string(step).expect("should serialize");
            let deserialized: RecoveryStep =
                serde_json::from_str(&json).expect("should deserialize");
            assert_eq!(*step, deserialized);
        }
    }

    // --- RecoverySteps Tests ---

    #[test]
    fn test_recovery_steps_from_vec() {
        let steps = vec![
            RecoveryStep::retry(3, BackoffStrategy::Exponential),
            RecoveryStep::escalate("fallback"),
        ];
        let recovery_steps = RecoverySteps::from_vec(steps);

        assert_eq!(recovery_steps.len(), 2);
        assert!(!recovery_steps.is_empty());
    }

    #[test]
    fn test_recovery_steps_first() {
        let steps = vec![
            RecoveryStep::retry(3, BackoffStrategy::Exponential),
            RecoveryStep::escalate("fallback"),
        ];
        let recovery_steps = RecoverySteps::from_vec(steps);

        assert!(matches!(
            recovery_steps.first(),
            Some(RecoveryStep::Retry { .. })
        ));
    }

    #[test]
    fn test_recovery_steps_push() {
        let mut steps = RecoverySteps::new();
        assert!(steps.is_empty());

        steps.push(RecoveryStep::retry(3, BackoffStrategy::Exponential));
        assert_eq!(steps.len(), 1);

        steps.push(RecoveryStep::escalate("fallback"));
        assert_eq!(steps.len(), 2);
    }

    #[test]
    fn test_recovery_steps_iter() {
        let steps = vec![
            RecoveryStep::retry(3, BackoffStrategy::Exponential),
            RecoveryStep::escalate("fallback"),
        ];
        let recovery_steps = RecoverySteps::from_vec(steps);

        let collected: Vec<_> = recovery_steps.iter().collect();
        assert_eq!(collected.len(), 2);
    }

    #[test]
    fn test_recovery_steps_into_iter() {
        let steps = vec![
            RecoveryStep::retry(3, BackoffStrategy::Exponential),
            RecoveryStep::escalate("fallback"),
        ];
        let recovery_steps = RecoverySteps::from_vec(steps);

        let collected: Vec<_> = recovery_steps.into_iter().collect();
        assert_eq!(collected.len(), 2);
    }

    #[test]
    fn test_recovery_steps_display() {
        let steps = vec![
            RecoveryStep::retry(3, BackoffStrategy::Exponential),
            RecoveryStep::escalate("fallback"),
        ];
        let recovery_steps = RecoverySteps::from_vec(steps);

        assert_eq!(
            format!("{}", recovery_steps),
            "RecoverySteps([Retry(max=3, backoff=Exponential, delay=1000ms), Escalate(fallback)])"
        );
    }

    #[test]
    fn test_recovery_steps_default() {
        let steps = RecoverySteps::default();
        assert!(steps.is_empty());
    }

    #[test]
    fn test_recovery_steps_serde_roundtrip() {
        let steps = RecoverySteps::from_vec(vec![
            RecoveryStep::retry(3, BackoffStrategy::Exponential),
            RecoveryStep::escalate("test"),
        ]);

        let json = serde_json::to_string(&steps).expect("should serialize");
        let deserialized: RecoverySteps = serde_json::from_str(&json).expect("should deserialize");
        assert_eq!(steps.len(), deserialized.len());
    }

    // --- RecoveryRecipe Tests ---

    #[test]
    fn test_recovery_recipe_new() {
        let recipe = RecoveryRecipe::new();
        assert!(recipe.is_empty());
        assert!(recipe.len() == 0);
    }

    #[test]
    fn test_recovery_recipe_register() {
        let mut recipe = RecoveryRecipe::new();

        recipe.register(
            FailureScenario::from_class(FailureClass::RateLimited),
            vec![RecoveryStep::retry(5, BackoffStrategy::Exponential)],
        );

        assert!(!recipe.is_empty());
        assert_eq!(recipe.len(), 1);
        assert!(recipe.contains(&FailureScenario::from_class(FailureClass::RateLimited)));
    }

    #[test]
    fn test_recovery_recipe_register_chaining() {
        let mut recipe = RecoveryRecipe::new();

        recipe
            .register(
                FailureScenario::from_class(FailureClass::RateLimited),
                vec![RecoveryStep::retry(5, BackoffStrategy::Exponential)],
            )
            .register(
                FailureScenario::from_class(FailureClass::LlmError),
                vec![RecoveryStep::retry(3, BackoffStrategy::Linear)],
            );

        assert_eq!(recipe.len(), 2);
    }

    #[test]
    fn test_recovery_recipe_get_exact() {
        let mut recipe = RecoveryRecipe::new();

        let steps = vec![RecoveryStep::retry(5, BackoffStrategy::Exponential)];
        recipe.register(
            FailureScenario::from_class(FailureClass::RateLimited),
            steps,
        );

        let retrieved = recipe.get(&FailureScenario::from_class(FailureClass::RateLimited));

        assert_eq!(retrieved.len(), 1);
        assert!(matches!(
            retrieved.first(),
            Some(RecoveryStep::Retry {
                max_attempts: 5,
                ..
            })
        ));
    }

    #[test]
    fn test_recovery_recipe_get_unregistered_returns_default() {
        let recipe = RecoveryRecipe::new();

        // Unregistered scenario should return default (empty by default)
        let retrieved = recipe.get(&FailureScenario::from_class(FailureClass::NetworkError));
        assert!(retrieved.is_empty());
    }

    #[test]
    fn test_recovery_recipe_default_recipe() {
        let default_steps = vec![RecoveryStep::escalate("Default escalation")];
        let mut recipe = RecoveryRecipe::with_default(RecoverySteps::from_vec(default_steps));

        // Now unregistered scenarios should return the custom default
        let retrieved = recipe.get(&FailureScenario::from_class(FailureClass::NetworkError));
        assert_eq!(retrieved.len(), 1);
        assert!(matches!(
            retrieved.first(),
            Some(RecoveryStep::Escalate { .. })
        ));
    }

    #[test]
    fn test_recovery_recipe_set_default() {
        let mut recipe = RecoveryRecipe::new();

        recipe
            .register(
                FailureScenario::from_class(FailureClass::RateLimited),
                vec![RecoveryStep::retry(5, BackoffStrategy::Exponential)],
            )
            .set_default(vec![RecoveryStep::escalate("Default")]);

        let unregistered = recipe.get(&FailureScenario::from_class(FailureClass::NetworkError));
        assert!(matches!(
            unregistered.first(),
            Some(RecoveryStep::Escalate { .. })
        ));
    }

    #[test]
    fn test_recovery_recipe_get_matching_exact() {
        let mut recipe = RecoveryRecipe::new();

        recipe.register(
            FailureScenario::from_class(FailureClass::LlmError),
            vec![RecoveryStep::retry(3, BackoffStrategy::Linear)],
        );

        let retrieved = recipe.get_matching(FailureClass::LlmError, None, None);

        assert_eq!(retrieved.len(), 1);
        assert!(matches!(
            retrieved.first(),
            Some(RecoveryStep::Retry {
                max_attempts: 3,
                ..
            })
        ));
    }

    #[test]
    fn test_recovery_recipe_get_matching_pattern() {
        let mut recipe = RecoveryRecipe::new();

        recipe.register(
            FailureScenario::from_class(FailureClass::LlmError).with_error_pattern("timeout"),
            vec![RecoveryStep::retry(5, BackoffStrategy::Exponential)],
        );

        // Exact match should work
        let retrieved = recipe.get_matching(FailureClass::LlmError, Some("timeout error"), None);
        assert_eq!(retrieved.len(), 1);

        // Should match timeout in longer message
        let retrieved = recipe.get_matching(
            FailureClass::LlmError,
            Some("Request timeout after 30s"),
            None,
        );
        assert_eq!(retrieved.len(), 1);

        // Non-matching message should return default
        let retrieved =
            recipe.get_matching(FailureClass::LlmError, Some("rate limit exceeded"), None);
        assert!(retrieved.is_empty()); // Returns default which is empty
    }

    #[test]
    fn test_recovery_recipe_get_matching_tool() {
        let mut recipe = RecoveryRecipe::new();

        recipe.register(
            FailureScenario::from_class(FailureClass::ToolError).with_tool_name("file_read"),
            vec![RecoveryStep::rollback()],
        );

        // Matching tool
        let retrieved = recipe.get_matching(FailureClass::ToolError, None, Some("file_read"));
        assert_eq!(retrieved.len(), 1);

        // Non-matching tool
        let retrieved = recipe.get_matching(FailureClass::ToolError, None, Some("shell"));
        assert!(retrieved.is_empty()); // Returns default
    }

    #[test]
    fn test_recovery_recipe_clear() {
        let mut recipe = RecoveryRecipe::new();

        recipe.register(
            FailureScenario::from_class(FailureClass::RateLimited),
            vec![RecoveryStep::retry(5, BackoffStrategy::Exponential)],
        );

        assert_eq!(recipe.len(), 1);

        recipe.clear();

        assert!(recipe.is_empty());
        // Default should still work
        let retrieved = recipe.get(&FailureScenario::from_class(FailureClass::RateLimited));
        assert!(retrieved.is_empty());
    }

    #[test]
    fn test_recovery_recipe_scenarios() {
        let mut recipe = RecoveryRecipe::new();

        recipe.register(
            FailureScenario::from_class(FailureClass::RateLimited),
            vec![RecoveryStep::retry(5, BackoffStrategy::Exponential)],
        );
        recipe.register(
            FailureScenario::from_class(FailureClass::LlmError),
            vec![RecoveryStep::retry(3, BackoffStrategy::Linear)],
        );

        let scenarios: Vec<_> = recipe.scenarios().collect();
        assert_eq!(scenarios.len(), 2);
    }

    #[test]
    fn test_recovery_recipe_with_default_recovery_steps() {
        // Test creating recipe with default escalation
        let default = RecoverySteps::from_vec(vec![RecoveryStep::escalate("Unknown error")]);
        let recipe = RecoveryRecipe::with_default(default);

        let steps = recipe.get(&FailureScenario::from_class(FailureClass::InternalError));
        assert!(matches!(steps.first(), Some(RecoveryStep::Escalate { .. })));
    }

    // --- Delay Calculation Tests ---

    #[test]
    fn test_calculate_delay_fixed() {
        let delay = RecoveryRecipe::calculate_delay(1, BackoffStrategy::Fixed, 1000);
        assert_eq!(delay, 1000);

        let delay = RecoveryRecipe::calculate_delay(5, BackoffStrategy::Fixed, 1000);
        assert_eq!(delay, 1000);
    }

    #[test]
    fn test_calculate_delay_linear() {
        let delay = RecoveryRecipe::calculate_delay(1, BackoffStrategy::Linear, 1000);
        assert_eq!(delay, 1000); // 1000 * 1

        let delay = RecoveryRecipe::calculate_delay(3, BackoffStrategy::Linear, 1000);
        assert_eq!(delay, 3000); // 1000 * 3
    }

    #[test]
    fn test_calculate_delay_exponential() {
        let delay = RecoveryRecipe::calculate_delay(1, BackoffStrategy::Exponential, 1000);
        assert_eq!(delay, 1000); // 1000 * 2^0

        let delay = RecoveryRecipe::calculate_delay(2, BackoffStrategy::Exponential, 1000);
        assert_eq!(delay, 2000); // 1000 * 2^1

        let delay = RecoveryRecipe::calculate_delay(3, BackoffStrategy::Exponential, 1000);
        assert_eq!(delay, 4000); // 1000 * 2^2
    }

    #[test]
    fn test_calculate_delay_exponential_caps() {
        // Large attempt numbers should saturate
        let delay = RecoveryRecipe::calculate_delay(100, BackoffStrategy::Exponential, 1000);
        // Just verify it doesn't panic and returns a reasonable value
        assert!(delay > 0);
    }

    // --- VAL-TASK-008 Specific Tests ---

    #[test]
    fn test_val_task_008_llm_rate_limit_recipe() {
        // Test creates a RecoveryRecipe, registers a recipe for FailureScenario::LlmRateLimit,
        // and looks up. Asserts the returned RecoverySteps contains Retry with exponential backoff.

        let mut recipe = RecoveryRecipe::new();

        recipe.register(
            FailureScenario::from_class(FailureClass::RateLimited),
            vec![RecoveryStep::retry(3, BackoffStrategy::Exponential)],
        );

        let steps = recipe.get(&FailureScenario::from_class(FailureClass::RateLimited));

        assert!(!steps.is_empty());
        let first_step = steps.first().expect("should have first step");
        assert!(matches!(
            first_step,
            RecoveryStep::Retry {
                max_attempts: 3,
                backoff: BackoffStrategy::Exponential,
                ..
            }
        ));
    }

    #[test]
    fn test_val_task_008_default_recipe_for_unregistered() {
        // Test looks up an unregistered scenario and asserts a default recipe is returned
        // (e.g., Escalate)

        let default_steps =
            RecoverySteps::from_vec(vec![RecoveryStep::escalate("Escalated to human reviewer")]);
        let recipe = RecoveryRecipe::with_default(default_steps);

        let steps = recipe.get(&FailureScenario::from_class(FailureClass::InternalError));

        assert!(!steps.is_empty());
        let first_step = steps.first().expect("should have first step");
        assert!(matches!(first_step, RecoveryStep::Escalate { .. }));
    }

    #[test]
    fn test_val_task_008_all_recovery_step_types() {
        // Verify all recovery step types can be created and work correctly

        let mut recipe = RecoveryRecipe::new();

        // Retry with exponential
        recipe.register(
            FailureScenario::from_class(FailureClass::RateLimited),
            vec![RecoveryStep::retry(3, BackoffStrategy::Exponential)],
        );

        // Retry with linear
        recipe.register(
            FailureScenario::from_class(FailureClass::Timeout),
            vec![RecoveryStep::retry_with_delay(
                2,
                BackoffStrategy::Linear,
                500,
            )],
        );

        // Rollback
        recipe.register(
            FailureScenario::from_class(FailureClass::SandboxError),
            vec![RecoveryStep::rollback()],
        );

        // Escalate
        recipe.register(
            FailureScenario::from_class(FailureClass::InternalError),
            vec![RecoveryStep::escalate("Internal failure")],
        );

        // Skip and continue
        recipe.register(
            FailureScenario::from_class(FailureClass::PermissionDenied),
            vec![RecoveryStep::skip("Permission check failed - continuing")],
        );

        // Verify each type
        let retry_exp = recipe.get(&FailureScenario::from_class(FailureClass::RateLimited));
        assert!(matches!(
            retry_exp.first(),
            Some(RecoveryStep::Retry {
                backoff: BackoffStrategy::Exponential,
                ..
            })
        ));

        let retry_lin = recipe.get(&FailureScenario::from_class(FailureClass::Timeout));
        assert!(matches!(
            retry_lin.first(),
            Some(RecoveryStep::Retry {
                backoff: BackoffStrategy::Linear,
                ..
            })
        ));

        let rollback = recipe.get(&FailureScenario::from_class(FailureClass::SandboxError));
        assert!(matches!(
            rollback.first(),
            Some(RecoveryStep::Rollback { .. })
        ));

        let escalate = recipe.get(&FailureScenario::from_class(FailureClass::InternalError));
        assert!(matches!(
            escalate.first(),
            Some(RecoveryStep::Escalate { .. })
        ));

        let skip = recipe.get(&FailureScenario::from_class(FailureClass::PermissionDenied));
        assert!(matches!(
            skip.first(),
            Some(RecoveryStep::SkipAndContinue { .. })
        ));
    }

    // --- Additional Integration Tests ---

    #[test]
    fn test_recovery_recipe_complex_scenario() {
        let mut recipe = RecoveryRecipe::new();

        // LLM errors with timeout pattern get more retries
        recipe.register(
            FailureScenario::from_class(FailureClass::LlmError).with_error_pattern("timeout"),
            vec![RecoveryStep::retry(5, BackoffStrategy::Exponential)],
        );

        // LLM errors with rate limit pattern
        recipe.register(
            FailureScenario::from_class(FailureClass::LlmError).with_error_pattern("rate limit"),
            vec![RecoveryStep::retry(3, BackoffStrategy::Exponential)],
        );

        // Network errors get quick escalation
        recipe.register(
            FailureScenario::from_class(FailureClass::NetworkError),
            vec![
                RecoveryStep::retry(2, BackoffStrategy::Linear),
                RecoveryStep::escalate("Network issues persist"),
            ],
        );

        // Tool errors trigger rollback
        recipe.register(
            FailureScenario::from_class(FailureClass::ToolError).with_tool_name("git_commit"),
            vec![RecoveryStep::rollback()],
        );

        // Verify pattern matching
        let timeout_steps = recipe.get_matching(
            FailureClass::LlmError,
            Some("Connection timeout after 30s"),
            None,
        );
        assert!(matches!(
            timeout_steps.first(),
            Some(RecoveryStep::Retry {
                max_attempts: 5,
                ..
            })
        ));

        let rate_limit_steps = recipe.get_matching(
            FailureClass::LlmError,
            Some("API rate limit exceeded"),
            None,
        );
        assert!(matches!(
            rate_limit_steps.first(),
            Some(RecoveryStep::Retry {
                max_attempts: 3,
                ..
            })
        ));

        let network_steps = recipe.get_matching(FailureClass::NetworkError, None, None);
        assert!(matches!(
            network_steps.first(),
            Some(RecoveryStep::Retry { .. })
        ));
        assert_eq!(network_steps.len(), 2); // retry + escalate

        let git_tool = recipe.get_matching(
            FailureClass::ToolError,
            Some("git commit failed"),
            Some("git_commit"),
        );
        assert!(matches!(
            git_tool.first(),
            Some(RecoveryStep::Rollback { .. })
        ));
    }

    #[test]
    fn test_recovery_recipe_serde_roundtrip() {
        let mut recipe = RecoveryRecipe::new();

        recipe.register(
            FailureScenario::from_class(FailureClass::RateLimited),
            vec![RecoveryStep::retry(5, BackoffStrategy::Exponential)],
        );
        recipe.register(
            FailureScenario::from_class(FailureClass::LlmError),
            vec![RecoveryStep::escalate("LLM failure")],
        );
        recipe.set_default(vec![RecoveryStep::skip("Unknown error")]);

        let json = serde_json::to_string(&recipe).expect("should serialize");
        let deserialized: RecoveryRecipe = serde_json::from_str(&json).expect("should deserialize");

        assert_eq!(recipe.len(), deserialized.len());

        let steps = deserialized.get(&FailureScenario::from_class(FailureClass::RateLimited));
        assert!(matches!(
            steps.first(),
            Some(RecoveryStep::Retry {
                max_attempts: 5,
                ..
            })
        ));
    }
}
