# Multi-Account Gmail Support

**Goal:** Allow different agents to access different Gmail accounts through the Carapace security layer. Each account gets its own OAuth credentials, content scrubbing rules, and audit trail.

**Constraint:** Do not break existing OpenClaw or single-account Gmail functionality. All changes must be backward-compatible with the current config format.

---

## Architecture After Change

```
Agent: Jarvis                          Agent: Wedding
  │                                      │
  │ gmail_search(account: "primary")     │ gmail_search(account: "wedding")
  ▼                                      ▼
gmail-mcp                              gmail-mcp
  │                                      │
  │ channel.search                       │ channel.search
  │   channel: "gmail"                   │   channel: "gmail"
  │   account: "primary"                 │   account: "wedding"
  ▼                                      ▼
carapace-daemon  (/var/run/carapace/gateway.sock)
  │
  │ routes by account name
  │
  ├─ "primary" → GmailAdapter(gmail-proxy-primary.sock)
  │                    │
  │                    ▼
  │              gmail-proxy (primary)
  │              /var/run/carapace/gmail-proxy-primary.sock
  │              OAuth: zimmermanhq@gmail.com
  │
  └─ "wedding" → GmailAdapter(gmail-proxy-wedding.sock)
                       │
                       ▼
                 gmail-proxy (wedding)
                 /var/run/carapace/gmail-proxy-wedding.sock
                 OAuth: wedding@gmail.com
```

---

## What Changes

### 1. Daemon Config (`config.rs`)

**Current config:**
```toml
[channels.gmail]
enabled = true
proxy_socket = "/var/run/carapace/gmail-proxy.sock"

[channels.gmail.inbound]
mode = "open"
```

**New config (backward-compatible):**
```toml
[channels.gmail]
enabled = true

# Default account — used when no account is specified in the request.
# This preserves backward compatibility: existing gmail-mcp instances
# that don't pass an account parameter hit this account.
default_account = "primary"

[channels.gmail.accounts.primary]
proxy_socket = "/var/run/carapace/gmail-proxy-primary.sock"

[channels.gmail.accounts.primary.inbound]
mode = "open"

[channels.gmail.accounts.wedding]
proxy_socket = "/var/run/carapace/gmail-proxy-wedding.sock"

[channels.gmail.accounts.wedding.inbound]
mode = "open"
```

**Backward compatibility:** If the old `proxy_socket` field is present (no `accounts` table), treat it as a single account named "default" and behave exactly as today. The new format is only activated when `accounts` is present.

**Rust changes in `config.rs`:**

```rust
#[derive(Debug, Deserialize)]
pub struct GmailChannelConfig {
    pub enabled: bool,

    // Legacy single-account field (backward compat)
    pub proxy_socket: Option<PathBuf>,

    // New multi-account field
    pub default_account: Option<String>,
    pub accounts: Option<HashMap<String, GmailAccountConfig>>,

    // Legacy inbound config (backward compat)
    pub inbound: Option<DirectionConfig>,
}

#[derive(Debug, Deserialize)]
pub struct GmailAccountConfig {
    pub proxy_socket: PathBuf,
    pub inbound: Option<DirectionConfig>,
}

impl GmailChannelConfig {
    /// Resolve to a map of account_name → GmailAccountConfig.
    /// Handles both legacy (single proxy_socket) and new (accounts map) formats.
    pub fn resolve_accounts(&self) -> HashMap<String, GmailAccountConfig> {
        if let Some(accounts) = &self.accounts {
            accounts.clone()
        } else if let Some(socket) = &self.proxy_socket {
            // Legacy format: single account named "default"
            let mut map = HashMap::new();
            map.insert("default".to_string(), GmailAccountConfig {
                proxy_socket: socket.clone(),
                inbound: self.inbound.clone(),
            });
            map
        } else {
            HashMap::new()
        }
    }

    pub fn default_account_name(&self) -> &str {
        self.default_account.as_deref().unwrap_or("default")
    }
}
```

**Estimated: ~40 lines changed in `config.rs`**

---

### 2. Server Initialization (`server.rs`)

**Current:** Creates one `GmailAdapter` and stores it in `AppState`.

