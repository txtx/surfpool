use clap::{ArgAction, CommandFactory, Parser, Subcommand};
use clap_complete::{Generator, Shell};
use hiro_system_kit::{self, Logger};
use std::{fs::File, path::PathBuf, process, str::FromStr};
use surfpool_core::solana_sdk::pubkey::Pubkey;
use surfpool_core::solana_sdk::signature::Keypair;
use surfpool_core::solana_sdk::signer::{EncodableKey, Signer};
use surfpool_types::{RpcConfig, SimnetConfig, SubgraphConfig, SurfpoolConfig};
use txtx_cloud::LoginCommand;
use txtx_core::manifest::WorkspaceManifest;
use txtx_gql::kit::helpers::fs::FileLocation;

use crate::runbook::handle_execute_runbook_command;

mod simnet;

#[derive(Clone)]
pub struct Context {
    pub logger: Option<Logger>,
    #[allow(dead_code)]
    pub tracer: bool,
}

pub const DEFAULT_SLOT_TIME_MS: &str = "400";
pub const DEFAULT_EXPLORER_PORT: &str = "8900";
pub const DEFAULT_SIMNET_PORT: &str = "8899";
pub const DEFAULT_TXTX_PORT: &str = "8488";
pub const DEFAULT_NETWORK_HOST: &str = "127.0.0.1";
pub const DEFAULT_RPC_URL: &str = "https://api.mainnet-beta.solana.com";
pub const DEFAULT_ID_SVC_URL: &str = "https://id.txtx.run/v1";
pub const DEFAULT_AUTH_SVC_URL: &str = "https://auth.txtx.run";
pub const DEFAULT_RUNBOOK: &str = "deployment";
pub const DEFAULT_AIRDROP_AMOUNT: &str = "10000000000000";
pub const DEFAULT_AIRDROPPED_KEYPAIR_PATH: &str = "~/.config/solana/id.json";

impl Context {
    #[allow(dead_code)]
    pub fn empty() -> Context {
        Context {
            logger: None,
            tracer: false,
        }
    }

    #[allow(dead_code)]
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
    #[clap(name = "start", bin_name = "start", aliases = &["simnet"])]
    Simnet(StartSimnet),
    /// Generate shell completions scripts
    #[clap(name = "completions", bin_name = "completions", aliases = &["completion"])]
    Completions(Completions),
    /// Run, runbook, run!
    #[clap(name = "run", bin_name = "run")]
    Run(ExecuteRunbook),
    /// List runbooks present in the current direcoty
    #[clap(name = "ls", bin_name = "ls")]
    List(ListRunbooks),
    /// Txtx cloud commands
    #[clap(subcommand, name = "cloud", bin_name = "cloud")]
    Cloud(CloudCommand),
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
    #[arg(long = "host", short = 'o', default_value = DEFAULT_NETWORK_HOST)]
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
    /// Watch programs (default: false)
    #[clap(long = "watch", action=ArgAction::SetTrue)]
    pub watch: bool,
    /// List of geyser plugins to load
    #[arg(long = "geyser-plugin-config", short = 'g')]
    pub plugin_config_path: Vec<String>,
}

impl StartSimnet {
    pub fn get_airdrop_addresses(&self) -> (Vec<Pubkey>, Vec<String>) {
        let mut airdrop_addresses = vec![];
        let mut errors = vec![];
        for address in self.airdrop_addresses.iter() {
            match Pubkey::from_str(&address).map_err(|e| e.to_string()) {
                Ok(pubkey) => {
                    airdrop_addresses.push(pubkey);
                }
                Err(e) => {
                    errors.push(format!(
                        "Unable to airdrop pubkey {}: Error parsing pubkey: {e}",
                        address
                    ));
                    continue;
                }
            }
        }

        for keypair_path in self.airdrop_keypair_path.iter() {
            let resolved = if keypair_path.starts_with("~") {
                format!(
                    "{}{}",
                    dirs::home_dir().unwrap().display(),
                    keypair_path[1..].to_string()
                )
            } else {
                keypair_path.clone()
            };
            let path = PathBuf::from(resolved);
            match Keypair::read_from_file(&path) {
                Ok(pubkey) => {
                    airdrop_addresses.push(pubkey.pubkey());
                }
                Err(e) => {
                    errors.push(format!(
                        "Unable to complete airdrop; Error reading keypair file: {}: {e}",
                        path.display()
                    ));
                    continue;
                }
            }
        }
        (airdrop_addresses, errors)
    }

