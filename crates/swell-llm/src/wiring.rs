//! Wiring diagnostics for LLM backends.
//!
//! This module provides WiringReport implementations for LLM backend types.

use swell_core::wiring::{WiringReport, WiringState};
use swell_core::LlmBackend;

use crate::anthropic::AnthropicBackend;
use crate::openai::OpenAIBackend;

// ========================================================================
// AnthropicBackend wiring report
// ========================================================================

impl WiringReport for AnthropicBackend {
    fn name(&self) -> &'static str {
        "AnthropicBackend"
    }

    fn identity(&self) -> String {
        format!("AnthropicBackend({})@{:p}", self.model(), self)
    }

    fn state(&self) -> WiringState {
        WiringState::Enabled
    }
}

// ========================================================================
// OpenAIBackend wiring report
// ========================================================================

impl WiringReport for OpenAIBackend {
    fn name(&self) -> &'static str {
        "OpenAIBackend"
    }

    fn identity(&self) -> String {
        format!("OpenAIBackend({})@{:p}", self.model(), self)
    }

    fn state(&self) -> WiringState {
        WiringState::Enabled
    }
}
