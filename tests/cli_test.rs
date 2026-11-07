mod common;

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

use serde_json::{Value, json};

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_proxy-cheap")
}

fn run(args: &[&str]) -> std::process::Output {
    Command::new(binary())
        .args(args)
        .output()
        .expect("failed to run proxy-cheap")
}

fn stdout_json(output: &std::process::Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "stdout was not JSON ({error}): {} / stderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

#[test]
fn help_lists_the_command_groups() {
    let output = run(&["--help"]);
    assert!(output.status.success());
    let text = String::from_utf8_lossy(&output.stdout);
    for command in [
        "services", "orders", "proxies", "legacy", "mcp", "webhook", "skill", "config",
    ] {
        assert!(text.contains(command), "help is missing {command}: {text}");
    }
}

#[test]
fn mcp_tools_prints_every_definition() {
    let output = run(&["mcp", "tools"]);
    assert!(output.status.success());
    let tools = stdout_json(&output);
    assert_eq!(tools.as_array().unwrap().len(), 25);
    let names: Vec<&str> = tools
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    assert!(names.contains(&"place_order"));
    assert!(names.contains(&"get_account_balance"));
}

#[test]
fn skill_prints_the_bundled_markdown() {
    let output = run(&["skill"]);
    assert!(output.status.success());
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.starts_with("---\nname: proxy-cheap"));
    assert!(text.contains("proxy-cheap services price"));
}

#[test]
fn config_path_uses_xdg_config_home() {
    let directory = tempfile::tempdir().unwrap();
    let output = Command::new(binary())
        .args(["config", "path"])
        .env("XDG_CONFIG_HOME", directory.path())
        .output()
        .unwrap();
    assert!(output.status.success());
    let path = stdout_json(&output)["path"].as_str().unwrap().to_string();
    assert_eq!(
        path,
        directory
            .path()
            .join("proxy-cheap/config.toml")
            .display()
            .to_string()
    );
}

#[test]
fn webhook_verify_accepts_a_valid_signature() {
    let secret = "test-secret";
    let event = "proxy.status.changed";
    let event_id = "evt-1";
    let body = r#"{"proxyId":1410379,"status":"CANCELED"}"#;
    let signature =
        proxy_cheap_mcp::webhook::signature(secret, event, event_id, body.as_bytes()).unwrap();
    let output = run(&[
        "webhook",
        "verify",
        "--secret",
        secret,
        "--event",
        event,
        "--event-id",
        event_id,
        "--body",
        body,
        "--signature",
        &signature,
    ]);
    assert!(output.status.success());
    let result = stdout_json(&output);
    assert_eq!(result["valid"], true);
    assert_eq!(result["expected"], signature);
}

#[test]
fn webhook_verify_fails_closed_with_exit_code_6() {
    let output = run(&[
        "webhook",
        "verify",
        "--secret",
        "test-secret",
        "--event",
        "proxy.status.changed",
        "--body",
        "{}",
        "--signature",
        "sha256=0000000000000000000000000000000000000000000000000000000000000000",
    ]);
    assert_eq!(output.status.code(), Some(6));
    let result = stdout_json(&output);
    assert_eq!(result["valid"], false);
}

