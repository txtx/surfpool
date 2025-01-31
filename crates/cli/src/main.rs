mod macros;

#[macro_use]
extern crate hiro_system_kit;

pub mod cli;

fn main() {
    cli::main();
}