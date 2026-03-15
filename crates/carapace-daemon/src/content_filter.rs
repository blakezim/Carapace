//! Regex-based content filtering.
//!
//! Scans the serialized JSON params of a request for sensitive patterns
//! (passwords, API keys, SSNs, etc.). Patterns with action `Block` reject
//! the request; patterns with action `Warn` log a warning but allow it.

use regex::Regex;
use tracing::warn;

use crate::config::{ContentFilterConfig, FilterAction};

/// Result of a content check.
#[derive(Debug, PartialEq, Eq)]
pub enum ContentCheckResult {
    Clean,
    Blocked { pattern: String },
}

/// A compiled pattern with its action.
struct CompiledPattern {
    regex: Regex,
    source: String,
    action: FilterAction,
}

/// Content filter with pre-compiled regex patterns.
pub struct ContentFilter {
    enabled: bool,
    patterns: Vec<CompiledPattern>,
}

impl ContentFilter {
    /// Construct from config. All patterns are compiled at construction time
    /// so invalid regex is caught early (config validation should have already
    /// verified this, but we handle errors gracefully here too).
    pub fn new(config: &ContentFilterConfig) -> Self {
        let patterns = config
            .patterns
            .iter()
            .filter_map(|entry| {
                match Regex::new(&entry.pattern) {
                    Ok(regex) => Some(CompiledPattern {
                        regex,
                        source: entry.pattern.clone(),
                        action: entry.action.clone(),
                    }),
                    Err(e) => {
                        warn!(pattern = %entry.pattern, error = %e, "skipping invalid regex");
                        None
                    }
                }
            })
            .collect();

        Self {
            enabled: config.enabled,
            patterns,
        }
    }

    /// Scan content for matching patterns.
    ///
    /// `content` should be the full serialized JSON params string.
    /// Returns `Blocked` on the first blocking match, or `Clean` if nothing
    /// blocks. Warn-action matches are logged but do not block.
    pub fn check(&self, content: &str) -> ContentCheckResult {
        if !self.enabled {
            return ContentCheckResult::Clean;
        }

        for pattern in &self.patterns {
            if pattern.regex.is_match(content) {
                match pattern.action {
                    FilterAction::Block => {
                        return ContentCheckResult::Blocked {
                            pattern: pattern.source.clone(),
                        };
                    }
                    FilterAction::Warn => {
                        warn!(
                            pattern = %pattern.source,
                            "content filter warning match (not blocking)"
                        );
                    }
                }
            }
        }

        ContentCheckResult::Clean
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{FilterAction, PatternEntry};

    fn filter_with(patterns: Vec<PatternEntry>) -> ContentFilter {
        ContentFilter::new(&ContentFilterConfig {
            enabled: true,
            patterns,
        })
    }

    #[test]
    fn clean_content_passes() {
        let filter = filter_with(vec![PatternEntry {
            pattern: r"(?i)password\s*[:=]".into(),
            action: FilterAction::Block,
        }]);
        assert_eq!(
            filter.check(r#"{"message": "hello world"}"#),
            ContentCheckResult::Clean
        );
    }

    #[test]
    fn blocked_pattern_catches() {
        let filter = filter_with(vec![PatternEntry {
            pattern: r"(?i)password\s*[:=]".into(),
            action: FilterAction::Block,
        }]);
        assert_eq!(
            filter.check(r#"{"message": "password = hunter2"}"#),
            ContentCheckResult::Blocked {
                pattern: r"(?i)password\s*[:=]".into()
            }
        );
    }

    #[test]
    fn warn_does_not_block() {
        let filter = filter_with(vec![PatternEntry {
            pattern: r"(?i)password\s*[:=]".into(),
            action: FilterAction::Warn,
        }]);
        assert_eq!(
            filter.check(r#"{"message": "password = hunter2"}"#),
            ContentCheckResult::Clean
        );
    }

    #[test]
    fn disabled_filter_allows_everything() {
        let filter = ContentFilter::new(&ContentFilterConfig {
            enabled: false,
            patterns: vec![PatternEntry {
                pattern: r".*".into(),
                action: FilterAction::Block,
            }],
        });
        assert_eq!(
            filter.check("literally anything"),
            ContentCheckResult::Clean
        );
    }

    #[test]
    fn ssn_pattern_blocks() {
        let filter = filter_with(vec![PatternEntry {
            pattern: r"\b\d{3}-\d{2}-\d{4}\b".into(),
            action: FilterAction::Block,
        }]);
        assert_eq!(
            filter.check(r#"{"ssn": "123-45-6789"}"#),
            ContentCheckResult::Blocked {
                pattern: r"\b\d{3}-\d{2}-\d{4}\b".into()
            }
        );
    }

    #[test]
    fn scans_nested_json_args() {
        let filter = filter_with(vec![PatternEntry {
            pattern: r"(?i)api[_-]?key\s*[:=]".into(),
            action: FilterAction::Block,
        }]);
        let nested = r#"{"args": {"config": {"api_key = sk-12345"}}}"#;
        assert_eq!(
            filter.check(nested),
            ContentCheckResult::Blocked {
                pattern: r"(?i)api[_-]?key\s*[:=]".into()
            }
        );
    }
}
