use std::sync::mpsc::{channel, Receiver};

use crate::tui;

use super::{Context, StartSimnet};
use surfpool_core::{simnet::SimnetEvent, start_simnet};

pub fn handle_start_simnet_command(cmd: &StartSimnet, ctx: &Context) -> Result<(), String> {
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
    // Start frontend - kept on main thread
    if cmd.no_tui {
        log_events(simnet_events_rx, ctx);
    } else {
        tui::simnet::start_app(simnet_events_rx).map_err(|e| format!("{}", e))?;
    }
    handle.join().map_err(|_e| format!("unable to terminate"))?
}

fn log_events(simnet_events_rx: Receiver<SimnetEvent>, ctx: &Context) {
    info!(
        ctx.expect_logger(),
        "Surfpool: The best place to train before surfing Solana"
    );
    while let Ok(event) = simnet_events_rx.recv() {
        match event {
            SimnetEvent::AccountUpdate(account) => {
                info!(
                    ctx.expect_logger(),
                    "Account retrieved from Mainnet {}", account
                );
            }
            SimnetEvent::ClockUpdate(clock) => {
                info!(ctx.expect_logger(), "Slot #{} ", clock.slot);
            }
            SimnetEvent::ErroLog(log) => {
                error!(ctx.expect_logger(), "{} ", log);
            }
            SimnetEvent::InfoLog(log) => {
                info!(ctx.expect_logger(), "{} ", log);
            }
            SimnetEvent::WarnLog(log) => {
                warn!(ctx.expect_logger(), "{} ", log);
            }
            SimnetEvent::TransactionReceived(transaction) => {
                info!(ctx.expect_logger(), "Transaction received");
            }
            SimnetEvent::BlockHashExpired => {}
        }
    }
}
