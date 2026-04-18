//! Wiring diagnostics for swell-state types.
//!
//! This module provides WiringReport implementations for state types.

use swell_core::wiring::{WiringReport, WiringState};

use crate::checkpoint_manager::CheckpointManager;

// ========================================================================
// CheckpointManager wiring report
// ========================================================================

impl WiringReport for CheckpointManager {
    fn name(&self) -> &'static str {
        "CheckpointManager"
    }

    fn identity(&self) -> String {
        format!("CheckpointManager@{:p}", self)
    }

    fn state(&self) -> WiringState {
        WiringState::Enabled
    }
}
