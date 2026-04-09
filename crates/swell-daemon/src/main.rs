use swell_core::init_tracing;
use swell_daemon::Daemon;
use tracing::info;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    init_tracing();

    let socket_path =
        std::env::var("SWELL_SOCKET").unwrap_or_else(|_| "/tmp/swell-daemon.sock".to_string());

    info!(path = %socket_path, "Starting swell-daemon");

    let daemon = Daemon::new(socket_path);
    if let Err(e) = daemon.run().await {
        eprintln!("Daemon error: {}", e);
        std::process::exit(1);
    }

    Ok(())
}
