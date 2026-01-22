#![allow(dead_code)]

use std::any::type_name;

use base64::prelude::*;
use bincode::Options;
use jsonrpc_core::{Error, Result};
use litesvm::types::TransactionMetadata;
use solana_client::{
    rpc_config::{RpcTokenAccountsFilter, RpcTransactionConfig},
    rpc_filter::RpcFilterType,
    rpc_request::{MAX_GET_CONFIRMED_SIGNATURES_FOR_ADDRESS2_LIMIT, TokenAccountsFilter},
};
use solana_commitment_config::CommitmentConfig;
use solana_hash::Hash;
use solana_message::{AccountKeys, VersionedMessage};
use solana_packet::PACKET_DATA_SIZE;
use solana_pubkey::{ParsePubkeyError, Pubkey};
use solana_signature::Signature;
use solana_transaction_status::{
    InnerInstruction, InnerInstructions, TransactionBinaryEncoding, UiInnerInstructions,
    UiTransactionEncoding, parse_ui_inner_instructions,
};

use crate::error::{SurfpoolError, SurfpoolResult};

pub fn convert_transaction_metadata_from_canonical(
    transaction_metadata: &TransactionMetadata,
) -> surfpool_types::TransactionMetadata {
    surfpool_types::TransactionMetadata {
        signature: transaction_metadata.signature,
        logs: transaction_metadata.logs.clone(),
        inner_instructions: transaction_metadata.inner_instructions.clone(),
        compute_units_consumed: transaction_metadata.compute_units_consumed,
        return_data: transaction_metadata.return_data.clone(),
        fee: transaction_metadata.fee,
    }
}

fn optimize_filters(filters: &mut [RpcFilterType]) {
    filters.iter_mut().for_each(|filter_type| {
        if let RpcFilterType::Memcmp(compare) = filter_type {
            if let Err(err) = compare.convert_to_raw_bytes() {
                // All filters should have been previously verified
                warn!("Invalid filter: bytes could not be decoded, {err}");
            }
        }
    })
}

fn verify_filter(input: &RpcFilterType) -> Result<()> {
    input
        .verify()
        .map_err(|e| Error::invalid_params(format!("Invalid param: {e:?}")))
}

pub fn verify_pubkey(input: &str) -> SurfpoolResult<Pubkey> {
    input
        .parse()
        .map_err(|e: ParsePubkeyError| SurfpoolError::invalid_pubkey(input, e.to_string()))
}

pub fn verify_pubkeys(input: &[String]) -> SurfpoolResult<Vec<Pubkey>> {
    input
        .iter()
        .enumerate()
        .map(|(i, s)| {
            verify_pubkey(s)
                .map_err(|e| SurfpoolError::invalid_pubkey_at_index(s, i, e.to_string()))
        })
        .collect::<SurfpoolResult<Vec<_>>>()
}

fn verify_hash(input: &str) -> Result<Hash> {
    input
        .parse()
        .map_err(|e| Error::invalid_params(format!("Invalid param: {e:?}")))
}

fn verify_signature(input: &str) -> Result<Signature> {
    input
        .parse()
        .map_err(|e| Error::invalid_params(format!("Invalid param: {e:?}")))
}

fn verify_token_account_filter(
    token_account_filter: RpcTokenAccountsFilter,
) -> Result<TokenAccountsFilter> {
    match token_account_filter {
        RpcTokenAccountsFilter::Mint(mint_str) => {
            let mint = verify_pubkey(&mint_str)?;
            Ok(TokenAccountsFilter::Mint(mint))
        }
        RpcTokenAccountsFilter::ProgramId(program_id_str) => {
            let program_id = verify_pubkey(&program_id_str)?;
            Ok(TokenAccountsFilter::ProgramId(program_id))
        }
    }
}

fn verify_and_parse_signatures_for_address_params(
    address: String,
    before: Option<String>,
    until: Option<String>,
    limit: Option<usize>,
) -> Result<(Pubkey, Option<Signature>, Option<Signature>, usize)> {
    let address = verify_pubkey(&address)?;
    let before = before
        .map(|ref before| verify_signature(before))
        .transpose()?;
    let until = until.map(|ref until| verify_signature(until)).transpose()?;
    let limit = limit.unwrap_or(MAX_GET_CONFIRMED_SIGNATURES_FOR_ADDRESS2_LIMIT);

    if limit == 0 || limit > MAX_GET_CONFIRMED_SIGNATURES_FOR_ADDRESS2_LIMIT {
        return Err(Error::invalid_params(format!(
            "Invalid limit; max {MAX_GET_CONFIRMED_SIGNATURES_FOR_ADDRESS2_LIMIT}"
        )));
    }
    Ok((address, before, until, limit))
}

