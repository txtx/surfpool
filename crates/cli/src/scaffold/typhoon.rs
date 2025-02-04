use crate::types::Framework;
use txtx_core::kit::helpers::fs::FileLocation;

pub fn try_get_programs_from_project(
    _base_location: FileLocation,
) -> Result<Option<(Framework, Vec<String>)>, String> {
    Ok(None)
}
