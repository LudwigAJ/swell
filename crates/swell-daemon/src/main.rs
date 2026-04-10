use swell_core::init_tracing;
use swell_daemon::Daemon;
use swell_daemon::dashboard::{start_dashboard_server, DashboardState};
use std::sync::Arc;
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

    // Start both servers concurrently
    let daemon_run = daemon.run();
    let dashboard_run = start_dashboard_server(Arc::clone(&daemon), dashboard_state, dashboard_port);

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
