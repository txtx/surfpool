use agave_geyser_plugin_interface::geyser_plugin_interface::{
    GeyserPlugin, ReplicaTransactionInfoV2, ReplicaTransactionInfoVersions,
};
use blake3::Hash;
use chrono::Utc;
use crossbeam::select;
use crossbeam_channel::{unbounded, Receiver, Sender};
use ipc_channel::{
    ipc::{IpcOneShotServer, IpcReceiver},
    router::RouterProxy,
};
use jsonrpc_core::MetaIoHandler;
use jsonrpc_http_server::{DomainsValidation, ServerBuilder};
use litesvm::{types::TransactionResult, LiteSVM};
use solana_account::Account;
use solana_client::{nonblocking::rpc_client::RpcClient, rpc_response::RpcPerfSample};
use solana_clock::{Clock, Slot};
use solana_epoch_info::EpochInfo;
use solana_feature_set::{disable_new_loader_v3_deployments, FeatureSet};
use solana_keypair::Keypair;
use solana_message::{v0::LoadedAddresses, Message, SimpleAddressLoader, VersionedMessage};
use solana_pubkey::Pubkey;
use solana_sdk::{
    bpf_loader_upgradeable::get_program_data_address,
    system_instruction,
    transaction::{MessageHash, VersionedTransaction},
};
use solana_signature::Signature;
use solana_signer::Signer;
use solana_transaction::sanitized::SanitizedTransaction;
use solana_transaction_status::{
    EncodedConfirmedTransactionWithStatusMeta, InnerInstruction, InnerInstructions,
    TransactionStatus, TransactionStatusMeta, UiTransactionEncoding,
};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    net::SocketAddr,
    sync::{Arc, RwLockWriteGuard},
    thread::{sleep, JoinHandle},
    time::{Duration, Instant},
};
use surfpool_subgraph::SurfpoolSubgraphPlugin;
use tokio::sync::RwLock;

use crate::{
    rpc::{
        self, accounts_data::AccountsData, accounts_scan::AccountsScan, admin::AdminRpc,
        bank_data::BankData, full::Full, minimal::Minimal, svm_tricks::SvmTricksRpc,
        utils::convert_transaction_metadata_from_canonical, SurfpoolMiddleware,
    },
    types::{SurfnetTransactionStatus, TransactionWithStatusMeta},
    PluginManagerCommand,
};
use surfpool_types::{
    BlockProductionMode, ClockCommand, ClockEvent, SchemaDataSourcingEvent, SubgraphPluginConfig,
    TransactionConfirmationStatus, TransactionMetadata, TransactionStatusEvent,
};
use surfpool_types::{SimnetCommand, SimnetEvent, SubgraphCommand, SurfpoolConfig};

const BLOCKHASH_SLOT_TTL: u64 = 75;

// #[cfg(clippy)]
// const SUBGRAPH_PLUGIN_BYTES: &[u8] = &[0];

// #[cfg(not(clippy))]
// const SUBGRAPH_PLUGIN_BYTES: &[u8] =
//     include_bytes!("../../../../target/release/libsurfpool_subgraph.dylib");

pub struct SurfnetSvm {
    pub svm: LiteSVM,
    pub transactions: HashMap<Signature, SurfnetTransactionStatus>,
    pub perf_samples: VecDeque<RpcPerfSample>,
    pub transactions_processed: u64,
    pub simnet_events_tx: Sender<SimnetEvent>,
    pub connection: SurfnetDataConnection,
    pub latest_epoch_info: EpochInfo,
}

pub enum SurfnetDataConnection {
    Local,
    Connected(String, EpochInfo),
}

impl SurfnetSvm {
    pub fn new() -> (Self, Receiver<SimnetEvent>) {
        let (simnet_events_tx, simnet_events_rx) = crossbeam_channel::bounded(1024);
        let mut feature_set = FeatureSet::all_enabled();
        // v2.2 of the solana_sdk deprecates the v3 loader, and enables the v4 loader by default.
        // In order to keep the v3 deployments enabled, we need to remove the
        // `disable_new_loader_v3_deployments` feature from the active set, and add it to the inactive set.
        let _ = feature_set
            .active
            .remove(&disable_new_loader_v3_deployments::id());
        feature_set
            .inactive
            .insert(disable_new_loader_v3_deployments::id());

        let svm = LiteSVM::new().with_feature_set(feature_set);

        (
            Self {
                svm,
                transactions: HashMap::new(),
                perf_samples: VecDeque::new(),
                transactions_processed: 0,
                simnet_events_tx,
                connection: SurfnetDataConnection::Local,
                latest_epoch_info: EpochInfo {
                    epoch: 0,
                    slot_index: 0,
                    slots_in_epoch: 0,
                    absolute_slot: 0,
                    block_height: 0,
                    transaction_count: None,
                },
            },
            simnet_events_rx,
        )
    }

