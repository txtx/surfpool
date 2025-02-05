use anyhow::{anyhow, Error, Result};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, str::FromStr};
use txtx_addon_network_svm::SvmNetworkAddon;
use txtx_core::kit::helpers::fs::FileLocation;
use txtx_core::kit::types::AuthorizationContext;
use url::Url;

use crate::types::Framework;

pub fn try_get_programs_from_project(
    base_location: FileLocation,
) -> Result<Option<(Framework, Vec<String>)>, String> {
    let mut manifest_location = base_location.clone();
    manifest_location.append_path("Anchor.toml")?;
    if manifest_location.exists() {
        let mut programs = vec![];

        // Load anchor_manifest_path toml
        let manifest = manifest_location.read_content_as_utf8()?;
        let manifest = AnchorManifest::from_str(&manifest)
            .map_err(|e| format!("unable to read Anchor.toml: {}", e))?;

        let mut target_location = base_location.clone();
        target_location.append_path("target")?;
        let addon = SvmNetworkAddon::new();
        let _function_spec = &addon.get_deploy_action_spec();
        let _context = AuthorizationContext::empty();

        if let Some((_, deployments)) = manifest.programs.iter().next() {
            for (program_name, _deployment) in deployments.iter() {
                programs.push(program_name.clone());
            }
        }

        Ok(Some((Framework::Anchor, programs)))
    } else {
        Ok(None)
    }
}

#[derive(Debug, Default)]
pub struct AnchorManifest {
    pub toolchain: ToolchainConfig,
    pub features: FeaturesConfig,
    pub registry: RegistryConfig,
    // pub provider: ProviderConfig,
    pub programs: ProgramsConfig,
    pub scripts: ScriptsConfig,
    pub workspace: WorkspaceConfig,
}

#[derive(Debug, Deserialize)]
pub struct AnchorManifestFile {
    toolchain: Option<ToolchainConfig>,
    features: Option<FeaturesConfig>,
    programs: Option<BTreeMap<String, BTreeMap<String, serde_json::Value>>>,
    registry: Option<RegistryConfig>,
    // provider: Provider,
    workspace: Option<WorkspaceConfig>,
    scripts: Option<ScriptsConfig>,
}

impl FromStr for AnchorManifest {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let cfg: AnchorManifestFile =
            toml::from_str(s).map_err(|e| anyhow!("Unable to deserialize config: {e}"))?;
        Ok(AnchorManifest {
            toolchain: cfg.toolchain.unwrap_or_default(),
            features: cfg.features.unwrap_or_default(),
            registry: cfg.registry.unwrap_or_default(),
            scripts: cfg.scripts.unwrap_or_default(),
            programs: cfg.programs.map_or(Ok(BTreeMap::new()), deser_programs)?,
            workspace: cfg.workspace.unwrap_or_default(),
        })
    }
}

#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct ToolchainConfig {
    pub anchor_version: Option<String>,
    pub solana_version: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FeaturesConfig {
    /// Enable account resolution.
    ///
    /// Not able to specify default bool value: https://github.com/serde-rs/serde/issues/368
    #[serde(default = "FeaturesConfig::get_default_resolution")]
    pub resolution: bool,
    /// Disable safety comment checks
    #[serde(default, rename = "skip-lint")]
    pub skip_lint: bool,
}

impl FeaturesConfig {
    fn get_default_resolution() -> bool {
        true
    }
}

impl Default for FeaturesConfig {
    fn default() -> Self {
        Self {
            resolution: Self::get_default_resolution(),
            skip_lint: false,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RegistryConfig {
    pub url: String,
}

impl Default for RegistryConfig {
    fn default() -> Self {
        Self {
            url: "https://api.apr.dev".to_string(),
        }
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ProgramDeployment {
    pub address: String,
    pub path: Option<String>,
    pub idl: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, Eq, PartialEq, Ord, PartialOrd)]
#[serde(rename_all = "snake_case")]
pub enum Cluster {
    Testnet,
    Mainnet,
    Devnet,
    #[default]
    Localnet,
    Debug,
    Custom(String, String),
}

impl FromStr for Cluster {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Cluster> {
        match s.to_lowercase().as_str() {
            "t" | "testnet" => Ok(Cluster::Testnet),
            "m" | "mainnet" => Ok(Cluster::Mainnet),
            "d" | "devnet" => Ok(Cluster::Devnet),
            "l" | "localnet" => Ok(Cluster::Localnet),
            "g" | "debug" => Ok(Cluster::Debug),
            _ if s.starts_with("http") => {
                let http_url = s;

                // Taken from:
                // https://github.com/solana-labs/solana/blob/aea8f0df1610248d29d8ca3bc0d60e9fabc99e31/web3.js/src/util/url.ts

                let mut ws_url = Url::parse(http_url)?;
                if let Some(port) = ws_url.port() {
                    ws_url.set_port(Some(port + 1))
                        .map_err(|_| anyhow!("Unable to set port"))?;
                }
                if ws_url.scheme() == "https" {
                    ws_url.set_scheme("wss")
                        .map_err(|_| anyhow!("Unable to set scheme"))?;
                } else {
                    ws_url.set_scheme("ws")
                        .map_err(|_| anyhow!("Unable to set scheme"))?;
                }


                Ok(Cluster::Custom(http_url.to_string(), ws_url.to_string()))
            }
            _ => Err(anyhow::Error::msg(
                "Cluster must be one of [localnet, testnet, mainnet, devnet] or be an http or https url\n",
            )),
        }
    }
}

#[derive(Debug, Default, Deserialize)]
pub struct ProviderConfig {
    pub cluster: Cluster,
    pub wallet: String,
}

pub type ScriptsConfig = BTreeMap<String, String>;

pub type ProgramsConfig = BTreeMap<Cluster, BTreeMap<String, ProgramDeployment>>;

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct WorkspaceConfig {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub members: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub types: String,
}

fn deser_programs(
    programs: BTreeMap<String, BTreeMap<String, serde_json::Value>>,
) -> Result<BTreeMap<Cluster, BTreeMap<String, ProgramDeployment>>> {
    programs
        .iter()
        .map(|(cluster, programs)| {
            let cluster: Cluster = cluster.parse()?;
            let programs = programs
                .iter()
                .map(|(name, program_id)| {
                    Ok((
                        name.clone(),
                        ProgramDeployment::try_from(match &program_id {
                            serde_json::Value::String(address) => ProgramDeployment {
                                address: address.clone(),
                                path: None,
                                idl: None,
                            },

                            serde_json::Value::Object(_) => {
                                serde_json::from_value(program_id.clone())
                                    .map_err(|_| anyhow!("Unable to read toml"))?
                            }
                            _ => return Err(anyhow!("Invalid toml type")),
                        })?,
                    ))
                })
                .collect::<Result<BTreeMap<String, ProgramDeployment>>>()?;
            Ok((cluster, programs))
        })
        .collect::<Result<BTreeMap<Cluster, BTreeMap<String, ProgramDeployment>>>>()
}

#[derive(Debug, Clone)]
pub struct BuildConfig {
    pub verifiable: bool,
    pub solana_version: Option<String>,
    pub docker_image: String,
}
