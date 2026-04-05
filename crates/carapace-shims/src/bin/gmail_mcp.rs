//! gmail-mcp — MCP (Model Context Protocol) server for the Carapace Gmail channel.
//!
//! Speaks the MCP stdio transport protocol so that any MCP-capable agent
//! (Claude Code, OpenClaw via acpx, etc.) can call Gmail tools directly.
//!
//! ## Transport
//! Reads newline-delimited JSON-RPC 2.0 from **stdin**, writes responses to
//! **stdout**. Diagnostic messages go to **stderr** only — stdout is reserved
//! for the MCP wire protocol.
//!
//! ## Tools exposed
//! | Tool | Description |
//! |------|-------------|
//! | `gmail_search` | Search emails using Gmail query syntax |
//! | `gmail_read_thread` | Fetch all messages in a thread |
//! | `gmail_create_draft` | Create a draft email (never sent automatically) |
//! | `gmail_status` | Check gateway and OAuth token health |
//!
//! ## Usage
//! Claude Code — add to `.claude/settings.json`:
//! ```json
//! {
//!   "mcpServers": {
//!     "gmail": {
//!       "command": "sudo",
//!       "args": ["-u", "carapace", "/usr/local/bin/gmail-mcp"]
//!     }
//!   }
//! }
//! ```
//!
//! OpenClaw (acpx plugin config):
//! ```json
//! {
//!   "mcpServers": {
//!     "gmail": {
//!       "command": "sudo",
//!       "args": ["-u", "carapace", "/usr/local/bin/gmail-mcp"]
//!     }
//!   }
//! }
//! ```

use std::io::{BufRead, Write};

use carapace_client::GatewayClient;
use serde_json::{json, Value};

// ── MCP protocol constants ───────────────────────────────────────────────────

const PROTOCOL_VERSION: &str = "2024-11-05";
const SERVER_NAME: &str = "carapace-gmail";
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

// ── Entry point ──────────────────────────────────────────────────────────────

fn main() {
    eprintln!("[gmail-mcp] starting — waiting for MCP messages on stdin");

    // Optional: target a specific Gmail account in the daemon's multi-account config.
    let gmail_account: Option<String> = std::env::var("GMAIL_ACCOUNT").ok();
    if let Some(ref acct) = gmail_account {
        eprintln!("[gmail-mcp] targeting account: {acct}");
    }

    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    // Lazy-connect to the gateway: we create the client on first tool call so
    // that the MCP handshake succeeds even if the daemon isn't running yet.
    let mut client: Option<GatewayClient> = None;

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) if l.trim().is_empty() => continue,
            Ok(l) => l,
            Err(e) => {
                eprintln!("[gmail-mcp] stdin read error: {e}");
                break;
            }
        };

        eprintln!("[gmail-mcp] recv: {line}");

        let req: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                let resp = json!({
                    "jsonrpc": "2.0",
                    "id": null,
                    "error": {"code": -32700, "message": format!("Parse error: {e}")}
                });
                writeln!(out, "{}", resp).ok();
                out.flush().ok();
                continue;
            }
        };

        let id = req.get("id").cloned().unwrap_or(Value::Null);
        let method = req.get("method").and_then(|v| v.as_str()).unwrap_or("");
        let params = req.get("params").cloned().unwrap_or(json!({}));

        let response = handle_message(method, &params, id.clone(), &mut client, &gmail_account);
        // Notifications return Null — nothing to write back.
        if response.is_null() {
            continue;
        }
        eprintln!("[gmail-mcp] send: {response}");
        writeln!(out, "{response}").ok();
        out.flush().ok();
    }

    eprintln!("[gmail-mcp] stdin closed — exiting");
}

// ── MCP message dispatcher ───────────────────────────────────────────────────

