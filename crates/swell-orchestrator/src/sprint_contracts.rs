//! Sprint Contract system for establishing success criteria between Generator and Evaluator agents.
//!
//! This module implements the `ContractNegotiator` that creates a `SprintContract` before a sprint
//! begins, ensuring both agents agree on what "done" looks like.
//!
//! # Architecture
//!
//! - [`GeneratorContext`] - Context provided by the Generator agent (plan, tools, task)
//! - [`EvaluatorContext`] - Context provided by the Evaluator agent (requirements, thresholds)
//! - [`SprintContract`] - Agreed-upon success criteria, validation gates, and acceptance threshold
//! - [`ContractNegotiator`] - Creates a SprintContract from Generator and Evaluator contexts
//! - [`ContractStatus`] - Whether the contract terms are satisfied
//!
//! # Example
//!
//! ```
//! use swell_orchestrator::{ContractNegotiator, GeneratorContext, EvaluatorContext, ValidationGate};
//!
//! let generator_ctx = GeneratorContext {
//!     task_description: "Implement feature X".to_string(),
//!     plan_steps: vec!["Write tests".to_string(), "Implement code".to_string()],
//!     tools_available: vec!["file_read".to_string(), "shell".to_string()],
//! };
//!
//! let evaluator_ctx = EvaluatorContext {
//!     validation_requirements: vec!["All tests pass".to_string(), "No clippy warnings".to_string()],
//!     required_gates: vec![ValidationGate::Lint, ValidationGate::Test],
//!     acceptance_threshold: 0.9,
//! };
//!
//! let negotiator = ContractNegotiator::new();
//! let contract = negotiator.negotiate(&generator_ctx, &evaluator_ctx);
//!
//! assert!(!contract.success_criteria.is_empty());
//! assert!(!contract.validation_gates.is_empty());
//! assert!(contract.acceptance_threshold > 0.0);
//! ```

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;

// ─── Validation Gate ──────────────────────────────────────────────────────────

/// A validation gate that must be passed during sprint evaluation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ValidationGate {
    /// Clippy lint check — no warnings allowed
    Lint,
    /// Cargo test suite — all tests must pass
    Test,
    /// Security scan gate
    Security,
    /// AI-based code review gate
    AiReview,
    /// Custom named gate (e.g., integration tests, benchmarks)
    Custom(String),
}

impl fmt::Display for ValidationGate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ValidationGate::Lint => write!(f, "lint"),
            ValidationGate::Test => write!(f, "test"),
            ValidationGate::Security => write!(f, "security"),
            ValidationGate::AiReview => write!(f, "ai_review"),
            ValidationGate::Custom(name) => write!(f, "custom:{name}"),
        }
    }
}

// ─── Context Types ─────────────────────────────────────────────────────────────

/// Context provided by the Generator agent during contract negotiation.
///
/// Describes what the generator plans to do and what resources it has access to.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneratorContext {
    /// High-level description of the task to be implemented
    pub task_description: String,
    /// Ordered list of planned steps the generator will execute
    pub plan_steps: Vec<String>,
    /// Names of tools available to the generator for implementation
    pub tools_available: Vec<String>,
}

impl GeneratorContext {
    /// Create a new GeneratorContext.
    pub fn new(task_description: impl Into<String>) -> Self {
        Self {
            task_description: task_description.into(),
            plan_steps: Vec::new(),
            tools_available: Vec::new(),
        }
    }

    /// Add a plan step.
    pub fn with_step(mut self, step: impl Into<String>) -> Self {
        self.plan_steps.push(step.into());
        self
    }

    /// Add an available tool.
    pub fn with_tool(mut self, tool: impl Into<String>) -> Self {
        self.tools_available.push(tool.into());
        self
    }
}