    pub fn rpc_config(&self) -> RpcConfig {
        RpcConfig {
            bind_host: self.network_host.clone(),
            bind_port: self.simnet_port,
            remote_rpc_url: self.rpc_url.clone(),
        }
    }

    pub fn simnet_config(&self, airdrop_addresses: Vec<Pubkey>) -> SimnetConfig {
        SimnetConfig {
            remote_rpc_url: self.rpc_url.clone(),
            slot_time: self.slot_time,
            runloop_trigger_mode: surfpool_types::RunloopTriggerMode::Clock,
            airdrop_addresses: airdrop_addresses,
            airdrop_token_amount: self.airdrop_token_amount,
        }
    }

    pub fn subgraph_config(&self) -> SubgraphConfig {
        SubgraphConfig {}
    }

    pub fn surfpool_config(&self, airdrop_addresses: Vec<Pubkey>) -> SurfpoolConfig {
        let mut plugin_config_path = self
            .plugin_config_path
            .iter()
            .map(|f| PathBuf::from(f))
            .collect::<Vec<_>>();

        if plugin_config_path.is_empty() {
            plugin_config_path.push(PathBuf::from("plugins"));
        }
        SurfpoolConfig {
            simnet: self.simnet_config(airdrop_addresses),
            rpc: self.rpc_config(),
            subgraph: self.subgraph_config(),
            plugin_config_path,
        }
    }
}

#[derive(Parser, PartialEq, Clone, Debug)]
struct Completions {
    /// Specify which shell to generation completions script for
    #[arg(ignore_case = true)]
    pub shell: Shell,
}

#[derive(Parser, PartialEq, Clone, Debug)]
pub struct ListRunbooks {
    /// Path to the manifest
    #[arg(long = "manifest-file-path", short = 'm', default_value = "./txtx.yml")]
    pub manifest_path: String,
}

#[derive(Subcommand, PartialEq, Clone, Debug)]
pub enum CloudCommand {
    /// Login to the Txtx Cloud
    #[clap(name = "login", bin_name = "login")]
    Login(LoginCommand),
}

