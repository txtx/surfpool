use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::{Generator, Shell};
use hiro_system_kit::{self, Logger};
use std::{fs::File, process};

mod simnet;

#[derive(Clone)]
pub struct Context {
    pub logger: Option<Logger>,
    pub tracer: bool,
}

pub const DEFAULT_BINDING_PORT: &str = "8899";
pub const DEFAULT_BINDING_ADDRESS: &str = "localhost";

#[allow(dead_code)]
impl Context {
    pub fn empty() -> Context {
        Context {
            logger: None,
            tracer: false,
        }
    }

    pub fn try_log<F>(&self, closure: F)
    where
        F: FnOnce(&Logger),
    {
        if let Some(ref logger) = self.logger {
            closure(logger)
        }
    }

    pub fn expect_logger(&self) -> &Logger {
        self.logger.as_ref().unwrap()
    }
}

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None, name = "surfpool", bin_name = "surfpool")]
struct Opts {
    #[clap(subcommand)]
    command: Command,
}

#[derive(Subcommand, PartialEq, Clone, Debug)]
enum Command {
    /// Start Simnet
    #[clap(name = "simnet", bin_name = "simnet", aliases = &["start"])]
    Simnet(StartSimnet),
    /// Generate shell completions scripts
    #[clap(name = "completions", bin_name = "completions", aliases = &["completion"])]
    Completions(Completions),
}

#[derive(Parser, PartialEq, Clone, Debug)]
pub struct StartSimnet {
    /// Path to the manifest
    #[arg(
        long = "manifest-file-path",
        short = 'm',
        default_value = "./Surfpool.toml"
    )]
    pub manifest_path: String,
    /// Set the port
    #[arg(long = "port", short = 'p', default_value = DEFAULT_BINDING_PORT )]
    pub network_binding_port: u16,
    /// Set the ip
    #[arg(long = "ip", short = 'i', default_value = DEFAULT_BINDING_ADDRESS )]
    pub network_binding_ip_address: String,
    /// Display streams of logs instead of terminal UI dashboard
    #[clap(long = "no-tui")]
    pub no_tui: bool,
}

#[derive(Parser, PartialEq, Clone, Debug)]
struct Completions {
    /// Specify which shell to generation completions script for
    #[arg(ignore_case = true)]
    pub shell: Shell,
}

pub fn main() {
    let logger = hiro_system_kit::log::setup_logger();
    let _guard = hiro_system_kit::log::setup_global_logger(logger.clone());
    let ctx = Context {
        logger: Some(logger),
        tracer: false,
    };

    let opts: Opts = match Opts::try_parse() {
        Ok(opts) => opts,
        Err(e) => {
            println!("{}", e);
            process::exit(1);
        }
    };

    if let Err(e) = hiro_system_kit::nestable_block_on(handle_command(opts, &ctx)) {
        error!(ctx.expect_logger(), "{e}");
        std::thread::sleep(std::time::Duration::from_millis(500));
        process::exit(1);
    }
}

async fn handle_command(opts: Opts, ctx: &Context) -> Result<(), String> {
    match opts.command {
        Command::Simnet(cmd) => simnet::handle_start_simnet_command(&cmd, ctx),
        Command::Completions(cmd) => generate_completion_helpers(&cmd),
    }
}

fn generate_completion_helpers(cmd: &Completions) -> Result<(), String> {
    let mut app = Opts::command();
    let file_name = cmd.shell.file_name("surfpool");
    let mut file = File::create(file_name.clone())
        .map_err(|e| format!("unable to create file {}: {}", file_name, e))?;
    clap_complete::generate(cmd.shell, &mut app, "surfpool", &mut file);
    println!("{} {}", green!("Created file"), file_name.clone());
    println!("Check your shell’s docs for how to enable completions for surfpool.");
    Ok(())
}
