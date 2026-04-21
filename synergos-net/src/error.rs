use thiserror::Error;

use crate::types::Blake3Hash;

#[derive(Debug, Error)]
pub enum SynergosNetError {
    #[error("QUIC connection error: {0}")]
    Quic(String),

    #[error("Tunnel error: {0}")]
    Tunnel(String),

    #[error("DNS resolution failed: {0}")]
    DnsResolution(String),

    #[error("TURN/STUN error: {0}")]
    Turn(String),

    #[error("Chain fork detected: expected {expected:?}, got {got:?}")]
    ChainFork {
        expected: Option<Blake3Hash>,
        got: Option<Blake3Hash>,
    },

    #[error("Peer not found: {0}")]
    PeerNotFound(String),

    #[error("Transfer error: {0}")]
    Transfer(String),

    #[error("Config error: {0}")]
    Config(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Gossip protocol error: {0}")]
    Gossip(String),

    #[error("Identity / signature error: {0}")]
    Identity(String),
}

pub type Result<T> = std::result::Result<T, SynergosNetError>;