    pub async fn connect(
        &mut self,
        rpc_url: &str,
    ) -> Result<EpochInfo, Box<dyn std::error::Error>> {
        let rpc_client = RpcClient::new(rpc_url.to_string());
        let epoch_info = rpc_client.get_epoch_info().await?;
        self.connection = SurfnetDataConnection::Connected(rpc_url.to_string(), epoch_info.clone());

        let _ = self
            .simnet_events_tx
            .send(SimnetEvent::Connected(rpc_url.to_string()));
        let _ = self
            .simnet_events_tx
            .send(SimnetEvent::EpochInfoUpdate(epoch_info.clone()));
        Ok(epoch_info)
    }

    pub fn airdrop_pubkeys(&mut self, lamports: u64, addresses: &Vec<Pubkey>) {
        for recipient in addresses.iter() {
            let _ = self.airdrop(&recipient, lamports);
            let _ = self.simnet_events_tx.send(SimnetEvent::info(format!(
                "Genesis airdrop successful {}: {}",
                recipient.to_string(),
                lamports
            )));
        }
    }

    pub fn airdrop(&mut self, pubkey: &Pubkey, lamports: u64) -> TransactionResult {
        let res = self.svm.airdrop(pubkey, lamports);
        if let Ok(ref tx_result) = res {
            let airdrop_keypair = Keypair::new();
            let slot = self.latest_epoch_info.absolute_slot;
            self.transactions.insert(
                tx_result.signature,
                SurfnetTransactionStatus::Processed(TransactionWithStatusMeta(
                    slot,
                    VersionedTransaction::try_new(
                        VersionedMessage::Legacy(Message::new(
                            &[system_instruction::transfer(
                                &airdrop_keypair.pubkey(),
                                &pubkey,
                                lamports,
                            )],
                            Some(&airdrop_keypair.pubkey()),
                        )),
                        &[airdrop_keypair],
                    )
                    .unwrap(),
                    convert_transaction_metadata_from_canonical(&tx_result),
                    None,
                )),
            );
        }
        res
    }

    pub fn expected_rpc_client(&self) -> RpcClient {
        match &self.connection {
            SurfnetDataConnection::Local => unreachable!(),
            SurfnetDataConnection::Connected(rpc_url, _) => {
                let rpc_client = RpcClient::new(rpc_url.to_string());
                rpc_client
            }
        }
    }

    pub async fn get_account_mut(
        &mut self,
        pubkey: &Pubkey,
        strategy: GetAccountStrategy,
    ) -> Result<Option<Account>, Box<dyn std::error::Error>> {
        let (result, factory) = match strategy {
            GetAccountStrategy::LocalOrDefault(factory) => (self.svm.get_account(pubkey), factory),
            GetAccountStrategy::ConnectionOrDefault(factory) => {
                let client = self.expected_rpc_client();
                let account = client.get_account(&pubkey).await?;
                self.svm.set_account(pubkey.clone(), account.clone());
                (Some(account), factory)
            }
            GetAccountStrategy::LocalThenConnectionOrDefault(factory) => {
                match self.svm.get_account(pubkey) {
                    Some(entry) => (Some(entry), factory),
                    None => {
                        let client = self.expected_rpc_client();
                        let account = client.get_account(&pubkey).await?;
                        self.svm.set_account(pubkey.clone(), account.clone());

                        if account.executable {
                            let program_data_address = get_program_data_address(pubkey);
                            let res = self
                                .get_account(
                                    &program_data_address,
                                    GetAccountStrategy::ConnectionOrDefault(None),
                                )
                                .await?;
                            if let Some(program_data) = res {
                                self.svm.set_account(program_data_address, program_data);
                            }
                        }
                        (Some(account), factory)
                    }
                }
            }
        };
        let account = match (result, factory) {
            (None, Some(factory)) => Some(factory(self)),
            (None, None) => None,
            (Some(account), _) => Some(account),
        };
        Ok(account)
    }

