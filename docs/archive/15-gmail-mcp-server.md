# Gmail MCP Server

`gmail-mcp` is a [Model Context Protocol](https://modelcontextprotocol.io) server that exposes the Carapace Gmail channel as named tools any MCP-capable agent can call natively.

---

## Why MCP instead of a CLI shim

The iMessage channel works because OpenClaw has a **built-in iMessage plugin** — the agent doesn't call CLI commands, OpenClaw's routing layer does. Gmail has no built-in OpenClaw plugin, so a CLI shim alone wouldn't teach the agent anything.

MCP solves this differently: the server publishes a `tools/list` response that contains a name, description, and JSON schema for each tool. When the agent connects, it reads those descriptions and immediately knows what each tool does and how to call it — no prior training required.

---

## Tools

| Tool | What it does |
|------|-------------|
| `gmail_search` | Search emails using Gmail query syntax (`from:`, `is:unread`, `subject:`, etc.) |
| `gmail_read_thread` | Fetch all messages in a thread by `thread_id` |
| `gmail_create_draft` | Create a draft email (saved to Drafts, **never sent automatically**) |
| `gmail_status` | Check proxy reachability and OAuth token health |

`channel.send` is not exposed. There is no `gmail_send` tool.

---

## Architecture

```
Agent (Claude Code / OpenClaw)
    │  MCP stdio (JSON-RPC 2.0 over stdin/stdout)
    ▼
gmail-mcp  (/usr/local/bin/gmail-mcp, runs as carapace)
    │  JSON-RPC 2.0 over Unix socket
    ▼
carapace-daemon  (/var/run/carapace/gateway.sock)
    │  HTTP/1.1 over Unix socket
    ▼
gmail-proxy  (/var/run/carapace/gmail-proxy.sock)
    │  HTTPS / OAuth 2.0
    ▼
Gmail API
```

The MCP server is a thin translation layer. It converts MCP `tools/call` requests into `channel.*` JSON-RPC calls on the gateway socket, then wraps the response in MCP's `content` block format.

---

## How agents learn to use it

When the agent connects, `gmail-mcp` responds to `tools/list` with tool definitions that include detailed descriptions:

```
gmail_search: Search emails using Gmail's standard query syntax.
  Returns a list of matching messages with id, thread_id, subject, from, date,
  and a plain-text snippet of the body (OTP codes and auth URLs are pre-scrubbed).
  Supported operators: from:, to:, subject:, is:unread, is:read, has:attachment,
  after:, before:, older_than:, newer_than:, in:inbox, in:sent, label:, cc:, bcc:, filename:.
  Operators that access trash, spam, or all-mail are blocked.
  Example queries: "from:boss@company.com is:unread", "subject:invoice newer_than:7d".
```

Claude has strong training on Gmail query syntax, so it will construct correct queries on the first try from these descriptions alone.

---

## Installation

### Build and copy the binary

```bash
# From the Carapace project directory (as blakezimmerman):
cargo build --release -p carapace-shims
sudo cp target/release/gmail-mcp /usr/local/bin/gmail-mcp
sudo chmod 755 /usr/local/bin/gmail-mcp
```

### Configure Claude Code

Add to `/Users/blakezimmerman/.claude/settings.json` (or `settings.local.json`):

```json
{
  "mcpServers": {
    "gmail": {
      "command": "sudo",
      "args": ["-u", "carapace", "/usr/local/bin/gmail-mcp"]
    }
  }
}
```

Restart Claude Code after editing. You should see `gmail_search`, `gmail_read_thread`, `gmail_create_draft`, and `gmail_status` appear in the tool list.

### Configure OpenClaw (acpx plugin)

In the OpenClaw `acpx` plugin config, add under `mcpServers`:

```json
{
  "mcpServers": {
    "gmail": {
      "command": "sudo",
      "args": ["-u", "carapace", "/usr/local/bin/gmail-mcp"]
    }
  }
}
```

---

## Testing the MCP server manually

You can drive the MCP handshake by hand to verify it's working before wiring it into an agent:

```bash
# Run the server as carapace, feed it MCP messages:
sudo -u carapace /usr/local/bin/gmail-mcp <<'EOF'
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test","version":"0"}}}
{"jsonrpc":"2.0","method":"notifications/initialized","params":{}}
{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}
{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"gmail_status","arguments":{}}}
EOF
```

Expected output (one JSON object per line):
1. `initialize` response with `protocolVersion` and `serverInfo`
2. (no response for the notification)
3. `tools/list` response with all four tool definitions
4. `gmail_status` response with `proxy_reachable: true, token_valid: true`

---

## What to ask the agent

Once configured, you can ask the agent naturally:

- *"Check my Gmail inbox for unread emails"*
- *"Search Gmail for emails from boss@company.com in the last week"*
- *"Read the full thread of that invoice email"*
- *"Draft a reply to [email] saying I'll follow up Thursday"*
- *"Is the Gmail connection healthy?"*

The agent will call the appropriate tool without you specifying the tool name.

---

## Deploying updates

```bash
cargo build --release -p carapace-shims
sudo cp target/release/gmail-mcp /usr/local/bin/gmail-mcp
# No daemon restart needed — the MCP server is spawned fresh per agent session.
```

---

## Logs

The MCP server writes diagnostic messages to stderr. When running under Claude Code or OpenClaw, stderr is typically captured by the host process. To see logs manually:

```bash
sudo -u carapace /usr/local/bin/gmail-mcp 2>/tmp/gmail-mcp.log &
# Then feed it messages...
tail -f /tmp/gmail-mcp.log
```

---

## How the MCP protocol works (brief)

MCP uses JSON-RPC 2.0 over stdio. The sequence for every session is:

1. **`initialize`** — client sends capabilities; server responds with its protocol version and capabilities
2. **`notifications/initialized`** — client notifies it's ready (no response)
3. **`tools/list`** — client asks what tools are available; server returns definitions
4. **`tools/call`** — client calls a tool by name with arguments; server returns a `content` array

The `content` array is an array of typed blocks. `gmail-mcp` always returns a single `text` block containing the JSON-serialized result from the Carapace gateway. The `isError: true` flag signals graceful tool failure (the agent can decide how to handle it) vs. a protocol-level error.
