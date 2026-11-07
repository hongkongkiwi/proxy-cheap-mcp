//! Credential and endpoint configuration.
//!
//! Resolution order, highest precedence first:
//! 1. command-line flags
//! 2. environment variables (`PROXY_CHEAP_API_KEY`, `PROXY_CHEAP_API_SECRET`, `PROXY_CHEAP_BASE_URL`)
//! 3. the config file (`$XDG_CONFIG_HOME/proxy-cheap/config.toml`, default `~/.config/proxy-cheap/config.toml`)
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
    pub base_url: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ClientConfig {
    pub api_key: Option<String>,
    pub api_secret: Option<String>,
    pub base_url: String,
    pub timeout_secs: u64,
    pub max_retries: u32,
}

impl ClientConfig {
    pub fn public(base_url: Option<String>) -> Result<Self> {
        Ok(Self {
            api_key: None,
            api_secret: None,
            base_url: normalise_base_url(base_url.as_deref().unwrap_or(DEFAULT_BASE_URL))?,
            timeout_secs: DEFAULT_TIMEOUT_SECS,
            max_retries: DEFAULT_RETRIES,
        })
    }

    pub fn resolve(
        api_key: Option<String>,
        api_secret: Option<String>,
        base_url: Option<String>,
        timeout_secs: u64,
        max_retries: u32,
    ) -> Result<Self> {
        let file = load_file()?.unwrap_or_default();
        let api_key = non_empty(api_key.or(file.api_key));
        let api_secret = non_empty(api_secret.or(file.api_secret));
        let base_url =
            non_empty(base_url.or(file.base_url)).unwrap_or_else(|| DEFAULT_BASE_URL.to_string());
        Ok(Self {
            api_key,
            api_secret,
            base_url: normalise_base_url(&base_url)?,
            timeout_secs,
            max_retries,
        })
    }

    pub fn require_credentials(&self) -> Result<(&str, &str)> {
        match (&self.api_key, &self.api_secret) {
            (Some(key), Some(secret)) => Ok((key, secret)),
            _ => Err(Error::MissingCredentials),
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

pub fn config_path() -> PathBuf {
    if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(dir).join("proxy-cheap").join("config.toml");
    }
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("proxy-cheap")
        .join("config.toml")
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
        fs::create_dir_all(parent).map_err(|error| {
            Error::Config(format!("could not create {}: {error}", parent.display()))
        })?;
    }
    let text = toml::to_string_pretty(config)
        .map_err(|error| Error::Config(format!("could not serialise config: {error}")))?;
    fs::write(path, text)
        .map_err(|error| Error::Config(format!("could not write {}: {error}", path.display())))?;
    restrict_permissions(path)?;
    Ok(())
}

#[cfg(unix)]
fn restrict_permissions(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let permissions = fs::Permissions::from_mode(0o600);
    fs::set_permissions(path, permissions)
        .map_err(|error| Error::Config(format!("could not secure {}: {error}", path.display())))
}

#[cfg(not(unix))]
fn restrict_permissions(_path: &Path) -> Result<()> {
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
            config.require_credentials(),
            Err(Error::MissingCredentials)
        ));
    }

    #[test]
    fn saves_and_loads_a_private_config_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("proxy-cheap/config.toml");
        let config = FileConfig {
            api_key: Some("key".to_string()),
            api_secret: Some("secret".to_string()),
            base_url: Some("https://api.proxy-cheap.com".to_string()),
        };
        save_to(&path, &config).unwrap();

        let loaded = load_from(&path).unwrap().unwrap();
        assert_eq!(loaded.api_key.as_deref(), Some("key"));
        assert_eq!(loaded.api_secret.as_deref(), Some("secret"));
        assert_eq!(
            loaded.base_url.as_deref(),
            Some("https://api.proxy-cheap.com")
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
    }
}
