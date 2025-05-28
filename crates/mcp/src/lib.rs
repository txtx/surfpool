use calculator::Calculator;
use rmcp::{ServiceExt, transport::stdio};

mod calculator;

#[derive(PartialEq, Clone, Debug, Default)]
pub struct McpOptions {}

pub async fn run_server(_opts: &McpOptions) -> Result<(), String> { 
    let service = Calculator::default().serve(stdio()).await.inspect_err(|e| {
        tracing::error!("serving error: {:?}", e);
    }).unwrap();
    service.waiting().await.unwrap();

    Ok(())
}
