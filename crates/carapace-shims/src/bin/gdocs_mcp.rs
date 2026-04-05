//! gdocs-mcp — MCP (Model Context Protocol) server for the Carapace Google Docs channel.
//!
//! Speaks the MCP stdio transport protocol so that any MCP-capable agent
//! (Claude Code, etc.) can call Google Docs/Drive tools directly.
//!
//! ## Transport
//! Reads newline-delimited JSON-RPC 2.0 from **stdin**, writes responses to
//! **stdout**. Diagnostic messages go to **stderr** only.
//!
//! ## Tools exposed
//! | Tool | Description |
//! |------|-------------|
//! | `gdocs_search` | Search Google Drive for files |
//! | `gdocs_read` | Read a Google Doc (structured content) |
//! | `gdocs_file_info` | Get file metadata |
//! | `gdocs_create` | Create a new Google Doc |
//! | `gdocs_copy` | Copy an existing file |
//! | `gdocs_append` | Append text to a doc the agent created |
//! | `gdocs_status` | Check proxy health |
//!
//! ## Usage
//! ```json
//! {
//!   "mcpServers": {
//!     "gdocs": {
//!       "command": "/usr/local/bin/gdocs-mcp",
//!       "env": { "GDOCS_ACCOUNT": "automations" }
//!     }
//!   }
//! }
//! ```

use std::io::{BufRead, Write};

use carapace_client::GatewayClient;
use serde_json::{json, Value};

const PROTOCOL_VERSION: &str = "2024-11-05";
const SERVER_NAME: &str = "carapace-gdocs";
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() {
    eprintln!("[gdocs-mcp] starting — waiting for MCP messages on stdin");

    let gdocs_account: Option<String> = std::env::var("GDOCS_ACCOUNT").ok();
    if let Some(ref acct) = gdocs_account {
        eprintln!("[gdocs-mcp] targeting account: {acct}");
    }

    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    let mut client: Option<GatewayClient> = None;

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) if l.trim().is_empty() => continue,
            Ok(l) => l,
            Err(e) => {
                eprintln!("[gdocs-mcp] stdin read error: {e}");
                break;
            }
        };

        eprintln!("[gdocs-mcp] recv: {line}");

        let req: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                let resp = json!({
                    "jsonrpc": "2.0",
                    "id": null,
                    "error": {"code": -32700, "message": format!("Parse error: {e}")}
                });
                writeln!(out, "{}", resp).ok();
                out.flush().ok();
                continue;
            }
        };

        let id = req.get("id").cloned().unwrap_or(Value::Null);
        let method = req.get("method").and_then(|v| v.as_str()).unwrap_or("");
        let params = req.get("params").cloned().unwrap_or(json!({}));

        let response = handle_message(method, &params, id.clone(), &mut client, &gdocs_account);
        if response.is_null() {
            continue;
        }
        eprintln!("[gdocs-mcp] send: {response}");
        writeln!(out, "{response}").ok();
        out.flush().ok();
    }

    eprintln!("[gdocs-mcp] stdin closed — exiting");
}

fn handle_message(
    method: &str,
    params: &Value,
    id: Value,
    client: &mut Option<GatewayClient>,
    gdocs_account: &Option<String>,
) -> Value {
    match method {
        "initialize" => {
            json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "protocolVersion": PROTOCOL_VERSION,
                    "capabilities": {
                        "tools": {}
                    },
                    "serverInfo": {
                        "name": SERVER_NAME,
                        "version": SERVER_VERSION
                    }
                }
            })
        }

        "notifications/initialized" => {
            eprintln!("[gdocs-mcp] initialized");
            Value::Null
        }

        "tools/list" => {
            json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "tools": tool_definitions()
                }
            })
        }

        "tools/call" => {
            let tool_name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or(json!({}));

            if client.is_none() {
                match GatewayClient::connect_default() {
                    Ok(c) => {
                        eprintln!("[gdocs-mcp] connected to gateway");
                        *client = Some(c);
                    }
                    Err(e) => {
                        return tool_error(
                            id,
                            format!(
                                "Cannot connect to Carapace daemon: {e}. \
                                 Make sure carapace-daemon is running."
                            ),
                        );
                    }
                }
            }

            let gw = client.as_mut().unwrap();
            call_tool(tool_name, &args, id, gw, gdocs_account)
        }

        _ => {
            json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {"code": -32601, "message": format!("Method not found: {method}")}
            })
        }
    }
}

