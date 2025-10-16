use std::collections::BTreeMap;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use jsonrpc_core::{BoxFuture, Error, Result, futures::future};
use jsonrpc_derive::rpc;
use solana_account::Account;
use solana_account_decoder::UiAccountEncoding;
use solana_client::rpc_response::{RpcLogsResponse, RpcResponseContext};
use solana_clock::Slot;
use solana_commitment_config::CommitmentConfig;
use solana_epoch_info::EpochInfo;
use solana_program_option::COption;
use solana_rpc_client_api::response::Response as RpcResponse;
use solana_system_interface::program as system_program;
use solana_transaction::versioned::VersionedTransaction;
use spl_associated_token_account::get_associated_token_address_with_program_id;
use surfpool_types::{
    ClockCommand, GetSurfnetInfoResponse, Idl, ResetAccountConfig, RpcProfileResultConfig,
    SimnetCommand, SimnetEvent, UiKeyedProfileResult,
    types::{AccountUpdate, SetSomeAccount, SupplyUpdate, TokenAccountUpdate, UuidOrSignature},
};

use super::{RunloopContext, SurfnetRpcContext};
use crate::{
    error::SurfpoolError,
    rpc::{
        State,
        utils::{verify_pubkey, verify_pubkeys},
    },
    surfnet::{GetAccountResult, locker::SvmAccessContext, svm::AccountFixture},
    types::{TimeTravelConfig, TokenAccount},
};

pub trait AccountUpdateExt {
    fn is_full_account_data_ext(&self) -> bool;
    fn to_account_ext(&self) -> Result<Option<Account>>;
    fn apply_ext(self, account: &mut GetAccountResult) -> Result<()>;
    fn expect_hex_data_ext(&self) -> Result<Vec<u8>>;
}

impl AccountUpdateExt for AccountUpdate {
    fn is_full_account_data_ext(&self) -> bool {
        self.lamports.is_some()
            && self.owner.is_some()
            && self.executable.is_some()
            && self.rent_epoch.is_some()
            && self.data.is_some()
    }

    fn to_account_ext(&self) -> Result<Option<Account>> {
        if self.is_full_account_data_ext() {
            Ok(Some(Account {
                lamports: self.lamports.unwrap(),
                owner: verify_pubkey(&self.owner.clone().unwrap())?,
                executable: self.executable.unwrap(),
                rent_epoch: self.rent_epoch.unwrap(),
                data: self.expect_hex_data_ext()?,
            }))
        } else {
            Ok(None)
        }
    }

    fn apply_ext(self, account_result: &mut GetAccountResult) -> Result<()> {
        account_result.apply_update(|account| {
            if let Some(lamports) = self.lamports {
                account.lamports = lamports;
            }
            if let Some(owner) = &self.owner {
                account.owner = verify_pubkey(owner)?;
            }
            if let Some(executable) = self.executable {
                account.executable = executable;
            }
            if let Some(rent_epoch) = self.rent_epoch {
                account.rent_epoch = rent_epoch;
            }
            if self.data.is_some() {
                account.data = self.expect_hex_data_ext()?;
            }
            Ok(())
        })?;
        Ok(())
    }

    fn expect_hex_data_ext(&self) -> Result<Vec<u8>> {
        let data = self.data.as_ref().expect("missing expected data field");
        hex::decode(data)
            .map_err(|e| Error::invalid_params(format!("Invalid hex data provided: {}", e)))
    }
}

pub trait TokenAccountUpdateExt {
    fn apply(self, token_account: &mut TokenAccount) -> Result<()>;
}

impl TokenAccountUpdateExt for TokenAccountUpdate {
    /// Apply the update to the account
    fn apply(self, token_account: &mut TokenAccount) -> Result<()> {
        if let Some(amount) = self.amount {
            token_account.set_amount(amount);
        }
        if let Some(delegate) = self.delegate {
            match delegate {
                SetSomeAccount::Account(pubkey) => {
                    token_account.set_delegate(COption::Some(verify_pubkey(&pubkey)?));
                }
                SetSomeAccount::NoAccount => {
                    token_account.set_delegate(COption::None);
                }
            }
        }
        if let Some(state) = self.state {
            token_account.set_state_from_str(state.as_str())?;
        }
        if let Some(delegated_amount) = self.delegated_amount {
            token_account.set_delegated_amount(delegated_amount);
        }
        if let Some(close_authority) = self.close_authority {
            match close_authority {
                SetSomeAccount::Account(pubkey) => {
                    token_account.set_close_authority(COption::Some(verify_pubkey(&pubkey)?));
                }
                SetSomeAccount::NoAccount => {
                    token_account.set_close_authority(COption::None);
                }
            }
        }
        Ok(())
    }
}

#[rpc]
pub trait SurfnetCheatcodes {
    type Metadata;

    /// A "cheat code" method for developers to set or update an account in Surfpool.
    ///
    /// This method allows developers to set or update the lamports, data, owner, executable status,
    /// and rent epoch of a given account.
    ///
    /// ## Parameters
    /// - `meta`: Metadata passed with the request, such as the client's request context.
    /// - `pubkey`: The public key of the account to be updated, as a base-58 encoded string.
    /// - `update`: The `AccountUpdate` struct containing the fields to update the account.
    ///
    /// ## Returns
    /// A `RpcResponse<()>` indicating whether the account update was successful.
    ///
    /// ## Example Request
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "method": "surfnet_setAccount",
    ///   "params": ["account_pubkey", {"lamports": 1000, "data": "base58string", "owner": "program_pubkey"}]
    /// }
    /// ```
    ///
    /// ## Example Response
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {},
    ///   "id": 1
    /// }
    /// ```
    ///
    /// # Notes
    /// This method is designed to help developers set or modify account properties within Surfpool.
    /// Developers can quickly test or update account attributes, such as lamports, program ownership, and executable status.
    ///
    /// # See Also
    /// - `getAccount`, `getAccountInfo`, `getAccountBalance`
    #[rpc(meta, name = "surfnet_setAccount")]
    fn set_account(
        &self,
        meta: Self::Metadata,
        pubkey: String,
        update: AccountUpdate,
    ) -> BoxFuture<Result<RpcResponse<()>>>;

    /// A "cheat code" method for developers to set or update a token account in Surfpool.
    ///
    /// This method allows developers to set or update various properties of a token account,
    /// including the token amount, delegate, state, delegated amount, and close authority.
    ///
    /// ## Parameters
    /// - `meta`: Metadata passed with the request, such as the client's request context.
    /// - `owner`: The base-58 encoded public key of the token account's owner.
    /// - `mint`: The base-58 encoded public key of the token mint (e.g., the token type).
    /// - `update`: The `TokenAccountUpdate` struct containing the fields to update the token account.
    /// - `token_program`: The optional base-58 encoded address of the token program (defaults to the system token program).
    ///
    /// ## Returns
    /// A `RpcResponse<()>` indicating whether the token account update was successful.
    ///
    /// ## Example Request
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "method": "surfnet_setTokenAccount",
    ///   "params": ["owner_pubkey", "mint_pubkey", {"amount": 1000, "state": "initialized"}]
    /// }
    /// ```
    ///
    /// ## Example Response
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {},
    ///   "id": 1
    /// }
    /// ```
    ///
    /// # Notes
    /// This method is designed to help developers quickly test or modify token account properties in Surfpool.
    /// Developers can update attributes such as token amounts, delegates, and authorities for specific token accounts.
    ///
    /// # See Also
    /// - `getTokenAccountInfo`, `getTokenAccountBalance`, `getTokenAccountDelegate`
    #[rpc(meta, name = "surfnet_setTokenAccount")]
    fn set_token_account(
        &self,
        meta: Self::Metadata,
        owner: String,
        mint: String,
        update: TokenAccountUpdate,
        token_program: Option<String>,
    ) -> BoxFuture<Result<RpcResponse<()>>>;

    #[rpc(meta, name = "surfnet_cloneProgramAccount")]
    fn clone_program_account(
        &self,
        meta: Self::Metadata,
        source_program_id: String,
        destination_program_id: String,
    ) -> BoxFuture<Result<RpcResponse<()>>>;

