//! Wiring diagnostics for orchestrator subsystems.
//!
//! This module provides wiring report implementations for orchestrator-internal
//! types and stubs for Tier-2 subsystems that are not yet wired.
//!
//! # Orphan Rule Note
//!
//! Due to Rust's orphan rule, implementations for types defined in other crates
//! (`swell-llm`, `swell-tools`, `swell-state`) must be placed in those crates.
//! Only implementations for types defined in `swell-orchestrator` itself are
//! provided here.

use std::sync::Arc;

use tokio::sync::RwLock;
use swell_core::wiring::{WiringReport, WiringState};

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
// Tier-2 stubs - CostGuard, PreToolHookManager
// These are not yet wired in production, so they return Disabled.
// ========================================================================

/// CostGuard stub - Tier 2.1 not wired.
///
/// This is a placeholder for the cost guard subsystem that should
/// enforce budget limits during task execution. It is not yet
/// implemented in production.
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
        WiringState::Disabled("Tier 2.1 not wired".to_string())
    }
}

/// PreToolHookManager stub - Tier 2.2 not wired.
///
/// This is a placeholder for the pre-tool hook manager that should
/// allow custom logic to run before tool execution. It is not yet
/// implemented in production.
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
        WiringState::Disabled("Tier 2.2 not wired".to_string())
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
