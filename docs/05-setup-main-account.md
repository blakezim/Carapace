# Setup: Your Main Account

This guide configures your main macOS account to use Carapace shims with OpenClaw.

**Prerequisites:** Complete [Setup: Carapace User](04-setup-carapace-user.md) first.

**Time required:** 15-30 minutes

## Overview

You will:

1. Create the socket directory with correct permissions
2. Add yourself to the carapace-clients group
3. Install shim tools
4. Start the Carapace daemon
5. Configure OpenClaw to use the shims
6. Verify everything works

---

## Step 1: Create Socket Directory

The Unix socket needs a directory that both users can access.

```bash
# Create the directory
sudo mkdir -p /var/run/carapace

# Create the shared group
sudo dseditgroup -o create carapace-clients

# Add carapace user to the group (owns the socket)
sudo dseditgroup -o edit -a carapace -t user carapace-clients

# Add yourself to the group (can connect to socket)
sudo dseditgroup -o edit -a $(whoami) -t user carapace-clients

# Set ownership and permissions
sudo chown carapace:carapace-clients /var/run/carapace
sudo chmod 750 /var/run/carapace
```

### Verify Group Membership

```bash
groups $(whoami)
# Should include: carapace-clients
```

**Important:** You may need to **log out and log back in** for the group membership to take effect.

---

## Step 2: Install Shim Tools

The shims are lightweight binaries that redirect commands to the Carapace daemon.

### Create Shim Directory

```bash
sudo mkdir -p /usr/local/carapace/bin
```

### Install Shims

```bash
# Option 1: Download releases (when available)
# sudo curl -L https://github.com/yourrepo/carapace/releases/latest/download/imsg-shim -o /usr/local/carapace/bin/imsg
# sudo curl -L https://github.com/yourrepo/carapace/releases/latest/download/signal-shim -o /usr/local/carapace/bin/signal-cli
# sudo curl -L https://github.com/yourrepo/carapace/releases/latest/download/discord-shim -o /usr/local/carapace/bin/discord-cli
# sudo curl -L https://github.com/yourrepo/carapace/releases/latest/download/gmail-shim -o /usr/local/carapace/bin/gog

# Option 2: Build from source
cd ~/Desktop/carapace  # or wherever you cloned it
cargo build --release --package imsg-shim
cargo build --release --package signal-shim
# etc.

sudo cp target/release/imsg-shim /usr/local/carapace/bin/imsg
sudo cp target/release/signal-shim /usr/local/carapace/bin/signal-cli
# etc.

# Set permissions
sudo chmod +x /usr/local/carapace/bin/*
```

### Add Shims to PATH

Add the shim directory to your PATH **before** any other locations where real tools might be:

```bash
# For zsh (default on modern macOS)
echo 'export PATH="/usr/local/carapace/bin:$PATH"' >> ~/.zshrc
source ~/.zshrc

# For bash
echo 'export PATH="/usr/local/carapace/bin:$PATH"' >> ~/.bash_profile
source ~/.bash_profile
```

### Verify PATH Order

```bash
which imsg
# Should output: /usr/local/carapace/bin/imsg

# NOT /usr/local/bin/imsg or /opt/homebrew/bin/imsg
```

---

## Step 3: Start the Carapace Daemon

The daemon runs as the carapace user but needs to be started.

### First-Time Start (Manual)

```bash
# Start the daemon
sudo -u carapace launchctl load /Users/carapace/Library/LaunchAgents/ai.carapace.gateway.plist
```

### Verify Daemon is Running

```bash
# Check launchctl
sudo -u carapace launchctl list | grep carapace
# Should show: ai.carapace.gateway with a PID

# Check the socket exists
ls -la /var/run/carapace/
# Should show: gateway.sock with permissions srwxrwx---

# Check daemon logs
sudo -u carapace tail -20 /Users/carapace/.local/share/carapace/daemon.log
```

### Troubleshooting Daemon Start

If the daemon doesn't start:

```bash
# Check for errors
sudo -u carapace tail -50 /Users/carapace/.local/share/carapace/daemon.err

# Try running manually to see errors
sudo -u carapace /usr/local/bin/carapace-daemon --config /Users/carapace/.config/carapace/config.toml
```

---

## Step 4: Test the Shims

Before configuring OpenClaw, verify the shims work:

### Test Connection

```bash
# List chats (should show your iMessage conversations)
imsg chats --limit 5
```

Expected output:
```
+1234567890 (John Doe): Last message preview...
+0987654321 (Jane Doe): Another message...
```

If you see this, the shim successfully connected to the daemon, which accessed the real iMessage database.

### Test Sending (to an allowlisted contact)

```bash
# Only works if recipient is in your allowlist!
imsg send "+1234567890" "Test from Carapace"
```

### Test Blocked Send

```bash
# Try sending to a non-allowlisted number
imsg send "+1999999999" "This should be blocked"
```

