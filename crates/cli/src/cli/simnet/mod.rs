use crate::{
    runbook::execute_runbook,
    scaffold::{detect_program_frameworks, scaffold_runbooks_layout},
    tui,
};

use super::{Context, StartSimnet};
use dialoguer::{console::Style, theme::ColorfulTheme, MultiSelect};
use surfpool_core::{
    simnet::SimnetEvent,
    start_simnet,
    types::{RpcConfig, RunloopTriggerMode, SimnetConfig, SurfpoolConfig},
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
            runloop_trigger_mode: RunloopTriggerMode::Clock,
        },
    };
    let remote_rpc_url = config.rpc.remote_rpc_url.clone();
    let local_rpc_url = config.rpc.get_socket_address();

    // Start backend - background task
    let (simnet_commands_tx, simnet_commands_rx) = crossbeam::channel::unbounded();
    let (simnet_events_tx, simnet_events_rx) = crossbeam::channel::unbounded();
    let ctx_cloned = ctx.clone();
    let _handle = hiro_system_kit::thread_named("simnet")
        .spawn(move || {
            let future = start_simnet(&config, simnet_events_tx, simnet_commands_rx);
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
        let theme = ColorfulTheme {
            values_style: Style::new().green(),
            hint_style: Style::new().cyan(),
            ..ColorfulTheme::default()
        };

        let selection = MultiSelect::with_theme(&theme)
            .with_prompt("Programs to deploy:")
            .items(&programs)
            .interact()
            .unwrap();

        let selected_programs = selection
            .iter()
            .map(|i| programs[*i].clone())
            .collect::<Vec<_>>();

        scaffold_runbooks_layout(selected_programs, &cmd.manifest_path)?;

        let (progress_tx, progress_rx) = crossbeam::channel::unbounded();
        let manifest_location = FileLocation::from_path_string(&cmd.manifest_path)?;
        execute_runbook("deployment", progress_tx, &manifest_location).await?;
        Some(progress_rx)
    } else {
        None
    };

    // Start frontend - kept on main thread
    if cmd.no_tui {
        log_events(simnet_events_rx, cmd.debug, deploy_progress_rx, ctx)?;
    } else {
        tui::simnet::start_app(
            simnet_events_rx,
            simnet_commands_tx,
            cmd.debug,
            deploy_progress_rx,
            &remote_rpc_url,
            &local_rpc_url,
        )
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
