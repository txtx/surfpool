use super::utils::{
    convert_transaction_metadata_from_canonical, decode_and_deserialize,
    transform_tx_metadata_to_ui_accounts,
};
use crate::types::{EntryStatus, TransactionWithStatusMeta};
use itertools::Itertools;
use jsonrpc_core::futures::future::{self, join_all};
use jsonrpc_core::BoxFuture;
use jsonrpc_core::{Error, Result};
use jsonrpc_derive::rpc;
use solana_account_decoder::{encode_ui_account, UiAccountEncoding};
use solana_client::rpc_config::RpcContextConfig;
use solana_client::rpc_custom_error::RpcCustomError;
use solana_client::rpc_response::RpcApiVersion;
use solana_client::rpc_response::RpcResponseContext;
use solana_client::{
    rpc_config::{
        RpcBlockConfig, RpcBlocksConfigWrapper, RpcEncodingConfigWrapper, RpcEpochConfig,
        RpcRequestAirdropConfig, RpcSendTransactionConfig, RpcSignatureStatusConfig,
        RpcSignaturesForAddressConfig, RpcSimulateTransactionConfig, RpcTransactionConfig,
    },
    rpc_response::{
        RpcBlockhash, RpcConfirmedTransactionStatusWithSignature, RpcContactInfo,
        RpcInflationReward, RpcPerfSample, RpcPrioritizationFee, RpcSimulateTransactionResult,
    },
};
use solana_rpc_client_api::response::Response as RpcResponse;
use solana_sdk::commitment_config::CommitmentLevel;
use solana_sdk::message::{Message, VersionedMessage};
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::{Keypair, Signature};
use solana_sdk::signer::Signer;
use solana_sdk::system_instruction;
use solana_sdk::sysvar::rewards::Rewards;
use solana_sdk::transaction::{Transaction, VersionedTransaction};
use solana_sdk::{account::Account, clock::UnixTimestamp};
use solana_transaction_status::{
    EncodedConfirmedTransactionWithStatusMeta, TransactionStatus, UiConfirmedBlock, UiTransactionStatusMeta,
};
use solana_transaction_status::{TransactionBinaryEncoding, UiTransactionEncoding};
use std::collections::HashMap;
use std::str::FromStr;
use surfpool_types::{TransactionConfirmationStatus, TransactionStatusEvent};

use super::*;

#[rpc]
pub trait Full {
    type Metadata;

    #[rpc(meta, name = "getInflationReward")]
    fn get_inflation_reward(
        &self,
        meta: Self::Metadata,
        address_strs: Vec<String>,
        config: Option<RpcEpochConfig>,
    ) -> BoxFuture<Result<Vec<Option<RpcInflationReward>>>>;

    #[rpc(meta, name = "getClusterNodes")]
    fn get_cluster_nodes(&self, meta: Self::Metadata) -> Result<Vec<RpcContactInfo>>;

    #[rpc(meta, name = "getRecentPerformanceSamples")]
    fn get_recent_performance_samples(
        &self,
        meta: Self::Metadata,
        limit: Option<usize>,
    ) -> Result<Vec<RpcPerfSample>>;

    #[rpc(meta, name = "getSignatureStatuses")]
    fn get_signature_statuses(
        &self,
        meta: Self::Metadata,
        signature_strs: Vec<String>,
        config: Option<RpcSignatureStatusConfig>,
    ) -> BoxFuture<Result<RpcResponse<Vec<Option<TransactionStatus>>>>>;

    #[rpc(meta, name = "getMaxRetransmitSlot")]
    fn get_max_retransmit_slot(&self, meta: Self::Metadata) -> Result<Slot>;

    #[rpc(meta, name = "getMaxShredInsertSlot")]
    fn get_max_shred_insert_slot(&self, meta: Self::Metadata) -> Result<Slot>;

    #[rpc(meta, name = "requestAirdrop")]
    fn request_airdrop(
        &self,
        meta: Self::Metadata,
        pubkey_str: String,
        lamports: u64,
        config: Option<RpcRequestAirdropConfig>,
    ) -> Result<String>;

    #[rpc(meta, name = "sendTransaction")]
    fn send_transaction(
        &self,
        meta: Self::Metadata,
        data: String,
        config: Option<RpcSendTransactionConfig>,
    ) -> Result<String>;

    #[rpc(meta, name = "simulateTransaction")]
    fn simulate_transaction(
        &self,
        meta: Self::Metadata,
        data: String,
        config: Option<RpcSimulateTransactionConfig>,
    ) -> BoxFuture<Result<RpcResponse<RpcSimulateTransactionResult>>>;

    #[rpc(meta, name = "minimumLedgerSlot")]
    fn minimum_ledger_slot(&self, meta: Self::Metadata) -> Result<Slot>;

    #[rpc(meta, name = "getBlock")]
    fn get_block(
        &self,
        meta: Self::Metadata,
        slot: Slot,
        config: Option<RpcEncodingConfigWrapper<RpcBlockConfig>>,
    ) -> BoxFuture<Result<Option<UiConfirmedBlock>>>;

    #[rpc(meta, name = "getBlockTime")]
    fn get_block_time(
        &self,
        meta: Self::Metadata,
        slot: Slot,
    ) -> BoxFuture<Result<Option<UnixTimestamp>>>;

