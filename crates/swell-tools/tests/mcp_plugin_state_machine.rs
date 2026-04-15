//! Plugin State Machine Tests
//!
//! These tests verify that the PluginState enum and PluginStateMachine
//! correctly enforce valid state transitions for MCP plugins.
//!
//! Valid lifecycle: Unconfigured → Validated → Starting → Healthy → Degraded → Failed → ShuttingDown → Stopped
//!
//! Reference: VAL-MCP-005

#[cfg(test)]
mod plugin_state_tests {

    use swell_tools::mcp::PluginState;

    /// Test that PluginState enum has all required states
    #[test]
    fn test_plugin_state_has_all_states() {
        // Verify all 8 states exist
        let states = [
            PluginState::Unconfigured,
            PluginState::Validated,
            PluginState::Starting,
            PluginState::Healthy,
            PluginState::Degraded,
            PluginState::Failed,
            PluginState::ShuttingDown,
            PluginState::Stopped,
        ];

        assert_eq!(states.len(), 8, "PluginState should have 8 states");
    }

    /// Test PluginState name() returns correct display names
    #[test]
    fn test_plugin_state_name() {
        assert_eq!(PluginState::Unconfigured.name(), "Unconfigured");
        assert_eq!(PluginState::Validated.name(), "Validated");
        assert_eq!(PluginState::Starting.name(), "Starting");
        assert_eq!(PluginState::Healthy.name(), "Healthy");
        assert_eq!(PluginState::Degraded.name(), "Degraded");
        assert_eq!(PluginState::Failed.name(), "Failed");
        assert_eq!(PluginState::ShuttingDown.name(), "ShuttingDown");
        assert_eq!(PluginState::Stopped.name(), "Stopped");
    }

    /// Test PluginState Display implementation
    #[test]
    fn test_plugin_state_display() {
        assert_eq!(format!("{}", PluginState::Unconfigured), "Unconfigured");
        assert_eq!(format!("{}", PluginState::Healthy), "Healthy");
        assert_eq!(format!("{}", PluginState::Stopped), "Stopped");
    }

    /// Test PluginState equality and copy
    #[test]
    fn test_plugin_state_equality() {
        let state1 = PluginState::Unconfigured;
        let state2 = PluginState::Unconfigured;
        let state3 = PluginState::Validated;

        assert_eq!(state1, state2);
        assert_ne!(state1, state3);
    }
}

#[cfg(test)]
mod plugin_state_transition_tests {

    use swell_tools::mcp::PluginState;

    /// Test valid transitions from Unconfigured
    #[test]
    fn test_unconfigured_valid_transitions() {
        let state = PluginState::Unconfigured;

        // Valid: Unconfigured → Validated
        assert!(state.can_transition_to(PluginState::Validated));

        // Invalid: cannot skip to any other state
        assert!(!state.can_transition_to(PluginState::Starting));
        assert!(!state.can_transition_to(PluginState::Healthy));
        assert!(!state.can_transition_to(PluginState::Degraded));
        assert!(!state.can_transition_to(PluginState::Failed));
        assert!(!state.can_transition_to(PluginState::ShuttingDown));
        assert!(!state.can_transition_to(PluginState::Stopped));
    }

    /// Test valid transitions from Validated
    #[test]
    fn test_validated_valid_transitions() {
        let state = PluginState::Validated;

        // Valid: Validated → Starting
        assert!(state.can_transition_to(PluginState::Starting));

        // Invalid
        assert!(!state.can_transition_to(PluginState::Unconfigured));
        assert!(!state.can_transition_to(PluginState::Healthy));
        assert!(!state.can_transition_to(PluginState::Degraded));
        assert!(!state.can_transition_to(PluginState::Failed));
        assert!(!state.can_transition_to(PluginState::ShuttingDown));
        assert!(!state.can_transition_to(PluginState::Stopped));
    }

    /// Test valid transitions from Starting
    #[test]
    fn test_starting_valid_transitions() {
        let state = PluginState::Starting;

        // Valid: Starting → Healthy, Degraded, or Failed
        assert!(state.can_transition_to(PluginState::Healthy));
        assert!(state.can_transition_to(PluginState::Degraded));
        assert!(state.can_transition_to(PluginState::Failed));

        // Invalid
        assert!(!state.can_transition_to(PluginState::Unconfigured));
        assert!(!state.can_transition_to(PluginState::Validated));
        assert!(!state.can_transition_to(PluginState::ShuttingDown));
        assert!(!state.can_transition_to(PluginState::Stopped));
    }

