//! OAuth 2.0 token management and interactive setup flow for Gmail.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use tokio::sync::RwLock;
use tokio::time::Instant;

use crate::gmail::types::TokenResponse;

/// Response from the OAuth token exchange endpoint.
#[derive(Debug, serde::Deserialize)]
struct OAuthTokenResponse {
    refresh_token: Option<String>,
    access_token: String,
    expires_in: u64,
}

/// Thread-safe OAuth access token manager with automatic refresh.
pub struct TokenManager {
    client_id: String,
    client_secret: String,
    refresh_token: String,
    token_url: String,
    http_client: reqwest::Client,
    cached: Arc<RwLock<Option<CachedToken>>>,
}

struct CachedToken {
    access_token: String,
    expires_at: Instant,
}

/// Refresh tokens 5 minutes before they actually expire.
const SAFETY_MARGIN_SECS: u64 = 5 * 60;

impl TokenManager {
    pub fn new(
        client_id: String,
        client_secret: String,
        refresh_token: String,
        token_url: String,
    ) -> Self {
        Self {
            client_id,
            client_secret,
            refresh_token,
            token_url,
            http_client: reqwest::Client::new(),
            cached: Arc::new(RwLock::new(None)),
        }
    }

    /// Return a valid access token, refreshing if the cached one is expired.
    pub async fn get_token(&self) -> Result<String> {
        {
            let guard = self.cached.read().await;
            if let Some(cached) = guard.as_ref() {
                if Instant::now() < cached.expires_at {
                    return Ok(cached.access_token.clone());
                }
            }
        }
        self.refresh().await
    }

    async fn refresh(&self) -> Result<String> {
        let body = format!(
            "grant_type=refresh_token&client_id={}&client_secret={}&refresh_token={}",
            url_encode(&self.client_id),
            url_encode(&self.client_secret),
            url_encode(&self.refresh_token),
        );

        let resp = self
            .http_client
            .post(&self.token_url)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .body(body)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(anyhow!("Token refresh failed with status {status}: {text}"));
        }

        let token_resp: TokenResponse = resp.json().await?;

        let expires_at = if token_resp.expires_in > SAFETY_MARGIN_SECS {
            Instant::now()
                + std::time::Duration::from_secs(token_resp.expires_in - SAFETY_MARGIN_SECS)
        } else {
            Instant::now()
        };

        let access_token = token_resp.access_token.clone();
        {
            let mut guard = self.cached.write().await;
            *guard = Some(CachedToken {
                access_token: access_token.clone(),
                expires_at,
            });
        }
        Ok(access_token)
    }

    /// Remaining seconds until the cached token expires, for the health endpoint.
    pub async fn expires_in_secs(&self) -> Option<u64> {
        let guard = self.cached.read().await;
        guard.as_ref().map(|c| {
            let now = Instant::now();
            if c.expires_at > now { (c.expires_at - now).as_secs() } else { 0 }
        })
    }

    /// Whether the cached token is present and not yet expired.
    pub async fn is_valid(&self) -> bool {
        let guard = self.cached.read().await;
        match guard.as_ref() {
            Some(cached) => Instant::now() < cached.expires_at,
            None => false,
        }
    }
}

// ---------------------------------------------------------------------------
// OAuth setup flow
// ---------------------------------------------------------------------------

/// Google client_secret JSON structure.
#[derive(Debug, serde::Deserialize)]
struct GoogleClientSecretFile {
    installed: Option<GoogleClientCreds>,
    web: Option<GoogleClientCreds>,
}

#[derive(Debug, serde::Deserialize)]
struct GoogleClientCreds {
    client_id: String,
    client_secret: String,
}

