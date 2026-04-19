//! Concurrent access tests for TaskStateMachine's DashMap-based storage.
//!
//! These tests verify that the DashMap<TaskId, Arc<RwLock<Task>>> storage
//! provides fine-grained concurrent access without global lock contention.

use std::sync::Arc;
use std::time::Duration;
use swell_core::{AgentId, Plan, PlanStep, RiskLevel, StepStatus, TaskId, TaskState};
use swell_orchestrator::state_machine::TaskStateMachine;
use uuid::Uuid;

/// Helper to create a test plan for a task.
fn create_test_plan(task_id: TaskId) -> Plan {
    Plan {
        id: Uuid::new_v4(),
        task_id,
        steps: vec![PlanStep {
            id: Uuid::new_v4(),
            description: "Test step".to_string(),
            affected_files: vec!["test.rs".to_string()],
            expected_tests: vec!["test_foo".to_string()],
            risk_level: RiskLevel::Low,
            dependencies: vec![],
            status: StepStatus::Pending,
        }],
        total_estimated_tokens: 1000,
        risk_assessment: "Low risk".to_string(),
    }
}

/// Test that multiple threads can concurrently create tasks without deadlock.
/// Uses 8+ threads to verify DashMap's fine-grained sharding.
#[test]
fn test_concurrent_task_creation() {
    let sm = Arc::new(TaskStateMachine::new());
    let num_threads = 8;
    let tasks_per_thread = 10;

    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..num_threads)
            .map(|i| {
                let sm = sm.clone();
                scope.spawn(move || {
                    for j in 0..tasks_per_thread {
                        let task_desc = format!("Task from thread {}, number {}", i, j);
                        let task = sm.create_task(task_desc);
                        assert_eq!(task.state, TaskState::Created);
                    }
                })
            })
            .collect();

        for handle in handles {
            handle.join().expect("Thread panicked");
        }
    });

    // Verify all tasks were created
    let all_tasks = sm.get_all_tasks();
    assert_eq!(all_tasks.len(), num_threads * tasks_per_thread);
}

/// Test that multiple threads can concurrently read different tasks.
/// This verifies that DashMap's fine-grained sharding allows parallel
/// access to different shards without global lock contention.
#[test]
fn test_concurrent_reads_different_tasks() {
    let sm = Arc::new(TaskStateMachine::new());

    // Create many tasks upfront
    let num_tasks = 20;
    let task_ids: Vec<_> = (0..num_tasks)
        .map(|i| sm.create_task(format!("Task {}", i)).id)
        .collect();

    // Set plans for tasks that need them
    for task_id in &task_ids {
        let plan = create_test_plan(*task_id);
        sm.set_plan(*task_id, plan).unwrap();
    }

    // Transition some tasks to different states
    for (i, task_id) in task_ids.iter().enumerate() {
        if i % 4 == 0 {
            sm.enrich_task(*task_id).unwrap();
        }
        if i % 4 == 1 {
            sm.enrich_task(*task_id).unwrap();
            sm.ready_task(*task_id).unwrap();
        }
    }

    let num_threads = 8;
    let reads_per_thread = 10;

    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..num_threads)
            .map(|thread_idx| {
                let sm = sm.clone();
                let task_ids = task_ids.clone();
                scope.spawn(move || {
                    for i in 0..reads_per_thread {
                        // Each thread reads different tasks based on its index
                        let task_idx = (thread_idx * reads_per_thread + i) % num_tasks;
                        let task_id = task_ids[task_idx];

                        let result = sm.get_task(task_id);
                        assert!(
                            result.is_ok(),
                            "Thread {} failed to read task {}: {:?}",
                            thread_idx,
                            task_id,
                            result.err()
                        );

                        let task = result.unwrap();
                        // Task should be in one of the states we set
                        assert!(
                            task.state == TaskState::Created
                                || task.state == TaskState::Enriched
                                || task.state == TaskState::Ready,
                            "Unexpected state {:?} for task {}",
                            task.state,
                            task_id
                        );
                    }
                })
            })
            .collect();

        for handle in handles {
            handle.join().expect("Thread panicked");
        }
    });
}

