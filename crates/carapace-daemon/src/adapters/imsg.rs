//! iMessage adapter — calls the real `imsg` binary via child processes.
//!
//! The `imsg` CLI (steipete/imsg) supports:
//! - `imsg send --to <handle> --text <message> [--file <path>] [--service imessage|sms|auto]`
//! - `imsg chats [--limit N] [--json]`
//! - `imsg history --chat-id <id> [--limit N] [--json]`

use std::path::PathBuf;

use serde::Serialize;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc;
use tracing::{debug, warn};

/// Errors from the iMessage adapter.
#[derive(Debug, thiserror::Error)]
pub enum AdapterError {
    #[error("imsg binary not found at {0}")]
    BinaryNotFound(PathBuf),

    #[error("Messages database not found at {0}")]
    DatabaseNotFound(PathBuf),

    #[error("imsg command failed (exit {exit_code}): {stderr}")]
    CommandFailed { stderr: String, exit_code: i32 },

    #[error("failed to parse imsg output as JSON: {0}")]
    OutputParse(String),

    #[error("I/O error running imsg: {0}")]
    Io(#[from] std::io::Error),
}

/// Result of a successful send.
#[derive(Debug, Serialize)]
pub struct SendResult {
    pub success: bool,
    pub stdout: String,
}

/// Health status of the iMessage adapter.
#[derive(Debug, Serialize)]
pub struct HealthStatus {
    pub binary_exists: bool,
    pub db_exists: bool,
    pub smoke_test_ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// iMessage adapter — wraps the `imsg` CLI binary.
pub struct ImsgAdapter {
    binary_path: PathBuf,
    db_path: PathBuf,
}

impl ImsgAdapter {
    pub fn new(binary_path: PathBuf, db_path: PathBuf) -> Self {
        Self {
            binary_path,
            db_path,
        }
    }

    /// Send a message via `imsg send`.
    pub async fn send(
        &self,
        recipient: &str,
        message: &str,
        attachments: &[String],
    ) -> Result<SendResult, AdapterError> {
        self.ensure_binary()?;

        // Run osascript as root via a narrow sudoers rule so it bypasses TCC.
        // The script is root-owned and read-only; arguments are passed as argv
        // (no AppleScript injection risk). Root bypasses the Apple Events TCC
        // check that blocks daemon-context processes from reaching Messages.app.
        debug!(recipient, "sending imsg via osascript");
        let output = Command::new("sudo")
            .arg("/usr/local/carapace/imsg-send")
            .arg(recipient)
            .arg(message)
            .output()
            .await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            return Err(AdapterError::CommandFailed {
                stderr,
                exit_code: output.status.code().unwrap_or(-1),
            });
        }

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        Ok(SendResult {
            success: true,
            stdout,
        })
    }

    /// List chats via `imsg chats --json`.
    pub async fn list_chats(
        &self,
        limit: Option<u32>,
    ) -> Result<serde_json::Value, AdapterError> {
        self.ensure_binary()?;

        let mut cmd = Command::new(&self.binary_path);
        cmd.arg("chats").arg("--json");

        if let Some(n) = limit {
            cmd.arg("--limit").arg(n.to_string());
        }

        debug!(?limit, "listing imsg chats");
        let output = cmd.output().await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            return Err(AdapterError::CommandFailed {
                stderr,
                exit_code: output.status.code().unwrap_or(-1),
            });
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        parse_json_lines(&stdout)
    }

    /// Get chat history via `imsg history --json`.
    pub async fn get_history(
        &self,
        chat_id: &str,
        limit: Option<u32>,
        before: Option<&str>,
    ) -> Result<serde_json::Value, AdapterError> {
        self.ensure_binary()?;

        let mut cmd = Command::new(&self.binary_path);
        cmd.arg("history")
            .arg("--chat-id")
            .arg(chat_id)
            .arg("--json");

        if let Some(n) = limit {
            cmd.arg("--limit").arg(n.to_string());
        }

        if let Some(ts) = before {
            cmd.arg("--end").arg(ts);
        }

        debug!(chat_id, ?limit, "fetching imsg history");
        let output = cmd.output().await?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            return Err(AdapterError::CommandFailed {
                stderr,
                exit_code: output.status.code().unwrap_or(-1),
            });
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        parse_json_lines(&stdout)
    }

    /// Health check: verify binary exists, db exists, run a smoke test.
    pub async fn health_check(&self) -> HealthStatus {
        let binary_exists = self.binary_path.exists();
        let db_exists = self.db_path.exists();

        if !binary_exists {
            return HealthStatus {
                binary_exists,
                db_exists,
                smoke_test_ok: false,
                error: Some(format!(
                    "binary not found at {}",
                    self.binary_path.display()
                )),
            };
        }

        // Smoke test: run `imsg chats --limit 1 --json`
        let smoke_result = Command::new(&self.binary_path)
            .arg("chats")
            .arg("--limit")
            .arg("1")
            .arg("--json")
            .output()
            .await;

        match smoke_result {
            Ok(output) if output.status.success() => HealthStatus {
                binary_exists,
                db_exists,
                smoke_test_ok: true,
                error: None,
            },
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                warn!(stderr, "imsg smoke test failed");
                HealthStatus {
                    binary_exists,
                    db_exists,
                    smoke_test_ok: false,
                    error: Some(stderr),
                }
            }
            Err(e) => HealthStatus {
                binary_exists,
                db_exists,
                smoke_test_ok: false,
                error: Some(e.to_string()),
            },
        }
    }

