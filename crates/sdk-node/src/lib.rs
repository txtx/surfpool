#[macro_use]
extern crate napi_derive;

use napi::{Error, Result, Status};
use solana_keypair::Keypair;
use solana_pubkey::Pubkey;
use solana_signer::Signer;
use surfpool_sdk::BlockProductionMode;

/// A running Surfpool instance with RPC/WS endpoints on dynamic ports.
#[napi]
pub struct Surfnet {
    inner: surfpool_sdk::Surfnet,
}

#[napi]
impl Surfnet {
    /// Start a surfnet with default settings (offline, transaction-mode blocks, 10 SOL payer).
    #[napi(factory)]
    pub fn start() -> Result<Self> {
        let inner = hiro_system_kit::nestable_block_on(surfpool_sdk::Surfnet::start())
            .map_err(|e| Error::new(Status::GenericFailure, e.to_string()))?;
        Ok(Self { inner })
    }

    /// Start a surfnet with custom configuration.
    #[napi(factory)]
    pub fn start_with_config(config: SurfnetConfig) -> Result<Self> {
        let mut builder = surfpool_sdk::Surfnet::builder();

        if let Some(offline) = config.offline {
            builder = builder.offline(offline);
        }
        if let Some(url) = config.remote_rpc_url {
            builder = builder.remote_rpc_url(url);
        }
        if let Some(mode) = config.block_production_mode.as_deref() {
            builder = builder.block_production_mode(match mode {
                "clock" => BlockProductionMode::Clock,
                "manual" => BlockProductionMode::Manual,
                _ => BlockProductionMode::Transaction,
            });
        }
        if let Some(ms) = config.slot_time_ms {
            builder = builder.slot_time_ms(ms as u64);
        }
        if let Some(lamports) = config.airdrop_sol {
            builder = builder.airdrop_sol(lamports as u64);
        }

        let inner = hiro_system_kit::nestable_block_on(builder.start())
            .map_err(|e| Error::new(Status::GenericFailure, e.to_string()))?;
        Ok(Self { inner })
    }

    /// The HTTP RPC URL (e.g. "http://127.0.0.1:12345").
    #[napi(getter)]
    pub fn rpc_url(&self) -> String {
        self.inner.rpc_url().to_string()
    }

    /// The WebSocket URL (e.g. "ws://127.0.0.1:12346").
    #[napi(getter)]
    pub fn ws_url(&self) -> String {
        self.inner.ws_url().to_string()
    }

    /// The pre-funded payer public key as base58 string.
    #[napi(getter)]
    pub fn payer(&self) -> String {
        self.inner.payer().pubkey().to_string()
    }

    /// The pre-funded payer secret key as a 64-byte Uint8Array.
    #[napi(getter)]
    pub fn payer_secret_key(&self) -> Vec<u8> {
        self.inner.payer().to_bytes().to_vec()
    }

    /// Fund a SOL account with lamports.
    #[napi]
    pub fn fund_sol(&self, address: String, lamports: f64) -> Result<()> {
        let pubkey: Pubkey = address
            .parse()
            .map_err(|e| Error::new(Status::InvalidArg, format!("Invalid address: {e}")))?;
        self.inner
            .cheatcodes()
            .fund_sol(&pubkey, lamports as u64)
            .map_err(|e| Error::new(Status::GenericFailure, e.to_string()))
    }

    /// Fund multiple SOL accounts with explicit lamport balances.
    #[napi]
    pub fn fund_sol_many(&self, accounts: Vec<SolAccountFunding>) -> Result<()> {
        let parsed_accounts = accounts
            .iter()
            .map(|account| {
                account
                    .address
                    .parse::<Pubkey>()
                    .map(|pubkey| (pubkey, account.lamports as u64))
                    .map_err(|e| {
                        Error::new(
                            Status::InvalidArg,
                            format!("Invalid address {}: {e}", account.address),
                        )
                    })
            })
            .collect::<Result<Vec<_>>>()?;
        let account_refs = parsed_accounts
            .iter()
            .map(|(pubkey, lamports)| (pubkey, *lamports))
            .collect::<Vec<_>>();

        self.inner
            .cheatcodes()
            .fund_sol_many(&account_refs)
            .map_err(|e| Error::new(Status::GenericFailure, e.to_string()))
    }