    /// Estimates the compute units that a given transaction will consume.
    ///
    /// This method simulates the transaction without committing its state changes
    /// and returns an estimation of the compute units used, along with logs and
    /// potential errors.
    ///
    /// ## Parameters
    /// - `meta`: Metadata passed with the request.
    /// - `transaction_data`: A base64 encoded string of the `VersionedTransaction`.
    /// - `tag`: An optional tag for the transaction.
    /// - `encoding`: An optional encoding for returned account data.
    ///
    /// ## Returns
    /// A `RpcResponse<ProfileResult>` containing the estimation details and a snapshot of the accounts before and after execution.
    ///
    /// ## Example Request
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "method": "surfnet_profileTransaction",
    ///   "params": ["base64_encoded_transaction_string", "optional_tag"]
    /// }
    /// ```
    #[rpc(meta, name = "surfnet_profileTransaction")]
    fn profile_transaction(
        &self,
        meta: Self::Metadata,
        transaction_data: String, // Base64 encoded VersionedTransaction
        tag: Option<String>,      // Optional tag for the transaction
        config: Option<RpcProfileResultConfig>,
    ) -> BoxFuture<Result<RpcResponse<UiKeyedProfileResult>>>;

    /// Retrieves all profiling results for a given tag.
    ///
    /// ## Parameters
    /// - `meta`: Metadata passed with the request.
    /// - `tag`: The tag to retrieve profiling results for.
    ///
    /// ## Returns
    /// A `RpcResponse<Vec<ProfileResult>>` containing the profiling results.
    #[rpc(meta, name = "surfnet_getProfileResultsByTag")]
    fn get_profile_results_by_tag(
        &self,
        meta: Self::Metadata,
        tag: String,
        config: Option<RpcProfileResultConfig>,
    ) -> Result<RpcResponse<Option<Vec<UiKeyedProfileResult>>>>;

    /// A "cheat code" method for developers to set or update the network supply information in Surfpool.
    ///
    /// This method allows developers to configure the total supply, circulating supply,
    /// non-circulating supply, and non-circulating accounts list that will be returned
    /// by the `getSupply` RPC method.
    ///
    /// ## Parameters
    /// - `meta`: Metadata passed with the request, such as the client's request context.
    /// - `update`: The `SupplyUpdate` struct containing the optional fields to update:
    ///   - `total`: Optional total supply in lamports
    ///   - `circulating`: Optional circulating supply in lamports  
    ///   - `non_circulating`: Optional non-circulating supply in lamports
    ///   - `non_circulating_accounts`: Optional list of non-circulating account addresses
    ///
    /// ## Returns
    /// A `RpcResponse<()>` indicating whether the supply update was successful.
    ///
    /// ## Example Request
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "method": "surfnet_setSupply",
    ///   "params": [{
    ///     "total": 1000000000000000,
    ///     "circulating": 800000000000000,
    ///     "non_circulating": 200000000000000,
    ///     "non_circulating_accounts": ["Account1...", "Account2..."]
    ///   }]
    /// }
    /// ```
    ///
    /// ## Example Response
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {},
    ///   "id": 1
    /// }
    /// ```
    ///
    /// # Notes
    /// This method is designed to help developers test supply-related functionality by
    /// allowing them to configure the values returned by `getSupply` without needing
    /// to connect to a real network or manipulate actual token supplies.
    ///
    /// # See Also
    /// - `getSupply`
    #[rpc(meta, name = "surfnet_setSupply")]
    fn set_supply(
        &self,
        meta: Self::Metadata,
        update: SupplyUpdate,
    ) -> BoxFuture<Result<RpcResponse<()>>>;

    /// A cheat code to set the upgrade authority of a program's ProgramData account.
    ///
    /// This method allows developers to directly patch the upgrade authority of a program's ProgramData account.
    ///
    /// ## Parameters
    /// - `meta`: Metadata passed with the request, such as the client's request context.
    /// - `program_id`: The base-58 encoded public key of the program.
    /// - `new_authority`: The base-58 encoded public key of the new authority. If omitted, the program will have no upgrade authority.
    ///
    /// ## Returns
    /// A `RpcResponse<()>` indicating whether the authority update was successful.
    ///
    /// ## Example Request
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "method": "surfnet_setProgramAuthority",
    ///   "params": [
    ///     "PROGRAM_ID_BASE58",
    ///     "NEW_AUTHORITY_BASE58"
    ///   ]
    /// }
    /// ```
    ///
    /// ## Example Response
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {},
    ///   "id": 1
    /// }
    /// ```
    #[rpc(meta, name = "surfnet_setProgramAuthority")]
    fn set_program_authority(
        &self,
        meta: Self::Metadata,
        program_id_str: String,
        new_authority_str: Option<String>,
    ) -> BoxFuture<Result<RpcResponse<()>>>;

