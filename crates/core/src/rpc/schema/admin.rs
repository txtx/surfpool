use schemars::JsonSchema;

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase", rename = "endpoints")]
pub enum Admin {
    #[schemars(description = "Immediately shuts down the RPC server.")]
    Exit,
    #[schemars(description = "Reloads a runtime plugin with new configuration.")]
    ReloadPlugin(ReloadPlugin),
    #[schemars(description = "Unloads a runtime plugin.")]
    UnloadPlugin(UnloadPlugin),
    #[schemars(
        description = "Dynamically loads a new plugin into the runtime from a configuration file."
    )]
    LoadPlugin(LoadPlugin),
    #[schemars(description = "Returns a list of all currently loaded plugin names.")]
    ListPlugins,
    #[schemars(description = "Returns the address of the RPC server.")]
    RpcAddress,
    #[schemars(description = "Sets a filter for log messages in the system.")]
    SetLogFilter(SetLogFilter),
    #[schemars(description = "Returns the system start time.")]
    StartTime,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReloadPlugin {
    pub name: String,
    pub config_file: String,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UnloadPlugin {
    pub name: String,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct LoadPlugin {
    pub config_file: String,
}

#[derive(JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SetLogFilter {
    pub filter: String,
}
