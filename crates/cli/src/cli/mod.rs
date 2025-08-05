use std::{env, fs::File, path::PathBuf, process, str::FromStr};

use clap::{ArgAction, CommandFactory, Parser, Subcommand};
use clap_complete::{Generator, Shell};
use hiro_system_kit::{self, Logger};
use solana_keypair::Keypair;
use solana_pubkey::Pubkey;
use solana_signer::{EncodableKey, Signer};
use surfpool_mcp::McpOptions;
use surfpool_types::{
    CHANGE_TO_DEFAULT_STUDIO_PORT_ONCE_SUPERVISOR_MERGED, DEFAULT_NETWORK_HOST, DEFAULT_RPC_PORT,
    DEFAULT_SLOT_TIME_MS, DEFAULT_WS_PORT, RpcConfig, SimnetConfig, StudioConfig, SubgraphConfig,
    SurfpoolConfig,
};
use txtx_cloud::LoginCommand;
use txtx_core::manifest::WorkspaceManifest;
use txtx_gql::kit::helpers::fs::FileLocation;

use crate::{cloud::CloudStartCommand, runbook::handle_execute_runbook_command};

mod simnet;

#[derive(Clone)]
pub struct Context {
    pub logger: Option<Logger>,
    #[allow(dead_code)]
    pub tracer: bool,
}

pub const DEFAULT_RPC_URL: &str = "https://api.mainnet-beta.solana.com";
pub const DEVNET_RPC_URL: &str = "https://api.devnet.solana.com";
pub const TESTNET_RPC_URL: &str = "https://api.testnet.solana.com";
pub const DEFAULT_ID_SVC_URL: &str = "https://id.txtx.run/v1";
pub const DEFAULT_CLOUD_URL: &str = "https://cloud.txtx.run";
pub const DEFAULT_SVM_GQL_URL: &str = "https://svm-cloud.gql.txtx.run/v1/graphql";
pub const DEFAULT_SVM_CLOUD_API_URL: &str = "https://svm-cloud-api.txtx.run/v1/surfnets";
pub const DEFAULT_RUNBOOK: &str = "deployment";
pub const DEFAULT_AIRDROP_AMOUNT: &str = "10000000000000";

lazy_static::lazy_static! {
    pub static ref DEFAULT_SOLANA_KEYPAIR_PATH: String = {
        PathBuf::from("~").join(".config").join("solana")
            .join("id.json")
            .display()
            .to_string()
    };
}

/// Gets the user's home directory, accounting for the Snap confinement environment.
/// We set out snap build to set this environment variable to the real home directory,
/// because by default, snaps run in a confined environment where the home directory is not
/// the user's actual home directory.
pub fn get_home_dir() -> String {
    if let Ok(real_home) = env::var("SNAP_REAL_HOME") {
        let path_buf = PathBuf::from(real_home);
        path_buf.display().to_string()
    } else {
        dirs::home_dir().unwrap().display().to_string()
    }
}

/// Resolves a path, expanding the `~` to the user's home directory if present.
pub fn resolve_path(path: &str) -> PathBuf {
    let path = if let Some(stripped) = path.strip_prefix("~") {
        let joined = format!("{}{}", get_home_dir(), stripped);
        joined
    } else {
        path.to_string()
    };
    PathBuf::from(path)
}

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
    /// Start MCP server
    #[clap(name = "mcp", bin_name = "mcp")]
    Mcp,
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
    #[arg(long = "port", short = 'p', default_value_t = DEFAULT_RPC_PORT)]
    pub simnet_port: u16,
    /// Set the Simnet WS port
    #[arg(long = "ws-port", short = 'w', default_value_t = DEFAULT_WS_PORT)]
    pub ws_port: u16,
    /// Set the Simnet host address
    #[arg(long = "host", short = 'o', default_value = DEFAULT_NETWORK_HOST)]
    pub network_host: String,
    /// Set the slot time
    #[arg(long = "slot-time", short = 't', default_value_t = DEFAULT_SLOT_TIME_MS)]
    pub slot_time: u64,
    /// Set a datasource RPC URL (cannot be used with --network). Can also be set via SURFPOOL_DATASOURCE_RPC_URL.
    #[arg(long = "rpc-url", short = 'u', conflicts_with = "network")]
    pub rpc_url: Option<String>,
    /// Choose a predefined network (cannot be used with --rpc-url)
    #[arg(long = "network", short = 'n', value_enum, conflicts_with = "rpc_url")]
    pub network: Option<NetworkType>,
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
    #[arg(long = "airdrop-keypair-path", short = 'k', default_value = DEFAULT_SOLANA_KEYPAIR_PATH.as_str())]
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
    /// Path to subgraph's sqlite database (default: :memory:)
    #[arg(long = "subgraph-database-path", short = 'd')]
    pub subgraph_database_path: Option<String>,
    /// Disable Studio (default: true)
    #[clap(long = "no-studio")]
    pub no_studio: bool,
    /// Set the Studio port
    #[arg(long = "studio-port", short = 's', default_value_t = CHANGE_TO_DEFAULT_STUDIO_PORT_ONCE_SUPERVISOR_MERGED)]
    pub studio_port: u16,
}