#[derive(Parser, PartialEq, Clone, Debug)]
#[command(group = clap::ArgGroup::new("execution_mode").multiple(false).args(["unsupervised", "web_console", "term_console"]).required(false))]
pub struct ExecuteRunbook {
    /// Path to the manifest
    #[arg(long = "manifest-file-path", short = 'm', default_value = "./txtx.yml")]
    pub manifest_path: String,
    /// Name of the runbook as indexed in the txtx.yml, or the path of the .tx file to run
    pub runbook: String,
    /// Execute the runbook without supervision
    #[arg(long = "unsupervised", short = 'u', action=ArgAction::SetTrue, group = "execution_mode")]
    pub unsupervised: bool,
    /// Execute the runbook with supervision via the browser UI (this is the default execution mode)
    #[arg(long = "browser", short = 'b', action=ArgAction::SetTrue, group = "execution_mode")]
    pub web_console: bool,
    /// Execute the runbook with supervision via the terminal console (coming soon)
    #[arg(long = "terminal", short = 't', action=ArgAction::SetTrue, group = "execution_mode")]
    pub term_console: bool,
    /// When running in unsupervised mode, print outputs in JSON format. If a directory is provided, the output will be written a file at the directory.
    #[arg(long = "output-json")]
    pub output_json: Option<Option<String>>,
    /// Pick a specific output to stdout at the end of the execution
    #[arg(long = "output", conflicts_with = "output_json")]
    pub output: Option<String>,
    /// Explain how the runbook will be executed.
    #[arg(long = "explain", action=ArgAction::SetTrue)]
    pub explain: bool,
    /// Set the port for hosting the web UI
    #[arg(long = "port", short = 'p', default_value = txtx_supervisor_ui::DEFAULT_BINDING_PORT )]
    #[cfg(feature = "supervisor_ui")]
    pub network_binding_port: u16,
    /// Set the port for hosting the web UI
    #[arg(long = "ip", short = 'i', default_value = txtx_supervisor_ui::DEFAULT_BINDING_ADDRESS )]
    #[cfg(feature = "supervisor_ui")]
    pub network_binding_ip_address: String,
    /// Choose the environment variable to set from those configured in the txtx.yml
    #[arg(long = "env")]
    pub environment: Option<String>,
    /// A set of inputs to use for batch processing
    #[arg(long = "input")]
    pub inputs: Vec<String>,
    /// Execute the Runbook even if the cached state suggests this Runbook has already been executed
    #[arg(long = "force", short = 'f')]
    pub force_execution: bool,
}

impl ExecuteRunbook {
    pub fn default_localnet(runbook_name: &str) -> ExecuteRunbook {
        ExecuteRunbook {
            manifest_path: "./txtx.yml".to_string(),
            runbook: runbook_name.to_string(),
            unsupervised: true,
            web_console: false,
            term_console: false,
            output_json: Some(Some("runbook-outputs".to_string())),
            output: None,
            explain: false,
            #[cfg(feature = "supervisor_ui")]
            network_binding_port: u16::from_str(txtx_supervisor_ui::DEFAULT_BINDING_PORT).unwrap(),
            #[cfg(feature = "supervisor_ui")]
            network_binding_ip_address: txtx_supervisor_ui::DEFAULT_BINDING_ADDRESS.to_string(),
            environment: Some("localnet".to_string()),
            inputs: vec![],
            force_execution: false,
        }
    }

    pub fn with_manifest_path(mut self, manifest_path: String) -> Self {
        self.manifest_path = manifest_path;
        self
    }

    pub fn do_start_supervisor_ui(&self) -> bool {
        self.web_console || (!self.unsupervised && !self.term_console)
    }
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
        Command::Run(cmd) => handle_execute_runbook_command(cmd).await,
        Command::List(cmd) => handle_list_command(cmd, ctx).await,
        Command::Cloud(cmd) => handle_cloud_commands(cmd).await,
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

pub async fn handle_list_command(cmd: ListRunbooks, _ctx: &Context) -> Result<(), String> {
    let manifest_location = FileLocation::from_path_string(&cmd.manifest_path)?;
    let manifest = WorkspaceManifest::from_location(&manifest_location)?;
    if manifest.runbooks.is_empty() {
        println!("{}: no runbooks referenced in the txtx.yml manifest.\nRun the command `txtx new` to create a new runbook.", yellow!("warning"));
        std::process::exit(1);
    }
    println!("{:<35}\t{}", "Name", yellow!("Description"));
    for runbook in manifest.runbooks {
        println!(
            "{:<35}\t{}",
            runbook.name,
            yellow!(format!("{}", runbook.description.unwrap_or("".into())))
        );
    }
    Ok(())
}

async fn handle_cloud_commands(cmd: CloudCommand) -> Result<(), String> {
    match cmd {
        CloudCommand::Login(cmd) => {
            txtx_cloud::login::handle_login_command(
                &cmd,
                DEFAULT_AUTH_SVC_URL,
                DEFAULT_TXTX_PORT,
                DEFAULT_ID_SVC_URL,
            )
            .await
        }
    }
}
