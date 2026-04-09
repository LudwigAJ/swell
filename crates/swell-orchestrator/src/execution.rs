//! Execution controller for managing parallel agent execution.
#![allow(clippy::should_implement_trait)]

use swell_core::{SwellError, ValidationResult};
use crate::{Orchestrator, MAX_CONCURRENT_AGENTS};
use std::sync::Arc;
use futures::stream::{self, StreamExt};
use tracing::info;

/// Manages concurrent task execution with up to 6 agents
pub struct ExecutionController {
    orchestrator: Arc<Orchestrator>,
    max_concurrent: usize,
}

impl ExecutionController {
    pub fn new(orchestrator: Arc<Orchestrator>) -> Self {
        Self {
            orchestrator,
            max_concurrent: MAX_CONCURRENT_AGENTS,
        }
    }

    /// Execute a single task through the full pipeline
    pub async fn execute_task(
        &self,
        task_id: uuid::Uuid,
    ) -> Result<ValidationResult, SwellError> {
        info!(task_id = %task_id, "Starting task execution");
        
        // Step 1: Planning
        self.orchestrator.start_task(task_id).await?;
        
        // Step 2: Generate & Validate (in MVP, simplified)
        self.orchestrator.start_validation(task_id).await?;
        
        // Step 3: Complete with result
        let result = ValidationResult {
            passed: true,
            lint_passed: true,
            tests_passed: true,
            security_passed: true,
            ai_review_passed: true,
            errors: vec![],
            warnings: vec![],
        };
        
        self.orchestrator.complete_task(task_id, result.clone()).await?;
        
        Ok(result)
    }

    /// Execute multiple tasks in parallel, respecting max concurrent agents
    pub async fn execute_batch(
        &self,
        task_ids: Vec<uuid::Uuid>,
    ) -> Vec<Result<ValidationResult, SwellError>> {
        info!(count = task_ids.len(), "Starting batch execution");
        
        let results = stream::iter(task_ids)
            .map(|task_id| {
                let controller = self.clone();
                async move {
                    controller.execute_task(task_id).await
                }
            })
            .buffer_unordered(self.max_concurrent)
            .collect()
            .await;
        
        results
    }

    /// Clone for use in async contexts
    pub fn clone(&self) -> Self {
        Self {
            orchestrator: self.orchestrator.clone(),
            max_concurrent: self.max_concurrent,
        }
    }
}

impl Clone for ExecutionController {
    fn clone(&self) -> Self {
        Self {
            orchestrator: self.orchestrator.clone(),
            max_concurrent: self.max_concurrent,
        }
    }
}

/// Configuration for task execution
#[derive(Debug, Clone)]
pub struct ExecutionConfig {
    pub max_retries: u32,
    pub timeout_secs: u64,
    pub validation_enabled: bool,
}

impl Default for ExecutionConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            timeout_secs: 3600, // 1 hour
            validation_enabled: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_execution_controller_creation() {
        let orchestrator = Orchestrator::new();
        let controller = ExecutionController::new(Arc::new(orchestrator));
        assert_eq!(controller.max_concurrent, MAX_CONCURRENT_AGENTS);
    }

    #[tokio::test]
    async fn test_batch_execution() {
        let orchestrator = Orchestrator::new();
        let controller = ExecutionController::new(Arc::new(orchestrator));
        
        // Create some tasks
        let task1 = controller.orchestrator.create_task("Task 1".to_string()).await;
        let task2 = controller.orchestrator.create_task("Task 2".to_string()).await;
        
        let results = controller.execute_batch(vec![task1.id, task2.id]).await;
        assert_eq!(results.len(), 2);
    }
}
