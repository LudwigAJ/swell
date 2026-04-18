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

use swell_core::wiring::{WiringReport, WiringState};

use crate::agents::AgentPool;
use crate::feature_leads::FeatureLeadManager;
use crate::frozen_spec::FrozenRequirementRegistry;
use crate::non_novel_retry::NonNovelRetryDetector;
use crate::novelty_check::NoveltyChecker;
use crate::FileLockManager;

// Import Orchestrator and ExecutionController for wiring implementations
use crate::{ExecutionController, Orchestrator};

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