/// Test that multiple threads can concurrently write to different tasks.
/// This verifies that the Arc<RwLock<Task>> per task allows writes to
/// individual tasks without locking the entire state machine.
#[test]
fn test_concurrent_writes_different_tasks() {
    let sm = Arc::new(TaskStateMachine::new());

    // Create many tasks upfront - each thread gets its own set of tasks
    let num_threads = 8;
    let tasks_per_thread = 5;
    let num_tasks = num_threads * tasks_per_thread;
    let task_ids: Vec<_> = (0..num_tasks)
        .map(|i| sm.create_task(format!("Task {}", i)).id)
        .collect();

    // Set plans for all tasks
    for task_id in &task_ids {
        let plan = create_test_plan(*task_id);
        sm.set_plan(*task_id, plan).unwrap();
    }

    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..num_threads)
            .map(|thread_idx| {
                let sm = sm.clone();
                let task_ids = task_ids.clone();
                scope.spawn(move || {
                    for i in 0..tasks_per_thread {
                        // Each thread operates on its own dedicated set of tasks
                        let task_idx = thread_idx * tasks_per_thread + i;
                        let task_id = task_ids[task_idx];

                        // Perform various state transitions on THIS thread's tasks
                        let result = sm.enrich_task(task_id);
                        assert!(
                            result.is_ok(),
                            "Thread {} failed to enrich task {}: {:?}",
                            thread_idx,
                            task_id,
                            result.err()
                        );

                        let result = sm.ready_task(task_id);
                        assert!(
                            result.is_ok(),
                            "Thread {} failed to ready task {}: {:?}",
                            thread_idx,
                            task_id,
                            result.err()
                        );

                        let result = sm.assign_task(task_id, AgentId::new());
                        assert!(
                            result.is_ok(),
                            "Thread {} failed to assign task {}: {:?}",
                            thread_idx,
                            task_id,
                            result.err()
                        );
                    }
                })
            })
            .collect();

        for handle in handles {
            handle.join().expect("Thread panicked");
        }
    });

    // Verify all tasks are in the expected state
    for task_id in &task_ids {
        let task = sm.get_task(*task_id).unwrap();
        assert_eq!(task.state, TaskState::Assigned);
    }
}

/// Test concurrent mixed read/write operations on different tasks.
/// This is the most realistic stress test simulating actual usage.
#[test]
fn test_concurrent_mixed_operations() {
    let sm = Arc::new(TaskStateMachine::new());

    // Create a pool of tasks
    let num_tasks = 24;
    let task_ids: Vec<_> = (0..num_tasks)
        .map(|i| sm.create_task(format!("Task {}", i)).id)
        .collect();

    // Pre-transition some tasks to intermediate states
    for (i, task_id) in task_ids.iter().enumerate() {
        let plan = create_test_plan(*task_id);
        sm.set_plan(*task_id, plan).unwrap();

        if i % 3 == 0 {
            sm.enrich_task(*task_id).unwrap();
        }
    }

    let num_threads = 12; // More than 8 to stress test sharding
    let ops_per_thread = 8;

    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..num_threads)
            .map(|thread_idx| {
                let sm = sm.clone();
                let task_ids = task_ids.clone();
                scope.spawn(move || {
                    for op_idx in 0..ops_per_thread {
                        let task_idx = (thread_idx * ops_per_thread + op_idx) % num_tasks;
                        let task_id = task_ids[task_idx];

                        // Perform a sequence of operations
                        let result = sm.get_task(task_id);
                        if result.is_err() {
                            continue; // Task might be in invalid state for read
                        }

                        // Try to transition through states
                        let _ = sm.enrich_task(task_id);
                        let _ = sm.ready_task(task_id);
                        let _ = sm.assign_task(task_id, AgentId::new());
                        let _ = sm.start_execution(task_id);
                        let _ = sm.start_validation(task_id);

                        // Read state again
                        let _ = sm.get_task(task_id);
                    }
                })
            })
            .collect();

        for handle in handles {
            handle.join().expect("Thread panicked");
        }
    });

    // Verify all tasks are still accessible (no data corruption)
    let all_tasks = sm.get_all_tasks();
    assert_eq!(all_tasks.len(), num_tasks);

    for task in all_tasks {
        // Task should be in one of the valid states after operations
        assert!(
            task.state == TaskState::Assigned
                || task.state == TaskState::Executing
                || task.state == TaskState::Validating,
            "Task {} in unexpected state {:?}",
            task.id,
            task.state
        );
    }
}

/// Test that concurrent operations on the SAME task are properly serialized
/// via the RwLock, preventing data races.
#[test]
fn test_same_task_concurrent_access_serialized() {
    let sm = Arc::new(TaskStateMachine::new());
    let task_id = sm.create_task("Shared task".to_string()).id;
    let plan = create_test_plan(task_id);
    sm.set_plan(task_id, plan).unwrap();

    sm.enrich_task(task_id).unwrap();
    sm.ready_task(task_id).unwrap();

    let num_threads = 8;
    let ops_per_thread = 20;

    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..num_threads)
            .map(|thread_idx| {
                let sm = sm.clone();
                scope.spawn(move || {
                    for _ in 0..ops_per_thread {
                        // All threads try to read/modify the same task
                        let result = sm.get_task(task_id);
                        assert!(
                            result.is_ok(),
                            "Thread {} failed to read task: {:?}",
                            thread_idx,
                            result.err()
                        );

                        let result = sm.assign_task(task_id, AgentId::new());
                        // May fail if another thread already assigned, that's ok
                        let _ = result;
                    }
                })
            })
            .collect();

        for handle in handles {
            handle.join().expect("Thread panicked");
        }
    });

    // Task should still be accessible and in a valid state
    let task = sm.get_task(task_id).unwrap();
    assert_eq!(task.state, TaskState::Assigned);
}

