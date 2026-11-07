//! Real-browser test for the cookie capture. Ignored by default because it
//! opens a visible browser and needs Chrome/Edge/Chromium installed.

use std::time::Duration;

use axum::Router;
use axum::http::header;
use axum::routing::get;
use proxy_cheap_mcp::browser::{Environment, LoginOptions, capture_session};

#[tokio::test]
#[ignore = "opens a visible browser window"]
async fn captures_the_session_cookie_from_a_local_page() {
    let app = Router::new().route(
        "/login",
        get(|| async {
            (
                [(
                    header::SET_COOKIE,
                    "JWTproduction=test-token; Path=/; SameSite=Lax",
                )],
                "logged in",
            )
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    let directory = tempfile::tempdir().unwrap();
    let mut options = LoginOptions::new(Environment::Local);
    options.app_url = Some(format!("http://{address}"));
    options.cookie_name = Some("JWTproduction".to_string());
    options.profile_dir = directory.path().join("profile");
    options.timeout = Duration::from_secs(30);

    let token = capture_session(&options).await.unwrap();
    assert_eq!(token, "test-token");
}