    /// Test valid transitions from Healthy
    #[test]
    fn test_healthy_valid_transitions() {
        let state = PluginState::Healthy;

        // Valid: Healthy → Degraded, Failed, or ShuttingDown
        assert!(state.can_transition_to(PluginState::Degraded));
        assert!(state.can_transition_to(PluginState::Failed));
        assert!(state.can_transition_to(PluginState::ShuttingDown));

        // Invalid
        assert!(!state.can_transition_to(PluginState::Unconfigured));
        assert!(!state.can_transition_to(PluginState::Validated));
        assert!(!state.can_transition_to(PluginState::Starting));
        assert!(!state.can_transition_to(PluginState::Stopped));
    }

    /// Test valid transitions from Degraded
    #[test]
    fn test_degraded_valid_transitions() {
        let state = PluginState::Degraded;

        // Valid: Degraded → Healthy, Failed, or ShuttingDown
        assert!(state.can_transition_to(PluginState::Healthy));
        assert!(state.can_transition_to(PluginState::Failed));
        assert!(state.can_transition_to(PluginState::ShuttingDown));

        // Invalid
        assert!(!state.can_transition_to(PluginState::Unconfigured));
        assert!(!state.can_transition_to(PluginState::Validated));
        assert!(!state.can_transition_to(PluginState::Starting));
        assert!(!state.can_transition_to(PluginState::Stopped));
    }

    /// Test valid transitions from Failed
    #[test]
    fn test_failed_valid_transitions() {
        let state = PluginState::Failed;

        // Valid: Failed → ShuttingDown
        assert!(state.can_transition_to(PluginState::ShuttingDown));

        // Invalid - cannot recover to healthy/validated
        assert!(!state.can_transition_to(PluginState::Unconfigured));
        assert!(!state.can_transition_to(PluginState::Validated));
        assert!(!state.can_transition_to(PluginState::Starting));
        assert!(!state.can_transition_to(PluginState::Healthy));
        assert!(!state.can_transition_to(PluginState::Degraded));
        assert!(!state.can_transition_to(PluginState::Stopped));
    }

    /// Test valid transitions from ShuttingDown
    #[test]
    fn test_shutting_down_valid_transitions() {
        let state = PluginState::ShuttingDown;

        // Valid: ShuttingDown → Stopped
        assert!(state.can_transition_to(PluginState::Stopped));

        // Invalid - terminal state (cannot go back)
        assert!(!state.can_transition_to(PluginState::Unconfigured));
        assert!(!state.can_transition_to(PluginState::Validated));
        assert!(!state.can_transition_to(PluginState::Starting));
        assert!(!state.can_transition_to(PluginState::Healthy));
        assert!(!state.can_transition_to(PluginState::Degraded));
        assert!(!state.can_transition_to(PluginState::Failed));
    }

    /// Test valid transitions from Stopped (terminal state)
    #[test]
    fn test_stopped_valid_transitions() {
        let state = PluginState::Stopped;

        // Stopped is a terminal state - no valid transitions
        assert!(!state.can_transition_to(PluginState::Unconfigured));
        assert!(!state.can_transition_to(PluginState::Validated));
        assert!(!state.can_transition_to(PluginState::Starting));
        assert!(!state.can_transition_to(PluginState::Healthy));
        assert!(!state.can_transition_to(PluginState::Degraded));
        assert!(!state.can_transition_to(PluginState::Failed));
        assert!(!state.can_transition_to(PluginState::ShuttingDown));
    }

