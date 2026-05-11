//! Wiring diagnostics for orchestrator subsystems.
//!
//! This module provides wiring report implementations for orchestrator-internal
//! types and status reports for Tier-2 subsystems.
//!
//! # Orphan Rule Note
//!
//! Due to Rust's orphan rule, implementations for types defined in other crates
//! (`swell-llm`, `swell-tools`, `swell-state`) must be placed in those crates.
//! Only implementations for types defined in `swell-orchestrator` itself are
//! provided here.

use std::sync::Arc;

use swell_core::wiring::{WiringReport, WiringState};
use tokio::sync::RwLock;

use crate::agents::AgentPool;
use crate::feature_leads::FeatureLeadManager;
use crate::frozen_spec::FrozenRequirementRegistry;
use crate::non_novel_retry::NonNovelRetryDetector;
use crate::novelty_check::NoveltyChecker;
use crate::state_machine::TaskStateMachine;
use crate::FileLockManager;

// Import Orchestrator and ExecutionController for wiring implementations
use crate::{ExecutionController, Orchestrator};

// Import types used in wiring manifest
use swell_llm::LlmBackend;
use swell_state::CheckpointManager;
use swell_tools::mcp_config::McpConfigManager;
use swell_tools::{BranchStrategy, CommitStrategy, WorktreePool};

// ========================================================================
// Orchestrator wiring report
// ========================================================================

impl WiringReport for Orchestrator {
    fn name(&self) -> &'static str {
        "Orchestrator"
    }

    fn identity(&self) -> String {
        format!("Orchestrator@{:p}", self)
    }

    fn state(&self) -> WiringState {
        WiringState::Enabled
    }
}

// ========================================================================
// TaskStateMachine wiring report
// ========================================================================

impl WiringReport for TaskStateMachine {
    fn name(&self) -> &'static str {
        "TaskStateMachine"
    }

    fn identity(&self) -> String {
        format!("TaskStateMachine@{:p}", self)
    }

    fn state(&self) -> WiringState {
        WiringState::Enabled
    }
}

// ========================================================================
// ExecutionController wiring report
// ========================================================================

impl WiringReport for ExecutionController {
    fn name(&self) -> &'static str {
        "ExecutionController"
    }

    fn identity(&self) -> String {
        format!("ExecutionController@{:p}", self)
    }

    fn state(&self) -> WiringState {
        WiringState::Enabled
    }
}

// ========================================================================
// FeatureLeadManager wiring report
// ========================================================================

impl WiringReport for FeatureLeadManager {
    fn name(&self) -> &'static str {
        "FeatureLeadManager"
    }

    fn identity(&self) -> String {
        format!("FeatureLeadManager@{:p}", self)
    }

    fn state(&self) -> WiringState {
        WiringState::Enabled
    }
}

// ========================================================================
// NoveltyChecker wiring report
// ========================================================================

impl WiringReport for NoveltyChecker {
    fn name(&self) -> &'static str {
        "NoveltyChecker"
    }

    fn identity(&self) -> String {
        format!("NoveltyChecker@{:p}", self)
    }

    fn state(&self) -> WiringState {
        WiringState::Enabled
    }
}

// ========================================================================
// FileLockManager wiring report
// ========================================================================

impl WiringReport for FileLockManager {
    fn name(&self) -> &'static str {
        "FileLockManager"
    }

    fn identity(&self) -> String {
        format!("FileLockManager@{:p}", self)
    }

    fn state(&self) -> WiringState {
        WiringState::Enabled
    }
}

// ========================================================================
// FrozenRequirementRegistry wiring report
// ========================================================================

impl WiringReport for FrozenRequirementRegistry {
    fn name(&self) -> &'static str {
        "FrozenRequirementRegistry"
    }

    fn identity(&self) -> String {
        format!("FrozenRequirementRegistry@{:p}", self)
    }

    fn state(&self) -> WiringState {
        WiringState::Enabled
    }
}

// ========================================================================
// NonNovelRetryDetector wiring report
// ========================================================================

impl WiringReport for NonNovelRetryDetector {
    fn name(&self) -> &'static str {
        "NonNovelRetryDetector"
    }

    fn identity(&self) -> String {
        format!("NonNovelRetryDetector@{:p}", self)
    }

    fn state(&self) -> WiringState {
        WiringState::Enabled
    }
}

// ========================================================================
// AgentPool wiring report
// ========================================================================

impl WiringReport for AgentPool {
    fn name(&self) -> &'static str {
        "AgentPool"
    }

    fn identity(&self) -> String {
        format!("AgentPool@{:p}", self)
    }

    fn state(&self) -> WiringState {
        WiringState::Enabled
    }
}

// ========================================================================
// Tier-2 runtime reports.
// CostGuard, kill switch controls, pre-tool command denial, and sandbox
// pre-hook routing are reached through production daemon/execution paths.
// ========================================================================

