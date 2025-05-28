use std::process::{Command, Stdio};

use serde::Serialize;

#[derive(Serialize)]
pub struct StartSurfnetResponse {
    success: Option<String>,
    error: Option<String>,
}

impl StartSurfnetResponse {
    pub fn success(message: String) -> Self {
        Self {
            success: Some(message),
            error: None,
        }
    }

    pub fn error(message: String) -> Self {
        Self {
            success: None,
            error: Some(message),
        }
    }
}

pub fn run() -> StartSurfnetResponse {
    // Prepare to start the "surfpool start" command as a subprocess
    let mut cmd = Command::new("surfpool");
    cmd.arg("start");
    // Detach the child process from stdin
    cmd.stdin(Stdio::null());
    // Attempt to spawn the process
    match cmd.spawn() {
        Ok(_child) => StartSurfnetResponse::success(format!("http://localhost:8899")),
        Err(e) => StartSurfnetResponse::error(format!("Failed to execute surfnet start: {}", e)),
    }
}
