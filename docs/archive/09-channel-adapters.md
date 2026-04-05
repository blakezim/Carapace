# Channel Adapters

Channel adapters are the daemon components that interact with real messaging tools. Each adapter implements a common interface and handles channel-specific details.

## Adapter Interface

```rust
#[async_trait]
pub trait ChannelAdapter: Send + Sync {
    /// Get channel identifier
    fn channel_id(&self) -> &str;

    /// Check if channel is healthy
    async fn health_check(&self) -> ChannelStatus;

    /// Send a message
    async fn send(&self, params: &SendParams) -> Result<SendResult>;

    /// List conversations
    async fn list_chats(&self, limit: u32, offset: u32) -> Result<ChatsResult>;

    /// Get message history
    async fn get_history(&self, chat_id: &str, limit: u32, before: Option<&str>) -> Result<HistoryResult>;

    /// Subscribe to incoming messages
    async fn watch(&self) -> Result<Box<dyn Stream<Item = IncomingMessage> + Send>>;
}
```

---

## iMessage Adapter

The iMessage adapter wraps the `imsg` CLI tool.

### Configuration

```toml
[channels.imsg]
enabled = true
real_binary = "/opt/homebrew/bin/imsg"
db_path = "/Users/carapace/Library/Messages/chat.db"
```

### Implementation

```rust
pub struct ImsgAdapter {
    binary_path: PathBuf,
    db_path: PathBuf,
}

impl ImsgAdapter {
    pub fn new(config: &ImsgConfig) -> Result<Self> {
        // Verify binary exists
        if !config.real_binary.exists() {
            return Err(Error::BinaryNotFound(config.real_binary.clone()));
        }

        // Verify database exists
        if !config.db_path.exists() {
            return Err(Error::DatabaseNotFound(config.db_path.clone()));
        }

        Ok(Self {
            binary_path: config.real_binary.clone(),
            db_path: config.db_path.clone(),
        })
    }
}

#[async_trait]
impl ChannelAdapter for ImsgAdapter {
    fn channel_id(&self) -> &str {
        "imsg"
    }

    async fn send(&self, params: &SendParams) -> Result<SendResult> {
        let mut cmd = Command::new(&self.binary_path);
        cmd.args(["send", &params.recipient, &params.message]);

        // Add attachments
        for attachment in &params.attachments {
            cmd.args(["--attachment", &attachment.path]);
        }

        let output = cmd.output().await?;

        if output.status.success() {
            Ok(SendResult {
                success: true,
                message_id: parse_message_id(&output.stdout),
                timestamp: Utc::now(),
            })
        } else {
            Err(Error::SendFailed(String::from_utf8_lossy(&output.stderr).to_string()))
        }
    }

    async fn list_chats(&self, limit: u32, _offset: u32) -> Result<ChatsResult> {
        let output = Command::new(&self.binary_path)
            .args(["chats", "--limit", &limit.to_string(), "--json"])
            .output()
            .await?;

        let chats: Vec<ImsgChat> = serde_json::from_slice(&output.stdout)?;

        Ok(ChatsResult {
            chats: chats.into_iter().map(Chat::from).collect(),
            total: chats.len() as u32,
            has_more: chats.len() as u32 == limit,
        })
    }

    async fn get_history(&self, chat_id: &str, limit: u32, _before: Option<&str>) -> Result<HistoryResult> {
        let output = Command::new(&self.binary_path)
            .args(["history", chat_id, "--limit", &limit.to_string(), "--json"])
            .output()
            .await?;

        let messages: Vec<ImsgMessage> = serde_json::from_slice(&output.stdout)?;

        Ok(HistoryResult {
            messages: messages.into_iter().map(Message::from).collect(),
            has_more: messages.len() as u32 == limit,
        })
    }

    async fn watch(&self) -> Result<Box<dyn Stream<Item = IncomingMessage> + Send>> {
        let mut child = Command::new(&self.binary_path)
            .args(["watch", "--json"])
            .stdout(Stdio::piped())
            .spawn()?;

        let stdout = child.stdout.take().unwrap();
        let reader = BufReader::new(stdout);

        let stream = reader
            .lines()
            .filter_map(|line| async move {
                let line = line.ok()?;
                let msg: ImsgWatchEvent = serde_json::from_str(&line).ok()?;
                Some(IncomingMessage::from(msg))
            });

        Ok(Box::new(stream))
    }

    async fn health_check(&self) -> ChannelStatus {
        // Check binary
        if !self.binary_path.exists() {
            return ChannelStatus::Unhealthy("Binary not found".into());
        }

        // Check database
        if !self.db_path.exists() {
            return ChannelStatus::Unhealthy("Database not found".into());
        }

        // Try a simple command
        match Command::new(&self.binary_path)
            .args(["chats", "--limit", "1"])
            .output()
            .await
        {
            Ok(output) if output.status.success() => ChannelStatus::Healthy,
            Ok(output) => ChannelStatus::Unhealthy(
                String::from_utf8_lossy(&output.stderr).to_string()
            ),
            Err(e) => ChannelStatus::Unhealthy(e.to_string()),
        }
    }
}
```

