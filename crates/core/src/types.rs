use std::{collections::HashSet, vec};

use base64::{Engine, prelude::BASE64_STANDARD};
use chrono::Utc;
use litesvm::types::TransactionMetadata;
use solana_account::Account;
use solana_account_decoder::parse_token::UiTokenAmount;
use solana_clock::{Epoch, Slot};
use solana_message::{
    AccountKeys, VersionedMessage,
    v0::{LoadedAddresses, LoadedMessage, MessageAddressTableLookup},
};
use solana_pubkey::Pubkey;
use solana_sdk::{
    program_option::COption,
    program_pack::Pack,
    reserved_account_keys::ReservedAccountKeys,
    transaction::{SanitizedTransaction, TransactionVersion, VersionedTransaction},
};
use solana_transaction_status::{
    Encodable, EncodableWithMeta, EncodeError, EncodedTransaction,
    EncodedTransactionWithStatusMeta, InnerInstruction, InnerInstructions,
    TransactionBinaryEncoding, TransactionConfirmationStatus, TransactionStatus,
    TransactionStatusMeta, TransactionTokenBalance, UiAccountsList, UiLoadedAddresses,
    UiTransaction, UiTransactionEncoding, UiTransactionStatusMeta,
    option_serializer::OptionSerializer,
    parse_accounts::{parse_legacy_message_accounts, parse_v0_message_accounts},
    parse_ui_inner_instructions,
};
use spl_token_2022::extension::StateWithExtensions;
use txtx_addon_kit::indexmap::IndexMap;

use crate::{
    error::{SurfpoolError, SurfpoolResult},
    surfnet::locker::{format_ui_amount, format_ui_amount_string},
};

#[derive(Debug, Clone)]
pub enum SurfnetTransactionStatus {
    Received,
    Processed(Box<TransactionWithStatusMeta>),
}

