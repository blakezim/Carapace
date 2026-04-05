# Services Reference

Every process running on the Mac, what it does, and how to manage it.

## LaunchDaemons

All in `/Library/LaunchDaemons/`. Start at boot, no login required.

| Service | Label | User | KeepAlive |
|---------|-------|------|-----------|
| Socket setup | `ai.carapace.setup` | root | No (runs once) |
| Gateway | `ai.carapace.gateway` | carapace | Yes |
| Gmail proxy (primary) | `ai.carapace.gmail-proxy` | root | Yes |
| Gmail proxy (automations) | `ai.carapace.gmail-proxy-automations` | carapace | Yes |
| GDocs proxy (hq) | `ai.carapace.gdocs-proxy-hq` | carapace | Yes |
| GDocs proxy (automations) | `ai.carapace.gdocs-proxy-automations` | carapace | Yes |

### Restart Commands

```bash
sudo launchctl kickstart -k system/ai.carapace.gateway
sudo launchctl kickstart -k system/ai.carapace.gmail-proxy
sudo launchctl kickstart -k system/ai.carapace.gmail-proxy-automations
sudo launchctl kickstart -k system/ai.carapace.gdocs-proxy-hq
sudo launchctl kickstart -k system/ai.carapace.gdocs-proxy-automations
```

## Claude Code Agents

Not managed by launchd. Each runs in a separate terminal window.

| Agent | Directory | Config Dir | Telegram Bot | Gmail Account | GDocs Account |
|-------|-----------|------------|-------------|---------------|---------------|
| Wedding | `~/agents/wedding/` | `~/.claude-wedding` | `@wedding_zim_bot` | primary (zimmermanhq) | hq (zimmermanhq) |
| Jarvis | `~/agents/jarvis/` | `~/.claude-jarvis` | `@jarvis_zimmerman_bot` | automations (automationsbz) | automations (automationsbz) |

### Start Commands

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

## Legacy (OpenClaw)

These are still running but will be decommissioned:

| Service | Label | User |
|---------|-------|------|
| OpenClaw gateway | `ai.openclaw.gateway` | openclaw |
| OpenClaw node | `ai.openclaw.node` | openclaw |

## Login Items

| Item | User | Purpose |
|------|------|---------|
| Messages.app | carapace | Must be running for iMessage sending |

After a reboot, switch to the carapace user via fast user switching to start the GUI session. Messages.app should auto-open. Switch back to your account afterward.

## Log Locations

| Log | Path |
|-----|------|
| Gateway stdout | `/Users/carapace/.local/share/carapace/daemon.log` |
| Gateway stderr | `/Users/carapace/.local/share/carapace/daemon.err` |
| Audit log | `/Users/carapace/.local/share/carapace/audit.log` |
| Gmail proxy (primary) | `gmail-proxy.log` / `gmail-proxy.err` |
| Gmail proxy (automations) | `gmail-proxy-automations.log` / `gmail-proxy-automations.err` |
| GDocs proxy (hq) | `gdocs-proxy-hq.log` / `gdocs-proxy-hq.err` |
| GDocs proxy (automations) | `gdocs-proxy-automations.log` / `gdocs-proxy-automations.err` |

All proxy logs are in `/Users/carapace/.local/share/carapace/`.

## Key Files

| File | What |
|------|------|
| `/var/run/carapace/gateway.sock` | Gateway socket |
| `/var/run/carapace/gmail-proxy.sock` | Primary Gmail proxy socket |
| `/var/run/carapace/gmail-proxy-automations.sock` | Automations Gmail proxy socket |
| `/var/run/carapace/gdocs-proxy-hq.sock` | Primary GDocs proxy socket |
| `/var/run/carapace/gdocs-proxy-automations.sock` | Automations GDocs proxy socket |
| `/Users/carapace/.config/carapace/config.toml` | Gateway daemon config |
| `/etc/carapace/gmail-proxy.toml` | Primary Gmail proxy config |
| `/etc/carapace/gmail-proxy-automations.toml` | Automations Gmail proxy config |
| `/etc/carapace/gdocs-proxy-hq.toml` | Primary GDocs proxy config |
| `/etc/carapace/gdocs-proxy-automations.toml` | Automations GDocs proxy config |
| `/etc/carapace/secrets.toml` | Primary Gmail OAuth token (0600) |
| `/etc/carapace/secrets-automations.toml` | Automations Gmail OAuth token (0600) |
| `/etc/carapace/secrets-gdocs-hq.toml` | Primary GDocs OAuth token (0600) |
| `/etc/carapace/secrets-gdocs-automations.toml` | Automations GDocs OAuth token (0600) |
| `~/agents/jarvis/CLAUDE.md` | Jarvis agent instructions |
| `~/agents/jarvis/.mcp.json` | Jarvis MCP servers |
| `~/agents/wedding/CLAUDE.md` | Wedding agent instructions |
| `~/agents/wedding/.mcp.json` | Wedding MCP servers |
