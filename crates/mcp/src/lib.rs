use rmcp::{transport::stdio, ServiceExt};
use surfpool::Surfpool;

mod surfpool;

#[derive(PartialEq, Clone, Debug, Default)]
pub struct McpOptions {}

/// Asynchronously runs the MCP server using the provided options.
///
/// # Arguments
///
/// * `_opts` - Reference to `McpOptions`
///
/// # Returns
///
/// * `Result<(), String>` - Returns `Ok(())` if the server runs successfully, or an error string otherwise.
pub async fn run_server(_opts: &McpOptions) -> Result<(), String> {
    let service = Surfpool
        .serve(stdio())
        .await
        .inspect_err(|e| {
            tracing::error!("serving error: {:?}", e);
        })
        .map_err(|e| e.to_string())?;

    service.waiting().await.map_err(|e| e.to_string())?;

    Ok(())
}
