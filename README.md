# Carapace

**Zero-Trust Security Gateway for OpenClaw**

Carapace provides OS-level isolation between OpenClaw's AI runtime and your messaging credentials. By running credentials in a separate macOS user account, Carapace ensures that even a fully compromised AI cannot access your iMessage, Signal, Discord, or Gmail accounts directly.

```
┌──────────────────────────────────────────────────────────────────────────────┐
│                              YOUR MAC                                        │
├──────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  YOUR ACCOUNT        OPENCLAW ACCOUNT         CARAPACE ACCOUNT               │
│  (personal use)      (AI runtime only)        (holds credentials)            │
│                                                                              │
│  ┌──────────────┐    ┌──────────────────┐     ┌─────────────────────────┐   │
│  │ You (admin)  │    │  OpenClaw        │     │  Carapace Daemon        │   │
│  │              │    │       │          │     │       │                 │   │
│  │ Not running  │    │       ▼          │     │       ▼                 │   │
│  │ the AI --    │    │  imsg (shim)  ───────► │  Allowlist + Rate Limit │   │
│  │ interact via │    │  signal (shim)   │     │       │                 │   │
│  │ iMessage     │    │  discord (shim)  │     │       ▼                 │   │
│  │              │    │                  │     │  Real credentials       │   │
│  │              │    │  NO credentials  │     │  • iCloud / iMessage    │   │
│  │              │    │  NO personal data│     │  • Signal account       │   │
│  └──────────────┘    └──────────────────┘     │  • Discord bot token    │   │
│                                               └─────────────────────────┘   │
└──────────────────────────────────────────────────────────────────────────────┘
```

## Why Carapace?

OpenClaw's built-in security (Docker sandboxing, tool policies, allowlists) provides defense-in-depth, but it all runs as **your user**. If the AI is manipulated via prompt injection, it could potentially access your personal files, iMessage database, keychain credentials, or modify its own allowlists.

Carapace solves this with **two layers of OS-level isolation**:

1. **OpenClaw runs as a dedicated user** -- it cannot access your personal files, keychain, SSH keys, or browser data
2. **Credentials live in a third user account** -- the AI can only reach iMessage through the Carapace gateway, which enforces allowlists, rate limits, and content filtering

The AI literally cannot access credentials or your personal data -- not through prompt injection, not through misconfiguration, not through bugs. The OS enforces both boundaries.

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
Phase 1: Create carapace user        ✅ complete
Phase 2: Configure permissions       ✅ complete
Phase 3: Gateway infrastructure      ✅ complete
Phase 4: Security middleware         ✅ complete
Phase 5: iMessage channel adapter    ✅ complete
```

All five phases are implemented and working. The gateway provides full
iMessage access through the `imsg` shim with allowlists, rate limiting,
content filtering, and audit logging.

See [SETUP.md](SETUP.md) for installation and operations guide.

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
| Prompt injection -> send to anyone | Software blocks | **OS blocks** |
| Read message database | Possible | **Impossible** |
| Exfiltrate credentials | Possible | **Impossible** |
| Modify allowlists | Possible | **Impossible** |
| Disable audit logging | Possible | **Impossible** |
| Access personal files / keychain | Possible | **Impossible** |

## Documentation

**Concepts:**
- [Overview & Motivation](docs/01-overview.md)
- [Architecture](docs/02-architecture.md)
- [Security Model](docs/03-security-model.md)

**Setup:**
- [Setup & Operations Guide](SETUP.md) -- **Start here for installation**
- [Setup: Carapace User](docs/04-setup-carapace-user.md) (Phase 1)
- [Setup: Your Account](docs/05-setup-main-account.md) (Phase 2)
- [Gateway Infrastructure](docs/13-gateway-infrastructure.md) (Phase 3)

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

Phases 1-5 are complete. The gateway is fully operational with iMessage support. Additional channels (Signal, Discord, Gmail) can be added as new adapters.

## Name

*Carapace* (n.): The hard upper shell of a crustacean. A protective covering.

OpenClaw has a crustacean theme (🦞). Carapace provides the protective shell.

## License

MIT License - see [LICENSE](LICENSE)

## Contributing

Contributions welcome! This could become an official OpenClaw deployment option (`openclaw setup --mode zero-trust`).

See the [Roadmap](docs/12-roadmap.md) for planned features.
