use std::{sync::mpsc::{channel, Receiver}, thread::sleep, time::Duration};

use crate::{scaffold::detect_program_frameworks, tui};

use super::{Context, StartSimnet};
use surfpool_core::{simnet::SimnetEvent, start_simnet};

pub async fn handle_start_simnet_command(cmd: &StartSimnet, ctx: &Context) -> Result<(), String> {
    let (simnet_events_tx, simnet_events_rx) = channel();
    let ctx_cloned = ctx.clone();
    // Start backend - background task
    let handle = hiro_system_kit::thread_named("simnet")
        .spawn(move || {
            let future = start_simnet(simnet_events_tx);
            if let Err(e) = hiro_system_kit::nestable_block_on(future) {
                error!(ctx_cloned.expect_logger(), "{e}");
                std::thread::sleep(std::time::Duration::from_millis(500));
                std::process::exit(1);
            }
            Ok(())
        })
        .map_err(|e| format!("{}", e))?;


    sleep(Duration::from_secs(10));
    match detect_program_frameworks(&cmd.manifest_path).await {
        Err(e) => error!(ctx.expect_logger(), "{}", e),
        Ok(Some(framework)) => {
            info!(
                ctx.expect_logger(),
                "Loading contracts from {:?}", framework
            )
        }
        _ => {}
    };

    // Start frontend - kept on main thread
    if cmd.no_tui {
        log_events(simnet_events_rx, cmd.debug, ctx);
    } else {
        tui::simnet::start_app(simnet_events_rx, cmd.debug).map_err(|e| format!("{}", e))?;
    }
    handle.join().map_err(|_e| format!("unable to terminate"))?
}

fn log_events(simnet_events_rx: Receiver<SimnetEvent>, include_debug_logs: bool, ctx: &Context) {
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
        }
    }
}