/// CostGuard runtime report.
///
/// The execution controller records agent token usage on the task and pauses
/// execution when the task reaches its configured token budget.
#[derive(Debug)]
pub struct CostGuard;

impl WiringReport for CostGuard {
    fn name(&self) -> &'static str {
        "CostGuard"
    }

    fn identity(&self) -> String {
        format!("CostGuard@{:p}", self)
    }

    fn state(&self) -> WiringState {
        WiringState::Enabled
    }
}

/// KillSwitch runtime report.
///
/// Daemon socket commands can now trigger/reset/query the `OrchestratorKillSwitch`
/// owned by `ExecutionController`; the turn loop checks it before each turn.
#[derive(Debug)]
pub struct KillSwitch;

impl WiringReport for KillSwitch {
    fn name(&self) -> &'static str {
        "KillSwitch"
    }

    fn identity(&self) -> String {
        "DaemonCommand -> ExecutionController::kill_switch".to_string()
    }

    fn state(&self) -> WiringState {
        WiringState::Enabled
    }
}

/// PreToolHookManager runtime report.
///
/// The production generator path now executes tool calls through ToolExecutor,
/// which installs command-denial hooks and the sandbox pre-hook.
#[derive(Debug)]
pub struct PreToolHookManager;

impl WiringReport for PreToolHookManager {
    fn name(&self) -> &'static str {
        "PreToolHookManager"
    }

    fn identity(&self) -> String {
        format!("PreToolHookManager@{:p}", self)
    }

    fn state(&self) -> WiringState {
        WiringState::Enabled
    }
}

/// `swell-sandbox` router runtime report.
///
/// `ToolExecutor` now links the lower-level `swell-sandbox` crate and consults
/// its backend probes before shell sandbox execution falls back to the
/// process-level OS sandbox.
#[derive(Debug)]
pub struct SwellSandboxRouter;

impl WiringReport for SwellSandboxRouter {
    fn name(&self) -> &'static str {
        "SwellSandboxRouter"
    }

    fn identity(&self) -> String {
        "swell-tools::sandbox_router -> swell-sandbox".to_string()
    }

    fn state(&self) -> WiringState {
        WiringState::Enabled
    }
}

/// LanceDB vector backend runtime report.
///
/// `TripleStreamService` can now carry a `VectorBackend`, and the existing
/// `LanceDbVectorStore` implements that trait for semantic retrieval.
#[derive(Debug)]
pub struct MemoryVectorBackend;

impl WiringReport for MemoryVectorBackend {
    fn name(&self) -> &'static str {
        "MemoryVectorBackend"
    }

    fn identity(&self) -> String {
        "swell-memory::LanceDbVectorStore as VectorBackend".to_string()
    }

    fn state(&self) -> WiringState {
        WiringState::Enabled
    }
}

/// Skill extraction runtime report.
///
/// `ExecutionController` now invokes `SkillExtractionService` after successful
/// validated generation and writes candidate skills under `.swell/skills/_candidates`.
#[derive(Debug)]
pub struct SkillExtraction;

impl WiringReport for SkillExtraction {
    fn name(&self) -> &'static str {
        "SkillExtraction"
    }

    fn identity(&self) -> String {
        "ExecutionController -> swell-memory::SkillExtractionService".to_string()
    }

    fn state(&self) -> WiringState {
        WiringState::Enabled
    }
}

// ========================================================================
// Wiring report wrappers for Arc-wrapped subsystem fields
// These allow wiring_manifest() to return Box<dyn WiringReport> for each field.
// ========================================================================

/// Wrapper for TaskStateMachine (stored behind Arc<RwLock<TaskStateMachine>>)
pub struct TaskStateMachineReport {
    pub(crate) inner: Arc<RwLock<TaskStateMachine>>,
}

impl TaskStateMachineReport {
    /// Create a new TaskStateMachineReport wrapping the given Arc.
    pub fn new(inner: Arc<RwLock<TaskStateMachine>>) -> Self {
        Self { inner }
    }
}

impl WiringReport for TaskStateMachineReport {
    fn name(&self) -> &'static str {
        "TaskStateMachine"
    }

    fn identity(&self) -> String {
        format!("TaskStateMachine@{:p}", Arc::as_ptr(&self.inner))
    }

    fn state(&self) -> WiringState {
        WiringState::Enabled
    }
}

/// Wrapper for AgentPool (stored behind Arc<RwLock<AgentPool>>)
pub struct AgentPoolReport {
    pub(crate) inner: Arc<RwLock<AgentPool>>,
}

impl AgentPoolReport {
    /// Create a new AgentPoolReport wrapping the given Arc.
    pub fn new(inner: Arc<RwLock<AgentPool>>) -> Self {
        Self { inner }
    }
}

