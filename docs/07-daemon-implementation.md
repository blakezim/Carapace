# Daemon Implementation

This document provides technical details for implementing the Carapace daemon.

## Overview

The daemon is a Rust application that:
1. Listens on a Unix domain socket
2. Receives JSON-RPC requests from shims
3. Applies security policies
4. Executes requests via channel adapters
5. Returns responses

## Project Structure

```
carapace-daemon/
├── Cargo.toml
├── src/
│   ├── main.rs              # Entry point
│   ├── config.rs            # Configuration loading
│   ├── server.rs            # Socket server
│   ├── router.rs            # Request routing
│   ├── protocol.rs          # JSON-RPC types
│   ├── middleware/
│   │   ├── mod.rs
│   │   ├── rate_limiter.rs
│   │   ├── allowlist.rs
│   │   ├── content_filter.rs
│   │   └── audit.rs
│   ├── adapters/
│   │   ├── mod.rs
│   │   ├── traits.rs        # ChannelAdapter trait
│   │   ├── imsg.rs
│   │   ├── signal.rs
│   │   ├── discord.rs
│   │   └── gmail.rs
│   └── dead_letter.rs
└── tests/
```

## Core Components

### Entry Point (`main.rs`)

```rust
use tokio::net::UnixListener;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<()> {
    // Parse CLI args
    let args = Args::parse();

    // Load configuration
    let config = Config::load(&args.config)?;

    // Initialize components
    let middleware = SecurityMiddleware::new(&config);
    let adapters = ChannelAdapters::new(&config).await?;
    let dead_letter = DeadLetterQueue::new(&config);

    // Create shared state
    let state = Arc::new(AppState {
        config,
        middleware,
        adapters,
        dead_letter,
    });

    // Create socket directory if needed
    create_socket_dir(&state.config.gateway.socket_path)?;

    // Bind to Unix socket
    let listener = UnixListener::bind(&state.config.gateway.socket_path)?;
    set_socket_permissions(&state.config.gateway.socket_path)?;

    tracing::info!("Carapace daemon listening on {}", state.config.gateway.socket_path);

    // Accept connections
    loop {
        let (stream, _) = listener.accept().await?;
        let state = Arc::clone(&state);

        tokio::spawn(async move {
            if let Err(e) = handle_connection(stream, state).await {
                tracing::error!("Connection error: {}", e);
            }
        });
    }
}
```

### Configuration (`config.rs`)

```rust
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
pub struct Config {
    pub gateway: GatewayConfig,
    pub security: SecurityConfig,
    pub channels: ChannelsConfig,
}

#[derive(Debug, Deserialize)]
pub struct GatewayConfig {
    pub socket_path: PathBuf,
    pub log_level: String,
}

#[derive(Debug, Deserialize)]
pub struct SecurityConfig {
    pub audit_log_path: PathBuf,
    pub dead_letter_path: PathBuf,
    pub rate_limit: RateLimitConfig,
    pub content_filter: ContentFilterConfig,
}

#[derive(Debug, Deserialize)]
pub struct ChannelConfig {
    pub enabled: bool,
    pub outbound: FilterConfig,
    pub inbound: FilterConfig,
}

#[derive(Debug, Deserialize)]
pub struct FilterConfig {
    pub mode: FilterMode,
    pub allowlist: Vec<String>,
    pub denylist: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FilterMode {
    Allowlist,
    Denylist,
    Open,
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: Config = toml::from_str(&content)?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<()> {
        // Validate regex patterns in content filter
        for pattern in &self.security.content_filter.patterns {
            Regex::new(&pattern.pattern)?;
        }
        Ok(())
    }
}
```

### Protocol Types (`protocol.rs`)

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: serde_json::Value,
    pub method: String,
    pub params: serde_json::Value,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

impl JsonRpcResponse {
    pub fn success(id: serde_json::Value, result: serde_json::Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn error(id: serde_json::Value, code: i32, message: &str) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.to_string(),
                data: None,
            }),
        }
    }
}

// Error codes
pub const ERR_ALLOWLIST: i32 = -32001;
pub const ERR_RATE_LIMIT: i32 = -32002;
pub const ERR_CONTENT_BLOCKED: i32 = -32003;
pub const ERR_CHANNEL_UNAVAILABLE: i32 = -32004;
pub const ERR_SEND_FAILED: i32 = -32005;
```

### Request Router (`router.rs`)

```rust
use crate::protocol::*;

pub async fn route_request(
    request: JsonRpcRequest,
    state: &AppState,
) -> JsonRpcResponse {
    match request.method.as_str() {
        "channel.send" => handle_send(request, state).await,
        "channel.list_chats" => handle_list_chats(request, state).await,
        "channel.get_history" => handle_get_history(request, state).await,
        "channel.watch" => handle_watch(request, state).await,
        "channel.status" => handle_status(request, state).await,
        "admin.get_dead_letters" => handle_get_dead_letters(request, state).await,
        "admin.reload_config" => handle_reload_config(request, state).await,
        _ => JsonRpcResponse::error(request.id, -32601, "Method not found"),
    }
}

