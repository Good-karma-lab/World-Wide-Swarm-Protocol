//! Configuration loading from TOML and environment variables.
//!
//! The connector reads its configuration from:
//! 1. A TOML config file (default: config/wws.toml)
//! 2. Environment variables (override TOML values)
//!
//! Environment variable prefix: WWS_

use std::net::SocketAddr;
use std::path::Path;

use serde::{Deserialize, Serialize};

/// Top-level connector configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectorConfig {
    /// Network configuration.
    #[serde(default)]
    pub network: NetworkConfig,
    /// Hierarchy configuration.
    #[serde(default)]
    pub hierarchy: HierarchyConfig,
    /// RPC server configuration.
    #[serde(default)]
    pub rpc: RpcConfig,
    /// Agent configuration.
    #[serde(default)]
    pub agent: AgentConfig,
    /// Logging configuration.
    #[serde(default)]
    pub logging: LoggingConfig,
    /// Swarm identity and multi-swarm configuration.
    #[serde(default)]
    pub swarm: SwarmConfig,
    /// HTTP file server configuration for agent onboarding.
    #[serde(default)]
    pub file_server: FileServerConfig,
    /// Agent identity configuration.
    #[serde(default)]
    pub identity: IdentityConfig,
}

/// Agent identity configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdentityConfig {
    /// Path to the Ed25519 identity key file (32-byte seed, mode 0600).
    #[serde(default = "default_identity_path")]
    pub path: std::path::PathBuf,
    /// Optional wws:// name to register on startup.
    #[serde(default)]
    pub wws_name: Option<String>,
}

fn default_identity_path() -> std::path::PathBuf {
    default_identity_dir().join("identity.key")
}

impl Default for IdentityConfig {
    fn default() -> Self {
        Self {
            path: default_identity_path(),
            wws_name: None,
        }
    }
}

/// HTTP file server configuration for serving agent onboarding docs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileServerConfig {
    /// Whether the file server is enabled.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Address to bind the HTTP file server to.
    #[serde(default = "default_file_server_addr")]
    pub bind_addr: String,
}

/// Network layer configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkConfig {
    /// Address to listen on for P2P connections.
    #[serde(default = "default_listen_addr")]
    pub listen_addr: String,
    /// Bootstrap peer multiaddresses.
    #[serde(default)]
    pub bootstrap_peers: Vec<String>,
    /// Whether mDNS local discovery is enabled. Enabled by default for zero-conf LAN discovery.
    #[serde(default = "default_true")]
    pub mdns_enabled: bool,
    /// Idle connection timeout in seconds.
    #[serde(default = "default_idle_timeout")]
    pub idle_connection_timeout_secs: u64,
    /// Run as a public bootstrap node (no agent bridge, high capacity).
    #[serde(default)]
    pub bootstrap_mode: bool,
    /// Enable QUIC transport alongside TCP.
    #[serde(default = "default_true")]
    pub enable_quic: bool,
    /// Enable circuit relay client (for NAT traversal).
    #[serde(default = "default_true")]
    pub enable_relay_client: bool,
    /// Enable relay server (bootstrap nodes set this true automatically).
    #[serde(default)]
    pub enable_relay_server: bool,
    /// Enable hole-punching upgrade (DCUtR).
    #[serde(default = "default_true")]
    pub enable_dcutr: bool,
}

/// Hierarchy and epoch configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HierarchyConfig {
    /// Branching factor (k).
    #[serde(default = "default_branching_factor")]
    pub branching_factor: u32,
    /// Epoch duration in seconds.
    #[serde(default = "default_epoch_duration")]
    pub epoch_duration_secs: u64,
    /// Leader timeout in seconds.
    #[serde(default = "default_leader_timeout")]
    pub leader_timeout_secs: u64,
    /// Keep-alive interval in seconds.
    #[serde(default = "default_keepalive_interval")]
    pub keepalive_interval_secs: u64,
}

/// JSON-RPC server configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcConfig {
    /// Address to bind the RPC server to.
    #[serde(default = "default_rpc_addr")]
    pub bind_addr: String,
    /// Maximum concurrent RPC connections.
    #[serde(default = "default_max_connections")]
    pub max_connections: usize,
    /// Request timeout in seconds.
    #[serde(default = "default_rpc_timeout")]
    pub request_timeout_secs: u64,
}

