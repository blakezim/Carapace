# Phase 3: Gateway Infrastructure

This document covers the foundational inter-process communication (IPC) between users. This is the core of Carapace - everything else builds on top of this.

## Goal

Establish reliable communication between:
- **Your account** (client/shim)
- **Carapace account** (daemon/server)

By the end of this phase, you'll have:
1. A daemon running as carapace that listens on a Unix socket
2. A client library that connects from your account
3. Proof that commands execute as the carapace user

---

## Prerequisites

Before starting, complete:
- [Phase 1: Create Carapace User](04-setup-carapace-user.md)
- [Phase 2: Configure Main Account](05-setup-main-account.md)

Verify setup:
```bash
# Socket directory exists with correct permissions
ls -la /var/run/carapace/
# drwxr-x--- carapace carapace-clients

# You're in the right group
groups $(whoami) | grep carapace-clients
```

---

## Understanding Unix Domain Sockets

Unix domain sockets are like network sockets, but for local communication only. They appear as files in the filesystem.

### Why Unix Sockets?

| Feature | Unix Socket | TCP Localhost | Named Pipe |
|---------|-------------|---------------|------------|
| Network exposure | None | Loopback only | None |
| Access control | File permissions | Port binding | File permissions |
| Performance | Fast (no network stack) | Slower | Fast |
| Bidirectional | Yes | Yes | Half-duplex |

### How They Work

```
SERVER (carapace user)                CLIENT (your user)
━━━━━━━━━━━━━━━━━━━━━━                ━━━━━━━━━━━━━━━━━━━━

1. bind("/var/run/carapace/gateway.sock")
   Creates socket file

2. listen()                           3. connect("/var/run/carapace/gateway.sock")
   Waits for connections                 Opens connection

4. accept()
   Returns connected stream

5. read() ◄──────────────────────────── write("hello")
                                         Sends data

6. write("hello back") ──────────────► read()
   Sends response                        Receives data

7. close()                              close()
```

### Permission Model

```
/var/run/carapace/gateway.sock
│
├── Owner: carapace        → Can read/write/delete
├── Group: carapace-clients → Can read/write (connect)
└── Others: (none)         → Cannot access
```

---

## Step 3.1: Minimal Echo Daemon

Start with the simplest possible working daemon.

### Project Setup

```bash
# As carapace user (or in a shared location)
mkdir -p ~/carapace-daemon
cd ~/carapace-daemon

cargo init --name carapace-daemon
```

### Cargo.toml

```toml
[package]
name = "carapace-daemon"
version = "0.1.0"
edition = "2021"

[dependencies]
tokio = { version = "1", features = ["full"] }
```

### src/main.rs (Echo Version)

```rust
use std::os::unix::fs::PermissionsExt;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;

const SOCKET_PATH: &str = "/var/run/carapace/gateway.sock";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Remove stale socket if it exists
    let _ = std::fs::remove_file(SOCKET_PATH);

    // Bind to socket
    let listener = UnixListener::bind(SOCKET_PATH)?;

    // Set permissions: owner + group can read/write
    std::fs::set_permissions(
        SOCKET_PATH,
        std::fs::Permissions::from_mode(0o770),
    )?;

    println!("Daemon listening on {}", SOCKET_PATH);

    // Accept connections
    loop {
        match listener.accept().await {
            Ok((stream, _addr)) => {
                println!("Client connected");

                tokio::spawn(async move {
                    let (reader, mut writer) = stream.into_split();
                    let mut reader = BufReader::new(reader);
                    let mut line = String::new();

                    loop {
                        line.clear();
                        match reader.read_line(&mut line).await {
                            Ok(0) => {
                                println!("Client disconnected");
                                break;
                            }
                            Ok(_) => {
                                println!("Received: {}", line.trim());
                                // Echo back
                                if let Err(e) = writer.write_all(line.as_bytes()).await {
                                    eprintln!("Write error: {}", e);
                                    break;
                                }
                            }
                            Err(e) => {
                                eprintln!("Read error: {}", e);
                                break;
                            }
                        }
                    }
                });
            }
            Err(e) => {
                eprintln!("Accept error: {}", e);
            }
        }
    }
}
```

### Build and Test

```bash
# Build
cargo build --release

# Run as carapace user
sudo -u carapace ./target/release/carapace-daemon

# In another terminal, test with netcat
nc -U /var/run/carapace/gateway.sock
hello
# Should echo: hello
world
# Should echo: world
```

**What you learned:**
- How to bind a Unix socket
- How to set socket permissions
- How to handle multiple connections with tokio

---

