# Carapace – Setup & First Test

This guide walks you through creating the `carapace` macOS user, configuring
permissions, building the Rust code, and running a full end-to-end test between
two user accounts.

---

## Prerequisites

- **macOS 13+** (Ventura or later recommended)
- **Rust toolchain** – install via [rustup.rs](https://rustup.rs) if you haven't already:
  ```bash
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
  ```
- **Admin access** on the Mac (you'll need `sudo` a few times)

---

## Phase 1: Create the Carapace User

This is the dedicated macOS user that will hold your messaging credentials,
isolated from the AI runtime.

### Option A: System Settings (GUI)

1. Open **System Settings → Users & Groups**
2. Click the **+** button (unlock if needed)
3. Fill in:
   - **Name**: `Carapace`
   - **Account Name**: `carapace`
   - **Password**: choose something strong (you'll rarely type it)
4. Set account type to **Standard** (not Admin)
5. Click **Create User**

### Option B: Command Line

```bash
# Create the user (you'll be prompted for the password)
sudo sysadminctl -addUser carapace \
    -fullName "Carapace" \
    -password - \
    -home /Users/carapace
```

### Post-Creation

1. **Log in** as `carapace` (fast user switching or log out/log in)
2. **Sign into iCloud** in System Settings → Apple ID (this enables iMessage)
3. Open **Messages.app** and verify it activates
4. Open **Terminal** and grant it **Full Disk Access**:
   - System Settings → Privacy & Security → Full Disk Access → add Terminal
5. Create config directories:
   ```bash
   mkdir -p ~/.config/carapace
   mkdir -p ~/.local/share/carapace
   mkdir -p ~/.local/bin
   ```
6. **Log out** of the carapace account

### Install imsg (From Your Main Account)

Homebrew is owned by your main user, so install `imsg` there and copy the
binary into the carapace user's private bin. This way only the carapace
user can execute it — the AI (running as your user) physically cannot.

```bash
# Install to get the binary
brew install steipete/tap/imsg

# Copy into carapace's home, locked to carapace-only
# (uses `which imsg` so it works on both Intel and Apple Silicon Macs)
sudo cp "$(which imsg)" /Users/carapace/.local/bin/imsg
sudo chown carapace /Users/carapace/.local/bin/imsg
sudo chmod 700 /Users/carapace/.local/bin/imsg

# Remove from your main account (recommended)
brew uninstall imsg
```

Verify it's locked down:
```bash
# This should fail with "permission denied" — that's correct!
/Users/carapace/.local/bin/imsg --help

# This should work:
sudo -u carapace /Users/carapace/.local/bin/imsg --help
```

### Hide from Login Screen (Optional)

Back in your main account:
```bash
sudo defaults write /Library/Preferences/com.apple.loginwindow HiddenUsersList -array-add carapace
```

---

## Phase 2: Configure Your Main Account

These commands set up the cross-user communication channel.

### Create the Shared Group

```bash
# Create the carapace-clients group
sudo dseditgroup -o create carapace-clients

# Add yourself to the group
sudo dseditgroup -o edit -a $(whoami) -t user carapace-clients

# Verify
dseditgroup -o checkmember -m $(whoami) carapace-clients
# Should say: "yes <your_username> is a member of carapace-clients"
```

### Create the Socket Directory

```bash
sudo mkdir -p /var/run/carapace
sudo chown carapace:carapace-clients /var/run/carapace
sudo chmod 750 /var/run/carapace
```

### ⚠️ Log Out and Back In

Group membership changes don't take effect until you log out of your
macOS session and log back in. This is required!

Verify after logging back in:
```bash
groups | tr ' ' '\n' | grep carapace-clients
# Should print: carapace-clients
```

---

## Phase 3: Build the Code

From the Carapace project root:

```bash
# Build everything
cargo build

# Or in release mode
cargo build --release
```

This produces two binaries:
- `target/debug/carapace-daemon` – the gateway server
- `target/debug/test-shim` – the test client

---

## Phase 4: Run the End-to-End Test

### Quick Local Test (Same User)

For a quick smoke test without full user separation:

```bash
# Terminal 1 – Start the daemon with a temp socket
CARAPACE_SOCKET_PATH=/tmp/carapace-test.sock cargo run --bin carapace-daemon

# Terminal 2 – Run the test shim
CARAPACE_SOCKET_PATH=/tmp/carapace-test.sock cargo run --bin test-shim
```

You should see all 5 tests pass. The `whoami` test will note that the daemon
user matches yours (since you haven't done cross-user yet).

### Full Cross-User Test (The Real Deal)

This is where it gets interesting – proving OS-level isolation.

**Terminal 1** – Start the daemon as the `carapace` user:
```bash
# Copy the daemon binary to a shared location
sudo cp target/debug/carapace-daemon /usr/local/bin/

# Run as carapace
sudo -u carapace /usr/local/bin/carapace-daemon
```

**Terminal 2** – Run the test shim as your normal user:
```bash
cargo run --bin test-shim
```

Expected output:
```
╔══════════════════════════════════════════════╗
║     Carapace Gateway – Test Shim v0.1.0     ║
╚══════════════════════════════════════════════╝

Connecting to daemon... OK ✓
1. ping .......... PASS ✓  (pong: true)
2. echo .......... PASS ✓  (echoed correctly)
3. whoami ........ PASS ✓  (user: carapace, uid: 502)
         ↳ Isolation verified! Daemon runs as "carapace", you are "blake"
4. execute ....... PASS ✓  (exit: 0, stdout: "cross-user execution works")
5. error case .... PASS ✓  (code: -32601, msg: "Unknown method: nonexistent.method")

─────────────────────────────────────────────
Results: 5 passed, 0 failed, 5 total
All tests passed! The gateway is working.
```

The critical line is **test 3 (whoami)**: it proves that the daemon is executing
as the `carapace` user while you're calling it from your own account. The OS
boundary is real.

### Manual Testing with netcat

You can also test the daemon manually:

```bash
# Send a ping
echo '{"jsonrpc":"2.0","id":1,"method":"ping","params":{}}' \
  | nc -U /var/run/carapace/gateway.sock

# Ask who the daemon is running as
echo '{"jsonrpc":"2.0","id":2,"method":"whoami","params":{}}' \
  | nc -U /var/run/carapace/gateway.sock

# Execute a command as carapace
echo '{"jsonrpc":"2.0","id":3,"method":"execute","params":{"command":"whoami"}}' \
  | nc -U /var/run/carapace/gateway.sock

# Test error handling
echo '{"jsonrpc":"2.0","id":4,"method":"bad.method","params":{}}' \
  | nc -U /var/run/carapace/gateway.sock
```

---

## Troubleshooting

### "Connection refused" / "No such file"
The daemon isn't running or the socket doesn't exist:
```bash
ls -la /var/run/carapace/gateway.sock
sudo -u carapace /usr/local/bin/carapace-daemon
```

### "Permission denied"
You're not in the `carapace-clients` group, or you haven't logged out/in:
```bash
groups | grep carapace-clients    # should appear
ls -la /var/run/carapace/         # should show carapace:carapace-clients
```

### Socket Already In Use
A previous daemon left a stale socket. The daemon cleans this up automatically
on start, but you can also remove it manually:
```bash
sudo rm /var/run/carapace/gateway.sock
```

### macOS Firewall / SIP
Unix domain sockets are purely local and aren't affected by the macOS
firewall or System Integrity Protection. No special exemptions needed.

---

## What's Next

With the gateway working, the next phases will add:
- **Phase 4**: Security middleware (rate limiting, allowlists, content filtering)
- **Phase 5**: iMessage adapter (real `imsg` passthrough)
- **Phase 6**: Additional channels (Signal, Discord, Gmail)

See [docs/12-roadmap.md](docs/12-roadmap.md) for the full plan.
