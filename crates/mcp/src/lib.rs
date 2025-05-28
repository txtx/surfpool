use calculator::Calculator;
use rmcp::{ServiceExt, transport::stdio};

mod calculator;

#[derive(PartialEq, Clone, Debug, Default)]
pub struct McpOptions {}

pub async fn run_server(opts: &McpOptions) -> Result<(), String> {

    // Create an instance of our counter router
    let service = Calculator::default().serve(stdio()).await.inspect_err(|e| {
        tracing::error!("serving error: {:?}", e);
    }).unwrap();
    println!("MCP server started");

    service.waiting().await.unwrap();

    println!("MCP server stopped");

    Ok(())
}
