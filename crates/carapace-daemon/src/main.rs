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
//!
//! # Or with a custom config file:
//! sudo -u carapace carapace-daemon --config /path/to/config.toml
//! ```

mod adapters;
mod allowlist;
mod audit;
mod channel_handler;
mod config;
mod content_filter;
mod dead_letter;
mod handler;
mod middleware;
mod protocol;
mod rate_limiter;
mod server;

use std::path::PathBuf;

use crate::config::Config;

/// Default socket path – matches the project's convention.
const DEFAULT_SOCKET_PATH: &str = "/var/run/carapace/gateway.sock";

/// Default config path – where install.sh puts it.
const DEFAULT_CONFIG_PATH: &str = "/Users/carapace/.config/carapace/config.toml";

/// Environment variable to override the socket path.
const ENV_SOCKET_PATH: &str = "CARAPACE_SOCKET_PATH";

/// Environment variable to override the config path.
const ENV_CONFIG_PATH: &str = "CARAPACE_CONFIG";

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

    // Load configuration.
    let config = load_config();

    // Determine socket path: CLI arg > env var > config file > hardcoded default.
    let socket_path = resolve_socket_path(&config);

    tracing::info!(
        socket_path = %socket_path.display(),
        version = env!("CARGO_PKG_VERSION"),
        "starting carapace daemon"
    );

    // Run the server (blocks forever).
    server::run(&socket_path, config).await?;

    Ok(())
}

/// Load configuration from: explicit `--config` > env var > default path > built-in defaults.
fn load_config() -> Config {
    let args: Vec<String> = std::env::args().collect();

    // --config <path>
    let config_path = args
        .iter()
        .position(|a| a == "--config")
        .and_then(|pos| args.get(pos + 1))
        .map(PathBuf::from)
        .or_else(|| std::env::var(ENV_CONFIG_PATH).ok().map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from(DEFAULT_CONFIG_PATH));

    if config_path.exists() {
        match Config::load(&config_path) {
            Ok(config) => {
                tracing::info!(path = %config_path.display(), "loaded config");
                return config;
            }
            Err(e) => {
                tracing::warn!(
                    path = %config_path.display(),
                    error = %e,
                    "failed to load config, using defaults"
                );
            }
        }
    } else {
        tracing::info!(
            path = %config_path.display(),
            "config file not found, using defaults"
        );
    }

    Config::defaults()
}

/// Resolve socket path: CLI arg > env var > config file > hardcoded default.
fn resolve_socket_path(config: &Config) -> PathBuf {
    let args: Vec<String> = std::env::args().collect();

    // --socket <path>
    if let Some(pos) = args.iter().position(|a| a == "--socket") {
        if let Some(path) = args.get(pos + 1) {
            return PathBuf::from(path);
        }
    }

    // Environment variable override
    if let Ok(path) = std::env::var(ENV_SOCKET_PATH) {
        return PathBuf::from(path);
    }

    // Config file value (if it's not the default — meaning user set it)
    let config_path = &config.gateway.socket_path;
    if !config_path.as_os_str().is_empty() {
        return config_path.clone();
    }

    PathBuf::from(DEFAULT_SOCKET_PATH)
}
