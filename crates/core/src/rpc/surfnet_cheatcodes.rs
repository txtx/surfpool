use std::fmt;

use crate::rpc::utils::verify_pubkey;
use crate::rpc::State;
use crate::surfnet::GetAccountStrategy;

use jsonrpc_core::futures::future;
use jsonrpc_core::BoxFuture;
use jsonrpc_core::{Error, Result};
use jsonrpc_derive::rpc;
use serde::de::Visitor;
use serde::Serialize;
use serde::{Deserialize, Deserializer, Serializer};
use serde_with::{serde_as, BytesOrString};
use solana_account::Account;
use solana_client::rpc_response::RpcResponseContext;
use solana_clock::Epoch;
use solana_rpc_client_api::response::Response as RpcResponse;
use solana_sdk::program_option::COption;
use solana_sdk::program_pack::Pack;
use solana_sdk::system_program;
use spl_associated_token_account::get_associated_token_address_with_program_id;
use spl_token::state::{Account as TokenAccount, AccountState};
use surfpool_types::SimnetEvent;

use super::RunloopContext;

#[serde_as]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountUpdate {
    /// providing this value sets the lamports in the account
    pub lamports: Option<u64>,
    /// providing this value sets the data held in this account
    #[serde_as(as = "Option<BytesOrString>")]
    pub data: Option<Vec<u8>>,
    ///  providing this value sets the program that owns this account. If executable, the program that loads this account.
    pub owner: Option<String>,
    /// providing this value sets whether this account's data contains a loaded program (and is now read-only)
    pub executable: Option<bool>,
    /// providing this value sets the epoch at which this account will next owe rent
    pub rent_epoch: Option<Epoch>,
}

impl AccountUpdate {
    fn is_full_account_data(&self) -> bool {
        self.lamports.is_some()
            && self.owner.is_some()
            && self.executable.is_some()
            && self.rent_epoch.is_some()
            && self.data.is_some()
    }
    /// Convert the update to an account if all fields are provided
    pub fn to_account(&self) -> Result<Option<Account>> {
        if self.is_full_account_data() {
            Ok(Some(Account {
                lamports: self.lamports.unwrap(),
                owner: verify_pubkey(&self.owner.clone().unwrap())?,
                executable: self.executable.unwrap(),
                rent_epoch: self.rent_epoch.unwrap(),
                data: self.expect_hex_data()?,
            }))
        } else {
            Ok(None)
        }
    }
    /// Apply the update to the account
    pub fn apply(self, account: &mut Account) -> Result<()> {
        if let Some(lamports) = self.lamports {
            account.lamports = lamports;
        }
        if let Some(owner) = &self.owner {
            account.owner = verify_pubkey(&owner)?;
        }
        if let Some(executable) = self.executable {
            account.executable = executable;
        }
        if let Some(rent_epoch) = self.rent_epoch {
            account.rent_epoch = rent_epoch;
        }
        if let Some(_) = &self.data {
            account.data = self.expect_hex_data()?;
        }
        Ok(())
    }

    pub fn expect_hex_data(&self) -> Result<Vec<u8>> {
        let data = self.data.as_ref().expect("missing expected data field");
        hex::decode(data)
            .map_err(|e| Error::invalid_params(format!("Invalid hex data provided: {}", e)))
    }
}

#[serde_as]
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenAccountUpdate {
    /// providing this value sets the amount of the token in the account data
    pub amount: Option<u64>,
    /// providing this value sets the delegate of the token account
    pub delegate: Option<SetSomeAccount>,
    /// providing this value sets the state of the token account
    pub state: Option<String>,
    /// providing this value sets the amount authorized to the delegate
    pub delegated_amount: Option<u64>,
    /// providing this value sets the close authority of the token account
    pub close_authority: Option<SetSomeAccount>,
}

#[derive(Debug, Clone)]
pub enum SetSomeAccount {
    Account(String),
    NoAccount,
}

impl<'de> Deserialize<'de> for SetSomeAccount {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct SetSomeAccountVisitor;

        impl<'de> Visitor<'de> for SetSomeAccountVisitor {
            type Value = SetSomeAccount;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a Pubkey String or the String 'null'")
            }

            fn visit_some<D>(self, deserializer: D) -> std::result::Result<Self::Value, D::Error>
            where
                D: Deserializer<'de>,
            {
                Deserialize::deserialize(deserializer).map(|v: String| match v.as_str() {
                    "null" => SetSomeAccount::NoAccount,
                    _ => SetSomeAccount::Account(v.to_string()),
                })
            }
        }

        deserializer.deserialize_option(SetSomeAccountVisitor)
    }
}

impl Serialize for SetSomeAccount {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            SetSomeAccount::Account(val) => serializer.serialize_str(&val),
            SetSomeAccount::NoAccount => serializer.serialize_str("null"),
        }
    }
}

