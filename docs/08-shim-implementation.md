# Shim Implementation

Shims are lightweight CLI tools that mimic the interface of real tools (imsg, signal-cli, etc.) but redirect all operations through the Carapace gateway.

## Design Goals

1. **Exact CLI Compatibility**: Users and tools should not notice they're using a shim
2. **Minimal Footprint**: Small binary, fast startup, no dependencies
3. **Clear Error Messages**: When blocked, explain why
4. **Graceful Failure**: Handle daemon unavailability gracefully

## Project Structure

```
carapace-shims/
├── Cargo.toml
├── src/
│   ├── lib.rs           # Shared client library
│   ├── client.rs        # Gateway client
│   ├── bin/
│   │   ├── imsg.rs      # imsg shim
│   │   ├── signal.rs    # signal-cli shim
│   │   ├── discord.rs   # discord shim
│   │   └── gmail.rs     # gog/gmail shim
│   └── formatters/
│       ├── mod.rs
│       ├── imsg.rs      # Format output like real imsg
│       └── signal.rs    # Format output like real signal-cli
```

## Gateway Client (`client.rs`)

```rust
use std::os::unix::net::UnixStream;
use std::io::{BufRead, BufReader, Write};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const SOCKET_PATH: &str = "/var/run/carapace/gateway.sock";

pub struct GatewayClient {
    stream: UnixStream,
    reader: BufReader<UnixStream>,
    next_id: u64,
}

impl GatewayClient {
    pub fn connect() -> Result<Self, ClientError> {
        let stream = UnixStream::connect(SOCKET_PATH)
            .map_err(|e| ClientError::Connection(e.to_string()))?;

        let reader = BufReader::new(stream.try_clone()?);

        Ok(Self {
            stream,
            reader,
            next_id: 1,
        })
    }

    pub fn call(&mut self, method: &str, params: Value) -> Result<Value, ClientError> {
        let id = self.next_id;
        self.next_id += 1;

        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        });

        // Send request
        let mut line = serde_json::to_string(&request)?;
        line.push('\n');
        self.stream.write_all(line.as_bytes())?;
        self.stream.flush()?;

        // Read response
        let mut response_line = String::new();
        self.reader.read_line(&mut response_line)?;

        let response: JsonRpcResponse = serde_json::from_str(&response_line)?;

        if response.id != serde_json::json!(id) {
            return Err(ClientError::Protocol("Response ID mismatch".into()));
        }

        if let Some(error) = response.error {
            return Err(ClientError::Gateway {
                code: error.code,
                message: error.message,
            });
        }

        response.result.ok_or(ClientError::Protocol("No result in response".into()))
    }

    /// Subscribe to streaming events (returns iterator)
    pub fn subscribe(&mut self, method: &str, params: Value) -> Result<EventStream, ClientError> {
        // Send subscription request
        let result = self.call(method, params)?;

        // Return event stream
        Ok(EventStream {
            reader: &mut self.reader,
        })
    }
}

pub struct EventStream<'a> {
    reader: &'a mut BufReader<UnixStream>,
}

impl<'a> Iterator for EventStream<'a> {
    type Item = Result<Value, ClientError>;

    fn next(&mut self) -> Option<Self::Item> {
        let mut line = String::new();
        match self.reader.read_line(&mut line) {
            Ok(0) => None, // EOF
            Ok(_) => {
                match serde_json::from_str::<JsonRpcNotification>(&line) {
                    Ok(notif) => Some(Ok(notif.params)),
                    Err(e) => Some(Err(ClientError::Parse(e.to_string()))),
                }
            }
            Err(e) => Some(Err(ClientError::Io(e.to_string()))),
        }
    }
}

#[derive(Debug)]
pub enum ClientError {
    Connection(String),
    Io(String),
    Parse(String),
    Protocol(String),
    Gateway { code: i32, message: String },
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientError::Connection(e) => write!(f, "Connection failed: {}", e),
            ClientError::Io(e) => write!(f, "IO error: {}", e),
            ClientError::Parse(e) => write!(f, "Parse error: {}", e),
            ClientError::Protocol(e) => write!(f, "Protocol error: {}", e),
            ClientError::Gateway { message, .. } => write!(f, "Error: {}", message),
        }
    }
}
```

## imsg Shim (`bin/imsg.rs`)

