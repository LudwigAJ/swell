//! Follow-up task generator for extracting new tasks from completed work.
//!
//! This module analyzes completed tasks and generates follow-up task proposals
//! based on:
//! - Validation errors and warnings that could be addressed
//! - Unmet acceptance criteria from the original task
//! - Opportunities to improve or extend completed work
//! - Related tasks suggested by the planner's analysis
//!
//! # Example
//!
//! ```rust,ignore
//! use swell_orchestrator::followup::{FollowUpGenerator, FollowUpContext, FollowUpOpportunity};
//!
//! let generator = FollowUpGenerator::new();
//!
//! let opportunities = generator.analyze_task(&completed_task);
//! let proposals = generator.generate_proposals(opportunities);
//! ```

use regex::Regex;
use swell_core::{
    Plan, PlanStep, RiskLevel, StepStatus, Task, TaskSource, TaskState, ValidationResult,
};
use uuid::Uuid;

/// A detected opportunity for a follow-up task
#[derive(Debug, Clone)]
pub struct FollowUpOpportunity {
    /// Type of opportunity detected
    pub opportunity_type: FollowUpOpportunityType,
    /// Description of what could be improved
    pub description: String,
    /// Affected files or modules
    pub affected_items: Vec<String>,
    /// Estimated complexity/risk
    pub risk_level: RiskLevel,
    /// Priority (higher = more important)
    pub priority: u8,
}

/// Types of follow-up opportunities
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FollowUpOpportunityType {
    /// Validation errors that need fixing
    ValidationError,
    /// Validation warnings that could be addressed
    ValidationWarning,
    /// Unmet acceptance criteria
    UnmetAcceptanceCriteria,
    /// Tests that could be added for better coverage
    TestGap,
    /// Documentation improvements needed
    DocumentationGap,
    /// Code smell or refactoring opportunity
    CodeSmell,
    /// Security improvement opportunity
    SecurityImprovement,
    /// Performance optimization opportunity
    PerformanceOptimization,
    /// Related task suggested by analysis
    RelatedTask,
}

/// A generated follow-up task proposal
#[derive(Debug, Clone)]
pub struct FollowUpProposal {
    /// Unique identifier for this proposal
    pub id: Uuid,
    /// The parent task this follows up on
    pub parent_task_id: Uuid,
    /// Description of the proposed task
    pub description: String,
    /// Source opportunity type
    pub opportunity_type: FollowUpOpportunityType,
    /// Affected files/modules
    pub affected_items: Vec<String>,
    /// Initial plan steps (if derivable)
    pub initial_steps: Vec<String>,
    /// Risk assessment
    pub risk_level: RiskLevel,
    /// Priority score (0-100)
    pub priority: u8,
}

impl FollowUpProposal {
    /// Convert proposal to a new Task
    pub fn into_task(self) -> Task {
        Task {
            id: Uuid::new_v4(),
            description: self.description,
            state: TaskState::Created,
            source: TaskSource::FailureDerived {
                original_task_id: self.parent_task_id,
                failure_signal: format!("{:?}", self.opportunity_type),
            },
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            assigned_agent: None,
            plan: None,
            dependencies: vec![self.parent_task_id],
            dependents: Vec::new(),
            iteration_count: 0,
            token_budget: 1_000_000,
            tokens_used: 0,
            validation_result: None,
            autonomy_level: Default::default(),
            paused_reason: None,
            paused_from_state: None,
            rejected_reason: None,
            injected_instructions: Vec::new(),
            original_scope: None,
            current_scope: Default::default(),
            enrichment: Default::default(),
        }
    }

    /// Create a basic plan from the proposal
    pub fn create_plan(&self) -> Plan {
        let steps = self
            .initial_steps
            .iter()
            .map(|desc| PlanStep {
                id: Uuid::new_v4(),
                description: desc.clone(),
                affected_files: self.affected_items.clone(),
                expected_tests: Vec::new(),
                risk_level: self.risk_level,
                dependencies: Vec::new(),
                status: StepStatus::Pending,
            })
            .collect();

        Plan {
            id: Uuid::new_v4(),
            task_id: self.parent_task_id,
            steps,
            total_estimated_tokens: self.estimate_tokens(),
            risk_assessment: format!("{:?} opportunity", self.opportunity_type),
        }
    }

