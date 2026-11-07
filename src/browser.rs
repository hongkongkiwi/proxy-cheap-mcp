//! Interactive browser login: launch a Chromium browser, wait for the
//! Proxy-Cheap dashboard session cookie, and return the JWT.
//!
//! Proxy-Cheap does not expose an OAuth or device-login flow. Its own browser
//! extension works by reading a `JWTproduction` cookie after the user logs in
//! at `app.proxy-cheap.com` and sending it as `Authorization: Bearer ...`.
//! This module reproduces that flow with a dedicated browser profile.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Duration;

use chromiumoxide::browser::{Browser, BrowserConfig};
use tokio_stream::StreamExt;

use crate::config;
use crate::error::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Environment {
    Production,
    Staging,
    Local,
}

impl Environment {
    pub fn label(self) -> &'static str {
        match self {
            Self::Production => "production",
            Self::Staging => "staging",
            Self::Local => "local",
        }
    }

    pub fn api_base(self) -> &'static str {
        match self {
            Self::Production => "https://api.proxy-cheap.com",
            Self::Staging => "https://api.staging.proxy-cheap.com",
            Self::Local => "https://api.proxycheap.land",
        }
    }

    pub fn app_base(self) -> &'static str {
        match self {
            Self::Production => "https://app.proxy-cheap.com",
            Self::Staging => "https://app.staging.proxy-cheap.com",
            Self::Local => "https://app.proxycheap.land",
        }
    }

    /// Cookie name used by the Proxy-Cheap dashboard and browser extension.
    pub fn cookie_name(self) -> &'static str {
        match self {
            Self::Production => "JWTproduction",
            Self::Staging => "JWTstaging",
            Self::Local => "JWTlocal",
        }
    }

    pub fn from_base_url(url: &str) -> Self {
        let url = url.to_ascii_lowercase();
        if url.contains("staging") {
            Self::Staging
        } else if url.contains("proxycheap.land")
            || url.contains("localhost")
            || url.contains("127.0.0.1")
        {
            Self::Local
        } else {
            Self::Production
        }
    }

    pub fn from_name(name: &str) -> Result<Self> {
        match name.to_ascii_lowercase().as_str() {
            "production" | "prod" => Ok(Self::Production),
            "staging" | "stage" => Ok(Self::Staging),
            "local" | "development" | "dev" => Ok(Self::Local),
            other => Err(Error::InvalidInput(format!(
                "invalid environment {other:?}: expected production, staging, or local"
            ))),
        }
    }
}

impl fmt::Display for Environment {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.label())
    }
}

impl FromStr for Environment {
    type Err = Error;

    fn from_str(value: &str) -> Result<Self> {
        Self::from_name(value)
    }
}

#[derive(Debug, Clone)]
pub struct LoginOptions {
    pub environment: Environment,
    /// Override the dashboard URL opened in the browser.
    pub app_url: Option<String>,
    /// Override the cookie name to wait for.
    pub cookie_name: Option<String>,
    /// Explicit browser executable.
    pub browser: Option<PathBuf>,
    /// Dedicated Chromium profile directory.
    pub profile_dir: PathBuf,
    pub timeout: Duration,
}

impl LoginOptions {
    pub fn new(environment: Environment) -> Self {
        Self {
            environment,
            app_url: None,
            cookie_name: None,
            browser: None,
            profile_dir: config::browser_profile_dir(),
            timeout: Duration::from_secs(300),
        }
    }

    fn login_url(&self) -> String {
        let app_url = self
            .app_url
            .clone()
            .unwrap_or_else(|| self.environment.app_base().to_string());
        format!("{}/login", app_url.trim_end_matches('/'))
    }

    fn cookie_name(&self) -> String {
        self.cookie_name
            .clone()
            .unwrap_or_else(|| self.environment.cookie_name().to_string())
    }
}

/// Open a visible browser, let the user log in, and return the session JWT.
///
/// The browser uses a dedicated profile so a running personal Chrome or Edge
/// instance is not touched. The browser is closed before this function returns.
const INIT_SCRIPT: &str = r#"
Object.defineProperty(navigator, 'webdriver', { get: () => undefined });
"#;

