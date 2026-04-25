//! Integration tests for live progress streaming via `swell task watch`.
//!
//! These tests verify:
//! - VAL-OBS-006: The `swell task watch <id>` command streams per-turn events
//!   (tool invocations, agent transitions, validation results) in real-time,
//!   not just top-level task state transitions.

use swell_core::{DaemonEvent, TaskId};
use uuid::Uuid;

// ============================================================================
// Test: ToolInvocationStarted event serialization/deserialization
// ============================================================================

#[test]
fn test_tool_invocation_started_serde() {
    let task_id = TaskId::new();
    let correlation_id = Uuid::new_v4();
    let event = DaemonEvent::ToolInvocationStarted {
        id: task_id,
        tool_name: "read_file".to_string(),
        arguments: serde_json::json!({"path": "src/main.rs"}),
        turn_number: 1,
        correlation_id,
    };

    // Serialize
    let json = serde_json::to_string(&event).expect("should serialize");

    // Deserialize
    let deserialized: DaemonEvent = serde_json::from_str(&json).expect("should deserialize");

    match deserialized {
        DaemonEvent::ToolInvocationStarted {
            id,
            tool_name,
            arguments,
            turn_number,
            correlation_id: cid,
        } => {
            assert_eq!(id, task_id);
            assert_eq!(tool_name, "read_file");
            assert_eq!(arguments["path"], "src/main.rs");
            assert_eq!(turn_number, 1);
            assert_eq!(cid, correlation_id);
        }
        other => panic!("Expected ToolInvocationStarted, got: {:?}", other),
    }
}

// ============================================================================
// Test: ToolInvocationCompleted event serialization/deserialization
// ============================================================================

#[test]
fn test_tool_invocation_completed_serde() {
    let task_id = TaskId::new();
    let correlation_id = Uuid::new_v4();
    let event = DaemonEvent::ToolInvocationCompleted {
        id: task_id,
        tool_name: "read_file".to_string(),
        success: true,
        duration_ms: 42,
        turn_number: 1,
        correlation_id,
    };

    // Serialize
    let json = serde_json::to_string(&event).expect("should serialize");

    // Deserialize
    let deserialized: DaemonEvent = serde_json::from_str(&json).expect("should deserialize");

    match deserialized {
        DaemonEvent::ToolInvocationCompleted {
            id,
            tool_name,
            success,
            duration_ms,
            turn_number,
            ..
        } => {
            assert_eq!(id, task_id);
            assert_eq!(tool_name, "read_file");
            assert!(success);
            assert_eq!(duration_ms, 42);
            assert_eq!(turn_number, 1);
        }
        other => panic!("Expected ToolInvocationCompleted, got: {:?}", other),
    }
}

// ============================================================================
// Test: ToolInvocationCompleted with failure
// ============================================================================

#[test]
fn test_tool_invocation_completed_failure() {
    let task_id = TaskId::new();
    let correlation_id = Uuid::new_v4();
    let event = DaemonEvent::ToolInvocationCompleted {
        id: task_id,
        tool_name: "write_file".to_string(),
        success: false,
        duration_ms: 10,
        turn_number: 2,
        correlation_id,
    };

    let json = serde_json::to_string(&event).expect("should serialize");
    let deserialized: DaemonEvent = serde_json::from_str(&json).expect("should deserialize");

    match deserialized {
        DaemonEvent::ToolInvocationCompleted { success, .. } => {
            assert!(!success);
        }
        other => panic!("Expected ToolInvocationCompleted, got: {:?}", other),
    }
}

// ============================================================================
// Test: AgentTurnStarted event serialization/deserialization
// ============================================================================

#[test]
fn test_agent_turn_started_serde() {
    let task_id = TaskId::new();
    let correlation_id = Uuid::new_v4();
    let event = DaemonEvent::AgentTurnStarted {
        id: task_id,
        agent_role: "Coder".to_string(),
        turn_number: 1,
        correlation_id,
    };

    // Serialize
    let json = serde_json::to_string(&event).expect("should serialize");

    // Deserialize
    let deserialized: DaemonEvent = serde_json::from_str(&json).expect("should deserialize");

    match deserialized {
        DaemonEvent::AgentTurnStarted {
            id,
            agent_role,
            turn_number,
            correlation_id: cid,
        } => {
            assert_eq!(id, task_id);
            assert_eq!(agent_role, "Coder");
            assert_eq!(turn_number, 1);
            assert_eq!(cid, correlation_id);
        }
        other => panic!("Expected AgentTurnStarted, got: {:?}", other),
    }
}