    /// Query the current maximum message rowid from the Messages database.
    /// Used to pass `--since-rowid` to `imsg watch` so replayed subscriptions
    /// don't re-deliver already-seen messages.
    pub async fn max_message_rowid(&self) -> Option<u64> {
        let output = tokio::process::Command::new("sqlite3")
            .arg(&self.db_path)
            .arg("SELECT MAX(ROWID) FROM message")
            .output()
            .await
            .ok()?;
        String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse::<u64>()
            .ok()
    }

    /// Start watching for incoming messages via `imsg watch --json`.
    ///
    /// Spawns the child process and a tokio task that reads JSON lines from
    /// stdout into an mpsc channel. Returns a `WatchHandle` (which kills the
    /// child on drop) and a receiver of parsed JSON events.
    ///
    /// Pass `since_rowid` to skip messages already seen before this watch
    /// session started — prevents re-delivery when subscriptions restart.
    pub fn watch(
        &self,
        buffer_size: usize,
        since_rowid: Option<u64>,
    ) -> Result<(WatchHandle, mpsc::Receiver<serde_json::Value>), AdapterError> {
        self.ensure_binary()?;

        let mut cmd = tokio::process::Command::new(&self.binary_path);
        cmd.arg("watch").arg("--json");
        if let Some(rowid) = since_rowid {
            cmd.arg("--since-rowid").arg(rowid.to_string());
        }

        let mut child = cmd
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()?;

        let stdout = child
            .stdout
            .take()
            .expect("stdout was piped but is None");

        let (tx, rx) = mpsc::channel(buffer_size);

        let reader_task = tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let trimmed = line.trim().to_string();
                if trimmed.is_empty() {
                    continue;
                }
                match serde_json::from_str::<serde_json::Value>(&trimmed) {
                    Ok(val) => {
                        if tx.send(val).await.is_err() {
                            break; // receiver dropped
                        }
                    }
                    Err(e) => {
                        warn!(error = %e, line = %trimmed, "skipping non-JSON watch line");
                    }
                }
            }
        });

        Ok((WatchHandle { child, _reader_task: reader_task }, rx))
    }

    fn ensure_binary(&self) -> Result<(), AdapterError> {
        if !self.binary_path.exists() {
            return Err(AdapterError::BinaryNotFound(self.binary_path.clone()));
        }
        Ok(())
    }
}

/// Handle for a running `imsg watch` child process.
///
/// Dropping the handle kills the child process, ensuring clean shutdown
/// when the client disconnects.
pub struct WatchHandle {
    child: tokio::process::Child,
    _reader_task: tokio::task::JoinHandle<()>,
}

impl Drop for WatchHandle {
    fn drop(&mut self) {
        // kill_on_drop is set, but we also abort the reader task
        let _ = self.child.start_kill();
        self._reader_task.abort();
    }
}

/// Parse JSON Lines output (one JSON object per line) into a JSON array.
fn parse_json_lines(output: &str) -> Result<serde_json::Value, AdapterError> {
    let items: Result<Vec<serde_json::Value>, _> = output
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .map(|l| serde_json::from_str(l))
        .collect();

    match items {
        Ok(arr) => Ok(serde_json::Value::Array(arr)),
        Err(e) => Err(AdapterError::OutputParse(format!("{e}: {output}"))),
    }
}

/// Encode a Rust string as an AppleScript string literal, safely handling
/// embedded double-quotes by concatenating with AppleScript's `quote` constant.
fn applescript_string(s: &str) -> String {
    let parts: Vec<&str> = s.split('"').collect();
    if parts.len() == 1 {
        format!("\"{}\"", s)
    } else {
        let inner = parts.join("\" & quote & \"");
        format!("\"{}\"", inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adapter_error_display() {
        let err = AdapterError::BinaryNotFound(PathBuf::from("/usr/bin/imsg"));
        assert!(err.to_string().contains("/usr/bin/imsg"));

        let err = AdapterError::CommandFailed {
            stderr: "not found".into(),
            exit_code: 1,
        };
        assert!(err.to_string().contains("exit 1"));
        assert!(err.to_string().contains("not found"));
    }

    #[test]
    fn send_result_serializes() {
        let result = SendResult {
            success: true,
            stdout: "sent".into(),
        };
        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["success"], true);
    }

    #[test]
    fn health_status_serializes() {
        let status = HealthStatus {
            binary_exists: true,
            db_exists: true,
            smoke_test_ok: true,
            error: None,
        };
        let json = serde_json::to_string(&status).unwrap();
        assert!(!json.contains("error")); // skipped when None
        assert!(json.contains("\"smoke_test_ok\":true"));
    }
}
