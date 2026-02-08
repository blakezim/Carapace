# Security Model

## Threat Model

### Adversary Capabilities

We assume an adversary who can:

1. **Send messages to the AI** (within OpenClaw's DM policy)
2. **Craft sophisticated prompt injections**
3. **Exploit any vulnerability in OpenClaw or its dependencies**
4. **Run arbitrary code as your user** (worst case)

We do NOT assume an adversary who can:

- Gain root/admin access to your Mac
- Physically access your machine
- Compromise the carapace user account directly

### Assets to Protect

| Asset | Value | Without Carapace | With Carapace |
|-------|-------|------------------|---------------|
| Message history | High | AI can read | Cannot access |
| iCloud credentials | Critical | In same keychain | Different keychain |
| Contact list | Medium | AI can enumerate | Filtered view only |
| API tokens (Discord, etc.) | High | In config files | Different user's files |
| Allowlist configuration | Medium | AI can modify | Cannot access |
| Audit logs | Medium | AI can delete | Cannot access |

## Attack Analysis

### Attack 1: Direct Database Access

**Without Carapace:**
```bash
# AI could be tricked into running:
sqlite3 ~/Library/Messages/chat.db "SELECT * FROM message LIMIT 100"
# Result: Full message history leaked
```

**With Carapace:**
```bash
# Same command runs as your user
sqlite3 ~/Library/Messages/chat.db "SELECT * FROM message LIMIT 100"
# Result: Error - database doesn't exist (it's in /Users/carapace/)

# Even if AI knows the path:
sqlite3 /Users/carapace/Library/Messages/chat.db "SELECT * FROM message"
# Result: Permission denied (different user's home directory)
```

### Attack 2: Credential Exfiltration

**Without Carapace:**
```bash
# AI reads Discord token from config
cat ~/.openclaw/credentials/discord-token
# Result: Token leaked

# AI accesses keychain
security find-generic-password -a "iCloud" -w
# Result: Credentials leaked
```

**With Carapace:**
```bash
# Config doesn't exist in your home
cat ~/.openclaw/credentials/discord-token
# Result: File not found (it's in /Users/carapace/)

# Keychain is user-specific
security find-generic-password -a "iCloud" -w
# Result: Item not found (iCloud is logged in as carapace user)
```

### Attack 3: Allowlist Bypass

**Without Carapace:**
```bash
# AI modifies its own allowlist
echo '["*"]' > ~/.openclaw/credentials/imessage-allowFrom.json
# Result: All recipients now allowed
```

**With Carapace:**
```bash
# Allowlist is in carapace's home
echo '["*"]' > /Users/carapace/.config/carapace/config.toml
# Result: Permission denied

# Even if AI finds the socket:
# It can only communicate via the protocol
# No way to modify server-side configuration
```

### Attack 4: Audit Log Tampering

**Without Carapace:**
```bash
# AI deletes evidence
rm -rf ~/.openclaw/agents/*/sessions/*.jsonl
# Result: Logs deleted, no trace
```

**With Carapace:**
```bash
# Logs are in carapace's home
rm -rf /Users/carapace/.local/share/carapace/audit.log
# Result: Permission denied

# Logs remain intact and tamper-proof
```

### Attack 5: Direct Tool Execution

**Without Carapace:**
```bash
# AI bypasses OpenClaw's tool policies
/usr/local/bin/imsg send "+1234567890" "spam"
# Result: Message sent, no filtering
```

**With Carapace:**
```bash
# Real imsg is in carapace's PATH, not yours
/usr/local/bin/imsg send "+1234567890" "spam"
# Result: Either command not found, or...

# If real imsg is somehow accessible:
# It can't access the database (wrong user)
# It can't access iCloud (wrong keychain)
# The command fails
```

### Attack 6: Socket Hijacking

**Attempted Attack:**
```bash
# AI tries to create fake socket
rm /var/run/carapace/gateway.sock
# Create malicious socket that logs credentials
```

**Protection:**
```bash
# Socket directory is owned by carapace
ls -la /var/run/carapace/
# drwxr-x--- carapace carapace-clients

# Your user cannot delete files in this directory
rm /var/run/carapace/gateway.sock
# Result: Permission denied
```

## Security Layers

### Layer 1: Unix File Permissions

The foundational security layer. Enforced by the kernel.

```
/Users/carapace/                 # 700 - carapace only
├── .config/carapace/            # 700 - carapace only
│   └── config.toml              # 600 - carapace only
├── .local/share/carapace/       # 700 - carapace only
│   ├── audit.log                # 600 - carapace only
│   └── dead_letters/            # 700 - carapace only
└── Library/Messages/chat.db     # 600 - carapace only

/var/run/carapace/               # 750 - carapace:carapace-clients
└── gateway.sock                 # 770 - carapace:carapace-clients
```

### Layer 2: Group-Based Socket Access

Only members of `carapace-clients` can connect to the socket.

```bash
# Check group membership
groups yourusername
# yourusername staff carapace-clients

# Non-members cannot connect
sudo -u randomuser nc -U /var/run/carapace/gateway.sock
# Result: Permission denied
```

### Layer 3: Protocol-Level Restrictions

Even with socket access, clients can only:

- Send well-formed JSON-RPC requests
- Use defined methods (send, receive, list_chats, etc.)
- Operate within rate limits
- Access only allowlisted recipients

There is no protocol method to:
- Read configuration
- Modify allowlists
- Access raw database
- Retrieve credentials

### Layer 4: Content Filtering

Messages are scanned for sensitive patterns before sending:

```toml
[security.content_filter]
patterns = [
    { pattern = "(?i)password\\s*[:=]", action = "block" },
    { pattern = "(?i)api[_-]?key", action = "block" },
    { pattern = "\\b\\d{3}-\\d{2}-\\d{4}\\b", action = "block" },  # SSN
    { pattern = "(?i)secret.*token", action = "block" },
]
```

### Layer 5: Rate Limiting

Prevents abuse even with valid access:

```toml
[security.rate_limit]
imsg = { requests = 30, per_seconds = 60 }
```

Rate limiter counts ALL attempts (including blocked ones) to prevent probing.

### Layer 6: Audit Logging

Every operation is logged with:
- Timestamp
- Channel
- Action (send, receive, list, etc.)
- Target (recipient or sender)
- Result (allowed, blocked, error)
- Reason (if blocked)

Logs are append-only and owned by carapace user.

## Comparison: OpenClaw Native vs Carapace

| Security Control | OpenClaw | Carapace | Enforcement |
|-----------------|----------|----------|-------------|
| DM Pairing | ✅ | ✅ | Software |
| Tool Policies | ✅ | ✅ | Software |
| Docker Sandbox | ✅ | N/A | Container |
| Rate Limiting | ✅ | ✅ | Software |
| Content Filtering | ⚠️ Limited | ✅ | Software |
| Credential Isolation | ❌ | ✅ | **OS Kernel** |
| Database Isolation | ❌ | ✅ | **OS Kernel** |
| Allowlist Immutability | ❌ | ✅ | **OS Kernel** |
| Audit Log Integrity | ❌ | ✅ | **OS Kernel** |

The key difference: Carapace's critical controls are **kernel-enforced**, not software-enforced.

## Residual Risks

Even with Carapace, some risks remain:

### 1. Authorized Abuse

If a message passes all filters (allowlisted recipient, under rate limit, no blocked content), it will be sent. Carapace doesn't evaluate message "appropriateness."

**Mitigation:** Careful allowlist curation, content filter patterns.

### 2. Carapace Account Compromise

If an attacker gains access to the carapace user account directly, all protections are bypassed.

**Mitigation:** Strong password, no remote login, hidden from login screen.

### 3. Kernel Vulnerabilities

A kernel exploit could bypass file permissions.

**Mitigation:** Keep macOS updated, this is an OS-level concern.

### 4. Physical Access

Someone with physical access could log in as carapace.

**Mitigation:** FileVault encryption, strong password.

### 5. Social Engineering

The AI could try to convince you to modify allowlists or add recipients.

**Mitigation:** Awareness, don't take configuration advice from the AI.

## Security Checklist

Before deploying Carapace:

- [ ] Carapace user has strong password
- [ ] Carapace user hidden from login screen
- [ ] Socket directory permissions correct (750)
- [ ] Socket permissions correct (770)
- [ ] Allowlists are minimal (only necessary contacts)
- [ ] Rate limits are reasonable
- [ ] Content filters include your sensitive patterns
- [ ] Audit logging is enabled
- [ ] You're in the carapace-clients group
- [ ] Real tools are NOT in your PATH

## Next Steps

- [Setup: Carapace User](04-setup-carapace-user.md) - Configure the secure side
- [Setup: Your Account](05-setup-main-account.md) - Configure your side
- [Configuration Reference](10-configuration-reference.md) - All config options