```rust
use clap::{Parser, Subcommand};
use carapace_shims::{GatewayClient, ClientError};
use serde_json::json;
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "imsg", about = "Send and receive iMessages")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Send a message
    Send {
        /// Recipient (phone number or email)
        recipient: String,

        /// Message text
        message: String,

        /// Attachment file paths
        #[arg(short, long)]
        attachment: Vec<String>,
    },

    /// List recent chats
    Chats {
        /// Maximum number of chats
        #[arg(short, long, default_value = "20")]
        limit: u32,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Get message history for a chat
    History {
        /// Chat ID
        chat_id: String,

        /// Maximum number of messages
        #[arg(short, long, default_value = "50")]
        limit: u32,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Watch for new messages
    Watch {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Get contact info
    Contact {
        /// Phone number or email
        identifier: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    let mut client = match GatewayClient::connect() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to connect to Carapace gateway: {}", e);
            eprintln!("Is the carapace-daemon running?");
            return ExitCode::from(1);
        }
    };

    let result = match cli.command {
        Commands::Send { recipient, message, attachment } => {
            handle_send(&mut client, recipient, message, attachment)
        }
        Commands::Chats { limit, json } => {
            handle_chats(&mut client, limit, json)
        }
        Commands::History { chat_id, limit, json } => {
            handle_history(&mut client, chat_id, limit, json)
        }
        Commands::Watch { json } => {
            handle_watch(&mut client, json)
        }
        Commands::Contact { identifier, json } => {
            handle_contact(&mut client, identifier, json)
        }
    };

    match result {
        Ok(_) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{}", e);
            ExitCode::from(1)
        }
    }
}

fn handle_send(
    client: &mut GatewayClient,
    recipient: String,
    message: String,
    attachments: Vec<String>,
) -> Result<(), ClientError> {
    let params = json!({
        "channel": "imsg",
        "recipient": recipient,
        "message": message,
        "attachments": attachments.iter().map(|a| json!({"path": a})).collect::<Vec<_>>()
    });

    let result = client.call("channel.send", params)?;

    // Format output like real imsg
    if let Some(msg_id) = result.get("message_id") {
        println!("Message sent (ID: {})", msg_id);
    } else {
        println!("Message sent");
    }

    Ok(())
}

fn handle_chats(
    client: &mut GatewayClient,
    limit: u32,
    json_output: bool,
) -> Result<(), ClientError> {
    let params = json!({
        "channel": "imsg",
        "limit": limit
    });

    let result = client.call("channel.list_chats", params)?;

    if json_output {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        // Format like real imsg output
        if let Some(chats) = result.get("chats").and_then(|c| c.as_array()) {
            for chat in chats {
                let display = chat.get("display_name").and_then(|n| n.as_str()).unwrap_or("Unknown");
                let last_msg = chat.get("last_message").and_then(|m| m.as_str()).unwrap_or("");
                let id = chat.get("id").and_then(|i| i.as_str()).unwrap_or("");

                // Truncate last message
                let preview: String = last_msg.chars().take(50).collect();
                println!("{} ({}): {}", display, id, preview);
            }
        }
    }

    Ok(())
}

fn handle_history(
    client: &mut GatewayClient,
    chat_id: String,
    limit: u32,
    json_output: bool,
) -> Result<(), ClientError> {
    let params = json!({
        "channel": "imsg",
        "chat_id": chat_id,
        "limit": limit
    });

    let result = client.call("channel.get_history", params)?;

    if json_output {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        if let Some(messages) = result.get("messages").and_then(|m| m.as_array()) {
            for msg in messages {
                let sender = msg.get("sender").and_then(|s| s.as_str()).unwrap_or("Unknown");
                let text = msg.get("text").and_then(|t| t.as_str()).unwrap_or("");
                let time = msg.get("timestamp").and_then(|t| t.as_str()).unwrap_or("");
                let from_me = msg.get("is_from_me").and_then(|f| f.as_bool()).unwrap_or(false);

                let direction = if from_me { "→" } else { "←" };
                println!("[{}] {} {}: {}", time, direction, sender, text);
            }
        }
    }

    Ok(())
}

fn handle_watch(
    client: &mut GatewayClient,
    json_output: bool,
) -> Result<(), ClientError> {
    let params = json!({
        "channel": "imsg"
    });

    let events = client.subscribe("channel.watch", params)?;

    for event in events {
        match event {
            Ok(msg) => {
                if json_output {
                    println!("{}", serde_json::to_string(&msg)?);
                } else {
                    let sender = msg.get("sender").and_then(|s| s.as_str()).unwrap_or("Unknown");
                    let text = msg.get("text").and_then(|t| t.as_str()).unwrap_or("");
                    println!("{}: {}", sender, text);
                }
            }
            Err(e) => {
                eprintln!("Error: {}", e);
            }
        }
    }

    Ok(())
}

fn handle_contact(
    client: &mut GatewayClient,
    identifier: String,
    json_output: bool,
) -> Result<(), ClientError> {
    // Contact lookup could be implemented similarly
    eprintln!("Contact lookup not yet implemented");
    Ok(())
}
```

