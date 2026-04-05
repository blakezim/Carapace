//! Google Docs/Drive adapter — talks to the gdocs-proxy daemon over its Unix socket HTTP API.
//!
//! The gdocs-proxy handles all OAuth, token refresh, and content scrubbing.
//! This adapter is a thin HTTP client that calls the proxy's endpoints.

use std::path::PathBuf;

use serde::Serialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tracing::{debug, warn};

/// Errors from the Google Docs adapter.
#[derive(Debug, thiserror::Error)]
pub enum AdapterError {
    #[error("gdocs-proxy socket not found at {0}")]
    SocketNotFound(PathBuf),

    #[error("failed to connect to gdocs-proxy: {0}")]
    Connect(std::io::Error),

    #[error("HTTP error from gdocs-proxy: status {status}")]
    HttpError { status: u16 },

    #[error("failed to parse gdocs-proxy response: {0}")]
    ParseError(String),

    #[error("I/O error communicating with gdocs-proxy: {0}")]
    Io(#[from] std::io::Error),
}

/// Health status of the Google Docs adapter.
#[derive(Debug, Serialize)]
pub struct HealthStatus {
    pub proxy_reachable: bool,
    pub token_valid: Option<bool>,
    pub token_expires_in_secs: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Google Docs adapter — proxies requests to the gdocs-proxy Unix socket.
pub struct GDocsAdapter {
    socket_path: PathBuf,
}

impl GDocsAdapter {
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

