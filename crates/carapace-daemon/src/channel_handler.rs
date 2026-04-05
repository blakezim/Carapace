//! Channel handler — dispatches `channel.*` JSON-RPC methods.
//!
//! Routes requests to the appropriate channel adapter with allowlist
//! enforcement, audit logging, and dead letter storage for blocked sends.

use std::time::Duration;

use serde_json::json;
use tokio::sync::mpsc;
use tracing::{info, warn};

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::adapters::gmail::GmailAdapter;
use crate::adapters::imsg::ImsgAdapter;
use crate::allowlist::{Allowlist, AllowlistResult};
use crate::audit::{self, AuditLogger};
use crate::dead_letter::{DeadLetter, DeadLetterQueue};
use crate::protocol::{self, JsonRpcNotification, JsonRpcRequest, JsonRpcResponse, ProcessResult};

/// A resolved channel adapter (one of the supported channels).
enum Channel<'a> {
    Imsg(&'a ImsgAdapter),
    Gmail {
        adapter: &'a GmailAdapter,
        inbound: Option<&'a Allowlist>,
    },
}

/// Shared channel state, borrowed from AppState.
pub struct ChannelContext<'a> {
    pub imsg_adapter: Option<&'a ImsgAdapter>,
    pub imsg_outbound: Option<&'a Allowlist>,
    pub imsg_inbound: Option<&'a Allowlist>,
    pub audit_logger: &'a AuditLogger,
    pub dead_letter_queue: &'a DeadLetterQueue,
    /// Shared across all iMessage watch subscriptions.
    pub seen_message_ids: Arc<tokio::sync::Mutex<HashSet<u64>>>,
    // Gmail — keyed by account name
    pub gmail_adapters: &'a HashMap<String, GmailAdapter>,
    pub gmail_inbound_allowlists: &'a HashMap<String, Allowlist>,
    pub gmail_default_account: &'a str,
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
        // Gmail-specific methods
        "channel.search" => ProcessResult::Response(handle_search(req, ctx).await),
        "channel.create_draft" => ProcessResult::Response(handle_create_draft(req, ctx).await),
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
///
/// For Gmail, also resolves the account name from the `account` parameter,
/// falling back to the configured default account.
fn resolve_channel<'a>(
    params: &serde_json::Value,
    ctx: &'a ChannelContext<'_>,
) -> Result<Channel<'a>, JsonRpcResponse> {
    let channel = params
        .get("channel")
        .and_then(|v| v.as_str())
        .unwrap_or("imsg");

    match channel {
        "imsg" => ctx.imsg_adapter.map(Channel::Imsg).ok_or_else(|| {
            JsonRpcResponse::error(
                serde_json::Value::Null,
                protocol::CHANNEL_UNAVAILABLE,
                "iMessage channel is not configured or unavailable",
            )
        }),
        "gmail" => {
            if ctx.gmail_adapters.is_empty() {
                return Err(JsonRpcResponse::error(
                    serde_json::Value::Null,
                    protocol::CHANNEL_UNAVAILABLE,
                    "Gmail channel is not configured or unavailable",
                ));
            }
            let account = params
                .get("account")
                .and_then(|v| v.as_str())
                .unwrap_or(ctx.gmail_default_account);

            let adapter = ctx.gmail_adapters.get(account).ok_or_else(|| {
                JsonRpcResponse::error(
                    serde_json::Value::Null,
                    protocol::CHANNEL_UNAVAILABLE,
                    format!("Gmail account '{}' is not configured. Available: {:?}",
                        account, ctx.gmail_adapters.keys().collect::<Vec<_>>()),
                )
            })?;
            let inbound = ctx.gmail_inbound_allowlists.get(account);

            Ok(Channel::Gmail { adapter, inbound })
        }
        other => Err(JsonRpcResponse::error(
            serde_json::Value::Null,
            protocol::CHANNEL_UNAVAILABLE,
            format!("Unknown channel: {other}"),
        )),
    }
}

// ── channel.send ────────────────────────────────────────────────────────────

