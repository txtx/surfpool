use crate::PluginManagerCommand;
use jsonrpc_core::BoxFuture;
use jsonrpc_core::Result;
use jsonrpc_derive::rpc;
use solana_client::rpc_config::RpcAccountIndex;
use solana_sdk::pubkey::Pubkey;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Duration;
use std::time::SystemTime;
use surfpool_types::subgraph::PluginConfig;
use uuid::Uuid;

use super::not_implemented_err;
use super::not_implemented_err_async;
use super::RunloopContext;

#[rpc]
pub trait AdminRpc {
    type Metadata;

    #[rpc(meta, name = "exit")]
    fn exit(&self, meta: Self::Metadata) -> Result<()>;

    #[rpc(meta, name = "reloadPlugin")]
    fn reload_plugin(
        &self,
        meta: Self::Metadata,
        name: String,
        config_file: String,
    ) -> BoxFuture<Result<()>>;

    #[rpc(meta, name = "unloadPlugin")]
    fn unload_plugin(&self, meta: Self::Metadata, name: String) -> BoxFuture<Result<()>>;

    #[rpc(meta, name = "loadPlugin")]
    fn load_plugin(&self, meta: Self::Metadata, config_file: String) -> BoxFuture<Result<String>>;

    #[rpc(meta, name = "listPlugins")]
    fn list_plugins(&self, meta: Self::Metadata) -> BoxFuture<Result<Vec<String>>>;

    #[rpc(meta, name = "rpcAddress")]
    fn rpc_addr(&self, meta: Self::Metadata) -> Result<Option<SocketAddr>>;

    #[rpc(name = "setLogFilter")]
    fn set_log_filter(&self, filter: String) -> Result<()>;

    #[rpc(meta, name = "startTime")]
    fn start_time(&self, meta: Self::Metadata) -> Result<SystemTime>;

    // #[rpc(meta, name = "startProgress")]
    // fn start_progress(&self, meta: Self::Metadata) -> Result<ValidatorStartProgress>;

    #[rpc(meta, name = "addAuthorizedVoter")]
    fn add_authorized_voter(&self, meta: Self::Metadata, keypair_file: String) -> Result<()>;

    #[rpc(meta, name = "addAuthorizedVoterFromBytes")]
    fn add_authorized_voter_from_bytes(&self, meta: Self::Metadata, keypair: Vec<u8>)
        -> Result<()>;

    #[rpc(meta, name = "removeAllAuthorizedVoters")]
    fn remove_all_authorized_voters(&self, meta: Self::Metadata) -> Result<()>;

    #[rpc(meta, name = "setIdentity")]
    fn set_identity(
        &self,
        meta: Self::Metadata,
        keypair_file: String,
        require_tower: bool,
    ) -> Result<()>;

    #[rpc(meta, name = "setIdentityFromBytes")]
    fn set_identity_from_bytes(
        &self,
        meta: Self::Metadata,
        identity_keypair: Vec<u8>,
        require_tower: bool,
    ) -> Result<()>;

    #[rpc(meta, name = "setStakedNodesOverrides")]
    fn set_staked_nodes_overrides(&self, meta: Self::Metadata, path: String) -> Result<()>;

    // #[rpc(meta, name = "contactInfo")]
    // fn contact_info(&self, meta: Self::Metadata) -> Result<AdminRpcContactInfo>;

    #[rpc(meta, name = "repairShredFromPeer")]
    fn repair_shred_from_peer(
        &self,
        meta: Self::Metadata,
        pubkey: Option<Pubkey>,
        slot: u64,
        shred_index: u64,
    ) -> Result<()>;

    // #[rpc(meta, name = "repairWhitelist")]
    // fn repair_whitelist(&self, meta: Self::Metadata) -> Result<AdminRpcRepairWhitelist>;

    #[rpc(meta, name = "setRepairWhitelist")]
    fn set_repair_whitelist(&self, meta: Self::Metadata, whitelist: Vec<Pubkey>) -> Result<()>;

    #[rpc(meta, name = "getSecondaryIndexKeySize")]
    fn get_secondary_index_key_size(
        &self,
        meta: Self::Metadata,
        pubkey_str: String,
    ) -> Result<HashMap<RpcAccountIndex, usize>>;

    #[rpc(meta, name = "setPublicTpuAddress")]
    fn set_public_tpu_address(
        &self,
        meta: Self::Metadata,
        public_tpu_addr: SocketAddr,
    ) -> Result<()>;

