//! Middleware pipeline — runs security checks between parsing and dispatch.
//!
//! Order: rate limiting → content filtering.
//! On rejection: audit + dead letter + return error response.
//! On pass: audit as allowed + return Allow.

use crate::audit::{self, AuditLogger};
use crate::content_filter::{ContentCheckResult, ContentFilter};
use crate::dead_letter::{DeadLetter, DeadLetterQueue};
use crate::protocol::{self, JsonRpcRequest, JsonRpcResponse};
use crate::rate_limiter::{RateLimitResult, RateLimiter};

/// The verdict from the middleware pipeline.
pub enum MiddlewareVerdict {
    /// Request is allowed — proceed to handler dispatch.
    Allow,
    /// Request is rejected — return this error response to the client.
    Reject(JsonRpcResponse),
}

/// Run the full security pipeline for a request.
///
/// Checks are run in order. The first rejection short-circuits.
pub async fn run_pipeline(
    req: &JsonRpcRequest,
    raw_params: &str,
    rate_limiter: &RateLimiter,
    content_filter: &ContentFilter,
    audit_logger: &AuditLogger,
    dead_letter_queue: &DeadLetterQueue,
) -> MiddlewareVerdict {
    // 1. Rate limiting
    match rate_limiter.check_and_record(&req.method) {
        RateLimitResult::Allowed => {}
        RateLimitResult::Exceeded { limit, window_secs } => {
            let reason = format!(
                "Rate limit exceeded: {limit} requests per {window_secs}s"
            );

            audit_logger
                .log(audit::blocked(&req.method, &req.id, &reason))
                .await;

            dead_letter_queue
                .store(
                    DeadLetter::new(
                        req.method.clone(),
                        req.id.clone(),
                        req.params.clone(),
                        reason.clone(),
                    ),
                )
                .await;

            return MiddlewareVerdict::Reject(JsonRpcResponse::error(
                req.id.clone(),
                protocol::RATE_LIMITED,
                reason,
            ));
        }
    }

    // 2. Content filtering
    match content_filter.check(raw_params) {
        ContentCheckResult::Clean => {}
        ContentCheckResult::Blocked { pattern } => {
            let reason = "Request blocked by content filter".to_string();

            audit_logger
                .log(
                    audit::blocked(&req.method, &req.id, &reason)
                        .with_pattern(&pattern),
                )
                .await;

            dead_letter_queue
                .store(
                    DeadLetter::new(
                        req.method.clone(),
                        req.id.clone(),
                        req.params.clone(),
                        reason.clone(),
                    )
                    .with_pattern(&pattern),
                )
                .await;

            return MiddlewareVerdict::Reject(JsonRpcResponse::error(
                req.id.clone(),
                protocol::CONTENT_BLOCKED,
                reason,
            ));
        }
    }

    // All checks passed.
    audit_logger
        .log(audit::allowed(&req.method, &req.id))
        .await;

    MiddlewareVerdict::Allow
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        ContentFilterConfig, FilterAction, PatternEntry, RateLimitConfig, RateLimitEntry,
    };
    use serde_json::json;
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn make_req(method: &str, params: serde_json::Value) -> JsonRpcRequest {
        JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: json!(1),
            method: method.into(),
            params,
        }
    }

    fn noop_audit() -> AuditLogger {
        AuditLogger::new(PathBuf::from("/dev/null"), false)
    }

    fn noop_dead_letter() -> DeadLetterQueue {
        DeadLetterQueue::new(PathBuf::from("/tmp/carapace-test-dead-letters"))
    }

    fn no_limit() -> RateLimiter {
        RateLimiter::new(RateLimitConfig {
            entries: HashMap::new(),
        })
    }

    fn no_filter() -> ContentFilter {
        ContentFilter::new(&ContentFilterConfig {
            enabled: false,
            patterns: vec![],
        })
    }

    #[tokio::test]
    async fn allows_clean_request() {
        let req = make_req("ping", json!({}));
        let verdict = run_pipeline(
            &req,
            "{}",
            &no_limit(),
            &no_filter(),
            &noop_audit(),
            &noop_dead_letter(),
        )
        .await;
        assert!(matches!(verdict, MiddlewareVerdict::Allow));
    }

    #[tokio::test]
    async fn rejects_rate_limited() {
        let mut entries = HashMap::new();
        entries.insert(
            "default".into(),
            RateLimitEntry {
                requests: 1,
                per_seconds: 60,
            },
        );
        let limiter = RateLimiter::new(RateLimitConfig { entries });

        let req = make_req("ping", json!({}));

        // First request allowed.
        let v = run_pipeline(
            &req,
            "{}",
            &limiter,
            &no_filter(),
            &noop_audit(),
            &noop_dead_letter(),
        )
        .await;
        assert!(matches!(v, MiddlewareVerdict::Allow));

        // Second request rejected.
        let v = run_pipeline(
            &req,
            "{}",
            &limiter,
            &no_filter(),
            &noop_audit(),
            &noop_dead_letter(),
        )
        .await;
        match v {
            MiddlewareVerdict::Reject(resp) => {
                assert_eq!(resp.error.unwrap().code, protocol::RATE_LIMITED);
            }
            MiddlewareVerdict::Allow => panic!("should have been rejected"),
        }
    }

    #[tokio::test]
    async fn rejects_content_blocked() {
        let filter = ContentFilter::new(&ContentFilterConfig {
            enabled: true,
            patterns: vec![PatternEntry {
                pattern: r"(?i)password\s*[:=]".into(),
                action: FilterAction::Block,
            }],
        });

        let req = make_req("execute", json!({"command": "password = hunter2"}));
        let raw_params = r#"{"command": "password = hunter2"}"#;

        let v = run_pipeline(
            &req,
            raw_params,
            &no_limit(),
            &filter,
            &noop_audit(),
            &noop_dead_letter(),
        )
        .await;
        match v {
            MiddlewareVerdict::Reject(resp) => {
                assert_eq!(resp.error.unwrap().code, protocol::CONTENT_BLOCKED);
            }
            MiddlewareVerdict::Allow => panic!("should have been rejected"),
        }
    }

    #[tokio::test]
    async fn rate_limit_checked_before_content_filter() {
        // Both rate limit AND content filter would reject, but rate limit
        // should fire first.
        let mut entries = HashMap::new();
        entries.insert(
            "default".into(),
            RateLimitEntry {
                requests: 0, // Always exceeded
                per_seconds: 60,
            },
        );
        let limiter = RateLimiter::new(RateLimitConfig { entries });

        let filter = ContentFilter::new(&ContentFilterConfig {
            enabled: true,
            patterns: vec![PatternEntry {
                pattern: r"(?i)password\s*[:=]".into(),
                action: FilterAction::Block,
            }],
        });

        let req = make_req("execute", json!({"command": "password = hunter2"}));
        let raw_params = r#"{"command": "password = hunter2"}"#;

        let v = run_pipeline(
            &req,
            raw_params,
            &limiter,
            &filter,
            &noop_audit(),
            &noop_dead_letter(),
        )
        .await;
        match v {
            MiddlewareVerdict::Reject(resp) => {
                // Should be rate limited, not content blocked.
                assert_eq!(resp.error.unwrap().code, protocol::RATE_LIMITED);
            }
            MiddlewareVerdict::Allow => panic!("should have been rejected"),
        }
    }
}
