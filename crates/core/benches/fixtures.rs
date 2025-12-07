use crossbeam_channel::Receiver;
use solana_keypair::Keypair;
use solana_message::{Message, VersionedMessage};
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use solana_system_interface::instruction::transfer;
use solana_transaction::versioned::VersionedTransaction;
use surfpool_core::surfnet::locker::SurfnetSvmLocker;
use surfpool_types::SimnetEvent;

pub const SIMPLE_TRANSFER_LAMPORTS: u64 = 10_000_000_000;
pub const MULTI_TRANSFER_LAMPORTS: u64 = 50_000_000_000;
pub const TRANSFER_AMOUNT_PER_RECIPIENT: u64 = 1_000_000;

/// Waits for the runloop to be ready by listening for Ready and Connected events.
/// In offline mode (no RPC connection), Connected event may never arrive, so we
/// proceed once Ready is received and we've waited a reasonable time.
pub fn wait_for_ready(simnet_events_rx: &Receiver<SimnetEvent>) {
    let mut ready = false;
    let mut connected = false;
    let mut ready_time: Option<std::time::Instant> = None;
    const MAX_WAIT_FOR_CONNECTED_SECS: u64 = 3; // Wait max 3 seconds for Connected after Ready

    loop {
        // Use shorter timeout if we already have Ready (to detect offline mode faster)
        let timeout = if ready {
            std::time::Duration::from_secs(1)
        } else {
            std::time::Duration::from_secs(10)
        };

        match simnet_events_rx.recv_timeout(timeout) {
            Ok(SimnetEvent::Ready) => {
                ready = true;
                ready_time = Some(std::time::Instant::now());
            }
            Ok(SimnetEvent::Connected(_)) => {
                connected = true;
            }
            Ok(SimnetEvent::Aborted(msg)) => panic!("Runloop aborted: {msg}"),
            Ok(_) => {
                // Ignore other events (Shutdown, ClockUpdate, logs, etc.)
            }
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {
                // If we have Ready but no Connected after reasonable time, assume offline mode
                if !ready {
                    panic!("Timeout waiting for Ready event after {} seconds", timeout.as_secs());
                }
                // Fall through to check if we've waited long enough for Connected
            }
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                panic!("Channel disconnected while waiting for runloop");
            }
        }

        // Exit if we have both Ready and Connected
        if ready && connected {
            return;
        }

        // Exit if we have Ready and waited long enough (offline mode - Connected will never come)
        if ready_time
            .map(|ready_instant| ready_instant.elapsed().as_secs() >= MAX_WAIT_FOR_CONNECTED_SECS)
            .unwrap_or(false)
        {
            return;
        }
    }
}

/// Creates a transfer transaction with the specified number of instructions.
pub fn create_transfer_transaction(
    svm_locker: &SurfnetSvmLocker,
    num_instructions: usize,
    airdrop_amount: u64,
    transfer_amount: u64,
) -> String {
    let payer = Keypair::new();
    let recipients: Vec<Pubkey> = (0..num_instructions)
        .map(|_| Pubkey::new_unique())
        .collect();

    svm_locker.with_svm_writer(|svm| {
        svm.airdrop(&payer.pubkey(), airdrop_amount).unwrap();
    });

    let latest_blockhash = svm_locker.with_svm_reader(|svm| svm.latest_blockhash());

    let instructions: Vec<_> = recipients
        .iter()
        .map(|recipient| transfer(&payer.pubkey(), recipient, transfer_amount))
        .collect();

    let message =
        Message::new_with_blockhash(&instructions, Some(&payer.pubkey()), &latest_blockhash);
    let tx = VersionedTransaction::try_new(VersionedMessage::Legacy(message), &[&payer]).unwrap();

    bs58::encode(bincode::serialize(&tx).unwrap()).into_string()
}
