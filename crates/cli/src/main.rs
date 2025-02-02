#[macro_use]
mod macros;

#[macro_use]
extern crate hiro_system_kit;

pub mod cli;
pub mod tui;

fn main() {
    cli::main();
}
