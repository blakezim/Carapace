# Pitfalls and Gotchas

Things that didn't work, workarounds discovered, and mistakes to avoid. Hard-won knowledge from building and deploying Carapace.

## Telegram Plugin + CLAUDE_CONFIG_DIR

**Problem:** The Telegram plugin always loads from `~/.claude/plugins/`, not from the `CLAUDE_CONFIG_DIR` path. This means the plugin reads the bot token from the global location, not the per-agent config.

**Symptom:** Only one agent receives Telegram messages, or messages route to the wrong agent.

**Fix:** Pass `TELEGRAM_BOT_TOKEN` as an environment variable when starting each agent:
```bash
CLAUDE_CONFIG_DIR=~/.claude-wedding TELEGRAM_BOT_TOKEN=<wedding-bot-token> claude --channels ...
```

**Root cause:** The plugin subprocess is spawned from the shared plugin cache, not from the agent's config dir.

## Telegram Messages Not Coming Through

**Symptom:** Agent shows "Listening for channel messages" but nothing arrives when you message the bot.

**Possible causes:**
1. Stuck updates in Telegram's queue. Flush them:
   ```bash
   curl -s "https://api.telegram.org/bot<token>/getUpdates?offset=-1"
   ```
2. Another process is polling the same bot token. Only one consumer can poll at a time.
3. `TELEGRAM_BOT_TOKEN` env var not set (plugin falls back to wrong token).
4. Bot token is in `.env` but not in the environment (see above).

## OAuth Secrets File Permission Denied

**Problem:** `gdocs-proxy setup` or `gmail-proxy setup` fails with "Permission denied" writing the secrets file.

**Cause:** The `carapace` user can't create new files in `/etc/carapace/` (owned by root). The setup process writes the refresh token to the secrets file path specified in config.

**Fix:** Pre-create the file with correct ownership before running setup:
```bash
sudo touch /etc/carapace/secrets-gdocs-hq.toml
sudo chown carapace /etc/carapace/secrets-gdocs-hq.toml
sudo chmod 600 /etc/carapace/secrets-gdocs-hq.toml
```

## Google API "Not Enabled" Errors

**Problem:** Proxy starts fine, health check shows token valid, but searches return 500 errors.

**Symptom:** `Drive search: API error 403 Forbidden: Google Drive API has not been used in project...`

**Fix:** Enable the required APIs in the Google Cloud Console for that project. Each Google account may use a different Cloud project. For gdocs-proxy, you need:
- Google Drive API
- Google Docs API
- Google Sheets API
- Google Slides API
- Google Forms API

**Gotcha:** Different Google accounts (zimmermanhq vs automationsbz) use different Cloud projects with different OAuth credentials.

## OAuth Token Doesn't Get Sheets/Slides/Forms Scopes

**Problem:** Drive search works but reading Sheets/Slides/Forms returns permission errors.

**Cause:** The OAuth token was issued before the new scopes were added to the proxy code.

**Fix:** Re-run OAuth setup to get a fresh token with all scopes:
```bash
sudo -u carapace gdocs-proxy setup --config /etc/carapace/gdocs-proxy-hq.toml
```

## gdocs_read Returns "Missing required param: chat_id"

**Problem:** The gdocs_read MCP tool routes through `channel.get_history`, which expects `chat_id`.

**Cause:** The MCP shim was passing `document_id` instead of `chat_id`.

**Fix:** This was fixed in commit `b76767d`. If you see this error, update the `gdocs-mcp` binary.

## gdocs_create Returns "Missing required param: to"

**Problem:** Creating a Google Doc fails with a Gmail-specific error.

**Cause:** `channel.create_draft` validated Gmail params (to, subject) before checking the channel type.

**Fix:** This was fixed by resolving the channel first, then validating channel-specific params.

## `launchctl load` Returns "Input/output error"

**Problem:** `sudo launchctl load /Library/LaunchDaemons/...` fails.

**Fix:** Use `launchctl bootstrap` instead (load is deprecated):
```bash
sudo launchctl bootstrap system /Library/LaunchDaemons/ai.carapace.gdocs-proxy-hq.plist
```

If it says "Bootstrap failed: 5" — the service is already loaded. Use `kickstart -k` to restart it.

## iMessage Sending Is Complicated

Sending iMessages from a daemon requires a multi-step workaround:

1. The daemon calls `sudo /usr/local/carapace/imsg-send` (NOPASSWD sudoers rule)
2. That runs as root
3. Root calls `launchctl asuser 502 osascript ...` to inject into carapace's GUI session
4. osascript sends Apple Events to Messages.app

**Why:** macOS isolates GUI sessions from system daemons. Processes in the system session (audit session 0) cannot reach GUI session services. The `launchctl asuser` trick bridges this gap.

**Requirement:** The carapace user must have an active GUI session (log in via fast user switching at least once after boot).

## Content Scrubbing Hides Too Much

**Problem:** The default OTP patterns (`\b\d{6}\b`, `\b\d{4}\b`) can match legitimate numbers in emails (prices, zip codes, etc.).

**Fix:** Customize the patterns in the gmail-proxy config. Make them more specific:
```toml
otp_patterns = [
    '(?i)(?:code|otp|verification|pin)\s*[:=]?\s*\d{4,6}',
]
```

## Bun/Node Not on PATH

**Problem:** MCP servers that use `npx` (GitHub, Kubernetes) fail because Node.js or Bun isn't installed or not on PATH.

**Fix:**
```bash
brew install node
curl -fsSL https://bun.sh/install | bash
```

Add to `~/.zshrc`:
```bash
export BUN_INSTALL="$HOME/.bun"
export PATH="$BUN_INSTALL/bin:$PATH"
```

## Two Gmail Accounts Overwriting Each Other's Secrets

**Problem:** Running OAuth setup for the second account overwrites the first account's `secrets.toml`.

**Cause:** Both configs pointed to the same secrets file name.

**Fix:** Use unique secrets file names:
- Primary: `secrets_file = "secrets.toml"`
- Automations: `secrets_file = "secrets-automations.toml"`

## Client Secret JSON for Different Google Accounts

Each Google account that uses Gmail or GDocs needs its own Google Cloud project with OAuth credentials. The `client_secret.json` from one account won't work for another. When adding a new account:

1. Log into Cloud Console as that Google account
2. Create a new project
3. Enable the required APIs
4. Create OAuth credentials
5. Add the account as a test user
6. Download client_secret.json
