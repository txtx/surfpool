use txtx_core::kit::helpers::fs::FileLocation;

use super::ProgramMetadata;
use crate::types::Framework;

pub fn try_get_programs_from_project(
    _base_location: FileLocation,
) -> Result<Option<(Framework, Vec<ProgramMetadata>)>, String> {
    Ok(None)
}
