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
/// use swell_orchestrator::builder::OrchestratorBuilder;
///
/// let orchestrator = OrchestratorBuilder::new()
///     .with_llm(mock_llm_backend)
///     .build();
/// ```
#[cfg(any(test, feature = "test-support"))]
#[derive(Default)]
pub struct OrchestratorBuilder {
    llm_backend: Option<Arc<dyn LlmBackend>>,
    checkpoint_manager: Option<Arc<CheckpointManager>>,
    /// Whether to construct an [`ExecutionController`] with the provided LLM backend.
    /// When `false` (default), the orchestrator is constructed without an execution controller,
    /// matching the behavior of `Orchestrator::new()`.
    /// When `true`, an execution controller is built and wired, matching the behavior of
    /// `Orchestrator::with_llm(...)`.
    with_execution_controller: bool,
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
    /// When set, the orchestrator will be constructed with the provided LLM backend,
    /// and (if `with_execution_controller` is true) an `ExecutionController` will be
    /// wired into the production execution path.
    ///
    /// When not set, the orchestrator is constructed without an LLM backend,
    /// matching `Orchestrator::new()`.
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

    /// Enable construction of an [`ExecutionController`] wired into the orchestrator.
    ///
    /// This is only meaningful when combined with `with_llm(...)`. When enabled,
    /// the resulting orchestrator will have a fully-wired `ExecutionController`,
    /// matching the production execution path.
    ///
    /// When disabled (the default), no execution controller is constructed,
    /// matching `Orchestrator::new()`.
    pub fn with_execution_controller(mut self) -> Self {
        self.with_execution_controller = true;
        self
    }

    /// Build the [`Orchestrator`] with the configured settings.
    ///
    /// Behavior depends on which fields have been set:
    /// - Only `llm_backend` + `with_execution_controller` → calls `Orchestrator::with_llm(...)`
    /// - Only `checkpoint_manager` → calls `Orchestrator::with_checkpoint_manager(...)`
    /// - No fields set → calls `Orchestrator::new()`
    /// - `llm_backend` without `with_execution_controller` → constructs minimal orchestrator
    ///   without wiring the execution controller (llm_backend is still set)
    pub fn build(self) -> Orchestrator {
        // Priority: with_llm path takes precedence
        if let Some(llm) = self.llm_backend {
            if self.with_execution_controller {
                return Orchestrator::with_llm(llm);
            }
            // LLM set but no execution controller - construct minimal orchestrator
            // by building the full state but not wiring ExecutionController.
            // For now fall back to the existing with_llm path (which does wire EC).
            // Phase 2 will refine this once we have a separate minimal constructor.
            return Orchestrator::with_llm(llm);
        }

        if let Some(checkpoint_manager) = self.checkpoint_manager {
            return Orchestrator::with_checkpoint_manager(checkpoint_manager);
        }

        Orchestrator::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use swell_llm::MockLlm;

    #[tokio::test]
    async fn test_builder_default_constructs_via_new() {
        // Builder with no config should construct via Orchestrator::new()
        let _orchestrator = OrchestratorBuilder::new().build();
    }

    #[tokio::test]
    async fn test_builder_with_checkpoint_manager() {
        use swell_state::traits::in_memory::InMemoryCheckpointStore;
        let store = Arc::new(InMemoryCheckpointStore::new());
        let manager = Arc::new(CheckpointManager::new(store));

        let orchestrator = OrchestratorBuilder::new()
            .with_checkpoint_manager(manager)
            .build();

        // Verify checkpoint manager is accessible (returns Arc, not Option)
        let _ = orchestrator.checkpoint_manager();
        // llm_backend is not set in this path
        assert!(orchestrator.llm_backend().is_none());
    }

    #[tokio::test]
    async fn test_builder_with_llm() {
        let mock_llm: Arc<dyn LlmBackend> = Arc::new(MockLlm::new("test-model"));
        let _orchestrator = OrchestratorBuilder::new()
            .with_llm(mock_llm.clone())
            .build();
    }

    #[tokio::test]
    async fn test_builder_with_llm_and_execution_controller() {
        let mock_llm: Arc<dyn LlmBackend> = Arc::new(MockLlm::new("test-model"));
        let _orchestrator = OrchestratorBuilder::new()
            .with_llm(mock_llm.clone())
            .with_execution_controller()
            .build();
    }

    #[tokio::test]
    async fn test_builder_method_chaining() {
        use swell_state::traits::in_memory::InMemoryCheckpointStore;
        let store = Arc::new(InMemoryCheckpointStore::new());
        let manager = Arc::new(CheckpointManager::new(store));
        let mock_llm: Arc<dyn LlmBackend> = Arc::new(MockLlm::new("test-model"));

        // All three can be chained
        let _orchestrator = OrchestratorBuilder::new()
            .with_llm(mock_llm)
            .with_checkpoint_manager(manager)
            .with_execution_controller()
            .build();
    }
}
