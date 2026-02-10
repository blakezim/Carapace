#!/usr/bin/env bash
#
# Carapace – Install & Setup Script
#
# This script handles the full installation of Carapace:
#   1. Install dependencies (Rust, Homebrew, imsg)
#   2. Create the carapace macOS user
#   3. Set up cross-user permissions (group, socket directory)
#   4. Build the Rust code
#   5. Install binaries
#   6. Create configuration and LaunchDaemon
#   7. Run the end-to-end test
#
# Usage:
#   ./install.sh              # Run all phases interactively
#   ./install.sh --phase 3    # Resume from a specific phase
#   ./install.sh --check      # Just verify the current setup
#
# The script is safe to re-run. It checks for existing state before
# making changes.

set -euo pipefail

# ── Colours & formatting ───────────────────────────────────────────────────

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
BOLD='\033[1m'
DIM='\033[2m'
RESET='\033[0m'

CARAPACE_USER="carapace"
CARAPACE_GROUP="carapace-clients"
SOCKET_DIR="/var/run/carapace"
SOCKET_PATH="$SOCKET_DIR/gateway.sock"
SHIM_DIR="/usr/local/carapace/bin"
DAEMON_INSTALL_PATH="/usr/local/bin/carapace-daemon"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# ── Helper functions ───────────────────────────────────────────────────────

info()    { echo -e "${BLUE}▸${RESET} $*"; }
success() { echo -e "${GREEN}✓${RESET} $*"; }
warn()    { echo -e "${YELLOW}⚠${RESET} $*"; }
fail()    { echo -e "${RED}✗${RESET} $*"; }
header()  { echo -e "\n${BOLD}${CYAN}═══ $* ═══${RESET}\n"; }

prompt_continue() {
    echo ""
    read -rp "$(echo -e "${YELLOW}Press Enter to continue (or Ctrl-C to abort)...${RESET}")" _
    echo ""
}

prompt_yn() {
    local prompt="$1"
    local default="${2:-y}"
    local yn
    if [[ "$default" == "y" ]]; then
        read -rp "$(echo -e "${YELLOW}${prompt} [Y/n]:${RESET} ")" yn
        yn="${yn:-y}"
    else
        read -rp "$(echo -e "${YELLOW}${prompt} [y/N]:${RESET} ")" yn
        yn="${yn:-n}"
    fi
    [[ "$yn" == [yY] ]]
}

check_cmd() {
    command -v "$1" &>/dev/null
}

# ── Phase: Check prerequisites ─────────────────────────────────────────────

check_prerequisites() {
    header "Checking prerequisites"

    # Must be on macOS
    if [[ "$(uname)" != "Darwin" ]]; then
        fail "Carapace requires macOS (detected: $(uname))"
        exit 1
    fi
    success "Running on macOS $(sw_vers -productVersion)"

    # Must not be root
    if [[ "$(id -u)" -eq 0 ]]; then
        fail "Don't run this script as root. Run as your normal user (sudo will be used where needed)."
        exit 1
    fi
    success "Running as $(whoami) (not root)"

    # Must have sudo access
    if ! sudo -n true 2>/dev/null; then
        info "This script needs sudo access for some steps."
        sudo -v || { fail "Could not get sudo access"; exit 1; }
    fi
    success "sudo access available"

    echo ""
}

# ── Phase 1: Install dependencies ──────────────────────────────────────────

