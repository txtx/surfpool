#[derive(Clone, Debug)]
pub struct SurfpoolConfig {
    pub simnet: SimnetConfig,
    pub rpc: RpcConfig,
}

#[derive(Clone, Debug)]
pub struct SimnetConfig {
    pub remote_rpc_url: String,
    pub slot_time: u64,
}

#[derive(Clone, Debug)]
pub struct RpcConfig {
    pub bind_address: String,
    pub bind_port: u16,
    pub remote_rpc_url: String,
}
