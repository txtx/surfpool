use std::collections::HashMap;

use agave_feature_set::FeatureSet;
use itertools::Itertools;
use litesvm::{
    LiteSVM,
    types::{FailedTransactionMetadata, SimulatedTransactionInfo, TransactionResult},
};
use solana_account::{Account, AccountSharedData};
use solana_loader_v3_interface::get_program_data_address;
use solana_program_option::COption;
use solana_pubkey::Pubkey;
use solana_transaction::versioned::VersionedTransaction;

use crate::{
    error::{SurfpoolError, SurfpoolResult},
    storage::{SqliteStorage, Storage, StorageConstructor},
    surfnet::{GetAccountResult, locker::is_supported_token_program},
};

#[derive(Clone)]
pub struct SurfnetLiteSvm {
    pub svm: LiteSVM,
    pub db: Option<Box<dyn Storage<String, AccountSharedData>>>,
}

impl SurfnetLiteSvm {
    pub fn new() -> Self {
        Self {
            svm: LiteSVM::new(),
            db: None,
        }
    }

    pub fn initialize(
        mut self,
        feature_set: FeatureSet,
        database_url: Option<&str>,
    ) -> SurfpoolResult<Self> {
        self.svm = LiteSVM::new()
            .with_blockhash_check(false)
            .with_sigverify(false)
            .with_feature_set(feature_set);

        create_native_mint(&mut self);

        if let Some(db_url) = database_url {
            let db: Box<dyn Storage<String, AccountSharedData>> =
                Box::new(SqliteStorage::connect(db_url, "accounts")?);
            self.db = Some(db);
        }

        Ok(self)
    }

    pub fn reset(&mut self, feature_set: FeatureSet) -> SurfpoolResult<()> {
        self.svm = LiteSVM::new()
            .with_blockhash_check(false)
            .with_sigverify(false)
            .with_feature_set(feature_set);

        create_native_mint(self);

        if let Some(db) = &mut self.db {
            db.clear()?;
        }
        Ok(())
    }

    /// Perform garbage collection by resetting the SVM state while retaining the database.
    /// This is useful for cleaning up unused accounts and reducing memory usage.
    /// If no database is configured, this function is a no-op.
    pub fn garbage_collect(&mut self, feature_set: FeatureSet) {
        // If no DB is configured, skip garbage collection
        if self.db.is_none() {
            return;
        }
        // todo: this is also resetting the log bytes limit and airdrop keypair, would be nice to avoid
        self.svm = LiteSVM::new()
            .with_blockhash_check(false)
            .with_sigverify(false)
            .with_feature_set(feature_set);

        create_native_mint(self);
    }

    pub fn apply_feature_config(&mut self, feature_set: FeatureSet) -> &mut Self {
        self.svm = LiteSVM::new()
            .with_blockhash_check(false)
            .with_sigverify(false)
            .with_feature_set(feature_set);

        create_native_mint(self);
        self
    }

    pub fn set_log_bytes_limit(&mut self, limit: Option<usize>) {
        self.svm.set_log_bytes_limit(limit);
    }

    pub fn set_sigverify(&mut self, sigverify: bool) {
        self.svm.set_sigverify(sigverify);
    }

    pub fn with_blockhash_check(mut self, check: bool) -> Self {
        self.svm = self.svm.with_blockhash_check(check);
        self
    }

    pub fn get_sysvar<T>(&self) -> T
    where
        T: solana_sysvar::Sysvar + solana_sysvar_id::SysvarId + serde::de::DeserializeOwned,
    {
        self.svm.get_sysvar()
    }

    pub fn set_sysvar<T>(&mut self, sysvar: &T)
    where
        T: solana_sysvar::Sysvar + solana_sysvar_id::SysvarId + solana_sysvar::SysvarSerialize,
    {
        self.svm.set_sysvar(sysvar);
    }

    pub fn expire_blockhash(&mut self) {
        self.svm.expire_blockhash();
    }

    pub fn send_transaction(&mut self, tx: impl Into<VersionedTransaction>) -> TransactionResult {
        self.svm.send_transaction(tx)
    }

    pub fn minimum_balance_for_rent_exemption(&self, data_len: usize) -> u64 {
        self.svm.minimum_balance_for_rent_exemption(data_len)
    }

    pub fn simulate_transaction(
        &self,
        tx: impl Into<VersionedTransaction>,
    ) -> Result<SimulatedTransactionInfo, FailedTransactionMetadata> {
        self.svm.simulate_transaction(tx)
    }

    pub fn airdrop(&mut self, pubkey: &Pubkey, lamports: u64) -> TransactionResult {
        self.svm.airdrop(pubkey, lamports)
    }

    pub fn get_account_no_db(&self, pubkey: &Pubkey) -> Option<Account> {
        self.svm.get_account(pubkey)
    }