    pub async fn set_account(
        &mut self,
        pubkey: &Pubkey,
        account: Account,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.svm.set_account(pubkey.clone(), account);
        Ok(())
    }

    pub fn get_latest_absolute_slot(&self) -> Slot {
        self.latest_epoch_info.absolute_slot
    }

    pub fn latest_blockhash(&self) -> solana_hash::Hash {
        self.svm.latest_blockhash()
    }

    pub async fn get_multiple_accounts_mut(
        &mut self,
        pubkeys: &Vec<Pubkey>,
        strategy: GetAccountStrategy,
    ) -> Result<Vec<Option<Account>>, Box<dyn std::error::Error>> {
        match strategy {
            GetAccountStrategy::LocalOrDefault(_) => {
                let mut accounts = vec![];
                for pubkey in pubkeys.iter() {
                    let account = self.svm.get_account(pubkey);
                    accounts.push(account);
                }
                Ok(accounts)
            }
            GetAccountStrategy::ConnectionOrDefault(_) => {
                let client = self.expected_rpc_client();
                let entry = client.get_multiple_accounts(pubkeys).await?;
                Ok(entry)
            }
            GetAccountStrategy::LocalThenConnectionOrDefault(_) => {
                // Retrieve accounts missing locally
                let mut missing_accounts = Vec::new();
                let mut fetched_accounts = HashMap::new();
                for pubkey in pubkeys.iter() {
                    match self.svm.get_account(pubkey) {
                        Some(entry) => {
                            fetched_accounts.insert(pubkey.clone(), entry.clone());
                        }
                        None => {
                            missing_accounts.push(pubkey.clone());
                        }
                    };
                }

                if missing_accounts.is_empty() {
                    let mut accounts = vec![];
                    for (_, account) in fetched_accounts.into_iter() {
                        accounts.push(Some(account));
                    }
                    return Ok(accounts);
                }

                let client = self.expected_rpc_client();
                let remote_accounts = client.get_multiple_accounts(&missing_accounts).await?;
                for (pubkey, remote_account) in missing_accounts.into_iter().zip(remote_accounts) {
                    if let Some(remote_account) = remote_account {
                        self.svm.set_account(pubkey, remote_account);
                    }
                }

                let mut accounts = vec![];
                for pubkey in pubkeys.iter() {
                    let account = self.svm.get_account(pubkey);
                    accounts.push(account);
                }
                Ok(accounts)
            }
        }
    }

    pub fn send_transaction(&mut self, tx: impl Into<VersionedTransaction>) -> TransactionResult {
        self.transactions_processed += 1;
        self.svm.send_transaction(tx)
    }

    pub async fn get_account(
        &self,
        pubkey: &Pubkey,
        strategy: GetAccountStrategy,
    ) -> Result<Option<Account>, Box<dyn std::error::Error>> {
        let (result, factory) = match strategy {
            GetAccountStrategy::LocalOrDefault(factory) => (self.svm.get_account(pubkey), factory),
            GetAccountStrategy::ConnectionOrDefault(factory) => {
                let client = self.expected_rpc_client();
                let entry = client.get_account(pubkey).await?;
                (Some(entry), factory)
            }
            GetAccountStrategy::LocalThenConnectionOrDefault(factory) => {
                match self.svm.get_account(pubkey) {
                    Some(entry) => (Some(entry), factory),
                    None => {
                        let client = self.expected_rpc_client();
                        let entry = client.get_account(pubkey).await?;
                        (Some(entry), factory)
                    }
                }
            }
        };
        let account = match (result, factory) {
            (None, Some(factory)) => Some(factory(self)),
            (None, None) => None,
            (Some(account), _) => Some(account),
        };
        Ok(account)
    }

