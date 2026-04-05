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
| OpenClaw gateway | `ai.openclaw.gateway` | openclaw | `openclaw gateway` | Yes |
| OpenClaw node | `ai.openclaw.node` | openclaw | `openclaw node` | Yes |

### ai.carapace.setup

Creates `/var/run/carapace/` with correct ownership (`carapace:carapace-clients`, mode 750) on every boot. Runs once at boot and exits — not a long-running service. Must run before the gateway daemon starts so the socket directory exists.

**Restart:** Not needed — only runs at boot.

### ai.carapace.gateway

The core Carapace security gateway. Listens on `/var/run/carapace/gateway.sock`. Routes all iMessage and Gmail requests through allowlists, rate limiting, content filtering, and audit logging. All MCP servers (gmail-mcp, future imsg-mcp) connect here.

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

| Agent | Directory | Config Dir | Channel | Gmail Account |
|-------|-----------|------------|---------|---------------|
| Wedding Agent | `~/agents/wedding/` | `~/.claude-wedding` | Telegram (`@wedding_bot`) | primary (zimmermanhq) |
| Jarvis | `~/agents/jarvis/` | `~/.claude-jarvis` | Telegram (`@jarvis_bot`) | automations (automationsbz) |

### Starting an agent

```bash
# Wedding Agent
cd ~/agents/wedding && CLAUDE_CONFIG_DIR=~/.claude-wedding claude --channels plugin:telegram@claude-plugins-official

# Jarvis
cd ~/agents/jarvis && CLAUDE_CONFIG_DIR=~/.claude-jarvis claude --channels plugin:telegram@claude-plugins-official
```

### Why CLAUDE_CONFIG_DIR?

Each agent needs its own Telegram bot token. The Telegram plugin stores tokens globally in `~/.claude/channels/telegram/.env`. Using separate config dirs gives each agent its own token storage so they don't conflict.

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

# 3. Start Claude Code agents in terminal tabs (see above)
```

---

## Quick Health Check

```bash
# Are daemons running?
ps aux | grep -E "carapace-daemon|gmail-proxy|openclaw" | grep -v grep

# Are sockets there?
ls /var/run/carapace/

# Is primary Gmail working?
echo '{"jsonrpc":"2.0","id":1,"method":"channel.status","params":{"channel":"gmail","account":"primary"}}' \
  | nc -U /var/run/carapace/gateway.sock

# Is automations Gmail working?
echo '{"jsonrpc":"2.0","id":2,"method":"channel.status","params":{"channel":"gmail","account":"automations"}}' \
  | nc -U /var/run/carapace/gateway.sock
```

---

## Key Files

| File | What |
|------|------|
| `/var/run/carapace/gateway.sock` | Gateway daemon socket |
| `/var/run/carapace/gmail-proxy.sock` | Primary Gmail proxy socket |
| `/var/run/carapace/gmail-proxy-automations.sock` | Automations Gmail proxy socket |
| `/Users/carapace/.config/carapace/config.toml` | Daemon config (accounts, allowlists, rate limits) |
| `/etc/carapace/gmail-proxy.toml` | Primary Gmail proxy config |
| `/etc/carapace/gmail-proxy-automations.toml` | Automations Gmail proxy config |
| `/etc/carapace/secrets.toml` | Primary Gmail OAuth token |
| `/etc/carapace/secrets-automations.toml` | Automations Gmail OAuth token |
| `/Users/carapace/.local/share/carapace/audit.log` | Audit log (all requests) |
| `~/agents/jarvis/CLAUDE.md` | Jarvis agent instructions |
| `~/agents/jarvis/.mcp.json` | Jarvis MCP servers |
| `~/agents/wedding/CLAUDE.md` | Wedding agent instructions |
| `~/agents/wedding/.mcp.json` | Wedding MCP servers |
