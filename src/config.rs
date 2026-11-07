//! Credential and endpoint configuration.
//!
//! Resolution order, highest precedence first:
//! 1. command-line flags
//! 2. environment variables (`PROXY_CHEAP_API_KEY`, `PROXY_CHEAP_API_SECRET`,
//!    `PROXY_CHEAP_SESSION_TOKEN`, `PROXY_CHEAP_BASE_URL`)
//! 3. the config file (`$XDG_CONFIG_HOME/proxy-cheap/config.toml`, default
//!    `~/.config/proxy-cheap/config.toml` on Unix)
//! 4. the public API defaults

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

pub const DEFAULT_BASE_URL: &str = "https://api.proxy-cheap.com";
pub const DEFAULT_TIMEOUT_SECS: u64 = 30;
pub const DEFAULT_RETRIES: u32 = 2;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct FileConfig {
    pub api_key: Option<String>,
    pub api_secret: Option<String>,
    /// Dashboard session JWT captured by `proxy-cheap login`.
    pub session_token: Option<String>,
    /// JWT expiry as Unix seconds, decoded locally for display and re-login.
    pub session_token_expires_at: Option<i64>,
    pub base_url: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ClientConfig {
    pub api_key: Option<String>,
    pub api_secret: Option<String>,
    pub session_token: Option<String>,
    pub session_token_expires_at: Option<i64>,
    pub base_url: String,
    pub timeout_secs: u64,
    pub max_retries: u32,
}

impl ClientConfig {
    pub fn public(base_url: Option<String>) -> Result<Self> {
        Ok(Self {
            api_key: None,
            api_secret: None,
            session_token: None,
            session_token_expires_at: None,
            base_url: normalise_base_url(base_url.as_deref().unwrap_or(DEFAULT_BASE_URL))?,
            timeout_secs: DEFAULT_TIMEOUT_SECS,
            max_retries: DEFAULT_RETRIES,
        })
    }

    pub fn resolve(
        api_key: Option<String>,
        api_secret: Option<String>,
        session_token: Option<String>,
        base_url: Option<String>,
        timeout_secs: u64,
        max_retries: u32,
    ) -> Result<Self> {
        let file = load_file()?.unwrap_or_default();
        let api_key = non_empty(api_key.or(file.api_key));
        let api_secret = non_empty(api_secret.or(file.api_secret));
        let session_token = non_empty(session_token.or(file.session_token));
        let session_token_expires_at = file.session_token_expires_at;
        let base_url =
            non_empty(base_url.or(file.base_url)).unwrap_or_else(|| DEFAULT_BASE_URL.to_string());
        Ok(Self {
            api_key,
            api_secret,
            session_token,
            session_token_expires_at,
            base_url: normalise_base_url(&base_url)?,
            timeout_secs,
            max_retries,
        })
    }

    pub fn has_api_key(&self) -> bool {
        self.api_key.is_some() && self.api_secret.is_some()
    }

    pub fn has_session_token(&self) -> bool {
        self.session_token.is_some()
    }

    /// Which credential the HTTP client will send. API keys win over a session
    /// token when both are configured.
    pub fn auth_method(&self) -> &'static str {
        if self.has_api_key() {
            "api-key"
        } else if self.has_session_token() {
            "session-token"
        } else {
            "none"
        }
    }

    pub fn require_auth(&self) -> Result<()> {
        if self.has_api_key() || self.has_session_token() {
            Ok(())
        } else {
            Err(Error::MissingCredentials)
        }
    }
}

fn non_empty(value: Option<String>) -> Option<String> {
    value
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
}

pub fn normalise_base_url(input: &str) -> Result<String> {
    let trimmed = input.trim().trim_end_matches('/');
    let url = reqwest::Url::parse(trimmed)
        .map_err(|error| Error::Config(format!("invalid base URL {input:?}: {error}")))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(Error::Config(format!(
            "invalid base URL {input:?}: expected http or https"
        )));
    }
    if url.host_str().is_none() {
        return Err(Error::Config(format!(
            "invalid base URL {input:?}: missing host"
        )));
    }
    Ok(trimmed.to_string())
}

/// Configuration root. Uses XDG on Unix (including macOS) so the documented
/// `~/.config/proxy-cheap` path is the real one.
pub fn config_home() -> PathBuf {
    if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(dir);
    }
    if cfg!(unix) {
        return dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".config");
    }
    dirs::config_dir().unwrap_or_else(|| PathBuf::from("."))
}

pub fn config_path() -> PathBuf {
    config_home().join("proxy-cheap").join("config.toml")
}

