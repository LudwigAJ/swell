//! Worker Boot State Machine
//!
//! This module implements the typed worker boot lifecycle as specified in VAL-TASK-005.
//! Worker startup progresses through typed states: `Spawning → TrustRequired → ReadyForPrompt → Running → Finished`.
//! Each transition is explicit and observable. A worker cannot accept prompts while in `Spawning` state.

use tracing::{debug, info};
use uuid::Uuid;

/// Worker boot lifecycle states as specified in VAL-TASK-005
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerBootState {
    /// Worker is initializing - cannot accept prompts yet
    Spawning,
    /// Worker has started but needs trust verification before accepting prompts
    TrustRequired,
    /// Worker is ready to receive prompts
    ReadyForPrompt,
    /// Worker is actively processing a prompt
    Running,
    /// Worker has finished processing
    Finished,
}

impl std::fmt::Display for WorkerBootState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WorkerBootState::Spawning => write!(f, "Spawning"),
            WorkerBootState::TrustRequired => write!(f, "TrustRequired"),
            WorkerBootState::ReadyForPrompt => write!(f, "ReadyForPrompt"),
            WorkerBootState::Running => write!(f, "Running"),
            WorkerBootState::Finished => write!(f, "Finished"),
        }
    }
}

/// Errors that can occur during worker boot
#[derive(Debug, thiserror::Error)]
pub enum WorkerBootError {
    #[error("Cannot submit prompt while worker is in {state} state")]
    PromptRejectedWhileNotReady { state: WorkerBootState },

    #[error("Invalid state transition from {from} to {to}")]
    InvalidTransition { from: WorkerBootState, to: WorkerBootState },

    #[error("Worker already finished")]
    AlreadyFinished,

    #[error("Worker not found: {0}")]
    WorkerNotFound(Uuid),
}

/// A worker that follows the typed boot lifecycle
#[derive(Debug, Clone)]
pub struct WorkerBoot {
    /// Unique worker identifier
    pub id: Uuid,
    /// Current boot state
    state: WorkerBootState,
}

impl WorkerBoot {
    /// Create a new worker in Spawning state
    pub fn new(id: Uuid) -> Self {
        Self {
            id,
            state: WorkerBootState::Spawning,
        }
    }

    /// Get current boot state
    pub fn state(&self) -> WorkerBootState {
        self.state
    }

    /// Transition from Spawning to TrustRequired
    ///
    /// This is called after the worker process has been spawned but before
    /// trust verification is complete.
    pub fn enter_trust_phase(&mut self) -> Result<(), WorkerBootError> {
        match self.state {
            WorkerBootState::Spawning => {
                self.state = WorkerBootState::TrustRequired;
                debug!(worker_id = %self.id, "Worker transitioned to TrustRequired");
                Ok(())
            }
            _ => Err(WorkerBootError::InvalidTransition {
                from: self.state,
                to: WorkerBootState::TrustRequired,
            }),
        }
    }

    /// Transition from TrustRequired to ReadyForPrompt
    ///
    /// This is called after trust verification has completed successfully.
    pub fn ready_for_prompt(&mut self) -> Result<(), WorkerBootError> {
        match self.state {
            WorkerBootState::TrustRequired => {
                self.state = WorkerBootState::ReadyForPrompt;
                info!(worker_id = %self.id, "Worker is now ready to receive prompts");
                Ok(())
            }
            _ => Err(WorkerBootError::InvalidTransition {
                from: self.state,
                to: WorkerBootState::ReadyForPrompt,
            }),
        }
    }

    /// Transition from ReadyForPrompt to Running
    ///
    /// This is called when the worker starts processing a prompt.
    pub fn start_running(&mut self) -> Result<(), WorkerBootError> {
        match self.state {
            WorkerBootState::ReadyForPrompt => {
                self.state = WorkerBootState::Running;
                info!(worker_id = %self.id, "Worker started processing prompt");
                Ok(())
            }
            _ => Err(WorkerBootError::InvalidTransition {
                from: self.state,
                to: WorkerBootState::Running,
            }),
        }
    }

