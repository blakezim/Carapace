# Architecture

## System Overview

Carapace consists of four main components:

1. **Shim Tools** - Drop-in replacements that redirect to the gateway
2. **Unix Domain Socket** - Secure IPC channel between users
3. **Carapace Daemon** - Security gateway running as carapace user
4. **Channel Adapters** - Integrations with real messaging tools

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                              CARAPACE ARCHITECTURE                           │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                              │
│  YOUR ACCOUNT                              CARAPACE ACCOUNT                  │
│  ────────────────                          ─────────────────                 │
│                                                                              │
│  ┌──────────────────┐                      ┌────────────────────────────┐   │
│  │   OpenClaw       │                      │   Carapace Daemon          │   │
│  │   Gateway        │                      │                            │   │
│  └────────┬─────────┘                      │  ┌──────────────────────┐  │   │
│           │                                │  │  Socket Listener     │  │   │
│           │ calls                          │  └──────────┬───────────┘  │   │
│           ▼                                │             │              │   │
│  ┌──────────────────┐                      │             ▼              │   │
│  │   Shim Tools     │                      │  ┌──────────────────────┐  │   │
│  │                  │    JSON-RPC          │  │  Request Router      │  │   │
│  │  /usr/local/     │    over Unix         │  └──────────┬───────────┘  │   │
│  │  carapace/bin/   │    Socket            │             │              │   │
│  │                  │◄────────────────────►│             ▼              │   │
│  │  • imsg          │                      │  ┌──────────────────────┐  │   │
│  │  • signal-cli    │                      │  │  Security Middleware │  │   │
│  │  • discord-cli   │                      │  │                      │  │   │
│  │  • gog           │                      │  │  • Rate Limiter      │  │   │
│  │                  │                      │  │  • Allowlist         │  │   │
│  └──────────────────┘                      │  │  • Content Filter    │  │   │
│                                            │  │  • Audit Logger      │  │   │
│  PATH has shims first                      │  └──────────┬───────────┘  │   │
│  Real tools not in PATH                    │             │              │   │
│                                            │             ▼              │   │
│                                            │  ┌──────────────────────┐  │   │
│                                            │  │  Channel Adapters    │  │   │
│                                            │  │                      │  │   │
│                                            │  │  • ImsgAdapter       │  │   │
│                                            │  │  • SignalAdapter     │  │   │
│                                            │  │  • DiscordAdapter    │  │   │
│                                            │  │  • GmailAdapter      │  │   │
│                                            │  └──────────┬───────────┘  │   │
│                                            │             │              │   │
│                                            │             ▼              │   │
│                                            │  ┌──────────────────────┐  │   │
│                                            │  │  Real Tools          │  │   │
│                                            │  │                      │  │   │
│                                            │  │  • /opt/homebrew/    │  │   │
│                                            │  │    bin/imsg          │  │   │
│                                            │  │  • signal-cli        │  │   │
│                                            │  │  • Discord.js        │  │   │
│                                            │  │  • Gmail API         │  │   │
│                                            │  └──────────────────────┘  │   │
│                                            │                            │   │
│                                            │  Files owned by carapace:  │   │
│                                            │  • ~/.config/carapace/     │   │
│                                            │  • ~/Library/Messages/     │   │
│                                            │  • Keychain credentials    │   │
│                                            └────────────────────────────┘   │
│                                                                              │
│  SOCKET: /var/run/carapace/gateway.sock                                     │
│  OWNER: carapace:carapace-clients                                            │
│  PERMS: srwxrwx--- (770)                                                     │
│                                                                              │
└─────────────────────────────────────────────────────────────────────────────┘
```

## Component Details

### 1. Shim Tools

Shims are lightweight binaries that emulate the CLI interface of real tools but redirect all operations through the gateway.

**Location:** `/usr/local/carapace/bin/`

**Behavior:**
```bash
# User runs:
imsg send "+1234567890" "Hello"

# This is actually the shim, which:
# 1. Parses CLI arguments
# 2. Connects to Unix socket
# 3. Sends JSON-RPC request
# 4. Receives response
# 5. Formats output exactly like real imsg
```

**Key Properties:**
- Exact CLI compatibility with real tools
- No credentials or sensitive data
- Fails gracefully if daemon unavailable
- Stateless (no local storage)

### 2. Unix Domain Socket

The socket provides secure IPC between your account and the carapace daemon.

**Location:** `/var/run/carapace/gateway.sock`

**Why Unix Sockets?**

| Property | Unix Socket | TCP Loopback | Named Pipe |
|----------|-------------|--------------|------------|
| Network exposure | None | Localhost only | None |
| Permission control | File permissions | Port access | File permissions |
| Performance | Fast | Moderate | Fast |
| Cross-platform | Unix/macOS | Universal | Windows-focused |

**Permission Model:**
```
Socket: /var/run/carapace/gateway.sock
Owner: carapace
Group: carapace-clients
Mode: srwxrwx--- (770)