    pub fn get_account(&self, pubkey: &Pubkey) -> SurfpoolResult<Option<Account>> {
        if let Some(account) = self.svm.get_account(pubkey) {
            return Ok(Some(account));
        } else if let Some(db) = &self.db {
            return Ok(db.get(&pubkey.to_string())?.map::<Account, _>(Into::into));
        }
        Ok(None)
    }

    pub fn get_account_result(&self, pubkey: &Pubkey) -> SurfpoolResult<GetAccountResult> {
        if let Some(account) = self.svm.get_account(pubkey) {
            return Ok(GetAccountResult::FoundAccount(
                *pubkey, account,
                // mark as not an account that should be updated in the SVM, since this is a local read and it already exists
                false,
            ));
        } else if let Some(db) = &self.db {
            let mut result = None;
            if let Some(account) = db.get(&pubkey.to_string())?.map::<Account, _>(Into::into) {
                if is_supported_token_program(&account.owner) {
                    if let Ok(token_account) = crate::types::TokenAccount::unpack(&account.data) {
                        let mint = db.get(&token_account.mint().to_string())?.map(Into::into);

                        result = Some(GetAccountResult::FoundTokenAccount(
                            (*pubkey, account.clone()),
                            (token_account.mint(), mint),
                        ));
                    };
                } else if account.executable {
                    let program_data_address = get_program_data_address(pubkey);

                    let program_data = db.get(&program_data_address.to_string())?.map(Into::into);

                    result = Some(GetAccountResult::FoundProgramAccount(
                        (*pubkey, account.clone()),
                        (program_data_address, program_data),
                    ));
                }

                return Ok(result.unwrap_or(GetAccountResult::FoundAccount(
                    *pubkey, account,
                    // Mark this account as needing to be updated in the SVM, since we pulled it from the db
                    true,
                )));
            }
        }
        Ok(GetAccountResult::None(*pubkey))
    }

    pub fn set_account(&mut self, pubkey: Pubkey, account: Account) -> SurfpoolResult<()> {
        self.set_account_in_db(pubkey, account.clone().into())?;

        self.svm.set_account(pubkey, account)?;
        Ok(())
    }

    pub fn delete_account(&mut self, pubkey: &Pubkey) -> SurfpoolResult<()> {
        self.delete_account_in_db(pubkey)?;

        // You can't delete an account using the LiteSvm, so we set it to an empty account
        // so it can be garbage collected later
        self.svm
            .set_account(*pubkey, Account::default())
            .map_err(|e| SurfpoolError::set_account(*pubkey, e))?;
        Ok(())
    }

    pub fn set_account_in_db(
        &mut self,
        pubkey: Pubkey,
        account: AccountSharedData,
    ) -> SurfpoolResult<()> {
        if let Some(db) = &mut self.db {
            db.store(pubkey.to_string(), account)?;
        }
        Ok(())
    }

    pub fn delete_account_in_db(&mut self, pubkey: &Pubkey) -> SurfpoolResult<()> {
        if let Some(db) = &mut self.db {
            db.take(&pubkey.to_string())?;
        }
        Ok(())
    }

    /// Get all accounts from both the LiteSVM state and the database, merging them together.
    /// Accounts in the LiteSVM state take precedence over those in the database.
    /// The resulting accounts are sorted by Pubkey.
    pub fn get_all_accounts(&self) -> SurfpoolResult<Vec<(Pubkey, AccountSharedData)>> {
        // In general, we trust the LiteSVM state as the most up-to-date source of truth for any given account,
        // But there's a chance that the account was garbage collected, meaning it exists in the DB but not in the SVM.
        // Therefore, we need to merge the two sources of accounts, prioritizing the SVM state.
        let mut accounts = HashMap::new();
        if let Some(db) = &self.db {
            let db_accounts = db.into_iter()?;
            for (key, account) in db_accounts {
                let pubkey = Pubkey::from_str_const(&key);
                accounts.insert(pubkey, account);
            }
        }
        for (pubkey, account) in self.svm.accounts_db().inner.iter() {
            if !accounts.contains_key(pubkey) {
                accounts.insert(*pubkey, account.clone());
            }
        }
        Ok(accounts
            .into_iter()
            .sorted_by(|a, b| a.0.cmp(&b.0))
            .collect())
    }
}

fn create_native_mint(svm: &mut SurfnetLiteSvm) {
    use solana_program_pack::Pack;
    use solana_sysvar::rent::Rent;
    use spl_token_interface::state::Mint;

    let mut data = vec![0; Mint::LEN];
    let mint = Mint {
        mint_authority: COption::None,
        supply: 0,
        decimals: spl_token_interface::native_mint::DECIMALS,
        is_initialized: true,
        freeze_authority: COption::None,
    };
    Mint::pack(mint, &mut data).unwrap();
    let account = Account {
        lamports: svm.get_sysvar::<Rent>().minimum_balance(data.len()),
        data,
        owner: spl_token_interface::ID,
        executable: false,
        rent_epoch: 0,
    };
    svm.set_account(spl_token_interface::native_mint::ID, account)
        .expect("Failed to create native mint account in SVM");
}
