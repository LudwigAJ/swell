use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use swell_core::{CliCommand, DaemonEvent, TaskState};
use swell_llm::LlmBackend;
use swell_memory::recall::RecallService;
use swell_memory::SqliteMemoryStore;
use swell_orchestrator::Orchestrator;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{watch, Mutex};
use tokio::time::{interval, timeout, Duration};
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::commands::handle_command;
use crate::events::EventEmitter;

/// Maximum time to wait for active connections to complete during shutdown
const SHUTDOWN_TIMEOUT_SECS: u64 = 30;

/// Polling interval for watch command to check for new events
const WATCH_POLL_INTERVAL_MS: u64 = 500;

/// Graceful shutdown configuration
const SHUTDOWN_BROADCAST_INTERVAL_SECS: u64 = 1;

pub struct Daemon {
    orchestrator: Arc<Mutex<Orchestrator>>,
    event_emitter: Arc<EventEmitter>,
    socket_path: String,
    /// Flag indicating shutdown has been requested
    shutdown_flag: Arc<AtomicBool>,
    /// Counter for active connections
    active_connections: Arc<AtomicUsize>,
    /// Channel to signal shutdown to all tasks
    shutdown_tx: Arc<watch::Sender<bool>>,
    /// Receiver for shutdown signal - shared across spawned tasks
    shutdown_rx: watch::Receiver<bool>,
    /// Time when the daemon was started
    start_time: std::time::Instant,
    /// Memory recall service for querying conversation logs (using Mutex for interior mutability)
    recall_service: Arc<Mutex<Option<RecallService>>>,
}

