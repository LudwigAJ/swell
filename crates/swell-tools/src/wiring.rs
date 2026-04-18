//! Wiring diagnostics for swell-tools types.
//!
//! This module provides WiringReport implementations for tools types.

use swell_core::wiring::{WiringReport, WiringState};

use crate::mcp_config::McpConfigManager;

// ========================================================================
// McpConfigManager wiring report
// ========================================================================

impl WiringReport for McpConfigManager {
    fn name(&self) -> &'static str {
        "McpConfigManager"
    }

    fn identity(&self) -> String {
        format!("McpConfigManager@{:p}", self)
    }

    fn state(&self) -> WiringState {
        WiringState::Enabled
    }
}