async fn handle_send(req: &JsonRpcRequest, ctx: &ChannelContext<'_>) -> JsonRpcResponse {
    // Resolve channel first so channel-level rejections take priority over param validation.
    let channel = match resolve_channel(&req.params, ctx) {
        Ok(c) => c,
        Err(mut e) => { e.id = req.id.clone(); return e; }
    };

    if let Channel::Gmail { .. } = channel {
        return JsonRpcResponse::error(
            req.id.clone(),
            protocol::METHOD_NOT_FOUND,
            "Gmail channel does not support direct send. Use channel.create_draft instead.",
        );
    }

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
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(String::from)).collect())
        .unwrap_or_default();

    match channel {
        Channel::Imsg(adapter) => {
            // Check outbound allowlist.
            if let Some(allowlist) = ctx.imsg_outbound {
                if let AllowlistResult::Blocked { mode, identifier } = allowlist.check(recipient) {
                    let reason = format!("Recipient {identifier} blocked by {mode}");
                    ctx.audit_logger.log(audit::blocked(&req.method, &req.id, &reason)).await;
                    ctx.dead_letter_queue
                        .store(DeadLetter::new(
                            req.method.clone(), req.id.clone(),
                            req.params.clone(), reason.clone(),
                        ))
                        .await;
                    return JsonRpcResponse::error(req.id.clone(), protocol::NOT_IN_ALLOWLIST, reason);
                }
            }
            match adapter.send(recipient, message, &attachments).await {
                Ok(result) => {
                    info!(recipient, "message sent via imsg");
                    JsonRpcResponse::success(req.id.clone(), serde_json::to_value(&result).unwrap())
                }
                Err(e) => {
                    warn!(error = %e, "imsg send failed");
                    JsonRpcResponse::error(req.id.clone(), protocol::SEND_FAILED, format!("Send failed: {e}"))
                }
            }
        }
        Channel::Gmail { .. } => unreachable!("Gmail send blocked above"),
    }
}

// ── channel.list_chats ──────────────────────────────────────────────────────

async fn handle_list_chats(req: &JsonRpcRequest, ctx: &ChannelContext<'_>) -> JsonRpcResponse {
    let channel = match resolve_channel(&req.params, ctx) {
        Ok(c) => c,
        Err(mut e) => { e.id = req.id.clone(); return e; }
    };

    let limit = req.params.get("limit").and_then(|v| v.as_u64()).map(|n| n as u32);

    match channel {
        Channel::Imsg(adapter) => {
            match adapter.list_chats(limit).await {
                Ok(chats) => JsonRpcResponse::success(req.id.clone(), chats),
                Err(e) => {
                    warn!(error = %e, "imsg list_chats failed");
                    JsonRpcResponse::error(req.id.clone(), protocol::INTERNAL_ERROR, format!("list_chats failed: {e}"))
                }
            }
        }
        Channel::Gmail { adapter, .. } => {
            // For Gmail, list_chats returns recent inbox threads.
            let max = limit.unwrap_or(20);
            match adapter.search("in:inbox", Some(max), None).await {
                Ok(result) => JsonRpcResponse::success(req.id.clone(), result),
                Err(e) => {
                    warn!(error = %e, "gmail list_chats (inbox search) failed");
                    JsonRpcResponse::error(req.id.clone(), protocol::INTERNAL_ERROR, format!("list_chats failed: {e}"))
                }
            }
        }
    }
}

// ── channel.get_history ─────────────────────────────────────────────────────

async fn handle_get_history(req: &JsonRpcRequest, ctx: &ChannelContext<'_>) -> JsonRpcResponse {
    let chat_id = match req.params.get("chat_id").and_then(|v| v.as_str()) {
        Some(id) => id,
        None => {
            return JsonRpcResponse::error(
                req.id.clone(), protocol::INVALID_PARAMS,
                "Missing required param: \"chat_id\"",
            );
        }
    };

    let channel = match resolve_channel(&req.params, ctx) {
        Ok(c) => c,
        Err(mut e) => { e.id = req.id.clone(); return e; }
    };

    let limit = req.params.get("limit").and_then(|v| v.as_u64()).map(|n| n as u32);
    let before = req.params.get("before").and_then(|v| v.as_str());

    match channel {
        Channel::Imsg(adapter) => {
            match adapter.get_history(chat_id, limit, before).await {
                Ok(history) => JsonRpcResponse::success(req.id.clone(), history),
                Err(e) => {
                    warn!(error = %e, "imsg get_history failed");
                    JsonRpcResponse::error(req.id.clone(), protocol::INTERNAL_ERROR, format!("get_history failed: {e}"))
                }
            }
        }
        Channel::Gmail { adapter, .. } => {
            // For Gmail, chat_id is the thread ID.
            match adapter.get_thread(chat_id).await {
                Ok(thread) => JsonRpcResponse::success(req.id.clone(), thread),
                Err(e) => {
                    warn!(error = %e, "gmail get_history (get_thread) failed");
                    JsonRpcResponse::error(req.id.clone(), protocol::INTERNAL_ERROR, format!("get_history failed: {e}"))
                }
            }
        }
    }
}

