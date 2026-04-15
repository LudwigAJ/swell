//! Dashboard API module providing REST and WebSocket endpoints
//! for real-time monitoring of tasks, agents, cost, and events.
//!
//! # REST Endpoints
//!
//! - `GET /api/tasks` - List all tasks with optional state filter
//! - `GET /api/tasks/:id` - Get task details by ID
//! - `GET /api/agents` - List all registered agents
//! - `GET /api/cost` - Get cost tracking information
//! - `GET /api/events` - Get recent events (paginated)
//!
//! # WebSocket
//!
//! - `WS /ws` - Real-time event streaming

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path, Query, State,
    },
    http::StatusCode,
    response::{Json, Response},
    routing::get,
    Router,
};
use chrono::{DateTime, Utc};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use tokio::sync::{broadcast, RwLock};
use tokio::time::Duration;
use uuid::Uuid;

use crate::events::EventEmitter;
use crate::server::Daemon;
use swell_core::{
    get_last_llm_model, get_total_llm_tokens, AgentRole, DaemonEvent, Task, TaskState,
};

/// Dashboard API state shared across all request handlers
#[derive(Clone)]
pub struct DashboardState {
    /// Reference to the event emitter for reading events
    event_emitter: Arc<EventEmitter>,
    /// Broadcast channel for real-time event updates to WebSocket clients
    event_broadcaster: Arc<RwLock<broadcast::Sender<DashboardEvent>>>,
    /// Cost tracking state
    cost_state: Arc<RwLock<CostState>>,
    /// Agent registry for dashboard
    agents: Arc<RwLock<HashMap<Uuid, AgentInfo>>>,
}