fn tool_definitions() -> Value {
    json!([
        {
            "name": "gdocs_search",
            "description": "\
Search Google Drive for files. Uses Drive query syntax. \
Returns a list of matching files with id, name, mime_type, created/modified times, and owner. \
Supported query operators: name contains 'text', mimeType = '...', modifiedTime > '2024-01-01', \
'text' in parents (search in folder), starred = true, sharedWithMe. \
Set docs_only=true to restrict results to Google Docs only. \
Example: name contains 'budget' — finds files with 'budget' in the name.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Google Drive search query (Drive query syntax). Leave empty to list recent files."
                    },
                    "max": {
                        "type": "integer",
                        "description": "Maximum number of results. Defaults to 20, max 100.",
                        "default": 20
                    },
                    "docs_only": {
                        "type": "boolean",
                        "description": "If true, only return Google Docs (not sheets, slides, etc.). Defaults to false.",
                        "default": false
                    }
                },
                "required": []
            }
        },
        {
            "name": "gdocs_read",
            "description": "\
Read a Google Doc, Sheet, Slides presentation, or Form by its file ID. Auto-detects the file type. \
For Docs: returns structured content with headings, paragraphs, links, and tables. \
For Sheets: returns sheet names and cell data in rows. \
For Slides: returns slide-by-slide text content. \
For Forms: returns questions, options, and responses. \
PDFs, images, and other binary files are not supported. \
Use the file ID from gdocs_search results or extract it from a Google URL \
(the part between /d/ and /edit).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "document_id": {
                        "type": "string",
                        "description": "The Google Docs document ID."
                    }
                },
                "required": ["document_id"]
            }
        },
        {
            "name": "gdocs_file_info",
            "description": "\
Get metadata about a file in Google Drive (name, type, owner, dates, link). \
Works for any file type, not just Google Docs.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "file_id": {
                        "type": "string",
                        "description": "The Google Drive file ID."
                    }
                },
                "required": ["file_id"]
            }
        },
        {
            "name": "gdocs_create",
            "description": "\
Create a new Google Doc with the given title and optional initial content. \
By default the document is created in the Drive root, but you can specify a folder_id \
to place it in a specific folder. \
Returns the document_id and a link to open it in the browser. \
The agent can later edit this document using gdocs_append.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "title": {
                        "type": "string",
                        "description": "Title for the new document."
                    },
                    "content": {
                        "type": "string",
                        "description": "Optional initial plain-text content for the document."
                    },
                    "folder_id": {
                        "type": "string",
                        "description": "Optional folder ID to create the document in. Use gdocs_create_folder to create folders, or gdocs_search to find existing ones."
                    }
                },
                "required": ["title"]
            }
        },
        {
            "name": "gdocs_copy",
            "description": "\
Copy an existing file in Google Drive. Returns the new file's ID and metadata. \
The copy is owned by the agent's account, so it can be edited freely. \
This is useful for creating an editable version of a read-only document.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "file_id": {
                        "type": "string",
                        "description": "The ID of the file to copy."
                    },
                    "title": {
                        "type": "string",
                        "description": "Optional title for the copy. Defaults to 'Copy of <original>'."
                    }
                },
                "required": ["file_id"]
            }
        },
        {
            "name": "gdocs_append",
            "description": "\
Append plain text to the end of a Google Doc. Only works on documents the agent \
created or copied (enforced by OAuth scope). \
Use this after gdocs_create or gdocs_copy to add content to a document.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "document_id": {
                        "type": "string",
                        "description": "The document ID to append text to."
                    },
                    "text": {
                        "type": "string",
                        "description": "Plain text to append to the end of the document."
                    }
                },
                "required": ["document_id", "text"]
            }
        },
        {
            "name": "gdocs_create_folder",
            "description": "\
Create a folder in Google Drive. Optionally specify a parent folder ID to nest it. \
Returns the folder's ID and a link to open it in Drive. \
Use this to organize documents into folders.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Name for the new folder."
                    },
                    "parent_id": {
                        "type": "string",
                        "description": "Optional parent folder ID. If omitted, the folder is created in the Drive root."
                    }
                },
                "required": ["name"]
            }
        },
        {
            "name": "gdocs_status",
            "description": "\
Check the health of the Google Docs proxy. Returns whether the proxy is reachable \
and whether the OAuth token is valid. \
Use this to diagnose connection problems before calling other gdocs tools.",
            "inputSchema": {
                "type": "object",
                "properties": {},
                "required": []
            }
        }
    ])
}

fn with_account(mut params: Value, gdocs_account: &Option<String>) -> Value {
    if let Some(ref acct) = gdocs_account {
        params["account"] = json!(acct);
    }
    params
}