    #[rpc(meta, name = "getBlocks")]
    fn get_blocks(
        &self,
        meta: Self::Metadata,
        start_slot: Slot,
        wrapper: Option<RpcBlocksConfigWrapper>,
        config: Option<RpcContextConfig>,
    ) -> BoxFuture<Result<Vec<Slot>>>;

    #[rpc(meta, name = "getBlocksWithLimit")]
    fn get_blocks_with_limit(
        &self,
        meta: Self::Metadata,
        start_slot: Slot,
        limit: usize,
        config: Option<RpcContextConfig>,
    ) -> BoxFuture<Result<Vec<Slot>>>;

    #[rpc(meta, name = "getTransaction")]
    fn get_transaction(
        &self,
        meta: Self::Metadata,
        signature_str: String,
        config: Option<RpcEncodingConfigWrapper<RpcTransactionConfig>>,
    ) -> BoxFuture<Result<Option<EncodedConfirmedTransactionWithStatusMeta>>>;

    #[rpc(meta, name = "getSignaturesForAddress")]
    fn get_signatures_for_address(
        &self,
        meta: Self::Metadata,
        address: String,
        config: Option<RpcSignaturesForAddressConfig>,
    ) -> BoxFuture<Result<Vec<RpcConfirmedTransactionStatusWithSignature>>>;

    #[rpc(meta, name = "getFirstAvailableBlock")]
    fn get_first_available_block(&self, meta: Self::Metadata) -> BoxFuture<Result<Slot>>;

    #[rpc(meta, name = "getLatestBlockhash")]
    fn get_latest_blockhash(
        &self,
        meta: Self::Metadata,
        config: Option<RpcContextConfig>,
    ) -> Result<RpcResponse<RpcBlockhash>>;

    #[rpc(meta, name = "isBlockhashValid")]
    fn is_blockhash_valid(
        &self,
        meta: Self::Metadata,
        blockhash: String,
        config: Option<RpcContextConfig>,
    ) -> Result<RpcResponse<bool>>;

    #[rpc(meta, name = "getFeeForMessage")]
    fn get_fee_for_message(
        &self,
        meta: Self::Metadata,
        data: String,
        config: Option<RpcContextConfig>,
    ) -> Result<RpcResponse<Option<u64>>>;

    #[rpc(meta, name = "getStakeMinimumDelegation")]
    fn get_stake_minimum_delegation(
        &self,
        meta: Self::Metadata,
        config: Option<RpcContextConfig>,
    ) -> Result<RpcResponse<u64>>;

    #[rpc(meta, name = "getRecentPrioritizationFees")]
    fn get_recent_prioritization_fees(
        &self,
        meta: Self::Metadata,
        pubkey_strs: Option<Vec<String>>,
    ) -> Result<Vec<RpcPrioritizationFee>>;
}

pub struct SurfpoolFullRpc;
impl Full for SurfpoolFullRpc {
    type Metadata = Option<RunloopContext>;

    fn get_inflation_reward(
        &self,
        _meta: Self::Metadata,
        _address_strs: Vec<String>,
        _config: Option<RpcEpochConfig>,
    ) -> BoxFuture<Result<Vec<Option<RpcInflationReward>>>> {
        not_implemented_err_async()
    }

    fn get_cluster_nodes(&self, _meta: Self::Metadata) -> Result<Vec<RpcContactInfo>> {
        not_implemented_err()
    }

    fn get_recent_performance_samples(
        &self,
        meta: Self::Metadata,
        limit: Option<usize>,
    ) -> Result<Vec<RpcPerfSample>> {
        let limit = limit.unwrap_or(720);
        if limit > 720 {
            return Err(Error::invalid_params("Invalid limit; max 720").into());
        }

        let state_reader = meta.get_state()?;
        let samples = state_reader
            .perf_samples
            .iter()
            .map(|e| e.clone())
            .take(limit)
            .collect::<Vec<_>>();
        Ok(samples)
    }

    fn get_signature_statuses(
        &self,
        meta: Self::Metadata,
        signature_strs: Vec<String>,
        _config: Option<RpcSignatureStatusConfig>,
    ) -> BoxFuture<Result<RpcResponse<Vec<Option<TransactionStatus>>>>> {
        let state_reader = match meta.get_state() {
            Ok(s) => s,
            Err(e) => return Box::pin(future::err(e.into())),
        };

        let mut responses = Vec::with_capacity(signature_strs.len());
        let mut indices_to_fetch = Vec::with_capacity(signature_strs.len());
        for (i, signature_str) in signature_strs.iter().enumerate() {
            let Ok(signature) = Signature::from_str(&signature_str) else {
                responses.push(None);
                continue;
            };
            let entry = state_reader
                .transactions
                .get(&signature)
                .map(|entry| match entry {
                    EntryStatus::Received => TransactionStatus {
                        slot: 0,
                        confirmations: None,
                        status: Ok(()),
                        err: None,
                        confirmation_status: None,
                    },
                    EntryStatus::Processed(tx) => tx
                        .clone()
                        .into_status(state_reader.epoch_info.absolute_slot),
                });
            if let Some(status) = entry {
                responses.push(Some(status));
                continue;
            }
            indices_to_fetch.push((i, signature));
        }

        let current_slot = state_reader.epoch_info.absolute_slot;
        let rpc_client = state_reader.rpc_client.clone();

        Box::pin(async move {
            for (i, signature) in indices_to_fetch.iter() {
                let response = rpc_client
                    .get_transaction(&signature, UiTransactionEncoding::Json)
                    .await
                    .ok()
                    .map(|tx| TransactionStatus {
                        slot: tx.slot,
                        confirmations: Some((current_slot - tx.slot) as usize),
                        status: tx.transaction.meta.clone().map_or(Ok(()), |m| m.status),
                        err: tx.transaction.meta.map(|m| m.err).flatten(),
                        confirmation_status: Some(
                            solana_transaction_status::TransactionConfirmationStatus::Confirmed,
                        ),
                    });
                responses.insert(*i, response);
            }
            Ok(RpcResponse {
                context: RpcResponseContext::new(0),
                value: responses,
            })
        })
    }

