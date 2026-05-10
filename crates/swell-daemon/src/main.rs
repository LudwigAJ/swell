use std::sync::Arc;
use swell_core::config::ConfigLoader;
use swell_core::ids::SocketPath;
use swell_core::init_tracing;
use swell_core::llm_config::{LlmBackendKind, LlmConfig, ResolvedProfile};
use swell_core::opentelemetry::{init_tracer_provider, OtelConfig};
use swell_daemon::dashboard::{start_dashboard_server, DashboardState};
use swell_daemon::Daemon;
use swell_llm::{AnthropicBackend, AnthropicProvider, LlmBackend, MockLlm};
use swell_orchestrator::OrchestratorEvent;
use tracing::{info, warn};

/// Plan-shaped JSON the dev-mode `MockLlm` returns. The PlannerAgent
/// parses this directly as its plan output, so a smoke run actually
/// reaches the Generator/Evaluator stages instead of bouncing on a
/// JSON parse error at planning time. Keep the shape minimal — the
/// fields that PlannerAgent reads are `steps[]`,
/// `total_estimated_tokens`, and `risk_assessment`.
const MOCK_PLAN_RESPONSE: &str = r#"{
    "steps": [
        {
            "description": "Smoke-test placeholder step",
            "affected_files": [],
            "expected_tests": [],
            "risk_level": "low"
        }
    ],
    "total_estimated_tokens": 100,
    "risk_assessment": "Smoke test (MockLlm)"
}"#;

fn mock_llm(model: &str) -> Arc<dyn LlmBackend> {
    Arc::new(MockLlm::with_response(model, MOCK_PLAN_RESPONSE)) as Arc<dyn LlmBackend>
}

/// Construct an LLM backend by reading the `llm` block from the
/// layered config (`~/.swell/settings.json`, `.swell/settings.json`,
/// `.swell/settings.local.json`, …). `${VAR}` references inside
/// `env.API_KEY` are resolved against the process environment, so
/// secrets stay out of the JSON file.
///
/// Falls back to `MockLlm` when no `llm` section is present, the
/// referenced env var is missing, or the resolved profile fails to
/// produce a working backend. This is the dev-mode path the smoke
/// test exercises.
fn construct_llm_backend() -> Arc<dyn LlmBackend> {
    let loaded = match ConfigLoader::new().load() {
        Ok(c) => c,
        Err(e) => {
            warn!(error = %e, "Failed to load settings; using MockLlm");
            return mock_llm("mock");
        }
    };

    let llm_value = match loaded.get("llm") {
        Some(v) => v.clone(),
        None => {
            info!("No `llm` section in settings; using MockLlm");
            return mock_llm("mock");
        }
    };

    let llm_cfg: LlmConfig = match serde_json::from_value(llm_value) {
        Ok(c) => c,
        Err(e) => {
            warn!(error = %e, "Malformed `llm` section in settings; using MockLlm");
            return mock_llm("mock");
        }
    };

    let resolved = match llm_cfg.resolve(None) {
        Ok(r) => r,
        Err(e) => {
            warn!(error = %e, "Failed to resolve LLM profile; using MockLlm");
            return mock_llm("mock");
        }
    };

    backend_from_profile(resolved)
}

fn backend_from_profile(p: ResolvedProfile) -> Arc<dyn LlmBackend> {
    match p.backend {
        LlmBackendKind::Anthropic => {
            // Capability gating priority:
            //   1. explicit `llm.models.<alias>.provider` in .swell  (durable)
            //   2. URL substring detection                            (fallback)
            //   3. Anthropic native                                   (default)
            let pinned = p.provider.as_deref().and_then(|name| {
                let parsed = AnthropicProvider::from_settings_name(name);
                if parsed.is_none() {
                    warn!(
                        alias = %p.alias,
                        provider = %name,
                        "Unknown provider name in .swell; falling back to URL detection"
                    );
                }
                parsed
            });
            info!(
                alias = %p.alias,
                model = %p.model,
                base_url = ?p.base_url,
                provider = ?pinned.as_ref().map(|p| p.name()),
                "Using AnthropicBackend"
            );
            // Resolve the provider profile: explicit pin wins, else URL
            // detection, else the Anthropic default. Then layer the
            // user's per-field caps overrides (`llm.models.<alias>.caps`)
            // on top — that's how unrecognised gateways get configured
            // without us shipping a code change.
            let provider = pinned
                .clone()
                .unwrap_or_else(|| swell_llm::AnthropicProvider::detect(p.base_url.as_deref()));
            let backend = if p.caps.is_empty() {
                match (pinned, p.base_url.clone()) {
                    (Some(prov), base_url) => {
                        AnthropicBackend::with_provider(&p.model, &p.api_key, base_url, prov)
                    }
                    (None, Some(url)) => {
                        AnthropicBackend::with_base_url(&p.model, &p.api_key, &url)
                    }
                    (None, None) => AnthropicBackend::new(&p.model, &p.api_key),
                }
            } else {
                info!(alias = %p.alias, "Applying user caps overrides from .swell");
                AnthropicBackend::with_provider_caps_and_retry(
                    &p.model,
                    &p.api_key,
                    p.base_url.clone(),
                    provider,
                    &p.caps,
                    swell_llm::LlmRetryConfig::default(),
                )
            };
            // Best-effort startup model validation. Spawned so a slow or
            // unavailable /v1/models endpoint does not delay daemon startup.
            let validator = backend.clone();
            tokio::spawn(async move {
                validator.warn_if_unknown_model().await;
            });
            Arc::new(backend) as Arc<dyn LlmBackend>
        }
        LlmBackendKind::Openai => {
            // OpenAI backend is parked. The legacy hand-rolled client in
            // `swell-llm/src/openai.rs` is intentionally not wired in
            // until a community OpenAI Rust SDK is available — see the
            // doc comment on `LlmBackendKind::Openai`. To use OpenAI-
            // shaped models today, route them through an Anthropic-
            // compatible gateway (e.g. via OpenRouter) and configure
            // them with `backend = "anthropic"`.
            warn!(
                alias = %p.alias,
                model = %p.model,
                "backend = \"openai\" is not currently supported (waiting on a community OpenAI Rust SDK). \
                 Falling back to MockLlm. Configure the model via `backend = \"anthropic\"` against an \
                 Anthropic-compatible gateway instead."
            );
            mock_llm(&p.model)
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

    // Register the default tool set on the orchestrator's shared
    // registry so agents can actually invoke `read_file`,
    // `write_file`, `edit_file`, `shell`, and `search`. Without this
    // the registry is empty and every tool call fails with
    // `Tool not found`.
    daemon.orchestrator().register_default_tools().await;

    // Best-effort MCP tool registration. No-ops if .swell/mcp.json is absent.
    let mcp_config_path = std::env::var("SWELL_MCP_CONFIG")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from(".swell/mcp.json"));
    daemon
        .orchestrator()
        .register_mcp_tools(mcp_config_path)
        .await;

    let dashboard_state = DashboardState::new(daemon.event_emitter());

    // Clone for the dashboard task before moving into spawned task
    let dashboard_state_for_events = Arc::new(dashboard_state.clone());

    // Spawn event subscription task that converts daemon events to DashboardEvents
    // and broadcasts them via the dashboard state
    let dashboard_for_broadcast = Arc::clone(&dashboard_state_for_events);
    let event_emitter = daemon.event_emitter();
    tokio::spawn(async move {
        // Subscribe to all daemon events for WebSocket broadcasting
        let mut event_rx = event_emitter.subscribe();

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