#[test]
fn stdio_mcp_server_handshake() {
    let mut child = Command::new(binary())
        .args(["mcp", "serve"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to spawn mcp server");

    {
        let mut stdin = child.stdin.take().unwrap();
        writeln!(
            stdin,
            "{}",
            json!({
                "jsonrpc": "2.0", "id": 1, "method": "initialize",
                "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test"}}
            })
        )
        .unwrap();
        writeln!(
            stdin,
            "{}",
            json!({"jsonrpc": "2.0", "method": "notifications/initialized"})
        )
        .unwrap();
        writeln!(
            stdin,
            "{}",
            json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"})
        )
        .unwrap();
        stdin.flush().unwrap();
    }

    let stdout = child.stdout.take().unwrap();
    let mut lines = BufReader::new(stdout).lines();
    let init: Value = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
    assert_eq!(init["id"], 1);
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18");

    let tools: Value = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
    assert_eq!(tools["id"], 2);
    assert_eq!(tools["result"]["tools"].as_array().unwrap().len(), 25);

    let status = child.wait().unwrap();
    assert!(status.success());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn services_list_uses_the_configured_base_url() {
    let mock = common::start().await;
    let base_url = mock.base_url().to_string();
    let output = tokio::task::spawn_blocking(move || {
        Command::new(binary())
            .args(["services", "list"])
            .env("PROXY_CHEAP_BASE_URL", base_url)
            .output()
            .unwrap()
    })
    .await
    .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let services = stdout_json(&output);
    assert_eq!(services.as_array().unwrap().len(), 2);
    assert_eq!(services[0]["id"], "static-datacenter-ipv4");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn table_output_renders_columns() {
    let mock = common::start().await;
    let base_url = mock.base_url().to_string();
    let output = tokio::task::spawn_blocking(move || {
        Command::new(binary())
            .args(["services", "list", "--table"])
            .env("PROXY_CHEAP_BASE_URL", base_url)
            .output()
            .unwrap()
    })
    .await
    .unwrap();
    assert!(output.status.success());
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("ID"));
    assert!(text.contains("static-datacenter-ipv4"));
    assert!(text.contains("basic, premium"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn proxies_list_sends_credentials_and_renders_a_table() {
    let mock = common::start().await;
    let base_url = mock.base_url().to_string();
    let output = tokio::task::spawn_blocking(move || {
        Command::new(binary())
            .args(["proxies", "list", "--table"])
            .env("PROXY_CHEAP_BASE_URL", base_url)
            .env("PROXY_CHEAP_API_KEY", "key")
            .env("PROXY_CHEAP_API_SECRET", "secret")
            .output()
            .unwrap()
    })
    .await
    .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("1410379"));
    assert!(text.contains("203.0.113.10"));
    assert_eq!(mock.last().header("x-api-key").as_deref(), Some("key"));
}

#[test]
fn login_reports_an_existing_session_without_opening_a_browser() {
    let directory = tempfile::tempdir().unwrap();
    let config_dir = directory.path().join("proxy-cheap");
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(
        config_dir.join("config.toml"),
        "session_token = \"opaque-token\"\n",
    )
    .unwrap();

    let output = Command::new(binary())
        .arg("login")
        .env("XDG_CONFIG_HOME", directory.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let result = stdout_json(&output);
    assert_eq!(result["alreadyLoggedIn"], true);
    assert_eq!(result["authMethod"], "session-token");
}

#[test]
fn logout_removes_the_session_but_keeps_api_keys_and_can_delete_browser_data() {
    let directory = tempfile::tempdir().unwrap();
    let config_dir = directory.path().join("proxy-cheap");
    fs::create_dir_all(&config_dir).unwrap();
    fs::write(
        config_dir.join("config.toml"),
        "api_key = \"key\"\napi_secret = \"secret\"\nsession_token = \"opaque-token\"\nsession_token_expires_at = 1893456000\n",
    )
    .unwrap();
    let profile = config_dir.join("browser-profile");
    fs::create_dir_all(&profile).unwrap();

    let output = Command::new(binary())
        .arg("logout")
        .env("XDG_CONFIG_HOME", directory.path())
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(stdout_json(&output)["removedSessionToken"], true);
    let text = fs::read_to_string(config_dir.join("config.toml")).unwrap();
    assert!(!text.contains("session_token"));
    assert!(text.contains("api_key = \"key\""));
    assert!(text.contains("api_secret = \"secret\""));
    assert!(
        profile.exists(),
        "logout without --browser-data must keep the profile"
    );

    let output = Command::new(binary())
        .args(["logout", "--browser-data"])
        .env("XDG_CONFIG_HOME", directory.path())
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(stdout_json(&output)["removedBrowserProfile"], true);
    assert!(!profile.exists());
}

#[test]
fn logout_without_a_config_file_is_a_no_op() {
    let directory = tempfile::tempdir().unwrap();
    let output = Command::new(binary())
        .arg("logout")
        .env("XDG_CONFIG_HOME", directory.path())
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(stdout_json(&output)["removedSessionToken"], false);
    assert!(!directory.path().join("proxy-cheap/config.toml").exists());
}
