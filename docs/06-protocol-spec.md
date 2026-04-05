# Protocol Specification

The Carapace gateway speaks JSON-RPC 2.0 over Unix sockets with newline-delimited framing.

## Transport

- **Socket:** `/var/run/carapace/gateway.sock`
- **Framing:** Each message is a single JSON line terminated by `\n`
- **Connection:** Persistent — multiple requests per connection

## Methods

### channel.send

Send a message via a channel. Currently only iMessage supports direct send. Gmail returns an error (use `channel.create_draft` instead). GDocs uses this for copy, append, and create_folder actions.

```json
{"jsonrpc":"2.0","id":1,"method":"channel.send","params":{
  "channel": "imsg",
  "recipient": "+19705551234",
  "message": "Hello!",
  "attachments": ["/path/to/file.jpg"]
}}
```

GDocs actions via channel.send:
```json
{"jsonrpc":"2.0","id":2,"method":"channel.send","params":{
  "channel": "gdocs",
  "account": "hq",
  "action": "copy",
  "file_id": "abc123"
}}
```

### channel.list_chats

List recent conversations (iMessage) or recent files (GDocs) or inbox threads (Gmail).

```json
{"jsonrpc":"2.0","id":2,"method":"channel.list_chats","params":{
  "channel": "imsg",
  "limit": 20
}}
```

### channel.get_history

Get message history for a chat (iMessage), thread (Gmail), or read a document (GDocs).

```json
{"jsonrpc":"2.0","id":3,"method":"channel.get_history","params":{
  "channel": "imsg",
  "chat_id": "+19705551234",
  "limit": 50
}}
```

### channel.search

Search messages (Gmail) or files (GDocs). Not supported on iMessage.

```json
{"jsonrpc":"2.0","id":4,"method":"channel.search","params":{
  "channel": "gmail",
  "account": "primary",
  "query": "from:boss@company.com is:unread",
  "max": 20
}}
```

### channel.create_draft

Create a Gmail draft or a new Google Doc.

Gmail:
```json
{"jsonrpc":"2.0","id":5,"method":"channel.create_draft","params":{
  "channel": "gmail",
  "to": "alice@example.com",
  "subject": "Hello",
  "body": "Hi Alice!"
}}
```

GDocs:
```json
{"jsonrpc":"2.0","id":6,"method":"channel.create_draft","params":{
  "channel": "gdocs",
  "title": "Meeting Notes",
  "content": "Initial content here",
  "folder_id": "optional_folder_id"
}}
```

### channel.watch

Subscribe to real-time message notifications. Returns an initial ack, then streams notifications.

```json
{"jsonrpc":"2.0","id":7,"method":"channel.watch","params":{
  "channel": "imsg"
}}
```

### channel.status

Health check for a channel.

```json
{"jsonrpc":"2.0","id":8,"method":"channel.status","params":{
  "channel": "gmail",
  "account": "primary"
}}
```

## Error Codes

| Code | Name | Meaning |
|------|------|---------|
| -32700 | Parse error | Invalid JSON |
| -32600 | Invalid request | Missing jsonrpc/id/method |
| -32601 | Method not found | Unknown method |
| -32602 | Invalid params | Missing or invalid parameters |
| -32603 | Internal error | Unexpected server error |
| -32001 | Not in allowlist | Recipient blocked by allowlist |
| -32002 | Rate limited | Too many requests |
| -32003 | Content blocked | Content filter matched a block pattern |
| -32004 | Channel unavailable | Channel not configured or adapter missing |
| -32005 | Send failed | Adapter-level send failure |

## Multi-Account

Gmail and GDocs support multiple accounts. Pass `"account": "<name>"` in params. If omitted, the `default_account` from config is used.
