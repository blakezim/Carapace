# Configuration Reference

Complete reference for the Carapace daemon configuration file.

**Location:** `/Users/carapace/.config/carapace/config.toml`

## Full Example

```toml
# Carapace Gateway Configuration
# All paths are relative to the carapace user's home directory unless absolute

# ============================================
# GATEWAY SETTINGS
# ============================================
[gateway]
# Unix socket path for client connections
socket_path = "/var/run/carapace/gateway.sock"

# Logging level: trace, debug, info, warn, error
log_level = "info"

# Request timeout in seconds
request_timeout = 30

# ============================================
# SECURITY SETTINGS
# ============================================
[security]
# Audit log file path
audit_log_path = "/Users/carapace/.local/share/carapace/audit.log"

# Dead letter queue directory
dead_letter_path = "/Users/carapace/.local/share/carapace/dead_letters"

# Enable/disable audit logging
audit_enabled = true

# ============================================
# RATE LIMITING
# ============================================
[security.rate_limit]
# Format: channel = { requests = N, per_seconds = M }
# Allows N requests per M seconds

imsg = { requests = 30, per_seconds = 60 }
signal = { requests = 20, per_seconds = 60 }
discord = { requests = 60, per_seconds = 60 }
gmail = { requests = 10, per_seconds = 60 }

# Global rate limit (applies if channel-specific not set)
default = { requests = 30, per_seconds = 60 }

# ============================================
# CONTENT FILTERING
# ============================================
[security.content_filter]
# Enable content filtering
enabled = true

# Patterns to check (regex)
# action: "block" (reject message) or "warn" (log but allow)
[[security.content_filter.patterns]]
pattern = "(?i)password\\s*[:=]"
action = "block"

[[security.content_filter.patterns]]
pattern = "(?i)api[_-]?key\\s*[:=]"
action = "block"

[[security.content_filter.patterns]]
pattern = "(?i)secret.*token"
action = "block"

[[security.content_filter.patterns]]
pattern = "\\b\\d{3}-\\d{2}-\\d{4}\\b"  # SSN pattern
action = "block"

[[security.content_filter.patterns]]
pattern = "(?i)credit\\s*card"
action = "warn"

# ============================================
# CHANNEL: iMessage
# ============================================
[channels.imsg]
# Enable this channel
enabled = true

# Path to the real imsg binary
real_binary = "/opt/homebrew/bin/imsg"

# Path to Messages database (for health checks)
db_path = "/Users/carapace/Library/Messages/chat.db"

# Outbound filtering (who can the AI message)
[channels.imsg.outbound]
# Mode: "allowlist", "denylist", or "open"
mode = "allowlist"

# Allowlist entries (when mode = "allowlist")
# Supports: exact match, prefix wildcard (*), domain wildcard (*@domain)
allowlist = [
    "+14155551234",           # Exact phone number
    "+1415555*",              # Prefix wildcard
    "email:user@icloud.com",  # Exact email
    "email:*@family.com",     # Domain wildcard
]

# Denylist entries (when mode = "denylist")
denylist = []

# Inbound filtering (whose messages can the AI see)
[channels.imsg.inbound]
mode = "allowlist"
allowlist = [
    "+14155551234",
    "+14155559999",
]
denylist = []

# ============================================
# CHANNEL: Signal
# ============================================
[channels.signal]
enabled = true

# Path to signal-cli
signal_cli_path = "/opt/homebrew/bin/signal-cli"

# Registered account phone number
account = "+14155551234"

[channels.signal.outbound]
mode = "allowlist"
allowlist = [
    "+14155559999",
    "group:BASE64GROUPID",  # Group identifiers
]

[channels.signal.inbound]
mode = "allowlist"
allowlist = [
    "+14155559999",
]

# ============================================
# CHANNEL: Discord
# ============================================
[channels.discord]
enabled = true

# Path to file containing bot token
token_file = "/Users/carapace/.config/carapace/discord_token"

[channels.discord.outbound]
mode = "allowlist"
allowlist = [
    "channel:123456789012345678",  # Channel IDs
    "user:987654321098765432",     # User IDs (for DMs)
]

[channels.discord.inbound]
mode = "allowlist"
allowlist = [
    "channel:123456789012345678",
    "user:987654321098765432",
]

# ============================================
# CHANNEL: Gmail
# ============================================
[channels.gmail]
enabled = true

# Directory containing OAuth credentials
credentials_path = "/Users/carapace/.config/gog"

[channels.gmail.outbound]
mode = "allowlist"
allowlist = [
    "friend@example.com",     # Exact email
    "*@mycompany.com",        # Domain wildcard
    "*@*.mycompany.com",      # Subdomain wildcard
]

[channels.gmail.inbound]
mode = "allowlist"
allowlist = [
    "*@mycompany.com",
]

# ============================================
# ADVANCED SETTINGS
# ============================================
[advanced]
# Maximum concurrent connections
max_connections = 100

# Watch buffer size (messages held before dropping)
watch_buffer_size = 1000

# Config hot-reload check interval (seconds, 0 = disabled)
config_reload_interval = 60
```