async fn handle_send(request: JsonRpcRequest, state: &AppState) -> JsonRpcResponse {
    // Parse params
    let params: SendParams = match serde_json::from_value(request.params.clone()) {
        Ok(p) => p,
        Err(e) => return JsonRpcResponse::error(request.id, -32602, &e.to_string()),
    };

    // Get channel adapter
    let adapter = match state.adapters.get(&params.channel) {
        Some(a) => a,
        None => return JsonRpcResponse::error(request.id, ERR_CHANNEL_UNAVAILABLE, "Channel not configured"),
    };

    // Apply security middleware
    if let Err(e) = state.middleware.check_outbound(&params).await {
        // Log to dead letter queue
        state.dead_letter.add(&params, &e).await;

        return match e {
            SecurityError::NotInAllowlist => {
                JsonRpcResponse::error(request.id, ERR_ALLOWLIST, "Recipient not in allowlist")
            }
            SecurityError::RateLimited => {
                JsonRpcResponse::error(request.id, ERR_RATE_LIMIT, "Rate limit exceeded")
            }
            SecurityError::ContentBlocked(pattern) => {
                JsonRpcResponse::error(request.id, ERR_CONTENT_BLOCKED, &format!("Content blocked: {}", pattern))
            }
        };
    }

    // Execute send
    match adapter.send(&params).await {
        Ok(result) => JsonRpcResponse::success(request.id, serde_json::to_value(result).unwrap()),
        Err(e) => JsonRpcResponse::error(request.id, ERR_SEND_FAILED, &e.to_string()),
    }
}
```

### Security Middleware

#### Rate Limiter (`middleware/rate_limiter.rs`)

```rust
use std::collections::HashMap;
use std::sync::RwLock;
use std::time::{Duration, Instant};

pub struct RateLimiter {
    limits: HashMap<String, RateLimit>,
    state: RwLock<HashMap<String, Vec<Instant>>>,
}

#[derive(Debug, Clone)]
pub struct RateLimit {
    pub requests: u32,
    pub per_seconds: u64,
}

impl RateLimiter {
    pub fn new(config: &RateLimitConfig) -> Self {
        Self {
            limits: config.clone(),
            state: RwLock::new(HashMap::new()),
        }
    }

    /// Record an attempt (call BEFORE checking to prevent probing)
    pub fn record_attempt(&self, channel: &str) {
        let mut state = self.state.write().unwrap();
        let timestamps = state.entry(channel.to_string()).or_default();
        timestamps.push(Instant::now());
    }

    /// Check if within rate limit
    pub fn check(&self, channel: &str) -> bool {
        let limit = match self.limits.get(channel) {
            Some(l) => l,
            None => return true, // No limit configured
        };

        let state = self.state.read().unwrap();
        let timestamps = match state.get(channel) {
            Some(t) => t,
            None => return true,
        };

        let window = Duration::from_secs(limit.per_seconds);
        let cutoff = Instant::now() - window;

        let recent_count = timestamps.iter().filter(|&&t| t > cutoff).count();
        recent_count <= limit.requests as usize
    }

    /// Periodic cleanup of old entries
    pub fn cleanup(&self) {
        let mut state = self.state.write().unwrap();
        let cutoff = Instant::now() - Duration::from_secs(3600); // 1 hour

        for timestamps in state.values_mut() {
            timestamps.retain(|&t| t > cutoff);
        }
    }
}
```

#### Allowlist (`middleware/allowlist.rs`)

```rust
use regex::Regex;

pub struct AllowlistValidator {
    config: HashMap<String, ChannelFilterConfig>,
}

impl AllowlistValidator {
    pub fn check_outbound(&self, channel: &str, recipient: &str) -> bool {
        let config = match self.config.get(channel) {
            Some(c) => &c.outbound,
            None => return false, // Channel not configured
        };

        match config.mode {
            FilterMode::Open => true,
            FilterMode::Allowlist => self.matches_any(&config.allowlist, recipient),
            FilterMode::Denylist => !self.matches_any(&config.denylist, recipient),
        }
    }

    pub fn check_inbound(&self, channel: &str, sender: &str) -> bool {
        let config = match self.config.get(channel) {
            Some(c) => &c.inbound,
            None => return false,
        };

        match config.mode {
            FilterMode::Open => true,
            FilterMode::Allowlist => self.matches_any(&config.allowlist, sender),
            FilterMode::Denylist => !self.matches_any(&config.denylist, sender),
        }
    }

    fn matches_any(&self, patterns: &[String], value: &str) -> bool {
        for pattern in patterns {
            if self.matches_pattern(pattern, value) {
                return true;
            }
        }
        false
    }

