#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunloopTriggerMode {
    Clock,
    Manual,
    Transaction,
}

#[derive(Clone, Debug)]
pub struct SurfpoolConfig {
    pub simnet: SimnetConfig,
    pub rpc: RpcConfig,
}

#[derive(Clone, Debug)]
pub struct SimnetConfig {
    pub remote_rpc_url: String,
    pub slot_time: u64,
    pub runloop_trigger_mode: RunloopTriggerMode,
}

#[derive(Clone, Debug)]
pub struct RpcConfig {
    pub bind_address: String,
    pub bind_port: u16,
    pub remote_rpc_url: String,
}