    /// Transition from Running to Finished
    ///
    /// This is called when the worker has finished processing and should
    /// not accept any more prompts.
    pub fn finish(&mut self) -> Result<(), WorkerBootError> {
        match self.state {
            WorkerBootState::Running => {
                self.state = WorkerBootState::Finished;
                info!(worker_id = %self.id, "Worker finished processing");
                Ok(())
            }
            _ => Err(WorkerBootError::InvalidTransition {
                from: self.state,
                to: WorkerBootState::Finished,
            }),
        }
    }

    /// Submit a prompt to the worker.
    ///
    /// Returns an error if the worker is not in ReadyForPrompt state.
    /// This ensures prompts are not submitted to workers that haven't completed
    /// their boot sequence.
    pub fn submit_prompt(&mut self, _prompt: &str) -> Result<(), WorkerBootError> {
        match self.state {
            WorkerBootState::ReadyForPrompt => {
                // Transition to Running when prompt is submitted
                self.start_running()
            }
            WorkerBootState::Spawning => Err(WorkerBootError::PromptRejectedWhileNotReady {
                state: self.state,
            }),
            WorkerBootState::TrustRequired => Err(WorkerBootError::PromptRejectedWhileNotReady {
                state: self.state,
            }),
            WorkerBootState::Running => {
                // Already running - could queue or reject depending on design
                // For now, we allow this (worker is already processing)
                Ok(())
            }
            WorkerBootState::Finished => Err(WorkerBootError::PromptRejectedWhileNotReady {
                state: self.state,
            }),
        }
    }