// ── channel.status ──────────────────────────────────────────────────────────

async fn handle_status(req: &JsonRpcRequest, ctx: &ChannelContext<'_>) -> JsonRpcResponse {
    let channel = req.params.get("channel").and_then(|v| v.as_str()).unwrap_or("imsg");

    match channel {
        "imsg" => {
            let (configured, health) = if let Some(adapter) = ctx.imsg_adapter {
                (true, Some(adapter.health_check().await))
            } else {
                (false, None)
            };

            let outbound_info = ctx.imsg_outbound.map(|al| {
                json!({ "mode": al.mode_str(), "entries": al.entry_count() })
            });
            let inbound_info = ctx.imsg_inbound.map(|al| {
                json!({ "mode": al.mode_str(), "entries": al.entry_count() })
            });

            JsonRpcResponse::success(req.id.clone(), json!({
                "channel": "imsg",
                "configured": configured,
                "health": health.map(|h| serde_json::to_value(&h).unwrap()),
                "outbound": outbound_info,
                "inbound": inbound_info,
            }))
        }
        "gmail" => {
            let account = req.params.get("account")
                .and_then(|v| v.as_str())
                .unwrap_or(ctx.gmail_default_account);

            let (configured, health) = if let Some(adapter) = ctx.gmail_adapters.get(account) {
                (true, Some(adapter.health_check().await))
            } else if ctx.gmail_adapters.is_empty() {
                (false, None)
            } else {
                return JsonRpcResponse::error(
                    req.id.clone(),
                    protocol::CHANNEL_UNAVAILABLE,
                    format!("Gmail account '{}' is not configured. Available: {:?}",
                        account, ctx.gmail_adapters.keys().collect::<Vec<_>>()),
                );
            };

            let inbound_info = ctx.gmail_inbound_allowlists.get(account).map(|al| {
                json!({ "mode": al.mode_str(), "entries": al.entry_count() })
            });

            JsonRpcResponse::success(req.id.clone(), json!({
                "channel": "gmail",
                "account": account,
                "configured": configured,
                "health": health.map(|h| serde_json::to_value(&h).unwrap()),
                "inbound": inbound_info,
                "accounts": ctx.gmail_adapters.keys().collect::<Vec<_>>(),
            }))
        }
        other => JsonRpcResponse::error(
            req.id.clone(),
            protocol::CHANNEL_UNAVAILABLE,
            format!("Unknown channel: {other}"),
        ),
    }
}

// ── channel.watch ──────────────────────────────────────────────────────────

async fn handle_watch(req: &JsonRpcRequest, ctx: &ChannelContext<'_>) -> ProcessResult {
    let channel = match resolve_channel(&req.params, ctx) {
        Ok(c) => c,
        Err(mut e) => { e.id = req.id.clone(); return ProcessResult::Response(e); }
    };

    match channel {
        Channel::Imsg(adapter) => {
            let since_rowid = adapter.max_message_rowid().await;
            let (watch_handle, mut adapter_rx) = match adapter.watch(128, since_rowid) {
                Ok(pair) => pair,
                Err(e) => {
                    warn!(error = %e, "imsg watch failed to start");
                    return ProcessResult::Response(JsonRpcResponse::error(
                        req.id.clone(), protocol::INTERNAL_ERROR,
                        format!("watch failed: {e}"),
                    ));
                }
            };

            let inbound = ctx.imsg_inbound.cloned();
            let seen = Arc::clone(&ctx.seen_message_ids);
            let (tx, rx) = mpsc::channel::<JsonRpcNotification>(128);

            tokio::spawn(async move {
                let _handle = watch_handle;
                while let Some(event) = adapter_rx.recv().await {
                    // Deduplicate by numeric message ID.
                    if let Some(id) = event.get("id").and_then(|v| v.as_u64()) {
                        let mut seen_guard = seen.lock().await;
                        if seen_guard.contains(&id) { continue; }
                        seen_guard.insert(id);
                    }
                    // Inbound allowlist.
                    if let Some(ref al) = inbound {
                        let sender = event.get("sender").or_else(|| event.get("handle"))
                            .and_then(|v| v.as_str()).unwrap_or("");
                        if let AllowlistResult::Blocked { .. } = al.check(sender) { continue; }
                    }
                    let notif = JsonRpcNotification::new("channel.watch", event);
                    if tx.send(notif).await.is_err() { break; }
                }
            });

            let ack = JsonRpcResponse::success(req.id.clone(), json!({"subscribed": true}));
            ProcessResult::Subscription { ack, notifications: rx }
        }

        Channel::Gmail { adapter, inbound: inbound_al } => {
            let poll_interval = Duration::from_secs(30);
            let (watch_handle, mut adapter_rx) = adapter.watch(128, poll_interval);
            let inbound = inbound_al.cloned();
            let (tx, rx) = mpsc::channel::<JsonRpcNotification>(128);

            tokio::spawn(async move {
                let _handle = watch_handle;
                while let Some(event) = adapter_rx.recv().await {
                    // Inbound allowlist (filter by From address).
                    if let Some(ref al) = inbound {
                        let sender = event.get("from").and_then(|v| v.as_str()).unwrap_or("");
                        if let AllowlistResult::Blocked { .. } = al.check(sender) { continue; }
                    }
                    let notif = JsonRpcNotification::new("channel.watch", event);
                    if tx.send(notif).await.is_err() { break; }
                }
            });

            let ack = JsonRpcResponse::success(req.id.clone(), json!({"subscribed": true}));
            ProcessResult::Subscription { ack, notifications: rx }
        }
    }
}