impl WiringReport for AgentPoolReport {
    fn name(&self) -> &'static str {
        "AgentPool"
    }

    fn identity(&self) -> String {
        format!("AgentPool@{:p}", Arc::as_ptr(&self.inner))
    }

    fn state(&self) -> WiringState {
        WiringState::Enabled
    }
}

/// Wrapper for CheckpointManager (stored behind Arc<CheckpointManager>)
pub struct CheckpointManagerReport {
    pub(crate) inner: Arc<CheckpointManager>,
}

impl CheckpointManagerReport {
    /// Create a new CheckpointManagerReport wrapping the given Arc.
    pub fn new(inner: Arc<CheckpointManager>) -> Self {
        Self { inner }
    }
}

impl WiringReport for CheckpointManagerReport {
    fn name(&self) -> &'static str {
        "CheckpointManager"
    }

    fn identity(&self) -> String {
        format!("CheckpointManager@{:p}", Arc::as_ptr(&self.inner))
    }

    fn state(&self) -> WiringState {
        WiringState::Enabled
    }
}

/// Wrapper for FeatureLeadManager (stored behind Arc<RwLock<FeatureLeadManager>>)
pub struct FeatureLeadManagerReport {
    pub(crate) inner: Arc<RwLock<FeatureLeadManager>>,
}

impl FeatureLeadManagerReport {
    /// Create a new FeatureLeadManagerReport wrapping the given Arc.
    pub fn new(inner: Arc<RwLock<FeatureLeadManager>>) -> Self {
        Self { inner }
    }
}

impl WiringReport for FeatureLeadManagerReport {
    fn name(&self) -> &'static str {
        "FeatureLeadManager"
    }

    fn identity(&self) -> String {
        format!("FeatureLeadManager@{:p}", Arc::as_ptr(&self.inner))
    }

    fn state(&self) -> WiringState {
        WiringState::Enabled
    }
}

/// Wrapper for McpConfigManager (stored behind Arc<McpConfigManager>)
pub struct McpConfigManagerReport {
    pub(crate) inner: Arc<McpConfigManager>,
}

impl McpConfigManagerReport {
    /// Create a new McpConfigManagerReport wrapping the given Arc.
    pub fn new(inner: Arc<McpConfigManager>) -> Self {
        Self { inner }
    }
}

impl WiringReport for McpConfigManagerReport {
    fn name(&self) -> &'static str {
        "McpConfigManager"
    }

    fn identity(&self) -> String {
        format!("McpConfigManager@{:p}", Arc::as_ptr(&self.inner))
    }

    fn state(&self) -> WiringState {
        WiringState::Enabled
    }
}

/// Wrapper for NoveltyChecker (stored behind Arc<RwLock<NoveltyChecker>>)
pub struct NoveltyCheckerReport {
    pub(crate) inner: Arc<RwLock<NoveltyChecker>>,
}

impl NoveltyCheckerReport {
    /// Create a new NoveltyCheckerReport wrapping the given Arc.
    pub fn new(inner: Arc<RwLock<NoveltyChecker>>) -> Self {
        Self { inner }
    }
}

impl WiringReport for NoveltyCheckerReport {
    fn name(&self) -> &'static str {
        "NoveltyChecker"
    }

    fn identity(&self) -> String {
        format!("NoveltyChecker@{:p}", Arc::as_ptr(&self.inner))
    }

    fn state(&self) -> WiringState {
        WiringState::Enabled
    }
}

/// Wrapper for FileLockManager (stored behind Arc<FileLockManager>)
pub struct FileLockManagerReport {
    pub(crate) inner: Arc<FileLockManager>,
}

impl FileLockManagerReport {
    /// Create a new FileLockManagerReport wrapping the given Arc.
    pub fn new(inner: Arc<FileLockManager>) -> Self {
        Self { inner }
    }
}

impl WiringReport for FileLockManagerReport {
    fn name(&self) -> &'static str {
        "FileLockManager"
    }

    fn identity(&self) -> String {
        format!("FileLockManager@{:p}", Arc::as_ptr(&self.inner))
    }

    fn state(&self) -> WiringState {
        WiringState::Enabled
    }
}

/// Wrapper for NonNovelRetryDetector (stored behind Arc<RwLock<NonNovelRetryDetector>>)
pub struct NonNovelRetryDetectorReport {
    pub(crate) inner: Arc<RwLock<NonNovelRetryDetector>>,
}

impl NonNovelRetryDetectorReport {
    /// Create a new NonNovelRetryDetectorReport wrapping the given Arc.
    pub fn new(inner: Arc<RwLock<NonNovelRetryDetector>>) -> Self {
        Self { inner }
    }
}