## Step 3.2: Add JSON-RPC Protocol

Now let's add structured messaging.

### Update Cargo.toml

```toml
[dependencies]
tokio = { version = "1", features = ["full"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

### Create src/protocol.rs

```rust
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: Value,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
}

impl JsonRpcResponse {
    pub fn success(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn error(id: Value, code: i32, message: &str) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.to_string(),
            }),
        }
    }
}
```

### Update src/main.rs

```rust
mod protocol;

use protocol::{JsonRpcRequest, JsonRpcResponse};
use serde_json::json;
use std::os::unix::fs::PermissionsExt;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;

const SOCKET_PATH: &str = "/var/run/carapace/gateway.sock";

fn handle_request(request: &JsonRpcRequest) -> JsonRpcResponse {
    match request.method.as_str() {
        "ping" => {
            JsonRpcResponse::success(request.id.clone(), json!({"pong": true}))
        }
        "echo" => {
            JsonRpcResponse::success(request.id.clone(), request.params.clone())
        }
        "whoami" => {
            // Return the user running the daemon
            let username = std::env::var("USER").unwrap_or_else(|_| "unknown".to_string());
            JsonRpcResponse::success(request.id.clone(), json!({"user": username}))
        }
        _ => {
            JsonRpcResponse::error(request.id.clone(), -32601, "Method not found")
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = std::fs::remove_file(SOCKET_PATH);
    let listener = UnixListener::bind(SOCKET_PATH)?;
    std::fs::set_permissions(SOCKET_PATH, std::fs::Permissions::from_mode(0o770))?;

    println!("Daemon listening on {}", SOCKET_PATH);

    loop {
        let (stream, _) = listener.accept().await?;
        println!("Client connected");

        tokio::spawn(async move {
            let (reader, mut writer) = stream.into_split();
            let mut reader = BufReader::new(reader);
            let mut line = String::new();

            loop {
                line.clear();
                match reader.read_line(&mut line).await {
                    Ok(0) => break,
                    Ok(_) => {
                        let response = match serde_json::from_str::<JsonRpcRequest>(&line) {
                            Ok(request) => {
                                println!("Request: {} ({})", request.method, request.id);
                                handle_request(&request)
                            }
                            Err(e) => {
                                JsonRpcResponse::error(
                                    json!(null),
                                    -32700,
                                    &format!("Parse error: {}", e),
                                )
                            }
                        };

                        let response_str = serde_json::to_string(&response).unwrap() + "\n";
                        if let Err(e) = writer.write_all(response_str.as_bytes()).await {
                            eprintln!("Write error: {}", e);
                            break;
                        }
                    }
                    Err(e) => {
                        eprintln!("Read error: {}", e);
                        break;
                    }
                }
            }
            println!("Client disconnected");
        });
    }
}
```

### Test JSON-RPC

```bash
# Rebuild and run
cargo build --release
sudo -u carapace ./target/release/carapace-daemon

# Test ping
echo '{"jsonrpc":"2.0","id":1,"method":"ping","params":{}}' | nc -U /var/run/carapace/gateway.sock
# {"jsonrpc":"2.0","id":1,"result":{"pong":true}}

# Test echo
echo '{"jsonrpc":"2.0","id":2,"method":"echo","params":{"msg":"hello"}}' | nc -U /var/run/carapace/gateway.sock
# {"jsonrpc":"2.0","id":2,"result":{"msg":"hello"}}

# Test whoami - THIS IS THE KEY TEST
echo '{"jsonrpc":"2.0","id":3,"method":"whoami","params":{}}' | nc -U /var/run/carapace/gateway.sock
# {"jsonrpc":"2.0","id":3,"result":{"user":"carapace"}}
```

**The `whoami` test proves it**: you're running `nc` as your user, but the daemon responds with `carapace`. Cross-user communication is working!

---

## Step 3.3: Client Library

Now build a reusable client library.

### Create Client Project

```bash
mkdir -p ~/carapace-client
cd ~/carapace-client
cargo init --name carapace-client --lib
```

### Cargo.toml

```toml
[package]
name = "carapace-client"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "1"
```

### src/lib.rs

```rust
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use thiserror::Error;

const DEFAULT_SOCKET_PATH: &str = "/var/run/carapace/gateway.sock";

#[derive(Error, Debug)]
pub enum ClientError {
    #[error("Connection failed: {0}")]
    Connection(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Gateway error ({code}): {message}")]
    Gateway { code: i32, message: String },

    #[error("Protocol error: {0}")]
    Protocol(String),
}

#[derive(Debug, Serialize)]
struct Request {
    jsonrpc: String,
    id: u64,
    method: String,
    params: Value,
}

#[derive(Debug, Deserialize)]
struct Response {
    #[allow(dead_code)]
    jsonrpc: String,
    id: Value,
    result: Option<Value>,
    error: Option<RpcError>,
}

#[derive(Debug, Deserialize)]
struct RpcError {
    code: i32,
    message: String,
}

pub struct GatewayClient {
    stream: UnixStream,
    reader: BufReader<UnixStream>,
    next_id: u64,
}

impl GatewayClient {
    /// Connect to the gateway socket
    pub fn connect() -> Result<Self, ClientError> {
        Self::connect_to(DEFAULT_SOCKET_PATH)
    }

    /// Connect to a specific socket path
    pub fn connect_to(path: &str) -> Result<Self, ClientError> {
        let stream = UnixStream::connect(path)?;
        let reader = BufReader::new(stream.try_clone()?);

        Ok(Self {
            stream,
            reader,
            next_id: 1,
        })
    }

    /// Call a method on the gateway
    pub fn call(&mut self, method: &str, params: Value) -> Result<Value, ClientError> {
        let id = self.next_id;
        self.next_id += 1;

        // Build request
        let request = Request {
            jsonrpc: "2.0".to_string(),
            id,
            method: method.to_string(),
            params,
        };

        // Send request
        let request_str = serde_json::to_string(&request)? + "\n";
        self.stream.write_all(request_str.as_bytes())?;
        self.stream.flush()?;

        // Read response
        let mut response_str = String::new();
        self.reader.read_line(&mut response_str)?;

        let response: Response = serde_json::from_str(&response_str)?;

        // Check for error
        if let Some(error) = response.error {
            return Err(ClientError::Gateway {
                code: error.code,
                message: error.message,
            });
        }

        response
            .result
            .ok_or_else(|| ClientError::Protocol("No result in response".to_string()))
    }

    /// Convenience method for ping
    pub fn ping(&mut self) -> Result<bool, ClientError> {
        let result = self.call("ping", serde_json::json!({}))?;
        Ok(result.get("pong").and_then(|v| v.as_bool()).unwrap_or(false))
    }
}
```

---

## Step 3.4: Test Shim

Create a simple binary that uses the client library.

### Create Test Shim Project

```bash
mkdir -p ~/carapace-test-shim
cd ~/carapace-test-shim
cargo init --name test-shim
```

### Cargo.toml

```toml
[package]
name = "test-shim"
version = "0.1.0"
edition = "2021"

[dependencies]
carapace-client = { path = "../carapace-client" }
serde_json = "1"
```

### src/main.rs

```rust
use carapace_client::GatewayClient;
use serde_json::json;

fn main() {
    println!("Connecting to gateway...");

    let mut client = match GatewayClient::connect() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to connect: {}", e);
            std::process::exit(1);
        }
    };

    println!("Connected!\n");

    // Test 1: Ping
    println!("Test 1: ping");
    match client.ping() {
        Ok(true) => println!("  ✓ Pong received\n"),
        Ok(false) => println!("  ✗ Unexpected response\n"),
        Err(e) => println!("  ✗ Error: {}\n", e),
    }

    // Test 2: Echo
    println!("Test 2: echo");
    match client.call("echo", json!({"message": "Hello, Carapace!"})) {
        Ok(result) => println!("  ✓ Echo: {}\n", result),
        Err(e) => println!("  ✗ Error: {}\n", e),
    }

    // Test 3: Whoami (proves cross-user execution)
    println!("Test 3: whoami");
    match client.call("whoami", json!({})) {
        Ok(result) => {
            let user = result.get("user").and_then(|v| v.as_str()).unwrap_or("unknown");
            println!("  ✓ Daemon running as: {}", user);
            if user == "carapace" {
                println!("  ✓ Cross-user communication working!\n");
            } else {
                println!("  ⚠ Expected 'carapace', got '{}'\n", user);
            }
        }
        Err(e) => println!("  ✗ Error: {}\n", e),
    }

    // Test 4: Unknown method (should error)
    println!("Test 4: unknown method");
    match client.call("nonexistent", json!({})) {
        Ok(_) => println!("  ✗ Should have failed\n"),
        Err(e) => println!("  ✓ Expected error: {}\n", e),
    }

    println!("All tests complete!");
}
```

### Run Test

```bash
# Make sure daemon is running
sudo -u carapace ~/carapace-daemon/target/release/carapace-daemon &

# Run test shim (as your user)
cargo run

# Expected output:
# Connecting to gateway...
# Connected!
#
# Test 1: ping
#   ✓ Pong received
#
# Test 2: echo
#   ✓ Echo: {"message":"Hello, Carapace!"}
#
# Test 3: whoami
#   ✓ Daemon running as: carapace
#   ✓ Cross-user communication working!
#
# Test 4: unknown method
#   ✓ Expected error: Gateway error (-32601): Method not found
#
# All tests complete!
```

---

## Step 3.5: Command Passthrough

Now add the ability to execute commands on the carapace side.

### Update Daemon (src/main.rs)

Add to the `handle_request` function:

```rust
"execute" => {
    // Extract command and args from params
    let command = match request.params.get("command").and_then(|v| v.as_str()) {
        Some(c) => c,
        None => return JsonRpcResponse::error(
            request.id.clone(),
            -32602,
            "Missing 'command' parameter"
        ),
    };

    let args: Vec<&str> = request.params
        .get("args")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .collect()
        })
        .unwrap_or_default();

    // Execute the command
    match std::process::Command::new(command)
        .args(&args)
        .output()
    {
        Ok(output) => {
            JsonRpcResponse::success(request.id.clone(), json!({
                "stdout": String::from_utf8_lossy(&output.stdout),
                "stderr": String::from_utf8_lossy(&output.stderr),
                "exit_code": output.status.code()
            }))
        }
        Err(e) => {
            JsonRpcResponse::error(
                request.id.clone(),
                -32000,
                &format!("Execution failed: {}", e)
            )
        }
    }
}
```

### Test Passthrough

```bash
# Rebuild and restart daemon
cargo build --release
sudo -u carapace ./target/release/carapace-daemon