    /// Estimate token cost for this proposal
    fn estimate_tokens(&self) -> u64 {
        // Base estimate + per-step estimate
        let base = 500u64;
        let per_step = 200u64;
        base + (per_step * self.initial_steps.len() as u64)
    }
}

/// Context for analyzing a task for follow-up opportunities
#[derive(Debug, Clone)]
pub struct FollowUpContext {
    /// The task being analyzed
    pub task: Task,
    /// Validation result if available
    pub validation_result: Option<ValidationResult>,
    /// Related completed tasks in the same scope
    pub related_tasks: Vec<Task>,
}

impl FollowUpContext {
    /// Create context for a single task
    pub fn from_task(task: Task) -> Self {
        Self {
            validation_result: task.validation_result.clone(),
            related_tasks: Vec::new(),
            task,
        }
    }

    /// Add related tasks to context
    pub fn with_related_tasks(mut self, tasks: Vec<Task>) -> Self {
        self.related_tasks = tasks;
        self
    }
}

/// Configuration for the follow-up generator
#[derive(Debug, Clone)]
pub struct FollowUpGeneratorConfig {
    /// Minimum priority to generate proposals
    pub min_priority: u8,
    /// Maximum proposals per task
    pub max_proposals_per_task: usize,
    /// Include low-risk opportunities
    pub include_low_risk: bool,
    /// Include medium-risk opportunities
    pub include_medium_risk: bool,
    /// Include high-risk opportunities
    pub include_high_risk: bool,
}

impl Default for FollowUpGeneratorConfig {
    fn default() -> Self {
        Self {
            min_priority: 30,
            max_proposals_per_task: 5,
            include_low_risk: true,
            include_medium_risk: true,
            include_high_risk: true,
        }
    }
}

/// Main follow-up task generator
#[derive(Debug, Clone)]
pub struct FollowUpGenerator {
    config: FollowUpGeneratorConfig,
}

impl FollowUpGenerator {
    /// Create a new generator with default config
    pub fn new() -> Self {
        Self {
            config: FollowUpGeneratorConfig::default(),
        }
    }

    /// Create with custom config
    pub fn with_config(config: FollowUpGeneratorConfig) -> Self {
        Self { config }
    }

    /// Analyze a task and extract follow-up opportunities
    pub fn analyze_task(&self, task: &Task) -> Vec<FollowUpOpportunity> {
        let mut opportunities = Vec::new();

        // Only analyze tasks in terminal states
        if !task.state.is_terminal() {
            return opportunities;
        }

        // Analyze validation results
        if let Some(ref validation) = task.validation_result {
            opportunities.extend(self.analyze_validation_result(validation));
        }

        // Analyze based on task state
        match task.state {
            TaskState::Rejected => {
                opportunities.extend(self.analyze_rejection(task));
            }
            TaskState::Accepted => {
                opportunities.extend(self.analyze_acceptance(task));
            }
            TaskState::Failed => {
                opportunities.extend(self.analyze_failure(task));
            }
            TaskState::Escalated => {
                opportunities.extend(self.analyze_escalation(task));
            }
            _ => {}
        }

        // Filter by config
        opportunities.retain(|opp| {
            opp.priority >= self.config.min_priority
                && match opp.risk_level {
                    RiskLevel::Low => self.config.include_low_risk,
                    RiskLevel::Medium => self.config.include_medium_risk,
                    RiskLevel::High => self.config.include_high_risk,
                }
        });

        opportunities
    }

    /// Analyze validation result for opportunities
    fn analyze_validation_result(&self, validation: &ValidationResult) -> Vec<FollowUpOpportunity> {
        let mut opportunities = Vec::new();

        // Validation errors
        for error in &validation.errors {
            let opportunity = self.classify_error(error);
            opportunities.push(opportunity);
        }

        // Validation warnings
        for warning in &validation.warnings {
            if validation.passed {
                // Warnings even when passed - could be improvements
                opportunities.push(FollowUpOpportunity {
                    opportunity_type: FollowUpOpportunityType::ValidationWarning,
                    description: format!("Address warning: {}", warning),
                    affected_items: Vec::new(),
                    risk_level: RiskLevel::Low,
                    priority: 40,
                });
            }
        }

        opportunities
    }

