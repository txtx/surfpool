use ratatui::widgets::Block;
use std::io::Write;
use txtx_addon_network_svm::SvmNetworkAddon;
use txtx_core::{
    kit::{
        channel::{unbounded, Sender},
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
    progress_tx: Sender<BlockEvent>,
    txtx_manifest_location: &FileLocation,
) -> Result<(), String> {
    let manifest = WorkspaceManifest::from_location(&txtx_manifest_location)?;
    let runbook_selector = vec![runbook_id.to_string()];
    let mut runbooks =
        read_runbooks_from_manifest(&manifest, &Some("localnet".into()), Some(&runbook_selector))?;
    let top_level_inputs_map =
        manifest.get_runbook_inputs(&Some("localnet".into()), &vec![], None)?;

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
