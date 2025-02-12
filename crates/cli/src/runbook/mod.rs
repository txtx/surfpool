use txtx_addon_network_svm::SvmNetworkAddon;
use txtx_core::{
    kit::{
        channel::Sender,
        helpers::fs::FileLocation,
        types::{frontend::BlockEvent, AuthorizationContext},
        Addon,
    },
    manifest::{file::read_runbooks_from_manifest, WorkspaceManifest},
    start_unsupervised_runbook_runloop,
    std::StdAddon,
    types::RunbookSnapshotContext,
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
pub const DEFAULT_ENVIRONMENT: &str = "localnet";
pub async fn execute_runbook(
    runbook_id: String,
    progress_tx: Sender<BlockEvent>,
    txtx_manifest_location: FileLocation,
) -> Result<(), String> {
    let manifest = WorkspaceManifest::from_location(&txtx_manifest_location)?;
    let runbook_selector = vec![runbook_id.to_string()];
    let mut runbooks = read_runbooks_from_manifest(
        &manifest,
        &Some(DEFAULT_ENVIRONMENT.into()),
        Some(&runbook_selector),
    )?;
    let top_level_inputs_map =
        manifest.get_runbook_inputs(&Some(DEFAULT_ENVIRONMENT.into()), &vec![], None)?;

    let Some((mut runbook, runbook_sources, _state, runbook_state_location)) =
        runbooks.swap_remove(&runbook_id)
    else {
        return Err(format!("Deployment {} not found", runbook_id));
    };

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

    if let Some(state_file_location) = runbook_state_location.clone() {
        match state_file_location.load_execution_snapshot(
            true,
            &runbook.runbook_id.name,
            &runbook.top_level_inputs_map.current_top_level_input_name(),
        ) {
            Ok(old_snapshot) => {
                let ctx = RunbookSnapshotContext::new();
                let execution_context_backups = runbook.backup_execution_contexts();
                let new = runbook.simulate_and_snapshot_flows(&old_snapshot).await?;
                let consolidated_changes = ctx.diff(old_snapshot, new);

                runbook.prepare_flows_for_new_plans(
                    &consolidated_changes.new_plans_to_add,
                    execution_context_backups,
                );

                let _ =
                    runbook.prepared_flows_for_updated_plans(&consolidated_changes.plans_to_update);
            }
            Err(e) => {
                println!("{} {}", red!("x"), e);
            }
        }
    }

    let res = start_unsupervised_runbook_runloop(&mut runbook, &progress_tx).await;
    if let Err(diags) = res {
        println!("{} Execution aborted", red!("x"));
        for diag in diags.iter() {
            println!("{}", format!("- {}", diag));
        }
        // write_runbook_transient_state(&mut runbook, runbook_state)?;
        return Ok(());
    }

    let _ = runbook.write_runbook_state(runbook_state_location)?;

    Ok(())
}
