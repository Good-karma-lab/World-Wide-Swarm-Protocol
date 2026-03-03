//! WWS State - CRDT state management and Merkle-DAG verification
//!
//! Provides the distributed state management infrastructure:
//! - OR-Set CRDT for conflict-free hot state (task statuses, agent lists)
//! - Merkle-DAG for bottom-up result verification and hash chaining
//! - Content-addressed storage with CID generation and DHT publishing
//! - Adaptive Granularity Algorithm for optimal task decomposition depth

pub mod content_store;
pub mod crdt;
pub mod granularity;
pub mod merkle_dag;
pub mod pn_counter;
pub mod reputation;

pub use content_store::ContentStore;
pub use crdt::OrSet;
pub use crdt::PnCounter;
pub use granularity::{GranularityAlgorithm, GranularityEngine};
pub use merkle_dag::MerkleDag;

use thiserror::Error;

/// Errors originating from the state layer.
#[derive(Error, Debug)]
pub enum StateError {
    #[error("CRDT merge conflict: {0}")]
    MergeConflict(String),

    #[error("Content not found: CID {0}")]
    ContentNotFound(String),

    #[error("Invalid CID: {0}")]
    InvalidCid(String),

    #[error("Merkle verification failed: {0}")]
    MerkleVerificationFailed(String),

    #[error("Storage error: {0}")]
    StorageError(String),

    #[error("Serialization error: {0}")]
    Serialization(String),

    #[error("Content too large: {size} bytes exceeds limit of {limit} bytes")]
    ContentTooLarge { size: usize, limit: usize },
}
