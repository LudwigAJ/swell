//! Gap Analyzer - compares specification to codebase to identify missing requirements.
//!
//! This module provides functionality to:
//! - Parse specification requirements from the spec document
//! - Compare to implemented features in the codebase
//! - Report gaps and missing items

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Category of specification requirement
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RequirementCategory {
    /// Core orchestration capabilities
    Orchestration,
    /// Agent implementation
    Agents,
    /// Validation pipeline
    Validation,
    /// Safety controls (sandbox, permissions, cost guard, etc.)
    Safety,
    /// Memory and persistence
    Memory,
    /// Tools and tool execution
    Tools,
    /// Git and branching workflow
    GitWorkflow,
    /// CLI and client interface
    Client,
    /// Observability and monitoring
    Observability,
}

/// Priority level of a requirement
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum RequirementPriority {
    /// MVP requirement - must be implemented
    MustHave,
    /// Should have for production quality
    ShouldHave,
    /// Nice to have but not critical
    NiceToHave,
    /// Future/Phase 2+ requirement
    Future,
}

/// Status of a requirement in the codebase
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImplementationStatus {
    /// Fully implemented and tested
    Implemented,
    /// Partially implemented (stub or incomplete)
    Partial,
    /// Not implemented but expected
    Missing,
    /// Out of scope for current phase
    OutOfScope,
}

/// A single specification requirement
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpecRequirement {
    /// Unique identifier for the requirement
    pub id: String,
    /// Human-readable description
    pub description: String,
    /// Category this requirement belongs to
    pub category: RequirementCategory,
    /// Priority level
    pub priority: RequirementPriority,
    /// Expected implementation location (crate or module)
    pub expected_location: Option<String>,
    /// Key functions/types that should exist
    pub expected_symbols: Vec<String>,
    /// Current implementation status
    pub status: ImplementationStatus,
    /// Notes about the implementation gap
    pub gap_notes: Option<String>,
}

/// A gap report for a category
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryGapReport {
    pub category: RequirementCategory,
    pub total_requirements: usize,
    pub implemented: usize,
    pub partial: usize,
    pub missing: usize,
    pub out_of_scope: usize,
    pub coverage_percentage: f64,
    pub requirements: Vec<SpecRequirement>,
}

/// Complete gap analysis report
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GapAnalysisReport {
    /// Overall coverage percentage
    pub overall_coverage_percentage: f64,
    /// Total requirements analyzed
    pub total_requirements: usize,
    /// Total implemented
    pub total_implemented: usize,
    /// Total partial
    pub total_partial: usize,
    /// Total missing
    pub total_missing: usize,
    /// Total out of scope
    pub total_out_of_scope: usize,
    /// Gap reports by category
    pub category_reports: Vec<CategoryGapReport>,
    /// Critical gaps that should be addressed first
    pub critical_gaps: Vec<String>,
    /// Recommendations for closing gaps
    pub recommendations: Vec<String>,
}

/// Configuration for gap analysis
#[derive(Debug, Clone)]
pub struct GapAnalyzerConfig {
    /// Whether to include out-of-scope items in analysis
    pub include_out_of_scope: bool,
    /// Whether to check for specific function signatures
    pub check_signatures: bool,
    /// Whether to verify trait implementations
    pub check_trait_impls: bool,
}

impl Default for GapAnalyzerConfig {
    fn default() -> Self {
        Self {
            include_out_of_scope: true,
            check_signatures: true,
            check_trait_impls: true,
        }
    }
}

/// Gap Analyzer for comparing specification to implementation
pub struct GapAnalyzer {
    config: GapAnalyzerConfig,
}

impl GapAnalyzer {
    /// Create a new GapAnalyzer with default config
    pub fn new() -> Self {
        Self {
            config: GapAnalyzerConfig::default(),
        }
    }

    /// Create a new GapAnalyzer with custom config
    pub fn with_config(config: GapAnalyzerConfig) -> Self {
        Self { config }
    }