### Recipient Formats

| Format | Example | Description |
|--------|---------|-------------|
| Phone (E.164) | `+14155551234` | International format |
| Email | `email:user@icloud.com` | iMessage email |

---

## Signal Adapter

The Signal adapter wraps `signal-cli`.

### Configuration

```toml
[channels.signal]
enabled = true
signal_cli_path = "/opt/homebrew/bin/signal-cli"
account = "+14155551234"
```

### Implementation

```rust
pub struct SignalAdapter {
    cli_path: PathBuf,
    account: String,
}

impl SignalAdapter {
    pub fn new(config: &SignalConfig) -> Result<Self> {
        Ok(Self {
            cli_path: config.signal_cli_path.clone(),
            account: config.account.clone(),
        })
    }
}

#[async_trait]
impl ChannelAdapter for SignalAdapter {
    fn channel_id(&self) -> &str {
        "signal"
    }

    async fn send(&self, params: &SendParams) -> Result<SendResult> {
        let mut cmd = Command::new(&self.cli_path);
        cmd.args(["-a", &self.account, "send", "-m", &params.message]);

        // Handle group vs individual
        if params.recipient.starts_with("group:") {
            let group_id = &params.recipient[6..];
            cmd.args(["-g", group_id]);
        } else {
            cmd.arg(&params.recipient);
        }

        // Attachments
        for attachment in &params.attachments {
            cmd.args(["-a", &attachment.path]);
        }

        let output = cmd.output().await?;

        if output.status.success() {
            Ok(SendResult {
                success: true,
                message_id: None,
                timestamp: Utc::now(),
            })
        } else {
            Err(Error::SendFailed(String::from_utf8_lossy(&output.stderr).to_string()))
        }
    }

    async fn watch(&self) -> Result<Box<dyn Stream<Item = IncomingMessage> + Send>> {
        let mut child = Command::new(&self.cli_path)
            .args(["-a", &self.account, "receive", "--json"])
            .stdout(Stdio::piped())
            .spawn()?;

        let stdout = child.stdout.take().unwrap();
        let reader = BufReader::new(stdout);

        let stream = reader
            .lines()
            .filter_map(|line| async move {
                let line = line.ok()?;
                let event: SignalEvent = serde_json::from_str(&line).ok()?;

                // Only return data messages
                if let SignalEvent::DataMessage(msg) = event {
                    Some(IncomingMessage {
                        channel: "signal".into(),
                        sender: msg.source,
                        text: msg.message,
                        timestamp: Utc::now(),
                        ..Default::default()
                    })
                } else {
                    None
                }
            });

        Ok(Box::new(stream))
    }

    // ... other methods
}
```

### Recipient Formats

| Format | Example | Description |
|--------|---------|-------------|
| Phone | `+14155551234` | Phone number |
| Group | `group:BASE64ID` | Group identifier |

---

## Discord Adapter

The Discord adapter uses the Discord API directly (via `serenity` or similar).

### Configuration

```toml
[channels.discord]
enabled = true
token_file = "/Users/carapace/.config/carapace/discord_token"
```

### Implementation

```rust
use serenity::all::*;

pub struct DiscordAdapter {
    http: Arc<Http>,
    cache: Arc<Cache>,
}

impl DiscordAdapter {
    pub async fn new(config: &DiscordConfig) -> Result<Self> {
        let token = std::fs::read_to_string(&config.token_file)?.trim().to_string();
        let http = Http::new(&token);

        // Validate token
        http.get_current_user().await?;

        Ok(Self {
            http: Arc::new(http),
            cache: Arc::new(Cache::new()),
        })
    }
}

#[async_trait]
impl ChannelAdapter for DiscordAdapter {
    fn channel_id(&self) -> &str {
        "discord"
    }

    async fn send(&self, params: &SendParams) -> Result<SendResult> {
        // Parse recipient
        let channel_id = if params.recipient.starts_with("channel:") {
            ChannelId::new(params.recipient[8..].parse()?)
        } else if params.recipient.starts_with("user:") {
            // Create DM channel
            let user_id = UserId::new(params.recipient[5..].parse()?);
            user_id.create_dm_channel(&self.http).await?.id
        } else {
            return Err(Error::InvalidRecipient(params.recipient.clone()));
        };

        // Send message
        let message = channel_id
            .send_message(&self.http, CreateMessage::new().content(&params.message))
            .await?;

        Ok(SendResult {
            success: true,
            message_id: Some(message.id.to_string()),
            timestamp: Utc::now(),
        })
    }

    async fn watch(&self) -> Result<Box<dyn Stream<Item = IncomingMessage> + Send>> {
        // Discord requires a gateway connection for events
        // This is more complex - needs a separate event loop
        // Simplified: use polling or webhook approach

        todo!("Discord watch requires gateway connection")
    }

    // ... other methods
}
```

