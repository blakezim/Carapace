//! Unix socket server – accepts connections and processes JSON-RPC messages.

use std::path::Path;
use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tracing::{error, info, warn};

use crate::handler;
use crate::protocol::{self, JsonRpcRequest, JsonRpcResponse};

/// Shared state available to every connection handler.
///
/// In later phases this will hold config, middleware, and adapters.
pub struct AppState {
    // Placeholder – will hold Config, RateLimiter, Allowlist, etc.
}

impl AppState {
    pub fn new() -> Self {
        Self {}
    }
}

/// Start the Unix socket server, listening at `socket_path`.
///
/// This function runs forever (until the process is killed).
pub async fn run(socket_path: &Path) -> std::io::Result<()> {
    // Clean up stale socket from a previous run.
    if socket_path.exists() {
        info!(?socket_path, "removing stale socket");
        std::fs::remove_file(socket_path)?;
    }

    // Ensure the parent directory exists.
    if let Some(parent) = socket_path.parent() {
        if !parent.exists() {
            std::fs::create_dir_all(parent)?;
            info!(?parent, "created socket directory");
        }
    }

    let listener = UnixListener::bind(socket_path)?;
    info!(?socket_path, "daemon listening");

    // Set socket permissions: owner + group can read/write (0o770).
    // This allows the carapace user and carapace-clients group to connect.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o770);
        std::fs::set_permissions(socket_path, perms)?;
        info!("socket permissions set to 0770");
    }

    let state = Arc::new(AppState::new());

    loop {
        match listener.accept().await {
            Ok((stream, _addr)) => {
                let state = Arc::clone(&state);
                tokio::spawn(async move {
                    if let Err(e) = handle_connection(stream, state).await {
                        warn!(error = %e, "connection error");
                    }
                });
            }
            Err(e) => {
                error!(error = %e, "accept error");
            }
        }
    }
}

/// Handle a single client connection.
///
/// Reads newline-delimited JSON-RPC requests and writes back responses.
/// The connection stays open until the client disconnects.
async fn handle_connection(stream: UnixStream, _state: Arc<AppState>) -> std::io::Result<()> {
    let (reader, mut writer) = stream.into_split();
    let mut reader = BufReader::new(reader);
    let mut line = String::new();

    info!("client connected");

    loop {
        line.clear();
        let bytes_read = reader.read_line(&mut line).await?;

        if bytes_read == 0 {
            info!("client disconnected");
            break;
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let response = process_message(trimmed);

        // Serialize and send, terminated by newline.
        let mut resp_json = serde_json::to_string(&response).unwrap_or_else(|e| {
            // Last resort – should never happen.
            format!(
                r#"{{"jsonrpc":"2.0","id":null,"error":{{"code":{},"message":"Serialization failed: {}"}}}}"#,
                protocol::INTERNAL_ERROR,
                e
            )
        });
        resp_json.push('\n');
        writer.write_all(resp_json.as_bytes()).await?;
    }

    Ok(())
}

/// Parse a raw JSON line into a request and dispatch it.
fn process_message(raw: &str) -> JsonRpcResponse {
    // 1. Try to parse as JSON
    let req: JsonRpcRequest = match serde_json::from_str(raw) {
        Ok(r) => r,
        Err(e) => {
            warn!(error = %e, "parse error");
            return JsonRpcResponse::error(
                serde_json::Value::Null,
                protocol::PARSE_ERROR,
                format!("Parse error: {e}"),
            );
        }
    };

    // 2. Validate JSON-RPC structure
    if let Err(e) = req.validate() {
        return JsonRpcResponse::error(
            req.id.clone(),
            protocol::INVALID_REQUEST,
            format!("Invalid request: {e}"),
        );
    }

    // 3. Dispatch to handler
    handler::handle_request(&req)
}
