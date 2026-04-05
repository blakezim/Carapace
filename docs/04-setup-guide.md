# Setup Guide

Complete instructions for setting up Carapace from scratch on a Mac.

## Prerequisites

- macOS (tested on Sonoma/Sequoia)
- Admin access to the Mac
- Rust toolchain (`rustup`)
- Node.js (`brew install node`)
- Bun (`curl -fsSL https://bun.sh/install | bash`)

## Phase 1: Create the carapace User

```bash
# Create the user (hidden, no home directory UI clutter)
sudo sysadminctl -addUser carapace -fullName "Carapace Service" -password "<secure-password>" -home /Users/carapace

# Create the carapace-clients group
sudo dseditgroup -o create carapace-clients

# Add carapace to its own group
sudo dseditgroup -o edit -a carapace -t user carapace-clients

# Add your main account to the group (for socket access)
sudo dseditgroup -o edit -a blakezimmerman -t user carapace-clients
```

## Phase 2: Set Up iCloud/iMessage (for carapace user)

1. Switch to the carapace user via fast user switching (menu bar)
2. Sign into iCloud with the designated Apple ID
3. Open Messages.app and verify iMessage works
4. Add Messages.app as a Login Item (System Settings > General > Login Items)
5. Switch back to your main account

## Phase 3: Build and Install Binaries

```bash
# From the Carapace repo:
cargo build --release

# Install all binaries
sudo cp target/release/carapace-daemon /usr/local/bin/
sudo cp target/release/gmail-proxy /usr/local/bin/
sudo cp target/release/gmail-mcp /usr/local/bin/
sudo cp target/release/gdocs-proxy /usr/local/bin/
sudo cp target/release/gdocs-mcp /usr/local/bin/
```

## Phase 4: Create Socket Directory (boot-time setup)

Create the LaunchDaemon that sets up `/var/run/carapace/` on every boot:

```bash
sudo tee /Library/LaunchDaemons/ai.carapace.setup.plist > /dev/null << 'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key><string>ai.carapace.setup</string>
    <key>ProgramArguments</key>
    <array>
        <string>/bin/sh</string><string>-c</string>
        <string>mkdir -p /var/run/carapace &amp;&amp; chown carapace:carapace-clients /var/run/carapace &amp;&amp; chmod 750 /var/run/carapace</string>
    </array>
    <key>RunAtLoad</key><true/>
</dict>
</plist>
EOF
sudo launchctl bootstrap system /Library/LaunchDaemons/ai.carapace.setup.plist
```

## Phase 5: Configure the Gateway Daemon

```bash
# Create config directory
sudo -u carapace mkdir -p /Users/carapace/.config/carapace
sudo -u carapace mkdir -p /Users/carapace/.local/share/carapace
```

Create `/Users/carapace/.config/carapace/config.toml` — see [Configuration Reference](05-configuration-reference.md) for all options. Minimal example:

```toml
[channels.gmail]
enabled = true
default_account = "primary"

[channels.gmail.accounts.primary]
proxy_socket = "/var/run/carapace/gmail-proxy.sock"

[channels.gmail.accounts.primary.inbound]
mode = "open"

[channels.gdocs]
enabled = true
default_account = "hq"

[channels.gdocs.accounts.hq]
proxy_socket = "/var/run/carapace/gdocs-proxy-hq.sock"
```

Install the LaunchDaemon:

```bash
sudo tee /Library/LaunchDaemons/ai.carapace.gateway.plist > /dev/null << 'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key><string>ai.carapace.gateway</string>
    <key>ProgramArguments</key>
    <array><string>/usr/local/bin/carapace-daemon</string></array>
    <key>UserName</key><string>carapace</string>
    <key>KeepAlive</key><true/>
    <key>StandardOutPath</key><string>/Users/carapace/.local/share/carapace/daemon.log</string>
    <key>StandardErrorPath</key><string>/Users/carapace/.local/share/carapace/daemon.err</string>
    <key>RunAtLoad</key><true/>
</dict>
</plist>
EOF
sudo launchctl bootstrap system /Library/LaunchDaemons/ai.carapace.gateway.plist
```

## Phase 6: Set Up Gmail

See [Gmail Channel](07-gmail-channel.md) for full details. Summary:

1. Create a Google Cloud project, enable Gmail API, create OAuth credentials
2. Create config at `/etc/carapace/gmail-proxy.toml`
3. Pre-create secrets file with correct ownership
4. Run `sudo -u carapace gmail-proxy setup --config /etc/carapace/gmail-proxy.toml`
5. Install LaunchDaemon plist
6. Add account to daemon config

## Phase 7: Set Up Google Docs/Drive

See [GDocs Channel](08-gdocs-channel.md) for full details. Summary:

1. Enable Google Drive API, Google Docs API, Google Sheets API, Google Slides API, Google Forms API in your Cloud project
2. Create config at `/etc/carapace/gdocs-proxy-<account>.toml`
3. Pre-create secrets file with correct ownership
4. Run `sudo -u carapace gdocs-proxy setup --config /etc/carapace/gdocs-proxy-<account>.toml`
5. Install LaunchDaemon plist
6. Add account to daemon config

## Phase 8: Set Up Claude Code Agents

See [Claude Code Agents](11-claude-code-agents.md) for full details. Summary:

1. Create agent directory (`~/agents/<name>/`)
2. Write `CLAUDE.md` with agent instructions
3. Write `.mcp.json` with MCP server config
4. Write `.claude/settings.json` with permissions
5. Start agent with `CLAUDE_CONFIG_DIR` and `TELEGRAM_BOT_TOKEN`

## Phase 9: Verify Everything

```bash
# Are all daemons running?
ps aux | grep -E "carapace-daemon|gmail-proxy|gdocs-proxy" | grep -v grep

# Are all sockets present?
ls /var/run/carapace/

# Test Gmail
echo '{"jsonrpc":"2.0","id":1,"method":"channel.status","params":{"channel":"gmail","account":"primary"}}' \
  | nc -U /var/run/carapace/gateway.sock

# Test GDocs
echo '{"jsonrpc":"2.0","id":2,"method":"channel.status","params":{"channel":"gdocs","account":"hq"}}' \
  | nc -U /var/run/carapace/gateway.sock
```
