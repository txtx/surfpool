use std::collections::{BTreeMap, HashMap};

use anyhow::{Result, anyhow};
use serde::Deserialize;
use txtx_gql::kit::helpers::fs::FileLocation;

use super::ProgramMetadata;

pub fn get_program_metadata_from_manifest_with_dep(
    dependency_indicator: &str,
    base_location: &FileLocation,
    manifest: &CargoManifestFile,
    artifacts_path: Option<&str>,
) -> Result<Option<ProgramMetadata>> {
    let Some(manifest) =
        manifest.get_manifest_with_dependency(dependency_indicator, base_location)?
    else {
        return Ok(None);
    };

    let Some(package) = manifest.package else {
        return Ok(None);
    };

    let program_name = package.name;

    let so_exists = {
        let so_path_str = if let Some(artifacts) = artifacts_path {
            format!("{}/{}.so", artifacts, program_name)
        } else {
            format!("target/deploy/{}.so", program_name)
        };
        let mut so_path = base_location.clone();
        so_path.append_path(&so_path_str).map_err(|e| {
            anyhow!("failed to construct path to program .so file for existence check: {e}")
        })?;
        so_path.exists()
    };

    Ok(Some(ProgramMetadata::new(&program_name, so_exists)))
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

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    fn temp_test_dir(test_name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("surfpool-{test_name}-{unique}"))
    }

    fn manifest_with_solana_program(package_name: &str) -> CargoManifestFile {
        CargoManifestFile::from_manifest_str(&format!(
            r#"
                [package]
                name = "{package_name}"
                version = "0.1.0"

                [dependencies]
                solana-program = "3"
            "#
        ))
        .expect("manifest should parse")
    }

    #[test]
    fn preserves_cargo_package_name_without_snake_case_conversion() {
        let base_dir = temp_test_dir("preserve-package-name");
        fs::create_dir_all(base_dir.join("target/deploy")).expect("test dir should be created");
        fs::write(base_dir.join("target/deploy/project2.so"), [])
            .expect("test .so file should be created");

        let base_location =
            FileLocation::from_path_string(base_dir.to_string_lossy().as_ref()).unwrap();
        let manifest = manifest_with_solana_program("project2");

        let metadata = get_program_metadata_from_manifest_with_dep(
            "solana-program",
            &base_location,
            &manifest,
            None,
        )
        .expect("lookup should succeed")
        .expect("program metadata should be returned");

        assert_eq!(metadata.name, "project2");
        assert!(metadata.so_exists);

        fs::remove_dir_all(&base_dir).expect("test dir should be cleaned up");
    }

    #[test]
    fn preserves_existing_snake_case_package_name() {
        let base_dir = temp_test_dir("preserve-snake-case-name");
        fs::create_dir_all(base_dir.join("target/deploy")).expect("test dir should be created");
        fs::write(base_dir.join("target/deploy/project_2.so"), [])
            .expect("test .so file should be created");

        let base_location =
            FileLocation::from_path_string(base_dir.to_string_lossy().as_ref()).unwrap();
        let manifest = manifest_with_solana_program("project_2");

        let metadata = get_program_metadata_from_manifest_with_dep(
            "solana-program",
            &base_location,
            &manifest,
            None,
        )
        .expect("lookup should succeed")
        .expect("program metadata should be returned");

        assert_eq!(metadata.name, "project_2");
        assert!(metadata.so_exists);

        fs::remove_dir_all(&base_dir).expect("test dir should be cleaned up");
    }
}
