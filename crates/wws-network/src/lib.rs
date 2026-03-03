//! WWS Network - P2P networking layer using libp2p
//!
//! Provides the foundational networking for the World Wide Swarm (WWS) Protocol:
//! - Peer discovery via mDNS and Kademlia DHT
//! - Message passing via GossipSub pub/sub
//! - Swarm size estimation from Kademlia routing table density
//! - Transport configuration with TCP + Noise + Yamux

pub mod behaviour;
pub mod discovery;
pub mod dns_bootstrap;
pub mod name_registry;
pub mod size_estimator;
pub mod swarm_host;
pub mod topics;
pub mod transport;

pub use behaviour::SwarmBehaviour;
pub use discovery::DiscoveryConfig;
pub use libp2p::{Multiaddr, PeerId};
pub use size_estimator::SwarmSizeEstimator;
pub use swarm_host::{NetworkEvent, SwarmHandle, SwarmHost, SwarmHostConfig};
pub use topics::TopicManager;
pub use transport::build_swarm;

use thiserror::Error;

/// Errors originating from the network layer.
#[derive(Error, Debug)]
pub enum NetworkError {
    #[error("Transport error: {0}")]
    Transport(String),

    #[error("Behaviour construction error: {0}")]
    Behaviour(String),

    #[error("Failed to publish message: {0}")]
    PublishError(String),

    #[error("Failed to subscribe to topic: {0}")]
    SubscriptionError(String),

    #[error("Dial error: {0}")]
    DialError(String),

    #[error("Listen error: {0}")]
    ListenError(String),

    #[error("DHT operation failed: {0}")]
    DhtError(String),

    #[error("Internal channel closed")]
    ChannelClosed,

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Protocol error: {0}")]
    Protocol(#[from] wws_protocol::ProtocolError),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}