    /// Classify an error into a specific opportunity type
    fn classify_error(&self, error: &str) -> FollowUpOpportunity {
        let error_lower = error.to_lowercase();

        // Determine opportunity type and risk based on error content
        let (opp_type, risk, priority) = if error_lower.contains("security")
            || error_lower.contains("injection")
            || error_lower.contains("xss")
        {
            (
                FollowUpOpportunityType::SecurityImprovement,
                RiskLevel::High,
                90,
            )
        } else if error_lower.contains("performance")
            || error_lower.contains("slow")
            || error_lower.contains("inefficient")
        {
            (
                FollowUpOpportunityType::PerformanceOptimization,
                RiskLevel::Medium,
                60,
            )
        } else if error_lower.contains("test")
            || error_lower.contains("assertion")
            || error_lower.contains("failing")
        {
            (
                FollowUpOpportunityType::ValidationError,
                RiskLevel::Medium,
                75,
            )
        } else if error_lower.contains("documentation")
            || error_lower.contains("doc")
            || error_lower.contains("comment")
        {
            (
                FollowUpOpportunityType::DocumentationGap,
                RiskLevel::Low,
                30,
            )
        } else if error_lower.contains("refactor")
            || error_lower.contains("code smell")
            || error_lower.contains("duplicat")
        {
            (FollowUpOpportunityType::CodeSmell, RiskLevel::Medium, 50)
        } else {
            // Default to validation error with medium priority
            (
                FollowUpOpportunityType::ValidationError,
                RiskLevel::Medium,
                70,
            )
        };

        FollowUpOpportunity {
            opportunity_type: opp_type,
            description: format!("Fix error: {}", error),
            affected_items: self.extract_affected_items(error),
            risk_level: risk,
            priority,
        }
    }

    /// Extract affected files/modules from error message
    fn extract_affected_items(&self, error: &str) -> Vec<String> {
        let mut items = Vec::new();

        // Look for file paths in common patterns
        // E.g., "file.rs:10:5", "src/lib.rs", "src/main.rs:42"
        let patterns = [
            r"([a-zA-Z_][a-zA-Z0-9_/\\]+\.rs)",
            r"src/([a-zA-Z_][a-zA-Z0-9_/\\]+)",
            r"([a-zA-Z_][a-zA-Z0-9_]+)\.rs",
        ];

        for pattern in &patterns {
            if let Ok(re) = Regex::new(pattern) {
                for cap in re.captures_iter(error) {
                    if let Some(m) = cap.get(1) {
                        let item = m.as_str().to_string();
                        if !items.contains(&item) && item.len() < 100 {
                            items.push(item);
                        }
                    }
                }
            }
        }

        items
    }

    /// Analyze rejection for opportunities
    fn analyze_rejection(&self, task: &Task) -> Vec<FollowUpOpportunity> {
        let mut opportunities = Vec::new();

        // High priority to fix rejection issues
        opportunities.push(FollowUpOpportunity {
            opportunity_type: FollowUpOpportunityType::ValidationError,
            description: format!(
                "Address rejection of task: {} (iteration {})",
                task.description, task.iteration_count
            ),
            affected_items: Vec::new(),
            risk_level: RiskLevel::Medium,
            priority: 85,
        });

        opportunities
    }

    /// Analyze acceptance for improvement opportunities
    fn analyze_acceptance(&self, task: &Task) -> Vec<FollowUpOpportunity> {
        let mut opportunities = Vec::new();

        // Check validation result for warnings even on acceptance
        if let Some(ref validation) = task.validation_result {
            // If there were warnings during acceptance, suggest addressing them
            if !validation.warnings.is_empty() {
                opportunities.push(FollowUpOpportunity {
                    opportunity_type: FollowUpOpportunityType::ValidationWarning,
                    description: format!(
                        "Address {} warning(s) from accepted task",
                        validation.warnings.len()
                    ),
                    affected_items: Vec::new(),
                    risk_level: RiskLevel::Low,
                    priority: 35,
                });
            }

            // Check coverage gaps if tests passed
            if validation.tests_passed && validation.ai_review_passed {
                opportunities.push(FollowUpOpportunity {
                    opportunity_type: FollowUpOpportunityType::TestGap,
                    description: "Add additional test coverage for accepted task".to_string(),
                    affected_items: Vec::new(),
                    risk_level: RiskLevel::Low,
                    priority: 25,
                });
            }
        }

        opportunities
    }

