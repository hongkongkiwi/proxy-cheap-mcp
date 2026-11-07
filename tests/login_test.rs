mod common;

use proxy_cheap_mcp::browser::Environment;
use proxy_cheap_mcp::config::{self, ClientConfig, FileConfig};
use proxy_cheap_mcp::login;

fn config(base_url: &str) -> ClientConfig {
    ClientConfig {
        api_key: None,
        api_secret: None,
        session_token: None,
        session_token_expires_at: None,
        base_url: base_url.to_string(),
        timeout_secs: 5,
        max_retries: 0,
    }
}

#[tokio::test]
async fn stores_a_valid_token() {
    let mock = common::start().await;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("proxy-cheap/config.toml");

    let outcome = login::store_token(
        &config(mock.base_url()),
        &path,
        "test-token",
        Environment::Production,
        directory.path().join("profile"),
    )
    .await
    .unwrap();
    assert_eq!(outcome.environment, Environment::Production);
    assert!(outcome.expires_at.is_none());

    let file = config::load_from(&path).unwrap().unwrap();
    assert_eq!(file.session_token.as_deref(), Some("test-token"));
    assert!(file.session_token_expires_at.is_none());
    assert_eq!(
        mock.last().header("authorization").as_deref(),
        Some("Bearer test-token")
    );
}

#[tokio::test]
async fn rejects_a_token_the_api_refuses() {
    let mock = common::start().await;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("config.toml");

    let error = login::store_token(
        &config(mock.base_url()),
        &path,
        "bad-token",
        Environment::Production,
        directory.path().to_path_buf(),
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("rejected the session token"));
    assert!(!path.exists());
}

#[tokio::test]
async fn preserves_existing_api_keys() {
    let mock = common::start().await;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("config.toml");
    config::save_to(
        &path,
        &FileConfig {
            api_key: Some("key".to_string()),
            api_secret: Some("secret".to_string()),
            session_token: None,
            session_token_expires_at: None,
            base_url: None,
        },
    )
    .unwrap();

    login::store_token(
        &config(mock.base_url()),
        &path,
        "test-token",
        Environment::Production,
        directory.path().join("profile"),
    )
    .await
    .unwrap();

    let file = config::load_from(&path).unwrap().unwrap();
    assert_eq!(file.api_key.as_deref(), Some("key"));
    assert_eq!(file.api_secret.as_deref(), Some("secret"));
    assert_eq!(file.session_token.as_deref(), Some("test-token"));
}
