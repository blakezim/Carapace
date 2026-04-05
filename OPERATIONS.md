# Carapace + OpenClaw Operations Guide

## How It Works (The Full Picture)

```
Your iPhone
    │  iMessage
    ▼
automationsbz@icloud.com  (carapace user's Apple ID)
    │
    ▼
Messages.app  ──────────────────────────────────────────────────────────┐
(running as carapace in GUI session)                                     │
    │                                                                    │
    │  imagent detects new message                                       │
    ▼                                                                    │
chat.db  (SQLite, /Users/carapace/Library/Messages/)                    │
    │                                                                    │
    │  imsg watch --json (real binary, /Users/carapace/.local/bin/imsg)  │
    ▼                                                                    │
carapace-daemon  (JSON-RPC over Unix socket)                            │
/var/run/carapace/gateway.sock                                          │
    │                                                                    │
    │  Security middleware:                                              │
    │    - Inbound allowlist check                                      │
    │    - Rate limiting                                                │
    │    - Content filter                                               │
    │    - Audit log                                                    │
    │                                                                    │
    │  watch event forwarded to subscriber                              │
    ▼                                                                    │
imsg rpc shim  (/usr/local/bin/imsg)                                   │
(JSON-RPC over stdin/stdout)                                            │
    │                                                                    │
    ▼                                                                    │
openclaw-gateway  (WebSocket, 127.0.0.1:18789)                         │
    │                                                                    │
    ▼                                                                    │
openclaw-node  (AI agent runtime)                                       │
    │                                                                    │
    │  Agent generates reply                                            │
    │                                                                    │
    │  messages.send via imsg rpc stdin                                 │
    ▼                                                                    │
imsg rpc shim  →  carapace-daemon  →  Security middleware               │
    │                                                                    │
    │  Outbound allowlist check                                         │
    │  (only +19703161639 can be messaged)                             │
    ▼                                                                    │
sudo /usr/local/carapace/imsg-send  (root, bypasses TCC)               │
    │                                                                    │
    │  launchctl asuser 502  (injects into carapace's GUI session)      │
    ▼                                                                    │
osascript  →  Apple Events  ────────────────────────────────────────────┘
```

---

## Components and Where They Live

| Component | Process | User | How It Starts |
|-----------|---------|------|---------------|
| carapace-daemon | `/usr/local/bin/carapace-daemon` | carapace | Login Item (carapace) |
| gmail-proxy | `/usr/local/bin/gmail-proxy` | carapace | LaunchDaemon (`ai.carapace.gmail-proxy`) |
| gmail-mcp | `/usr/local/bin/gmail-mcp` | carapace | Spawned per-session by Claude Code / OpenClaw |
| openclaw-gateway | `openclaw-gateway` | openclaw | LaunchDaemon (auto, boot) |
| openclaw-node | `openclaw-node` | openclaw | Spawned by gateway |
| imsg rpc shim | `/usr/local/bin/imsg rpc` | openclaw | Spawned by openclaw-node |
| Messages.app | Messages | carapace | Must be open (Login Item recommended) |
| real imsg binary | `/Users/carapace/.local/bin/imsg` | carapace | Spawned by daemon for reads |

**Key files:**
- Gateway socket: `/var/run/carapace/gateway.sock`
- Gmail proxy socket: `/var/run/carapace/gmail-proxy.sock`
- Carapace config: `/Users/carapace/.config/carapace/config.toml`
- Gmail proxy config: `/etc/carapace/gmail-proxy.toml`
- Gmail OAuth secrets: `/etc/carapace/secrets.toml` (chmod 0600)
- Audit log: `/Users/carapace/.local/share/carapace/audit.log`
- Daemon log: `/Users/carapace/.local/share/carapace/daemon.log`
- Daemon errors: `/Users/carapace/.local/share/carapace/daemon.err`
- Gmail proxy log: `/Users/carapace/.local/share/carapace/gmail-proxy.log`
- Gmail proxy errors: `/Users/carapace/.local/share/carapace/gmail-proxy.err`
- OpenClaw gateway log: `/Users/openclaw/.local/share/openclaw/gateway.log`
- OpenClaw gateway errors: `/Users/openclaw/.local/share/openclaw/gateway.err`
- iMessage shim RPC log: `/tmp/imsg_rpc.log`
- Send wrapper: `/usr/local/carapace/imsg-send`
- Send AppleScript: `/usr/local/carapace/send-imessage.scpt`
- sudoers rule: `/etc/sudoers.d/carapace-imessage`