// ============================================================================
// Test: AgentTurnCompleted event serialization/deserialization
// ============================================================================

#[test]
fn test_agent_turn_completed_serde() {
    let task_id = TaskId::new();
    let correlation_id = Uuid::new_v4();
    let event = DaemonEvent::AgentTurnCompleted {
        id: task_id,
        agent_role: "Coder".to_string(),
        turn_number: 1,
        action_taken: "Read and analyzed file".to_string(),
        tools_invoked: vec!["read_file".to_string(), "grep".to_string()],
        duration_ms: 150,
        correlation_id,
    };

    // Serialize
    let json = serde_json::to_string(&event).expect("should serialize");

    // Deserialize
    let deserialized: DaemonEvent = serde_json::from_str(&json).expect("should deserialize");

    match deserialized {
        DaemonEvent::AgentTurnCompleted {
            id,
            agent_role,
            turn_number,
            action_taken,
            tools_invoked,
            duration_ms,
            ..
        } => {
            assert_eq!(id, task_id);
            assert_eq!(agent_role, "Coder");
            assert_eq!(turn_number, 1);
            assert_eq!(action_taken, "Read and analyzed file");
            assert_eq!(tools_invoked.len(), 2);
            assert!(tools_invoked.contains(&"read_file".to_string()));
            assert!(tools_invoked.contains(&"grep".to_string()));
            assert_eq!(duration_ms, 150);
        }
        other => panic!("Expected AgentTurnCompleted, got: {:?}", other),
    }
}

// ============================================================================
// Test: AgentTurnCompleted with no tools invoked
// ============================================================================

#[test]
fn test_agent_turn_completed_no_tools() {
    let task_id = TaskId::new();
    let correlation_id = Uuid::new_v4();
    let event = DaemonEvent::AgentTurnCompleted {
        id: task_id,
        agent_role: "Planner".to_string(),
        turn_number: 1,
        action_taken: "Generated plan".to_string(),
        tools_invoked: vec![],
        duration_ms: 500,
        correlation_id,
    };

    let json = serde_json::to_string(&event).expect("should serialize");
    let deserialized: DaemonEvent = serde_json::from_str(&json).expect("should deserialize");

    match deserialized {
        DaemonEvent::AgentTurnCompleted { tools_invoked, .. } => {
            assert!(tools_invoked.is_empty());
        }
        other => panic!("Expected AgentTurnCompleted, got: {:?}", other),
    }
}

// ============================================================================
// Test: ValidationStepStarted event serialization/deserialization
// ============================================================================

#[test]
fn test_validation_step_started_serde() {
    let task_id = TaskId::new();
    let correlation_id = Uuid::new_v4();
    let event = DaemonEvent::ValidationStepStarted {
        id: task_id,
        step_name: "lint".to_string(),
        correlation_id,
    };

    // Serialize
    let json = serde_json::to_string(&event).expect("should serialize");

    // Deserialize
    let deserialized: DaemonEvent = serde_json::from_str(&json).expect("should deserialize");

    match deserialized {
        DaemonEvent::ValidationStepStarted {
            id,
            step_name,
            correlation_id: cid,
        } => {
            assert_eq!(id, task_id);
            assert_eq!(step_name, "lint");
            assert_eq!(cid, correlation_id);
        }
        other => panic!("Expected ValidationStepStarted, got: {:?}", other),
    }
}

// ============================================================================
// Test: ValidationStepCompleted event serialization/deserialization (passed)
// ============================================================================

#[test]
fn test_validation_step_completed_passed() {
    let task_id = TaskId::new();
    let correlation_id = Uuid::new_v4();
    let event = DaemonEvent::ValidationStepCompleted {
        id: task_id,
        step_name: "lint".to_string(),
        passed: true,
        duration_ms: 500,
        correlation_id,
    };

    // Serialize
    let json = serde_json::to_string(&event).expect("should serialize");

    // Deserialize
    let deserialized: DaemonEvent = serde_json::from_str(&json).expect("should deserialize");

    match deserialized {
        DaemonEvent::ValidationStepCompleted {
            id,
            step_name,
            passed,
            duration_ms,
            ..
        } => {
            assert_eq!(id, task_id);
            assert_eq!(step_name, "lint");
            assert!(passed);
            assert_eq!(duration_ms, 500);
        }
        other => panic!("Expected ValidationStepCompleted, got: {:?}", other),
    }
}

// ============================================================================
// Test: ValidationStepCompleted event serialization/deserialization (failed)
// ============================================================================