install_dependencies() {
    header "Phase 1: Installing Dependencies"

    # ── Homebrew ──
    if check_cmd brew; then
        success "Homebrew already installed ($(brew --version | head -1))"
    else
        info "Installing Homebrew..."
        /bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"

        # Add to PATH (Apple Silicon: /opt/homebrew, Intel: /usr/local)
        if [[ -f /opt/homebrew/bin/brew ]]; then
            eval "$(/opt/homebrew/bin/brew shellenv)"
        elif [[ -f /usr/local/bin/brew ]]; then
            eval "$(/usr/local/bin/brew shellenv)"
        fi
        success "Homebrew installed"
    fi

    # ── Rust ──
    if check_cmd rustc; then
        success "Rust already installed ($(rustc --version))"
    else
        info "Installing Rust via rustup..."
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
        source "$HOME/.cargo/env"
        success "Rust installed ($(rustc --version))"
    fi

    # Ensure cargo is on PATH
    if ! check_cmd cargo; then
        if [[ -f "$HOME/.cargo/env" ]]; then
            source "$HOME/.cargo/env"
        fi
    fi

    if ! check_cmd cargo; then
        fail "cargo not found on PATH after install. Try: source ~/.cargo/env"
        exit 1
    fi
    success "cargo available ($(cargo --version))"

    echo ""
}

# ── Phase 2: Create carapace user ──────────────────────────────────────────

create_carapace_user() {
    header "Phase 2: Create Carapace User"

    # Check if user already exists
    if id "$CARAPACE_USER" &>/dev/null; then
        success "User '$CARAPACE_USER' already exists (uid: $(id -u $CARAPACE_USER))"
    else
        info "Creating macOS user '$CARAPACE_USER'..."
        echo ""
        echo -e "  ${DIM}You'll be prompted for a password for the carapace account.${RESET}"
        echo -e "  ${DIM}Choose something strong – you'll rarely need to type it.${RESET}"
        echo ""

        sudo sysadminctl -addUser "$CARAPACE_USER" \
            -fullName "Carapace Gateway" \
            -password - \
            -home "/Users/$CARAPACE_USER"

        if id "$CARAPACE_USER" &>/dev/null; then
            success "User '$CARAPACE_USER' created (uid: $(id -u $CARAPACE_USER))"
        else
            fail "Failed to create user '$CARAPACE_USER'"
            exit 1
        fi
    fi

    # Hide from login screen (optional, skip if already hidden)
    local hidden_list
    hidden_list="$(sudo defaults read /Library/Preferences/com.apple.loginwindow HiddenUsersList 2>/dev/null || echo "")"
    if echo "$hidden_list" | grep -q "$CARAPACE_USER"; then
        success "User already hidden from login screen"
    elif prompt_yn "Hide carapace user from the login screen?" "y"; then
        sudo defaults write /Library/Preferences/com.apple.loginwindow \
            HiddenUsersList -array-add "$CARAPACE_USER" 2>/dev/null || true
        success "User hidden from login screen"
    fi

    echo ""
    echo -e "${BOLD}Manual steps required:${RESET}"
    echo ""
    echo "  Before continuing, you need to log in as the carapace user and"
    echo "  set up iCloud + iMessage. This can't be automated."
    echo ""
    echo "  1. Open System Settings → Users & Groups"
    echo "  2. Click on 'Carapace Gateway' and log in (or use fast user switching)"
    echo "  3. Sign into iCloud (System Settings → Apple ID)"
    echo "  4. Open Messages.app and verify it activates"
    echo "  5. Grant Full Disk Access to Terminal:"
    echo "     System Settings → Privacy & Security → Full Disk Access → add Terminal"
    echo "  6. Create config directories:"
    echo "     mkdir -p ~/.config/carapace ~/.local/share/carapace ~/.local/bin"
    echo "  7. Log out of the carapace account"
    echo ""

    prompt_continue

    # Install imsg into the carapace user's private bin directory.
    # Homebrew is owned by the main user, so we install there first
    # then copy the binary to a location only carapace can access.
    info "Setting up imsg for the carapace user..."

    local imsg_target="/Users/$CARAPACE_USER/.local/bin/imsg"

    if sudo -u "$CARAPACE_USER" test -f "$imsg_target" 2>/dev/null; then
        success "imsg already installed at $imsg_target"
    else
        # Make sure imsg is available via Homebrew on the main account
        if ! check_cmd imsg; then
            info "Installing imsg via Homebrew (on your account, to grab the binary)..."
            brew install steipete/tap/imsg
        fi

        local imsg_src
        imsg_src="$(which imsg 2>/dev/null || echo "$(brew --prefix 2>/dev/null || echo /usr/local)/bin/imsg")"

        if [[ -f "$imsg_src" ]]; then
            info "Copying imsg to $imsg_target (carapace-only access)..."
            sudo mkdir -p "/Users/$CARAPACE_USER/.local/bin"
            sudo cp "$imsg_src" "$imsg_target"
            sudo chown "$CARAPACE_USER" "$imsg_target"
            sudo chmod 700 "$imsg_target"
            success "imsg installed at $imsg_target (owned by carapace, mode 700)"

            if prompt_yn "Remove imsg from your main account? (recommended for isolation)" "y"; then
                brew uninstall imsg 2>/dev/null || true
                success "imsg removed from main account"
            fi
        else
            warn "Could not find imsg binary. Install it manually:"
            warn "  brew install steipete/tap/imsg"
            warn "  sudo cp \$(which imsg) $imsg_target"
            warn "  sudo chown carapace $imsg_target"
            warn "  sudo chmod 700 $imsg_target"
            echo ""
        fi
    fi
}

