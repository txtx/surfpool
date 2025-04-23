use std::fmt;

use crate::rpc::utils::verify_pubkey;
use crate::rpc::State;

use jsonrpc_core::futures::future;
use jsonrpc_core::BoxFuture;
use jsonrpc_core::{Error, Result};
use jsonrpc_derive::rpc;
use serde::de::Visitor;
use serde::Serialize;
use serde::{Deserialize, Deserializer, Serializer};
use serde_with::{serde_as, BytesOrString};
use solana_account::Account;
use solana_client::rpc_custom_error::RpcCustomError;
use solana_client::rpc_response::RpcResponseContext;
use solana_clock::Epoch;
use solana_pubkey::Pubkey;
use solana_rpc_client_api::response::Response as RpcResponse;
use solana_sdk::clock::Epoch;
use solana_sdk::program_option::COption;
use solana_sdk::program_pack::Pack;
use solana_sdk::pubkey::Pubkey;
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
                owner: verify_pubkey(&self.owner.clone().unwrap())
                    .map_err(|e| Error::invalid_params(format!("Invalid owner: {}", e.message)))?,
                executable: self.executable.unwrap(),
                rent_epoch: self.rent_epoch.unwrap(),
                data: self.data.clone().unwrap(),
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
        if let Some(owner) = self.owner {
            account.owner = verify_pubkey(&owner)
                .map_err(|e| Error::invalid_params(format!("Invalid owner: {}", e.message)))?;
        }
        if let Some(executable) = self.executable {
            account.executable = executable;
        }
        if let Some(rent_epoch) = self.rent_epoch {
            account.rent_epoch = rent_epoch;
        }
        if let Some(data) = &self.data {
            account.data = data.clone();
        }
        Ok(())
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
                    token_account.delegate =
                        COption::Some(verify_pubkey(&pubkey).map_err(|e| {
                            Error::invalid_params(format!("Invalid delegate: {}", e.message))
                        })?);
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
                    token_account.close_authority =
                        COption::Some(verify_pubkey(&pubkey).map_err(|e| {
                            Error::invalid_params(format!("Invalid close authority: {}", e.message))
                        })?);
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

    #[rpc(meta, name = "svm_setAccount")]
    fn set_account(
        &self,
        meta: Self::Metadata,
        pubkey: String,
        update: AccountUpdate,
    ) -> BoxFuture<Result<RpcResponse<()>>>;

    #[rpc(meta, name = "svm_setTokenAccount")]
    fn set_token_account(
        &self,
        meta: Self::Metadata,
        owner: String,
        mint: String,
        update: TokenAccountUpdate,
        token_program: Option<String>,
    ) -> BoxFuture<Result<RpcResponse<()>>>;
}

pub fn write_account(
    meta: Option<RunloopContext>,
    pubkey: Pubkey,
    account: Account,
) -> std::result::Result<(), String> {
    let mut state_reader = meta.get_state_mut().map_err(|e| e.to_string())?;
    state_reader
        .svm
        .set_account(pubkey, account.clone())
        .map_err(|err| format!("failed to save fetched account '{pubkey:?}': {err:?}"))?;
    Ok(())
}

pub struct SurfpoolSvmTricksRpc;
impl SvmTricksRpc for SurfpoolSvmTricksRpc {
    type Metadata = Option<RunloopContext>;