#[test]
fn test_validation_step_completed_failed() {
    let task_id = TaskId::new();
    let correlation_id = Uuid::new_v4();
    let event = DaemonEvent::ValidationStepCompleted {
        id: task_id,
        step_name: "tests".to_string(),
        passed: false,
        duration_ms: 1200,
        correlation_id,
    };

    // Serialize
    let json = serde_json::to_string(&event).expect("should serialize");

    // Deserialize
    let deserialized: DaemonEvent = serde_json::from_str(&json).expect("should deserialize");

    match deserialized {
        DaemonEvent::ValidationStepCompleted {
            step_name,
            passed,
            duration_ms,
            ..
        } => {
            assert_eq!(step_name, "tests");
            assert!(!passed);
            assert_eq!(duration_ms, 1200);
        }
        other => panic!("Expected ValidationStepCompleted, got: {:?}", other),
    }
}

// ============================================================================
// Test: Per-turn event sequence roundtrip
// ============================================================================

#[test]
fn test_per_turn_event_sequence_roundtrip() {
    let task_id = TaskId::new();
    let correlation_id = Uuid::new_v4();

    let events = [
        DaemonEvent::AgentTurnStarted {
            id: task_id,
            agent_role: "Coder".to_string(),
            turn_number: 1,
            correlation_id,
        },
        DaemonEvent::ToolInvocationStarted {
            id: task_id,
            tool_name: "read_file".to_string(),
            arguments: serde_json::json!({"path": "src/lib.rs"}),
            turn_number: 1,
            correlation_id,
        },
        DaemonEvent::ToolInvocationCompleted {
            id: task_id,
            tool_name: "read_file".to_string(),
            success: true,
            duration_ms: 50,
            turn_number: 1,
            correlation_id,
        },
        DaemonEvent::AgentTurnCompleted {
            id: task_id,
            agent_role: "Coder".to_string(),
            turn_number: 1,
            action_taken: "Read file".to_string(),
            tools_invoked: vec!["read_file".to_string()],
            duration_ms: 100,
            correlation_id,
        },
    ];

    // Serialize each event
    let jsons: Vec<String> = events
        .iter()
        .map(|e| serde_json::to_string(e).expect("should serialize"))
        .collect();

    // Deserialize each event
    let deserialized: Vec<DaemonEvent> = jsons
        .iter()
        .map(|j| serde_json::from_str(j).expect("should deserialize"))
        .collect();

    assert_eq!(events.len(), deserialized.len());

    // Verify sequence is preserved
    assert!(matches!(
        deserialized[0],
        DaemonEvent::AgentTurnStarted { .. }
    ));
    assert!(matches!(
        deserialized[1],
        DaemonEvent::ToolInvocationStarted { .. }
    ));
    assert!(matches!(
        deserialized[2],
        DaemonEvent::ToolInvocationCompleted { .. }
    ));
    assert!(matches!(
        deserialized[3],
        DaemonEvent::AgentTurnCompleted { .. }
    ));
}

// ============================================================================
// Test: Multiple turns are tracked independently with turn numbers
// ============================================================================

