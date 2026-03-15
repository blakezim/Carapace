//! Channel handler — dispatches `channel.*` JSON-RPC methods.
//!
//! Routes requests to the appropriate channel adapter with allowlist
//! enforcement, audit logging, and dead letter storage for blocked sends.

use serde_json::json;
use tokio::sync::mpsc;
use tracing::{info, warn};

use crate::adapters::imsg::ImsgAdapter;
use crate::allowlist::{Allowlist, AllowlistResult};
use crate::audit::{self, AuditLogger};
use crate::dead_letter::{DeadLetter, DeadLetterQueue};
use crate::protocol::{self, JsonRpcNotification, JsonRpcRequest, JsonRpcResponse, ProcessResult};

/// Shared channel state, borrowed from AppState.
pub struct ChannelContext<'a> {
    pub imsg_adapter: Option<&'a ImsgAdapter>,
    pub imsg_outbound: Option<&'a Allowlist>,
    pub imsg_inbound: Option<&'a Allowlist>,
    pub audit_logger: &'a AuditLogger,
    pub dead_letter_queue: &'a DeadLetterQueue,
}

/// Handle a `channel.*` JSON-RPC request.
pub async fn handle_channel_request(
    req: &JsonRpcRequest,
    ctx: &ChannelContext<'_>,
) -> ProcessResult {
    info!(method = %req.method, id = %req.id, "handling channel request");

    match req.method.as_str() {
        "channel.send" => ProcessResult::Response(handle_send(req, ctx).await),
        "channel.list_chats" => ProcessResult::Response(handle_list_chats(req, ctx).await),
        "channel.get_history" => ProcessResult::Response(handle_get_history(req, ctx).await),
        "channel.status" => ProcessResult::Response(handle_status(req, ctx).await),
        "channel.watch" => handle_watch(req, ctx).await,
        _ => {
            warn!(method = %req.method, "unknown channel method");
            ProcessResult::Response(JsonRpcResponse::error(
                req.id.clone(),
                protocol::METHOD_NOT_FOUND,
                format!("Unknown method: {}", req.method),
            ))
        }
    }
}

/// Resolve which channel is being targeted and verify it's available.
fn resolve_channel<'a>(
    params: &serde_json::Value,
    ctx: &'a ChannelContext<'_>,
) -> Result<&'a ImsgAdapter, JsonRpcResponse> {
    let channel = params
        .get("channel")
        .and_then(|v| v.as_str())
        .unwrap_or("imsg");

    match channel {
        "imsg" => ctx.imsg_adapter.ok_or_else(|| {
            JsonRpcResponse::error(
                serde_json::Value::Null,
                protocol::CHANNEL_UNAVAILABLE,
                "iMessage channel is not configured or unavailable",
            )
        }),
        other => Err(JsonRpcResponse::error(
            serde_json::Value::Null,
            protocol::CHANNEL_UNAVAILABLE,
            format!("Unknown channel: {other}"),
        )),
    }
}

// ── channel.send ────────────────────────────────────────────────────────────

async fn handle_send(req: &JsonRpcRequest, ctx: &ChannelContext<'_>) -> JsonRpcResponse {
    // Validate required params.
    let recipient = match req.params.get("recipient").and_then(|v| v.as_str()) {
        Some(r) => r,
        None => {
            return JsonRpcResponse::error(
                req.id.clone(),
                protocol::INVALID_PARAMS,
                "Missing required param: \"recipient\"",
            );
        }
    };

    let message = match req.params.get("message").and_then(|v| v.as_str()) {
        Some(m) => m,
        None => {
            return JsonRpcResponse::error(
                req.id.clone(),
                protocol::INVALID_PARAMS,
                "Missing required param: \"message\"",
            );
        }
    };

    let attachments: Vec<String> = req
        .params
        .get("attachments")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    // Resolve channel adapter.
    let adapter = match resolve_channel(&req.params, ctx) {
        Ok(a) => a,
        Err(mut e) => {
            e.id = req.id.clone();
            return e;
        }
    };

    // Check outbound allowlist.
    if let Some(allowlist) = ctx.imsg_outbound {
        if let AllowlistResult::Blocked { mode, identifier } = allowlist.check(recipient) {
            let reason = format!("Recipient {identifier} blocked by {mode}");

            ctx.audit_logger
                .log(audit::blocked(&req.method, &req.id, &reason))
                .await;

            ctx.dead_letter_queue
                .store(DeadLetter::new(
                    req.method.clone(),
                    req.id.clone(),
                    req.params.clone(),
                    reason.clone(),
                ))
                .await;

            return JsonRpcResponse::error(
                req.id.clone(),
                protocol::NOT_IN_ALLOWLIST,
                reason,
            );
        }
    }

    // Call adapter.
    match adapter.send(recipient, message, &attachments).await {
        Ok(result) => {
            info!(recipient, "message sent via imsg");
            JsonRpcResponse::success(req.id.clone(), serde_json::to_value(&result).unwrap())
        }
        Err(e) => {
            warn!(error = %e, "imsg send failed");
            JsonRpcResponse::error(
                req.id.clone(),
                protocol::SEND_FAILED,
                format!("Send failed: {e}"),
            )
        }
    }
}