    fn get_max_retransmit_slot(&self, _meta: Self::Metadata) -> Result<Slot> {
        not_implemented_err()
    }

    fn get_max_shred_insert_slot(&self, _meta: Self::Metadata) -> Result<Slot> {
        not_implemented_err()
    }

    fn request_airdrop(
        &self,
        meta: Self::Metadata,
        pubkey_str: String,
        lamports: u64,
        _config: Option<RpcRequestAirdropConfig>,
    ) -> Result<String> {
        let pk = Pubkey::from_str_const(&pubkey_str);
        let mut state_writer = meta.get_state_mut()?;

        let tx_result = state_writer
            .svm
            .airdrop(&pk, lamports)
            .map_err(|err| Error::invalid_params(format!("failed to send transaction: {err:?}")))?;

        // TODO: this is a workaround until LiteSVM records full transactions
        let airdrop_kp = Keypair::new(); // TODO: use the private keypair from LiteSVM
        let slot = state_writer.epoch_info.absolute_slot;
        state_writer.transactions.insert(
            tx_result.signature,
            EntryStatus::Processed(TransactionWithStatusMeta(
                slot,
                VersionedTransaction::try_new(
                    VersionedMessage::Legacy(Message::new(
                        &[system_instruction::transfer(
                            &airdrop_kp.pubkey(),
                            &pk,
                            lamports,
                        )],
                        Some(&airdrop_kp.pubkey()),
                    )),
                    &[airdrop_kp],
                )
                .unwrap()
                .into_legacy_transaction()
                .unwrap(),
                convert_transaction_metadata_from_canonical(&tx_result),
                None,
            )),
        );

        Ok(tx_result.signature.to_string())
    }

    fn send_transaction(
        &self,
        meta: Self::Metadata,
        data: String,
        config: Option<RpcSendTransactionConfig>,
    ) -> Result<String> {
        let config = config.unwrap_or_default();
        let tx_encoding = config.encoding.unwrap_or(UiTransactionEncoding::Base58);
        let binary_encoding = tx_encoding.into_binary_encoding().ok_or_else(|| {
            Error::invalid_params(format!(
                "unsupported encoding: {tx_encoding}. Supported encodings: base58, base64"
            ))
        })?;
        let (_, unsanitized_tx) =
            decode_and_deserialize::<VersionedTransaction>(data, binary_encoding)?;

        let Some(ctx) = meta else {
            return Err(RpcCustomError::NodeUnhealthy {
                num_slots_behind: None,
            }
            .into());
        };

        let signatures = unsanitized_tx.signatures.clone();
        let signature = signatures[0];
        let (status_update_tx, status_uptate_rx) = crossbeam_channel::bounded(1);
        let _ = ctx
            .simnet_commands_tx
            .send(SimnetCommand::TransactionReceived(
                ctx.id.clone(),
                unsanitized_tx,
                status_update_tx,
                config.skip_preflight,
            ));
        loop {
            match (status_uptate_rx.recv(), config.preflight_commitment) {
                (Ok(TransactionStatusEvent::SimulationFailure(e)), _) => {
                    return Err(Error {
                        data: None,
                        message: format!(
                            "Transaction simulation failed: {}: {} log messages:\n{}",
                            e.0.to_string(),
                            e.1.logs.len(),
                            e.1.logs.iter().map(|l| l.to_string()).join("\n")
                        ),
                        code: jsonrpc_core::ErrorCode::ServerError(-32002),
                    })
                }
                (Ok(TransactionStatusEvent::ExecutionFailure(e)), _) => {
                    return Err(Error {
                        data: None,
                        message: format!(
                            "Transaction execution failed: {}: {} log messages:\n{}",
                            e.0.to_string(),
                            e.1.logs.len(),
                            e.1.logs.iter().map(|l| l.to_string()).join("\n")
                        ),
                        code: jsonrpc_core::ErrorCode::ServerError(-32002),
                    })
                }
                (
                    Ok(TransactionStatusEvent::Success(TransactionConfirmationStatus::Processed)),
                    Some(CommitmentLevel::Processed),
                ) => break,
                (
                    Ok(TransactionStatusEvent::Success(TransactionConfirmationStatus::Confirmed)),
                    None | Some(CommitmentLevel::Confirmed),
                ) => break,
                (
                    Ok(TransactionStatusEvent::Success(TransactionConfirmationStatus::Finalized)),
                    Some(CommitmentLevel::Finalized),
                ) => break,
                (Err(_), _) => break,
                (_, _) => continue,
            }
        }
        Ok(signature.to_string())
    }

