//! Audit logging — append-only JSON-line log of every request.
//!
//! Audit failures are logged via tracing but **never** propagated to callers.
//! A broken audit log must not block legitimate requests.

use std::path::PathBuf;
use std::time::SystemTime;

use serde::Serialize;
use tokio::fs::OpenOptions;
use tokio::io::AsyncWriteExt;
use tracing::warn;

/// Append-only audit logger.
pub struct AuditLogger {
    path: PathBuf,
    enabled: bool,
}

/// Status recorded for an audited request.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditStatus {
    Allowed,
    Blocked,
    Error,
}

/// A single audit log entry, serialized as one JSON line.
#[derive(Debug, Serialize)]
pub struct AuditEntry {
    pub timestamp: String,
    pub method: String,
    pub request_id: serde_json::Value,
    pub status: AuditStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub matched_pattern: Option<String>,
}

impl AuditLogger {
    pub fn new(path: PathBuf, enabled: bool) -> Self {
        Self { path, enabled }
    }

    /// Append an audit entry as a JSON line.
    ///
    /// Errors are swallowed — audit must never block request processing.
    pub async fn log(&self, entry: AuditEntry) {
        if !self.enabled {
            return;
        }
        if let Err(e) = self.write_entry(&entry).await {
            warn!(error = %e, "audit log write failed");
        }
    }

    async fn write_entry(&self, entry: &AuditEntry) -> std::io::Result<()> {
        // Ensure parent directory exists.
        if let Some(parent) = self.path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .await?;

        let mut line = serde_json::to_string(entry)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        line.push('\n');
        file.write_all(line.as_bytes()).await?;
        Ok(())
    }
}

impl AuditEntry {
    pub fn new(
        method: String,
        request_id: serde_json::Value,
        status: AuditStatus,
    ) -> Self {
        Self {
            timestamp: now_rfc3339(),
            method,
            request_id,
            status,
            reason: None,
            matched_pattern: None,
        }
    }

    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }

    pub fn with_pattern(mut self, pattern: impl Into<String>) -> Self {
        self.matched_pattern = Some(pattern.into());
        self
    }
}

/// Format current time as ISO-8601 / RFC-3339 without pulling in `chrono`.
pub fn now_rfc3339() -> String {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();

    let secs = now.as_secs();

    // Break epoch seconds into date/time components.
    let days = secs / 86400;
    let time_secs = secs % 86400;
    let hours = time_secs / 3600;
    let minutes = (time_secs % 3600) / 60;
    let seconds = time_secs % 60;

    // Convert days since epoch to Y-M-D (simplified Gregorian).
    let (year, month, day) = days_to_ymd(days);

    format!(
        "{year:04}-{month:02}-{day:02}T{hours:02}:{minutes:02}:{seconds:02}Z"
    )
}

/// Convert days since Unix epoch to (year, month, day).
fn days_to_ymd(days: u64) -> (u64, u64, u64) {
    // Algorithm from http://howardhinnant.github.io/date_algorithms.html
    let z = days + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

/// Convenience: build an "allowed" entry.
pub fn allowed(method: &str, id: &serde_json::Value) -> AuditEntry {
    AuditEntry::new(method.to_string(), id.clone(), AuditStatus::Allowed)
}

/// Convenience: build a "blocked" entry.
pub fn blocked(method: &str, id: &serde_json::Value, reason: &str) -> AuditEntry {
    AuditEntry::new(method.to_string(), id.clone(), AuditStatus::Blocked)
        .with_reason(reason)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn now_rfc3339_format() {
        let ts = now_rfc3339();
        // Should look like "2025-01-15T12:34:56Z"
        assert!(ts.ends_with('Z'));
        assert_eq!(ts.len(), 20);
        assert_eq!(&ts[4..5], "-");
        assert_eq!(&ts[7..8], "-");
        assert_eq!(&ts[10..11], "T");
    }

    #[test]
    fn audit_entry_builder() {
        let entry = AuditEntry::new(
            "execute".into(),
            serde_json::json!(42),
            AuditStatus::Blocked,
        )
        .with_reason("rate limited")
        .with_pattern("n/a");

        let json = serde_json::to_string(&entry).unwrap();
        assert!(json.contains("\"method\":\"execute\""));
        assert!(json.contains("\"status\":\"blocked\""));
        assert!(json.contains("\"reason\":\"rate limited\""));
    }

    #[test]
    fn disabled_logger_is_noop() {
        let logger = AuditLogger::new(PathBuf::from("/nonexistent/path"), false);
        // Should not panic or error since it's disabled.
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            logger
                .log(AuditEntry::new(
                    "ping".into(),
                    serde_json::json!(1),
                    AuditStatus::Allowed,
                ))
                .await;
        });
    }
}
