# Roadmap

Development roadmap for Carapace, structured for incremental learning and implementation.

## Philosophy

**Build in layers, understand each one.**

We start with the simplest possible working system and add complexity incrementally. Each phase produces something you can run, test, and understand before moving on.

---

## Phase Overview

```
Phase 1: Create carapace user (GUI)
    │
    ▼
Phase 2: Configure main account (permissions)
    │
    ▼
Phase 3: Gateway infrastructure (socket IPC)  ◄── YOU ARE HERE
    │
    ▼
Phase 4: Security middleware (filtering)
    │
    ▼
Phase 5: iMessage adapter (first channel)
    │
    ▼
Phase 6: Additional channels (Signal, Discord, Gmail)
    │
    ▼
Phase 7: Production hardening
    │
    ▼
Phase 8: User experience & tooling
    │
    ▼
Phase 9: OpenClaw integration
```

---

## Phase 1: Create Carapace User ✅

**Goal:** Create the macOS user that will hold credentials

**Status:** Documentation complete, manual process

**What you do:**
1. System Settings → Users & Groups → Add "carapace"
2. Log in as carapace
3. Sign into iCloud
4. Log out

**Outcome:** A separate user account exists that can be logged into iCloud

**See:** [04-setup-carapace-user.md](04-setup-carapace-user.md)

---

## Phase 2: Configure Main Account ✅

**Goal:** Set up permissions for cross-user communication

**Status:** Documentation complete, manual process

**What you do:**
1. Create `carapace-clients` group
2. Add yourself to the group
3. Create `/var/run/carapace/` with correct permissions
4. Verify you can access the directory

**Outcome:** Your account can connect to sockets owned by carapace user

**See:** [05-setup-main-account.md](05-setup-main-account.md)

---

## Phase 3: Gateway Infrastructure 🚧

**Goal:** Establish basic communication between users via Unix socket

**Status:** IN PROGRESS

This is the foundational piece. Before we add security middleware or channel adapters, we need to understand how two processes (running as different users) communicate.

### 3.1: Minimal Daemon (Echo Server)

Build the simplest possible daemon:
- Listens on Unix socket
- Accepts connections
- Echoes back whatever it receives

```rust
// Minimal daemon - just echo
async fn main() {
    let listener = UnixListener::bind("/var/run/carapace/gateway.sock")?;

    loop {
        let (stream, _) = listener.accept().await?;
        tokio::spawn(async move {
            // Read line, echo it back
            let mut reader = BufReader::new(&stream);
            let mut writer = &stream;

            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).await? == 0 {
                    break; // EOF
                }
                writer.write_all(line.as_bytes()).await?;
            }
        });
    }
}
```

**Test:**
```bash
# Terminal 1 (as carapace): Run daemon
sudo -u carapace ./carapace-daemon

# Terminal 2 (as you): Connect and test
nc -U /var/run/carapace/gateway.sock
hello
# Should echo: hello
```

### 3.2: JSON-RPC Protocol

Add structured messaging:
- Parse JSON-RPC requests
- Return JSON-RPC responses
- Handle unknown methods gracefully

```rust
// Simple JSON-RPC handler
fn handle_request(request: &str) -> String {
    let parsed: JsonRpcRequest = serde_json::from_str(request)?;

    let response = match parsed.method.as_str() {
        "ping" => JsonRpcResponse::success(parsed.id, json!({"pong": true})),
        "echo" => JsonRpcResponse::success(parsed.id, parsed.params),
        _ => JsonRpcResponse::error(parsed.id, -32601, "Method not found"),
    };

    serde_json::to_string(&response)
}
```

**Test:**
```bash
# Send JSON-RPC request
echo '{"jsonrpc":"2.0","id":1,"method":"ping","params":{}}' | nc -U /var/run/carapace/gateway.sock
# Should return: {"jsonrpc":"2.0","id":1,"result":{"pong":true}}
```

### 3.3: Minimal Client (Shim Library)

Build a client library that shims will use:
- Connect to socket
- Send requests
- Receive responses

```rust
// Minimal client
pub struct GatewayClient {
    stream: UnixStream,
}

impl GatewayClient {
    pub fn connect() -> Result<Self> {
        let stream = UnixStream::connect("/var/run/carapace/gateway.sock")?;
        Ok(Self { stream })
    }

    pub fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        // Send request
        let request = json!({"jsonrpc":"2.0","id":1,"method":method,"params":params});
        writeln!(self.stream, "{}", serde_json::to_string(&request)?)?;

        // Read response
        let mut response = String::new();
        BufReader::new(&self.stream).read_line(&mut response)?;

        let parsed: JsonRpcResponse = serde_json::from_str(&response)?;
        Ok(parsed.result)
    }
}
```

### 3.4: Test Harness

Create a simple test shim to verify the whole flow:

