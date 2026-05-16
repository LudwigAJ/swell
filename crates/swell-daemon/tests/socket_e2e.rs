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
    CliCommand, DaemonEvent, DataResponse, Goal, MilestoneRunOutcome, MilestoneStatus, Plan,
    PlanStep, RiskLevel, SocketPath, StepStatus, TaskState,
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

// PR 02 (TriggerRegistry) — daemon-bootstrap integration test from
// `plan/flow_integration_plan/02_trigger_registry.md` and the "single socket
// integration test" the `18` audit calls out for the trigger spine.
//
// This test installs a `BeforeTask` HaltTrigger on the live `Orchestrator`
// before the daemon starts accepting connections, then sends `TaskCreate`
// + `TaskExecute` over the Unix socket and asserts the task fails with the
// trigger's halt reason surfaced through the wire — proving the trigger
// registry is reachable from production daemon execution and not just
// from the unit-test path on a hand-constructed `ExecutionController`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn before_task_halt_trigger_short_circuits_daemon_task_execute() {
    use async_trait::async_trait;
    use swell_orchestrator::triggers::{Stage, Trigger, TriggerContext, TriggerOutcome};

    std::env::remove_var("SWELL_STRICT");
    swell_orchestrator::execution::reset_wiring_probe_counts();

    let temp = TempDir::new().expect("tempdir");
    let swell_dir = temp.path().join(".swell");
    std::fs::create_dir_all(&swell_dir).unwrap();
    // No-op validation config so a non-halted task would otherwise pass.
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

    let socket_path = SocketPath::new(temp.path().join("daemon-trigger.sock"));
    // Even though the BeforeTask halt should fire before the planner /
    // generator run, the daemon constructs a ScenarioMockLlm by default —
    // give it one scripted response per agent so a regression that lets
    // execution proceed past the halt is caught as "scenario consumed > 0"
    // rather than a panic on an empty scenario.
    let scenario_llm = Arc::new(ScenarioMockLlm::new(
        "halt-smoke",
        vec![
            ScenarioStep::text("planner-should-never-run"),
            ScenarioStep::text("generator-should-never-run"),
        ],
    ));
    let llm: Arc<dyn LlmBackend> = scenario_llm.clone();
    let daemon = Daemon::new(socket_path.clone(), llm);
    let orchestrator = daemon.orchestrator();

    struct HaltTrigger;
    #[async_trait]
    impl Trigger for HaltTrigger {
        fn name(&self) -> &'static str {
            "daemon_smoke_halt"
        }
        fn stages(&self) -> &'static [Stage] {
            &[Stage::BeforeTask]
        }
        async fn run(&self, _ctx: &TriggerContext) -> TriggerOutcome {
            TriggerOutcome::Halt("denied by daemon smoke trigger".into())
        }
    }
    orchestrator.install_trigger(Arc::new(HaltTrigger));
    assert!(
        orchestrator.trigger_names().contains(&"daemon_smoke_halt"),
        "trigger should be registered before daemon starts"
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

    let created = send_command(
        &socket_path,
        CliCommand::TaskCreate {
            description: "trigger-halt smoke".to_string(),
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

    let validation = final_task
        .validation_result
        .as_ref()
        .expect("BeforeTask halt must still surface a ValidationResult");
    assert!(
        !validation.passed,
        "halted task must not be marked passed: {validation:?}"
    );
    assert!(
        validation.errors.iter().any(
            |e| e.contains("daemon_smoke_halt") && e.contains("denied by daemon smoke trigger")
        ),
        "halt reason must surface in validation errors: {:?}",
        validation.errors
    );
    assert_ne!(
        final_task.state,
        TaskState::Accepted,
        "halted task must not reach Accepted"
    );
    assert_eq!(
        scenario_llm.current_index(),
        0,
        "BeforeTask halt must short-circuit before planner / generator consume scripted LLM responses"
    );

    daemon_task.abort();
}

// F4 slice from `plan/flow_integration_plan/09_git_integration.md`:
// installing `git_commit` alongside `validator_gate` via
// `.swell/triggers.json` re-routes both validation and commit through the
// trigger spine. Validator path is asserted by the F3 test below; this
// test focuses on the F4 contract:
//
// 1. Both triggers register from `.swell/triggers.json`.
// 2. The task reaches `Accepted` (so neither trigger silently halted).
// 3. With `git_commit` installed, `execute_task` skips its inline
//    `commit_successful_task` call — covered by the `was_committed_by_trigger`
//    flag in `TaskTriggerState`; verified end-to-end by the daemon still
//    accepting the task while no LLM-side commit synthesis path could have
//    fired (no scripted responses for that).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn git_commit_trigger_registers_and_does_not_block_accepted_task() {
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
    std::fs::write(
        swell_dir.join("triggers.json"),
        r#"{
  "validator_gate": { "stages": ["AfterTask"], "enabled": true },
  "git_commit": { "stages": ["AfterTask"], "enabled": true }
}"#,
    )
    .unwrap();
    std::env::set_current_dir(temp.path()).unwrap();

    let socket_path = SocketPath::new(temp.path().join("daemon-git-commit.sock"));
    let scenario_llm = Arc::new(ScenarioMockLlm::new(
        "git-commit-smoke",
        vec![
            ScenarioStep::text(
                r#"{
  "steps": [
    {
      "description": "Report that the git_commit trigger path is reachable",
      "affected_files": [],
      "expected_tests": [],
      "risk_level": "low"
    }
  ],
  "total_estimated_tokens": 100,
  "risk_assessment": "Low risk git_commit smoke"
}"#,
            ),
            ScenarioStep::text("Git-commit smoke completed without file changes."),
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

    let names = orchestrator.trigger_names();
    assert!(
        names.contains(&"validator_gate") && names.contains(&"git_commit"),
        "daemon bootstrap must install both validator_gate and git_commit from triggers.json: {:?}",
        names
    );

    let created = send_command(
        &socket_path,
        CliCommand::TaskCreate {
            description: "git_commit smoke".to_string(),
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
    assert_eq!(execute_count, 1, "ExecutionController invoked once");
    assert_eq!(
        validation_count, 0,
        "validator_gate trigger should still replace the inline call"
    );
    assert_eq!(
        final_task.state,
        TaskState::Accepted,
        "git_commit trigger must not block an otherwise-passing task"
    );
    assert!(
        final_task.validation_result.is_some(),
        "validation result still stored on the task"
    );

    daemon_task.abort();
}

// F3 slice from `plan/flow_integration_plan/08_validation_gates.md`:
// installing `validator_gate` via `.swell/triggers.json` re-routes the
// validation that today lives inline in `execute_task` through the trigger
// spine. The inline `ValidationOrchestrator` call must be skipped (the
// wiring probe `validation_count` stays at 0), but the task must still
// reach `Accepted` because the trigger writes a passing
// `TaskValidationResult` into the per-fire `TaskTriggerState`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn validator_gate_trigger_replaces_inline_validation_call() {
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
    // Opt validator_gate in via triggers.json. The daemon registers the
    // factory by default; only this entry asks for it to fire.
    std::fs::write(
        swell_dir.join("triggers.json"),
        r#"{ "validator_gate": { "stages": ["AfterTask"], "enabled": true } }"#,
    )
    .unwrap();
    std::env::set_current_dir(temp.path()).unwrap();

    let socket_path = SocketPath::new(temp.path().join("daemon-validator-gate.sock"));
    let scenario_llm = Arc::new(ScenarioMockLlm::new(
        "validator-gate-smoke",
        vec![
            ScenarioStep::text(
                r#"{
  "steps": [
    {
      "description": "Report that the validator_gate trigger path is reachable",
      "affected_files": [],
      "expected_tests": [],
      "risk_level": "low"
    }
  ],
  "total_estimated_tokens": 100,
  "risk_assessment": "Low risk validator_gate smoke"
}"#,
            ),
            ScenarioStep::text("Validator-gate smoke completed without file changes."),
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

    assert!(
        orchestrator.trigger_names().contains(&"validator_gate"),
        "daemon bootstrap must install validator_gate from triggers.json: {:?}",
        orchestrator.trigger_names()
    );

    let created = send_command(
        &socket_path,
        CliCommand::TaskCreate {
            description: "validator_gate smoke".to_string(),
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
        validation_count, 0,
        "validator_gate trigger must replace the inline ValidationOrchestrator call \
         (inline counter stays at 0)"
    );
    assert!(
        final_task.validation_result.is_some(),
        "validator_gate trigger must populate ValidationResult on the task"
    );
    assert_eq!(
        final_task.state,
        TaskState::Accepted,
        "trigger-produced passing validation should let the smoke task reach Accepted"
    );

    daemon_task.abort();
}

// F9 slice from `plan/flow_integration_plan/07_memory_consolidation.md`:
// installing `memory_write` alongside `validator_gate` and `git_commit` via
// `.swell/triggers.json` re-routes skill extraction through the trigger
// spine. With no tool calls produced by the scripted scenario, the trigger
// short-circuits on the empty-trace branch but still marks the task as
// handled — proving the wire-up without depending on actual skill-extraction
// side effects. The task must still reach `Accepted`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn memory_write_trigger_registers_and_does_not_block_accepted_task() {
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
    std::fs::write(
        swell_dir.join("triggers.json"),
        r#"{
  "validator_gate": { "stages": ["AfterTask"], "enabled": true },
  "git_commit":     { "stages": ["AfterTask"], "enabled": true },
  "memory_write":   { "stages": ["AfterTask"], "enabled": true }
}"#,
    )
    .unwrap();
    std::env::set_current_dir(temp.path()).unwrap();

    let socket_path = SocketPath::new(temp.path().join("daemon-memory-write.sock"));
    let scenario_llm = Arc::new(ScenarioMockLlm::new(
        "memory-write-smoke",
        vec![
            ScenarioStep::text(
                r#"{
  "steps": [
    {
      "description": "Report that the memory_write trigger path is reachable",
      "affected_files": [],
      "expected_tests": [],
      "risk_level": "low"
    }
  ],
  "total_estimated_tokens": 100,
  "risk_assessment": "Low risk memory_write smoke"
}"#,
            ),
            ScenarioStep::text("Memory-write smoke completed without file changes."),
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

    let names = orchestrator.trigger_names();
    assert!(
        names.contains(&"validator_gate")
            && names.contains(&"git_commit")
            && names.contains(&"memory_write"),
        "daemon bootstrap must install all three built-in triggers from triggers.json: {:?}",
        names
    );

    let created = send_command(
        &socket_path,
        CliCommand::TaskCreate {
            description: "memory_write smoke".to_string(),
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
    assert_eq!(execute_count, 1, "ExecutionController invoked once");
    assert_eq!(
        validation_count, 0,
        "validator_gate trigger still replaces the inline validation call"
    );
    assert_eq!(
        final_task.state,
        TaskState::Accepted,
        "memory_write trigger must not block an otherwise-passing task"
    );

    daemon_task.abort();
}

// CliCommand::ProjectRun drives the MilestoneScheduler through the daemon
// socket and returns a `DataResponse::ProjectRunReport`. Smallest end-to-end
// proof that the scheduler has a daemon entry point: create a project + a
// milestone + a task assigned to the milestone via the orchestrator handle,
// send `ProjectRun` over the wire, and assert the report has the milestone
// `Done` and the task `Accepted`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn project_run_drives_milestone_scheduler_through_unix_socket() {
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

    let socket_path = SocketPath::new(temp.path().join("daemon-project-run.sock"));
    let scenario_llm = Arc::new(ScenarioMockLlm::new(
        "project-run-smoke",
        vec![
            ScenarioStep::text(
                r#"{
  "steps": [
    {
      "description": "Report that the project-run smoke path is reachable",
      "affected_files": [],
      "expected_tests": [],
      "risk_level": "low"
    }
  ],
  "total_estimated_tokens": 100,
  "risk_assessment": "Low risk project-run smoke"
}"#,
            ),
            ScenarioStep::text("Project-run smoke completed without file changes."),
        ],
    ));
    let llm: Arc<dyn LlmBackend> = scenario_llm.clone();
    let daemon = Daemon::new(socket_path.clone(), llm);
    let orchestrator = daemon.orchestrator();

    let task = orchestrator
        .create_task("project-run smoke task".to_string(), vec![])
        .await
        .expect("create task");
    let task_id = task.id;
    let project = orchestrator
        .create_project(Goal::new("project-run smoke", task_id))
        .await;
    let milestone = orchestrator
        .create_milestone(project.id, "m1".to_string())
        .await
        .expect("create milestone");
    orchestrator
        .assign_task_to_milestone(task_id, milestone.id)
        .await
        .expect("assign task to milestone");

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

    let response = send_command(
        &socket_path,
        CliCommand::ProjectRun {
            project_id: project.id,
        },
    )
    .await;

    let report = match response {
        DaemonEvent::DataResponse(data) => match *data {
            DataResponse::ProjectRunReport {
                project_id,
                all_done,
                attempted,
                stalled,
                ..
            } => {
                assert_eq!(project_id, project.id);
                (all_done, attempted, stalled)
            }
            other => panic!("expected ProjectRunReport, got {other:?}"),
        },
        other => panic!("expected DataResponse, got {other:?}"),
    };
    let (all_done, attempted, stalled) = report;
    assert!(
        stalled.is_empty(),
        "no milestone should stall, got {stalled:?}; attempted={attempted:?}"
    );
    assert!(
        all_done,
        "all_done must be true when the milestone is Done; attempted={attempted:?}"
    );
    assert_eq!(attempted.len(), 1, "exactly one milestone attempted");
    assert_eq!(attempted[0].milestone_id, milestone.id);
    assert!(
        matches!(attempted[0].outcome, MilestoneRunOutcome::Done),
        "milestone outcome must be Done, got {:?}",
        attempted[0].outcome
    );

    let m = orchestrator
        .get_milestone(milestone.id)
        .await
        .expect("milestone exists");
    assert_eq!(m.status, MilestoneStatus::Done);

    let final_task = orchestrator.get_task(task_id).await.expect("task exists");
    assert_eq!(
        final_task.state,
        TaskState::Accepted,
        "task within milestone must reach Accepted; was {:?}",
        final_task.state
    );

    daemon_task.abort();
}

/// PR 04 (plan/flow_integration_plan/04_researcher_handoff.md): the
/// daemon installs the `researcher` factory at boot, and an entry in
/// `.swell/triggers.json` with `mode = "live"` builds a trigger that
/// is registered alongside the other built-ins. We don't drive a
/// failure path here — just prove the factory wires through and the
/// trigger appears on `orchestrator.trigger_names()`. Confirms the
/// daemon's `register_mode_switched_researcher_factory` call site
/// reaches the config loader.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn researcher_trigger_live_mode_registers_from_config() {
    std::env::remove_var("SWELL_STRICT");
    swell_orchestrator::execution::reset_wiring_probe_counts();

    let temp = TempDir::new().expect("tempdir");
    let swell_dir = temp.path().join(".swell");
    std::fs::create_dir_all(&swell_dir).unwrap();
    std::fs::write(
        swell_dir.join("triggers.json"),
        r#"{
  "researcher": {
    "stages": ["OnMilestoneBlocked", "OnTaskFailed"],
    "enabled": true,
    "config": { "mode": "live", "use_tools": true, "max_invocations": 2 }
  }
}"#,
    )
    .unwrap();
    std::env::set_current_dir(temp.path()).unwrap();

    let socket_path = SocketPath::new(temp.path().join("daemon-researcher.sock"));
    // Daemon won't actually call the LLM in this smoke (no failure
    // path fires); MockLlm is fine.
    let llm: Arc<dyn LlmBackend> = Arc::new(MockLlm::new("researcher-smoke"));
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

    let names = orchestrator.trigger_names();
    assert!(
        names.contains(&"researcher"),
        "researcher trigger must be installed from .swell/triggers.json; got {names:?}"
    );

    daemon_task.abort();
}

/// PR 04 (plan/flow_integration_plan/04_researcher_handoff.md): live-path
/// end-to-end smoke proving the entire researcher → reroute → recovery
/// milestone spine is reachable through the daemon socket. This is the
/// proof-of-life for PR 04 — every prior researcher slice (reroute
/// consumer, trigger spine, LLM diagnostic, tool-aware loop, mode-switched
/// daemon wiring) was unit-tested in isolation; here they all run
/// together against a real failing task.
///
/// Topology:
///   Project P
///     ├── Milestone A (one task, forced to fail via in-process BeforeTask
///     │   halt trigger so we don't depend on validation-shell behavior)
///     └── Milestone B (empty recovery milestone, depends_on A so it
///         would `stall` under normal DAG order — only a reroute can
///         reach it)
///
/// Flow:
///   1. `.swell/triggers.json` registers the researcher in `mode: "live"`,
///      `use_tools: false` (single-shot, so each fire = exactly one LLM
///      call against the scripted backend) on `[OnTaskFailed,
///      OnMilestoneBlocked]`.
///   2. ScenarioMockLlm scripts two verdict turns, both `replan` with
///      `replan_milestone = <B.id>`. Fire #1 comes from
///      `fire_on_task_failed` inside `execute_task`'s BeforeTask halt
///      branch; fire #2 comes from the scheduler's `fire_on_blocked`.
///   3. Send `ProjectRun(P)`.
///
/// Assertions:
///   - Milestone A → `BlockedByTaskFailure`.
///   - Milestone B → `Done`.
///   - Stalled is empty (without the reroute, B would stall because
///     A is blocked).
///   - Scripted LLM consumed both verdict steps (proves the researcher
///     actually called out to the shared LLM backend on both fires).
///   - Milestone A status = `Blocked`, milestone B status = `Done`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn researcher_live_mode_reroutes_failed_task_to_recovery_milestone() {
    use async_trait::async_trait;
    use swell_orchestrator::triggers::{Stage, Trigger, TriggerContext, TriggerOutcome};

    std::env::remove_var("SWELL_STRICT");
    swell_orchestrator::execution::reset_wiring_probe_counts();

    let temp = TempDir::new().expect("tempdir");
    let swell_dir = temp.path().join(".swell");
    std::fs::create_dir_all(&swell_dir).unwrap();
    // No-op validation (the BeforeTask halt fires before validation runs;
    // but the daemon still inspects this file at boot).
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

    let socket_path = SocketPath::new(temp.path().join("daemon-researcher-live-reroute.sock"));

    // `Daemon::new` captures its LLM up front, but the scripted verdict
    // needs milestone B's UUID — which we only know after creating B
    // through the daemon's orchestrator. Resolve the ordering via a
    // tiny late-bound shim: construct the daemon with `LateBoundScenarioLlm`,
    // create entities, then set the script.
    let scripted: Arc<std::sync::RwLock<Option<Arc<ScenarioMockLlm>>>> =
        Arc::new(std::sync::RwLock::new(None));
    let backend: Arc<dyn LlmBackend> = Arc::new(LateBoundScenarioLlm {
        inner: Arc::clone(&scripted),
    });

    let daemon = Daemon::new(socket_path.clone(), backend);
    let orchestrator = daemon.orchestrator();

    // Build project: A (with one task), B (empty, depends on A).
    let task_a = orchestrator
        .create_task("force-failure task".to_string(), vec![])
        .await
        .expect("create task A");
    let task_a_id = task_a.id;

    let project = orchestrator
        .create_project(Goal::new("researcher live-path smoke", task_a_id))
        .await;
    let milestone_a = orchestrator
        .create_milestone(project.id, "A".to_string())
        .await
        .expect("create milestone A");
    let milestone_b = orchestrator
        .create_milestone(project.id, "B".to_string())
        .await
        .expect("create milestone B");

    orchestrator
        .assign_task_to_milestone(task_a_id, milestone_a.id)
        .await
        .expect("assign task to milestone A");

    // B depends on A so the only way the scheduler reaches B is via the
    // researcher's reroute. Without reroute, B would stall (A blocked
    // → upstream dep unmet).
    {
        let sm = orchestrator.state_machine();
        let sm = sm.read().await;
        sm.with_milestone_mut(milestone_b.id, |m| {
            m.depends_on.push(milestone_a.id);
            Ok(())
        })
        .expect("wire B depends_on A");
    }

    // Now we know B.id — script the LLM. Two researcher fires per
    // failing task: one from OnTaskFailed, one from OnMilestoneBlocked.
    // Each fire is a single-shot single LLM call (use_tools: false in
    // the config below).
    let verdict_json = format!(
        r#"{{"verdict":"replan","replan_milestone":"{}","reason":"reroute to recovery milestone"}}"#,
        milestone_b.id
    );
    let scenario = Arc::new(ScenarioMockLlm::new(
        "researcher-live-smoke",
        vec![
            ScenarioStep::text(verdict_json.clone()),
            ScenarioStep::text(verdict_json),
        ],
    ));
    *scripted.write().unwrap() = Some(Arc::clone(&scenario));

    // Install BeforeTask halt trigger to force task A into Failed. This
    // fires OnTaskFailed from inside `execute_task`, which the researcher
    // (config-loaded below) then consumes.
    struct ForceFailTrigger;
    #[async_trait]
    impl Trigger for ForceFailTrigger {
        fn name(&self) -> &'static str {
            "force_fail_before_task"
        }
        fn stages(&self) -> &'static [Stage] {
            &[Stage::BeforeTask]
        }
        async fn run(&self, _ctx: &TriggerContext) -> TriggerOutcome {
            TriggerOutcome::Halt("force-failure smoke trigger".into())
        }
    }
    orchestrator.install_trigger(Arc::new(ForceFailTrigger));

    // Write the researcher trigger config. `daemon.run()` reads this at
    // boot before binding the socket and installs the trigger through
    // the mode-switched factory. `use_tools: false` keeps every fire as
    // a single LLM call so the scripted scenario stays counted exactly.
    std::fs::write(
        swell_dir.join("triggers.json"),
        r#"{
  "researcher": {
    "stages": ["OnTaskFailed", "OnMilestoneBlocked"],
    "enabled": true,
    "config": { "mode": "live", "use_tools": false, "max_invocations": 2 }
  }
}"#,
    )
    .unwrap();

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

    // Sanity: both triggers are wired before we drive ProjectRun.
    let names = orchestrator.trigger_names();
    assert!(
        names.contains(&"force_fail_before_task"),
        "force-fail trigger must be installed; got {names:?}"
    );
    assert!(
        names.contains(&"researcher"),
        "researcher trigger must be installed from .swell/triggers.json; got {names:?}"
    );

    let response = send_command(
        &socket_path,
        CliCommand::ProjectRun {
            project_id: project.id,
        },
    )
    .await;

    let (all_done, attempted, stalled) = match response {
        DaemonEvent::DataResponse(data) => match *data {
            DataResponse::ProjectRunReport {
                project_id,
                all_done,
                attempted,
                stalled,
                ..
            } => {
                assert_eq!(project_id, project.id);
                (all_done, attempted, stalled)
            }
            other => panic!("expected ProjectRunReport, got {other:?}"),
        },
        other => panic!("expected DataResponse, got {other:?}"),
    };

    assert!(
        stalled.is_empty(),
        "stalled must be empty — recovery milestone B should have been \
         reached via reroute, not left to stall. attempted={attempted:?} \
         stalled={stalled:?}"
    );
    assert_eq!(
        attempted.len(),
        2,
        "scheduler must attempt both milestones (A then B-via-reroute); \
         got attempted={attempted:?}"
    );
    assert_eq!(attempted[0].milestone_id, milestone_a.id);
    assert!(
        matches!(
            attempted[0].outcome,
            MilestoneRunOutcome::BlockedByTaskFailure { failing_task } if failing_task == task_a_id
        ),
        "milestone A must end BlockedByTaskFailure(task_a); got {:?}",
        attempted[0].outcome
    );
    assert_eq!(attempted[1].milestone_id, milestone_b.id);
    assert!(
        matches!(attempted[1].outcome, MilestoneRunOutcome::Done),
        "milestone B (recovery) must end Done; got {:?}",
        attempted[1].outcome
    );
    assert!(
        !all_done,
        "all_done must be false because milestone A ended \
         BlockedByTaskFailure; got all_done=true"
    );

    let m_a = orchestrator
        .get_milestone(milestone_a.id)
        .await
        .expect("milestone A exists");
    assert_eq!(
        m_a.status,
        MilestoneStatus::Blocked,
        "milestone A must end Blocked"
    );
    let m_b = orchestrator
        .get_milestone(milestone_b.id)
        .await
        .expect("milestone B exists");
    assert_eq!(
        m_b.status,
        MilestoneStatus::Done,
        "milestone B (recovery) must end Done"
    );

    // The researcher trigger fires twice for a single failing task in a
    // single milestone: once from `fire_on_task_failed` inside
    // `execute_task`, once from `fire_on_blocked` inside the scheduler.
    // Both are single-shot LLM calls because `use_tools: false`.
    assert_eq!(
        scenario.current_index(),
        2,
        "researcher must consume two verdict steps (OnTaskFailed + \
         OnMilestoneBlocked); got {}",
        scenario.current_index()
    );

    daemon_task.abort();
}

