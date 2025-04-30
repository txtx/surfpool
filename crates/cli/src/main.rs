#[macro_use]
mod macros;
mod cloud;

#[macro_use]
extern crate hiro_system_kit;

mod cli;
// mod manifest;
mod http;
mod runbook;
mod scaffold;
mod tui;
mod types;

fn main() {
    cli::main();
}
