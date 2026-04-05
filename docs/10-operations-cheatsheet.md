# Operations Cheatsheet

Quick reference for day-to-day operations. Print this out and tape it to the wall.

## After Reboot / Power Loss

```bash
# 1. Switch to carapace user via fast user switching (for Messages.app)
#    Menu bar > user icon > carapace > log in
#    Then switch back to your account

# 2. All LaunchDaemons auto-start. Verify:
ps aux | grep -E "carapace-daemon|gmail-proxy|gdocs-proxy" | grep -v grep

# 3. Verify sockets exist:
ls /var/run/carapace/

# 4. Start Claude Code agents in separate terminal windows:

# Terminal 1 — Wedding Agent
cd ~/agents/wedding && \
  CLAUDE_CONFIG_DIR=~/.claude-wedding \
  TELEGRAM_BOT_TOKEN=8358937707:AAHXKBw-40yqTWSqNxuRj2SeIFMM9lMqSyk \
  claude --channels plugin:telegram@claude-plugins-official

# Terminal 2 — Jarvis
cd ~/agents/jarvis && \
  CLAUDE_CONFIG_DIR=~/.claude-jarvis \
  TELEGRAM_BOT_TOKEN=8713850029:AAF3egNCMvV-jiN7YwXDYKNCeS0UgWo0qH4 \
  claude --channels plugin:telegram@claude-plugins-official
```

## Health Check

```bash
# All daemons running?
ps aux | grep -E "carapace-daemon|gmail-proxy|gdocs-proxy" | grep -v grep

# All sockets present?
ls /var/run/carapace/

# Gmail healthy?
echo '{"jsonrpc":"2.0","id":1,"method":"channel.status","params":{"channel":"gmail","account":"primary"}}' \
  | nc -U /var/run/carapace/gateway.sock

echo '{"jsonrpc":"2.0","id":2,"method":"channel.status","params":{"channel":"gmail","account":"automations"}}' \
  | nc -U /var/run/carapace/gateway.sock

# GDocs healthy?
echo '{"jsonrpc":"2.0","id":3,"method":"channel.status","params":{"channel":"gdocs","account":"hq"}}' \
  | nc -U /var/run/carapace/gateway.sock

echo '{"jsonrpc":"2.0","id":4,"method":"channel.status","params":{"channel":"gdocs","account":"automations"}}' \
  | nc -U /var/run/carapace/gateway.sock
```

## Restart Individual Services

```bash
# Gateway daemon
sudo launchctl kickstart -k system/ai.carapace.gateway

# Gmail proxies
sudo launchctl kickstart -k system/ai.carapace.gmail-proxy
sudo launchctl kickstart -k system/ai.carapace.gmail-proxy-automations

# GDocs proxies
sudo launchctl kickstart -k system/ai.carapace.gdocs-proxy-hq
sudo launchctl kickstart -k system/ai.carapace.gdocs-proxy-automations
```

## Restart Everything

```bash
sudo launchctl kickstart -k system/ai.carapace.gateway
sudo launchctl kickstart -k system/ai.carapace.gmail-proxy
sudo launchctl kickstart -k system/ai.carapace.gmail-proxy-automations
sudo launchctl kickstart -k system/ai.carapace.gdocs-proxy-hq
sudo launchctl kickstart -k system/ai.carapace.gdocs-proxy-automations
# Then restart agents in their terminal windows (Ctrl+C, re-run start command)
```

## Deploy Code Changes

```bash
# Build everything
cargo build --release

# Install binaries
sudo cp target/release/carapace-daemon /usr/local/bin/
sudo cp target/release/gmail-proxy /usr/local/bin/
sudo cp target/release/gmail-mcp /usr/local/bin/
sudo cp target/release/gdocs-proxy /usr/local/bin/
sudo cp target/release/gdocs-mcp /usr/local/bin/

# Restart affected services
sudo launchctl kickstart -k system/ai.carapace.gateway
sudo launchctl kickstart -k system/ai.carapace.gmail-proxy
sudo launchctl kickstart -k system/ai.carapace.gmail-proxy-automations
sudo launchctl kickstart -k system/ai.carapace.gdocs-proxy-hq
sudo launchctl kickstart -k system/ai.carapace.gdocs-proxy-automations

# MCP servers (gmail-mcp, gdocs-mcp) don't need restarts —
# they're spawned fresh per agent session. Just restart the agents.
```

## Check Logs

```bash
# Gateway errors
tail -20 /Users/carapace/.local/share/carapace/daemon.err

# Gmail proxy errors
tail -20 /Users/carapace/.local/share/carapace/gmail-proxy.err
tail -20 /Users/carapace/.local/share/carapace/gmail-proxy-automations.err

# GDocs proxy errors
tail -20 /Users/carapace/.local/share/carapace/gdocs-proxy-hq.err
tail -20 /Users/carapace/.local/share/carapace/gdocs-proxy-automations.err

# Audit log (all requests)
tail -20 /Users/carapace/.local/share/carapace/audit.log
```

## Quick Manual Tests

```bash
# Gmail search
echo '{"jsonrpc":"2.0","id":1,"method":"channel.search","params":{"channel":"gmail","account":"primary","query":"in:inbox","max":3}}' \
  | nc -U /var/run/carapace/gateway.sock

# GDocs search
echo '{"jsonrpc":"2.0","id":1,"method":"channel.search","params":{"channel":"gdocs","account":"hq","query":"name contains '\''test'\''"}}' \
  | nc -U /var/run/carapace/gateway.sock

# Confirm Gmail send is blocked
echo '{"jsonrpc":"2.0","id":1,"method":"channel.send","params":{"channel":"gmail","recipient":"test@example.com","message":"test"}}' \
  | nc -U /var/run/carapace/gateway.sock
# Expected: error -32601 (method not found)
```
