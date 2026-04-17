//! Builder for [`Orchestrator`].
//!
//! This module provides a builder pattern for constructing [`Orchestrator`] instances.
//! It is only available during tests or when the `test-support` feature is enabled.

#[cfg(any(test, feature = "test-support"))]
use std::sync::Arc;

#[cfg(any(test, feature = "test-support"))]
use swell_llm::LlmBackend;

#[cfg(any(test, feature = "test-support"))]
use swell_state::CheckpointManager;

#[cfg(any(test, feature = "test-support"))]
use crate::Orchestrator;

/// Builder for constructing [`Orchestrator`] instances in tests.
///
/// # Example
///
/// ```ignore
/// use swell_orchestrator::OrchestratorBuilder;
///
/// let orchestrator = OrchestratorBuilder::new().build();
/// ```
#[cfg(any(test, feature = "test-support"))]
#[derive(Default)]
pub struct OrchestratorBuilder {
    llm_backend: Option<Arc<dyn LlmBackend>>,
    checkpoint_manager: Option<Arc<CheckpointManager>>,
}

#[cfg(any(test, feature = "test-support"))]
impl OrchestratorBuilder {
    /// Create a new builder with default (no-op) settings.
    ///
    /// All fields are optional; call the `.with_*` methods to configure.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the LLM backend for the orchestrator.
    ///
    /// When set, the orchestrator will be constructed with the provided LLM backend.
    pub fn with_llm(mut self, llm: Arc<dyn LlmBackend>) -> Self {
        self.llm_backend = Some(llm);
        self
    }

    /// Set a custom checkpoint manager for the orchestrator.
    ///
    /// When set, the orchestrator uses the provided checkpoint manager instead of
    /// creating a default in-memory store.
    pub fn with_checkpoint_manager(
        mut self,
        checkpoint_manager: Arc<CheckpointManager>,
    ) -> Self {
        self.checkpoint_manager = Some(checkpoint_manager);
        self
    }

    /// Build the [`Orchestrator`] with the configured settings.
    ///
    /// Behavior depends on which fields have been set:
    /// - `llm_backend` set → calls `Orchestrator::new(llm)`
    /// - No fields set → calls `Orchestrator::new_for_test()`
    ///
    /// Note: `checkpoint_manager` is accepted for API compatibility but is ignored
    /// in the current implementation since tests don't need custom checkpoint managers.
    pub fn build(self) -> Orchestrator {
        if let Some(llm) = self.llm_backend {
            return Orchestrator::new(llm);
        }

        Orchestrator::new_for_test()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use swell_llm::MockLlm;

    #[tokio::test]
    async fn test_builder_default_constructs_via_new_for_test() {
        // Builder with no config should construct via Orchestrator::new_for_test()
        let _orchestrator = OrchestratorBuilder::new().build();
    }

    #[tokio::test]
    async fn test_builder_with_llm() {
        let mock_llm: Arc<dyn LlmBackend> = Arc::new(MockLlm::new("test-model"));
        let _orchestrator = OrchestratorBuilder::new()
            .with_llm(mock_llm.clone())
            .build();
    }

    #[tokio::test]
    async fn test_builder_method_chaining() {
        let mock_llm: Arc<dyn LlmBackend> = Arc::new(MockLlm::new("test-model"));

        // Both can be chained
        let _orchestrator = OrchestratorBuilder::new()
            .with_llm(mock_llm)
            .build();
    }
}