// ── channel.list_chats ──────────────────────────────────────────────────────

async fn handle_list_chats(req: &JsonRpcRequest, ctx: &ChannelContext<'_>) -> JsonRpcResponse {
    let adapter = match resolve_channel(&req.params, ctx) {
        Ok(a) => a,
        Err(mut e) => {
            e.id = req.id.clone();
            return e;
        }
    };

    let limit = req
        .params
        .get("limit")
        .and_then(|v| v.as_u64())
        .map(|n| n as u32);

    match adapter.list_chats(limit).await {
        Ok(chats) => JsonRpcResponse::success(req.id.clone(), chats),
        Err(e) => {
            warn!(error = %e, "imsg list_chats failed");
            JsonRpcResponse::error(
                req.id.clone(),
                protocol::INTERNAL_ERROR,
                format!("list_chats failed: {e}"),
            )
        }
    }
}

// ── channel.get_history ─────────────────────────────────────────────────────

async fn handle_get_history(req: &JsonRpcRequest, ctx: &ChannelContext<'_>) -> JsonRpcResponse {
    let chat_id = match req.params.get("chat_id").and_then(|v| v.as_str()) {
        Some(id) => id,
        None => {
            return JsonRpcResponse::error(
                req.id.clone(),
                protocol::INVALID_PARAMS,
                "Missing required param: \"chat_id\"",
            );
        }
    };

    let adapter = match resolve_channel(&req.params, ctx) {
        Ok(a) => a,
        Err(mut e) => {
            e.id = req.id.clone();
            return e;
        }
    };

    let limit = req
        .params
        .get("limit")
        .and_then(|v| v.as_u64())
        .map(|n| n as u32);

    let before = req.params.get("before").and_then(|v| v.as_str());

    match adapter.get_history(chat_id, limit, before).await {
        Ok(history) => JsonRpcResponse::success(req.id.clone(), history),
        Err(e) => {
            warn!(error = %e, "imsg get_history failed");
            JsonRpcResponse::error(
                req.id.clone(),
                protocol::INTERNAL_ERROR,
                format!("get_history failed: {e}"),
            )
        }
    }
}

// ── channel.status ──────────────────────────────────────────────────────────

async fn handle_status(req: &JsonRpcRequest, ctx: &ChannelContext<'_>) -> JsonRpcResponse {
    let channel = req
        .params
        .get("channel")
        .and_then(|v| v.as_str())
        .unwrap_or("imsg");

    match channel {
        "imsg" => {
            let (configured, health) = if let Some(adapter) = ctx.imsg_adapter {
                (true, Some(adapter.health_check().await))
            } else {
                (false, None)
            };

            let outbound_info = ctx.imsg_outbound.map(|al| {
                json!({
                    "mode": al.mode_str(),
                    "entries": al.entry_count(),
                })
            });

            let inbound_info = ctx.imsg_inbound.map(|al| {
                json!({
                    "mode": al.mode_str(),
                    "entries": al.entry_count(),
                })
            });

            JsonRpcResponse::success(
                req.id.clone(),
                json!({
                    "channel": "imsg",
                    "configured": configured,
                    "health": health.map(|h| serde_json::to_value(&h).unwrap()),
                    "outbound": outbound_info,
                    "inbound": inbound_info,
                }),
            )
        }
        other => JsonRpcResponse::error(
            req.id.clone(),
            protocol::CHANNEL_UNAVAILABLE,
            format!("Unknown channel: {other}"),
        ),
    }
}