const MAX_BASE58_SIZE: usize = 1683; // Golden, bump if PACKET_DATA_SIZE changes
const MAX_BASE64_SIZE: usize = 1644; // Golden, bump if PACKET_DATA_SIZE changes
pub fn decode_and_deserialize<T>(
    encoded: String,
    encoding: TransactionBinaryEncoding,
) -> Result<(Vec<u8>, T)>
where
    T: serde::de::DeserializeOwned,
{
    let wire_output = match encoding {
        TransactionBinaryEncoding::Base58 => {
            if encoded.len() > MAX_BASE58_SIZE {
                return Err(Error::invalid_params(format!(
                    "base58 encoded {} too large: {} bytes (max: encoded/raw {}/{})",
                    type_name::<T>(),
                    encoded.len(),
                    MAX_BASE58_SIZE,
                    PACKET_DATA_SIZE,
                )));
            }
            bs58::decode(encoded)
                .into_vec()
                .map_err(|e| Error::invalid_params(format!("invalid base58 encoding: {e:?}")))?
        }
        TransactionBinaryEncoding::Base64 => {
            if encoded.len() > MAX_BASE64_SIZE {
                return Err(Error::invalid_params(format!(
                    "base64 encoded {} too large: {} bytes (max: encoded/raw {}/{})",
                    type_name::<T>(),
                    encoded.len(),
                    MAX_BASE64_SIZE,
                    PACKET_DATA_SIZE,
                )));
            }
            BASE64_STANDARD
                .decode(encoded)
                .map_err(|e| Error::invalid_params(format!("invalid base64 encoding: {e:?}")))?
        }
    };
    if wire_output.len() > PACKET_DATA_SIZE {
        return Err(Error::invalid_params(format!(
            "decoded {} too large: {} bytes (max: {} bytes)",
            type_name::<T>(),
            wire_output.len(),
            PACKET_DATA_SIZE
        )));
    }
    bincode::options()
        .with_limit(PACKET_DATA_SIZE as u64)
        .with_fixint_encoding()
        .allow_trailing_bytes()
        .deserialize_from(&wire_output[..])
        .map_err(|err| {
            Error::invalid_params(format!(
                "failed to deserialize {}: {}",
                type_name::<T>(),
                &err.to_string()
            ))
        })
        .map(|output| (wire_output, output))
}

pub fn transform_tx_metadata_to_ui_accounts(
    meta: TransactionMetadata,
    message: &VersionedMessage,
    loaded_addresses: Option<&solana_message::v0::LoadedAddresses>,
) -> Vec<UiInnerInstructions> {
    // Create AccountKeys from the transaction message with loaded addresses from ALTs
    let account_keys = AccountKeys::new(message.static_account_keys(), loaded_addresses);

    meta.inner_instructions
        .into_iter()
        .enumerate()
        .filter_map(|(i, ixs)| {
            let instructions: Vec<InnerInstruction> = ixs
                .iter()
                .map(|ix| InnerInstruction {
                    instruction: ix.instruction.clone(),
                    stack_height: Some(ix.stack_height as u32),
                })
                .collect();
            if instructions.is_empty() {
                None
            } else {
                // Create InnerInstructions and then parse it into UiInnerInstructions
                // This will properly convert CompiledInstruction to UiInstruction format
                let inner_instructions = InnerInstructions {
                    index: i as u8,
                    instructions,
                };
                Some(parse_ui_inner_instructions(
                    inner_instructions,
                    &account_keys,
                ))
            }
        })
        .collect()
}

/// Returns true if the error indicates the remote method is not supported.
pub fn is_method_not_supported_error<E: std::fmt::Display>(err: &E) -> bool {
    let msg = err.to_string().to_lowercase();
    msg.contains("not supported")
        || msg.contains("unsupported")
        || msg.contains("unavailable")
        || msg.contains("method blocked")
        || msg.contains("invalid request")
        || msg.contains("is blocked")
        || msg.contains("if you need this method")
        || msg.contains("client error 410")
        || msg.contains("410 gone")
        || msg.contains("(410 gone)")
        || msg.contains(" status 410")
        || msg.contains("http 410")
        || msg.contains("client error (410")
}

pub fn get_default_transaction_config() -> RpcTransactionConfig {
    RpcTransactionConfig {
        encoding: Some(UiTransactionEncoding::Json),
        commitment: Some(CommitmentConfig::default()),
        max_supported_transaction_version: Some(0),
    }
}

pub fn adjust_default_transaction_config(config: &mut RpcTransactionConfig) {
    if config.encoding.is_none() {
        config.encoding = Some(UiTransactionEncoding::Json);
    }
    if config.max_supported_transaction_version.is_none() {
        config.max_supported_transaction_version = Some(0);
    }
    if config.commitment.is_none() {
        config.commitment = Some(CommitmentConfig::default());
    }
}
