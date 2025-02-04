use crate::{runbook::execute_runbook, scaffold::detect_program_frameworks, tui};

use super::{Context, StartSimnet};
use surfpool_core::{
    simnet::SimnetEvent,
    start_simnet,
    types::{RpcConfig, SimnetConfig, SurfpoolConfig},
};
use txtx_core::kit::{channel::Receiver, helpers::fs::FileLocation, types::frontend::BlockEvent};

pub async fn handle_start_simnet_command(cmd: &StartSimnet, ctx: &Context) -> Result<(), String> {
    let config = SurfpoolConfig {
        rpc: RpcConfig {
            remote_rpc_url: cmd.rpc_url.clone(),
            bind_port: cmd.network_binding_port,
            bind_address: cmd.network_binding_ip_address.clone(),
        },
        simnet: SimnetConfig {
            remote_rpc_url: cmd.rpc_url.clone(),
            slot_time: cmd.slot_time,
        },
    };

    let (simnet_events_tx, simnet_events_rx) = crossbeam::channel::unbounded();
    let ctx_cloned = ctx.clone();
    // Start backend - background task
    let _handle = hiro_system_kit::thread_named("simnet")
        .spawn(move || {
            let future = start_simnet(&config, simnet_events_tx);
            if let Err(e) = hiro_system_kit::nestable_block_on(future) {
                error!(ctx_cloned.expect_logger(), "{e}");
                std::thread::sleep(std::time::Duration::from_millis(500));
                std::process::exit(1);
            }
            Ok::<(), String>(())
        })
        .map_err(|e| format!("{}", e))?;

    loop {
        match simnet_events_rx.recv() {
            Ok(SimnetEvent::Aborted(error)) => return Err(error),
            Ok(SimnetEvent::Shutdown) => return Ok(()),
            Ok(SimnetEvent::Ready) => break,
            _other => continue,
        }
    }

    // Initialize if required

    // Propose deployments (use --no-deploy)

    let deployment = match detect_program_frameworks(&cmd.manifest_path).await {
        Err(e) => {
            error!(ctx.expect_logger(), "{}", e);
            None
        }
        Ok(deployment) => deployment,
    };

    let deploy_progress_rx = if let Some((framework, programs)) = deployment {
        let (progress_tx, progress_rx) = crossbeam::channel::unbounded();
        let manifest_location = FileLocation::from_path_string(&cmd.manifest_path)?;
        execute_runbook("v1", progress_tx, &manifest_location).await?;
        Some(progress_rx)
    } else {
        None
    };

    // Start frontend - kept on main thread
    if cmd.no_tui {
        log_events(simnet_events_rx, cmd.debug, deploy_progress_rx, ctx)?;
    } else {
        tui::simnet::start_app(simnet_events_rx, cmd.debug, deploy_progress_rx)
            .map_err(|e| format!("{}", e))?;
    }
    Ok(())
}

fn log_events(
    simnet_events_rx: Receiver<SimnetEvent>,
    include_debug_logs: bool,
    deploy_progress_rx: Option<Receiver<BlockEvent>>,
    ctx: &Context,
) -> Result<(), String> {
    info!(
        ctx.expect_logger(),
        "Surfpool: The best place to train before surfing Solana"
    );
    while let Ok(event) = simnet_events_rx.recv() {
        match event {
            SimnetEvent::AccountUpdate(_, account) => {
                info!(
                    ctx.expect_logger(),
                    "Account retrieved from Mainnet {}", account
                );
            }
            SimnetEvent::EpochInfoUpdate(epoch_info) => {
                info!(
                    ctx.expect_logger(),
                    "Connection established. Epoch {}, Slot {}.",
                    epoch_info.epoch,
                    epoch_info.slot_index
                );
            }
            SimnetEvent::ClockUpdate(clock) => {
                if include_debug_logs {
                    info!(ctx.expect_logger(), "Slot #{} ", clock.slot);
                }
            }
            SimnetEvent::ErrorLog(_, log) => {
                error!(ctx.expect_logger(), "{} ", log);
            }
            SimnetEvent::InfoLog(_, log) => {
                info!(ctx.expect_logger(), "{} ", log);
            }
            SimnetEvent::WarnLog(_, log) => {
                warn!(ctx.expect_logger(), "{} ", log);
            }
            SimnetEvent::DebugLog(_, log) => {
                if include_debug_logs {
                    debug!(ctx.expect_logger(), "{} ", log);
                }
            }
            SimnetEvent::TransactionReceived(_, _transaction) => {
                info!(ctx.expect_logger(), "Transaction received");
            }
            SimnetEvent::BlockHashExpired => {}
            SimnetEvent::Aborted(error) => {
                return Err(error);
            }
            SimnetEvent::Shutdown => break,
            SimnetEvent::Ready => {}
        }
    }
    Ok(())
}
