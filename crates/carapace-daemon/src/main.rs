//! Carapace Gateway Daemon
//!
//! A Unix socket server that listens for JSON-RPC requests from shim tools
//! and executes commands as the `carapace` user. This provides OS-level
//! isolation between an AI runtime and messaging credentials.
//!
//! # Usage
//!
//! ```bash
//! # Run as the carapace user:
//! sudo -u carapace carapace-daemon
//!
//! # Or with a custom socket path:
//! sudo -u carapace carapace-daemon --socket /tmp/carapace-test.sock
//! ```

mod handler;
mod protocol;
mod server;

use std::path::PathBuf;

/// Default socket path – matches the project's convention.
const DEFAULT_SOCKET_PATH: &str = "/var/run/carapace/gateway.sock";

/// Environment variable to override the socket path.
const ENV_SOCKET_PATH: &str = "CARAPACE_SOCKET_PATH";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialise structured logging.
    // Respects RUST_LOG env var (e.g. RUST_LOG=debug).
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    // Determine socket path: CLI arg > env var > default.
    let socket_path = parse_socket_path();

    tracing::info!(
        socket_path = %socket_path.display(),
        version = env!("CARGO_PKG_VERSION"),
        "starting carapace daemon"
    );

    // Run the server (blocks forever).
    server::run(&socket_path).await?;

    Ok(())
}

/// Parse the socket path from CLI args or environment.
fn parse_socket_path() -> PathBuf {
    let args: Vec<String> = std::env::args().collect();

    // Simple arg parsing: --socket <path>
    if let Some(pos) = args.iter().position(|a| a == "--socket") {
        if let Some(path) = args.get(pos + 1) {
            return PathBuf::from(path);
        }
    }

    // Environment variable override
    if let Ok(path) = std::env::var(ENV_SOCKET_PATH) {
        return PathBuf::from(path);
    }

    PathBuf::from(DEFAULT_SOCKET_PATH)
}