    /// Test valid_transitions() method returns correct targets
    #[test]
    fn test_valid_transitions_method() {
        assert_eq!(
            PluginState::Unconfigured.valid_transitions(),
            vec![PluginState::Validated]
        );

        assert_eq!(
            PluginState::Validated.valid_transitions(),
            vec![PluginState::Starting]
        );

        let mut starting_transitions = PluginState::Starting.valid_transitions();
        starting_transitions.sort_by_key(|s| format!("{:?}", s));
        assert_eq!(
            starting_transitions,
            vec![
                PluginState::Degraded,
                PluginState::Failed,
                PluginState::Healthy
            ]
        );

        let mut healthy_transitions = PluginState::Healthy.valid_transitions();
        healthy_transitions.sort_by_key(|s| format!("{:?}", s));
        assert_eq!(
            healthy_transitions,
            vec![
                PluginState::Degraded,
                PluginState::Failed,
                PluginState::ShuttingDown
            ]
        );

        let mut degraded_transitions = PluginState::Degraded.valid_transitions();
        degraded_transitions.sort_by_key(|s| format!("{:?}", s));
        assert_eq!(
            degraded_transitions,
            vec![
                PluginState::Failed,
                PluginState::Healthy,
                PluginState::ShuttingDown
            ]
        );

        assert_eq!(
            PluginState::Failed.valid_transitions(),
            vec![PluginState::ShuttingDown]
        );

        assert_eq!(
            PluginState::ShuttingDown.valid_transitions(),
            vec![PluginState::Stopped]
        );

        assert!(PluginState::Stopped.valid_transitions().is_empty());
    }
}

#[cfg(test)]
mod plugin_state_machine_tests {

    use swell_tools::mcp::{PluginState, PluginStateMachine, PluginStateTransitionError};

    /// Test that new PluginStateMachine starts in Unconfigured state
    #[test]
    fn test_state_machine_new_starts_unconfigured() {
        let sm = PluginStateMachine::new();
        assert_eq!(sm.current_state(), PluginState::Unconfigured);
    }

    /// Test that with_state() creates machine in specified state
    #[test]
    fn test_state_machine_with_state() {
        let sm = PluginStateMachine::with_state(PluginState::Healthy);
        assert_eq!(sm.current_state(), PluginState::Healthy);

        let sm = PluginStateMachine::with_state(PluginState::Stopped);
        assert_eq!(sm.current_state(), PluginState::Stopped);
    }

    /// Test successful valid transitions
    #[test]
    fn test_successful_valid_transitions() {
        let mut sm = PluginStateMachine::new();

        // Valid: Unconfigured → Validated
        let result = sm.transition_to(PluginState::Validated);
        assert!(result.is_ok());
        assert_eq!(sm.current_state(), PluginState::Validated);

        // Valid: Validated → Starting
        let result = sm.transition_to(PluginState::Starting);
        assert!(result.is_ok());
        assert_eq!(sm.current_state(), PluginState::Starting);

        // Valid: Starting → Healthy
        let result = sm.transition_to(PluginState::Healthy);
        assert!(result.is_ok());
        assert_eq!(sm.current_state(), PluginState::Healthy);
    }

    /// Test that invalid transitions return errors
    #[test]
    fn test_invalid_transitions_return_errors() {
        let mut sm = PluginStateMachine::new();

        // Invalid: Unconfigured → Healthy (skipping Validated and Starting)
        let result = sm.transition_to(PluginState::Healthy);
        assert!(result.is_err());
        assert_eq!(sm.current_state(), PluginState::Unconfigured);

        // Invalid: Unconfigured → Starting
        let result = sm.transition_to(PluginState::Starting);
        assert!(result.is_err());
        assert_eq!(sm.current_state(), PluginState::Unconfigured);

        // Invalid: Unconfigured → Degraded
        let result = sm.transition_to(PluginState::Degraded);
        assert!(result.is_err());
        assert_eq!(sm.current_state(), PluginState::Unconfigured);
    }

    /// Test error contains correct from/to information
    #[test]
    fn test_transition_error_contains_details() {
        let mut sm = PluginStateMachine::new();

        let result = sm.transition_to(PluginState::Healthy);
        assert!(result.is_err());

        let error = result.unwrap_err();
        assert_eq!(error.from, PluginState::Unconfigured);
        assert_eq!(error.to, PluginState::Healthy);
        assert!(error.message.contains("Unconfigured"));
        assert!(error.message.contains("Healthy"));
    }

    /// Test error display implementation
    #[test]
    fn test_transition_error_display() {
        let error =
            PluginStateTransitionError::new(PluginState::Unconfigured, PluginState::Healthy);

        let display = format!("{}", error);
        assert!(display.contains("Invalid state transition"));
        assert!(display.contains("Unconfigured"));
        assert!(display.contains("Healthy"));
    }