    async fn put(&self, path: &str, body: serde_json::Value) -> Result<serde_json::Value, AdapterError> {
        if !self.socket_path.exists() {
            return Err(AdapterError::SocketNotFound(self.socket_path.clone()));
        }
        let body_str = serde_json::to_string(&body)
            .map_err(|e| AdapterError::ParseError(e.to_string()))?;

        let stream = UnixStream::connect(&self.socket_path)
            .await
            .map_err(AdapterError::Connect)?;

        let request = format!(
            "PUT {path} HTTP/1.1\r\nHost: localhost\r\n\
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

    /// Search Drive for files.
    pub async fn search(
        &self,
        query: &str,
        max: Option<u32>,
        docs_only: bool,
        page_token: Option<&str>,
    ) -> Result<serde_json::Value, AdapterError> {
        let max = max.unwrap_or(20);
        let mut path = format!(
            "/search?q={}&max={max}&docs_only={docs_only}",
            url_encode(query)
        );
        if let Some(pt) = page_token {
            path.push_str(&format!("&page_token={}", url_encode(pt)));
        }
        debug!(query, max, docs_only, "gdocs search");
        self.get(&path).await
    }

    /// Read a Google Doc (structured content).
    pub async fn read_document(&self, doc_id: &str) -> Result<serde_json::Value, AdapterError> {
        debug!(doc_id, "gdocs read_document");
        self.get(&format!("/doc/{}", url_encode(doc_id))).await
    }

    /// Get file metadata.
    pub async fn get_file_info(&self, file_id: &str) -> Result<serde_json::Value, AdapterError> {
        debug!(file_id, "gdocs get_file_info");
        self.get(&format!("/file/{}", url_encode(file_id))).await
    }

    /// Create a folder in Google Drive.
    pub async fn create_folder(
        &self,
        name: &str,
        parent_id: Option<&str>,
    ) -> Result<serde_json::Value, AdapterError> {
        debug!(name, "gdocs create_folder");
        let mut payload = serde_json::json!({"name": name});
        if let Some(pid) = parent_id {
            payload["parent_id"] = serde_json::json!(pid);
        }
        self.post("/folders", payload).await
    }

    /// Create a new Google Doc, optionally in a specific folder.
    pub async fn create_document(
        &self,
        title: &str,
        content: Option<&str>,
        folder_id: Option<&str>,
    ) -> Result<serde_json::Value, AdapterError> {
        debug!(title, "gdocs create_document");
        let mut payload = serde_json::json!({"title": title});
        if let Some(c) = content {
            payload["content"] = serde_json::json!(c);
        }
        if let Some(fid) = folder_id {
            payload["folder_id"] = serde_json::json!(fid);
        }
        self.post("/docs", payload).await
    }

    /// Copy a file.
    pub async fn copy_file(
        &self,
        file_id: &str,
        title: Option<&str>,
    ) -> Result<serde_json::Value, AdapterError> {
        debug!(file_id, "gdocs copy_file");
        let mut path = format!("/docs/copy/{}", url_encode(file_id));
        if let Some(t) = title {
            path.push_str(&format!("?title={}", url_encode(t)));
        }
        self.post(&path, serde_json::json!({})).await
    }

    /// Append text to a document.
    pub async fn append_text(
        &self,
        doc_id: &str,
        text: &str,
    ) -> Result<serde_json::Value, AdapterError> {
        debug!(doc_id, "gdocs append_text");
        let payload = serde_json::json!({"append_text": text});
        self.put(&format!("/doc/{}", url_encode(doc_id)), payload).await
    }

    /// Health check.
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
}

// ---------------------------------------------------------------------------
// HTTP/1.1 over Unix socket helpers (same pattern as gmail adapter)
// ---------------------------------------------------------------------------

async fn parse_response(
    read_half: tokio::io::ReadHalf<UnixStream>,
) -> Result<serde_json::Value, AdapterError> {
    let mut buf = Vec::new();
    let mut reader = tokio::io::BufReader::new(read_half);
    reader.read_to_end(&mut buf).await?;

    let raw = String::from_utf8_lossy(&buf);

    let sep = raw.find("\r\n\r\n").ok_or_else(|| {
        AdapterError::ParseError(format!(
            "no HTTP header separator in response (got {} bytes: {:?})",
            raw.len(),
            &raw[..raw.len().min(200)]
        ))
    })?;
    let headers = &raw[..sep];
    let raw_body = &raw[sep + 4..];

    let status: u16 = headers
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())
        .unwrap_or(0);

    let is_chunked = headers
        .lines()
        .any(|l| l.to_lowercase().contains("transfer-encoding: chunked"));

    let body = if is_chunked {
        decode_chunked(raw_body).map_err(AdapterError::ParseError)?
    } else {
        raw_body.to_string()
    };

    if !(200..300).contains(&status) {
        warn!(status, body = %body, "gdocs-proxy returned error");
        return Err(AdapterError::HttpError { status });
    }

    serde_json::from_str(&body)
        .map_err(|e| AdapterError::ParseError(format!("{e}: {body}")))
}

fn decode_chunked(input: &str) -> Result<String, String> {
    let mut result = String::new();
    let mut remaining = input;

    loop {
        let size_end = remaining.find("\r\n")
            .ok_or_else(|| format!("chunked: missing size line in: {:?}", &remaining[..remaining.len().min(50)]))?;
        let size_str = remaining[..size_end].trim();
        let size_str = size_str.split(';').next().unwrap_or(size_str).trim();
        let chunk_size = usize::from_str_radix(size_str, 16)
            .map_err(|e| format!("chunked: invalid chunk size {:?}: {e}", size_str))?;

        remaining = &remaining[size_end + 2..];

        if chunk_size == 0 {
            break;
        }

        if remaining.len() < chunk_size {
            return Err(format!("chunked: body too short for chunk size {chunk_size}"));
        }
        result.push_str(&remaining[..chunk_size]);
        remaining = &remaining[chunk_size..];

        if remaining.starts_with("\r\n") {
            remaining = &remaining[2..];
        }
    }

    Ok(result)
}

fn url_encode(s: &str) -> String {
    s.chars()
        .flat_map(|c| match c {
            'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => vec![c],
            c => format!("%{:02X}", c as u32).chars().collect(),
        })
        .collect()
}