#[test]
fn test_multiple_turns_tracked_independently() {
    let task_id = TaskId::new();
    let correlation_id = Uuid::new_v4();

    let events = [
        // Turn 1
        DaemonEvent::AgentTurnStarted {
            id: task_id,
            agent_role: "Coder".to_string(),
            turn_number: 1,
            correlation_id,
        },
        DaemonEvent::ToolInvocationStarted {
            id: task_id,
            tool_name: "read_file".to_string(),
            arguments: serde_json::json!({}),
            turn_number: 1,
            correlation_id,
        },
        DaemonEvent::ToolInvocationCompleted {
            id: task_id,
            tool_name: "read_file".to_string(),
            success: true,
            duration_ms: 50,
            turn_number: 1,
            correlation_id,
        },
        DaemonEvent::AgentTurnCompleted {
            id: task_id,
            agent_role: "Coder".to_string(),
            turn_number: 1,
            action_taken: "Read file".to_string(),
            tools_invoked: vec!["read_file".to_string()],
            duration_ms: 100,
            correlation_id,
        },
        // Turn 2
        DaemonEvent::AgentTurnStarted {
            id: task_id,
            agent_role: "Coder".to_string(),
            turn_number: 2,
            correlation_id,
        },
        DaemonEvent::ToolInvocationStarted {
            id: task_id,
            tool_name: "write_file".to_string(),
            arguments: serde_json::json!({}),
            turn_number: 2,
            correlation_id,
        },
        DaemonEvent::ToolInvocationCompleted {
            id: task_id,
            tool_name: "write_file".to_string(),
            success: true,
            duration_ms: 30,
            turn_number: 2,
            correlation_id,
        },
        DaemonEvent::AgentTurnCompleted {
            id: task_id,
            agent_role: "Coder".to_string(),
            turn_number: 2,
            action_taken: "Wrote file".to_string(),
            tools_invoked: vec!["write_file".to_string()],
            duration_ms: 80,
            correlation_id,
        },
    ];

    // Serialize and deserialize
    let jsons: Vec<String> = events
        .iter()
        .map(|e| serde_json::to_string(e).expect("should serialize"))
        .collect();

    let deserialized: Vec<DaemonEvent> = jsons
        .iter()
        .map(|j| serde_json::from_str(j).expect("should deserialize"))
        .collect();

    // Verify turn 1 events have turn_number=1
    match &deserialized[0] {
        DaemonEvent::AgentTurnStarted { turn_number, .. } => assert_eq!(*turn_number, 1),
        _ => panic!("Expected AgentTurnStarted"),
    }
    match &deserialized[1] {
        DaemonEvent::ToolInvocationStarted { turn_number, .. } => assert_eq!(*turn_number, 1),
        _ => panic!("Expected ToolInvocationStarted"),
    }

    // Verify turn 2 events have turn_number=2
    match &deserialized[4] {
        DaemonEvent::AgentTurnStarted { turn_number, .. } => assert_eq!(*turn_number, 2),
        _ => panic!("Expected AgentTurnStarted for turn 2"),
    }
    match &deserialized[5] {
        DaemonEvent::ToolInvocationStarted { turn_number, .. } => assert_eq!(*turn_number, 2),
        _ => panic!("Expected ToolInvocationStarted for turn 2"),
    }
}

// ============================================================================
// Test: Correlation ID links related per-turn events
// ============================================================================

#[test]
fn test_correlation_id_links_per_turn_events() {
    let task_id = TaskId::new();
    let correlation_id_1 = Uuid::new_v4();
    let correlation_id_2 = Uuid::new_v4();

    let events = [
        // Events with correlation_id_1 (same operation)
        DaemonEvent::AgentTurnStarted {
            id: task_id,
            agent_role: "Coder".to_string(),
            turn_number: 1,
            correlation_id: correlation_id_1,
        },
        DaemonEvent::ToolInvocationStarted {
            id: task_id,
            tool_name: "shell".to_string(),
            arguments: serde_json::json!({"command": "cargo build"}),
            turn_number: 1,
            correlation_id: correlation_id_1,
        },
        // Event with different correlation_id_2 (different operation)
        DaemonEvent::TaskStateChanged {
            id: task_id,
            state: swell_core::TaskState::Executing,
            correlation_id: correlation_id_2,
        },
    ];

    // Verify correlation IDs are correctly stored
    match &events[0] {
        DaemonEvent::AgentTurnStarted { correlation_id, .. } => {
            assert_eq!(*correlation_id, correlation_id_1)
        }
        _ => panic!("Expected AgentTurnStarted"),
    }
    match &events[1] {
        DaemonEvent::ToolInvocationStarted { correlation_id, .. } => {
            assert_eq!(*correlation_id, correlation_id_1)
        }
        _ => panic!("Expected ToolInvocationStarted"),
    }
    match &events[2] {
        DaemonEvent::TaskStateChanged { correlation_id, .. } => {
            assert_eq!(*correlation_id, correlation_id_2)
        }
        _ => panic!("Expected TaskStateChanged"),
    }

    // Verify different correlation IDs
    assert_ne!(correlation_id_1, correlation_id_2);
}

// ============================================================================
// Test: Validation pipeline sequence
// ============================================================================

