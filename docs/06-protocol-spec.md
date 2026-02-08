# Protocol Specification

Carapace uses JSON-RPC 2.0 over Unix domain sockets for communication between shims and the daemon.

## Transport

### Socket Location

```
/var/run/carapace/gateway.sock
```

### Connection

Shims connect to the socket using standard Unix socket APIs:

```rust
use std::os::unix::net::UnixStream;

let stream = UnixStream::connect("/var/run/carapace/gateway.sock")?;
```

### Message Framing

Messages are newline-delimited JSON. Each JSON-RPC message is followed by a newline (`\n`).

```
{"jsonrpc":"2.0","id":1,"method":"channel.send","params":{...}}\n
{"jsonrpc":"2.0","id":1,"result":{...}}\n
```

### Connection Lifecycle

1. Client connects to socket
2. Client sends requests, daemon sends responses
3. For streaming (watch), daemon sends multiple notifications
4. Client closes connection when done

Connections are not authenticated beyond Unix socket permissions (group membership).

---

## JSON-RPC Format

### Request

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "channel.send",
  "params": {
    "channel": "imsg",
    "recipient": "+1234567890",
    "message": "Hello, world!"
  }
}
```

| Field | Type | Description |
|-------|------|-------------|
| `jsonrpc` | string | Always `"2.0"` |
| `id` | number/string | Request identifier (echoed in response) |
| `method` | string | Method to invoke |
| `params` | object | Method parameters |

### Response (Success)

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "success": true,
    "message_id": "abc123",
    "timestamp": "2026-02-04T10:30:00Z"
  }
}
```

### Response (Error)

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "error": {
    "code": -32001,
    "message": "Recipient not in allowlist",
    "data": {
      "recipient": "+1234567890",
      "dead_letter_id": "dl-456"
    }
  }
}
```

### Notification (No Response Expected)

Used for streaming events:

```json
{
  "jsonrpc": "2.0",
  "method": "channel.message",
  "params": {
    "channel": "imsg",
    "sender": "+1234567890",
    "message": "Hello!",
    "timestamp": "2026-02-04T10:30:00Z"
  }
}
```

---

## Error Codes

| Code | Name | Description |
|------|------|-------------|
| -32700 | Parse error | Invalid JSON |
| -32600 | Invalid request | Not a valid JSON-RPC request |
| -32601 | Method not found | Unknown method |
| -32602 | Invalid params | Invalid method parameters |
| -32603 | Internal error | Server error |
| -32001 | Not in allowlist | Recipient/sender not allowed |
| -32002 | Rate limited | Too many requests |
| -32003 | Content blocked | Message contains blocked patterns |
| -32004 | Channel unavailable | Channel not configured or not responding |
| -32005 | Send failed | Underlying tool failed to send |

---

## Methods

### `channel.send`

Send a message through a channel.

**Request:**
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "channel.send",
  "params": {
    "channel": "imsg",
    "recipient": "+1234567890",
    "message": "Hello!",
    "attachments": [
      {
        "path": "/tmp/image.png",
        "mime_type": "image/png"
      }
    ]
  }
}
```

| Param | Type | Required | Description |
|-------|------|----------|-------------|
| `channel` | string | Yes | Channel identifier (imsg, signal, discord, gmail) |
| `recipient` | string | Yes | Recipient identifier (phone, email, channel ID) |
| `message` | string | Yes | Message text |
| `attachments` | array | No | File attachments |

