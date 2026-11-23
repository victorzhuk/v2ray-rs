use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("packet: {0}")]
    Packet(String),
    #[error("strategy: {0}")]
    Strategy(String),
    #[error("firewall: {0}")]
    Firewall(String),
    #[error("timed out: {0}")]
    TimedOut(String),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("rawsend: {0}")]
    RawSend(String),
    #[error("queue: {0}")]
    Queue(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