---

## What Starts Automatically vs. What Doesn't

### Starts automatically on every boot (no action needed):
- `gmail-proxy` — LaunchDaemon with KeepAlive
- `openclaw-gateway` — LaunchDaemon with KeepAlive
- `openclaw-node` — spawned by gateway
- `imsg rpc shim` — spawned by openclaw-node

### Starts automatically only when carapace is logged in:
- `carapace-daemon` — Login Item (starts when carapace's GUI session opens)
- `Messages.app` — only if added as a Login Item for carapace

### Does NOT automatically restart if it crashes:
- `carapace-daemon` — Login Items don't have KeepAlive. If it crashes, you must switch to carapace and relaunch it, or log carapace out and back in.

---

## After a Reboot or Power Loss

The system does not fully self-heal after a reboot. Here's what you need to do:

### Step 1 — Log in as carapace via fast user switching

The openclaw-gateway starts automatically, but without the carapace-daemon running, all iMessage operations will fail silently.

1. Click the clock/user icon in the menu bar → switch to **carapace**
2. Log in (or if already set up for auto-login, it may log in automatically)
3. The `carapace-daemon` Login Item should start within a few seconds
4. Make sure **Messages.app** is open (add it as a Login Item for carapace if it isn't already)
5. Switch back to your main account — carapace's session keeps running in the background

### Step 2 — Restart the openclaw gateway

After the carapace-daemon is running, restart the openclaw gateway so it reconnects to the fresh socket:

```bash
sudo launchctl kickstart -k system/ai.openclaw.gateway
```

### Step 3 — Verify everything is running

```bash
# One carapace-daemon process
ps aux | grep carapace-daemon | grep -v grep

# Socket exists with right permissions
ls -la /var/run/carapace/gateway.sock

# Messages flowing (send a test iMessage and watch)
tail -f /tmp/imsg_rpc.log
```

---

## Restarting Individual Components

### Restart the openclaw gateway (most common need):
```bash
sudo launchctl kickstart -k system/ai.openclaw.gateway
```
Do this if: the agent stops responding, sends time out, or you change the openclaw config.

### Restart gmail-proxy:
```bash
sudo launchctl kickstart -k system/ai.carapace.gmail-proxy
```
Do this if: Gmail search stops working, token errors appear in the gmail-proxy log, or you update the gmail-proxy binary.

### Restart the carapace-daemon:
Switch to the carapace user, open Terminal, and run:
```bash
pkill carapace-daemon
# It won't auto-restart — start it manually:
/usr/local/bin/carapace-daemon &
```
Or log the carapace user out and back in via fast user switching (the Login Item will restart it).

After restarting the daemon, also restart the openclaw gateway so the shim reconnects.

### Restart Messages.app (carapace):
Switch to carapace, quit and reopen Messages.app. No other restarts needed.

### Check if something is broken:
```bash
# Is carapace-daemon running?
ps aux | grep carapace-daemon | grep -v grep

# Is gmail-proxy running?
ps aux | grep gmail-proxy | grep -v grep

# Is Messages.app running as carapace?
ps aux | grep Messages | grep carapace | grep -v grep

# Are the sockets there?
ls -la /var/run/carapace/

# Recent daemon errors?
tail -20 /Users/carapace/.local/share/carapace/daemon.err

# Recent gmail-proxy errors?
tail -20 /Users/carapace/.local/share/carapace/gmail-proxy.err

# Recent gateway errors?
sudo tail -20 /Users/openclaw/.local/share/openclaw/gateway.err

# Is the agent receiving messages?
tail -20 /tmp/imsg_rpc.log

# Is Gmail working?
echo '{"jsonrpc":"2.0","id":1,"method":"channel.status","params":{"channel":"gmail"}}' \
  | sudo -u carapace nc -U /var/run/carapace/gateway.sock
```

---

## Deploying Code Changes

After editing Rust source code, rebuild and copy the new binary, then restart the relevant service.

### carapace-daemon changed:
```bash
# In your main account, from the Carapace project directory:
cargo build --release -p carapace-daemon
sudo cp target/release/carapace-daemon /usr/local/bin/carapace-daemon
sudo launchctl kickstart -k system/ai.carapace.gateway
```

### gmail-proxy changed:
```bash
cargo build --release -p gmail-proxy
sudo cp target/release/gmail-proxy /usr/local/bin/gmail-proxy
sudo launchctl kickstart -k system/ai.carapace.gmail-proxy
```

### gmail-mcp (MCP server) changed:
```bash
cargo build --release -p carapace-shims
sudo cp target/release/gmail-mcp /usr/local/bin/gmail-mcp
# No restart needed — spawned fresh per agent session.
```

### Both changed:
```bash
cargo build --release
sudo cp target/release/carapace-daemon /usr/local/bin/carapace-daemon
sudo cp target/release/gmail-proxy /usr/local/bin/gmail-proxy
sudo cp target/release/gmail-mcp /usr/local/bin/gmail-mcp
sudo launchctl kickstart -k system/ai.carapace.gateway
sudo launchctl kickstart -k system/ai.carapace.gmail-proxy
```

> **Note:** `launchctl kickstart -k` stops the running instance and immediately starts a fresh one. The `-k` flag means "kill then start" — you don't need to bootout/bootstrap manually.

---

## The Sending Problem (And Why It's So Complicated)

Sending iMessages from a background daemon on macOS is genuinely hard. Here's what we tried and why each approach failed or required workarounds:

### Why not just call `imsg send` from the daemon?
`imsg send` needs to talk to Messages.app via IPC. Messages.app runs in carapace's GUI session (audit session ~100171). The daemon runs in audit session 0 (the system session). macOS strictly isolates these — processes in the system session cannot reach GUI session services directly.

### Why not run the daemon as a LaunchAgent?
We tried. The problem is that LaunchAgents loaded via `sudo launchctl bootstrap gui/$UID` always get `spawn type = daemon (3)` — which means they're still in the system session despite being registered under the user's UID. The binary needs GUI entitlements (code signing) to get `spawn type = background app`. Our unsigned Rust binary can't get those entitlements without an Apple Developer account and app bundle packaging.

### Why not use osascript directly from the daemon?
Apple Events (which osascript uses) also require authorization via TCC (Transparency, Consent, and Control). The daemon's process identity is used for the TCC check, not osascript's. Getting TCC approval for a daemon process requires either a GUI prompt (which daemons can't show) or direct database modification (blocked by SIP).

### What actually works — and why:
1. The daemon calls `sudo /usr/local/carapace/imsg-send` (NOPASSWD rule, no TTY needed)
2. That wrapper runs as **root**
3. Root runs `launchctl asuser 502 osascript ...` — this injects osascript into carapace's GUI session
4. osascript has TCC approval (granted interactively from carapace's terminal)
5. Apple Events reach Messages.app successfully

The daemon runs as a **Login Item** (not a LaunchAgent) so it starts in carapace's real GUI session, which is what allows the sudo → launchctl asuser → osascript chain to work.

---

## Could This Be Simpler?

Yes. The complexity exists because of the three-user isolation model (blakezimmerman → openclaw → carapace). The security benefit: if OpenClaw is compromised, the attacker can only send iMessages to numbers on the allowlist. They cannot access your personal files, keychain, SSH keys, or browser data.

### Simpler option A: Run everything as carapace
If you're comfortable with the AI and iMessage credentials sharing the same user account, you could run OpenClaw as carapace instead of openclaw. This eliminates the cross-session complexity entirely — the daemon can call `imsg send` directly with no sudo or session tricks. The tradeoff: a compromised AI agent would have access to carapace's iCloud credentials.

### Simpler option B: Single user, no isolation
Run the carapace-daemon as a Login Item under your own account, point it at your own iMessage database, and run OpenClaw as your user. Completely eliminates multi-user complexity. Zero security isolation — a compromised agent has access to everything.

### Simpler option C: Keep isolation, fix the daemon startup
The most impactful improvement to the current setup: make Messages.app a Login Item for the carapace user, and investigate whether an Apple Developer account + proper app bundle + entitlements would let the daemon run as a proper LaunchAgent (eliminating the Login Item / fast user switching requirement). This is the "right" long-term architecture but requires more macOS packaging work.

---

## Security Model Summary

| What's protected | How |
|-----------------|-----|
| Outbound recipients | Allowlist in carapace config — only `+19703161639` can be messaged |
| Send rate | Rate limiter — max 5 sends per 60 seconds |
| Message content | Content filter blocks passwords, API keys, SSNs |
| All sends | Audit log at `/Users/carapace/.local/share/carapace/audit.log` |
| Real imsg binary | Mode 700, only carapace can execute it |
| Gateway socket | Mode 770, carapace-clients group only |
| Your personal data | openclaw user has no access to blakezimmerman's files |

---

## Quick Reference Card

**Is iMessage working?**
```bash
tail -f /tmp/imsg_rpc.log   # Send an iMessage and watch for watch.event
```

**Is Gmail working?**
```bash
echo '{"jsonrpc":"2.0","id":1,"method":"channel.status","params":{"channel":"gmail"}}' \
  | sudo -u carapace nc -U /var/run/carapace/gateway.sock
# → {"proxy_reachable":true,"token_valid":true,...}
```

**Restart everything:**
```bash
sudo pkill carapace-daemon          # won't auto-restart; switch to carapace and relaunch manually
sudo launchctl kickstart -k system/ai.carapace.gmail-proxy
sudo launchctl kickstart -k system/ai.openclaw.gateway
```

**Restart just openclaw:**
```bash
sudo launchctl kickstart -k system/ai.openclaw.gateway
```

**Restart just gmail-proxy:**
```bash
sudo launchctl kickstart -k system/ai.carapace.gmail-proxy
```

**Deploy updated binaries:**
```bash
cargo build --release
sudo cp target/release/carapace-daemon /usr/local/bin/carapace-daemon
sudo cp target/release/gmail-proxy /usr/local/bin/gmail-proxy
sudo launchctl kickstart -k system/ai.carapace.gateway
sudo launchctl kickstart -k system/ai.carapace.gmail-proxy
```

**Check audit log:**
```bash
tail -f /Users/carapace/.local/share/carapace/audit.log
```

**Test iMessage send manually:**
```bash
sudo /usr/local/carapace/imsg-send "+19703161639" "test"
```

**Test iMessage receive manually:**
```bash
echo '{"jsonrpc":"2.0","id":1,"method":"channel.list_chats","params":{"channel":"imsg"}}' \
  | sudo -u carapace nc -U /var/run/carapace/gateway.sock
```

**Test Gmail search manually:**
```bash
echo '{"jsonrpc":"2.0","id":1,"method":"channel.search","params":{"channel":"gmail","query":"in:inbox","max":3}}' \
  | sudo -u carapace nc -U /var/run/carapace/gateway.sock
```

**Confirm Gmail send is blocked:**
```bash
echo '{"jsonrpc":"2.0","id":1,"method":"channel.send","params":{"channel":"gmail","recipient":"test@example.com","message":"test"}}' \
  | sudo -u carapace nc -U /var/run/carapace/gateway.sock
# → error -32601: Gmail channel does not support direct send
```