#[derive(clap::ValueEnum, PartialEq, Clone, Debug)]
pub enum NetworkType {
    /// Solana Mainnet-Beta (https://api.mainnet-beta.solana.com)
    Mainnet,
    /// Solana Devnet (https://api.devnet.solana.com)
    Devnet,
    /// Solana Testnet (https://api.testnet.solana.com)
    Testnet,
}

impl StartSimnet {
    pub fn get_airdrop_addresses(&self) -> (Vec<Pubkey>, Vec<String>) {
        let mut airdrop_addresses = vec![];
        let mut errors = vec![];
        for address in self.airdrop_addresses.iter() {
            match Pubkey::from_str(address).map_err(|e| e.to_string()) {
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
            let path = resolve_path(keypair_path);
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
            ws_port: self.ws_port,
        }
    }

    pub fn studio_config(&self) -> StudioConfig {
        StudioConfig {
            bind_host: self.network_host.clone(),
            bind_port: self.studio_port,
        }
    }

    pub fn simnet_config(&self, airdrop_addresses: Vec<Pubkey>) -> SimnetConfig {
        let remote_rpc_url = match &self.network {
            Some(NetworkType::Mainnet) => DEFAULT_RPC_URL.to_string(),
            Some(NetworkType::Devnet) => DEVNET_RPC_URL.to_string(),
            Some(NetworkType::Testnet) => TESTNET_RPC_URL.to_string(),
            None => match self.rpc_url {
                Some(ref rpc_url) => rpc_url.clone(),
                None => match env::var("SURFPOOL_DATASOURCE_RPC_URL") {
                    Ok(value) => value,
                    _ => DEFAULT_RPC_URL.to_string(),
                },
            },
        };

        SimnetConfig {
            remote_rpc_url,
            slot_time: self.slot_time,
            block_production_mode: surfpool_types::BlockProductionMode::Clock,
            airdrop_addresses,
            airdrop_token_amount: self.airdrop_token_amount,
            expiry: None,
        }
    }

    pub fn subgraph_config(&self) -> SubgraphConfig {
        SubgraphConfig {}
    }

    pub fn surfpool_config(&self, airdrop_addresses: Vec<Pubkey>) -> SurfpoolConfig {
        let plugin_config_path = self
            .plugin_config_path
            .iter()
            .map(PathBuf::from)
            .collect::<Vec<_>>();

        SurfpoolConfig {
            simnets: vec![self.simnet_config(airdrop_addresses)],
            rpc: self.rpc_config(),
            subgraph: self.subgraph_config(),
            studio: self.studio_config(),
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
    /// Start a new Cloud Surfnet instance
    #[clap(name = "start", bin_name = "start")]
    Start(CloudStartCommand),
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

#[derive(Subcommand, PartialEq, Clone, Debug)]
pub enum McpCommand {}

pub async fn handle_mcp_command(_ctx: &Context) -> Result<(), String> {
    surfpool_mcp::run_server(&McpOptions::default()).await?;
    Ok(())
}

async fn handle_command(opts: Opts, ctx: &Context) -> Result<(), String> {
    match opts.command {
        Command::Simnet(cmd) => simnet::handle_start_local_surfnet_command(&cmd, ctx).await,
        Command::Completions(cmd) => generate_completion_helpers(&cmd),
        Command::Run(cmd) => handle_execute_runbook_command(cmd).await,
        Command::List(cmd) => handle_list_command(cmd, ctx).await,
        Command::Cloud(cmd) => handle_cloud_commands(cmd).await,
        Command::Mcp => handle_mcp_command(ctx).await,
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
        println!(
            "{}: no runbooks referenced in the txtx.yml manifest.\nRun the command `txtx new` to create a new runbook.",
            yellow!("warning")
        );
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
                DEFAULT_CLOUD_URL,
                &CHANGE_TO_DEFAULT_STUDIO_PORT_ONCE_SUPERVISOR_MERGED.to_string(),
                DEFAULT_ID_SVC_URL,
            )
            .await
        }
        CloudCommand::Start(cmd) => {
            cmd.start(
                DEFAULT_CLOUD_URL,
                &CHANGE_TO_DEFAULT_STUDIO_PORT_ONCE_SUPERVISOR_MERGED.to_string(),
                DEFAULT_ID_SVC_URL,
                DEFAULT_SVM_GQL_URL,
                DEFAULT_SVM_CLOUD_API_URL,
            )
            .await
        }
    }
}
