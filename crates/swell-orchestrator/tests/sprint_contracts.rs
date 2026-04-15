//! Sprint Contract integration tests.
//!
//! Validates VAL-OBS-011: Sprint contracts — ContractNegotiator establishes
//! SprintContract between Generator and Evaluator agents with success_criteria,
//! validation_gates, and acceptance_threshold.

use std::collections::HashMap;
use swell_orchestrator::{
    ContractNegotiator, ContractStatus, EvaluatorContext, GeneratorContext, ValidationGate,
};

// ── Contract creation — all required fields ───────────────────────────────────

/// VAL-OBS-011: ContractNegotiator::negotiate() returns a SprintContract with
/// non-empty success_criteria and at least one validation gate.
#[test]
fn test_contract_creation_with_all_required_fields() {
    let gen = GeneratorContext::new("Implement sprint contract feature")
        .with_step("Write failing tests")
        .with_step("Implement ContractNegotiator")
        .with_step("Add integration tests")
        .with_tool("file_read")
        .with_tool("shell");

    let eval = EvaluatorContext::new(1.0)
        .with_requirement("All tests must pass")
        .with_requirement("No clippy warnings allowed")
        .with_gate(ValidationGate::Lint)
        .with_gate(ValidationGate::Test);

    let negotiator = ContractNegotiator::new();
    let contract = negotiator.negotiate(&gen, &eval);

    // success_criteria: non-empty (task description + plan steps + eval requirements)
    assert!(
        !contract.success_criteria.is_empty(),
        "SprintContract.success_criteria must be non-empty"
    );
    // Includes at minimum the task description
    assert!(
        contract
            .success_criteria
            .iter()
            .any(|c| c.contains("Implement sprint contract feature")),
        "success_criteria must contain the task description"
    );

    // validation_gates: at least one gate
    assert!(
        !contract.validation_gates.is_empty(),
        "SprintContract.validation_gates must have at least one gate"
    );
    assert!(
        contract.validation_gates.contains(&ValidationGate::Lint),
        "Lint gate should be present"
    );
    assert!(
        contract.validation_gates.contains(&ValidationGate::Test),
        "Test gate should be present"
    );

    // acceptance_threshold: valid value
    assert!(
        (contract.acceptance_threshold - 1.0).abs() < f64::EPSILON,
        "acceptance_threshold should be 1.0, got {}",
        contract.acceptance_threshold
    );
}

// ── Contract creation — minimal contexts ─────────────────────────────────────

#[test]
fn test_contract_created_with_minimal_generator_context() {
    let gen = GeneratorContext::new("Fix bug");
    let eval = EvaluatorContext::new(0.9).with_gate(ValidationGate::Test);

    let contract = ContractNegotiator::new().negotiate(&gen, &eval);

    assert!(
        !contract.success_criteria.is_empty(),
        "success_criteria must not be empty even with minimal generator context"
    );
    assert!(
        !contract.validation_gates.is_empty(),
        "validation_gates must not be empty"
    );
}

#[test]
fn test_contract_uses_default_gates_when_evaluator_has_none() {
    let gen = GeneratorContext::new("Some task");
    let eval = EvaluatorContext::new(0.8); // no gates explicitly added

    let contract = ContractNegotiator::new().negotiate(&gen, &eval);

    // Should have at least one gate (defaults to Lint + Test)
    assert!(
        !contract.validation_gates.is_empty(),
        "validation_gates must default to at least one gate when evaluator provides none"
    );
}

// ── Contract enforcement during execution ────────────────────────────────────

#[test]
fn test_contract_enforcement_all_gates_pass() {
    let gen = GeneratorContext::new("Add caching")
        .with_step("Design cache struct")
        .with_step("Implement and test");

    let eval = EvaluatorContext::new(1.0)
        .with_requirement("Cache must be thread-safe")
        .with_gate(ValidationGate::Lint)
        .with_gate(ValidationGate::Test);

    let contract = ContractNegotiator::new().negotiate(&gen, &eval);

    let mut gate_results = HashMap::new();
    gate_results.insert("lint".to_string(), true);
    gate_results.insert("test".to_string(), true);

    assert_eq!(
        contract.evaluate(&gate_results),
        ContractStatus::Accepted,
        "Contract should be Accepted when all gates pass"
    );
}

