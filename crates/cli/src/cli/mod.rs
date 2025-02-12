use clap::{ArgAction, CommandFactory, Parser, Subcommand};
use clap_complete::{Generator, Shell};
use hiro_system_kit::{self, Logger};
use std::{fs::File, process};

mod simnet;

#[allow(dead_code)]
#[derive(Clone)]
pub struct Context {
    pub logger: Option<Logger>,
    pub tracer: bool,
}

pub const DEFAULT_SLOT_TIME_MS: &str = "400";
pub const DEFAULT_EXPLORER_PORT: &str = "8900";
pub const DEFAULT_SIMNET_PORT: &str = "8899";
pub const DEFAULT_NETWORK_HOST: &str = "127.0.0.1";
pub const DEFAULT_RPC_URL: &str = "https://api.mainnet-beta.solana.com";
pub const DEFAULT_RUNBOOK: &str = "deployment";
pub const DEFAULT_AIRDROP_AMOUNT: &str = "10000000000000";
pub const DEFAULT_AIRDROPPED_KEYPAIR_PATH: &str = "~/.config/solana/id.json";

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
    #[clap(name = "run", bin_name = "run", aliases = &["run", "start", "simnet"])]
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
    /// Set the Simnet RPC port
    #[arg(long = "port", short = 'p', default_value = DEFAULT_SIMNET_PORT)]
    pub simnet_port: u16,
    /// Set the Simnet host address
    #[arg(long = "host", short = 'h', default_value = DEFAULT_NETWORK_HOST)]
    pub network_host: String,
    /// Set the slot time
    #[arg(long = "slot-time", short = 's', default_value = DEFAULT_SLOT_TIME_MS)]
    pub slot_time: u64,
    /// Set the ip
    #[arg(long = "rpc-url", short = 'u', default_value = DEFAULT_RPC_URL)]
    pub rpc_url: String,
    /// Display streams of logs instead of terminal UI dashboard (default: false)
    #[clap(long = "no-tui")]
    pub no_tui: bool,
    /// Include debug logs (default: false)
    #[clap(long = "debug", action=ArgAction::SetTrue)]
    pub debug: bool,
    /// Disable auto deployments (default: false)
    #[clap(long = "no-deploy")]
    pub no_deploy: bool,
    /// List of runbooks-id to run  
    #[arg(long = "runbook", short = 'r', default_value = DEFAULT_RUNBOOK)]
    pub runbooks: Vec<String>,
    /// List of pubkeys to airdrop
    #[arg(long = "airdrop", short = 'a')]
    pub airdrop_addresses: Vec<String>,
    /// Quantity of tokens to airdrop
    #[arg(long = "airdrop-amount", short = 'q', default_value = DEFAULT_AIRDROP_AMOUNT)]
    pub airdrop_token_amount: u64,
    /// List of keypair paths to airdrop
    #[arg(long = "airdrop-keypair-path", short = 'k', default_value = DEFAULT_AIRDROPPED_KEYPAIR_PATH)]
    pub airdrop_keypair_path: Vec<String>,
    /// Disable explorer (default: false)
    #[clap(long = "no-explorer")]
    pub no_explorer: bool,
    /// List of geyser plugins to load
    #[arg(long = "geyser-plugin-config", short = 'g')]
    pub plugin_config_path: Vec<String>,
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
        Command::Simnet(cmd) => simnet::handle_start_simnet_command(&cmd, ctx).await,
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