/// Context provided by the Evaluator agent during contract negotiation.
///
/// Describes the quality requirements and validation strategy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvaluatorContext {
    /// Human-readable validation requirements (e.g., "All tests must pass")
    pub validation_requirements: Vec<String>,
    /// Ordered list of gates that must pass for sprint acceptance
    pub required_gates: Vec<ValidationGate>,
    /// Minimum fraction of gates that must pass (0.0–1.0) for acceptance.
    /// For example, 1.0 means all gates must pass; 0.8 means 80% must pass.
    pub acceptance_threshold: f64,
}

impl EvaluatorContext {
    /// Create a new EvaluatorContext with an acceptance threshold.
    ///
    /// # Panics
    ///
    /// Panics in debug builds if `acceptance_threshold` is not in [0.0, 1.0].
    pub fn new(acceptance_threshold: f64) -> Self {
        debug_assert!(
            (0.0..=1.0).contains(&acceptance_threshold),
            "acceptance_threshold must be in [0.0, 1.0], got {acceptance_threshold}"
        );
        Self {
            validation_requirements: Vec::new(),
            required_gates: Vec::new(),
            acceptance_threshold,
        }
    }

    /// Add a validation requirement.
    pub fn with_requirement(mut self, requirement: impl Into<String>) -> Self {
        self.validation_requirements.push(requirement.into());
        self
    }

    /// Add a required validation gate.
    pub fn with_gate(mut self, gate: ValidationGate) -> Self {
        self.required_gates.push(gate);
        self
    }
}

// ─── Sprint Contract ───────────────────────────────────────────────────────────

/// Contract status — whether the contract's acceptance criteria are met.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContractStatus {
    /// All required gates passed the threshold — sprint accepted.
    Accepted,
    /// Insufficient gates passed — sprint rejected.
    Rejected {
        /// Names of gates that failed.
        failed_gates: Vec<String>,
    },
    /// Contract has not been evaluated yet.
    Pending,
}

/// An established agreement between the Generator and Evaluator agents for a sprint.
///
/// The contract specifies exactly what success looks like so both agents have a
/// shared understanding before execution begins. During and after execution, the
/// contract can be used to evaluate whether the sprint succeeded.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SprintContract {
    /// Ordered success criteria derived from both generator and evaluator contexts.
    ///
    /// Each criterion is a human-readable string describing a single requirement.
    /// The list is non-empty: at minimum it includes "task description" plus all
    /// validation requirements contributed by the evaluator.
    pub success_criteria: Vec<String>,

    /// Validation gates that must run during sprint evaluation.
    ///
    /// The list contains at minimum all [`ValidationGate`]s from the evaluator context.
    /// If the evaluator context provides no gates, a default set of [`ValidationGate::Lint`]
    /// and [`ValidationGate::Test`] is used.
    pub validation_gates: Vec<ValidationGate>,

    /// Minimum fraction of gates that must pass for sprint acceptance (0.0–1.0).
    ///
    /// Derived from [`EvaluatorContext::acceptance_threshold`]. A value of 1.0 requires
    /// every gate to pass; 0.8 allows up to 20% of gates to fail.
    pub acceptance_threshold: f64,

    /// Additional metadata captured during negotiation for observability.
    #[serde(default)]
    pub metadata: HashMap<String, String>,
}

impl SprintContract {
    /// Evaluate whether the given gate results satisfy this contract.
    ///
    /// `gate_results` maps each gate's display string to `true` (passed) or `false` (failed).
    /// Returns [`ContractStatus::Accepted`] if the fraction of passing gates is ≥ `acceptance_threshold`,
    /// otherwise [`ContractStatus::Rejected`] with the list of failing gate names.
    pub fn evaluate(&self, gate_results: &HashMap<String, bool>) -> ContractStatus {
        if self.validation_gates.is_empty() {
            return ContractStatus::Accepted;
        }

        let mut failed = Vec::new();
        let mut passed = 0usize;

        for gate in &self.validation_gates {
            let key = gate.to_string();
            let gate_passed = gate_results.get(&key).copied().unwrap_or(false);
            if gate_passed {
                passed += 1;
            } else {
                failed.push(key);
            }
        }

        let pass_fraction = passed as f64 / self.validation_gates.len() as f64;
        if pass_fraction >= self.acceptance_threshold {
            ContractStatus::Accepted
        } else {
            ContractStatus::Rejected {
                failed_gates: failed,
            }
        }
    }
}