    /// Test full happy path lifecycle: Unconfigured → Validated → Starting → Healthy
    #[test]
    fn test_full_lifecycle_happy_path() {
        let mut sm = PluginStateMachine::new();

        // Progress through the happy path
        assert_eq!(sm.current_state(), PluginState::Unconfigured);

        sm.transition_to(PluginState::Validated).unwrap();
        assert_eq!(sm.current_state(), PluginState::Validated);

        sm.transition_to(PluginState::Starting).unwrap();
        assert_eq!(sm.current_state(), PluginState::Starting);

        sm.transition_to(PluginState::Healthy).unwrap();
        assert_eq!(sm.current_state(), PluginState::Healthy);

        // Can still go to Degraded
        sm.transition_to(PluginState::Degraded).unwrap();
        assert_eq!(sm.current_state(), PluginState::Degraded);

        // Can recover to Healthy
        sm.transition_to(PluginState::Healthy).unwrap();
        assert_eq!(sm.current_state(), PluginState::Healthy);

        // Graceful shutdown
        sm.transition_to(PluginState::ShuttingDown).unwrap();
        assert_eq!(sm.current_state(), PluginState::ShuttingDown);

        sm.transition_to(PluginState::Stopped).unwrap();
        assert_eq!(sm.current_state(), PluginState::Stopped);
    }

    /// Test degraded path: Unconfigured → Validated → Starting → Degraded → Healthy
    #[test]
    fn test_degraded_recovery_path() {
        let mut sm = PluginStateMachine::new();

        sm.transition_to(PluginState::Validated).unwrap();
        sm.transition_to(PluginState::Starting).unwrap();
        sm.transition_to(PluginState::Degraded).unwrap();
        assert_eq!(sm.current_state(), PluginState::Degraded);

        // Recover to Healthy
        sm.transition_to(PluginState::Healthy).unwrap();
        assert_eq!(sm.current_state(), PluginState::Healthy);
    }

    /// Test failure path: Unconfigured → Validated → Starting → Failed → ShuttingDown → Stopped
    #[test]
    fn test_failure_path() {
        let mut sm = PluginStateMachine::new();

        sm.transition_to(PluginState::Validated).unwrap();
        sm.transition_to(PluginState::Starting).unwrap();
        sm.transition_to(PluginState::Failed).unwrap();
        assert_eq!(sm.current_state(), PluginState::Failed);

        // Shutdown from Failed
        sm.transition_to(PluginState::ShuttingDown).unwrap();
        assert_eq!(sm.current_state(), PluginState::ShuttingDown);

        sm.transition_to(PluginState::Stopped).unwrap();
        assert_eq!(sm.current_state(), PluginState::Stopped);
    }

    /// Test that Stopped is truly terminal (no transitions allowed)
    #[test]
    fn test_stopped_is_terminal() {
        let mut sm = PluginStateMachine::with_state(PluginState::Stopped);

        let result = sm.transition_to(PluginState::Starting);
        assert!(result.is_err());
        assert_eq!(sm.current_state(), PluginState::Stopped);

        let result = sm.transition_to(PluginState::Unconfigured);
        assert!(result.is_err());
        assert_eq!(sm.current_state(), PluginState::Stopped);
    }

    /// Test that Failed cannot go back to any working state
    #[test]
    fn test_failed_cannot_recover() {
        let mut sm = PluginStateMachine::with_state(PluginState::Failed);

        // Cannot go back to Healthy
        let result = sm.transition_to(PluginState::Healthy);
        assert!(result.is_err());
        assert_eq!(sm.current_state(), PluginState::Failed);

        // Cannot go back to Degraded
        let result = sm.transition_to(PluginState::Degraded);
        assert!(result.is_err());

        // Cannot go back to Starting
        let result = sm.transition_to(PluginState::Starting);
        assert!(result.is_err());

        // Only valid: Failed → ShuttingDown
        let result = sm.transition_to(PluginState::ShuttingDown);
        assert!(result.is_ok());
    }