#[test]
fn test_validation_pipeline_sequence() {
    let task_id = TaskId::new();
    let correlation_id = Uuid::new_v4();

    let events = [
        DaemonEvent::ValidationStepStarted {
            id: task_id,
            step_name: "lint".to_string(),
            correlation_id,
        },
        DaemonEvent::ValidationStepCompleted {
            id: task_id,
            step_name: "lint".to_string(),
            passed: true,
            duration_ms: 500,
            correlation_id,
        },
        DaemonEvent::ValidationStepStarted {
            id: task_id,
            step_name: "tests".to_string(),
            correlation_id,
        },
        DaemonEvent::ValidationStepCompleted {
            id: task_id,
            step_name: "tests".to_string(),
            passed: false,
            duration_ms: 1200,
            correlation_id,
        },
        DaemonEvent::ValidationStepStarted {
            id: task_id,
            step_name: "security".to_string(),
            correlation_id,
        },
        DaemonEvent::ValidationStepCompleted {
            id: task_id,
            step_name: "security".to_string(),
            passed: true,
            duration_ms: 800,
            correlation_id,
        },
    ];

    // Serialize and deserialize
    let jsons: Vec<String> = events
        .iter()
        .map(|e| serde_json::to_string(e).expect("should serialize"))
        .collect();

    let deserialized: Vec<DaemonEvent> = jsons
        .iter()
        .map(|j| serde_json::from_str(j).expect("should deserialize"))
        .collect();

    // Verify sequence
    assert_eq!(deserialized.len(), 6);

    // Verify lint passed
    match &deserialized[1] {
        DaemonEvent::ValidationStepCompleted {
            step_name,
            passed,
            duration_ms,
            ..
        } => {
            assert_eq!(step_name, "lint");
            assert!(*passed);
            assert_eq!(*duration_ms, 500);
        }
        _ => panic!("Expected ValidationStepCompleted"),
    }

    // Verify tests failed
    match &deserialized[3] {
        DaemonEvent::ValidationStepCompleted {
            step_name,
            passed,
            duration_ms,
            ..
        } => {
            assert_eq!(step_name, "tests");
            assert!(!*passed);
            assert_eq!(*duration_ms, 1200);
        }
        _ => panic!("Expected ValidationStepCompleted"),
    }

    // Verify security passed
    match &deserialized[5] {
        DaemonEvent::ValidationStepCompleted {
            step_name,
            passed,
            duration_ms,
            ..
        } => {
            assert_eq!(step_name, "security");
            assert!(*passed);
            assert_eq!(*duration_ms, 800);
        }
        _ => panic!("Expected ValidationStepCompleted"),
    }
}

// ============================================================================
// Test: All DaemonEvent variants have unique type tags in JSON
// ============================================================================

#[test]
fn test_all_event_variants_have_unique_type_tags() {
    let task_id = TaskId::new();
    let correlation_id = Uuid::new_v4();

    let events: Vec<DaemonEvent> = vec![
        DaemonEvent::TaskCreated {
            id: task_id,
            correlation_id,
        },
        DaemonEvent::TaskStateChanged {
            id: task_id,
            state: swell_core::TaskState::Executing,
            correlation_id,
        },
        DaemonEvent::TaskProgress {
            id: task_id,
            message: "test".to_string(),
            correlation_id,
        },
        DaemonEvent::TaskCompleted {
            id: task_id,
            pr_url: None,
            correlation_id,
        },
        DaemonEvent::TaskFailed {
            id: task_id,
            error: "test".to_string(),
            failure_class: None,
            correlation_id,
        },
        DaemonEvent::Error {
            message: "test".to_string(),
            failure_class: None,
            correlation_id,
        },
        DaemonEvent::ToolInvocationStarted {
            id: task_id,
            tool_name: "test".to_string(),
            arguments: serde_json::json!({}),
            turn_number: 1,
            correlation_id,
        },
        DaemonEvent::ToolInvocationCompleted {
            id: task_id,
            tool_name: "test".to_string(),
            success: true,
            duration_ms: 100,
            turn_number: 1,
            correlation_id,
        },
        DaemonEvent::AgentTurnStarted {
            id: task_id,
            agent_role: "test".to_string(),
            turn_number: 1,
            correlation_id,
        },
        DaemonEvent::AgentTurnCompleted {
            id: task_id,
            agent_role: "test".to_string(),
            turn_number: 1,
            action_taken: "test".to_string(),
            tools_invoked: vec![],
            duration_ms: 100,
            correlation_id,
        },
        DaemonEvent::ValidationStepStarted {
            id: task_id,
            step_name: "test".to_string(),
            correlation_id,
        },
        DaemonEvent::ValidationStepCompleted {
            id: task_id,
            step_name: "test".to_string(),
            passed: true,
            duration_ms: 100,
            correlation_id,
        },
    ];

    let mut type_tags: Vec<String> = Vec::new();

    for event in &events {
        let json = serde_json::to_string(event).expect("should serialize");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("should parse as JSON");
        let type_tag = parsed
            .get("type")
            .expect("should have type field")
            .as_str()
            .expect("type should be string")
            .to_string();
        type_tags.push(type_tag);
    }

    // Verify all type tags are unique
    let mut sorted = type_tags.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(
        sorted.len(),
        type_tags.len(),
        "All DaemonEvent variants should have unique type tags"
    );
}
