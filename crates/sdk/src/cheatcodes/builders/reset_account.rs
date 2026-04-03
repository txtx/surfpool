use solana_pubkey::Pubkey;

use crate::cheatcodes::builders::CheatcodeBuilder;

pub struct ResetAccount {
    address: Pubkey,
    include_owned_accounts: Option<bool>,
}

impl ResetAccount {
    pub fn new(address: Pubkey) -> Self {
        Self {
            address,
            include_owned_accounts: None,
        }
    }

    pub fn include_owned_accounts(mut self, include_owned_accounts: bool) -> Self {
        self.include_owned_accounts = Some(include_owned_accounts);
        self
    }
}

impl CheatcodeBuilder for ResetAccount {
    const METHOD: &'static str = "surfnet_resetAccount";

    fn build(self) -> serde_json::Value {
        let mut params = vec![self.address.to_string().into()];

        if let Some(include_owned_accounts) = self.include_owned_accounts {
            params.push(serde_json::json!({
                "includeOwnedAccounts": include_owned_accounts
            }));
        }

        serde_json::Value::Array(params)
    }
}
