# Gmail Channel

The Gmail channel gives AI agents read access to a Gmail inbox and the ability to create draft emails. Sending and deleting are intentionally not supported.

---

## Architecture

```
Gmail API (Google)
    │  HTTPS / OAuth 2.0
    ▼
gmail-proxy daemon
(/var/run/carapace/gmail-proxy.sock  — HTTP/1.1 over Unix socket)
    │
    │  Content scrubbing:
    │    - OTP codes redacted ([REDACTED])
    │    - Auth/reset URLs stripped
    │    - Messages with AI-BLOCKED label hidden
    │    - Query operators validated (no trash/spam access)
    │
    ▼
carapace-daemon  (GmailAdapter — HTTP client over Unix socket)
(/var/run/carapace/gateway.sock  — JSON-RPC 2.0 over Unix socket)
    │
    │  Security middleware:
    │    - Rate limiting
    │    - Content filter
    │    - Audit log
    │    - Inbound allowlist (sender filtering)
    │
    ▼
AI agent (OpenClaw / any JSON-RPC client)
```

### Two daemons, not one

The Gmail channel uses two separate processes:

| Process | Binary | Runs As | Socket |
|---------|--------|---------|--------|
| `gmail-proxy` | `/usr/local/bin/gmail-proxy` | carapace | `/var/run/carapace/gmail-proxy.sock` |
| `carapace-daemon` | `/usr/local/bin/carapace-daemon` | carapace | `/var/run/carapace/gateway.sock` |

`gmail-proxy` holds the OAuth credentials and talks to Google. `carapace-daemon` talks to `gmail-proxy` and adds the allowlist, rate limiting, audit log, and content filter on top. AI agents only ever see the gateway socket — they have no path to the OAuth tokens.

---

## What Agents Can Do

| Method | Description |
|--------|-------------|
| `channel.search` | Search emails using Gmail query syntax |
| `channel.get_history` | Fetch all messages in a thread (by thread ID) |
| `channel.create_draft` | Create a draft email (not sent automatically) |
| `channel.watch` | Stream new unread emails as they arrive |
| `channel.status` | Check if the proxy is reachable and token is valid |
| `channel.send` | **Blocked** — returns error -32601 |

---

## JSON-RPC Examples

```bash
# Search inbox
echo '{"jsonrpc":"2.0","id":1,"method":"channel.search","params":{"channel":"gmail","query":"in:inbox","max":10}}' \
  | sudo -u carapace nc -U /var/run/carapace/gateway.sock

# Get a thread (use thread_id from search results)
echo '{"jsonrpc":"2.0","id":2,"method":"channel.get_history","params":{"channel":"gmail","chat_id":"<thread_id>"}}' \
  | sudo -u carapace nc -U /var/run/carapace/gateway.sock

# Create a draft
echo '{"jsonrpc":"2.0","id":3,"method":"channel.create_draft","params":{"channel":"gmail","to":"someone@example.com","subject":"Hello","body":"Draft body here."}}' \
  | sudo -u carapace nc -U /var/run/carapace/gateway.sock

# Check status
echo '{"jsonrpc":"2.0","id":4,"method":"channel.status","params":{"channel":"gmail"}}' \
  | sudo -u carapace nc -U /var/run/carapace/gateway.sock

# Confirm send is blocked
echo '{"jsonrpc":"2.0","id":5,"method":"channel.send","params":{"channel":"gmail","recipient":"anyone@example.com","message":"test"}}' \
  | sudo -u carapace nc -U /var/run/carapace/gateway.sock
# → error -32601: "Gmail channel does not support direct send. Use channel.create_draft instead."
```

---

## Content Scrubbing

The `gmail-proxy` scrubs email content before it reaches the agent:

| What | How |
|------|-----|
| OTP / verification codes | Replaced with `[REDACTED]` |
| Auth/reset/verify URLs | Stripped from body |
| Messages labelled `AI-BLOCKED` | Hidden from all responses |
| Dangerous query operators | `in:trash`, `in:spam`, `label:`, `in:anywhere` are blocked at the query parser level |

To hide a specific email or thread from the agent permanently, apply the `AI-BLOCKED` label in Gmail. The proxy will never return it.

---

## Configuration Files

**`/etc/carapace/gmail-proxy.toml`** — gmail-proxy config:
```toml
[auth]
client_id     = "..."
client_secret = "..."
secrets_file  = "secrets.toml"   # relative to this file; must be chmod 0600

[gmail]
account = "zimmermanhq@gmail.com"

[scrub]
blocked_label = "AI-BLOCKED"
otp_patterns  = ['(?i)\b\d{6}\b', '(?i)\b\d{4}\b']
url_strip_patterns = ['(?i)https?://[^\s]*(?:reset|verify|confirm|login|signin|auth|token)[^\s]*']
strip_links   = false

[proxy]
socket_path = "/var/run/carapace/gmail-proxy.sock"
```

**`/Users/carapace/.config/carapace/config.toml`** — carapace-daemon config (Gmail section):
```toml
[channels.gmail]
enabled      = true
proxy_socket = "/var/run/carapace/gmail-proxy.sock"

[channels.gmail.inbound]
mode = "open"   # or "allowlist" with allowlist = ["sender@example.com"]
```

---

## OAuth Tokens

The OAuth refresh token is stored in `/etc/carapace/secrets.toml` (chmod 0600, owned by carapace).

Tokens are refreshed automatically by `gmail-proxy` before they expire. You should not need to re-run the OAuth setup unless:
- You revoke access in your Google account settings
- The secrets file is deleted or corrupted
- You rotate the OAuth client credentials

To re-run OAuth setup:
```bash
sudo -u carapace gmail-proxy setup --config /etc/carapace/gmail-proxy.toml --client-json ~/client_secret.json
```

---

## launchd Service

The `gmail-proxy` runs as a launchd daemon. The plist is at `ai.carapace.gmail-proxy.plist` in the project root and should be installed at `/Library/LaunchDaemons/ai.carapace.gmail-proxy.plist`.

```bash
# Install the plist (one-time)
sudo cp ai.carapace.gmail-proxy.plist /Library/LaunchDaemons/
sudo launchctl bootstrap system /Library/LaunchDaemons/ai.carapace.gmail-proxy.plist

# Start / restart
sudo launchctl kickstart -k system/ai.carapace.gmail-proxy

# Stop
sudo launchctl bootout system/ai.carapace.gmail-proxy

# Check status
sudo launchctl print system/ai.carapace.gmail-proxy
```

Logs:
- stdout: `/Users/carapace/.local/share/carapace/gmail-proxy.log`
- stderr: `/Users/carapace/.local/share/carapace/gmail-proxy.err`