    #[rpc(meta, name = "setPublicTpuForwardsAddress")]
    fn set_public_tpu_forwards_address(
        &self,
        meta: Self::Metadata,
        public_tpu_forwards_addr: SocketAddr,
    ) -> Result<()>;
}

pub struct SurfpoolAdminRpc;
impl AdminRpc for SurfpoolAdminRpc {
    type Metadata = Option<RunloopContext>;

    fn exit(&self, _meta: Self::Metadata) -> Result<()> {
        not_implemented_err()
    }

    fn reload_plugin(
        &self,
        _meta: Self::Metadata,
        _name: String,
        _config_file: String,
    ) -> BoxFuture<Result<()>> {
        not_implemented_err_async()
    }

    fn unload_plugin(&self, _meta: Self::Metadata, _name: String) -> BoxFuture<Result<()>> {
        not_implemented_err_async()
    }

    fn load_plugin(&self, meta: Self::Metadata, config_file: String) -> BoxFuture<Result<String>> {
        let config = match serde_json::from_str::<PluginConfig>(&config_file)
            .map_err(|e| format!("failed to parse plugin config: {e}"))
        {
            Ok(config) => config,
            Err(e) => return Box::pin(async move { Err(jsonrpc_core::Error::invalid_params(&e)) }),
        };
        let ctx = meta.unwrap();
        let uuid = Uuid::new_v4();
        let (tx, rx) = crossbeam_channel::bounded(1);
        let _ = ctx
            .plugin_manager_commands_tx
            .send(PluginManagerCommand::LoadConfig(uuid, config, tx));

        let Ok(endpoint_url) = rx.recv_timeout(Duration::from_secs(10)) else {
            return Box::pin(async move { Err(jsonrpc_core::Error::internal_error()) });
        };
        Box::pin(async move { Ok(endpoint_url) })
    }

    fn list_plugins(&self, _meta: Self::Metadata) -> BoxFuture<Result<Vec<String>>> {
        not_implemented_err_async()
    }

    fn rpc_addr(&self, _meta: Self::Metadata) -> Result<Option<SocketAddr>> {
        not_implemented_err()
    }

    fn set_log_filter(&self, _filter: String) -> Result<()> {
        not_implemented_err()
    }

    fn start_time(&self, _meta: Self::Metadata) -> Result<SystemTime> {
        not_implemented_err()
    }

    fn add_authorized_voter(&self, _meta: Self::Metadata, _keypair_file: String) -> Result<()> {
        not_implemented_err()
    }

    fn add_authorized_voter_from_bytes(
        &self,
        _meta: Self::Metadata,
        _keypair: Vec<u8>,
    ) -> Result<()> {
        not_implemented_err()
    }

    fn remove_all_authorized_voters(&self, _meta: Self::Metadata) -> Result<()> {
        not_implemented_err()
    }

    fn set_identity(
        &self,
        _meta: Self::Metadata,
        _keypair_file: String,
        _require_tower: bool,
    ) -> Result<()> {
        not_implemented_err()
    }

    fn set_identity_from_bytes(
        &self,
        _meta: Self::Metadata,
        _identity_keypair: Vec<u8>,
        _require_tower: bool,
    ) -> Result<()> {
        not_implemented_err()
    }

    fn set_staked_nodes_overrides(&self, _meta: Self::Metadata, _path: String) -> Result<()> {
        not_implemented_err()
    }

    fn repair_shred_from_peer(
        &self,
        _meta: Self::Metadata,
        _pubkey: Option<Pubkey>,
        _slot: u64,
        _shred_index: u64,
    ) -> Result<()> {
        not_implemented_err()
    }

    fn set_repair_whitelist(&self, _meta: Self::Metadata, _whitelist: Vec<Pubkey>) -> Result<()> {
        not_implemented_err()
    }

    fn get_secondary_index_key_size(
        &self,
        _meta: Self::Metadata,
        _pubkey_str: String,
    ) -> Result<HashMap<RpcAccountIndex, usize>> {
        not_implemented_err()
    }

    fn set_public_tpu_address(
        &self,
        _meta: Self::Metadata,
        _public_tpu_addr: SocketAddr,
    ) -> Result<()> {
        not_implemented_err()
    }

    fn set_public_tpu_forwards_address(
        &self,
        _meta: Self::Metadata,
        _public_tpu_forwards_addr: SocketAddr,
    ) -> Result<()> {
        not_implemented_err()
    }
}