    pub async fn get_transaction(
        &self,
        signature: &Signature,
        encoding: Option<UiTransactionEncoding>,
    ) -> Result<
        Option<(EncodedConfirmedTransactionWithStatusMeta, TransactionStatus)>,
        Box<dyn std::error::Error>,
    > {
        let mut tx = self
            .transactions
            .get(&signature)
            .map(|entry| entry.expect_processed().clone().into());

        if tx.is_none() {
            let client = self.expected_rpc_client();
            let entry = client
                .get_transaction(signature, encoding.unwrap_or(UiTransactionEncoding::Json))
                .await?;
            tx = Some(entry);
        }

        let mut response = None;
        if let Some(tx) = tx {
            let status = TransactionStatus {
                slot: tx.slot,
                confirmations: Some((self.get_latest_absolute_slot() - tx.slot) as usize),
                status: tx.transaction.clone().meta.map_or(Ok(()), |m| m.status),
                err: tx.transaction.clone().meta.map(|m| m.err).flatten(),
                confirmation_status: Some(
                    solana_transaction_status::TransactionConfirmationStatus::Confirmed,
                ),
            };
            response = Some((tx, status));
        }
        Ok(response)
    }

    pub async fn tick(&mut self) {

        // iterate on transactions that has been processed
    }
}

pub type AccountFactory = Box<dyn Fn(&SurfnetSvm) -> Account + Send + Sync>;
// pub type AccountFactory = Box<dyn Fn() -> Account>;

pub enum GetAccountStrategy {
    LocalOrDefault(Option<AccountFactory>),
    ConnectionOrDefault(Option<AccountFactory>),
    LocalThenConnectionOrDefault(Option<AccountFactory>),
}