    /// Get all specification requirements
    pub fn get_spec_requirements() -> Vec<SpecRequirement> {
        vec![
            // ============================================
            // ORCHESTRATION REQUIREMENTS
            // ============================================
            SpecRequirement {
                id: "ORCH-001".to_string(),
                description: "TaskStateMachine with 10 states (Created→Enriched→Ready→Assigned→Executing→Validating→Accepted/Rejected→Failed/Escalated)".to_string(),
                category: RequirementCategory::Orchestration,
                priority: RequirementPriority::MustHave,
                expected_location: Some("swell-orchestrator::state_machine".to_string()),
                expected_symbols: vec!["TaskStateMachine".to_string()],
                status: ImplementationStatus::Implemented,
                gap_notes: None,
            },
            SpecRequirement {
                id: "ORCH-002".to_string(),
                description: "TaskGraph for dependency tracking with topological sort".to_string(),
                category: RequirementCategory::Orchestration,
                priority: RequirementPriority::MustHave,
                expected_location: Some("swell-orchestrator::task_graph".to_string()),
                expected_symbols: vec!["TaskGraph".to_string()],
                status: ImplementationStatus::Implemented,
                gap_notes: None,
            },
            SpecRequirement {
                id: "ORCH-003".to_string(),
                description: "AgentPool with register/reserve/release/available_count".to_string(),
                category: RequirementCategory::Orchestration,
                priority: RequirementPriority::MustHave,
                expected_location: Some("swell-orchestrator::agents".to_string()),
                expected_symbols: vec!["AgentPool".to_string()],
                status: ImplementationStatus::Implemented,
                gap_notes: None,
            },
            SpecRequirement {
                id: "ORCH-004".to_string(),
                description: "ExecutionController for Planner→Generator→Evaluator pipeline".to_string(),
                category: RequirementCategory::Orchestration,
                priority: RequirementPriority::MustHave,
                expected_location: Some("swell-orchestrator::execution".to_string()),
                expected_symbols: vec!["ExecutionController".to_string()],
                status: ImplementationStatus::Implemented,
                gap_notes: None,
            },
            SpecRequirement {
                id: "ORCH-005".to_string(),
                description: "Scheduler with priority-based queue and max workers enforcement".to_string(),
                category: RequirementCategory::Orchestration,
                priority: RequirementPriority::MustHave,
                expected_location: Some("swell-orchestrator::scheduler".to_string()),
                expected_symbols: vec!["Scheduler".to_string()],
                status: ImplementationStatus::Implemented,
                gap_notes: None,
            },
            SpecRequirement {
                id: "ORCH-006".to_string(),
                description: "PolicyEngine evaluating YAML-defined policies with deny-first semantics".to_string(),
                category: RequirementCategory::Orchestration,
                priority: RequirementPriority::MustHave,
                expected_location: Some("swell-orchestrator::policy".to_string()),
                expected_symbols: vec!["PolicyEngine".to_string()],
                status: ImplementationStatus::Implemented,
                gap_notes: None,
            },
            SpecRequirement {
                id: "ORCH-007".to_string(),
                description: "DriftDetector comparing modified files against plan".to_string(),
                category: RequirementCategory::Orchestration,
                priority: RequirementPriority::MustHave,
                expected_location: Some("swell-orchestrator::drift_detector".to_string()),
                expected_symbols: vec!["DriftDetector".to_string()],
                status: ImplementationStatus::Implemented,
                gap_notes: None,
            },
            SpecRequirement {
                id: "ORCH-008".to_string(),
                description: "FeatureLead sub-orchestrator for complex tasks with 2-level max depth".to_string(),
                category: RequirementCategory::Orchestration,
                priority: RequirementPriority::ShouldHave,
                expected_location: Some("swell-orchestrator::feature_leads".to_string()),
                expected_symbols: vec!["FeatureLead".to_string(), "FeatureLeadManager".to_string()],
                status: ImplementationStatus::Implemented,
                gap_notes: None,
            },
            SpecRequirement {
                id: "ORCH-009".to_string(),
                description: "Backlog aggregation from 4 sources (plan tasks, failure-derived, spec-gap, improvements)".to_string(),
                category: RequirementCategory::Orchestration,
                priority: RequirementPriority::ShouldHave,
                expected_location: Some("swell-orchestrator::backlog".to_string()),
                expected_symbols: vec!["WorkBacklog".to_string()],
                status: ImplementationStatus::Implemented,
                gap_notes: None,
            },
            SpecRequirement {
                id: "ORCH-010".to_string(),
                description: "TaskBoard with cost tracking and token budget management".to_string(),
                category: RequirementCategory::Orchestration,
                priority: RequirementPriority::MustHave,
                expected_location: Some("swell-orchestrator::task_board".to_string()),
                expected_symbols: vec!["TaskBoard".to_string()],
                status: ImplementationStatus::Implemented,
                gap_notes: None,
            },
            SpecRequirement {
                id: "ORCH-011".to_string(),
                description: "RetryPolicy with model switching and escalation after threshold".to_string(),
                category: RequirementCategory::Orchestration,
                priority: RequirementPriority::MustHave,
                expected_location: Some("swell-orchestrator::retry_policy".to_string()),
                expected_symbols: vec!["RetryPolicy".to_string()],
                status: ImplementationStatus::Implemented,
                gap_notes: None,
            },
            SpecRequirement {
                id: "ORCH-012".to_string(),
                description: "FollowUpGenerator for task follow-up proposals".to_string(),
                category: RequirementCategory::Orchestration,
                priority: RequirementPriority::ShouldHave,
                expected_location: Some("swell-orchestrator::followup_generator".to_string()),
                expected_symbols: vec!["FollowUpGenerator".to_string()],
                status: ImplementationStatus::Implemented,
                gap_notes: None,
            },
            SpecRequirement {
                id: "ORCH-013".to_string(),
                description: "GapAnalyzer for comparing spec to implementation".to_string(),
                category: RequirementCategory::Orchestration,
                priority: RequirementPriority::MustHave,
                expected_location: Some("swell-orchestrator::gap_analyzer".to_string()),
                expected_symbols: vec!["GapAnalyzer".to_string()],
                status: ImplementationStatus::Implemented,
                gap_notes: None,
            },

            // ============================================
            // AGENTS REQUIREMENTS
            // ============================================
            SpecRequirement {
                id: "AGENT-001".to_string(),
                description: "PlannerAgent with LLM calls to generate structured plans".to_string(),
                category: RequirementCategory::Agents,
                priority: RequirementPriority::MustHave,
                expected_location: Some("swell-orchestrator::agents".to_string()),
                expected_symbols: vec!["PlannerAgent".to_string()],
                status: ImplementationStatus::Implemented,
                gap_notes: None,
            },
            SpecRequirement {
                id: "AGENT-002".to_string(),
                description: "GeneratorAgent implementing ReAct loop".to_string(),
                category: RequirementCategory::Agents,
                priority: RequirementPriority::MustHave,
                expected_location: Some("swell-orchestrator::agents".to_string()),
                expected_symbols: vec!["GeneratorAgent".to_string()],
                status: ImplementationStatus::Implemented,
                gap_notes: None,
            },
            SpecRequirement {
                id: "AGENT-003".to_string(),
                description: "EvaluatorAgent validating generated code with confidence score".to_string(),
                category: RequirementCategory::Agents,
                priority: RequirementPriority::MustHave,
                expected_location: Some("swell-orchestrator::agents".to_string()),
                expected_symbols: vec!["EvaluatorAgent".to_string()],
                status: ImplementationStatus::Implemented,
                gap_notes: None,
            },
            SpecRequirement {
                id: "AGENT-004".to_string(),
                description: "CoderAgent for implementing code changes with diff-based modifications".to_string(),
                category: RequirementCategory::Agents,
                priority: RequirementPriority::ShouldHave,
                expected_location: Some("swell-orchestrator::agents".to_string()),
                expected_symbols: vec!["CoderAgent".to_string()],
                status: ImplementationStatus::Implemented,
                gap_notes: None,
            },
            SpecRequirement {
                id: "AGENT-005".to_string(),
                description: "TestWriterAgent generating tests from acceptance criteria".to_string(),
                category: RequirementCategory::Agents,
                priority: RequirementPriority::ShouldHave,
                expected_location: Some("swell-orchestrator::agents".to_string()),
                expected_symbols: vec!["TestWriterAgent".to_string()],
                status: ImplementationStatus::Implemented,
                gap_notes: None,
            },
            SpecRequirement {
                id: "AGENT-006".to_string(),
                description: "ReviewerAgent for semantic code review".to_string(),
                category: RequirementCategory::Agents,
                priority: RequirementPriority::ShouldHave,
                expected_location: Some("swell-orchestrator::agents".to_string()),
                expected_symbols: vec!["ReviewerAgent".to_string()],
                status: ImplementationStatus::Implemented,
                gap_notes: None,
            },
            SpecRequirement {
                id: "AGENT-007".to_string(),
                description: "RefactorerAgent for code restructuring with behavior preservation".to_string(),
                category: RequirementCategory::Agents,
                priority: RequirementPriority::ShouldHave,
                expected_location: Some("swell-orchestrator::agents".to_string()),
                expected_symbols: vec!["RefactorerAgent".to_string()],
                status: ImplementationStatus::Implemented,
                gap_notes: None,
            },
            SpecRequirement {
                id: "AGENT-008".to_string(),
                description: "DocWriterAgent for documentation generation".to_string(),
                category: RequirementCategory::Agents,
                priority: RequirementPriority::ShouldHave,
                expected_location: Some("swell-orchestrator::agents".to_string()),
                expected_symbols: vec!["DocWriterAgent".to_string()],
                status: ImplementationStatus::Implemented,
                gap_notes: None,
            },
            SpecRequirement {
                id: "AGENT-009".to_string(),
                description: "SystemPromptBuilder assembling agent context from project config".to_string(),
                category: RequirementCategory::Agents,
                priority: RequirementPriority::MustHave,
                expected_location: Some("swell-orchestrator::agents".to_string()),
                expected_symbols: vec!["SystemPromptBuilder".to_string()],
                status: ImplementationStatus::Implemented,
                gap_notes: None,
            },
            SpecRequirement {
                id: "AGENT-010".to_string(),
                description: "ReAct loop implementation (Think→Act→Observe→Repeat)".to_string(),
                category: RequirementCategory::Agents,
                priority: RequirementPriority::MustHave,
                expected_location: Some("swell-orchestrator::agents".to_string()),
                expected_symbols: vec!["ReactLoop".to_string()],
                status: ImplementationStatus::Implemented,
                gap_notes: None,
            },
            SpecRequirement {
                id: "AGENT-011".to_string(),
                description: "ContextCondensation at 75% window utilization".to_string(),
                category: RequirementCategory::Agents,
                priority: RequirementPriority::MustHave,
                expected_location: Some("swell-orchestrator::agents".to_string()),
                expected_symbols: vec!["ContextCondensation".to_string()],
                status: ImplementationStatus::Implemented,
                gap_notes: None,
            },

            // ============================================
            // SAFETY REQUIREMENTS
            // ============================================
            SpecRequirement {
                id: "SAFETY-001".to_string(),
                description: "Doom-loop detection with max iterations threshold".to_string(),
                category: RequirementCategory::Safety,
                priority: RequirementPriority::MustHave,
                expected_location: Some("swell-orchestrator::alerts".to_string()),
                expected_symbols: vec!["LoopDetectionState".to_string(), "AlertManager".to_string()],
                status: ImplementationStatus::Implemented,
                gap_notes: None,
            },
            SpecRequirement {
                id: "SAFETY-002".to_string(),
                description: "CostGuard budget tracking with warning at 75% and stop at 100%".to_string(),
                category: RequirementCategory::Safety,
                priority: RequirementPriority::MustHave,
                expected_location: Some("swell-orchestrator::task_board".to_string()),
                expected_symbols: vec!["CostModel".to_string()],
                status: ImplementationStatus::Implemented,
                gap_notes: None,
            },
            SpecRequirement {
                id: "SAFETY-003".to_string(),
                description: "Three-tier permission model (auto-approve, log, confirm)".to_string(),
                category: RequirementCategory::Safety,
                priority: RequirementPriority::MustHave,
                expected_location: Some("swell-orchestrator::policy".to_string()),
                expected_symbols: vec!["PolicyEngine".to_string(), "PolicyAction".to_string()],
                status: ImplementationStatus::Implemented,
                gap_notes: None,
            },
            SpecRequirement {
                id: "SAFETY-004".to_string(),
                description: "Kill switch for immediate task freezing".to_string(),
                category: RequirementCategory::Safety,
                priority: RequirementPriority::MustHave,
                expected_location: Some("swell-orchestrator::state_machine".to_string()),
                expected_symbols: vec!["TaskStateMachine::pause_task".to_string()],
                status: ImplementationStatus::Implemented,
                gap_notes: None,
            },
            SpecRequirement {
                id: "SAFETY-005".to_string(),
                description: "AutonomyController with graduated approval levels (Pair/Sprint/Autonomous)".to_string(),
                category: RequirementCategory::Safety,
                priority: RequirementPriority::MustHave,
                expected_location: Some("swell-orchestrator::autonomy".to_string()),
                expected_symbols: vec!["AutonomyController".to_string()],
                status: ImplementationStatus::Implemented,
                gap_notes: None,
            },
            SpecRequirement {
                id: "SAFETY-006".to_string(),
                description: "Audit logging for all agent actions with timestamps".to_string(),
                category: RequirementCategory::Safety,
                priority: RequirementPriority::ShouldHave,
                expected_location: Some("swell-orchestrator".to_string()),
                expected_symbols: vec!["OrchestratorEvent".to_string()],
                status: ImplementationStatus::Implemented,
                gap_notes: None,
            },

            // ============================================
            // VALIDATION REQUIREMENTS
            // ============================================
            SpecRequirement {
                id: "VALID-001".to_string(),
                description: "LintGate for configurable lint/format checks (see .swell/validation.json)".to_string(),
                category: RequirementCategory::Validation,
                priority: RequirementPriority::MustHave,
                expected_location: Some("swell-validation".to_string()),
                expected_symbols: vec!["LintGate".to_string()],
                status: ImplementationStatus::Implemented,
                gap_notes: None,
            },
            SpecRequirement {
                id: "VALID-002".to_string(),
                description: "TestGate for configurable test runner (see .swell/validation.json)".to_string(),
                category: RequirementCategory::Validation,
                priority: RequirementPriority::MustHave,
                expected_location: Some("swell-validation".to_string()),
                expected_symbols: vec!["TestGate".to_string()],
                status: ImplementationStatus::Implemented,
                gap_notes: None,
            },
            SpecRequirement {
                id: "VALID-003".to_string(),
                description: "SecurityGate for SAST scanning (stub for MVP)".to_string(),
                category: RequirementCategory::Validation,
                priority: RequirementPriority::ShouldHave,
                expected_location: Some("swell-validation".to_string()),
                expected_symbols: vec!["SecurityGate".to_string()],
                status: ImplementationStatus::Implemented,
                gap_notes: None,
            },
            SpecRequirement {
                id: "VALID-004".to_string(),
                description: "ValidationPipeline for running multiple gates in order".to_string(),
                category: RequirementCategory::Validation,
                priority: RequirementPriority::MustHave,
                expected_location: Some("swell-validation".to_string()),
                expected_symbols: vec!["ValidationPipeline".to_string()],
                status: ImplementationStatus::Implemented,
                gap_notes: None,
            },
            SpecRequirement {
                id: "VALID-005".to_string(),
                description: "ValidationContext with task_id, workspace_path, changed_files".to_string(),
                category: RequirementCategory::Validation,
                priority: RequirementPriority::MustHave,
                expected_location: Some("swell-validation".to_string()),
                expected_symbols: vec!["ValidationContext".to_string()],
                status: ImplementationStatus::Implemented,
                gap_notes: None,
            },

            // ============================================
            // MEMORY REQUIREMENTS (Phase 2+)
            // ============================================
            SpecRequirement {
                id: "MEM-001".to_string(),
                description: "Memory blocks (project, user, task context)".to_string(),
                category: RequirementCategory::Memory,
                priority: RequirementPriority::Future,
                expected_location: Some("swell-memory".to_string()),
                expected_symbols: vec!["MemoryBlock".to_string()],
                status: ImplementationStatus::OutOfScope,
                gap_notes: Some("Memory blocks are in swell-memory but not fully implemented".to_string()),
            },
            SpecRequirement {
                id: "MEM-002".to_string(),
                description: "SqliteMemoryStore for persistent memory".to_string(),
                category: RequirementCategory::Memory,
                priority: RequirementPriority::Future,
                expected_location: Some("swell-memory".to_string()),
                expected_symbols: vec!["SqliteMemoryStore".to_string()],
                status: ImplementationStatus::OutOfScope,
                gap_notes: Some("MVP uses in-memory store".to_string()),
            },
            SpecRequirement {
                id: "MEM-003".to_string(),
                description: "Vector search with code embeddings".to_string(),
                category: RequirementCategory::Memory,
                priority: RequirementPriority::Future,
                expected_location: Some("swell-memory".to_string()),
                expected_symbols: vec!["VectorStore".to_string()],
                status: ImplementationStatus::OutOfScope,
                gap_notes: Some("Vector search deferred to V2".to_string()),
            },
            SpecRequirement {
                id: "MEM-004".to_string(),
                description: "Knowledge graph for code structure".to_string(),
                category: RequirementCategory::Memory,
                priority: RequirementPriority::Future,
                expected_location: Some("swell-memory".to_string()),
                expected_symbols: vec!["KnowledgeGraph".to_string()],
                status: ImplementationStatus::OutOfScope,
                gap_notes: Some("KG deferred to V2".to_string()),
            },

            // ============================================
            // TOOLS REQUIREMENTS
            // ============================================
            SpecRequirement {
                id: "TOOL-001".to_string(),
                description: "FileReadTool for reading file contents".to_string(),
                category: RequirementCategory::Tools,
                priority: RequirementPriority::MustHave,
                expected_location: Some("swell-tools".to_string()),
                expected_symbols: vec!["FileReadTool".to_string()],
                status: ImplementationStatus::Implemented,
                gap_notes: None,
            },
            SpecRequirement {
                id: "TOOL-002".to_string(),
                description: "FileWriteTool for writing file contents".to_string(),
                category: RequirementCategory::Tools,
                priority: RequirementPriority::MustHave,
                expected_location: Some("swell-tools".to_string()),
                expected_symbols: vec!["FileWriteTool".to_string()],
                status: ImplementationStatus::Implemented,
                gap_notes: None,
            },
            SpecRequirement {
                id: "TOOL-003".to_string(),
                description: "ShellTool for command execution".to_string(),
                category: RequirementCategory::Tools,
                priority: RequirementPriority::MustHave,
                expected_location: Some("swell-tools".to_string()),
                expected_symbols: vec!["ShellTool".to_string()],
                status: ImplementationStatus::Implemented,
                gap_notes: None,
            },
            SpecRequirement {
                id: "TOOL-004".to_string(),
                description: "GitTool for git operations (status, diff, commit, branch)".to_string(),
                category: RequirementCategory::Tools,
                priority: RequirementPriority::MustHave,
                expected_location: Some("swell-tools".to_string()),
                expected_symbols: vec!["GitTool".to_string()],
                status: ImplementationStatus::Implemented,
                gap_notes: None,
            },
            SpecRequirement {
                id: "TOOL-005".to_string(),
                description: "ToolRegistry for tool registration and retrieval".to_string(),
                category: RequirementCategory::Tools,
                priority: RequirementPriority::MustHave,
                expected_location: Some("swell-tools".to_string()),
                expected_symbols: vec!["ToolRegistry".to_string()],
                status: ImplementationStatus::Implemented,
                gap_notes: None,
            },
            SpecRequirement {
                id: "TOOL-006".to_string(),
                description: "MCP client for external tool integration".to_string(),
                category: RequirementCategory::Tools,
                priority: RequirementPriority::Future,
                expected_location: Some("swell-tools".to_string()),
                expected_symbols: vec!["McpClient".to_string()],
                status: ImplementationStatus::Implemented,
                gap_notes: Some("MCP client stub exists".to_string()),
            },

            // ============================================
            // GIT WORKFLOW REQUIREMENTS
            // ============================================
            SpecRequirement {
                id: "GIT-001".to_string(),
                description: "One branch per task enforcement".to_string(),
                category: RequirementCategory::GitWorkflow,
                priority: RequirementPriority::MustHave,
                expected_location: Some("swell-tools::git".to_string()),
                expected_symbols: vec!["git_branch".to_string()],
                status: ImplementationStatus::Implemented,
                gap_notes: None,
            },
            SpecRequirement {
                id: "GIT-002".to_string(),
                description: "Atomic commits with provenance metadata".to_string(),
                category: RequirementCategory::GitWorkflow,
                priority: RequirementPriority::ShouldHave,
                expected_location: Some("swell-tools::git".to_string()),
                expected_symbols: vec!["git_commit".to_string()],
                status: ImplementationStatus::Implemented,
                gap_notes: None,
            },
            SpecRequirement {
                id: "GIT-003".to_string(),
                description: "Feature branch PR creation".to_string(),
                category: RequirementCategory::GitWorkflow,
                priority: RequirementPriority::MustHave,
                expected_location: Some("swell-tools".to_string()),
                expected_symbols: vec!["GitTool".to_string()],
                status: ImplementationStatus::Implemented,
                gap_notes: None,
            },

            // ============================================
            // CLIENT REQUIREMENTS
            // ============================================
            SpecRequirement {
                id: "CLIENT-001".to_string(),
                description: "CLI client for task creation".to_string(),
                category: RequirementCategory::Client,
                priority: RequirementPriority::MustHave,
                expected_location: Some("swell-cli".to_string()),
                expected_symbols: vec!["main".to_string()],
                status: ImplementationStatus::Implemented,
                gap_notes: None,
            },
            SpecRequirement {
                id: "CLIENT-002".to_string(),
                description: "CLI commands: task, list, watch, approve, cancel".to_string(),
                category: RequirementCategory::Client,
                priority: RequirementPriority::MustHave,
                expected_location: Some("swell-cli".to_string()),
                expected_symbols: vec!["task".to_string(), "list".to_string(), "watch".to_string()],
                status: ImplementationStatus::Implemented,
                gap_notes: None,
            },
            SpecRequirement {
                id: "CLIENT-003".to_string(),
                description: "Daemon server with Unix socket communication".to_string(),
                category: RequirementCategory::Client,
                priority: RequirementPriority::MustHave,
                expected_location: Some("swell-daemon".to_string()),
                expected_symbols: vec!["serve".to_string()],
                status: ImplementationStatus::Implemented,
                gap_notes: None,
            },

            // ============================================
            // OBSERVABILITY REQUIREMENTS
            // ============================================
            SpecRequirement {
                id: "OBS-001".to_string(),
                description: "MetricsCollector for task completion rate, PR acceptance rate".to_string(),
                category: RequirementCategory::Observability,
                priority: RequirementPriority::MustHave,
                expected_location: Some("swell-orchestrator::metrics".to_string()),
                expected_symbols: vec!["MetricsCollector".to_string()],
                status: ImplementationStatus::Implemented,
                gap_notes: None,
            },
            SpecRequirement {
                id: "OBS-002".to_string(),
                description: "OrchestratorMetrics with aggregated statistics".to_string(),
                category: RequirementCategory::Observability,
                priority: RequirementPriority::MustHave,
                expected_location: Some("swell-orchestrator::metrics".to_string()),
                expected_symbols: vec!["OrchestratorMetrics".to_string()],
                status: ImplementationStatus::Implemented,
                gap_notes: None,
            },
            SpecRequirement {
                id: "OBS-003".to_string(),
                description: "AlertManager for system alerts".to_string(),
                category: RequirementCategory::Observability,
                priority: RequirementPriority::MustHave,
                expected_location: Some("swell-orchestrator::alerts".to_string()),
                expected_symbols: vec!["AlertManager".to_string()],
                status: ImplementationStatus::Implemented,
                gap_notes: None,
            },
            SpecRequirement {
                id: "OBS-004".to_string(),
                description: "OpenTelemetry tracing with GenAI semantic conventions".to_string(),
                category: RequirementCategory::Observability,
                priority: RequirementPriority::Future,
                expected_location: Some("swell-core".to_string()),
                expected_symbols: vec!["Trace".to_string()],
                status: ImplementationStatus::OutOfScope,
                gap_notes: Some("OTel integration deferred to observability milestone".to_string()),
            },
        ]
    }

