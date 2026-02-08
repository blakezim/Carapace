# Setup: Carapace User

This guide walks through setting up the carapace user account, which holds all messaging credentials and runs the security gateway.

**Time required:** 30-60 minutes (one-time setup)

## Prerequisites

- macOS 14+ (Sonoma or later)
- Admin access to your Mac
- Apple ID (for iMessage, can be your existing one or a dedicated one)
- Phone numbers for Signal registration (if using Signal)

## Overview

You will:

1. Create a new macOS user called "carapace"
2. Log in as that user
3. Set up all messaging channels (iCloud, Signal, Discord, etc.)
4. Install and configure the Carapace daemon
5. Log out and return to your normal account

After this, you won't need to log into the carapace account for normal use.

---

## Step 1: Create the Carapace User

### Via System Settings (Recommended)

1. Open **System Settings**
2. Go to **Users & Groups**
3. Click the **+** button (you may need to unlock with your password)
4. Fill in:
   - **Full Name:** Carapace Gateway
   - **Account Name:** carapace
   - **Password:** (choose a strong password and save it somewhere secure)
   - **Verify:** (re-enter password)
   - **Password hint:** (optional, leave blank for security)
5. Set **Account Type** to **Standard** (not Administrator)
6. Click **Create User**

### Via Command Line (Alternative)

```bash
# Create user with sysadminctl
sudo sysadminctl -addUser carapace -fullName "Carapace Gateway" -password -

# You'll be prompted to enter and confirm the password
```

---

## Step 2: Log In as Carapace

1. Click the **Apple menu** → **Log Out [Your Name]**
2. At the login screen, click **Other User** or select **Carapace Gateway**
3. Enter the password you created
4. Wait for the desktop to load (first login may take a minute)

You're now logged in as the carapace user. All the following steps are done in this context.

---

## Step 3: Set Up iCloud / iMessage

### Sign Into iCloud

1. Open **System Settings**
2. Click **Sign in with your Apple ID** (or **Apple ID** if already showing)
3. Enter your Apple ID credentials
   - You can use your existing Apple ID, or
   - Create a dedicated Apple ID for the bot (more isolated)
4. Complete any two-factor authentication prompts
5. Wait for iCloud to sync

### Enable iMessage

1. Still in System Settings, go to **Messages** (or **iMessage**)
2. Ensure **Enable Messages in iCloud** is checked
3. Open the **Messages** app (in Applications)
4. Wait for your message history to sync (this may take several minutes)
5. Verify you can see your conversations

### Grant Full Disk Access

The daemon needs Full Disk Access to read the Messages database.

1. Open **System Settings** → **Privacy & Security** → **Full Disk Access**
2. Click the **+** button
3. Navigate to **Applications** → **Utilities** → **Terminal.app**
4. Add it and toggle it **ON**
5. You may need to restart Terminal for this to take effect

---

## Step 4: Set Up Signal (Optional)

If you want Carapace to handle Signal messages:

### Install Homebrew (if not already installed)

```bash
/bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
```

Follow the prompts to add Homebrew to your PATH.

### Install signal-cli

```bash
brew install signal-cli
```

### Register Your Phone Number

```bash
# Start registration (you'll receive an SMS)
signal-cli -a +1YOURPHONENUMBER register

# Enter the verification code
signal-cli -a +1YOURPHONENUMBER verify 123456
```

### Test It Works

```bash
# Send a test message
signal-cli -a +1YOURPHONENUMBER send -m "Test from Carapace" +1RECIPIENTNUMBER
```

---

## Step 5: Set Up Discord Bot (Optional)

If you want Carapace to handle Discord messages:

### Create a Discord Bot

1. Open **Safari** and go to https://discord.com/developers/applications
2. Click **New Application**
3. Name it (e.g., "OpenClaw Bot" or "Carapace Bot")
4. Click **Create**
5. Go to the **Bot** section in the left sidebar
6. Click **Add Bot**, then **Yes, do it!**

### Get the Bot Token

1. Under the bot's username, click **Reset Token**
2. Click **Yes, do it!**
3. **Copy the token immediately** (you can only see it once)
4. Save it securely:

```bash
mkdir -p ~/.config/carapace
echo "YOUR_BOT_TOKEN_HERE" > ~/.config/carapace/discord_token
chmod 600 ~/.config/carapace/discord_token
```

### Invite the Bot to Your Server

1. Go to **OAuth2** → **URL Generator** in the Discord developer portal
2. Under **Scopes**, select:
   - `bot`
   - `applications.commands`
3. Under **Bot Permissions**, select:
   - Send Messages
   - Read Message History
   - View Channels
4. Copy the generated URL at the bottom
5. Open it in your browser
6. Select your server and click **Authorize**

---

## Step 6: Set Up Gmail (Optional)

If you want Carapace to handle Gmail:

### Install gog

```bash
brew install charmbracelet/tap/gog
```

### Authenticate

```bash
gog auth login
```

This will open a browser for OAuth authentication. Follow the prompts to authorize.

Tokens are saved to `~/.config/gog/`.

---

## Step 7: Install imsg

The real imsg tool that accesses the Messages database:

```bash
brew install steipete/tap/imsg
```

### Test It Works

```bash
# List recent chats
imsg chats --limit 5

# You should see your iMessage conversations
```

If you get a permission error, ensure Full Disk Access is granted to Terminal.

---

## Step 8: Install Carapace Daemon

### Create Directories

```bash
mkdir -p ~/.config/carapace
mkdir -p ~/.local/share/carapace
mkdir -p ~/.local/share/carapace/dead_letters
```

### Install the Daemon Binary

