use std::net::TcpListener;

pub fn get_free_port() -> Result<u16, String> {
    let listener =
        TcpListener::bind("127.0.0.1:0").map_err(|e| format!("Failed to bind to port 0: {}", e))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("failed to parse address: {}", e))?
        .port();
    drop(listener);
    Ok(port)
}