    /// Analyze gaps between specification and implementation
    pub fn analyze(&self) -> GapAnalysisReport {
        let requirements = Self::get_spec_requirements();

        // Filter based on config
        let filtered: Vec<SpecRequirement> = if self.config.include_out_of_scope {
            requirements
        } else {
            requirements
                .into_iter()
                .filter(|r| r.status != ImplementationStatus::OutOfScope)
                .collect()
        };

        // Group by category
        let mut by_category: HashMap<RequirementCategory, Vec<SpecRequirement>> = HashMap::new();
        for req in &filtered {
            by_category
                .entry(req.category.clone())
                .or_default()
                .push(req.clone());
        }

        // Calculate category reports
        let category_reports: Vec<CategoryGapReport> = by_category
            .into_iter()
            .map(|(category, reqs)| {
                let total = reqs.len();
                let implemented = reqs
                    .iter()
                    .filter(|r| r.status == ImplementationStatus::Implemented)
                    .count();
                let partial = reqs
                    .iter()
                    .filter(|r| r.status == ImplementationStatus::Partial)
                    .count();
                let missing = reqs
                    .iter()
                    .filter(|r| r.status == ImplementationStatus::Missing)
                    .count();
                let out_of_scope = reqs
                    .iter()
                    .filter(|r| r.status == ImplementationStatus::OutOfScope)
                    .count();
                let coverage = if total > 0 {
                    (implemented as f64 / total as f64) * 100.0
                } else {
                    100.0
                };

                CategoryGapReport {
                    category,
                    total_requirements: total,
                    implemented,
                    partial,
                    missing,
                    out_of_scope,
                    coverage_percentage: coverage,
                    requirements: reqs,
                }
            })
            .collect();

        // Calculate totals
        let total_requirements = filtered.len();
        let total_implemented = filtered
            .iter()
            .filter(|r| r.status == ImplementationStatus::Implemented)
            .count();
        let total_partial = filtered
            .iter()
            .filter(|r| r.status == ImplementationStatus::Partial)
            .count();
        let total_missing = filtered
            .iter()
            .filter(|r| r.status == ImplementationStatus::Missing)
            .count();
        let total_out_of_scope = filtered
            .iter()
            .filter(|r| r.status == ImplementationStatus::OutOfScope)
            .count();
        let overall_coverage = if total_requirements > 0 {
            (total_implemented as f64 / total_requirements as f64) * 100.0
        } else {
            100.0
        };

        // Find critical gaps (MustHave that are Missing)
        let critical_gaps: Vec<String> = filtered
            .iter()
            .filter(|r| {
                r.priority == RequirementPriority::MustHave
                    && r.status == ImplementationStatus::Missing
            })
            .map(|r| format!("{}: {}", r.id, r.description))
            .collect();

        // Generate recommendations
        let recommendations = self.generate_recommendations(&filtered);

        GapAnalysisReport {
            overall_coverage_percentage: overall_coverage,
            total_requirements,
            total_implemented,
            total_partial,
            total_missing,
            total_out_of_scope,
            category_reports,
            critical_gaps,
            recommendations,
        }
    }