# ── Phase 3: Set up cross-user permissions ─────────────────────────────────

setup_permissions() {
    header "Phase 3: Cross-User Permissions"

    # ── Create group ──
    if dseditgroup -o read "$CARAPACE_GROUP" &>/dev/null; then
        success "Group '$CARAPACE_GROUP' already exists"
    else
        info "Creating group '$CARAPACE_GROUP'..."
        if ! sudo dseditgroup -o create "$CARAPACE_GROUP"; then
            fail "Failed to create group '$CARAPACE_GROUP'"
            exit 1
        fi
        success "Group '$CARAPACE_GROUP' created"
    fi

    # ── Add carapace user to group ──
    if dseditgroup -o checkmember -m "$CARAPACE_USER" "$CARAPACE_GROUP" &>/dev/null; then
        success "'$CARAPACE_USER' is already in '$CARAPACE_GROUP'"
    else
        info "Adding '$CARAPACE_USER' to group..."
        if ! sudo dseditgroup -o edit -a "$CARAPACE_USER" -t user "$CARAPACE_GROUP"; then
            fail "Failed to add '$CARAPACE_USER' to '$CARAPACE_GROUP'"
            exit 1
        fi
        success "Added '$CARAPACE_USER' to '$CARAPACE_GROUP'"
    fi

    # ── Add current user to group ──
    local me
    me="$(whoami)"
    if dseditgroup -o checkmember -m "$me" "$CARAPACE_GROUP" &>/dev/null; then
        success "You ($me) are already in '$CARAPACE_GROUP'"
    else
        info "Adding you ($me) to '$CARAPACE_GROUP'..."
        if ! sudo dseditgroup -o edit -a "$me" -t user "$CARAPACE_GROUP"; then
            fail "Failed to add '$me' to '$CARAPACE_GROUP'"
            exit 1
        fi
        success "Added '$me' to '$CARAPACE_GROUP'"
        echo ""
        warn "You MUST log out and log back in for the group change to take effect!"
        warn "The daemon connection will fail until you do."
        echo ""
    fi

    # ── Socket directory ──
    if [[ -d "$SOCKET_DIR" ]]; then
        success "Socket directory $SOCKET_DIR exists"
    else
        info "Creating socket directory..."
        sudo mkdir -p "$SOCKET_DIR"
        success "Created $SOCKET_DIR"
    fi

    info "Setting socket directory ownership and permissions..."
    sudo chown "${CARAPACE_USER}:${CARAPACE_GROUP}" "$SOCKET_DIR"
    sudo chmod 750 "$SOCKET_DIR"
    success "Socket directory: $(ls -ld "$SOCKET_DIR")"

    echo ""
}

# ── Phase 4: Build the Rust code ───────────────────────────────────────────

