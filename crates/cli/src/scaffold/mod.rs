use dialoguer::{console::Style, theme::ColorfulTheme, Input, Select};
use std::{
    env,
    fs::{self, File},
};
use txtx_core::{
    kit::helpers::fs::FileLocation,
    manifest::{RunbookMetadata, WorkspaceManifest},
    templates::{build_manifest_data, TXTX_MANIFEST_TEMPLATE, TXTX_README_TEMPLATE},
};

use crate::{runbook::execute_runbook, types::Framework};

mod anchor;
mod native;
mod steel;
mod typhoon;

pub async fn detect_program_frameworks(
    manifest_path: &str,
) -> Result<Option<(Framework, Vec<String>)>, String> {
    let manifest_location = FileLocation::from_path_string(manifest_path)?;
    let base_dir = manifest_location.get_parent_location()?;
    // Look for Anchor project layout
    if let Some((framework, programs, runbook)) =
        anchor::try_get_programs_from_project(base_dir.clone())?
    {
        scaffold_runbooks_layout(runbook, base_dir)?;
        return Ok(Some((framework, programs)));
    }

    // Look for Native project layout
    if let Some((framework, programs)) = native::try_get_programs_from_project(base_dir.clone())? {
        return Ok(Some((framework, programs)));
    }

    // Look for Steel project layout
    if let Some((framework, programs)) = steel::try_get_programs_from_project(base_dir.clone())? {
        return Ok(Some((framework, programs)));
    }

    // Look for Typhoon project layout
    if let Some((framework, programs)) = typhoon::try_get_programs_from_project(base_dir.clone())? {
        return Ok(Some((framework, programs)));
    }

    Ok(None)
}

pub fn scaffold_runbooks_layout(
    runbook_source: String,
    base_location: FileLocation,
) -> Result<(), String> {
    let theme = ColorfulTheme {
        values_style: Style::new().green(),
        hint_style: Style::new().cyan(),
        ..ColorfulTheme::default()
    };

    let mut txtx_manifest_location = base_location.clone();
    txtx_manifest_location.append_path("txtx.yml")?;
    let manifest_res = WorkspaceManifest::from_location(&txtx_manifest_location);

    // Pick a name for the workspace. By default, we suggest the name of the current directory
    let mut manifest = match manifest_res {
        Ok(manifest) => manifest,
        Err(_) => {
            let current_dir = env::current_dir()
                .ok()
                .and_then(|d| d.file_name().map(|f| f.to_string_lossy().to_string()));
            let default = match current_dir {
                Some(dir) => dir,
                _ => "".to_string(),
            };

            // Ask for the name of the workspace
            let name: String = Input::with_theme(&theme)
                .with_prompt("Enter the name of this workspace")
                .default(default)
                .interact_text()
                .unwrap();
            WorkspaceManifest::new(name)
        }
    };

    let action = "deployments";
    let runbook_name = "v1";
    let description = "Deployment 101".to_string();
    let runbook = RunbookMetadata::new(&action, &runbook_name, Some(description));
    let runbook_id = &runbook.name.clone();
    manifest.runbooks.push(runbook);

    let mut runbook_file_location = base_location.clone();
    runbook_file_location.append_path("runbooks")?;

    let manifest_location = if let Some(location) = manifest.location.clone() {
        location
    } else {
        let manifest_name = "txtx.yml";
        let mut manifest_location = base_location.clone();
        let _ = manifest_location.append_path(manifest_name);
        let _ = File::create(manifest_location.to_string()).expect("creation failed");
        println!("{} {}", green!("Created manifest"), manifest_name);
        manifest_location
    };
    let mut manifest_file = File::create(manifest_location.to_string()).expect("creation failed");

    let manifest_file_data = build_manifest_data(&manifest);
    let template =
        mustache::compile_str(TXTX_MANIFEST_TEMPLATE).expect("Failed to compile template");
    template
        .render_data(&mut manifest_file, &manifest_file_data)
        .expect("Failed to render template");

    // Create runbooks directory
    match runbook_file_location.exists() {
        true => {}
        false => {
            fs::create_dir_all(&runbook_file_location.to_string()).map_err(|e| {
                format!(
                    "unable to create parent directory {}\n{}",
                    runbook_file_location.to_string(),
                    e
                )
            })?;
            println!("{} runbooks", green!("Created directory"));
        }
    }

    let mut readme_file_path = runbook_file_location.clone();
    readme_file_path.append_path("README.md")?;
    match readme_file_path.exists() {
        true => {}
        false => {
            let mut readme_file =
                File::create(readme_file_path.to_string()).expect("creation failed");
            let readme_file_data = build_manifest_data(&manifest);
            let template =
                mustache::compile_str(TXTX_README_TEMPLATE).expect("Failed to compile template");
            template
                .render_data(&mut readme_file, &readme_file_data)
                .expect("Failed to render template");
            println!("{} runbooks/README.md", green!("Created file"));
        }
    }

    // Create runbooks subdirectory
    runbook_file_location.append_path(action)?;
    match runbook_file_location.exists() {
        true => {}
        false => {
            fs::create_dir_all(&runbook_file_location.to_string()).map_err(|e| {
                format!(
                    "unable to create parent directory {}\n{}",
                    runbook_file_location.to_string(),
                    e
                )
            })?;
            println!(
                "{} {}",
                green!("Created directory"),
                runbook_file_location
                    .get_relative_path_from_base(&base_location)
                    .unwrap()
            );
        }
    }

    // Create runbook
    runbook_file_location.append_path(&format!("{}.tx", runbook_id))?;

    match runbook_file_location.exists() {
        true => {
            // return Err(format!(
            //     "file {} already exists. choose a different runbook name, or rename the existing file",
            //     runbook_file_location.to_string()
            // ))
            return Ok(());
        }
        false => {
            let runbook_file =
                File::create(runbook_file_location.to_string()).expect("creation failed");
            runbook_file_location.write_content(runbook_source.as_bytes())?;
            println!(
                "{} {}",
                green!("Created runbook"),
                runbook_file_location
                    .get_relative_path_from_base(&base_location)
                    .unwrap()
            );
        }
    }
    Ok(())
}