**Response:**
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "success": true,
    "message_id": "abc123",
    "timestamp": "2026-02-04T10:30:00Z"
  }
}
```

---

### `channel.list_chats`

List conversations for a channel.

**Request:**
```json
{
  "jsonrpc": "2.0",
  "id": 2,
  "method": "channel.list_chats",
  "params": {
    "channel": "imsg",
    "limit": 20,
    "offset": 0
  }
}
```

| Param | Type | Required | Description |
|-------|------|----------|-------------|
| `channel` | string | Yes | Channel identifier |
| `limit` | number | No | Max results (default: 20) |
| `offset` | number | No | Skip first N results (default: 0) |

**Response:**
```json
{
  "jsonrpc": "2.0",
  "id": 2,
  "result": {
    "chats": [
      {
        "id": "chat123",
        "participants": ["+1234567890"],
        "display_name": "John Doe",
        "last_message": "See you tomorrow!",
        "last_message_time": "2026-02-04T09:15:00Z",
        "unread_count": 2
      }
    ],
    "total": 45,
    "has_more": true
  }
}
```

**Note:** Only chats with allowlisted participants are returned.

---

### `channel.get_history`

Get message history for a specific chat.

**Request:**
```json
{
  "jsonrpc": "2.0",
  "id": 3,
  "method": "channel.get_history",
  "params": {
    "channel": "imsg",
    "chat_id": "chat123",
    "limit": 50,
    "before": "2026-02-04T10:00:00Z"
  }
}
```

| Param | Type | Required | Description |
|-------|------|----------|-------------|
| `channel` | string | Yes | Channel identifier |
| `chat_id` | string | Yes | Chat/conversation ID |
| `limit` | number | No | Max messages (default: 50) |
| `before` | string | No | Get messages before this timestamp |

**Response:**
```json
{
  "jsonrpc": "2.0",
  "id": 3,
  "result": {
    "messages": [
      {
        "id": "msg456",
        "sender": "+1234567890",
        "text": "Hello!",
        "timestamp": "2026-02-04T09:00:00Z",
        "is_from_me": false,
        "attachments": []
      },
      {
        "id": "msg457",
        "sender": "me",
        "text": "Hi there!",
        "timestamp": "2026-02-04T09:01:00Z",
        "is_from_me": true,
        "attachments": []
      }
    ],
    "has_more": true
  }
}
```

---

### `channel.watch`

Subscribe to incoming messages (streaming).

**Request:**
```json
{
  "jsonrpc": "2.0",
  "id": 4,
  "method": "channel.watch",
  "params": {
    "channel": "imsg",
    "include_history": false
  }
}
```

| Param | Type | Required | Description |
|-------|------|----------|-------------|
| `channel` | string | Yes | Channel identifier |
| `include_history` | boolean | No | Include recent messages first (default: false) |

**Response (acknowledgment):**
```json
{
  "jsonrpc": "2.0",
  "id": 4,
  "result": {
    "subscribed": true,
    "subscription_id": "sub789"
  }
}
```

**Notifications (streaming):**
```json
{
  "jsonrpc": "2.0",
  "method": "channel.message",
  "params": {
    "subscription_id": "sub789",
    "channel": "imsg",
    "chat_id": "chat123",
    "message": {
      "id": "msg999",
      "sender": "+1234567890",
      "text": "New message!",
      "timestamp": "2026-02-04T10:35:00Z",
      "is_from_me": false
    }
  }
}
```

**Note:** Only messages from allowlisted senders are forwarded.

---

### `channel.status`

Check channel health and configuration.

**Request:**
```json
{
  "jsonrpc": "2.0",
  "id": 5,
  "method": "channel.status",
  "params": {
    "channel": "imsg"
  }
}
```

**Response:**
```json
{
  "jsonrpc": "2.0",
  "id": 5,
  "result": {
    "channel": "imsg",
    "enabled": true,
    "healthy": true,
    "last_check": "2026-02-04T10:30:00Z",
    "outbound_mode": "allowlist",
    "outbound_allowlist_count": 5,
    "inbound_mode": "allowlist",
    "inbound_allowlist_count": 3
  }
}
```

---

### `admin.get_dead_letters`

Retrieve blocked message metadata (admin only).

**Request:**
```json
{
  "jsonrpc": "2.0",
  "id": 6,
  "method": "admin.get_dead_letters",
  "params": {
    "limit": 20,
    "since": "2026-02-04T00:00:00Z"
  }
}
```

**Response:**
```json
{
  "jsonrpc": "2.0",
  "id": 6,
  "result": {
    "dead_letters": [
      {
        "id": "dl-456",
        "timestamp": "2026-02-04T10:30:05Z",
        "channel": "imsg",
        "direction": "outbound",
        "recipient": "+9999999999",
        "reason": "allowlist",
        "content_hash": "sha256:abc123..."
      }
    ],
    "total": 12
  }
}
```

**Note:** Message content is NOT stored, only metadata.

---

### `admin.reload_config`

Hot-reload configuration without restarting the daemon.

**Request:**
```json
{
  "jsonrpc": "2.0",
  "id": 7,
  "method": "admin.reload_config",
  "params": {}
}
```

**Response:**
```json
{
  "jsonrpc": "2.0",
  "id": 7,
  "result": {
    "reloaded": true,
    "timestamp": "2026-02-04T10:35:00Z"
  }
}
```

---

## Channel-Specific Parameters

### iMessage (`imsg`)

**Recipient formats:**
- Phone: `+1234567890` (E.164 format)
- Email: `email:someone@icloud.com`

**Chat ID format:** Internal iMessage chat GUID

### Signal (`signal`)

**Recipient formats:**
- Phone: `+1234567890`
- Group: `group:BASE64GROUPID`

### Discord (`discord`)

**Recipient formats:**
- Channel: `channel:123456789012345678`
- User DM: `user:987654321098765432`

### Gmail (`gmail`)

**Recipient formats:**
- Email: `someone@example.com`

**Additional params for send:**
- `subject`: Email subject line
- `thread_id`: Reply to existing thread

---

## Example Session

```
CLIENT: {"jsonrpc":"2.0","id":1,"method":"channel.status","params":{"channel":"imsg"}}
SERVER: {"jsonrpc":"2.0","id":1,"result":{"channel":"imsg","enabled":true,"healthy":true,...}}

CLIENT: {"jsonrpc":"2.0","id":2,"method":"channel.list_chats","params":{"channel":"imsg","limit":5}}
SERVER: {"jsonrpc":"2.0","id":2,"result":{"chats":[...],"total":45,"has_more":true}}

CLIENT: {"jsonrpc":"2.0","id":3,"method":"channel.send","params":{"channel":"imsg","recipient":"+1234567890","message":"Hello!"}}
SERVER: {"jsonrpc":"2.0","id":3,"result":{"success":true,"message_id":"abc123",...}}

CLIENT: {"jsonrpc":"2.0","id":4,"method":"channel.watch","params":{"channel":"imsg"}}
SERVER: {"jsonrpc":"2.0","id":4,"result":{"subscribed":true,"subscription_id":"sub789"}}
SERVER: {"jsonrpc":"2.0","method":"channel.message","params":{"subscription_id":"sub789",...}}
SERVER: {"jsonrpc":"2.0","method":"channel.message","params":{"subscription_id":"sub789",...}}
...
```

---

## Implementation Notes

### Concurrency

- Multiple clients can connect simultaneously
- Each connection is independent
- Watch subscriptions are per-connection

### Timeouts

- Request timeout: 30 seconds
- Watch keepalive: ping every 30 seconds

### Error Handling

- Parse errors close the connection
- Method errors return error responses
- Channel errors are wrapped in error responses

### Backpressure

- Watch notifications are buffered (limit: 1000)
- If buffer full, oldest notifications dropped
- Client should process notifications promptly