pub fn start_clock(mut slot_time: u64) -> (Receiver<ClockEvent>, Sender<ClockCommand>) {
    let (clock_event_tx, clock_event_rx) = unbounded::<ClockEvent>();
    let (clock_command_tx, clock_command_rx) = unbounded::<ClockCommand>();

    let _handle = hiro_system_kit::thread_named("clock").spawn(move || {
        let mut enabled = true;
        let mut block_hash_timeout = Instant::now();

        loop {
            match clock_command_rx.try_recv() {
                Ok(ClockCommand::Pause) => {
                    enabled = false;
                }
                Ok(ClockCommand::Resume) => {
                    enabled = true;
                }
                Ok(ClockCommand::Toggle) => {
                    enabled = !enabled;
                }
                Ok(ClockCommand::UpdateSlotInterval(updated_slot_time)) => {
                    slot_time = updated_slot_time;
                }
                Err(_e) => {}
            }
            sleep(Duration::from_millis(slot_time));
            if enabled {
                let _ = clock_event_tx.send(ClockEvent::Tick);
                // Todo: the block expiration is not completely accurate.
                if block_hash_timeout.elapsed()
                    > Duration::from_millis(BLOCKHASH_SLOT_TTL * slot_time)
                {
                    let _ = clock_event_tx.send(ClockEvent::ExpireBlockHash);
                    block_hash_timeout = Instant::now();
                }
            }
        }
    });

    let mut runloop_trigger_mode = simnet.runloop_trigger_mode.clone();
    let mut transactions_to_process = vec![];
    let mut num_transactions = 0;
    loop {
        let mut transactions_processed = vec![];
        let mut create_slot = false;

        select! {
            recv(clock_event_rx) -> msg => match msg {
                Ok(event) => {
                    match event {
                        ClockEvent::Tick => {
                            if runloop_trigger_mode.eq(&RunloopTriggerMode::Clock) {
                                create_slot = true;
                            }
                        }
                        ClockEvent::ExpireBlockHash => {
                            if let Ok(mut ctx) = context.write() {
                                ctx.svm.expire_blockhash();
                            }
                        }
                    }
                },
                Err(_) => {},
            },
            recv(simnet_commands_rx) -> msg => match msg {
                Ok(event) => {
                    match event {
                        SimnetCommand::SlotForward => {
                            runloop_trigger_mode = RunloopTriggerMode::Manual;
                            create_slot = true;
                        }
                        SimnetCommand::SlotBackward => {

                        }
                        SimnetCommand::UpdateClock(update) => {
                            let _ = clock_command_tx.send(update);
                            continue
                        }
                        SimnetCommand::UpdateRunloopMode(update) => {
                            runloop_trigger_mode = update;
                            continue
                        }
                        SimnetCommand::TransactionReceived(key, transaction, status_tx, skip_preflight) => {
                            let signature = transaction.signatures[0].clone();
                            transactions_to_process.push((key, transaction, status_tx, skip_preflight));
                            if let Ok(mut ctx) = context.write() {
                                ctx.transactions.insert(
                                    signature,
                                    EntryStatus::Received,
                                );
                            }
                        }
                    }
                },
                Err(_) => {},
            },
        }

        let unix_timestamp: i64 = Utc::now().timestamp();
        let Ok(mut ctx) = context.write() else {
            continue;
        };

        // Handle the accumulated transactions
        for (key, transaction, status_tx, skip_preflight) in transactions_to_process.drain(..) {
            // verify valid signatures on the transaction
            {
                if transaction
                    .verify_with_results()
                    .iter()
                    .any(|valid| !*valid)
                {
                    let _ = simnet_events_tx.try_send(SimnetEvent::error(format!(
                        "Transaction verification failed: {}",
                        transaction.signatures[0]
                    )));
                    continue;
                }
            }

            // find accounts that are needed for this transaction but are missing from the local
            // svm cached, fetch them from the RPC, and insert them locally
            {
                let accounts = match &transaction.message {
                    VersionedMessage::Legacy(message) => message.account_keys.clone(),
                    VersionedMessage::V0(message) => message.account_keys.clone(),
                };
                for account_pubkey in accounts.iter() {
                    match insert_account_from_remote_if_not_in_local(
                        &mut ctx,
                        account_pubkey,
                        &rpc_client,
                    )
                    .await
                    {
                        Ok(pubkeys) => {
                            for pubkey in pubkeys {
                                let _ =
                                    simnet_events_tx.try_send(SimnetEvent::account_update(pubkey));
                            }
                        }
                        Err(event) => {
                            let _ = simnet_events_tx.try_send(event);
                        }
                    }
                }
            }

            // if not skipping preflight, simulate the transaction
            if !skip_preflight {
                let (meta, err) = match ctx.svm.simulate_transaction(transaction.clone()) {
                    Ok(res) => {
                        let transaction_meta =
                            convert_transaction_metadata_from_canonical(&res.meta);
                        let _ = plugins_data_tx.send((
                            transaction.clone(),
                            transaction_meta.clone(),
                            ctx.epoch_info.absolute_slot,
                        ));
                        (transaction_meta, None)
                    }
                    Err(res) => {
                        let _ = simnet_events_tx.try_send(SimnetEvent::error(format!(
                            "Transaction simulation failed: {}",
                            res.err.to_string()
                        )));
                        (
                            convert_transaction_metadata_from_canonical(&res.meta),
                            Some(res.err),
                        )
                    }
                };

                if let Some(e) = &err {
                    let _ = status_tx
                        .try_send(TransactionStatusEvent::SimulationFailure((e.clone(), meta)));
                    continue;
                }
            }

            // send the transaction to the SVM
            let (meta, err) = match ctx.svm.send_transaction(transaction.clone()) {
                Ok(res) => (convert_transaction_metadata_from_canonical(&res), None),
                Err(res) => {
                    let _ = simnet_events_tx.try_send(SimnetEvent::error(format!(
                        "Transaction execution failed: {}",
                        res.err.to_string()
                    )));
                    (
                        convert_transaction_metadata_from_canonical(&res.meta),
                        Some(res.err),
                    )
                }
            };

            // send the transaction status events and mark this transaction as processed
            {
                let slot = ctx.epoch_info.absolute_slot;
                ctx.transactions.insert(
                    transaction.signatures[0],
                    EntryStatus::Processed(TransactionWithStatusMeta(
                        slot,
                        transaction.clone(),
                        meta.clone(),
                        err.clone(),
                    )),
                );
                if let Some(e) = &err {
                    let _ = status_tx
                        .try_send(TransactionStatusEvent::ExecutionFailure((e.clone(), meta)));
                } else {
                    let _ = status_tx.try_send(TransactionStatusEvent::Success(
                        TransactionConfirmationStatus::Processed,
                    ));
                    let _ =
                        simnet_events_tx.try_send(SimnetEvent::transaction_processed(meta, err));
                }
                transactions_processed.push((key, transaction, status_tx));
                num_transactions += 1;
            }
        }

        if !create_slot {
            continue;
        }

        for (_key, _transaction, status_tx) in transactions_processed.iter() {
            let _ = status_tx.try_send(TransactionStatusEvent::Success(
                TransactionConfirmationStatus::Confirmed,
            ));
        }

        ctx.epoch_info.slot_index += 1;
        ctx.epoch_info.absolute_slot += 1;
        let slot = ctx.epoch_info.slot_index;

        if ctx.perf_samples.len() > 30 {
            ctx.perf_samples.pop_back();
        }
        ctx.perf_samples.push_front(RpcPerfSample {
            slot,
            num_slots: 1,
            sample_period_secs: 1,
            num_transactions,
            num_non_vote_transactions: None,
        });
        num_transactions = 0;

        if ctx.epoch_info.slot_index > slots_in_epoch {
            ctx.epoch_info.slot_index = 0;
            ctx.epoch_info.epoch += 1;
        }
        let clock: Clock = Clock {
            slot: ctx.epoch_info.slot_index,
            epoch: ctx.epoch_info.epoch,
            unix_timestamp,
            epoch_start_timestamp: 0, // todo
            leader_schedule_epoch: 0, // todo
        };
        let _ = simnet_events_tx.send(SimnetEvent::ClockUpdate(clock.clone()));
        ctx.svm.set_sysvar(&clock);
    }
}