### Recipient Formats

| Format | Example | Description |
|--------|---------|-------------|
| Channel | `channel:123456789012345678` | Channel ID |
| User DM | `user:987654321098765432` | User ID (creates DM) |

---

## Gmail Adapter

The Gmail adapter uses OAuth and the Gmail API.

### Configuration

```toml
[channels.gmail]
enabled = true
credentials_path = "/Users/carapace/.config/gog"
```

### Implementation

```rust
pub struct GmailAdapter {
    gog_path: PathBuf,
    credentials_path: PathBuf,
}

impl GmailAdapter {
    pub fn new(config: &GmailConfig) -> Result<Self> {
        Ok(Self {
            gog_path: which::which("gog")?,
            credentials_path: config.credentials_path.clone(),
        })
    }
}

#[async_trait]
impl ChannelAdapter for GmailAdapter {
    fn channel_id(&self) -> &str {
        "gmail"
    }

    async fn send(&self, params: &SendParams) -> Result<SendResult> {
        // Gmail send requires subject
        let subject = params.metadata
            .get("subject")
            .map(|s| s.as_str())
            .unwrap_or("Message from OpenClaw");

        let mut cmd = Command::new(&self.gog_path);
        cmd.args(["send", "--to", &params.recipient, "--subject", subject]);

        // Pipe message body to stdin
        cmd.stdin(Stdio::piped());

        let mut child = cmd.spawn()?;

        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(params.message.as_bytes()).await?;
        }

        let output = child.wait_with_output().await?;

        if output.status.success() {
            Ok(SendResult {
                success: true,
                message_id: None,
                timestamp: Utc::now(),
            })
        } else {
            Err(Error::SendFailed(String::from_utf8_lossy(&output.stderr).to_string()))
        }
    }

    async fn list_chats(&self, limit: u32, _offset: u32) -> Result<ChatsResult> {
        // List recent email threads
        let output = Command::new(&self.gog_path)
            .args(["list", "--limit", &limit.to_string(), "--json"])
            .output()
            .await?;

        let threads: Vec<GmailThread> = serde_json::from_slice(&output.stdout)?;

        Ok(ChatsResult {
            chats: threads.into_iter().map(Chat::from).collect(),
            total: threads.len() as u32,
            has_more: threads.len() as u32 == limit,
        })
    }

    // ... other methods
}
```

### Recipient Formats

| Format | Example | Description |
|--------|---------|-------------|
| Email | `user@example.com` | Email address |

### Additional Parameters

| Parameter | Description |
|-----------|-------------|
| `subject` | Email subject line |
| `thread_id` | Reply to existing thread |
| `cc` | CC recipients |
| `bcc` | BCC recipients |

---

## Adding New Adapters

To add support for a new messaging platform:

1. Create a new file in `adapters/`
2. Implement the `ChannelAdapter` trait
3. Add configuration struct
4. Register in `ChannelAdapters::new()`
5. Create corresponding shim in `carapace-shims`

### Template

```rust
pub struct NewChannelAdapter {
    // Channel-specific fields
}

impl NewChannelAdapter {
    pub fn new(config: &NewChannelConfig) -> Result<Self> {
        // Initialize
    }
}

#[async_trait]
impl ChannelAdapter for NewChannelAdapter {
    fn channel_id(&self) -> &str {
        "newchannel"
    }

    async fn health_check(&self) -> ChannelStatus {
        // Check connectivity
    }

    async fn send(&self, params: &SendParams) -> Result<SendResult> {
        // Implement send
    }

    async fn list_chats(&self, limit: u32, offset: u32) -> Result<ChatsResult> {
        // Implement list
    }

    async fn get_history(&self, chat_id: &str, limit: u32, before: Option<&str>) -> Result<HistoryResult> {
        // Implement history
    }

    async fn watch(&self) -> Result<Box<dyn Stream<Item = IncomingMessage> + Send>> {
        // Implement watch
    }
}
```