#[test]
fn test_contract_enforcement_fails_when_gates_fail() {
    let gen = GeneratorContext::new("Refactor module");
    let eval = EvaluatorContext::new(1.0)
        .with_gate(ValidationGate::Lint)
        .with_gate(ValidationGate::Test);

    let contract = ContractNegotiator::new().negotiate(&gen, &eval);

    let mut gate_results = HashMap::new();
    gate_results.insert("lint".to_string(), true);
    gate_results.insert("test".to_string(), false); // test fails

    match contract.evaluate(&gate_results) {
        ContractStatus::Rejected { failed_gates } => {
            assert!(
                failed_gates.contains(&"test".to_string()),
                "Rejected contract should list 'test' as a failed gate"
            );
        }
        ContractStatus::Accepted => panic!("Expected Rejected, got Accepted"),
        ContractStatus::Pending => panic!("Expected Rejected, got Pending"),
    }
}

#[test]
fn test_contract_enforcement_partial_threshold() {
    let gen = GeneratorContext::new("Update dependencies");
    let eval = EvaluatorContext::new(0.5) // only 50% of gates need to pass
        .with_gate(ValidationGate::Lint)
        .with_gate(ValidationGate::Test);

    let contract = ContractNegotiator::new().negotiate(&gen, &eval);

    // Lint passes, Test fails → 50% pass rate ≥ 0.5 threshold
    let mut gate_results = HashMap::new();
    gate_results.insert("lint".to_string(), true);
    gate_results.insert("test".to_string(), false);

    assert_eq!(
        contract.evaluate(&gate_results),
        ContractStatus::Accepted,
        "Contract should be Accepted when pass rate meets acceptance_threshold"
    );
}

#[test]
fn test_contract_enforcement_security_and_ai_review_gates() {
    let gen = GeneratorContext::new("Implement auth module");
    let eval = EvaluatorContext::new(1.0)
        .with_requirement("No security vulnerabilities")
        .with_gate(ValidationGate::Security)
        .with_gate(ValidationGate::AiReview);

    let contract = ContractNegotiator::new().negotiate(&gen, &eval);
    assert!(
        contract
            .validation_gates
            .contains(&ValidationGate::Security),
        "Security gate should be present"
    );
    assert!(
        contract
            .validation_gates
            .contains(&ValidationGate::AiReview),
        "AiReview gate should be present"
    );

    let mut gate_results = HashMap::new();
    gate_results.insert("security".to_string(), true);
    gate_results.insert("ai_review".to_string(), true);

    assert_eq!(contract.evaluate(&gate_results), ContractStatus::Accepted);
}

#[test]
fn test_contract_enforcement_custom_gate() {
    let gen = GeneratorContext::new("Add integration tests");
    let eval = EvaluatorContext::new(1.0)
        .with_gate(ValidationGate::Custom("integration_suite".to_string()));

    let contract = ContractNegotiator::new().negotiate(&gen, &eval);
    assert!(
        contract
            .validation_gates
            .contains(&ValidationGate::Custom("integration_suite".to_string())),
        "Custom gate should be in validation_gates"
    );

    let mut results = HashMap::new();
    results.insert("custom:integration_suite".to_string(), true);
    assert_eq!(contract.evaluate(&results), ContractStatus::Accepted);
}

// ── Success criteria content ──────────────────────────────────────────────────

#[test]
fn test_success_criteria_includes_all_evaluator_requirements() {
    let gen = GeneratorContext::new("task");
    let eval = EvaluatorContext::new(1.0)
        .with_requirement("Requirement A")
        .with_requirement("Requirement B")
        .with_requirement("Requirement C");

    let contract = ContractNegotiator::new().negotiate(&gen, &eval);

    for req in ["Requirement A", "Requirement B", "Requirement C"] {
        assert!(
            contract.success_criteria.contains(&req.to_string()),
            "'{req}' should be in success_criteria"
        );
    }
}

#[test]
fn test_success_criteria_includes_numbered_plan_steps() {
    let gen = GeneratorContext::new("Build feature")
        .with_step("Alpha step")
        .with_step("Beta step");
    let eval = EvaluatorContext::new(1.0);

    let contract = ContractNegotiator::new().negotiate(&gen, &eval);

    let criteria_str = contract.success_criteria.join("\n");
    assert!(
        criteria_str.contains("Alpha step"),
        "Plan step 'Alpha step' should appear in success_criteria"
    );
    assert!(
        criteria_str.contains("Beta step"),
        "Plan step 'Beta step' should appear in success_criteria"
    );
}
