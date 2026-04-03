use solana_pubkey::Pubkey;

use crate::cheatcodes::builders::CheatcodeBuilder;

pub struct SetAccount {
    address: Pubkey,
    lamports: Option<u64>,
    data: Option<Vec<u8>>,
    owner: Option<Pubkey>,
    rent_epoch: Option<u64>,
    executable: Option<bool>,
}

impl SetAccount {
    pub fn new(address: Pubkey) -> Self {
        Self {
            address,
            lamports: None,
            data: None,
            owner: None,
            rent_epoch: None,
            executable: None,
        }
    }

    pub fn lamports(mut self, lamports: u64) -> Self {
        self.lamports = Some(lamports);
        self
    }

    pub fn data(mut self, data: Vec<u8>) -> Self {
        self.data = Some(data);
        self
    }

    pub fn owner(mut self, owner: Pubkey) -> Self {
        self.owner = Some(owner);
        self
    }

    pub fn rent_epoch(mut self, rent_epoch: u64) -> Self {
        self.rent_epoch = Some(rent_epoch);
        self
    }

    pub fn executable(mut self, executable: bool) -> Self {
        self.executable = Some(executable);
        self
    }
}

impl CheatcodeBuilder for SetAccount {
    const METHOD: &'static str = "surfnet_setAccount";
    fn build(self) -> serde_json::Value {
        let mut account_info = serde_json::Map::new();
        if let Some(lamports) = self.lamports {
            account_info.insert("lamports".to_string(), lamports.into());
        }
        if let Some(data) = self.data {
            account_info.insert("data".to_string(), hex::encode(data).into());
        }
        if let Some(owner) = self.owner {
            account_info.insert("owner".to_string(), owner.to_string().into());
        }
        if let Some(rent_epoch) = self.rent_epoch {
            account_info.insert("rentEpoch".to_string(), rent_epoch.into());
        }
        if let Some(executable) = self.executable {
            account_info.insert("executable".to_string(), executable.into());
        }

        serde_json::json!([self.address.to_string(), account_info])
    }
}