/// PR 04 (plan/flow_integration_plan/04_researcher_handoff.md): live-path
/// E2E for `Handoff::SplitMilestone`. Proves the researcher → orchestrator
/// `split_milestone` factory → scheduler-reroute-into-first-child path
/// is reachable through the production daemon socket.
///
/// Topology:
///   Project P
///     └── Milestone A (one task, forced to fail via in-process
///         BeforeTask halt). No sibling milestones — the only way
///         the scheduler reaches a non-blocked terminal state is by
///         the researcher actually creating new milestones at fire
///         time.
///
/// Flow:
///   1. `.swell/triggers.json` registers the researcher in `mode: "live"`,
///      `use_tools: false`, `max_invocations: 1` on `[OnTaskFailed,
///      OnMilestoneBlocked]`. The `max_invocations: 1` is deliberate:
///      the OnTaskFailed fire consumes the budget, the OnMilestoneBlocked
///      fire short-circuits with `Halt("researcher budget exceeded")`
///      which is observational at that stage (does NOT override the
///      Reroute from OnTaskFailed). One LLM call total.
///   2. ScenarioMockLlm scripts one verdict turn:
///      `{"verdict":"split_milestone","sub_plans":[{"name":"narrow-x"}]}`.
///   3. Send `ProjectRun(P)`.
///
/// Assertions:
///   - Project ends with 2 milestones: A (Blocked) and a new
///     child titled "narrow-x" (Done).
///   - `attempted = [A=BlockedByTaskFailure, narrow-x=Done]`.
///   - Stalled is empty.
///   - LLM consumed exactly one verdict step.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[serial]
async fn researcher_live_mode_split_milestone_creates_children_and_walks_first() {
    use async_trait::async_trait;
    use swell_orchestrator::triggers::{Stage, Trigger, TriggerContext, TriggerOutcome};

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

    let socket_path = SocketPath::new(temp.path().join("daemon-researcher-split.sock"));

    // Split-milestone responses don't reference UUIDs (the child IDs
    // are minted by the orchestrator at apply time), so we don't need
    // the late-bound shim — a plain ScenarioMockLlm is enough.
    let scenario = Arc::new(ScenarioMockLlm::new(
        "researcher-split-smoke",
        vec![ScenarioStep::text(
            r#"{"verdict":"split_milestone","reason":"milestone too broad","sub_plans":[{"name":"narrow-x","description":"narrowed scope"}]}"#,
        )],
    ));
    let llm: Arc<dyn LlmBackend> = scenario.clone();

    let daemon = Daemon::new(socket_path.clone(), llm);
    let orchestrator = daemon.orchestrator();

    let task_a = orchestrator
        .create_task("force-failure task".to_string(), vec![])
        .await
        .expect("create task A");
    let task_a_id = task_a.id;
    let project = orchestrator
        .create_project(Goal::new("split smoke", task_a_id))
        .await;
    let milestone_a = orchestrator
        .create_milestone(project.id, "A".to_string())
        .await
        .expect("create milestone A");
    orchestrator
        .assign_task_to_milestone(task_a_id, milestone_a.id)
        .await
        .expect("assign task to milestone A");

    // BeforeTask halt → fail task A → fire OnTaskFailed → researcher
    // fires → split_milestone verdict → orchestrator creates child.
    struct ForceFailTrigger;
    #[async_trait]
    impl Trigger for ForceFailTrigger {
        fn name(&self) -> &'static str {
            "force_fail_before_task"
        }
        fn stages(&self) -> &'static [Stage] {
            &[Stage::BeforeTask]
        }
        async fn run(&self, _ctx: &TriggerContext) -> TriggerOutcome {
            TriggerOutcome::Halt("force-failure smoke trigger".into())
        }
    }
    orchestrator.install_trigger(Arc::new(ForceFailTrigger));

    // `max_invocations: 1` keeps the script size at exactly one verdict.
    std::fs::write(
        swell_dir.join("triggers.json"),
        r#"{
  "researcher": {
    "stages": ["OnTaskFailed", "OnMilestoneBlocked"],
    "enabled": true,
    "config": { "mode": "live", "use_tools": false, "max_invocations": 1 }
  }
}"#,
    )
    .unwrap();

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

    let response = send_command(
        &socket_path,
        CliCommand::ProjectRun {
            project_id: project.id,
        },
    )
    .await;

    let (attempted, stalled) = match response {
        DaemonEvent::DataResponse(data) => match *data {
            DataResponse::ProjectRunReport {
                project_id,
                attempted,
                stalled,
                ..
            } => {
                assert_eq!(project_id, project.id);
                (attempted, stalled)
            }
            other => panic!("expected ProjectRunReport, got {other:?}"),
        },
        other => panic!("expected DataResponse, got {other:?}"),
    };

    assert!(
        stalled.is_empty(),
        "no milestone should stall — the split child must be reachable; \
         attempted={attempted:?} stalled={stalled:?}"
    );
    // attempted = [A=BlockedByTaskFailure, child=Done]
    assert_eq!(
        attempted.len(),
        2,
        "expected source + one child to be attempted; got {attempted:?}"
    );
    assert_eq!(attempted[0].milestone_id, milestone_a.id);
    assert!(
        matches!(
            attempted[0].outcome,
            MilestoneRunOutcome::BlockedByTaskFailure { failing_task } if failing_task == task_a_id
        ),
        "source milestone must end BlockedByTaskFailure(task_a); got {:?}",
        attempted[0].outcome
    );
    let child_entry = &attempted[1];
    assert!(
        matches!(child_entry.outcome, MilestoneRunOutcome::Done),
        "split child must end Done; got {:?}",
        child_entry.outcome
    );
    assert_ne!(
        child_entry.milestone_id, milestone_a.id,
        "child must be a NEW milestone, not the source"
    );

    // Verify the project really has two milestones now and the new
    // one is the named child.
    let project_milestones = orchestrator
        .get_milestones_for_project(project.id)
        .await
        .expect("project milestones");
    assert_eq!(
        project_milestones.len(),
        2,
        "project must contain source + one child after split"
    );
    let child = project_milestones
        .iter()
        .find(|m| m.id != milestone_a.id)
        .expect("child milestone present");
    assert_eq!(child.title, "narrow-x");
    assert_eq!(child.id, child_entry.milestone_id);
    assert_eq!(child.status, MilestoneStatus::Done);
    let source_post = project_milestones
        .iter()
        .find(|m| m.id == milestone_a.id)
        .unwrap();
    assert_eq!(
        source_post.status,
        MilestoneStatus::Blocked,
        "source milestone must be parked Blocked after split"
    );

    // Budget cap=1 + only one fire-with-diagnostic = exactly one LLM call.
    assert_eq!(
        scenario.current_index(),
        1,
        "researcher must consume exactly one verdict (OnTaskFailed). The \
         OnMilestoneBlocked fire is halted pre-diagnostic by the budget \
         cap, so no second LLM call. Got {}",
        scenario.current_index()
    );

    daemon_task.abort();
}

