use anyhow::{anyhow, Result};
use txtx_core::kit::helpers::fs::FileLocation;

use super::{
    utils::{get_program_metadata_from_manifest_with_dep, CargoManifestFile},
    ProgramMetadata,
};
use crate::types::Framework;

/// This function attempts to load a program from a native project.
/// It looks for a `Cargo.toml` file in the specified base location.
/// If the `Cargo.toml` has a package with the `pinocchio` dependency,
/// it is considered a native project.
pub fn try_get_programs_from_project(
    base_location: FileLocation,
) -> Result<Option<(Framework, Vec<ProgramMetadata>)>> {
    let mut manifest_location = base_location.clone();
    manifest_location
        .append_path("Cargo.toml")
        .map_err(|e| anyhow!("{e}"))?;
    if manifest_location.exists() {
        let manifest = manifest_location
            .read_content_as_utf8()
            .map_err(|e| anyhow!("{e}"))?;
        let manifest = CargoManifestFile::from_manifest_str(&manifest)
            .map_err(|e| anyhow!("unable to read Cargo.toml: {}", e))?;

        let Some(program_metadata) =
            get_program_metadata_from_manifest_with_dep("pinocchio", &base_location, &manifest)?
        else {
            return Ok(None);
        };

        Ok(Some((Framework::Pinocchio, vec![program_metadata])))
    } else {
        Ok(None)
    }
}
