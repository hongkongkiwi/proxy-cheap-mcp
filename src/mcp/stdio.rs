//! MCP server over stdio (one JSON-RPC message per line).

use std::sync::Arc;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};

use crate::error::{Error, Result};
use crate::mcp::protocol::{McpServer, parse_error};

pub async fn serve(server: Arc<McpServer>) -> Result<()> {
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut stdout = BufWriter::new(tokio::io::stdout());

    while let Some(line) = lines
        .next_line()
        .await
        .map_err(|error| Error::Other(anyhow::anyhow!("could not read stdin: {error}")))?
    {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let message: Value = match serde_json::from_str(line) {
            Ok(message) => message,
            Err(error) => {
                let response = parse_error(error.to_string());
                stdout.write_all(response.to_string().as_bytes()).await?;
                stdout.write_all(b"\n").await?;
                stdout.flush().await?;
                continue;
            }
        };
        if let Some(response) = server.handle(message).await {
            let text = serde_json::to_string(&response)?;
            stdout.write_all(text.as_bytes()).await?;
            stdout.write_all(b"\n").await?;
            stdout.flush().await?;
        }
    }
    Ok(())
}