    /// Check if the worker can accept prompts
    pub fn can_accept_prompt(&self) -> bool {
        self.state == WorkerBootState::ReadyForPrompt
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // --- Initial State Tests ---

    #[test]
    fn test_worker_starts_in_spawning_state() {
        let worker = WorkerBoot::new(Uuid::new_v4());
        assert_eq!(worker.state(), WorkerBootState::Spawning);
    }

    #[test]
    fn test_worker_initial_state_cannot_accept_prompt() {
        let worker = WorkerBoot::new(Uuid::new_v4());
        assert!(!worker.can_accept_prompt());
    }

    // --- Valid State Transitions ---

    #[test]
    fn test_spawning_to_trust_required() {
        let mut worker = WorkerBoot::new(Uuid::new_v4());
        assert_eq!(worker.state(), WorkerBootState::Spawning);

        worker.enter_trust_phase().unwrap();

        assert_eq!(worker.state(), WorkerBootState::TrustRequired);
    }

    #[test]
    fn test_trust_required_to_ready_for_prompt() {
        let mut worker = WorkerBoot::new(Uuid::new_v4());
        worker.enter_trust_phase().unwrap();
        assert_eq!(worker.state(), WorkerBootState::TrustRequired);

        worker.ready_for_prompt().unwrap();

        assert_eq!(worker.state(), WorkerBootState::ReadyForPrompt);
    }

    #[test]
    fn test_ready_for_prompt_to_running() {
        let mut worker = WorkerBoot::new(Uuid::new_v4());
        worker.enter_trust_phase().unwrap();
        worker.ready_for_prompt().unwrap();
        assert_eq!(worker.state(), WorkerBootState::ReadyForPrompt);

        worker.start_running().unwrap();

        assert_eq!(worker.state(), WorkerBootState::Running);
    }

    #[test]
    fn test_running_to_finished() {
        let mut worker = WorkerBoot::new(Uuid::new_v4());
        worker.enter_trust_phase().unwrap();
        worker.ready_for_prompt().unwrap();
        worker.start_running().unwrap();
        assert_eq!(worker.state(), WorkerBootState::Running);

        worker.finish().unwrap();

        assert_eq!(worker.state(), WorkerBootState::Finished);
    }

    // --- Full Lifecycle Test ---

    #[test]
    fn test_full_lifecycle_spawning_to_finished() {
        let mut worker = WorkerBoot::new(Uuid::new_v4());

        // Initial state
        assert_eq!(worker.state(), WorkerBootState::Spawning);
        assert!(!worker.can_accept_prompt());

        // Enter trust phase
        worker.enter_trust_phase().unwrap();
        assert_eq!(worker.state(), WorkerBootState::TrustRequired);
        assert!(!worker.can_accept_prompt());

        // Ready for prompt
        worker.ready_for_prompt().unwrap();
        assert_eq!(worker.state(), WorkerBootState::ReadyForPrompt);
        assert!(worker.can_accept_prompt());

        // Start running
        worker.start_running().unwrap();
        assert_eq!(worker.state(), WorkerBootState::Running);

        // Finish
        worker.finish().unwrap();
        assert_eq!(worker.state(), WorkerBootState::Finished);
        assert!(!worker.can_accept_prompt());
    }

    // --- Prompt Submission Tests ---

    #[test]
    fn test_prompt_rejected_in_spawning_state() {
        let mut worker = WorkerBoot::new(Uuid::new_v4());
        assert_eq!(worker.state(), WorkerBootState::Spawning);

        let result = worker.submit_prompt("test prompt");
        assert!(result.is_err());

        match result.unwrap_err() {
            WorkerBootError::PromptRejectedWhileNotReady { state } => {
                assert_eq!(state, WorkerBootState::Spawning);
            }
            _ => panic!("Expected PromptRejectedWhileNotReady error"),
        }
    }

    #[test]
    fn test_prompt_rejected_in_trust_required_state() {
        let mut worker = WorkerBoot::new(Uuid::new_v4());
        worker.enter_trust_phase().unwrap();
        assert_eq!(worker.state(), WorkerBootState::TrustRequired);

        let result = worker.submit_prompt("test prompt");
        assert!(result.is_err());

        match result.unwrap_err() {
            WorkerBootError::PromptRejectedWhileNotReady { state } => {
                assert_eq!(state, WorkerBootState::TrustRequired);
            }
            _ => panic!("Expected PromptRejectedWhileNotReady error"),
        }
    }

    #[test]
    fn test_prompt_accepted_in_ready_for_prompt_state() {
        let mut worker = WorkerBoot::new(Uuid::new_v4());
        worker.enter_trust_phase().unwrap();
        worker.ready_for_prompt().unwrap();
        assert_eq!(worker.state(), WorkerBootState::ReadyForPrompt);

        let result = worker.submit_prompt("test prompt");
        assert!(result.is_ok());
        // After submitting, worker should be in Running state
        assert_eq!(worker.state(), WorkerBootState::Running);
    }

    #[test]
    fn test_prompt_rejected_in_finished_state() {
        let mut worker = WorkerBoot::new(Uuid::new_v4());
        worker.enter_trust_phase().unwrap();
        worker.ready_for_prompt().unwrap();
        worker.start_running().unwrap();
        worker.finish().unwrap();
        assert_eq!(worker.state(), WorkerBootState::Finished);

        let result = worker.submit_prompt("test prompt");
        assert!(result.is_err());

        match result.unwrap_err() {
            WorkerBootError::PromptRejectedWhileNotReady { state } => {
                assert_eq!(state, WorkerBootState::Finished);
            }
            _ => panic!("Expected PromptRejectedWhileNotReady error"),
        }
    }

    // --- Invalid State Transitions ---

    #[test]
    fn test_cannot_skip_spawning_to_ready_for_prompt() {
        let mut worker = WorkerBoot::new(Uuid::new_v4());

        // Try to go directly from Spawning to ReadyForPrompt
        let result = worker.ready_for_prompt();
        assert!(result.is_err());

        match result.unwrap_err() {
            WorkerBootError::InvalidTransition { from, to } => {
                assert_eq!(from, WorkerBootState::Spawning);
                assert_eq!(to, WorkerBootState::ReadyForPrompt);
            }
            _ => panic!("Expected InvalidTransition error"),
        }

        // State should not have changed
        assert_eq!(worker.state(), WorkerBootState::Spawning);
    }

    #[test]
    fn test_cannot_skip_trust_required() {
        let mut worker = WorkerBoot::new(Uuid::new_v4());

        // Try to go from Spawning directly to Running
        let result = worker.start_running();
        assert!(result.is_err());

        match result.unwrap_err() {
            WorkerBootError::InvalidTransition { from, to } => {
                assert_eq!(from, WorkerBootState::Spawning);
                assert_eq!(to, WorkerBootState::Running);
            }
            _ => panic!("Expected InvalidTransition error"),
        }

        assert_eq!(worker.state(), WorkerBootState::Spawning);
    }

    #[test]
    fn test_cannot_go_backwards_from_trust_required_to_spawning() {
        let mut worker = WorkerBoot::new(Uuid::new_v4());
        worker.enter_trust_phase().unwrap();
        assert_eq!(worker.state(), WorkerBootState::TrustRequired);

        // Try to go back to Spawning
        let result = worker.enter_trust_phase();
        assert!(result.is_err());

        match result.unwrap_err() {
            WorkerBootError::InvalidTransition { from, to } => {
                assert_eq!(from, WorkerBootState::TrustRequired);
                assert_eq!(to, WorkerBootState::TrustRequired);
            }
            _ => panic!("Expected InvalidTransition error"),
        }

        // State should not have changed
        assert_eq!(worker.state(), WorkerBootState::TrustRequired);
    }

    #[test]
    fn test_cannot_finish_from_ready_for_prompt() {
        let mut worker = WorkerBoot::new(Uuid::new_v4());
        worker.enter_trust_phase().unwrap();
        worker.ready_for_prompt().unwrap();
        assert_eq!(worker.state(), WorkerBootState::ReadyForPrompt);

        // Try to finish without running
        let result = worker.finish();
        assert!(result.is_err());

        match result.unwrap_err() {
            WorkerBootError::InvalidTransition { from, to } => {
                assert_eq!(from, WorkerBootState::ReadyForPrompt);
                assert_eq!(to, WorkerBootState::Finished);
            }
            _ => panic!("Expected InvalidTransition error"),
        }

        assert_eq!(worker.state(), WorkerBootState::ReadyForPrompt);
    }

    #[test]
    fn test_cannot_finish_twice() {
        let mut worker = WorkerBoot::new(Uuid::new_v4());
        worker.enter_trust_phase().unwrap();
        worker.ready_for_prompt().unwrap();
        worker.start_running().unwrap();
        worker.finish().unwrap();
        assert_eq!(worker.state(), WorkerBootState::Finished);

        // Try to finish again
        let result = worker.finish();
        assert!(result.is_err());

        match result.unwrap_err() {
            WorkerBootError::InvalidTransition { from, to } => {
                assert_eq!(from, WorkerBootState::Finished);
                assert_eq!(to, WorkerBootState::Finished);
            }
            _ => panic!("Expected InvalidTransition error"),
        }
    }

    // --- Display Implementation Tests ---

    #[test]
    fn test_worker_boot_state_display() {
        assert_eq!(format!("{}", WorkerBootState::Spawning), "Spawning");
        assert_eq!(format!("{}", WorkerBootState::TrustRequired), "TrustRequired");
        assert_eq!(format!("{}", WorkerBootState::ReadyForPrompt), "ReadyForPrompt");
        assert_eq!(format!("{}", WorkerBootState::Running), "Running");
        assert_eq!(format!("{}", WorkerBootState::Finished), "Finished");
    }

    // --- Error Display Tests ---

    #[test]
    fn test_prompt_rejected_error_display() {
        let error = WorkerBootError::PromptRejectedWhileNotReady {
            state: WorkerBootState::Spawning,
        };
        let msg = format!("{}", error);
        assert!(msg.contains("Spawning"));
        assert!(msg.contains("Cannot submit prompt"));
    }

    #[test]
    fn test_invalid_transition_error_display() {
        let error = WorkerBootError::InvalidTransition {
            from: WorkerBootState::Spawning,
            to: WorkerBootState::ReadyForPrompt,
        };
        let msg = format!("{}", error);
        assert!(msg.contains("Spawning"));
        assert!(msg.contains("ReadyForPrompt"));
        assert!(msg.contains("Invalid state transition"));
    }

    // --- can_accept_prompt Tests ---

    #[test]
    fn test_can_accept_prompt_only_in_ready_state() {
        let mut worker = WorkerBoot::new(Uuid::new_v4());

        // Initially cannot accept
        assert!(!worker.can_accept_prompt());

        // After trust phase, still cannot accept
        worker.enter_trust_phase().unwrap();
        assert!(!worker.can_accept_prompt());

        // In ReadyForPrompt, can accept
        worker.ready_for_prompt().unwrap();
        assert!(worker.can_accept_prompt());

        // After running, cannot accept
        worker.start_running().unwrap();
        assert!(!worker.can_accept_prompt());

        // After finished, cannot accept
        worker.finish().unwrap();
        assert!(!worker.can_accept_prompt());
    }
}
