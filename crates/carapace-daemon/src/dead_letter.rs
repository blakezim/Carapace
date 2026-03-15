//! Dead letter queue — stores blocked request metadata for review.
//!
//! Each blocked request is written as a pretty-printed JSON file in the
//! dead letters directory. Like audit logging, errors are swallowed to
//! avoid blocking request processing.

use std::path::PathBuf;

use serde::Serialize;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tracing::warn;

use crate::audit::now_rfc3339;

/// Stores blocked requests as JSON files.
pub struct DeadLetterQueue {
    dir: PathBuf,
}

/// Metadata about a blocked request.
#[derive(Debug, Serialize)]
pub struct DeadLetter {
    pub timestamp: String,
    pub method: String,
    pub request_id: serde_json::Value,
    pub params: serde_json::Value,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matched_pattern: Option<String>,
}

impl DeadLetterQueue {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// Store a dead letter. Errors are logged but never propagated.
    pub async fn store(&self, letter: DeadLetter) {
        if let Err(e) = self.write_letter(&letter).await {
            warn!(error = %e, "dead letter write failed");
        }
    }

    async fn write_letter(&self, letter: &DeadLetter) -> std::io::Result<()> {
        fs::create_dir_all(&self.dir).await?;

        // Filename: {timestamp}_{id}.json — sanitise timestamp for filesystem.
        let ts = letter.timestamp.replace(':', "-");
        let id = match &letter.request_id {
            serde_json::Value::Number(n) => n.to_string(),
            serde_json::Value::String(s) => s.clone(),
            other => format!("{other}"),
        };
        let filename = format!("{ts}_{id}.json");
        let path = self.dir.join(filename);

        let json = serde_json::to_string_pretty(letter)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

        let mut file = fs::File::create(&path).await?;
        file.write_all(json.as_bytes()).await?;
        file.write_all(b"\n").await?;
        Ok(())
    }
}

impl DeadLetter {
    pub fn new(
        method: String,
        request_id: serde_json::Value,
        params: serde_json::Value,
        reason: String,
    ) -> Self {
        Self {
            timestamp: now_rfc3339(),
            method,
            request_id,
            params,
            reason,
            matched_pattern: None,
        }
    }

    pub fn with_pattern(mut self, pattern: impl Into<String>) -> Self {
        self.matched_pattern = Some(pattern.into());
        self
    }
}