build_code() {
    header "Phase 4: Building Carapace"

    cd "$SCRIPT_DIR"

    if [[ ! -f "Cargo.toml" ]]; then
        fail "Cargo.toml not found in $SCRIPT_DIR"
        fail "Run this script from the Carapace project root."
        exit 1
    fi

    info "Building in release mode (this may take a minute on first run)..."
    if ! cargo build --release; then
        fail "Build failed – see errors above"
        exit 1
    fi

    # Verify binaries exist
    local daemon_bin="$SCRIPT_DIR/target/release/carapace-daemon"
    local shim_bin="$SCRIPT_DIR/target/release/test-shim"

    if [[ -f "$daemon_bin" ]]; then
        success "Built: carapace-daemon ($(du -h "$daemon_bin" | cut -f1))"
    else
        fail "carapace-daemon binary not found at $daemon_bin"
        exit 1
    fi

    if [[ -f "$shim_bin" ]]; then
        success "Built: test-shim ($(du -h "$shim_bin" | cut -f1))"
    else
        fail "test-shim binary not found at $shim_bin"
        exit 1
    fi

    echo ""
}

# ── Phase 5: Install binaries ──────────────────────────────────────────────

install_binaries() {
    header "Phase 5: Installing Binaries"

    cd "$SCRIPT_DIR"

    # ── Daemon ──
    info "Installing carapace-daemon to $DAEMON_INSTALL_PATH..."
    sudo cp "target/release/carapace-daemon" "$DAEMON_INSTALL_PATH"
    sudo chmod 755 "$DAEMON_INSTALL_PATH"
    success "Daemon installed: $DAEMON_INSTALL_PATH"

    # ── Test shim ──
    info "Installing test-shim to $SHIM_DIR..."
    sudo mkdir -p "$SHIM_DIR"
    sudo cp "target/release/test-shim" "$SHIM_DIR/test-shim"
    sudo chmod 755 "$SHIM_DIR/test-shim"
    success "Test shim installed: $SHIM_DIR/test-shim"

    echo ""
}

# ── Phase 6: Create configuration & LaunchDaemon ──────────────────────────

setup_config() {
    header "Phase 6: Configuration & LaunchDaemon"

    local config_dir="/Users/$CARAPACE_USER/.config/carapace"
    local data_dir="/Users/$CARAPACE_USER/.local/share/carapace"
    local config_file="$config_dir/config.toml"
    local plist_path="/Library/LaunchDaemons/ai.carapace.gateway.plist"

    # ── Config directories ──
    info "Ensuring config directories exist..."
    sudo -u "$CARAPACE_USER" mkdir -p "$config_dir" 2>/dev/null || \
        sudo mkdir -p "$config_dir" && sudo chown "$CARAPACE_USER" "$config_dir"
    sudo -u "$CARAPACE_USER" mkdir -p "$data_dir" 2>/dev/null || \
        sudo mkdir -p "$data_dir" && sudo chown "$CARAPACE_USER" "$data_dir"
    sudo -u "$CARAPACE_USER" mkdir -p "$data_dir/dead_letters" 2>/dev/null || \
        sudo mkdir -p "$data_dir/dead_letters" && sudo chown "$CARAPACE_USER" "$data_dir/dead_letters"
    success "Config directories ready"

    # ── Config file ──
    if sudo -u "$CARAPACE_USER" test -f "$config_file" 2>/dev/null; then
        success "Config file already exists at $config_file"
        if prompt_yn "Overwrite with default config?" "n"; then
            write_default_config "$config_file"
        fi
    else
        info "Creating default config..."
        write_default_config "$config_file"
    fi

    # ── LaunchDaemon ──
    if [[ -f "$plist_path" ]]; then
        success "LaunchDaemon plist already exists"
        if prompt_yn "Overwrite it?" "n"; then
            write_launch_daemon "$plist_path"
        fi
    else
        info "Creating LaunchDaemon..."
        write_launch_daemon "$plist_path"
    fi

    echo ""
    echo -e "  ${DIM}The LaunchDaemon will start the daemon at boot as the carapace user.${RESET}"
    echo -e "  ${DIM}No need to log in as carapace or use fast user switching.${RESET}"
    echo ""
}