fn start_geyser_plugin_thread(
    plugin_manager_commands_rx: Receiver<PluginManagerCommand>,
    subgraph_commands_tx: Sender<SubgraphCommand>,
    simnet_events_tx: Sender<SimnetEvent>,
    plugins_data_rx: Receiver<(VersionedTransaction, TransactionMetadata, Slot)>,
) -> Result<JoinHandle<Result<(), String>>, String> {
    let handle = hiro_system_kit::thread_named("Geyser Plugins Handler").spawn(move || {
        let mut plugin_manager = vec![];

        let ipc_router = RouterProxy::new();
        // Note:
        // At the moment, surfpool-subgraph is the only plugin that we're mounting.
        // Please open an issue http://github.com/txtx/surfpool/issues/new if this is a feature you need!
        //
        // Proof of concept:
        //
        // let geyser_plugin_config_file = PathBuf::from("../../surfpool_subgraph_plugin.json");
        // let contents = "{\"name\": \"surfpool-subgraph\", \"libpath\": \"target/release/libsurfpool_subgraph.dylib\"}";
        // let result: serde_json::Value = json5::from_str(&contents).unwrap();
        // let libpath = result["libpath"]
        //     .as_str()
        //     .unwrap();
        // let mut libpath = PathBuf::from(libpath);
        // if libpath.is_relative() {
        //     let config_dir = geyser_plugin_config_file.parent().ok_or_else(|| {
        //         GeyserPluginManagerError::CannotOpenConfigFile(format!(
        //             "Failed to resolve parent of {geyser_plugin_config_file:?}",
        //         ))
        //     }).unwrap();
        //     libpath = config_dir.join(libpath);
        // }
        // let plugin_name = result["name"].as_str().map(|s| s.to_owned()).unwrap_or(format!("surfpool-subgraph"));
        // let (plugin, lib) = unsafe {
        //     let lib = match Library::new(&surfpool_subgraph_path) {
        //         Ok(lib) => lib,
        //         Err(e) => {
        //             let _ = simnet_events_tx_copy.send(SimnetEvent::ErrorLog(Local::now(), format!("Unable to load plugin {}: {}", plugin_name, e.to_string())));
        //             continue;
        //         }
        //     };
        //     let constructor: Symbol<PluginConstructor> = lib
        //         .get(b"_create_plugin")
        //         .map_err(|e| format!("{}", e.to_string()))?;
        //     let plugin_raw = constructor();
        //     (Box::from_raw(plugin_raw), lib)
        // };

        let err = loop {
            select! {
                recv(plugin_manager_commands_rx) -> msg => {
                    match msg {
                        Ok(event) => {
                            match event {
                                PluginManagerCommand::LoadConfig(uuid, config, notifier) => {
                                    let _ = subgraph_commands_tx.send(SubgraphCommand::CreateSubgraph(uuid.clone(), config.data.clone(), notifier));
                                    let mut plugin = SurfpoolSubgraphPlugin::default();

                                    let (server, ipc_token) = IpcOneShotServer::<IpcReceiver<SchemaDataSourcingEvent>>::new().expect("Failed to create IPC one-shot server.");
                                    let subgraph_plugin_config = SubgraphPluginConfig {
                                        uuid,
                                        ipc_token,
                                        subgraph_request: config.data.clone()
                                    };

                                    let config_file = match serde_json::to_string(&subgraph_plugin_config) {
                                        Ok(c) => c,
                                        Err(e) => {
                                            let _ = simnet_events_tx.send(SimnetEvent::error(format!("Failed to serialize subgraph plugin config: {:?}", e)));
                                            continue;
                                        }
                                    };

                                    if let Err(e) = plugin.on_load(&config_file, false) {
                                        let _ = simnet_events_tx.send(SimnetEvent::error(format!("Failed to load Geyser plugin: {:?}", e)));
                                    };
                                    if let Ok((_, rx)) = server.accept() {
                                        let subgraph_rx = ipc_router.route_ipc_receiver_to_new_crossbeam_receiver::<SchemaDataSourcingEvent>(rx);
                                        let _ = subgraph_commands_tx.send(SubgraphCommand::ObserveSubgraph(subgraph_rx));
                                    };
                                    let plugin: Box<dyn GeyserPlugin> = Box::new(plugin);
                                    plugin_manager.push(plugin);
                                    let _ = simnet_events_tx.send(SimnetEvent::PluginLoaded("surfpool-subgraph".into()));
                                }
                            }
                        },
                        Err(e) => {
                            break format!("Failed to read plugin manager command: {:?}", e);
                        },
                    }
                },
                recv(plugins_data_rx) -> msg => match msg {
                    Err(e) => {
                        break format!("Failed to read new transaction to send to Geyser plugin: {e}");
                    },
                    Ok((transaction, transaction_metadata, slot)) => {
                        let mut inner_instructions = vec![];
                        for (i,inner) in transaction_metadata.inner_instructions.iter().enumerate() {
                            inner_instructions.push(
                                InnerInstructions {
                                    index: i as u8,
                                    instructions: inner.iter().map(|i| InnerInstruction {
                                        instruction: i.instruction.clone(),
                                        stack_height: Some(i.stack_height as u32)
                                    }).collect()
                                }
                            )
                        }

                        let transaction_status_meta = TransactionStatusMeta {
                            status: Ok(()),
                            fee: 0,
                            pre_balances: vec![],
                            post_balances: vec![],
                            inner_instructions: Some(inner_instructions),
                            log_messages: Some(transaction_metadata.logs.clone()),
                            pre_token_balances: None,
                            post_token_balances: None,
                            rewards: None,
                            loaded_addresses: LoadedAddresses {
                                writable: vec![],
                                readonly: vec![],
                            },
                            return_data: Some(transaction_metadata.return_data.clone()),
                            compute_units_consumed: Some(transaction_metadata.compute_units_consumed),
                        };

                        let transaction = match SanitizedTransaction::try_create(transaction, MessageHash::Compute, None, SimpleAddressLoader::Disabled, &HashSet::new()) {
                        Ok(tx) => tx,
                            Err(e) => {
                                let _ = simnet_events_tx.send(SimnetEvent::error(format!("Failed to notify Geyser plugin of new transaction: failed to serialize transaction: {:?}", e)));
                                continue;
                            }
                        };

                        let transaction_replica = ReplicaTransactionInfoV2 {
                            signature: &transaction_metadata.signature,
                            is_vote: false,
                            transaction: &transaction,
                            transaction_status_meta: &transaction_status_meta,
                            index: 0
                        };
                        for plugin in plugin_manager.iter() {
                            if let Err(e) = plugin.notify_transaction(ReplicaTransactionInfoVersions::V0_0_2(&transaction_replica), slot) {
                                let _ = simnet_events_tx.send(SimnetEvent::error(format!("Failed to notify Geyser plugin of new transaction: {:?}", e)));
                            };
                        }
                    }
                }
            }
        };
        Err(err)
    }).map_err(|e| format!("Failed to spawn Geyser Plugins Handler thread: {:?}", e))?;
    Ok(handle)
}

