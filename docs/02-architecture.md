# Architecture

## System Components

```
+---------------------------+     +---------------------------+
|  blakezimmerman (main)    |     |  carapace (service user)  |
|                           |     |                           |
|  Claude Code agents       |     |  carapace-daemon          |
|  (Jarvis, Wedding)        |     |  gmail-proxy (x2)         |
|                           |     |  gdocs-proxy (x2)         |
|  MCP servers:             |     |  Messages.app (GUI)       |
|    gmail-mcp              |     |  imsg binary              |
|    gdocs-mcp              |     |                           |
+-------------|-------------+     +-------------|-------------+
              |                                 |
              +--- Unix socket -----------------+
                   /var/run/carapace/gateway.sock
                   (mode 0770, group: carapace-clients)
```

## User Model

| User | Purpose | What It Owns |
|------|---------|-------------|
| `blakezimmerman` | Main account, runs Claude Code agents | Agent configs, SSH keys, code repos |
| `carapace` | Service account, owns all sensitive credentials | iCloud account, Gmail OAuth tokens, GDocs OAuth tokens, Messages.app |
| `openclaw` | Legacy agent runtime (being decommissioned) | OpenClaw config and logs |

The `carapace-clients` group grants socket access. Both `blakezimmerman` and `openclaw` are members.

## Daemon (carapace-daemon)

The central gateway process. Runs as `carapace` via LaunchDaemon.

**Responsibilities:**
- Listen on Unix socket for JSON-RPC requests
- Route requests to channel adapters (iMessage, Gmail, GDocs)
- Enforce security middleware on every request
- Manage watch subscriptions for real-time message streaming

**Security middleware pipeline (every request):**
1. Rate limiting (per-channel, configurable)
2. Content filtering (regex patterns, block/warn)
3. Allowlist enforcement (per-channel, per-direction)
4. Audit logging (all requests with verdict)
5. Dead letter storage (blocked messages saved for review)

## Channel Proxies

Each external service has a dedicated proxy that handles OAuth and API-specific concerns:

### gmail-proxy
- Manages OAuth 2.0 token refresh
- Content scrubbing (OTP redaction, auth URL stripping)
- Label-based filtering (AI-BLOCKED label hides messages)
- Query validation (blocks access to trash/spam/drafts)
- One instance per Gmail account

### gdocs-proxy
- Manages OAuth 2.0 token refresh for Drive/Docs/Sheets/Slides/Forms
- Auto-detects file type and routes to correct Google API
- Structured content output (headings, tables, cell data, slide text)
- Content scrubbing with configurable redact patterns
- No delete or share endpoints exposed
- One instance per Google account

## MCP Servers

MCP (Model Context Protocol) servers bridge between Claude Code agents and the Carapace gateway:

| Server | Binary | Tools |
|--------|--------|-------|
| gmail-mcp | `/usr/local/bin/gmail-mcp` | gmail_search, gmail_read_thread, gmail_create_draft, gmail_status |
| gdocs-mcp | `/usr/local/bin/gdocs-mcp` | gdocs_search, gdocs_read, gdocs_file_info, gdocs_create, gdocs_copy, gdocs_append, gdocs_create_folder, gdocs_status |

MCP servers are spawned per-agent-session by Claude Code. They connect to the gateway socket on first tool call.

## Data Flow (outbound Gmail draft example)

```
Agent calls gmail_create_draft("alice@example.com", "Hello", "...")
    |
    v
gmail-mcp: JSON-RPC call to gateway socket
    |
    v
carapace-daemon: rate limit check -> content filter -> audit log
    |
    v
GmailAdapter: HTTP POST to gmail-proxy Unix socket
    |
    v
gmail-proxy: OAuth token refresh -> Gmail API drafts.create
    |
    v
Response flows back up the chain
```

## Socket Layout

All sockets live in `/var/run/carapace/` (created at boot by `ai.carapace.setup`):

| Socket | Owner | Purpose |
|--------|-------|---------|
| `gateway.sock` | carapace:carapace-clients (0770) | Main gateway |
| `gmail-proxy.sock` | carapace:carapace-clients (0660) | Primary Gmail |
| `gmail-proxy-automations.sock` | carapace:carapace-clients (0660) | Automations Gmail |
| `gdocs-proxy-hq.sock` | carapace:carapace-clients (0660) | Primary GDocs |
| `gdocs-proxy-automations.sock` | carapace:carapace-clients (0660) | Automations GDocs |
