use std::sync::Arc;
use swell_core::ids::SocketPath;
use swell_core::init_tracing;
use swell_core::opentelemetry::{init_tracer_provider, OtelConfig};
use swell_daemon::dashboard::{start_dashboard_server, DashboardState};
use swell_daemon::Daemon;
use swell_llm::{AnthropicBackend, LlmBackend, MockLlm, OpenAIBackend};
use swell_orchestrator::OrchestratorEvent;
use tracing::{info, warn};

/// Construct an LLM backend from environment configuration.
///
/// The backend is selected based on `SWELL_PROVIDER` env var (defaults to "anthropic").
/// Model is configured via `SWELL_MODEL` env var.
/// API keys are read from `ANTHROPIC_API_KEY` or `OPENAI_API_KEY`.
fn construct_llm_backend() -> Arc<dyn LlmBackend> {
    let provider = std::env::var("SWELL_PROVIDER").unwrap_or_else(|_| "anthropic".to_string());
    let model =
        std::env::var("SWELL_MODEL").unwrap_or_else(|_| "claude-sonnet-4-20250514".to_string());

    match provider.as_str() {
        "anthropic" => {
            let api_key = std::env::var("ANTHROPIC_API_KEY").unwrap_or_else(|_| {
                warn!("ANTHROPIC_API_KEY not set, using MockLlm for development");
                "mock".to_string()
            });

            if api_key == "mock" {
                info!(model = %model, "Using MockLlm backend (ANTHROPIC_API_KEY not set)");
                Arc::new(MockLlm::new(&model)) as Arc<dyn LlmBackend>
            } else {
                info!(model = %model, provider = %provider, "Using AnthropicBackend");
                Arc::new(AnthropicBackend::new(&model, &api_key)) as Arc<dyn LlmBackend>
            }
        }
        "openai" => {
            let api_key = std::env::var("OPENAI_API_KEY").unwrap_or_else(|_| {
                warn!("OPENAI_API_KEY not set, using MockLlm for development");
                "mock".to_string()
            });

            if api_key == "mock" {
                info!(model = %model, "Using MockLlm backend (OPENAI_API_KEY not set)");
                Arc::new(MockLlm::new(&model)) as Arc<dyn LlmBackend>
            } else {
                match OpenAIBackend::new(&model, &api_key) {
                    Ok(backend) => {
                        info!(model = %model, provider = %provider, "Using OpenAIBackend");
                        Arc::new(backend) as Arc<dyn LlmBackend>
                    }
                    Err(e) => {
                        warn!(error = %e, model = %model, "Failed to create OpenAIBackend, using MockLlm");
                        Arc::new(MockLlm::new(&model)) as Arc<dyn LlmBackend>
                    }
                }
            }
        }
        _ => {
            warn!(provider = %provider, "Unknown SWELL_PROVIDER, using MockLlm");
            Arc::new(MockLlm::new(&model)) as Arc<dyn LlmBackend>
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_tracing();

    // Initialize OpenTelemetry tracing - this must happen before any LLM execution
    // so that spans created by LLM backends are exported to the configured OTLP endpoint
    let otel_config = OtelConfig::from_env();
    match init_tracer_provider(otel_config.clone()) {
        Ok(_tracer) => {
            info!(
                service_name = %otel_config.service_name,
                otlp_endpoint = ?otel_config.otlp_endpoint,
                "OpenTelemetry tracer provider initialized"
            );
        }
        Err(e) => {
            // Log the error but don't fail startup - we can still run without OTLP export
            warn!(
                error = %e,
                "Failed to initialize OpenTelemetry tracer provider, spans will use no-op provider"
            );
        }
    }

    let socket_path = SocketPath::from_string(
        &std::env::var("SWELL_SOCKET").unwrap_or_else(|_| "/tmp/swell-daemon.sock".to_string()),
    );

    // Dashboard server port (default 3100)
    let dashboard_port: u16 = std::env::var("SWELL_DASHBOARD_PORT")
        .unwrap_or_else(|_| "3100".to_string())
        .parse()
        .unwrap_or(3100);

    // Construct the LLM backend from environment configuration
    let llm_backend = construct_llm_backend();

    info!(path = %socket_path, port = dashboard_port, "Starting swell-daemon");

    let daemon = Arc::new(Daemon::new(socket_path, llm_backend));
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
                    dashboard_for_broadcast
                        .broadcast_event(dashboard_event)
                        .await;
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
        let mut event_rx = orchestrator.subscribe();

        loop {
            match event_rx.recv().await {
                Ok(OrchestratorEvent::AgentRegistered {
                    agent_id,
                    role,
                    model,
                }) => {
                    tracing::debug!(agent_id = %agent_id, role = ?role, model = %model, "Agent registered, updating dashboard");
                    dashboard_for_agents
                        .register_agent(agent_id, role, model)
                        .await;
                }
                Ok(_) => {
                    // Ignore other orchestrator events for now
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    // Events dropped - continue
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    tracing::info!(
                        "Orchestrator event channel closed, stopping agent registration forwarding"
                    );
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
