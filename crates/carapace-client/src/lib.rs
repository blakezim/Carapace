//! Carapace Gateway Client Library
//!
//! Provides [`GatewayClient`] – a synchronous client for connecting to the
//! Carapace daemon over its Unix domain socket and making JSON-RPC calls.
//!
//! This crate is intentionally synchronous so that shims can be small,
//! fast-starting binaries without pulling in an async runtime.
//!
//! # Example
//!
//! ```no_run
//! use carapace_client::GatewayClient;
//! use serde_json::json;
//!
//! let mut client = GatewayClient::connect_default().unwrap();
//! let result = client.call("ping", json!({})).unwrap();
//! println!("Got: {}", result);
//! ```

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Default socket path matching the daemon's default.
const DEFAULT_SOCKET_PATH: &str = "/var/run/carapace/gateway.sock";

/// Environment variable to override the socket path.
const ENV_SOCKET_PATH: &str = "CARAPACE_SOCKET_PATH";

// ── Error types ────────────────────────────────────────────────────────────

/// Errors returned by the gateway client.
#[derive(Debug, thiserror::Error)]
pub enum ClientError {
    /// Could not connect to the daemon socket.
    #[error("connection failed: {0}")]
    Connection(String),

    /// I/O error during read/write.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Could not parse the daemon's response as JSON.
    #[error("invalid JSON from daemon: {0}")]
    Parse(String),

    /// The daemon returned a JSON-RPC error.
    #[error("gateway error {code}: {message}")]
    Gateway { code: i32, message: String },

    /// The response didn't match the expected request ID.
    #[error("response ID mismatch: expected {expected}, got {got}")]
    IdMismatch { expected: u64, got: String },
}

// ── Internal JSON-RPC types (kept private) ─────────────────────────────────

#[derive(Serialize)]
struct RpcRequest {
    jsonrpc: &'static str,
    id: u64,
    method: String,
    params: serde_json::Value,
}

#[derive(Deserialize)]
struct RpcResponse {
    #[allow(dead_code)]
    jsonrpc: String,
    id: serde_json::Value,
    result: Option<serde_json::Value>,
    error: Option<RpcError>,
}

#[derive(Deserialize)]
struct RpcError {
    code: i32,
    message: String,
}

// ── GatewayClient ──────────────────────────────────────────────────────────

/// A synchronous client for the Carapace gateway daemon.
///
/// Maintains a persistent connection to the Unix domain socket.
/// Each [`call`](GatewayClient::call) sends a JSON-RPC request and waits
/// for the response.
pub struct GatewayClient {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
    next_id: u64,
}

impl GatewayClient {
    /// Connect to the daemon at the default socket path.
    ///
    /// The path is determined by (in order of priority):
    /// 1. `CARAPACE_SOCKET_PATH` environment variable
    /// 2. `/var/run/carapace/gateway.sock`
    pub fn connect_default() -> Result<Self, ClientError> {
        let path = std::env::var(ENV_SOCKET_PATH)
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from(DEFAULT_SOCKET_PATH));
        Self::connect(&path)
    }

    /// Connect to the daemon at a specific socket path.
    pub fn connect(socket_path: &Path) -> Result<Self, ClientError> {
        let stream = UnixStream::connect(socket_path).map_err(|e| {
            ClientError::Connection(format!(
                "Cannot connect to daemon at {}: {e}. Is the daemon running?",
                socket_path.display()
            ))
        })?;

        let reader = BufReader::new(stream.try_clone().map_err(|e| {
            ClientError::Connection(format!("Failed to clone stream: {e}"))
        })?);

        Ok(Self {
            reader,
            writer: stream,
            next_id: 1,
        })
    }

    /// Send a JSON-RPC request and wait for the response.
    ///
    /// Returns the `result` field on success, or a [`ClientError::Gateway`]
    /// if the daemon returned an error.
    pub fn call(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, ClientError> {
        let id = self.next_id;
        self.next_id += 1;

        // Build and send the request.
        let request = RpcRequest {
            jsonrpc: "2.0",
            id,
            method: method.to_string(),
            params,
        };

        let mut request_json = serde_json::to_string(&request)
            .map_err(|e| ClientError::Parse(format!("Failed to serialize request: {e}")))?;
        request_json.push('\n');

        self.writer.write_all(request_json.as_bytes())?;
        self.writer.flush()?;

        // Read the response (one newline-delimited JSON line).
        let mut line = String::new();
        self.reader.read_line(&mut line)?;

        if line.is_empty() {
            return Err(ClientError::Connection(
                "Daemon closed the connection unexpectedly".into(),
            ));
        }

        let response: RpcResponse = serde_json::from_str(line.trim())
            .map_err(|e| ClientError::Parse(format!("{e}: {line}")))?;

        // Verify the response ID matches.
        let resp_id = match &response.id {
            serde_json::Value::Number(n) => n.as_u64().unwrap_or(0),
            _ => 0,
        };
        if resp_id != id {
            return Err(ClientError::IdMismatch {
                expected: id,
                got: response.id.to_string(),
            });
        }

        // Check for errors.
        if let Some(err) = response.error {
            return Err(ClientError::Gateway {
                code: err.code,
                message: err.message,
            });
        }

        Ok(response.result.unwrap_or(serde_json::Value::Null))
    }
}
