use std::{
    collections::HashMap,
    net::SocketAddr,
    time::{Duration, SystemTime},
};

use jsonrpc_core::{BoxFuture, Result};
use jsonrpc_derive::rpc;
use solana_client::{rpc_config::RpcAccountIndex, rpc_custom_error::RpcCustomError};
use solana_pubkey::Pubkey;
use surfpool_types::{SimnetCommand, SimnetEvent};
use txtx_addon_network_svm_types::subgraph::PluginConfig;
use uuid::Uuid;

use super::{RunloopContext, not_implemented_err, not_implemented_err_async};
use crate::PluginManagerCommand;

#[rpc]
pub trait AdminRpc {
    type Metadata;

    /// Immediately shuts down the RPC server.
    ///
    /// This administrative endpoint is typically used during controlled shutdowns of the validator
    /// or service exposing the RPC interface. It allows remote administrators to gracefully terminate
    /// the process, stopping all RPC activity.
    ///
    /// ## Returns
    /// - [`Result<()>`] — A unit result indicating successful shutdown, or an error if the call fails.
    ///
    /// ## Example Request (JSON-RPC)
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 42,
    ///   "method": "exit",
    ///   "params": []
    /// }
    /// ```
    ///
    /// # Notes
    /// - This method is privileged and should only be accessible to trusted clients.
    /// - Use with extreme caution in production environments.
    /// - If successful, the RPC server process will terminate immediately after processing this call.
    ///
    /// # Security
    /// Access to this method should be tightly restricted. Implement proper authorization mechanisms
    /// via the RPC metadata to prevent accidental or malicious use.
    #[rpc(meta, name = "exit")]
    fn exit(&self, meta: Self::Metadata) -> Result<()>;

    #[rpc(meta, name = "reloadPlugin")]
    fn reload_plugin(
        &self,
        meta: Self::Metadata,
        name: String,
        config_file: String,
    ) -> BoxFuture<Result<()>>;

    /// Reloads a runtime plugin with new configuration.
    ///
    /// This administrative endpoint is used to dynamically reload a plugin without restarting
    /// the entire RPC server or validator. It is useful for applying updated configurations
    /// to a plugin that supports hot-reloading.
    ///
    /// ## Parameters
    /// - `name`: The identifier of the plugin to reload.
    /// - `config_file`: Path to the new configuration file to load for the plugin.
    ///
    /// ## Returns
    /// - [`BoxFuture<Result<()>>`] — A future resolving to a unit result on success, or an error if
    ///   reloading fails.
    ///
    /// ## Example Request (JSON-RPC)
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 101,
    ///   "method": "reloadPlugin",
    ///   "params": ["myPlugin", "/etc/plugins/my_plugin_config.toml"]
    /// }
    /// ```
    ///
    /// # Notes
    /// - The plugin must support reloading in order for this to succeed.
    /// - A failed reload will leave the plugin in its previous state.
    /// - This method is intended for administrators and should be properly secured.
    ///
    /// # Security
    /// Ensure only trusted clients can invoke this method. Use metadata-based access control to limit exposure.
    #[rpc(meta, name = "unloadPlugin")]
    fn unload_plugin(&self, meta: Self::Metadata, name: String) -> BoxFuture<Result<()>>;

    /// Dynamically loads a new plugin into the runtime from a configuration file.
    ///
    /// This administrative endpoint is used to add a new plugin to the system at runtime,
    /// based on the configuration provided. It enables extensibility without restarting
    /// the validator or RPC server.
    ///
    /// ## Parameters
    /// - `config_file`: Path to the plugin's configuration file, which defines its behavior and settings.
    ///
    /// ## Returns
    /// - [`BoxFuture<Result<String>>`] — A future resolving to the name or identifier of the loaded plugin,
    ///   or an error if the plugin could not be loaded.
    ///
    /// ## Example Request (JSON-RPC)
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 102,
    ///   "method": "loadPlugin",
    ///   "params": ["/etc/plugins/my_plugin_config.toml"]
    /// }
    /// ```
    ///
    /// # Notes
    /// - The plugin system must be initialized and support runtime loading.
    /// - The config file should be well-formed and point to a valid plugin implementation.
    /// - Duplicate plugin names may lead to conflicts or errors.
    ///
    /// # Security
    /// This method should be restricted to administrators only. Validate inputs and use access control.
    #[rpc(meta, name = "loadPlugin")]
    fn load_plugin(&self, meta: Self::Metadata, config_file: String) -> BoxFuture<Result<String>>;

