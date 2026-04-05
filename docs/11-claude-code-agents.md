# Claude Code Agents

How to set up and run Claude Code as an always-on personal assistant with Telegram integration.

## Agent Architecture

Each agent is an independent Claude Code instance with:
- Its own working directory (`~/agents/<name>/`)
- Its own config directory (`~/.claude-<name>/`)
- Its own Telegram bot
- Its own MCP server configuration
- Its own permission settings

## Current Agents

### Jarvis (Personal Assistant)

**Purpose:** General-purpose assistant — email triage, Kubernetes management, GitHub, code work.

**Directory:** `~/agents/jarvis/`
**Config:** `~/.claude-jarvis/`
**Telegram:** `@jarvis_zimmerman_bot`
**Gmail:** automations (automationsbz@gmail.com)
**GDocs:** automations (automationsbz@gmail.com)

### Wedding Agent

**Purpose:** Wedding planning — vendor coordination, timeline management, document review.

**Directory:** `~/agents/wedding/`
**Config:** `~/.claude-wedding/`
**Telegram:** `@wedding_zim_bot`
**Gmail:** primary (zimmermanhq@gmail.com)
**GDocs:** hq (zimmermanhq@gmail.com)

## Setting Up a New Agent

### 1. Create the Directory

```bash
mkdir -p ~/agents/myagent
```

### 2. Write CLAUDE.md

This is the agent's system prompt — its personality, standing instructions, and context.

```markdown
# My Agent

You are a personal assistant...

## Standing Instructions
- Check email daily for...
- Prioritize messages from...
```

### 3. Configure MCP Servers (.mcp.json)

```json
{
  "mcpServers": {
    "github": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-github"],
      "env": {
        "GITHUB_PERSONAL_ACCESS_TOKEN": "${GITHUB_TOKEN}"
      }
    },
    "gmail": {
      "command": "/usr/local/bin/gmail-mcp",
      "env": {
        "GMAIL_ACCOUNT": "automations"
      }
    },
    "gdocs": {
      "command": "/usr/local/bin/gdocs-mcp",
      "env": {
        "GDOCS_ACCOUNT": "automations"
      }
    },
    "kubernetes": {
      "command": "npx",
      "args": ["-y", "mcp-server-kubernetes"],
      "env": {
        "KUBECONFIG": "${HOME}/.kube/config"
      }
    }
  }
}
```

### 4. Set Permissions (.claude/settings.json)

```bash
mkdir -p ~/agents/myagent/.claude
```

```json
{
  "permissions": {
    "allow": [
      "Read", "Edit", "Write", "Glob", "Grep",
      "WebFetch", "WebSearch", "Bash", "Agent",
      "mcp__github__*",
      "mcp__gmail__*",
      "mcp__gdocs__*",
      "mcp__kubernetes__*",
      "mcp__telegram__*",
      "Channels__telegram__*"
    ]
  }
}
```

### 5. Set Up Telegram Bot

1. Message `@BotFather` on Telegram
2. `/newbot` — follow prompts to create the bot
3. Save the bot token
4. Message the new bot from your Telegram account (this registers your chat_id)

### 6. Configure Telegram Access

```bash
mkdir -p ~/.claude-myagent/channels/telegram/approved
echo "TELEGRAM_BOT_TOKEN=<your-token>" > ~/.claude-myagent/channels/telegram/.env
echo '{"dmPolicy":"pairing","allowFrom":["<your-telegram-user-id>"],"groups":{},"pending":{}}' > ~/.claude-myagent/channels/telegram/access.json
```

To find your Telegram user ID, check the output from `getUpdates` after messaging the bot:
```bash
curl -s "https://api.telegram.org/bot<token>/getUpdates" | python3 -m json.tool
```

### 7. Start the Agent

```bash
cd ~/agents/myagent && \
  CLAUDE_CONFIG_DIR=~/.claude-myagent \
  TELEGRAM_BOT_TOKEN=<your-token> \
  claude --channels plugin:telegram@claude-plugins-official
```

## Multi-Agent: Why Both Env Vars?

**`CLAUDE_CONFIG_DIR`** isolates each agent's auth tokens, session history, and settings. Without it, all agents would share the same `~/.claude/` directory and conflict.

**`TELEGRAM_BOT_TOKEN`** must be passed as an environment variable because the Telegram plugin always loads from `~/.claude/plugins/` regardless of `CLAUDE_CONFIG_DIR`. The plugin reads `process.env.TELEGRAM_BOT_TOKEN` at startup, so the token must be in the environment.

The `.env` file in the config dir is **not sufficient** on its own — it's a backup/reference but the plugin doesn't read from the `CLAUDE_CONFIG_DIR` path.

## GITHUB_TOKEN

Store it in macOS Keychain for security:

```bash
security add-generic-password -s "github-token" -a "$USER" -w "ghp_your_token_here"
```

Add to `~/.zshrc`:
```bash
export GITHUB_TOKEN=$(security find-generic-password -s "github-token" -w 2>/dev/null)
```

Create the token at github.com/settings/tokens with scopes:
- `repo` (full repository access)
- `read:org` (read org membership)
- Do NOT enable `write:issues` if you're concerned about prompt injection via @mentions
