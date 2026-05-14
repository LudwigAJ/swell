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
