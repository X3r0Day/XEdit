use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("not attached to a process")]
    NotAttached,

    #[error("no process with pid {0}")]
    NoSuchProcess(u32),

    #[error("address {0:#x} is not mapped")]
    Unmapped(u64),

    #[error("permission denied: {0}")]
    Denied(String),

    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}
