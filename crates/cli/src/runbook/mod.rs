use dialoguer::{console::Style, theme::ColorfulTheme, Confirm};
use surfpool_types::SimnetEvent;
use txtx_addon_network_svm::SvmNetworkAddon;
use txtx_core::{
    kit::{
        channel::Sender,
        helpers::fs::FileLocation,
        types::{frontend::BlockEvent, AuthorizationContext},
        Addon,
    },
    manifest::{file::read_runbooks_from_manifest, WorkspaceManifest},
    runbook::{ConsolidatedChanges, SynthesizedChange},
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

pub async fn execute_runbook(
    runbook_id: String,
    progress_tx: Sender<BlockEvent>,
    txtx_manifest_location: FileLocation,
    simnet_events_tx: crossbeam::channel::Sender<SimnetEvent>,
) -> Result<(), String> {
    let manifest = WorkspaceManifest::from_location(&txtx_manifest_location)?;
    let runbook_selector = vec![runbook_id.to_string()];
    let mut runbooks =
        read_runbooks_from_manifest(&manifest, &Some("localnet".into()), Some(&runbook_selector))?;
    let top_level_inputs_map =
        manifest.get_runbook_inputs(&Some("localnet".into()), &vec![], None)?;

    let Some((mut runbook, runbook_sources, _, runbook_state_location)) =
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
        let _ = simnet_events_tx.send(SimnetEvent::warn(format!("{:?}", diags)));
    }

    runbook.enable_full_execution_mode();

    let previous_state_opt = if let Some(state_file_location) = runbook_state_location.clone() {
        match state_file_location.load_execution_snapshot(
            true,
            &runbook.runbook_id.name,
            &runbook.top_level_inputs_map.current_top_level_input_name(),
        ) {
            Ok(snapshot) => Some(snapshot),
            Err(e) => {
                let _ = simnet_events_tx.send(SimnetEvent::warn(format!("{:?}", e)));
                None
            }
        }
    } else {
        None
    };

    if let Some(old) = previous_state_opt {
        let ctx = RunbookSnapshotContext::new();

        let execution_context_backups = runbook.backup_execution_contexts();
        let new = runbook.simulate_and_snapshot_flows(&old).await?;

        for flow_context in runbook.flow_contexts.iter() {
            if old.flows.get(&flow_context.name).is_none() {
                let _ = simnet_events_tx.send(SimnetEvent::info(format!(
                    "Previous snapshot not found for flow {}",
                    flow_context.name
                )));
            };
        }

        let consolidated_changes = ctx.diff(old, new);

        let Some(consolidated_changes) = display_snapshot_diffing(consolidated_changes) else {
            return Ok(());
        };

        runbook.prepare_flows_for_new_plans(
            &consolidated_changes.new_plans_to_add,
            execution_context_backups,
        );

        let (actions_to_re_execute, actions_to_execute) =
            runbook.prepared_flows_for_updated_plans(&consolidated_changes.plans_to_update);

        let has_actions = actions_to_re_execute
            .iter()
            .filter(|(_, actions)| !actions.is_empty())
            .count();
        if has_actions > 0 {
            let _ = simnet_events_tx.send(SimnetEvent::info(
                "The following actions will be re-executed:",
            ));
            for (context, actions) in actions_to_re_execute.iter() {
                let documentation_missing = black!("<description field empty>");
                let _ = simnet_events_tx.send(SimnetEvent::info(format!("{}", context)));
                for (action_name, documentation) in actions.into_iter() {
                    let _ = simnet_events_tx.send(SimnetEvent::info(format!(
                        "- {}: {}",
                        action_name,
                        documentation.as_ref().unwrap_or(&documentation_missing)
                    )));
                }
            }
        }

        let has_actions = actions_to_execute
            .iter()
            .filter(|(_, actions)| !actions.is_empty())
            .count();
        if has_actions > 0 {
            println!(
                "The following actions have been added and will be executed for the first time:"
            );
            for (context, actions) in actions_to_execute.iter() {
                let documentation_missing = black!("<description field empty>");
                println!("\n{}", green!(format!("{}", context)));
                for (action_name, documentation) in actions.into_iter() {
                    println!(
                        "- {}: {}",
                        action_name,
                        documentation.as_ref().unwrap_or(&documentation_missing)
                    );
                }
            }
            println!("\n");
        }

        let theme = ColorfulTheme {
            values_style: Style::new().green(),
            ..ColorfulTheme::default()
        };

        let confirm = Confirm::with_theme(&theme)
            .with_prompt("Do you want to continue?")
            .interact()
            .unwrap();

        if !confirm {
            return Ok(());
        }
    }

    let res = start_unsupervised_runbook_runloop(&mut runbook, &progress_tx).await;
    if let Err(diags) = res {
        let _ = simnet_events_tx.send(SimnetEvent::warn("Runbook execution aborted"));
        for diag in diags.iter() {
            let _ = simnet_events_tx.send(SimnetEvent::warn(format!("{}", diag)));
        }
        // write_runbook_transient_state(&mut runbook, runbook_state)?;
        return Ok(());
    }

    match runbook.write_runbook_state(runbook_state_location) {
        Ok(Some(location)) => {
            let _ = simnet_events_tx.send(SimnetEvent::info(format!(
                "Saved execution state to {}",
                location
            )));
        }
        Ok(None) => {}
        Err(e) => {
            let _ = simnet_events_tx.send(SimnetEvent::warn(format!(
                "Failed to write runbook state: {}",
                e
            )));
        }
    };

    Ok(())
}

