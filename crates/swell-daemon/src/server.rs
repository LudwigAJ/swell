use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use swell_core::{CliCommand, DaemonEvent, TaskState};
use swell_orchestrator::Orchestrator;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{watch, Mutex};
use tokio::time::{timeout, Duration};
use tracing::{error, info, warn};

/// Maximum time to wait for active connections to complete during shutdown
const SHUTDOWN_TIMEOUT_SECS: u64 = 30;

/// Graceful shutdown configuration
const SHUTDOWN_BROADCAST_INTERVAL_SECS: u64 = 1;

pub struct Daemon {
    orchestrator: Arc<Mutex<Orchestrator>>,
    socket_path: String,
    /// Flag indicating shutdown has been requested
    shutdown_flag: Arc<AtomicBool>,
    /// Counter for active connections
    active_connections: Arc<AtomicUsize>,
    /// Channel to signal shutdown to all tasks
    shutdown_tx: Arc<watch::Sender<bool>>,
    /// Receiver for shutdown signal - shared across spawned tasks
    shutdown_rx: watch::Receiver<bool>,
}

impl Daemon {
    pub fn new(socket_path: String) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        Self {
            orchestrator: Arc::new(Mutex::new(Orchestrator::new())),
            socket_path,
            shutdown_flag: Arc::new(AtomicBool::new(false)),
            active_connections: Arc::new(AtomicUsize::new(0)),
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
        }
    }

    /// Check if shutdown has been requested
    fn is_shutting_down(&self) -> bool {
        self.shutdown_flag.load(Ordering::Relaxed)
    }

    /// Request shutdown - this signals all tasks to stop accepting new work
    /// This method is available for programmatic shutdown (e.g., via internal API)
    #[allow(dead_code)]
    fn request_shutdown(&self) {
        info!("Shutdown requested, stopping new connections...");
        self.shutdown_flag.store(true, Ordering::Relaxed);
        let _ = self.shutdown_tx.send(true);
    }

    pub async fn run(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Remove existing socket file
        if std::path::Path::new(&self.socket_path).exists() {
            std::fs::remove_file(&self.socket_path)?;
        }

        let listener = UnixListener::bind(&self.socket_path)?;
        info!(path = %self.socket_path, "Daemon listening");

        // Spawn SIGTERM handler
        let shutdown_flag = Arc::clone(&self.shutdown_flag);
        let shutdown_tx = Arc::clone(&self.shutdown_tx);
        tokio::spawn(async move {
            if let Err(e) = handle_sigterm(shutdown_flag, shutdown_tx).await {
                error!(error = %e, "SIGTERM handler error");
            }
        });

        // Accept loop
        loop {
            // Check for shutdown before accepting
            if self.is_shutting_down() {
                info!("Shutdown in progress, no longer accepting new connections");
                break;
            }

            // Use timeout on accept to allow checking shutdown flag periodically
            match timeout(Duration::from_secs(SHUTDOWN_BROADCAST_INTERVAL_SECS), listener.accept()).await {
                Ok(Ok((stream, _))) => {
                    // Increment active connections
                    self.active_connections.fetch_add(1, Ordering::Relaxed);
                    let active_connections = Arc::clone(&self.active_connections);
                    let shutdown_rx = self.shutdown_rx.clone();
                    let orchestrator = Arc::clone(&self.orchestrator);

                    tokio::spawn(async move {
                        let result = handle_connection_with_shutdown(stream, orchestrator, shutdown_rx).await;
                        // Decrement active connections when done
                        active_connections.fetch_sub(1, Ordering::Relaxed);
                        if let Err(e) = result {
                            error!(error = %e, "Connection handler error");
                        }
                    });
                }
                Ok(Err(e)) => {
                    error!(error = %e, "Failed to accept connection");
                }
                Err(_) => {
                    // Timeout - loop back to check shutdown flag
                    continue;
                }
            }
        }

        // Wait for active connections to complete
        self.wait_for_active_connections().await?;

        // Clean up socket file
        self.cleanup_socket().await;

        info!("Daemon shutdown complete");
        Ok(())
    }

    /// Wait for all active connections to complete
    async fn wait_for_active_connections(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut remaining = self.active_connections.load(Ordering::Relaxed);
        let start = std::time::Instant::now();

        while remaining > 0 {
            if start.elapsed().as_secs() >= SHUTDOWN_TIMEOUT_SECS {
                warn!(
                    remaining = remaining,
                    elapsed = start.elapsed().as_secs(),
                    "Shutdown timeout reached, forcing exit with active connections"
                );
                return Ok(());
            }

            info!(
                remaining = remaining,
                elapsed = start.elapsed().as_secs(),
                "Waiting for active connections to complete..."
            );

            // Wait a bit before checking again
            tokio::time::sleep(Duration::from_secs(SHUTDOWN_BROADCAST_INTERVAL_SECS)).await;
            remaining = self.active_connections.load(Ordering::Relaxed);
        }

        info!("All active connections completed");
        Ok(())
    }

    /// Remove the socket file
    async fn cleanup_socket(&self) {
        if std::path::Path::new(&self.socket_path).exists() {
            match std::fs::remove_file(&self.socket_path) {
                Ok(()) => info!(path = %self.socket_path, "Socket file removed"),
                Err(e) => warn!(path = %self.socket_path, error = %e, "Failed to remove socket file"),
            }
        }
    }
}

