use std::net::TcpListener;

use solana_sdk::rpc_port::DEFAULT_RPC_PORT;

pub fn is_port_available(port: u16) -> bool {
    TcpListener::bind(("127.0.0.1", port)).is_ok()
}

pub fn find_next_available_surfnet_port() -> Result<(u16, u16), String> {
    let mut surfnet_id: u16 = 0;
    while surfnet_id.checked_add(1).is_some() {
        let port = DEFAULT_RPC_PORT + (surfnet_id * 10000);
        let ws_port = port.saturating_sub(9);
        if ws_port > 0 && is_port_available(port) && is_port_available(ws_port) {
            return Ok((surfnet_id, port));
        }
        surfnet_id += 1;
    }
    Err("No available surfnet ports of format 127.0.0.1:x8899 found.".to_string())
}