    /// Returns a list of all currently loaded plugin names.
    ///
    /// This administrative RPC method is used to inspect which plugins have been successfully
    /// loaded into the runtime. It can be useful for debugging or operational monitoring.
    ///
    /// ## Returns
    /// - `Vec<String>` — A list of plugin names currently active in the system.
    ///
    /// ## Example Request (JSON-RPC)
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 103,
    ///   "method": "listPlugins",
    ///   "params": []
    /// }
    /// ```
    ///
    /// ## Example Response
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": ["tx_filter", "custom_logger"],
    ///   "id": 103
    /// }
    /// ```
    ///
    /// # Notes
    /// - Only plugins that have been successfully loaded will appear in this list.
    /// - This method is read-only and safe to call frequently.
    #[rpc(meta, name = "listPlugins")]
    fn list_plugins(&self, meta: Self::Metadata) -> BoxFuture<Result<Vec<String>>>;

    /// Returns the address of the RPC server.
    ///
    /// This RPC method retrieves the network address (IP and port) the RPC server is currently
    /// listening on. It can be useful for service discovery or monitoring the server’s network status.
    ///
    /// ## Returns
    /// - `Option<SocketAddr>` — The network address of the RPC server, or `None` if no address is available.
    ///
    /// ## Example Request (JSON-RPC)
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 104,
    ///   "method": "rpcAddress",
    ///   "params": []
    /// }
    /// ```
    ///
    /// ## Example Response
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": "127.0.0.1:8080",
    ///   "id": 104
    /// }
    /// ```
    ///
    /// # Notes
    /// - This method is useful for finding the address of a running RPC server, especially in dynamic environments.
    /// - If the server is not configured or is running without network exposure, the result may be `None`.
    #[rpc(meta, name = "rpcAddress")]
    fn rpc_addr(&self, meta: Self::Metadata) -> Result<Option<SocketAddr>>;

    /// Returns the system start time.
    ///
    /// This RPC method retrieves the timestamp of when the system was started, represented as
    /// a `SystemTime`. It can be useful for measuring uptime or for tracking the system's runtime
    /// in logs or monitoring systems.
    ///
    /// ## Returns
    /// - `SystemTime` — The timestamp representing when the system was started.
    ///
    /// ## Example Request (JSON-RPC)
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 106,
    ///   "method": "startTime",
    ///   "params": []
    /// }
    /// ```
    ///
    /// ## Example Response
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": "2025-04-24T12:34:56Z",
    ///   "id": 106
    /// }
    /// ```
    ///
    /// # Notes
    /// - The result is a `SystemTime` in UTC, reflecting the moment the system was initialized.
    /// - This method is useful for monitoring system uptime and verifying system health.
    #[rpc(meta, name = "startTime")]
    fn start_time(&self, meta: Self::Metadata) -> Result<SystemTime>;

    // #[rpc(meta, name = "startProgress")]
    // fn start_progress(&self, meta: Self::Metadata) -> Result<ValidatorStartProgress>;

    /// Adds an authorized voter to the system.
    ///
    /// This RPC method allows an authorized user to add a new voter to the list of authorized
    /// voters. A voter is typically an entity that can participate in governance actions, such as
    /// voting on proposals or other decision-making processes.
    ///
    /// ## Parameters
    /// - `keypair_file`: A string representing the path to the file containing the keypair of the new voter.
    ///   The keypair is used to authenticate and authorize the voter to participate in governance.
    ///
    /// ## Returns
    /// - `()` — A unit result indicating that the operation was successful.
    ///
    /// ## Example Request (JSON-RPC)
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 107,
    ///   "method": "addAuthorizedVoter",
    ///   "params": ["path/to/voter_keypair.json"]
    /// }
    /// ```
    ///
    /// ## Example Response
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": null,
    ///   "id": 107
    /// }
    /// ```
    ///
    /// # Notes
    /// - This method typically requires administrative or elevated permissions to execute.
    /// - The `keypair_file` should contain a valid keypair for the new voter, formatted according to the system's expectations.
    /// - Once added, the new voter will be able to participate in governance actions.
    #[rpc(meta, name = "addAuthorizedVoter")]
    fn add_authorized_voter(&self, meta: Self::Metadata, keypair_file: String) -> Result<()>;

