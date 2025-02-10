use crate::simnet::EntryStatus;

use super::utils::{decode_and_deserialize, transform_tx_metadata_to_ui_accounts};
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
use solana_sdk::message::VersionedMessage;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::Signature;
use solana_sdk::transaction::{Transaction, VersionedTransaction};
use solana_sdk::{account::Account, clock::UnixTimestamp};
use solana_transaction_status::{
    EncodedConfirmedTransactionWithStatusMeta, TransactionConfirmationStatus, TransactionStatus,
    UiConfirmedBlock,
};
use solana_transaction_status::{TransactionBinaryEncoding, UiTransactionEncoding};
use std::str::FromStr;

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
        unimplemented!()
    }

    fn get_cluster_nodes(&self, _meta: Self::Metadata) -> Result<Vec<RpcContactInfo>> {
        unimplemented!()
    }

    fn get_recent_performance_samples(
        &self,
        _meta: Self::Metadata,
        _limit: Option<usize>,
    ) -> Result<Vec<RpcPerfSample>> {
        unimplemented!()
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
                .history
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
                        confirmation_status: Some(TransactionConfirmationStatus::Confirmed),
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
        unimplemented!()
    }

    fn get_max_shred_insert_slot(&self, _meta: Self::Metadata) -> Result<Slot> {
        unimplemented!()
    }

    fn request_airdrop(
        &self,
        meta: Self::Metadata,
        pubkey_str: String,
        lamports: u64,
        _config: Option<RpcRequestAirdropConfig>,
    ) -> Result<String> {
        let pk = Pubkey::from_str_const(&pubkey_str);
        let mut state_reader = meta.get_state_mut()?;

        let tx_result = state_reader
            .svm
            .airdrop(&pk, lamports)
            .map_err(|err| Error::invalid_params(format!("failed to send transaction: {err:?}")))?;

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
        let _ = ctx.mempool_tx.send((ctx.id.clone(), unsanitized_tx));

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
            // // TODO: LiteSVM does not enable replacing the current blockhash

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
        unimplemented!()
    }

    fn get_block(
        &self,
        _meta: Self::Metadata,
        _slot: Slot,
        _config: Option<RpcEncodingConfigWrapper<RpcBlockConfig>>,
    ) -> BoxFuture<Result<Option<UiConfirmedBlock>>> {
        unimplemented!()
    }

    fn get_block_time(
        &self,
        _meta: Self::Metadata,
        _slot: Slot,
    ) -> BoxFuture<Result<Option<UnixTimestamp>>> {
        unimplemented!()
    }

    fn get_blocks(
        &self,
        _meta: Self::Metadata,
        _start_slot: Slot,
        _wrapper: Option<RpcBlocksConfigWrapper>,
        _config: Option<RpcContextConfig>,
    ) -> BoxFuture<Result<Vec<Slot>>> {
        unimplemented!()
    }

    fn get_blocks_with_limit(
        &self,
        _meta: Self::Metadata,
        _start_slot: Slot,
        _limit: usize,
        _config: Option<RpcContextConfig>,
    ) -> BoxFuture<Result<Vec<Slot>>> {
        unimplemented!()
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
            .history
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
        unimplemented!()
    }

    fn get_first_available_block(&self, _meta: Self::Metadata) -> BoxFuture<Result<Slot>> {
        unimplemented!()
    }

    fn get_latest_blockhash(
        &self,
        meta: Self::Metadata,
        _config: Option<RpcContextConfig>,
    ) -> Result<RpcResponse<RpcBlockhash>> {
        // Retrieve svm state
        let Some(ctx) = meta else {
            return Err(RpcCustomError::NodeUnhealthy {
                num_slots_behind: None,
            }
            .into());
        };
        // Lock read access
        let Ok(state_reader) = ctx.state.read() else {
            return Err(RpcCustomError::NodeUnhealthy {
                num_slots_behind: None,
            }
            .into());
        };
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
        unimplemented!()
    }

    fn get_recent_prioritization_fees(
        &self,
        _meta: Self::Metadata,
        _pubkey_strs: Option<Vec<String>>,
    ) -> Result<Vec<RpcPrioritizationFee>> {
        unimplemented!()
    }
}
