//! Custom NetworkBehaviour composing Kademlia + GossipSub + mDNS + Identify + Ping + AutoNAT
//! + Circuit Relay client + DCUtR hole-punching.
//!
//! This module defines the composite behaviour for the WWS network node.
//! Each sub-behaviour handles a specific aspect of peer-to-peer communication:
//! - **Kademlia**: Distributed hash table for peer routing and record storage
//! - **GossipSub**: Pub/sub messaging for protocol messages
//! - **mDNS**: Local network peer discovery
//! - **Identify**: Peer identification and capability exchange
//! - **Ping**: Connection liveness checking
//! - **AutoNAT**: NAT traversal status detection
//! - **Relay client**: Circuit relay for NAT traversal via public relay nodes
//! - **DCUtR**: Direct Connection Upgrade through Relay (hole-punching)

use std::time::Duration;

use libp2p::{
    autonat, dcutr, gossipsub, identify, kad, mdns, ping, relay,
    identity::Keypair,
    swarm::NetworkBehaviour,
    StreamProtocol,
};

use crate::NetworkError;

/// Composite NetworkBehaviour for a WWS node.
///
/// The libp2p derive macro auto-generates a `SwarmBehaviourEvent` enum
/// with one variant per field, used for routing events in the swarm host.
#[derive(NetworkBehaviour)]
pub struct SwarmBehaviour {
    /// Kademlia DHT for distributed routing and storage.
    pub kademlia: kad::Behaviour<kad::store::MemoryStore>,
    /// GossipSub pub/sub for broadcasting protocol messages.
    pub gossipsub: gossipsub::Behaviour,
    /// mDNS for automatic local peer discovery.
    pub mdns: mdns::tokio::Behaviour,
    /// Identify protocol for exchanging peer metadata.
    pub identify: identify::Behaviour,
    /// Ping for measuring round-trip times and liveness.
    pub ping: ping::Behaviour,
    /// AutoNAT for detecting NAT status and public reachability.
    pub autonat: autonat::Behaviour,
    /// Circuit relay client for NAT traversal via public relay nodes.
    pub relay_client: relay::client::Behaviour,
    /// DCUtR: Direct Connection Upgrade through Relay for hole-punching.
    pub dcutr: dcutr::Behaviour,
}

/// Configuration for constructing the composite behaviour.
#[derive(Debug, Clone)]
pub struct BehaviourConfig {
    /// Protocol version string for identify.
    pub protocol_version: String,
    /// Kademlia protocol name.
    pub kad_protocol: String,
    /// GossipSub heartbeat interval.
    pub gossipsub_heartbeat: Duration,
    /// Whether to use strict GossipSub validation.
    pub gossipsub_strict: bool,
    /// mDNS query interval.
    pub mdns_query_interval: Duration,
    /// Ping interval.
    pub ping_interval: Duration,
}

impl Default for BehaviourConfig {
    fn default() -> Self {
        Self {
            protocol_version: wws_protocol::PROTOCOL_VERSION.to_string(),
            kad_protocol: "/wws/kad/1.0.0".to_string(),
            gossipsub_heartbeat: Duration::from_secs(1),
            gossipsub_strict: false,
            mdns_query_interval: Duration::from_secs(5),
            ping_interval: Duration::from_secs(15),
        }
    }
}

impl SwarmBehaviour {
    /// Construct a new composite behaviour from a keypair, configuration, and relay client.
    ///
    /// The `relay_client` is extracted from the SwarmBuilder (via `.with_relay_client()`)
    /// and must be passed in — it cannot be constructed independently.
    pub fn new(
        key: &Keypair,
        config: &BehaviourConfig,
        relay_client: relay::client::Behaviour,
    ) -> Result<Self, NetworkError> {
        let peer_id = key.public().to_peer_id();

        // -- Kademlia --
        let store = kad::store::MemoryStore::new(peer_id);
        let kad_protocol = StreamProtocol::try_from_owned(config.kad_protocol.clone())
            .map_err(|e| NetworkError::Behaviour(format!("Invalid Kademlia protocol: {e}")))?;
        let mut kad_config = kad::Config::new(kad_protocol);
        kad_config.set_query_timeout(Duration::from_secs(60));
        let kademlia = kad::Behaviour::with_config(peer_id, store, kad_config);

        // -- GossipSub --
        let gossipsub_config = gossipsub::ConfigBuilder::default()
            .heartbeat_interval(config.gossipsub_heartbeat)
            .validation_mode(if config.gossipsub_strict {
                gossipsub::ValidationMode::Strict
            } else {
                gossipsub::ValidationMode::Permissive
            })
            .flood_publish(true)
            .mesh_outbound_min(0)
            .build()
            .map_err(|e| NetworkError::Behaviour(format!("GossipSub config error: {e}")))?;

        let gossipsub = gossipsub::Behaviour::new(
            gossipsub::MessageAuthenticity::Signed(key.clone()),
            gossipsub_config,
        )
        .map_err(|e| NetworkError::Behaviour(format!("GossipSub init error: {e}")))?;

        // -- mDNS --
        let mdns_config = mdns::Config {
            query_interval: config.mdns_query_interval,
            ..Default::default()
        };
        let mdns = mdns::tokio::Behaviour::new(mdns_config, peer_id)
            .map_err(|e| NetworkError::Behaviour(format!("mDNS init error: {e}")))?;

        // -- Identify --
        let identify_config =
            identify::Config::new(config.protocol_version.clone(), key.public())
                .with_push_listen_addr_updates(true);
        let identify = identify::Behaviour::new(identify_config);

        // -- Ping --
        let ping = ping::Behaviour::new(
            ping::Config::new().with_interval(config.ping_interval),
        );

        // -- AutoNAT --
        let autonat = autonat::Behaviour::new(peer_id, autonat::Config::default());

        // -- DCUtR (hole-punching) --
        // Requires the relay_client to already be in scope; dcutr only needs peer_id.
        let dcutr = dcutr::Behaviour::new(peer_id);

        Ok(Self {
            kademlia,
            gossipsub,
            mdns,
            identify,
            ping,
            autonat,
            relay_client,
            dcutr,
        })
    }
}