fn start_rpc_server_thread(
    config: &SurfpoolConfig,
    simnet_events_tx: &Sender<SimnetEvent>,
    simnet_commands_tx: &Sender<SimnetCommand>,
    context: Arc<RwLock<GlobalState>>,
    epoch_info: &EpochInfo,
) -> Result<(Receiver<PluginManagerCommand>, JoinHandle<()>), String> {
    let (plugin_manager_commands_tx, plugin_manager_commands_rx) = unbounded();

    let middleware = SurfpoolMiddleware::new(
        context,
        &simnet_commands_tx,
        &simnet_events_tx,
        &plugin_manager_commands_tx,
        &config.rpc,
    );
    let server_bind: SocketAddr = config
        .rpc
        .get_socket_address()
        .parse::<SocketAddr>()
        .map_err(|e| e.to_string())?;

    let simnet_events_tx = simnet_events_tx.clone();
    let epoch_info = epoch_info.clone();

    let mut io = MetaIoHandler::with_middleware(middleware);
    io.extend_with(rpc::minimal::SurfpoolMinimalRpc.to_delegate());
    io.extend_with(rpc::full::SurfpoolFullRpc.to_delegate());
    io.extend_with(rpc::accounts_data::SurfpoolAccountsDataRpc.to_delegate());
    io.extend_with(rpc::accounts_scan::SurfpoolAccountsScanRpc.to_delegate());
    io.extend_with(rpc::bank_data::SurfpoolBankDataRpc.to_delegate());
    io.extend_with(rpc::svm_tricks::SurfpoolSvmTricksRpc.to_delegate());
    io.extend_with(rpc::admin::SurfpoolAdminRpc.to_delegate());

    if !config.plugin_config_path.is_empty() {
        io.extend_with(rpc::admin::SurfpoolAdminRpc.to_delegate());
    }

    let _ = std::net::TcpListener::bind(server_bind)
        .map_err(|e| format!("Failed to start RPC server: {}", e))?;

    let _handle = hiro_system_kit::thread_named("RPC Handler")
        .spawn(move || {
            let server = match ServerBuilder::new(io)
                .cors(DomainsValidation::Disabled)
                .start_http(&server_bind)
            {
                Ok(server) => server,
                Err(e) => {
                    let _ = simnet_events_tx.send(SimnetEvent::Aborted(format!(
                        "Failed to start RPC server: {:?}",
                        e
                    )));
                    return;
                }
            };
            let _ = simnet_events_tx.send(SimnetEvent::Ready);
            let _ = simnet_events_tx.send(SimnetEvent::EpochInfoUpdate(epoch_info));

            server.wait();
            let _ = simnet_events_tx.send(SimnetEvent::Shutdown);
        })
        .map_err(|e| format!("Failed to spawn RPC Handler thread: {:?}", e))?;
    Ok((plugin_manager_commands_rx, _handle))
}

