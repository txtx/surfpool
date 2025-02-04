#[macro_use]
mod macros;

#[macro_use]
extern crate hiro_system_kit;

pub mod cli;
// pub mod manifest;
pub mod runbook;
pub mod scaffold;
pub mod tui;
pub mod types;

fn main() {
    cli::main();
}
