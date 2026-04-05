# Carapace Operations

This file is a quick pointer. Full documentation is in the `docs/` directory.

## After Reboot / Power Loss

See [Operations Cheatsheet](docs/10-operations-cheatsheet.md) for the full recovery procedure.

Quick version:
1. Switch to carapace user (fast user switching) to start Messages.app, then switch back
2. Verify daemons: `ps aux | grep -E "carapace-daemon|gmail-proxy|gdocs-proxy" | grep -v grep`
3. Start agents in separate terminal windows (see commands below)

## Start Agents

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

## Deploy Code Changes

```bash
cargo build --release
sudo cp target/release/carapace-daemon /usr/local/bin/
sudo cp target/release/gmail-proxy /usr/local/bin/
sudo cp target/release/gmail-mcp /usr/local/bin/
sudo cp target/release/gdocs-proxy /usr/local/bin/
sudo cp target/release/gdocs-mcp /usr/local/bin/
sudo launchctl kickstart -k system/ai.carapace.gateway
sudo launchctl kickstart -k system/ai.carapace.gmail-proxy
sudo launchctl kickstart -k system/ai.carapace.gmail-proxy-automations
sudo launchctl kickstart -k system/ai.carapace.gdocs-proxy-hq
sudo launchctl kickstart -k system/ai.carapace.gdocs-proxy-automations
# Restart agents (Ctrl+C, re-run start command)
```

## Documentation Index

| Doc | What |
|-----|------|
| [Overview](docs/01-overview.md) | What Carapace is and why it exists |
| [Architecture](docs/02-architecture.md) | System design, components, data flow |
| [Security Model](docs/03-security-model.md) | Threat model, attack analysis, security layers |
| [Setup Guide](docs/04-setup-guide.md) | Complete setup from scratch |
| [Configuration Reference](docs/05-configuration-reference.md) | All TOML config options |
| [Protocol Spec](docs/06-protocol-spec.md) | JSON-RPC 2.0 protocol reference |
| [Gmail Channel](docs/07-gmail-channel.md) | Gmail proxy setup, scrubbing, tools |
| [GDocs Channel](docs/08-gdocs-channel.md) | Google Docs/Drive/Sheets/Slides/Forms proxy |
| [Services Reference](docs/09-services-reference.md) | All running services, restart commands |
| [Operations Cheatsheet](docs/10-operations-cheatsheet.md) | Power loss recovery, deploy, health checks |
| [Claude Code Agents](docs/11-claude-code-agents.md) | Agent setup, Telegram, permissions |
| [Pitfalls and Gotchas](docs/12-pitfalls-and-gotchas.md) | What didn't work, workarounds |
| [Developer Guide](docs/13-developer-guide.md) | Building, testing, extending Carapace |
| [Troubleshooting](docs/14-troubleshooting.md) | Diagnostics and fixes |
| [Archive](docs/archive/) | Old docs (OpenClaw migration plan, roadmap, etc.) |
