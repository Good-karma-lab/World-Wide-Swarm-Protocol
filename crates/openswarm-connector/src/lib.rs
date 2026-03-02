//! WWS.Connector - JSON-RPC API server and sidecar for AI agents
//!
//! The connector is the interface between an AI agent (e.g., OpenClaw) and
//! the OpenSwarm network. It initializes and orchestrates all subsystems:
//! - Network layer (libp2p swarm)
//! - Hierarchy management (pyramid, elections, geo-clustering)
//! - Consensus (RFP, voting, cascade)
//! - State management (CRDT, Merkle-DAG, content store)
//!
//! The connector exposes a JSON-RPC 2.0 API over TCP for the local agent
//! to interact with the swarm.

pub mod agent_bridge;
pub mod config;
pub mod connector;
pub mod file_server;
pub mod identity_store;
pub mod operator_console;
pub mod reputation;
pub mod rpc_server;
pub mod tui;

pub use config::ConnectorConfig;
pub use connector::OpenSwarmConnector;
pub use file_server::FileServer;
pub use rpc_server::RpcServer;
