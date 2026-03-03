/// Default branching factor (k) for the pyramidal hierarchy.
/// Each node oversees exactly k subordinate nodes.
pub const DEFAULT_BRANCHING_FACTOR: u32 = 10;

/// Default epoch duration in seconds (1 hour).
pub const DEFAULT_EPOCH_DURATION_SECS: u64 = 3600;

/// Keep-alive interval in seconds.
pub const KEEPALIVE_INTERVAL_SECS: u64 = 10;

/// Leader failover timeout in seconds.
/// If a leader is silent for this duration, succession election triggers.
pub const LEADER_TIMEOUT_SECS: u64 = 30;

/// Commit-Reveal timeout: how long to wait for all proposal hashes.
pub const COMMIT_REVEAL_TIMEOUT_SECS: u64 = 60;

/// Voting phase timeout in seconds.
pub const VOTING_TIMEOUT_SECS: u64 = 120;

/// Maximum hierarchy depth to prevent infinite recursion.
pub const MAX_HIERARCHY_DEPTH: u32 = 10;

/// GossipSub topic prefix.
pub const TOPIC_PREFIX: &str = "/wws/1.0.0";

/// JSON-RPC protocol version.
pub const JSONRPC_VERSION: &str = "2.0";

/// Protocol version string.
pub const PROTOCOL_VERSION: &str = "/wws/1.0.0";

/// Proof of Work difficulty (number of leading zero bits required).
pub const POW_DIFFICULTY: u32 = 24;

/// Default public swarm ID. All nodes join this swarm by default.
pub const DEFAULT_SWARM_ID: &str = "public";

/// Default swarm display name.
pub const DEFAULT_SWARM_NAME: &str = "WWS Public";

/// DHT key prefix for swarm registry records.
pub const SWARM_REGISTRY_PREFIX: &str = "/wws/registry/";

/// DHT key prefix for swarm membership records.
pub const SWARM_MEMBERSHIP_PREFIX: &str = "/wws/membership/";

/// Swarm announcement interval in seconds.
pub const SWARM_ANNOUNCE_INTERVAL_SECS: u64 = 30;

/// Default well-known bootstrap peers.
/// These are entry points only — not required after joining the mesh.
pub const DEFAULT_BOOTSTRAP_PEERS: &[&str] = &[
    "/dns4/bootstrap1.wws.dev/tcp/9000/p2p/12D3KooWPLACEHOLDER1",
    "/dns4/bootstrap2.wws.dev/tcp/9000/p2p/12D3KooWPLACEHOLDER2",
    "/dns4/bootstrap3.wws.dev/tcp/9000/p2p/12D3KooWPLACEHOLDER3",
];

/// Bootstrap mode: maximum connected peers
pub const BOOTSTRAP_MAX_PEERS: u32 = 10_000;
/// Bootstrap mode: Kademlia replication factor
pub const BOOTSTRAP_REPLICATION_FACTOR: usize = 20;
