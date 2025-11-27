use std::time::Duration;

use jsonrpc_core::{BoxFuture, Result};
use jsonrpc_derive::rpc;
use solana_client::rpc_custom_error::RpcCustomError;
use surfpool_types::{SimnetCommand, SimnetEvent};
use txtx_addon_network_svm_types::subgraph::PluginConfig;
use uuid::Uuid;

use super::RunloopContext;
use crate::{PluginInfo, PluginManagerCommand, rpc::State};

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
    /// - `Vec<PluginInfo>` — A list of plugin information objects, each containing:
    ///   - `plugin_name`: The name of the plugin (e.g., "surfpool-subgraph")
    ///   - `uuid`: The unique identifier of the plugin instance
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
    ///   "result": [
    ///     {
    ///       "plugin_name": "surfpool-subgraph",
    ///       "uuid": "550e8400-e29b-41d4-a716-446655440000"
    ///     }
    ///   ],
    ///   "id": 103
    /// }
    /// ```
    ///
    /// # Notes
    /// - Only plugins that have been successfully loaded will appear in this list.
    /// - This method is read-only and safe to call frequently.
    #[rpc(meta, name = "listPlugins")]
    fn list_plugins(&self, meta: Self::Metadata) -> BoxFuture<Result<Vec<PluginInfo>>>;

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
    /// - The result is a `String` in UTC, reflecting the moment the system was initialized.
    /// - This method is useful for monitoring system uptime and verifying system health.
    #[rpc(meta, name = "startTime")]
    fn start_time(&self, meta: Self::Metadata) -> Result<String>;
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

    fn list_plugins(&self, meta: Self::Metadata) -> BoxFuture<Result<Vec<PluginInfo>>> {
        let Some(ctx) = meta else {
            return Box::pin(async move { Err(jsonrpc_core::Error::internal_error()) });
        };

        let (tx, rx) = crossbeam_channel::bounded(1);
        let _ = ctx
            .plugin_manager_commands_tx
            .send(PluginManagerCommand::ListPlugins(tx));

        let Ok(plugin_list) = rx.recv_timeout(Duration::from_secs(10)) else {
            return Box::pin(async move { Err(jsonrpc_core::Error::internal_error()) });
        };

        Box::pin(async move { Ok(plugin_list) })
    }

    fn start_time(&self, meta: Self::Metadata) -> Result<String> {
        let svm_locker = meta.get_svm_locker()?;
        let system_time = svm_locker.get_start_time();

        let datetime_utc: chrono::DateTime<chrono::Utc> = system_time.into();
        Ok(datetime_utc.to_rfc3339())
    }
}