write_default_config() {
    local config_file="$1"
    sudo tee "$config_file" > /dev/null << 'CONFIGEOF'
# Carapace Gateway Configuration
# Edit this file to configure channels, allowlists, and security.

[gateway]
socket_path = "/var/run/carapace/gateway.sock"
log_level = "info"                # trace | debug | info | warn | error
request_timeout = 30              # seconds

[security]
audit_log_path = "/Users/carapace/.local/share/carapace/audit.log"
dead_letter_path = "/Users/carapace/.local/share/carapace/dead_letters"
audit_enabled = true

[security.rate_limit]
# { requests = N, per_seconds = N } — per channel
imsg    = { requests = 30, per_seconds = 60 }
signal  = { requests = 20, per_seconds = 60 }
discord = { requests = 60, per_seconds = 60 }
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
pattern = '(?i)secret.*token'
action  = "block"

[[security.content_filter.patterns]]
pattern = '\b\d{3}-\d{2}-\d{4}\b'
action  = "block"                 # SSN pattern


# ── iMessage ───────────────────────────────────────────────────────────────
[channels.imsg]
enabled     = true
real_binary = "/Users/carapace/.local/bin/imsg"
db_path     = "/Users/carapace/Library/Messages/chat.db"

[channels.imsg.outbound]
mode      = "allowlist"           # allowlist | denylist | open
allowlist = [
    # "+14155551234",
    # "email:friend@icloud.com",
]

[channels.imsg.inbound]
mode      = "allowlist"
allowlist = [
    # "+14155551234",
]


# ── Signal (uncomment to enable) ──────────────────────────────────────────
# [channels.signal]
# enabled         = true
# signal_cli_path = "/usr/local/bin/signal-cli"
# account         = "+1YOURPHONENUMBER"
#
# [channels.signal.outbound]
# mode      = "allowlist"
# allowlist = []
#
# [channels.signal.inbound]
# mode      = "allowlist"
# allowlist = []


# ── Discord (uncomment to enable) ─────────────────────────────────────────
# [channels.discord]
# enabled    = true
# token_file = "/Users/carapace/.config/carapace/discord_token"
#
# [channels.discord.outbound]
# mode      = "allowlist"
# allowlist = []


# ── Gmail (uncomment to enable) ───────────────────────────────────────────
# [channels.gmail]
# enabled          = true
# credentials_path = "/Users/carapace/.config/gog"
#
# [channels.gmail.outbound]
# mode      = "allowlist"
# allowlist = []
CONFIGEOF

    sudo chown "$CARAPACE_USER" "$config_file"
    sudo chmod 600 "$config_file"
    success "Config written to $config_file"
}

write_launch_daemon() {
    local plist_path="$1"
    sudo tee "$plist_path" > /dev/null << 'PLISTEOF'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN"
  "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>ai.carapace.gateway</string>

    <key>ProgramArguments</key>
    <array>
        <string>/usr/local/bin/carapace-daemon</string>
    </array>

    <key>UserName</key>
    <string>carapace</string>

    <key>GroupName</key>
    <string>carapace-clients</string>

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
        <key>RUST_LOG</key>
        <string>info</string>
    </dict>
</dict>
</plist>
PLISTEOF

    sudo chown root:wheel "$plist_path"
    sudo chmod 644 "$plist_path"
    success "LaunchDaemon written to $plist_path"
}

# ── Phase 7: Start daemon & test ───────────────────────────────────────────