// ─── Contract Negotiator ───────────────────────────────────────────────────────

/// Establishes a [`SprintContract`] between Generator and Evaluator agents.
///
/// The negotiator merges the generator's task knowledge with the evaluator's quality
/// requirements to produce a mutually agreed contract before sprint execution begins.
#[derive(Debug, Default, Clone)]
pub struct ContractNegotiator;

impl ContractNegotiator {
    /// Create a new `ContractNegotiator`.
    pub fn new() -> Self {
        Self
    }

    /// Negotiate a [`SprintContract`] between the Generator and Evaluator agents.
    ///
    /// # Contract Formation Rules
    ///
    /// **success_criteria:**
    /// 1. The task description from `generator` is always included as the first criterion.
    /// 2. Each plan step is appended as "Step N: <step>".
    /// 3. All `validation_requirements` from `evaluator` are appended verbatim.
    ///
    /// **validation_gates:**
    /// - If `evaluator.required_gates` is non-empty, those gates are used directly.
    /// - If empty, a default set `[Lint, Test]` is used so there is always ≥1 gate.
    ///
    /// **acceptance_threshold:**
    /// - Taken directly from `evaluator.acceptance_threshold`.
    ///
    /// # Example
    ///
    /// ```
    /// use swell_orchestrator::{ContractNegotiator, GeneratorContext, EvaluatorContext, ValidationGate};
    ///
    /// let gen = GeneratorContext::new("Implement caching layer")
    ///     .with_step("Add cache struct")
    ///     .with_step("Wire cache into handler");
    ///
    /// let eval = EvaluatorContext::new(1.0)
    ///     .with_requirement("Cache must be thread-safe")
    ///     .with_gate(ValidationGate::Test)
    ///     .with_gate(ValidationGate::Lint);
    ///
    /// let contract = ContractNegotiator::new().negotiate(&gen, &eval);
    ///
    /// assert!(!contract.success_criteria.is_empty());
    /// assert_eq!(contract.validation_gates.len(), 2);
    /// assert!((contract.acceptance_threshold - 1.0).abs() < f64::EPSILON);
    /// ```
    pub fn negotiate(
        &self,
        generator: &GeneratorContext,
        evaluator: &EvaluatorContext,
    ) -> SprintContract {
        // ── Build success_criteria ──────────────────────────────────────────
        let mut success_criteria = Vec::new();

        // Primary criterion: task description
        success_criteria.push(format!("Task: {}", generator.task_description));

        // Plan steps from generator
        for (i, step) in generator.plan_steps.iter().enumerate() {
            success_criteria.push(format!("Step {}: {}", i + 1, step));
        }

        // Validation requirements from evaluator
        for req in &evaluator.validation_requirements {
            success_criteria.push(req.clone());
        }

        // ── Build validation_gates ──────────────────────────────────────────
        let validation_gates = if evaluator.required_gates.is_empty() {
            // Default: always have at least Lint + Test
            vec![ValidationGate::Lint, ValidationGate::Test]
        } else {
            evaluator.required_gates.clone()
        };

        // ── Clamp acceptance_threshold to [0.0, 1.0] defensively ───────────
        let acceptance_threshold = evaluator.acceptance_threshold.clamp(0.0, 1.0);

        // ── Build metadata for observability ───────────────────────────────
        let mut metadata = HashMap::new();
        metadata.insert(
            "generator_tools".to_string(),
            generator.tools_available.join(", "),
        );
        metadata.insert("gate_count".to_string(), validation_gates.len().to_string());

        SprintContract {
            success_criteria,
            validation_gates,
            acceptance_threshold,
            metadata,
        }
    }
}

