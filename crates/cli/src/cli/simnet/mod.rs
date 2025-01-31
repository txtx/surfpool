use super::{Context, StartSimnet};
use surfpool_core::start_simnet;

pub async fn handle_start_simnet_command(_cmd: &StartSimnet, _ctx: &Context) -> Result<(), String> {
    println!("Where you train before surfing Solana");
    start_simnet();
    Ok(())
}