/// Agent bridge configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    /// Agent name/identifier.
    #[serde(default = "default_agent_name")]
    pub name: String,
    /// Agent capabilities.
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// Whether MCP compatibility mode is enabled.
    #[serde(default)]
    pub mcp_compatible: bool,
}

/// Logging configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfig {
    /// Log level filter (e.g., "info", "debug", "wws=debug,libp2p=info").
    #[serde(default = "default_log_level")]
    pub level: String,
    /// Whether to output JSON-formatted logs.
    #[serde(default)]
    pub json_format: bool,
}

/// Swarm identity and multi-swarm configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmConfig {
    /// Swarm ID to join on startup. Defaults to "public" (the open swarm).
    #[serde(default = "default_swarm_id")]
    pub swarm_id: String,
    /// Authentication token for private swarms (not required for public swarm).
    #[serde(default)]
    pub token: Option<String>,
    /// Human-readable name for the swarm (used when creating a new swarm).
    #[serde(default = "default_swarm_name")]
    pub name: String,
    /// Whether this node's swarm is public (joinable without token).
    #[serde(default = "default_true")]
    pub is_public: bool,
    /// Interval in seconds between swarm announcements on the DHT.
    #[serde(default = "default_swarm_announce_interval")]
    pub announce_interval_secs: u64,
}

// -- Defaults --

fn default_listen_addr() -> String {
    "/ip4/0.0.0.0/tcp/0".to_string()
}
fn default_true() -> bool {
    true
}
fn default_idle_timeout() -> u64 {
    60
}
fn default_branching_factor() -> u32 {
    wws_protocol::DEFAULT_BRANCHING_FACTOR
}
fn default_epoch_duration() -> u64 {
    wws_protocol::DEFAULT_EPOCH_DURATION_SECS
}
fn default_leader_timeout() -> u64 {
    wws_protocol::LEADER_TIMEOUT_SECS
}
fn default_keepalive_interval() -> u64 {
    wws_protocol::KEEPALIVE_INTERVAL_SECS
}
fn default_rpc_addr() -> String {
    "127.0.0.1:9370".to_string()
}
fn default_max_connections() -> usize {
    10
}
fn default_rpc_timeout() -> u64 {
    30
}
fn default_agent_name() -> String {
    "wws-agent".to_string()
}
pub fn default_identity_dir() -> std::path::PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".wws")
}
fn default_log_level() -> String {
    "info".to_string()
}
fn default_swarm_id() -> String {
    wws_protocol::DEFAULT_SWARM_ID.to_string()
}
fn default_swarm_name() -> String {
    wws_protocol::DEFAULT_SWARM_NAME.to_string()
}
fn default_swarm_announce_interval() -> u64 {
    wws_protocol::SWARM_ANNOUNCE_INTERVAL_SECS
}
fn default_file_server_addr() -> String {
    "127.0.0.1:9371".to_string()
}

// -- Trait impls --

impl Default for SwarmConfig {
    fn default() -> Self {
        Self {
            swarm_id: default_swarm_id(),
            token: None,
            name: default_swarm_name(),
            is_public: true,
            announce_interval_secs: default_swarm_announce_interval(),
        }
    }
}

impl Default for FileServerConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            bind_addr: default_file_server_addr(),
        }
    }
}

impl Default for ConnectorConfig {
    fn default() -> Self {
        Self {
            network: NetworkConfig::default(),
            hierarchy: HierarchyConfig::default(),
            rpc: RpcConfig::default(),
            agent: AgentConfig::default(),
            logging: LoggingConfig::default(),
            swarm: SwarmConfig::default(),
            file_server: FileServerConfig::default(),
            identity: IdentityConfig::default(),
        }
    }
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            listen_addr: default_listen_addr(),
            bootstrap_peers: Vec::new(),
            mdns_enabled: true,
            idle_connection_timeout_secs: default_idle_timeout(),
            bootstrap_mode: false,
            enable_quic: true,
            enable_relay_client: true,
            enable_relay_server: false,
            enable_dcutr: true,
        }
    }
}

impl Default for HierarchyConfig {
    fn default() -> Self {
        Self {
            branching_factor: default_branching_factor(),
            epoch_duration_secs: default_epoch_duration(),
            leader_timeout_secs: default_leader_timeout(),
            keepalive_interval_secs: default_keepalive_interval(),
        }
    }
}