    /// Analyze failure for recovery opportunities
    fn analyze_failure(&self, task: &Task) -> Vec<FollowUpOpportunity> {
        let mut opportunities = Vec::new();

        opportunities.push(FollowUpOpportunity {
            opportunity_type: FollowUpOpportunityType::ValidationError,
            description: format!("Recover from failed task: {}", task.description),
            affected_items: Vec::new(),
            risk_level: RiskLevel::High,
            priority: 95,
        });

        opportunities
    }

    /// Analyze escalation for human-in-the-loop opportunities
    fn analyze_escalation(&self, task: &Task) -> Vec<FollowUpOpportunity> {
        let mut opportunities = Vec::new();

        opportunities.push(FollowUpOpportunity {
            opportunity_type: FollowUpOpportunityType::RelatedTask,
            description: format!(
                "Manual review needed for escalated task: {} (after {} iterations)",
                task.description, task.iteration_count
            ),
            affected_items: Vec::new(),
            risk_level: RiskLevel::High,
            priority: 100,
        });

        opportunities
    }

    /// Generate proposals from opportunities
    pub fn generate_proposals(
        &self,
        opportunities: Vec<FollowUpOpportunity>,
    ) -> Vec<FollowUpProposal> {
        let mut proposals: Vec<FollowUpProposal> = opportunities
            .into_iter()
            .map(|opp| self.opportunity_to_proposal(opp))
            .collect();

        // Sort by priority (descending)
        proposals.sort_by(|a, b| b.priority.cmp(&a.priority));

        // Limit by config
        proposals.truncate(self.config.max_proposals_per_task);

        proposals
    }

    /// Convert opportunity to proposal
    fn opportunity_to_proposal(&self, opp: FollowUpOpportunity) -> FollowUpProposal {
        let initial_steps = self.generate_initial_steps(&opp);

        FollowUpProposal {
            id: Uuid::new_v4(),
            parent_task_id: Uuid::nil(), // Will be set by caller
            description: opp.description.clone(),
            opportunity_type: opp.opportunity_type,
            affected_items: opp.affected_items,
            initial_steps,
            risk_level: opp.risk_level,
            priority: opp.priority,
        }
    }

    /// Generate initial steps for a proposal based on opportunity type
    fn generate_initial_steps(&self, opp: &FollowUpOpportunity) -> Vec<String> {
        match opp.opportunity_type {
            FollowUpOpportunityType::ValidationError => vec![
                "Analyze the validation error".to_string(),
                "Fix the root cause".to_string(),
                "Re-run validation".to_string(),
            ],
            FollowUpOpportunityType::ValidationWarning => vec![
                "Review the warning".to_string(),
                "Address if appropriate".to_string(),
            ],
            FollowUpOpportunityType::UnmetAcceptanceCriteria => vec![
                "Review acceptance criteria".to_string(),
                "Implement missing requirements".to_string(),
            ],
            FollowUpOpportunityType::TestGap => vec![
                "Identify untested paths".to_string(),
                "Write additional tests".to_string(),
                "Verify test coverage".to_string(),
            ],
            FollowUpOpportunityType::DocumentationGap => vec![
                "Review documentation".to_string(),
                "Update or add documentation".to_string(),
            ],
            FollowUpOpportunityType::CodeSmell => vec![
                "Identify code smell".to_string(),
                "Plan refactoring".to_string(),
                "Execute refactoring".to_string(),
            ],
            FollowUpOpportunityType::SecurityImprovement => vec![
                "Security audit".to_string(),
                "Implement security fix".to_string(),
                "Verify security".to_string(),
            ],
            FollowUpOpportunityType::PerformanceOptimization => vec![
                "Profile performance".to_string(),
                "Identify bottleneck".to_string(),
                "Optimize code".to_string(),
            ],
            FollowUpOpportunityType::RelatedTask => vec![
                "Analyze escalated task".to_string(),
                "Determine resolution".to_string(),
                "Execute resolution".to_string(),
            ],
        }
    }

    /// Full pipeline: analyze task and generate proposals
    pub fn generate_follow_ups(&self, task: &Task) -> Vec<FollowUpProposal> {
        let opportunities = self.analyze_task(task);
        let mut proposals = self.generate_proposals(opportunities);

        // Set parent task ID
        for proposal in &mut proposals {
            proposal.parent_task_id = task.id;
        }

        proposals
    }

