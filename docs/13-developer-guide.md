# Developer Guide

How to build, test, and extend Carapace.

## Project Structure

```
Carapace/
  Cargo.toml                      # Workspace root
  crates/
    carapace-daemon/              # Gateway daemon (JSON-RPC server)
      src/
        main.rs                   # Entry point, CLI args
        server.rs                 # Unix socket server, AppState, connection handling
        config.rs                 # TOML config parsing and validation
        channel_handler.rs        # channel.* method routing and dispatch
        handler.rs                # Non-channel methods (ping, echo, whoami)
        protocol.rs               # JSON-RPC types and error codes
        middleware.rs              # Security pipeline orchestration
        rate_limiter.rs           # Token bucket rate limiter
        allowlist.rs              # Per-channel allowlist/denylist
        content_filter.rs         # Regex content scanning
        audit.rs                  # Audit log writer
        dead_letter.rs            # Blocked message storage
        adapters/
          mod.rs
          imsg.rs                 # iMessage adapter (calls real imsg binary)
          gmail.rs                # Gmail adapter (HTTP over Unix socket to gmail-proxy)
          gdocs.rs                # GDocs adapter (HTTP over Unix socket to gdocs-proxy)

    carapace-client/              # Client library for connecting to the gateway
      src/lib.rs                  # GatewayClient struct (connect, call, subscribe)

    carapace-shims/               # MCP servers and legacy CLI shims
      src/bin/
        gmail_mcp.rs              # Gmail MCP server (stdio transport)
        gdocs_mcp.rs              # GDocs MCP server (stdio transport)
        imsg_shim.rs              # iMessage CLI shim (legacy, for OpenClaw)
        test_shim.rs              # Test client for the gateway

    gmail-proxy/                  # Gmail OAuth proxy daemon
      src/
        main.rs                   # Entry point (setup / serve subcommands)
        auth.rs                   # OAuth token management and setup flow
        config.rs                 # Proxy-specific config
        gmail/
          client.rs               # Gmail API HTTP client
          types.rs                # Gmail API types, sanitized output
        proxy/
          routes.rs               # Axum HTTP route handlers
        scrub/
          content.rs              # OTP/URL/sender scrubbing
          labels.rs               # AI-BLOCKED label filtering
          query.rs                # Gmail query AST parsing and validation

    gdocs-proxy/                  # Google Docs/Drive OAuth proxy daemon
      src/
        main.rs                   # Entry point (setup / serve subcommands)
        auth.rs                   # OAuth token management and setup flow
        config.rs                 # Proxy-specific config
        docs/
          client.rs               # Drive + Docs + Sheets + Slides + Forms API client
          types.rs                # API types, structured document output
        proxy/
          routes.rs               # Axum HTTP route handlers + format converters
```

## Building

```bash
# Full workspace
cargo build --release

# Individual crate
cargo build --release -p carapace-daemon
cargo build --release -p gmail-proxy
cargo build --release -p gdocs-proxy
cargo build --release -p carapace-shims
```

## Testing

```bash
# Unit tests (all crates)
cargo test

# Just daemon unit tests (skip integration tests that need real iMessage)
cargo test -p carapace-daemon --bin carapace-daemon

# Integration tests (need carapace user + imsg binary)
cargo test -p carapace-daemon --test integration
```

## Adding a New Channel

### 1. Create the Adapter

Add `crates/carapace-daemon/src/adapters/newchannel.rs`:

```rust
pub struct NewChannelAdapter { ... }

impl NewChannelAdapter {
    pub fn new(...) -> Self { ... }
    pub async fn send(&self, ...) -> Result<...> { ... }
    pub async fn health_check(&self) -> HealthStatus { ... }
}
```

Register in `adapters/mod.rs`.

### 2. Add Config

In `config.rs`, add the channel to `ChannelsConfig` and create config structs.

### 3. Wire Into Server

In `server.rs`, add the adapter to `AppState` and initialize it in `AppState::new()`.

### 4. Add to Channel Handler

In `channel_handler.rs`:
- Add variant to `Channel` enum
- Add to `ChannelContext`
- Add to `resolve_channel()`
- Handle in each `handle_*` method

### 5. Build MCP Server (Optional)

If agents need to use the channel, create `crates/carapace-shims/src/bin/newchannel_mcp.rs` following the pattern in `gmail_mcp.rs`.

## Adding a New Proxy

If the channel needs an OAuth proxy (like Gmail or GDocs):

1. Create `crates/newchannel-proxy/` following `gmail-proxy` or `gdocs-proxy` as a template
2. Add to workspace `Cargo.toml`
3. Implement `setup` (OAuth flow) and `serve` (Axum HTTP server on Unix socket) subcommands
4. Create LaunchDaemon plist
5. Create adapter in daemon that calls the proxy over HTTP/Unix socket

## Key Patterns

### HTTP Over Unix Socket (Adapters)

All proxy adapters use raw HTTP/1.1 over Unix sockets. The pattern:

```rust
let stream = UnixStream::connect(&self.socket_path).await?;
let request = format!("GET /path HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
// write request, read response, parse HTTP, extract JSON body
```

### MCP Servers

MCP servers are synchronous stdin/stdout programs:
- Read JSON-RPC from stdin (line-delimited)
- Write JSON-RPC to stdout
- Diagnostic output to stderr only
- Lazy-connect to gateway on first tool call
- Handle: `initialize`, `notifications/initialized`, `tools/list`, `tools/call`

### OAuth Token Management

Both proxies use the same `TokenManager` pattern:
- Cache access token in memory with `RwLock`
- Refresh 5 minutes before expiry
- `get_token()` returns cached or refreshes automatically
