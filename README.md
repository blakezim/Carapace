# Carapace

**Zero-Trust Security Gateway for OpenClaw**

Carapace provides OS-level isolation between OpenClaw's AI runtime and your messaging credentials. By running credentials in a separate macOS user account, Carapace ensures that even a fully compromised AI cannot access your iMessage, Signal, Discord, or Gmail accounts directly.

```
┌─────────────────────────────────────────────────────────────────────────┐
│                            YOUR MAC                                      │
├─────────────────────────────────────────────────────────────────────────┤
│                                                                          │
│   YOUR ACCOUNT                         CARAPACE ACCOUNT                  │
│   (runs OpenClaw)                      (holds credentials)               │
│                                                                          │
│   ┌─────────────────────┐              ┌─────────────────────────────┐  │
│   │  OpenClaw           │   Socket     │  Carapace Daemon            │  │
│   │       │             │◄────────────►│       │                     │  │
│   │       ▼             │              │       ▼                     │  │
│   │  imsg (shim)        │              │  Allowlist + Rate Limit     │  │
│   │  signal (shim)      │              │       │                     │  │
│   │  discord (shim)     │              │       ▼                     │  │
│   │                     │              │  Real credentials           │  │
│   │  NO credentials     │              │  • iCloud / iMessage        │  │
│   │  NO database access │              │  • Signal account           │  │
│   └─────────────────────┘              │  • Discord bot token        │  │
│                                        └─────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────────────┘
```

## Why Carapace?

OpenClaw's built-in security (Docker sandboxing, tool policies, allowlists) provides defense-in-depth, but it all runs as **your user**. If the AI is manipulated via prompt injection, it could potentially:

- Access your iMessage database directly
- Read credentials from your keychain
- Modify its own allowlists
- Disable audit logging

Carapace solves this by putting credentials in a **separate macOS user account**. The AI literally cannot access them—not through prompt injection, not through misconfiguration, not through bugs. The OS enforces the boundary.

## Features

- **OS-Level Isolation**: Credentials live in a separate user account
- **Transparent Shims**: Drop-in replacements for `imsg`, `signal-cli`, etc.
- **Bidirectional Filtering**: Control who the AI can message AND who can message it
- **Rate Limiting**: Prevent spam and abuse
- **Content Filtering**: Block sensitive patterns (passwords, API keys)
- **Audit Logging**: Tamper-proof logs (owned by carapace user)
- **Dead Letter Queue**: Review blocked messages
- **Multi-Channel**: iMessage, Signal, Discord, Gmail (extensible)

## Development Approach

We build Carapace in phases, understanding each layer before adding the next:

```
Phase 1: Create carapace user        ✅ (documentation complete)
Phase 2: Configure permissions       ✅ (documentation complete)
Phase 3: Gateway infrastructure      🚧 IN PROGRESS
Phase 4: Security middleware         📋 planned
Phase 5: Channel adapters            📋 planned
```

### Current Focus: Phase 3 - Gateway Infrastructure

Before adding security or channel support, we're building the foundational IPC:

1. **Minimal daemon** - Unix socket server running as carapace user
2. **JSON-RPC protocol** - Structured request/response messaging
3. **Client library** - Reusable connection code for shims
4. **Command passthrough** - Prove commands execute as carapace user

See [Gateway Infrastructure](docs/13-gateway-infrastructure.md) for implementation details.

## Quick Start

### Phase 1: Create Carapace User

1. System Settings → Users & Groups → Add "carapace"
2. Log in as carapace
3. Sign into iCloud (for iMessage)
4. Log out

### Phase 2: Configure Your Account

```bash
# Create shared group and socket directory
sudo dseditgroup -o create carapace-clients
sudo dseditgroup -o edit -a $(whoami) -t user carapace-clients
sudo mkdir -p /var/run/carapace
sudo chown carapace:carapace-clients /var/run/carapace
sudo chmod 750 /var/run/carapace

# Log out and back in for group membership to take effect
```

See [Setup Guides](docs/04-setup-carapace-user.md) for detailed instructions.

## How It Works

1. OpenClaw calls `imsg send "+1234567890" "Hello"`
2. This runs the **shim** (not real imsg)
3. Shim connects to Carapace daemon via Unix socket
4. Daemon checks allowlist, rate limit, content filter
5. If allowed, daemon runs **real imsg** (as carapace user)
6. Response flows back through the shim

The AI never sees the real credentials, database, or tools.

## Security Model

| Attack Vector | Standard OpenClaw | With Carapace |
|---------------|------------------|---------------|
| Prompt injection → send to anyone | Software blocks | **OS blocks** |
| Read message database | Possible | **Impossible** |
| Exfiltrate credentials | Possible | **Impossible** |
| Modify allowlists | Possible | **Impossible** |
| Disable audit logging | Possible | **Impossible** |

## Documentation

**Concepts:**
- [Overview & Motivation](docs/01-overview.md)
- [Architecture](docs/02-architecture.md)
- [Security Model](docs/03-security-model.md)

**Setup:**
- [Setup: Carapace User](docs/04-setup-carapace-user.md) (Phase 1)
- [Setup: Your Account](docs/05-setup-main-account.md) (Phase 2)
- [Gateway Infrastructure](docs/13-gateway-infrastructure.md) (Phase 3) ← **Start here for implementation**

**Technical Reference:**
- [Protocol Specification](docs/06-protocol-spec.md)
- [Daemon Implementation](docs/07-daemon-implementation.md)
- [Shim Implementation](docs/08-shim-implementation.md)
- [Channel Adapters](docs/09-channel-adapters.md)
- [Configuration Reference](docs/10-configuration-reference.md)

**Operations:**
- [Troubleshooting](docs/11-troubleshooting.md)
- [Roadmap](docs/12-roadmap.md)

## Project Status

🚧 **Under Development** 🚧

This project is in active development. The architecture is designed, documentation is complete, and implementation is in progress.

## Name

*Carapace* (n.): The hard upper shell of a crustacean. A protective covering.

OpenClaw has a crustacean theme (🦞). Carapace provides the protective shell.

## License

MIT License - see [LICENSE](LICENSE)

## Contributing

Contributions welcome! This could become an official OpenClaw deployment option (`openclaw setup --mode zero-trust`).

See the [Roadmap](docs/12-roadmap.md) for planned features.
