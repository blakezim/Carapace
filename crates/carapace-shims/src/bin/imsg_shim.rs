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

    /// Persistent JSON-RPC server over stdin/stdout (used by OpenClaw).
    Rpc,
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

        Commands::Rpc => {
            use std::io::BufRead;
            use std::sync::{Arc, Mutex};

            // Dedicated writer thread owns stdout so both the main loop and the
            // watch background thread can write without blocking each other.
            let (tx, rx) = std::sync::mpsc::channel::<String>();
            std::thread::spawn(move || {
                use std::io::Write;
                let stdout = std::io::stdout();
                let mut out = stdout.lock();
                for line in rx {
                    writeln!(out, "{line}").ok();
                    out.flush().ok();
                }
            });

            let stdin = std::io::stdin();
            let tx = Arc::new(Mutex::new(tx));

            for line in stdin.lock().lines() {
                let line = match line {
                    Ok(l) if l.trim().is_empty() => continue,
                    Ok(l) => l,
                    Err(_) => break,
                };

                let req: serde_json::Value = match serde_json::from_str(&line) {
                    Ok(v) => v,
                    Err(e) => {
                        let resp = json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":format!("Parse error: {e}")}});
                        tx.lock().unwrap().send(resp.to_string()).ok();
                        continue;
                    }
                };

                let id = req.get("id").cloned().unwrap_or(json!(null));
                let method = req.get("method").and_then(|v| v.as_str()).unwrap_or("");
                let params = req.get("params").cloned().unwrap_or(json!({}));
                eprintln!("[imsg-rpc] method={method} params={params}");
                if let Ok(mut f) = std::fs::OpenOptions::new().append(true).create(true).open("/tmp/imsg_rpc.log") {
                    use std::io::Write;
                    writeln!(f, "method={method} params={params}").ok();
                }

                let (gw_method, gw_params) = match method {
                    "chats" | "list_chats" | "chats.list" => {
                        let mut p = json!({"channel": "imsg"});
                        if let Some(limit) = params.get("limit") { p["limit"] = limit.clone(); }
                        ("channel.list_chats", p)
                    }
                    "send" | "messages.send" => {
                        let mut p = json!({
                            "channel": "imsg",
                            "recipient": params.get("to").or_else(|| params.get("recipient")).cloned().unwrap_or(json!("")),
                            "message": params.get("text").or_else(|| params.get("message")).cloned().unwrap_or(json!("")),
                        });
                        if let Some(files) = params.get("files") { p["attachments"] = files.clone(); }
                        ("channel.send", p)
                    }
                    "history" | "get_history" | "messages.list" | "chats.history" => {
                        let mut p = json!({
                            "channel": "imsg",
                            "chat_id": params.get("chat_id").cloned().unwrap_or(json!("")),
                        });
                        if let Some(limit) = params.get("limit") { p["limit"] = limit.clone(); }
                        ("channel.get_history", p)
                    }
                    "status" | "channel.status" => ("channel.status", json!({"channel": "imsg"})),
                    "watch.subscribe" | "watch" => {
                        // Spawn a background thread so the main stdin loop keeps running
                        // and can handle messages.send while events are streaming.
                        let tx2 = Arc::clone(&tx);
                        std::thread::spawn(move || {
                            match GatewayClient::connect_default() {
                                Ok(watch_client) => {
                                    match watch_client.subscribe("channel.watch", json!({"channel": "imsg"})) {
                                        Ok((_ack, subscription)) => {
                                            let ok = json!({"jsonrpc":"2.0","id":id,"result":{"subscription":1}});
                                            tx2.lock().unwrap().send(ok.to_string()).ok();
                                            if let Ok(mut f) = std::fs::OpenOptions::new().append(true).create(true).open("/tmp/imsg_rpc.log") {
                                                use std::io::Write;
                                                writeln!(f, "watch.subscribe: entered event loop").ok();
                                            }
                                            for event in subscription {
                                                match event {
                                                    Ok(value) => {
                                                        if let Ok(mut f) = std::fs::OpenOptions::new().append(true).create(true).open("/tmp/imsg_rpc.log") {
                                                            use std::io::Write;
                                                            writeln!(f, "watch.event: {value}").ok();
                                                        }
                                                        let notif = json!({"jsonrpc":"2.0","method":"message","params":{"message":value}});
                                                        tx2.lock().unwrap().send(notif.to_string()).ok();
                                                    }
                                                    Err(e) => {
                                                        if let Ok(mut f) = std::fs::OpenOptions::new().append(true).create(true).open("/tmp/imsg_rpc.log") {
                                                            use std::io::Write;
                                                            writeln!(f, "watch.event error: {e}").ok();
                                                        }
                                                        break;
                                                    }
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            let err = json!({"jsonrpc":"2.0","id":id,"error":{"code":-32603,"message":e.to_string()}});
                                            tx2.lock().unwrap().send(err.to_string()).ok();
                                        }
                                    }
                                }
                                Err(e) => {
                                    let err = json!({"jsonrpc":"2.0","id":id,"error":{"code":-32603,"message":e.to_string()}});
                                    tx2.lock().unwrap().send(err.to_string()).ok();
                                }
                            }
                        });
                        continue;
                    }
                    _ => {
                        let resp = json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":format!("Method not found: {method}")}});
                        tx.lock().unwrap().send(resp.to_string()).ok();
                        continue;
                    }
                };

                let resp = match client.call(gw_method, gw_params) {
                    Ok(result) => json!({"jsonrpc":"2.0","id":id,"result":result}),
                    Err(carapace_client::ClientError::Gateway { code, message }) => {
                        json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
                    }
                    Err(e) => {
                        json!({"jsonrpc":"2.0","id":id,"error":{"code":-32603,"message":e.to_string()}})
                    }
                };

                tx.lock().unwrap().send(resp.to_string()).ok();
            }
        }

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