# Test command execution
echo '{"jsonrpc":"2.0","id":1,"method":"execute","params":{"command":"whoami","args":[]}}' | nc -U /var/run/carapace/gateway.sock
# {"jsonrpc":"2.0","id":1,"result":{"stdout":"carapace\n","stderr":"","exit_code":0}}

echo '{"jsonrpc":"2.0","id":2,"method":"execute","params":{"command":"id","args":[]}}' | nc -U /var/run/carapace/gateway.sock
# {"jsonrpc":"2.0","id":2,"result":{"stdout":"uid=502(carapace) gid=20(staff)...\n","stderr":"","exit_code":0}}

echo '{"jsonrpc":"2.0","id":3,"method":"execute","params":{"command":"ls","args":["-la","/Users/carapace"]}}' | nc -U /var/run/carapace/gateway.sock
# Lists carapace's home directory!
```

**This is the foundation!** You can now execute commands as the carapace user from your account.

---

## What We Built

```
┌─────────────────────────────────────────────────────────────┐
│                    PHASE 3 COMPLETE                          │
├─────────────────────────────────────────────────────────────┤
│                                                              │
│   YOUR ACCOUNT                      CARAPACE ACCOUNT         │
│                                                              │
│   ┌─────────────────┐              ┌─────────────────────┐  │
│   │  test-shim      │              │  carapace-daemon    │  │
│   │  (or any        │   Socket     │                     │  │
│   │   client)       │◄────────────►│  - Echo             │  │
│   │                 │              │  - Ping             │  │
│   │  Uses:          │              │  - Whoami           │  │
│   │  carapace-client│              │  - Execute          │  │
│   │  library        │              │                     │  │
│   └─────────────────┘              └─────────────────────┘  │
│                                                              │
│   Socket: /var/run/carapace/gateway.sock                    │
│   Protocol: JSON-RPC 2.0 over newline-delimited JSON        │
│                                                              │
└─────────────────────────────────────────────────────────────┘
```

## Next Steps

With the gateway infrastructure in place, Phase 4 adds security:

1. **Audit logging** - Log every request
2. **Rate limiting** - Prevent abuse
3. **Allowlist** - Control what can be executed
4. **Content filtering** - Block sensitive patterns

See [12-roadmap.md](12-roadmap.md) for the full plan.

---

## Troubleshooting

### "Connection refused"

```bash
# Is daemon running?
ps aux | grep carapace-daemon

# Does socket exist?
ls -la /var/run/carapace/gateway.sock
```

### "Permission denied"

```bash
# Are you in the right group?
groups $(whoami) | grep carapace-clients

# If not, add yourself and re-login
sudo dseditgroup -o edit -a $(whoami) -t user carapace-clients
```

### Socket permissions wrong

```bash
# Check permissions
ls -la /var/run/carapace/gateway.sock
# Should be: srwxrwx---

# Fix if needed (daemon should set this on startup)
sudo -u carapace chmod 770 /var/run/carapace/gateway.sock
```