start_and_test() {
    header "Phase 7: Start Daemon & Run Tests"

    local plist_path="/Library/LaunchDaemons/ai.carapace.gateway.plist"
    local data_dir="/Users/$CARAPACE_USER/.local/share/carapace"
    local log_file="$data_dir/daemon.log"
    local err_file="$data_dir/daemon.err"

    # ── Pre-flight: validate plist references ──
    info "Validating LaunchDaemon plist..."

    local plist_user plist_group
    plist_user="$(/usr/libexec/PlistBuddy -c "Print :UserName" "$plist_path" 2>/dev/null || echo "")"
    plist_group="$(/usr/libexec/PlistBuddy -c "Print :GroupName" "$plist_path" 2>/dev/null || echo "")"

    if [[ -n "$plist_user" ]] && ! id "$plist_user" &>/dev/null; then
        fail "Plist UserName '$plist_user' does not exist as a macOS user"
        fail "Fix the plist or re-run: $0 --phase 6"
        return
    fi

    if [[ -n "$plist_group" ]] && ! dseditgroup -o read "$plist_group" &>/dev/null; then
        fail "Plist GroupName '$plist_group' does not exist as a macOS group"
        fail "launchd will silently refuse to start the daemon with a non-existent group."
        fail "Fix the plist or re-run: $0 --phase 6"
        return
    fi

    local daemon_bin
    daemon_bin="$(/usr/libexec/PlistBuddy -c "Print :ProgramArguments:0" "$plist_path" 2>/dev/null || echo "")"
    if [[ -n "$daemon_bin" ]] && [[ ! -x "$daemon_bin" ]]; then
        fail "Daemon binary not found or not executable: $daemon_bin"
        fail "Re-run: $0 --phase 5"
        return
    fi

    success "Plist OK (user=$plist_user, group=$plist_group, bin=$daemon_bin)"

    # Check group membership first
    if ! groups "$(whoami)" | grep -q "$CARAPACE_GROUP"; then
        warn "You are not yet in the '$CARAPACE_GROUP' group in this session."
        warn "You probably need to log out and back in first."
        echo ""
        if ! prompt_yn "Try running the test anyway?" "n"; then
            echo ""
            echo -e "${BOLD}Next steps:${RESET}"
            echo "  1. Log out of macOS and log back in"
            echo "  2. Re-run: $0 --phase 7"
            return
        fi
    fi

    # Stop any existing daemon
    if sudo launchctl list | grep -q "ai.carapace.gateway"; then
        info "Stopping existing daemon..."
        sudo launchctl unload "$plist_path" 2>/dev/null || true
        sleep 1
    fi

    # Clean stale socket
    if [[ -e "$SOCKET_PATH" ]]; then
        info "Removing stale socket..."
        sudo rm -f "$SOCKET_PATH"
    fi

    # Truncate old logs so we only see output from this launch attempt
    sudo -u "$CARAPACE_USER" truncate -s 0 "$log_file" 2>/dev/null || true
    sudo -u "$CARAPACE_USER" truncate -s 0 "$err_file" 2>/dev/null || true

    # Start daemon
    info "Starting daemon via LaunchDaemon..."
    sudo launchctl load "$plist_path"
    sleep 2

    # ── Verify daemon is actually alive (not just loaded) ──
    # launchctl list format: PID<tab>Status<tab>Label
    # A "-" PID means the process is not running. Non-zero status means it exited with an error.
    local lctl_line
    lctl_line="$(sudo launchctl list | grep "ai.carapace.gateway" || echo "")"

    if [[ -z "$lctl_line" ]]; then
        fail "Daemon is not loaded. launchctl load may have failed."
        echo "  Try: sudo launchctl load $plist_path"
        return
    fi

    local daemon_pid lctl_status
    daemon_pid="$(echo "$lctl_line" | awk '{print $1}')"
    lctl_status="$(echo "$lctl_line" | awk '{print $2}')"

    if [[ "$daemon_pid" == "-" ]]; then
        fail "Daemon loaded but process is not running (exit status: $lctl_status)"
        echo ""
        if [[ -s "$err_file" ]]; then
            echo -e "  ${DIM}── daemon stderr ──${RESET}"
            sudo -u "$CARAPACE_USER" tail -20 "$err_file" 2>/dev/null | sed 's/^/  /'
        elif [[ -s "$log_file" ]]; then
            echo -e "  ${DIM}── daemon stdout ──${RESET}"
            sudo -u "$CARAPACE_USER" tail -20 "$log_file" 2>/dev/null | sed 's/^/  /'
        else
            echo "  No log output found. launchd likely could not start the process at all."
            echo "  Common causes:"
            echo "    • GroupName in plist references a non-existent group"
            echo "    • UserName in plist references a non-existent user"
            echo "    • The daemon binary is missing or not executable"
            echo ""
            echo "  Debug with: sudo launchctl print system/ai.carapace.gateway"
        fi
        return
    fi

    success "Daemon is running (pid: $daemon_pid)"

    # Verify socket exists
    if [[ -S "$SOCKET_PATH" ]]; then
        success "Socket exists: $(ls -l "$SOCKET_PATH")"
    else
        fail "Socket not found at $SOCKET_PATH"
        echo ""
        if [[ -s "$err_file" ]]; then
            echo -e "  ${DIM}── daemon stderr ──${RESET}"
            sudo -u "$CARAPACE_USER" tail -20 "$err_file" 2>/dev/null | sed 's/^/  /'
        elif [[ -s "$log_file" ]]; then
            echo -e "  ${DIM}── daemon stdout ──${RESET}"
            sudo -u "$CARAPACE_USER" tail -20 "$log_file" 2>/dev/null | sed 's/^/  /'
        else
            echo "  No daemon log output found."
        fi
        return
    fi

    # Run the test shim
    echo ""
    info "Running test-shim..."
    echo ""
    "$SHIM_DIR/test-shim" || true

    echo ""
}