pub fn display_snapshot_diffing(
    consolidated_changes: ConsolidatedChanges,
) -> Option<ConsolidatedChanges> {
    let synthesized_changes = consolidated_changes.get_synthesized_changes();

    if synthesized_changes.is_empty() && consolidated_changes.new_plans_to_add.is_empty() {
        println!(
            "{} Latest snapshot in sync with latest runbook updates\n",
            green!("✓")
        );
        return None;
    }

    if !consolidated_changes.new_plans_to_add.is_empty() {
        println!("\n{}", yellow!("New chain to synchronize:"));
        println!("{}\n", consolidated_changes.new_plans_to_add.join(", "));
    }

    let has_critical_changes = synthesized_changes
        .iter()
        .filter(|(c, _)| match c {
            SynthesizedChange::Edition(_, _) => true,
            SynthesizedChange::FormerFailure(_, _) => false,
            SynthesizedChange::Addition(_) => false,
        })
        .count();
    if has_critical_changes > 0 {
        println!("\n{}\n", yellow!("Changes detected:"));
        for (i, (change, _impacted)) in synthesized_changes.iter().enumerate() {
            match change {
                SynthesizedChange::Edition(change, _) => {
                    let formatted_change = change
                        .iter()
                        .map(|c| {
                            if c.starts_with("-") {
                                red!(c)
                            } else {
                                green!(c)
                            }
                        })
                        .collect::<Vec<_>>()
                        .join("");
                    println!("{}. The following edits:\n-------------------------\n{}\n-------------------------", i + 1, formatted_change);
                    println!("will introduce breaking changes.\n\n");
                }
                SynthesizedChange::FormerFailure(_construct_to_run, command_name) => {
                    println!("{}. The action error:\n-------------------------\n{}\n-------------------------", i + 1, command_name);
                    println!("will be re-executed.\n\n");
                }
                SynthesizedChange::Addition(_new_construct_did) => {}
            }
        }
    }

    let unexecuted = synthesized_changes
        .iter()
        .filter(|(c, _)| match c {
            SynthesizedChange::Edition(_, _) => false,
            SynthesizedChange::FormerFailure(_, _) => true,
            SynthesizedChange::Addition(_) => false,
        })
        .count();
    if unexecuted > 0 {
        println!("\n{}", yellow!("Runbook Recovery Plan"));
        println!("The previous runbook execution was interrupted before completion, causing the following actions to be aborted:");

        for (_i, (change, _impacted)) in synthesized_changes.iter().enumerate() {
            match change {
                SynthesizedChange::Edition(_, _) => {}
                SynthesizedChange::FormerFailure(_construct_to_run, command_name) => {
                    println!("- {}", command_name);
                }
                SynthesizedChange::Addition(_new_construct_did) => {}
            }
        }
        println!("These actions will be re-executed in the next run.\n");
    }

    Some(consolidated_changes)
}