    /// Generate recommendations based on gaps
    fn generate_recommendations(&self, requirements: &[SpecRequirement]) -> Vec<String> {
        let mut recommendations = Vec::new();

        // Check for missing MustHave requirements
        let missing_must_have: Vec<_> = requirements
            .iter()
            .filter(|r| {
                r.priority == RequirementPriority::MustHave
                    && r.status == ImplementationStatus::Missing
            })
            .collect();

        if !missing_must_have.is_empty() {
            recommendations.push(format!(
                "Address {} missing MustHave requirements before MVP completion",
                missing_must_have.len()
            ));
        }

        // Check for partial implementations
        let partial_count = requirements
            .iter()
            .filter(|r| r.status == ImplementationStatus::Partial)
            .count();

        if partial_count > 0 {
            recommendations.push(format!(
                "Complete {} partially implemented requirements",
                partial_count
            ));
        }

        // Check category coverage
        let category_coverage: HashMap<RequirementCategory, f64> = requirements
            .iter()
            .fold(HashMap::new(), |mut acc, r| {
                let entry = acc.entry(r.category.clone()).or_insert((0.0, 0.0));
                if r.status == ImplementationStatus::Implemented {
                    entry.0 += 1.0;
                }
                entry.1 += 1.0;
                acc
            })
            .into_iter()
            .map(|(k, (impld, total))| {
                let coverage = if total > 0.0 {
                    (impld / total) * 100.0
                } else {
                    100.0
                };
                (k, coverage)
            })
            .collect();

        // Flag categories with < 70% coverage
        for (category, coverage) in &category_coverage {
            if *coverage < 70.0 {
                recommendations.push(format!(
                    "Improve {:?} category coverage from {:.1}%",
                    category, coverage
                ));
            }
        }

        if recommendations.is_empty() {
            recommendations
                .push("All MVP requirements are implemented or appropriately deferred".to_string());
        }

        recommendations
    }