    /// Fund a token account (creates the ATA if needed).
    /// Uses spl_token program by default. Pass token_program for Token-2022.
    #[napi]
    pub fn fund_token(
        &self,
        owner: String,
        mint: String,
        amount: f64,
        token_program: Option<String>,
    ) -> Result<()> {
        let owner_pk: Pubkey = owner
            .parse()
            .map_err(|e| Error::new(Status::InvalidArg, format!("Invalid owner: {e}")))?;
        let mint_pk: Pubkey = mint
            .parse()
            .map_err(|e| Error::new(Status::InvalidArg, format!("Invalid mint: {e}")))?;
        let tp = token_program
            .map(|s| {
                s.parse::<Pubkey>().map_err(|e| {
                    Error::new(Status::InvalidArg, format!("Invalid token program: {e}"))
                })
            })
            .transpose()?;

        self.inner
            .cheatcodes()
            .fund_token(&owner_pk, &mint_pk, amount as u64, tp.as_ref())
            .map_err(|e| Error::new(Status::GenericFailure, e.to_string()))
    }

    /// Set the token balance for a wallet/mint pair.
    #[napi]
    pub fn set_token_balance(
        &self,
        owner: String,
        mint: String,
        amount: f64,
        token_program: Option<String>,
    ) -> Result<()> {
        let owner_pk: Pubkey = owner
            .parse()
            .map_err(|e| Error::new(Status::InvalidArg, format!("Invalid owner: {e}")))?;
        let mint_pk: Pubkey = mint
            .parse()
            .map_err(|e| Error::new(Status::InvalidArg, format!("Invalid mint: {e}")))?;
        let tp = token_program
            .map(|s| {
                s.parse::<Pubkey>().map_err(|e| {
                    Error::new(Status::InvalidArg, format!("Invalid token program: {e}"))
                })
            })
            .transpose()?;

        self.inner
            .cheatcodes()
            .set_token_balance(&owner_pk, &mint_pk, amount as u64, tp.as_ref())
            .map_err(|e| Error::new(Status::GenericFailure, e.to_string()))
    }

