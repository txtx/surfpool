use std::fmt;

pub type SurfnetResult<T> = Result<T, SurfnetError>;

#[derive(Debug)]
pub enum SurfnetError {
    /// Failed to bind to a free port
    PortAllocation(String),
    /// The surfnet runtime failed to start
    Startup(String),
    /// The surfnet runtime thread panicked or exited unexpectedly
    Runtime(String),
    /// An RPC cheatcode call failed
    Cheatcode(String),
    /// The surfnet was shut down or aborted during startup
    Aborted(String),
}

impl fmt::Display for SurfnetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SurfnetError::PortAllocation(msg) => write!(f, "port allocation failed: {msg}"),
            SurfnetError::Startup(msg) => write!(f, "surfnet startup failed: {msg}"),
            SurfnetError::Runtime(msg) => write!(f, "surfnet runtime error: {msg}"),
            SurfnetError::Cheatcode(msg) => write!(f, "cheatcode failed: {msg}"),
            SurfnetError::Aborted(msg) => write!(f, "surfnet aborted: {msg}"),
        }
    }
}

impl std::error::Error for SurfnetError {}