fn handle_message(method: &str, params: &Value, id: Value, client: &mut Option<GatewayClient>, gmail_account: &Option<String>) -> Value {
    match method {
        // ── MCP handshake ────────────────────────────────────────────────────
        "initialize" => {
            json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "protocolVersion": PROTOCOL_VERSION,
                    "capabilities": {
                        "tools": {}
                    },
                    "serverInfo": {
                        "name": SERVER_NAME,
                        "version": SERVER_VERSION
                    }
                }
            })
        }

        // Notification — no response required or expected.
        "notifications/initialized" => {
            eprintln!("[gmail-mcp] initialized");
            return Value::Null;
        }

        // ── Tool discovery ───────────────────────────────────────────────────
        "tools/list" => {
            json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "tools": tool_definitions()
                }
            })
        }

        // ── Tool invocation ──────────────────────────────────────────────────
        "tools/call" => {
            let tool_name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or(json!({}));

            // Connect to gateway on first tool call.
            if client.is_none() {
                match GatewayClient::connect_default() {
                    Ok(c) => {
                        eprintln!("[gmail-mcp] connected to gateway");
                        *client = Some(c);
                    }
                    Err(e) => {
                        return tool_error(id, format!(
                            "Cannot connect to Carapace daemon: {e}. \
                             Make sure carapace-daemon is running."
                        ));
                    }
                }
            }

            let gw = client.as_mut().unwrap();
            call_tool(tool_name, &args, id, gw, gmail_account)
        }

        // ── Unknown method ───────────────────────────────────────────────────
        _ => {
            json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {"code": -32601, "message": format!("Method not found: {method}")}
            })
        }
    }
}

// ── Tool definitions (returned by tools/list) ────────────────────────────────
//
// These descriptions are what the agent reads to understand how to use each
// tool. Be specific: name the query syntax, describe the return shape, note
// constraints. Agents use this text as their only documentation.

fn tool_definitions() -> Value {
    json!([
        {
            "name": "gmail_search",
            "description": "\
Search emails in Gmail using Gmail's standard query syntax. \
Returns a list of matching messages with id, thread_id, subject, from, date, and a \
plain-text snippet of the body (OTP codes and auth URLs are pre-scrubbed). \
Supported operators: from:, to:, subject:, is:unread, is:read, has:attachment, \
after:, before:, older_than:, newer_than:, in:inbox, in:sent, label:, cc:, bcc:, filename:. \
Operators that access trash, spam, or all-mail are blocked. \
Example queries: \"from:boss@company.com is:unread\", \"subject:invoice newer_than:7d\".",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Gmail search query (same syntax as the Gmail search box). Required."
                    },
                    "max": {
                        "type": "integer",
                        "description": "Maximum number of messages to return. Defaults to 20, max 50.",
                        "default": 20
                    }
                },
                "required": ["query"]
            }
        },
        {
            "name": "gmail_read_thread",
            "description": "\
Fetch all messages in a Gmail thread by its thread_id. \
Use the thread_id from gmail_search results to retrieve the full conversation. \
Returns an array of messages in chronological order, each with from, to, date, subject, \
and plain-text body (scrubbed). This is the right tool when you want to read a full \
email conversation rather than just the snippet from search.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "thread_id": {
                        "type": "string",
                        "description": "The Gmail thread ID to fetch (from gmail_search result's thread_id field)."
                    }
                },
                "required": ["thread_id"]
            }
        },
        {
            "name": "gmail_create_draft",
            "description": "\
Create a Gmail draft email. The draft is saved to the Drafts folder and is NOT sent \
automatically — a human must open Gmail and send it manually. \
Use this when you need to compose an email for review before sending. \
Returns the draft_id of the created draft.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "to": {
                        "type": "string",
                        "description": "Recipient email address."
                    },
                    "subject": {
                        "type": "string",
                        "description": "Email subject line."
                    },
                    "body": {
                        "type": "string",
                        "description": "Plain-text email body."
                    },
                    "cc": {
                        "type": "string",
                        "description": "Optional CC email address."
                    }
                },
                "required": ["to", "subject", "body"]
            }
        },
        {
            "name": "gmail_status",
            "description": "\
Check the health of the Gmail channel. Returns whether the gmail-proxy is reachable \
and whether the OAuth token is valid and when it expires. \
Use this to diagnose connection problems before calling other Gmail tools.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "required": []
            }
        }
    ])
}