**New:** Creates a `HashMap<String, GmailAdapter>` — one per account.

```rust
// Current
pub struct AppState {
    // ...
    pub gmail_adapter: Option<GmailAdapter>,
    pub gmail_inbound_allowlist: Option<Allowlist>,
    // ...
}

// New
pub struct AppState {
    // ...
    pub gmail_adapters: HashMap<String, GmailAdapter>,
    pub gmail_inbound_allowlists: HashMap<String, Allowlist>,
    pub gmail_default_account: String,
    // ...
}
```

**Initialization loop:**
```rust
let mut gmail_adapters = HashMap::new();
let mut gmail_inbound_allowlists = HashMap::new();

if let Some(gmail_config) = &config.channels.as_ref().and_then(|c| c.gmail.as_ref()) {
    if gmail_config.enabled {
        for (name, account) in gmail_config.resolve_accounts() {
            let adapter = GmailAdapter::new(account.proxy_socket.clone());
            gmail_adapters.insert(name.clone(), adapter);

            if let Some(inbound) = &account.inbound {
                if let Some(list) = &inbound.allowlist {
                    gmail_inbound_allowlists.insert(name.clone(), Allowlist::new(list.clone()));
                }
            }
        }
    }
}
```

**Estimated: ~30 lines changed in `server.rs`**

---

### 3. Channel Handler (`channel_handler.rs`)

**Current:** All `channel: "gmail"` requests route to the single adapter.

**New:** Requests include an optional `account` parameter. If omitted, use the default account.

```rust
// Extract account name from params, fall back to default
fn resolve_gmail_account(params: &Value, state: &AppState) -> Result<String, JsonRpcResponse> {
    let account = params.get("account")
        .and_then(|v| v.as_str())
        .unwrap_or(&state.gmail_default_account);

    if state.gmail_adapters.contains_key(account) {
        Ok(account.to_string())
    } else {
        Err(JsonRpcResponse::error(
            id,
            -32004,
            &format!("Gmail account '{}' not configured", account),
        ))
    }
}
```

Then in each handler method, replace:
```rust
// Old
let adapter = state.gmail_adapter.as_ref().ok_or(...)?;

// New
let account = resolve_gmail_account(&params, &state)?;
let adapter = state.gmail_adapters.get(&account).unwrap();
```

**Estimated: ~20 lines changed in `channel_handler.rs`**

---

### 4. Gmail MCP Server (`gmail_mcp.rs`)

**Current:** Tool calls pass `channel: "gmail"` with no account parameter.

**New:** Accept an optional `GMAIL_ACCOUNT` env var. If set, include `account` in every RPC call. Also add an `account` parameter to each tool so the agent can override per-call.

```rust
// At startup
let default_account: Option<String> = std::env::var("GMAIL_ACCOUNT").ok();

// In each tool handler, when building the RPC params:
let mut params = json!({
    "channel": "gmail",
    "query": query,
    "max": max_results,
});
// Add account if configured or if agent specified one
let account = tool_params.get("account")
    .and_then(|v| v.as_str())
    .map(String::from)
    .or(default_account.clone());
if let Some(acct) = account {
    params["account"] = json!(acct);
}
```

**Tool definitions get an optional `account` parameter:**
```rust
json!({
    "name": "gmail_search",
    "description": "Search Gmail using Gmail query syntax. ...",
    "inputSchema": {
        "type": "object",
        "properties": {
            "query": { "type": "string", "description": "Gmail search query" },
            "max_results": { "type": "integer", "description": "Max results (default 10)" },
            "account": { "type": "string", "description": "Gmail account name (optional, uses default if omitted)" }
        },
        "required": ["query"]
    }
})
```

**Estimated: ~30 lines changed in `gmail_mcp.rs`**

---

### 5. Per-Account Gmail Proxy Instances

Each account needs its own:
- Config file: `/etc/carapace/gmail-proxy-<name>.toml`
- Secrets file: `/etc/carapace/secrets-<name>.toml`
- Socket: `/var/run/carapace/gmail-proxy-<name>.sock`
- LaunchDaemon plist

