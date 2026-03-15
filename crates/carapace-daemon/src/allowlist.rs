//! Allowlist enforcement for channel directions (outbound/inbound).
//!
//! Supports three modes:
//! - **Allowlist**: only identifiers in the list are allowed
//! - **Denylist**: all identifiers are allowed except those in the list
//! - **Open**: all identifiers are allowed (no filtering)

use crate::config::{AllowlistMode, DirectionConfig};

/// Result of an allowlist check.
#[derive(Debug, PartialEq, Eq)]
pub enum AllowlistResult {
    Allowed,
    Blocked { mode: String, identifier: String },
}

/// Allowlist checker constructed from a direction config.
#[derive(Clone)]
pub struct Allowlist {
    mode: AllowlistMode,
    /// Normalized entries (lowercase, trimmed).
    entries: Vec<String>,
}

impl Allowlist {
    /// Build from a direction config. Entries are normalized for matching.
    pub fn new(config: &DirectionConfig) -> Self {
        let entries = config
            .allowlist
            .iter()
            .map(|s| normalize(s))
            .collect();

        Self {
            mode: config.mode.clone(),
            entries,
        }
    }

    /// Check whether an identifier is allowed.
    pub fn check(&self, identifier: &str) -> AllowlistResult {
        let normalized = normalize(identifier);

        match self.mode {
            AllowlistMode::Open => AllowlistResult::Allowed,
            AllowlistMode::Allowlist => {
                if self.entries.contains(&normalized) {
                    AllowlistResult::Allowed
                } else {
                    AllowlistResult::Blocked {
                        mode: "allowlist".into(),
                        identifier: identifier.to_string(),
                    }
                }
            }
            AllowlistMode::Denylist => {
                if self.entries.contains(&normalized) {
                    AllowlistResult::Blocked {
                        mode: "denylist".into(),
                        identifier: identifier.to_string(),
                    }
                } else {
                    AllowlistResult::Allowed
                }
            }
        }
    }

    /// Return the number of entries in the list.
    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    /// Return the mode as a string.
    pub fn mode_str(&self) -> &str {
        match self.mode {
            AllowlistMode::Allowlist => "allowlist",
            AllowlistMode::Denylist => "denylist",
            AllowlistMode::Open => "open",
        }
    }
}

/// Normalize an identifier for comparison: trim, lowercase, strip `email:` prefix.
fn normalize(s: &str) -> String {
    let trimmed = s.trim();
    let stripped = trimmed
        .strip_prefix("email:")
        .or_else(|| trimmed.strip_prefix("EMAIL:"))
        .unwrap_or(trimmed);
    stripped.trim().to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AllowlistMode, DirectionConfig};

    fn make_config(mode: AllowlistMode, entries: Vec<&str>) -> DirectionConfig {
        DirectionConfig {
            mode,
            allowlist: entries.into_iter().map(String::from).collect(),
        }
    }

    #[test]
    fn allowlist_allows_listed() {
        let config = make_config(AllowlistMode::Allowlist, vec!["+1234567890", "friend@icloud.com"]);
        let al = Allowlist::new(&config);
        assert_eq!(al.check("+1234567890"), AllowlistResult::Allowed);
        assert_eq!(al.check("friend@icloud.com"), AllowlistResult::Allowed);
    }

    #[test]
    fn allowlist_blocks_unlisted() {
        let config = make_config(AllowlistMode::Allowlist, vec!["+1234567890"]);
        let al = Allowlist::new(&config);
        assert_eq!(
            al.check("+9999999999"),
            AllowlistResult::Blocked {
                mode: "allowlist".into(),
                identifier: "+9999999999".into(),
            }
        );
    }

    #[test]
    fn denylist_blocks_listed() {
        let config = make_config(AllowlistMode::Denylist, vec!["blocked@example.com"]);
        let al = Allowlist::new(&config);
        assert_eq!(
            al.check("blocked@example.com"),
            AllowlistResult::Blocked {
                mode: "denylist".into(),
                identifier: "blocked@example.com".into(),
            }
        );
    }

    #[test]
    fn denylist_allows_unlisted() {
        let config = make_config(AllowlistMode::Denylist, vec!["blocked@example.com"]);
        let al = Allowlist::new(&config);
        assert_eq!(al.check("friend@example.com"), AllowlistResult::Allowed);
    }

    #[test]
    fn open_allows_all() {
        let config = make_config(AllowlistMode::Open, vec![]);
        let al = Allowlist::new(&config);
        assert_eq!(al.check("anyone@example.com"), AllowlistResult::Allowed);
        assert_eq!(al.check("+0000000000"), AllowlistResult::Allowed);
    }

    #[test]
    fn case_insensitive_matching() {
        let config = make_config(AllowlistMode::Allowlist, vec!["Friend@iCloud.com"]);
        let al = Allowlist::new(&config);
        assert_eq!(al.check("friend@icloud.com"), AllowlistResult::Allowed);
        assert_eq!(al.check("FRIEND@ICLOUD.COM"), AllowlistResult::Allowed);
    }

    #[test]
    fn trims_whitespace() {
        let config = make_config(AllowlistMode::Allowlist, vec!["  +1234567890  "]);
        let al = Allowlist::new(&config);
        assert_eq!(al.check("+1234567890"), AllowlistResult::Allowed);
        assert_eq!(al.check("  +1234567890  "), AllowlistResult::Allowed);
    }

    #[test]
    fn strips_email_prefix() {
        let config = make_config(AllowlistMode::Allowlist, vec!["email:user@example.com"]);
        let al = Allowlist::new(&config);
        assert_eq!(al.check("user@example.com"), AllowlistResult::Allowed);
        assert_eq!(al.check("email:user@example.com"), AllowlistResult::Allowed);
    }

    #[test]
    fn empty_allowlist_blocks_all() {
        let config = make_config(AllowlistMode::Allowlist, vec![]);
        let al = Allowlist::new(&config);
        assert_eq!(
            al.check("anyone@example.com"),
            AllowlistResult::Blocked {
                mode: "allowlist".into(),
                identifier: "anyone@example.com".into(),
            }
        );
    }

    #[test]
    fn entry_count_and_mode() {
        let config = make_config(AllowlistMode::Denylist, vec!["a", "b", "c"]);
        let al = Allowlist::new(&config);
        assert_eq!(al.entry_count(), 3);
        assert_eq!(al.mode_str(), "denylist");
    }
}