---

## Configuration Sections

### `[gateway]`

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `socket_path` | string | `/var/run/carapace/gateway.sock` | Unix socket path |
| `log_level` | string | `info` | Log verbosity |
| `request_timeout` | integer | `30` | Request timeout (seconds) |

### `[security]`

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `audit_log_path` | string | `~/.local/share/carapace/audit.log` | Audit log file |
| `dead_letter_path` | string | `~/.local/share/carapace/dead_letters` | Dead letter directory |
| `audit_enabled` | boolean | `true` | Enable audit logging |

### `[security.rate_limit]`

Per-channel rate limits. Format: `{ requests = N, per_seconds = M }`

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `imsg` | object | `{ requests = 30, per_seconds = 60 }` | iMessage rate limit |
| `signal` | object | `{ requests = 20, per_seconds = 60 }` | Signal rate limit |
| `discord` | object | `{ requests = 60, per_seconds = 60 }` | Discord rate limit |
| `gmail` | object | `{ requests = 10, per_seconds = 60 }` | Gmail rate limit |
| `default` | object | `{ requests = 30, per_seconds = 60 }` | Default for unconfigured channels |

### `[security.content_filter]`

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `enabled` | boolean | `true` | Enable content filtering |
| `patterns` | array | `[]` | List of patterns to check |

Each pattern:
| Key | Type | Description |
|-----|------|-------------|
| `pattern` | string | Regex pattern |
| `action` | string | `block` or `warn` |

### `[channels.<name>]`

Common channel settings:

| Key | Type | Description |
|-----|------|-------------|
| `enabled` | boolean | Enable this channel |

### `[channels.<name>.outbound]` / `[channels.<name>.inbound]`

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `mode` | string | `allowlist` | `allowlist`, `denylist`, or `open` |
| `allowlist` | array | `[]` | Allowed identifiers |
| `denylist` | array | `[]` | Denied identifiers |

---

## Allowlist Pattern Syntax

| Pattern | Matches | Example |
|---------|---------|---------|
| Exact | Exact string | `+14155551234` |
| Prefix wildcard | Strings starting with prefix | `+1415*` |
| Domain wildcard | Emails at domain | `*@company.com` |
| Subdomain wildcard | Emails at domain and subdomains | `*@*.company.com` |

### Examples

```toml
allowlist = [
    # Exact matches
    "+14155551234",
    "email:user@icloud.com",
    "channel:123456789",

    # Prefix wildcards
    "+1415*",           # All +1415 numbers
    "+1*",              # All US numbers

    # Email domain wildcards
    "*@company.com",    # anyone@company.com
    "*@*.company.com",  # anyone@sub.company.com

    # Mixed
    "email:*@family.com",
]
```

---

## Environment Variables

Configuration can be overridden via environment variables:

| Variable | Config Path | Description |
|----------|-------------|-------------|
| `CARAPACE_SOCKET_PATH` | `gateway.socket_path` | Socket path |
| `CARAPACE_LOG_LEVEL` | `gateway.log_level` | Log level |
| `CARAPACE_CONFIG` | N/A | Config file path |

---

## Validation

The daemon validates configuration at startup:

1. **Required fields**: All enabled channels must have required fields
2. **File paths**: Binary paths must exist
3. **Regex patterns**: Content filter patterns must be valid regex
4. **Permissions**: Config file should be readable only by carapace user

### Common Validation Errors

```
Error: Channel 'imsg' enabled but real_binary not found: /opt/homebrew/bin/imsg
Error: Invalid regex pattern in content_filter: unclosed group
Error: Config file permissions too open: expected 600, got 644
```

---

## Hot Reloading

Configuration can be reloaded without restarting:

```bash
# Via protocol
# Send admin.reload_config request

# Via launchctl
sudo -u carapace launchctl kickstart -k gui/$(id -u carapace)/ai.carapace.gateway
```

Reloaded settings:
- Allowlists/denylists
- Rate limits
- Content filter patterns

Not reloaded (require restart):
- Socket path
- Binary paths
- Channel enable/disable
