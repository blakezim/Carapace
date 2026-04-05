//! Content scrubbing — redacts OTP codes, auth URLs, and optionally all links.

use regex::Regex;

pub struct ContentScrubber {
    otp_patterns: Vec<Regex>,
    url_strip_patterns: Vec<Regex>,
    blocked_sender_patterns: Vec<Regex>,
    strip_links: bool,
}

pub struct SenderCheckResult {
    pub blocked: bool,
    pub reason: Option<String>,
}

impl SenderCheckResult {
    pub fn is_blocked(&self) -> bool {
        self.blocked
    }
}

impl ContentScrubber {
    pub fn new(
        otp_patterns: Vec<Regex>,
        url_strip_patterns: Vec<Regex>,
        blocked_sender_patterns: Vec<Regex>,
        strip_links: bool,
    ) -> Self {
        Self {
            otp_patterns,
            url_strip_patterns,
            blocked_sender_patterns,
            strip_links,
        }
    }

    pub fn check_sender(&self, from: &str) -> SenderCheckResult {
        for pattern in &self.blocked_sender_patterns {
            if pattern.is_match(from) {
                return SenderCheckResult {
                    blocked: true,
                    reason: Some(format!("Sender matches blocked pattern: {pattern}")),
                };
            }
        }
        SenderCheckResult { blocked: false, reason: None }
    }

    pub fn scrub_body(&self, body: &str) -> String {
        let mut result = body.to_string();

        for pattern in &self.otp_patterns {
            result = pattern.replace_all(&result, "[REDACTED]").to_string();
        }

        for pattern in &self.url_strip_patterns {
            result = pattern.replace_all(&result, "[REDACTED]").to_string();
        }

        if self.strip_links {
            let url_pattern = Regex::new(r"https?://\S+").unwrap();
            result = url_pattern.replace_all(&result, "[link removed]").to_string();
        }

        result
    }
}
