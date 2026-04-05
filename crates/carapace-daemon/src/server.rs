//! Unix socket server – accepts connections and processes JSON-RPC messages.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tracing::{error, info, warn};

use crate::adapters::gmail::GmailAdapter;
use crate::adapters::imsg::ImsgAdapter;
use crate::allowlist::Allowlist;
use crate::audit::AuditLogger;
use crate::channel_handler::{self, ChannelContext};
use crate::config::Config;
use crate::content_filter::ContentFilter;
use crate::dead_letter::DeadLetterQueue;
use crate::handler;
use crate::middleware::{self, MiddlewareVerdict};
use crate::protocol::{self, JsonRpcRequest, JsonRpcResponse, ProcessResult};
use crate::rate_limiter::RateLimiter;

/// Shared state available to every connection handler.
pub struct AppState {
    pub rate_limiter: RateLimiter,
    pub content_filter: ContentFilter,
    pub audit_logger: AuditLogger,
    pub dead_letter_queue: DeadLetterQueue,
    // iMessage channel
    pub imsg_adapter: Option<ImsgAdapter>,
    pub imsg_outbound: Option<Allowlist>,
    pub imsg_inbound: Option<Allowlist>,
    /// iMessage IDs already forwarded to a watch subscriber.
    pub seen_message_ids: Arc<tokio::sync::Mutex<std::collections::HashSet<u64>>>,
    // Gmail channel
    pub gmail_adapter: Option<GmailAdapter>,
    pub gmail_inbound: Option<Allowlist>,
}

impl AppState {
    pub fn new(config: &Config) -> Self {
        // Build iMessage adapter and allowlists if the channel is configured.
        let (imsg_adapter, imsg_outbound, imsg_inbound) =
            if let Some(ref imsg_config) = config.channels.imsg {
                if !imsg_config.enabled {
                    tracing::info!("imsg channel is disabled in config");
                    (None, None, None)
                } else if !imsg_config.real_binary.exists() {
                    tracing::warn!(
                        path = %imsg_config.real_binary.display(),
                        "imsg channel enabled but binary not found — channel unavailable"
                    );
                    (
                        None,
                        Some(Allowlist::new(&imsg_config.outbound)),
                        Some(Allowlist::new(&imsg_config.inbound)),
                    )
                } else {
                    tracing::info!(
                        binary = %imsg_config.real_binary.display(),
                        "imsg channel enabled"
                    );
                    (
                        Some(ImsgAdapter::new(
                            imsg_config.real_binary.clone(),
                            imsg_config.db_path.clone(),
                        )),
                        Some(Allowlist::new(&imsg_config.outbound)),
                        Some(Allowlist::new(&imsg_config.inbound)),
                    )
                }
            } else {
                (None, None, None)
            };

        // Build Gmail adapter if configured.
        let (gmail_adapter, gmail_inbound) =
            if let Some(ref gmail_config) = config.channels.gmail {
                if !gmail_config.enabled {
                    tracing::info!("gmail channel is disabled in config");
                    (None, None)
                } else {
                    tracing::info!(
                        socket = %gmail_config.proxy_socket.display(),
                        "gmail channel enabled"
                    );
                    (
                        Some(GmailAdapter::new(gmail_config.proxy_socket.clone())),
                        Some(Allowlist::new(&gmail_config.inbound)),
                    )
                }
            } else {
                (None, None)
            };

        Self {
            rate_limiter: RateLimiter::new(config.security.rate_limit.clone()),
            content_filter: ContentFilter::new(&config.security.content_filter),
            audit_logger: AuditLogger::new(
                config.security.audit_log_path.clone(),
                config.security.audit_enabled,
            ),
            dead_letter_queue: DeadLetterQueue::new(config.security.dead_letter_path.clone()),
            imsg_adapter,
            imsg_outbound,
            imsg_inbound,
            seen_message_ids: Arc::new(tokio::sync::Mutex::new(std::collections::HashSet::new())),
            gmail_adapter,
            gmail_inbound,
        }
    }
}

/// Start the Unix socket server, listening at `socket_path`.
///
/// This function runs forever (until the process is killed).
pub async fn run(socket_path: &Path, config: Config) -> std::io::Result<()> {
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

    // Set socket ownership to carapace:carapace-clients and permissions to 0770.
    // This allows the carapace user and carapace-clients group members to connect.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o770);
        std::fs::set_permissions(socket_path, perms)?;

        // Set group to carapace-clients so cross-user connections work.
        if let Some(group) = nix::unistd::Group::from_name("carapace-clients")
            .ok()
            .flatten()
        {
            nix::unistd::chown(socket_path, None, Some(group.gid))?;
            info!(group = "carapace-clients", "socket permissions set to 0770");
        } else {
            warn!("carapace-clients group not found — socket may not be accessible to clients");
            info!("socket permissions set to 0770");
        }
    }

    let state = Arc::new(AppState::new(&config));

    // Spawn background cleanup task for the rate limiter.
    {
        let state = Arc::clone(&state);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(300));
            loop {
                interval.tick().await;
                state.rate_limiter.cleanup();
            }
        });
    }

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

