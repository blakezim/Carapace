//! JSON-RPC 2.0 protocol types for the Carapace gateway.
//!
//! The gateway uses newline-delimited JSON over a Unix domain socket.
//! Each message is a single line of JSON terminated by `\n`.

use serde::{Deserialize, Serialize};

// ── Standard JSON-RPC error codes ──────────────────────────────────────────

pub const PARSE_ERROR: i32 = -32700;
pub const INVALID_REQUEST: i32 = -32600;
pub const METHOD_NOT_FOUND: i32 = -32601;
pub const INVALID_PARAMS: i32 = -32602;
pub const INTERNAL_ERROR: i32 = -32603;

// ── Carapace-specific error codes ──────────────────────────────────────────

pub const NOT_IN_ALLOWLIST: i32 = -32001;
pub const RATE_LIMITED: i32 = -32002;
pub const CONTENT_BLOCKED: i32 = -32003;
pub const CHANNEL_UNAVAILABLE: i32 = -32004;
pub const SEND_FAILED: i32 = -32005;

// ── Request ────────────────────────────────────────────────────────────────

/// A JSON-RPC 2.0 request coming from a client (shim).
#[derive(Debug, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: serde_json::Value,
    pub method: String,
    #[serde(default = "default_params")]
    pub params: serde_json::Value,
}

fn default_params() -> serde_json::Value {
    serde_json::Value::Object(serde_json::Map::new())
}

// ── Response ───────────────────────────────────────────────────────────────

/// A JSON-RPC 2.0 response sent back to the client.
#[derive(Debug, Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

/// The error object inside a JSON-RPC error response.
#[derive(Debug, Serialize, Clone)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

// ── Constructors ───────────────────────────────────────────────────────────

impl JsonRpcResponse {
    /// Build a successful response.
    pub fn success(id: serde_json::Value, result: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: Some(result),
            error: None,
        }
    }

    /// Build an error response.
    pub fn error(id: serde_json::Value, code: i32, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.into(),
                data: None,
            }),
        }
    }

    /// Build an error response with extra data.
    pub fn error_with_data(
        id: serde_json::Value,
        code: i32,
        message: impl Into<String>,
        data: serde_json::Value,
    ) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.into(),
                data: Some(data),
            }),
        }
    }
}

// ── Validation ─────────────────────────────────────────────────────────────

/// Errors that can occur when validating a raw JSON-RPC request.
#[derive(Debug, thiserror::Error)]
pub enum ValidationError {
    #[error("missing or invalid \"jsonrpc\" field (must be \"2.0\")")]
    BadVersion,

    #[error("missing \"id\" field")]
    MissingId,

    #[error("missing \"method\" field")]
    MissingMethod,
}

impl JsonRpcRequest {
    /// Validate that the request conforms to JSON-RPC 2.0.
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.jsonrpc != "2.0" {
            return Err(ValidationError::BadVersion);
        }
        if self.id.is_null() {
            return Err(ValidationError::MissingId);
        }
        if self.method.is_empty() {
            return Err(ValidationError::MissingMethod);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_success_response() {
        let resp = JsonRpcResponse::success(
            serde_json::json!(1),
            serde_json::json!({"pong": true}),
        );
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"result\""));
        assert!(!json.contains("\"error\""));
    }

    #[test]
    fn round_trip_error_response() {
        let resp = JsonRpcResponse::error(
            serde_json::json!(1),
            METHOD_NOT_FOUND,
            "Method not found",
        );
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"error\""));
        assert!(!json.contains("\"result\""));
    }

    #[test]
    fn deserialize_request() {
        let raw = r#"{"jsonrpc":"2.0","id":1,"method":"ping","params":{}}"#;
        let req: JsonRpcRequest = serde_json::from_str(raw).unwrap();
        assert_eq!(req.method, "ping");
        assert!(req.validate().is_ok());
    }

    #[test]
    fn missing_params_gets_default() {
        let raw = r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#;
        let req: JsonRpcRequest = serde_json::from_str(raw).unwrap();
        assert!(req.params.is_object());
    }
}