Expected output:
```
Error: Recipient not in allowlist
```

---

## Step 5: Configure OpenClaw

### Install OpenClaw (if not already installed)

```bash
npm install -g openclaw@latest
```

### Run Onboarding

```bash
openclaw onboard
```

During onboarding, OpenClaw will detect `imsg` in your PATH. Since the shim is first in PATH, it will use that.

### Manual Configuration

If you've already set up OpenClaw, edit the config to point to shims:

```bash
nano ~/.openclaw/openclaw.json
```

```json5
{
  channels: {
    imessage: {
      cliPath: "/usr/local/carapace/bin/imsg",
      // Note: no dbPath needed, the daemon handles it
    },
    // If using Signal:
    signal: {
      cliPath: "/usr/local/carapace/bin/signal-cli",
    },
    // Other channels as configured
  },
}
```

---

## Step 6: Verify OpenClaw Works

### Start OpenClaw

```bash
openclaw gateway
```

### Test via OpenClaw

Send a message to yourself or an allowlisted contact through OpenClaw's interface (Telegram, Discord, WhatsApp, or direct).

Check the audit log:

```bash
sudo -u carapace tail -f /Users/carapace/.local/share/carapace/audit.log
```

You should see entries for each operation OpenClaw performs.

---

## Step 7: (Optional) Daemon Auto-Start on Boot

The daemon is configured to start when the carapace user logs in, but macOS doesn't log in hidden users automatically.

### Option A: Keep Carapace Logged In (Background)

You can keep the carapace user logged in without showing on screen:

```bash
# Enable fast user switching
sudo defaults write /Library/Preferences/.GlobalPreferences MultipleSessionEnabled -bool true

# Log into carapace (from login screen), then switch back to your account
# Carapace stays logged in the background
```

### Option B: LaunchDaemon (System-Wide)

For a more robust solution, create a system-wide daemon:

```bash
sudo cat > /Library/LaunchDaemons/ai.carapace.gateway.plist << 'EOF'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>ai.carapace.gateway</string>
    <key>ProgramArguments</key>
    <array>
        <string>/usr/local/bin/carapace-daemon</string>
        <string>--config</string>
        <string>/Users/carapace/.config/carapace/config.toml</string>
    </array>
    <key>UserName</key>
    <string>carapace</string>
    <key>GroupName</key>
    <string>carapace</string>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardOutPath</key>
    <string>/Users/carapace/.local/share/carapace/daemon.log</string>
    <key>StandardErrorPath</key>
    <string>/Users/carapace/.local/share/carapace/daemon.err</string>
</dict>
</plist>
EOF

sudo launchctl load /Library/LaunchDaemons/ai.carapace.gateway.plist
```

**Note:** This runs the daemon at system startup, not user login.

---

## Quick Reference

| Task | Command |
|------|---------|
| Check daemon status | `sudo -u carapace launchctl list \| grep carapace` |
| View daemon logs | `sudo -u carapace tail -f ~/.local/share/carapace/daemon.log` |
| View audit log | `sudo -u carapace tail -f ~/.local/share/carapace/audit.log` |
| Restart daemon | `sudo -u carapace launchctl kickstart -k gui/$(id -u carapace)/ai.carapace.gateway` |
| Test shim | `imsg chats --limit 5` |
| Check PATH | `which imsg` |
| Check group | `groups $(whoami)` |

---

## Troubleshooting

### "Connection refused" from shims

1. **Is the daemon running?**
   ```bash
   sudo -u carapace launchctl list | grep carapace
   ```

2. **Does the socket exist?**
   ```bash
   ls -la /var/run/carapace/gateway.sock
   ```

3. **Are you in the right group?**
   ```bash
   groups $(whoami) | grep carapace-clients
   ```
   If not, you may need to log out and back in.

### "Permission denied" on socket

```bash
# Check socket permissions
ls -la /var/run/carapace/

# Should be:
# drwxr-x--- carapace carapace-clients /var/run/carapace/
# srwxrwx--- carapace carapace-clients gateway.sock

# Fix if needed:
sudo chown carapace:carapace-clients /var/run/carapace
sudo chmod 750 /var/run/carapace
```

### Shim not found / wrong tool runs

```bash
# Check PATH order
echo $PATH

# Should start with /usr/local/carapace/bin

# Check which tool runs
which imsg

# Should be /usr/local/carapace/bin/imsg
```

### OpenClaw not using shims

Check OpenClaw's config:

```bash
cat ~/.openclaw/openclaw.json | grep cliPath
```

Should show `/usr/local/carapace/bin/imsg`.

---

## Next Steps

- [Protocol Specification](06-protocol-spec.md) - Understand the JSON-RPC protocol
- [Configuration Reference](10-configuration-reference.md) - All config options
- [Troubleshooting](11-troubleshooting.md) - More detailed problem-solving