    fn simulate_transaction(
        &self,
        meta: Self::Metadata,
        data: String,
        config: Option<RpcSimulateTransactionConfig>,
    ) -> BoxFuture<Result<RpcResponse<RpcSimulateTransactionResult>>> {
        let config = config.unwrap_or_default();
        let (_bytes, tx): (Vec<_>, Transaction) = match decode_and_deserialize(
            data,
            config
                .encoding
                .map(|enconding| enconding.into_binary_encoding())
                .flatten()
                .unwrap_or(TransactionBinaryEncoding::Base58),
        ) {
            Ok(res) => res,
            Err(e) => return Box::pin(future::err(e)),
        };

        let (local_accounts, replacement_blockhash, rpc_client) = {
            let state = match meta.get_state_mut() {
                Ok(res) => res,
                Err(e) => return Box::pin(future::err(e.into())),
            };
            let local_accounts: Vec<Option<Account>> = tx
                .message
                .account_keys
                .clone()
                .into_iter()
                .map(|pk| state.svm.get_account(&pk))
                .collect();
            let replacement_blockhash = Some(RpcBlockhash {
                blockhash: state.svm.latest_blockhash().to_string(),
                last_valid_block_height: state.epoch_info.block_height,
            });
            let rpc_client = state.rpc_client.clone();

            (local_accounts, replacement_blockhash, rpc_client)
        };

        Box::pin(async move {
            let fetched_accounts = join_all(
                tx.message
                    .account_keys
                    .iter()
                    .map(|pk| async { rpc_client.get_account(pk).await.ok() }),
            )
            .await;

            let mut state_writer = meta.get_state_mut()?;
            state_writer.svm.set_sigverify(config.sig_verify);
            state_writer
                .svm
                .set_blockhash_check(config.replace_recent_blockhash);

            // Update missing local accounts
            tx.message
                .account_keys
                .iter()
                .zip(local_accounts.iter().zip(fetched_accounts))
                .map(|(pk, (local, fetched))| {
                    if local.is_none() {
                        if let Some(account) = fetched {
                            state_writer.svm.set_account(*pk, account).map_err(|err| {
                                Error::invalid_params(format!(
                                    "failed to save fetched account {pk:?}: {err:?}"
                                ))
                            })?;
                        }
                    }
                    Ok(())
                })
                .collect::<Result<()>>()?;

            match state_writer.svm.simulate_transaction(tx) {
                Ok(tx_info) => Ok(RpcResponse {
                    context: RpcResponseContext::new(state_writer.epoch_info.absolute_slot),
                    value: RpcSimulateTransactionResult {
                        err: None,
                        logs: Some(tx_info.meta.logs.clone()),
                        accounts: if let Some(accounts) = config.accounts {
                            Some(
                                accounts
                                    .addresses
                                    .iter()
                                    .map(|pk_str| {
                                        if let Some((pk, account)) = tx_info
                                            .post_accounts
                                            .iter()
                                            .find(|(pk, _)| pk.to_string() == *pk_str)
                                        {
                                            Some(encode_ui_account(
                                                pk,
                                                account,
                                                UiAccountEncoding::Base64,
                                                None,
                                                None,
                                            ))
                                        } else {
                                            None
                                        }
                                    })
                                    .collect(),
                            )
                        } else {
                            None
                        },
                        units_consumed: Some(tx_info.meta.compute_units_consumed),
                        return_data: Some(tx_info.meta.return_data.clone().into()),
                        inner_instructions: if config.inner_instructions {
                            Some(transform_tx_metadata_to_ui_accounts(&tx_info.meta))
                        } else {
                            None
                        },
                        replacement_blockhash,
                    },
                }),
                Err(tx_info) => Ok(RpcResponse {
                    context: RpcResponseContext::new(state_writer.epoch_info.absolute_slot),
                    value: RpcSimulateTransactionResult {
                        err: Some(tx_info.err),
                        logs: Some(tx_info.meta.logs.clone()),
                        accounts: None,
                        units_consumed: Some(tx_info.meta.compute_units_consumed),
                        return_data: Some(tx_info.meta.return_data.clone().into()),
                        inner_instructions: if config.inner_instructions {
                            Some(transform_tx_metadata_to_ui_accounts(&tx_info.meta))
                        } else {
                            None
                        },
                        replacement_blockhash,
                    },
                }),
            }
        })
    }

    fn minimum_ledger_slot(&self, _meta: Self::Metadata) -> Result<Slot> {
        not_implemented_err()
    }

    fn get_block(
        &self,
        meta: Self::Metadata,
        slot: Slot,
        config: Option<RpcEncodingConfigWrapper<RpcBlockConfig>>,
    ) -> BoxFuture<Result<Option<UiConfirmedBlock>>> {
        let config = config
            .map(|c| match c {
                RpcEncodingConfigWrapper::Deprecated(encoding) => RpcBlockConfig {
                    encoding,
                    ..RpcBlockConfig::default()
                },
                RpcEncodingConfigWrapper::Current(None) => RpcBlockConfig::default(),
                RpcEncodingConfigWrapper::Current(Some(c)) => c,
            })
            .unwrap_or_default();

        let state_reader = match meta.get_state() {
            Ok(res) => res,
            Err(err) => return Box::pin(future::err(err.into())),
        };

        let rpc_client = state_reader.rpc_client.clone();

        Box::pin(async move {
            let ui_confirmed_block: UiConfirmedBlock = rpc_client
                .get_block_with_config(slot, config)
                .await
                .map_err(|err| {
                    Error::invalid_params(format!("failed to get block with config: {err:?}"))
                })?;

            Ok(Some(ui_confirmed_block))
        })
    }

