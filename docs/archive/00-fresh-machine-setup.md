# Fresh Machine Setup Guide

Everything you need to get Carapace running on a brand new macOS installation — iMessage, Gmail, OpenClaw, and Claude Code MCP, from scratch.

**Time required:** 2-3 hours (mostly waiting for things to install and sync)

---

## What You're Building

```
Your iPhone ──iMessage──► carapace user's Apple ID
                                   │
                              Messages.app
                                   │
                              chat.db (SQLite)
                                   │
                         imsg (real binary, private to carapace)
                                   │
                         carapace-daemon  ◄──── gmail-proxy ◄──── Gmail API
                         /var/run/carapace/gateway.sock
                                   │
                              Security layer:
                              - allowlist
                              - rate limit
                              - content filter
                              - audit log
                                   │
                    ┌──────────────┴──────────────┐
                    │                             │
             imsg shim                       gmail-mcp
         /usr/local/bin/imsg         /usr/local/bin/gmail-mcp
                    │                             │
             openclaw-gateway              Claude Code / OpenClaw
             127.0.0.1:18789                (via MCP stdio)
                    │
              openclaw agents
              (main, sunnysidelab, wedding)
```

Three macOS users:
- **You** (`blakezimmerman`) — your normal account, runs Claude Code
- **carapace** — holds messaging credentials, runs the gateway daemon
- **openclaw** — runs the AI agent runtime

---

## Before You Start

You'll need:
- [ ] A Mac running macOS 14+ (Sonoma or later)
- [ ] Admin access (your account must be an Administrator)
- [ ] An Apple ID to use for iMessage (can be your existing one or a dedicated one)
- [ ] A Google account to use for Gmail
- [ ] The Carapace source code cloned somewhere (e.g. `~/Code/Carapace`)

---

## Part 1: Install Developer Tools on Your Main Account

Do all of these steps as your normal user (not as root).

### 1.1 — Install Homebrew

```bash
/bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
```

Follow the prompts. When it finishes, it will print a line like `eval "$(/opt/homebrew/bin/brew shellenv)"` — run that line to add Homebrew to your current session. Then add it permanently:

```bash
# For Apple Silicon Macs (M1/M2/M3/M4):
echo 'eval "$(/opt/homebrew/bin/brew shellenv)"' >> ~/.zprofile
source ~/.zprofile

# For Intel Macs, Homebrew is already on PATH automatically
```

Verify: `brew --version` should print a version number.

### 1.2 — Install Rust

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
source ~/.cargo/env
```

Verify: `cargo --version` and `rustc --version` should both print version numbers.

Add cargo to your shell permanently:
```bash
echo 'source "$HOME/.cargo/env"' >> ~/.zprofile
```

### 1.3 — Install Node.js (for OpenClaw)

```bash
brew install node
```

Verify: `node --version` and `npm --version` should print version numbers.

### 1.4 — Clone the Carapace Repo

If you haven't already:
```bash
mkdir -p ~/Code
cd ~/Code
git clone https://github.com/blakezimmerman/Carapace.git
cd Carapace
```

---

## Part 2: Create the carapace User

The `carapace` user holds iMessage credentials and runs the security gateway. It never needs to be used interactively after setup.

### 2.1 — Create the User

```bash
sudo sysadminctl -addUser carapace \
    -fullName "Carapace Gateway" \
    -password -
```

You'll be prompted to enter and confirm a password. Use something strong — you'll rarely need to type it. Save it in your password manager.

Verify: `id carapace` should print the user's UID and GID.

### 2.2 — Hide from Login Screen

```bash
sudo defaults write /Library/Preferences/com.apple.loginwindow \
    HiddenUsersList -array-add carapace