```rust
// test-shim: Just calls ping and prints result
fn main() {
    let mut client = GatewayClient::connect().expect("Failed to connect");
    let result = client.call("ping", json!({})).expect("Call failed");
    println!("Response: {}", result);
}
```

**Test the full flow:**
```bash
# As carapace: daemon running
# As you: run test shim
./test-shim
# Should print: Response: {"pong":true}
```

### 3.5: Passthrough Command

Add a passthrough that executes a command on the carapace side:

```rust
// In daemon
"execute" => {
    let cmd = params["command"].as_str()?;
    let args: Vec<&str> = params["args"].as_array()?
        .iter()
        .filter_map(|v| v.as_str())
        .collect();

    let output = Command::new(cmd)
        .args(&args)
        .output()?;

    JsonRpcResponse::success(parsed.id, json!({
        "stdout": String::from_utf8_lossy(&output.stdout),
        "stderr": String::from_utf8_lossy(&output.stderr),
        "exit_code": output.status.code()
    }))
}
```

**Test:**
```bash
# Via client
echo '{"jsonrpc":"2.0","id":1,"method":"execute","params":{"command":"whoami","args":[]}}' | nc -U /var/run/carapace/gateway.sock
# Should return: {"jsonrpc":"2.0","id":1,"result":{"stdout":"carapace\n","stderr":"","exit_code":0}}
```

This proves the core concept: **your account sends a request, carapace account executes it**.

### Phase 3 Deliverables

- [ ] Daemon that listens on Unix socket
- [ ] JSON-RPC request/response handling
- [ ] Client library for connecting to daemon
- [ ] Test shim that verifies connectivity
- [ ] Passthrough command execution
- [ ] Documentation of what we learned

**See:** [13-gateway-infrastructure.md](13-gateway-infrastructure.md) (to be created)

---

## Phase 4: Security Middleware

**Goal:** Add security controls between request and execution

**Status:** Planned

Once Phase 3 works, we add security layers:

### 4.1: Request Logging
- Log every request to audit file
- Include timestamp, method, params

### 4.2: Rate Limiting
- Track requests per time window
- Reject when limit exceeded
- Count ALL attempts (prevent probing)

### 4.3: Allowlist Validation
- Configure allowed commands/recipients
- Check before execution
- Return clear error when blocked

### 4.4: Content Filtering
- Scan request content for patterns
- Block sensitive content
- Log matches

### 4.5: Dead Letter Queue
- Store blocked request metadata
- Allow review of what was blocked

**Outcome:** Same passthrough, but with security controls

---

## Phase 5: iMessage Adapter

**Goal:** Replace generic passthrough with iMessage-specific handling

**Status:** Planned

### 5.1: Send Messages
- Parse imsg send command format
- Execute via real imsg
- Format response

### 5.2: List Chats
- Query via imsg chats
- Filter by allowlist
- Return formatted list

### 5.3: Get History
- Query specific chat
- Filter messages
- Return history

### 5.4: Watch (Streaming)
- Subscribe to new messages
- Filter inbound by allowlist
- Stream to client

### 5.5: imsg Shim
- Full CLI compatibility
- All imsg commands supported

**Outcome:** Drop-in replacement for imsg that goes through gateway

---

## Phase 6: Additional Channels

**Goal:** Support Signal, Discord, Gmail

**Status:** Planned

- [ ] Signal adapter (signal-cli)
- [ ] Signal shim
- [ ] Discord adapter (serenity)
- [ ] Discord shim
- [ ] Gmail adapter
- [ ] Gmail shim

---

## Phase 7: Production Hardening

**Goal:** Make it reliable and secure for real use

**Status:** Planned

- [ ] Daemon auto-restart
- [ ] Crash recovery
- [ ] Config validation
- [ ] Security audit
- [ ] Performance testing

---

## Phase 8: User Experience

**Goal:** Make setup easy

**Status:** Planned

- [ ] Install script
- [ ] Management CLI (`carapace-ctl`)
- [ ] Homebrew formula
- [ ] Documentation improvements

---

## Phase 9: OpenClaw Integration

**Goal:** Native OpenClaw support

**Status:** Planned

- [ ] OpenClaw plugin
- [ ] `openclaw setup --mode zero-trust`
- [ ] Upstream contribution

---

## Current Focus: Phase 3

**What to build next:**

1. **Minimal daemon** (echo server) - ~30 lines of Rust
2. **JSON-RPC parsing** - add structured protocol
3. **Client library** - reusable connection code
4. **Test shim** - verify end-to-end
5. **Passthrough execution** - prove the concept

This gives us the foundation everything else builds on.

---

## Contributing

Want to help? Phase 3 is the best place to start:

1. Fork the repo
2. Build the minimal daemon
3. Test with `nc` or the test shim
4. Add JSON-RPC handling
5. Submit PR

The goal is small, focused PRs that each add one piece.
