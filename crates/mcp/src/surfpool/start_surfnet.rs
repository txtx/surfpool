use std::{process::Command, time::Duration};

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
    pub kind: StartSurfnetKind,
    pub surfnet_url: String,
    pub surfnet_id: u16,
}

#[derive(Serialize)]
pub enum StartSurfnetKind {
    Command(SerializeCommand),
    Headless,
}

#[derive(Serialize)]
pub struct SerializeCommand {
    pub program: String,
    pub args: Vec<String>,
}

impl From<Command> for SerializeCommand {
    fn from(command: Command) -> Self {
        Self {
            program: command.get_program().to_string_lossy().to_string(),
            args: command
                .get_args()
                .map(|arg| arg.to_string_lossy().to_string())
                .collect(),
        }
    }
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

pub fn generate_command(rpc_port: u16, ws_port: u16) -> Command {
    let mut cmd = Command::new("surfpool");
    cmd.arg("start");
    cmd.arg("--port").arg(format!("{}", rpc_port));
    cmd.arg("--ws-port").arg(format!("{}", ws_port));
    cmd.arg("--no-deploy");
    cmd
}

pub fn run_command(surfnet_id: u16, rpc_port: u16, ws_port: u16) -> StartSurfnetResponse {
    let command = generate_command(rpc_port, ws_port);
    let surfnet_url = format!("http://127.0.0.1:{}", rpc_port);

    StartSurfnetResponse::success(StartSurfnetSuccess {
        kind: StartSurfnetKind::Command(command.into()),
        surfnet_url,
        surfnet_id,
    })
}

pub fn run_headless(surfnet_id: u16, rpc_port: u16, ws_port: u16) -> StartSurfnetResponse {
    let (surfnet_svm, simnet_events_rx, geyser_events_rx) = SurfnetSvm::new();

    let (simnet_commands_tx, simnet_commands_rx) = crossbeam_channel::unbounded();
    let (subgraph_commands_tx, _subgraph_commands_rx) = crossbeam_channel::unbounded();

    let simnet_events_tx = surfnet_svm.simnet_events_tx.clone();

    let mut config = SurfpoolConfig::default();
    config.rpc.bind_port = rpc_port;
    config.rpc.ws_port = ws_port;

    let simnet_config = SimnetConfig {
        expiry: Some(15 * 60 * 1000),
        ..Default::default()
    };

    config.simnets = vec![simnet_config];

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
            Ok(Ok(_)) => {}
            Ok(Err(e)) => {
                let _ = simnet_events_tx.send(SimnetEvent::error(format!(
                    "Surfnet operational error: {}",
                    e
                )));
            }
            Err(panic_payload) => {
                let panic_msg = match panic_payload.downcast_ref::<&'static str>() {
                    Some(s) => *s,
                    None => match panic_payload.downcast_ref::<String>() {
                        Some(s) => s.as_str(),
                        None => "Surfnet thread panicked with an unknown payload",
                    },
                };
                let _ = simnet_events_tx.send(SimnetEvent::error(format!(
                    "Surfnet thread panic: {}",
                    panic_msg
                )));
            }
        }
        Ok::<(), String>(())
    });

    let res = match handle {
        Ok(_) => loop {
            match simnet_events_rx.recv_timeout(Duration::from_secs(25)) {
                Ok(received_event) => match received_event {
                    SimnetEvent::Aborted(error) => {
                        return StartSurfnetResponse::error(error);
                    }
                    SimnetEvent::Ready => {
                        let surfnet_url = format!("http://127.0.0.1:{}", rpc_port);
                        break StartSurfnetResponse::success(StartSurfnetSuccess {
                            kind: StartSurfnetKind::Headless,
                            surfnet_url,
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

    let _handle = hiro_system_kit::thread_named("surfnet-termination-handler").spawn(move || {
        loop {
            match simnet_events_rx.recv() {
                Ok(received_event) => match received_event {
                    SimnetEvent::Aborted(reason) => {
                        eprintln!("Surfnet instance terminated: {}", reason);

                        break;
                    }
                    SimnetEvent::Shutdown => {
                        eprintln!("Surfnet instance has shut down.");
                        break;
                    }
                    _ => {}
                },
                Err(e) => {
                    eprintln!(
                        "Error receiving simnet event in termination handler: {:?}",
                        e
                    );
                    break;
                }
            }
        }
    });

    res
}
