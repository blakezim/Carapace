//! Gmail adapter — talks to the gmail-proxy daemon over its Unix socket HTTP API.
//!
//! The gmail-proxy handles all OAuth, token refresh, and content scrubbing.
//! This adapter is a thin HTTP client that calls the proxy's endpoints and
//! wraps the results into the same types/errors used by the iMessage adapter.

use std::path::PathBuf;
use std::time::Duration;

use serde::Serialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;
use tokio::net::UnixStream;
use tracing::{debug, warn};

/// Errors from the Gmail adapter.
#[derive(Debug, thiserror::Error)]
pub enum AdapterError {
    #[error("gmail-proxy socket not found at {0}")]
    SocketNotFound(PathBuf),

    #[error("failed to connect to gmail-proxy: {0}")]
    Connect(std::io::Error),

    #[error("HTTP error from gmail-proxy: status {status}")]
    HttpError { status: u16 },

    #[error("failed to parse gmail-proxy response: {0}")]
    ParseError(String),

    #[error("I/O error communicating with gmail-proxy: {0}")]
    Io(#[from] std::io::Error),
}

/// Result of a successful draft creation.
#[derive(Debug, Serialize)]
pub struct DraftResult {
    pub draft_id: String,
    pub message_id: String,
    pub thread_id: String,
}

/// Health status of the Gmail adapter.
#[derive(Debug, Serialize)]
pub struct HealthStatus {
    pub proxy_reachable: bool,
    pub token_valid: Option<bool>,
    pub token_expires_in_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Gmail adapter — proxies requests to the gmail-proxy Unix socket.
pub struct GmailAdapter {
    socket_path: PathBuf,
}

impl GmailAdapter {
    pub fn new(socket_path: PathBuf) -> Self {
        Self { socket_path }
    }

    // ── HTTP primitives over Unix socket ────────────────────────────────────

    async fn get(&self, path: &str) -> Result<serde_json::Value, AdapterError> {
        if !self.socket_path.exists() {
            return Err(AdapterError::SocketNotFound(self.socket_path.clone()));
        }
        let stream = UnixStream::connect(&self.socket_path)
            .await
            .map_err(AdapterError::Connect)?;

        let request = format!(
            "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"
        );
        let (read_half, mut write_half) = tokio::io::split(stream);
        write_half.write_all(request.as_bytes()).await?;
        drop(write_half);

        parse_response(read_half).await
    }

    async fn post(&self, path: &str, body: serde_json::Value) -> Result<serde_json::Value, AdapterError> {
        if !self.socket_path.exists() {
            return Err(AdapterError::SocketNotFound(self.socket_path.clone()));
        }
        let body_str = serde_json::to_string(&body)
            .map_err(|e| AdapterError::ParseError(e.to_string()))?;

        let stream = UnixStream::connect(&self.socket_path)
            .await
            .map_err(AdapterError::Connect)?;

        let request = format!(
            "POST {path} HTTP/1.1\r\nHost: localhost\r\n\
             Content-Type: application/json\r\nContent-Length: {}\r\n\
             Connection: close\r\n\r\n{body_str}",
            body_str.len()
        );
        let (read_half, mut write_half) = tokio::io::split(stream);
        write_half.write_all(request.as_bytes()).await?;
        drop(write_half);

        parse_response(read_half).await
    }

    // ── Public API methods ───────────────────────────────────────────────────

    /// Search emails. Returns the raw proxy JSON (messages array + pagination).
    pub async fn search(
        &self,
        query: &str,
        max: Option<u32>,
        page_token: Option<&str>,
    ) -> Result<serde_json::Value, AdapterError> {
        let max = max.unwrap_or(20);
        let mut path = format!(
            "/search?q={}&max={max}",
            url_encode(query)
        );
        if let Some(pt) = page_token {
            path.push_str(&format!("&page_token={}", url_encode(pt)));
        }
        debug!(query, max, "gmail search");
        self.get(&path).await
    }

