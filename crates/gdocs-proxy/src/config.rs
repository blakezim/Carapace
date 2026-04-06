//! Configuration loading for the Google Docs proxy.
//!
//! Two-file structure (same pattern as gmail-proxy):
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
pub struct AccountConfig {
    /// The Google account email address.
    pub account: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ScrubConfig {
    /// Strip all links from document text output.
    #[serde(default)]
    pub strip_links: bool,
    /// Regex patterns to redact from document content (e.g. OTPs, tokens).
    #[serde(default)]
    pub redact_patterns: Vec<String>,
    /// Folder IDs that are blocked from search results and reads.
    /// Files anywhere inside these folders (including nested subfolders) are hidden.
    #[serde(default)]
    pub blocked_folders: Vec<String>,
}

impl Default for ScrubConfig {
    fn default() -> Self {
        Self {
            strip_links: false,
            redact_patterns: vec![],
            blocked_folders: vec![],
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct ProxyConfig {
    #[serde(default = "default_socket_path")]
    pub socket_path: String,
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            socket_path: default_socket_path(),
        }
    }
}

fn default_socket_path() -> String {
    "/var/run/carapace/gdocs-proxy.sock".into()
}

#[derive(Debug, Deserialize, Clone)]
pub struct Secrets {
    pub refresh_token: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ConfigFile {
    pub auth: AuthConfig,
    pub gdocs: AccountConfig,
    #[serde(default)]
    pub scrub: ScrubConfig,
    #[serde(default)]
    pub proxy: ProxyConfig,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub auth: AuthConfig,
    pub gdocs: AccountConfig,
    pub scrub: ScrubConfig,
    pub proxy: ProxyConfig,
    pub secrets: Secrets,
}

/// Validate that all regex patterns in the config compile successfully.
pub fn validate_patterns(config: &ConfigFile) -> Result<()> {
    for (i, pat) in config.scrub.redact_patterns.iter().enumerate() {
        regex::Regex::new(pat)
            .with_context(|| format!("invalid regex in redact_patterns[{i}]: {pat}"))?;
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
        gdocs: file.gdocs,
        scrub: file.scrub,
        proxy: file.proxy,
        secrets,
    })
}

/// Default config file path.
pub fn default_config_path() -> PathBuf {
    PathBuf::from("/etc/carapace/gdocs-proxy.toml")
}
