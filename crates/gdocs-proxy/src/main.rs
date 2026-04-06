//! Google Docs proxy daemon for Carapace.
//!
//! Usage:
//!   gdocs-proxy setup  --config /etc/carapace/gdocs-proxy.toml [--client-json /path/to/client_secret.json]
//!   gdocs-proxy serve  [--config /path/to/config.toml]

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use clap::{Parser, Subcommand};

use gdocs_proxy::auth::{run_oauth_setup, TokenManager};
use gdocs_proxy::config;
use gdocs_proxy::docs::client::DocsClient;
use gdocs_proxy::proxy::routes::{build_router, AppState};

#[derive(Parser)]
#[command(name = "gdocs-proxy", about = "Secure Google Docs/Drive proxy for Carapace")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Interactive OAuth setup — runs once to obtain and store the refresh token.
    Setup {
        /// Path to config.toml
        #[arg(long, default_value = "/etc/carapace/gdocs-proxy.toml")]
        config: PathBuf,
        /// Path to the Google client_secret JSON downloaded from Google Cloud Console.
        #[arg(long)]
        client_json: Option<PathBuf>,
    },
    /// Run the proxy server.
    Serve {
        /// Path to config.toml
        #[arg(long)]
        config: Option<PathBuf>,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Setup { config, client_json } => {
            run_oauth_setup(config, client_json).await?;
        }
        Command::Serve { config } => {
            serve(config).await?;
        }
    }
    Ok(())
}

async fn serve(config_path: Option<PathBuf>) -> anyhow::Result<()> {
    let config_path = config_path.unwrap_or_else(config::default_config_path);

    // Load config + secrets (enforces 0600 on secrets file).
    let cfg = config::load_config(&config_path, false)?;

    // Initialize tracing.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "gdocs_proxy=info,warn".parse().unwrap()),
        )
        .init();

    tracing::info!(account = %cfg.gdocs.account, "Starting gdocs-proxy");

    // Create token manager and validate credentials at startup.
    let token_manager = Arc::new(TokenManager::new(
        cfg.auth.client_id.clone(),
        cfg.auth.client_secret.clone(),
        cfg.secrets.refresh_token.clone(),
        "https://oauth2.googleapis.com/token".into(),
    ));
    token_manager
        .get_token()
        .await
        .context("Initial token refresh failed — check your OAuth credentials and secrets")?;
    tracing::info!("OAuth token valid");

    // Create Docs/Drive client.
    let docs = Arc::new(DocsClient::new(
        token_manager.clone(),
        cfg.gdocs.account.clone(),
    ));

    // Compile scrub patterns.
    let scrub_patterns: Vec<regex::Regex> = cfg
        .scrub
        .redact_patterns
        .iter()
        .map(|p| regex::Regex::new(p).unwrap())
        .collect();

    if !cfg.scrub.blocked_folders.is_empty() {
        tracing::info!(
            count = cfg.scrub.blocked_folders.len(),
            "Blocked folders configured"
        );
    }

    let state = Arc::new(AppState {
        docs,
        token_manager: token_manager.clone(),
        scrub_patterns,
        strip_links: cfg.scrub.strip_links,
        blocked_folders: cfg.scrub.blocked_folders.clone(),
        start_time: std::time::Instant::now(),
    });

    let app = build_router(state);

    let socket_path = std::path::Path::new(&cfg.proxy.socket_path);
    if let Some(parent) = socket_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create socket dir {}", parent.display()))?;
    }
    if socket_path.exists() {
        std::fs::remove_file(socket_path)
            .context("Failed to remove stale socket file")?;
    }

    let listener = tokio::net::UnixListener::bind(socket_path)
        .with_context(|| format!("Failed to bind Unix socket at {}", socket_path.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(socket_path, std::fs::Permissions::from_mode(0o660))
            .context("Failed to set socket permissions")?;
        if let Some(group) = nix::unistd::Group::from_name("carapace-clients").ok().flatten() {
            let _ = nix::unistd::chown(socket_path, None, Some(group.gid));
        }
        tracing::info!("Socket permissions set to 0660");
    }

    tracing::info!(socket = %socket_path.display(), "gdocs-proxy listening");

    axum::serve(listener, app.into_make_service())
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("Server error")?;

    tracing::info!("Shutting down");
    Ok(())
}

async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("Failed to install CTRL+C handler");
    tracing::info!("Received shutdown signal");
}
