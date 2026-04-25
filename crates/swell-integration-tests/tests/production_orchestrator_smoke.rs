//! Production-path smoke test for `Orchestrator::new()`.
//!
//! Daemon and orchestrator unit tests rely on `Orchestrator::new_for_test()`
//! and `OrchestratorBuilder`, which take shortcuts (e.g. mock backends, default
//! checkpoint manager). This test pins the *production* constructor —
//! `Orchestrator::new(llm)` — against silent regression.
//!
//! We don't drive a task all the way to Completed (that requires a full
//! validation pipeline and real tool registry); we assert that:
//!  1. The production constructor runs without panic.
//!  2. Submitting a task emits a `TaskCreated` event over the production
//!     `OrchestratorEvent` stream.

use std::sync::Arc;
use std::time::Duration;

use swell_llm::{LlmBackend, MockLlm};
use swell_orchestrator::{Orchestrator, OrchestratorEvent};

#[tokio::test]
async fn production_orchestrator_new_emits_task_created_event() {
    let llm: Arc<dyn LlmBackend> = Arc::new(MockLlm::new("smoke-model"));

    let orchestrator = Orchestrator::new(llm);
    let mut events = orchestrator.subscribe();

    let task = orchestrator
        .create_task("smoke task".to_string(), Vec::new())
        .await
        .expect("production orchestrator must accept a task");

    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            panic!("timed out waiting for TaskCreated event from production orchestrator");
        }

        match tokio::time::timeout(remaining, events.recv()).await {
            Ok(Ok(OrchestratorEvent::TaskCreated(id))) if id == task.id => return,
            Ok(Ok(_)) => continue,
            Ok(Err(e)) => panic!("event stream closed before TaskCreated: {e:?}"),
            Err(_) => panic!("timed out waiting for TaskCreated event"),
        }
    }
}