    /// Adds an authorized voter to the system using a byte-encoded keypair.
    ///
    /// This RPC method allows an authorized user to add a new voter by directly providing the
    /// keypair in the form of a byte vector (`Vec<u8>`). This can be useful for systems where keypairs
    /// are serialized or passed in a non-file-based format.
    ///
    /// ## Parameters
    /// - `keypair`: A vector of bytes representing the keypair of the new voter. This keypair will be used to
    ///   authenticate and authorize the voter to participate in governance actions.
    ///
    /// ## Returns
    /// - `()` — A unit result indicating the operation was successful.
    ///
    /// ## Example Request (JSON-RPC)
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 108,
    ///   "method": "addAuthorizedVoterFromBytes",
    ///   "params": ["<base64-encoded-keypair>"]
    /// }
    /// ```
    ///
    /// ## Example Response
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": null,
    ///   "id": 108
    /// }
    /// ```
    ///
    /// # Notes
    /// - This method allows for adding a voter directly from a byte-encoded keypair, which may be useful for
    ///   systems that store keypairs in non-traditional formats (e.g., databases, serialized data).
    /// - The `keypair` should be provided in the correct format expected by the system (typically a base64 or raw binary format).
    /// - This method typically requires administrative or elevated permissions to execute.
    /// - Once added, the new voter will be able to participate in governance actions.
    #[rpc(meta, name = "addAuthorizedVoterFromBytes")]
    fn add_authorized_voter_from_bytes(&self, meta: Self::Metadata, keypair: Vec<u8>)
    -> Result<()>;

    /// Removes all authorized voters from the system.
    ///
    /// This RPC method removes all voters from the list of authorized voters. This action
    /// is typically an administrative function and may be used to reset or clean up the list of
    /// voters in the system.
    ///
    /// ## Parameters
    /// - None.
    ///
    /// ## Returns
    /// - `()` — A unit result indicating that the operation was successful.
    ///
    /// ## Example Request (JSON-RPC)
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 109,
    ///   "method": "removeAllAuthorizedVoters",
    ///   "params": []
    /// }
    /// ```
    ///
    /// ## Example Response
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": null,
    ///   "id": 109
    /// }
    /// ```
    ///
    /// # Notes
    /// - This method will remove all voters and may not be reversible. It is a critical operation that
    ///   typically requires elevated administrative permissions.
    /// - Use this method with caution as it will prevent any previously authorized voter from participating
    ///   in governance actions until they are re-added.
    #[rpc(meta, name = "removeAllAuthorizedVoters")]
    fn remove_all_authorized_voters(&self, meta: Self::Metadata) -> Result<()>;

    /// Sets the identity for the system using the provided keypair.
    ///
    /// This RPC method allows you to set the system's identity by specifying a keypair file.
    /// The identity set here is typically used to authenticate the system and validate its
    /// actions in governance or other sensitive operations. The `require_tower` flag ensures
    /// that a specific security feature, called "tower," is enabled for the identity.
    ///
    /// ## Parameters
    /// - `keypair_file`: The file path to the keypair that will be used to set the system's identity.
    /// - `require_tower`: A boolean flag indicating whether the tower security feature should be enforced
    ///   for this identity.
    ///
    /// ## Returns
    /// - `()` — A unit result indicating that the operation was successful.
    ///
    /// ## Example Request (JSON-RPC)
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 110,
    ///   "method": "setIdentity",
    ///   "params": ["/path/to/keypair.json", true]
    /// }
    /// ```
    ///
    /// ## Example Response
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": null,
    ///   "id": 110
    /// }
    /// ```
    ///
    /// # Notes
    /// - This method allows for setting the identity of the system, which could have security and governance
    ///   implications. It is typically used when initializing or reconfiguring the system's identity.
    /// - The `require_tower` flag is optional but can be used to add an extra layer of security for the identity.
    /// - The method usually requires administrative permissions and a valid keypair to execute.
    #[rpc(meta, name = "setIdentity")]
    fn set_identity(
        &self,
        meta: Self::Metadata,
        keypair_file: String,
        require_tower: bool,
    ) -> Result<()>;