    /// Test invalid transitions specified in validation contract
    #[test]
    fn test_contract_invalid_transitions() {
        // Test: Unconfigured → Healthy (skipping Validated and Starting)
        let mut sm1 = PluginStateMachine::new();
        let result = sm1.transition_to(PluginState::Healthy);
        assert!(result.is_err());

        // Test: Stopped → Starting (can't restart from terminal)
        let mut sm2 = PluginStateMachine::with_state(PluginState::Stopped);
        let result = sm2.transition_to(PluginState::Starting);
        assert!(result.is_err());

        // Test: Failed → Validated (can't go back to validated from failed)
        let mut sm3 = PluginStateMachine::with_state(PluginState::Failed);
        let result = sm3.transition_to(PluginState::Validated);
        assert!(result.is_err());
    }

    /// Test all valid transitions from each state succeed
    #[test]
    fn test_all_valid_transitions_succeed() {
        // Test Unconfigured → Validated
        let mut sm = PluginStateMachine::new();
        sm.transition_to(PluginState::Validated).unwrap();

        // Test Validated → Starting
        let mut sm = PluginStateMachine::with_state(PluginState::Validated);
        sm.transition_to(PluginState::Starting).unwrap();

        // Test Starting → Healthy, Degraded, Failed
        let mut sm = PluginStateMachine::with_state(PluginState::Starting);
        sm.transition_to(PluginState::Healthy).unwrap();
        let mut sm = PluginStateMachine::with_state(PluginState::Starting);
        sm.transition_to(PluginState::Degraded).unwrap();
        let mut sm = PluginStateMachine::with_state(PluginState::Starting);
        sm.transition_to(PluginState::Failed).unwrap();

        // Test Healthy → Degraded, Failed, ShuttingDown
        let mut sm = PluginStateMachine::with_state(PluginState::Healthy);
        sm.transition_to(PluginState::Degraded).unwrap();
        let mut sm = PluginStateMachine::with_state(PluginState::Healthy);
        sm.transition_to(PluginState::Failed).unwrap();
        let mut sm = PluginStateMachine::with_state(PluginState::Healthy);
        sm.transition_to(PluginState::ShuttingDown).unwrap();

        // Test Degraded → Healthy, Failed, ShuttingDown
        let mut sm = PluginStateMachine::with_state(PluginState::Degraded);
        sm.transition_to(PluginState::Healthy).unwrap();
        let mut sm = PluginStateMachine::with_state(PluginState::Degraded);
        sm.transition_to(PluginState::Failed).unwrap();
        let mut sm = PluginStateMachine::with_state(PluginState::Degraded);
        sm.transition_to(PluginState::ShuttingDown).unwrap();

        // Test Failed → ShuttingDown
        let mut sm = PluginStateMachine::with_state(PluginState::Failed);
        sm.transition_to(PluginState::ShuttingDown).unwrap();

        // Test ShuttingDown → Stopped
        let mut sm = PluginStateMachine::with_state(PluginState::ShuttingDown);
        sm.transition_to(PluginState::Stopped).unwrap();
    }

    /// Test serde round-trip for PluginState
    #[test]
    fn test_plugin_state_serde() {
        let state = PluginState::Healthy;
        let json = serde_json::to_string(&state).unwrap();
        let parsed: PluginState = serde_json::from_str(&json).unwrap();
        assert_eq!(state, parsed);
    }

    /// Test serde round-trip for PluginStateMachine
    #[test]
    fn test_plugin_state_machine_serde() {
        let sm = PluginStateMachine::with_state(PluginState::Degraded);
        let json = serde_json::to_string(&sm).unwrap();
        let parsed: PluginStateMachine = serde_json::from_str(&json).unwrap();
        assert_eq!(sm.current_state(), parsed.current_state());
    }

    /// Test serde round-trip for PluginStateTransitionError
    #[test]
    fn test_plugin_state_transition_error_serde() {
        let error = swell_tools::mcp::PluginStateTransitionError::new(
            PluginState::Unconfigured,
            PluginState::Healthy,
        );
        let json = serde_json::to_string(&error).unwrap();
        let parsed: swell_tools::mcp::PluginStateTransitionError =
            serde_json::from_str(&json).unwrap();
        assert_eq!(error.from, parsed.from);
        assert_eq!(error.to, parsed.to);
        assert_eq!(error.message, parsed.message);
    }
}