**Example: `/etc/carapace/gmail-proxy-wedding.toml`**
```toml
[auth]
client_id     = "..."
client_secret = "..."
secrets_file  = "secrets-wedding.toml"

[gmail]
account = "wedding@gmail.com"

[scrub]
blocked_label      = "AI-BLOCKED"
otp_patterns       = ['(?i)\b\d{6}\b']
url_strip_patterns = ['(?i)https?://[^\s]*(?:reset|verify|confirm|login|signin|auth|token)[^\s]*']
strip_links        = false

[proxy]
socket_path = "/var/run/carapace/gmail-proxy-wedding.sock"
```

**LaunchDaemon: `/Library/LaunchDaemons/ai.carapace.gmail-proxy-wedding.plist`**
```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>ai.carapace.gmail-proxy-wedding</string>
    <key>ProgramArguments</key>
    <array>
        <string>/usr/local/bin/gmail-proxy</string>
        <string>serve</string>
        <string>--config</string>
        <string>/etc/carapace/gmail-proxy-wedding.toml</string>
    </array>
    <key>UserName</key>
    <string>carapace</string>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardOutPath</key>
    <string>/Users/carapace/.local/share/carapace/gmail-proxy-wedding.log</string>
    <key>StandardErrorPath</key>
    <string>/Users/carapace/.local/share/carapace/gmail-proxy-wedding.err</string>
</dict>
</plist>
```

**OAuth setup per account:**
```bash
sudo -u carapace gmail-proxy setup \
    --config /etc/carapace/gmail-proxy-wedding.toml \
    --client-json ~/client_secret.json
```

---

### 6. Agent MCP Config

Each agent's `.mcp.json` specifies which account to use via env var:

**Jarvis (primary account):**
```json
{
  "mcpServers": {
    "gmail": {
      "command": "/usr/local/bin/gmail-mcp",
      "env": { "GMAIL_ACCOUNT": "primary" }
    }
  }
}
```

**Wedding agent (wedding account):**
```json
{
  "mcpServers": {
    "gmail": {
      "command": "/usr/local/bin/gmail-mcp",
      "env": { "GMAIL_ACCOUNT": "wedding" }
    }
  }
}
```

If `GMAIL_ACCOUNT` is not set, the daemon uses `default_account` from its config. This means existing setups (OpenClaw, current Claude Code) continue to work without changes.

---

## Build Order

```
Step 1: Config changes          ← config.rs: add GmailAccountConfig, resolve_accounts()
Step 2: Server init             ← server.rs: HashMap<String, GmailAdapter>
Step 3: Channel handler         ← channel_handler.rs: resolve_gmail_account()
Step 4: Gmail MCP               ← gmail_mcp.rs: GMAIL_ACCOUNT env var, account param
Step 5: Tests                   ← update existing tests, add multi-account tests
Step 6: Second gmail-proxy      ← config, OAuth setup, launchd plist
Step 7: Agent configs           ← .mcp.json per agent with GMAIL_ACCOUNT env
```

**Estimated code changes:** ~120 lines across 4 files
**Estimated total time:** One focused afternoon

---

## Backward Compatibility Checklist

- [ ] Old config format (single `proxy_socket`) still works — daemon treats it as one account named "default"
- [ ] Old gmail-mcp (no `GMAIL_ACCOUNT` env var) still works — daemon uses `default_account`
- [ ] OpenClaw continues to work without changes — its gmail-mcp doesn't pass an account parameter, so it hits the default
- [ ] Existing gmail-proxy instance unchanged — just add new instances alongside it
- [ ] Rate limiting works per-account (each account is a separate channel key)
- [ ] Audit log includes account name for traceability

---

## Risk Assessment

| Risk | Mitigation |
|------|-----------|
| Breaking existing single-account setup | Backward-compat config parsing; old format → "default" account |
| Breaking OpenClaw | OpenClaw's gmail-mcp doesn't change; daemon routes no-account requests to default |
| OAuth token confusion between accounts | Each account has its own secrets file and proxy instance |
| Daemon crash on bad config | Config validation at load time; fail fast with clear error |

**Recommendation:** Branch from `main` before starting. Tag the current working state as `v0.1-stable` so you can always roll back.
