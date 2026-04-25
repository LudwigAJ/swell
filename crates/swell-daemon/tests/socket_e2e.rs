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
use swell_core::{CliCommand, DaemonEvent, SocketPath};
use swell_daemon::Daemon;
use swell_llm::{LlmBackend, MockLlm};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

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

    let mut stream = UnixStream::connect(socket_path.as_path_buf())
        .await
        .expect("client connect");

    let cmd = CliCommand::TaskCreate {
        description: "socket-smoke task".to_string(),
    };
    let payload = serde_json::to_vec(&cmd).expect("serialize CliCommand");
    stream.write_all(&payload).await.expect("write payload");
    stream.shutdown().await.expect("shutdown write half");

    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.expect("read response");

    let event: DaemonEvent =
        serde_json::from_slice(&response).expect("response must deserialize as DaemonEvent");

    match event {
        DaemonEvent::TaskCreated { .. } => {}
        other => panic!("expected TaskCreated, got {other:?}"),
    }

    daemon_task.abort();
}