/// Serialize a JSON-RPC response and write it as a newline-terminated line.
async fn write_response(
    writer: &mut tokio::net::unix::OwnedWriteHalf,
    response: &JsonRpcResponse,
) -> std::io::Result<()> {
    let mut json = serde_json::to_string(response).unwrap_or_else(|e| {
        format!(
            r#"{{"jsonrpc":"2.0","id":null,"error":{{"code":{},"message":"Serialization failed: {}"}}}}"#,
            protocol::INTERNAL_ERROR,
            e
        )
    });
    json.push('\n');
    writer.write_all(json.as_bytes()).await
}

/// Handle a single client connection.
///
/// Reads newline-delimited JSON-RPC requests and writes back responses.
/// The connection stays open until the client disconnects.
async fn handle_connection(stream: UnixStream, state: Arc<AppState>) -> std::io::Result<()> {
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

        let result = process_message(trimmed, &state).await;

        match result {
            ProcessResult::Response(response) => {
                write_response(&mut writer, &response).await?;
            }
            ProcessResult::Subscription {
                ack,
                mut notifications,
            } => {
                // Send the ack first.
                write_response(&mut writer, &ack).await?;

                // Enter streaming loop: forward notifications until
                // the stream ends or the client disconnects.
                loop {
                    tokio::select! {
                        maybe_notif = notifications.recv() => {
                            match maybe_notif {
                                Some(notif) => {
                                    let mut json = serde_json::to_string(&notif)
                                        .unwrap_or_default();
                                    json.push('\n');
                                    if writer.write_all(json.as_bytes()).await.is_err() {
                                        info!("client disconnected during stream");
                                        return Ok(());
                                    }
                                }
                                None => {
                                    // Notification channel closed (watch process exited).
                                    info!("watch stream ended");
                                    break;
                                }
                            }
                        }
                        bytes = reader.read_line(&mut line) => {
                            match bytes {
                                Ok(0) | Err(_) => {
                                    info!("client disconnected during stream");
                                    return Ok(());
                                }
                                Ok(_) => {
                                    // Client sent data during streaming — ignore it.
                                    line.clear();
                                }
                            }
                        }
                    }
                }
                // Stream ended — close connection. The client's subscribe()
                // consumed the GatewayClient, so no more requests will come.
                return Ok(());
            }
        }
    }

    Ok(())
}

/// Parse a raw JSON line into a request, run middleware, and dispatch.
async fn process_message(raw: &str, state: &AppState) -> ProcessResult {
    // 1. Try to parse as JSON
    let req: JsonRpcRequest = match serde_json::from_str(raw) {
        Ok(r) => r,
        Err(e) => {
            warn!(error = %e, "parse error");
            return ProcessResult::Response(JsonRpcResponse::error(
                serde_json::Value::Null,
                protocol::PARSE_ERROR,
                format!("Parse error: {e}"),
            ));
        }
    };

    // 2. Validate JSON-RPC structure
    if let Err(e) = req.validate() {
        return ProcessResult::Response(JsonRpcResponse::error(
            req.id.clone(),
            protocol::INVALID_REQUEST,
            format!("Invalid request: {e}"),
        ));
    }

    // 3. Run security middleware pipeline
    let raw_params = serde_json::to_string(&req.params).unwrap_or_default();
    match middleware::run_pipeline(
        &req,
        &raw_params,
        &state.rate_limiter,
        &state.content_filter,
        &state.audit_logger,
        &state.dead_letter_queue,
    )
    .await
    {
        MiddlewareVerdict::Allow => {}
        MiddlewareVerdict::Reject(response) => return ProcessResult::Response(response),
    }

    // 4. Dispatch to handler
    if req.method.starts_with("channel.") {
        let ctx = ChannelContext {
            imsg_adapter: state.imsg_adapter.as_ref(),
            imsg_outbound: state.imsg_outbound.as_ref(),
            imsg_inbound: state.imsg_inbound.as_ref(),
            audit_logger: &state.audit_logger,
            dead_letter_queue: &state.dead_letter_queue,
            seen_message_ids: Arc::clone(&state.seen_message_ids),
            gmail_adapter: state.gmail_adapter.as_ref(),
            gmail_inbound: state.gmail_inbound.as_ref(),
        };
        channel_handler::handle_channel_request(&req, &ctx).await
    } else {
        ProcessResult::Response(handler::handle_request(&req))
    }
}
