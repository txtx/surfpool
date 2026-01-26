use std::{fs, str::FromStr};

use log::info;
use solana_signature::Signature;
use surfpool_core::surfnet::{remote::SurfnetRemoteClient, svm::SurfnetSvm};
use surfpool_types::{channel, ReplayConfig, ReplayResult, SimnetCommand};

use super::{Context, ReplayCommand};

/// Handles the replay command by fetching and re-executing transactions from mainnet.
///
/// This is a lightweight standalone handler that initializes an ephemeral SVM
/// without a full RPC server. For interactive use with full surfpool features,
/// use `surfpool start` with the `surfnet_replayTransaction` RPC method instead.
pub async fn handle_replay_command(cmd: ReplayCommand, _ctx: &Context) -> Result<(), String> {
    // Step 1: Parse signatures from args or file
    let signatures = if let Some(file_path) = &cmd.from_file {
        parse_signatures_from_file(file_path)?
    } else if cmd.signatures.is_empty() {
        return Err(
            "No transaction signatures provided. Use positional args or --from-file".to_string(),
        );
    } else {
        cmd.signatures
            .iter()
            .map(|s| Signature::from_str(s))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("Invalid signature: {}", e))?
    };

    if signatures.is_empty() {
        return Err("No transaction signatures provided".to_string());
    }

    // Step 2: Determine RPC URL
    let rpc_url = cmd.datasource_rpc_url();
    println!("Fetching transactions from: {}", rpc_url);

    // Step 3: Initialize ephemeral SVM (no persistence, no RPC server overhead)
    let (surfnet_svm, _simnet_events_rx, _geyser_events_rx) =
        SurfnetSvm::new_with_db(None, "replay")
            .map_err(|e| format!("Failed to initialize SVM: {}", e))?;

    // Step 4: Create command channel (needed for time travel coordination)
    let (simnet_commands_tx, _simnet_commands_rx): (
        channel::Sender<SimnetCommand>,
        channel::Receiver<SimnetCommand>,
    ) = channel::unbounded();

    let svm_locker = surfpool_core::surfnet::locker::SurfnetSvmLocker::new(surfnet_svm);

    // Step 5: Create remote client for mainnet data fetching
    let remote_client = SurfnetRemoteClient::new(&rpc_url);

    // Step 6: Build replay config from CLI flags
    let replay_config = ReplayConfig {
        profile: Some(cmd.profile),
        time_travel: Some(!cmd.skip_time_travel),
    };

    // Step 7: Replay each transaction
    let mut results: Vec<ReplayResult> = Vec::new();
    let total = signatures.len();

    for (i, signature) in signatures.iter().enumerate() {
        println!(
            "\n[{}/{}] Replaying: {}",
            i + 1,
            total,
            signature
        );

        match svm_locker
            .replay_transaction(
                &remote_client,
                *signature,
                replay_config.clone(),
                simnet_commands_tx.clone(),
            )
            .await
        {
            Ok(result) => {
                print_replay_result(&result);
                results.push(result);
            }
            Err(e) => {
                eprintln!("  Error: {}", e);
                // Continue with other transactions
            }
        }
    }

    // Step 8: Output results to file if requested
    if let Some(output_path) = &cmd.output {
        let json = serde_json::to_string_pretty(&results)
            .map_err(|e| format!("Failed to serialize results: {}", e))?;
        fs::write(output_path, json)
            .map_err(|e| format!("Failed to write output file: {}", e))?;
        info!("Results written to {}", output_path);
        println!("\nResults written to: {}", output_path);
    }

    // Summary
    let successful = results.iter().filter(|r| r.success).count();
    let failed = results.len() - successful;
    println!(
        "\n=== Summary ===\nTotal: {} | Success: {} | Failed: {}",
        total, successful, failed
    );

    Ok(())
}

/// Parses transaction signatures from a file (one per line).
fn parse_signatures_from_file(file_path: &str) -> Result<Vec<Signature>, String> {
    let content =
        fs::read_to_string(file_path).map_err(|e| format!("Failed to read file: {}", e))?;

    content
        .lines()
        .filter(|line| !line.trim().is_empty() && !line.trim().starts_with('#'))
        .map(|line| {
            Signature::from_str(line.trim())
                .map_err(|e| format!("Invalid signature '{}': {}", line.trim(), e))
        })
        .collect()
}

/// Prints a human-readable summary of a replay result.
fn print_replay_result(result: &ReplayResult) {
    println!("\n--- Transaction {} ---", result.signature);
    println!(
        "Status: {}",
        if result.success { "SUCCESS" } else { "FAILED" }
    );
    println!(
        "Original slot: {} | Replay slot: {}",
        result.original_slot, result.replay_slot
    );

    if let Some(block_time) = result.original_block_time {
        if let Some(dt) = chrono::DateTime::from_timestamp(block_time, 0) {
            println!("Original time: {}", dt.format("%Y-%m-%d %H:%M:%S UTC"));
        }
    }

    println!("Compute units: {}", result.compute_units_consumed);

    if let Some(ref error) = result.error {
        println!("Error: {}", error);
    }

    if !result.logs.is_empty() {
        println!("Logs ({}):", result.logs.len());
        for log in result.logs.iter().take(10) {
            println!("  {}", log);
        }
        if result.logs.len() > 10 {
            println!("  ... and {} more", result.logs.len() - 10);
        }
    }

    if let Some(ref warning) = result.state_warning {
        println!("Warning: {}", warning);
    }
}
