# Carapace Overview

Carapace is a security gateway for AI agents on macOS. It sits between your AI agents (Claude Code, OpenClaw) and sensitive services (iMessage, Gmail, Google Drive) — enforcing allowlists, rate limits, content filtering, and audit logging.

## The Problem

AI agents that can send messages, read email, and access files are powerful — but dangerous if compromised. A prompt injection or jailbreak could:

- Send messages to anyone in your contacts
- Exfiltrate sensitive emails or documents
- Spam recipients or leak credentials
- Access data the agent was never meant to see

## The Solution

Carapace uses **OS-level user isolation** — not just software controls — to enforce security boundaries:

- A dedicated `carapace` macOS user owns all sensitive credentials (iCloud, Gmail OAuth tokens)
- Agents connect through a Unix socket with strict file permissions
- Every request passes through a security middleware pipeline before reaching any external service
- No agent can bypass the gateway — the credentials simply aren't accessible to them

## Design Principles

| Principle | How Carapace Implements It |
|-----------|---------------------------|
| Defense in Depth | Six security layers: file permissions, socket access, protocol restrictions, allowlists, content filtering, audit logging |
| Least Privilege | Agents can only reach services explicitly configured in the gateway |
| Fail Secure | Unknown channels, methods, or recipients are denied by default |
| Transparency | Every request is audit-logged with timestamp, method, parameters, and verdict |

## What Carapace Manages

| Channel | Capabilities | Restrictions |
|---------|-------------|-------------|
| iMessage | Send, receive, list chats, watch for new messages | Outbound allowlist, rate limiting |
| Gmail | Search, read threads, create drafts | No direct send (drafts only), content scrubbing, blocked label filtering |
| Google Docs/Drive | Search, read (Docs/Sheets/Slides/Forms), create, copy, folders | No delete, no sharing/permission changes, content scrubbing |

## Architecture at a Glance

```
Claude Code Agent (Jarvis)          Claude Code Agent (Wedding)
    |                                   |
    | MCP (gmail-mcp, gdocs-mcp)        | MCP (gmail-mcp, gdocs-mcp)
    v                                   v
carapace-daemon  <-- Unix socket (/var/run/carapace/gateway.sock)
    |
    |-- Security middleware (allowlist, rate limit, content filter, audit)
    |
    +-- gmail-proxy (OAuth, content scrubbing) --> Gmail API
    +-- gdocs-proxy (OAuth, structured read)   --> Drive/Docs/Sheets/Slides/Forms API
    +-- imsg adapter                           --> Messages.app
```

## Who This Is For

Carapace is purpose-built for a single Mac running AI agents that interact with real messaging and productivity services. It's not a general-purpose API gateway — it's a personal security layer for autonomous AI assistants.
