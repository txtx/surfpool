use std::process::{Command, Stdio};

use serde::Serialize;
use surfpool_core::start_local_surfnet;

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

    let _handle = hiro_system_kit::thread_named("surfnet")
        .spawn(move || {
            let future = start_local_surfnet(
                surfnet_svm,
                config_copy,
                subgraph_commands_tx,
                simnet_commands_tx_copy,
                simnet_commands_rx,
                geyser_events_rx,
            );
            if let Err(e) = hiro_system_kit::nestable_block_on(future) {
                // Error handling
                std::process::exit(1);
            }
            Ok::<(), String>(())
        })
        .map_err(|e| StartSurfnetResponse::error(format!("Failed to execute surfnet start: {}", e)))?;

    StartSurfnetResponse::success(format!("http://127.0.0.1:8899"))
}
