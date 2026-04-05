# Services Reference

Everything running on the Mac, what it does, and how to manage it.

---

## LaunchDaemons (start at boot, no login required)

All located in `/Library/LaunchDaemons/`.

| Service | Label | User | Binary | KeepAlive |
|---------|-------|------|--------|-----------|
| Socket setup | `ai.carapace.setup` | root | `/bin/sh` | No (runs once) |
| Gateway daemon | `ai.carapace.gateway` | carapace | `/usr/local/bin/carapace-daemon` | Yes |
| Gmail proxy (primary) | `ai.carapace.gmail-proxy` | root | `/usr/local/bin/gmail-proxy` | Yes |
| Gmail proxy (automations) | `ai.carapace.gmail-proxy-automations` | carapace | `/usr/local/bin/gmail-proxy` | Yes |
| GDocs proxy (hq) | `ai.carapace.gdocs-proxy-hq` | carapace | `/usr/local/bin/gdocs-proxy` | Yes |
| GDocs proxy (automations) | `ai.carapace.gdocs-proxy-automations` | carapace | `/usr/local/bin/gdocs-proxy` | Yes |
| OpenClaw gateway | `ai.openclaw.gateway` | openclaw | `openclaw gateway` | Yes |
| OpenClaw node | `ai.openclaw.node` | openclaw | `openclaw node` | Yes |

### ai.carapace.setup

Creates `/var/run/carapace/` with correct ownership (`carapace:carapace-clients`, mode 750) on every boot. Runs once at boot and exits — not a long-running service. Must run before the gateway daemon starts so the socket directory exists.

**Restart:** Not needed — only runs at boot.

### ai.carapace.gateway

The core Carapace security gateway. Listens on `/var/run/carapace/gateway.sock`. Routes all iMessage, Gmail, and Google Docs requests through allowlists, rate limiting, content filtering, and audit logging. All MCP servers (gmail-mcp, gdocs-mcp, future imsg-mcp) connect here.

**Config:** `/Users/carapace/.config/carapace/config.toml`
**Logs:** `/Users/carapace/.local/share/carapace/daemon.log` (stdout), `daemon.err` (stderr)
**Audit:** `/Users/carapace/.local/share/carapace/audit.log`

**Restart:**
```bash
sudo launchctl kickstart -k system/ai.carapace.gateway
```

**When to restart:** After changing the daemon config, deploying a new `carapace-daemon` binary, or if agents can't connect to the gateway socket.

### ai.carapace.gmail-proxy

Gmail OAuth proxy for the **primary** account (`zimmermanhq@gmail.com`). Handles token refresh, content scrubbing (OTP redaction, auth URL stripping, AI-BLOCKED filtering), and exposes an HTTP API over Unix socket.

**Config:** `/etc/carapace/gmail-proxy.toml`
**Secrets:** `/etc/carapace/secrets.toml` (OAuth refresh token, mode 0600)
**Socket:** `/var/run/carapace/gmail-proxy.sock`
**Logs:** `/Users/carapace/.local/share/carapace/gmail-proxy.log`, `gmail-proxy.err`

**Restart:**
```bash
sudo launchctl kickstart -k system/ai.carapace.gmail-proxy
```

**When to restart:** After updating the `gmail-proxy` binary, changing the config, or if Gmail tools return "proxy not reachable."

### ai.carapace.gmail-proxy-automations

Gmail OAuth proxy for the **automations** account (`automationsbz@gmail.com`). Same as above but separate config, secrets, and socket.

**Config:** `/etc/carapace/gmail-proxy-automations.toml`
**Secrets:** `/etc/carapace/secrets-automations.toml`
**Socket:** `/var/run/carapace/gmail-proxy-automations.sock`
**Logs:** `/Users/carapace/.local/share/carapace/gmail-proxy-automations.log`, `gmail-proxy-automations.err`

**Restart:**
```bash
sudo launchctl kickstart -k system/ai.carapace.gmail-proxy-automations
```

### ai.carapace.gdocs-proxy-hq

Google Docs/Drive OAuth proxy for the **primary** account (`zimmermanhq@gmail.com`). Handles token refresh, structured document reading, doc creation, file copying, and folder management. Exposes an HTTP API over Unix socket.

