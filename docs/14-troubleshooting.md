# Troubleshooting

## Quick Diagnostics

Run these first when something isn't working:

```bash
# 1. Are all daemons running?
ps aux | grep -E "carapace-daemon|gmail-proxy|gdocs-proxy" | grep -v grep

# 2. Are all sockets present?
ls -la /var/run/carapace/

# 3. Check health through the gateway
echo '{"jsonrpc":"2.0","id":1,"method":"channel.status","params":{"channel":"gmail","account":"primary"}}' | nc -U /var/run/carapace/gateway.sock
echo '{"jsonrpc":"2.0","id":2,"method":"channel.status","params":{"channel":"gdocs","account":"hq"}}' | nc -U /var/run/carapace/gateway.sock

# 4. Check recent errors
tail -10 /Users/carapace/.local/share/carapace/daemon.err
tail -10 /Users/carapace/.local/share/carapace/gmail-proxy.err
tail -10 /Users/carapace/.local/share/carapace/gdocs-proxy-hq.err
```

## Connection Issues

### "Connection refused" or "No such file" on gateway socket

**Cause:** Gateway daemon not running or socket directory doesn't exist.

```bash
# Check if running
ps aux | grep carapace-daemon | grep -v grep

# If not running, check why
sudo launchctl list | grep carapace

# Restart
sudo launchctl kickstart -k system/ai.carapace.gateway

# If socket dir missing (after reboot, setup didn't run)
sudo mkdir -p /var/run/carapace
sudo chown carapace:carapace-clients /var/run/carapace
sudo chmod 750 /var/run/carapace
```

### "Permission denied" connecting to socket

**Cause:** Your user isn't in the `carapace-clients` group.

```bash
# Check group membership
id blakezimmerman | grep carapace-clients

# Add to group
sudo dseditgroup -o edit -a blakezimmerman -t user carapace-clients

# You may need to log out and back in for group changes to take effect
```

### Proxy shows "socket not found"

**Cause:** The proxy daemon isn't running.

```bash
# Check which proxy is down
ls -la /var/run/carapace/*.sock

# Restart the missing one
sudo launchctl kickstart -k system/ai.carapace.gmail-proxy  # or gdocs-proxy-hq, etc.
```

## Gmail Issues

### "proxy not reachable" in status check

Restart the gmail-proxy:
```bash
sudo launchctl kickstart -k system/ai.carapace.gmail-proxy
```

### "token_valid: false"

The OAuth token has expired and refresh failed. Check the proxy error log:
```bash
tail -20 /Users/carapace/.local/share/carapace/gmail-proxy.err
```

If the refresh token is revoked, re-run OAuth setup:
```bash
sudo -u carapace gmail-proxy setup --config /etc/carapace/gmail-proxy.toml
```

### Search returns no results but emails exist

Check if the query uses blocked operators:
- `is:draft`, `in:trash`, `in:spam`, `in:anywhere` are blocked
- `label:AI-BLOCKED` is blocked
- Only whitelisted operators are allowed (see config)

### OTP codes you need are being scrubbed

Temporarily adjust `otp_patterns` in the gmail-proxy config, or use the AI-BLOCKED label on sensitive emails instead.

## Google Docs Issues

### Search works but read returns 500

Check which file type you're trying to read. `gdocs_read` only supports:
- Google Docs (`application/vnd.google-apps.document`)
- Google Sheets (`application/vnd.google-apps.spreadsheet`)
- Google Slides (`application/vnd.google-apps.presentation`)
- Google Forms (`application/vnd.google-apps.form`)

PDFs, images, and other files will return an error. Use `gdocs_file_info` to check.

### "API has not been used in project"

Enable the required API in Google Cloud Console. See [Pitfalls](12-pitfalls-and-gotchas.md#google-api-not-enabled-errors).

## Agent Issues

### Agent shows "Listening" but Telegram messages don't arrive

See [Pitfalls: Telegram Plugin](12-pitfalls-and-gotchas.md#telegram-plugin--claude_config_dir).

Key checklist:
1. Is `TELEGRAM_BOT_TOKEN` set in the environment (not just `.env` file)?
2. Is there another process polling the same bot token?
3. Are there stuck updates? Flush: `curl -s "https://api.telegram.org/bot<token>/getUpdates?offset=-1"`

### MCP server fails to connect

The MCP server connects to the gateway on first tool call. If the gateway isn't running:
```bash
sudo launchctl kickstart -k system/ai.carapace.gateway
```

Then retry the tool call — the MCP server will reconnect.

### Permission prompts over Telegram

Add the tool to the agent's `.claude/settings.json` allow list:
```json
"mcp__gmail__*",
"mcp__gdocs__*"
```

## Performance

### Slow Gmail searches

Reduce `search_fetch_concurrency` in gmail-proxy config if you're hitting Google's rate limits. Or increase it (default: 4) if searches are slow.

### Gateway not responding

Check if the rate limiter is blocking requests:
```bash
tail -20 /Users/carapace/.local/share/carapace/audit.log | grep "rate"
```

Adjust limits in the daemon config.