impl Daemon {
    /// Create a new daemon with the provided LLM backend.
    ///
    /// The LLM backend is threaded into the orchestrator via [`Orchestrator::new`].
    /// This enables the production runtime to construct reachable execution dependencies.
    ///
    /// # Arguments
    /// * `socket_path` - Path to the Unix socket for client connections
    /// * `llm` - The LLM backend to use for agent execution
    pub fn new(socket_path: String, llm: Arc<dyn LlmBackend>) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        Self {
            orchestrator: Arc::new(Mutex::new(Orchestrator::new(llm))),
            event_emitter: Arc::new(EventEmitter::new()),
            socket_path,
            shutdown_flag: Arc::new(AtomicBool::new(false)),
            active_connections: Arc::new(AtomicUsize::new(0)),
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
            start_time: std::time::Instant::now(),
            recall_service: Arc::new(Mutex::new(None)),
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

    /// Get the event emitter for the daemon
    pub fn event_emitter(&self) -> Arc<EventEmitter> {
        Arc::clone(&self.event_emitter)
    }

    /// Get the orchestrator for the daemon
    pub fn orchestrator(&self) -> Arc<Mutex<Orchestrator>> {
        Arc::clone(&self.orchestrator)
    }

    /// Get the active connections counter for the daemon
    pub fn active_connections(&self) -> Arc<AtomicUsize> {
        Arc::clone(&self.active_connections)
    }

    /// Get the daemon start time
    pub fn start_time(&self) -> std::time::Instant {
        self.start_time
    }

    /// Get the daemon uptime as a Duration
    pub fn uptime(&self) -> std::time::Duration {
        self.start_time.elapsed()
    }

    /// Get the recall service (if initialized)
    pub fn recall_service(&self) -> Arc<Mutex<Option<RecallService>>> {
        Arc::clone(&self.recall_service)
    }

    pub async fn run(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Remove existing socket file
        if std::path::Path::new(&self.socket_path).exists() {
            std::fs::remove_file(&self.socket_path)?;
        }

        // Initialize the memory recall service
        let swell_dir = std::env::current_dir()
            .map(|p| p.join(".swell"))
            .unwrap_or_else(|_| std::path::PathBuf::from(".swell"));
        let memory_db_path = swell_dir.join("memory.db");
        let database_url = format!("sqlite:{}?mode=rwc", memory_db_path.display());

        match SqliteMemoryStore::create(&database_url).await {
            Ok(store) => {
                // The conversation_logs schema is now initialized automatically in create()
                let recall = RecallService::new(store);
                // Store the recall service using the Mutex
                let mut guard = self.recall_service.lock().await;
                *guard = Some(recall);
                info!(path = %memory_db_path.display(), "Memory recall service initialized");
            }
            Err(e) => {
                warn!(error = %e, path = %memory_db_path.display(), "Failed to initialize memory store, MemoryQuery will not be available");
            }
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
            match timeout(
                Duration::from_secs(SHUTDOWN_BROADCAST_INTERVAL_SECS),
                listener.accept(),
            )
            .await
            {
                Ok(Ok((stream, _))) => {
                    // Increment active connections
                    self.active_connections.fetch_add(1, Ordering::Relaxed);
                    let active_connections = Arc::clone(&self.active_connections);
                    let shutdown_rx = self.shutdown_rx.clone();
                    let orchestrator = Arc::clone(&self.orchestrator);
                    let event_emitter = Arc::clone(&self.event_emitter);
                    let recall_service = Arc::clone(&self.recall_service);
                    let start_time = self.start_time;

                    tokio::spawn(async move {
                        let result = handle_connection_with_shutdown(
                            stream,
                            orchestrator,
                            event_emitter,
                            shutdown_rx,
                            Arc::clone(&active_connections),
                            start_time,
                            recall_service,
                        )
                        .await;
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
    async fn wait_for_active_connections(
        &self,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
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
                Err(e) => {
                    warn!(path = %self.socket_path, error = %e, "Failed to remove socket file")
                }
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
    event_emitter: Arc<EventEmitter>,
    mut shutdown_rx: watch::Receiver<bool>,
    active_connections: Arc<AtomicUsize>,
    start_time: std::time::Instant,
    recall_service: Arc<Mutex<Option<RecallService>>>,
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
            let correlation_id = EventEmitter::new_correlation_id();
            let response = DaemonEvent::Error {
                message: format!("Invalid command: {}", e),
                failure_class: None,
                correlation_id,
            };
            let response_json = serde_json::to_string(&response)?;
            stream.write_all(response_json.as_bytes()).await?;
            stream.flush().await?;
            return Ok(());
        }
    };

    // Handle TaskWatch specially - stream events instead of single response
    if let CliCommand::TaskWatch { task_id } = command {
        handle_watch_connection(stream, task_id, orchestrator, event_emitter, shutdown_rx).await?;
        return Ok(());
    }

    let response = handle_command(
        command,
        orchestrator,
        event_emitter,
        active_connections,
        start_time,
        recall_service,
    )
    .await;

    let response_json = serde_json::to_string(&response)?;
    stream.write_all(response_json.as_bytes()).await?;
    stream.flush().await?;

    Ok(())
}

/// Handle a watch connection that streams events for a specific task.
/// Sends the current state immediately, then polls for new events and streams them.
async fn handle_watch_connection(
    mut stream: UnixStream,
    task_id: Uuid,
    orchestrator: Arc<Mutex<Orchestrator>>,
    event_emitter: Arc<EventEmitter>,
    mut shutdown_rx: watch::Receiver<bool>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    info!(task_id = %task_id, "Starting watch connection");

    // Verify task exists and get current state
    let current_state = {
        let orch = orchestrator.lock().await;
        match orch.get_task(task_id).await {
            Ok(task) => Some(task.state),
            Err(_) => None,
        }
    };

    // Send initial state or error
    let initial_event = match current_state {
        Some(state) => {
            let correlation_id = EventEmitter::new_correlation_id();
            DaemonEvent::TaskStateChanged {
                id: task_id,
                state,
                correlation_id,
            }
        }
        None => {
            let correlation_id = EventEmitter::new_correlation_id();
            DaemonEvent::Error {
                message: format!("Task not found: {}", task_id),
                failure_class: None,
                correlation_id,
            }
        }
    };

    let initial_json = serde_json::to_string(&initial_event)?;
    stream.write_all(initial_json.as_bytes()).await?;
    stream.write_all(b"\n").await?; // Delimiter for streaming
    stream.flush().await?;

    // If task not found, exit immediately
    if current_state.is_none() {
        return Ok(());
    }

    // Track terminal states - once task reaches these, stop watching
    let terminal_states = [
        TaskState::Accepted,
        TaskState::Rejected,
        TaskState::Failed,
        TaskState::Escalated,
    ];

    // Get initial sequence number AFTER sending initial state to ensure we capture
    // all events that happened before or during the initial state send.
    // Subtract 1 because current_sequence() returns the next sequence to be assigned,
    // and we want to capture events at that sequence number (not just after it).
    let initial_sequence = event_emitter.current_sequence().await.saturating_sub(1);

    // Poll for new events
    let mut poll_interval = interval(Duration::from_millis(WATCH_POLL_INTERVAL_MS));
    let mut last_sequence = initial_sequence;

    loop {
        tokio::select! {
            _ = poll_interval.tick() => {
                // Check for new events for this task
                let new_events = event_emitter.get_events_since_for_task(task_id, last_sequence).await;

                // Track the sequence number to advance to after processing
                // Events are ordered by sequence, so we can use the last event's sequence
                let mut reached_terminal = false;

                for event in new_events {
                    let json = serde_json::to_string(&event)?;
                    stream.write_all(json.as_bytes()).await?;
                    stream.write_all(b"\n").await?;
                    stream.flush().await?;

                    // If this is a state change, check for terminal state
                    if let DaemonEvent::TaskStateChanged { state, .. } = &event {
                        // Check if task reached terminal state
                        if terminal_states.contains(state) {
                            info!(task_id = %task_id, state = ?state, "Task reached terminal state, ending watch");
                            reached_terminal = true;
                            break;
                        }
                    }
                }

                if reached_terminal {
                    return Ok(());
                }

                // Update last_sequence to avoid reprocessing events
                // Use the current sequence which is >= all events we've seen
                last_sequence = event_emitter.current_sequence().await;
            }
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    info!("Watch connection closed - shutdown in progress");
                    return Ok(());
                }
            }
        }
    }
}