**Config:** `/etc/carapace/gdocs-proxy-hq.toml`
**Secrets:** `/etc/carapace/secrets-gdocs-hq.toml` (OAuth refresh token, mode 0600)
**Socket:** `/var/run/carapace/gdocs-proxy-hq.sock`
**Logs:** `/Users/carapace/.local/share/carapace/gdocs-proxy-hq.log`, `gdocs-proxy-hq.err`

**Restart:**
```bash
sudo launchctl kickstart -k system/ai.carapace.gdocs-proxy-hq
```

**When to restart:** After updating the `gdocs-proxy` binary, changing the config, or if GDocs tools return "proxy not reachable."

### ai.carapace.gdocs-proxy-automations

Google Docs/Drive OAuth proxy for the **automations** account (`automationsbz@gmail.com`). Same as above but separate config, secrets, and socket.

**Config:** `/etc/carapace/gdocs-proxy-automations.toml`
**Secrets:** `/etc/carapace/secrets-gdocs-automations.toml`
**Socket:** `/var/run/carapace/gdocs-proxy-automations.sock`
**Logs:** `/Users/carapace/.local/share/carapace/gdocs-proxy-automations.log`, `gdocs-proxy-automations.err`

**Restart:**
```bash
sudo launchctl kickstart -k system/ai.carapace.gdocs-proxy-automations
```

### ai.openclaw.gateway

The OpenClaw AI agent gateway. Runs the WebSocket server at `127.0.0.1:18789`. Spawns agent nodes to handle conversations.

**User:** openclaw
**Logs:** `/Users/openclaw/.local/share/openclaw/gateway.log`, `gateway.err`

**Restart:**
```bash
sudo launchctl kickstart -k system/ai.openclaw.gateway
```

**Note:** This will be decommissioned after the Claude Code migration is verified.

### ai.openclaw.node

The OpenClaw agent node process. Handles iMessage conversations via the imsg RPC shim.

**User:** openclaw
**Logs:** `/Users/openclaw/.local/share/openclaw/node.log`, `node.err`

**Restart:**
```bash
sudo launchctl kickstart -k system/ai.openclaw.node
```

**Note:** This will be decommissioned after the Claude Code migration is verified.

---

## Claude Code Agents (manual, terminal tabs)

Not managed by launchd. Each agent runs in a separate terminal tab. Kill it by closing the tab or pressing `Ctrl+C`.

| Agent | Directory | Config Dir | Channel | Gmail Account | GDocs Account |
|-------|-----------|------------|---------|---------------|---------------|
| Wedding Agent | `~/agents/wedding/` | `~/.claude-wedding` | Telegram (`@wedding_zim_bot`) | primary (zimmermanhq) | hq (zimmermanhq) |
| Jarvis | `~/agents/jarvis/` | `~/.claude-jarvis` | Telegram (`@jarvis_zimmerman_bot`) | automations (automationsbz) | automations (automationsbz) |

### Starting an agent

**Important:** You must pass `TELEGRAM_BOT_TOKEN` as an environment variable. The Telegram plugin reads from the env, not from `CLAUDE_CONFIG_DIR`. Without this, the plugin uses the token from `~/.claude/` and messages won't route correctly.

```bash
# Wedding Agent
cd ~/agents/wedding && \
  CLAUDE_CONFIG_DIR=~/.claude-wedding \
  TELEGRAM_BOT_TOKEN=8358937707:AAHXKBw-40yqTWSqNxuRj2SeIFMM9lMqSyk \
  claude --channels plugin:telegram@claude-plugins-official

# Jarvis
cd ~/agents/jarvis && \
  CLAUDE_CONFIG_DIR=~/.claude-jarvis \
  TELEGRAM_BOT_TOKEN=8713850029:AAF3egNCMvV-jiN7YwXDYKNCeS0UgWo0qH4 \
  claude --channels plugin:telegram@claude-plugins-official
```

### Why CLAUDE_CONFIG_DIR and TELEGRAM_BOT_TOKEN?

Each agent needs its own Telegram bot token. `CLAUDE_CONFIG_DIR` isolates each agent's auth, history, and settings. However, the Telegram plugin always loads from `~/.claude/plugins/` regardless of `CLAUDE_CONFIG_DIR`, so the bot token must be passed explicitly via `TELEGRAM_BOT_TOKEN` in the environment.

