//! Request handler – dispatches JSON-RPC methods to their implementations.

use std::process::Command;

use serde_json::{json, Value};
use tracing::{info, warn};

use crate::protocol::{self, JsonRpcRequest, JsonRpcResponse};

/// Handle a validated JSON-RPC request and produce a response.
pub fn handle_request(req: &JsonRpcRequest) -> JsonRpcResponse {
    info!(method = %req.method, id = %req.id, "handling request");

    match req.method.as_str() {
        "ping" => handle_ping(req),
        "echo" => handle_echo(req),
        "whoami" => handle_whoami(req),
        "execute" => handle_execute(req),
        _ => {
            warn!(method = %req.method, "unknown method");
            JsonRpcResponse::error(
                req.id.clone(),
                protocol::METHOD_NOT_FOUND,
                format!("Unknown method: {}", req.method),
            )
        }
    }
}

// ── ping ───────────────────────────────────────────────────────────────────

/// Responds with `{"pong": true}` – used to verify the daemon is alive.
fn handle_ping(req: &JsonRpcRequest) -> JsonRpcResponse {
    JsonRpcResponse::success(req.id.clone(), json!({ "pong": true }))
}

// ── echo ───────────────────────────────────────────────────────────────────

/// Echoes back whatever the client sends in `params.message`.
fn handle_echo(req: &JsonRpcRequest) -> JsonRpcResponse {
    let message = req
        .params
        .get("message")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    JsonRpcResponse::success(
        req.id.clone(),
        json!({ "echo": message }),
    )
}

// ── whoami ─────────────────────────────────────────────────────────────────

/// Returns the Unix user the daemon is running as. This proves isolation:
/// the daemon runs as `carapace`, not as the caller's user.
fn handle_whoami(req: &JsonRpcRequest) -> JsonRpcResponse {
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .unwrap_or_else(|_| {
            // Fall back to `id -un`
            Command::new("id")
                .arg("-un")
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|| "unknown".into())
        });

    let uid = unsafe { libc::getuid() };

    JsonRpcResponse::success(
        req.id.clone(),
        json!({
            "user": user,
            "uid": uid,
        }),
    )
}

// ── execute ────────────────────────────────────────────────────────────────

/// Execute an arbitrary command *as the carapace user*.
///
/// This is the key passthrough that proves cross-user execution.
/// In later phases, this will be replaced by channel adapters that
/// call specific tools (imsg, signal-cli, etc.) with security middleware.
///
/// Params:
///   - `command` (string): The binary to run.
///   - `args` (array of strings, optional): Arguments.
///
/// Returns:
///   - `stdout`, `stderr`, `exit_code`
fn handle_execute(req: &JsonRpcRequest) -> JsonRpcResponse {
    let command = match req.params.get("command").and_then(|v| v.as_str()) {
        Some(cmd) => cmd,
        None => {
            return JsonRpcResponse::error(
                req.id.clone(),
                protocol::INVALID_PARAMS,
                "Missing required param: \"command\"",
            );
        }
    };

    let args: Vec<String> = req
        .params
        .get("args")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    info!(command, ?args, "executing command");

    match Command::new(command).args(&args).output() {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            let exit_code = output.status.code().unwrap_or(-1);

            JsonRpcResponse::success(
                req.id.clone(),
                json!({
                    "stdout": stdout,
                    "stderr": stderr,
                    "exit_code": exit_code,
                }),
            )
        }
        Err(e) => JsonRpcResponse::error(
            req.id.clone(),
            protocol::INTERNAL_ERROR,
            format!("Failed to execute \"{command}\": {e}"),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::JsonRpcRequest;

    fn make_request(method: &str, params: Value) -> JsonRpcRequest {
        JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: json!(1),
            method: method.into(),
            params,
        }
    }

    #[test]
    fn ping_returns_pong() {
        let req = make_request("ping", json!({}));
        let resp = handle_request(&req);
        let result = resp.result.unwrap();
        assert_eq!(result["pong"], true);
    }

    #[test]
    fn echo_returns_message() {
        let req = make_request("echo", json!({"message": "hello world"}));
        let resp = handle_request(&req);
        let result = resp.result.unwrap();
        assert_eq!(result["echo"], "hello world");
    }

    #[test]
    fn whoami_returns_user() {
        let req = make_request("whoami", json!({}));
        let resp = handle_request(&req);
        let result = resp.result.unwrap();
        assert!(result.get("user").is_some());
        assert!(result.get("uid").is_some());
    }

    #[test]
    fn execute_runs_echo() {
        let req = make_request(
            "execute",
            json!({"command": "echo", "args": ["hello"]}),
        );
        let resp = handle_request(&req);
        let result = resp.result.unwrap();
        assert_eq!(result["stdout"].as_str().unwrap().trim(), "hello");
        assert_eq!(result["exit_code"], 0);
    }

    #[test]
    fn execute_missing_command() {
        let req = make_request("execute", json!({}));
        let resp = handle_request(&req);
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, protocol::INVALID_PARAMS);
    }

    #[test]
    fn unknown_method_returns_error() {
        let req = make_request("nonexistent.method", json!({}));
        let resp = handle_request(&req);
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, protocol::METHOD_NOT_FOUND);
    }
}