impl Default for RpcConfig {
    fn default() -> Self {
        Self {
            bind_addr: default_rpc_addr(),
            max_connections: default_max_connections(),
            request_timeout_secs: default_rpc_timeout(),
        }
    }
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            name: default_agent_name(),
            capabilities: Vec::new(),
            mcp_compatible: false,
        }
    }
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: default_log_level(),
            json_format: false,
        }
    }
}

impl ConnectorConfig {
    /// Load configuration from a TOML file.
    pub fn from_file(path: &Path) -> Result<Self, anyhow::Error> {
        let content = std::fs::read_to_string(path)?;
        let config: ConnectorConfig = toml::from_str(&content)?;
        Ok(config)
    }

    /// Load configuration from a TOML file, with environment variable overrides.
    ///
    /// Environment variables use the prefix `WWS_` and path separators `__`.
    /// For example: `WWS_RPC__BIND_ADDR=127.0.0.1:9999`
    pub fn load(path: Option<&Path>) -> Result<Self, anyhow::Error> {
        let mut config = if let Some(path) = path {
            if path.exists() {
                Self::from_file(path)?
            } else {
                tracing::warn!(
                    path = %path.display(),
                    "Config file not found, using defaults"
                );
                Self::default()
            }
        } else {
            Self::default()
        };

        // Apply environment variable overrides.
        config.apply_env_overrides();

        Ok(config)
    }

    /// Apply environment variable overrides to the configuration.
    fn apply_env_overrides(&mut self) {
        if let Ok(val) = std::env::var("WWS_LISTEN_ADDR") {
            self.network.listen_addr = val;
        }
        if let Ok(val) = std::env::var("WWS_RPC_BIND_ADDR") {
            self.rpc.bind_addr = val;
        }
        if let Ok(val) = std::env::var("WWS_LOG_LEVEL") {
            self.logging.level = val;
        }
        if let Ok(val) = std::env::var("WWS_BRANCHING_FACTOR") {
            if let Ok(k) = val.parse() {
                self.hierarchy.branching_factor = k;
            }
        }
        if let Ok(val) = std::env::var("WWS_EPOCH_DURATION") {
            if let Ok(d) = val.parse() {
                self.hierarchy.epoch_duration_secs = d;
            }
        }
        if let Ok(val) = std::env::var("WWS_AGENT_NAME") {
            self.agent.name = val;
        }
        if let Ok(val) = std::env::var("WWS_BOOTSTRAP_PEERS") {
            self.network.bootstrap_peers = val.split(',').map(|s| s.trim().to_string()).collect();
        }
        if let Ok(val) = std::env::var("WWS_SWARM_ID") {
            self.swarm.swarm_id = val;
        }
        if let Ok(val) = std::env::var("WWS_SWARM_TOKEN") {
            self.swarm.token = Some(val);
        }
        if let Ok(val) = std::env::var("WWS_SWARM_NAME") {
            self.swarm.name = val;
        }
        if let Ok(val) = std::env::var("WWS_SWARM_PUBLIC") {
            self.swarm.is_public = val == "true" || val == "1";
        }
        if let Ok(val) = std::env::var("WWS_FILE_SERVER_ADDR") {
            self.file_server.bind_addr = val;
        }
        if let Ok(val) = std::env::var("WWS_FILE_SERVER_ENABLED") {
            self.file_server.enabled = val == "true" || val == "1";
        }
        if let Ok(val) = std::env::var("WWS_IDENTITY_PATH") {
            self.identity.path = std::path::PathBuf::from(val);
        }
        if let Ok(val) = std::env::var("WWS_NAME") {
            self.identity.wws_name = Some(val);
        }
    }

    /// Parse the RPC bind address into a SocketAddr.
    pub fn rpc_socket_addr(&self) -> Result<SocketAddr, anyhow::Error> {
        Ok(self.rpc.bind_addr.parse()?)
    }
}

#[cfg(test)]
mod config_tests {
    use super::*;

    #[test]
    fn network_config_default_enables_mdns() {
        let config = NetworkConfig::default();
        assert!(config.mdns_enabled, "mDNS should be enabled by default for zero-conf discovery");
    }
}