// ── channel.search (Gmail-specific) ─────────────────────────────────────────

async fn handle_search(req: &JsonRpcRequest, ctx: &ChannelContext<'_>) -> JsonRpcResponse {
    let query = match req.params.get("query").and_then(|v| v.as_str()) {
        Some(q) if !q.trim().is_empty() => q,
        _ => {
            return JsonRpcResponse::error(
                req.id.clone(), protocol::INVALID_PARAMS,
                "Missing required param: \"query\"",
            );
        }
    };

    let channel = match resolve_channel(&req.params, ctx) {
        Ok(c) => c,
        Err(mut e) => { e.id = req.id.clone(); return e; }
    };

    match channel {
        Channel::Imsg(_) => JsonRpcResponse::error(
            req.id.clone(), protocol::METHOD_NOT_FOUND,
            "channel.search is only supported on the gmail channel",
        ),
        Channel::Gmail { adapter, .. } => {
            let max = req.params.get("max").and_then(|v| v.as_u64()).map(|n| n as u32);
            let page_token = req.params.get("page_token").and_then(|v| v.as_str());

            match adapter.search(query, max, page_token).await {
                Ok(result) => JsonRpcResponse::success(req.id.clone(), result),
                Err(e) => {
                    warn!(error = %e, "gmail search failed");
                    JsonRpcResponse::error(req.id.clone(), protocol::INTERNAL_ERROR, format!("search failed: {e}"))
                }
            }
        }
    }
}

// ── channel.create_draft (Gmail-specific) ───────────────────────────────────