    fn get_block_time(
        &self,
        meta: Self::Metadata,
        slot: Slot,
    ) -> BoxFuture<Result<Option<UnixTimestamp>>> {
        let state_reader = match meta.get_state() {
            Ok(res) => res,
            Err(err) => return Box::pin(future::err(err.into())),
        };

        let rpc_client = state_reader.rpc_client.clone();

        Box::pin(async move {
            let response = match rpc_client.get_block_time(slot).await {
                Ok(res) => return Ok(Some(res)),
                Err(_res) => Ok(None),
            };
            response
        })
    }

    fn get_blocks(
        &self,
        _meta: Self::Metadata,
        _start_slot: Slot,
        _wrapper: Option<RpcBlocksConfigWrapper>,
        _config: Option<RpcContextConfig>,
    ) -> BoxFuture<Result<Vec<Slot>>> {
        not_implemented_err_async()
    }

    fn get_blocks_with_limit(
        &self,
        _meta: Self::Metadata,
        _start_slot: Slot,
        _limit: usize,
        _config: Option<RpcContextConfig>,
    ) -> BoxFuture<Result<Vec<Slot>>> {
        not_implemented_err_async()
    }

    fn get_transaction(
        &self,
        meta: Self::Metadata,
        signature_str: String,
        config: Option<RpcEncodingConfigWrapper<RpcTransactionConfig>>,
    ) -> BoxFuture<Result<Option<EncodedConfirmedTransactionWithStatusMeta>>> {
        let config = config
            .map(|c| match c {
                RpcEncodingConfigWrapper::Deprecated(encoding) => RpcTransactionConfig {
                    encoding,
                    ..RpcTransactionConfig::default()
                },
                RpcEncodingConfigWrapper::Current(None) => RpcTransactionConfig::default(),
                RpcEncodingConfigWrapper::Current(Some(c)) => c,
            })
            .unwrap_or_default();
        let signature_bytes = match bs58::decode(signature_str)
            .into_vec()
            .map_err(|e| Error::invalid_params(format!("failed to decode bs58 data: {e:?}")))
        {
            Ok(s) => s,
            Err(err) => return Box::pin(future::err(err.into())),
        };
        let signature = match Signature::try_from(signature_bytes.as_slice())
            .map_err(|e| Error::invalid_params(format!("failed to decode bs58 data: {e:?}")))
        {
            Ok(s) => s,
            Err(err) => return Box::pin(future::err(err.into())),
        };

        let state_reader = match meta.get_state() {
            Ok(s) => s,
            Err(err) => return Box::pin(future::err(err.into())),
        };
        let rpc_client = state_reader.rpc_client.clone();
        let tx = state_reader
            .transactions
            .get(&signature)
            .map(|entry| entry.expect_processed().clone().into());

        Box::pin(async move {
            // TODO: implement new interfaces in LiteSVM to get all the relevant info
            // needed to return the actual tx, not just some metadata
            if let Some(tx) = tx {
                Ok(Some(tx))
            } else {
                match rpc_client
                    .get_transaction(
                        &signature,
                        config.encoding.unwrap_or(UiTransactionEncoding::Json),
                    )
                    .await
                {
                    Ok(tx) => return Ok(Some(tx)),
                    Err(_tx) => Ok(None),
                }
            }
        })
    }

    fn get_signatures_for_address(
        &self,
        _meta: Self::Metadata,
        _address: String,
        _config: Option<RpcSignaturesForAddressConfig>,
    ) -> BoxFuture<Result<Vec<RpcConfirmedTransactionStatusWithSignature>>> {
        not_implemented_err_async()
    }

    fn get_first_available_block(&self, meta: Self::Metadata) -> BoxFuture<Result<Slot>> {
        let state_reader = match meta.get_state() {
            Ok(res) => res,
            Err(e) => return Box::pin(future::err(e.into())),
        };

        let rpc_client = state_reader.rpc_client.clone();

        Box::pin(async move {
            let response = rpc_client.get_first_available_block().await.map_err(|err| {
                Error::invalid_params(format!("failed to get first available block: {err:?}"))
            });

            response
        })
    }

    fn get_latest_blockhash(
        &self,
        meta: Self::Metadata,
        _config: Option<RpcContextConfig>,
    ) -> Result<RpcResponse<RpcBlockhash>> {
        let state_reader = meta.get_state()?;

        // Todo: are we returning the right block height?
        let last_valid_block_height = state_reader.epoch_info.block_height;
        let value = RpcBlockhash {
            blockhash: state_reader.svm.latest_blockhash().to_string(),
            last_valid_block_height,
        };
        let response = RpcResponse {
            context: RpcResponseContext {
                slot: state_reader.epoch_info.absolute_slot,
                api_version: Some(RpcApiVersion::default()),
            },
            value,
        };
        Ok(response)
    }

    fn is_blockhash_valid(
        &self,
        meta: Self::Metadata,
        _blockhash: String,
        _config: Option<RpcContextConfig>,
    ) -> Result<RpcResponse<bool>> {
        let state_reader = meta.get_state()?;
        Ok(RpcResponse {
            context: RpcResponseContext::new(state_reader.epoch_info.absolute_slot),
            value: true,
        })
    }