impl SurfnetTransactionStatus {
    pub fn expect_processed(&self) -> &TransactionWithStatusMeta {
        match &self {
            SurfnetTransactionStatus::Received => unreachable!(),
            SurfnetTransactionStatus::Processed(status) => status,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct TransactionWithStatusMeta {
    pub slot: u64,
    pub transaction: VersionedTransaction,
    pub meta: TransactionStatusMeta,
}

impl TransactionWithStatusMeta {
    pub fn into_status(&self, current_slot: u64) -> TransactionStatus {
        TransactionStatus {
            slot: self.slot,
            confirmations: Some((current_slot - self.slot) as usize),
            status: self.meta.status.clone(),
            err: match &self.meta.status {
                Ok(_) => None,
                Err(e) => Some(e.clone()),
            },
            confirmation_status: Some(TransactionConfirmationStatus::Finalized),
        }
    }
    pub fn new(
        slot: u64,
        transaction: VersionedTransaction,
        transaction_meta: TransactionMetadata,
        accounts_before: &[Option<Account>],
        accounts_after: &[Option<Account>],
        pre_token_accounts_with_indexes: &[(usize, TokenAccount)],
        post_token_accounts_with_indexes: &[(usize, TokenAccount)],
        token_mints: Vec<MintAccount>,
        token_program_ids: &[Pubkey],
        loaded_addresses: LoadedAddresses,
    ) -> Self {
        let signatures_len = transaction.signatures.len();
        Self {
            slot,
            transaction,
            meta: TransactionStatusMeta {
                status: Ok(()),
                fee: 5000 * signatures_len as u64,
                pre_balances: accounts_before
                    .iter()
                    .map(|a| a.clone().map(|a| a.lamports).unwrap_or(0))
                    .collect(),
                post_balances: accounts_after
                    .iter()
                    .map(|a| a.clone().map(|a| a.lamports).unwrap_or(0))
                    .collect(),
                inner_instructions: Some(
                    transaction_meta
                        .inner_instructions
                        .iter()
                        .enumerate()
                        .filter_map(|(i, ixs)| {
                            if ixs.is_empty() {
                                None
                            } else {
                                Some(InnerInstructions {
                                    index: i as u8,
                                    instructions: ixs
                                        .iter()
                                        .map(|ix| InnerInstruction {
                                            instruction: ix.instruction.clone(),
                                            stack_height: Some(ix.stack_height as u32),
                                        })
                                        .collect(),
                                })
                            }
                        })
                        .collect(),
                ),
                log_messages: Some(transaction_meta.logs),
                pre_token_balances: Some(
                    pre_token_accounts_with_indexes
                        .iter()
                        .zip(token_mints.clone())
                        .zip(token_program_ids)
                        .map(|(((i, a), mint), token_program)| TransactionTokenBalance {
                            account_index: *i as u8,
                            mint: a.mint().to_string(),
                            ui_token_amount: UiTokenAmount {
                                ui_amount: Some(format_ui_amount(a.amount(), mint.decimals())),
                                decimals: mint.decimals(),
                                amount: a.amount().to_string(),
                                ui_amount_string: format_ui_amount_string(
                                    a.amount(),
                                    mint.decimals(),
                                ),
                            },
                            owner: a.owner().to_string(),
                            program_id: token_program.to_string(),
                        })
                        .collect(),
                ),
                post_token_balances: Some(
                    post_token_accounts_with_indexes
                        .iter()
                        .zip(token_mints)
                        .zip(token_program_ids)
                        .map(|(((i, a), mint), token_program)| TransactionTokenBalance {
                            account_index: *i as u8,
                            mint: a.mint().to_string(),
                            ui_token_amount: UiTokenAmount {
                                ui_amount: Some(format_ui_amount(a.amount(), mint.decimals())),
                                decimals: mint.decimals(),
                                amount: a.amount().to_string(),
                                ui_amount_string: format_ui_amount_string(
                                    a.amount(),
                                    mint.decimals(),
                                ),
                            },
                            owner: a.owner().to_string(),
                            program_id: token_program.to_string(),
                        })
                        .collect(),
                ),
                rewards: Some(vec![]),
                loaded_addresses,
                return_data: Some(transaction_meta.return_data),
                compute_units_consumed: Some(transaction_meta.compute_units_consumed),
            },
        }
    }

    pub fn encode(
        &self,
        encoding: UiTransactionEncoding,
        max_supported_transaction_version: Option<u8>,
        show_rewards: bool,
    ) -> Result<EncodedTransactionWithStatusMeta, EncodeError> {
        let version = self.validate_version(max_supported_transaction_version)?;
        Ok(EncodedTransactionWithStatusMeta {
            transaction: match encoding {
                UiTransactionEncoding::Binary => EncodedTransaction::LegacyBinary(
                    bs58::encode(bincode::serialize(&self.transaction).unwrap()).into_string(),
                ),
                UiTransactionEncoding::Base58 => EncodedTransaction::Binary(
                    bs58::encode(bincode::serialize(&self.transaction).unwrap()).into_string(),
                    TransactionBinaryEncoding::Base58,
                ),
                UiTransactionEncoding::Base64 => EncodedTransaction::Binary(
                    BASE64_STANDARD.encode(bincode::serialize(&self.transaction).unwrap()),
                    TransactionBinaryEncoding::Base64,
                ),
                UiTransactionEncoding::Json => EncodedTransaction::Json(UiTransaction {
                    signatures: self
                        .transaction
                        .signatures
                        .iter()
                        .map(ToString::to_string)
                        .collect(),
                    message: match &self.transaction.message {
                        VersionedMessage::Legacy(message) => {
                            message.encode(UiTransactionEncoding::Json)
                        }
                        VersionedMessage::V0(message) => message.json_encode(),
                    },
                }),
                UiTransactionEncoding::JsonParsed => EncodedTransaction::Json(UiTransaction {
                    signatures: self
                        .transaction
                        .signatures
                        .iter()
                        .map(ToString::to_string)
                        .collect(),
                    message: match &self.transaction.message {
                        VersionedMessage::Legacy(message) => {
                            message.encode(UiTransactionEncoding::JsonParsed)
                        }
                        VersionedMessage::V0(message) => {
                            message.encode_with_meta(UiTransactionEncoding::JsonParsed, &self.meta)
                        }
                    },
                }),
            },
            meta: Some(match encoding {
                UiTransactionEncoding::JsonParsed => {
                    parse_ui_transaction_status_meta_with_account_keys(
                        self.meta.clone(),
                        self.transaction.message.static_account_keys(),
                        show_rewards,
                    )
                }
                _ => {
                    let mut meta = parse_ui_transaction_status_meta(self.meta.clone());
                    if !show_rewards {
                        meta.rewards = OptionSerializer::None;
                    }
                    meta
                }
            }),
            version,
        })
    }

    pub fn to_json_accounts(
        &self,
        max_supported_transaction_version: Option<u8>,
        show_rewards: bool,
    ) -> Result<EncodedTransactionWithStatusMeta, EncodeError> {
        let version = self.validate_version(max_supported_transaction_version)?;
        let reserved_account_keys = ReservedAccountKeys::new_all_activated();

        let account_keys = match &self.transaction.message {
            VersionedMessage::Legacy(message) => parse_legacy_message_accounts(message),
            VersionedMessage::V0(message) => {
                let loaded_message = LoadedMessage::new_borrowed(
                    message,
                    &self.meta.loaded_addresses,
                    &reserved_account_keys.active,
                );
                parse_v0_message_accounts(&loaded_message)
            }
        };

        Ok(EncodedTransactionWithStatusMeta {
            transaction: EncodedTransaction::Accounts(UiAccountsList {
                signatures: self
                    .transaction
                    .signatures
                    .iter()
                    .map(ToString::to_string)
                    .collect(),
                account_keys,
            }),
            meta: Some(build_simple_ui_transaction_status_meta(
                self.meta.clone(),
                show_rewards,
            )),
            version,
        })
    }

    fn validate_version(
        &self,
        max_supported_transaction_version: Option<u8>,
    ) -> Result<Option<TransactionVersion>, EncodeError> {
        match (
            max_supported_transaction_version,
            self.transaction.version(),
        ) {
            // Set to none because old clients can't handle this field
            (None, TransactionVersion::LEGACY) => Ok(None),
            (None, TransactionVersion::Number(version)) => {
                Err(EncodeError::UnsupportedTransactionVersion(version))
            }
            (Some(_), TransactionVersion::LEGACY) => Ok(Some(TransactionVersion::LEGACY)),
            (Some(max_version), TransactionVersion::Number(version)) => {
                if version <= max_version {
                    Ok(Some(TransactionVersion::Number(version)))
                } else {
                    Err(EncodeError::UnsupportedTransactionVersion(version))
                }
            }
        }
    }
    pub fn from_failure(
        slot: u64,
        transaction: VersionedTransaction,
        failure: &litesvm::types::FailedTransactionMetadata,
        accounts_before: &[Option<Account>],
        accounts_after: &[Option<Account>],
        pre_token_accounts_with_indexes: &[(usize, TokenAccount)],
        token_mints: Vec<MintAccount>,
        token_program_ids: &[Pubkey],
        loaded_addresses: LoadedAddresses,
    ) -> Self {
        let pre_balances: Vec<u64> = accounts_before
            .iter()
            .map(|a| a.as_ref().map(|a| a.lamports).unwrap_or(0))
            .collect();

        let fee = 5000 * transaction.signatures.len() as u64;

        let post_balances: Vec<u64> = accounts_after
            .iter()
            .map(|a| a.clone().map(|a| a.lamports).unwrap_or(0))
            .collect();

        let balances: Vec<TransactionTokenBalance> = pre_token_accounts_with_indexes
            .iter()
            .zip(token_mints)
            .zip(token_program_ids)
            .map(|(((i, a), mint), token_program)| TransactionTokenBalance {
                account_index: *i as u8,
                mint: a.mint().to_string(),
                ui_token_amount: UiTokenAmount {
                    ui_amount: Some(format_ui_amount(a.amount(), mint.decimals())),
                    decimals: mint.decimals(),
                    amount: a.amount().to_string(),
                    ui_amount_string: format_ui_amount_string(a.amount(), mint.decimals()),
                },
                owner: a.owner().to_string(),
                program_id: token_program.to_string(),
            })
            .collect();

        Self {
            slot,
            transaction,
            meta: TransactionStatusMeta {
                status: Err(failure.err.clone()),
                fee,
                pre_balances,
                post_balances,
                inner_instructions: Some(
                    failure
                        .meta
                        .inner_instructions
                        .iter()
                        .enumerate()
                        .filter_map(|(i, ixs)| {
                            if ixs.is_empty() {
                                None
                            } else {
                                Some(InnerInstructions {
                                    index: i as u8,
                                    instructions: ixs
                                        .iter()
                                        .map(|ix| InnerInstruction {
                                            instruction: ix.instruction.clone(),
                                            stack_height: Some(ix.stack_height as u32),
                                        })
                                        .collect(),
                                })
                            }
                        })
                        .collect(),
                ),
                log_messages: Some(failure.meta.logs.clone()),
                pre_token_balances: Some(balances.clone()),
                post_token_balances: Some(balances),
                rewards: Some(vec![]),
                loaded_addresses,
                return_data: Some(failure.meta.return_data.clone()),
                compute_units_consumed: Some(failure.meta.compute_units_consumed),
            },
        }
    }
}

fn parse_ui_transaction_status_meta_with_account_keys(
    meta: TransactionStatusMeta,
    static_keys: &[Pubkey],
    show_rewards: bool,
) -> UiTransactionStatusMeta {
    let account_keys = AccountKeys::new(static_keys, Some(&meta.loaded_addresses));
    UiTransactionStatusMeta {
        err: meta.status.clone().err(),
        status: meta.status,
        fee: meta.fee,
        pre_balances: meta.pre_balances,
        post_balances: meta.post_balances,
        inner_instructions: meta
            .inner_instructions
            .map(|ixs| {
                ixs.into_iter()
                    .map(|ix| parse_ui_inner_instructions(ix, &account_keys))
                    .collect()
            })
            .into(),
        log_messages: meta.log_messages.into(),
        pre_token_balances: meta
            .pre_token_balances
            .map(|balance| balance.into_iter().map(Into::into).collect())
            .into(),
        post_token_balances: meta
            .post_token_balances
            .map(|balance| balance.into_iter().map(Into::into).collect())
            .into(),
        rewards: if show_rewards { meta.rewards } else { None }.into(),
        loaded_addresses: OptionSerializer::Skip,
        return_data: OptionSerializer::or_skip(
            meta.return_data.map(|return_data| return_data.into()),
        ),
        compute_units_consumed: OptionSerializer::or_skip(meta.compute_units_consumed),
    }
}

fn parse_ui_transaction_status_meta(meta: TransactionStatusMeta) -> UiTransactionStatusMeta {
    UiTransactionStatusMeta {
        err: meta.status.clone().err(),
        status: meta.status,
        fee: meta.fee,
        pre_balances: meta.pre_balances,
        post_balances: meta.post_balances,
        inner_instructions: meta
            .inner_instructions
            .map(|ixs| ixs.into_iter().map(Into::into).collect())
            .into(),
        log_messages: meta.log_messages.into(),
        pre_token_balances: meta
            .pre_token_balances
            .map(|balance| balance.into_iter().map(Into::into).collect())
            .into(),
        post_token_balances: meta
            .post_token_balances
            .map(|balance| balance.into_iter().map(Into::into).collect())
            .into(),
        rewards: meta.rewards.into(),
        loaded_addresses: Some(UiLoadedAddresses::from(&meta.loaded_addresses)).into(),
        return_data: OptionSerializer::or_skip(
            meta.return_data.map(|return_data| return_data.into()),
        ),
        compute_units_consumed: OptionSerializer::or_skip(meta.compute_units_consumed),
    }
}

fn build_simple_ui_transaction_status_meta(
    meta: TransactionStatusMeta,
    show_rewards: bool,
) -> UiTransactionStatusMeta {
    UiTransactionStatusMeta {
        err: meta.status.clone().err(),
        status: meta.status,
        fee: meta.fee,
        pre_balances: meta.pre_balances,
        post_balances: meta.post_balances,
        inner_instructions: OptionSerializer::Skip,
        log_messages: OptionSerializer::Skip,
        pre_token_balances: meta
            .pre_token_balances
            .map(|balance| balance.into_iter().map(Into::into).collect())
            .into(),
        post_token_balances: meta
            .post_token_balances
            .map(|balance| balance.into_iter().map(Into::into).collect())
            .into(),
        rewards: if show_rewards {
            meta.rewards.into()
        } else {
            OptionSerializer::Skip
        },
        loaded_addresses: OptionSerializer::Skip,
        return_data: OptionSerializer::Skip,
        compute_units_consumed: OptionSerializer::Skip,
    }
}

pub fn surfpool_tx_metadata_to_litesvm_tx_metadata(
    metadata: &surfpool_types::TransactionMetadata,
) -> litesvm::types::TransactionMetadata {
    litesvm::types::TransactionMetadata {
        compute_units_consumed: metadata.compute_units_consumed,
        logs: metadata.logs.clone(),
        return_data: metadata.return_data.clone(),
        inner_instructions: metadata.inner_instructions.clone(),
        signature: metadata.signature,
    }
}

#[derive(Debug, Clone)]
pub enum RemoteRpcResult<T> {
    Ok(T),
    MethodNotSupported,
}

impl<T> RemoteRpcResult<T> {
    /// Converts RemoteRpcResult to SurfpoolResult
    pub fn into_result(self) -> SurfpoolResult<T> {
        match self {
            RemoteRpcResult::Ok(value) => Ok(value),
            RemoteRpcResult::MethodNotSupported => Err(SurfpoolError::rpc_method_not_supported()),
        }
    }

    /// Converts RemoteRpcResult to SurfpoolResult, treating MethodNotSupported as a default value
    pub fn into_result_or_default(self, default: T) -> SurfpoolResult<T> {
        match self {
            RemoteRpcResult::Ok(value) => Ok(value),
            RemoteRpcResult::MethodNotSupported => Ok(default),
        }
    }

    /// Handles RemoteRpcResult with a callback for MethodNotSupported that returns a default value
    pub fn handle_method_not_supported<F>(self, on_not_supported: F) -> T
    where
        F: FnOnce() -> T,
    {
        match self {
            RemoteRpcResult::Ok(value) => value,
            RemoteRpcResult::MethodNotSupported => on_not_supported(),
        }
    }

    /// Maps the Ok variant while preserving MethodNotSupported
    pub fn map<U, F>(self, f: F) -> RemoteRpcResult<U>
    where
        F: FnOnce(T) -> U,
    {
        match self {
            RemoteRpcResult::Ok(value) => RemoteRpcResult::Ok(f(value)),
            RemoteRpcResult::MethodNotSupported => RemoteRpcResult::MethodNotSupported,
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq)]
pub enum TokenAccount {
    SplToken2022(spl_token_2022::state::Account),
    SplToken(spl_token::state::Account),
}

impl TokenAccount {
    pub fn unpack(bytes: &[u8]) -> SurfpoolResult<Self> {
        if let Ok(account) = spl_token_2022::state::Account::unpack(bytes) {
            Ok(Self::SplToken2022(account))
        } else if let Ok(account) = spl_token::state::Account::unpack(bytes) {
            Ok(Self::SplToken(account))
        } else if let Ok(account) =
            StateWithExtensions::<spl_token_2022::state::Account>::unpack(bytes)
        {
            Ok(Self::SplToken2022(account.base))
        } else {
            Err(SurfpoolError::unpack_token_account())
        }
    }

    pub fn new(token_program_id: &Pubkey, owner: Pubkey, mint: Pubkey) -> Self {
        if token_program_id == &spl_token_2022::id() {
            Self::SplToken2022(spl_token_2022::state::Account {
                mint,
                owner,
                state: spl_token_2022::state::AccountState::Initialized,
                ..Default::default()
            })
        } else {
            Self::SplToken(spl_token::state::Account {
                mint,
                owner,
                state: spl_token::state::AccountState::Initialized,
                ..Default::default()
            })
        }
    }

    pub fn pack_into_vec(&self) -> Vec<u8> {
        match self {
            Self::SplToken2022(account) => {
                let mut dst = [0u8; spl_token_2022::state::Account::LEN];
                account.pack_into_slice(&mut dst);
                dst.to_vec()
            }
            Self::SplToken(account) => {
                let mut dst = [0u8; spl_token::state::Account::LEN];
                account.pack_into_slice(&mut dst);
                dst.to_vec()
            }
        }
    }

    pub fn owner(&self) -> Pubkey {
        match self {
            Self::SplToken2022(account) => account.owner,
            Self::SplToken(account) => account.owner,
        }
    }

    pub fn mint(&self) -> Pubkey {
        match self {
            Self::SplToken2022(account) => account.mint,
            Self::SplToken(account) => account.mint,
        }
    }

    pub fn delegate(&self) -> COption<Pubkey> {
        match self {
            Self::SplToken2022(account) => account.delegate,
            Self::SplToken(account) => account.delegate,
        }
    }

    pub fn set_delegate(&mut self, delegate: COption<Pubkey>) {
        match self {
            Self::SplToken2022(account) => account.delegate = delegate,
            Self::SplToken(account) => account.delegate = delegate,
        }
    }

    pub fn set_delegated_amount(&mut self, delegated_amount: u64) {
        match self {
            Self::SplToken2022(account) => account.delegated_amount = delegated_amount,
            Self::SplToken(account) => account.delegated_amount = delegated_amount,
        }
    }

    pub fn set_close_authority(&mut self, close_authority: COption<Pubkey>) {
        match self {
            Self::SplToken2022(account) => account.close_authority = close_authority,
            Self::SplToken(account) => account.close_authority = close_authority,
        }
    }

    pub fn amount(&self) -> u64 {
        match self {
            Self::SplToken2022(account) => account.amount,
            Self::SplToken(account) => account.amount,
        }
    }

    pub fn set_amount(&mut self, amount: u64) {
        match self {
            Self::SplToken2022(account) => account.amount = amount,
            Self::SplToken(account) => account.amount = amount,
        }
    }

    pub fn get_packed_len_for_token_program_id(token_program_id: &Pubkey) -> usize {
        if *token_program_id == spl_token::id() {
            spl_token::state::Account::get_packed_len()
        } else {
            spl_token_2022::state::Account::get_packed_len()
        }
    }

    pub fn set_state_from_str(&mut self, state: &str) -> SurfpoolResult<()> {
        match self {
            Self::SplToken2022(account) => {
                account.state = match state {
                    "uninitialized" => spl_token_2022::state::AccountState::Uninitialized,
                    "frozen" => spl_token_2022::state::AccountState::Frozen,
                    "initialized" => spl_token_2022::state::AccountState::Initialized,
                    _ => {
                        return Err(SurfpoolError::invalid_token_account_state(
                            &state.to_string(),
                        ));
                    }
                }
            }
            Self::SplToken(account) => {
                account.state = match state {
                    "uninitialized" => spl_token::state::AccountState::Uninitialized,
                    "frozen" => spl_token::state::AccountState::Frozen,
                    "initialized" => spl_token::state::AccountState::Initialized,
                    _ => {
                        return Err(SurfpoolError::invalid_token_account_state(
                            &state.to_string(),
                        ));
                    }
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub enum MintAccount {
    SplToken2022(spl_token_2022::state::Mint),
    SplToken(spl_token::state::Mint),
}

impl MintAccount {
    pub fn unpack(bytes: &[u8]) -> SurfpoolResult<Self> {
        if let Ok(mint) = StateWithExtensions::<spl_token_2022::state::Mint>::unpack(&bytes) {
            Ok(Self::SplToken2022(mint.base))
        } else if let Ok(mint) = spl_token_2022::state::Mint::unpack(bytes) {
            Ok(Self::SplToken2022(mint))
        } else if let Ok(mint) = spl_token::state::Mint::unpack(bytes) {
            Ok(Self::SplToken(mint))
        } else {
            Err(SurfpoolError::unpack_mint_account())
        }
    }

    pub fn decimals(&self) -> u8 {
        match self {
            Self::SplToken2022(mint) => mint.decimals,
            Self::SplToken(mint) => mint.decimals,
        }
    }
}

pub struct GeyserAccountUpdate {
    pub pubkey: Pubkey,
    pub account: Account,
    pub slot: u64,
    pub sanitized_transaction: SanitizedTransaction,
    pub write_version: u64,
}
impl GeyserAccountUpdate {
    pub fn new(
        pubkey: Pubkey,
        account: Account,
        slot: u64,
        sanitized_transaction: SanitizedTransaction,
        write_version: u64,
    ) -> Self {
        Self {
            pubkey,
            account,
            slot,
            sanitized_transaction,
            write_version,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TimeTravelConfig {
    AbsoluteEpoch(Epoch),
    AbsoluteSlot(Slot),
    AbsoluteTimestamp(u64),
}

impl Default for TimeTravelConfig {
    fn default() -> Self {
        // chrono timestamp in ms, 1 hour from now
        Self::AbsoluteTimestamp(Utc::now().timestamp_millis() as u64 + 3600000)
    }
}

#[derive(Debug)]
pub enum TimeTravelError {
    PastTimestamp { target: u64, current: u64 },
    PastSlot { target: u64, current: u64 },
    PastEpoch { target: u64, current: u64 },
    ZeroSlotTime,
}

impl std::fmt::Display for TimeTravelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TimeTravelError::PastTimestamp { target, current } => {
                write!(
                    f,
                    "Cannot travel to past timestamp: target={}, current={}",
                    target, current
                )
            }
            TimeTravelError::PastSlot { target, current } => {
                write!(
                    f,
                    "Cannot travel to past slot: target={}, current={}",
                    target, current
                )
            }
            TimeTravelError::PastEpoch { target, current } => {
                write!(
                    f,
                    "Cannot travel to past epoch: target={}, current={}",
                    target, current
                )
            }
            TimeTravelError::ZeroSlotTime => {
                write!(f, "Cannot calculate time travel with zero slot time")
            }
        }
    }
}

impl std::error::Error for TimeTravelError {}

#[derive(Debug, Default)]
/// Tracks the loaded addresses with its associated index within an Address Lookup Table
pub struct IndexedLoadedAddresses {
    pub writable: Vec<(u8, Pubkey)>,
    pub readonly: Vec<(u8, Pubkey)>,
}

impl IndexedLoadedAddresses {
    pub fn new(writable: Vec<(u8, Pubkey)>, readonly: Vec<(u8, Pubkey)>) -> Self {
        Self { writable, readonly }
    }
    pub fn writable_keys(&self) -> Vec<&Pubkey> {
        self.writable.iter().map(|(_, pubkey)| pubkey).collect()
    }
    pub fn readonly_keys(&self) -> Vec<&Pubkey> {
        self.readonly.iter().map(|(_, pubkey)| pubkey).collect()
    }
    pub fn is_empty(&self) -> bool {
        self.writable.is_empty() && self.readonly.is_empty()
    }
}

#[derive(Debug, Default)]
/// Maps an Address Lookup Table entry to its indexed loaded addresses
pub struct TransactionLoadedAddresses(IndexMap<Pubkey, IndexedLoadedAddresses>);

impl TransactionLoadedAddresses {
    pub fn new() -> Self {
        Self(IndexMap::new())
    }

    /// Filters the loaded addresses based on the provided writable and readonly sets.
    pub fn filter_from_members(
        &self,
        writable: &HashSet<Pubkey>,
        readonly: &HashSet<Pubkey>,
    ) -> Self {
        let mut filtered = Self::new();
        for (pubkey, loaded_addresses) in &self.0 {
            let mut new_loaded_addresses = IndexedLoadedAddresses::default();
            new_loaded_addresses.writable.extend(
                loaded_addresses
                    .writable
                    .iter()
                    .filter(|&(_, addr)| writable.contains(addr)),
            );
            new_loaded_addresses.readonly.extend(
                loaded_addresses
                    .readonly
                    .iter()
                    .filter(|&(_, addr)| readonly.contains(addr)),
            );
            if !new_loaded_addresses.is_empty() {
                filtered.insert(*pubkey, new_loaded_addresses);
            }
        }
        filtered
    }

    pub fn insert(&mut self, pubkey: Pubkey, loaded_addresses: IndexedLoadedAddresses) {
        self.0.insert(pubkey, loaded_addresses);
    }

    pub fn insert_members(
        &mut self,
        pubkey: Pubkey,
        writable: Vec<(u8, Pubkey)>,
        readonly: Vec<(u8, Pubkey)>,
    ) {
        self.0
            .insert(pubkey, IndexedLoadedAddresses::new(writable, readonly));
    }

    pub fn loaded_addresses(&self) -> LoadedAddresses {
        let mut loaded = LoadedAddresses::default();
        for (_, loaded_addresses) in &self.0 {
            loaded.writable.extend(loaded_addresses.writable_keys());
            loaded.readonly.extend(loaded_addresses.readonly_keys());
        }
        loaded
    }

    pub fn all_loaded_addresses(&self) -> Vec<&Pubkey> {
        let mut writable = vec![];
        let mut readonly = vec![];

        for (_, loaded_addresses) in &self.0 {
            writable.extend(loaded_addresses.writable_keys());
            readonly.extend(loaded_addresses.readonly_keys());
        }

        writable.append(&mut readonly);
        writable
    }

    pub fn alt_addresses(&self) -> Vec<Pubkey> {
        self.0.keys().cloned().collect()
    }

    pub fn to_address_table_lookups(&self) -> Vec<MessageAddressTableLookup> {
        self.0
            .iter()
            .map(|(pubkey, loaded_addresses)| MessageAddressTableLookup {
                account_key: *pubkey,
                writable_indexes: loaded_addresses
                    .writable
                    .iter()
                    .map(|(idx, _)| *idx)
                    .collect(),
                readonly_indexes: loaded_addresses
                    .readonly
                    .iter()
                    .map(|(idx, _)| *idx)
                    .collect(),
            })
            .collect()
    }

    pub fn writable_len(&self) -> usize {
        self.0
            .values()
            .map(|loaded_addresses| loaded_addresses.writable.len())
            .sum()
    }

    pub fn readonly_len(&self) -> usize {
        self.0
            .values()
            .map(|loaded_addresses| loaded_addresses.readonly.len())
            .sum()
    }
}
