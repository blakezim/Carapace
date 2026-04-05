# Migration Plan: OpenClaw → Claude Code Agents

**Goal:** Replace OpenClaw with Claude Code as the agent runtime. Use iMessage as the primary mobile interface. Run multiple specialized agents managed from your phone. Enable Kubernetes cluster management, web app deployment, Gmail triage, and general personal assistance — all through text messages.

**What changes:** The agent runtime (OpenClaw → Claude Code). The `openclaw` user is decommissioned.
**What stays:** The Carapace security layer (carapace-daemon, allowlists, rate limiting, content filtering, audit log, gmail-proxy).

---

## Architecture Overview

```
Your iPhone (iMessage)
    │
    ▼
Messages.app (carapace GUI session)
    │
    ▼
chat.db
    │  Claude Code native iMessage channel plugin
    │  (reads chat.db directly for inbound messages)
    ▼
Claude Code session  (running in tmux as blakezimmerman)
    │
    │  Decides what to do based on CLAUDE.md instructions
    │
    ├─ Reply via iMessage?  → imsg-mcp → carapace-daemon → security → Messages.app
    ├─ Check Gmail?         → gmail-mcp → carapace-daemon → gmail-proxy → Gmail API
    ├─ Kubernetes work?     → kubernetes MCP server → kubectl → cluster
    ├─ GitHub work?         → github MCP server → GitHub API
    ├─ Deploy a web app?    → Bash tool → kubectl/helm/docker
    └─ Delegate to another agent? → Agent SDK / subagent
```

### Key Architectural Decisions

**Why native iMessage channel instead of Telegram + bridge:**
The original plan used Telegram as a mobile interface and required a custom bridge daemon to forward iMessages into Telegram. Claude Code now has a native iMessage channel plugin (`plugin:imessage@claude-plugins-official`) that reads the Messages database directly. This eliminates the bridge entirely and lets you text from your iPhone natively.

**Where Carapace security still applies:**
- **Outbound iMessages** — all sends go through `imsg-mcp` → `carapace-daemon` → allowlist + rate limit + content filter + audit log. The agent cannot text anyone not on the allowlist.
- **Gmail** — all access goes through `gmail-mcp` → `carapace-daemon` → `gmail-proxy` → content scrubbing + AI-BLOCKED label filtering.
- **Inbound iMessages** — the native iMessage plugin reads chat.db directly, bypassing the daemon's inbound allowlist. This is acceptable: inbound filtering was about hiding messages from the AI (a "nice to have"), while outbound filtering is the critical security boundary (preventing the AI from contacting unauthorized people). If you want inbound filtering, you can add it back via a custom channel plugin or pre-tool hook (see Open Questions).

**Why decommission the openclaw user:**
The `openclaw` user existed to isolate the OpenClaw runtime from your personal files. Claude Code runs as your user (`blakezimmerman`) by design — it needs access to your code, configs, and tools. Isolation now comes from Claude Code's permission system (pre-approved tool allowlists) rather than OS-level user separation. The `carapace` user stays because it holds iMessage/iCloud credentials and runs the security gateway.

---

## Multi-Agent Architecture

You want multiple agents for different domains. There are two viable approaches:

### Option A: Single Dispatcher Agent (Recommended to Start)

One agent handles all iMessage conversations and uses Claude Code's built-in subagent system to delegate specialized work:

```
iMessage → Main Agent (~/agents/main/)
               │
               ├─ You ask about Kubernetes → spawns subagent with k8s tools
               ├─ You ask about email     → uses gmail-mcp directly
               ├─ You ask about code      → uses Bash/Read/Edit directly
               └─ You ask about wedding   → spawns subagent with wedding context
```

**Pros:** Simple setup, one iMessage channel, one tmux session, all context in one place.
**Cons:** Single CLAUDE.md gets large, all tools loaded at once.

**How subagents work in Claude Code:**
Claude Code can spawn specialized agents via the Agent tool. Each subagent gets its own context window, tools, and instructions. The main agent orchestrates them. You define subagent behavior in `.claude/agents/` YAML files:

