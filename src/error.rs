//! Error types shared by the CLI, API client, MCP server, and webhook receiver.

use serde_json::Value;
use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error(
        "missing API credentials: run `proxy-cheap login` for an interactive browser login, \
         set PROXY_CHEAP_API_KEY and PROXY_CHEAP_API_SECRET, or set \
         PROXY_CHEAP_SESSION_TOKEN"
    )]
    MissingCredentials,

    #[error("{0}")]
    Config(String),

    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),

    #[error("API error ({status}): {message}")]
    Api {
        status: u16,
        message: String,
        body: Option<Value>,
    },

    #[error("webhook signature verification failed")]
    SignatureMismatch,

    #[error("{0}")]
    InvalidInput(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Json(#[from] serde_json::Error),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl Error {
    /// Build an API error from a response status and raw body.
    pub fn from_response(status: u16, body: &[u8]) -> Self {
        let parsed = serde_json::from_slice::<Value>(body).ok();
        if (300..400).contains(&status) {
            return Self::Api {
                status,
                message: "authentication failed: the API redirected to its login page; check the API key and secret".to_string(),
                body: parsed,
            };
        }
        if let Some(value) = parsed {
            return Self::from_json(status, value);
        }
        let message = if body.is_empty() {
            format!("empty response (HTTP {status})")
        } else {
            let text = String::from_utf8_lossy(body);
            let trimmed = text.trim();
            if trimmed.starts_with('<') {
                "the API returned an HTML page instead of JSON; this usually means the credentials were rejected".to_string()
            } else {
                trimmed.chars().take(500).collect()
            }
        };
        Self::Api {
            status,
            message,
            body: None,
        }
    }

    /// Build an API error from an already-parsed JSON body.
    pub fn from_json(status: u16, value: Value) -> Self {
        Self::Api {
            status,
            message: describe_body(&value),
            body: Some(value),
        }
    }

    /// Process exit code, stable for scripting.
    pub fn exit_code(&self) -> u8 {
        match self {
            Self::InvalidInput(_) => 2,
            Self::MissingCredentials | Self::Config(_) => 3,
            Self::Api { status, .. }
                if *status == 401 || *status == 403 || (300..400).contains(status) =>
            {
                3
            }
            Self::Api { .. } => 4,
            Self::Network(_) => 5,
            Self::SignatureMismatch => 6,
            Self::Io(_) | Self::Json(_) | Self::Other(_) => 1,
        }
    }

    /// The structured API body, when the server returned JSON.
    pub fn api_body(&self) -> Option<&Value> {
        match self {
            Self::Api { body, .. } => body.as_ref(),
            _ => None,
        }
    }
}

fn describe_body(value: &Value) -> String {
    if let Some(message) = value.get("message").and_then(Value::as_str) {
        return message.to_string();
    }
    if let Some(errors) = value.get("errors") {
        if let Some(description) = describe_field_errors(errors) {
            return description;
        }
    }
    if let Some(detail) = value.get("detail").and_then(Value::as_str) {
        let title = value
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("request failed");
        return format!("{title}: {detail}");
    }
    if let Some(violations) = value.get("violations").and_then(Value::as_array) {
        let parts: Vec<String> = violations
            .iter()
            .map(|violation| {
                let field = violation
                    .get("propertyPath")
                    .and_then(Value::as_str)
                    .unwrap_or("request");
                let title = violation
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or("invalid");
                format!("{field}: {title}")
            })
            .collect();
        if !parts.is_empty() {
            return parts.join("; ");
        }
    }
    let compact = serde_json::to_string(value).unwrap_or_default();
    compact.chars().take(500).collect()
}

fn describe_field_errors(errors: &Value) -> Option<String> {
    let map = errors.as_object()?;
    let mut parts = Vec::new();
    for (field, entries) in map {
        let Some(entries) = entries.as_array() else {
            continue;
        };
        for entry in entries {
            let code = entry
                .get("code")
                .and_then(Value::as_str)
                .unwrap_or("INVALID");
            let label = if field.is_empty() {
                "request"
            } else {
                field.as_str()
            };
            match entry.get("parameters").and_then(Value::as_object) {
                Some(params) if !params.is_empty() => {
                    let params: Vec<String> = params
                        .iter()
                        .map(|(key, value)| {
                            let value = match value {
                                Value::String(text) => text.clone(),
                                other => other.to_string(),
                            };
                            format!("{key}={value}")
                        })
                        .collect();
                    parts.push(format!("{label}: {code} ({})", params.join(", ")));
                }
                _ => parts.push(format!("{label}: {code}")),
            }
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("; "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describes_v2_validation_errors() {
        let body = br#"{"errors":{"region":[{"code":"NO_SUCH_REGION","parameters":{"value":"California"}}]}}"#;
        let error = Error::from_response(422, body);
        assert_eq!(
            error.to_string(),
            "API error (422): region: NO_SUCH_REGION (value=California)"
        );
    }

    #[test]
    fn describes_plain_message() {
        let error = Error::from_response(401, br#"{"message":"INVALID_CREDENTIALS"}"#);
        assert_eq!(error.to_string(), "API error (401): INVALID_CREDENTIALS");
    }

    #[test]
    fn explains_login_redirects() {
        let error = Error::from_response(302, b"<html>login</html>");
        assert_eq!(error.exit_code(), 3);
        assert!(error.to_string().contains("redirected"));
    }
}