Directory: /var/run/carapace/
Owner: carapace
Group: carapace-clients
Mode: drwxr-x--- (750)
```

Only the carapace user and members of `carapace-clients` group can connect.

### 3. Carapace Daemon

The daemon is the security gateway. It runs as the carapace user and is the only component with access to credentials.

**Responsibilities:**
- Listen on Unix socket
- Authenticate incoming connections
- Route requests to appropriate channel adapters
- Enforce security policies (rate limit, allowlist, content filter)
- Log all activity
- Manage dead letter queue

**Process Model:**
```
carapace-daemon (main process)
├── Socket listener (async)
├── Request handler pool
├── Channel adapter threads
│   ├── imsg watch (if enabled)
│   ├── signal receive (if enabled)
│   └── discord events (if enabled)
└── Maintenance tasks
    ├── Log rotation
    ├── Rate limit cleanup
    └── Config reload watcher
```

### 4. Channel Adapters

Each messaging platform has a dedicated adapter that knows how to interact with the real tools.

**Adapter Interface:**
```rust
trait ChannelAdapter {
    /// Send a message
    async fn send(&self, request: SendRequest) -> Result<SendResponse>;

    /// Subscribe to incoming messages
    async fn receive(&self) -> impl Stream<Item = IncomingMessage>;

    /// List conversations
    async fn list_chats(&self, limit: u32) -> Result<Vec<Chat>>;

    /// Get message history
    async fn get_history(&self, chat_id: &str, limit: u32) -> Result<Vec<Message>>;

    /// Check channel health
    async fn status(&self) -> ChannelStatus;
}
```

## Data Flow

### Outbound Message (AI → World)

```
1. OpenClaw decides to send a message
   └─► Calls: imsg send "+1234567890" "Hello"

2. Shim receives command
   └─► Parses arguments
   └─► Connects to /var/run/carapace/gateway.sock

3. Shim sends JSON-RPC request
   └─► {"jsonrpc":"2.0","method":"channel.send","params":{...}}

4. Daemon receives request
   └─► Authenticates connection (group membership)

5. Security middleware checks
   ├─► Rate limiter: Is this within limits?
   ├─► Allowlist: Is recipient approved?
   ├─► Content filter: Any blocked patterns?
   └─► Audit logger: Record the attempt

6. If BLOCKED:
   └─► Log to dead letter queue
   └─► Return error to shim

7. If ALLOWED:
   └─► Channel adapter executes real command
   └─► As carapace user with real credentials
   └─► Return success to shim

8. Shim formats response
   └─► Prints output matching real imsg format
```

### Inbound Message (World → AI)

```
1. Real message arrives
   └─► imsg watch detects new message (as carapace user)

2. Channel adapter receives event
   └─► Parses message metadata

3. Inbound security check
   ├─► Is sender in inbound allowlist?
   └─► Audit logger: Record receipt

4. If BLOCKED:
   └─► Message hidden from OpenClaw
   └─► Logged for review

5. If ALLOWED:
   └─► Forward to connected shim streams
   └─► OpenClaw sees the message
```

## Security Boundaries

### What Your Account CAN Do

- Connect to the Unix socket (via group membership)
- Send requests through the gateway
- Receive responses and message streams
- See error messages when blocked

### What Your Account CANNOT Do

- Access carapace user's files
- Read the message database directly
- Modify allowlists or configuration
- Bypass rate limiting
- See the audit logs (read-only access could be granted)
- Access credentials or tokens

### What Carapace Account CAN Do

- Read and write all messaging databases
- Access all configured credentials
- Send messages to anyone (but only via daemon)
- Modify configuration
- Read and write audit logs

## Failure Modes

| Failure | Behavior | Recovery |
|---------|----------|----------|
| Daemon not running | Shims return connection error | Start daemon |
| Socket permissions wrong | Shims return permission denied | Fix permissions |
| Channel adapter crash | That channel unavailable | Daemon restarts adapter |
| Config file invalid | Daemon refuses to start | Fix config, restart |
| Rate limit exceeded | Requests denied with error | Wait for limit reset |
| Disk full | Audit logs may fail | Free space |

## Performance Considerations

### Latency

Each request adds ~1-5ms for:
- Socket connection
- JSON serialization
- Security checks
- Response formatting

This is negligible for messaging use cases.

### Throughput

The daemon can handle hundreds of requests per second. Rate limiting is the practical constraint, not system performance.

### Memory

- Daemon: ~20-50MB baseline
- Per shim process: ~5MB
- Audit logs: ~1KB per entry (rotated)

## Next Steps

- [Security Model](03-security-model.md) - Detailed threat analysis
- [Protocol Specification](06-protocol-spec.md) - JSON-RPC details
- [Daemon Implementation](07-daemon-implementation.md) - Building the daemon