    /// Get requirements by category
    pub fn get_requirements_by_category(category: RequirementCategory) -> Vec<SpecRequirement> {
        Self::get_spec_requirements()
            .into_iter()
            .filter(|r| r.category == category)
            .collect()
    }

    /// Get only missing requirements
    pub fn get_missing_requirements() -> Vec<SpecRequirement> {
        Self::get_spec_requirements()
            .into_iter()
            .filter(|r| r.status == ImplementationStatus::Missing)
            .collect()
    }

    /// Get only implemented requirements
    pub fn get_implemented_requirements() -> Vec<SpecRequirement> {
        Self::get_spec_requirements()
            .into_iter()
            .filter(|r| r.status == ImplementationStatus::Implemented)
            .collect()
    }
}

impl Default for GapAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gap_analyzer_new() {
        let analyzer = GapAnalyzer::new();
        let report = analyzer.analyze();

        // Overall coverage should be high since most requirements are implemented
        assert!(report.overall_coverage_percentage >= 80.0);
        // Out of scope items are included by default
        assert!(report.total_out_of_scope > 0);
    }

    #[test]
    fn test_gap_analyzer_exclude_out_of_scope() {
        let config = GapAnalyzerConfig {
            include_out_of_scope: false,
            ..Default::default()
        };
        let analyzer = GapAnalyzer::with_config(config);
        let report = analyzer.analyze();

        // Out of scope should be 0
        assert_eq!(report.total_out_of_scope, 0);
    }

    #[test]
    fn test_orchestration_coverage() {
        let requirements =
            GapAnalyzer::get_requirements_by_category(RequirementCategory::Orchestration);

        // Orchestration should have multiple requirements
        assert!(requirements.len() >= 10);

        // All orchestration requirements should be implemented
        for req in &requirements {
            assert_ne!(
                req.status,
                ImplementationStatus::Missing,
                "Missing orchestration requirement: {}",
                req.id
            );
        }
    }

    #[test]
    fn test_agents_coverage() {
        let requirements = GapAnalyzer::get_requirements_by_category(RequirementCategory::Agents);

        // Should have all agent types
        assert!(requirements.len() >= 8);

        for req in &requirements {
            assert_ne!(
                req.status,
                ImplementationStatus::Missing,
                "Missing agent requirement: {}",
                req.id
            );
        }
    }

    #[test]
    fn test_safety_coverage() {
        let requirements = GapAnalyzer::get_requirements_by_category(RequirementCategory::Safety);

        // Should have all safety requirements
        assert!(requirements.len() >= 5);

        for req in &requirements {
            assert_ne!(
                req.status,
                ImplementationStatus::Missing,
                "Missing safety requirement: {}",
                req.id
            );
        }
    }

    #[test]
    fn test_critical_gaps() {
        let analyzer = GapAnalyzer::new();
        let report = analyzer.analyze();

        // Critical gaps should be empty since all MustHave are implemented
        assert!(
            report.critical_gaps.is_empty(),
            "Unexpected critical gaps: {:?}",
            report.critical_gaps
        );
    }

    #[test]
    fn test_recommendations() {
        let analyzer = GapAnalyzer::new();
        let report = analyzer.analyze();

        // Should have at least one recommendation
        assert!(!report.recommendations.is_empty());

        // Should mention if there are partial implementations
        if report.total_partial > 0 {
            assert!(
                report.recommendations.iter().any(|r| r.contains("partial")),
                "Should mention partial implementations"
            );
        }
    }

    #[test]
    fn test_get_implemented_requirements() {
        let implemented = GapAnalyzer::get_implemented_requirements();

        // Should have many implemented requirements
        assert!(implemented.len() >= 30);

        // All should be Implemented status
        for req in &implemented {
            assert_eq!(req.status, ImplementationStatus::Implemented);
        }
    }

    #[test]
    fn test_get_missing_requirements() {
        let missing = GapAnalyzer::get_missing_requirements();

        // Missing should be empty or minimal
        assert_eq!(
            missing.len(),
            0,
            "Found missing requirements: {:?}",
            missing
        );
    }

    #[test]
    fn test_spec_requirement_serialization() {
        let req = SpecRequirement {
            id: "TEST-001".to_string(),
            description: "Test requirement".to_string(),
            category: RequirementCategory::Orchestration,
            priority: RequirementPriority::MustHave,
            expected_location: Some("test::module".to_string()),
            expected_symbols: vec!["TestStruct".to_string()],
            status: ImplementationStatus::Implemented,
            gap_notes: None,
        };

        let json = serde_json::to_string(&req).unwrap();
        let deserialized: SpecRequirement = serde_json::from_str(&json).unwrap();

        assert_eq!(req.id, deserialized.id);
        assert_eq!(req.category, deserialized.category);
        assert_eq!(req.status, deserialized.status);
    }

    #[test]
    fn test_gap_analysis_report_serialization() {
        let analyzer = GapAnalyzer::new();
        let report = analyzer.analyze();

        let json = serde_json::to_string(&report).unwrap();
        let deserialized: GapAnalysisReport = serde_json::from_str(&json).unwrap();

        assert_eq!(report.total_requirements, deserialized.total_requirements);
        assert_eq!(report.total_implemented, deserialized.total_implemented);
    }

    #[test]
    fn test_category_enum_derives() {
        let cat = RequirementCategory::Orchestration;
        assert_eq!(cat, RequirementCategory::Orchestration);

        // Test equality
        assert_eq!(RequirementCategory::Agents, RequirementCategory::Agents);
        assert_ne!(RequirementCategory::Agents, RequirementCategory::Safety);
    }

    #[test]
    fn test_priority_ordering() {
        assert!(RequirementPriority::MustHave < RequirementPriority::ShouldHave);
        assert!(RequirementPriority::ShouldHave < RequirementPriority::NiceToHave);
        assert!(RequirementPriority::NiceToHave < RequirementPriority::Future);
    }

    #[test]
    fn test_category_gap_report_calculation() {
        let requirements = vec![
            SpecRequirement {
                id: "TEST-001".to_string(),
                description: "Test 1".to_string(),
                category: RequirementCategory::Tools,
                priority: RequirementPriority::MustHave,
                expected_location: None,
                expected_symbols: vec![],
                status: ImplementationStatus::Implemented,
                gap_notes: None,
            },
            SpecRequirement {
                id: "TEST-002".to_string(),
                description: "Test 2".to_string(),
                category: RequirementCategory::Tools,
                priority: RequirementPriority::MustHave,
                expected_location: None,
                expected_symbols: vec![],
                status: ImplementationStatus::Implemented,
                gap_notes: None,
            },
            SpecRequirement {
                id: "TEST-003".to_string(),
                description: "Test 3".to_string(),
                category: RequirementCategory::Tools,
                priority: RequirementPriority::ShouldHave,
                expected_location: None,
                expected_symbols: vec![],
                status: ImplementationStatus::Partial,
                gap_notes: None,
            },
        ];

        let report = CategoryGapReport {
            category: RequirementCategory::Tools,
            total_requirements: 3,
            implemented: 2,
            partial: 1,
            missing: 0,
            out_of_scope: 0,
            coverage_percentage: 66.67,
            requirements,
        };

        assert_eq!(report.total_requirements, 3);
        assert_eq!(report.implemented, 2);
        assert_eq!(report.partial, 1);
        assert!((report.coverage_percentage - 66.67).abs() < 0.01);
    }
}
