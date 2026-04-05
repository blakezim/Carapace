# Overview & Motivation

## The Problem

OpenClaw is a powerful open-source AI assistant that can connect to messaging platforms like iMessage, Signal, Discord, and Gmail. This creates a fundamental security challenge:

**The AI that processes untrusted input (messages from the world) has direct access to sensitive credentials and communication tools.**

This is like giving a stranger the keys to your house and asking them to sort your mail.

### OpenClaw's Current Security Model

OpenClaw provides multiple security layers:

1. **DM Pairing**: Unknown senders must be approved before the bot processes their messages
2. **Docker Sandboxing**: Tools can run inside containers to limit blast radius
3. **Tool Policies**: Allow/deny lists control which tools are available
4. **Elevated Mode Gates**: Extra controls for host execution

These are good defenses, but they share a critical weakness: **they're all software controls running as your user**.

### What Can Go Wrong

Even with all protections enabled, a sophisticated attack could:

| Attack | How It Works | Impact |
|--------|--------------|--------|
| **Direct database access** | Read `~/Library/Messages/chat.db` | All message history leaked |
| **Credential theft** | Access keychain or config files | Account takeover |
| **Allowlist modification** | Edit OpenClaw's allowlist files | Remove restrictions |
| **Audit log tampering** | Delete or modify log files | Cover tracks |
| **Tool bypass** | Call real `imsg` directly | Skip all filtering |

OpenClaw's own documentation acknowledges this:

> "If you need **stronger isolation between agents, run them under separate OS users or separate hosts**."

> "**Any process/user with filesystem access can read those logs.** Treat disk access as the trust boundary."

## The Solution: OS-Level Isolation

Carapace implements exactly what OpenClaw recommends: **separate OS users**.

```
┌─────────────────────────────────────────────────────────────────┐
│                    TRUST BOUNDARY                                │
│                   (OS User Separation)                           │
├─────────────────────────────────────────────────────────────────┤
│                                                                  │
│   UNTRUSTED SIDE                    TRUSTED SIDE                │
│   (Your Account)                    (Carapace Account)          │
│                                                                  │
│   • Runs OpenClaw                   • Holds all credentials      │
│   • Processes untrusted input       • Logged into iCloud         │
│   • Uses shim tools                 • Runs real tools            │
│   • Cannot access credentials       • Enforces allowlists        │
│   • Cannot modify allowlists        • Writes audit logs          │
│                                                                  │
│   Unix socket is the ONLY           Daemon is the ONLY           │
│   way to communicate                way to send messages         │
│                                                                  │
└─────────────────────────────────────────────────────────────────┘
```

### Why This Works

1. **File Permissions Are Kernel-Enforced**
   - Not software policy—actual OS enforcement
   - Cannot be bypassed by prompt injection
   - Cannot be bypassed by bugs or misconfiguration

2. **Credentials Don't Exist in OpenClaw's Context**
   - Not "hidden" or "restricted"—literally not present
   - iCloud login is on carapace user
   - Keychain items are user-specific

3. **Gateway Controls Are Immutable to OpenClaw**
   - Allowlists stored in carapace's home directory
   - Audit logs owned by carapace user
   - Configuration cannot be modified by AI

## Design Principles

### 1. Defense in Depth

Carapace adds a layer **on top of** OpenClaw's existing security, not replacing it. Use both:

- OpenClaw's pairing for initial access control
- Carapace's isolation for hard boundaries

### 2. Principle of Least Privilege

The AI gets exactly what it needs and nothing more:

- ✅ Send messages to allowlisted recipients
- ✅ Receive messages from allowlisted senders
- ❌ Direct database access
- ❌ Credential access
- ❌ Configuration changes

### 3. Fail Secure

When in doubt, block:

- Unknown recipients → blocked, logged to dead letter queue
- Rate limit exceeded → blocked, request denied
- Suspicious content → blocked, alert logged
- Socket connection fails → OpenClaw sees error, not credentials

### 4. Transparency

- Shims emulate real tool behavior exactly
- Audit logs capture all activity
- Dead letter queue preserves blocked request metadata
- No hidden behaviors

## Who Should Use Carapace

### Recommended For

- **Multi-user bots**: When multiple people can message your AI
- **High-value credentials**: When compromise would be costly
- **Compliance requirements**: When you need provable isolation
- **Peace of mind**: When you want hard guarantees, not "probably safe"

### May Be Overkill For

- **Personal-only use**: Just you messaging your own AI
- **No sensitive channels**: AI only uses web search, no messaging
- **Ephemeral setups**: Testing or development environments

## Comparison with Alternatives

| Approach | Isolation | Setup Complexity | Credential Safety |
|----------|-----------|------------------|-------------------|
| OpenClaw (default) | Software | Simple | Software controls |
| OpenClaw + Docker | Container | Moderate | Gateway still has access |
| OpenClaw + Carapace | OS User | Moderate | **Kernel enforced** |
| Separate machine | Hardware | Complex | Network isolated |

Carapace provides the strongest practical isolation for single-machine deployments.

## Next Steps

- [Architecture](02-architecture.md) - Technical details of how Carapace works
- [Security Model](03-security-model.md) - Detailed threat analysis
- [Setup Guide](04-setup-carapace-user.md) - Get started with installation