    /// Sets the identity for the system using a keypair provided as a byte array.
    ///
    /// This RPC method allows you to set the system's identity by directly providing a byte array
    /// representing the keypair. The `require_tower` flag is used to enforce the "tower" security feature
    /// for this identity, if needed.
    ///
    /// ## Parameters
    /// - `identity_keypair`: A byte array representing the keypair to set the system's identity.
    /// - `require_tower`: A boolean flag indicating whether the tower security feature should be enforced
    ///   for this identity.
    ///
    /// ## Returns
    /// - `()` — A unit result indicating that the operation was successful.
    ///
    /// ## Example Request (JSON-RPC)
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 111,
    ///   "method": "setIdentityFromBytes",
    ///   "params": [[72, 101, 108, 108, 111], true]
    /// }
    /// ```
    ///
    /// ## Example Response
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": null,
    ///   "id": 111
    /// }
    /// ```
    ///
    /// # Notes
    /// - This method is useful for scenarios where the keypair is not stored in a file but is instead available
    ///   as a byte array (e.g., for programmatically generated keypairs).
    /// - The `require_tower` flag, when set to `true`, enforces additional security for the identity.
    /// - The method typically requires administrative permissions to execute.
    #[rpc(meta, name = "setIdentityFromBytes")]
    fn set_identity_from_bytes(
        &self,
        meta: Self::Metadata,
        identity_keypair: Vec<u8>,
        require_tower: bool,
    ) -> Result<()>;

    /// Sets the overrides for staked nodes using a specified path.
    ///
    /// This RPC method allows you to configure overrides for staked nodes by specifying the path
    /// to a configuration file. This is typically used for adjusting the parameters or behavior
    /// of the staked nodes in the system, such as custom settings for node management or staking
    /// operations.
    ///
    /// ## Parameters
    /// - `path`: The file path to the configuration file that contains the staked node overrides.
    ///   This file should define the necessary settings for overriding the default staked node configuration.
    ///
    /// ## Returns
    /// - `()` — A unit result indicating that the operation was successful.
    ///
    /// ## Example Request (JSON-RPC)
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 112,
    ///   "method": "setStakedNodesOverrides",
    ///   "params": ["/path/to/overrides.json"]
    /// }
    /// ```
    ///
    /// ## Example Response
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": null,
    ///   "id": 112
    /// }
    /// ```
    ///
    /// # Notes
    /// - This method is used for overriding the configuration of staked nodes in the system.
    ///   The `path` parameter should point to a file that contains the necessary overrides.
    /// - It may require administrative permissions to modify the staked node configurations.
    /// - The path file should be in a format understood by the system (e.g., JSON, YAML).
    #[rpc(meta, name = "setStakedNodesOverrides")]
    fn set_staked_nodes_overrides(&self, meta: Self::Metadata, path: String) -> Result<()>;

    /// Repairs a shred from a peer node in the network.
    ///
    /// This RPC method triggers the repair of a specific shred from a peer node, using the given
    /// `pubkey` (if provided), `slot`, and `shred_index`. This is typically used in cases where
    /// a shred is missing or corrupted and needs to be retrieved from another node in the network.
    ///
    /// ## Parameters
    /// - `pubkey` (Optional): The public key of the node from which to request the shred. If `None`,
    ///   the system may choose any peer to attempt the repair from.
    /// - `slot`: The slot number where the shred is located.
    /// - `shred_index`: The index of the specific shred within the given slot that needs repair.
    ///
    /// ## Returns
    /// - `()` — A unit result indicating that the operation was successful.
    ///
    /// ## Example Request (JSON-RPC)
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 113,
    ///   "method": "repairShredFromPeer",
    ///   "params": ["PubkeyHere", 12345, 6789]
    /// }
    /// ```
    ///
    /// ## Example Response
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": null,
    ///   "id": 113
    /// }
    /// ```
    ///
    /// # Notes
    /// - The method may require specific network configurations or permissions to allow for the repair of the shred.
    /// - If the `pubkey` is provided, the system will attempt to retrieve the shred from the specified peer node. Otherwise,
    ///   it will attempt the repair from any available node in the network.
    #[rpc(meta, name = "repairShredFromPeer")]
    fn repair_shred_from_peer(
        &self,
        meta: Self::Metadata,
        pubkey: Option<Pubkey>,
        slot: u64,
        shred_index: u64,
    ) -> Result<()>;