    fn set_account(
        &self,
        meta: Self::Metadata,
        pubkey_str: String,
        update: AccountUpdate,
    ) -> BoxFuture<Result<RpcResponse<()>>> {
        let pubkey = match verify_pubkey(&pubkey_str) {
            Ok(res) => res,
            Err(e) => return Box::pin(future::err(e)),
        };
        let state_reader = match meta.get_state() {
            Ok(res) => res,
            Err(e) => return Box::pin(future::err(e.into())),
        };

        let absolute_slot = state_reader.epoch_info.absolute_slot;
        let account = state_reader.svm.get_account(&pubkey);
        let rpc_client = state_reader.rpc_client.clone();
        drop(state_reader);

        let full_account_update = match update.to_account() {
            Err(e) => return Box::pin(future::err(e)),
            Ok(res) => res,
        };
        if let Some(account) = full_account_update {
            return match write_account(meta, pubkey, account).map_err(|e| Error::invalid_params(e))
            {
                Ok(_) => Box::pin(future::ok(RpcResponse {
                    context: RpcResponseContext::new(absolute_slot),
                    value: (),
                })),
                Err(e) => Box::pin(future::err(e)),
            };
        } else {
            return Box::pin(async move {
                let mut account = match account {
                    Some(account) => account,
                    None => {
                        if let Some(fetched_account) = rpc_client.get_account(&pubkey).await.ok() {
                            fetched_account
                        } else {
                            let Some(ctx) = &meta else {
                                return Err(RpcCustomError::NodeUnhealthy {
                                    num_slots_behind: None,
                                }
                                .into());
                            };
                            let _ = ctx.simnet_events_tx.send(SimnetEvent::info(
                                format!("Account {pubkey} not found, creating a new account from default values"),
                            ));
                            Account {
                                lamports: 0,
                                owner: system_program::id(),
                                executable: false,
                                rent_epoch: 0,
                                data: vec![],
                            }
                        }
                    }
                };
                if let Err(e) = update.apply(&mut account) {
                    return Err(e);
                };
                return match write_account(meta, pubkey, account)
                    .map_err(|e| Error::invalid_params(e))
                {
                    Ok(_) => Ok(RpcResponse {
                        context: RpcResponseContext::new(absolute_slot),
                        value: (),
                    }),
                    Err(e) => Err(e),
                };
            });
        }
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
            Err(e) => return Box::pin(future::err(e)),
        };

        let mint = match verify_pubkey(&mint_str) {
            Ok(res) => res,
            Err(e) => return Box::pin(future::err(e)),
        };

        let token_program_id = match some_token_program_str {
            Some(token_program_str) => match verify_pubkey(&token_program_str) {
                Ok(res) => res,
                Err(e) => return Box::pin(future::err(e)),
            },
            None => spl_token::id(),
        };

        let associated_token_account =
            get_associated_token_address_with_program_id(&owner, &mint, &token_program_id);

        let state_reader = match meta.get_state() {
            Ok(res) => res,
            Err(e) => return Box::pin(future::err(e.into())),
        };

        let absolute_slot = state_reader.epoch_info.absolute_slot;
        let account = state_reader.svm.get_account(&associated_token_account);
        let minimum_rent = state_reader
            .svm
            .minimum_balance_for_rent_exemption(TokenAccount::LEN);
        let rpc_client = state_reader.rpc_client.clone();
        drop(state_reader);

        return Box::pin(async move {
            let mut token_account = match account {
                Some(account) => account,
                None => {
                    if let Some(fetched_account) =
                        rpc_client.get_account(&associated_token_account).await.ok()
                    {
                        fetched_account
                    } else {
                        let Some(ctx) = &meta else {
                            return Err(RpcCustomError::NodeUnhealthy {
                                num_slots_behind: None,
                            }
                            .into());
                        };
                        let _ = ctx.simnet_events_tx.send(SimnetEvent::info(
                                format!("Associated token account {associated_token_account} not found, creating a new account from default values"),
                            ));
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
                    }
                }
            };

            let mut token_account_data = match TokenAccount::unpack(&token_account.data) {
                Ok(token_account_data) => token_account_data,
                Err(e) => {
                    return Err(Error::invalid_params(format!(
                        "Failed to unpack token account data: {}",
                        e
                    )))
                }
            };

            if let Err(e) = update.apply(&mut token_account_data) {
                return Err(e);
            };

            let mut final_account_bytes = [0; TokenAccount::LEN];
            token_account_data.pack_into_slice(&mut final_account_bytes);
            token_account.data = final_account_bytes.to_vec();
            return match write_account(meta, associated_token_account, token_account)
                .map_err(|e| Error::invalid_params(e))
            {
                Ok(_) => Ok(RpcResponse {
                    context: RpcResponseContext::new(absolute_slot),
                    value: (),
                }),
                Err(e) => Err(e),
            };
        });
    }
}
