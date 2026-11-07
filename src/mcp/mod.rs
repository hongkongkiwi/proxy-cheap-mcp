//! Model Context Protocol server for the Proxy-Cheap API.

pub mod http;
pub mod protocol;
pub mod stdio;
pub mod tools;

pub use protocol::McpServer;