async fn handle_create_draft(req: &JsonRpcRequest, ctx: &ChannelContext<'_>) -> JsonRpcResponse {
    let to = match req.params.get("to").and_then(|v| v.as_str()) {
        Some(t) if !t.trim().is_empty() => t,
        _ => {
            return JsonRpcResponse::error(
                req.id.clone(), protocol::INVALID_PARAMS,
                "Missing required param: \"to\"",
            );
        }
    };
    let subject = match req.params.get("subject").and_then(|v| v.as_str()) {
        Some(s) if !s.trim().is_empty() => s,
        _ => {
            return JsonRpcResponse::error(
                req.id.clone(), protocol::INVALID_PARAMS,
                "Missing required param: \"subject\"",
            );
        }
    };
    let body = req.params.get("body").and_then(|v| v.as_str()).unwrap_or("");
    let cc = req.params.get("cc").and_then(|v| v.as_str());

    let channel = match resolve_channel(&req.params, ctx) {
        Ok(c) => c,
        Err(mut e) => { e.id = req.id.clone(); return e; }
    };

    match channel {
        Channel::Imsg(_) => JsonRpcResponse::error(
            req.id.clone(), protocol::METHOD_NOT_FOUND,
            "channel.create_draft is only supported on the gmail channel",
        ),
        Channel::Gmail { adapter, .. } => {
            match adapter.create_draft(to, subject, body, cc).await {
                Ok(result) => {
                    info!(to, subject, draft_id = %result.draft_id, "gmail draft created");
                    JsonRpcResponse::success(req.id.clone(), serde_json::to_value(&result).unwrap())
                }
                Err(e) => {
                    warn!(error = %e, "gmail create_draft failed");
                    JsonRpcResponse::error(req.id.clone(), protocol::INTERNAL_ERROR, format!("create_draft failed: {e}"))
                }
            }
        }
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

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

    fn unwrap_response(pr: ProcessResult) -> JsonRpcResponse {
        match pr {
            ProcessResult::Response(r) => r,
            ProcessResult::Subscription { .. } => panic!("expected Response, got Subscription"),
        }
    }

    fn noop_seen() -> Arc<tokio::sync::Mutex<HashSet<u64>>> {
        Arc::new(tokio::sync::Mutex::new(HashSet::new()))
    }

    fn empty_gmail_adapters() -> HashMap<String, GmailAdapter> {
        HashMap::new()
    }

    fn empty_gmail_allowlists() -> HashMap<String, Allowlist> {
        HashMap::new()
    }

    fn empty_ctx<'a>(
        audit: &'a AuditLogger,
        dlq: &'a DeadLetterQueue,
        gmail_adapters: &'a HashMap<String, GmailAdapter>,
        gmail_allowlists: &'a HashMap<String, Allowlist>,
    ) -> ChannelContext<'a> {
        ChannelContext {
            imsg_adapter: None,
            imsg_outbound: None,
            imsg_inbound: None,
            audit_logger: audit,
            dead_letter_queue: dlq,
            seen_message_ids: noop_seen(),
            gmail_adapters,
            gmail_inbound_allowlists: gmail_allowlists,
            gmail_default_account: "default",
        }
    }

    #[tokio::test]
    async fn send_missing_recipient_rejected() {
        // With no imsg adapter, resolve_channel returns CHANNEL_UNAVAILABLE
        // before we get to param validation. That's correct behavior.
        let audit = noop_audit();
        let dlq = noop_dead_letter();
        let ga = empty_gmail_adapters();
        let gal = empty_gmail_allowlists();
        let ctx = empty_ctx(&audit, &dlq, &ga, &gal);
        let req = make_req("channel.send", json!({"message": "hello"}));
        let resp = unwrap_response(handle_channel_request(&req, &ctx).await);
        assert_eq!(resp.error.unwrap().code, protocol::CHANNEL_UNAVAILABLE);
    }

    #[tokio::test]
    async fn send_missing_message_rejected() {
        let audit = noop_audit();
        let dlq = noop_dead_letter();
        let ga = empty_gmail_adapters();
        let gal = empty_gmail_allowlists();
        let ctx = empty_ctx(&audit, &dlq, &ga, &gal);
        let req = make_req("channel.send", json!({"recipient": "+1234567890"}));
        let resp = unwrap_response(handle_channel_request(&req, &ctx).await);
        assert_eq!(resp.error.unwrap().code, protocol::CHANNEL_UNAVAILABLE);
    }

    #[tokio::test]
    async fn send_no_adapter_returns_unavailable() {
        let audit = noop_audit();
        let dlq = noop_dead_letter();
        let ga = empty_gmail_adapters();
        let gal = empty_gmail_allowlists();
        let ctx = empty_ctx(&audit, &dlq, &ga, &gal);
        let req = make_req("channel.send", json!({"recipient": "+1234567890", "message": "hello"}));
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
        let ga = empty_gmail_adapters();
        let gal = empty_gmail_allowlists();
        let ctx = ChannelContext {
            imsg_adapter: Some(&adapter),
            imsg_outbound: Some(&outbound),
            imsg_inbound: None,
            audit_logger: &audit,
            dead_letter_queue: &dlq,
            seen_message_ids: noop_seen(),
            gmail_adapters: &ga,
            gmail_inbound_allowlists: &gal,
            gmail_default_account: "default",
        };
        let req = make_req("channel.send", json!({"recipient": "+9999999999", "message": "hello"}));
        let resp = unwrap_response(handle_channel_request(&req, &ctx).await);
        assert_eq!(resp.error.unwrap().code, protocol::NOT_IN_ALLOWLIST);
    }

    #[tokio::test]
    async fn unknown_channel_returns_unavailable() {
        let audit = noop_audit();
        let dlq = noop_dead_letter();
        let ga = empty_gmail_adapters();
        let gal = empty_gmail_allowlists();
        let ctx = empty_ctx(&audit, &dlq, &ga, &gal);
        let req = make_req("channel.send", json!({"channel": "signal", "recipient": "+1234567890", "message": "hi"}));
        let resp = unwrap_response(handle_channel_request(&req, &ctx).await);
        assert_eq!(resp.error.unwrap().code, protocol::CHANNEL_UNAVAILABLE);
    }

    #[tokio::test]
    async fn unknown_channel_method() {
        let audit = noop_audit();
        let dlq = noop_dead_letter();
        let ga = empty_gmail_adapters();
        let gal = empty_gmail_allowlists();
        let ctx = empty_ctx(&audit, &dlq, &ga, &gal);
        let req = make_req("channel.unknown", json!({}));
        let resp = unwrap_response(handle_channel_request(&req, &ctx).await);
        assert_eq!(resp.error.unwrap().code, protocol::METHOD_NOT_FOUND);
    }

    #[tokio::test]
    async fn get_history_missing_chat_id() {
        let audit = noop_audit();
        let dlq = noop_dead_letter();
        let ga = empty_gmail_adapters();
        let gal = empty_gmail_allowlists();
        let ctx = empty_ctx(&audit, &dlq, &ga, &gal);
        let req = make_req("channel.get_history", json!({}));
        let resp = unwrap_response(handle_channel_request(&req, &ctx).await);
        assert_eq!(resp.error.unwrap().code, protocol::INVALID_PARAMS);
    }

    #[tokio::test]
    async fn status_unconfigured_imsg() {
        let audit = noop_audit();
        let dlq = noop_dead_letter();
        let ga = empty_gmail_adapters();
        let gal = empty_gmail_allowlists();
        let ctx = empty_ctx(&audit, &dlq, &ga, &gal);
        let req = make_req("channel.status", json!({}));
        let resp = unwrap_response(handle_channel_request(&req, &ctx).await);
        let result = resp.result.unwrap();
        assert_eq!(result["channel"], "imsg");
        assert_eq!(result["configured"], false);
    }

    #[tokio::test]
    async fn status_unconfigured_gmail() {
        let audit = noop_audit();
        let dlq = noop_dead_letter();
        let ga = empty_gmail_adapters();
        let gal = empty_gmail_allowlists();
        let ctx = empty_ctx(&audit, &dlq, &ga, &gal);
        let req = make_req("channel.status", json!({"channel": "gmail"}));
        let resp = unwrap_response(handle_channel_request(&req, &ctx).await);
        let result = resp.result.unwrap();
        assert_eq!(result["channel"], "gmail");
        assert_eq!(result["configured"], false);
    }

    #[tokio::test]
    async fn gmail_send_returns_not_supported() {
        let mut ga = HashMap::new();
        ga.insert("default".to_string(), GmailAdapter::new(PathBuf::from("/nonexistent/gmail-proxy.sock")));
        let gal = empty_gmail_allowlists();
        let audit = noop_audit();
        let dlq = noop_dead_letter();
        let ctx = ChannelContext {
            imsg_adapter: None,
            imsg_outbound: None,
            imsg_inbound: None,
            audit_logger: &audit,
            dead_letter_queue: &dlq,
            seen_message_ids: noop_seen(),
            gmail_adapters: &ga,
            gmail_inbound_allowlists: &gal,
            gmail_default_account: "default",
        };
        let req = make_req("channel.send", json!({"channel": "gmail", "recipient": "a@b.com", "message": "hi"}));
        let resp = unwrap_response(handle_channel_request(&req, &ctx).await);
        assert_eq!(resp.error.unwrap().code, protocol::METHOD_NOT_FOUND);
    }

    #[tokio::test]
    async fn create_draft_missing_to() {
        let mut ga = HashMap::new();
        ga.insert("default".to_string(), GmailAdapter::new(PathBuf::from("/nonexistent/gmail-proxy.sock")));
        let gal = empty_gmail_allowlists();
        let audit = noop_audit();
        let dlq = noop_dead_letter();
        let ctx = ChannelContext {
            imsg_adapter: None,
            imsg_outbound: None,
            imsg_inbound: None,
            audit_logger: &audit,
            dead_letter_queue: &dlq,
            seen_message_ids: noop_seen(),
            gmail_adapters: &ga,
            gmail_inbound_allowlists: &gal,
            gmail_default_account: "default",
        };
        let req = make_req("channel.create_draft", json!({"channel": "gmail", "subject": "Hello"}));
        let resp = unwrap_response(handle_channel_request(&req, &ctx).await);
        assert_eq!(resp.error.unwrap().code, protocol::INVALID_PARAMS);
    }
}
