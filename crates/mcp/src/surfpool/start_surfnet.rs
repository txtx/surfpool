use serde::Serialize;
use surfpool_core::{start_local_surfnet, surfnet::svm::SurfnetSvm};
use surfpool_types::{SimnetEvent, SurfpoolConfig};

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
    let (surfnet_svm, simnet_events_rx, geyser_events_rx) = SurfnetSvm::new();

    let (simnet_commands_tx, simnet_commands_rx) = crossbeam_channel::unbounded();
    let (subgraph_commands_tx, _subgraph_commands_rx) = crossbeam_channel::unbounded();

    let simnet_events_tx = surfnet_svm.simnet_events_tx.clone();

    let config = SurfpoolConfig::default();

    let handle = hiro_system_kit::thread_named("surfnet").spawn(move || {
        let future = start_local_surfnet(
            surfnet_svm,
            config,
            subgraph_commands_tx,
            simnet_commands_tx,
            simnet_commands_rx,
            geyser_events_rx,
        );

        if let Err(e) = hiro_system_kit::nestable_block_on(future) {
            let _ = simnet_events_tx.send(SimnetEvent::error(format!("Surfnet error: {}", e)));
            std::process::exit(1);
        }
        Ok::<(), String>(())
    });

    match handle {
        Ok(_) => loop {
            match simnet_events_rx.recv() {
                Ok(SimnetEvent::Aborted(error)) => {
                    return StartSurfnetResponse::error(error);
                }
                Ok(SimnetEvent::Connected(_)) | Ok(SimnetEvent::Ready) => {
                    return StartSurfnetResponse::success(
                        "Surfnet started successfully at http://127.0.0.1:8899".to_string(),
                    );
                }
                Ok(SimnetEvent::ErrorLog(_, error)) => {
                    return StartSurfnetResponse::error(error);
                }
                _other => continue,
            }
        },
        Err(e) => StartSurfnetResponse::error(format!("Failed to spawn surfnet thread: {}", e)),
    }
}