/// Run the interactive OAuth setup flow.
///
/// 1. Reads credentials from --client-json or existing config
/// 2. Starts an ephemeral local HTTP server for the OAuth callback
/// 3. Opens the browser for Google OAuth consent
/// 4. Exchanges the authorization code for a refresh token
/// 5. Writes secrets.toml (0600)
pub async fn run_oauth_setup(config_path: PathBuf, client_json: Option<PathBuf>) -> Result<()> {
    if !config_path.exists() {
        anyhow::bail!(
            "Config file not found at {}.\n\
             Create it first (see config.example.toml in the Carapace repo).",
            config_path.display()
        );
    }

    let config_dir = config_path
        .parent()
        .unwrap_or(Path::new("."))
        .to_path_buf();

    let config_text = std::fs::read_to_string(&config_path)
        .with_context(|| format!("failed to read {}", config_path.display()))?;

    let client_id: String;
    let client_secret: String;

    if let Some(json_path) = client_json {
        let json_text = std::fs::read_to_string(&json_path)
            .with_context(|| format!("failed to read {}", json_path.display()))?;
        let parsed: GoogleClientSecretFile = serde_json::from_str(&json_text)
            .context("failed to parse client_secret JSON")?;
        let creds = parsed.installed.or(parsed.web)
            .context("client_secret JSON has neither 'installed' nor 'web' key")?;
        client_id = creds.client_id;
        client_secret = creds.client_secret;

        // Patch client_id / client_secret lines in the config file in-place.
        let updated: String = config_text
            .lines()
            .map(|line| {
                let t = line.trim_start();
                if t.starts_with("client_id") && t.contains('=') {
                    format!("client_id = \"{}\"", client_id)
                } else if t.starts_with("client_secret") && t.contains('=') {
                    format!("client_secret = \"{}\"", client_secret)
                } else {
                    line.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");

        std::fs::write(&config_path, &updated)
            .with_context(|| format!("failed to update {}", config_path.display()))?;
        println!("Updated client_id and client_secret in {}", config_path.display());
    } else {
        let parsed: toml::Value =
            toml::from_str(&config_text).context("failed to parse config.toml")?;
        client_id = parsed
            .get("auth").and_then(|a| a.get("client_id")).and_then(|v| v.as_str())
            .context("client_id not found in config.toml")?.to_string();
        client_secret = parsed
            .get("auth").and_then(|a| a.get("client_secret")).and_then(|v| v.as_str())
            .context("client_secret not found in config.toml")?.to_string();

        if client_id.contains("YOUR_CLIENT") || client_secret.contains("YOUR_CLIENT") {
            anyhow::bail!(
                "config.toml still has placeholder credentials.\n\
                 Either edit them manually or use --client-json to import from Google's JSON file."
            );
        }
    }

    // Spin up ephemeral local HTTP server for the OAuth callback.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let redirect_uri = format!("http://127.0.0.1:{port}");

    let (tx, rx) = tokio::sync::oneshot::channel::<String>();
    let tx = Arc::new(tokio::sync::Mutex::new(Some(tx)));

    let callback_handler = {
        let tx = tx.clone();
        move |axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>| {
            let tx = tx.clone();
            async move {
                if let Some(code) = params.get("code") {
                    if let Some(sender) = tx.lock().await.take() {
                        let _ = sender.send(code.clone());
                    }
                    axum::response::Html(
                        "<html><body><h1>Authorization successful!</h1>\
                         <p>You can close this tab and return to the terminal.</p></body></html>"
                            .to_string(),
                    )
                } else {
                    let error = params.get("error").cloned().unwrap_or_else(|| "unknown error".into());
                    axum::response::Html(format!(
                        "<html><body><h1>Authorization failed</h1><p>{error}</p></body></html>"
                    ))
                }
            }
        }
    };

    let app = axum::Router::new().route("/", axum::routing::get(callback_handler));

    // Scopes: readonly (read emails) + compose (create drafts, but NOT send from our code)
    let scopes = "https://www.googleapis.com/auth/gmail.readonly%20\
                  https://www.googleapis.com/auth/gmail.compose";

    let auth_url = format!(
        "https://accounts.google.com/o/oauth2/v2/auth?\
         client_id={}&redirect_uri={}&response_type=code&\
         scope={scopes}&access_type=offline&prompt=consent",
        url_encode(&client_id),
        url_encode(&redirect_uri),
    );

    println!("\nOpening browser for Google OAuth consent...");
    println!("If the browser doesn't open automatically, visit:\n\n  {auth_url}\n");

    if let Err(e) = open::that(&auth_url) {
        eprintln!("Warning: could not open browser: {e}");
    }

    let server = axum::serve(listener, app);
    let code = tokio::select! {
        result = server => {
            result.context("OAuth callback server error")?;
            anyhow::bail!("callback server exited unexpectedly");
        }
        code = rx => { code.context("failed to receive authorization code")? }
    };

    println!("Received authorization code. Exchanging for tokens...");

    let http = reqwest::Client::new();
    let token_body = format!(
        "grant_type=authorization_code&code={}&client_id={}&client_secret={}&redirect_uri={}",
        url_encode(&code),
        url_encode(&client_id),
        url_encode(&client_secret),
        url_encode(&redirect_uri),
    );

    let resp = http
        .post("https://oauth2.googleapis.com/token")
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(token_body)
        .send()
        .await
        .context("failed to exchange authorization code")?;

    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("Token exchange failed with status {status}: {text}");
    }

    let token_resp: OAuthTokenResponse =
        resp.json().await.context("failed to parse token response")?;

    let refresh_token = token_resp.refresh_token.context(
        "No refresh_token in response.\n\
         Try revoking access at https://myaccount.google.com/permissions and re-running setup.",
    )?;

    // Write secrets.toml with 0600 permissions.
    let secrets_path = config_dir.join("secrets.toml");
    let secrets_content = format!("refresh_token = \"{refresh_token}\"\n");
    std::fs::write(&secrets_path, &secrets_content)
        .with_context(|| format!("failed to write {}", secrets_path.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&secrets_path, std::fs::Permissions::from_mode(0o600))?;
    }

    println!("\nSetup complete!");
    println!("  Config:  {}", config_path.display());
    println!("  Secrets: {} (0600)", secrets_path.display());
    println!("\nStart the proxy with:\n  gmail-proxy serve --config {}", config_path.display());

    Ok(())
}

/// Percent-encode a string for use in URL query parameters.
fn url_encode(s: &str) -> String {
    s.chars()
        .flat_map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => {
                vec![c]
            }
            c => format!("%{:02X}", c as u32).chars().collect(),
        })
        .collect()
}
