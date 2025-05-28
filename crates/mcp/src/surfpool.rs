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
    #[tool(description = "Spin up a new surfnet")]
    fn start_surfnet(&self) -> Json<bool> {
        // Starting a new surfnet runloop
        let mut cmd = Command::new("surfpool");
        cmd.arg("start");
        cmd.stdin(Stdio::null());
        match cmd.spawn() {
            Ok(_child) => Json(true),
            Err(e) => {
                eprintln!("Failed to execute surfnet start: {}", e);
                Json(false)
            }
        }
    }
}

#[tool(tool_box)]
impl ServerHandler for Surfpool {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some("surfpool mcp server".into()),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}
