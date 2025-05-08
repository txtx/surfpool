use std::fmt;

use clap::{builder::PossibleValue, Parser, ValueEnum};
use dialoguer::{console::Style, theme::ColorfulTheme, Input, Select};
use surfpool_types::{BlockProductionMode, CreateNetworkRequest, CreateNetworkResponse};
use txtx_cloud::{
    auth::AuthConfig, login::pat_login, workspace::fetch_svm_workspaces, LoginCommand,
};
use txtx_gql::kit::{reqwest, uuid::Uuid};

use crate::cli::DEFAULT_RPC_URL;

#[derive(Parser, PartialEq, Clone, Debug)]
pub struct CloudStartCommand {
    /// The name of the workspace
    #[arg(long = "workspace", short = 'w')]
    pub workspace_name: Option<String>,

    /// The name of the surfnet that will be created
    #[arg(long = "name", short = 'n')]
    pub name: Option<String>,

    /// A description for the surfnet
    #[arg(long = "description", short = 'd')]
    pub description: Option<String>,

    /// The RPC url to use for the datasource
    #[arg(long = "rpc-url", short = 'u', default_value = DEFAULT_RPC_URL)]
    pub datasource_rpc_url: String,

    /// The block production mode for the surfnet. Options are `clock`, `transaction`, and `manual`.
    #[arg(long = "block-production", short = 'b')]
    pub block_production_mode: Option<CliBlockProductionMode>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CliBlockProductionMode(BlockProductionMode);
impl ValueEnum for CliBlockProductionMode {
    fn value_variants<'a>() -> &'a [Self] {
        &[
            CliBlockProductionMode(BlockProductionMode::Clock),
            CliBlockProductionMode(BlockProductionMode::Transaction),
            CliBlockProductionMode(BlockProductionMode::Manual),
        ]
    }

    fn to_possible_value(&self) -> Option<PossibleValue> {
        match self.0 {
            BlockProductionMode::Clock => Some(PossibleValue::new("clock")),
            BlockProductionMode::Transaction => Some(PossibleValue::new("transaction")),
            BlockProductionMode::Manual => Some(PossibleValue::new("manual")),
        }
    }
}

impl CliBlockProductionMode {
    fn choices() -> Vec<String> {
        vec![
            "Produce blocks every 400ms".to_string(),
            "Only produce blocks when transactions are received".to_string(),
            "Full manual control (via RPC methods / cloud.txtx.run)".to_string(),
        ]
    }
    fn from_index(index: usize) -> Result<Self, String> {
        let m = match index {
            0 => BlockProductionMode::Clock,
            1 => BlockProductionMode::Transaction,
            2 => BlockProductionMode::Manual,
            _ => return Err(format!("invalid block production mode index: {}", index)),
        };
        Ok(Self(m))
    }
}

impl fmt::Display for CliBlockProductionMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            BlockProductionMode::Clock => write!(f, "clock"),
            BlockProductionMode::Transaction => write!(f, "transaction"),
            BlockProductionMode::Manual => write!(f, "manual"),
        }
    }
}

