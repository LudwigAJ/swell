//! LLM trait re-exports and extensions.
//!
//! Re-exports the core LLM traits from swell-core for convenience.

pub use swell_core::{
    LlmBackend, LlmMessage, LlmRole, LlmResponse, LlmToolCall,
    LlmToolDefinition, LlmUsage, LlmConfig,
};