// ─── Unit Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn default_generator_ctx() -> GeneratorContext {
        GeneratorContext::new("Implement feature X")
            .with_step("Write failing tests")
            .with_step("Implement the feature")
            .with_tool("file_read")
            .with_tool("shell")
    }

    fn default_evaluator_ctx() -> EvaluatorContext {
        EvaluatorContext::new(1.0)
            .with_requirement("All tests must pass")
            .with_requirement("No clippy warnings")
            .with_gate(ValidationGate::Lint)
            .with_gate(ValidationGate::Test)
    }

    // ── ContractNegotiator::negotiate ────────────────────────────────────────

    #[test]
    fn test_negotiate_returns_non_empty_success_criteria() {
        let gen = default_generator_ctx();
        let eval = default_evaluator_ctx();
        let contract = ContractNegotiator::new().negotiate(&gen, &eval);
        assert!(
            !contract.success_criteria.is_empty(),
            "success_criteria must be non-empty"
        );
    }

    #[test]
    fn test_negotiate_includes_task_description_in_criteria() {
        let gen = GeneratorContext::new("Implement caching layer");
        let eval = EvaluatorContext::new(0.8);
        let contract = ContractNegotiator::new().negotiate(&gen, &eval);
        assert!(
            contract.success_criteria[0].contains("Implement caching layer"),
            "First criterion must include the task description"
        );
    }

    #[test]
    fn test_negotiate_includes_plan_steps_in_criteria() {
        let gen = GeneratorContext::new("task")
            .with_step("Step A")
            .with_step("Step B");
        let eval = EvaluatorContext::new(1.0);
        let contract = ContractNegotiator::new().negotiate(&gen, &eval);
        let found_a = contract
            .success_criteria
            .iter()
            .any(|c| c.contains("Step A"));
        let found_b = contract
            .success_criteria
            .iter()
            .any(|c| c.contains("Step B"));
        assert!(
            found_a,
            "Plan step 'Step A' should appear in success_criteria"
        );
        assert!(
            found_b,
            "Plan step 'Step B' should appear in success_criteria"
        );
    }

    #[test]
    fn test_negotiate_includes_evaluator_requirements_in_criteria() {
        let gen = GeneratorContext::new("task");
        let eval = EvaluatorContext::new(1.0)
            .with_requirement("All tests must pass")
            .with_requirement("Security gate must pass");
        let contract = ContractNegotiator::new().negotiate(&gen, &eval);
        assert!(
            contract
                .success_criteria
                .contains(&"All tests must pass".to_string()),
            "Evaluator requirement should be in success_criteria"
        );
        assert!(
            contract
                .success_criteria
                .contains(&"Security gate must pass".to_string()),
            "Evaluator requirement should be in success_criteria"
        );
    }

    #[test]
    fn test_negotiate_uses_evaluator_gates_when_provided() {
        let gen = GeneratorContext::new("task");
        let eval = EvaluatorContext::new(1.0)
            .with_gate(ValidationGate::Lint)
            .with_gate(ValidationGate::Test)
            .with_gate(ValidationGate::Security);
        let contract = ContractNegotiator::new().negotiate(&gen, &eval);
        assert_eq!(
            contract.validation_gates,
            vec![
                ValidationGate::Lint,
                ValidationGate::Test,
                ValidationGate::Security
            ]
        );
    }

    #[test]
    fn test_negotiate_uses_default_gates_when_evaluator_provides_none() {
        let gen = GeneratorContext::new("task");
        let eval = EvaluatorContext::new(1.0); // no gates added
        let contract = ContractNegotiator::new().negotiate(&gen, &eval);
        assert!(
            !contract.validation_gates.is_empty(),
            "Default gates should be added when evaluator provides none"
        );
        assert!(
            contract.validation_gates.contains(&ValidationGate::Lint),
            "Default gates should include Lint"
        );
        assert!(
            contract.validation_gates.contains(&ValidationGate::Test),
            "Default gates should include Test"
        );
    }

    #[test]
    fn test_negotiate_sets_acceptance_threshold() {
        let gen = GeneratorContext::new("task");
        let eval = EvaluatorContext::new(0.75);
        let contract = ContractNegotiator::new().negotiate(&gen, &eval);
        assert!(
            (contract.acceptance_threshold - 0.75).abs() < f64::EPSILON,
            "acceptance_threshold should be 0.75, got {}",
            contract.acceptance_threshold
        );
    }

    // ── SprintContract::evaluate ─────────────────────────────────────────────

    #[test]
    fn test_contract_accepted_when_all_gates_pass() {
        let gen = GeneratorContext::new("task");
        let eval = EvaluatorContext::new(1.0)
            .with_gate(ValidationGate::Lint)
            .with_gate(ValidationGate::Test);
        let contract = ContractNegotiator::new().negotiate(&gen, &eval);

        let mut results = HashMap::new();
        results.insert("lint".to_string(), true);
        results.insert("test".to_string(), true);

        assert_eq!(contract.evaluate(&results), ContractStatus::Accepted);
    }

    #[test]
    fn test_contract_rejected_when_gates_fail_below_threshold() {
        let gen = GeneratorContext::new("task");
        let eval = EvaluatorContext::new(1.0) // all must pass
            .with_gate(ValidationGate::Lint)
            .with_gate(ValidationGate::Test);
        let contract = ContractNegotiator::new().negotiate(&gen, &eval);

        let mut results = HashMap::new();
        results.insert("lint".to_string(), true);
        results.insert("test".to_string(), false); // test fails

        match contract.evaluate(&results) {
            ContractStatus::Rejected { failed_gates } => {
                assert!(failed_gates.contains(&"test".to_string()));
            }
            other => panic!("Expected Rejected, got {other:?}"),
        }
    }

    #[test]
    fn test_contract_partial_acceptance_threshold() {
        let gen = GeneratorContext::new("task");
        let eval = EvaluatorContext::new(0.5) // 50% must pass
            .with_gate(ValidationGate::Lint)
            .with_gate(ValidationGate::Test);
        let contract = ContractNegotiator::new().negotiate(&gen, &eval);

        let mut results = HashMap::new();
        results.insert("lint".to_string(), true);
        results.insert("test".to_string(), false);

        // 1/2 = 50% ≥ 0.5 threshold, so accepted
        assert_eq!(contract.evaluate(&results), ContractStatus::Accepted);
    }

    #[test]
    fn test_contract_with_custom_gate() {
        let gen = GeneratorContext::new("task");
        let eval = EvaluatorContext::new(1.0)
            .with_gate(ValidationGate::Custom("integration_tests".to_string()));
        let contract = ContractNegotiator::new().negotiate(&gen, &eval);

        assert!(
            contract
                .validation_gates
                .contains(&ValidationGate::Custom("integration_tests".to_string())),
            "Custom gate should be present"
        );

        let mut results = HashMap::new();
        results.insert("custom:integration_tests".to_string(), true);
        assert_eq!(contract.evaluate(&results), ContractStatus::Accepted);
    }

    #[test]
    fn test_negotiate_full_contract_all_required_fields() {
        let gen = default_generator_ctx();
        let eval = default_evaluator_ctx();
        let contract = ContractNegotiator::new().negotiate(&gen, &eval);

        // All three required fields must be non-trivially populated
        assert!(
            !contract.success_criteria.is_empty(),
            "success_criteria must not be empty"
        );
        assert!(
            !contract.validation_gates.is_empty(),
            "validation_gates must not be empty"
        );
        assert!(
            contract.acceptance_threshold >= 0.0 && contract.acceptance_threshold <= 1.0,
            "acceptance_threshold must be in [0.0, 1.0]"
        );
    }
}