    /// A cheat code to get the transaction profile for a given signature or UUID.
    ///
    /// ## Parameters
    /// - `meta`: Metadata passed with the request, such as the client's request context.
    /// - `signature_or_uuid`: The transaction signature (as a base-58 string) or a UUID (as a string) for which to retrieve the profile.
    ///
    /// ## Returns
    /// A `RpcResponse<Option<ProfileResult>>` containing the transaction profile if found, or `None` if not found.
    ///
    /// ## Example Request
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "method": "surfnet_getTransactionProfile",
    ///   "params": [
    ///     "5Nf3...TxSignatureOrUuidHere"
    ///   ]
    /// }
    /// ```
    ///
    /// ## Example Response
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "context": {
    ///     "slot": 355684457,
    ///     "apiVersion": "2.2.2"
    ///   },
    ///   "value": { /* ...ProfileResult object... */ },
    ///   "id": 1
    /// }
    /// ```
    #[rpc(meta, name = "surfnet_getTransactionProfile")]
    fn get_transaction_profile(
        &self,
        meta: Self::Metadata,
        signature_or_uuid: UuidOrSignature,
        config: Option<RpcProfileResultConfig>,
    ) -> Result<RpcResponse<Option<UiKeyedProfileResult>>>;

    /// A cheat code to register an IDL for a given program in memory.
    ///
    /// ## Parameters
    /// - `meta`: Metadata passed with the request, such as the client's request context.
    /// - `idl`: The full IDL object to be registered in memory. The `address` field should match the program's public key.
    /// - `slot` (optional): The slot at which to register the IDL. If omitted, uses the latest slot.
    ///
    /// ## Returns
    /// A `RpcResponse<()>` indicating whether the IDL registration was successful.
    ///
    /// ## Example Request (with slot)
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "method": "surfnet_registerIdl",
    ///   "params": [
    ///     {
    ///       "address": "4EXSeLGxVBpAZwq7vm6evLdewpcvE2H56fpqL2pPiLFa",
    ///       "metadata": {
    ///         "name": "test",
    ///         "version": "0.1.0",
    ///         "spec": "0.1.0",
    ///         "description": "Created with Anchor"
    ///       },
    ///       "instructions": [
    ///         {
    ///           "name": "initialize",
    ///           "discriminator": [175,175,109,31,13,152,155,237],
    ///           "accounts": [],
    ///           "args": []
    ///         }
    ///       ],
    ///       "accounts": [],
    ///       "types": [],
    ///       "events": [],
    ///       "errors": [],
    ///       "constants": [],
    ///       "state": null
    ///     },
    ///     355684457
    ///   ]
    /// }
    /// ```
    /// ## Example Request (without slot)
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "method": "surfnet_registerIdl",
    ///   "params": [
    ///     {
    ///       "address": "4EXSeLGxVBpAZwq7vm6evLdewpcvE2H56fpqL2pPiLFa",
    ///       "metadata": {
    ///         "name": "test",
    ///         "version": "0.1.0",
    ///         "spec": "0.1.0",
    ///         "description": "Created with Anchor"
    ///       },
    ///       "instructions": [
    ///         {
    ///           "name": "initialize",
    ///           "discriminator": [175,175,109,31,13,152,155,237],
    ///           "accounts": [],
    ///           "args": []
    ///         }
    ///       ],
    ///       "accounts": [],
    ///       "types": [],
    ///       "events": [],
    ///       "errors": [],
    ///       "constants": [],
    ///       "state": null
    ///     }
    ///   ]
    /// }
    /// ```
    ///
    /// ## Example Response
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "context": {
    ///     "slot": 355684457,
    ///     "apiVersion": "2.2.2"
    ///   },
    ///   "value": null,
    ///   "id": 1
    /// }
    /// ```
    #[rpc(meta, name = "surfnet_registerIdl")]
    fn register_idl(
        &self,
        meta: Self::Metadata,
        idl: Idl,
        slot: Option<Slot>,
    ) -> Result<RpcResponse<()>>;

    /// A cheat code to get the registered IDL for a given program ID.
    ///
    /// ## Parameters
    /// - `meta`: Metadata passed with the request, such as the client's request context.
    /// - `program_id`: The base-58 encoded public key of the program whose IDL is being requested.
    /// - `slot` (optional): The slot at which to query the IDL. If omitted, uses the latest slot.
    ///
    /// ## Returns
    /// A `RpcResponse<Option<Idl>>` containing the IDL if it exists, or `None` if not found.
    ///
    /// ## Example Request (with slot)
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "method": "surfnet_getIdl",
    ///   "params": [
    ///     "4EXSeLGxVBpAZwq7vm6evLdewpcvE2H56fpqL2pPiLFa",
    ///     355684457
    ///   ]
    /// }
    /// ```
    /// ## Example Request (without slot)
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "method": "surfnet_getIdl",
    ///   "params": [
    ///     "4EXSeLGxVBpAZwq7vm6evLdewpcvE2H56fpqL2pPiLFa"
    ///   ]
    /// }
    /// ```
    ///
    /// ## Example Response
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "context": {
    ///     "slot": 355684457,
    ///     "apiVersion": "2.2.2"
    ///   },
    ///   "value": { /* ...IDL object... */ },
    ///   "id": 1
    /// }
    /// ```
    #[rpc(meta, name = "surfnet_getActiveIdl")]
    fn get_idl(
        &self,
        meta: Self::Metadata,
        program_id: String,
        slot: Option<Slot>,
    ) -> Result<RpcResponse<Option<Idl>>>;

    /// A cheat code to get the last 50 local signatures from the local network.
    /// ## Example Request
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "method": "surfnet_getLocalSignatures",
    ///   "params": [ { "limit": 50 } ]
    /// }
    ///
    /// ## Example Response
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "signature": String,
    ///     "err": Option<TransactionError>,
    ///     "slot": u64,
    ///   },
    ///   "id": 1
    /// }
    #[rpc(meta, name = "surfnet_getLocalSignatures")]
    fn get_local_signatures(
        &self,
        meta: Self::Metadata,
        limit: Option<u64>,
    ) -> BoxFuture<Result<RpcResponse<Vec<RpcLogsResponse>>>>;

    /// A cheat code to jump forward or backward in time on the local network.
    /// Useful for testing epoch-based or time-sensitive logic.
    ///
    /// ## Parameters
    /// - `config` (optional): A `TimeTravelConfig` specifying how to modify the clock:
    ///   - `absoluteTimestamp(u64)`: Moves time to the specified UNIX timestamp.
    ///   - `absoluteSlot(u64)`: Moves to the specified absolute slot.
    ///   - `absoluteEpoch(u64)`: Advances time to the specified epoch (each epoch = 432,000 slots).
    ///
    /// ## Returns
    /// An `EpochInfo` object reflecting the updated clock state.
    ///
    /// ## Example Request
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "method": "surfnet_timeTravel",
    ///   "params": [ { "absoluteSlot": 512 } ]
    /// }
    /// ```
    ///
    /// ## Example Response
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "epoch": 512,
    ///     "slot_index": 0,
    ///     "slots_in_epoch": 432000,
    ///     "absolute_slot": 221184000,
    ///     "block_height": 650000000,
    ///     "transaction_count": 923472834
    ///   },
    ///   "id": 1
    /// }
    #[rpc(meta, name = "surfnet_timeTravel")]
    fn time_travel(
        &self,
        meta: Self::Metadata,
        config: Option<TimeTravelConfig>,
    ) -> Result<EpochInfo>;

    /// A cheat code to freeze the Surfnet clock on the local network.
    /// All time progression halts until resumed.
    ///
    /// ## Returns
    /// An `EpochInfo` object showing the current clock state at the moment of pause.
    ///
    /// ## Example Request
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "method": "surfnet_pauseClock",
    ///   "params": []
    /// }
    /// ```
    ///
    /// ## Example Response
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "epoch": 512,
    ///     "slot_index": 0,
    ///     "slots_in_epoch": 432000,
    ///     "absolute_slot": 221184000,
    ///     "block_height": 650000000,
    ///     "transaction_count": 923472834
    ///   },
    ///   "id": 1
    /// }
    /// ```
    #[rpc(meta, name = "surfnet_pauseClock")]
    fn pause_clock(&self, meta: Self::Metadata) -> Result<EpochInfo>;

    /// A cheat code to resume Solana clock progression after it was paused.
    /// The validator will start producing new slots again.
    ///
    /// ## Parameters
    ///
    /// ## Returns
    /// An `EpochInfo` object reflecting the resumed clock state.
    ///
    /// ## Example Request
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "method": "surfnet_resumeClock",
    ///   "params": []
    /// }
    /// ```
    ///
    /// ## Example Response
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "epoch": 512,
    ///     "slot_index": 0,
    ///     "slots_in_epoch": 432000,
    ///     "absolute_slot": 221184000,
    ///     "block_height": 650000000,
    ///     "transaction_count": 923472834
    ///   },
    ///   "id": 1
    /// }
    /// ```
    #[rpc(meta, name = "surfnet_resumeClock")]
    fn resume_clock(&self, meta: Self::Metadata) -> Result<EpochInfo>;

    /// A cheat code to reset an account on the local network.
    ///
    /// ## Parameters
    /// - `meta`: Metadata passed with the request, such as the client's request context.
    /// - `pubkey_str`: The base-58 encoded public key of the account to reset.
    /// - `config`: A `ResetAccountConfig` specifying how to reset the account. If omitted, the account will be reset without cascading to owned accounts.
    ///
    /// ## Returns
    /// An `RpcResponse<()>` indicating whether the account reset was successful.
    ///
    /// ## Example Request
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "method": "surfnet_resetAccount",
    ///   "params": [ "4EXSeLGxVBpAZwq7vm6evLdewpcvE2H56fpqL2pPiLFa", { "recursive": true } ]
    /// }
    /// ```
    ///
    /// ## Example Response
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "context": {
    ///       "slot": 123456789,
    ///       "apiVersion": "2.3.8"
    ///     },
    ///     "value": null
    ///   },
    ///   "id": 1
    /// }
    /// ```
    #[rpc(meta, name = "surfnet_resetAccount")]
    fn reset_account(
        &self,
        meta: Self::Metadata,
        pubkey_str: String,
        config: Option<ResetAccountConfig>,
    ) -> Result<RpcResponse<()>>;

    /// A cheat code to export all accounts as fixtures for testing.
    ///
    /// ## Parameters
    /// - `encoding` (optional): The encoding to use for account data. Defaults to `"base64"`.
    ///   - `"base64"`: Returns raw account data as base64 encoded strings
    ///   - `"jsonParsed"`: Attempts to parse known account types (tokens, programs with IDLs, etc.)
    ///
    /// ## Returns
    /// A `HashMap<String, AccountFixture>` where:
    /// - Key: The account's public key as a base-58 string
    /// - Value: An `AccountFixture` containing the account's full state
    ///
    /// ## Example Request
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "method": "surfnet_exportSnapshot",
    ///   "params": ["base64"]
    /// }
    /// ```
    ///
    /// ## Example Response
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "4EXSeLGxVBpAZwq7vm6evLdewpcvE2H56fpqL2pPiLFa": {
    ///       "pubkey": "4EXSeLGxVBpAZwq7vm6evLdewpcvE2H56fpqL2pPiLFa",
    ///       "lamports": 1000000,
    ///       "owner": "11111111111111111111111111111111",
    ///       "executable": false,
    ///       "rentEpoch": 0,
    ///       "data": ["...", "base64"]
    ///     }
    ///   },
    ///   "id": 1
    /// }
    /// ```
    ///
    #[rpc(meta, name = "surfnet_exportSnapshot")]
    fn export_account_fixtures(
        &self,
        meta: Self::Metadata,
        encoding: Option<UiAccountEncoding>,
    ) -> Result<BTreeMap<String, AccountFixture>>;

    /// A cheat code to get Surfnet network information.
    ///
    /// ## Parameters
    /// - `meta`: Metadata passed with the request, such as the client's request context.
    ///
    /// ## Returns
    /// A `RpcResponse<GetSurfnetInfoResponse>` containing the Surfnet network information.
    ///
    /// ## Example Request
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 1,
    ///   "method": "surfnet_getSurfnetInfo"
    /// }
    /// ```
    ///
    /// ## Example Response
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "context": {
    ///       "slot": 369027326,
    ///       "apiVersion": "2.3.8"
    ///     },
    ///     "value": {
    ///       "runbookExecutions": [
    ///         {
    ///           "startedAt": 1758747828,
    ///           "completedAt": 1758747828,
    ///           "runbookId": "deployment"
    ///         }
    ///       ]
    ///     }
    ///   },
    ///   "id": 1
    /// }
    /// ```
    ///
    #[rpc(meta, name = "surfnet_getSurfnetInfo")]
    fn get_surfnet_info(&self, meta: Self::Metadata)
    -> Result<RpcResponse<GetSurfnetInfoResponse>>;
}

#[derive(Clone)]
pub struct SurfnetCheatcodesRpc;
impl SurfnetCheatcodes for SurfnetCheatcodesRpc {
    type Metadata = Option<RunloopContext>;

    fn set_account(
        &self,
        meta: Self::Metadata,
        pubkey_str: String,
        update: AccountUpdate,
    ) -> BoxFuture<Result<RpcResponse<()>>> {
        let pubkey = match verify_pubkey(&pubkey_str) {
            Ok(res) => res,
            Err(e) => return e.into(),
        };
        let account_update_opt = match update.to_account_ext() {
            Err(e) => return Box::pin(future::err(e)),
            Ok(res) => res,
        };

        let SurfnetRpcContext {
            svm_locker,
            remote_ctx,
        } = match meta.get_rpc_context(CommitmentConfig::confirmed()) {
            Ok(res) => res,
            Err(e) => return e.into(),
        };

        Box::pin(async move {
            let (account_to_set, latest_absolute_slot) = if let Some(account) = account_update_opt {
                (
                    GetAccountResult::FoundAccount(pubkey, account, true),
                    svm_locker.get_latest_absolute_slot(),
                )
            } else {
                // otherwise, we need to fetch the account and apply the update
                let SvmAccessContext {
                    slot, inner: mut account_result_to_update,
                    ..
                } = svm_locker.get_account(&remote_ctx, &pubkey, Some(Box::new(move |svm_locker| {

                            // if the account does not exist locally or in the remote, create a new account with default values
                            let _ = svm_locker.simnet_events_tx().send(SimnetEvent::info(format!(
                                "Account {pubkey} not found, creating a new account from default values"
                            )));
                            GetAccountResult::FoundAccount(
                                pubkey,
                                solana_account::Account {
                                    lamports: 0,
                                    owner: system_program::id(),
                                    executable: false,
                                    rent_epoch: 0,
                                    data: vec![],
                                },
                                true, // indicate that the account should be updated in the SVM, since it's new
                            )
                }))).await?;

                update.apply_ext(&mut account_result_to_update)?;
                (account_result_to_update, slot)
            };

            svm_locker.write_account_update(account_to_set);

            Ok(RpcResponse {
                context: RpcResponseContext::new(latest_absolute_slot),
                value: (),
            })
        })
    }

    fn set_token_account(
        &self,
        meta: Self::Metadata,
        owner_str: String,
        mint_str: String,
        update: TokenAccountUpdate,
        some_token_program_str: Option<String>,
    ) -> BoxFuture<Result<RpcResponse<()>>> {
        let owner = match verify_pubkey(&owner_str) {
            Ok(res) => res,
            Err(e) => return e.into(),
        };

        let mint = match verify_pubkey(&mint_str) {
            Ok(res) => res,
            Err(e) => return e.into(),
        };

        let token_program_id = match some_token_program_str {
            Some(token_program_str) => match verify_pubkey(&token_program_str) {
                Ok(res) => res,
                Err(e) => return e.into(),
            },
            None => spl_token::id(),
        };

        let associated_token_account =
            get_associated_token_address_with_program_id(&owner, &mint, &token_program_id);

        let SurfnetRpcContext {
            svm_locker,
            remote_ctx,
        } = match meta.get_rpc_context(CommitmentConfig::confirmed()) {
            Ok(res) => res,
            Err(e) => return e.into(),
        };

        Box::pin(async move {
            let get_mint_result = svm_locker
                .get_account(&remote_ctx, &mint, None)
                .await?
                .inner;
            svm_locker.write_account_update(get_mint_result);

            let SvmAccessContext {
                slot,
                inner: mut token_account,
                ..
            } = svm_locker
                .get_account(
                    &remote_ctx,
                    &associated_token_account,
                    Some(Box::new(move |svm_locker| {
                        let minimum_rent = svm_locker.with_svm_reader(|svm_reader| {
                            svm_reader.inner.minimum_balance_for_rent_exemption(
                                TokenAccount::get_packed_len_for_token_program_id(
                                    &token_program_id,
                                ),
                            )
                        });

                        let default = TokenAccount::new(&token_program_id, owner, mint);
                        let data = default.pack_into_vec();
                        GetAccountResult::FoundAccount(
                            associated_token_account,
                            Account {
                                lamports: minimum_rent,
                                owner: token_program_id,
                                executable: false,
                                rent_epoch: 0,
                                data,
                            },
                            true, // indicate that the account should be updated in the SVM, since it's new
                        )
                    })),
                )
                .await?;

            let mut token_account_data = TokenAccount::unpack(token_account.expected_data())
                .map_err(|e| {
                    Error::invalid_params(format!("Failed to unpack token account data: {}", e))
                })?;

            update.apply(&mut token_account_data)?;

            let final_account_bytes = token_account_data.pack_into_vec();
            token_account.apply_update(|account| {
                account.data = final_account_bytes.clone();
                Ok(())
            })?;
            svm_locker.write_account_update(token_account);

            Ok(RpcResponse {
                context: RpcResponseContext::new(slot),
                value: (),
            })
        })
    }

    /// Clones a program account from one program ID to another.
    /// A program account contains a pointer to a program data account, which is a PDA derived from the program ID.
    /// So, when cloning a program account, we need to clone the program data account as well.
    ///
    /// This method will:
    ///  1. Get the program account for the source program ID.
    ///  2. Get the program data account for the source program ID.
    ///  3. Calculate the program data address for the destination program ID.
    ///  4. Set the destination program account's data to point to the calculated destination program address.
    ///  5. Copy the source program data account to the destination program data account.
    fn clone_program_account(
        &self,
        meta: Self::Metadata,
        source_program_id: String,
        destination_program_id: String,
    ) -> BoxFuture<Result<RpcResponse<()>>> {
        let source_program_id = match verify_pubkey(&source_program_id) {
            Ok(res) => res,
            Err(e) => return e.into(),
        };
        let destination_program_id = match verify_pubkey(&destination_program_id) {
            Ok(res) => res,
            Err(e) => return e.into(),
        };

        let SurfnetRpcContext {
            svm_locker,
            remote_ctx,
        } = match meta.get_rpc_context(CommitmentConfig::confirmed()) {
            Ok(res) => res,
            Err(e) => return e.into(),
        };

        Box::pin(async move {
            let SvmAccessContext { slot, .. } = svm_locker
                .clone_program_account(&remote_ctx, &source_program_id, &destination_program_id)
                .await?;

            Ok(RpcResponse {
                context: RpcResponseContext::new(slot),
                value: (),
            })
        })
    }

    fn profile_transaction(
        &self,
        meta: Self::Metadata,
        transaction_data_b64: String,
        tag: Option<String>,
        config: Option<RpcProfileResultConfig>,
    ) -> BoxFuture<Result<RpcResponse<UiKeyedProfileResult>>> {
        Box::pin(async move {
            let transaction_bytes = STANDARD
                .decode(&transaction_data_b64)
                .map_err(|e| SurfpoolError::invalid_base64_data("transaction", e))?;
            let transaction: VersionedTransaction = bincode::deserialize(&transaction_bytes)
                .map_err(|e| SurfpoolError::deserialize_error("transaction", e))?;

            let SurfnetRpcContext {
                svm_locker,
                remote_ctx,
            } = meta.get_rpc_context(CommitmentConfig::confirmed())?;

            let SvmAccessContext {
                slot, inner: uuid, ..
            } = svm_locker
                .profile_transaction(&remote_ctx, transaction, tag.clone())
                .await?;

            let key = UuidOrSignature::Uuid(uuid);

            let config = config.unwrap_or_default();
            let ui_result = svm_locker
                .get_profile_result(key, &config)?
                .ok_or(SurfpoolError::expected_profile_not_found(&key))?;

            Ok(RpcResponse {
                context: RpcResponseContext::new(slot),
                value: ui_result,
            })
        })
    }

    fn get_profile_results_by_tag(
        &self,
        meta: Self::Metadata,
        tag: String,
        config: Option<RpcProfileResultConfig>,
    ) -> Result<RpcResponse<Option<Vec<UiKeyedProfileResult>>>> {
        let config = config.unwrap_or_default();
        let svm_locker = meta.get_svm_locker()?;
        let profiles = svm_locker.get_profile_results_by_tag(tag, &config)?;
        let slot = svm_locker.get_latest_absolute_slot();
        Ok(RpcResponse {
            context: RpcResponseContext::new(slot),
            value: profiles,
        })
    }

    fn set_supply(
        &self,
        meta: Self::Metadata,
        update: SupplyUpdate,
    ) -> BoxFuture<Result<RpcResponse<()>>> {
        let svm_locker = match meta.get_svm_locker() {
            Ok(locker) => locker,
            Err(e) => return e.into(),
        };

        // validate non-circulating accounts are valid pubkeys
        if let Some(ref non_circulating_accounts) = update.non_circulating_accounts {
            if let Err(e) = verify_pubkeys(non_circulating_accounts) {
                return e.into();
            }
        }

        Box::pin(async move {
            let latest_absolute_slot = svm_locker.with_svm_writer(|svm_writer| {
                // update the supply fields if provided
                if let Some(total) = update.total {
                    svm_writer.total_supply = total;
                }

                if let Some(circulating) = update.circulating {
                    svm_writer.circulating_supply = circulating;
                }

                if let Some(non_circulating) = update.non_circulating {
                    svm_writer.non_circulating_supply = non_circulating;
                }

                if let Some(ref accounts) = update.non_circulating_accounts {
                    svm_writer.non_circulating_accounts = accounts.clone();
                }

                svm_writer.get_latest_absolute_slot()
            });

            Ok(RpcResponse {
                context: RpcResponseContext::new(latest_absolute_slot),
                value: (),
            })
        })
    }

    fn set_program_authority(
        &self,
        meta: Self::Metadata,
        program_id_str: String,
        new_authority_str: Option<String>,
    ) -> BoxFuture<Result<RpcResponse<()>>> {
        let program_id = match verify_pubkey(&program_id_str) {
            Ok(res) => res,
            Err(e) => return e.into(),
        };
        let new_authority = if let Some(ref new_authority_str) = new_authority_str {
            match verify_pubkey(new_authority_str) {
                Ok(res) => Some(res),
                Err(e) => return e.into(),
            }
        } else {
            None
        };

        let SurfnetRpcContext {
            svm_locker,
            remote_ctx,
        } = match meta.get_rpc_context(CommitmentConfig::confirmed()) {
            Ok(res) => res,
            Err(e) => return e.into(),
        };
        Box::pin(async move {
            let SvmAccessContext { slot, .. } = svm_locker
                .set_program_authority(&remote_ctx, program_id, new_authority)
                .await?;

            Ok(RpcResponse {
                context: RpcResponseContext::new(slot),
                value: (),
            })
        })
    }

    fn get_transaction_profile(
        &self,
        meta: Self::Metadata,
        signature_or_uuid: UuidOrSignature,
        config: Option<RpcProfileResultConfig>,
    ) -> Result<RpcResponse<Option<UiKeyedProfileResult>>> {
        let config = config.unwrap_or_default();
        let svm_locker = meta.get_svm_locker()?;
        let profile_result = svm_locker.get_profile_result(signature_or_uuid, &config)?;
        let context_slot = profile_result
            .as_ref()
            .map(|pr| pr.slot)
            .unwrap_or_else(|| svm_locker.get_latest_absolute_slot());
        Ok(RpcResponse {
            context: RpcResponseContext::new(context_slot),
            value: profile_result,
        })
    }

    fn register_idl(
        &self,
        meta: Self::Metadata,
        idl: Idl,
        slot: Option<Slot>,
    ) -> Result<RpcResponse<()>> {
        let svm_locker = match meta.get_svm_locker() {
            Ok(locker) => locker,
            Err(e) => return Err(e.into()),
        };
        svm_locker.register_idl(idl, slot);
        Ok(RpcResponse {
            context: RpcResponseContext::new(svm_locker.get_latest_absolute_slot()),
            value: (),
        })
    }

    fn get_idl(
        &self,
        meta: Self::Metadata,
        program_id: String,
        slot: Option<Slot>,
    ) -> Result<RpcResponse<Option<Idl>>> {
        let svm_locker = match meta.get_svm_locker() {
            Ok(locker) => locker,
            Err(e) => return Err(e.into()),
        };
        let program_id = match verify_pubkey(&program_id) {
            Ok(pk) => pk,
            Err(e) => return Err(e.into()),
        };
        let idl = svm_locker.get_idl(&program_id, slot);
        let slot = slot.unwrap_or_else(|| svm_locker.get_latest_absolute_slot());
        Ok(RpcResponse {
            context: RpcResponseContext::new(slot),
            value: idl,
        })
    }

    fn get_local_signatures(
        &self,
        meta: Self::Metadata,
        limit: Option<u64>,
    ) -> BoxFuture<Result<RpcResponse<Vec<RpcLogsResponse>>>> {
        let svm_locker = match meta.get_svm_locker() {
            Ok(locker) => locker,
            Err(e) => return e.into(),
        };

        let limit = limit.unwrap_or(50);
        let latest = svm_locker.get_latest_absolute_slot();
        if limit == 0 {
            return Box::pin(async move {
                Ok(RpcResponse {
                    context: RpcResponseContext::new(latest),
                    value: Vec::new(),
                })
            });
        }

        let mut items: Vec<(
            String,
            Slot,
            Option<solana_transaction_error::TransactionError>,
            Vec<String>,
        )> = svm_locker.with_svm_reader(|svm_reader| {
            svm_reader
                .transactions
                .iter()
                .map(|(sig, status)| {
                    let (transaction_with_status_meta, _) = status.expect_processed();
                    (
                        sig.to_string(),
                        transaction_with_status_meta.slot,
                        transaction_with_status_meta.meta.status.clone().err(),
                        transaction_with_status_meta
                            .meta
                            .log_messages
                            .clone()
                            .unwrap_or_default(),
                    )
                })
                .collect()
        });

        items.sort_by(|a, b| b.1.cmp(&a.1));
        items.truncate(limit as usize);

        let value: Vec<RpcLogsResponse> = items
            .into_iter()
            .map(|(signature, _slot, err, logs)| RpcLogsResponse {
                signature,
                err,
                logs,
            })
            .collect();

        Box::pin(async move {
            Ok(RpcResponse {
                context: RpcResponseContext::new(latest),
                value,
            })
        })
    }

    fn pause_clock(&self, meta: Self::Metadata) -> Result<EpochInfo> {
        let key = meta.as_ref().map(|ctx| ctx.id.clone()).unwrap_or_default();
        let surfnet_command_tx: crossbeam_channel::Sender<SimnetCommand> =
            meta.get_surfnet_command_tx()?;
        let _ = surfnet_command_tx.send(SimnetCommand::CommandClock(key, ClockCommand::Pause));
        meta.with_svm_reader(|svm_reader| svm_reader.latest_epoch_info.clone())
            .map_err(Into::into)
    }

    fn resume_clock(&self, meta: Self::Metadata) -> Result<EpochInfo> {
        let key = meta.as_ref().map(|ctx| ctx.id.clone()).unwrap_or_default();
        let surfnet_command_tx: crossbeam_channel::Sender<SimnetCommand> =
            meta.get_surfnet_command_tx()?;
        let _ = surfnet_command_tx.send(SimnetCommand::CommandClock(key, ClockCommand::Resume));
        meta.with_svm_reader(|svm_reader| svm_reader.latest_epoch_info.clone())
            .map_err(Into::into)
    }

    fn time_travel(
        &self,
        meta: Self::Metadata,
        config: Option<TimeTravelConfig>,
    ) -> Result<EpochInfo> {
        let key = meta.as_ref().map(|ctx| ctx.id.clone()).unwrap_or_default();
        let time_travel_config = config.unwrap_or_default();
        let simnet_command_tx = meta.get_surfnet_command_tx()?;
        let svm_locker = meta.get_svm_locker()?;

        let epoch_info = svm_locker.time_travel(key, simnet_command_tx, time_travel_config)?;

        Ok(epoch_info)
    }

    fn reset_account(
        &self,
        meta: Self::Metadata,
        pubkey: String,
        config: Option<ResetAccountConfig>,
    ) -> Result<RpcResponse<()>> {
        let svm_locker = meta.get_svm_locker()?;
        let pubkey = verify_pubkey(&pubkey)?;
        svm_locker.reset_account(pubkey, config.unwrap_or_default())?;
        Ok(RpcResponse {
            context: RpcResponseContext::new(svm_locker.get_latest_absolute_slot()),
            value: (),
        })
    }

    fn get_surfnet_info(
        &self,
        meta: Self::Metadata,
    ) -> Result<RpcResponse<GetSurfnetInfoResponse>> {
        let svm_locker = meta.get_svm_locker()?;
        let runbook_executions = svm_locker.runbook_executions();
        Ok(RpcResponse {
            context: RpcResponseContext::new(svm_locker.get_latest_absolute_slot()),
            value: GetSurfnetInfoResponse::new(runbook_executions),
        })
    }

    fn export_account_fixtures(
        &self,
        meta: Self::Metadata,
        encoding: Option<UiAccountEncoding>,
    ) -> Result<BTreeMap<String, AccountFixture>> {
        let encoding = encoding.unwrap_or(UiAccountEncoding::Base64);
        meta.with_svm_reader(|svm_reader| svm_reader.export_accounts_as_fixtures(encoding))
            .map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use solana_account_decoder::{
        UiAccountData, UiAccountEncoding, parse_account_data::ParsedAccount,
    };
    use solana_keypair::Keypair;
    use solana_program_pack::Pack;
    use solana_signer::Signer;
    use solana_system_interface::instruction::create_account;
    use solana_transaction::Transaction;
    use spl_associated_token_account::{
        get_associated_token_address_with_program_id, instruction::create_associated_token_account,
    };
    use spl_token::state::Mint;
    use spl_token_2022::instruction::{initialize_mint2, mint_to, transfer_checked};
    use surfpool_types::{RpcProfileDepth, UiAccountChange, UiAccountProfileState};

    use super::*;
    use crate::{rpc::surfnet_cheatcodes::SurfnetCheatcodesRpc, tests::helpers::TestSetup};

    #[tokio::test(flavor = "multi_thread")]
    async fn test_get_transaction_profile() {
        // Create connection to local validator
        let client = TestSetup::new(SurfnetCheatcodesRpc);
        let recent_blockhash = client
            .context
            .svm_locker
            .with_svm_reader(|svm_reader| svm_reader.latest_blockhash());

        // Generate a new keypair for the fee payer
        let payer = Keypair::new();

        let owner = Keypair::new();

        // Generate a second keypair for the token recipient
        let recipient = Keypair::new();

        // Airdrop 1 SOL to fee payer
        client
            .context
            .svm_locker
            .airdrop(&payer.pubkey(), 1_000_000_000)
            .unwrap();

        // Airdrop 1 SOL to recipient for rent exemption
        client
            .context
            .svm_locker
            .airdrop(&recipient.pubkey(), 1_000_000_000)
            .unwrap();

        // Generate keypair to use as address of mint
        let mint = Keypair::new();

        // Get default mint account size (in bytes), no extensions enabled
        let mint_space = Mint::LEN;
        let mint_rent = client.context.svm_locker.with_svm_reader(|svm_reader| {
            svm_reader
                .inner
                .minimum_balance_for_rent_exemption(mint_space)
        });

        // Instruction to create new account for mint (token 2022 program)
        let create_account_instruction = create_account(
            &payer.pubkey(),       // payer
            &mint.pubkey(),        // new account (mint)
            mint_rent,             // lamports
            mint_space as u64,     // space
            &spl_token_2022::id(), // program id
        );

        // Instruction to initialize mint account data
        let initialize_mint_instruction = initialize_mint2(
            &spl_token_2022::id(),
            &mint.pubkey(),        // mint
            &payer.pubkey(),       // mint authority
            Some(&payer.pubkey()), // freeze authority
            2,                     // decimals
        )
        .unwrap();

        // Calculate the associated token account address for fee_payer
        let source_ata = get_associated_token_address_with_program_id(
            &owner.pubkey(),       // owner
            &mint.pubkey(),        // mint
            &spl_token_2022::id(), // program_id
        );

        // Instruction to create associated token account for fee_payer
        let create_source_ata_instruction = create_associated_token_account(
            &payer.pubkey(),       // funding address
            &owner.pubkey(),       // wallet address
            &mint.pubkey(),        // mint address
            &spl_token_2022::id(), // program id
        );

        // Calculate the associated token account address for recipient
        let destination_ata = get_associated_token_address_with_program_id(
            &recipient.pubkey(),   // owner
            &mint.pubkey(),        // mint
            &spl_token_2022::id(), // program_id
        );

        // Instruction to create associated token account for recipient
        let create_destination_ata_instruction = create_associated_token_account(
            &payer.pubkey(),       // funding address
            &recipient.pubkey(),   // wallet address
            &mint.pubkey(),        // mint address
            &spl_token_2022::id(), // program id
        );

        // Amount of tokens to mint (100 tokens with 2 decimal places)
        let amount = 100_00;

        // Create mint_to instruction to mint tokens to the source token account
        let mint_to_instruction = mint_to(
            &spl_token_2022::id(),
            &mint.pubkey(),     // mint
            &source_ata,        // destination
            &payer.pubkey(),    // authority
            &[&payer.pubkey()], // signer
            amount,             // amount
        )
        .unwrap();

        // Create transaction and add instructions
        let transaction = Transaction::new_signed_with_payer(
            &[
                create_account_instruction,
                initialize_mint_instruction,
                create_source_ata_instruction,
                create_destination_ata_instruction,
                mint_to_instruction,
            ],
            Some(&payer.pubkey()),
            &[&payer, &mint],
            recent_blockhash,
        );

        let signature = transaction.signatures[0];

        let (status_tx, _status_rx) = crossbeam_channel::unbounded();
        client
            .context
            .svm_locker
            .process_transaction(&None, transaction.into(), status_tx.clone(), false, true)
            .await
            .unwrap();

        // get profile and verify data
        {
            let ui_profile_result = client
                .rpc
                .get_transaction_profile(
                    Some(client.context.clone()),
                    UuidOrSignature::Signature(signature),
                    Some(RpcProfileResultConfig {
                        depth: Some(RpcProfileDepth::Instruction),
                        ..Default::default()
                    }),
                )
                .unwrap()
                .value
                .expect("missing profile result for processed transaction");

            // instruction 1: create_account
            {
                let ix_profile = ui_profile_result
                    .instruction_profiles
                    .as_ref()
                    .unwrap()
                    .get(0)
                    .expect("instruction profile should exist");
                assert!(
                    ix_profile.error_message.is_none(),
                    "Profile should succeed, found error: {}",
                    ix_profile.error_message.as_ref().unwrap()
                );
                assert_eq!(ix_profile.compute_units_consumed, 150);
                assert!(ix_profile.error_message.is_none());
                let account_states = &ix_profile.account_states;

                let UiAccountProfileState::Writable(sender_account_change) = account_states
                    .get(&payer.pubkey())
                    .expect("Payer account state should be present")
                else {
                    panic!("Expected account state to be Writable");
                };

                match sender_account_change {
                    UiAccountChange::Update(before, after) => {
                        assert_eq!(
                            after.lamports,
                            before.lamports - mint_rent - (2 * 5000), // two signers, so 2 * 5000 for fees
                            "Payer account should be original balance minus rent"
                        );
                    }
                    other => {
                        panic!("Expected account state to be an Update, got: {:?}", other);
                    }
                }

                let UiAccountProfileState::Writable(mint_account_change) = account_states
                    .get(&mint.pubkey())
                    .expect("Mint account state should be present")
                else {
                    panic!("Expected mint account state to be Writable");
                };
                match mint_account_change {
                    UiAccountChange::Create(mint_account) => {
                        assert_eq!(
                            mint_account.lamports, mint_rent,
                            "Mint account should have the correct rent amount"
                        );
                        assert_eq!(
                            mint_account.owner,
                            spl_token_2022::id().to_string(),
                            "Mint account should be owned by the SPL Token program"
                        );
                        // initialized account data should be empty bytes
                        assert_eq!(
                        mint_account.data,
                        UiAccountData::Binary(
                            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA==".into(),
                            UiAccountEncoding::Base64
                        ),
                    );
                    }
                    other => {
                        panic!("Expected account state to be an Update, got: {:?}", other);
                    }
                }
            }

            // instruction 2: initialize mint
            {
                let ix_profile = ui_profile_result
                    .instruction_profiles
                    .as_ref()
                    .unwrap()
                    .get(1)
                    .expect("instruction profile should exist");

                assert!(
                    ix_profile.error_message.is_none(),
                    "Profile should succeed, found error: {}",
                    ix_profile.error_message.as_ref().unwrap()
                );
                assert_eq!(ix_profile.compute_units_consumed, 1031);
                let account_states = &ix_profile.account_states;

                assert!(account_states.get(&payer.pubkey()).is_none());

                let UiAccountProfileState::Writable(mint_account_change) = account_states
                    .get(&mint.pubkey())
                    .expect("Mint account state should be present")
                else {
                    panic!("Expected mint account state to be Writable");
                };
                match mint_account_change {
                    UiAccountChange::Update(_before, after) => {
                        assert_eq!(
                            after.lamports, mint_rent,
                            "Mint account should have the correct rent amount"
                        );
                        assert_eq!(
                            after.owner,
                            spl_token_2022::id().to_string(),
                            "Mint account should be owned by the SPL Token program"
                        );
                        // initialized account data should be empty bytes
                        assert_eq!(
                            after.data,
                            UiAccountData::Json(ParsedAccount {
                                program: "spl-token-2022".to_string(),
                                parsed: json!({
                                    "info": {
                                        "decimals": 2,
                                        "freezeAuthority": payer.pubkey().to_string(),
                                        "mintAuthority": payer.pubkey().to_string(),
                                        "isInitialized": true,
                                        "supply": "0",
                                    },
                                    "type": "mint"
                                }),
                                space: 82,
                            }),
                        );
                    }
                    other => {
                        panic!("Expected account state to be an Update, got: {:?}", other);
                    }
                }
            }

            // instruction 3: create token account
            {
                let ix_profile = ui_profile_result
                    .instruction_profiles
                    .as_ref()
                    .unwrap()
                    .get(2)
                    .expect("instruction profile should exist");
                assert!(
                    ix_profile.error_message.is_none(),
                    "Profile should succeed, found error: {}",
                    ix_profile.error_message.as_ref().unwrap()
                );

                let account_states = &ix_profile.account_states;

                let UiAccountProfileState::Writable(sender_account_change) = account_states
                    .get(&payer.pubkey())
                    .expect("Payer account state should be present")
                else {
                    panic!("Expected account state to be Writable");
                };

                match sender_account_change {
                    UiAccountChange::Update(before, after) => {
                        assert_eq!(
                            after.lamports,
                            before.lamports - 2074080,
                            "Payer account should be original balance minus rent"
                        );
                    }
                    other => {
                        panic!("Expected account state to be an Update, got: {:?}", other);
                    }
                }

                let UiAccountProfileState::Writable(mint_account_change) = account_states
                    .get(&mint.pubkey())
                    .expect("Mint account state should be present")
                else {
                    panic!("Expected mint account state to be Writable");
                };
                match mint_account_change {
                    UiAccountChange::Unchanged(mint_account) => {
                        assert!(mint_account.is_some())
                    }
                    other => {
                        panic!("Expected account state to be Unchanged, got: {:?}", other);
                    }
                }

                let UiAccountProfileState::Writable(source_ata_change) = account_states
                    .get(&source_ata)
                    .expect("account state should be present")
                else {
                    panic!("Expected account state to be Writable");
                };

                match source_ata_change {
                    UiAccountChange::Create(new) => {
                        assert_eq!(
                            new.lamports, 2074080,
                            "Source ATA should have the correct lamports after creation"
                        );
                        assert_eq!(
                            new.owner,
                            spl_token_2022::id().to_string(),
                            "Source ATA should be owned by the SPL Token program"
                        );

                        match &new.data {
                            UiAccountData::Json(parsed) => {
                                assert_eq!(
                                    parsed,
                                    &ParsedAccount {
                                        program: "spl-token-2022".into(),
                                        parsed: json!({
                                            "info": {
                                                "extensions": [
                                                    {
                                                        "extension": "immutableOwner"
                                                    }
                                                ],
                                                "isNative": false,
                                                "mint": mint.pubkey().to_string(),
                                                "owner": owner.pubkey().to_string(),
                                                "state": "initialized",
                                                "tokenAmount": {
                                                    "amount": "0",
                                                    "decimals": 2,
                                                    "uiAmount": 0.0,
                                                    "uiAmountString": "0"
                                                }
                                            },
                                            "type": "account"
                                        }),
                                        space: 170
                                    }
                                );
                            }
                            _ => panic!("Expected source ATA data to be JSON"),
                        }
                    }
                    other => {
                        panic!("Expected account state to be Create, got: {:?}", other);
                    }
                }
            }

            // instruction 4: create destination ATA
            {
                let ix_profile = ui_profile_result
                    .instruction_profiles
                    .as_ref()
                    .unwrap()
                    .get(3)
                    .expect("instruction profile should exist");
                assert!(
                    ix_profile.error_message.is_none(),
                    "Profile should succeed, found error: {}",
                    ix_profile.error_message.as_ref().unwrap()
                );

                let account_states = &ix_profile.account_states;

                let UiAccountProfileState::Writable(sender_account_change) = account_states
                    .get(&payer.pubkey())
                    .expect("Payer account state should be present")
                else {
                    panic!("Expected account state to be Writable");
                };

                match sender_account_change {
                    UiAccountChange::Update(before, after) => {
                        assert_eq!(
                            after.lamports,
                            before.lamports - 2074080,
                            "Payer account should be original balance minus rent"
                        );
                    }
                    other => {
                        panic!("Expected account state to be an Update, got: {:?}", other);
                    }
                }

                let UiAccountProfileState::Writable(mint_account_change) = account_states
                    .get(&mint.pubkey())
                    .expect("Mint account state should be present")
                else {
                    panic!("Expected mint account state to be Writable");
                };
                match mint_account_change {
                    UiAccountChange::Unchanged(mint_account) => {
                        assert!(mint_account.is_some())
                    }
                    other => {
                        panic!("Expected account state to be Unchanged, got: {:?}", other);
                    }
                }

                let UiAccountProfileState::Writable(destination_ata_change) = account_states
                    .get(&destination_ata)
                    .expect("account state should be present")
                else {
                    panic!("Expected account state to be Writable");
                };

                match destination_ata_change {
                    UiAccountChange::Create(new) => {
                        assert_eq!(
                            new.lamports, 2074080,
                            "Source ATA should have the correct lamports after creation"
                        );
                        assert_eq!(
                            new.owner,
                            spl_token_2022::id().to_string(),
                            "Source ATA should be owned by the SPL Token program"
                        );
                        match &new.data {
                            UiAccountData::Json(parsed) => {
                                assert_eq!(
                                    parsed,
                                    &ParsedAccount {
                                        program: "spl-token-2022".into(),
                                        parsed: json!({
                                            "info": {
                                                "extensions": [
                                                    {
                                                        "extension": "immutableOwner"
                                                    }
                                                ],
                                                "isNative": false,
                                                "mint": mint.pubkey().to_string(),
                                                "owner": recipient.pubkey().to_string(),
                                                "state": "initialized",
                                                "tokenAmount": {
                                                    "amount": "0",
                                                    "decimals": 2,
                                                    "uiAmount": 0.0,
                                                    "uiAmountString": "0"
                                                }
                                            },
                                            "type": "account"
                                        }),
                                        space: 170
                                    }
                                );
                            }
                            _ => panic!("Expected source ATA data to be JSON"),
                        }
                    }
                    other => {
                        panic!("Expected account state to be Create, got: {:?}", other);
                    }
                }
            }

            // instruction 5: mint to
            {
                let ix_profile = ui_profile_result
                    .instruction_profiles
                    .as_ref()
                    .unwrap()
                    .get(4)
                    .expect("instruction profile should exist");
                assert!(
                    ix_profile.error_message.is_none(),
                    "Profile should succeed, found error: {}",
                    ix_profile.error_message.as_ref().unwrap()
                );

                let account_states = &ix_profile.account_states;

                let UiAccountProfileState::Writable(sender_account_change) = account_states
                    .get(&payer.pubkey())
                    .expect("Payer account state should be present")
                else {
                    panic!("Expected account state to be Writable");
                };

                match sender_account_change {
                    UiAccountChange::Unchanged(unchanged) => {
                        assert!(unchanged.is_some(), "Payer account should remain unchanged");
                    }
                    other => {
                        panic!("Expected account state to be Unchanged, got: {:?}", other);
                    }
                }

                let UiAccountProfileState::Writable(mint_account_change) = account_states
                    .get(&mint.pubkey())
                    .expect("Mint account state should be present")
                else {
                    panic!("Expected mint account state to be Writable");
                };
                match mint_account_change {
                    UiAccountChange::Update(before, after) => {
                        assert_eq!(
                            after.lamports, before.lamports,
                            "Lamports should stay the same for mint account"
                        );
                        assert_eq!(
                            after.data,
                            UiAccountData::Json(ParsedAccount {
                                program: "spl-token-2022".into(),
                                parsed: json!({
                                    "info": {
                                        "decimals": 2,
                                        "freezeAuthority": payer.pubkey().to_string(),
                                        "isInitialized": true,
                                        "mintAuthority": payer.pubkey().to_string(),
                                        "supply": "10000",
                                    },
                                    "type": "mint"
                                }),
                                space: 82
                            }),
                            "Data should stay the same for mint account"
                        );
                    }
                    other => {
                        panic!("Expected account state to be Update, got: {:?}", other);
                    }
                }
            }

            assert_eq!(
                ui_profile_result.transaction_profile.compute_units_consumed,
                ui_profile_result
                    .instruction_profiles
                    .as_ref()
                    .unwrap()
                    .iter()
                    .map(|ix| ix.compute_units_consumed)
                    .sum::<u64>(),
            )
        }
        // Get the latest blockhash for the transfer transaction
        let recent_blockhash = client
            .context
            .svm_locker
            .with_svm_reader(|svm_reader| svm_reader.latest_blockhash());

        // Amount of tokens to transfer (0.50 tokens with 2 decimals)
        let transfer_amount = 50;

        // Create transfer_checked instruction to send tokens from source to destination
        let transfer_instruction = transfer_checked(
            &spl_token_2022::id(),               // program id
            &source_ata,                         // source
            &mint.pubkey(),                      // mint
            &destination_ata,                    // destination
            &owner.pubkey(),                     // owner of source
            &[&payer.pubkey(), &owner.pubkey()], // signers
            transfer_amount,                     // amount
            2,                                   // decimals
        )
        .unwrap();

        // Create transaction for transferring tokens
        let transaction = Transaction::new_signed_with_payer(
            &[transfer_instruction],
            Some(&payer.pubkey()),
            &[&payer, &owner],
            recent_blockhash,
        );
        let signature = transaction.signatures[0];
        let (status_tx, _status_rx) = crossbeam_channel::unbounded();
        // Send and confirm transaction
        client
            .context
            .svm_locker
            .process_transaction(&None, transaction.clone().into(), status_tx, true, true)
            .await
            .unwrap();

        {
            let profile_result = client
                .rpc
                .get_transaction_profile(
                    Some(client.context.clone()),
                    UuidOrSignature::Signature(signature),
                    Some(RpcProfileResultConfig {
                        depth: Some(RpcProfileDepth::Instruction),
                        ..Default::default()
                    }),
                )
                .unwrap()
                .value
                .expect("missing profile result for processed transaction");

            assert!(
                profile_result.transaction_profile.error_message.is_none(),
                "Transaction should succeed, found error: {}",
                profile_result
                    .transaction_profile
                    .error_message
                    .as_ref()
                    .unwrap()
            );

            assert_eq!(
                profile_result.instruction_profiles.as_ref().unwrap().len(),
                1
            );

            let ix_profile = profile_result
                .instruction_profiles
                .as_ref()
                .unwrap()
                .get(0)
                .expect("instruction profile should exist");
            assert!(
                ix_profile.error_message.is_none(),
                "Profile should succeed, found error: {}",
                ix_profile.error_message.as_ref().unwrap()
            );

            let mut account_states = ix_profile.account_states.clone();

            let UiAccountProfileState::Writable(owner_account_change) = account_states
                .swap_remove(&owner.pubkey())
                .expect("account state should be present")
            else {
                panic!("Expected account state to be Writable");
            };

            let UiAccountChange::Unchanged(unchanged) = owner_account_change else {
                panic!(
                    "Expected account state to be Unchanged, got: {:?}",
                    owner_account_change
                );
            };
            assert!(unchanged.is_none(), "Owner account shouldn't exist");

            let UiAccountProfileState::Writable(sender_account_change) = account_states
                .swap_remove(&payer.pubkey())
                .expect("Payer account state should be present")
            else {
                panic!("Expected account state to be Writable");
            };

            match sender_account_change {
                UiAccountChange::Update(before, after) => {
                    assert_eq!(after.lamports, before.lamports - 10000);
                }
                other => {
                    panic!("Expected account state to be an Update, got: {:?}", other);
                }
            }

            let UiAccountProfileState::Readonly = account_states
                .swap_remove(&mint.pubkey())
                .expect("Mint account state should be present")
            else {
                panic!("Expected mint account state to be Readonly");
            };
            let UiAccountProfileState::Readonly = account_states
                .swap_remove(&spl_token_2022::ID)
                .expect("account state should be present")
            else {
                panic!("Expected account state to be Readonly");
            };

            let UiAccountProfileState::Writable(source_ata_change) = account_states
                .swap_remove(&source_ata)
                .expect("account state should be present")
            else {
                panic!("Expected account state to be Writable");
            };

            match source_ata_change {
                UiAccountChange::Update(before, after) => {
                    assert_eq!(
                        after.lamports, before.lamports,
                        "Source ATA lamports should remain unchanged"
                    );
                    assert_eq!(
                        after.data,
                        UiAccountData::Json(ParsedAccount {
                            program: "spl-token-2022".into(),
                            parsed: json!({
                                "info": {
                                    "extensions": [
                                        {
                                            "extension": "immutableOwner"
                                        }
                                    ],
                                    "isNative": false,
                                    "mint": mint.pubkey().to_string(),
                                    "owner": owner.pubkey().to_string(),
                                    "state": "initialized",
                                    "tokenAmount": {
                                        "amount": "9950",
                                        "decimals": 2,
                                        "uiAmount": 99.5,
                                        "uiAmountString": "99.5"
                                    }
                                },
                                "type": "account"
                            }),
                            space: 170
                        }),
                        "Source ATA data should be updated after transfer"
                    );
                }
                other => {
                    panic!("Expected account state to be Update, got: {:?}", other);
                }
            }

            let UiAccountProfileState::Writable(destination_ata_change) = account_states
                .swap_remove(&destination_ata)
                .expect("account state should be present")
            else {
                panic!("Expected account state to be Writable");
            };

            match destination_ata_change {
                UiAccountChange::Update(before, after) => {
                    assert_eq!(
                        after.lamports, before.lamports,
                        "Destination ATA lamports should remain unchanged"
                    );
                    assert_eq!(
                        after.data,
                        UiAccountData::Json(ParsedAccount {
                            program: "spl-token-2022".into(),
                            parsed: json!({
                                "info": {
                                    "extensions": [
                                        {
                                            "extension": "immutableOwner"
                                        }
                                    ],
                                    "isNative": false,
                                    "mint": mint.pubkey().to_string(),
                                    "owner": recipient.pubkey().to_string(),
                                    "state": "initialized",
                                    "tokenAmount": {
                                        "amount": transfer_amount.to_string(),
                                        "decimals": 2,
                                        "uiAmount": 0.5,
                                        "uiAmountString": "0.5"
                                    }
                                },
                                "type": "account"
                            }),
                            space: 170
                        }),
                        "Destination ATA data should be updated after transfer"
                    );
                }
                other => {
                    panic!("Expected account state to be Update, got: {:?}", other);
                }
            }

            assert!(
                account_states.is_empty(),
                "All account states should have been processed, found: {:?}",
                account_states
            );
        }
    }
}
