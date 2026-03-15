//! imsg — CLI shim that mirrors the real `imsg` interface but routes
//! all commands through the Carapace gateway daemon.
//!
//! # Usage
//!
//! ```bash
//! imsg send --to "+1234567890" --text "Hello!"
//! imsg chats [--limit 10] [--json]
//! imsg history --chat-id "chat123" [--limit 20] [--json]
//! imsg status
//! ```

use carapace_client::GatewayClient;
use clap::{Parser, Subcommand};
use serde_json::json;

/// Carapace iMessage shim — routes imsg commands through the gateway daemon.
#[derive(Parser)]
#[command(name = "imsg", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Send a message.
    Send {
        /// Recipient handle (phone number or email).
        #[arg(long = "to")]
        to: String,

        /// Message text.
        #[arg(long = "text")]
        text: String,

        /// File attachment(s).
        #[arg(long = "file")]
        file: Vec<String>,
    },

    /// List recent chats.
    Chats {
        /// Maximum number of chats to return.
        #[arg(long)]
        limit: Option<u32>,

        /// Output as raw JSON.
        #[arg(long)]
        json: bool,
    },

    /// Get message history for a chat.
    History {
        /// Chat identifier.
        #[arg(long = "chat-id")]
        chat_id: String,

        /// Maximum number of messages to return.
        #[arg(long)]
        limit: Option<u32>,

        /// Output as raw JSON.
        #[arg(long)]
        json: bool,
    },

    /// Check iMessage channel status.
    Status,

    /// Watch for incoming messages (streaming).
    Watch {
        /// Output as raw JSON lines.
        #[arg(long)]
        json: bool,
    },
}

fn main() {
    let cli = Cli::parse();

    let mut client = match GatewayClient::connect_default() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Error: Cannot connect to Carapace daemon: {e}");
            eprintln!();
            eprintln!("Make sure the daemon is running:");
            eprintln!("  sudo -u carapace carapace-daemon");
            std::process::exit(1);
        }
    };

    match cli.command {
        Commands::Send { to, text, file } => {
            let mut params = json!({
                "channel": "imsg",
                "recipient": to,
                "message": text,
            });
            if !file.is_empty() {
                params["attachments"] = json!(file);
            }

            match client.call("channel.send", params) {
                Ok(result) => {
                    if result.get("success") == Some(&json!(true)) {
                        println!("Message sent to {to}");
                    } else {
                        println!("{}", serde_json::to_string_pretty(&result).unwrap());
                    }
                }
                Err(e) => {
                    print_error(e);
                    std::process::exit(1);
                }
            }
        }

        Commands::Chats { limit, json: raw } => {
            let mut params = json!({"channel": "imsg"});
            if let Some(n) = limit {
                params["limit"] = json!(n);
            }

            match client.call("channel.list_chats", params) {
                Ok(result) => {
                    if raw {
                        println!("{}", serde_json::to_string_pretty(&result).unwrap());
                    } else {
                        print_chats(&result);
                    }
                }
                Err(e) => {
                    print_error(e);
                    std::process::exit(1);
                }
            }
        }

        Commands::History {
            chat_id,
            limit,
            json: raw,
        } => {
            let mut params = json!({
                "channel": "imsg",
                "chat_id": chat_id,
            });
            if let Some(n) = limit {
                params["limit"] = json!(n);
            }

            match client.call("channel.get_history", params) {
                Ok(result) => {
                    if raw {
                        println!("{}", serde_json::to_string_pretty(&result).unwrap());
                    } else {
                        print_history(&result);
                    }
                }
                Err(e) => {
                    print_error(e);
                    std::process::exit(1);
                }
            }
        }

        Commands::Status => match client.call("channel.status", json!({"channel": "imsg"})) {
            Ok(result) => {
                println!("{}", serde_json::to_string_pretty(&result).unwrap());
            }
            Err(e) => {
                print_error(e);
                std::process::exit(1);
            }
        },

        Commands::Watch { json: raw } => {
            let (_ack, subscription) =
                match client.subscribe("channel.watch", json!({"channel": "imsg"})) {
                    Ok(pair) => pair,
                    Err(e) => {
                        print_error(e);
                        std::process::exit(1);
                    }
                };

            if !raw {
                eprintln!("Watching for incoming messages... (Ctrl+C to stop)");
            }

            for event in subscription {
                match event {
                    Ok(value) => {
                        if raw {
                            println!("{}", serde_json::to_string(&value).unwrap());
                        } else {
                            let sender = value
                                .get("sender")
                                .or_else(|| value.get("handle"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("?");
                            let text = value
                                .get("text")
                                .or_else(|| value.get("body"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("");
                            println!("  [{sender}]: {text}");
                        }
                    }
                    Err(e) => {
                        eprintln!("Error: {e}");
                        std::process::exit(1);
                    }
                }
            }
        }
    }
}

/// Print a gateway error with a human-readable message.
fn print_error(err: carapace_client::ClientError) {
    match err {
        carapace_client::ClientError::Gateway { code, message } => {
            let hint = match code {
                -32001 => "Recipient not in allowlist",
                -32002 => "Rate limited, try again later",
                -32003 => "Message blocked by content filter",
                -32004 => "iMessage channel not available",
                -32005 => "Send failed",
                _ => "",
            };
            if hint.is_empty() {
                eprintln!("Error ({code}): {message}");
            } else {
                eprintln!("Error: {hint}");
                eprintln!("  Detail: {message}");
            }
        }
        other => {
            eprintln!("Error: {other}");
        }
    }
}

/// Pretty-print chat list in human-readable form.
fn print_chats(value: &serde_json::Value) {
    if let Some(chats) = value.as_array() {
        if chats.is_empty() {
            println!("No chats found.");
            return;
        }
        for chat in chats {
            let id = chat
                .get("chat_id")
                .or_else(|| chat.get("id"))
                .map(|v| v.to_string())
                .unwrap_or_else(|| "?".into());
            let display = chat
                .get("display_name")
                .or_else(|| chat.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("(unnamed)");
            println!("  {id}: {display}");
        }
    } else {
        // Not an array — just dump it.
        println!("{}", serde_json::to_string_pretty(value).unwrap());
    }
}

/// Pretty-print message history in human-readable form.
fn print_history(value: &serde_json::Value) {
    if let Some(messages) = value.as_array() {
        if messages.is_empty() {
            println!("No messages found.");
            return;
        }
        for msg in messages {
            let sender = msg
                .get("sender")
                .or_else(|| msg.get("handle"))
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let text = msg
                .get("text")
                .or_else(|| msg.get("body"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let date = msg
                .get("date")
                .or_else(|| msg.get("timestamp"))
                .map(|v| v.to_string())
                .unwrap_or_default();

            if date.is_empty() {
                println!("  [{sender}]: {text}");
            } else {
                println!("  [{sender} @ {date}]: {text}");
            }
        }
    } else {
        println!("{}", serde_json::to_string_pretty(value).unwrap());
    }
}