# ── Quick local test (no carapace user needed) ─────────────────────────────

quick_test() {
    header "Quick Local Test (same-user)"

    cd "$SCRIPT_DIR"

    local tmp_sock="/tmp/carapace-test-$$.sock"

    info "Starting daemon with temp socket: $tmp_sock"
    CARAPACE_SOCKET_PATH="$tmp_sock" cargo run --release -p carapace-daemon &
    local daemon_pid=$!
    sleep 2

    if kill -0 "$daemon_pid" 2>/dev/null; then
        success "Daemon running (pid: $daemon_pid)"
    else
        fail "Daemon failed to start"
        return
    fi

    echo ""
    info "Running test-shim..."
    echo ""
    CARAPACE_SOCKET_PATH="$tmp_sock" cargo run --release -p carapace-shims --bin test-shim || true

    echo ""
    info "Stopping daemon..."
    kill "$daemon_pid" 2>/dev/null || true
    wait "$daemon_pid" 2>/dev/null || true
    rm -f "$tmp_sock"
    success "Done"
}

# ── Status check ───────────────────────────────────────────────────────────

check_status() {
    header "Carapace Status Check"

    local all_ok=true

    # User
    if id "$CARAPACE_USER" &>/dev/null; then
        success "User '$CARAPACE_USER' exists (uid: $(id -u $CARAPACE_USER))"
    else
        fail "User '$CARAPACE_USER' does not exist"
        all_ok=false
    fi

    # Group
    if dseditgroup -o read "$CARAPACE_GROUP" &>/dev/null; then
        success "Group '$CARAPACE_GROUP' exists"
    else
        fail "Group '$CARAPACE_GROUP' does not exist"
        all_ok=false
    fi

    # Group membership (current user)
    if groups "$(whoami)" | grep -q "$CARAPACE_GROUP"; then
        success "You ($(whoami)) are in '$CARAPACE_GROUP'"
    else
        fail "You ($(whoami)) are NOT in '$CARAPACE_GROUP'"
        all_ok=false
    fi

    # Socket directory
    if [[ -d "$SOCKET_DIR" ]]; then
        success "Socket directory exists: $(ls -ld "$SOCKET_DIR")"
    else
        fail "Socket directory $SOCKET_DIR missing"
        all_ok=false
    fi

    # Daemon binary
    if [[ -x "$DAEMON_INSTALL_PATH" ]]; then
        success "Daemon installed at $DAEMON_INSTALL_PATH"
    else
        fail "Daemon not found at $DAEMON_INSTALL_PATH"
        all_ok=false
    fi

    # Test shim
    if [[ -x "$SHIM_DIR/test-shim" ]]; then
        success "Test shim installed at $SHIM_DIR/test-shim"
    else
        fail "Test shim not found at $SHIM_DIR/test-shim"
        all_ok=false
    fi

    # LaunchDaemon
    if [[ -f "/Library/LaunchDaemons/ai.carapace.gateway.plist" ]]; then
        success "LaunchDaemon plist exists"
    else
        fail "LaunchDaemon plist missing"
        all_ok=false
    fi

    # Daemon running
    if sudo launchctl list 2>/dev/null | grep -q "ai.carapace.gateway"; then
        success "Daemon is running"
    else
        warn "Daemon is NOT running"
    fi

    # Socket
    if [[ -S "$SOCKET_PATH" ]]; then
        success "Socket exists at $SOCKET_PATH"
    else
        warn "Socket does not exist (daemon may not be running)"
    fi

    # Rust toolchain
    if check_cmd cargo; then
        success "Rust toolchain: $(rustc --version)"
    else
        fail "Rust not installed"
        all_ok=false
    fi

    echo ""
    if $all_ok; then
        success "All checks passed!"
    else
        fail "Some checks failed – see above"
    fi
}