    /// Sets the whitelist of nodes allowed to repair shreds.
    ///
    /// This RPC method sets a list of nodes (identified by their public keys) that are permitted
    /// to repair shreds in the network. The whitelist controls which nodes have the authority to
    /// perform repairs. Any node not included in the whitelist will be restricted from initiating
    /// shred repair operations.
    ///
    /// ## Parameters
    /// - `whitelist`: A vector of `Pubkey` values representing the public keys of the nodes
    ///   that are authorized to repair shreds.
    ///
    /// ## Returns
    /// - `()` — A unit result indicating that the operation was successful.
    ///
    /// ## Example Request (JSON-RPC)
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 114,
    ///   "method": "setRepairWhitelist",
    ///   "params": [["Pubkey1", "Pubkey2", "Pubkey3"]]
    /// }
    /// ```
    ///
    /// ## Example Response
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": null,
    ///   "id": 114
    /// }
    /// ```
    ///
    /// # Notes
    /// - This method is generally used by network administrators to control which nodes are trusted
    ///   to perform repairs on the network.
    /// - The whitelist ensures that only authorized nodes can engage in potentially sensitive network repair actions.
    #[rpc(meta, name = "setRepairWhitelist")]
    fn set_repair_whitelist(&self, meta: Self::Metadata, whitelist: Vec<Pubkey>) -> Result<()>;

    /// Retrieves the size of the secondary index key for a given account.
    ///
    /// This RPC method returns the size of the secondary index key associated with a specific
    /// account, identified by its public key. The secondary index key is used in the indexing
    /// mechanism to quickly access account-related data.
    ///
    /// ## Parameters
    /// - `pubkey_str`: A string representing the public key of the account for which the
    ///   secondary index key size is being queried.
    ///
    /// ## Returns
    /// - `HashMap<RpcAccountIndex, usize>`: A mapping of account indices to their respective key sizes.
    ///
    /// ## Example Request (JSON-RPC)
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 115,
    ///   "method": "getSecondaryIndexKeySize",
    ///   "params": ["PubkeyString"]
    /// }
    /// ```
    ///
    /// ## Example Response
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": {
    ///     "programId": 128,
    ///     "splTokenMint": 256,
    ///     "splTokenOwner": 192
    ///   },
    ///   "id": 115
    /// }
    /// ```
    ///
    /// # Notes
    /// - The returned `HashMap` will contain index types as keys, and the size of each key
    #[rpc(meta, name = "getSecondaryIndexKeySize")]
    fn get_secondary_index_key_size(
        &self,
        meta: Self::Metadata,
        pubkey_str: String,
    ) -> Result<HashMap<RpcAccountIndex, usize>>;

    /// Sets the public TPU (Transaction Processing Unit) address.
    ///
    /// This RPC method is used to configure the public TPU address of the node. The TPU address
    /// is used for communication between the validator and the network, allowing the node to
    /// send and receive transactions.
    ///
    /// ## Parameters
    /// - `public_tpu_addr`: A `SocketAddr` representing the public TPU address to be set.
    ///
    /// ## Returns
    /// - `Result<()>`: Returns `Ok(())` if the operation was successful, or an error if
    ///   there was an issue setting the TPU address.
    ///
    /// ## Example Request (JSON-RPC)
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 118,
    ///   "method": "setPublicTpuAddress",
    ///   "params": ["127.0.0.1:8000"]
    /// }
    /// ```
    ///
    /// ## Example Response
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": null,
    ///   "id": 118
    /// }
    /// ```
    ///
    /// # Notes
    /// - The TPU address is important for a validator node to participate in transaction
    ///   processing and communication with the rest of the Solana network.
    /// - This method is typically used to configure the node's external-facing address, allowing
    ///   it to communicate with clients or other validators.
    ///
    /// ## Errors
    /// - If the provided address is invalid or there is a failure when setting the address,
    ///   an error will be returned.
    #[rpc(meta, name = "setPublicTpuAddress")]
    fn set_public_tpu_address(
        &self,
        meta: Self::Metadata,
        public_tpu_addr: SocketAddr,
    ) -> Result<()>;