fn call_tool(
    name: &str,
    args: &Value,
    id: Value,
    gw: &mut GatewayClient,
    gdocs_account: &Option<String>,
) -> Value {
    match name {
        "gdocs_search" => {
            let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
            let max = args.get("max").and_then(|v| v.as_u64()).unwrap_or(20);
            let docs_only = args.get("docs_only").and_then(|v| v.as_bool()).unwrap_or(false);

            let gw_params = with_account(
                json!({
                    "channel": "gdocs",
                    "query": query,
                    "max": max,
                    "docs_only": docs_only,
                }),
                gdocs_account,
            );

            match gw.call("channel.search", gw_params) {
                Ok(result) => tool_success(id, result),
                Err(e) => tool_error(id, format!("gdocs_search failed: {e}")),
            }
        }

        "gdocs_read" => {
            let doc_id = match args.get("document_id").and_then(|v| v.as_str()) {
                Some(d) => d,
                None => return tool_error(id, "Missing required argument: \"document_id\""),
            };

            let gw_params = with_account(
                json!({
                    "channel": "gdocs",
                    "chat_id": doc_id,
                }),
                gdocs_account,
            );

            match gw.call("channel.get_history", gw_params) {
                Ok(result) => tool_success(id, result),
                Err(e) => tool_error(id, format!("gdocs_read failed: {e}")),
            }
        }

        "gdocs_file_info" => {
            let file_id = match args.get("file_id").and_then(|v| v.as_str()) {
                Some(f) => f,
                None => return tool_error(id, "Missing required argument: \"file_id\""),
            };

            let gw_params = with_account(
                json!({
                    "channel": "gdocs",
                    "action": "file_info",
                    "file_id": file_id,
                }),
                gdocs_account,
            );

            match gw.call("channel.status", gw_params) {
                Ok(result) => tool_success(id, result),
                Err(e) => tool_error(id, format!("gdocs_file_info failed: {e}")),
            }
        }

        "gdocs_create" => {
            let title = match args.get("title").and_then(|v| v.as_str()) {
                Some(t) => t,
                None => return tool_error(id, "Missing required argument: \"title\""),
            };

            let mut gw_params = with_account(
                json!({
                    "channel": "gdocs",
                    "action": "create",
                    "title": title,
                }),
                gdocs_account,
            );
            if let Some(content) = args.get("content").and_then(|v| v.as_str()) {
                gw_params["content"] = json!(content);
            }
            if let Some(folder_id) = args.get("folder_id").and_then(|v| v.as_str()) {
                gw_params["folder_id"] = json!(folder_id);
            }

            match gw.call("channel.create_draft", gw_params) {
                Ok(result) => tool_success(id, result),
                Err(e) => tool_error(id, format!("gdocs_create failed: {e}")),
            }
        }

        "gdocs_copy" => {
            let file_id = match args.get("file_id").and_then(|v| v.as_str()) {
                Some(f) => f,
                None => return tool_error(id, "Missing required argument: \"file_id\""),
            };

            let mut gw_params = with_account(
                json!({
                    "channel": "gdocs",
                    "action": "copy",
                    "file_id": file_id,
                }),
                gdocs_account,
            );
            if let Some(title) = args.get("title").and_then(|v| v.as_str()) {
                gw_params["title"] = json!(title);
            }

            match gw.call("channel.send", gw_params) {
                Ok(result) => tool_success(id, result),
                Err(e) => tool_error(id, format!("gdocs_copy failed: {e}")),
            }
        }

        "gdocs_append" => {
            let doc_id = match args.get("document_id").and_then(|v| v.as_str()) {
                Some(d) => d,
                None => return tool_error(id, "Missing required argument: \"document_id\""),
            };
            let text = match args.get("text").and_then(|v| v.as_str()) {
                Some(t) => t,
                None => return tool_error(id, "Missing required argument: \"text\""),
            };

            let gw_params = with_account(
                json!({
                    "channel": "gdocs",
                    "action": "append",
                    "document_id": doc_id,
                    "text": text,
                }),
                gdocs_account,
            );

            match gw.call("channel.send", gw_params) {
                Ok(result) => tool_success(id, result),
                Err(e) => tool_error(id, format!("gdocs_append failed: {e}")),
            }
        }

        "gdocs_create_folder" => {
            let name = match args.get("name").and_then(|v| v.as_str()) {
                Some(n) => n,
                None => return tool_error(id, "Missing required argument: \"name\""),
            };

            let mut gw_params = with_account(
                json!({
                    "channel": "gdocs",
                    "action": "create_folder",
                    "name": name,
                }),
                gdocs_account,
            );
            if let Some(parent_id) = args.get("parent_id").and_then(|v| v.as_str()) {
                gw_params["parent_id"] = json!(parent_id);
            }

            match gw.call("channel.send", gw_params) {
                Ok(result) => tool_success(id, result),
                Err(e) => tool_error(id, format!("gdocs_create_folder failed: {e}")),
            }
        }

        "gdocs_status" => {
            let gw_params = with_account(json!({"channel": "gdocs"}), gdocs_account);
            match gw.call("channel.status", gw_params) {
                Ok(result) => tool_success(id, result),
                Err(e) => tool_error(id, format!("gdocs_status failed: {e}")),
            }
        }

        _ => tool_error(id, format!("Unknown tool: {name}")),
    }
}

fn tool_success(id: Value, result: Value) -> Value {
    let text = serde_json::to_string_pretty(&result).unwrap_or_else(|_| result.to_string());
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "content": [
                {"type": "text", "text": text}
            ],
            "isError": false
        }
    })
}

fn tool_error(id: Value, message: impl Into<String>) -> Value {
    let msg = message.into();
    eprintln!("[gdocs-mcp] tool error: {msg}");
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "content": [
                {"type": "text", "text": msg}
            ],
            "isError": true
        }
    })
}