/// Dedicated Chromium profile used by `proxy-cheap login`.
pub fn browser_profile_dir() -> PathBuf {
    config_home().join("proxy-cheap").join("browser-profile")
}

pub fn load_file() -> Result<Option<FileConfig>> {
    load_from(&config_path())
}

pub fn load_from(path: &Path) -> Result<Option<FileConfig>> {
    match fs::read_to_string(path) {
        Ok(text) => {
            let config: FileConfig = toml::from_str(&text).map_err(|error| {
                Error::Config(format!("invalid config file {}: {error}", path.display()))
            })?;
            Ok(Some(config))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(Error::Config(format!(
            "could not read config file {}: {error}",
            path.display()
        ))),
    }
}

pub fn save_to(path: &Path, config: &FileConfig) -> Result<()> {
    if let Some(parent) = path.parent() {
        let created = !parent.exists();
        fs::create_dir_all(parent).map_err(|error| {
            Error::Config(format!("could not create {}: {error}", parent.display()))
        })?;
        if created {
            restrict_permissions(parent, 0o700)?;
        }
    }
    let text = toml::to_string_pretty(config)
        .map_err(|error| Error::Config(format!("could not serialise config: {error}")))?;
    fs::write(path, text)
        .map_err(|error| Error::Config(format!("could not write {}: {error}", path.display())))?;
    restrict_permissions(path, 0o600)?;
    Ok(())
}

#[cfg(unix)]
pub fn restrict_permissions(path: &Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let permissions = fs::Permissions::from_mode(mode);
    fs::set_permissions(path, permissions)
        .map_err(|error| Error::Config(format!("could not secure {}: {error}", path.display())))
}

#[cfg(not(unix))]
pub fn restrict_permissions(_path: &Path, _mode: u32) -> Result<()> {
    Ok(())
}

pub fn redact(secret: &str) -> String {
    if secret.chars().count() <= 8 {
        "********".to_string()
    } else {
        let prefix: String = secret.chars().take(4).collect();
        let suffix: String = secret
            .chars()
            .rev()
            .take(4)
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        format!("{prefix}...{suffix}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalises_trailing_slash() {
        assert_eq!(
            normalise_base_url("https://api.proxy-cheap.com/").unwrap(),
            "https://api.proxy-cheap.com"
        );
    }

    #[test]
    fn rejects_non_http_urls() {
        assert!(normalise_base_url("ftp://example.com").is_err());
        assert!(normalise_base_url("not a url").is_err());
    }

    #[test]
    fn redacts_long_secrets() {
        assert_eq!(redact("abcdefghijklmnop"), "abcd...mnop");
        assert_eq!(redact("short"), "********");
    }

    #[test]
    fn missing_credentials_are_reported() {
        let config = ClientConfig::public(None).unwrap();
        assert!(matches!(
            config.require_auth(),
            Err(Error::MissingCredentials)
        ));
        assert_eq!(config.auth_method(), "none");
    }

    #[test]
    fn session_tokens_count_as_credentials() {
        let mut config = ClientConfig::public(None).unwrap();
        config.session_token = Some("token".to_string());
        assert!(config.require_auth().is_ok());
        assert_eq!(config.auth_method(), "session-token");
        config.api_key = Some("key".to_string());
        config.api_secret = Some("secret".to_string());
        assert_eq!(config.auth_method(), "api-key");
    }

    #[test]
    fn saves_and_loads_a_private_config_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("proxy-cheap/config.toml");
        let config = FileConfig {
            api_key: Some("key".to_string()),
            api_secret: Some("secret".to_string()),
            session_token: Some("header.payload.signature".to_string()),
            session_token_expires_at: Some(1_893_456_000),
            base_url: Some("https://api.proxy-cheap.com".to_string()),
        };
        save_to(&path, &config).unwrap();

        let loaded = load_from(&path).unwrap().unwrap();
        assert_eq!(loaded.api_key.as_deref(), Some("key"));
        assert_eq!(loaded.api_secret.as_deref(), Some("secret"));
        assert_eq!(
            loaded.session_token.as_deref(),
            Some("header.payload.signature")
        );
        assert_eq!(loaded.session_token_expires_at, Some(1_893_456_000));
        assert_eq!(
            loaded.base_url.as_deref(),
            Some("https://api.proxy-cheap.com")
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
            let parent_mode = fs::metadata(path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(parent_mode, 0o700);
        }
    }

    #[test]
    fn config_path_ends_with_the_expected_suffix() {
        assert!(config_path().ends_with("proxy-cheap/config.toml"));
        assert!(browser_profile_dir().ends_with("proxy-cheap/browser-profile"));
    }
}