    /// Sets the public TPU forwards address.
    ///
    /// This RPC method configures the public address for TPU forwarding. It is used to specify
    /// a separate address for forwarding transactions to a different destination, often used
    /// for specialized network configurations or load balancing.
    ///
    /// ## Parameters
    /// - `public_tpu_forwards_addr`: A `SocketAddr` representing the public TPU forwards
    ///   address to be set.
    ///
    /// ## Returns
    /// - `Result<()>`: Returns `Ok(())` if the operation was successful, or an error if
    ///   there was an issue setting the TPU forwards address.
    ///
    /// ## Example Request (JSON-RPC)
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "id": 118,
    ///   "method": "setPublicTpuForwardsAddress",
    ///   "params": ["127.0.0.1:9000"]
    /// }
    /// ```
    ///
    /// ## Example Response
    /// ```json
    /// {
    ///   "jsonrpc": "2.0",
    ///   "result": null,
    ///   "id": 118
    /// }
    /// ```
    ///
    /// # Notes
    /// - This method is typically used for advanced network configurations, where a node might
    ///   want to forward its transaction processing requests to another address.
    /// - The provided `SocketAddr` should be valid and reachable.
    ///
    /// ## Errors
    /// - If the provided address is invalid or there is a failure when setting the address,
    ///   an error will be returned.
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

    fn exit(&self, meta: Self::Metadata) -> Result<()> {
        let Some(ctx) = meta else {
            return Err(RpcCustomError::NodeUnhealthy {
                num_slots_behind: None,
            }
            .into());
        };

        let _ = ctx
            .simnet_commands_tx
            .send(SimnetCommand::Terminate(ctx.id));

        Ok(())
    }

    fn reload_plugin(
        &self,
        meta: Self::Metadata,
        name: String,
        config_file: String,
    ) -> BoxFuture<Result<()>> {
        // Parse the UUID from the name parameter
        let uuid = match Uuid::parse_str(&name) {
            Ok(uuid) => uuid,
            Err(e) => {
                return Box::pin(async move {
                    Err(jsonrpc_core::Error::invalid_params(format!(
                        "Invalid UUID: {}",
                        e
                    )))
                });
            }
        };

        // Parse the new configuration
        let config = match serde_json::from_str::<PluginConfig>(&config_file)
            .map_err(|e| format!("failed to deserialize plugin config: {e}"))
        {
            Ok(config) => config,
            Err(e) => return Box::pin(async move { Err(jsonrpc_core::Error::invalid_params(&e)) }),
        };

        let Some(ctx) = meta else {
            return Box::pin(async move { Err(jsonrpc_core::Error::internal_error()) });
        };

        let simnet_events_tx = ctx.svm_locker.simnet_events_tx();
        let _ = simnet_events_tx.try_send(SimnetEvent::info(format!(
            "Reloading plugin with UUID - {}",
            uuid
        )));

        let (tx, rx) = crossbeam_channel::bounded(1);
        let _ = ctx
            .plugin_manager_commands_tx
            .send(PluginManagerCommand::ReloadPlugin(uuid, config, tx));

        let Ok(_endpoint_url) = rx.recv_timeout(Duration::from_secs(10)) else {
            return Box::pin(async move { Err(jsonrpc_core::Error::internal_error()) });
        };

        Box::pin(async move {
            let _ = simnet_events_tx.try_send(SimnetEvent::info(format!(
                "Reloaded plugin with UUID - {}",
                uuid
            )));
            Ok(())
        })
    }

    fn unload_plugin(&self, meta: Self::Metadata, name: String) -> BoxFuture<Result<()>> {
        // Parse the UUID from the name parameter
        let uuid = match Uuid::parse_str(&name) {
            Ok(uuid) => uuid,
            Err(e) => {
                return Box::pin(async move {
                    Err(jsonrpc_core::Error::invalid_params(format!(
                        "Invalid UUID: {}",
                        e
                    )))
                });
            }
        };

        let Some(ctx) = meta else {
            return Box::pin(async move { Err(jsonrpc_core::Error::internal_error()) });
        };

        let simnet_events_tx = ctx.svm_locker.simnet_events_tx();
        let (tx, rx) = crossbeam_channel::bounded(1);
        let _ = ctx
            .plugin_manager_commands_tx
            .send(PluginManagerCommand::UnloadPlugin(uuid, tx));

        let Ok(result) = rx.recv_timeout(Duration::from_secs(10)) else {
            return Box::pin(async move { Err(jsonrpc_core::Error::internal_error()) });
        };

        Box::pin(async move {
            match result {
                Ok(()) => {
                    let _ = simnet_events_tx.try_send(SimnetEvent::info(format!(
                        "Unloaded plugin with UUID - {}",
                        uuid
                    )));
                    Ok(())
                }
                Err(e) => Err(jsonrpc_core::Error::invalid_params(&e)),
            }
        })
    }

