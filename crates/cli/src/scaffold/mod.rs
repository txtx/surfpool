use dialoguer::{console::Style, theme::ColorfulTheme, Confirm, Input, MultiSelect};
use std::{
    env,
    fs::{self, File},
};
use txtx_addon_network_svm::templates::{
    get_interpolated_addon_template, get_interpolated_anchor_program_deployment_template,
    get_interpolated_header_template, get_interpolated_mainnet_signer_template,
    get_interpolated_signer_template,
};
use txtx_core::{
    kit::{helpers::fs::FileLocation, indexmap::indexmap},
    manifest::{RunbookMetadata, WorkspaceManifest},
    templates::{build_manifest_data, TXTX_MANIFEST_TEMPLATE, TXTX_README_TEMPLATE},
};

use crate::types::Framework;

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
    if let Some((framework, programs)) = anchor::try_get_programs_from_project(base_dir.clone())
        .map_err(|e| format!("Invalid Anchor project: {e}"))?
    {
        return Ok(Some((framework, programs)));
    }

    // Look for Native project layout
    if let Some((framework, programs)) = native::try_get_programs_from_project(base_dir.clone())
        .map_err(|e| format!("Invalid Native project: {e}"))?
    {
        return Ok(Some((framework, programs)));
    }

    // Look for Steel project layout
    if let Some((framework, programs)) = steel::try_get_programs_from_project(base_dir.clone())
        .map_err(|e| format!("Invalid Steel project: {e}"))?
    {
        return Ok(Some((framework, programs)));
    }

    // Look for Typhoon project layout
    if let Some((framework, programs)) = typhoon::try_get_programs_from_project(base_dir.clone())
        .map_err(|e| format!("Invalid Typhoon project: {e}"))?
    {
        return Ok(Some((framework, programs)));
    }

    Ok(None)
}

