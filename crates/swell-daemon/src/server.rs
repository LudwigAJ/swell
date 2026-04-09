use std::sync::Arc;
use tokio::net::{UnixListener, UnixStream};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::Mutex;
use swell_core::{CliCommand, DaemonEvent, TaskState};
use swell_orchestrator::Orchestrator;
use tracing::{info, error, warn};

pub struct Daemon {
    orchestrator: Arc<Mutex<Orchestrator>>,
    socket_path: String,
}

impl Daemon {
    pub fn new(socket_path: String) -> Self {
        Self {
            orchestrator: Arc::new(Mutex::new(Orchestrator::new())),
            socket_path,
        }
    }

    pub async fn run(&self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Remove existing socket file
        if std::path::Path::new(&self.socket_path).exists() {
            std::fs::remove_file(&self.socket_path)?;
        }

        let listener = UnixListener::bind(&self.socket_path)?;
        info!(path = %self.socket_path, "Daemon listening");

        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let orchestrator = Arc::clone(&self.orchestrator);
                    tokio::spawn(async move {
                        if let Err(e) = handle_connection(stream, orchestrator).await {
                            error!(error = %e, "Connection handler error");
                        }
                    });
                }
                Err(e) => {
                    error!(error = %e, "Failed to accept connection");
                }
            }
        }
    }
}

async fn handle_connection(
    stream: UnixStream,
    orchestrator: Arc<Mutex<Orchestrator>>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut stream = stream;
    let mut buf = vec![0u8; 4096];
    
    let n = stream.read_buf(&mut buf).await?;
    if n == 0 {
        return Ok(());
    }

    let input = String::from_utf8_lossy(&buf[..n]);
    let command: CliCommand = match serde_json::from_str(&input) {
        Ok(cmd) => cmd,
        Err(e) => {
            let response = serde_json::to_string(&DaemonEvent::Error {
                message: format!("Invalid command: {}", e)
            })?;
            stream.write_all(response.as_bytes()).await?;
            return Ok(());
        }
    };

    let response = match command {
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
                Err(e) => DaemonEvent::Error { message: e.to_string() },
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
                pr_url: Some(json) 
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
                Err(e) => DaemonEvent::Error { message: e.to_string() },
            }
        }
    };

    let response_json = serde_json::to_string(&response)?;
    stream.write_all(response_json.as_bytes()).await?;
    stream.flush().await?;

    Ok(())
}