impl DashboardState {
    pub fn new(event_emitter: Arc<EventEmitter>) -> Self {
        let (tx, _) = broadcast::channel(1000);
        Self {
            event_emitter,
            event_broadcaster: Arc::new(RwLock::new(tx)),
            cost_state: Arc::new(RwLock::new(CostState::default())),
            agents: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Broadcast an event to all connected WebSocket clients
    pub async fn broadcast_event(&self, event: DashboardEvent) {
        let _ = self.event_broadcaster.write().await.send(event);
    }

    /// Update cost tracking
    pub async fn update_cost(&self, tokens_used: u64, model: &str) {
        let mut cost = self.cost_state.write().await;
        cost.total_tokens += tokens_used;
        cost.last_updated = Utc::now();

        // Update per-model breakdown
        let entry = cost
            .by_model
            .entry(model.to_string())
            .or_insert_with(|| ModelCost {
                model: model.to_string(),
                tokens: 0,
                estimated_cost_usd: 0.0,
            });
        entry.tokens += tokens_used;
        // Rough estimate: $3.5/1M tokens for Claude 3.5 Sonnet
        entry.estimated_cost_usd = entry.tokens as f64 * 3.5 / 1_000_000.0;
        cost.total_estimated_cost_usd = cost.by_model.values().map(|m| m.estimated_cost_usd).sum();
    }

    /// Register an agent
    pub async fn register_agent(&self, id: Uuid, role: AgentRole, model: String) {
        let mut agents = self.agents.write().await;
        agents.insert(
            id,
            AgentInfo {
                id,
                role,
                model,
                current_task: None,
                registered_at: Utc::now(),
            },
        );
    }

    /// Update agent's current task
    pub async fn update_agent_task(&self, agent_id: Uuid, task_id: Option<Uuid>) {
        let mut agents = self.agents.write().await;
        if let Some(agent) = agents.get_mut(&agent_id) {
            agent.current_task = task_id;
        }
    }

    /// Sync cost data from global tracker into local cost state.
    /// Call this periodically to ensure /api/cost reflects live LLM usage.
    pub async fn sync_cost_from_global(&self) {
        // Use swell_core's global tracker functions (sync call in async context)
        let total = get_total_llm_tokens();
        let model = get_last_llm_model();

        // Update local cost state with global total
        let mut cost = self.cost_state.write().await;
        if total > cost.total_tokens {
            let delta = total - cost.total_tokens;
            cost.total_tokens = total;
            cost.last_updated = Utc::now();
            // Update per-model using last known model
            let entry = cost
                .by_model
                .entry(model.clone())
                .or_insert_with(|| ModelCost {
                    model: model.clone(),
                    tokens: 0,
                    estimated_cost_usd: 0.0,
                });
            entry.tokens = entry.tokens.saturating_add(delta);
            entry.estimated_cost_usd = entry.tokens as f64 * 3.5 / 1_000_000.0;
            cost.total_estimated_cost_usd =
                cost.by_model.values().map(|m| m.estimated_cost_usd).sum();
        }
    }
}

/// Cost tracking state
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CostState {
    pub total_tokens: u64,
    pub total_estimated_cost_usd: f64,
    pub by_model: HashMap<String, ModelCost>,
    pub last_updated: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCost {
    pub model: String,
    pub tokens: u64,
    pub estimated_cost_usd: f64,
}

/// Agent information for dashboard
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentInfo {
    pub id: Uuid,
    pub role: AgentRole,
    pub model: String,
    pub current_task: Option<Uuid>,
    pub registered_at: DateTime<Utc>,
}

/// Dashboard event types for WebSocket streaming
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum DashboardEvent {
    /// A task was created
    TaskCreated {
        id: Uuid,
        description: String,
        correlation_id: Uuid,
    },
    /// Task state changed
    TaskStateChanged {
        id: Uuid,
        state: TaskState,
        correlation_id: Uuid,
    },
    /// Task progress update
    TaskProgress {
        id: Uuid,
        message: String,
        correlation_id: Uuid,
    },
    /// Task completed
    TaskCompleted {
        id: Uuid,
        pr_url: Option<String>,
        correlation_id: Uuid,
    },
    /// Task failed
    TaskFailed {
        id: Uuid,
        error: String,
        correlation_id: Uuid,
    },
    /// Agent registered
    AgentRegistered { id: Uuid, role: String },
    /// Agent task assigned
    AgentTaskAssigned { agent_id: Uuid, task_id: Uuid },
    /// Cost update
    CostUpdated {
        total_tokens: u64,
        estimated_cost_usd: f64,
    },
    /// Error occurred
    Error {
        message: String,
        correlation_id: Uuid,
    },
}

impl From<DaemonEvent> for DashboardEvent {
    fn from(event: DaemonEvent) -> Self {
        match event {
            DaemonEvent::TaskCreated { id, correlation_id } => {
                DashboardEvent::TaskCreated {
                    id,
                    description: String::new(), // Task description not in event
                    correlation_id,
                }
            }
            DaemonEvent::TaskStateChanged {
                id,
                state,
                correlation_id,
            } => DashboardEvent::TaskStateChanged {
                id,
                state,
                correlation_id,
            },
            DaemonEvent::TaskProgress {
                id,
                message,
                correlation_id,
            } => DashboardEvent::TaskProgress {
                id,
                message,
                correlation_id,
            },
            DaemonEvent::TaskCompleted {
                id,
                pr_url,
                correlation_id,
            } => DashboardEvent::TaskCompleted {
                id,
                pr_url,
                correlation_id,
            },
            DaemonEvent::TaskFailed {
                id,
                error,
                failure_class: _,
                correlation_id,
            } => DashboardEvent::TaskFailed {
                id,
                error,
                correlation_id,
            },
            DaemonEvent::Error {
                message,
                failure_class: _,
                correlation_id,
            } => DashboardEvent::Error {
                message,
                correlation_id,
            },
            // Per-turn events are mapped to TaskProgress for dashboard display
            // These are lower-level observability events that provide fine-grained progress
            DaemonEvent::ToolInvocationStarted {
                id,
                tool_name,
                turn_number,
                correlation_id,
                ..
            } => DashboardEvent::TaskProgress {
                id,
                message: format!("[Turn {}] Invoking tool '{}'", turn_number, tool_name),
                correlation_id,
            },
            DaemonEvent::ToolInvocationCompleted {
                id,
                tool_name,
                success,
                duration_ms,
                turn_number,
                correlation_id,
                ..
            } => DashboardEvent::TaskProgress {
                id,
                message: format!(
                    "[Turn {}] Tool '{}' completed ({} in {}ms)",
                    turn_number,
                    tool_name,
                    if success { "success" } else { "failed" },
                    duration_ms
                ),
                correlation_id,
            },
            DaemonEvent::AgentTurnStarted {
                id,
                agent_role,
                turn_number,
                correlation_id,
                ..
            } => DashboardEvent::TaskProgress {
                id,
                message: format!(
                    "[Turn {}] Agent '{}' starting turn",
                    turn_number, agent_role
                ),
                correlation_id,
            },
            DaemonEvent::AgentTurnCompleted {
                id,
                agent_role,
                turn_number,
                action_taken,
                tools_invoked,
                duration_ms,
                correlation_id,
                ..
            } => DashboardEvent::TaskProgress {
                id,
                message: format!(
                    "[Turn {}] Agent '{}' completed - {} ({} tools invoked, {}ms)",
                    turn_number,
                    agent_role,
                    action_taken,
                    tools_invoked.len(),
                    duration_ms
                ),
                correlation_id,
            },
            DaemonEvent::ValidationStepStarted {
                id,
                step_name,
                correlation_id,
                ..
            } => DashboardEvent::TaskProgress {
                id,
                message: format!("Starting validation: '{}'", step_name),
                correlation_id,
            },
            DaemonEvent::ValidationStepCompleted {
                id,
                step_name,
                passed,
                duration_ms,
                correlation_id,
                ..
            } => DashboardEvent::TaskProgress {
                id,
                message: format!(
                    "Validation '{}' {} ({}ms)",
                    step_name,
                    if passed { "passed" } else { "failed" },
                    duration_ms
                ),
                correlation_id,
            },
            // TaskDetails contains full task JSON - emit as progress with task ID
            DaemonEvent::TaskDetails {
                id,
                task_json,
                correlation_id,
            } => DashboardEvent::TaskProgress {
                id,
                message: format!("Task details retrieved ({} bytes)", task_json.len()),
                correlation_id,
            },
            // DaemonHealth - emit as system progress with uptime info
            DaemonEvent::DaemonHealth {
                uptime_seconds,
                total_tasks,
                ..
            } => DashboardEvent::TaskProgress {
                id: Uuid::nil(),
                message: format!("Daemon health: {} tasks, {}s uptime", total_tasks, uptime_seconds),
                correlation_id: Uuid::nil(),
            },
            // ConfigValue - emit as progress with config key and value
            DaemonEvent::ConfigValue {
                key,
                value,
                source_file,
                correlation_id,
            } => DashboardEvent::TaskProgress {
                id: Uuid::nil(),
                message: format!(
                    "Config '{}' = {} (from: {:?})",
                    key,
                    value,
                    source_file.as_deref().unwrap_or("unknown")
                ),
                correlation_id,
            },
            // MemoryResults - emit as progress with count info
            DaemonEvent::MemoryResults {
                results,
                count,
                correlation_id,
            } => DashboardEvent::TaskProgress {
                id: Uuid::nil(),
                message: format!("Memory query returned {} results ({} bytes)", count, results.len()),
                correlation_id,
            },
        }
    }
}

// ============================================================================
// REST API Handlers
// ============================================================================

/// Query parameters for task listing
#[derive(Debug, Deserialize)]
pub struct TaskListQuery {
    pub state: Option<TaskState>,
    pub limit: Option<usize>,
}

/// GET /api/tasks - List all tasks
async fn list_tasks(
    State(state): State<AppState>,
    Query(query): Query<TaskListQuery>,
) -> Json<Vec<Task>> {
    let orch_arc = state.daemon.orchestrator();
    let orchestrator = orch_arc.lock().await;
    let tasks = orchestrator.get_all_tasks().await;
    drop(orchestrator);

    let filtered: Vec<Task> = if let Some(state_filter) = query.state {
        tasks
            .into_iter()
            .filter(|t| t.state == state_filter)
            .collect()
    } else {
        tasks
    };

    let limited: Vec<Task> = if let Some(limit) = query.limit {
        filtered.into_iter().take(limit).collect()
    } else {
        filtered
    };

    Json(limited)
}

/// GET /api/tasks/:id - Get task details
async fn get_task(
    State(state): State<AppState>,
    Path(task_id): Path<Uuid>,
) -> Result<Json<Task>, StatusCode> {
    let orch_arc = state.daemon.orchestrator();
    let orchestrator = orch_arc.lock().await;
    let result = orchestrator.get_task(task_id).await;
    drop(orchestrator);

    match result {
        Ok(task) => Ok(Json(task)),
        Err(_) => Err(StatusCode::NOT_FOUND),
    }
}

/// GET /api/agents - List all registered agents
async fn list_agents(State(state): State<AppState>) -> Json<Vec<AgentInfo>> {
    let agents = state.dashboard.agents.read().await;
    Json(agents.values().cloned().collect())
}

/// GET /api/cost - Get cost information
async fn get_cost(State(state): State<AppState>) -> Json<CostState> {
    let cost = state.dashboard.cost_state.read().await;
    Json(cost.clone())
}

/// Query parameters for events listing
#[derive(Debug, Deserialize)]
pub struct EventListQuery {
    pub limit: Option<usize>,
    pub since_sequence: Option<u64>,
}

/// GET /api/events - Get recent events
async fn list_events(
    State(state): State<AppState>,
    Query(query): Query<EventListQuery>,
) -> Json<Vec<DaemonEvent>> {
    let limit = query.limit.unwrap_or(100);
    let events = state.dashboard.event_emitter.get_all_events().await;
    let limited: Vec<DaemonEvent> = events.into_iter().rev().take(limit).collect();
    Json(limited)
}

/// Health check endpoint
async fn health_check() -> &'static str {
    "OK"
}

// ============================================================================
// WebSocket Handler
// ============================================================================

/// WebSocket connection handler
async fn websocket_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> Response {
    let dashboard = Arc::clone(&state.dashboard);
    ws.on_upgrade(|socket| websocket_stream(socket, dashboard))
}

/// Handle WebSocket connection and stream events
async fn websocket_stream(socket: WebSocket, dashboard: Arc<DashboardState>) {
    let (mut sender, mut receiver) = socket.split();
    let mut broadcast_rx = dashboard.event_broadcaster.read().await.subscribe();

    // Send initial connection success message
    let welcome = serde_json::json!({
        "type": "Connected",
        "message": "Dashboard WebSocket connected"
    });
    if sender
        .send(Message::Text(welcome.to_string()))
        .await
        .is_err()
    {
        return;
    }

    // Main loop: stream events to client and handle incoming messages
    loop {
        tokio::select! {
            // Receive broadcast events
            event = broadcast_rx.recv() => {
                match event {
                    Ok(dashboard_event) => {
                        let json = serde_json::to_string(&dashboard_event).unwrap_or_default();
                        if sender.send(Message::Text(json)).await.is_err() {
                            return;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(skipped = n, "WebSocket lagged behind broadcast");
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        break;
                    }
                }
            }
            // Receive messages from client
            msg = receiver.next() => {
                match msg {
                    Some(Ok(Message::Ping(data))) => {
                        let _ = sender.send(Message::Pong(data)).await;
                    }
                    Some(Ok(Message::Text(text))) => {
                        tracing::debug!(msg = %text, "WebSocket received text message");
                    }
                    Some(Ok(Message::Close(_))) | Some(Err(_)) | None => {
                        break;
                    }
                    _ => {}
                }
            }
            // Keepalive ping every 30 seconds
            _ = tokio::time::sleep(Duration::from_secs(30)) => {
                if sender.send(Message::Ping(vec![1])).await.is_err() {
                    return;
                }
            }
        }
    }

    // Final close
    let _ = sender.close().await;
}

// ============================================================================
// Server Startup
// ============================================================================

/// Start the Dashboard HTTP server
pub async fn start_dashboard_server(
    daemon: Arc<Daemon>,
    dashboard_state: DashboardState,
    port: u16,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let dashboard = Arc::new(dashboard_state);

    // Create shared state combining daemon and dashboard
    let app_state = AppState { daemon, dashboard };

    let app = Router::new()
        .route("/api/tasks", get(list_tasks))
        .route("/api/tasks/:id", get(get_task))
        .route("/api/agents", get(list_agents))
        .route("/api/cost", get(get_cost))
        .route("/api/events", get(list_events))
        .route("/health", get(health_check))
        .route("/ws", get(websocket_handler))
        .with_state(app_state);

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    let listener = tokio::net::TcpListener::bind(addr).await?;

    tracing::info!(
        port = port,
        "Dashboard API server listening on http://localhost:{}",
        port
    );

    let service = axum::serve(listener, app);

    // Run the server
    if let Err(e) = service.await {
        tracing::error!(error = %e, "Dashboard server error");
        return Err(Box::new(e));
    }

    Ok(())
}

/// Combined application state for REST handlers
#[derive(Clone)]
pub struct AppState {
    pub daemon: Arc<Daemon>,
    pub dashboard: Arc<DashboardState>,
}

impl AppState {
    pub fn new(daemon: Arc<Daemon>, dashboard: DashboardState) -> Self {
        Self {
            daemon,
            dashboard: Arc::new(dashboard),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::EventEmitter;
    use std::sync::Arc;

    fn create_test_dashboard_state() -> DashboardState {
        let emitter = Arc::new(EventEmitter::new());
        DashboardState::new(emitter)
    }

    #[tokio::test]
    async fn test_dashboard_state_broadcast() {
        let state = create_test_dashboard_state();

        // Subscribe BEFORE broadcasting to receive the event
        let mut rx = state.event_broadcaster.read().await.subscribe();

        let event = DashboardEvent::Error {
            message: "Test error".to_string(),
            correlation_id: Uuid::new_v4(),
        };

        // Broadcast should not panic
        state.broadcast_event(event.clone()).await;

        // Check that receiver gets the event
        let received = rx.recv().await.unwrap();
        match received {
            DashboardEvent::Error { message, .. } => {
                assert_eq!(message, "Test error");
            }
            _ => panic!("Expected Error event"),
        }
    }

    #[tokio::test]
    async fn test_cost_update() {
        let state = create_test_dashboard_state();

        state.update_cost(1000, "claude-3-5-sonnet-20241022").await;

        let cost = state.cost_state.read().await;
        assert_eq!(cost.total_tokens, 1000);
        assert!(cost.total_estimated_cost_usd > 0.0);
    }

    #[tokio::test]
    async fn test_agent_registration() {
        let state = create_test_dashboard_state();
        let agent_id = Uuid::new_v4();

        state
            .register_agent(agent_id, AgentRole::Planner, "claude-sonnet-4".to_string())
            .await;

        let agents = state.agents.read().await;
        assert!(agents.contains_key(&agent_id));
        let agent = agents.get(&agent_id).unwrap();
        assert_eq!(agent.role, AgentRole::Planner);
        assert_eq!(agent.model, "claude-sonnet-4");
    }

    #[tokio::test]
    async fn test_daemon_event_to_dashboard_event() {
        let correlation_id = Uuid::new_v4();
        let task_id = Uuid::new_v4();

        let daemon_event = DaemonEvent::TaskStateChanged {
            id: task_id,
            state: TaskState::Executing,
            correlation_id,
        };

        let dashboard_event: DashboardEvent = daemon_event.into();

        match dashboard_event {
            DashboardEvent::TaskStateChanged {
                id,
                state,
                correlation_id: cid,
            } => {
                assert_eq!(id, task_id);
                assert_eq!(state, TaskState::Executing);
                assert_eq!(cid, correlation_id);
            }
            _ => panic!("Expected TaskStateChanged event"),
        }
    }
}
