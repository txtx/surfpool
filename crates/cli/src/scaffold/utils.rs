use std::collections::{BTreeMap, HashMap};

use anyhow::{anyhow, Result};
use convert_case::{Case, Casing};
use serde::Deserialize;
use txtx_addon_network_svm::codec::idl::IdlRef;
use txtx_gql::kit::helpers::fs::FileLocation;

use super::ProgramMetadata;

pub fn get_program_metadata_from_manifest_with_dep(
    dependency_indicator: &str,
    base_location: &FileLocation,
    manifest: &CargoManifestFile,
) -> Result<Option<ProgramMetadata>> {
    let Some(manifest) =
        manifest.get_manifest_with_dependency(dependency_indicator, base_location)?
    else {
        return Ok(None);
    };

    let Some(package) = manifest.package else {
        return Ok(None);
    };

    let program_name = package.name.to_case(Case::Snake);

    let mut potential_idl_path = base_location.clone();
    let _ = potential_idl_path.append_path(&format!("idl/{program_name}.json"));
    let idl = if potential_idl_path.exists() {
        let idl_content = potential_idl_path
            .read_content()
            .map_err(|e| anyhow!("failed to read program idl: {e}"))?;

        let idl_ref = IdlRef::from_bytes(&idl_content)
            .map_err(|e| anyhow!("failed to convert idl to anchor-style idl: {e}"))?;

        let idl = serde_json::to_string_pretty(&idl_ref.idl)
            .map_err(|e| anyhow!("failed to serialize idl: {e}"))?;
        Some(idl)
    } else {
        None
    };

    Ok(Some(ProgramMetadata::new(&program_name, &idl)))
}

#[derive(Debug, Clone, Deserialize)]
pub struct CargoManifestFile {
    pub package: Option<Package>,
    pub dependencies: Option<BTreeMap<String, Dependency>>,
    pub workspace: Option<Workspace>,
}

impl CargoManifestFile {
    pub fn from_manifest_str(manifest: &str) -> Result<Self, String> {
        let manifest: CargoManifestFile =
            toml::from_str(manifest).map_err(|e| format!("failed to parse Cargo.toml: {}", e))?;
        Ok(manifest)
    }

    pub fn get_manifest_with_dependency(
        &self,
        name: &str,
        base_location: &FileLocation,
    ) -> Result<Option<CargoManifestFile>> {
        if let Some(deps) = &self.dependencies {
            if deps.get(name).is_some() {
                return Ok(Some(self.clone()));
            }
        }
        if let Some(workspace) = self.workspace.as_ref() {
            for member_manifest in workspace.get_member_cargo_manifests(base_location)? {
                if let Some(manifest) =
                    member_manifest.get_manifest_with_dependency(name, base_location)?
                {
                    return Ok(Some(manifest));
                }
            }
        }
        Ok(None)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Package {
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Workspace {
    pub members: Vec<String>,

    #[serde(rename = "workspace.dependencies")]
    #[allow(dead_code)]
    pub workspace_dependencies: Option<HashMap<String, Dependency>>,
}

impl Workspace {
    pub fn get_member_cargo_manifests(
        &self,
        base_location: &FileLocation,
    ) -> Result<Vec<CargoManifestFile>> {
        let mut member_manifests = vec![];
        for member in &self.members {
            let mut member_location = base_location.clone();
            member_location
                .append_path(member)
                .map_err(|e| anyhow!("failed to append path: {}", e))?;
            member_location
                .append_path("Cargo.toml")
                .map_err(|e| anyhow!("failed to append path: {}", e))?;
            if member_location.exists() {
                let manifest = member_location
                    .read_content_as_utf8()
                    .map_err(|e| anyhow!("{e}"))?;
                let manifest = CargoManifestFile::from_manifest_str(&manifest)
                    .map_err(|e| anyhow!("unable to read Cargo.toml: {}", e))?;
                member_manifests.push(manifest);
            }
        }
        Ok(member_manifests)
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
#[allow(dead_code)]
pub enum Dependency {
    Version(String),
    Detailed(DependencyDetail),
}

#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct DependencyDetail {
    pub version: Option<String>,
    pub features: Option<Vec<String>>,
    pub path: Option<String>,
}
