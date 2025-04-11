#![allow(dead_code)]

use std::{any::type_name, sync::Arc};

use base64::prelude::*;
use bincode::Options;
use jsonrpc_core::{Error, Result};
use litesvm::types::TransactionMetadata;
use solana_client::{
    rpc_config::RpcTokenAccountsFilter,
    rpc_custom_error::RpcCustomError,
    rpc_filter::RpcFilterType,
    rpc_request::{TokenAccountsFilter, MAX_GET_CONFIRMED_SIGNATURES_FOR_ADDRESS2_LIMIT},
};
use solana_hash::Hash;
use solana_packet::PACKET_DATA_SIZE;
use solana_pubkey::{ParsePubkeyError, Pubkey};
use solana_runtime::verify_precompiles::verify_precompiles;
use solana_signature::Signature;
use solana_transaction::sanitized::SanitizedTransaction;
use solana_transaction_status::{
    InnerInstruction, InnerInstructions, TransactionBinaryEncoding, UiInnerInstructions,
};

pub fn convert_transaction_metadata_from_canonical(
    transaction_metadata: &TransactionMetadata,
) -> surfpool_types::TransactionMetadata {
    surfpool_types::TransactionMetadata {
        signature: transaction_metadata.signature.clone(),
        logs: transaction_metadata.logs.clone(),
        inner_instructions: transaction_metadata.inner_instructions.clone(),
        compute_units_consumed: transaction_metadata.compute_units_consumed.clone(),
        return_data: transaction_metadata.return_data.clone(),
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

fn verify_transaction(
    transaction: &SanitizedTransaction,
    feature_set: &Arc<solana_feature_set::FeatureSet>,
) -> Result<()> {
    #[allow(clippy::question_mark)]
    if transaction.verify().is_err() {
        return Err(RpcCustomError::TransactionSignatureVerificationFailure.into());
    }

    let move_precompile_verification_to_svm =
        feature_set.is_active(&solana_feature_set::move_precompile_verification_to_svm::id());
    if !move_precompile_verification_to_svm {
        if let Err(e) = verify_precompiles(transaction, feature_set) {
            return Err(RpcCustomError::TransactionPrecompileVerificationFailure(e).into());
        }
    }

    Ok(())
}

fn verify_filter(input: &RpcFilterType) -> Result<()> {
    input
        .verify()
        .map_err(|e| Error::invalid_params(format!("Invalid param: {e:?}")))
}

pub fn verify_pubkey(input: &str) -> Result<Pubkey> {
    input.parse().map_err(|e: ParsePubkeyError| {
        Error::invalid_params(format!("Invalid Pubkey: {}", e.to_string()))
    })
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
    meta: &TransactionMetadata,
) -> Vec<UiInnerInstructions> {
    meta.inner_instructions
        .iter()
        .enumerate()
        .map(|(i, ixs)| {
            InnerInstructions {
                index: i as u8,
                instructions: ixs
                    .iter()
                    .map(|ix| InnerInstruction {
                        instruction: ix.instruction.clone(),
                        stack_height: Some(ix.stack_height as u32),
                    })
                    .collect(),
            }
            .into()
        })
        .collect()
}
