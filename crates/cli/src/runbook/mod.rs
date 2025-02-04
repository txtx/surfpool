use std::io::Write;
use txtx_addon_network_svm::SvmNetworkAddon;
use txtx_core::{
    kit::{
        channel::unbounded,
        helpers::fs::FileLocation,
        types::{
            frontend::{BlockEvent, ProgressBarStatusColor},
            AuthorizationContext,
        },
        Addon,
    },
    manifest::{file::read_runbooks_from_manifest, WorkspaceManifest},
    start_unsupervised_runbook_runloop,
    std::StdAddon,
};

pub fn get_addon_by_namespace(namespace: &str) -> Option<Box<dyn Addon>> {
    let available_addons: Vec<Box<dyn Addon>> =
        vec![Box::new(StdAddon::new()), Box::new(SvmNetworkAddon::new())];
    for addon in available_addons.into_iter() {
        if namespace.starts_with(&format!("{}", addon.get_namespace())) {
            return Some(addon);
        }
    }
    None
}

pub async fn execute_runbook(
    runbook_id: &str,
    manifest_location: &FileLocation,
) -> Result<(), String> {
    let base_dir_location = manifest_location.get_parent_location()?;
    let mut txtx_manifest_location = base_dir_location.clone();
    txtx_manifest_location.append_path("txtx.yml")?;

    let manifest = WorkspaceManifest::from_location(&txtx_manifest_location)?;
    let runbook_selector = vec![runbook_id.to_string()];
    let mut runbooks = read_runbooks_from_manifest(&manifest, &None, Some(&runbook_selector))?;
    let top_level_inputs_map = manifest.get_runbook_inputs(&None, &vec![], None)?;

    let Some((mut runbook, runbook_sources, state, smt)) = runbooks.swap_remove(runbook_id) else {
        return Err(format!("Deployment {} not found", runbook_id));
    };
    println!("{} '{}' successfully checked", green!("✓"), runbook_id);

    let authorization_context = AuthorizationContext::new(manifest.location.clone().unwrap());
    let res = runbook
        .build_contexts_from_sources(
            runbook_sources,
            top_level_inputs_map,
            authorization_context,
            get_addon_by_namespace,
        )
        .await;
    if let Err(diags) = res {
        println!("{:?}", diags);
    }

    runbook.enable_full_execution_mode();

    let (progress_tx, progress_rx) = unbounded();
    // should not be generating actions
    let _ = hiro_system_kit::thread_named("Display background tasks logs").spawn(move || {
        while let Ok(msg) = progress_rx.recv() {
            match msg {
                BlockEvent::UpdateProgressBarStatus(update) => {
                    match update.new_status.status_color {
                        ProgressBarStatusColor::Yellow => {
                            print!(
                                "\r{} {} {:<150}",
                                yellow!("→"),
                                yellow!(format!("{}", update.new_status.status)),
                                update.new_status.message,
                            );
                        }
                        ProgressBarStatusColor::Green => {
                            print!(
                                "\r{} {} {:<150}\n",
                                green!("✓"),
                                green!(format!("{}", update.new_status.status)),
                                update.new_status.message,
                            );
                        }
                        ProgressBarStatusColor::Red => {
                            print!(
                                "\r{} {} {:<150}\n",
                                red!("x"),
                                red!(format!("{}", update.new_status.status)),
                                update.new_status.message,
                            );
                        }
                        ProgressBarStatusColor::Purple => {
                            print!(
                                "\r{} {} {:<150}\n",
                                purple!("→"),
                                purple!(format!("{}", update.new_status.status)),
                                update.new_status.message,
                            );
                        }
                    };
                    std::io::stdout().flush().unwrap();
                }
                _ => {}
            }
        }
    });

    let res = start_unsupervised_runbook_runloop(&mut runbook, &progress_tx).await;
    if let Err(diags) = res {
        println!("{} Execution aborted", red!("x"));
        for diag in diags.iter() {
            println!("{}", red!(format!("- {}", diag)));
        }
        // write_runbook_transient_state(&mut runbook, runbook_state)?;
        return Ok(());
    }

    if let Err(diags) = res {
        for diag in diags.iter() {
            println!("{} {}", red!("x"), diag);
        }
        std::process::exit(1);
    }

    Ok(())
}