impl TokenAccountUpdate {
    /// Apply the update to the account
    pub fn apply(self, token_account: &mut TokenAccount) -> Result<()> {
        if let Some(amount) = self.amount {
            token_account.amount = amount;
        }
        if let Some(delegate) = self.delegate {
            match delegate {
                SetSomeAccount::Account(pubkey) => {
                    token_account.delegate = COption::Some(verify_pubkey(&pubkey)?);
                }
                SetSomeAccount::NoAccount => {
                    token_account.delegate = COption::None;
                }
            }
        }
        if let Some(state) = self.state {
            token_account.state = match state.as_str() {
                "uninitialized" => AccountState::Uninitialized,
                "frozen" => AccountState::Frozen,
                "initialized" => AccountState::Initialized,
                _ => {
                    return Err(Error::invalid_params(format!(
                        "Invalid token account state: {}",
                        state
                    )))
                }
            };
        }
        if let Some(delegated_amount) = self.delegated_amount {
            token_account.delegated_amount = delegated_amount;
        }
        if let Some(close_authority) = self.close_authority {
            match close_authority {
                SetSomeAccount::Account(pubkey) => {
                    token_account.close_authority = COption::Some(verify_pubkey(&pubkey)?);
                }
                SetSomeAccount::NoAccount => {
                    token_account.close_authority = COption::None;
                }
            }
        }
        Ok(())
    }
}

#[rpc]
pub trait SvmTricksRpc {
    type Metadata;

    /// A "cheat code" method for developers to set or update an account in Surfpool.
    ///
    /// This method allows developers to set or update the lamports, data, owner, executable status,
    /// and rent epoch of a given account.
    ///
    /// ## Parameters
    /// - `meta`: Metadata passed with the request, such as the client’s request context.
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
    /// - `meta`: Metadata passed with the request, such as the client’s request context.
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
}

pub struct SurfnetCheatcodesRpc;
impl SvmTricksRpc for SurfnetCheatcodesRpc {
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
        let account_update = match update.to_account() {
            Err(e) => return Box::pin(future::err(e)),
            Ok(res) => res,
        };

        let svm_locker = match meta.get_svm_locker() {
            Ok(locker) => locker,
            Err(e) => return e.into(),
        };

        Box::pin(async move {
            let mut svm_writer = svm_locker.write().await;

            // if the account update is contains all fields, we can directly set the account
            let account_to_set = if let Some(account) = account_update {
                account
            } else {
                // otherwise, we need to fetch the account and apply the update
                let mut account_to_update = svm_writer
                    .get_account(
                        &pubkey,
                        GetAccountStrategy::LocalThenConnectionOrDefault(Some(Box::new(move |surfnet_svm| {
                            // if the account does not exist locally or in the remote, create a new account with default values
                            let _ = surfnet_svm.simnet_events_tx.send(SimnetEvent::info(format!(
                                "Account {pubkey} not found, creating a new account from default values"
                            )));

                            Account {
                                lamports: 0,
                                owner: system_program::id(),
                                executable: false,
                                rent_epoch: 0,
                                data: vec![],
                            }
                        }))),
                    )
                    .await?.unwrap();

                update.apply(&mut account_to_update)?;
                account_to_update
            };
            svm_writer.set_account(&pubkey, account_to_set)?;

            Ok(RpcResponse {
                context: RpcResponseContext::new(svm_writer.get_latest_absolute_slot()),
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

        let svm_locker = match meta.get_svm_locker() {
            Ok(res) => res,
            Err(e) => return Box::pin(future::err(e.into())),
        };

        return Box::pin(async move {
            let mut svm_writer = svm_locker.write().await;
            let mut token_account = svm_writer
                .get_account(
                    &associated_token_account,
                    GetAccountStrategy::LocalThenConnectionOrDefault(Some(Box::new(move |surfnet_svm| {
                        let _ = surfnet_svm.simnet_events_tx.send(SimnetEvent::info(
                            format!("Associated token account {associated_token_account} not found, creating a new account from default values"),
                        ));

                        let minimum_rent = surfnet_svm
                            .inner
                            .minimum_balance_for_rent_exemption(TokenAccount::LEN);

                        let mut data = [0; TokenAccount::LEN];
                        let default = TokenAccount {
                            mint,
                            owner,
                            state: AccountState::Initialized,
                            ..Default::default()
                        };
                        default.pack_into_slice(&mut data);
                        Account {
                            lamports: minimum_rent,
                            owner: token_program_id,
                            executable: false,
                            rent_epoch: 0,
                            data: data.to_vec(),
                        }
                    }))),
                )
                .await?
                .unwrap();
            let mut token_account_data =
                TokenAccount::unpack(&token_account.data).map_err(|e| {
                    Error::invalid_params(format!("Failed to unpack token account data: {}", e))
                })?;

            if let Err(e) = update.apply(&mut token_account_data) {
                return Err(e);
            };

            let mut final_account_bytes = [0; TokenAccount::LEN];
            token_account_data.pack_into_slice(&mut final_account_bytes);
            token_account.data = final_account_bytes.to_vec();
            svm_writer.set_account(&associated_token_account, token_account)?;

            Ok(RpcResponse {
                context: RpcResponseContext::new(svm_writer.get_latest_absolute_slot()),
                value: (),
            })
        });
    }
}