impl CloudStartCommand {
    pub async fn start(
        &self,
        auth_service_url: &str,
        auth_callback_port: &str,
        id_service_url: &str,
        svm_gql_url: &str,
        svm_cloud_api_url: &str,
    ) -> Result<(), String> {
        let mut auth_config = match AuthConfig::read_from_system_config()
            .map_err(|e| format!("failed to authenticate user: {e}"))?
        {
            Some(auth_config) => auth_config,
            None => {
                println!(
                    "{} Not authenticated, you must log in to continue",
                    yellow!("-")
                );
                txtx_cloud::login::handle_login_command(
                    &LoginCommand::default(),
                    auth_service_url,
                    auth_callback_port,
                    id_service_url,
                )
                .await?;
                AuthConfig::read_from_system_config()
                    .map_err(|e| format!("failed to authenticate user: {e}"))?
                    .ok_or(
                        "failed to authenticate user: no auth data found after login".to_string(),
                    )?
            }
        };

        auth_config
            .refresh_session_if_needed(&id_service_url)
            .await?;

        let theme = ColorfulTheme {
            values_style: Style::new().green(),
            hint_style: Style::new().cyan(),
            ..ColorfulTheme::default()
        };

        let mut workspaces = fetch_svm_workspaces(&auth_config.access_token, svm_gql_url)
            .await
            .map_err(|e| format!("failed to get available workspaces: {}", e))?;

        if workspaces.is_empty() {
            if let Some(pat) = auth_config.pat {
                let jwt_manager = txtx_cloud::auth::jwt::JwtManager::initialize(id_service_url)
                    .await
                    .map_err(|e| format!("Failed to initialize JWT manager: {}", e))?;

                // Retry, this time with a brand new refresh token
                let auth_config = pat_login(&id_service_url, &jwt_manager, &pat)
                    .await
                    .map_err(|e| format!("failed to login with PAT: {}", e))?;

                workspaces = fetch_svm_workspaces(&auth_config.access_token, svm_gql_url)
                    .await
                    .map_err(|e| format!("failed to get available workspaces: {}", e))?;

                if workspaces.is_empty() {
                    return Err("no workspaces found".to_string());
                }
            }
        }

        let (workspace_names, workspace_ids): (Vec<String>, Vec<Uuid>) = workspaces
            .iter()
            .map(|workspace| (workspace.name.clone(), workspace.id))
            .unzip();

        let selected_workspace_idx = Select::with_theme(&theme)
            .with_prompt("Select the workspace to create the surfnet in")
            .items(&workspace_names)
            .default(0)
            .interact()
            .map_err(|e| format!("unable to fetch workspaces: {e}"))?;

        let workspace_id = workspace_ids[selected_workspace_idx];

        let name = match &self.name {
            Some(name) => name.clone(),
            None => {
                // Ask for the name of the surfnet
                Input::with_theme(&theme)
                    .with_prompt("Enter the name of the surfnet")
                    .interact_text()
                    .map_err(|e| format!("unable to get surfnet name: {e}"))?
            }
        };

        let description = match &self.description {
            Some(description) => Some(description.clone()),
            None => {
                // Ask for the description of the surfnet
                let description: String = Input::with_theme(&theme)
                    .with_prompt("Enter an optional description for the surfnet")
                    .allow_empty(true)
                    .interact_text()
                    .map_err(|e| format!("unable to get surfnet description: {e}"))?;
                if description.is_empty() {
                    None
                } else {
                    Some(description)
                }
            }
        };

        let datasource_rpc_url: String = Input::with_theme(&theme)
            .with_prompt("Enter the RPC URL for the datasource")
            .default(self.datasource_rpc_url.clone())
            .interact_text()
            .map_err(|e| format!("unable to get datasource RPC URL: {e}"))?;

        let block_production_mode = match &self.block_production_mode {
            Some(mode) => mode.0.clone(),
            None => {
                // Ask for the block production mode

                let selected_mode = Select::with_theme(&theme)
                    .with_prompt("Select the block production mode")
                    .items(&CliBlockProductionMode::choices())
                    .default(0)
                    .interact()
                    .map_err(|e| format!("unable to get block production mode: {e}"))?;

                CliBlockProductionMode::from_index(selected_mode)?.0
            }
        };

        println!("{} Spinning up your hosted Surfnet...", yellow!("→"));
        let request = CreateNetworkRequest::new(
            workspace_id,
            name,
            description,
            datasource_rpc_url,
            block_production_mode,
        );
        let client = reqwest::Client::new();
        let res = client
            .post(svm_cloud_api_url)
            .bearer_auth(auth_config.access_token)
            .json(&request)
            .send()
            .await
            .map_err(|e| format!("failed to send request to start cloud surfnet: {e}"))?;

        if !res.status().is_success() {
            let err = res
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(format!("failed to start cloud surfnet: {}", err));
        }

        let res = res
            .json::<CreateNetworkResponse>()
            .await
            .map_err(|e| format!("failed to parse response: {e}"))?;

        println!(
            "\n🌊 Surf is up for network '{}'\n- Dashboard: {}\n- Rpc url:   {}\n",
            request.name,
            green!("https://cloud.txtx.run/networks"),
            green!(res.rpc_url)
        );

        Ok(())
    }
}
