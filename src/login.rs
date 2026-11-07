//! Session-token login: validate a captured dashboard JWT and persist it.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

use crate::api::ProxyCheapClient;
use crate::browser::{self, Environment, LoginOptions};
use crate::config::{self, ClientConfig};
use crate::error::{Error, Result};

#[derive(Debug, Clone)]
pub struct LoginOutcome {
    pub environment: Environment,
    pub expires_at: Option<i64>,
    pub profile_dir: PathBuf,
}

/// Launch the browser, capture the dashboard JWT, validate it against the REST
/// API, and save it to the config file.
pub async fn capture_and_store(
    config: &ClientConfig,
    path: &Path,
    options: &LoginOptions,
) -> Result<LoginOutcome> {
    let token = browser::capture_session(options).await?;
    store_token(
        config,
        path,
        &token,
        options.environment,
        options.profile_dir.clone(),
    )
    .await
}

/// Validate a JWT against `GET /account/balance` and persist it.
///
/// The token is only written after the REST API accepts it, so a dashboard
/// session that cannot call the documented API never overwrites good
/// credentials.
pub async fn store_token(
    config: &ClientConfig,
    path: &Path,
    token: &str,
    environment: Environment,
    profile_dir: PathBuf,
) -> Result<LoginOutcome> {
    validate_token(config, token).await?;

    let expires_at = jwt_expiry(token);
    if expires_at.is_some_and(|expiry| expiry <= now_unix()) {
        return Err(Error::Config(
            "the captured session token has already expired; run `proxy-cheap login` again"
                .to_string(),
        ));
    }

    let mut file = config::load_from(path)?.unwrap_or_default();
    file.session_token = Some(token.to_string());
    file.session_token_expires_at = expires_at;
    config::save_to(path, &file)?;

    Ok(LoginOutcome {
        environment,
        expires_at,
        profile_dir,
    })
}

async fn validate_token(config: &ClientConfig, token: &str) -> Result<()> {
    let validation = ClientConfig {
        api_key: None,
        api_secret: None,
        session_token: Some(token.to_string()),
        session_token_expires_at: None,
        base_url: config.base_url.clone(),
        timeout_secs: config.timeout_secs,
        max_retries: 0,
    };
    let client = ProxyCheapClient::new(&validation)?;
    match client.balance().await {
        Ok(_) => Ok(()),
        Err(Error::Api { status, .. })
            if status == 401 || status == 403 || (300..400).contains(&status) =>
        {
            Err(Error::Config(
                "the browser login succeeded, but the Proxy-Cheap REST API rejected the session \
                 token. A dashboard session may not grant API access. Create an API key at \
                 https://app.proxy-cheap.com/api-keys and run `proxy-cheap config init` instead."
                    .to_string(),
            ))
        }
        Err(error) => Err(error),
    }
}

/// Decode the `exp` claim from a JWT without verifying its signature.
pub fn jwt_expiry(token: &str) -> Option<i64> {
    let payload = token.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload).ok()?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    value.get("exp").and_then(serde_json::Value::as_i64)
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_jwt_expiry() {
        let payload = serde_json::json!({"exp": 1_893_456_000_i64, "sub": "test"});
        let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&payload).unwrap());
        let token = format!("header.{payload}.signature");
        assert_eq!(jwt_expiry(&token), Some(1_893_456_000));
    }

    #[test]
    fn handles_malformed_tokens() {
        assert_eq!(jwt_expiry("not-a-jwt"), None);
        assert_eq!(jwt_expiry("a.b.c"), None);
        assert_eq!(jwt_expiry(""), None);
    }
}