---

## Login Items (require carapace GUI session)

These are not LaunchDaemons — they start when the `carapace` user logs in via fast user switching.

| Item | Purpose |
|------|---------|
| Messages.app | Must be running for iMessage sending to work. The `imsg-send` wrapper injects into this session via `launchctl asuser`. |

**After a reboot:** Switch to the carapace user via the menu bar to start the GUI session. Messages.app should auto-open (it's a Login Item). Switch back to your account afterward.

---

## Restart Everything (after reboot or major issue)

```bash
# 1. Log into carapace via fast user switching (for Messages.app)
# 2. Switch back to your account, then:

sudo launchctl kickstart -k system/ai.carapace.gateway
sudo launchctl kickstart -k system/ai.carapace.gmail-proxy
sudo launchctl kickstart -k system/ai.carapace.gmail-proxy-automations
sudo launchctl kickstart -k system/ai.carapace.gdocs-proxy-hq
sudo launchctl kickstart -k system/ai.carapace.gdocs-proxy-automations

# 3. Start Claude Code agents in separate terminal windows (see above)
```

---

## Quick Health Check

```bash
# Are daemons running?
ps aux | grep -E "carapace-daemon|gmail-proxy|gdocs-proxy|openclaw" | grep -v grep

# Are sockets there?
ls /var/run/carapace/

# Is primary Gmail working?
echo '{"jsonrpc":"2.0","id":1,"method":"channel.status","params":{"channel":"gmail","account":"primary"}}' \
  | nc -U /var/run/carapace/gateway.sock

# Is automations Gmail working?
echo '{"jsonrpc":"2.0","id":2,"method":"channel.status","params":{"channel":"gmail","account":"automations"}}' \
  | nc -U /var/run/carapace/gateway.sock

# Is hq GDocs working?
echo '{"jsonrpc":"2.0","id":3,"method":"channel.status","params":{"channel":"gdocs","account":"hq"}}' \
  | nc -U /var/run/carapace/gateway.sock

# Is automations GDocs working?
echo '{"jsonrpc":"2.0","id":4,"method":"channel.status","params":{"channel":"gdocs","account":"automations"}}' \
  | nc -U /var/run/carapace/gateway.sock
```

---

## Key Files

| File | What |
|------|------|
| `/var/run/carapace/gateway.sock` | Gateway daemon socket |
| `/var/run/carapace/gmail-proxy.sock` | Primary Gmail proxy socket |
| `/var/run/carapace/gmail-proxy-automations.sock` | Automations Gmail proxy socket |
| `/var/run/carapace/gdocs-proxy-hq.sock` | Primary GDocs proxy socket |
| `/var/run/carapace/gdocs-proxy-automations.sock` | Automations GDocs proxy socket |
| `/Users/carapace/.config/carapace/config.toml` | Daemon config (accounts, allowlists, rate limits) |
| `/etc/carapace/gmail-proxy.toml` | Primary Gmail proxy config |
| `/etc/carapace/gmail-proxy-automations.toml` | Automations Gmail proxy config |
| `/etc/carapace/gdocs-proxy-hq.toml` | Primary GDocs proxy config |
| `/etc/carapace/gdocs-proxy-automations.toml` | Automations GDocs proxy config |
| `/etc/carapace/secrets.toml` | Primary Gmail OAuth token |
| `/etc/carapace/secrets-automations.toml` | Automations Gmail OAuth token |
| `/etc/carapace/secrets-gdocs-hq.toml` | Primary GDocs OAuth token |
| `/etc/carapace/secrets-gdocs-automations.toml` | Automations GDocs OAuth token |
| `/Users/carapace/.local/share/carapace/audit.log` | Audit log (all requests) |
| `~/agents/jarvis/CLAUDE.md` | Jarvis agent instructions |
| `~/agents/jarvis/.mcp.json` | Jarvis MCP servers (github, gmail, kubernetes, gdocs) |
| `~/agents/wedding/CLAUDE.md` | Wedding agent instructions |
| `~/agents/wedding/.mcp.json` | Wedding MCP servers (github, gmail, gdocs) |
