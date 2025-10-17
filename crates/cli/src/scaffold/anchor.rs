#![allow(dead_code)]

use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
    str::FromStr,
};

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use txtx_core::kit::helpers::fs::FileLocation;
use url::Url;
use walkdir::WalkDir;

use super::ProgramMetadata;
use crate::{scaffold::GenesisEntry, types::Framework};

pub fn try_get_programs_from_project(
    base_location: FileLocation,
) -> Result<Option<(Framework, Vec<ProgramMetadata>, Option<Vec<GenesisEntry>>)>, String> {
    let mut manifest_location = base_location.clone();
    manifest_location.append_path("Anchor.toml")?;
    if manifest_location.exists() {
        let mut programs = vec![];

        // Load anchor_manifest_path toml
        let manifest = manifest_location.read_content_as_utf8()?;
        let manifest = AnchorManifest::from_manifest_str(&manifest, &base_location)
            .map_err(|e| format!("unable to read Anchor.toml: {}", e))?;

        let mut target_location = base_location.clone();
        target_location.append_path("target")?;
        if let Some((_, deployments)) = manifest.programs.iter().next() {
            for (program_name, deployment) in deployments.iter() {
                programs.push(ProgramMetadata::new(program_name, &deployment.idl));
            }
        }
        let mut genesis_entries = manifest
            .test
            .as_ref()
            .and_then(|test| test.genesis.as_ref())
            .cloned()
            .unwrap_or_default();
        if let Some(test_configs) = TestConfig::discover_test_toml(&base_location.expect_path_buf())
            .map_err(|e| {
                format!(
                    "failed to discover Test.toml files in workspace: {}",
                    e.to_string()
                )
            })?
        {
            for (_, config) in test_configs.test_suite_configs.iter() {
                if let Some(test_config) = config.test.as_ref() {
                    if let Some(genesis) = test_config.genesis.as_ref() {
                        genesis_entries.extend(genesis.clone());
                    }
                }
            }
        }

        Ok(Some((
            Framework::Anchor,
            programs,
            if genesis_entries.is_empty() {
                None
            } else {
                Some(genesis_entries)
            },
        )))
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
    pub test: Option<TestValidatorConfig>,
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
    test: Option<TestValidatorConfig>,
}

impl AnchorManifest {
    pub fn from_manifest_str(manifest_str: &str, base_location: &FileLocation) -> Result<Self> {
        let cfg: AnchorManifestFile = toml::from_str(manifest_str)
            .map_err(|e| anyhow!("Unable to deserialize config: {e}"))?;
        Ok(AnchorManifest {
            toolchain: cfg.toolchain.unwrap_or_default(),
            features: cfg.features.unwrap_or_default(),
            registry: cfg.registry.unwrap_or_default(),
            scripts: cfg.scripts.unwrap_or_default(),
            programs: cfg
                .programs
                .map_or(Ok(BTreeMap::new()), |p| deser_programs(p, base_location))?,
            workspace: cfg.workspace.unwrap_or_default(),
            test: cfg.test,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestTomlFile {
    pub extends: Option<Vec<String>>,
    pub test: Option<TestValidatorConfig>,
    pub scripts: Option<ScriptsConfig>,
}

impl TestTomlFile {
    fn from_path(path: impl AsRef<Path>) -> Result<Self, anyhow::Error> {
        let s = std::fs::read_to_string(&path)?;
        let parsed_toml: Self = toml::from_str(&s)?;
        let mut current_toml = TestTomlFile {
            extends: None,
            test: None,
            scripts: None,
        };
        if let Some(bases) = &parsed_toml.extends {
            for base in bases {
                let mut canonical_base = base.clone();
                canonical_base = canonicalize_filepath_from_origin(&canonical_base, &path)?;
                current_toml.merge(TestTomlFile::from_path(&canonical_base)?);
            }
        }
        current_toml.merge(parsed_toml);

        if let Some(test) = &mut current_toml.test {
            if let Some(genesis_programs) = &mut test.genesis {
                for entry in genesis_programs {
                    entry.program = canonicalize_filepath_from_origin(&entry.program, &path)?;
                }
            }
        }
        Ok(current_toml)
    }
}

impl From<TestTomlFile> for TestToml {
    fn from(value: TestTomlFile) -> Self {
        Self {
            test: value.test,
            scripts: value.scripts.unwrap_or_default(),
        }
    }
}

impl TestTomlFile {
    fn merge(&mut self, other: Self) {
        let mut my_scripts = self.scripts.take();
        match &mut my_scripts {
            None => my_scripts = other.scripts,
            Some(my_scripts) => {
                if let Some(other_scripts) = other.scripts {
                    for (name, script) in other_scripts {
                        my_scripts.insert(name, script);
                    }
                }
            }
        }

        let mut my_test = self.test.take();
        match &mut my_test {
            Some(my_test) => {
                if let Some(other_test) = other.test {
                    if let Some(other_genesis) = other_test.genesis {
                        match &mut my_test.genesis {
                            Some(my_genesis) => {
                                for other_entry in other_genesis {
                                    match my_genesis
                                        .iter()
                                        .position(|g| *g.address == other_entry.address)
                                    {
                                        None => my_genesis.push(other_entry),
                                        Some(i) => my_genesis[i] = other_entry,
                                    }
                                }
                            }
                            None => my_test.genesis = Some(other_genesis),
                        }
                    }
                }
            }
            None => my_test = other.test,
        };

        // Instantiating a new Self object here ensures that
        // this function will fail to compile if new fields get added
        // to Self. This is useful as a reminder if they also require merging
        *self = Self {
            test: my_test,
            scripts: my_scripts,
            extends: self.extends.take(),
        };
    }
}

fn canonicalize_filepath_from_origin(
    file_path: impl AsRef<Path>,
    origin: impl AsRef<Path>,
) -> Result<String> {
    use anyhow::Context;
    let previous_dir = std::env::current_dir()?;
    std::env::set_current_dir(origin.as_ref().parent().unwrap())?;
    let result = std::fs::canonicalize(&file_path)
        .with_context(|| {
            format!(
                "Error reading (possibly relative) path: {}. If relative, this is the path that was used as the current path: {}",
                &file_path.as_ref().display(),
                &origin.as_ref().display()
            )
        })?
        .display()
        .to_string();
    std::env::set_current_dir(previous_dir)?;
    Ok(result)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestToml {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub test: Option<TestValidatorConfig>,
    pub scripts: ScriptsConfig,
}
impl TestToml {
    pub fn from_path(p: impl AsRef<Path>) -> Result<Self> {
        TestTomlFile::from_path(&p).map(Into::into).map_err(|e| {
            anyhow!(
                "Unable to read Test.toml at {}: {}",
                p.as_ref().display(),
                e
            )
        })
    }
}
#[derive(Debug, Clone)]
pub struct TestConfig {
    pub test_suite_configs: HashMap<PathBuf, TestToml>,
}
fn is_hidden(entry: &walkdir::DirEntry) -> bool {
    entry
        .file_name()
        .to_str()
        .map(|s| (s != "." && (s.starts_with('.') || s.starts_with("./."))) || s == "target")
        .unwrap_or(false)
}
impl TestConfig {
    pub fn discover_test_toml(root: impl AsRef<Path>) -> Result<Option<Self>> {
        let walker = WalkDir::new(root).into_iter();
        let mut test_suite_configs = HashMap::new();
        for entry in walker.filter_entry(|e| !is_hidden(e)) {
            let entry = entry?;
            if entry.file_name() == "Test.toml" {
                let entry_path = entry.path();
                let test_toml = TestToml::from_path(entry_path)?;
                test_suite_configs.insert(entry.path().into(), test_toml);
            }
        }

        Ok(match test_suite_configs.is_empty() {
            true => None,
            false => Some(Self { test_suite_configs }),
        })
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct AnchorProgramDeployment {
    pub address: String,
    pub path: Option<String>,
    pub idl: Option<String>,
}

impl AnchorProgramDeployment {
    pub fn new(
        program_name: &str,
        program_id: &serde_json::Value,
        base_location: &FileLocation,
    ) -> Result<Self> {
        let mut idl_location = base_location.clone();
        let _ = idl_location.append_path(&format!("target/idl/{program_name}.json"));
        let idl = if idl_location.exists() {
            Some(
                idl_location
                    .read_content_as_utf8()
                    .map_err(|e| anyhow!("failed to read program idl: {e}"))?,
            )
        } else {
            None
        };
        match &program_id {
            serde_json::Value::String(address) => Ok(AnchorProgramDeployment {
                address: address.clone(),
                path: None,
                idl,
            }),

            serde_json::Value::Object(_) => {
                let dep: AnchorProgramDeployment = serde_json::from_value(program_id.clone())
                    .map_err(|_| anyhow!("Unable to read Anchor.toml"))?;
                Ok(AnchorProgramDeployment {
                    address: dep.address,
                    idl,
                    path: dep.path,
                })
            }
            _ => Err(anyhow!(
                "Invalid type for program definition in Anchor.toml"
            )),
        }
    }
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
                    ws_url
                        .set_port(Some(port + 1))
                        .map_err(|_| anyhow!("Unable to set port"))?;
                }
                if ws_url.scheme() == "https" {
                    ws_url
                        .set_scheme("wss")
                        .map_err(|_| anyhow!("Unable to set scheme"))?;
                } else {
                    ws_url
                        .set_scheme("ws")
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

pub type ProgramsConfig = BTreeMap<Cluster, BTreeMap<String, AnchorProgramDeployment>>;

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct WorkspaceConfig {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub members: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub types: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestValidatorConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub genesis: Option<Vec<GenesisEntry>>,
}

fn deser_programs(
    programs: BTreeMap<String, BTreeMap<String, serde_json::Value>>,
    base_location: &FileLocation,
) -> Result<BTreeMap<Cluster, BTreeMap<String, AnchorProgramDeployment>>> {
    programs
        .iter()
        .map(|(cluster, programs)| {
            let cluster: Cluster = cluster.parse()?;
            let programs = programs
                .iter()
                .map(|(name, program_id)| {
                    Ok((
                        name.clone(),
                        AnchorProgramDeployment::new(name, program_id, base_location)?,
                    ))
                })
                .collect::<Result<BTreeMap<String, AnchorProgramDeployment>>>()?;
            Ok((cluster, programs))
        })
        .collect::<Result<BTreeMap<Cluster, BTreeMap<String, AnchorProgramDeployment>>>>()
}

#[derive(Debug, Clone)]
pub struct BuildConfig {
    pub verifiable: bool,
    pub solana_version: Option<String>,
    pub docker_image: String,
}