```

After this, you'll access the carapace account via fast user switching (the user icon in the menu bar), not the main login screen.

---

## Part 3: Set Up iCloud and iMessage as carapace

This section must be done interactively. Log in as carapace, do the setup, then log out.

### 3.1 — Switch to the carapace User

Click the clock or user icon in the menu bar → **Switch to Another User** → **Other User**.

Type `carapace` as the username and enter the password you set.

Wait for the desktop to appear. The first login may take a minute.

### 3.2 — Sign Into iCloud

1. Open **System Settings**
2. Click **Sign in with your Apple ID** at the top
3. Enter your Apple ID email and password
4. Complete any two-factor authentication prompts on your iPhone/iPad
5. On the "Make This Your New Mac?" screen, click **Customize Settings** and uncheck everything except iCloud Drive — you don't need most iCloud features
6. Wait for the sync to complete (the progress bar in System Settings → Apple ID)

### 3.3 — Enable iMessage

1. Open **System Settings** → **Messages** (scroll down in the sidebar)
2. Toggle **iMessage** on if it's off
3. Open the **Messages** app from the Applications folder
4. Wait for your message history to sync — this can take several minutes
5. Verify you can see your conversations

### 3.4 — Grant Full Disk Access

The daemon needs Full Disk Access to read the Messages database.

1. Open **System Settings** → **Privacy & Security** → **Full Disk Access**
2. Click the lock icon to unlock (enter the carapace password)
3. Click the **+** button
4. Navigate to **Applications** → **Utilities** → **Terminal** and click **Open**
5. Toggle Terminal **on** in the list

> If you prefer, you can also add `/usr/local/bin/carapace-daemon` directly after you install it in Part 5.

### 3.5 — Create Config Directories

Open Terminal as carapace and run:

```bash
mkdir -p ~/.config/carapace
mkdir -p ~/.local/share/carapace
mkdir -p ~/.local/share/carapace/dead_letters
mkdir -p ~/.local/bin
```

### 3.6 — Add Messages.app as a Login Item

So Messages.app starts automatically when the carapace session opens (needed for sending iMessages):

1. Open **System Settings** → **General** → **Login Items**
2. Click **+** under "Open at Login"
3. Navigate to **Applications** → **Messages** and click **Add**

### 3.7 — Log Out of carapace

Click **Apple menu** → **Log Out Carapace Gateway** → **Log Out**.

You're back to your normal session. All remaining steps are done from your main account unless noted.

---

## Part 4: Create the openclaw User

The `openclaw` user runs the AI agent runtime, isolated from your personal files.

### 4.1 — Create the User

```bash
sudo sysadminctl -addUser openclaw \
    -fullName "OpenClaw Agent" \
    -password -
```

Again, use a strong password and save it.

### 4.2 — Hide from Login Screen

```bash
sudo defaults write /Library/Preferences/com.apple.loginwindow \
    HiddenUsersList -array-add openclaw
```

---

## Part 5: Set Up Cross-User Permissions

The two users need to share a socket directory.

```bash
# Create the shared group
sudo dseditgroup -o create carapace-clients

# Add carapace to the group (it owns the socket)
sudo dseditgroup -o edit -a carapace -t user carapace-clients

# Add openclaw to the group (it connects to the socket via the shim)
sudo dseditgroup -o edit -a openclaw -t user carapace-clients

# Add yourself to the group (so you can test with nc)
sudo dseditgroup -o edit -a $(whoami) -t user carapace-clients

# Create the socket directory
sudo mkdir -p /var/run/carapace

# Set ownership and permissions
sudo chown carapace:carapace-clients /var/run/carapace
sudo chmod 750 /var/run/carapace
```

**Log out and log back in** for the group membership to take effect in your current session:

```bash
# After logging back in, verify:
groups $(whoami)
# Should include: carapace-clients
```

---

## Part 6: Build and Install Carapace Binaries

From your main account, in the Carapace project directory:

```bash
cd ~/Code/Carapace
cargo build --release
```

This takes a few minutes on first run. Then install the binaries:

```bash
# Core gateway daemon
sudo cp target/release/carapace-daemon /usr/local/bin/carapace-daemon
sudo chmod 755 /usr/local/bin/carapace-daemon

# Gmail OAuth proxy
sudo cp target/release/gmail-proxy /usr/local/bin/gmail-proxy
sudo chmod 755 /usr/local/bin/gmail-proxy

# Gmail MCP server (for agents)
sudo cp target/release/gmail-mcp /usr/local/bin/gmail-mcp
sudo chmod 755 /usr/local/bin/gmail-mcp