    /// Fetch a single message by ID.
    pub async fn get_message(&self, id: &str) -> Result<serde_json::Value, AdapterError> {
        debug!(id, "gmail get_message");
        self.get(&format!("/message/{}", url_encode(id))).await
    }

    /// Fetch all messages in a thread.
    pub async fn get_thread(&self, thread_id: &str) -> Result<serde_json::Value, AdapterError> {
        debug!(thread_id, "gmail get_thread");
        self.get(&format!("/thread/{}", url_encode(thread_id))).await
    }

    /// Create a draft email.
    pub async fn create_draft(
        &self,
        to: &str,
        subject: &str,
        body: &str,
        cc: Option<&str>,
    ) -> Result<DraftResult, AdapterError> {
        debug!(to, subject, "gmail create_draft");
        let mut payload = serde_json::json!({
            "to": to,
            "subject": subject,
            "body": body,
        });
        if let Some(cc_addr) = cc {
            payload["cc"] = serde_json::json!(cc_addr);
        }
        let resp = self.post("/drafts", payload).await?;
        let draft_id = resp.get("draft_id").and_then(|v| v.as_str())
            .ok_or_else(|| AdapterError::ParseError("missing draft_id in response".into()))?
            .to_string();
        let message_id = resp.get("message_id").and_then(|v| v.as_str())
            .unwrap_or("").to_string();
        let thread_id = resp.get("thread_id").and_then(|v| v.as_str())
            .unwrap_or("").to_string();
        Ok(DraftResult { draft_id, message_id, thread_id })
    }

    /// Health check: connect to the proxy and call /health.
    pub async fn health_check(&self) -> HealthStatus {
        if !self.socket_path.exists() {
            return HealthStatus {
                proxy_reachable: false,
                token_valid: None,
                token_expires_in_secs: None,
                error: Some(format!("socket not found at {}", self.socket_path.display())),
            };
        }
        match self.get("/health").await {
            Ok(resp) => {
                let token_valid = resp
                    .pointer("/token/valid")
                    .and_then(|v| v.as_bool());
                let expires_in = resp
                    .pointer("/token/expires_in_secs")
                    .and_then(|v| v.as_u64());
                HealthStatus {
                    proxy_reachable: true,
                    token_valid,
                    token_expires_in_secs: expires_in,
                    error: None,
                }
            }
            Err(e) => HealthStatus {
                proxy_reachable: false,
                token_valid: None,
                token_expires_in_secs: None,
                error: Some(e.to_string()),
            },
        }
    }