    fn matches_pattern(&self, pattern: &str, value: &str) -> bool {
        // Exact match
        if pattern == value {
            return true;
        }

        // Wildcard match (simple * at end)
        if pattern.ends_with('*') {
            let prefix = &pattern[..pattern.len() - 1];
            return value.starts_with(prefix);
        }

        // Domain wildcard for emails (*@domain.com)
        if pattern.starts_with("*@") {
            let domain = &pattern[1..]; // "@domain.com"
            return value.ends_with(domain);
        }

        false
    }
}
```

#### Content Filter (`middleware/content_filter.rs`)

```rust
use regex::Regex;
use std::sync::OnceLock;
use std::collections::HashMap;

pub struct ContentFilter {
    patterns: Vec<CompiledPattern>,
}

struct CompiledPattern {
    regex: Regex,
    action: FilterAction,
}

#[derive(Debug, Clone)]
pub enum FilterAction {
    Block,
    Warn,
}

impl ContentFilter {
    pub fn new(config: &ContentFilterConfig) -> Result<Self> {
        let patterns = config
            .patterns
            .iter()
            .map(|p| {
                Ok(CompiledPattern {
                    regex: Regex::new(&p.pattern)?,
                    action: p.action.clone(),
                })
            })
            .collect::<Result<Vec<_>>>()?;

        Ok(Self { patterns })
    }

    pub fn check(&self, content: &str) -> Option<String> {
        for pattern in &self.patterns {
            if pattern.regex.is_match(content) {
                match pattern.action {
                    FilterAction::Block => {
                        return Some(pattern.regex.as_str().to_string());
                    }
                    FilterAction::Warn => {
                        tracing::warn!("Content matched warning pattern: {}", pattern.regex.as_str());
                    }
                }
            }
        }
        None
    }
}
```

#### Audit Logger (`middleware/audit.rs`)

```rust
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use chrono::Utc;
use serde::Serialize;

pub struct AuditLogger {
    path: PathBuf,
}

#[derive(Serialize)]
struct AuditEntry {
    timestamp: String,
    action: String,
    channel: String,
    direction: String,
    target: String,
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    request_id: Option<String>,
}

impl AuditLogger {
    pub fn new(path: &Path) -> Self {
        Self { path: path.to_path_buf() }
    }

    pub async fn log_outbound(&self, channel: &str, recipient: &str, status: &str, reason: Option<&str>) {
        self.log(AuditEntry {
            timestamp: Utc::now().to_rfc3339(),
            action: "send".to_string(),
            channel: channel.to_string(),
            direction: "outbound".to_string(),
            target: recipient.to_string(),
            status: status.to_string(),
            reason: reason.map(String::from),
            request_id: None,
        }).await;
    }

    pub async fn log_inbound(&self, channel: &str, sender: &str, status: &str) {
        self.log(AuditEntry {
            timestamp: Utc::now().to_rfc3339(),
            action: "receive".to_string(),
            channel: channel.to_string(),
            direction: "inbound".to_string(),
            target: sender.to_string(),
            status: status.to_string(),
            reason: None,
            request_id: None,
        }).await;
    }

    async fn log(&self, entry: AuditEntry) {
        let line = serde_json::to_string(&entry).unwrap();

        // Append to log file
        if let Ok(mut file) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            let _ = writeln!(file, "{}", line);
        }
    }
}
```

## Building

### Cargo.toml

```toml
[package]
name = "carapace-daemon"
version = "0.1.0"
edition = "2021"

[dependencies]
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
toml = "0.8"
regex = "1"
chrono = { version = "0.4", features = ["serde"] }
tracing = "0.1"
tracing-subscriber = "0.3"
clap = { version = "4", features = ["derive"] }
thiserror = "1"
sha2 = "0.10"
```

### Build Commands

```bash
# Debug build
cargo build

# Release build
cargo build --release

# Run tests
cargo test

# Run with config
cargo run -- --config /path/to/config.toml
```

## Testing

### Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_allowlist_exact_match() {
        let validator = AllowlistValidator::new(/* config */);
        assert!(validator.check_outbound("imsg", "+1234567890"));
        assert!(!validator.check_outbound("imsg", "+9999999999"));
    }

    #[test]
    fn test_content_filter() {
        let filter = ContentFilter::new(/* config */)?;
        assert!(filter.check("my password is secret123").is_some());
        assert!(filter.check("hello world").is_none());
    }

    #[test]
    fn test_rate_limiter() {
        let limiter = RateLimiter::new(/* config */);
        for _ in 0..30 {
            limiter.record_attempt("imsg");
        }
        assert!(limiter.check("imsg"));
        limiter.record_attempt("imsg");
        assert!(!limiter.check("imsg"));
    }
}
```

### Integration Tests

```rust
#[tokio::test]
async fn test_full_send_flow() {
    // Start daemon
    let daemon = TestDaemon::start().await;

    // Connect as client
    let client = UnixStream::connect(daemon.socket_path()).await?;

    // Send request
    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "channel.send",
        "params": {
            "channel": "imsg",
            "recipient": "+1234567890",
            "message": "Test"
        }
    });

    // Verify response
    let response = client.send_request(request).await?;
    assert!(response.result.is_some());
}
```
