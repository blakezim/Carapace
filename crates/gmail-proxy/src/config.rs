//! Configuration loading for the Gmail proxy.
//!
//! The proxy uses two files:
//!   - `config.toml` — non-secret settings (paths, scrub rules, etc.)
//!   - `secrets.toml` — OAuth refresh token; must be 0600

use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize, Clone)]
pub struct AuthConfig {
    pub client_id: String,
    pub client_secret: String,
    pub secrets_file: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct GmailAccountConfig {
    pub account: String,
    #[serde(default)]
    pub watch_labels: Vec<String>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ScrubConfig {
    #[serde(default = "default_blocked_label")]
    pub blocked_label: String,
    #[serde(default)]
    pub strip_links: bool,
    #[serde(default)]
    pub otp_patterns: Vec<String>,
    #[serde(default)]
    pub blocked_sender_patterns: Vec<String>,
    #[serde(default)]
    pub url_strip_patterns: Vec<String>,
    #[serde(default = "default_allowed_operators")]
    pub allowed_operators: Vec<String>,
}

impl Default for ScrubConfig {
    fn default() -> Self {
        Self {
            blocked_label: default_blocked_label(),
            strip_links: false,
            otp_patterns: vec![
                r"(?i)\b\d{6}\b".into(),
                r"(?i)\b\d{4}\b".into(),
            ],
            blocked_sender_patterns: vec![],
            url_strip_patterns: vec![
                r"(?i)https?://[^\s]*(?:reset|verify|confirm|login|signin|auth|token)[^\s]*".into(),
            ],
            allowed_operators: default_allowed_operators(),
        }
    }
}

fn default_blocked_label() -> String {
    "AI-BLOCKED".into()
}

fn default_allowed_operators() -> Vec<String> {
    vec![
        "from".into(), "to".into(), "subject".into(),
        "after".into(), "before".into(), "older_than".into(), "newer_than".into(),
        "is".into(), "has".into(), "in".into(), "filename".into(),
        "cc".into(), "bcc".into(), "deliveredto".into(),
    ]
}

#[derive(Debug, Deserialize, Clone)]
pub struct ProxyConfig {
    #[serde(default = "default_socket_path")]
    pub socket_path: String,
    #[serde(default = "default_concurrency")]
    pub search_fetch_concurrency: usize,
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            socket_path: default_socket_path(),
            search_fetch_concurrency: default_concurrency(),
        }
    }
}

fn default_socket_path() -> String {
    "/var/run/carapace/gmail-proxy.sock".into()
}

fn default_concurrency() -> usize {
    4
}

#[derive(Debug, Deserialize, Clone)]
pub struct Secrets {
    pub refresh_token: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ConfigFile {
    pub auth: AuthConfig,
    pub gmail: GmailAccountConfig,
    #[serde(default)]
    pub scrub: ScrubConfig,
    #[serde(default)]
    pub proxy: ProxyConfig,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub auth: AuthConfig,
    pub gmail: GmailAccountConfig,
    pub scrub: ScrubConfig,
    pub proxy: ProxyConfig,
    pub secrets: Secrets,
}

/// Validate that all regex patterns in the config compile successfully.
pub fn validate_patterns(config: &ConfigFile) -> Result<()> {
    for (i, pat) in config.scrub.otp_patterns.iter().enumerate() {
        regex::Regex::new(pat)
            .with_context(|| format!("invalid regex in otp_patterns[{i}]: {pat}"))?;
    }
    for (i, pat) in config.scrub.blocked_sender_patterns.iter().enumerate() {
        regex::Regex::new(pat)
            .with_context(|| format!("invalid regex in blocked_sender_patterns[{i}]: {pat}"))?;
    }
    for (i, pat) in config.scrub.url_strip_patterns.iter().enumerate() {
        regex::Regex::new(pat)
            .with_context(|| format!("invalid regex in url_strip_patterns[{i}]: {pat}"))?;
    }
    Ok(())
}

/// Load configuration from the given TOML config file path.
///
/// Resolves the secrets file relative to the config file's parent directory.
/// On Unix, enforces that the secrets file has 0600 permissions.
pub fn load_config(path: &Path, skip_permission_check: bool) -> Result<Config> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read config file: {}", path.display()))?;

    let file: ConfigFile = toml::from_str(&text)
        .with_context(|| format!("failed to parse config file: {}", path.display()))?;

    validate_patterns(&file).context("config pattern validation failed")?;

    let config_dir = path.parent().unwrap_or(Path::new("."));
    let secrets_path = config_dir.join(&file.auth.secrets_file);

    if !skip_permission_check {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let metadata = std::fs::metadata(&secrets_path)
                .with_context(|| format!("failed to stat secrets file: {}", secrets_path.display()))?;
            let mode = metadata.permissions().mode() & 0o777;
            if mode != 0o600 {
                anyhow::bail!(
                    "secrets file {} has permissions {:04o}, expected 0600",
                    secrets_path.display(),
                    mode
                );
            }
        }
    }

    let secrets_text = std::fs::read_to_string(&secrets_path)
        .with_context(|| format!("failed to read secrets file: {}", secrets_path.display()))?;

    let secrets: Secrets = toml::from_str(&secrets_text)
        .with_context(|| format!("failed to parse secrets file: {}", secrets_path.display()))?;

    Ok(Config {
        auth: file.auth,
        gmail: file.gmail,
        scrub: file.scrub,
        proxy: file.proxy,
        secrets,
    })
}

/// Default config file path: ~/.config/gmail-proxy/config.toml
pub fn default_config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("gmail-proxy")
        .join("config.toml")
}
