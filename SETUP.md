# Carapace -- Setup & Operations Guide

All five phases of Carapace are implemented and working:

1. Carapace user creation & iCloud sign-in
2. Cross-user permissions (group, socket directory)
3. Gateway infrastructure (daemon, JSON-RPC, client library)
4. Security middleware (rate limiting, allowlists, content filtering, audit log)
5. iMessage channel adapter (send, list_chats, get_history, watch)

This guide covers the full install, the gotchas we hit along the way, and how
to integrate with OpenClaw via the `imsg` shim.

---

## Prerequisites

- **macOS 14+** (Sonoma or later -- imsg requires it)
- **Rust toolchain** -- install via [rustup.rs](https://rustup.rs):
  ```bash
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
  ```
- **Admin access** on the Mac (you'll need `sudo` a few times)

---

## Phase 1: Create the Carapace User

This is the dedicated macOS user that holds your messaging credentials,
isolated from the AI runtime.

### Create the User

```bash
sudo sysadminctl -addUser carapace \
    -fullName "Carapace Gateway" \
    -password - \
    -home /Users/carapace
```

Or use System Settings -> Users & Groups -> (+) to create a Standard user
named `carapace`.

### Post-Creation (Must Be Done as Carapace)

Log in as the `carapace` user (fast user switching or log out/log in):

1. **Sign into iCloud** -- System Settings -> Apple ID (enables iMessage)
2. **Open Messages.app** and verify it activates
3. **Grant Full Disk Access to Terminal** -- System Settings -> Privacy &
   Security -> Full Disk Access -> add Terminal
4. **Create config directories:**
   ```bash
   mkdir -p ~/.config/carapace ~/.local/share/carapace ~/.local/bin
   ```
5. **Log out** of the carapace account

### Install imsg (Real Binary)

> **Gotcha:** Do NOT use `$(which imsg)` from a Homebrew install. Homebrew's
> formula uses `write_exec_script`, which puts a *wrapper shell script* on
> PATH -- not the actual Mach-O binary. Copying that wrapper to another user
> gives you a script that can't find the real binary in Homebrew's libexec.
>
> The zip from GitHub releases also contains resource bundles
> (PhoneNumberKit, SQLite) that must sit alongside the binary or imsg will
> crash on launch. Homebrew scatters these into its own Cellar layout.

Download the universal binary directly from GitHub releases:

```bash
# Download and extract
curl -fsSL -o /tmp/imsg-macos.zip \
  https://github.com/steipete/imsg/releases/download/v0.5.0/imsg-macos.zip
unzip -q /tmp/imsg-macos.zip -d /tmp/imsg

# Install binary + resource bundles (carapace-only access)
sudo mkdir -p /Users/carapace/.local/bin
sudo cp /tmp/imsg/imsg /Users/carapace/.local/bin/imsg
sudo cp -R /tmp/imsg/*.bundle /Users/carapace/.local/bin/
sudo chown -R carapace /Users/carapace/.local/bin/
sudo chmod 700 /Users/carapace/.local/bin/imsg

# Clean up
rm -rf /tmp/imsg /tmp/imsg-macos.zip
```

Verify isolation:
```bash
# This should fail with "permission denied" -- correct!
/Users/carapace/.local/bin/imsg --help

# This should work:
sudo -u carapace /Users/carapace/.local/bin/imsg --help
```

### Hide from Login Screen (Optional)

```bash
sudo defaults write /Library/Preferences/com.apple.loginwindow \
    HiddenUsersList -array-add carapace
```

---

## Phase 2: Cross-User Permissions

### Create the Shared Group

```bash
sudo dseditgroup -o create carapace-clients
sudo dseditgroup -o edit -a carapace -t user carapace-clients
sudo dseditgroup -o edit -a $(whoami) -t user carapace-clients
```

### Create the Socket Directory

```bash
sudo mkdir -p /var/run/carapace
sudo chown carapace:carapace-clients /var/run/carapace
sudo chmod 750 /var/run/carapace
```

### Log Out and Back In

Group membership changes don't take effect until you log out of your macOS
session and log back in. **This is required.**

```bash
# Verify after logging back in:
groups | tr ' ' '\n' | grep carapace-clients
# Should print: carapace-clients
```

### Audit Log Permissions

The carapace user needs write access to the audit log path. The default config
uses `/Users/carapace/.local/share/carapace/audit.log`, which is already owned
by carapace. If you change `audit_log_path` in the config, make sure the
carapace user can write to it.

---

## Phase 3: Build & Install

```bash
# From the Carapace project root:
cargo build --release
```

This produces three binaries:
- `target/release/carapace-daemon` -- the gateway server
- `target/release/test-shim` -- the test client
- `target/release/imsg` -- the drop-in CLI shim for OpenClaw

### Install Binaries

```bash
# Daemon
sudo cp target/release/carapace-daemon /usr/local/bin/carapace-daemon
sudo chmod 755 /usr/local/bin/carapace-daemon

# Test shim
sudo mkdir -p /usr/local/carapace/bin
sudo cp target/release/test-shim /usr/local/carapace/bin/test-shim
sudo chmod 755 /usr/local/carapace/bin/test-shim

# imsg shim (this replaces any Homebrew imsg on your main account)
sudo cp target/release/imsg /usr/local/bin/imsg
sudo chmod 755 /usr/local/bin/imsg
```

> **After rebuilding:** If you rebuild the daemon (`cargo build --release`),
> you must copy the new binary to `/usr/local/bin/` and restart. The
> LaunchDaemon runs whatever is at `/usr/local/bin/carapace-daemon`, not the
> build directory.

---

## Phase 4: Configuration

### Daemon Config

The install script writes a default config to
`/Users/carapace/.config/carapace/config.toml`. Key settings:

```toml
[gateway]
socket_path = "/var/run/carapace/gateway.sock"

[security]
audit_log_path = "/Users/carapace/.local/share/carapace/audit.log"
dead_letter_path = "/Users/carapace/.local/share/carapace/dead_letters"
audit_enabled = true

[channels.imsg]
enabled     = true
real_binary = "/Users/carapace/.local/bin/imsg"
db_path     = "/Users/carapace/Library/Messages/chat.db"

[channels.imsg.outbound]
mode      = "allowlist"
allowlist = ["+14155551234"]
```

See the generated config from `install.sh` for the full template including
rate limits, content filter patterns, and additional channel stubs.

> **Gotcha: Dev vs production paths.** If you tested with `config.dev.toml`
> first, make sure the production config at
> `/Users/carapace/.config/carapace/config.toml` has the correct paths:
>
> | Setting | Production (correct) | Dev (wrong for prod) |
> |---------|---------------------|---------------------|
> | `socket_path` | `/var/run/carapace/gateway.sock` | `/tmp/carapace-test.sock` |
> | `audit_log_path` | `/Users/carapace/.local/share/carapace/audit.log` | `/tmp/carapace-audit.log` |
> | `dead_letter_path` | `/Users/carapace/.local/share/carapace/dead_letters` | `/tmp/carapace-dead-letters` |
>
> The `/tmp/` paths will cause "permission denied" errors when the daemon
> runs as the carapace user, because files in `/tmp/` may be owned by root
> or your main user. Always use paths under `/Users/carapace/` for
> production.

### Socket Group Ownership

The daemon automatically sets the socket's group to `carapace-clients` on
startup (`chgrp` + `chmod 770`). You don't need to manually fix socket
permissions after each restart.

### LaunchDaemon (Auto-Start at Boot)

The install script creates `/Library/LaunchDaemons/ai.carapace.gateway.plist`:

```bash
# Load the daemon (starts immediately and on every boot)
sudo launchctl load /Library/LaunchDaemons/ai.carapace.gateway.plist

# Check status
sudo launchctl list | grep carapace

# Stop
sudo launchctl unload /Library/LaunchDaemons/ai.carapace.gateway.plist
```

The plist runs the daemon as the `carapace` user with `GroupName`
`carapace-clients`, so launchd handles the user context. Logs go to:
- stdout: `/Users/carapace/.local/share/carapace/daemon.log`
- stderr: `/Users/carapace/.local/share/carapace/daemon.err`

### Dev/Test Config

For local development without the carapace user:

```bash
# Use the dev config with a temp socket
carapace-daemon --config ./config.dev.toml
```

This uses `/tmp/carapace-test.sock` and `/tmp/` for audit/dead-letter paths.
These paths are intentionally in `/tmp/` for dev -- don't copy them into the
production config.

---

## Phase 5: Testing

### Quick Smoke Test (Same User)

No carapace user needed -- runs everything as your user:

```bash
# Terminal 1: start daemon with temp socket
CARAPACE_SOCKET_PATH=/tmp/carapace-test.sock cargo run --release -p carapace-daemon

# Terminal 2: run test shim
CARAPACE_SOCKET_PATH=/tmp/carapace-test.sock cargo run --release -p carapace-shims --bin test-shim
```

### Full Cross-User Test

Start the daemon as carapace, call from your account:

```bash
# Terminal 1: start daemon as carapace
sudo -u carapace /usr/local/bin/carapace-daemon

# Terminal 2: run test shim as yourself
/usr/local/carapace/bin/test-shim
```

The `whoami` test proves OS-level isolation: daemon runs as `carapace`, you
call from your own account.

### iMessage Testing via netcat

Test the iMessage channel directly over the Unix socket:

```bash
# Channel status
echo '{"jsonrpc":"2.0","id":1,"method":"channel.status","params":{"channel":"imsg"}}' \
  | nc -U /var/run/carapace/gateway.sock

# List chats
echo '{"jsonrpc":"2.0","id":2,"method":"channel.list_chats","params":{"channel":"imsg"}}' \
  | nc -U /var/run/carapace/gateway.sock

# Send a message (recipient must be in the outbound allowlist)
echo '{"jsonrpc":"2.0","id":3,"method":"channel.send","params":{"channel":"imsg","recipient":"+14155551234","message":"test from netcat"}}' \
  | nc -U /var/run/carapace/gateway.sock
```

### Security Guardrail Tests

```bash
# Rate limit -- send many requests quickly, should get -32002 error
for i in $(seq 1 10); do
  echo '{"jsonrpc":"2.0","id":'$i',"method":"channel.send","params":{"channel":"imsg","recipient":"+14155551234","message":"spam test '$i'"}}' \
    | nc -U /var/run/carapace/gateway.sock
done

# Content filter -- should get -32003 error (blocked pattern)
echo '{"jsonrpc":"2.0","id":1,"method":"channel.send","params":{"channel":"imsg","recipient":"+14155551234","message":"my password: hunter2"}}' \
  | nc -U /var/run/carapace/gateway.sock

# Allowlist -- send to a number NOT in the allowlist, should get -32001 error
echo '{"jsonrpc":"2.0","id":1,"method":"channel.send","params":{"channel":"imsg","recipient":"+19999999999","message":"test"}}' \
  | nc -U /var/run/carapace/gateway.sock
```

---

## Phase 6: OpenClaw Integration

The `imsg` shim binary is a drop-in CLI replacement that mirrors the real
`imsg` interface but routes every command through the Carapace gateway.
OpenClaw calls `imsg` as usual -- it doesn't know or care that a security
layer sits in between.

### Recommended: Run OpenClaw as a Separate User

Since you interact with OpenClaw via iMessage (not at the keyboard), there's
no reason for it to run as your personal user. Running it as a dedicated
`openclaw` user adds a second isolation boundary:

```
YOUR USER (personal account -- browsing, dev, etc.)
    |
OPENCLAW USER (AI runtime only -- no personal data, no credentials)
    |  connects via socket (carapace-clients group)
    v
CARAPACE USER (credentials only -- iCloud, iMessage, real imsg binary)
```

This means even if the AI is fully compromised, it cannot access your personal
files, keychain, SSH keys, browser data, etc. It can only reach iMessage
through the Carapace gateway (with allowlists, rate limits, and content
filtering enforced).

To set up:

```bash
# Create the openclaw user
sudo sysadminctl -addUser openclaw \
    -fullName "OpenClaw Runtime" \
    -password - \
    -home /Users/openclaw

# Add to carapace-clients so the shim can reach the socket
sudo dseditgroup -o edit -a openclaw -t user carapace-clients

# Hide from login screen
sudo defaults write /Library/Preferences/com.apple.loginwindow \
    HiddenUsersList -array-add openclaw
```

The `imsg` shim at `/usr/local/bin/imsg` is already `755`, so the openclaw
user can run it. Install and configure OpenClaw in `/Users/openclaw/`.

### Install the Shim

The shim was already built in Phase 3 and installed to `/usr/local/bin/imsg`.
Verify it's the compiled shim (not a Homebrew wrapper):

```bash
file /usr/local/bin/imsg
# Should say: Mach-O 64-bit executable ...

imsg status
# Should return JSON from the gateway, not the real imsg
```

### How It Works

```
OpenClaw calls:  imsg send --to "+1234" --text "Hello"
                   |
                   v
           /usr/local/bin/imsg  (Carapace shim)
                   |
                   v
         Unix socket --> Carapace daemon (as carapace user)
                   |
                   v
         Allowlist --> Rate limit --> Content filter --> Audit log
                   |
                   v
         /Users/carapace/.local/bin/imsg  (real binary)
                   |
                   v
              iMessage sent
```

### Verify End-to-End

```bash
# Check channel status
imsg status

# List chats (should return your iMessage chat list)
imsg chats --json

# Send a test message (recipient must be in outbound allowlist)
imsg send --to "+14155551234" --text "test from shim"
```

### Supported Commands

| Command | Description |
|---------|-------------|
| `imsg status` | Check iMessage channel health |
| `imsg chats [--limit N] [--json]` | List recent chats |
| `imsg history --chat-id ID [--limit N] [--json]` | Get message history |
| `imsg send --to HANDLE --text MSG [--file PATH]` | Send a message |
| `imsg watch [--json]` | Stream incoming messages |

---

## Troubleshooting

### "Unknown method: channel.status" (or channel.send, etc.)

The daemon binary is outdated -- it was built before iMessage channel support
was added. Rebuild and reinstall:
```bash
cargo build --release
sudo cp target/release/carapace-daemon /usr/local/bin/carapace-daemon
sudo pkill -f carapace-daemon
sudo -u carapace /usr/local/bin/carapace-daemon
```

### "Connection refused" / "No such file"

The daemon isn't running or the socket doesn't exist:
```bash
ls -la /var/run/carapace/gateway.sock
sudo launchctl list | grep carapace
```

### "Permission denied" on socket

You're not in the `carapace-clients` group, or haven't logged out/in since
being added:
```bash
groups | grep carapace-clients
```

### "audit log write failed: Permission denied"

The `audit_log_path` in the config points somewhere the carapace user can't
write. Common cause: the config still has `/tmp/` paths from dev testing.
Fix the config to use `/Users/carapace/.local/share/carapace/audit.log`:
```bash
sudo cat /Users/carapace/.config/carapace/config.toml | grep audit
# If it shows /tmp/..., fix it:
sudo sed -i '' 's|/tmp/carapace-audit.log|/Users/carapace/.local/share/carapace/audit.log|' \
    /Users/carapace/.config/carapace/config.toml
```

### imsg crashes with "bundle not found"

The resource bundles (PhoneNumberKit, SQLite) aren't alongside the binary.
Re-download from GitHub releases and copy the `*.bundle` directories into the
same directory as the imsg binary.

### Daemon exits immediately (LaunchDaemon)

Check stderr log and verify the plist references valid user/group:
```bash
sudo cat /Users/carapace/.local/share/carapace/daemon.err
sudo launchctl print system/ai.carapace.gateway
```

Common causes:
- `GroupName` in plist references a group that doesn't exist
- Config file missing or unreadable by carapace user
- Socket directory doesn't exist or wrong permissions

### Socket already in use

The daemon cleans stale sockets on startup, but if needed:
```bash
sudo rm /var/run/carapace/gateway.sock
```

### macOS Firewall / SIP

Unix domain sockets are purely local. No firewall or SIP exemptions needed.

---

## Automated Install

The `install.sh` script automates all of the above:

```bash
./install.sh              # Run all phases
./install.sh --phase 5    # Resume from a specific phase
./install.sh --check      # Verify current installation
./install.sh --quick-test # Quick same-user smoke test
```

See [README.md](README.md) for the project overview and architecture diagram.