/// Handle SIGTERM signal
async fn handle_sigterm(
    shutdown_flag: Arc<AtomicBool>,
    shutdown_tx: Arc<watch::Sender<bool>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Use tokio's signal handling
    // Note: This requires the "tokio/unix" feature which is included in "full"
    use tokio::signal::unix::{signal, SignalKind};

    let mut sigterm = signal(SignalKind::terminate())?;

    sigterm.recv().await;
    info!("Received SIGTERM");

    // Set shutdown flag and broadcast
    shutdown_flag.store(true, Ordering::Relaxed);
    let _ = shutdown_tx.send(true);

    Ok(())
}

/// Handle a connection with shutdown awareness
async fn handle_connection_with_shutdown(
    stream: UnixStream,
    orchestrator: Arc<Mutex<Orchestrator>>,
    mut shutdown_rx: watch::Receiver<bool>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Check if shutdown was already requested before we started
    if *shutdown_rx.borrow() {
        info!("Connection rejected - shutdown in progress");
        return Ok(());
    }

    let mut stream = stream;
    let mut buf = Vec::with_capacity(4096);

    // Read with timeout to allow checking shutdown signal
    loop {
        tokio::select! {
            result = stream.read_buf(&mut buf) => {
                let n = result?;
                if n == 0 {
                    return Ok(());
                }
                break;
            }
            _ = shutdown_rx.changed() => {
                // Shutdown signal received
                if *shutdown_rx.borrow() {
                    info!("Connection closed - shutdown in progress");
                    return Ok(());
                }
            }
        }
    }

    let input = String::from_utf8_lossy(&buf[..]);
    let command: CliCommand = match serde_json::from_str(&input) {
        Ok(cmd) => cmd,
        Err(e) => {
            let response = serde_json::to_string(&DaemonEvent::Error {
                message: format!("Invalid command: {}", e),
            })?;
            stream.write_all(response.as_bytes()).await?;
            stream.flush().await?;
            return Ok(());
        }
    };

    let response = handle_command(command, orchestrator).await;

    let response_json = serde_json::to_string(&response)?;
    stream.write_all(response_json.as_bytes()).await?;
    stream.flush().await?;

    Ok(())
}

/// Handle a parsed command
async fn handle_command(
    command: CliCommand,
    orchestrator: Arc<Mutex<Orchestrator>>,
) -> DaemonEvent {
    match command {
        CliCommand::TaskCreate { description } => {
            let orch = orchestrator.lock().await;
            let task = orch.create_task(description.clone()).await;
            info!(task_id = %task.id, "Task created via CLI");
            DaemonEvent::TaskCreated(task.id)
        }
        CliCommand::TaskApprove { task_id } => {
            let orch = orchestrator.lock().await;
            match orch.start_task(task_id).await {
                Ok(()) => DaemonEvent::TaskStateChanged {
                    id: task_id,
                    state: TaskState::Ready,
                },
                Err(e) => DaemonEvent::Error {
                    message: e.to_string(),
                },
            }
        }
        CliCommand::TaskReject { task_id, reason } => {
            warn!(task_id = %task_id, reason = %reason, "Task rejected");
            DaemonEvent::TaskStateChanged {
                id: task_id,
                state: TaskState::Rejected,
            }
        }
        CliCommand::TaskCancel { task_id } => {
            info!(task_id = %task_id, "Task cancelled");
            DaemonEvent::TaskStateChanged {
                id: task_id,
                state: TaskState::Failed,
            }
        }
        CliCommand::TaskList => {
            let orch = orchestrator.lock().await;
            let tasks = orch.get_all_tasks().await;
            let json = serde_json::to_string(&tasks).unwrap_or_else(|_| "[]".to_string());
            // Send as a special event
            DaemonEvent::TaskCompleted {
                id: uuid::Uuid::nil(), // nil UUID indicates list response
                pr_url: Some(json),
            }
        }
        CliCommand::TaskWatch { task_id } => {
            // For MVP, just return current state
            let orch = orchestrator.lock().await;
            match orch.get_task(task_id).await {
                Ok(task) => DaemonEvent::TaskStateChanged {
                    id: task_id,
                    state: task.state,
                },
                Err(e) => DaemonEvent::Error {
                    message: e.to_string(),
                },
            }
        }
    }
}
