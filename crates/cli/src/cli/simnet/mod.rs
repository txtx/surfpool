use std::sync::mpsc::channel;

use crate::tui;

use super::{Context, StartSimnet};
use surfpool_core::start_simnet;

pub fn handle_start_simnet_command(_cmd: &StartSimnet, ctx: &Context) -> Result<(), String> {
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
    tui::simnet::start_app(simnet_events_rx).map_err(|e| format!("{}", e))?;
    handle.join().map_err(|_e| format!("unable to terminate"))?
}
