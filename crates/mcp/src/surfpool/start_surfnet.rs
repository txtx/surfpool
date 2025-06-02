use serde::Serialize;
use surfpool_core::{start_local_surfnet, surfnet::svm::SurfnetSvm};
use surfpool_types::{SimnetConfig, SimnetEvent, SurfpoolConfig};

#[derive(Serialize)]
pub struct StartSurfnetResponse {
    pub success: Option<StartSurfnetSuccess>,
    pub error: Option<String>,
}

#[derive(Serialize)]
pub struct StartSurfnetSuccess {
    pub surfnet_url: String,
    pub surfnet_id: u16,
}


impl StartSurfnetResponse {
    pub fn success(data: StartSurfnetSuccess) -> Self {
        Self {
            success: Some(data),
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

pub fn run(surfnet_id: u16) -> StartSurfnetResponse {
    let (surfnet_svm, simnet_events_rx, geyser_events_rx) = SurfnetSvm::new();

    let (simnet_commands_tx, simnet_commands_rx) = crossbeam_channel::unbounded();
    let (subgraph_commands_tx, _subgraph_commands_rx) = crossbeam_channel::unbounded();

    let simnet_events_tx = surfnet_svm.simnet_events_tx.clone();

    let mut config = SurfpoolConfig::default();
    let rpc_port = 8899 + 10000 * surfnet_id;
    let ws_port = 8890 + 10000 * surfnet_id;
    config.rpc.bind_port = rpc_port;
    config.rpc.ws_port = ws_port;

    let mut simnet_config = SimnetConfig::default();
    simnet_config.expiry = Some(15 * 60 * 1000);

    config.simnets = vec![simnet_config];

    let rpc_config = config.rpc.clone();

    let handle = hiro_system_kit::thread_named("surfnet").spawn(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let future = start_local_surfnet(
                surfnet_svm,
                config,
                subgraph_commands_tx,
                simnet_commands_tx,
                simnet_commands_rx,
                geyser_events_rx,
            );
            hiro_system_kit::nestable_block_on(future)
        }));

        match result {
            Ok(Ok(_)) => {
                
            }
            Ok(Err(e)) => {
                let _ = simnet_events_tx.send(SimnetEvent::error(format!("Surfnet operational error: {}", e)));
            }
            Err(panic_payload) => {
                let panic_msg = match panic_payload.downcast_ref::<&'static str>() {
                    Some(s) => *s,
                    None => match panic_payload.downcast_ref::<String>() {
                        Some(s) => s.as_str(),
                        None => "Surfnet thread panicked with an unknown payload",
                    },
                };
                let _ = simnet_events_tx.send(SimnetEvent::error(format!("Surfnet thread panic: {}", panic_msg)));
            }
        }
        Ok::<(), String>(())
    });

    let res = match handle {
        Ok(_) => loop {
            match simnet_events_rx.recv_timeout(std::time::Duration::from_secs(25)) {
                Ok(received_event) => match received_event {
                    SimnetEvent::Aborted(error) => {
                        return StartSurfnetResponse::error(error);
                    }
                    SimnetEvent::Ready => {
                        break StartSurfnetResponse::success(StartSurfnetSuccess {
                            surfnet_url: format!("http://{}", rpc_config.get_socket_address()),
                            surfnet_id,
                        });
                    }
                    SimnetEvent::ErrorLog(_, error) => {
                        return StartSurfnetResponse::error(error);
                    }
                    _other_simnet_event => continue,
                },
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                    return StartSurfnetResponse::error(
                        "Surfnet initialization timed out waiting for an event.".to_string(),
                    );
                }
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                    return StartSurfnetResponse::error(
                        "Surfnet channel disconnected while waiting for event.".to_string(),
                    );
                }
            }
        },
        Err(e) => StartSurfnetResponse::error(format!("Failed to spawn surfnet thread: {}", e)),
    };

    let handle = hiro_system_kit::thread_named("surfnet-termination-handler").spawn(move || {
        loop {
            match simnet_events_rx.recv() {
                Ok(received_event) => match received_event {
                    SimnetEvent::Shutdown => {
                        // Send event to MCP saying that the surfnet has been terminated
                        break;
                    }
                    _ => {}
                },
                Err(_) => {}
            }
        }
    });

    res
}