    fn load_plugin(&self, meta: Self::Metadata, config_file: String) -> BoxFuture<Result<String>> {
        let config = match serde_json::from_str::<PluginConfig>(&config_file)
            .map_err(|e| format!("failed to deserialize plugin config: {e}"))
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

        let _ = ctx
            .svm_locker
            .simnet_events_tx()
            .try_send(SimnetEvent::info(format!(
                "Loaded plugin with UUID - {}",
                uuid
            )));

        // Return only the endpoint URL
        Box::pin(async move { Ok(endpoint_url) })
    }

    fn list_plugins(&self, _meta: Self::Metadata) -> BoxFuture<Result<Vec<String>>> {
        not_implemented_err_async("list_plugins")
    }

    fn rpc_addr(&self, _meta: Self::Metadata) -> Result<Option<SocketAddr>> {
        not_implemented_err("rpc_addr")
    }

    fn start_time(&self, meta: Self::Metadata) -> Result<SystemTime> {
        let ctx = meta.unwrap();
        let (tx, rx) = crossbeam_channel::bounded(1);

        let _ = ctx
            .plugin_manager_commands_tx
            .send(PluginManagerCommand::GetStartTime(tx));

        let Ok(result) = rx.recv_timeout(Duration::from_secs(10)) else {
            return Err(jsonrpc_core::Error::internal_error());
        };

        let simnet_events_tx = ctx.svm_locker.simnet_events_tx();

        match result {
            Ok(start_time) => {
                let _ = simnet_events_tx
                    .try_send(SimnetEvent::info(format!("Start time: {:?}", start_time)));
                Ok(start_time)
            }
            Err(e) => {
                let _ = simnet_events_tx
                    .try_send(SimnetEvent::error(format!(
                        "Failed to get start time: {}",
                        e
                    )))
                    .ok();
                Err(jsonrpc_core::Error::internal_error())
            }
        }
    }

    fn add_authorized_voter(&self, _meta: Self::Metadata, _keypair_file: String) -> Result<()> {
        not_implemented_err("add_authorized_voter")
    }

    fn add_authorized_voter_from_bytes(
        &self,
        _meta: Self::Metadata,
        _keypair: Vec<u8>,
    ) -> Result<()> {
        not_implemented_err("add_authorized_voter_from_bytes")
    }

    fn remove_all_authorized_voters(&self, _meta: Self::Metadata) -> Result<()> {
        not_implemented_err("remove_all_authorized_voters")
    }

    fn set_identity(
        &self,
        _meta: Self::Metadata,
        _keypair_file: String,
        _require_tower: bool,
    ) -> Result<()> {
        not_implemented_err("set_identity")
    }

    fn set_identity_from_bytes(
        &self,
        _meta: Self::Metadata,
        _identity_keypair: Vec<u8>,
        _require_tower: bool,
    ) -> Result<()> {
        not_implemented_err("set_identity_from_bytes")
    }

    fn set_staked_nodes_overrides(&self, _meta: Self::Metadata, _path: String) -> Result<()> {
        not_implemented_err("set_staked_nodes_overrides")
    }

    fn repair_shred_from_peer(
        &self,
        _meta: Self::Metadata,
        _pubkey: Option<Pubkey>,
        _slot: u64,
        _shred_index: u64,
    ) -> Result<()> {
        not_implemented_err("repair_shred_from_peer")
    }

    fn set_repair_whitelist(&self, _meta: Self::Metadata, _whitelist: Vec<Pubkey>) -> Result<()> {
        not_implemented_err("set_repair_whitelist")
    }

    fn get_secondary_index_key_size(
        &self,
        _meta: Self::Metadata,
        _pubkey_str: String,
    ) -> Result<HashMap<RpcAccountIndex, usize>> {
        not_implemented_err("get_secondary_index_key_size")
    }

    fn set_public_tpu_address(
        &self,
        _meta: Self::Metadata,
        _public_tpu_addr: SocketAddr,
    ) -> Result<()> {
        not_implemented_err("set_public_tpu_address")
    }

    fn set_public_tpu_forwards_address(
        &self,
        _meta: Self::Metadata,
        _public_tpu_forwards_addr: SocketAddr,
    ) -> Result<()> {
        not_implemented_err("set_public_tpu_forwards_address")
    }
}
