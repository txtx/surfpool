use base64::{Engine as _, engine::general_purpose::STANDARD};
use jsonrpc_core::{BoxFuture, Error, Result, futures::future};
use jsonrpc_derive::rpc;
use solana_account::Account;
use solana_account_decoder::UiAccountEncoding;
use solana_client::rpc_response::RpcResponseContext;
use solana_clock::{Clock, Epoch, Slot};
use solana_commitment_config::CommitmentConfig;
use solana_epoch_info::EpochInfo;
use solana_rpc_client_api::response::Response as RpcResponse;
use solana_sdk::{program_option::COption, system_program, transaction::VersionedTransaction};
use spl_associated_token_account::get_associated_token_address_with_program_id;
use surfpool_types::{
    ClockCommand, Idl, SimnetCommand, SimnetEvent,
    types::{
        AccountUpdate, ProfileResult, SetSomeAccount, SupplyUpdate, TokenAccountUpdate,
        UuidOrSignature,
    },
};

use super::{RunloopContext, SurfnetRpcContext};
use crate::{
    error::SurfpoolError,
    rpc::{
        State,
        utils::{verify_pubkey, verify_pubkeys},
    },
    surfnet::{GetAccountResult, locker::SvmAccessContext},
    types::TokenAccount,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TimeTravelConfig {
    Reset,
    Epoch(Epoch),
    AbsoluteSlot(Slot),
    Timestamp(i64),
}

impl Default for TimeTravelConfig {
    fn default() -> Self {
        Self::Reset
    }
}

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
        encoding: Option<UiAccountEncoding>,
    ) -> BoxFuture<Result<RpcResponse<ProfileResult>>>;

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
    ) -> BoxFuture<Result<RpcResponse<Option<Vec<ProfileResult>>>>>;

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
    ) -> BoxFuture<Result<RpcResponse<Option<ProfileResult>>>>;

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

    /// A cheat code to jump forward or backward in time on the local network.
    /// Useful for testing epoch-based or time-sensitive logic.
    ///
    /// ## Parameters
    /// - `config` (optional): A `TimeTravelConfig` specifying how to modify the clock:
    ///   - `reset`: Resets the clock to match mainnet.
    ///   - `timestamp(u64)`: Moves time to the specified UNIX timestamp.
    ///   - `absoluteSlot(u64)`: Moves to the specified absolute slot.
    ///   - `epoch(u64)`: Advances time to the specified epoch (each epoch = 432,000 slots).
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
    ///   "params": [ { "epoch": 512 } ]
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
    /// ```    #[rpc(meta, name = "surfnet_timeTravel")]
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
    /// - `meta`: Metadata passed with the request, such as the client's request context.
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
        encoding: Option<UiAccountEncoding>,
    ) -> BoxFuture<Result<RpcResponse<ProfileResult>>> {
        let SurfnetRpcContext {
            svm_locker,
            remote_ctx,
        } = match meta.get_rpc_context(()) {
            Ok(ctx) => ctx,
            Err(e) => return e.into(),
        };
        let remote_ctx = remote_ctx.map(|(client, _)| client);

        let transaction_bytes = match STANDARD.decode(&transaction_data_b64) {
            Ok(bytes) => bytes,
            Err(e) => return SurfpoolError::invalid_base64_data("transaction", e).into(),
        };

        let transaction: VersionedTransaction = match bincode::deserialize(&transaction_bytes) {
            Ok(tx) => tx,
            Err(e) => return SurfpoolError::deserialize_error("transaction", e).into(),
        };

        Box::pin(async move {
            let SvmAccessContext {
                slot,
                inner: profile_result,
                ..
            } = svm_locker
                .profile_transaction(&remote_ctx, transaction, encoding, tag.clone())
                .await?;

            Ok(RpcResponse {
                context: RpcResponseContext::new(slot),
                value: profile_result,
            })
        })
    }

    fn get_profile_results_by_tag(
        &self,
        meta: Self::Metadata,
        tag: String,
    ) -> BoxFuture<Result<RpcResponse<Option<Vec<ProfileResult>>>>> {
        let svm_locker = match meta.get_svm_locker() {
            Ok(locker) => locker,
            Err(e) => return e.into(),
        };
        Box::pin(async move {
            let profiles = svm_locker.get_profile_results_by_tag(tag)?;
            let slot = svm_locker.get_latest_absolute_slot();
            Ok(RpcResponse {
                context: RpcResponseContext::new(slot),
                value: profiles,
            })
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

                svm_writer.updated_at = chrono::Utc::now().timestamp_millis() as u64;
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
    ) -> BoxFuture<Result<RpcResponse<Option<ProfileResult>>>> {
        let svm_locker = match meta.get_svm_locker() {
            Ok(locker) => locker,
            Err(e) => return e.into(),
        };
        Box::pin(async move {
            let profile_result = svm_locker.get_profile_result(signature_or_uuid.clone())?;
            let context_slot = profile_result
                .as_ref()
                .map(|pr| pr.slot)
                .unwrap_or_else(|| svm_locker.get_latest_absolute_slot());
            Ok(RpcResponse {
                context: RpcResponseContext::new(context_slot),
                value: profile_result,
            })
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

    fn pause_clock(&self, meta: Self::Metadata) -> Result<EpochInfo> {
        let surfnet_command_tx: crossbeam_channel::Sender<SimnetCommand> =
            meta.get_surfnet_command_tx()?;
        let _ = surfnet_command_tx.send(SimnetCommand::CommandClock(ClockCommand::Pause));
        meta.with_svm_reader(|svm_reader| svm_reader.latest_epoch_info.clone())
            .map_err(Into::into)
    }

    fn resume_clock(&self, meta: Self::Metadata) -> Result<EpochInfo> {
        let surfnet_command_tx: crossbeam_channel::Sender<SimnetCommand> =
            meta.get_surfnet_command_tx()?;
        let _ = surfnet_command_tx.send(SimnetCommand::CommandClock(ClockCommand::Resume));
        meta.with_svm_reader(|svm_reader| svm_reader.latest_epoch_info.clone())
            .map_err(Into::into)
    }

    fn time_travel(
        &self,
        meta: Self::Metadata,
        config: Option<TimeTravelConfig>,
    ) -> Result<EpochInfo> {
        let surfnet_command_tx = meta.get_surfnet_command_tx()?;
        let current_epoch_info =
            meta.with_svm_reader(|svm_reader| svm_reader.latest_epoch_info.clone())?;

        let behavior = config.unwrap_or_default();
        let clock_update: Clock = match behavior {
            TimeTravelConfig::Reset => {
                // Reset to mainnet
                unimplemented!()
            }
            TimeTravelConfig::Timestamp(timestamp) => {
                // If the timestamp is passed, we compute the difference between now and the target, deduct the amount of epochs + slot elapsed and updated the clock accordingly
                unimplemented!()
            }
            TimeTravelConfig::AbsoluteSlot(slot) => {
                // If the absolute slot is passed, we compute the corresponding epoch + timestamp, and update the clock accordingly
                unimplemented!()
            }
            TimeTravelConfig::Epoch(epoch) => {
                // If the epoch is passed, we multiply it by 432,000, and set the absolute slot + timestamp accordingly (based on the block blocktime)
                unimplemented!()
            }
        };
        let new_epoch_info = unimplemented!();

        let _ = surfnet_command_tx.send(SimnetCommand::UpdateInternalClock(clock_update));

        Ok(new_epoch_info)
    }
}