    /// Start polling for new messages.
    ///
    /// Spawns a background task that polls `/search?q=is:unread` every
    /// `poll_interval`, deduplicates by message ID, and sends new messages to
    /// the returned channel. Drop the `WatchHandle` to stop polling.
    pub fn watch(
        &self,
        buffer_size: usize,
        poll_interval: Duration,
    ) -> (WatchHandle, mpsc::Receiver<serde_json::Value>) {
        let (tx, rx) = mpsc::channel(buffer_size);
        let (stop_tx, mut stop_rx) = tokio::sync::oneshot::channel::<()>();
        let socket_path = self.socket_path.clone();

        let task = tokio::spawn(async move {
            let adapter = GmailAdapter::new(socket_path);
            let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
            let mut interval = tokio::time::interval(poll_interval);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

            loop {
                tokio::select! {
                    _ = &mut stop_rx => break,
                    _ = interval.tick() => {}
                }

                let result = adapter
                    .search("is:unread newer_than:1d", Some(50), None)
                    .await;

                let messages = match result {
                    Ok(v) => v
                        .get("messages")
                        .and_then(|m| m.as_array())
                        .cloned()
                        .unwrap_or_default(),
                    Err(e) => {
                        warn!(error = %e, "gmail watch poll failed");
                        continue;
                    }
                };

                for msg in messages {
                    let id = match msg.get("id").and_then(|v| v.as_str()) {
                        Some(id) => id.to_string(),
                        None => continue,
                    };
                    if seen.insert(id) {
                        if tx.send(msg).await.is_err() {
                            return; // receiver dropped
                        }
                    }
                }
            }
        });

        (WatchHandle { _task: task, _stop: stop_tx }, rx)
    }
}

/// Handle for a running Gmail watch poll task.
///
/// Dropping the handle stops the polling task.
pub struct WatchHandle {
    _task: tokio::task::JoinHandle<()>,
    _stop: tokio::sync::oneshot::Sender<()>,
}

// ---------------------------------------------------------------------------
// HTTP/1.1 over Unix socket helpers
// ---------------------------------------------------------------------------

/// Read the full HTTP response from the read half of a split UnixStream,
/// parse the status code, decode chunked transfer encoding if present,
/// and return the JSON body.
async fn parse_response(
    read_half: tokio::io::ReadHalf<UnixStream>,
) -> Result<serde_json::Value, AdapterError> {
    let mut buf = Vec::new();
    let mut reader = tokio::io::BufReader::new(read_half);
    reader.read_to_end(&mut buf).await?;

    let raw = String::from_utf8_lossy(&buf);

    // Split headers / body on \r\n\r\n
    let sep = raw.find("\r\n\r\n").ok_or_else(|| {
        AdapterError::ParseError(format!(
            "no HTTP header separator in response (got {} bytes: {:?})",
            raw.len(),
            &raw[..raw.len().min(200)]
        ))
    })?;
    let headers = &raw[..sep];
    let raw_body = &raw[sep + 4..];

    // Extract status code from "HTTP/1.1 200 OK"
    let status: u16 = headers
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())
        .unwrap_or(0);

    // Decode chunked transfer encoding if used.
    let is_chunked = headers
        .lines()
        .any(|l| l.to_lowercase().contains("transfer-encoding: chunked"));

    let body = if is_chunked {
        decode_chunked(raw_body).map_err(|e| AdapterError::ParseError(e))?
    } else {
        raw_body.to_string()
    };

    if !(200..300).contains(&status) {
        warn!(status, body = %body, "gmail-proxy returned error");
        return Err(AdapterError::HttpError { status });
    }

    serde_json::from_str(&body)
        .map_err(|e| AdapterError::ParseError(format!("{e}: {body}")))
}

/// Decode HTTP/1.1 chunked transfer encoding.
fn decode_chunked(input: &str) -> Result<String, String> {
    let mut result = String::new();
    let mut remaining = input;

    loop {
        // Each chunk starts with a hex size line.
        let size_end = remaining.find("\r\n")
            .ok_or_else(|| format!("chunked: missing size line in: {:?}", &remaining[..remaining.len().min(50)]))?;
        let size_str = remaining[..size_end].trim();
        // Strip any chunk extensions (e.g. "a; ext=val")
        let size_str = size_str.split(';').next().unwrap_or(size_str).trim();
        let chunk_size = usize::from_str_radix(size_str, 16)
            .map_err(|e| format!("chunked: invalid chunk size {:?}: {e}", size_str))?;

        remaining = &remaining[size_end + 2..];

        if chunk_size == 0 {
            break; // last chunk
        }

        if remaining.len() < chunk_size {
            return Err(format!("chunked: body too short for chunk size {chunk_size}"));
        }
        result.push_str(&remaining[..chunk_size]);
        remaining = &remaining[chunk_size..];

        // Skip the trailing \r\n after the chunk data.
        if remaining.starts_with("\r\n") {
            remaining = &remaining[2..];
        }
    }

    Ok(result)
}

/// Minimal percent-encoding for URL path segments and query values.
fn url_encode(s: &str) -> String {
    s.chars()
        .flat_map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => vec![c],
            c => format!("%{:02X}", c as u32).chars().collect(),
        })
        .collect()
}