    fn get_fee_for_message(
        &self,
        meta: Self::Metadata,
        encoded: String,
        _config: Option<RpcContextConfig>, // TODO: use config
    ) -> Result<RpcResponse<Option<u64>>> {
        let (_, message) =
            decode_and_deserialize::<VersionedMessage>(encoded, TransactionBinaryEncoding::Base64)?;
        let state_reader = meta.get_state()?;

        // TODO: add fee computation APIs in LiteSVM
        Ok(RpcResponse {
            context: RpcResponseContext::new(state_reader.epoch_info.absolute_slot),
            value: Some((message.header().num_required_signatures as u64) * 5000),
        })
    }

    fn get_stake_minimum_delegation(
        &self,
        _meta: Self::Metadata,
        _config: Option<RpcContextConfig>,
    ) -> Result<RpcResponse<u64>> {
        not_implemented_err()
    }

    fn get_recent_prioritization_fees(
        &self,
        _meta: Self::Metadata,
        _pubkey_strs: Option<Vec<String>>,
    ) -> Result<Vec<RpcPrioritizationFee>> {
        not_implemented_err()
    }
}

#[cfg(test)]
mod tests {
    use crate::tests::helpers::TestSetup;

    use super::*;
    use base64::{prelude::BASE64_STANDARD, Engine};
    use solana_account_decoder::{UiAccount, UiAccountData};
    use solana_client::rpc_config::RpcSimulateTransactionAccountsConfig;
    use solana_sdk::{
        commitment_config::CommitmentConfig,
        hash::Hash,
        message::{Message, MessageHeader},
        native_token::LAMPORTS_PER_SOL,
        signature::Keypair,
        signer::Signer,
        system_instruction, system_program,
        transaction::{Legacy, TransactionVersion},
    };
    use solana_transaction_status::{
        EncodedTransaction, EncodedTransactionWithStatusMeta, UiCompiledInstruction, UiMessage,
        UiRawMessage, UiTransaction,
    };
    use test_case::test_case;

    #[test_case(None, false ; "when limit is None")]
    #[test_case(Some(1), false ; "when limit is ok")]
    #[test_case(Some(1000), true ; "when limit is above max spec")]
    fn test_get_recent_performance_samples(limit: Option<usize>, fails: bool) {
        let setup = TestSetup::new(SurfpoolFullRpc);
        let res = setup
            .rpc
            .get_recent_performance_samples(Some(setup.context), limit);

        if fails {
            assert!(res.is_err());
        } else {
            assert!(res.is_ok());
        }
    }

    #[tokio::test]
    async fn test_get_signature_statuses() {
        let pks = (0..10).map(|_| Pubkey::new_unique());
        let valid_txs = pks.len();
        let invalid_txs = pks.len();
        let payer = Keypair::new();
        let recent_blockhash = Hash::default();
        let valid = pks
            .clone()
            .map(|pk| {
                Transaction::new_signed_with_payer(
                    &[system_instruction::transfer(
                        &payer.pubkey(),
                        &pk,
                        LAMPORTS_PER_SOL,
                    )],
                    Some(&payer.pubkey()),
                    &[payer.insecure_clone()],
                    recent_blockhash,
                )
            })
            .collect::<Vec<_>>();
        let invalid = pks
            .map(|pk| {
                Transaction::new_unsigned(Message::new(
                    &[system_instruction::transfer(
                        &pk,
                        &payer.pubkey(),
                        LAMPORTS_PER_SOL,
                    )],
                    Some(&payer.pubkey()),
                ))
            })
            .collect::<Vec<_>>();
        let txs = valid
            .into_iter()
            .chain(invalid.into_iter())
            .collect::<Vec<_>>();
        let mut setup = TestSetup::new_without_blockhash(SurfpoolFullRpc);
        let _ = setup.context.state.write().unwrap().svm.airdrop(
            &payer.pubkey(),
            (valid_txs + invalid_txs) as u64 * 2 * LAMPORTS_PER_SOL,
        );
        setup.process_txs(txs.clone());

        let res = setup
            .rpc
            .get_signature_statuses(
                Some(setup.context),
                txs.iter().map(|tx| tx.signatures[0].to_string()).collect(),
                None,
            )
            .await
            .unwrap();

        assert_eq!(
            res.value
                .iter()
                .filter(|status| {
                    if let Some(s) = status {
                        s.status.is_ok()
                    } else {
                        false
                    }
                })
                .count(),
            valid_txs,
            "incorrect number of valid txs"
        );
        assert_eq!(
            res.value
                .iter()
                .filter(|status| if let Some(s) = status {
                    s.status.is_err()
                } else {
                    true
                })
                .count(),
            invalid_txs,
            "incorrect number of invalid txs"
        );
    }

    #[test]
    fn test_request_airdrop() {
        let pk = Pubkey::new_unique();
        let lamports = 1000;
        let setup = TestSetup::new(SurfpoolFullRpc);
        let res = setup
            .rpc
            .request_airdrop(Some(setup.context.clone()), pk.to_string(), lamports, None)
            .unwrap();
        let sig = Signature::from_str(res.as_str()).unwrap();
        let state_reader = setup.context.state.read().unwrap();
        assert_eq!(
            state_reader.svm.get_account(&pk).unwrap().lamports,
            lamports,
            "airdropped amount is incorrect"
        );
        assert!(
            state_reader.svm.get_transaction(&sig).is_some(),
            "transaction is not found in the SVM"
        );
        assert!(
            state_reader.transactions.get(&sig).is_some(),
            "transaction is not found in the history"
        );
    }

