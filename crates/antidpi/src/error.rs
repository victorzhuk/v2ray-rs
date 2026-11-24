use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("packet: {0}")]
    Packet(String),
    #[error("strategy: {0}")]
    Strategy(String),
    #[error("firewall: {0}")]
    Firewall(String),
    /// A live foreign owner holds the journal and its rules, verified before
    /// this request mutated anything. Only `install-rules` maps this to its
    /// dedicated exit status; every other command reports it as a normal
    /// failure.
    #[error("foreign owner: {0}")]
    ForeignLiveOwner(String),
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
