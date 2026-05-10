//! End-to-end smoke test for the daemon's Unix-socket protocol.
//!
//! Spawns a real `Daemon::run()`, connects a client, sends `TaskCreate`
//! over the socket, and asserts the response is a `TaskCreated` event.
//! Catches socket-binding / serialization regressions that pure unit
//! tests miss.
//!
//! `Daemon::run()` writes a SQLite memory store at
//! `<cwd>/.swell/memory.db`, so this test chdirs into a `TempDir`
//! before spawning the daemon. That makes it process-global state,
//! hence `#[serial]`.

use serial_test::serial;
use std::sync::Arc;
use std::time::Duration;
use swell_core::{
    CliCommand, DaemonEvent, Plan, PlanStep, RiskLevel, SocketPath, StepStatus, TaskState,
};
use swell_daemon::Daemon;
use swell_llm::{LlmBackend, MockLlm, ScenarioMockLlm, ScenarioStep};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

async fn send_command(socket_path: &SocketPath, cmd: CliCommand) -> DaemonEvent {
    let mut stream = UnixStream::connect(socket_path.as_path_buf())
        .await
        .expect("client connect");

    let payload = serde_json::to_vec(&cmd).expect("serialize CliCommand");
    stream.write_all(&payload).await.expect("write payload");
    stream.shutdown().await.expect("shutdown write half");

    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .await
        .expect("read response");

    serde_json::from_slice(&response).expect("response must deserialize as DaemonEvent")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn task_create_round_trips_through_unix_socket() {
    // CostGuard / PreToolHookManager wiring is intentionally Disabled
    // today (Tier 2.x not wired). `Daemon::run()` exits on Disabled
    // subsystems when SWELL_STRICT=1 is set, which the CI test matrix
    // does for one job. Force permissive mode so this smoke test
    // measures socket plumbing, not strict-mode policy.
    std::env::remove_var("SWELL_STRICT");

    let temp = TempDir::new().expect("tempdir");
    std::fs::create_dir_all(temp.path().join(".swell")).unwrap();
    std::env::set_current_dir(temp.path()).unwrap();

    let socket_path = SocketPath::new(temp.path().join("daemon.sock"));
    let llm: Arc<dyn LlmBackend> = Arc::new(MockLlm::new("smoke"));
    let daemon = Daemon::new(socket_path.clone(), llm);

    let socket_for_task = socket_path.clone();
    let daemon_task = tokio::spawn(async move {
        let _ = daemon.run().await;
        let _ = socket_for_task; // keep alive in task scope
    });

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline {
        if socket_path.as_path_buf().exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(
        socket_path.as_path_buf().exists(),
        "daemon socket never bound at {}",
        socket_path
    );

    let cmd = CliCommand::TaskCreate {
        description: "socket-smoke task".to_string(),
    };
    let event = send_command(&socket_path, cmd).await;

    match event {
        DaemonEvent::TaskCreated { .. } => {}
        other => panic!("expected TaskCreated, got {other:?}"),
    }

    daemon_task.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn daemon_smoke_e2e_task_execute_reaches_execution_and_validation() {
    std::env::remove_var("SWELL_STRICT");
    swell_orchestrator::execution::reset_wiring_probe_counts();

    let temp = TempDir::new().expect("tempdir");
    let swell_dir = temp.path().join(".swell");
    std::fs::create_dir_all(&swell_dir).unwrap();
    std::fs::write(
        swell_dir.join("validation.json"),
        r#"{
  "version": "1.0.0",
  "lint": { "commands": [["true"]], "output_format": "generic" },
  "test": { "commands": [["true"]], "concurrency": "parallel" }
}"#,
    )
    .unwrap();
    std::env::set_current_dir(temp.path()).unwrap();

    let socket_path = SocketPath::new(temp.path().join("daemon-exec.sock"));
    let scenario_llm = Arc::new(ScenarioMockLlm::new(
        "daemon-smoke",
        vec![
            ScenarioStep::text(
                r#"{
  "steps": [
    {
      "description": "Report that the daemon execution smoke path is reachable",
      "affected_files": [],
      "expected_tests": [],
      "risk_level": "low"
    }
  ],
  "total_estimated_tokens": 100,
  "risk_assessment": "Low risk daemon smoke"
}"#,
            ),
            ScenarioStep::text("Smoke task completed without file changes."),
        ],
    ));
    let llm: Arc<dyn LlmBackend> = scenario_llm.clone();
    let daemon = Daemon::new(socket_path.clone(), llm);
    let orchestrator = daemon.orchestrator();

    let socket_for_task = socket_path.clone();
    let daemon_task = tokio::spawn(async move {
        let _ = daemon.run().await;
        let _ = socket_for_task;
    });

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline {
        if socket_path.as_path_buf().exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(
        socket_path.as_path_buf().exists(),
        "daemon socket never bound at {}",
        socket_path
    );

    let created = send_command(
        &socket_path,
        CliCommand::TaskCreate {
            description: "daemon smoke e2e".to_string(),
        },
    )
    .await;
    let task_id = match created {
        DaemonEvent::TaskCreated { id, .. } => id,
        other => panic!("expected TaskCreated, got {other:?}"),
    };

    let execute_ack = send_command(&socket_path, CliCommand::TaskExecute { task_id }).await;
    match execute_ack {
        DaemonEvent::TaskStateChanged {
            id,
            state: TaskState::Executing,
            ..
        } if id == task_id => {}
        other => panic!("expected TaskExecute Executing ack, got {other:?}"),
    }

    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let final_task = loop {
        let task = orchestrator.get_task(task_id).await.expect("task exists");
        if matches!(
            task.state,
            TaskState::Accepted | TaskState::Rejected | TaskState::Failed
        ) {
            break task;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!(
                "task did not reach terminal state; last state was {:?}",
                task.state
            );
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    };

    let (execute_count, validation_count) = swell_orchestrator::execution::wiring_probe_counts();
    assert_eq!(
        execute_count, 1,
        "daemon must invoke ExecutionController once"
    );
    assert_eq!(
        validation_count, 1,
        "ExecutionController must invoke ValidationOrchestrator once"
    );
    assert!(
        final_task.validation_result.is_some(),
        "ValidationOrchestrator result must be stored on the task"
    );
    assert_eq!(
        final_task.state,
        TaskState::Accepted,
        "no-op validation config should let the smoke task pass"
    );
    assert_eq!(
        scenario_llm.current_index(),
        2,
        "planner and generator should each consume one scripted LLM response"
    );

    daemon_task.abort();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn task_approve_resumes_awaiting_approval_through_execution_controller() {
    std::env::remove_var("SWELL_STRICT");
    swell_orchestrator::execution::reset_wiring_probe_counts();

    let temp = TempDir::new().expect("tempdir");
    let swell_dir = temp.path().join(".swell");
    std::fs::create_dir_all(&swell_dir).unwrap();
    std::fs::write(
        swell_dir.join("validation.json"),
        r#"{
  "version": "1.0.0",
  "lint": { "commands": [["true"]], "output_format": "generic" },
  "test": { "commands": [["true"]], "concurrency": "parallel" }
}"#,
    )
    .unwrap();
    std::env::set_current_dir(temp.path()).unwrap();

    let socket_path = SocketPath::new(temp.path().join("daemon-approve.sock"));
    let scenario_llm = Arc::new(ScenarioMockLlm::new(
        "daemon-approve-smoke",
        vec![ScenarioStep::text(
            "Approved smoke task completed without file changes.",
        )],
    ));
    let llm: Arc<dyn LlmBackend> = scenario_llm.clone();
    let daemon = Daemon::new(socket_path.clone(), llm);
    let orchestrator = daemon.orchestrator();

    let task = orchestrator
        .create_task("approval resumes execution".to_string(), vec![])
        .await
        .expect("create task");
    let task_id = task.id;
    let plan = Plan {
        id: uuid::Uuid::new_v4(),
        task_id,
        steps: vec![PlanStep {
            id: uuid::Uuid::new_v4(),
            description: "Complete the approved daemon smoke task".to_string(),
            affected_files: vec![],
            expected_tests: vec![],
            risk_level: RiskLevel::Low,
            dependencies: vec![],
            status: StepStatus::Pending,
        }],
        total_estimated_tokens: 100,
        risk_assessment: "Low risk approval smoke".to_string(),
    };
    orchestrator
        .set_plan(task_id, plan)
        .await
        .expect("set plan");
    orchestrator.start_task(task_id).await.expect("start task");
    assert_eq!(
        orchestrator
            .get_task(task_id)
            .await
            .expect("task exists")
            .state,
        TaskState::AwaitingApproval
    );

    let socket_for_task = socket_path.clone();
    let daemon_task = tokio::spawn(async move {
        let _ = daemon.run().await;
        let _ = socket_for_task;
    });

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline {
        if socket_path.as_path_buf().exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(
        socket_path.as_path_buf().exists(),
        "daemon socket never bound at {}",
        socket_path
    );

    let approve_ack = send_command(&socket_path, CliCommand::TaskApprove { task_id }).await;
    match approve_ack {
        DaemonEvent::TaskStateChanged {
            id,
            state: TaskState::Executing,
            ..
        } if id == task_id => {}
        other => panic!("expected TaskApprove Executing ack, got {other:?}"),
    }

    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    let final_task = loop {
        let task = orchestrator.get_task(task_id).await.expect("task exists");
        if matches!(
            task.state,
            TaskState::Accepted | TaskState::Rejected | TaskState::Failed
        ) {
            break task;
        }
        if tokio::time::Instant::now() >= deadline {
            panic!(
                "task did not reach terminal state after approval; last state was {:?}",
                task.state
            );
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    };

    let (execute_count, validation_count) = swell_orchestrator::execution::wiring_probe_counts();
    assert_eq!(
        execute_count, 1,
        "TaskApprove should resume execution through ExecutionController"
    );
    assert_eq!(
        validation_count, 1,
        "approved execution must invoke ValidationOrchestrator once"
    );
    assert_eq!(
        final_task.state,
        TaskState::Accepted,
        "no-op validation config should let the approved task pass"
    );
    assert_eq!(
        scenario_llm.current_index(),
        1,
        "post-approval generator should consume one response"
    );

    daemon_task.abort();
}