```yaml
# ~/agents/main/.claude/agents/k8s-operator.yaml
name: k8s-operator
model: sonnet
instructions: |
  You are a Kubernetes operator. You manage Blake's cluster.
  Always check current state before making changes.
  Never delete namespaces without explicit confirmation.
allowed_tools:
  - Bash
  - Read
  - mcp__kubernetes__*
```

### Option B: Multiple Independent Agents

Each agent runs in its own tmux window with its own channel:

```
~/agents/
├── main/          ← iMessage channel (your phone)
│   ├── CLAUDE.md
│   ├── .mcp.json  ← gmail + imessage + github + kubernetes
│   └── .claude/settings.json
│
├── sunnysidelab/  ← Telegram or Discord channel
│   ├── CLAUDE.md
│   ├── .mcp.json  ← github + kubernetes
│   └── .claude/settings.json
│
└── infra/         ← Scheduled tasks only (no channel)
    ├── CLAUDE.md
    ├── .mcp.json  ← kubernetes + github
    └── .claude/settings.json
```

**Pros:** Clean separation, per-agent permissions, per-agent memory.
**Cons:** Need separate channels per agent (can't have two agents on one iMessage number), more tmux windows to manage.

**Recommendation:** Start with Option A. Move to Option B only when the main agent's context or CLAUDE.md becomes unwieldy. The migration is easy — just split the CLAUDE.md and .mcp.json into separate directories.

---

## Phase 1 — Build `imsg-mcp` (MCP Server for iMessage Sends)

Copy `gmail_mcp.rs`, change tool names and JSON-RPC method names. ~150 lines of Rust.

**Tools exposed:**

| Tool | JSON-RPC call | Description |
|------|---------------|-------------|
| `imsg_send` | `channel.send` | Send an iMessage to an allowlisted number |
| `imsg_list_chats` | `channel.list_chats` | List recent iMessage conversations |
| `imsg_get_history` | `channel.get_history` | Read messages from a specific chat |
| `imsg_status` | `channel.status` | Check if daemon and Messages.app are reachable |

`imsg_watch` is intentionally omitted — the native iMessage channel plugin handles inbound event detection.

**Build and install:**
```bash
cd ~/Code/Carapace
cargo build --release -p carapace-shims
sudo cp target/release/imsg-mcp /usr/local/bin/imsg-mcp
sudo chmod 755 /usr/local/bin/imsg-mcp
```

No new infrastructure — reuses the existing carapace-daemon socket, allowlist, rate limiter, and audit log.

**Add sudoers rule** so your user can invoke it as carapace:
```bash
# Add to /etc/sudoers.d/carapace-imessage (or create a new file):
blakezimmerman ALL=(carapace) NOPASSWD: /usr/local/bin/imsg-mcp
```

---

## Phase 2 — Create the Agent Directory

```bash
mkdir -p ~/agents/main/.claude/agents
mkdir -p ~/agents/main/.claude/skills
```

### `~/agents/main/CLAUDE.md`

```markdown
# Main Agent — Blake's Personal Assistant

You are Blake's primary personal assistant, running 24/7 on his Mac.
You receive messages via iMessage and have access to email, code, and infrastructure.

## About Blake
- Software engineer, based in NYC
- Code lives in ~/Code/
- Main project: Carapace (~/Code/Carapace) — security gateway for AI agents
- Side project: SunnySideLab — web apps deployed on Kubernetes

## Communication Rules
- You receive messages via iMessage (native channel)
- To reply via iMessage, use the `imsg_send` tool (goes through Carapace security)
- ALWAYS confirm before sending any iMessage or creating any email draft
- Keep responses concise when replying via iMessage — phone screens are small
- If a task will take time, acknowledge immediately then follow up when done

## Standing Instructions

### Messages
- "check messages" → use imsg_list_chats + imsg_get_history, summarize recent activity
- "reply to [name]" → draft the reply, show it to me, send only after I confirm

### Email
- "triage inbox" → gmail_search "in:inbox is:unread", summarize by priority
- "draft a reply to [subject]" → gmail_read_thread, then gmail_create_draft
- NEVER send emails directly — only create drafts

### Kubernetes
- "check the cluster" → get node status, pod health, any alerts
- "deploy [app]" → confirm the image tag and namespace before applying
- NEVER delete namespaces or PVCs without explicit confirmation
- ALWAYS do a dry-run before applying changes to production

### Code
- "check on [repo]" → git status, recent commits, open PRs
- For code changes, create a branch and PR — never push directly to main

## Memory
- Keep your memory updated with ongoing projects, deadlines, and context
- When I mention something important, save it to memory proactively
```

### `~/agents/main/.mcp.json`

```json
{
  "mcpServers": {
    "gmail": {
      "command": "sudo",
      "args": ["-u", "carapace", "/usr/local/bin/gmail-mcp"]
    },
    "imessage": {
      "command": "sudo",
      "args": ["-u", "carapace", "/usr/local/bin/imsg-mcp"]
    },
    "github": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-github"],
      "env": {
        "GITHUB_PERSONAL_ACCESS_TOKEN": "${GITHUB_TOKEN}"
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

### `~/agents/main/.claude/settings.json` — Full Permissions

This is the critical file that lets the agent run autonomously without permission prompts. Without this, Claude Code will pause and wait for you to approve every tool use — which doesn't work when you're texting from your phone.

```json
{
  "permissions": {
    "allow": [
      "Read",
      "Glob",
      "Grep",
      "WebFetch(domain:github.com)",
      "WebFetch(domain:api.github.com)",
      "WebSearch",
      "Bash(kubectl *)",
      "Bash(helm *)",
      "Bash(docker *)",
      "Bash(git *)",
      "Bash(gh *)",
      "Bash(cargo *)",
      "Bash(npm *)",
      "Bash(node *)",
      "Bash(ls *)",
      "Bash(cat *)",
      "Bash(grep *)",
      "Bash(find *)",
      "Bash(ps *)",
      "Bash(curl *)",
      "Bash(echo *)",
      "Bash(date)",
      "Bash(whoami)",
      "Bash(pwd)",
      "Bash(which *)",
      "Bash(env)",
      "mcp__gmail__*",
      "mcp__imessage__*",
      "mcp__github__*",
      "mcp__kubernetes__*"
    ],
    "deny": [
      "Bash(rm -rf /)",
      "Bash(rm -rf ~)",
      "Bash(sudo rm *)",
      "Bash(kubectl delete namespace *)",
      "Bash(kubectl delete pvc *)",
      "Edit(/etc/*)",
      "Edit(~/.ssh/*)",
      "Edit(~/.kube/*)"
    ]
  }
}
```

**Why this approach over `--dangerously-skip-permissions`:**
The allowlist is explicit — you can see exactly what the agent can and cannot do. `--dangerously-skip-permissions` gives blanket access to everything including destructive operations. The deny list acts as a safety net for the most dangerous commands. You can expand the allow list over time as you discover what the agent needs.

**Alternative — use `bypassPermissions` mode:**
If the allowlist approach is too restrictive and you're comfortable with full access:
```json
{
  "permissions": {
    "defaultMode": "bypassPermissions",
    "deny": [
      "Bash(rm -rf /)",
      "Bash(rm -rf ~)",
      "Bash(kubectl delete namespace *)"
    ]
  }
}
```

This skips all permission prompts except for the explicit deny rules. Only recommended if the agent runs on a dedicated machine or in a container.

---

## Phase 3 — iMessage Channel Setup

The iMessage channel plugin reads the local Messages database to detect incoming messages. It needs access to `chat.db`.

### Prerequisites

- Claude Code v2.1.80+ (check with `claude --version`)
- Claude Pro or Max subscription (API key auth doesn't work for Channels)
- Bun runtime: `curl -fsSL https://bun.sh/install | bash`
- The Messages database must be accessible to your user

### The chat.db Access Problem

Claude Code's iMessage plugin reads `/Users/<you>/Library/Messages/chat.db`. But in your setup, iMessage runs under the `carapace` user, so the database is at `/Users/carapace/Library/Messages/chat.db` — which your user can't read.

**Options:**

**Option A — Run one iMessage account under your user (Recommended):**
Sign into iMessage on your own macOS account too. This gives you a local `chat.db` that the iMessage plugin can read directly. The `carapace` user's iMessage account is still used for *sending* (via the `imsg-mcp` → daemon → `imsg-send` pipeline). This means:
- Inbound: Claude reads YOUR chat.db (your Apple ID's messages)
- Outbound: Claude sends via CARAPACE's Messages.app (carapace's Apple ID)
- If you want inbound and outbound on the same Apple ID, use the same Apple ID on both accounts, or use Option B.

**Option B — Symlink or grant access to carapace's chat.db:**
```bash
# Add your user to a group that can read carapace's Messages directory
sudo chmod 750 /Users/carapace/Library/Messages
sudo chown carapace:carapace-clients /Users/carapace/Library/Messages
sudo chmod 640 /Users/carapace/Library/Messages/chat.db
sudo chown carapace:carapace-clients /Users/carapace/Library/Messages/chat.db
```
Then configure the iMessage plugin to point to that path. This may require Full Disk Access for the Claude Code process.

**Option C — Custom channel plugin that reads via carapace-daemon:**
Instead of using the native iMessage plugin, build a custom Claude Code channel plugin that connects to the carapace-daemon's `channel.watch` endpoint. This preserves full Carapace security (including inbound allowlists) but requires writing a small Node.js/Bun channel plugin. See the Appendix for a skeleton.

### Setup Steps

```bash
# Install the plugin (one time)
cd ~/agents/main
claude
# Inside Claude Code:
#   /plugin install imessage@claude-plugins-official

# Configure (if needed — the plugin auto-detects the local Messages database)
#   /imessage:configure

# Lock down access
#   /imessage:access policy allowlist
#   /imessage:access pair <code>  (send a message from your phone to trigger pairing)

# Exit and relaunch with channels enabled:
#   claude --channels plugin:imessage@claude-plugins-official
```

### Test It

1. From your iPhone, send an iMessage to the number/Apple ID associated with the account Claude is watching
2. In the tmux session, you should see Claude wake up and process the message
3. If Claude's CLAUDE.md says to reply, it will use `imsg_send` to respond

---

## Phase 4 — Kubernetes and GitHub MCP Servers

### Kubernetes MCP Server

Two mature options:

**Option 1 — `mcp-server-kubernetes` (Node.js, wraps kubectl):**
```bash
# Test it works
npx mcp-server-kubernetes
# Should start and wait for MCP commands
```

Requires: `kubectl` installed and configured (`~/.kube/config` pointing to your cluster).

**Option 2 — `kubernetes-mcp-server` (Go, native K8s API):**
```bash
# Clone and build
git clone https://github.com/containers/kubernetes-mcp-server
cd kubernetes-mcp-server
make build
sudo cp bin/kubernetes-mcp-server /usr/local/bin/
```

Better performance, no kubectl dependency, 40+ tools. But requires a Go build step.

**Recommendation:** Start with Option 1 (npx). Switch to Option 2 if you hit performance issues or need advanced features.

**What the agent can do with Kubernetes MCP:**
- Check pod/node/deployment status
- View logs from any pod
- Scale deployments up/down
- Apply manifests (create/update resources)
- Describe resources for debugging
- List services, ingresses, configmaps, secrets (names only)

### GitHub MCP Server

```bash
# Already configured in .mcp.json above. Just need the token:
export GITHUB_TOKEN="ghp_..."  # or add to ~/.zshrc
```

**What the agent can do with GitHub MCP:**
- List/create/close issues and PRs
- Read PR diffs and comments
- Create branches
- Search code and repositories
- Manage releases

### Deploying Web Apps via the Agent

With Kubernetes MCP + Bash + GitHub MCP, the agent can:

1. **Build and push a Docker image:**
   ```
   You: "deploy the latest sunnysidelab frontend"
   Agent: runs `docker build`, `docker push`, `kubectl set image`
   ```

2. **Apply Kubernetes manifests:**
   ```
   You: "create a new staging environment for the API"
   Agent: generates manifests, does a dry-run, shows you the plan, applies on confirmation
   ```

3. **Full CI/CD pipeline:**
   ```
   You: "the API PR #42 is ready, deploy it to staging"
   Agent: merges PR, waits for CI, watches the rollout, reports status
   ```

The key is the CLAUDE.md instructions — tell the agent your deployment patterns, namespaces, image registries, and conventions.

---

## Phase 5 — Always-On tmux Session with Boot Automation

### Manual Start

```bash
# Create the tmux session
tmux new-session -d -s agents -n main

# Start the main agent
tmux send-keys -t agents:main \
  "cd ~/agents/main && claude --channels plugin:imessage@claude-plugins-official" Enter
```

### Reconnect

```bash
tmux attach -t agents
# Ctrl+B then D to detach (agent keeps running)
```

### Auto-Start on Boot (LaunchAgent)

Create a LaunchAgent (not LaunchDaemon — it runs as your user):

```bash
cat > ~/Library/LaunchAgents/com.blakezimmerman.claude-agents.plist << 'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.blakezimmerman.claude-agents</string>
    <key>ProgramArguments</key>
    <array>
        <string>/bin/bash</string>
        <string>-c</string>
        <string>/usr/local/bin/tmux new-session -d -s agents -n main 2>/dev/null; /usr/local/bin/tmux send-keys -t agents:main "cd ~/agents/main &amp;&amp; claude --channels plugin:imessage@claude-plugins-official" Enter</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>StandardOutPath</key>
    <string>/tmp/claude-agents.log</string>
    <key>StandardErrorPath</key>
    <string>/tmp/claude-agents.err</string>
    <key>EnvironmentVariables</key>
    <dict>
        <key>HOME</key>
        <string>/Users/blakezimmerman</string>
        <key>PATH</key>
        <string>/usr/local/bin:/usr/bin:/bin:/opt/homebrew/bin</string>
        <key>GITHUB_TOKEN</key>
        <string>ghp_YOUR_TOKEN_HERE</string>
    </dict>
</dict>
</plist>
EOF

# Load it (starts immediately and on every future login)
launchctl load ~/Library/LaunchAgents/com.blakezimmerman.claude-agents.plist
```

### Permission Handling on Boot

The agent will pause if it hits a permission prompt you haven't pre-approved. Mitigations:

1. **Pre-approve everything** in `.claude/settings.json` (Phase 2) — this is the primary defense
2. **Run manually first** — launch the agent, trigger every tool once (send a test iMessage, run kubectl, etc.), approve each prompt. Claude Code remembers approvals per-directory.
3. **Monitor on first boot** — `tmux attach -t agents` after reboot to check for prompts
4. **Nuclear option** — add `--dangerously-skip-permissions` to the tmux command. Only if you're comfortable with zero guardrails on the Claude Code side (Carapace still enforces its own guardrails for iMessage/Gmail).

---

## Phase 6 — Scheduled Automation

Claude Code supports scheduled tasks that run without manual triggering.

### In-Session Scheduling (`/loop`)

While attached to the agent's tmux session:
```
/loop 30m check kubernetes cluster health and alert me via iMessage if anything is wrong
/loop 2h triage my inbox and send me a summary via iMessage
/loop 1h check if any GitHub PRs need my review
```

These persist for 7 days or until the session ends.

### Desktop Scheduled Tasks

More durable than `/loop` — survives session restarts:
```
# From the Claude Code session:
CronCreate "0 9 * * *" "Triage inbox and send morning briefing via iMessage"
CronCreate "*/30 * * * *" "Check Kubernetes cluster health, alert if issues"
CronCreate "0 17 * * 1-5" "End of day summary: PRs merged, deployments, pending items"
```

### Cloud Scheduled Tasks

For tasks that should run even when your Mac is off:
- Configure at `claude.ai/code` → Scheduled tasks
- These run on Anthropic's infrastructure (no local file access)
- Good for: GitHub PR reviews, notification digests, monitoring dashboards
- Bad for: anything requiring local access (iMessage, kubectl to private cluster)

---

## Phase 7 — Decommission OpenClaw

Only do this after Phases 1-5 are verified working.

### Verify Checklist

- [ ] Agent receives iMessages and responds via iMessage
- [ ] Agent can search/read Gmail and create drafts
- [ ] Agent can run kubectl commands against the cluster
- [ ] Agent can read/create GitHub PRs and issues
- [ ] Agent survives a reboot and auto-starts
- [ ] Agent doesn't pause on permission prompts

### Removal Steps

```bash
# 1. Stop OpenClaw
sudo launchctl bootout system/ai.openclaw.gateway

# 2. Remove OpenClaw LaunchDaemon
sudo rm /Library/LaunchDaemons/ai.openclaw.gateway.plist

# 3. Remove the imsg RPC shim (replaced by imsg-mcp)
sudo rm /usr/local/bin/imsg

# 4. Optionally remove the openclaw user entirely
sudo sysadminctl -deleteUser openclaw

# 5. Remove openclaw from carapace-clients group
sudo dseditgroup -o edit -d openclaw -t user carapace-clients

# 6. Clean up openclaw's home directory
sudo rm -rf /Users/openclaw
```

**Keep these — they're still used:**
- `carapace-daemon` (security gateway)
- `gmail-proxy` (OAuth + content scrubbing)
- `gmail-mcp` (MCP server for Claude Code)
- `imsg-mcp` (new, Phase 1)
- The `carapace` user and all its config
- All sudoers rules in `/etc/sudoers.d/carapace-*`
- The `/var/run/carapace/` socket directory

---

## Build Order Summary

```
Phase 1: imsg-mcp              ← ~150 lines Rust, copy gmail_mcp.rs pattern
Phase 2: Agent directory        ← CLAUDE.md + .mcp.json + settings.json
Phase 3: iMessage channel       ← install plugin, pair, test
Phase 4: K8s + GitHub MCP       ← npm install, add to .mcp.json, test
Phase 5: tmux + boot automation ← LaunchAgent plist, test reboot
Phase 6: Scheduled tasks        ← /loop and CronCreate for recurring work
Phase 7: Decommission OpenClaw  ← only after everything is verified
```

Estimated time: Phase 1 is an afternoon. Phases 2-4 are a day. Phase 5 is an hour. Phase 6 is ongoing. Phase 7 is 30 minutes.

---

## What This Replaces vs. What Stays

| Component | Status After Migration |
|-----------|----------------------|
| `carapace-daemon` | **Stays** — iMessage + Gmail security gateway |
| `gmail-proxy` | **Stays** — OAuth, content scrubbing, AI-BLOCKED filtering |
| `gmail-mcp` | **Stays** — already works with Claude Code |
| `imsg-mcp` | **New** — Phase 1, outbound iMessage sends through Carapace |
| `carapace` user | **Stays** — holds iMessage/iCloud credentials, runs gateway |
| OpenClaw gateway | **Removed** — replaced by Claude Code sessions |
| OpenClaw agents | **Removed** — replaced by CLAUDE.md + tmux sessions |
| `openclaw` user | **Removed** — no longer needed |
| `imsg` RPC shim | **Removed** — replaced by imsg-mcp + native iMessage channel |

---

## File Layout After Migration

```
~/agents/main/                      ← Main agent working directory
├── CLAUDE.md                       ← Agent personality and instructions
├── .mcp.json                       ← gmail + imessage + github + kubernetes
└── .claude/
    ├── settings.json               ← Pre-approved permissions
    ├── agents/
    │   └── k8s-operator.yaml       ← Subagent definitions (optional)
    └── skills/                     ← Custom skills (optional)

~/Library/LaunchAgents/
└── com.blakezimmerman.claude-agents.plist  ← Boot automation

/usr/local/bin/
├── carapace-daemon                 ← Security gateway
├── gmail-proxy                     ← Gmail OAuth proxy
├── gmail-mcp                       ← Gmail MCP server
└── imsg-mcp                        ← iMessage MCP server (NEW)

/var/run/carapace/
├── gateway.sock                    ← Daemon socket
└── gmail-proxy.sock                ← Gmail proxy socket

/Users/carapace/
├── .config/carapace/config.toml    ← Daemon config (allowlists, rate limits)
├── .local/share/carapace/          ← Logs, audit trail
└── Library/Messages/chat.db        ← iMessage database
```

---

## Open Questions

### iMessage channel plugin and the carapace user's chat.db
The native iMessage plugin expects `chat.db` at `~/Library/Messages/chat.db` (your user). If iMessage only runs under the `carapace` user, you need one of the workarounds in Phase 3. The cleanest long-term solution may be Option C (custom channel plugin via carapace-daemon) but Option A (sign into iMessage on your account too) is the fastest path.

### Channels feature stability
Claude Code Channels is in **Research Preview** as of April 2026. The API and plugin interface may change. The core Claude Code runtime, MCP support, and headless mode are all GA and stable. If Channels breaks in an update, you can fall back to the Agent SDK to build a custom bridge (see Appendix).

### Multiple agents on one iMessage number
If you go with Option B (multiple independent agents), you can't have two Claude Code instances watching the same `chat.db` — they'll both see every message. Solutions:
- Use a dispatcher pattern (Option A) where one agent routes to subagents
- Use different channels per agent (main on iMessage, sunnysidelab on Telegram, infra on Discord)
- Use message prefixes ("@k8s check pods" routes to the k8s subagent)

### Private Kubernetes cluster access
The Kubernetes MCP server uses your local `~/.kube/config`. If your cluster requires VPN access, make sure the VPN is connected before the agent starts. Consider a `PreToolUse` hook that checks VPN status before any kubectl command.

### Cost considerations
Each Claude Code session with Channels uses Claude API credits continuously while listening. A Pro plan should be sufficient for one agent. Multiple agents with heavy usage may need Max. Monitor your usage at claude.ai after the first week.

---

## Appendix A: Custom iMessage Channel Plugin (Option C)

If you need inbound iMessages to go through the Carapace security layer (inbound allowlist, audit logging), build a custom channel plugin instead of using the native one:

```typescript
// ~/agents/imsg-channel/index.ts
// A Claude Code channel plugin that reads iMessages via carapace-daemon
import { connect } from "net";

const SOCKET_PATH = "/var/run/carapace/gateway.sock";

export default {
  name: "carapace-imessage",
  version: "0.1.0",

  async *events() {
    // Connect to carapace-daemon and subscribe to watch events
    const sock = connect(SOCKET_PATH);
    const request = JSON.stringify({
      jsonrpc: "2.0",
      id: 1,
      method: "channel.watch",
      params: { channel: "imsg" }
    }) + "\n";

    sock.write(request);

    // Parse newline-delimited JSON notifications
    let buffer = "";
    for await (const chunk of sock) {
      buffer += chunk.toString();
      let newlineIdx;
      while ((newlineIdx = buffer.indexOf("\n")) !== -1) {
        const line = buffer.slice(0, newlineIdx);
        buffer = buffer.slice(newlineIdx + 1);
        try {
          const msg = JSON.parse(line);
          if (msg.method === "watch.event") {
            yield {
              type: "message",
              from: msg.params.sender,
              text: msg.params.text,
              timestamp: msg.params.timestamp
            };
          }
        } catch {}
      }
    }
  }
};
```

Register it with: `claude --channels ./agents/imsg-channel/ --dangerously-load-development-channels`

This is more work but gives you full Carapace security on both inbound and outbound.

---

## Appendix B: Agent SDK Fallback

If Channels becomes unstable or you need more control, you can use the Claude Agent SDK to build a fully custom agent loop:

```python
# ~/agents/custom/agent.py
import asyncio
from claude_agent_sdk import query, ClaudeAgentOptions

async def handle_message(sender: str, text: str):
    """Process an incoming iMessage and respond."""
    prompt = f"[iMessage from {sender}]\n{text}"

    async for message in query(
        prompt=prompt,
        options=ClaudeAgentOptions(
            cwd="/Users/blakezimmerman/agents/main",
            allowed_tools=[
                "Read", "Bash", "Glob", "Grep",
                "mcp__gmail__*",
                "mcp__imessage__*",
                "mcp__kubernetes__*",
                "mcp__github__*"
            ],
            mcp_servers={
                "gmail": {"command": "sudo", "args": ["-u", "carapace", "/usr/local/bin/gmail-mcp"]},
                "imessage": {"command": "sudo", "args": ["-u", "carapace", "/usr/local/bin/imsg-mcp"]},
                "kubernetes": {"command": "npx", "args": ["-y", "mcp-server-kubernetes"]},
                "github": {
                    "command": "npx",
                    "args": ["-y", "@modelcontextprotocol/server-github"],
                    "env": {"GITHUB_PERSONAL_ACCESS_TOKEN": "..."}
                }
            }
        )
    ):
        print(message)

# Wire this up to carapace-daemon's channel.watch via a socket connection
```

This gives you complete control over the agent loop, message routing, error handling, and multi-agent coordination. It's more code to maintain but zero dependency on the Channels research preview.
