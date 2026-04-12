use std::sync::Arc;
use swell_core::init_tracing;
use swell_daemon::dashboard::{start_dashboard_server, DashboardState};
use swell_daemon::Daemon;
use swell_orchestrator::OrchestratorEvent;
use tracing::info;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_tracing();

    let socket_path =
        std::env::var("SWELL_SOCKET").unwrap_or_else(|_| "/tmp/swell-daemon.sock".to_string());

    // Dashboard server port (default 3100)
    let dashboard_port: u16 = std::env::var("SWELL_DASHBOARD_PORT")
        .unwrap_or_else(|_| "3100".to_string())
        .parse()
        .unwrap_or(3100);

    info!(path = %socket_path, port = dashboard_port, "Starting swell-daemon");

    let daemon = Arc::new(Daemon::new(socket_path));
    let dashboard_state = DashboardState::new(daemon.event_emitter());

    // Clone for the dashboard task before moving into spawned task
    let dashboard_state_for_events = Arc::new(dashboard_state.clone());

    // Spawn event subscription task that converts daemon events to DashboardEvents
    // and broadcasts them via the dashboard state
    let dashboard_for_broadcast = Arc::clone(&dashboard_state_for_events);
    let event_emitter = daemon.event_emitter();
    tokio::spawn(async move {
        // Subscribe to all daemon events for WebSocket broadcasting
        let mut event_rx = event_emitter.subscribe().await;

        loop {
            match event_rx.recv().await {
                Ok(daemon_event) => {
                    let dashboard_event = daemon_event.into();
                    dashboard_for_broadcast.broadcast_event(dashboard_event).await;
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    // Events dropped - that's OK for dashboard, we just continue
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    tracing::info!("Event emitter closed, stopping dashboard event forwarding");
                    break;
                }
            }
        }
    });

    // Spawn task to handle OrchestratorEvent::AgentRegistered and update dashboard
    let dashboard_for_agents = Arc::clone(&dashboard_state_for_events);
    let orchestrator = daemon.orchestrator();
    tokio::spawn(async move {
        // Subscribe to orchestrator events for agent registration
        let orch = orchestrator.lock().await;
        let mut event_rx = orch.subscribe();

        loop {
            match event_rx.recv().await {
                Ok(OrchestratorEvent::AgentRegistered { agent_id, role, model }) => {
                    tracing::debug!(agent_id = %agent_id, role = ?role, model = %model, "Agent registered, updating dashboard");
                    dashboard_for_agents.register_agent(agent_id, role, model).await;
                }
                Ok(_) => {
                    // Ignore other orchestrator events for now
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    // Events dropped - continue
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    tracing::info!("Orchestrator event channel closed, stopping agent registration forwarding");
                    break;
                }
            }
        }
    });

    // Also spawn a task to sync global cost data into dashboard state periodically
    let dashboard_for_cost = Arc::clone(&dashboard_state_for_events);
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(5));
        loop {
            interval.tick().await;
            dashboard_for_cost.sync_cost_from_global().await;
        }
    });

    // Start both servers concurrently
    let daemon_run = daemon.run();
    let dashboard_run =
        start_dashboard_server(Arc::clone(&daemon), dashboard_state, dashboard_port);

    tokio::select! {
        result = daemon_run => {
            if let Err(e) = result {
                eprintln!("Daemon error: {}", e);
                std::process::exit(1);
            }
        }
        result = dashboard_run => {
            if let Err(e) = result {
                eprintln!("Dashboard server error: {}", e);
                std::process::exit(1);
            }
        }
    }

    Ok(())
}