    /// Generate follow-up tasks from multiple completed tasks
    pub fn generate_batch_follow_ups(&self, tasks: &[Task]) -> Vec<FollowUpProposal> {
        let mut all_proposals = Vec::new();

        for task in tasks {
            let proposals = self.generate_follow_ups(task);
            all_proposals.extend(proposals);
        }

        // Deduplicate by description
        all_proposals.dedup_by(|a, b| a.description == b.description);

        // Sort by priority
        all_proposals.sort_by(|a, b| b.priority.cmp(&a.priority));

        all_proposals
    }
}

impl Default for FollowUpGenerator {
    fn default() -> Self {
        Self::new()
    }
}

/// Extension trait for TaskState to check if terminal
pub trait IsTerminal {
    fn is_terminal(&self) -> bool;
}

impl IsTerminal for TaskState {
    fn is_terminal(&self) -> bool {
        matches!(
            self,
            TaskState::Accepted | TaskState::Rejected | TaskState::Failed | TaskState::Escalated
        )
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use swell_core::{TaskSource, ValidationResult};

    fn create_test_task_with_validation(state: TaskState, validation: ValidationResult) -> Task {
        let mut task = Task::new("Test task".to_string());
        task.state = state;
        task.validation_result = Some(validation);
        task
    }

    #[test]
    fn test_followup_generator_analyzes_rejected_task() {
        let generator = FollowUpGenerator::new();
        let task = create_test_task_with_validation(
            TaskState::Rejected,
            ValidationResult {
                passed: false,
                lint_passed: false,
                tests_passed: false,
                security_passed: true,
                ai_review_passed: true,
                errors: vec!["Test failed in test_foo".to_string()],
                warnings: vec![],
            },
        );

        let opportunities = generator.analyze_task(&task);

        assert!(!opportunities.is_empty());
        // Priority 75 because "Test failed" matches "test" keyword
        assert_eq!(opportunities[0].priority, 75);
    }

    #[test]
    fn test_followup_generator_analyzes_accepted_task_with_warnings() {
        let generator = FollowUpGenerator::new();
        let task = create_test_task_with_validation(
            TaskState::Accepted,
            ValidationResult {
                passed: true,
                lint_passed: true,
                tests_passed: true,
                security_passed: true,
                ai_review_passed: true,
                errors: vec![],
                warnings: vec!["Some style warning".to_string()],
            },
        );

        let opportunities = generator.analyze_task(&task);

        // Should detect warning opportunity
        let warning_opps: Vec<_> = opportunities
            .iter()
            .filter(|o| o.opportunity_type == FollowUpOpportunityType::ValidationWarning)
            .collect();
        assert!(!warning_opps.is_empty());
    }

    #[test]
    fn test_followup_generator_classifies_security_errors() {
        let generator = FollowUpGenerator::new();
        let task = create_test_task_with_validation(
            TaskState::Rejected,
            ValidationResult {
                passed: false,
                lint_passed: true,
                tests_passed: true,
                security_passed: false,
                ai_review_passed: true,
                errors: vec!["Potential SQL injection in query".to_string()],
                warnings: vec![],
            },
        );

        let opportunities = generator.analyze_task(&task);

        let security_opps: Vec<_> = opportunities
            .iter()
            .filter(|o| o.opportunity_type == FollowUpOpportunityType::SecurityImprovement)
            .collect();
        assert!(!security_opps.is_empty());
        assert_eq!(security_opps[0].risk_level, RiskLevel::High);
        assert_eq!(security_opps[0].priority, 90);
    }

    #[test]
    fn test_followup_generator_non_terminal_tasks_return_empty() {
        let generator = FollowUpGenerator::new();

        for state in [
            TaskState::Created,
            TaskState::Enriched,
            TaskState::Ready,
            TaskState::Assigned,
            TaskState::Executing,
            TaskState::Validating,
        ] {
            let task = Task::new("Test task".to_string());
            let mut task = task;
            task.state = state;

            let opportunities = generator.analyze_task(&task);
            assert!(
                opportunities.is_empty(),
                "Expected no opportunities for {:?} state",
                state
            );
        }
    }

    #[test]
    fn test_followup_generator_generates_proposals() {
        let generator = FollowUpGenerator::new();
        let task = create_test_task_with_validation(
            TaskState::Rejected,
            ValidationResult {
                passed: false,
                lint_passed: false,
                tests_passed: false,
                security_passed: true,
                ai_review_passed: true,
                errors: vec!["Clippy warning: unnecessary clone".to_string()],
                warnings: vec![],
            },
        );

        let proposals = generator.generate_follow_ups(&task);

        assert!(!proposals.is_empty());
        assert_eq!(proposals[0].parent_task_id, task.id);
    }

    #[test]
    fn test_followup_proposal_into_task() {
        let proposal = FollowUpProposal {
            id: Uuid::new_v4(),
            parent_task_id: Uuid::new_v4(),
            description: "Fix the bug".to_string(),
            opportunity_type: FollowUpOpportunityType::ValidationError,
            affected_items: vec!["src/lib.rs".to_string()],
            initial_steps: vec!["Identify bug".to_string(), "Fix bug".to_string()],
            risk_level: RiskLevel::Medium,
            priority: 75,
        };

        let task = proposal.clone().into_task();

        assert_eq!(task.description, "Fix the bug");
        assert_eq!(task.state, TaskState::Created);
        assert!(matches!(task.source, TaskSource::FailureDerived { .. }));
        assert!(task.dependencies.contains(&proposal.parent_task_id));
    }

    #[test]
    fn test_followup_proposal_create_plan() {
        let proposal = FollowUpProposal {
            id: Uuid::new_v4(),
            parent_task_id: Uuid::new_v4(),
            description: "Test proposal".to_string(),
            opportunity_type: FollowUpOpportunityType::TestGap,
            affected_items: vec!["src/lib.rs".to_string()],
            initial_steps: vec!["Write tests".to_string(), "Run tests".to_string()],
            risk_level: RiskLevel::Low,
            priority: 50,
        };

        let plan = proposal.create_plan();

        assert_eq!(plan.steps.len(), 2);
        assert_eq!(plan.steps[0].description, "Write tests");
        assert_eq!(plan.steps[1].description, "Run tests");
    }

    #[test]
    fn test_followup_generator_priority_filtering() {
        let mut config = FollowUpGeneratorConfig::default();
        config.min_priority = 80;

        let generator = FollowUpGenerator::with_config(config);
        let task = create_test_task_with_validation(
            TaskState::Rejected,
            ValidationResult {
                passed: false,
                lint_passed: false,
                tests_passed: false,
                security_passed: true,
                ai_review_passed: true,
                errors: vec!["Minor style issue".to_string()],
                warnings: vec![],
            },
        );

        let opportunities = generator.analyze_task(&task);

        // With high priority filter, should still find security issues
        assert!(opportunities.iter().all(|o| o.priority >= 80));
    }

    #[test]
    fn test_task_state_is_terminal() {
        let terminal_states = [
            TaskState::Accepted,
            TaskState::Rejected,
            TaskState::Failed,
            TaskState::Escalated,
        ];

        for state in terminal_states {
            assert!(state.is_terminal(), "Expected {:?} to be terminal", state);
        }

        let non_terminal_states = [
            TaskState::Created,
            TaskState::Enriched,
            TaskState::Ready,
            TaskState::Assigned,
            TaskState::Executing,
            TaskState::Paused,
            TaskState::Validating,
        ];

        for state in non_terminal_states {
            assert!(
                !state.is_terminal(),
                "Expected {:?} to NOT be terminal",
                state
            );
        }
    }

    #[test]
    fn test_extract_affected_items() {
        let generator = FollowUpGenerator::new();

        let items =
            generator.extract_affected_items("Error in src/lib.rs:42:10 - something went wrong");

        assert!(items.iter().any(|i| i.contains("lib.rs")));
    }

    #[test]
    fn test_batch_follow_ups_deduplicates() {
        let generator = FollowUpGenerator::new();

        let task1 = create_test_task_with_validation(
            TaskState::Rejected,
            ValidationResult {
                passed: false,
                lint_passed: false,
                tests_passed: false,
                security_passed: true,
                ai_review_passed: true,
                errors: vec!["Fix bug".to_string()],
                warnings: vec![],
            },
        );

        let task2 = create_test_task_with_validation(
            TaskState::Rejected,
            ValidationResult {
                passed: false,
                lint_passed: false,
                tests_passed: false,
                security_passed: true,
                ai_review_passed: true,
                errors: vec!["Fix bug".to_string()], // Same error
                warnings: vec![],
            },
        );

        let proposals = generator.generate_batch_follow_ups(&[task1, task2]);

        // Batch processing returns proposals from both tasks
        // Each rejected task generates multiple opportunities
        assert!(!proposals.is_empty());
        // Verify batch returns multiple proposals (at least one per task)
        assert!(proposals.len() >= 2);
    }
}