    /// Fund multiple wallets with the same token and amount.
    #[napi]
    pub fn fund_token_many(
        &self,
        owners: Vec<String>,
        mint: String,
        amount: f64,
        token_program: Option<String>,
    ) -> Result<()> {
        let owner_pubkeys = owners
            .iter()
            .map(|owner| {
                owner.parse::<Pubkey>().map_err(|e| {
                    Error::new(Status::InvalidArg, format!("Invalid owner {owner}: {e}"))
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let owner_refs = owner_pubkeys.iter().collect::<Vec<_>>();
        let mint_pk: Pubkey = mint
            .parse()
            .map_err(|e| Error::new(Status::InvalidArg, format!("Invalid mint: {e}")))?;
        let tp = token_program
            .map(|s| {
                s.parse::<Pubkey>().map_err(|e| {
                    Error::new(Status::InvalidArg, format!("Invalid token program: {e}"))
                })
            })
            .transpose()?;

        self.inner
            .cheatcodes()
            .fund_token_many(&owner_refs, &mint_pk, amount as u64, tp.as_ref())
            .map_err(|e| Error::new(Status::GenericFailure, e.to_string()))
    }

    /// Set arbitrary account data.
    #[napi]
    pub fn set_account(
        &self,
        address: String,
        lamports: f64,
        data: Vec<u8>,
        owner: String,
    ) -> Result<()> {
        let addr: Pubkey = address
            .parse()
            .map_err(|e| Error::new(Status::InvalidArg, format!("Invalid address: {e}")))?;
        let owner_pk: Pubkey = owner
            .parse()
            .map_err(|e| Error::new(Status::InvalidArg, format!("Invalid owner: {e}")))?;
        self.inner
            .cheatcodes()
            .set_account(&addr, lamports as u64, &data, &owner_pk)
            .map_err(|e| Error::new(Status::GenericFailure, e.to_string()))
    }

    /// Move Surfnet time forward to an absolute epoch.
    #[napi]
    pub fn time_travel_to_epoch(&self, epoch: f64) -> Result<EpochInfoValue> {
        self.inner
            .cheatcodes()
            .time_travel_to_epoch(epoch as u64)
            .map(EpochInfoValue::from)
            .map_err(|e| Error::new(Status::GenericFailure, e.to_string()))
    }

    /// Move Surfnet time forward to an absolute slot.
    #[napi]
    pub fn time_travel_to_slot(&self, slot: f64) -> Result<EpochInfoValue> {
        self.inner
            .cheatcodes()
            .time_travel_to_slot(slot as u64)
            .map(EpochInfoValue::from)
            .map_err(|e| Error::new(Status::GenericFailure, e.to_string()))
    }

    /// Move Surfnet time forward to an absolute Unix timestamp in milliseconds.
    #[napi]
    pub fn time_travel_to_timestamp(&self, timestamp: f64) -> Result<EpochInfoValue> {
        self.inner
            .cheatcodes()
            .time_travel_to_timestamp(timestamp as u64)
            .map(EpochInfoValue::from)
            .map_err(|e| Error::new(Status::GenericFailure, e.to_string()))
    }

    /// Deploy a program by discovering local program artifacts.
    #[napi]
    pub fn deploy_program(&self, program_name: String) -> Result<String> {
        self.inner
            .cheatcodes()
            .deploy_program(&program_name)
            .map_err(|e| Error::new(Status::GenericFailure, e.to_string()))
            .map(|program_id| program_id.to_string())
    }

    /// Get the associated token address for a wallet/mint pair.
    #[napi]
    pub fn get_ata(
        &self,
        owner: String,
        mint: String,
        token_program: Option<String>,
    ) -> Result<String> {
        let owner_pk: Pubkey = owner
            .parse()
            .map_err(|e| Error::new(Status::InvalidArg, format!("Invalid owner: {e}")))?;
        let mint_pk: Pubkey = mint
            .parse()
            .map_err(|e| Error::new(Status::InvalidArg, format!("Invalid mint: {e}")))?;
        let tp = token_program
            .map(|s| {
                s.parse::<Pubkey>().map_err(|e| {
                    Error::new(Status::InvalidArg, format!("Invalid token program: {e}"))
                })
            })
            .transpose()?;
        Ok(self
            .inner
            .cheatcodes()
            .get_ata(&owner_pk, &mint_pk, tp.as_ref())
            .to_string())
    }

    /// Generate a new random keypair. Returns [publicKey, secretKey] as base58 and bytes.
    #[napi]
    pub fn new_keypair() -> KeypairInfo {
        let kp = Keypair::new();
        KeypairInfo {
            public_key: kp.pubkey().to_string(),
            secret_key: kp.to_bytes().to_vec(),
        }
    }
}

#[napi(object)]
pub struct SurfnetConfig {
    pub offline: Option<bool>,
    pub remote_rpc_url: Option<String>,
    pub block_production_mode: Option<String>,
    pub slot_time_ms: Option<f64>,
    pub airdrop_sol: Option<f64>,
}

#[napi(object)]
pub struct KeypairInfo {
    pub public_key: String,
    pub secret_key: Vec<u8>,
}

#[napi(object)]
pub struct SolAccountFunding {
    pub address: String,
    pub lamports: f64,
}

#[napi(object)]
pub struct EpochInfoValue {
    pub epoch: f64,
    pub slot_index: f64,
    pub slots_in_epoch: f64,
    pub absolute_slot: f64,
    pub block_height: f64,
    pub transaction_count: Option<f64>,
}

impl From<solana_epoch_info::EpochInfo> for EpochInfoValue {
    fn from(value: solana_epoch_info::EpochInfo) -> Self {
        Self {
            epoch: value.epoch as f64,
            slot_index: value.slot_index as f64,
            slots_in_epoch: value.slots_in_epoch as f64,
            absolute_slot: value.absolute_slot as f64,
            block_height: value.block_height as f64,
            transaction_count: value.transaction_count.map(|count| count as f64),
        }
    }
}