# ── Main ───────────────────────────────────────────────────────────────────

main() {
    echo ""
    echo -e "${BOLD}╔══════════════════════════════════════════════════╗${RESET}"
    echo -e "${BOLD}║          Carapace Installer v0.1.0               ║${RESET}"
    echo -e "${BOLD}║   Zero-Trust Security Gateway for OpenClaw       ║${RESET}"
    echo -e "${BOLD}╚══════════════════════════════════════════════════╝${RESET}"
    echo ""

    local start_phase=0

    # Parse arguments
    while [[ $# -gt 0 ]]; do
        case "$1" in
            --phase)
                start_phase="$2"
                shift 2
                ;;
            --check)
                check_status
                exit 0
                ;;
            --quick-test)
                check_prerequisites
                quick_test
                exit 0
                ;;
            --help|-h)
                echo "Usage: $0 [options]"
                echo ""
                echo "Options:"
                echo "  --phase N      Resume from phase N (1-7)"
                echo "  --check        Check current installation status"
                echo "  --quick-test   Run a quick same-user test (no setup needed)"
                echo "  --help         Show this help"
                echo ""
                echo "Phases:"
                echo "  1  Install dependencies (Rust, Homebrew)"
                echo "  2  Create carapace macOS user"
                echo "  3  Set up cross-user permissions"
                echo "  4  Build the Rust code"
                echo "  5  Install binaries"
                echo "  6  Create configuration & LaunchDaemon"
                echo "  7  Start daemon & run end-to-end test"
                exit 0
                ;;
            *)
                fail "Unknown option: $1"
                exit 1
                ;;
        esac
    done

    check_prerequisites

    [[ $start_phase -le 1 ]] && install_dependencies
    [[ $start_phase -le 2 ]] && create_carapace_user
    [[ $start_phase -le 3 ]] && setup_permissions
    [[ $start_phase -le 4 ]] && build_code
    [[ $start_phase -le 5 ]] && install_binaries
    [[ $start_phase -le 6 ]] && setup_config
    [[ $start_phase -le 7 ]] && start_and_test

    header "Installation Complete"

    echo -e "  ${GREEN}Carapace is installed and running.${RESET}"
    echo ""
    echo "  Next steps:"
    echo "    • Edit the allowlist: sudo -u carapace nano /Users/carapace/.config/carapace/config.toml"
    echo "    • View daemon logs:   sudo -u carapace tail -f /Users/carapace/.local/share/carapace/daemon.log"
    echo "    • Check status:       $0 --check"
    echo "    • Quick smoke test:   $0 --quick-test"
    echo ""
}

main "$@"