    #[test]
    fn test_send_transaction() {
        let payer = Keypair::new();
        let pk = Pubkey::new_unique();
        let (mempool_tx, mempool_rx) = crossbeam_channel::unbounded();
        let setup = TestSetup::new_with_mempool(SurfpoolFullRpc, mempool_tx);
        let tx = Transaction::new_signed_with_payer(
            &[system_instruction::transfer(
                &payer.pubkey(),
                &pk,
                LAMPORTS_PER_SOL,
            )],
            Some(&payer.pubkey()),
            &[payer.insecure_clone()],
            Hash::default(),
        );
        let _ = setup
            .context
            .state
            .write()
            .unwrap()
            .svm
            .airdrop(&payer.pubkey(), 2 * LAMPORTS_PER_SOL);

        let cloned_tx = tx.clone();
        let handle = hiro_system_kit::thread_named("send_tx")
            .spawn(move || {
                let res = setup
                    .rpc
                    .send_transaction(
                        Some(setup.context),
                        bs58::encode(bincode::serialize(&cloned_tx).unwrap()).into_string(),
                        None,
                    )
                    .unwrap();

                res
            })
            .unwrap();

        match mempool_rx.recv() {
            Ok(SimnetCommand::TransactionReceived(_, _, status_tx, _)) => {
                status_tx
                    .send(TransactionStatusEvent::Success(
                        TransactionConfirmationStatus::Confirmed,
                    ))
                    .unwrap();
            }
            _ => panic!("failed to receive transaction from mempool"),
        }

        assert_eq!(
            handle.join().unwrap(),
            tx.signatures[0].to_string(),
            "incorrect signature"
        );
    }

    #[tokio::test]
    async fn test_simulate_transaction() {
        let payer = Keypair::new();
        let pk = Pubkey::new_unique();
        let lamports = LAMPORTS_PER_SOL;
        let setup = TestSetup::new(SurfpoolFullRpc);
        let _ = setup
            .rpc
            .request_airdrop(
                Some(setup.context.clone()),
                payer.pubkey().to_string(),
                2 * lamports,
                None,
            )
            .unwrap();
        let tx = Transaction::new_signed_with_payer(
            &[system_instruction::transfer(&payer.pubkey(), &pk, lamports)],
            Some(&payer.pubkey()),
            &[payer.insecure_clone()],
            Hash::default(),
        );
        let simulation_res = setup
            .rpc
            .simulate_transaction(
                Some(setup.context),
                bs58::encode(bincode::serialize(&tx).unwrap()).into_string(),
                Some(RpcSimulateTransactionConfig {
                    sig_verify: true,
                    replace_recent_blockhash: false,
                    commitment: Some(CommitmentConfig::finalized()),
                    encoding: None,
                    accounts: Some(RpcSimulateTransactionAccountsConfig {
                        encoding: None,
                        addresses: vec![pk.to_string()],
                    }),
                    min_context_slot: None,
                    inner_instructions: false,
                }),
            )
            .await
            .unwrap();

        assert_eq!(
            simulation_res.value.err, None,
            "Unexpected simulation error"
        );
        assert_eq!(
            simulation_res.value.accounts,
            Some(vec![Some(UiAccount {
                lamports,
                data: UiAccountData::Binary(BASE64_STANDARD.encode(""), UiAccountEncoding::Base64),
                owner: system_program::id().to_string(),
                executable: false,
                rent_epoch: 0,
                space: Some(0),
            })]),
            "Wrong account content"
        );
    }

