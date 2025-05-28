use std::process::{Command, Stdio};

use rmcp::{
    handler::server::wrapper::Json,
    model::{ServerCapabilities, ServerInfo},
    tool, ServerHandler,
};

#[derive(Debug, Clone, Default)]
pub struct Surfpool;

#[tool(tool_box)]
impl Surfpool {
    /// Spin up a new surfnet process.
    /// This method is exposed as a tool and can be invoked remotely.
    #[tool(description = "Spin up a new surfnet")]
    fn start_surfnet(&self) -> Json<bool> {
        // Prepare to start the "surfpool start" command as a subprocess
        let mut cmd = Command::new("surfpool");
        cmd.arg("start");
        // Detach the child process from stdin
        cmd.stdin(Stdio::null());
        // Attempt to spawn the process
        match cmd.spawn() {
            Ok(_child) => Json(true), // Successfully started the process
            Err(e) => {
                // Print error to stderr if process fails to start
                eprintln!("Failed to execute surfnet start: {}", e);
                Json(false)
            }
        }
    }
}

#[tool(tool_box)]
impl ServerHandler for Surfpool {
    /// Return information about the server, including its capabilities and description.
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(
                "Surfpool MCP server, your personal surfnet manager to start surfing on Solana!"
                    .into(),
            ),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}