# iMessage shim (OpenClaw calls this; it routes through the daemon)
sudo cp target/release/imsg /usr/local/bin/imsg
sudo chmod 755 /usr/local/bin/imsg
```

### 6.1 — Install the real imsg binary for carapace

The daemon needs the real `imsg` binary to read the Messages database. We install it privately into carapace's home directory so only carapace can run it:

```bash
# Download from GitHub releases (universal binary)
IMSG_VERSION="v0.5.0"
curl -fsSL -o /tmp/imsg-macos.zip \
    "https://github.com/steipete/imsg/releases/download/${IMSG_VERSION}/imsg-macos.zip"

# Extract
unzip -q /tmp/imsg-macos.zip -d /tmp/imsg-extract

# Install binary and required resource bundles
sudo mkdir -p /Users/carapace/.local/bin
sudo cp /tmp/imsg-extract/imsg /Users/carapace/.local/bin/imsg
sudo chown carapace /Users/carapace/.local/bin/imsg
sudo chmod 700 /Users/carapace/.local/bin/imsg  # only carapace can run it

# Install resource bundles (required or imsg crashes)
for bundle in /tmp/imsg-extract/*.bundle; do
    bname="$(basename "$bundle")"
    sudo cp -R "$bundle" "/Users/carapace/.local/bin/$bname"
    sudo chown -R carapace "/Users/carapace/.local/bin/$bname"
done

rm -rf /tmp/imsg-macos.zip /tmp/imsg-extract
```

---

## Part 7: Configure the carapace-daemon

Create the configuration file:

```bash
sudo tee /Users/carapace/.config/carapace/config.toml > /dev/null << 'EOF'
[gateway]
socket_path = "/var/run/carapace/gateway.sock"
log_level   = "info"

[security]
audit_log_path   = "/Users/carapace/.local/share/carapace/audit.log"
dead_letter_path = "/Users/carapace/.local/share/carapace/dead_letters"
audit_enabled    = true

[security.rate_limit]
imsg    = { requests = 30, per_seconds = 60 }
gmail   = { requests = 10, per_seconds = 60 }
default = { requests = 30, per_seconds = 60 }

[security.content_filter]
enabled = true

[[security.content_filter.patterns]]
pattern = '(?i)password\s*[:=]'
action  = "block"

[[security.content_filter.patterns]]
pattern = '(?i)api[_-]?key\s*[:=]'
action  = "block"

[[security.content_filter.patterns]]
pattern = '\b\d{3}-\d{2}-\d{4}\b'
action  = "block"

# ── iMessage ──────────────────────────────────────────────────────────────
[channels.imsg]
enabled     = true
real_binary = "/Users/carapace/.local/bin/imsg"
db_path     = "/Users/carapace/Library/Messages/chat.db"

[channels.imsg.outbound]
mode      = "allowlist"
allowlist = [
    # Add phone numbers the AI is allowed to message:
    # "+14155551234",
]

[channels.imsg.inbound]
mode = "open"   # or "allowlist" to filter who the AI can see messages from

# ── Gmail ─────────────────────────────────────────────────────────────────
[channels.gmail]
enabled      = true
proxy_socket = "/var/run/carapace/gmail-proxy.sock"

[channels.gmail.inbound]
mode = "open"
EOF

sudo chown carapace /Users/carapace/.config/carapace/config.toml
sudo chmod 600 /Users/carapace/.config/carapace/config.toml
```

**Edit the allowlist** before continuing — add the phone numbers the AI is allowed to message:

```bash
sudo -u carapace nano /Users/carapace/.config/carapace/config.toml
```

---

## Part 8: Set Up iMessage Sending

Sending iMessages from a background process on macOS requires a workaround: the daemon calls a privileged helper script that injects into the carapace GUI session via `launchctl asuser`.

### 8.1 — Create the Send Helper Directory

```bash
sudo mkdir -p /usr/local/carapace
```

### 8.2 — Create the AppleScript

```bash
sudo tee /usr/local/carapace/send-imessage.scpt > /dev/null << 'EOF'
on run argv
    set theRecipient to item 1 of argv
    set theMessage to item 2 of argv
    tell application "Messages"
        set targetService to 1st service whose service type = iMessage
        set targetBuddy to buddy theRecipient of targetService
        send theMessage to targetBuddy
    end tell
end run
EOF
sudo chmod 644 /usr/local/carapace/send-imessage.scpt
```

### 8.3 — Create the Send Wrapper

```bash
CARAPACE_UID=$(id -u carapace)

sudo tee /usr/local/carapace/imsg-send > /dev/null << EOF
#!/bin/bash
# Runs osascript inside the carapace GUI session so it can reach Messages.app
exec launchctl asuser $CARAPACE_UID osascript /usr/local/carapace/send-imessage.scpt "\$1" "\$2"
EOF
sudo chmod 755 /usr/local/carapace/imsg-send
```

### 8.4 — Grant Password-Free sudo for the Wrapper

```bash
sudo tee /etc/sudoers.d/carapace-imessage > /dev/null << 'EOF'
# Allow carapace-daemon to send iMessages without a password prompt
carapace ALL=(root) NOPASSWD: /usr/local/carapace/imsg-send
EOF
sudo chmod 440 /etc/sudoers.d/carapace-imessage
```

### 8.5 — Grant osascript Automation Permission

This must be done interactively from the carapace user's Terminal. Switch to carapace via fast user switching, open Terminal, and run:

```bash
# Grant Terminal automation access to Messages (triggers the permission dialog)
osascript -e 'tell application "Messages" to get name'
```

A dialog will appear: "Terminal wants to control Messages." Click **OK**.

Switch back to your main account when done.

---

## Part 9: Install the Carapace Gateway LaunchDaemon

This makes the daemon start automatically at every boot (no login required):

```bash
sudo cp ~/Code/Carapace/ai.carapace.gateway.agent.plist \
    /Library/LaunchDaemons/ai.carapace.gateway.plist

# Set correct ownership
sudo chown root:wheel /Library/LaunchDaemons/ai.carapace.gateway.plist
sudo chmod 644 /Library/LaunchDaemons/ai.carapace.gateway.plist

# Start it now (first time)
sudo launchctl bootstrap system /Library/LaunchDaemons/ai.carapace.gateway.plist
```

Verify it's running:

```bash
sudo launchctl print system/ai.carapace.gateway
# Look for: state = running

# Check the socket exists
ls -la /var/run/carapace/gateway.sock
# Should show: srwxrwx--- ... carapace carapace-clients gateway.sock
```

Check the logs if it doesn't start:

```bash
tail -30 /Users/carapace/.local/share/carapace/daemon.err
```

---

## Part 10: Set Up Gmail

### 10.1 — Create a Google Cloud Project and OAuth App

1. Go to [Google Cloud Console](https://console.cloud.google.com)
2. Create a new project (e.g. "Carapace Gmail")
3. Go to **APIs & Services** → **Enable APIs** → search for **Gmail API** → Enable it
4. Go to **APIs & Services** → **OAuth consent screen**:
   - Select **External**
   - Fill in app name (e.g. "Carapace"), your email, and developer email
   - Skip scopes for now, save
   - Under **Test users**, add the Gmail address you want to access
5. Go to **APIs & Services** → **Credentials** → **Create Credentials** → **OAuth client ID**:
   - Application type: **Desktop app**
   - Name: anything (e.g. "Carapace Desktop")
   - Click **Create**
6. Click **Download JSON** and save the file as `~/client_secret.json`

### 10.2 — Create Config Directories

```bash
sudo mkdir -p /etc/carapace
sudo chown carapace /etc/carapace
sudo chmod 750 /etc/carapace
```

### 10.3 — Run the OAuth Setup

```bash
sudo -u carapace gmail-proxy setup \
    --config /etc/carapace/gmail-proxy.toml \
    --client-json ~/client_secret.json
```

This will:
1. Create `/etc/carapace/gmail-proxy.toml`
2. Create `/etc/carapace/secrets.toml` (the OAuth refresh token — chmod 0600)
3. Open a browser to authorize access — log in with the Gmail account you want to use
4. After authorizing, you can close the browser tab

The secrets file never leaves the machine. Gmail Proxy refreshes the access token automatically.

### 10.4 — Create the AI-BLOCKED Label in Gmail

Open Gmail in your browser. Click **More** in the left sidebar → **Create new label** → type `AI-BLOCKED` → click **Create**. Apply this label to any emails you never want the agent to see.

### 10.5 — Edit the gmail-proxy Config

```bash
sudo -u carapace nano /etc/carapace/gmail-proxy.toml
```

Verify it looks like this (fill in your Gmail address):

```toml
[auth]
client_id     = "..."       # from your OAuth app
client_secret = "..."
secrets_file  = "secrets.toml"

[gmail]
account = "you@gmail.com"   # ← your Gmail address

[scrub]
blocked_label      = "AI-BLOCKED"
otp_patterns       = ['(?i)\b\d{6}\b', '(?i)\b\d{4}\b']
url_strip_patterns = ['(?i)https?://[^\s]*(?:reset|verify|confirm|login|signin|auth|token)[^\s]*']
strip_links        = false

[proxy]
socket_path = "/var/run/carapace/gmail-proxy.sock"
```

### 10.6 — Install the gmail-proxy LaunchDaemon

```bash
sudo cp ~/Code/Carapace/ai.carapace.gmail-proxy.plist \
    /Library/LaunchDaemons/ai.carapace.gmail-proxy.plist

sudo chown root:wheel /Library/LaunchDaemons/ai.carapace.gmail-proxy.plist
sudo chmod 644 /Library/LaunchDaemons/ai.carapace.gmail-proxy.plist

sudo launchctl bootstrap system /Library/LaunchDaemons/ai.carapace.gmail-proxy.plist
```

Verify:

```bash
# Check status
sudo launchctl print system/ai.carapace.gmail-proxy

# Test Gmail
echo '{"jsonrpc":"2.0","id":1,"method":"channel.status","params":{"channel":"gmail"}}' \
  | sudo -u carapace nc -U /var/run/carapace/gateway.sock
# Should return: {"proxy_reachable":true,"token_valid":true,...}
```

---

## Part 11: Install and Configure OpenClaw

### 11.1 — Install OpenClaw

```bash
sudo npm install -g openclaw@latest
```

### 11.2 — Create the openclaw User's Directory Structure

```bash
sudo -u openclaw mkdir -p /Users/openclaw/.openclaw
sudo -u openclaw mkdir -p /Users/openclaw/.local/share/openclaw
```

### 11.3 — Run OpenClaw Onboarding as openclaw

Switch to the openclaw user via fast user switching and open Terminal, then:

```bash
openclaw onboard
```

Follow the prompts. When asked about channels, configure iMessage with the path to the shim:
- iMessage CLI path: `/usr/local/bin/imsg`

After onboarding completes, note down the webchat URL (usually `http://127.0.0.1:18789`).

Switch back to your main account.

### 11.4 — Configure the iMessage Channel cliPath

If onboarding didn't pick it up automatically, set it manually:

```bash
sudo -u openclaw nano /Users/openclaw/.openclaw/openclaw.json
```

Find the iMessage section and make sure it has:
```json
"channels": {
  "imessage": {
    "cliPath": "/usr/local/bin/imsg"
  }
}
```

### 11.5 — Configure Gmail (MCP Server)

Run this as your main account:

```bash
NODE=/Users/openclaw/.nvm/versions/node/v22.22.2/bin/node
OPENCLAW=/Users/openclaw/.nvm/versions/node/v22.22.2/lib/node_modules/openclaw/dist/index.js

sudo -u openclaw $NODE $OPENCLAW mcp set gmail \
  '{"command":"sudo","args":["-u","carapace","/usr/local/bin/gmail-mcp"]}'
```

> **Note:** The Node path above may differ depending on which version was installed. Find yours with: `sudo -u openclaw ls /Users/openclaw/.nvm/versions/node/`

Verify it was set:
```bash
sudo -u openclaw $NODE $OPENCLAW mcp list
# Should show: gmail → sudo -u carapace /usr/local/bin/gmail-mcp
```

### 11.6 — Install the OpenClaw LaunchDaemon

OpenClaw should have created its own LaunchDaemon during onboarding. Check:

```bash
ls /Library/LaunchDaemons/ | grep openclaw
```

If it's not there, create it:

```bash
OPENCLAW_UID=$(id -u openclaw)
sudo tee /Library/LaunchDaemons/ai.openclaw.gateway.plist > /dev/null << EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>ai.openclaw.gateway</string>
    <key>ProgramArguments</key>
    <array>
        <string>/usr/local/bin/openclaw-gateway</string>
    </array>
    <key>UserName</key>
    <string>openclaw</string>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardOutPath</key>
    <string>/Users/openclaw/.local/share/openclaw/gateway.log</string>
    <key>StandardErrorPath</key>
    <string>/Users/openclaw/.local/share/openclaw/gateway.err</string>
    <key>EnvironmentVariables</key>
    <dict>
        <key>HOME</key>
        <string>/Users/openclaw</string>
    </dict>
</dict>
</plist>
EOF

sudo chown root:wheel /Library/LaunchDaemons/ai.openclaw.gateway.plist
sudo chmod 644 /Library/LaunchDaemons/ai.openclaw.gateway.plist
sudo launchctl bootstrap system /Library/LaunchDaemons/ai.openclaw.gateway.plist
```

---

## Part 12: Configure Claude Code MCP

This lets Claude Code (running as you) call the Gmail tools directly.

In the Carapace project directory, create `.mcp.json`:

```bash
cat > ~/Code/Carapace/.mcp.json << 'EOF'
{
  "mcpServers": {
    "gmail": {
      "command": "sudo",
      "args": ["-u", "carapace", "/usr/local/bin/gmail-mcp"]
    }
  }
}
EOF
```

Restart Claude Code. You should see `gmail_search`, `gmail_read_thread`, `gmail_create_draft`, and `gmail_status` appear in the available tools.

---

## Part 13: Verification

Run through these checks to confirm everything is working.

### Check: Gateway daemon is running

```bash
ps aux | grep carapace-daemon | grep -v grep
# Should show a carapace-daemon process
```

### Check: Socket exists with correct permissions

```bash
ls -la /var/run/carapace/
# gateway.sock   srwxrwx---  carapace  carapace-clients
# gmail-proxy.sock (if gmail-proxy is running)
```

### Check: iMessage reading works

```bash
echo '{"jsonrpc":"2.0","id":1,"method":"channel.list_chats","params":{"channel":"imsg","limit":3}}' \
  | sudo -u carapace nc -U /var/run/carapace/gateway.sock
```

Should return a list of your iMessage conversations.

### Check: iMessage sending works

```bash
# Replace with a number on your allowlist
sudo /usr/local/carapace/imsg-send "+1YOURNUMBER" "Test from Carapace"
```

The message should arrive on your iPhone.

> **If this fails:** Make sure you are logged into the carapace user via fast user switching (Messages.app must be running in that session). See Part 3.6 about adding Messages.app as a Login Item.

### Check: Gmail works

```bash
echo '{"jsonrpc":"2.0","id":1,"method":"channel.status","params":{"channel":"gmail"}}' \
  | sudo -u carapace nc -U /var/run/carapace/gateway.sock
# → {"proxy_reachable":true,"token_valid":true,...}

echo '{"jsonrpc":"2.0","id":2,"method":"channel.search","params":{"channel":"gmail","query":"in:inbox","max":3}}' \
  | sudo -u carapace nc -U /var/run/carapace/gateway.sock
```

### Check: OpenClaw gateway is running

```bash
ps aux | grep openclaw | grep -v grep
# Should show openclaw-gateway
```

Open `http://127.0.0.1:18789` in your browser — you should see the OpenClaw web UI.

---

## Part 14: After a Reboot or Power Loss

The following start **automatically** on every boot — no action needed:
- `gmail-proxy` (LaunchDaemon)
- `carapace-daemon` (LaunchDaemon)
- `openclaw-gateway` (LaunchDaemon)

The following require manual action if the carapace session isn't active:
- **Messages.app** — must be running in the carapace GUI session for iMessage sending to work

**After a reboot, to restore full iMessage sending:**

1. Click the user icon in the menu bar → **Other User** → type `carapace` + password
2. Wait for the desktop. Messages.app should auto-open (you added it as a Login Item in Part 3.6)
3. Switch back to your main account
4. Restart OpenClaw so it reconnects to the fresh socket:
   ```bash
   sudo launchctl kickstart -k system/ai.openclaw.gateway
   ```

**Quick health check after a reboot:**
```bash
# Are the daemons running?
ps aux | grep -E "carapace-daemon|gmail-proxy|openclaw" | grep -v grep

# Are the sockets there?
ls /var/run/carapace/

# Is Gmail responding?
echo '{"jsonrpc":"2.0","id":1,"method":"channel.status","params":{"channel":"gmail"}}' \
  | sudo -u carapace nc -U /var/run/carapace/gateway.sock
```

---

## Troubleshooting

### "Connection refused" from imsg shim

The daemon isn't running or the socket doesn't exist.
```bash
ps aux | grep carapace-daemon | grep -v grep
ls /var/run/carapace/gateway.sock
tail -20 /Users/carapace/.local/share/carapace/daemon.err
```

### "Permission denied" on socket

You're not in the `carapace-clients` group, or your group membership hasn't taken effect.
```bash
groups $(whoami)   # should include carapace-clients
# If not: log out and log back in
```

### iMessage send fails / no message arrives

1. Is carapace logged in via fast user switching? (Messages.app must be running)
2. Is the recipient in the outbound allowlist?
3. Did osascript get Automation permission? (Part 8.5)

```bash
sudo /usr/local/carapace/imsg-send "+1NUMBER" "test"
# If it fails, check:
sudo -u carapace launchctl asuser $(id -u carapace) osascript \
    /usr/local/carapace/send-imessage.scpt "+1NUMBER" "test"
```

### Gmail returns "proxy not reachable"

The gmail-proxy daemon isn't running.
```bash
ps aux | grep gmail-proxy | grep -v grep
tail -20 /Users/carapace/.local/share/carapace/gmail-proxy.err
sudo launchctl kickstart -k system/ai.carapace.gmail-proxy
```

### OpenClaw not receiving iMessages

Check that the shim is the one OpenClaw calls:
```bash
sudo -u openclaw cat /Users/openclaw/.openclaw/openclaw.json | grep cliPath
# Should show: /usr/local/bin/imsg
```

Restart OpenClaw:
```bash
sudo launchctl kickstart -k system/ai.openclaw.gateway
```

### Claude Code doesn't show gmail_ tools

1. Verify `.mcp.json` exists in the project root: `ls ~/Code/Carapace/.mcp.json`
2. Verify gmail-mcp works: `sudo -u carapace /usr/local/bin/gmail-mcp` (should wait for stdin)
3. Restart Claude Code (`/restart` or close and reopen)

---

## Key Files Reference

| File | What it is |
|------|-----------|
| `/Users/carapace/.config/carapace/config.toml` | Daemon config (allowlists, rate limits) |
| `/etc/carapace/gmail-proxy.toml` | Gmail proxy config |
| `/etc/carapace/secrets.toml` | Gmail OAuth refresh token (chmod 0600) |
| `/var/run/carapace/gateway.sock` | Main gateway socket |
| `/var/run/carapace/gmail-proxy.sock` | Gmail proxy socket |
| `/Users/carapace/.local/share/carapace/audit.log` | Every request logged here |
| `/Users/carapace/.local/share/carapace/daemon.log` | Daemon stdout |
| `/Users/carapace/.local/share/carapace/daemon.err` | Daemon stderr |
| `/Users/carapace/.local/share/carapace/gmail-proxy.log` | Gmail proxy stdout |
| `/Users/openclaw/.local/share/openclaw/gateway.err` | OpenClaw errors |
| `/tmp/imsg_rpc.log` | iMessage shim RPC log (debug) |
| `/usr/local/carapace/imsg-send` | Send wrapper (sudo → launchctl asuser) |
| `/usr/local/carapace/send-imessage.scpt` | AppleScript for Messages.app |
| `/etc/sudoers.d/carapace-imessage` | Passwordless sudo rule for send wrapper |
| `~/Code/Carapace/.mcp.json` | Claude Code MCP config |

---

## See Also

- [Operations Guide](../OPERATIONS.md) — day-to-day operations, restart commands, deploy workflow
- [Gmail Channel](14-gmail-channel.md) — Gmail architecture and JSON-RPC examples
- [Gmail MCP Server](15-gmail-mcp-server.md) — how agents use Gmail tools
- [Roadmap](12-roadmap.md) — what's planned next