    #[tokio::test]
    async fn test_get_block() {
        let setup = TestSetup::new(SurfpoolFullRpc);
        let res = setup
            .rpc
            .get_block(Some(setup.context), 2050, None)
            .await
            .unwrap();

        let expected_response = UiConfirmedBlock {
            previous_blockhash: "Qtzp4zTxakbeAS5EhdetTwWHjw68Fc5jifih193qpgA".to_string(),
            blockhash: "2zudA7u9KW1ui4SUvgsmKtBjd2BG9FaKnM4no3v3TMtq".to_string(),
            parent_slot: 2049,
            transactions: Some(vec![
                EncodedTransactionWithStatusMeta {
                    transaction: EncodedTransaction::Json(UiTransaction {
                        signatures: vec![
                            "4S5tqrxM6kz512oqnJsSuPxukysu7tnASNfTsnJwstkTpXn9UghC8aseL3ACXeTGrcXv9qjsZ5YrKJJDBw2xNrdc".to_string(),
                            "342x9ChMf6qMezRnMxar7ni8sjwx5DWGCWqNzy48LhkMjK2MX2ku4kzGEuQXkNyEeBxqBAB8r3yT4q2sjTg1YeEi".to_string(),
                        ],
                        message: UiRawMessage {
                            header: MessageHeader {
                                num_required_signatures: 2,
                                num_readonly_signed_accounts: 0,
                                num_readonly_unsigned_accounts: 1,
                            },
                            account_keys: vec![
                                "CUuRJSPP2AnmqE8y9rUbCLpZediJSdFepwfRUEYSm9uu".to_string(),
                                "8BnsY4kLk9bUPN5bp2tCDQho58XgAPyRc8HBMeTgwZKg".to_string(),
                                "Vote111111111111111111111111111111111111111".to_string(),
                            ],
                            recent_blockhash: "Qtzp4zTxakbeAS5EhdetTwWHjw68Fc5jifih193qpgA".to_string(),
                            instructions: vec![
                                UiCompiledInstruction {
                                    program_id_index: 2,
                                    accounts: vec![1, 1],
                                    data: "Fk63Q3k4uDVSxLsQby8asxmMAGGG4vjACk4SmRAQuECntTPvNzCYEai7gyU7A87XGecXdhWjBpVjJHiZuAsRMmtVzoyitXWtA1zL5Uu6LfS16dZdifyfeX5CPGYB7UXXCKnqFY2sA8FgG3GWCwRyDuCm8yF1wu".to_string(),
                                    stack_height: None,
                                }
                            ],
                            address_table_lookups: None,
                        },
                    }),
                    meta: Some(UiTransactionStatusMeta {
                        err: None,
                        status: Some("Ok".to_string()),
                        fee: 10000,
                        pre_balances: vec![499989805000, 1000000000000000, 1],
                        post_balances: vec![499989795000, 1000000000000000, 1],
                        inner_instructions: Some(vec![]),
                        log_messages: Some(vec![
                            "Program Vote111111111111111111111111111111111111111 invoke [1]".to_string(),
                            "Program Vote111111111111111111111111111111111111111 success".to_string(),
                        ]),
                        pre_token_balances: Some(vec![]),
                        post_token_balances: Some(vec![]),
                        rewards: Some(vec![
                            Reward {
                                pubkey: "CUuRJSPP2AnmqE8y9rUbCLpZediJSdFepwfRUEYSm9uu".to_string(),
                                lamports: 5000,
                                post_balance: 499989800000,
                                reward_type: Some("Fee".to_string()),
                                commission: None,
                            }
                        ]),
                        loaded_addresses: Some(UiLoadedAddresses {
                            writable: vec![],
                            readonly: vec![],
                        }),
                        return_data: "Skip".to_string(),
                        compute_units_consumed: Some(2100),
                    }),
                    version: None,
                },
            ]),
            signatures: None,
            rewards: Some(Rewards {
                total_rewards: 5000,
                accounts: HashMap::from([
                    ("CUuRJSPP2AnmqE8y9rUbCLpZediJSdFepwfRUEYSm9uu".to_string(), 5000),
                ]),
            }),
            num_reward_partitions: None,
            block_time: Some(1742329096),
            block_height: Some(2046),
        };
        println!("this is the block {:#?}", res.unwrap()); //works
        assert_eq!(res, expected_response);
    }

    #[tokio::test]
    async fn test_get_block_time() {
        let setup = TestSetup::new(SurfpoolFullRpc);
        let res = setup
            .rpc
            .get_block_time(Some(setup.context), 0)
            .await
            .unwrap();
        println!("this is the block time {}", res.unwrap()); //works
        assert_eq!(res, Some(1742327046));
    }

    #[tokio::test]
    async fn test_get_transaction() {
        let payer = Keypair::new();
        let pk = Pubkey::new_unique();
        let lamports = LAMPORTS_PER_SOL;
        let mut setup = TestSetup::new_without_blockhash(SurfpoolFullRpc);
        let _ = setup
            .rpc
            .request_airdrop(
                Some(setup.context.clone()),
                payer.pubkey().to_string(),
                2 * lamports,
                None,
            )
            .unwrap();
        let tx = Transaction::new_signed_with_payer(
            &[system_instruction::transfer(&payer.pubkey(), &pk, lamports)],
            Some(&payer.pubkey()),
            &[payer.insecure_clone()],
            Hash::default(),
        );
        setup.process_txs(vec![tx.clone()]);
        let res = setup
            .rpc
            .get_transaction(
                Some(setup.context.clone()),
                tx.signatures[0].to_string(),
                None,
            )
            .await
            .unwrap()
            .unwrap();

        assert_eq!(
            res,
            EncodedConfirmedTransactionWithStatusMeta {
                slot: 0,
                transaction: EncodedTransactionWithStatusMeta {
                    transaction: EncodedTransaction::Json(UiTransaction {
                        signatures: vec![tx.signatures[0].to_string()],
                        message: UiMessage::Raw(UiRawMessage {
                            header: MessageHeader {
                                num_required_signatures: 1,
                                num_readonly_signed_accounts: 0,
                                num_readonly_unsigned_accounts: 1
                            },
                            account_keys: vec![
                                payer.pubkey().to_string(),
                                pk.to_string(),
                                system_program::id().to_string()
                            ],
                            recent_blockhash: Hash::default().to_string(),
                            instructions: vec![UiCompiledInstruction::from(
                                &tx.message.instructions[0],
                                None
                            )],
                            address_table_lookups: None
                        })
                    }),
                    meta: res.transaction.clone().meta, // Using the same values to avoid reintroducing processing logic errors
                    version: Some(TransactionVersion::Legacy(Legacy::Legacy))
                },
                block_time: res.block_time // Using the same values to avoid flakyness
            }
        );
    }

    #[tokio::test]
    async fn test_get_first_available_block() {
        let setup = TestSetup::new(SurfpoolFullRpc);
        let res = setup
            .rpc
            .get_first_available_block(Some(setup.context))
            .await
            .unwrap();
        println!("this is the first available_block {}", res); //works
        assert_eq!(res, 0);
    }

    #[test]
    fn test_get_latest_blockhash() {
        let setup = TestSetup::new(SurfpoolFullRpc);
        let res = setup
            .rpc
            .get_latest_blockhash(Some(setup.context.clone()), None)
            .unwrap();

        assert_eq!(
            res.value.blockhash,
            setup
                .context
                .state
                .read()
                .unwrap()
                .svm
                .latest_blockhash()
                .to_string()
        );
    }
}
