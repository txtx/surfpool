mod calculator;

#[derive(PartialEq, Clone, Debug, Default)]
pub struct McpOptions {
}

pub async fn run_server(opts: &McpOptions) -> Result<(), String> {
    let transport = (stdin(), stdout());
    let server = calculator::Calculator::new();
    let service = server.serve(transport).await.map_err(|e| e.to_string())?;
    let _ = service.waiting().await;
    Ok(())
}