// ── Tool call dispatcher ─────────────────────────────────────────────────────

/// Inject the `account` field into a gateway params object if an account is configured.
fn with_account(mut params: Value, gmail_account: &Option<String>) -> Value {
    if let Some(ref acct) = gmail_account {
        params["account"] = json!(acct);
    }
    params
}

fn call_tool(name: &str, args: &Value, id: Value, gw: &mut GatewayClient, gmail_account: &Option<String>) -> Value {
    match name {
        "gmail_search" => {
            let query = match args.get("query").and_then(|v| v.as_str()) {
                Some(q) => q,
                None => return tool_error(id, "Missing required argument: \"query\""),
            };
            let max = args.get("max").and_then(|v| v.as_u64()).unwrap_or(20);

            let gw_params = with_account(json!({
                "channel": "gmail",
                "query": query,
                "max": max,
            }), gmail_account);

            match gw.call("channel.search", gw_params) {
                Ok(result) => tool_success(id, result),
                Err(e) => tool_error(id, format!("gmail_search failed: {e}")),
            }
        }

        "gmail_read_thread" => {
            let thread_id = match args.get("thread_id").and_then(|v| v.as_str()) {
                Some(t) => t,
                None => return tool_error(id, "Missing required argument: \"thread_id\""),
            };

            let gw_params = with_account(json!({
                "channel": "gmail",
                "chat_id": thread_id,
            }), gmail_account);

            match gw.call("channel.get_history", gw_params) {
                Ok(result) => tool_success(id, result),
                Err(e) => tool_error(id, format!("gmail_read_thread failed: {e}")),
            }
        }

        "gmail_create_draft" => {
            let to = match args.get("to").and_then(|v| v.as_str()) {
                Some(v) => v,
                None => return tool_error(id, "Missing required argument: \"to\""),
            };
            let subject = match args.get("subject").and_then(|v| v.as_str()) {
                Some(v) => v,
                None => return tool_error(id, "Missing required argument: \"subject\""),
            };
            let body = match args.get("body").and_then(|v| v.as_str()) {
                Some(v) => v,
                None => return tool_error(id, "Missing required argument: \"body\""),
            };

            let mut gw_params = with_account(json!({
                "channel": "gmail",
                "to": to,
                "subject": subject,
                "body": body,
            }), gmail_account);
            if let Some(cc) = args.get("cc").and_then(|v| v.as_str()) {
                gw_params["cc"] = json!(cc);
            }

            match gw.call("channel.create_draft", gw_params) {
                Ok(result) => tool_success(id, result),
                Err(e) => tool_error(id, format!("gmail_create_draft failed: {e}")),
            }
        }

        "gmail_status" => {
            let gw_params = with_account(json!({"channel": "gmail"}), gmail_account);
            match gw.call("channel.status", gw_params) {
                Ok(result) => tool_success(id, result),
                Err(e) => tool_error(id, format!("gmail_status failed: {e}")),
            }
        }

        _ => tool_error(id, format!("Unknown tool: {name}")),
    }
}

// ── MCP response helpers ─────────────────────────────────────────────────────

/// Wrap a successful tool result in the MCP `tools/call` response envelope.
///
/// MCP expects `content` to be an array of content blocks. We serialize the
/// gateway result as a single `text` block so the agent can read it directly.
fn tool_success(id: Value, result: Value) -> Value {
    let text = serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string());
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "content": [
                {"type": "text", "text": text}
            ],
            "isError": false
        }
    })
}

/// Wrap an error message in the MCP `tools/call` error response envelope.
///
/// Using `isError: true` (rather than a JSON-RPC error object) tells the agent
/// that the tool ran but failed gracefully — it can decide how to proceed.
fn tool_error(id: Value, message: impl Into<String>) -> Value {
    let msg = message.into();
    eprintln!("[gmail-mcp] tool error: {msg}");
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "content": [
                {"type": "text", "text": msg}
            ],
            "isError": true
        }
    })
}