pub async fn capture_session(options: &LoginOptions) -> Result<String> {
    let browser_path = resolve_browser(options.browser.as_deref())?;
    fs::create_dir_all(&options.profile_dir).map_err(|error| {
        Error::Config(format!(
            "could not create browser profile {}: {error}",
            options.profile_dir.display()
        ))
    })?;
    config::restrict_permissions(&options.profile_dir, 0o700)?;

    // chromiumoxide's default args include --enable-automation, which makes
    // Cloudflare Turnstile loop on the dashboard login. Launch a plain
    // headful browser instead and hide navigator.webdriver before navigation.
    // App mode is a compact window without tabs or an address bar.
    let login_url = options.login_url();
    let config = BrowserConfig::builder()
        .with_head()
        .disable_default_args()
        .arg(("app", login_url.as_str()))
        .arg("no-first-run")
        .arg("no-default-browser-check")
        .arg(("disable-blink-features", "AutomationControlled"))
        .arg(("lang", "en-US"))
        .arg(("window-position", "140,100"))
        .arg("use-mock-keychain")
        .arg(("password-store", "basic"))
        .chrome_executable(&browser_path)
        .user_data_dir(&options.profile_dir)
        .window_size(980, 760)
        .launch_timeout(Duration::from_secs(30))
        .request_timeout(Duration::from_secs(30))
        .build()
        .map_err(|error| Error::Config(format!("could not configure the browser: {error}")))?;

    tracing::info!(
        browser = %browser_path.display(),
        profile = %options.profile_dir.display(),
        "launching browser"
    );
    let (mut browser, mut handler) = Browser::launch(config).await.map_err(|error| {
        Error::Config(format!(
            "could not launch {}: {error}. Close any browser using the profile {} and try again.",
            browser_path.display(),
            options.profile_dir.display()
        ))
    })?;
    let mut handler_task = tokio::spawn(async move {
        while let Some(event) = handler.next().await {
            if event.is_err() {
                break;
            }
        }
    });

    let page = match browser.pages().await {
        Ok(mut pages) => pages.pop(),
        Err(_) => None,
    };
    let page = match page {
        Some(page) => page,
        None => browser
            .new_page("about:blank")
            .await
            .map_err(|error| Error::Config(format!("could not open a browser tab: {error}")))?,
    };
    page.add_init_script(INIT_SCRIPT)
        .await
        .map_err(|error| Error::Config(format!("could not prepare the browser page: {error}")))?;
    page.goto(login_url.clone()).await.map_err(|error| {
        Error::Config(format!(
            "could not open {login_url} in the browser: {error}"
        ))
    })?;
    let cookie_name = options.cookie_name();
    tracing::info!(
        "log in in the opened browser window; waiting up to {}s for the {cookie_name} cookie",
        options.timeout.as_secs()
    );

    let deadline = tokio::time::Instant::now() + options.timeout;
    let mut last_error: Option<String> = None;

    loop {
        match browser.get_cookies().await {
            Ok(cookies) => {
                if let Some(cookie) = cookies
                    .into_iter()
                    .find(|cookie| cookie.name == cookie_name && !cookie.value.trim().is_empty())
                {
                    let token = cookie.value;
                    close_browser(&mut browser, &mut handler_task).await;
                    return Ok(token);
                }
            }
            Err(error) => last_error = Some(error.to_string()),
        }

        if handler_task.is_finished() {
            let message = last_error.unwrap_or_else(|| "the browser was closed".to_string());
            handler_task.abort();
            return Err(Error::Config(format!(
                "the browser closed before login completed: {message}"
            )));
        }
        if tokio::time::Instant::now() >= deadline {
            close_browser(&mut browser, &mut handler_task).await;
            let detail = last_error.unwrap_or_else(|| "no cookie was set".to_string());
            return Err(Error::Config(format!(
                "timed out after {}s waiting for the {cookie_name} cookie ({detail})",
                options.timeout.as_secs()
            )));
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

async fn close_browser(browser: &mut Browser, handler_task: &mut tokio::task::JoinHandle<()>) {
    let _ = browser.close().await;
    let _ = tokio::time::timeout(Duration::from_secs(5), browser.wait()).await;
    handler_task.abort();
}

/// Resolve a Chromium-family executable from an explicit path, the
/// `PROXY_CHEAP_BROWSER` environment variable, or the usual install locations.
pub fn resolve_browser(explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        return validate_browser_path(path);
    }
    if let Some(path) = std::env::var_os("PROXY_CHEAP_BROWSER") {
        return validate_browser_path(Path::new(&path));
    }
    for candidate in candidate_paths() {
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err(Error::Config(
        "no Chromium browser found. Install Google Chrome, Microsoft Edge, Brave, or Chromium, \
         pass --browser /path/to/browser, or set PROXY_CHEAP_BROWSER."
            .to_string(),
    ))
}

fn validate_browser_path(path: &Path) -> Result<PathBuf> {
    if path.is_file() {
        Ok(path.to_path_buf())
    } else {
        Err(Error::Config(format!(
            "browser executable not found: {}",
            path.display()
        )))
    }
}

/// Candidate browser executables, in preference order.
pub fn candidate_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    #[cfg(target_os = "macos")]
    {
        let apps = [
            "Google Chrome",
            "Microsoft Edge",
            "Brave Browser",
            "Chromium",
        ];
        for app in apps {
            paths.push(PathBuf::from(format!(
                "/Applications/{app}.app/Contents/MacOS/{app}"
            )));
            if let Some(home) = dirs::home_dir() {
                paths.push(home.join(format!("Applications/{app}.app/Contents/MacOS/{app}")));
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        for binary in [
            "google-chrome",
            "microsoft-edge",
            "brave-browser",
            "chromium",
            "chromium-browser",
        ] {
            if let Some(path) = find_in_path(binary) {
                paths.push(path);
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        for variable in ["PROGRAMFILES", "PROGRAMFILES(X86)", "LOCALAPPDATA"] {
            let Some(base) = std::env::var_os(variable) else {
                continue;
            };
            let base = PathBuf::from(base);
            for relative in [
                "Google/Chrome/Application/chrome.exe",
                "Microsoft/Edge/Application/msedge.exe",
                "BraveSoftware/Brave-Browser/Application/brave.exe",
            ] {
                paths.push(base.join(relative));
            }
        }
    }

    paths
}

#[cfg(target_os = "linux")]
fn find_in_path(binary: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|directory| directory.join(binary))
        .find(|candidate| candidate.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_environments() {
        assert_eq!(Environment::Production.cookie_name(), "JWTproduction");
        assert_eq!(Environment::Staging.cookie_name(), "JWTstaging");
        assert_eq!(Environment::Local.cookie_name(), "JWTlocal");
        assert_eq!(
            Environment::from_base_url("https://api.staging.proxy-cheap.com"),
            Environment::Staging
        );
        assert_eq!(
            Environment::from_base_url("https://api.proxycheap.land"),
            Environment::Local
        );
        assert_eq!(
            Environment::from_base_url("https://api.proxy-cheap.com"),
            Environment::Production
        );
        assert_eq!(
            "staging".parse::<Environment>().unwrap(),
            Environment::Staging
        );
        assert!("nope".parse::<Environment>().is_err());
    }

    #[test]
    fn builds_login_urls() {
        let mut options = LoginOptions::new(Environment::Production);
        assert_eq!(options.login_url(), "https://app.proxy-cheap.com/login");
        options.app_url = Some("http://127.0.0.1:9000/".to_string());
        assert_eq!(options.login_url(), "http://127.0.0.1:9000/login");
    }

    #[test]
    fn rejects_missing_explicit_browser() {
        let error = resolve_browser(Some(Path::new("/no/such/browser"))).unwrap_err();
        assert!(error.to_string().contains("not found"));
    }

    #[test]
    fn candidate_list_is_populated_for_known_platforms() {
        #[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows"))]
        assert!(!candidate_paths().is_empty());
    }
}