## Signal Shim (`bin/signal.rs`)

```rust
use clap::{Parser, Subcommand};
use carapace_shims::{GatewayClient, ClientError};
use serde_json::json;

#[derive(Parser)]
#[command(name = "signal-cli", about = "Send and receive Signal messages")]
struct Cli {
    /// Account phone number
    #[arg(short, long)]
    account: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Send a message
    Send {
        /// Message text
        #[arg(short, long)]
        message: String,

        /// Recipient phone number(s)
        #[arg(trailing_var_arg = true)]
        recipients: Vec<String>,

        /// Send to group
        #[arg(short, long)]
        group: Option<String>,

        /// Attachment file paths
        #[arg(short, long)]
        attachment: Vec<String>,
    },

    /// Receive messages
    Receive {
        /// Output as JSON
        #[arg(long)]
        json: bool,

        /// Timeout in seconds
        #[arg(short, long, default_value = "5")]
        timeout: u32,
    },
}

fn main() -> std::process::ExitCode {
    let cli = Cli::parse();

    let mut client = match GatewayClient::connect() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to connect: {}", e);
            return std::process::ExitCode::from(1);
        }
    };

    let result = match cli.command {
        Commands::Send { message, recipients, group, attachment } => {
            handle_send(&mut client, message, recipients, group, attachment)
        }
        Commands::Receive { json, timeout } => {
            handle_receive(&mut client, json, timeout)
        }
    };

    match result {
        Ok(_) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{}", e);
            std::process::ExitCode::from(1)
        }
    }
}

fn handle_send(
    client: &mut GatewayClient,
    message: String,
    recipients: Vec<String>,
    group: Option<String>,
    attachments: Vec<String>,
) -> Result<(), ClientError> {
    let recipient = if let Some(g) = group {
        format!("group:{}", g)
    } else if recipients.len() == 1 {
        recipients[0].clone()
    } else {
        return Err(ClientError::Protocol("Specify one recipient or --group".into()));
    };

    let params = json!({
        "channel": "signal",
        "recipient": recipient,
        "message": message,
        "attachments": attachments.iter().map(|a| json!({"path": a})).collect::<Vec<_>>()
    });

    client.call("channel.send", params)?;
    println!("Message sent");

    Ok(())
}

fn handle_receive(
    client: &mut GatewayClient,
    json_output: bool,
    _timeout: u32,
) -> Result<(), ClientError> {
    let params = json!({
        "channel": "signal"
    });

    let events = client.subscribe("channel.watch", params)?;

    for event in events {
        match event {
            Ok(msg) => {
                if json_output {
                    println!("{}", serde_json::to_string(&msg)?);
                } else {
                    let sender = msg.get("sender").and_then(|s| s.as_str()).unwrap_or("Unknown");
                    let text = msg.get("text").and_then(|t| t.as_str()).unwrap_or("");
                    println!("Envelope from: {}", sender);
                    println!("Body: {}", text);
                    println!();
                }
            }
            Err(e) => {
                eprintln!("Error: {}", e);
            }
        }
    }

    Ok(())
}
```

## Building Shims

### Cargo.toml

```toml
[package]
name = "carapace-shims"
version = "0.1.0"
edition = "2021"

[lib]
name = "carapace_shims"
path = "src/lib.rs"

[[bin]]
name = "imsg-shim"
path = "src/bin/imsg.rs"

[[bin]]
name = "signal-shim"
path = "src/bin/signal.rs"

[[bin]]
name = "discord-shim"
path = "src/bin/discord.rs"

[[bin]]
name = "gmail-shim"
path = "src/bin/gmail.rs"

[dependencies]
clap = { version = "4", features = ["derive"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
```

### Build Commands

```bash
# Build all shims
cargo build --release

# Install to system
sudo cp target/release/imsg-shim /usr/local/carapace/bin/imsg
sudo cp target/release/signal-shim /usr/local/carapace/bin/signal-cli
# etc.
```

## Testing Shims

### Manual Testing

```bash
# Test connection
./target/release/imsg-shim chats --limit 5

# Test send (to allowlisted recipient)
./target/release/imsg-shim send "+1234567890" "Test message"

# Test blocked send
./target/release/imsg-shim send "+9999999999" "Should fail"
# Expected: Error: Recipient not in allowlist
```

### Compatibility Testing

Compare shim output to real tool output:

```bash
# Real tool
/opt/homebrew/bin/imsg chats --limit 5 > real_output.txt

# Shim (with gateway running)
/usr/local/carapace/bin/imsg chats --limit 5 > shim_output.txt

# Compare (should be similar format)
diff real_output.txt shim_output.txt
```
