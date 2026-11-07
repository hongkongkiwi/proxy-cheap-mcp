//! CLI, MCP server, and agent skill for the Proxy-Cheap API.

pub mod api;
pub mod cli;
pub mod config;
pub mod error;
pub mod mcp;
pub mod skill;
pub mod webhook;

pub use api::ProxyCheapClient;
pub use config::ClientConfig;
pub use error::{Error, Result};
