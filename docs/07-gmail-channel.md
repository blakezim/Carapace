# Gmail Channel

The Gmail channel provides search, read, and draft creation through a secure OAuth proxy.

## Architecture

```
Agent -> gmail-mcp -> carapace-daemon -> gmail-proxy -> Gmail API
```

Two separate processes:
- **gmail-proxy** — handles OAuth, content scrubbing, query validation
- **carapace-daemon** — handles allowlists, rate limits, audit logging

## Accounts

Each Gmail account gets its own proxy instance:

| Account | Email | Config | Socket |
|---------|-------|--------|--------|
| primary | zimmermanhq@gmail.com | `/etc/carapace/gmail-proxy.toml` | `gmail-proxy.sock` |
| automations | automationsbz@gmail.com | `/etc/carapace/gmail-proxy-automations.toml` | `gmail-proxy-automations.sock` |

## OAuth Scopes

- `gmail.readonly` — read emails, search, list labels
- `gmail.compose` — create drafts (NOT send)

## What Agents Can Do

| Tool | What It Does |
|------|-------------|
| `gmail_search` | Search using Gmail query syntax (from:, to:, subject:, is:unread, etc.) |
| `gmail_read_thread` | Read all messages in a thread by thread_id |
| `gmail_create_draft` | Create a draft (human must manually send it) |
| `gmail_status` | Check proxy health and token status |

## What Agents Cannot Do

- Send emails directly (drafts only)
- Access trash, spam, or drafts folder via search
- See messages labeled AI-BLOCKED
- See OTP codes or auth URLs (scrubbed to [REDACTED])
- Use disallowed search operators

## Content Scrubbing

The proxy scrubs content before returning it to agents:

| What | How |
|------|-----|
| OTP codes | Regex patterns replace 4-6 digit codes with [REDACTED] |
| Auth URLs | URLs containing reset/verify/confirm/login/auth/token are stripped |
| Blocked senders | Messages from matching senders are silently hidden |
| Blocked label | Messages with AI-BLOCKED label are excluded from all results |
| Link stripping | Optional: all URLs can be replaced with [link removed] |

## Setup Steps

### 1. Create Google Cloud Project

- Go to console.cloud.google.com
- Create a project (or use existing)
- Enable the Gmail API
- Create OAuth 2.0 credentials (Desktop app type)
- Add your email as a test user (if app is in testing mode)
- Download the client_secret.json

### 2. Create Config

```bash
sudo tee /etc/carapace/gmail-proxy.toml > /dev/null << 'EOF'
[auth]
client_id = "YOUR_CLIENT_ID"
client_secret = "YOUR_CLIENT_SECRET"
secrets_file = "secrets.toml"

[gmail]
account = "you@gmail.com"

[scrub]
blocked_label = "AI-BLOCKED"
otp_patterns = ['(?i)\b\d{6}\b', '(?i)\b\d{4}\b']
url_strip_patterns = ['(?i)https?://[^\s]*(?:reset|verify|confirm|login|signin|auth|token)[^\s]*']
blocked_sender_patterns = []
strip_links = false
allowed_operators = ["from", "to", "subject", "after", "before", "older_than", "newer_than", "is", "has", "in", "filename", "cc", "bcc"]

[proxy]
socket_path = "/var/run/carapace/gmail-proxy.sock"
search_fetch_concurrency = 4
EOF

sudo chown carapace /etc/carapace/gmail-proxy.toml
```

### 3. Pre-create Secrets File

```bash
sudo touch /etc/carapace/secrets.toml
sudo chown carapace /etc/carapace/secrets.toml
sudo chmod 600 /etc/carapace/secrets.toml
```

### 4. Run OAuth Setup

```bash
sudo -u carapace gmail-proxy setup --config /etc/carapace/gmail-proxy.toml --client-json /path/to/client_secret.json
```

This opens a browser for Google consent. After approval, the refresh token is saved to secrets.toml.

### 5. Create AI-BLOCKED Label

In Gmail (mail.google.com), create a label called `AI-BLOCKED`. Apply it to any messages you want hidden from agents.

### 6. Install LaunchDaemon

Create plist at `/Library/LaunchDaemons/ai.carapace.gmail-proxy.plist` and bootstrap it.

### 7. Add to Daemon Config

Add the account to `/Users/carapace/.config/carapace/config.toml` and restart the gateway.

## Multi-Account

To add a second Gmail account, repeat steps 1-7 with different config/secrets filenames and socket path. The MCP server uses `GMAIL_ACCOUNT` env var to select which account:

```json
{
  "gmail": {
    "command": "/usr/local/bin/gmail-mcp",
    "env": { "GMAIL_ACCOUNT": "automations" }
  }
}
```