async fn insert_account_from_remote_if_not_in_local(
    ctx: &mut RwLockWriteGuard<'_, GlobalState>,
    account_pubkey: &Pubkey,
    rpc: &RpcClient,
) -> Result<Vec<Pubkey>, SimnetEvent> {
    let mut updated_pubkeys = vec![];

    if ctx.svm.get_account(&account_pubkey).is_none() {
        let res = rpc
            .get_account_with_commitment(&account_pubkey, CommitmentConfig::processed())
            .await
            .map_err(|e| {
                SimnetEvent::error(format!(
                    "unable to retrieve account {}: {}",
                    account_pubkey, e
                ))
            })?;
        if let Some(account) = res.value {
            if account.executable {
                let program_data_address = get_program_data_address(account_pubkey);

                if ctx.svm.get_account(&program_data_address).is_none() {
                    let res = rpc
                        .get_account_with_commitment(
                            &program_data_address,
                            CommitmentConfig::processed(),
                        )
                        .await
                        .map_err(|e| {
                            SimnetEvent::error(format!(
                                "unable to retrieve account {}: {}",
                                program_data_address, e
                            ))
                        })?;
                    if let Some(program_data_account) = res.value {
                        let _ = ctx
                            .svm
                            .set_account(program_data_address, program_data_account.clone())
                            .map_err(|e| {
                                SimnetEvent::error(format!(
                                    "unable to set account {}: {}",
                                    program_data_address, e
                                ))
                            })?;
                        updated_pubkeys.push(program_data_address);
                    }
                }
            }

            let _ = ctx
                .svm
                .set_account(*account_pubkey, account.clone())
                .map_err(|e| {
                    SimnetEvent::error(format!("unable to set account {}: {}", account_pubkey, e))
                })?;

            updated_pubkeys.push(*account_pubkey);
        }
    }
    Ok(updated_pubkeys)
}