impl WiringReport for NonNovelRetryDetectorReport {
    fn name(&self) -> &'static str {
        "NonNovelRetryDetector"
    }

    fn identity(&self) -> String {
        format!("NonNovelRetryDetector@{:p}", Arc::as_ptr(&self.inner))
    }

    fn state(&self) -> WiringState {
        WiringState::Enabled
    }
}

/// Wrapper for FrozenRequirementRegistry (stored as plain FrozenRequirementRegistry)
pub struct FrozenRequirementRegistryReport {
    pub(crate) inner: FrozenRequirementRegistry,
}

impl FrozenRequirementRegistryReport {
    /// Create a new FrozenRequirementRegistryReport wrapping the given registry.
    pub fn new(inner: FrozenRequirementRegistry) -> Self {
        Self { inner }
    }
}

impl WiringReport for FrozenRequirementRegistryReport {
    fn name(&self) -> &'static str {
        "FrozenRequirementRegistry"
    }

    fn identity(&self) -> String {
        format!("FrozenRequirementRegistry@{:p}", &self.inner as *const _)
    }

    fn state(&self) -> WiringState {
        WiringState::Enabled
    }
}

/// Wrapper for LLM backend (Arc<dyn LlmBackend>)
pub struct LlmBackendReport {
    pub(crate) inner: Arc<dyn LlmBackend>,
}

impl LlmBackendReport {
    /// Create a new LlmBackendReport wrapping the given Arc.
    pub fn new(inner: Arc<dyn LlmBackend>) -> Self {
        Self { inner }
    }
}

impl WiringReport for LlmBackendReport {
    fn name(&self) -> &'static str {
        "LlmBackend"
    }

    fn identity(&self) -> String {
        // Include the model name in the identity for disambiguation
        format!("{}@{:p}", self.inner.model(), Arc::as_ptr(&self.inner))
    }

    fn state(&self) -> WiringState {
        WiringState::Enabled
    }
}

/// Wrapper for ExecutionController (Arc<ExecutionController>)
pub struct ExecutionControllerReport {
    pub(crate) inner: Arc<ExecutionController>,
}

impl ExecutionControllerReport {
    /// Create a new ExecutionControllerReport wrapping the given Arc.
    pub fn new(inner: Arc<ExecutionController>) -> Self {
        Self { inner }
    }
}

impl WiringReport for ExecutionControllerReport {
    fn name(&self) -> &'static str {
        "ExecutionController"
    }

    fn identity(&self) -> String {
        format!("ExecutionController@{:p}", Arc::as_ptr(&self.inner))
    }

    fn state(&self) -> WiringState {
        WiringState::Enabled
    }
}

/// Wrapper for WorktreePool (Arc<WorktreePool>)
pub struct WorktreePoolReport {
    pub(crate) inner: Arc<WorktreePool>,
}

impl WorktreePoolReport {
    /// Create a new WorktreePoolReport wrapping the given Arc.
    pub fn new(inner: Arc<WorktreePool>) -> Self {
        Self { inner }
    }
}

impl WiringReport for WorktreePoolReport {
    fn name(&self) -> &'static str {
        "WorktreePool"
    }

    fn identity(&self) -> String {
        format!(
            "WorktreePool@{:p}:{}",
            Arc::as_ptr(&self.inner),
            self.inner.config().worktree_dir.display()
        )
    }

    fn state(&self) -> WiringState {
        WiringState::Enabled
    }
}

/// Wrapper for BranchStrategy (Arc<BranchStrategy>)
pub struct BranchStrategyReport {
    pub(crate) inner: Arc<BranchStrategy>,
}

impl BranchStrategyReport {
    /// Create a new BranchStrategyReport wrapping the given Arc.
    pub fn new(inner: Arc<BranchStrategy>) -> Self {
        Self { inner }
    }
}

impl WiringReport for BranchStrategyReport {
    fn name(&self) -> &'static str {
        "BranchStrategy"
    }

    fn identity(&self) -> String {
        format!(
            "BranchStrategy@{:p}:{}",
            Arc::as_ptr(&self.inner),
            self.inner.config().branch_prefix
        )
    }

    fn state(&self) -> WiringState {
        WiringState::Enabled
    }
}

/// Wrapper for CommitStrategy (Arc<CommitStrategy>)
pub struct CommitStrategyReport {
    pub(crate) inner: Arc<CommitStrategy>,
}

impl CommitStrategyReport {
    /// Create a new CommitStrategyReport wrapping the given Arc.
    pub fn new(inner: Arc<CommitStrategy>) -> Self {
        Self { inner }
    }
}

impl WiringReport for CommitStrategyReport {
    fn name(&self) -> &'static str {
        "CommitStrategy"
    }

    fn identity(&self) -> String {
        format!(
            "CommitStrategy@{:p}:{}",
            Arc::as_ptr(&self.inner),
            self.inner.generator_id()
        )
    }

    fn state(&self) -> WiringState {
        WiringState::Enabled
    }
}
