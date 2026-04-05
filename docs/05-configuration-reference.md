# Configuration Reference

The gateway daemon reads its config from `/Users/carapace/.config/carapace/config.toml`.

## Full Example

```toml
[gateway]
socket_path = "/var/run/carapace/gateway.sock"
log_level = "info"
request_timeout = 30

[security]
audit_log_path = "/Users/carapace/.local/share/carapace/audit.log"
dead_letter_path = "/Users/carapace/.local/share/carapace/dead_letters"
audit_enabled = true

[security.rate_limit]
default = { requests = 30, per_seconds = 60 }
imsg    = { requests = 10, per_seconds = 30 }

[security.content_filter]
enabled = true

[[security.content_filter.patterns]]
pattern = '(?i)password\s*[:=]'
action  = "block"

[[security.content_filter.patterns]]
pattern = '\b\d{3}-\d{2}-\d{4}\b'
action  = "warn"

# ── iMessage channel ─────────────────────────────────────────────

[channels.imsg]
enabled = true
real_binary = "/Users/carapace/.local/bin/imsg"
db_path = "/Users/carapace/Library/Messages/chat.db"

[channels.imsg.outbound]
mode = "allowlist"
allowlist = ["+19705551234", "friend@icloud.com"]

[channels.imsg.inbound]
mode = "open"

# ── Gmail channel ────────────────────────────────────────────────

[channels.gmail]
enabled = true
default_account = "primary"

[channels.gmail.accounts.primary]
proxy_socket = "/var/run/carapace/gmail-proxy.sock"

[channels.gmail.accounts.primary.inbound]
mode = "open"

[channels.gmail.accounts.automations]
proxy_socket = "/var/run/carapace/gmail-proxy-automations.sock"

[channels.gmail.accounts.automations.inbound]
mode = "open"

# ── Google Docs channel ──────────────────────────────────────────

[channels.gdocs]
enabled = true
default_account = "hq"

[channels.gdocs.accounts.hq]
proxy_socket = "/var/run/carapace/gdocs-proxy-hq.sock"

[channels.gdocs.accounts.automations]
proxy_socket = "/var/run/carapace/gdocs-proxy-automations.sock"
```

## Section Reference

### [gateway]

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `socket_path` | string | `/var/run/carapace/gateway.sock` | Unix socket path |
| `log_level` | string | `"info"` | Log level (trace, debug, info, warn, error) |
| `request_timeout` | integer | `30` | Request timeout in seconds |

### [security]

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `audit_log_path` | string | `/Users/carapace/.local/share/carapace/audit.log` | Audit log file |
| `dead_letter_path` | string | `/Users/carapace/.local/share/carapace/dead_letters` | Blocked message storage |
| `audit_enabled` | bool | `true` | Enable/disable audit logging |

### [security.rate_limit]

Map of channel name (or `"default"`) to rate limit rule:

```toml
[security.rate_limit]
default = { requests = 30, per_seconds = 60 }
imsg    = { requests = 10, per_seconds = 30 }
gmail   = { requests = 20, per_seconds = 60 }
```

### [security.content_filter]

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `enabled` | bool | `true` | Enable/disable content filtering |
| `patterns` | array | `[]` | List of `{ pattern, action }` rules |

Actions: `"block"` (reject + dead letter), `"warn"` (allow + flag in audit).

### [channels.imsg]

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `enabled` | bool | `true` | Enable iMessage channel |
| `real_binary` | string | `/Users/carapace/.local/bin/imsg` | Path to real imsg binary |
| `db_path` | string | `/Users/carapace/Library/Messages/chat.db` | iMessage database path |

### [channels.imsg.outbound] / [channels.imsg.inbound]

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `mode` | string | `"allowlist"` | `"allowlist"`, `"denylist"`, or `"open"` |
| `allowlist` | array | `[]` | List of phone numbers or iCloud emails |

### [channels.gmail]

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `enabled` | bool | `true` | Enable Gmail channel |
| `default_account` | string | `"default"` | Account to use when none specified |

Each account under `[channels.gmail.accounts.<name>]`:

| Key | Type | Description |
|-----|------|-------------|
| `proxy_socket` | string | Unix socket path for this account's gmail-proxy |

### [channels.gdocs]

| Key | Type | Default | Description |
|-----|------|---------|-------------|
| `enabled` | bool | `true` | Enable Google Docs channel |
| `default_account` | string | `"default"` | Account to use when none specified |

Each account under `[channels.gdocs.accounts.<name>]`:

| Key | Type | Description |
|-----|------|-------------|
| `proxy_socket` | string | Unix socket path for this account's gdocs-proxy |

## Proxy Configuration Files

Each proxy has its own config in `/etc/carapace/`:

### gmail-proxy config

```toml
[auth]
client_id = "YOUR_CLIENT_ID"
client_secret = "YOUR_CLIENT_SECRET"
secrets_file = "secrets.toml"    # relative to config dir

[gmail]
account = "you@gmail.com"

[scrub]
blocked_label = "AI-BLOCKED"
strip_links = false
otp_patterns = ['(?i)\b\d{6}\b', '(?i)\b\d{4}\b']
url_strip_patterns = ['(?i)https?://[^\s]*(?:reset|verify|confirm|login|signin|auth|token)[^\s]*']
blocked_sender_patterns = []
allowed_operators = ["from", "to", "subject", "after", "before", "older_than", "newer_than", "is", "has", "in", "filename", "cc", "bcc"]

[proxy]
socket_path = "/var/run/carapace/gmail-proxy.sock"
search_fetch_concurrency = 4
```

### gdocs-proxy config

```toml
[auth]
client_id = "YOUR_CLIENT_ID"
client_secret = "YOUR_CLIENT_SECRET"
secrets_file = "secrets-gdocs.toml"

[gdocs]
account = "you@gmail.com"

[scrub]
strip_links = false
redact_patterns = []

[proxy]
socket_path = "/var/run/carapace/gdocs-proxy.sock"
```

## Hot Reloading

The daemon does not currently support hot reloading of its config. Restart it after changes:

```bash
sudo launchctl kickstart -k system/ai.carapace.gateway
```

Proxy configs also require a restart of the respective proxy.
