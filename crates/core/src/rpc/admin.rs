use jsonrpc_core::{BoxFuture, Error, ErrorCode, Result};
use jsonrpc_derive::rpc;
use solana_client::rpc_custom_error::RpcCustomError;
use surfpool_types::SimnetCommand;

use super::RunloopContext;
use crate::{PluginInfo, rpc::State, surfnet::PluginCommand};

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
        let Some(_ctx) = meta else {
            return Err(RpcCustomError::NodeUnhealthy {
                num_slots_behind: None,
            }
            .into());
        };

        unsafe {
            libc::raise(libc::SIGTERM);
        }

        Ok(())
    }

    fn reload_plugin(
        &self,
        meta: Self::Metadata,
        name: String,
        config_file: String,
    ) -> BoxFuture<Result<()>> {
        Box::pin(async move {
            let Some(ctx) = meta else {
                return Err(RpcCustomError::NodeUnhealthy {
                    num_slots_behind: None,
                }
                .into());
            };
            let (tx, rx) = crossbeam_channel::bounded(1);
            ctx.plugin_commands_tx
                .send(PluginCommand::Reload {
                    name,
                    config_file,
                    response_tx: tx,
                })
                .map_err(|_| Error::internal_error())?;
            rx.recv()
                .map_err(|_| Error::internal_error())?
                .map_err(|e| Error {
                    code: ErrorCode::InternalError,
                    message: e,
                    data: None,
                })
        })
    }

    fn unload_plugin(&self, meta: Self::Metadata, name: String) -> BoxFuture<Result<()>> {
        Box::pin(async move {
            let Some(ctx) = meta else {
                return Err(RpcCustomError::NodeUnhealthy {
                    num_slots_behind: None,
                }
                .into());
            };
            let (tx, rx) = crossbeam_channel::bounded(1);
            ctx.plugin_commands_tx
                .send(PluginCommand::Unload {
                    name,
                    response_tx: tx,
                })
                .map_err(|_| Error::internal_error())?;
            rx.recv()
                .map_err(|_| Error::internal_error())?
                .map_err(|e| Error {
                    code: ErrorCode::InternalError,
                    message: e,
                    data: None,
                })
        })
    }

    fn load_plugin(&self, meta: Self::Metadata, config_file: String) -> BoxFuture<Result<String>> {
        Box::pin(async move {
            let Some(ctx) = meta else {
                return Err(RpcCustomError::NodeUnhealthy {
                    num_slots_behind: None,
                }
                .into());
            };
            let (tx, rx) = crossbeam_channel::bounded(1);
            ctx.plugin_commands_tx
                .send(PluginCommand::Load {
                    config_file,
                    response_tx: tx,
                })
                .map_err(|_| Error::internal_error())?;
            rx.recv()
                .map_err(|_| Error::internal_error())?
                .map(|info| info.plugin_name)
                .map_err(|e| Error {
                    code: ErrorCode::InternalError,
                    message: e,
                    data: None,
                })
        })
    }

    fn list_plugins(&self, meta: Self::Metadata) -> BoxFuture<Result<Vec<PluginInfo>>> {
        Box::pin(async move {
            let Some(ctx) = meta else {
                return Err(RpcCustomError::NodeUnhealthy {
                    num_slots_behind: None,
                }
                .into());
            };
            let (tx, rx) = crossbeam_channel::bounded(1);
            ctx.plugin_commands_tx
                .send(PluginCommand::List { response_tx: tx })
                .map_err(|_| Error::internal_error())?;
            Ok(rx.recv().map_err(|_| Error::internal_error())?)
        })
    }

    fn start_time(&self, meta: Self::Metadata) -> Result<String> {
        let svm_locker = meta.get_svm_locker()?;
        let system_time = svm_locker.get_start_time();

        let datetime_utc: chrono::DateTime<chrono::Utc> = system_time.into();
        Ok(datetime_utc.to_rfc3339())
    }
}