/// Test that the state machine completes all operations within a reasonable
/// time, indicating no deadlocks.
#[test]
fn test_no_deadlock_under_contention() {
    let sm = Arc::new(TaskStateMachine::new());

    // Create many tasks
    let num_tasks = 32;
    let task_ids: Vec<_> = (0..num_tasks)
        .map(|i| sm.create_task(format!("Task {}", i)).id)
        .collect();

    for task_id in &task_ids {
        let plan = create_test_plan(*task_id);
        sm.set_plan(*task_id, plan).unwrap();
    }

    let num_threads = 16; // High thread count to stress locks
    let deadline = std::time::Instant::now() + Duration::from_secs(5);

    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..num_threads)
            .map(|thread_idx| {
                let sm = sm.clone();
                let task_ids = task_ids.clone();
                scope.spawn(move || {
                    let mut completed = 0;
                    while std::time::Instant::now() < deadline {
                        let task_idx = (thread_idx + completed) % num_tasks;
                        let task_id = task_ids[task_idx];

                        let _ = sm.enrich_task(task_id);
                        let _ = sm.ready_task(task_id);
                        let _ = sm.get_task(task_id);

                        completed += 1;
                        if completed >= 50 {
                            break;
                        }
                    }
                })
            })
            .collect();

        for handle in handles {
            handle.join().expect("Thread panicked");
        }
    });

    // If we got here, no deadlock occurred
    let all_tasks = sm.get_all_tasks();
    assert_eq!(all_tasks.len(), num_tasks);
}

/// Test that get_tasks_by_state works correctly under concurrent access.
#[test]
fn test_get_tasks_by_state_concurrent() {
    let sm = Arc::new(TaskStateMachine::new());

    // Create tasks and transition them to various states
    let num_tasks = 20;
    let task_ids: Vec<_> = (0..num_tasks)
        .map(|i| sm.create_task(format!("Task {}", i)).id)
        .collect();

    for task_id in &task_ids {
        let plan = create_test_plan(*task_id);
        sm.set_plan(*task_id, plan).unwrap();
    }

    // Transition each task to Created, Enriched, or Ready
    for (i, task_id) in task_ids.iter().enumerate() {
        match i % 3 {
            0 => {
                sm.enrich_task(*task_id).unwrap();
                sm.ready_task(*task_id).unwrap();
            }
            1 => {
                sm.enrich_task(*task_id).unwrap();
            }
            _ => {}
        }
    }

    // Spawn threads that repeatedly query get_tasks_by_state
    let num_threads = 8;

    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..num_threads)
            .map(|_| {
                let sm = sm.clone();
                scope.spawn(move || {
                    for _ in 0..100 {
                        let created = sm.get_tasks_by_state(TaskState::Created);
                        let enriched = sm.get_tasks_by_state(TaskState::Enriched);
                        let ready = sm.get_tasks_by_state(TaskState::Ready);

                        // Verify counts are consistent
                        let total = created.len() + enriched.len() + ready.len();
                        assert_eq!(total, num_tasks);
                    }
                })
            })
            .collect();

        for handle in handles {
            handle.join().expect("Thread panicked");
        }
    });
}

/// Test remove_task and create_task race condition handling.
#[test]
fn test_concurrent_remove_and_create() {
    let sm = Arc::new(TaskStateMachine::new());

    // Pre-create some tasks
    let initial_ids: Vec<_> = (0..10)
        .map(|i| sm.create_task(format!("Initial task {}", i)).id)
        .collect();

    let num_threads = 8;
    let ops_per_thread = 15;

    std::thread::scope(|scope| {
        // Half the threads remove tasks, half create tasks
        let remove_handles: Vec<_> = (0..num_threads / 2)
            .map(|thread_idx| {
                let sm = sm.clone();
                let initial_ids = initial_ids.clone();
                scope.spawn(move || {
                    for i in 0..ops_per_thread {
                        let idx = (thread_idx + i) % initial_ids.len();
                        let task_id = initial_ids[idx];
                        let _ = sm.remove_task(task_id);
                    }
                })
            })
            .collect();

        let create_handles: Vec<_> = (num_threads / 2..num_threads)
            .map(|thread_idx| {
                let sm = sm.clone();
                scope.spawn(move || {
                    for i in 0..ops_per_thread {
                        let task = sm.create_task(format!(
                            "New task from thread {}",
                            thread_idx * ops_per_thread + i
                        ));
                        // Just creating is enough, we don't need to track
                        let _ = task.id;
                    }
                })
            })
            .collect();

        for handle in remove_handles {
            handle.join().expect("Thread panicked");
        }
        for handle in create_handles {
            handle.join().expect("Thread panicked");
        }
    });

    // Final state should have some tasks (we removed 8*15/2 = 60 initial tasks and added 8*15/2 = 60 new ones)
    // But some initial tasks may have already been removed
    let final_tasks = sm.get_all_tasks();
    assert!(!final_tasks.is_empty(), "All tasks were removed");
}
