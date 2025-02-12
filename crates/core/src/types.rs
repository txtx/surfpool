use solana_sdk::pubkey::Pubkey;
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum RunloopTriggerMode {
    #[default]
    Clock,
    Manual,
    Transaction,
}

#[derive(Clone, Debug, Default)]
pub struct SurfpoolConfig {
    pub simnet: SimnetConfig,
    pub rpc: RpcConfig,
    pub plugin_config_path: Vec<PathBuf>,
}

#[derive(Clone, Debug)]
pub struct SimnetConfig {
    pub remote_rpc_url: String,
    pub slot_time: u64,
    pub runloop_trigger_mode: RunloopTriggerMode,
    pub airdrop_addresses: Vec<Pubkey>,
    pub airdrop_token_amount: u64,
}

impl Default for SimnetConfig {
    fn default() -> Self {
        Self {
            remote_rpc_url: "https://api.mainnet-beta.solana.com".to_string(),
            slot_time: 0,
            runloop_trigger_mode: RunloopTriggerMode::Clock,
            airdrop_addresses: vec![],
            airdrop_token_amount: 0,
        }
    }
}

#[derive(Clone, Debug)]
pub struct RpcConfig {
    pub bind_host: String,
    pub bind_port: u16,
    pub remote_rpc_url: String,
}

impl RpcConfig {
    pub fn get_socket_address(&self) -> String {
        format!("{}:{}", self.bind_host, self.bind_port)
    }
}

impl Default for RpcConfig {
    fn default() -> Self {
        Self {
            remote_rpc_url: "https://api.mainnet-beta.solana.com".to_string(),
            bind_host: "127.0.0.1".to_string(),
            bind_port: 8899,
        }
    }
}