/// Test-only LLM backend that defers to a `ScenarioMockLlm` set after
/// construction. Lets the test create the daemon (which captures its
/// LLM up front), discover the recovery milestone's id from the live
/// orchestrator, then bind a script that embeds that id.
struct LateBoundScenarioLlm {
    inner: Arc<std::sync::RwLock<Option<Arc<ScenarioMockLlm>>>>,
}

impl std::fmt::Debug for LateBoundScenarioLlm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LateBoundScenarioLlm").finish()
    }
}

#[async_trait::async_trait]
impl LlmBackend for LateBoundScenarioLlm {
    fn model(&self) -> &str {
        "researcher-live-smoke"
    }

    async fn chat(
        &self,
        messages: Vec<swell_llm::LlmMessage>,
        tools: Option<Vec<swell_llm::LlmToolDefinition>>,
        config: swell_llm::LlmConfig,
    ) -> Result<swell_llm::LlmResponse, swell_core::SwellError> {
        let inner = self.bound();
        inner.chat(messages, tools, config).await
    }

    async fn health_check(&self) -> bool {
        match self.inner.read().ok().and_then(|g| g.clone()) {
            Some(inner) => inner.health_check().await,
            None => true,
        }
    }

    async fn stream(
        &self,
        messages: Vec<swell_llm::LlmMessage>,
        tools: Option<Vec<swell_llm::LlmToolDefinition>>,
        config: swell_llm::LlmConfig,
    ) -> Result<
        std::pin::Pin<
            Box<
                dyn futures::Stream<Item = Result<swell_core::StreamEvent, swell_core::SwellError>>
                    + Send,
            >,
        >,
        swell_core::SwellError,
    > {
        let inner = self.bound();
        inner.stream(messages, tools, config).await
    }
}

impl LateBoundScenarioLlm {
    fn bound(&self) -> Arc<ScenarioMockLlm> {
        let guard = self.inner.read().expect("scripted LLM poisoned");
        guard
            .as_ref()
            .cloned()
            .expect("scripted LLM not bound before first call")
    }
}
