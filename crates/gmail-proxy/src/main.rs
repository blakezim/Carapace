//! Gmail proxy daemon for Carapace.
//!
//! Usage:
//!   gmail-proxy setup  --config /etc/carapace/gmail-proxy.toml [--client-json /path/to/client_secret.json]
//!   gmail-proxy serve  [--config /path/to/config.toml]

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use clap::{Parser, Subcommand};

use gmail_proxy::auth::{run_oauth_setup, TokenManager};
use gmail_proxy::config;
use gmail_proxy::gmail::client::GmailClient;
use gmail_proxy::proxy::routes::{build_router, AppState};
use gmail_proxy::scrub::content::ContentScrubber;
use gmail_proxy::scrub::labels::LabelFilter;

#[derive(Parser)]
#[command(name = "gmail-proxy", about = "Secure Gmail proxy for Carapace")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Interactive OAuth setup — runs once to obtain and store the refresh token.
    Setup {
        /// Path to config.toml
        #[arg(long, default_value = "/etc/carapace/gmail-proxy.toml")]
        config: PathBuf,
        /// Path to the Google client_secret JSON downloaded from Google Cloud Console.
        /// If provided, client_id and client_secret are extracted and saved into config.toml.
        #[arg(long)]
        client_json: Option<PathBuf>,
    },
    /// Run the proxy server.
    Serve {
        /// Path to config.toml (default: ~/.config/gmail-proxy/config.toml)
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
                .unwrap_or_else(|_| "gmail_proxy=info,warn".parse().unwrap()),
        )
        .init();

    tracing::info!(account = %cfg.gmail.account, "Starting gmail-proxy");

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
        .context("Initial token refresh failed — check your OAuth credentials and secrets.toml")?;
    tracing::info!("OAuth token valid");

    // Create Gmail client.
    let gmail = Arc::new(GmailClient::new(
        token_manager.clone(),
        "https://gmail.googleapis.com/gmail/v1/users/me".into(),
        cfg.gmail.account.clone(),
    ));

    // Resolve the blocked label: look it up by name to get its ID.
    let labels = gmail
        .list_labels()
        .await
        .context("Failed to list Gmail labels")?;

    let blocked_label = labels
        .labels
        .unwrap_or_default()
        .into_iter()
        .find(|l| l.name.eq_ignore_ascii_case(&cfg.scrub.blocked_label))
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Label '{}' not found in Gmail.\n\
                 Create it at mail.google.com, or change [scrub] blocked_label in config.toml.",
                cfg.scrub.blocked_label
            )
        })?;

    tracing::info!(
        label = %blocked_label.name,
        id = %blocked_label.id,
        "Resolved blocked label"
    );

    // Build content scrubber from compiled regexes.
    let scrubber = Arc::new(ContentScrubber::new(
        cfg.scrub.otp_patterns.iter()
            .map(|p| regex::Regex::new(p).unwrap())
            .collect(),
        cfg.scrub.url_strip_patterns.iter()
            .map(|p| regex::Regex::new(p).unwrap())
            .collect(),
        cfg.scrub.blocked_sender_patterns.iter()
            .map(|p| regex::Regex::new(p).unwrap())
            .collect(),
        cfg.scrub.strip_links,
    ));

    let label_filter = Arc::new(LabelFilter::new(
        blocked_label.id.clone(),
        blocked_label.name.clone(),
    ));

    let state = Arc::new(AppState {
        gmail: gmail.clone(),
        label_filter,
        scrubber,
        allowed_operators: cfg.scrub.allowed_operators.clone(),
        blocked_label: cfg.scrub.blocked_label.clone(),
        max_query_depth: 10,
        search_concurrency: cfg.proxy.search_fetch_concurrency,
        token_manager: token_manager.clone(),
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
        // Try to set group ownership to carapace-clients so the daemon can reach us.
        if let Some(group) = nix::unistd::Group::from_name("carapace-clients").ok().flatten() {
            let _ = nix::unistd::chown(socket_path, None, Some(group.gid));
        }
        tracing::info!("Socket permissions set to 0660");
    }

    tracing::info!(socket = %socket_path.display(), "gmail-proxy listening");

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