```bash
# Option 1: Download release (when available)
# curl -L https://github.com/yourrepo/carapace/releases/latest/download/carapace-daemon-macos-arm64 -o /usr/local/bin/carapace-daemon
# chmod +x /usr/local/bin/carapace-daemon

# Option 2: Build from source (for now)
cd ~/Desktop
git clone https://github.com/yourrepo/carapace.git
cd carapace
cargo build --release
cp target/release/carapace-daemon /usr/local/bin/
```

---

## Step 9: Configure Carapace

Create the configuration file:

```bash
cat > ~/.config/carapace/config.toml << 'EOF'
# Carapace Gateway Configuration

[gateway]
socket_path = "/var/run/carapace/gateway.sock"
log_level = "info"

[security]
audit_log_path = "/Users/carapace/.local/share/carapace/audit.log"
dead_letter_path = "/Users/carapace/.local/share/carapace/dead_letters"

[security.rate_limit]
# Requests per minute per channel
imsg = { requests = 30, per_seconds = 60 }
signal = { requests = 20, per_seconds = 60 }
discord = { requests = 60, per_seconds = 60 }
gmail = { requests = 10, per_seconds = 60 }

[security.content_filter]
enabled = true
patterns = [
    { pattern = "(?i)password\\s*[:=]", action = "block" },
    { pattern = "(?i)api[_-]?key\\s*[:=]", action = "block" },
    { pattern = "(?i)secret.*token", action = "block" },
    { pattern = "\\b\\d{3}-\\d{2}-\\d{4}\\b", action = "block" },
]

# ============================================
# CHANNEL: iMessage
# ============================================
[channels.imsg]
enabled = true
real_binary = "/opt/homebrew/bin/imsg"
db_path = "/Users/carapace/Library/Messages/chat.db"

[channels.imsg.outbound]
# Who can the AI send messages TO?
mode = "allowlist"  # "allowlist" | "denylist" | "open"
allowlist = [
    # Add phone numbers and emails here:
    # "+14155551234",
    # "email:friend@icloud.com",
]

[channels.imsg.inbound]
# Who can send messages that the AI sees?
mode = "allowlist"
allowlist = [
    # Add phone numbers and emails here:
    # "+14155551234",
]

# ============================================
# CHANNEL: Signal (uncomment if using)
# ============================================
# [channels.signal]
# enabled = true
# signal_cli_path = "/opt/homebrew/bin/signal-cli"
# account = "+1YOURPHONENUMBER"
#
# [channels.signal.outbound]
# mode = "allowlist"
# allowlist = []
#
# [channels.signal.inbound]
# mode = "allowlist"
# allowlist = []

# ============================================
# CHANNEL: Discord (uncomment if using)
# ============================================
# [channels.discord]
# enabled = true
# token_file = "/Users/carapace/.config/carapace/discord_token"
#
# [channels.discord.outbound]
# mode = "allowlist"
# allowlist = [
#     # "channel:123456789012345678",
#     # "user:987654321098765432",
# ]

# ============================================
# CHANNEL: Gmail (uncomment if using)
# ============================================
# [channels.gmail]
# enabled = true
# credentials_path = "/Users/carapace/.config/gog"
#
# [channels.gmail.outbound]
# mode = "allowlist"
# allowlist = [
#     # "friend@example.com",
#     # "*@mycompany.com",
# ]
EOF
```

### Edit the Allowlists

```bash
nano ~/.config/carapace/config.toml
```

Add your trusted contacts to the allowlists. Be conservative—you can always add more later.

---

## Step 10: Set Up the Daemon to Run Automatically

### Create LaunchAgent

```bash
mkdir -p ~/Library/LaunchAgents

cat > ~/Library/LaunchAgents/ai.carapace.gateway.plist << 'EOF'
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
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardOutPath</key>
    <string>/Users/carapace/.local/share/carapace/daemon.log</string>
    <key>StandardErrorPath</key>
    <string>/Users/carapace/.local/share/carapace/daemon.err</string>
    <key>EnvironmentVariables</key>
    <dict>
        <key>HOME</key>
        <string>/Users/carapace</string>
    </dict>
</dict>
</plist>
EOF
```

### Note About Socket Directory

The socket directory `/var/run/carapace/` will be created when you set up your main account. For now, the daemon won't be able to start until that's done.

---

## Step 11: Hide Carapace User (Optional but Recommended)

Hide the carapace user from the login screen for security:

```bash
sudo dscl . create /Users/carapace IsHidden 1
```

To log in later, you'll need to click "Other User" and type "carapace" as the username.

---

## Step 12: Log Out

1. Click **Apple menu** → **Log Out Carapace Gateway**
2. Log back into your normal account

---

## What's Next

Now continue to [Setup: Your Main Account](05-setup-main-account.md) to:

1. Create the socket directory with correct permissions
2. Install the shim tools
3. Configure OpenClaw to use the shims
4. Start the daemon

---

## Maintenance

### Adding Contacts to Allowlist

You'll need to edit the config as the carapace user:

```bash
# From your main account:
sudo -u carapace nano /Users/carapace/.config/carapace/config.toml

# Then reload the daemon:
sudo -u carapace launchctl kickstart -k gui/$(id -u carapace)/ai.carapace.gateway
```

### Viewing Logs

```bash
# Daemon logs
sudo -u carapace tail -f /Users/carapace/.local/share/carapace/daemon.log

# Audit logs
sudo -u carapace tail -f /Users/carapace/.local/share/carapace/audit.log
```

### Checking Daemon Status

```bash
sudo -u carapace launchctl list | grep carapace
```