pub fn scaffold_iac_layout(
    programs: Vec<String>,
    base_location: &FileLocation,
) -> Result<(), String> {
    let theme = ColorfulTheme {
        values_style: Style::new().green(),
        hint_style: Style::new().cyan(),
        ..ColorfulTheme::default()
    };

    let selection = MultiSelect::with_theme(&theme)
        .with_prompt("Programs to deploy:")
        .items(&programs)
        .interact()
        .unwrap();

    let selected_programs = selection
        .iter()
        .map(|i| programs[*i].clone())
        .collect::<Vec<_>>();

    let mut target_location = base_location.clone();
    target_location.append_path("target")?;

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

    let mut runbook_src: String = String::new();
    runbook_src.push_str(&get_interpolated_header_template(&format!(
        "Manage {} deployment through Crypto Infrastructure as Code",
        manifest.name
    )));
    runbook_src.push_str(&get_interpolated_addon_template(
        "input.rpc_api_url",
        "input.network_id",
    ));

    let mut signer_mainnet = String::new();
    // signer_mainnet.push_str(&get_interpolated_header_template(&format!("Runbook")));
    // signer_mainnet.push_str(&get_interpolated_addon_template("http://localhost:8899"));
    signer_mainnet.push_str(&get_interpolated_mainnet_signer_template(
        "input.authority_keypair_json",
    ));

    let mut signer_testnet = String::new();
    // signer_testnet.push_str(&get_interpolated_header_template(&format!("Runbook")));
    // signer_testnet.push_str(&get_interpolated_addon_template("http://localhost:8899"));
    signer_testnet.push_str(&get_interpolated_signer_template(
        "input.authority_keypair_json",
    ));

    let mut signer_simnet = String::new();
    // signer_simnet.push_str(&get_interpolated_header_template(&format!("Runbook")));
    // signer_simnet.push_str(&get_interpolated_addon_template("http://localhost:8899"));
    signer_simnet.push_str(&get_interpolated_signer_template(
        "input.authority_keypair_json",
    ));

    for program_name in selected_programs.iter() {
        runbook_src.push_str(&get_interpolated_anchor_program_deployment_template(
            program_name,
        ));
        // Configure initialize instruction
        // let args = vec![
        //     Value::string("hellosol".into()),
        //     Value::string(target_location.to_string())
        // ];
        // let command = GetProgramFromAnchorProject::run(function_spec, &context, &args);
    }

    let runbook_name = "deployment";
    let description = Some("Deploy programs".to_string());
    let location = format!("runbooks/deployment");

    let runbook = RunbookMetadata {
        location,
        description,
        name: runbook_name.to_string(),
        state: None,
    };

    let mut collision = false;
    for r in manifest.runbooks.iter() {
        if r.name.eq(&runbook.name) {
            collision = true;
        }
    }
    if !collision {
        manifest.runbooks.push(runbook);
    } else {
        // todo
    }

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

    manifest.environments.insert(
        "localnet".into(),
        indexmap! {
            "network_id".to_string() => "localnet".to_string(),
            "rpc_api_url".to_string() => "http://127.0.0.1:8899".to_string(),
            "payer_keypair_json".to_string() => "~/.config/solana/id.json".to_string(),
            "authority_keypair_json".to_string() => "~/.config/solana/id.json".to_string(),
        },
    );
    manifest.environments.insert(
        "devnet".into(),
        indexmap! {
            "network_id".to_string() => "devnet".to_string(),
            "rpc_api_url".to_string() => "https://api.devnet.solana.com".to_string(),
            "payer_keypair_json".to_string() => "~/.config/solana/id.json".to_string(),
            "authority_keypair_json".to_string() => "~/.config/solana/id.json".to_string(),
        },
    );

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

    runbook_file_location.append_path(&format!("deployment"))?;
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
        }
    }

    // Create runbook
    runbook_file_location.append_path(&format!("main.tx"))?;
    match runbook_file_location.exists() {
        true => {
            // return Err(format!(
            //     "file {} already exists. choose a different runbook name, or rename the existing file",
            //     runbook_file_location.to_string()
            // ))
            return Ok(());
        }
        false => {
            let _ = File::create(runbook_file_location.to_string()).expect("creation failed");
            runbook_file_location.write_content(runbook_src.as_bytes())?;
            println!(
                "{} {}",
                green!("Created file"),
                runbook_file_location
                    .get_relative_path_from_base(&base_location)
                    .unwrap()
            );
            let mut base_dir = runbook_file_location.get_parent_location().unwrap();
            base_dir.append_path(&format!("signers.localnet.tx"))?;
            let _ = File::create(base_dir.to_string()).expect("creation failed");
            base_dir.write_content(signer_simnet.as_bytes())?;
            println!(
                "{} {}",
                green!("Created file"),
                base_dir
                    .get_relative_path_from_base(&base_location)
                    .unwrap()
            );
            let mut base_dir = base_dir.get_parent_location().unwrap();
            base_dir.append_path(&format!("signers.devnet.tx"))?;
            let _ = File::create(base_dir.to_string()).expect("creation failed");
            base_dir.write_content(signer_testnet.as_bytes())?;
            println!(
                "{} {}",
                green!("Created file"),
                base_dir
                    .get_relative_path_from_base(&base_location)
                    .unwrap()
            );
            let mut base_dir = base_dir.get_parent_location().unwrap();
            base_dir.append_path(&format!("signers.mainnet.tx"))?;
            let _ = File::create(base_dir.to_string()).expect("creation failed");
            base_dir.write_content(signer_mainnet.as_bytes())?;
            println!(
                "{} {}",
                green!("Created file"),
                base_dir
                    .get_relative_path_from_base(&base_location)
                    .unwrap()
            );
        }
    }

    println!("\n{}\n", runbook_src);

    let confirmation = Confirm::with_theme(&theme)
        .with_prompt(
            "Review your deployment in 'runbooks/deployment/main.tx' and confirm to continue",
        )
        .interact()
        .unwrap();

    if !confirmation {
        println!("Deployment canceled");
    }

    Ok(())
}