// ── channel.watch ──────────────────────────────────────────────────────

async fn handle_watch(req: &JsonRpcRequest, ctx: &ChannelContext<'_>) -> ProcessResult {
    let adapter = match resolve_channel(&req.params, ctx) {
        Ok(a) => a,
        Err(mut e) => {
            e.id = req.id.clone();
            return ProcessResult::Response(e);
        }
    };

    let (watch_handle, mut adapter_rx) = match adapter.watch(128) {
        Ok(pair) => pair,
        Err(e) => {
            warn!(error = %e, "imsg watch failed to start");
            return ProcessResult::Response(JsonRpcResponse::error(
                req.id.clone(),
                protocol::INTERNAL_ERROR,
                format!("watch failed: {e}"),
            ));
        }
    };

    let inbound = ctx.imsg_inbound.cloned();
    let (tx, rx) = mpsc::channel::<JsonRpcNotification>(128);

    // Spawn a filtering task that reads from the adapter and applies
    // the inbound allowlist before forwarding notifications.
    tokio::spawn(async move {
        let _handle = watch_handle; // move ownership so child lives until task ends

        while let Some(event) = adapter_rx.recv().await {
            // Check inbound allowlist if configured.
            if let Some(ref al) = inbound {
                let sender = event
                    .get("sender")
                    .or_else(|| event.get("handle"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                if let AllowlistResult::Blocked { .. } = al.check(sender) {
                    continue; // silently drop
                }
            }

            let notification =
                JsonRpcNotification::new("channel.watch", event);
            if tx.send(notification).await.is_err() {
                break; // client disconnected
            }
        }
    });

    let ack = JsonRpcResponse::success(
        req.id.clone(),
        json!({"subscribed": true}),
    );

    ProcessResult::Subscription {
        ack,
        notifications: rx,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audit::AuditLogger;
    use crate::config::{AllowlistMode, DirectionConfig};
    use crate::dead_letter::DeadLetterQueue;
    use std::path::PathBuf;

    fn make_req(method: &str, params: serde_json::Value) -> JsonRpcRequest {
        JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: json!(1),
            method: method.into(),
            params,
        }
    }

    fn noop_audit() -> AuditLogger {
        AuditLogger::new(PathBuf::from("/dev/null"), false)
    }

    fn noop_dead_letter() -> DeadLetterQueue {
        DeadLetterQueue::new(PathBuf::from("/tmp/carapace-test-channel-dead-letters"))
    }

    /// Helper: unwrap ProcessResult::Response, panicking on Subscription.
    fn unwrap_response(pr: ProcessResult) -> JsonRpcResponse {
        match pr {
            ProcessResult::Response(r) => r,
            ProcessResult::Subscription { .. } => panic!("expected Response, got Subscription"),
        }
    }

    #[tokio::test]
    async fn send_missing_recipient_rejected() {
        let audit = noop_audit();
        let dlq = noop_dead_letter();
        let ctx = ChannelContext {
            imsg_adapter: None,
            imsg_outbound: None,
            imsg_inbound: None,
            audit_logger: &audit,
            dead_letter_queue: &dlq,
        };

        let req = make_req("channel.send", json!({"message": "hello"}));
        let resp = unwrap_response(handle_channel_request(&req, &ctx).await);
        assert_eq!(resp.error.unwrap().code, protocol::INVALID_PARAMS);
    }

    #[tokio::test]
    async fn send_missing_message_rejected() {
        let audit = noop_audit();
        let dlq = noop_dead_letter();
        let ctx = ChannelContext {
            imsg_adapter: None,
            imsg_outbound: None,
            imsg_inbound: None,
            audit_logger: &audit,
            dead_letter_queue: &dlq,
        };

        let req = make_req("channel.send", json!({"recipient": "+1234567890"}));
        let resp = unwrap_response(handle_channel_request(&req, &ctx).await);
        assert_eq!(resp.error.unwrap().code, protocol::INVALID_PARAMS);
    }

    #[tokio::test]
    async fn send_no_adapter_returns_unavailable() {
        let audit = noop_audit();
        let dlq = noop_dead_letter();
        let ctx = ChannelContext {
            imsg_adapter: None,
            imsg_outbound: None,
            imsg_inbound: None,
            audit_logger: &audit,
            dead_letter_queue: &dlq,
        };

        let req = make_req(
            "channel.send",
            json!({"recipient": "+1234567890", "message": "hello"}),
        );
        let resp = unwrap_response(handle_channel_request(&req, &ctx).await);
        assert_eq!(resp.error.unwrap().code, protocol::CHANNEL_UNAVAILABLE);
    }

    #[tokio::test]
    async fn send_blocked_by_allowlist() {
        let adapter = ImsgAdapter::new(
            PathBuf::from("/nonexistent/imsg"),
            PathBuf::from("/nonexistent/chat.db"),
        );
        let outbound = Allowlist::new(&DirectionConfig {
            mode: AllowlistMode::Allowlist,
            allowlist: vec!["+1111111111".into()],
        });
        let audit = noop_audit();
        let dlq = noop_dead_letter();
        let ctx = ChannelContext {
            imsg_adapter: Some(&adapter),
            imsg_outbound: Some(&outbound),
            imsg_inbound: None,
            audit_logger: &audit,
            dead_letter_queue: &dlq,
        };

        let req = make_req(
            "channel.send",
            json!({"recipient": "+9999999999", "message": "hello"}),
        );
        let resp = unwrap_response(handle_channel_request(&req, &ctx).await);
        assert_eq!(resp.error.unwrap().code, protocol::NOT_IN_ALLOWLIST);
    }

    #[tokio::test]
    async fn unknown_channel_returns_unavailable() {
        let audit = noop_audit();
        let dlq = noop_dead_letter();
        let ctx = ChannelContext {
            imsg_adapter: None,
            imsg_outbound: None,
            imsg_inbound: None,
            audit_logger: &audit,
            dead_letter_queue: &dlq,
        };

        let req = make_req(
            "channel.send",
            json!({"channel": "signal", "recipient": "+1234567890", "message": "hi"}),
        );
        let resp = unwrap_response(handle_channel_request(&req, &ctx).await);
        assert_eq!(resp.error.unwrap().code, protocol::CHANNEL_UNAVAILABLE);
    }

    #[tokio::test]
    async fn unknown_channel_method() {
        let audit = noop_audit();
        let dlq = noop_dead_letter();
        let ctx = ChannelContext {
            imsg_adapter: None,
            imsg_outbound: None,
            imsg_inbound: None,
            audit_logger: &audit,
            dead_letter_queue: &dlq,
        };

        let req = make_req("channel.unknown", json!({}));
        let resp = unwrap_response(handle_channel_request(&req, &ctx).await);
        assert_eq!(resp.error.unwrap().code, protocol::METHOD_NOT_FOUND);
    }

    #[tokio::test]
    async fn get_history_missing_chat_id() {
        let audit = noop_audit();
        let dlq = noop_dead_letter();
        let ctx = ChannelContext {
            imsg_adapter: None,
            imsg_outbound: None,
            imsg_inbound: None,
            audit_logger: &audit,
            dead_letter_queue: &dlq,
        };

        let req = make_req("channel.get_history", json!({}));
        let resp = unwrap_response(handle_channel_request(&req, &ctx).await);
        assert_eq!(resp.error.unwrap().code, protocol::INVALID_PARAMS);
    }

    #[tokio::test]
    async fn status_unconfigured_channel() {
        let audit = noop_audit();
        let dlq = noop_dead_letter();
        let ctx = ChannelContext {
            imsg_adapter: None,
            imsg_outbound: None,
            imsg_inbound: None,
            audit_logger: &audit,
            dead_letter_queue: &dlq,
        };

        let req = make_req("channel.status", json!({}));
        let resp = unwrap_response(handle_channel_request(&req, &ctx).await);
        let result = resp.result.unwrap();
        assert_eq!(result["channel"], "imsg");
        assert_eq!(result["configured"], false);
    }
}
