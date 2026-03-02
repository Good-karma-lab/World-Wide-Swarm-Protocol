//! The OpenSwarmConnector struct that ties everything together.
//!
//! Initializes and orchestrates all subsystems: network, hierarchy,
//! consensus, and state management. Provides the high-level API
//! used by the RPC server and agent bridge.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, RwLock};

use openswarm_consensus::{CascadeEngine, RfpCoordinator, VotingEngine};
use openswarm_hierarchy::{
    EpochManager, GeoCluster, PyramidAllocator, SuccessionManager,
    elections::ElectionManager,
    epoch::EpochConfig,
    pyramid::PyramidConfig,
};
use openswarm_network::{
    Multiaddr, PeerId,
    NetworkEvent, SwarmHandle, SwarmHost, SwarmHostConfig,
    discovery::DiscoveryConfig,
    transport::TransportConfig,
};
use openswarm_protocol::*;
use openswarm_state::{ContentStore, GranularityAlgorithm, MerkleDag, OrSet};

use crate::config::ConnectorConfig;
use crate::reputation::{RepEvent, RepEventType, ReputationLedger, observer_weighted_points};
use crate::tui::{LogCategory, LogEntry};

const ACTIVE_MEMBER_STALENESS_SECS: u64 = 45;
const PARTICIPATION_POLL_STALENESS_SECS: u64 = 180;
const EXECUTION_ASSIGNMENT_TIMEOUT_SECS: i64 = 420;
const PROPOSAL_STAGE_TIMEOUT_SECS: i64 = 30;
const VOTING_STAGE_TIMEOUT_SECS: i64 = 30;

/// Maximum concurrent active tasks per principal (budget enforcement, Moltbook insight #19).
pub const MAX_CONCURRENT_INJECTIONS: usize = 50;
/// Maximum blast radius (sum of rollback_cost weights) per principal (Moltbook insight #19).
pub const MAX_BLAST_RADIUS: u32 = 200;

/// Returns the blast radius cost for a rollback_cost string value.
/// null/None → 0, "low" → 1, "medium" → 3, "high" → 10.
pub fn blast_radius_cost(rollback_cost: Option<&str>) -> u32 {
    match rollback_cost {
        Some("high") => 10,
        Some("medium") => 3,
        Some("low") => 1,
        _ => 0,
    }
}

/// Information about a known swarm tracked by this connector.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SwarmRecord {
    /// Swarm ID.
    pub swarm_id: SwarmId,
    /// Human-readable name.
    pub name: String,
    /// Whether the swarm is public.
    pub is_public: bool,
    /// Number of agents last reported in this swarm.
    pub agent_count: u64,
    /// Whether this connector is a member of this swarm.
    pub joined: bool,
    /// Last seen timestamp.
    pub last_seen: chrono::DateTime<chrono::Utc>,
}

/// A timeline event for a task lifecycle.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TaskTimelineEvent {
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub stage: String,
    pub detail: String,
    pub actor: Option<String>,
}

/// Debug trace record for peer-to-peer traffic.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MessageTraceEvent {
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub direction: String,
    pub peer: Option<String>,
    pub topic: String,
    pub method: Option<String>,
    pub task_id: Option<String>,
    pub size_bytes: usize,
    pub outcome: String,
}

#[derive(Debug, Clone, Default)]
pub struct AgentActivity {
    pub tasks_assigned_count: u64,
    pub tasks_processed_count: u64,
    pub plans_proposed_count: u64,
    pub plans_revealed_count: u64,
    pub votes_cast_count: u64,
    pub tasks_injected_count: u64,
    /// tasks_injected_count / tasks_processed_count (principal accountability).
    pub contribution_ratio: f64,
    /// Number of tasks that timed out with no signal (FailedSilently).
    pub silent_failure_count: u64,
    /// Total task outcomes reported (for computing silent_failure_rate).
    pub total_outcomes_reported: u64,
}

impl AgentActivity {
    pub fn silent_failure_rate(&self) -> f64 {
        if self.total_outcomes_reported == 0 {
            0.0
        } else {
            self.silent_failure_count as f64 / self.total_outcomes_reported as f64
        }
    }
}

/// A direct message received from another agent via the swarm DM topic.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct InboxMessage {
    pub from: String,
    pub to: String,
    pub content: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// Pending key rotation (48-hour grace window).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PendingKeyRotation {
    pub agent_did: String,
    pub old_pubkey_hex: String,
    pub new_pubkey_hex: String,
    pub rotation_timestamp: i64,
    pub grace_expires: chrono::DateTime<chrono::Utc>,
}

/// Pending emergency revocation (24-hour challenge window).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PendingRevocation {
    pub agent_did: String,
    pub recovery_pubkey_hex: String,
    pub new_primary_pubkey_hex: String,
    pub revocation_timestamp: i64,
    pub challenge_expires: chrono::DateTime<chrono::Utc>,
}

/// Guardian designation for social recovery.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GuardianDesignation {
    pub agent_did: String,
    pub guardians: Vec<String>,
    pub threshold: u32,
}

/// A single guardian vote for social recovery.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GuardianVote {
    pub guardian_did: String,
    pub target_did: String,
    pub new_pubkey: String,
    pub timestamp: i64,
}

#[derive(Debug, Clone, Default)]
pub struct TaskVoteRequirement {
    pub expected_proposers: usize,
    pub expected_voters: usize,
    pub tier_level: u32,
}

/// Status of the connector.
#[derive(Debug, Clone)]
pub enum ConnectorStatus {
    /// Initializing subsystems.
    Initializing,
    /// Connected to the swarm and operational.
    Running,
    /// Participating in an election.
    InElection,
    /// Shutting down.
    ShuttingDown,
}

/// Shared state accessible by the RPC server and event handlers.
pub struct ConnectorState {
    /// Our agent identity.
    pub agent_id: AgentId,
    /// Current status.
    pub status: ConnectorStatus,
    /// Epoch manager.
    pub epoch_manager: EpochManager,
    /// Pyramid allocator.
    pub pyramid: PyramidAllocator,
    /// Election manager (current epoch).
    pub election: Option<ElectionManager>,
    /// Geo-cluster manager.
    pub geo_cluster: GeoCluster,
    /// Succession manager.
    pub succession: SuccessionManager,
    /// Active RFP coordinators, keyed by task ID.
    pub rfp_coordinators: std::collections::HashMap<String, RfpCoordinator>,
    /// Active voting engines, keyed by task ID.
    pub voting_engines: std::collections::HashMap<String, VotingEngine>,
    /// Cascade engine for the current root task.
    pub cascade: CascadeEngine,
    /// CRDT set tracking active tasks.
    pub task_set: OrSet<String>,
    /// Full task metadata keyed by task ID.
    pub task_details: std::collections::HashMap<String, Task>,
    /// Per-task timeline events keyed by task ID.
    pub task_timelines: std::collections::HashMap<String, Vec<TaskTimelineEvent>>,
    /// CRDT set tracking active agents.
    pub agent_set: OrSet<String>,
    /// CRDT set tracking known swarm members (agent identities).
    pub member_set: OrSet<String>,
    /// Last seen timestamp for known swarm members.
    pub member_last_seen: std::collections::HashMap<String, chrono::DateTime<chrono::Utc>>,
    /// Human-readable display names for agents.
    pub agent_names: std::collections::HashMap<String, String>,
    /// Per-agent activity counters for operator diagnostics.
    pub agent_activity: std::collections::HashMap<String, AgentActivity>,
    /// Per-task expected participation constraints for proposals/voting.
    pub task_vote_requirements: std::collections::HashMap<String, TaskVoteRequirement>,
    /// Last time each agent polled tasks from its local connector loop.
    pub member_last_task_poll: std::collections::HashMap<String, chrono::DateTime<chrono::Utc>>,
    /// Last time each agent submitted a task result.
    pub member_last_result: std::collections::HashMap<String, chrono::DateTime<chrono::Utc>>,
    /// Optional textual result payload by task ID.
    pub task_result_text: std::collections::HashMap<String, String>,
    /// Deferred plan reveals waiting for commit quorum, keyed by task/proposer.
    pub pending_plan_reveals: std::collections::HashMap<String, std::collections::HashMap<String, Plan>>,
    /// Merkle DAG for result verification.
    pub merkle_dag: MerkleDag,
    /// Content-addressed storage.
    pub content_store: ContentStore,
    /// Granularity algorithm.
    pub granularity: GranularityAlgorithm,
    /// Current tier assignment for this agent.
    pub my_tier: Tier,
    /// Our parent agent ID (None if Tier-1).
    pub parent_id: Option<AgentId>,
    /// Maps agent_id -> assigned tier (for all known agents).
    pub agent_tiers: std::collections::HashMap<String, Tier>,
    /// Maps agent_id -> parent agent_id (for Tier-2+ agents).
    pub agent_parents: std::collections::HashMap<String, String>,
    /// Current pyramid layout (recomputed on swarm size changes).
    pub current_layout: Option<openswarm_hierarchy::pyramid::PyramidLayout>,
    /// Tracks subordinates for each coordinator: parent_id -> [child_ids].
    pub subordinates: std::collections::HashMap<String, Vec<String>>,
    /// Stores task results (artifacts) keyed by task_id.
    pub task_results: std::collections::HashMap<String, Artifact>,
    /// Network statistics cache.
    pub network_stats: NetworkStats,
    /// Event log for the TUI.
    pub event_log: Vec<LogEntry>,
    /// P2P message trace log for debugging and web dashboard.
    pub message_trace: Vec<MessageTraceEvent>,
    /// Timestamp when the connector started.
    pub start_time: chrono::DateTime<chrono::Utc>,
    /// The swarm ID this connector is currently a member of.
    pub current_swarm_id: SwarmId,
    /// Registry of all known swarms (discovered via DHT/GossipSub).
    pub known_swarms: std::collections::HashMap<String, SwarmRecord>,
    /// Swarm token for private swarm authentication (if any).
    pub swarm_token: Option<SwarmToken>,
    /// Active holonic boards, keyed by task_id.
    pub active_holons: std::collections::HashMap<String, HolonState>,
    /// Deliberation messages per task (proposal submissions, critiques, synthesis).
    pub deliberation_messages: std::collections::HashMap<String, Vec<DeliberationMessage>>,
    /// Per-voter ballot records per task, for full visibility.
    pub ballot_records: std::collections::HashMap<String, Vec<BallotRecord>>,
    /// IRV round history per task (populated after voting completes).
    pub irv_rounds: std::collections::HashMap<String, Vec<IrvRound>>,
    /// Board invitation acceptances per task: task_id -> Vec<BoardAcceptParams>.
    pub board_acceptances: std::collections::HashMap<String, Vec<BoardAcceptParams>>,
    /// Agent name registry: human-readable name -> DID.
    pub name_registry: std::collections::HashMap<String, String>,
    /// Inbox of direct messages received from other agents.
    pub inbox: Vec<InboxMessage>,
    /// Outbox of direct messages sent by this agent.
    pub outbox: Vec<InboxMessage>,
    /// Per-agent last inject timestamps for rate limiting (max 10 per 60s).
    pub inject_rate_limiter: std::collections::HashMap<String, Vec<chrono::DateTime<chrono::Utc>>>,
    /// Per-agent reputation ledgers (event log + scores).
    pub reputation_ledgers: std::collections::HashMap<String, ReputationLedger>,
    /// Rate limiting for reputation event submission: agent_id -> timestamps of recent events.
    pub rep_event_rate_limiter: std::collections::HashMap<String, Vec<chrono::DateTime<chrono::Utc>>>,
    /// Pending key rotation records (agent_did -> rotation).
    pub pending_key_rotations: std::collections::HashMap<String, PendingKeyRotation>,
    /// Pending emergency revocations (agent_did -> revocation).
    pub pending_revocations: std::collections::HashMap<String, PendingRevocation>,
    /// Guardian designations (agent_did -> designation).
    pub guardian_designations: std::collections::HashMap<String, GuardianDesignation>,
    /// Guardian recovery votes (target_did -> Vec<vote>).
    pub guardian_votes: std::collections::HashMap<String, Vec<GuardianVote>>,
    /// Commitment receipts by receipt_id (Moltbook insight #14).
    pub receipts: std::collections::HashMap<String, openswarm_protocol::CommitmentReceipt>,
    /// Clarification requests by clarification_id (Moltbook insight #20).
    pub clarifications: std::collections::HashMap<String, openswarm_protocol::ClarificationRequest>,
    /// Path to persist the agent's chosen display name across restarts.
    pub name_file_path: Option<std::path::PathBuf>,
}

impl ConnectorState {
    /// Returns true if the given agent_id has sufficient reputation to inject tasks.
    /// The local agent (self) is always allowed.
    pub fn has_inject_reputation(&self, agent_id: &str) -> bool {
        // Delegates to can_inject_task with simple task complexity threshold
        self.can_inject_task(agent_id, 0.5)
    }

    /// Check and update rate limit for task injection.
    /// Returns true if the agent is within the rate limit (max 10 injections per 60 seconds).
    pub fn check_and_update_inject_rate_limit(&mut self, agent_id: &str) -> bool {
        let now = chrono::Utc::now();
        let window = chrono::Duration::seconds(60);
        let max_per_window: usize = 10;
        let timestamps = self.inject_rate_limiter
            .entry(agent_id.to_string())
            .or_insert_with(Vec::new);
        // Remove timestamps older than the window
        timestamps.retain(|&t| now - t < window);
        if timestamps.len() >= max_per_window {
            return false;
        }
        timestamps.push(now);
        true
    }

    /// Get or create the reputation ledger for an agent.
    pub fn ledger_mut(&mut self, agent_id: &str) -> &mut ReputationLedger {
        self.reputation_ledgers
            .entry(agent_id.to_string())
            .or_default()
    }

    /// Apply an objective or observer-weighted reputation event to an agent.
    pub fn apply_rep_event(
        &mut self,
        agent_id: &str,
        event_type: RepEventType,
        task_id: Option<String>,
    ) {
        let base = event_type.base_points();
        let is_obj = event_type.is_objective();
        // Capture agent_id as String before any mutable borrows
        let my_id = self.agent_id.to_string();
        let observer_score = self
            .reputation_ledgers
            .get(&my_id)
            .map(|l| l.effective_score())
            .unwrap_or(0);
        let effective = observer_weighted_points(base, observer_score, is_obj);
        let event = RepEvent {
            event_type,
            base_points: base,
            observer: my_id,
            observer_score,
            effective_points: effective,
            task_id,
            timestamp: chrono::Utc::now(),
            evidence: None,
        };
        self.reputation_ledgers
            .entry(agent_id.to_string())
            .or_default()
            .apply_event(event);
    }

    /// Check whether an agent can inject a task of the given complexity.
    ///
    /// Self (local connector) is always allowed. Others must meet tier requirements.
    pub fn can_inject_task(&self, agent_id: &str, complexity: f64) -> bool {
        if self.agent_id.to_string() == agent_id {
            return true;
        }
        let score = self
            .reputation_ledgers
            .get(agent_id)
            .map(|l| l.effective_score())
            .unwrap_or(0);
        let min_score = crate::reputation::ScoreTier::min_inject_score(complexity);
        score >= min_score
    }

    /// Count unverified (AgentFulfilled) receipts for a given agent.
    pub fn unverified_receipt_count(&self, agent_id: &str) -> usize {
        self.receipts.values()
            .filter(|r| r.agent_id == agent_id
                && r.commitment_state == openswarm_protocol::CommitmentState::AgentFulfilled)
            .count()
    }

    /// Compute blast radius for a principal's active receipts.
    pub fn principal_blast_radius(&self, principal_id: &str) -> u32 {
        self.receipts.values()
            .filter(|r| r.agent_id == principal_id
                && matches!(r.commitment_state,
                    openswarm_protocol::CommitmentState::Active
                    | openswarm_protocol::CommitmentState::AgentFulfilled))
            .map(|r| blast_radius_cost(r.rollback_cost.as_deref()))
            .sum()
    }

    /// Count active (non-terminal) tasks across the entire swarm as a conservative upper bound
    /// for a principal's active injection count.
    ///
    /// NOTE: Because `Task` does not store an `injector_id` field, we cannot filter by principal.
    /// This returns a global count and will over-estimate for any individual principal.
    /// TODO: store `injector_id` on `Task` to make this per-principal.
    pub fn principal_active_injection_count(&self, _principal_id: &str) -> usize {
        self.task_details.values()
            .filter(|t| {
                // Count tasks this principal injected that are still active
                // We approximate: tasks where assigned_to != principal (they're running), not yet done
                // Actually track by injector via activity - simplified: count non-terminal tasks
                // injected by this principal via agent_activity injected_count vs processed
                // For simplicity: count all non-terminal tasks where the injector recorded is this principal
                // Since we don't store injector on task, count active tasks globally as a conservative bound
                // The actual check should use task.assigned_to but this gives a safe upper bound
                matches!(t.status,
                    openswarm_protocol::TaskStatus::Pending
                    | openswarm_protocol::TaskStatus::InProgress
                    | openswarm_protocol::TaskStatus::ProposalPhase
                    | openswarm_protocol::TaskStatus::VotingPhase
                    | openswarm_protocol::TaskStatus::PendingReview)
            })
            .count()
    }

    /// Compute guardian quality score for a DID.
    /// Returns (quality_score, guardian_count) where quality_score is 0.0–1.0.
    pub fn guardian_quality_score(&self, agent_did: &str) -> (f64, usize) {
        let designation = match self.guardian_designations.get(agent_did) {
            Some(d) => d,
            None => return (0.0, 0),
        };
        let tier_score = |did: &str| -> f64 {
            let score = self.reputation_ledgers.get(did).map(|l| l.effective_score()).unwrap_or(0);
            use crate::reputation::ScoreTier;
            match ScoreTier::from_score(score) {
                ScoreTier::Newcomer => 0.1,
                ScoreTier::Member => 0.2,
                ScoreTier::Trusted => 0.5,
                ScoreTier::Established => 0.75,
                ScoreTier::Veteran => 1.0,
                ScoreTier::Suspended => 0.0,
            }
        };
        let n = designation.guardians.len();
        if n == 0 {
            return (0.0, 0);
        }
        let sum: f64 = designation.guardians.iter().map(|g| tier_score(g)).sum();
        (sum / n as f64, n)
    }

    /// Check the reputation event submission rate limit (max 20 per agent per hour).
    pub fn check_rep_event_rate_limit(&mut self, agent_id: &str) -> bool {
        let now = chrono::Utc::now();
        let window = chrono::Duration::hours(1);
        let max_per_window: usize = 20;
        let timestamps = self
            .rep_event_rate_limiter
            .entry(agent_id.to_string())
            .or_insert_with(Vec::new);
        timestamps.retain(|&t| now - t < window);
        if timestamps.len() >= max_per_window {
            return false;
        }
        timestamps.push(now);
        true
    }

    /// Push a log entry, capping the log at 1000 entries.
    pub fn push_log(&mut self, category: LogCategory, message: String) {
        if self.event_log.len() >= 1000 {
            self.event_log.remove(0);
        }
        self.event_log.push(LogEntry {
            timestamp: chrono::Utc::now(),
            category,
            message,
        });
    }

    pub fn push_message_trace(&mut self, event: MessageTraceEvent) {
        if self.message_trace.len() >= 5000 {
            self.message_trace.remove(0);
        }
        self.message_trace.push(event);
    }

    pub fn push_task_timeline_event(
        &mut self,
        task_id: &str,
        stage: &str,
        detail: impl Into<String>,
        actor: Option<String>,
    ) {
        let timeline = self.task_timelines.entry(task_id.to_string()).or_default();
        timeline.push(TaskTimelineEvent {
            timestamp: chrono::Utc::now(),
            stage: stage.to_string(),
            detail: detail.into(),
            actor,
        });
        if timeline.len() > 500 {
            timeline.remove(0);
        }
    }

    pub fn mark_member_seen(&mut self, agent_id: &str) {
        self.mark_member_seen_with_name(agent_id, None);
    }

    pub fn mark_member_seen_with_name(&mut self, agent_id: &str, name: Option<&str>) {
        if agent_id.trim().is_empty() {
            return;
        }
        self.member_set.add(agent_id.to_string());
        self.member_last_seen
            .insert(agent_id.to_string(), chrono::Utc::now());
        if let Some(n) = name.map(str::trim).filter(|n| !n.is_empty()) {
            self.agent_names.insert(agent_id.to_string(), n.to_string());
        }
    }

    pub fn mark_member_polled_tasks(&mut self, agent_id: &str) {
        self.member_last_task_poll
            .insert(agent_id.to_string(), chrono::Utc::now());
    }

    pub fn mark_member_submitted_result(&mut self, agent_id: &str) {
        self.member_last_result
            .insert(agent_id.to_string(), chrono::Utc::now());
    }

    fn activity_mut(&mut self, agent_id: &str) -> &mut AgentActivity {
        self.agent_activity
            .entry(agent_id.to_string())
            .or_default()
    }

    pub fn bump_tasks_assigned(&mut self, agent_id: &str) {
        self.activity_mut(agent_id).tasks_assigned_count += 1;
    }

    pub fn bump_tasks_injected(&mut self, agent_id: &str) {
        let a = self.activity_mut(agent_id);
        a.tasks_injected_count += 1;
        let processed = a.tasks_processed_count.max(1) as f64;
        a.contribution_ratio = a.tasks_injected_count as f64 / processed;
    }

    pub fn bump_tasks_processed(&mut self, agent_id: &str) {
        self.activity_mut(agent_id).tasks_processed_count += 1;
        // Recalculate contribution ratio
        {
            let a = self.activity_mut(agent_id);
            let injected = a.tasks_injected_count as f64;
            let processed = a.tasks_processed_count as f64;
            a.contribution_ratio = if processed > 0.0 { injected / processed } else { 0.0 };
        }
        self.apply_rep_event(agent_id, RepEventType::TaskExecutedVerified, None);
    }

    pub fn bump_plans_proposed(&mut self, agent_id: &str) {
        self.activity_mut(agent_id).plans_proposed_count += 1;
    }

    pub fn bump_plans_revealed(&mut self, agent_id: &str) {
        self.activity_mut(agent_id).plans_revealed_count += 1;
    }

    pub fn bump_votes_cast(&mut self, agent_id: &str) {
        self.activity_mut(agent_id).votes_cast_count += 1;
        self.apply_rep_event(agent_id, RepEventType::VoteCastInIrv, None);
    }

    pub fn active_member_ids(&self, max_staleness: Duration) -> Vec<String> {
        let now = chrono::Utc::now();
        let mut ids: Vec<String> = self
            .member_last_seen
            .iter()
            .filter_map(|(agent_id, seen)| {
                now.signed_duration_since(*seen)
                    .to_std()
                    .ok()
                    .filter(|age| *age <= max_staleness)
                    .map(|_| agent_id.clone())
            })
            .collect();

        let self_id = self.agent_id.to_string();
        if !ids.iter().any(|id| id == &self_id) {
            ids.push(self_id);
        }

        ids.sort();
        ids.dedup();
        ids
    }

    pub fn active_member_count(&self, max_staleness: Duration) -> usize {
        self.active_member_ids(max_staleness).len()
    }

    pub fn prune_stale_members(&mut self, max_staleness: Duration) {
        let now = chrono::Utc::now();
        let stale_ids: Vec<String> = self
            .member_last_seen
            .iter()
            .filter_map(|(agent_id, seen)| {
                let age = now.signed_duration_since(*seen).to_std().ok()?;
                if age > max_staleness {
                    Some(agent_id.clone())
                } else {
                    None
                }
            })
            .collect();

        for stale in stale_ids {
            if stale != self.agent_id.to_string() {
                self.member_set.remove(&stale);
                self.member_last_seen.remove(&stale);
                self.member_last_task_poll.remove(&stale);
                self.member_last_result.remove(&stale);
                self.agent_activity.remove(&stale);
                self.agent_tiers.remove(&stale);
                self.agent_parents.remove(&stale);
            }
        }
    }
}

/// The main WWS.Connector that orchestrates all subsystems.
///
/// Created from a configuration, it initializes the network, hierarchy,
/// consensus, and state modules, then runs the event loop that ties
/// them together.
pub struct OpenSwarmConnector {
    /// Shared mutable state.
    pub state: Arc<RwLock<ConnectorState>>,
    /// Network handle for sending commands to the swarm.
    pub network_handle: SwarmHandle,
    /// Channel for receiving network events.
    event_rx: Option<mpsc::Receiver<NetworkEvent>>,
    /// The swarm host (to be spawned).
    swarm_host: Option<SwarmHost>,
    /// Configuration.
    config: ConnectorConfig,
}

impl OpenSwarmConnector {
    /// Create a new connector from configuration.
    ///
    /// Initializes all subsystems but does not start the event loop.
    /// Call `run()` to start processing.
    pub fn new(config: ConnectorConfig) -> Result<Self, anyhow::Error> {
        // Build network configuration.
        let listen_addr = config.network.listen_addr.parse()
            .map_err(|e| anyhow::anyhow!("Invalid listen address: {}", e))?;

        // Parse bootstrap peer multiaddresses into (PeerId, Multiaddr) pairs.
        let bootstrap_peers = Self::parse_bootstrap_peers(&config.network.bootstrap_peers);

        let swarm_config = SwarmHostConfig {
            listen_addr,
            transport: TransportConfig::default(),
            discovery: DiscoveryConfig {
                mdns_enabled: config.network.mdns_enabled,
                bootstrap_peers,
                ..Default::default()
            },
            ..Default::default()
        };

        let (swarm_host, network_handle, event_rx) = SwarmHost::new(swarm_config)?;
        let local_peer_id = network_handle.local_peer_id();
        let agent_id = AgentId::new(format!("did:swarm:{}", local_peer_id));

        // Initialize hierarchy.
        let pyramid_config = PyramidConfig {
            branching_factor: config.hierarchy.branching_factor,
            ..Default::default()
        };
        let epoch_config = EpochConfig {
            duration_secs: config.hierarchy.epoch_duration_secs,
            ..Default::default()
        };

        // Build swarm identity.
        let current_swarm_id = SwarmId::new(config.swarm.swarm_id.clone());
        let swarm_token = config.swarm.token.as_ref().map(|t| SwarmToken::new(t.clone()));

        // Initialize known swarms with our own swarm.
        let mut known_swarms = std::collections::HashMap::new();
        known_swarms.insert(
            current_swarm_id.as_str().to_string(),
            SwarmRecord {
                swarm_id: current_swarm_id.clone(),
                name: config.swarm.name.clone(),
                is_public: config.swarm.is_public,
                agent_count: 1,
                joined: true,
                last_seen: chrono::Utc::now(),
            },
        );

        let state = ConnectorState {
            agent_id: agent_id.clone(),
            status: ConnectorStatus::Initializing,
            epoch_manager: EpochManager::new(epoch_config),
            pyramid: PyramidAllocator::new(pyramid_config),
            election: None,
            geo_cluster: GeoCluster::default(),
            succession: SuccessionManager::new(),
            rfp_coordinators: std::collections::HashMap::new(),
            voting_engines: std::collections::HashMap::new(),
            cascade: CascadeEngine::new(),
            task_set: OrSet::new(agent_id.to_string()),
            task_details: std::collections::HashMap::new(),
            task_timelines: std::collections::HashMap::new(),
            agent_set: OrSet::new(agent_id.to_string()),
            member_set: OrSet::new(agent_id.to_string()),
            member_last_seen: {
                let mut m = std::collections::HashMap::new();
                m.insert(agent_id.to_string(), chrono::Utc::now());
                m
            },
            agent_names: {
                let mut m = std::collections::HashMap::new();
                m.insert(agent_id.to_string(), config.agent.name.clone());
                m
            },
            agent_activity: {
                let mut m = std::collections::HashMap::new();
                m.insert(agent_id.to_string(), AgentActivity::default());
                m
            },
            task_vote_requirements: std::collections::HashMap::new(),
            member_last_task_poll: std::collections::HashMap::new(),
            member_last_result: std::collections::HashMap::new(),
            task_result_text: std::collections::HashMap::new(),
            pending_plan_reveals: std::collections::HashMap::new(),
            merkle_dag: MerkleDag::new(),
            content_store: ContentStore::new(),
            granularity: GranularityAlgorithm::default(),
            my_tier: Tier::Executor,
            parent_id: None,
            agent_tiers: std::collections::HashMap::new(),
            agent_parents: std::collections::HashMap::new(),
            current_layout: None,
            subordinates: std::collections::HashMap::new(),
            task_results: std::collections::HashMap::new(),
            network_stats: NetworkStats {
                total_agents: 1,
                hierarchy_depth: 1,
                branching_factor: config.hierarchy.branching_factor,
                current_epoch: 1,
                my_tier: Tier::Executor,
                subordinate_count: 0,
                parent_id: None,
            },
            event_log: Vec::new(),
            message_trace: Vec::new(),
            start_time: chrono::Utc::now(),
            current_swarm_id,
            known_swarms,
            swarm_token,
            active_holons: std::collections::HashMap::new(),
            deliberation_messages: std::collections::HashMap::new(),
            ballot_records: std::collections::HashMap::new(),
            irv_rounds: std::collections::HashMap::new(),
            board_acceptances: std::collections::HashMap::new(),
            name_registry: std::collections::HashMap::new(),
            inbox: Vec::new(),
            outbox: Vec::new(),
            inject_rate_limiter: std::collections::HashMap::new(),
            reputation_ledgers: std::collections::HashMap::new(),
            rep_event_rate_limiter: std::collections::HashMap::new(),
            pending_key_rotations: std::collections::HashMap::new(),
            pending_revocations: std::collections::HashMap::new(),
            guardian_designations: std::collections::HashMap::new(),
            guardian_votes: std::collections::HashMap::new(),
            receipts: std::collections::HashMap::new(),
            clarifications: std::collections::HashMap::new(),
            name_file_path: None,
        };

        Ok(Self {
            state: Arc::new(RwLock::new(state)),
            network_handle,
            event_rx: Some(event_rx),
            swarm_host: Some(swarm_host),
            config,
        })
    }

    /// Start the connector, running the swarm and event loop.
    ///
    /// This spawns the swarm host as a background task and runs
    /// the main event processing loop.
    pub async fn run(mut self) -> Result<(), anyhow::Error> {
        // Take and spawn the swarm host.
        let swarm_host = self
            .swarm_host
            .take()
            .ok_or_else(|| anyhow::anyhow!("SwarmHost already consumed"))?;

        tokio::spawn(async move {
            if let Err(e) = swarm_host.run().await {
                tracing::error!(error = %e, "Swarm host error");
            }
        });

        // Subscribe to core topics.
        self.network_handle.subscribe_core_topics().await?;

        // Subscribe to our swarm's topics (if not the default public swarm,
        // since core topics already include the public swarm).
        let swarm_id_str = self.config.swarm.swarm_id.clone();
        if swarm_id_str != openswarm_protocol::DEFAULT_SWARM_ID {
            self.network_handle
                .subscribe_swarm_topics(&swarm_id_str)
                .await?;
        }

        self.subscribe_task_assignment_topics(&swarm_id_str).await;

        // Subscribe to the shared DM topic so we receive direct messages.
        let dm_topic = openswarm_protocol::SwarmTopics::dm_for(&swarm_id_str);
        if let Err(e) = self.network_handle.subscribe(&dm_topic).await {
            tracing::warn!(err = %e, "Failed to subscribe to DM topic");
        }

        // Connect to bootstrap peers to join the swarm network immediately.
        self.connect_to_bootstrap_peers().await;

        // Initiate Kademlia bootstrap to populate the DHT routing table.
        if !self.config.network.bootstrap_peers.is_empty() {
            if let Err(e) = self.network_handle.bootstrap().await {
                tracing::warn!(error = %e, "Kademlia bootstrap initiation failed");
            }
        }

        // Update status.
        {
            let mut state = self.state.write().await;
            state.status = ConnectorStatus::Running;
            state.push_log(
                LogCategory::System,
                format!(
                    "WWS.Connector started (swarm: {} [{}])",
                    self.config.swarm.name, swarm_id_str
                ),
            );
        }

        tracing::info!("WWS.Connector is running");

        // Take the event receiver out of self so we can use both in the loop.
        let mut event_rx = self
            .event_rx
            .take()
            .ok_or_else(|| anyhow::anyhow!("Event receiver already consumed"))?;
        let keepalive_secs = self.config.hierarchy.keepalive_interval_secs;
        let mut keepalive_interval =
            tokio::time::interval(Duration::from_secs(keepalive_secs));
        let mut epoch_tick = tokio::time::interval(Duration::from_secs(1));
        let announce_secs = self.config.swarm.announce_interval_secs;
        let mut swarm_announce_interval =
            tokio::time::interval(Duration::from_secs(announce_secs));
        let mut bootstrap_retry_interval = tokio::time::interval(Duration::from_secs(20));
        // Voting completion check every 5 seconds
        let mut voting_check_interval = tokio::time::interval(Duration::from_secs(5));
        let mut execution_timeout_interval = tokio::time::interval(Duration::from_secs(10));

        loop {
            tokio::select! {
                Some(event) = event_rx.recv() => {
                    self.handle_network_event(event).await;
                }
                _ = keepalive_interval.tick() => {
                    self.send_keepalive().await;
                }
                _ = epoch_tick.tick() => {
                    self.check_epoch_transition().await;
                }
                _ = swarm_announce_interval.tick() => {
                    self.announce_swarm().await;
                }
                _ = bootstrap_retry_interval.tick() => {
                    self.connect_to_bootstrap_peers().await;
                    if !self.config.network.bootstrap_peers.is_empty() {
                        let _ = self.network_handle.bootstrap().await;
                    }
                }
                _ = voting_check_interval.tick() => {
                    self.check_voting_completion().await;
                }
                _ = execution_timeout_interval.tick() => {
                    self.check_execution_timeouts().await;
                }
            }
        }
    }

    /// Handle a network event from the swarm.
    async fn handle_network_event(&self, event: NetworkEvent) {
        match event {
            NetworkEvent::MessageReceived { topic, data, source, .. } => {
                {
                    let mut state = self.state.write().await;
                    let decoded = serde_json::from_slice::<SwarmMessage>(&data).ok();
                    let task_id = decoded
                        .as_ref()
                        .and_then(|m| m.params.get("task_id").and_then(|v| v.as_str()).map(|s| s.to_string()));
                    state.push_message_trace(MessageTraceEvent {
                        timestamp: chrono::Utc::now(),
                        direction: "inbound".to_string(),
                        peer: Some(source.to_string()),
                        topic: topic.clone(),
                        method: decoded.as_ref().map(|m| m.method.clone()),
                        task_id,
                        size_bytes: data.len(),
                        outcome: "received".to_string(),
                    });
                    state.push_log(
                        LogCategory::Message,
                        format!("Message received on {} from {}", topic, source),
                    );
                }
                self.handle_message(&topic, &data, source).await;
            }
            NetworkEvent::PeerConnected(peer) => {
                tracing::debug!(peer = %peer, "Peer connected");
                let mut state = self.state.write().await;
                state.agent_set.add(peer.to_string());
                state.mark_member_seen(&format!("did:swarm:{}", peer));
                state.push_log(
                    LogCategory::Peer,
                    format!("Connected: {}", peer),
                );
                state.network_stats.total_agents = state
                    .active_member_count(Duration::from_secs(ACTIVE_MEMBER_STALENESS_SECS))
                    as u64;
            }
            NetworkEvent::PeerDisconnected(peer) => {
                tracing::debug!(peer = %peer, "Peer disconnected");
                let mut state = self.state.write().await;
                state.agent_set.remove(&peer.to_string());
                state.push_log(
                    LogCategory::Peer,
                    format!("Disconnected: {}", peer),
                );
            }
            NetworkEvent::PingRtt { peer, rtt } => {
                tracing::trace!(peer = %peer, rtt_ms = rtt.as_millis(), "Ping RTT");
            }
            _ => {}
        }
    }

    /// Handle a protocol message received on a topic.
    async fn handle_message(
        &self,
        topic: &str,
        data: &[u8],
        source: openswarm_network::PeerId,
    ) {
        let message: SwarmMessage = match serde_json::from_slice(data) {
            Ok(m) => m,
            Err(e) => {
                tracing::warn!(error = %e, "Failed to parse swarm message");
                let mut state = self.state.write().await;
                state.push_log(
                    LogCategory::Error,
                    format!("Failed to parse message on {}: {}", topic, e),
                );
                state.push_message_trace(MessageTraceEvent {
                    timestamp: chrono::Utc::now(),
                    direction: "inbound".to_string(),
                    peer: Some(source.to_string()),
                    topic: topic.to_string(),
                    method: None,
                    task_id: None,
                    size_bytes: data.len(),
                    outcome: "parse_error".to_string(),
                });
                return;
            }
        };

        match ProtocolMethod::from_str(&message.method) {
            Some(ProtocolMethod::KeepAlive) => {
                if let Ok(params) = serde_json::from_value::<KeepAliveParams>(message.params) {
                    let mut state = self.state.write().await;
                    state.succession.record_keepalive(&params.agent_id);
                    state.mark_member_seen_with_name(
                        params.agent_id.as_str(),
                        params.agent_name.as_deref(),
                    );
                    if let Some(ts) = params.last_task_poll_at {
                        state
                            .member_last_task_poll
                            .insert(params.agent_id.to_string(), ts);
                    }
                    if let Some(ts) = params.last_result_at {
                        state.member_last_result.insert(params.agent_id.to_string(), ts);
                    }
                    let active_members =
                        state.active_member_ids(Duration::from_secs(ACTIVE_MEMBER_STALENESS_SECS));
                    Self::recompute_hierarchy_from_members(&mut state, &active_members);
                    state.push_log(
                        LogCategory::Message,
                        format!(
                            "KeepAlive from {}",
                            params
                                .agent_name
                                .clone()
                                .unwrap_or_else(|| params.agent_id.to_string())
                        ),
                    );
                }
            }
            Some(ProtocolMethod::AgentKeepAlive) => {
                if let Ok(params) = serde_json::from_value::<KeepAliveParams>(message.params) {
                    let mut state = self.state.write().await;
                    state.mark_member_seen_with_name(
                        params.agent_id.as_str(),
                        params.agent_name.as_deref(),
                    );
                    if let Some(ts) = params.last_task_poll_at {
                        state
                            .member_last_task_poll
                            .insert(params.agent_id.to_string(), ts);
                    }
                    if let Some(ts) = params.last_result_at {
                        state.member_last_result.insert(params.agent_id.to_string(), ts);
                    }
                    let active_members =
                        state.active_member_ids(Duration::from_secs(ACTIVE_MEMBER_STALENESS_SECS));
                    Self::recompute_hierarchy_from_members(&mut state, &active_members);
                    state.push_log(
                        LogCategory::System,
                        format!(
                            "Agent heartbeat: {}",
                            params
                                .agent_name
                                .clone()
                                .unwrap_or_else(|| params.agent_id.to_string())
                        ),
                    );
                }
            }
            Some(ProtocolMethod::Candidacy) => {
                if let Ok(params) = serde_json::from_value::<CandidacyParams>(message.params) {
                    let mut state = self.state.write().await;
                    if let Some(ref mut election) = state.election {
                        if let Err(e) = election.register_candidate(&params) {
                            tracing::warn!(error = %e, "Failed to register candidate");
                        }
                    }
                }
            }
            Some(ProtocolMethod::ElectionVote) => {
                if let Ok(params) = serde_json::from_value::<ElectionVoteParams>(message.params) {
                    let mut state = self.state.write().await;
                    if let Some(ref mut election) = state.election {
                        if let Err(e) = election.record_vote(params) {
                            tracing::warn!(error = %e, "Failed to record election vote");
                        }
                    }
                }
            }
            Some(ProtocolMethod::TierAssignment) => {
                if let Ok(params) = serde_json::from_value::<TierAssignmentParams>(message.params)
                {
                    let level = Self::tier_to_level(params.tier);
                    let mut state = self.state.write().await;
                    if params.assigned_agent == state.agent_id {
                        state.my_tier = params.tier;
                        state.parent_id = Some(params.parent_id);
                        state.network_stats.my_tier = params.tier;
                        tracing::info!(tier = ?params.tier, "Tier assignment received");
                    }
                    drop(state);

                    if let Some(level) = level {
                        let swarm_id = {
                            let state = self.state.read().await;
                            state.current_swarm_id.as_str().to_string()
                        };
                        let topic = SwarmTopics::tasks_for(&swarm_id, level);
                        if let Err(e) = self.network_handle.subscribe(&topic).await {
                            tracing::debug!(error = %e, topic = %topic, "Failed to subscribe assigned tier topic");
                        }
                    }
                }
            }
            Some(ProtocolMethod::TaskInjection) => {
                if let Ok(params) = serde_json::from_value::<TaskInjectionParams>(message.params) {
                    let mut state = self.state.write().await;

                    // Tier-filtered task reception: only process tasks for our tier level
                    let my_tier = state.my_tier;
                    let task_tier_level = params.task.tier_level;

                    // Each tier processes tasks at its level:
                    // - Tier1 processes tier_level 1
                    // - Tier2 processes tier_level 2
                    // - TierN(n) processes tier_level n
                    // - Executor processes any tier_level (leaf workers)
                    let my_tier_level = my_tier.depth();
                    let should_process = match my_tier {
                        Tier::Executor => true, // Executors handle any level (leaf work)
                        _ => my_tier_level == task_tier_level, // Coordinators only handle their level
                    };

                    if !should_process {
                        tracing::debug!(
                            task_id = %params.task.task_id,
                            my_tier = ?my_tier,
                            task_tier = task_tier_level,
                            "Ignoring task for different tier"
                        );
                        drop(state);
                        return;
                    }

                    state.task_set.add(params.task.task_id.clone());
                    state
                        .task_details
                        .insert(params.task.task_id.clone(), params.task.clone());
                    state.push_task_timeline_event(
                        &params.task.task_id,
                        "injected",
                        format!("Task injected: {}", params.task.description),
                        None,
                    );
                    state.push_log(
                        LogCategory::Task,
                        format!(
                            "Task injected at my tier: {} ({})",
                            params.task.task_id, params.task.description
                        ),
                    );

                    // All coordinator tiers initialize RFP for competitive planning
                    let is_coordinator = my_tier != Tier::Executor;
                    let task_id = params.task.task_id.clone();
                    let epoch = params.task.epoch;

                    if is_coordinator {
                        // Count agents at my tier level for quorum
                        let my_tier_agents = state.agent_tiers.values()
                            .filter(|t| **t == my_tier)
                            .count();

                        if my_tier_agents > 0 {
                            let mut rfp = RfpCoordinator::new(
                                task_id.clone(),
                                epoch,
                                my_tier_agents,
                            );

                            if let Err(e) = rfp.inject_task(&params.task) {
                                tracing::error!(error = %e, "Failed to initialize RFP");
                            } else {
                                state.rfp_coordinators.insert(task_id.clone(), rfp);
                                state.push_log(
                                    LogCategory::Task,
                                    format!("RFP initialized for task {} with {} {:?} agents", task_id, my_tier_agents, my_tier),
                                );
                            }
                        }
                    }

                    // Create holon record for this task in Forming status
                    let my_agent_id = state.agent_id.clone();
                    let task_tier = params.task.tier_level;
                    let parent_task_id = params.task.parent_task_id.clone();
                    state.active_holons.entry(task_id.clone()).or_insert_with(|| HolonState {
                        task_id: task_id.clone(),
                        chair: my_agent_id,
                        members: Vec::new(),
                        adversarial_critic: None,
                        depth: task_tier,
                        parent_holon: parent_task_id,
                        child_holons: Vec::new(),
                        subtask_assignments: std::collections::HashMap::new(),
                        status: HolonStatus::Forming,
                        created_at: chrono::Utc::now(),
                    });

                    tracing::info!(
                        task_id = %params.task.task_id,
                        my_tier = ?my_tier,
                        is_coordinator,
                        "Task received and accepted"
                    );

                    let swarm_id = state.current_swarm_id.as_str().to_string();
                    drop(state);

                    self.subscribe_task_flow_topics(&swarm_id, &task_id).await;
                }
            }
            Some(ProtocolMethod::TaskAssignment) => {
                if let Ok(params) = serde_json::from_value::<TaskAssignmentParams>(message.params) {
                    let mut state = self.state.write().await;
                    let mut task = params.task.clone();
                    task.assigned_to = Some(params.assignee.clone());
                    task.status = TaskStatus::InProgress;
                    if task.deadline.is_none() {
                        task.deadline = Some(
                            chrono::Utc::now()
                                + chrono::Duration::seconds(EXECUTION_ASSIGNMENT_TIMEOUT_SECS),
                        );
                    }

                    let task_id = task.task_id.clone();
                    let parent_task_id = params.parent_task_id.clone();
                    let assigned_here = params.assignee == state.agent_id;

                    if let Some(existing) = state.task_details.get(&task_id) {
                        if matches!(existing.status, TaskStatus::Completed) {
                            task.status = TaskStatus::Completed;
                        }
                    }

                    state.task_details.insert(task_id.clone(), task);
                    if let Some(parent) = state.task_details.get_mut(&parent_task_id) {
                        if !parent.subtasks.iter().any(|id| id == &task_id) {
                            parent.subtasks.push(task_id.clone());
                        }
                    }
                    if assigned_here {
                        state.task_set.add(task_id.clone());
                    }

                    state.mark_member_seen(params.assignee.as_str());
                    state.bump_tasks_assigned(params.assignee.as_str());
                    state.push_task_timeline_event(
                        &task_id,
                        if assigned_here { "assigned" } else { "assignment_observed" },
                        format!(
                            "Assigned by plan {} under parent {}",
                            params.winning_plan_id, params.parent_task_id
                        ),
                        Some(params.assignee.to_string()),
                    );
                    state.push_log(
                        LogCategory::Task,
                        if assigned_here {
                            format!(
                                "Task assigned: {} to {} (plan={}, parent={})",
                                task_id, params.assignee, params.winning_plan_id, params.parent_task_id
                            )
                        } else {
                            format!(
                                "Task assignment observed: {} -> {} (plan={}, parent={})",
                                task_id, params.assignee, params.winning_plan_id, params.parent_task_id
                            )
                        },
                    );
                    state.push_log(
                        LogCategory::System,
                        format!(
                            "AUDIT assignment.observe task_id={} assignee={} parent={} local_assignee={}",
                            task_id,
                            params.assignee,
                            params.parent_task_id,
                            assigned_here
                        ),
                    );

                    let swarm_id = state.current_swarm_id.as_str().to_string();
                    drop(state);
                    self.subscribe_task_flow_topics(&swarm_id, &task_id).await;
                }
            }
            Some(ProtocolMethod::ProposalCommit) => {
                if let Ok(params) =
                    serde_json::from_value::<ProposalCommitParams>(message.params)
                {
                    let mut state = self.state.write().await;
                    // A ProposalCommit is proof of activity — mark proposer as active before
                    // the participation check to avoid KeepAlive propagation race conditions.
                    state.mark_member_seen(params.proposer.as_str());
                    state.mark_member_polled_tasks(params.proposer.as_str());
                    if !Self::is_participating_member_for_task(
                        &state,
                        &params.task_id,
                        params.proposer.as_str(),
                        Duration::from_secs(PARTICIPATION_POLL_STALENESS_SECS),
                    ) {
                        state.push_log(
                            LogCategory::Task,
                            format!(
                                "Ignoring proposal commit from non-responding proposer {} for task {}",
                                params.proposer, params.task_id
                            ),
                        );
                        return;
                    }
                    if let Some(task) = state.task_details.get_mut(&params.task_id) {
                        task.status = TaskStatus::ProposalPhase;
                    }

                    let requirement = Self::expected_vote_requirement_for_task(&state, &params.task_id);
                    state
                        .task_vote_requirements
                        .insert(params.task_id.clone(), requirement.clone());
                    let injected_task = state
                        .task_details
                        .get(&params.task_id)
                        .cloned()
                        .unwrap_or(Task {
                            task_id: params.task_id.clone(),
                            parent_task_id: None,
                            epoch: params.epoch,
                            status: TaskStatus::Pending,
                            description: "Observed proposal commit".to_string(),
                            assigned_to: None,
                            tier_level: requirement.tier_level,
                            subtasks: Vec::new(),
                            created_at: chrono::Utc::now(),
                            deadline: None,
                            ..Default::default()
                        });
                    {
                        let rfp = state
                            .rfp_coordinators
                            .entry(params.task_id.clone())
                            .or_insert_with(|| {
                                RfpCoordinator::new(
                                    params.task_id.clone(),
                                    params.epoch,
                                    requirement.expected_proposers,
                                )
                            });
                        if matches!(rfp.phase(), openswarm_consensus::rfp::RfpPhase::Idle) {
                            let _ = rfp.inject_task(&injected_task);
                        }
                        if let Err(e) = rfp.record_commit(&params) {
                            tracing::warn!(error = %e, "Failed to record proposal commit");
                        } else if matches!(rfp.phase(), openswarm_consensus::rfp::RfpPhase::CommitPhase) {
                            let _ = rfp.transition_to_reveal();
                        }
                    }

                    let flush_pending = state
                        .rfp_coordinators
                        .get(&params.task_id)
                        .map(|rfp| matches!(rfp.phase(), openswarm_consensus::rfp::RfpPhase::RevealPhase))
                        .unwrap_or(false);

                    if flush_pending {
                        let mut pending_reveals = state
                            .pending_plan_reveals
                            .remove(&params.task_id)
                            .unwrap_or_default()
                            .into_iter()
                            .collect::<Vec<(String, Plan)>>();
                        pending_reveals.sort_by(|a, b| a.0.cmp(&b.0));
                        let mut revealed_proposers = Vec::new();
                        if let Some(rfp) = state.rfp_coordinators.get_mut(&params.task_id) {
                            for (_, pending_plan) in pending_reveals {
                                let reveal = ProposalRevealParams {
                                    task_id: params.task_id.clone(),
                                    plan: pending_plan,
                                };
                                if let Err(e) = rfp.record_reveal(&reveal) {
                                    tracing::warn!(error = %e, "Failed to record deferred proposal reveal");
                                } else {
                                    revealed_proposers.push(reveal.plan.proposer.to_string());
                                }
                            }
                        }
                        for proposer in revealed_proposers {
                            state.bump_plans_revealed(&proposer);
                        }
                    }

                    let proposal_owners = state
                        .rfp_coordinators
                        .get(&params.task_id)
                        .map(|rfp| {
                            rfp.reveals
                                .values()
                                .map(|r| (r.plan.plan_id.clone(), r.plan.proposer.clone()))
                                .collect::<std::collections::HashMap<String, AgentId>>()
                        })
                        .unwrap_or_default();
                    if !proposal_owners.is_empty() {
                        let voting = state.voting_engines.entry(params.task_id.clone()).or_insert_with(|| {
                            VotingEngine::new(
                                openswarm_consensus::voting::VotingConfig::default(),
                                params.task_id.clone(),
                                params.epoch,
                            )
                        });
                        voting.set_proposals(proposal_owners);
                    }

                    state.push_task_timeline_event(
                        &params.task_id,
                        "proposal_commit",
                        format!("Commit hash {}", params.plan_hash),
                        Some(params.proposer.to_string()),
                    );
                    state.bump_plans_proposed(params.proposer.as_str());
                    state.push_log(
                        LogCategory::Task,
                        format!(
                            "Plan commit for task {} from {} (hash={})",
                            params.task_id,
                            params.proposer,
                            params.plan_hash
                        ),
                    );
                    state.push_log(
                        LogCategory::System,
                        format!(
                            "AUDIT proposal.commit task_id={} proposer={} hash={}",
                            params.task_id, params.proposer, params.plan_hash
                        ),
                    );
                }
            }
            Some(ProtocolMethod::ProposalReveal) => {
                if let Ok(params) =
                    serde_json::from_value::<ProposalRevealParams>(message.params)
                {
                    let mut state = self.state.write().await;
                    // A ProposalReveal is proof of activity — mark proposer as active before
                    // the participation check to avoid KeepAlive propagation race conditions.
                    state.mark_member_seen(params.plan.proposer.as_str());
                    state.mark_member_polled_tasks(params.plan.proposer.as_str());
                    if !Self::is_participating_member_for_task(
                        &state,
                        &params.task_id,
                        params.plan.proposer.as_str(),
                        Duration::from_secs(PARTICIPATION_POLL_STALENESS_SECS),
                    ) {
                        state.push_log(
                            LogCategory::Task,
                            format!(
                                "Ignoring proposal reveal from non-responding proposer {} for task {}",
                                params.plan.proposer, params.task_id
                            ),
                        );
                        return;
                    }
                    state
                        .task_details
                        .entry(params.task_id.clone())
                        .and_modify(|task| {
                            if matches!(task.status, TaskStatus::Pending | TaskStatus::ProposalPhase)
                            {
                                task.status = TaskStatus::VotingPhase;
                            }
                        });

                    let requirement = Self::expected_vote_requirement_for_task(&state, &params.task_id);
                    state
                        .task_vote_requirements
                        .insert(params.task_id.clone(), requirement.clone());

                    let injected_task = state
                        .task_details
                        .get(&params.task_id)
                        .cloned()
                        .unwrap_or(Task {
                            task_id: params.task_id.clone(),
                            parent_task_id: None,
                            epoch: params.plan.epoch,
                            status: TaskStatus::Pending,
                            description: "Observed proposal reveal".to_string(),
                            assigned_to: None,
                            tier_level: requirement.tier_level,
                            subtasks: Vec::new(),
                            created_at: chrono::Utc::now(),
                            deadline: None,
                            ..Default::default()
                        });

                    let should_queue_reveal = {
                        let rfp = state
                            .rfp_coordinators
                            .entry(params.task_id.clone())
                            .or_insert_with(|| {
                                RfpCoordinator::new(
                                    params.task_id.clone(),
                                    params.plan.epoch,
                                    requirement.expected_proposers,
                                )
                            });
                        if matches!(rfp.phase(), openswarm_consensus::rfp::RfpPhase::Idle) {
                            let _ = rfp.inject_task(&injected_task);
                        }
                        if matches!(
                            rfp.phase(),
                            openswarm_consensus::rfp::RfpPhase::RevealPhase
                                | openswarm_consensus::rfp::RfpPhase::ReadyForVoting
                        ) {
                            if let Err(e) = rfp.record_reveal(&params) {
                                tracing::warn!(error = %e, "Failed to record proposal reveal");
                            }
                            false
                        } else {
                            true
                        }
                    };

                    if should_queue_reveal {
                        state
                            .pending_plan_reveals
                            .entry(params.task_id.clone())
                            .or_default()
                            .insert(params.plan.proposer.to_string(), params.plan.clone());
                    }

                    let proposal_owners = state
                        .rfp_coordinators
                        .get(&params.task_id)
                        .map(|rfp| {
                            rfp.reveals
                                .values()
                                .map(|r| (r.plan.plan_id.clone(), r.plan.proposer.clone()))
                                .collect::<std::collections::HashMap<String, AgentId>>()
                        })
                        .unwrap_or_default();

                    let voting = state.voting_engines.entry(params.task_id.clone()).or_insert_with(|| {
                        VotingEngine::new(
                            openswarm_consensus::voting::VotingConfig::default(),
                            params.task_id.clone(),
                            params.plan.epoch,
                        )
                    });
                    voting.set_proposals(proposal_owners);

                    state.push_task_timeline_event(
                        &params.task_id,
                        "proposal_reveal",
                        format!("{} subtasks revealed", params.plan.subtasks.len()),
                        Some(params.plan.proposer.to_string()),
                    );
                    state.bump_plans_revealed(params.plan.proposer.as_str());
                    state.push_log(
                        LogCategory::Task,
                        format!(
                            "Plan reveal for task {} by {} ({} subtasks): {}",
                            params.task_id,
                            params.plan.proposer,
                            params.plan.subtasks.len(),
                            params
                                .plan
                                .subtasks
                                .iter()
                                .map(|s| format!("{}:{}", s.index, s.description))
                                .collect::<Vec<_>>()
                                .join(" | ")
                        ),
                    );
                    state.push_log(
                        LogCategory::System,
                        format!(
                            "AUDIT proposal.reveal task_id={} proposer={} subtasks={}",
                            params.task_id,
                            params.plan.proposer,
                            params.plan.subtasks.len()
                        ),
                    );
                }
            }
            Some(ProtocolMethod::ConsensusVote) => {
                if let Ok(params) =
                    serde_json::from_value::<ConsensusVoteParams>(message.params)
                {
                    let task_id = params.task_id.clone();
                    let voter = params.voter.clone();
                    let rankings_preview = params.rankings.join(" > ");
                    let mut state = self.state.write().await;
                    if !Self::is_participating_member_for_task(
                        &state,
                        &task_id,
                        voter.as_str(),
                        Duration::from_secs(PARTICIPATION_POLL_STALENESS_SECS),
                    ) {
                        state.push_log(
                            LogCategory::Vote,
                            format!(
                                "Ignoring vote from non-responding voter {} for task {}",
                                voter, task_id
                            ),
                        );
                        return;
                    }
                    state.mark_member_seen(voter.as_str());
                    if let Some(task) = state.task_details.get_mut(&task_id) {
                        // Only advance to VotingPhase from pre-voting states.
                        // Never overwrite InProgress/Completed/Failed tasks — a stale
                        // ConsensusVote arriving after TaskAssignment must not revert the task.
                        if matches!(
                            task.status,
                            TaskStatus::Pending | TaskStatus::ProposalPhase | TaskStatus::VotingPhase
                        ) {
                            task.status = TaskStatus::VotingPhase;
                        }
                    }
                    if let Some(voting) = state.voting_engines.get_mut(&task_id) {
                        let ranked_vote = RankedVote {
                            voter: voter.clone(),
                            task_id: params.task_id.clone(),
                            epoch: params.epoch,
                            rankings: params.rankings.clone(),
                            critic_scores: params.critic_scores.clone(),
                        };
                        if let Err(e) = voting.record_vote(ranked_vote) {
                            tracing::warn!(error = %e, "Failed to record consensus vote");
                        }
                    }
                    // Record ballot for deliberation visibility
                    state.ballot_records.entry(task_id.clone()).or_default().push(BallotRecord {
                        task_id: task_id.clone(),
                        voter: voter.clone(),
                        rankings: params.rankings,
                        critic_scores: params.critic_scores,
                        timestamp: chrono::Utc::now(),
                        irv_round_when_eliminated: None,
                    });
                    // Also record as a deliberation message (proposal score phase)
                    {
                        let rankings_str = format!("Rankings: {}", rankings_preview);
                        state.deliberation_messages.entry(task_id.clone()).or_default().push(DeliberationMessage {
                            id: uuid::Uuid::new_v4().to_string(),
                            task_id: task_id.clone(),
                            timestamp: chrono::Utc::now(),
                            speaker: voter.clone(),
                            round: 2,
                            message_type: DeliberationType::CritiqueFeedback,
                            content: rankings_str,
                            referenced_plan_id: None,
                            critic_scores: None,
                        });
                    }
                    state.push_task_timeline_event(
                        &task_id,
                        "vote_recorded",
                        format!("Rankings: {}", rankings_preview),
                        Some(voter.to_string()),
                    );
                    state.bump_votes_cast(voter.as_str());
                    state.push_log(
                        LogCategory::Vote,
                        format!(
                            "Vote for task {} from {}: {}",
                            task_id,
                            voter,
                            rankings_preview
                        ),
                    );
                }
            }
            Some(ProtocolMethod::ResultSubmission) => {
                let raw_params = message.params.clone();
                if let Ok(params) =
                    serde_json::from_value::<ResultSubmissionParams>(raw_params.clone())
                {
                    let mut state = self.state.write().await;
                    if let Some(task) = state.task_details.get(&params.task_id) {
                        let assignee_ok = params.is_synthesis
                            || task.assigned_to.is_none()
                            || task.assigned_to.as_ref() == Some(&params.agent_id);
                        if !assignee_ok {
                            state.push_log(
                                LogCategory::Task,
                                format!(
                                    "Ignoring result for task {} from non-assignee {}",
                                    params.task_id, params.agent_id
                                ),
                            );
                            return;
                        }
                        if task.parent_task_id.is_none() && task.subtasks.is_empty() {
                            state.push_log(
                                LogCategory::Task,
                                format!(
                                    "Rejected direct root result for task {} (no subtasks)",
                                    params.task_id
                                ),
                            );
                            return;
                        }
                    }
                    if let Some(task) = state.task_details.get_mut(&params.task_id) {
                        task.status = TaskStatus::Completed;
                        task.assigned_to = Some(params.agent_id.clone());
                    }
                    state.task_set.remove(&params.task_id);
                    state.mark_member_submitted_result(params.agent_id.as_str());
                    state.bump_tasks_processed(params.agent_id.as_str());
                    state.mark_member_seen(params.agent_id.as_str());
                    // Update holon status to Done on result submission
                    if let Some(holon) = state.active_holons.get_mut(&params.task_id) {
                        holon.status = HolonStatus::Done;
                    }
                    // Populate task_result_text FIRST so deliberation message can read it
                    if let Some(content) = raw_params.get("content").and_then(|v| v.as_str()) {
                        if !content.trim().is_empty() {
                            state
                                .task_result_text
                                .insert(params.task_id.clone(), content.to_string());
                        }
                    }
                    // Record synthesis result as deliberation message
                    if let Some(text) = state.task_result_text.get(&params.task_id).cloned() {
                        if !text.is_empty() {
                            state.deliberation_messages.entry(params.task_id.clone()).or_default().push(DeliberationMessage {
                                id: uuid::Uuid::new_v4().to_string(),
                                task_id: params.task_id.clone(),
                                timestamp: chrono::Utc::now(),
                                speaker: params.agent_id.clone(),
                                round: 3,
                                message_type: DeliberationType::SynthesisResult,
                                content: text,
                                referenced_plan_id: None,
                                critic_scores: None,
                            });
                        }
                    }
                    // Store the artifact content CID as leaf content bytes in the DAG.
                    state.merkle_dag.add_leaf(
                        params.task_id.clone(),
                        params.artifact.content_cid.as_bytes(),
                    );
                    let dag_nodes = state.merkle_dag.node_count();
                    state.push_task_timeline_event(
                        &params.task_id,
                        "result_submitted",
                        format!("Artifact {} (dag_nodes={})", params.artifact.artifact_id, dag_nodes),
                        Some(params.agent_id.to_string()),
                    );
                    state.push_log(
                        LogCategory::Task,
                        format!(
                            "Result received for task {} from {} (artifact={}, dag_nodes={})",
                            params.task_id,
                            params.agent_id,
                            params.artifact.artifact_id,
                            dag_nodes
                        ),
                    );
                    state.push_log(
                        LogCategory::System,
                        format!(
                            "AUDIT result.observe task_id={} agent={} artifact={}",
                            params.task_id, params.agent_id, params.artifact.artifact_id
                        ),
                    );
                }
            }
            Some(ProtocolMethod::Succession) => {
                if let Ok(params) = serde_json::from_value::<SuccessionParams>(message.params) {
                    tracing::info!(
                        failed = %params.failed_leader,
                        new = %params.new_leader,
                        "Succession notification received"
                    );
                }
            }
            Some(ProtocolMethod::SwarmAnnounce) => {
                if let Ok(params) =
                    serde_json::from_value::<SwarmAnnounceParams>(message.params)
                {
                    let mut state = self.state.write().await;
                    let swarm_key = params.swarm_id.as_str().to_string();
                    let is_new = !state.known_swarms.contains_key(&swarm_key);

                    let record = state
                        .known_swarms
                        .entry(swarm_key.clone())
                        .or_insert_with(|| SwarmRecord {
                            swarm_id: params.swarm_id.clone(),
                            name: params.name.clone(),
                            is_public: params.is_public,
                            agent_count: params.agent_count,
                            joined: false,
                            last_seen: chrono::Utc::now(),
                        });

                    record.agent_count = params.agent_count;
                    record.last_seen = chrono::Utc::now();
                    record.name = params.name.clone();

                    if is_new {
                        state.push_log(
                            LogCategory::System,
                            format!(
                                "Discovered swarm: {} ({}, {} agents)",
                                params.name,
                                if params.is_public { "public" } else { "private" },
                                params.agent_count
                            ),
                        );
                        tracing::info!(
                            swarm_id = %params.swarm_id,
                            name = %params.name,
                            public = params.is_public,
                            agents = params.agent_count,
                            "Discovered new swarm"
                        );
                    }
                }
            }
            Some(ProtocolMethod::SwarmJoin) => {
                if let Ok(params) =
                    serde_json::from_value::<SwarmJoinParams>(message.params)
                {
                    let state = self.state.read().await;
                    // Only process join requests for our swarm.
                    if params.swarm_id == state.current_swarm_id {
                        tracing::info!(
                            agent = %params.agent_id,
                            swarm = %params.swarm_id,
                            "Join request for our swarm"
                        );
                    }
                }
            }
            Some(ProtocolMethod::SwarmLeave) => {
                if let Ok(params) =
                    serde_json::from_value::<SwarmLeaveParams>(message.params)
                {
                    let mut state = self.state.write().await;
                    if let Some(record) = state.known_swarms.get_mut(params.swarm_id.as_str()) {
                        record.agent_count = record.agent_count.saturating_sub(1);
                    }
                    state.push_log(
                        LogCategory::Peer,
                        format!("{} left swarm {}", params.agent_id, params.swarm_id),
                    );
                }
            }
            Some(ProtocolMethod::BoardInvite) => {
                if let Ok(params) = serde_json::from_value::<BoardInviteParams>(message.params) {
                    let mut state = self.state.write().await;
                    // Create or update holon in Forming state
                    let holon = state.active_holons.entry(params.task_id.clone()).or_insert_with(|| {
                        HolonState {
                            task_id: params.task_id.clone(),
                            chair: params.chair.clone(),
                            members: Vec::new(),
                            adversarial_critic: None,
                            depth: params.depth,
                            parent_holon: None,
                            child_holons: Vec::new(),
                            subtask_assignments: std::collections::HashMap::new(),
                            status: HolonStatus::Forming,
                            created_at: chrono::Utc::now(),
                        }
                    });
                    holon.status = HolonStatus::Forming;
                    state.push_log(
                        LogCategory::Task,
                        format!(
                            "Board invite for task {} (depth={}, chair={}, complexity={:.2})",
                            params.task_id, params.depth, params.chair, params.complexity_estimate
                        ),
                    );
                }
            }
            Some(ProtocolMethod::BoardAccept) => {
                if let Ok(params) = serde_json::from_value::<BoardAcceptParams>(message.params) {
                    let mut state = self.state.write().await;
                    state.board_acceptances
                        .entry(params.task_id.clone())
                        .or_default()
                        .push(params.clone());
                    if let Some(holon) = state.active_holons.get_mut(&params.task_id) {
                        if !holon.members.iter().any(|m| m == &params.agent_id) {
                            holon.members.push(params.agent_id.clone());
                        }
                    }
                    state.push_log(
                        LogCategory::Task,
                        format!("Board accept: {} for task {}", params.agent_id, params.task_id),
                    );
                }
            }
            Some(ProtocolMethod::BoardDecline) => {
                if let Ok(params) = serde_json::from_value::<BoardDeclineParams>(message.params) {
                    let mut state = self.state.write().await;
                    state.push_log(
                        LogCategory::Task,
                        format!("Board decline: {} for task {}", params.agent_id, params.task_id),
                    );
                }
            }
            Some(ProtocolMethod::BoardReady) => {
                if let Ok(params) = serde_json::from_value::<BoardReadyParams>(message.params) {
                    let mut state = self.state.write().await;
                    let holon = state.active_holons.entry(params.task_id.clone()).or_insert_with(|| {
                        HolonState {
                            task_id: params.task_id.clone(),
                            chair: params.chair_id.clone(),
                            members: params.members.clone(),
                            adversarial_critic: params.adversarial_critic.clone(),
                            depth: 0,
                            parent_holon: None,
                            child_holons: Vec::new(),
                            subtask_assignments: std::collections::HashMap::new(),
                            status: HolonStatus::Deliberating,
                            created_at: chrono::Utc::now(),
                        }
                    });
                    holon.chair = params.chair_id.clone();
                    holon.members = params.members.clone();
                    holon.adversarial_critic = params.adversarial_critic.clone();
                    holon.status = HolonStatus::Deliberating;
                    state.push_log(
                        LogCategory::Task,
                        format!(
                            "Board ready for task {} ({} members, chair={})",
                            params.task_id, params.members.len(), params.chair_id
                        ),
                    );
                }
            }
            Some(ProtocolMethod::BoardDissolve) => {
                if let Ok(params) = serde_json::from_value::<BoardDissolveParams>(message.params) {
                    let mut state = self.state.write().await;
                    if let Some(holon) = state.active_holons.get_mut(&params.task_id) {
                        holon.status = HolonStatus::Done;
                    }
                    state.push_log(
                        LogCategory::Task,
                        format!("Board dissolved for task {}", params.task_id),
                    );
                }
            }
            Some(ProtocolMethod::DiscussionCritique) => {
                if let Ok(params) = serde_json::from_value::<DiscussionCritiqueParams>(message.params) {
                    let mut state = self.state.write().await;
                    // Store as deliberation message
                    let msg = DeliberationMessage {
                        id: uuid::Uuid::new_v4().to_string(),
                        task_id: params.task_id.clone(),
                        timestamp: chrono::Utc::now(),
                        speaker: params.voter_id.clone(),
                        round: params.round,
                        message_type: DeliberationType::CritiqueFeedback,
                        content: params.content.clone(),
                        referenced_plan_id: None,
                        critic_scores: Some(params.plan_scores.clone()),
                    };
                    state.deliberation_messages
                        .entry(params.task_id.clone())
                        .or_default()
                        .push(msg);
                    // Also record in the rfp coordinator
                    if let Some(rfp) = state.rfp_coordinators.get_mut(&params.task_id) {
                        let _ = rfp.record_critique(
                            params.voter_id.clone(),
                            params.plan_scores.clone(),
                            params.content.clone(),
                        );
                    }
                    // Update BallotRecord for this voter with their critic_scores (P2P path)
                    if let Some(ballots) = state.ballot_records.get_mut(&params.task_id) {
                        if let Some(ballot) = ballots.iter_mut().find(|b| b.voter == params.voter_id) {
                            ballot.critic_scores = params.plan_scores.clone();
                        }
                    }
                    // Update holon status to Voting after critique
                    if let Some(holon) = state.active_holons.get_mut(&params.task_id) {
                        if matches!(holon.status, HolonStatus::Deliberating) {
                            holon.status = HolonStatus::Voting;
                        }
                    }
                    state.push_log(
                        LogCategory::Vote,
                        format!(
                            "Critique from {} for task {} (round {}, {} plan scores)",
                            params.voter_id, params.task_id, params.round, params.plan_scores.len()
                        ),
                    );
                }
            }
            Some(ProtocolMethod::DirectMessage) => {
                // A direct message addressed to an agent on the swarm DM topic.
                // Filter: only store if addressed to this connector's agent.
                let to = message.params.get("to").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let from = message.params.get("from").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let content = message.params.get("content").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let mut state = self.state.write().await;
                let my_id = state.agent_id.to_string();
                if to == my_id && !from.is_empty() && !content.is_empty() {
                    state.inbox.push(InboxMessage {
                        from: from.clone(),
                        to: to.clone(),
                        content: content.clone(),
                        timestamp: chrono::Utc::now(),
                    });
                    state.push_log(
                        LogCategory::Message,
                        format!("DM from {}: {}", &from[..from.len().min(20)], &content[..content.len().min(80)]),
                    );
                }
            }
            _ => {
                tracing::debug!(
                    method = %message.method,
                    topic = %topic,
                    "Unhandled protocol message"
                );
            }
        }
    }

    /// Announce this node's swarm to the network via GossipSub.
    ///
    /// Periodically broadcasts a SwarmAnnounce message on the global
    /// swarm discovery topic and the swarm-specific announcement topic.
    /// Also publishes the swarm info to the Kademlia DHT for internet-wide
    /// discovery.
    async fn announce_swarm(&self) {
        let state = self.state.read().await;
        let staleness = Duration::from_secs(self.config.hierarchy.keepalive_interval_secs.saturating_mul(3).max(30));
        let agent_count = state.active_member_count(staleness) as u64;
        let params = SwarmAnnounceParams {
            swarm_id: state.current_swarm_id.clone(),
            name: self.config.swarm.name.clone(),
            is_public: self.config.swarm.is_public,
            agent_id: state.agent_id.clone(),
            agent_count,
            description: String::new(),
            timestamp: chrono::Utc::now(),
        };
        drop(state);

        let msg = SwarmMessage::new(
            ProtocolMethod::SwarmAnnounce.as_str(),
            serde_json::to_value(&params).unwrap_or_default(),
            String::new(),
        );

        if let Ok(data) = serde_json::to_vec(&msg) {
            // Publish to the global discovery topic.
            let discovery_topic = SwarmTopics::swarm_discovery();
            if let Err(e) = self.network_handle.publish(&discovery_topic, data.clone()).await {
                tracing::debug!(error = %e, "Failed to publish swarm announcement to discovery topic");
                let mut state = self.state.write().await;
                state.push_message_trace(MessageTraceEvent {
                    timestamp: chrono::Utc::now(),
                    direction: "outbound".to_string(),
                    peer: None,
                    topic: discovery_topic.clone(),
                    method: Some(ProtocolMethod::SwarmAnnounce.as_str().to_string()),
                    task_id: None,
                    size_bytes: data.len(),
                    outcome: format!("error: {}", e),
                });
            } else {
                let mut state = self.state.write().await;
                state.push_message_trace(MessageTraceEvent {
                    timestamp: chrono::Utc::now(),
                    direction: "outbound".to_string(),
                    peer: None,
                    topic: discovery_topic.clone(),
                    method: Some(ProtocolMethod::SwarmAnnounce.as_str().to_string()),
                    task_id: None,
                    size_bytes: data.len(),
                    outcome: "published".to_string(),
                });
            }

            // Publish to the swarm-specific announcement topic.
            let announce_topic = SwarmTopics::swarm_announce(params.swarm_id.as_str());
            if let Err(e) = self.network_handle.publish(&announce_topic, data).await {
                tracing::debug!(error = %e, "Failed to publish swarm announcement to swarm topic");
                let mut state = self.state.write().await;
                state.push_message_trace(MessageTraceEvent {
                    timestamp: chrono::Utc::now(),
                    direction: "outbound".to_string(),
                    peer: None,
                    topic: announce_topic,
                    method: Some(ProtocolMethod::SwarmAnnounce.as_str().to_string()),
                    task_id: None,
                    size_bytes: 0,
                    outcome: format!("error: {}", e),
                });
            } else {
                let mut state = self.state.write().await;
                state.push_message_trace(MessageTraceEvent {
                    timestamp: chrono::Utc::now(),
                    direction: "outbound".to_string(),
                    peer: None,
                    topic: announce_topic,
                    method: Some(ProtocolMethod::SwarmAnnounce.as_str().to_string()),
                    task_id: None,
                    size_bytes: 0,
                    outcome: "published".to_string(),
                });
            }
        }

        // Also register in DHT for internet-wide discovery.
        let dht_key = format!(
            "{}{}",
            openswarm_protocol::SWARM_REGISTRY_PREFIX,
            params.swarm_id
        );
        let dht_value = serde_json::json!({
            "swarm_id": params.swarm_id.as_str(),
            "name": params.name,
            "is_public": params.is_public,
            "agent_count": params.agent_count,
            "timestamp": params.timestamp.to_rfc3339(),
        });
        if let Ok(value_bytes) = serde_json::to_vec(&dht_value) {
            if let Err(e) = self
                .network_handle
                .put_dht_record(dht_key.into_bytes(), value_bytes)
                .await
            {
                tracing::debug!(error = %e, "Failed to publish swarm info to DHT");
            }
        }
    }

    /// Send a keep-alive message to the swarm.
    async fn send_keepalive(&self) {
        let state = self.state.read().await;
        let swarm_id = state.current_swarm_id.clone();
        let self_id = state.agent_id.to_string();
        // Use the registered agent name if one was set (e.g. "marie-curie"),
        // falling back to the startup --agent-name flag.
        let current_name = state.agent_names.get(&self_id)
            .cloned()
            .unwrap_or_else(|| self.config.agent.name.clone());
        let params = KeepAliveParams {
            agent_id: state.agent_id.clone(),
            agent_name: Some(current_name),
            last_task_poll_at: state.member_last_task_poll.get(&self_id).cloned(),
            last_result_at: state.member_last_result.get(&self_id).cloned(),
            epoch: state.epoch_manager.current_epoch(),
            timestamp: chrono::Utc::now(),
        };
        drop(state);

        let msg = SwarmMessage::new(
            ProtocolMethod::KeepAlive.as_str(),
            serde_json::to_value(&params).unwrap_or_default(),
            String::new(), // Signature would be computed in production.
        );

        if let Ok(data) = serde_json::to_vec(&msg) {
            let topic = SwarmTopics::keepalive_for(swarm_id.as_str());
            if let Err(e) = self.network_handle.publish(&topic, data).await {
                tracing::debug!(error = %e, "Failed to send keepalive");
                let mut state = self.state.write().await;
                state.push_message_trace(MessageTraceEvent {
                    timestamp: chrono::Utc::now(),
                    direction: "outbound".to_string(),
                    peer: None,
                    topic,
                    method: Some(ProtocolMethod::KeepAlive.as_str().to_string()),
                    task_id: None,
                    size_bytes: 0,
                    outcome: format!("error: {}", e),
                });
            } else {
                let mut state = self.state.write().await;
                state.push_message_trace(MessageTraceEvent {
                    timestamp: chrono::Utc::now(),
                    direction: "outbound".to_string(),
                    peer: None,
                    topic,
                    method: Some(ProtocolMethod::KeepAlive.as_str().to_string()),
                    task_id: None,
                    size_bytes: 0,
                    outcome: "published".to_string(),
                });
            }
        }
    }

    /// Check for epoch transitions and trigger elections if needed.
    async fn check_epoch_transition(&self) {
        let swarm_size = self
            .network_handle
            .estimated_swarm_size()
            .await
            .unwrap_or(1);

        let mut state = self.state.write().await;
        let stale_ttl = Duration::from_secs(self.config.hierarchy.keepalive_interval_secs.saturating_mul(3).max(30));
        state.prune_stale_members(stale_ttl);
        state.network_stats.total_agents = state.active_member_count(stale_ttl) as u64;
        if let Some(action) = state.epoch_manager.tick(swarm_size) {
            match action {
                openswarm_hierarchy::epoch::EpochAction::TriggerElection {
                    new_epoch,
                    estimated_swarm_size,
                } => {
                    tracing::info!(
                        new_epoch,
                        swarm_size = estimated_swarm_size,
                        "Triggering new epoch election"
                    );
                    // Recompute pyramid layout.
                    if let Ok(layout) = state.pyramid.compute_layout(estimated_swarm_size) {
                        state.network_stats.hierarchy_depth = layout.depth;
                    }
                    // Initialize election for new epoch.
                    let election_config = openswarm_hierarchy::elections::ElectionConfig::default();
                    state.election = Some(ElectionManager::new(election_config, new_epoch));
                    state.status = ConnectorStatus::InElection;
                    state.push_log(
                        LogCategory::Epoch,
                        format!("Epoch {} election triggered (swarm size: {})", new_epoch, estimated_swarm_size),
                    );
                }
                openswarm_hierarchy::epoch::EpochAction::FinalizeTransition { epoch } => {
                    tracing::info!(epoch, "Finalizing epoch transition");
                    // In production, this would tally votes and advance the epoch.
                    state.status = ConnectorStatus::Running;
                    state.push_log(
                        LogCategory::Epoch,
                        format!("Epoch {} transition finalized", epoch),
                    );
                }
            }
        }
    }

    /// Check if any voting engines have reached quorum and run IRV.
    async fn check_voting_completion(&self) {
        let mut state = self.state.write().await;
        let mut completed_votes = Vec::new();
        let mut assignments_to_run: Vec<(String, String)> = Vec::new();

        // Collect voting results first (to avoid borrow issues)
        let mut results_to_process = Vec::new();

        let task_ids: Vec<String> = state.voting_engines.keys().cloned().collect();
        let mut pending_logs: Vec<String> = Vec::new();

        for task_id in task_ids {
            let mut single_proposal_id: Option<String> = None;
            if let Some(proposal_owners) = state.rfp_coordinators.get(&task_id).map(|rfp| {
                rfp.reveals
                    .values()
                    .map(|r| (r.plan.plan_id.clone(), r.plan.proposer.clone()))
                    .collect::<std::collections::HashMap<String, AgentId>>()
            }) {
                if proposal_owners.len() == 1 {
                    single_proposal_id = proposal_owners.keys().next().cloned();
                }
                if !proposal_owners.is_empty() {
                    if let Some(v) = state.voting_engines.get_mut(&task_id) {
                        v.set_proposals(proposal_owners);
                    }
                }
            }

            let requirement = Self::expected_vote_requirement_for_task(&state, &task_id);
            state
                .task_vote_requirements
                .insert(task_id.clone(), requirement.clone());

            let (ballot_count, proposal_count) = match state.voting_engines.get(&task_id) {
                Some(v) => (v.ballot_count(), v.proposal_count()),
                None => continue,
            };
            let mut expected_votes = requirement.expected_voters.max(1);
            let mut expected_proposals = requirement.expected_proposers.max(1);

            if let Some(task) = state.task_details.get(&task_id) {
                let age_secs = chrono::Utc::now()
                    .signed_duration_since(task.created_at)
                    .num_seconds();
                if age_secs >= PROPOSAL_STAGE_TIMEOUT_SECS {
                    // Force-advance RFP from CommitPhase if P2P commits didn't arrive in time.
                    // This ensures the local proposal can proceed to voting even without full quorum.
                    let pending_reveals = state.pending_plan_reveals.remove(&task_id).unwrap_or_default();
                    if let Some(rfp) = state.rfp_coordinators.get_mut(&task_id) {
                        if matches!(rfp.phase(), openswarm_consensus::rfp::RfpPhase::CommitPhase) {
                            if rfp.commit_count() > 0 {
                                let _ = rfp.transition_to_reveal();
                                // Flush any pending reveals that arrived while in CommitPhase
                                let mut pending_sorted: Vec<(String, openswarm_protocol::Plan)> =
                                    pending_reveals.into_iter().collect();
                                pending_sorted.sort_by(|a, b| a.0.cmp(&b.0));
                                for (_, plan) in pending_sorted {
                                    let reveal = openswarm_protocol::messages::ProposalRevealParams {
                                        task_id: task_id.clone(),
                                        plan,
                                    };
                                    let _ = rfp.record_reveal(&reveal);
                                }
                            }
                        }
                    }
                    // Re-sync voting engine with proposals now that reveals may have been processed
                    if let Some(proposal_owners) = state.rfp_coordinators.get(&task_id).map(|rfp| {
                        rfp.reveals
                            .values()
                            .map(|r| (r.plan.plan_id.clone(), r.plan.proposer.clone()))
                            .collect::<std::collections::HashMap<String, AgentId>>()
                    }) {
                        if !proposal_owners.is_empty() {
                            if let Some(v) = state.voting_engines.get_mut(&task_id) {
                                v.set_proposals(proposal_owners);
                            }
                        }
                    }
                    // Recalculate proposal_count after potential forced reveal
                    let proposal_count_now = state.voting_engines.get(&task_id).map(|v| v.proposal_count()).unwrap_or(0);
                    expected_proposals = expected_proposals.min(proposal_count_now.max(1));
                }
                if age_secs >= VOTING_STAGE_TIMEOUT_SECS {
                    expected_votes = expected_votes.min(ballot_count.max(1));
                }
            }

            // Recalculate proposal_count (may have changed due to forced reveal above)
            let (ballot_count, proposal_count) = match state.voting_engines.get(&task_id) {
                Some(v) => (v.ballot_count(), v.proposal_count()),
                None => continue,
            };

            // Strict participation gate: all expected tier members must propose and vote.
            let task_age_secs = state
                .task_details
                .get(&task_id)
                .map(|task| chrono::Utc::now().signed_duration_since(task.created_at).num_seconds())
                .unwrap_or(0);

            if ballot_count == 0
                && proposal_count == 1
                && task_age_secs >= VOTING_STAGE_TIMEOUT_SECS
            {
                if let Some(winner) = single_proposal_id {
                    state.push_log(
                        LogCategory::Vote,
                        format!(
                            "Voting timeout for task {}; selecting sole proposal {}",
                            task_id, winner
                        ),
                    );
                    state.push_task_timeline_event(
                        &task_id,
                        "plan_selected",
                        format!("Sole proposal {} selected after voting timeout", winner),
                        None,
                    );
                    if let Some(task) = state.task_details.get_mut(&task_id) {
                        task.status = TaskStatus::InProgress;
                    }
                    assignments_to_run.push((task_id.clone(), winner));
                    completed_votes.push(task_id.clone());
                    continue;
                }
            }

            if proposal_count >= expected_proposals && ballot_count >= expected_votes {
                tracing::info!(
                    task_id = %task_id,
                    ballot_count,
                    proposal_count,
                    expected_votes,
                    expected_proposals,
                    "Voting quorum reached, running IRV"
                );

                // Run Instant Runoff Voting to select winner
                let irv_result = {
                    let voting_engine = match state.voting_engines.get_mut(&task_id) {
                        Some(v) => v,
                        None => continue,
                    };
                    let result = voting_engine.run_irv();
                    // Persist IRV rounds for API visibility
                    let rounds = voting_engine.irv_rounds().to_vec();
                    (result, rounds)
                };

                let (irv_result, irv_rounds) = irv_result;
                // Persist IRV rounds to state
                if !irv_rounds.is_empty() {
                    state.irv_rounds.insert(task_id.clone(), irv_rounds);
                }

                match irv_result {
                    Ok(result) => {
                        results_to_process.push((task_id.clone(), Ok(result)));
                        completed_votes.push(task_id.clone());
                    }
                    Err(e) => {
                        results_to_process.push((task_id.clone(), Err(e)));
                    }
                }
            } else {
                pending_logs.push(format!(
                    "Voting pending for task {}: proposals {}/{} votes {}/{}",
                    task_id, proposal_count, expected_proposals, ballot_count, expected_votes
                ));
            }
        }

        for msg in pending_logs {
            state.push_log(LogCategory::Vote, msg);
        }

        // Process results (now we can mutably borrow state again)
        for (task_id, result) in results_to_process {
            match result {
                Ok(voting_result) => {
                    state.push_log(
                        LogCategory::Vote,
                        format!(
                            "Voting complete for task {}: winner = {} ({} rounds, {} votes)",
                            task_id, voting_result.winner, voting_result.rounds, voting_result.total_votes
                        ),
                    );

                    state.push_task_timeline_event(
                        &task_id,
                        "plan_selected",
                        format!("Plan {} selected by IRV after {} rounds", voting_result.winner, voting_result.rounds),
                        None,
                    );

                    // Update task status to InProgress
                    if let Some(task) = state.task_details.get_mut(&task_id) {
                        task.status = TaskStatus::InProgress;
                    }

                    // Update holon status to Executing
                    if let Some(holon) = state.active_holons.get_mut(&task_id) {
                        holon.status = HolonStatus::Executing;
                    }

                    tracing::info!(
                        task_id = %task_id,
                        winner = %voting_result.winner,
                        rounds = voting_result.rounds,
                        "Voting completed successfully"
                    );

                    assignments_to_run.push((task_id.clone(), voting_result.winner.clone()));
                }
                Err(e) => {
                    tracing::error!(
                        error = %e,
                        task_id = %task_id,
                        "IRV execution failed"
                    );
                    state.push_log(
                        LogCategory::Vote,
                        format!("Voting failed for task {}: {}", task_id, e),
                    );
                }
            }
        }

        // Remove completed voting engines
        for task_id in completed_votes {
            state.voting_engines.remove(&task_id);
            state.task_vote_requirements.remove(&task_id);
        }

        drop(state);

        for (task_id, winner_plan_id) in assignments_to_run {
            if let Err(e) = self.assign_subtasks_from_winner(&task_id, &winner_plan_id).await {
                tracing::error!(
                    task_id = %task_id,
                    winner = %winner_plan_id,
                    error = %e,
                    "Failed to assign subtasks after voting"
                );
            }
        }
    }

    async fn check_execution_timeouts(&self) {
        let now = chrono::Utc::now();
        let mut publishes: Vec<(String, Vec<u8>, String)> = Vec::new();

        {
            let mut state = self.state.write().await;
            let my_id = state.agent_id.to_string();
            let swarm_id = state.current_swarm_id.as_str().to_string();
            let poll_staleness = Duration::from_secs(PARTICIPATION_POLL_STALENESS_SECS);
            let seen_staleness = Duration::from_secs(ACTIVE_MEMBER_STALENESS_SECS);
            let active_members: std::collections::HashSet<String> =
                state.active_member_ids(seen_staleness).into_iter().collect();

            let timed_out_tasks: Vec<String> = state
                .task_details
                .iter()
                .filter_map(|(task_id, task)| {
                    if !matches!(task.status, TaskStatus::InProgress) {
                        return None;
                    }
                    if task.parent_task_id.is_none() {
                        return None;
                    }
                    match task.deadline {
                        Some(deadline) if deadline <= now => Some(task_id.clone()),
                        _ => None,
                    }
                })
                .collect();

            for task_id in timed_out_tasks {
                let Some(task_snapshot) = state.task_details.get(&task_id).cloned() else {
                    continue;
                };
                let Some(parent_id) = task_snapshot.parent_task_id.clone() else {
                    continue;
                };
                let old_assignee = task_snapshot.assigned_to.clone();
                let my_subordinates = state
                    .subordinates
                    .get(&my_id)
                    .cloned()
                    .unwrap_or_default();
                if !my_subordinates
                    .iter()
                    .any(|id| Some(id.as_str()) == old_assignee.as_ref().map(|a| a.as_str()))
                {
                    continue;
                }
                let expected_tier = old_assignee
                    .as_ref()
                    .and_then(|a| state.agent_tiers.get(a.as_str()).copied());

                let mut candidates = my_subordinates
                    .into_iter()
                    .filter(|candidate| {
                        if Some(candidate.as_str()) == old_assignee.as_ref().map(|a| a.as_str()) {
                            return false;
                        }
                        if !active_members.contains(candidate) {
                            return false;
                        }
                        if !Self::member_loop_active(&state, candidate, poll_staleness) {
                            return false;
                        }
                        if let Some(tier) = expected_tier {
                            return state.agent_tiers.get(candidate).copied().unwrap_or(Tier::Executor)
                                == tier;
                        }
                        true
                    })
                    .collect::<Vec<_>>();

                candidates.sort();
                let Some(new_assignee) = candidates.into_iter().next() else {
                    continue;
                };

                if let Some(task) = state.task_details.get_mut(&task_id) {
                    task.assigned_to = Some(AgentId::new(new_assignee.clone()));
                    task.status = TaskStatus::InProgress;
                    task.deadline = Some(now + chrono::Duration::seconds(EXECUTION_ASSIGNMENT_TIMEOUT_SECS));
                }
                state.bump_tasks_assigned(&new_assignee);
                state.push_task_timeline_event(
                    &task_id,
                    "reassigned",
                    format!(
                        "Task reassigned due to timeout: {} -> {}",
                        old_assignee
                            .as_ref()
                            .map(|a| a.as_str())
                            .unwrap_or("unassigned"),
                        new_assignee
                    ),
                    Some(my_id.clone()),
                );
                state.push_log(
                    LogCategory::Task,
                    format!(
                        "Task {} reassigned due to non-response: {} -> {}",
                        task_id,
                        old_assignee
                            .as_ref()
                            .map(|a| a.as_str())
                            .unwrap_or("unassigned"),
                        new_assignee
                    ),
                );

                let Some(mut reassigned_task) = state.task_details.get(&task_id).cloned() else {
                    continue;
                };
                reassigned_task.assigned_to = Some(AgentId::new(new_assignee.clone()));

                let assign_params = TaskAssignmentParams {
                    task: reassigned_task,
                    assignee: AgentId::new(new_assignee),
                    parent_task_id: parent_id,
                    winning_plan_id: "reassign-timeout".to_string(),
                };
                let assign_msg = SwarmMessage::new(
                    ProtocolMethod::TaskAssignment.as_str(),
                    serde_json::to_value(&assign_params).unwrap_or_default(),
                    String::new(),
                );
                if let Ok(data) = serde_json::to_vec(&assign_msg) {
                    let topic = SwarmTopics::tasks_for(swarm_id.as_str(), task_snapshot.tier_level);
                    publishes.push((topic, data, task_id.clone()));
                }
            }
        }

        for (topic, data, task_id) in publishes {
            if let Err(e) = self.network_handle.publish(&topic, data).await {
                tracing::error!(task_id = %task_id, topic = %topic, error = %e, "Failed to publish reassignment");
            }
        }
    }

    /// Assign subtasks from the winning plan to subordinate agents.
    async fn assign_subtasks_from_winner(
        &self,
        task_id: &str,
        winner_plan_id: &str,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut state = self.state.write().await;

        // Get the winning plan from RFP coordinator
        let winning_plan = {
            let rfp = state.rfp_coordinators.get(task_id)
                .ok_or("RFP coordinator not found")?;

            // Get the revealed proposal matching the winner
            let revealed_proposal = rfp.reveals
                .values()
                .find(|p| p.plan.plan_id == winner_plan_id)
                .ok_or("Winning plan not found in reveals")?;

            revealed_proposal.plan.clone()
        };

        // Get my subordinates for assignment
        let raw_subordinates: Vec<AgentId> = state.subordinates
            .get(state.agent_id.as_str())
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(AgentId::new)
            .collect();

        // In single-node mode (swarm_size==1), self-assign so the sole connector executes.
        // In multi-node mode, Tier1 nodes without subordinates are NOT the designated
        // coordinator — skip assignment to avoid duplicate assignments from multiple Tier1s.
        let swarm_size = state.active_member_count(
            std::time::Duration::from_secs(ACTIVE_MEMBER_STALENESS_SECS)
        ) as usize;
        let subordinates: Vec<AgentId> = if raw_subordinates.is_empty() {
            if swarm_size <= 1 {
                tracing::info!(
                    task_id = %task_id,
                    agent_id = %state.agent_id,
                    "No subordinates (single-node mode): self-assigning subtasks"
                );
                vec![state.agent_id.clone()]
            } else {
                tracing::info!(
                    task_id = %task_id,
                    agent_id = %state.agent_id,
                    swarm_size,
                    "No subordinates in multi-node swarm: skipping assignment (not the designated coordinator)"
                );
                return Ok(());
            }
        } else {
            raw_subordinates
        };

        // Idempotency: if subtasks already exist for this task, another coordinator already
        // assigned them. Skip to avoid competing assignments from multiple Tier1 nodes.
        let first_subtask_id = format!("{}-st-1", task_id);
        if state.task_details.contains_key(&first_subtask_id) {
            tracing::info!(
                task_id = %task_id,
                agent_id = %state.agent_id,
                "Subtasks already assigned by another coordinator — skipping duplicate assignment"
            );
            return Ok(());
        }

        let parent_tier = state.task_details
            .get(task_id)
            .map(|t| t.tier_level)
            .unwrap_or(1);

        let swarm_id = state.current_swarm_id.clone();
        let mut subtask_ids = Vec::new();
        let mut assignment_messages = Vec::new();

        const COMPLEXITY_RECURSE_THRESHOLD: f64 = 0.4;

        // Create subtasks and assignment messages
        for (idx, subtask_spec) in winning_plan.subtasks.iter().enumerate() {
            let subtask_id = format!("{}-st-{}", task_id, idx + 1);
            let is_complex = subtask_spec.estimated_complexity > COMPLEXITY_RECURSE_THRESHOLD;

            if is_complex {
                // High-complexity subtask: spawn a sub-holon via TaskInjection so any
                // available coordinator forms a new deliberation board for it.
                let subtask = Task {
                    task_id: subtask_id.clone(),
                    parent_task_id: Some(task_id.to_string()),
                    epoch: winning_plan.epoch,
                    status: TaskStatus::Pending,
                    description: subtask_spec.description.clone(),
                    assigned_to: None,
                    // Keep at parent tier so coordinator-tier agents pick it up
                    tier_level: parent_tier,
                    subtasks: Vec::new(),
                    created_at: chrono::Utc::now(),
                    deadline: None,
                    capabilities_required: subtask_spec.required_capabilities.clone(),
                    ..Default::default()
                };

                state.task_details.insert(subtask_id.clone(), subtask.clone());
                subtask_ids.push(subtask_id.clone());

                // Create a child HolonState for this sub-holon
                let my_agent_id = state.agent_id.clone();
                state.active_holons.entry(subtask_id.clone()).or_insert_with(|| HolonState {
                    task_id: subtask_id.clone(),
                    chair: my_agent_id,
                    members: Vec::new(),
                    adversarial_critic: None,
                    depth: parent_tier + 1,
                    parent_holon: Some(task_id.to_string()),
                    child_holons: Vec::new(),
                    subtask_assignments: std::collections::HashMap::new(),
                    status: HolonStatus::Forming,
                    created_at: chrono::Utc::now(),
                });
                // Register sub-holon in parent's child_holons list
                if let Some(parent_holon) = state.active_holons.get_mut(task_id) {
                    if !parent_holon.child_holons.contains(&subtask_id) {
                        parent_holon.child_holons.push(subtask_id.clone());
                    }
                }

                state.push_task_timeline_event(
                    task_id,
                    "sub_holon_forming",
                    format!(
                        "Sub-holon forming for complex subtask {} (complexity={:.2})",
                        subtask_id, subtask_spec.estimated_complexity
                    ),
                    None,
                );

                let inject_params = TaskInjectionParams {
                    task: subtask,
                    originator: state.agent_id.clone(),
                };
                let inject_msg = SwarmMessage::new(
                    ProtocolMethod::TaskInjection.as_str(),
                    serde_json::to_value(&inject_params).unwrap_or_default(),
                    String::new(),
                );
                if let Ok(data) = serde_json::to_vec(&inject_msg) {
                    let topic = SwarmTopics::tasks_for(swarm_id.as_str(), parent_tier);
                    assignment_messages.push((topic, data));
                }

                tracing::info!(
                    task_id = %task_id,
                    subtask_id = %subtask_id,
                    complexity = subtask_spec.estimated_complexity,
                    "Complex subtask spawning sub-holon via TaskInjection"
                );
            } else {
                // Low-complexity subtask: direct assignment to a subordinate executor
                let assignee = subordinates[idx % subordinates.len()].clone();

                let subtask = Task {
                    task_id: subtask_id.clone(),
                    parent_task_id: Some(task_id.to_string()),
                    epoch: winning_plan.epoch,
                    status: TaskStatus::InProgress,
                    description: subtask_spec.description.clone(),
                    assigned_to: Some(assignee.clone()),
                    tier_level: (parent_tier + 1).min(openswarm_protocol::MAX_HIERARCHY_DEPTH),
                    subtasks: Vec::new(),
                    created_at: chrono::Utc::now(),
                    deadline: Some(
                        chrono::Utc::now()
                            + chrono::Duration::seconds(EXECUTION_ASSIGNMENT_TIMEOUT_SECS),
                    ),
                    capabilities_required: subtask_spec.required_capabilities.clone(),
                    ..Default::default()
                };

                state.task_details.insert(subtask_id.clone(), subtask.clone());
                state.bump_tasks_assigned(assignee.as_str());
                subtask_ids.push(subtask_id.clone());

                state.push_task_timeline_event(
                    task_id,
                    "subtask_assigned",
                    format!("Subtask {} assigned to {}", subtask_id, assignee),
                    Some(assignee.to_string()),
                );

                let assign_params = TaskAssignmentParams {
                    task: subtask,
                    assignee: assignee.clone(),
                    parent_task_id: task_id.to_string(),
                    winning_plan_id: winner_plan_id.to_string(),
                };
                let assign_msg = SwarmMessage::new(
                    ProtocolMethod::TaskAssignment.as_str(),
                    serde_json::to_value(&assign_params).unwrap_or_default(),
                    String::new(),
                );
                if let Ok(data) = serde_json::to_vec(&assign_msg) {
                    let topic =
                        SwarmTopics::tasks_for(swarm_id.as_str(), assign_params.task.tier_level);
                    assignment_messages.push((topic, data));
                }

                tracing::info!(
                    task_id = %task_id,
                    subtask_id = %subtask_id,
                    assignee = %assignee,
                    "Subtask assigned to subordinate"
                );
            }
        }

        // Update parent task with subtask IDs
        if let Some(parent_task) = state.task_details.get_mut(task_id) {
            parent_task.subtasks = subtask_ids.clone();
        }

        state.push_log(
            LogCategory::Task,
            format!(
                "Assigned {} subtasks from winning plan {} to {} subordinates",
                winning_plan.subtasks.len(),
                winner_plan_id,
                subordinates.len()
            ),
        );

        // Drop the write lock before publishing
        drop(state);

        // Subscribe coordinator to each subtask's result topic so we receive completion updates
        for st_id in &subtask_ids {
            self.subscribe_task_flow_topics(swarm_id.as_str(), st_id).await;
        }

        // Publish all assignment messages
        for (topic, data) in assignment_messages {
            if let Err(e) = self.network_handle.publish(&topic, data).await {
                tracing::error!(
                    topic = %topic,
                    error = %e,
                    "Failed to publish task assignment"
                );
                let mut state = self.state.write().await;
                state.push_message_trace(MessageTraceEvent {
                    timestamp: chrono::Utc::now(),
                    direction: "outbound".to_string(),
                    peer: None,
                    topic,
                    method: Some(ProtocolMethod::TaskAssignment.as_str().to_string()),
                    task_id: Some(task_id.to_string()),
                    size_bytes: 0,
                    outcome: format!("error: {}", e),
                });
            } else {
                let mut state = self.state.write().await;
                state.push_message_trace(MessageTraceEvent {
                    timestamp: chrono::Utc::now(),
                    direction: "outbound".to_string(),
                    peer: None,
                    topic,
                    method: Some(ProtocolMethod::TaskAssignment.as_str().to_string()),
                    task_id: Some(task_id.to_string()),
                    size_bytes: 0,
                    outcome: "published".to_string(),
                });
            }
        }

        Ok(())
    }

    /// Parse bootstrap peer multiaddresses (e.g. "/ip4/1.2.3.4/tcp/9000/p2p/12D3...")
    /// into (PeerId, Multiaddr) pairs for the discovery layer.
    fn parse_bootstrap_peers(addrs: &[String]) -> Vec<(PeerId, Multiaddr)> {
        let mut peers = Vec::new();
        for addr_str in addrs {
            let addr_str = addr_str.trim();
            if addr_str.is_empty() {
                continue;
            }
            match addr_str.parse::<Multiaddr>() {
                Ok(addr) => {
                    // Extract the PeerId from the /p2p/<peer_id> component of the multiaddr string.
                    if let Some(peer_id) = Self::extract_peer_id_from_addr(addr_str) {
                        peers.push((peer_id, addr));
                    } else {
                        tracing::warn!(
                            addr = %addr_str,
                            "Bootstrap address missing /p2p/<peer_id> component, skipping"
                        );
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        addr = %addr_str,
                        error = %e,
                        "Failed to parse bootstrap multiaddress, skipping"
                    );
                }
            }
        }
        peers
    }

    /// Extract a PeerId from a multiaddress string by finding the /p2p/<id> segment.
    fn extract_peer_id_from_addr(addr: &str) -> Option<PeerId> {
        let parts: Vec<&str> = addr.split('/').collect();
        // Find the "p2p" component and take the next element as the peer ID.
        for (i, part) in parts.iter().enumerate() {
            if *part == "p2p" {
                if let Some(id_str) = parts.get(i + 1) {
                    return id_str.parse::<PeerId>().ok();
                }
            }
        }
        None
    }

    /// Dial bootstrap peers to establish connections immediately on startup.
    async fn connect_to_bootstrap_peers(&self) {
        for addr_str in &self.config.network.bootstrap_peers {
            let addr_str = addr_str.trim();
            if addr_str.is_empty() {
                continue;
            }
            if let Ok(addr) = addr_str.parse::<Multiaddr>() {
                match self.network_handle.dial(addr.clone()).await {
                    Ok(()) => {
                        tracing::info!(addr = %addr, "Dialing bootstrap peer");
                        let mut state = self.state.write().await;
                        state.push_log(
                            LogCategory::System,
                            format!("Dialing bootstrap peer: {}", addr),
                        );
                    }
                    Err(e) => {
                        tracing::warn!(addr = %addr, error = %e, "Failed to dial bootstrap peer");
                    }
                }
            }
        }
    }

    async fn subscribe_task_assignment_topics(&self, swarm_id: &str) {
        for tier in 1..=openswarm_protocol::MAX_HIERARCHY_DEPTH {
            let topic = SwarmTopics::tasks_for(swarm_id, tier);
            if let Err(e) = self.network_handle.subscribe(&topic).await {
                tracing::debug!(error = %e, topic = %topic, "Failed to subscribe task assignment topic");
            }
        }
    }

    async fn subscribe_task_flow_topics(&self, swarm_id: &str, task_id: &str) {
        let proposals_topic = SwarmTopics::proposals_for(swarm_id, task_id);
        let voting_topic = SwarmTopics::voting_for(swarm_id, task_id);
        let results_topic = SwarmTopics::results_for(swarm_id, task_id);

        if let Err(e) = self.network_handle.subscribe(&proposals_topic).await {
            tracing::debug!(error = %e, topic = %proposals_topic, "Failed to subscribe proposals topic");
        }
        if let Err(e) = self.network_handle.subscribe(&voting_topic).await {
            tracing::debug!(error = %e, topic = %voting_topic, "Failed to subscribe voting topic");
        }
        if let Err(e) = self.network_handle.subscribe(&results_topic).await {
            tracing::debug!(error = %e, topic = %results_topic, "Failed to subscribe results topic");
        }
    }

    fn recompute_hierarchy_from_members(state: &mut ConnectorState, members: &[String]) {
        if members.is_empty() {
            return;
        }

        let swarm_size = members.len() as u64;
        let mut sorted_agents = members.to_vec();
        sorted_agents.sort();

        state.agent_tiers.clear();
        state.agent_parents.clear();
        state.subordinates.clear();

        let k = Self::dynamic_branching_factor(swarm_size) as usize;
        let distribution = openswarm_hierarchy::PyramidAllocator::distribute(swarm_size, k as u64);
        let tier_sizes: Vec<usize> = distribution.tiers.iter().map(|n| *n as usize).collect();
        let levels = tier_sizes.len().max(1);

        let mut offsets = Vec::with_capacity(levels + 1);
        offsets.push(0usize);
        for size in &tier_sizes {
            let prev = *offsets.last().unwrap_or(&0);
            offsets.push(prev + *size);
        }

        for level in 0..levels {
            let start = offsets[level];
            let end = *offsets.get(level + 1).unwrap_or(&start);
            for idx in start..end.min(sorted_agents.len()) {
                let member_id = sorted_agents[idx].clone();
                let tier = if levels == 1 {
                    Tier::Executor
                } else if level == (levels - 1) {
                    Tier::Executor
                } else if level == 0 {
                    Tier::Tier1
                } else if level == 1 {
                    Tier::Tier2
                } else {
                    Tier::TierN((level + 1) as u32)
                };
                state.agent_tiers.insert(member_id, tier);
            }
        }

        for level in 1..levels {
            let child_start = offsets[level];
            let child_end = *offsets.get(level + 1).unwrap_or(&child_start);
            let parent_start = offsets[level - 1];
            let parent_end = *offsets.get(level).unwrap_or(&parent_start);
            let parent_count = parent_end.saturating_sub(parent_start);

            if parent_count == 0 {
                continue;
            }

            for child_idx in child_start..child_end.min(sorted_agents.len()) {
                let local_child_idx = child_idx.saturating_sub(child_start);
                let mut parent_local_idx = local_child_idx / k.max(1);
                if parent_local_idx >= parent_count {
                    parent_local_idx = parent_count - 1;
                }
                let parent_idx = parent_start + parent_local_idx;
                if let (Some(child_id), Some(parent_id)) =
                    (sorted_agents.get(child_idx), sorted_agents.get(parent_idx))
                {
                    state
                        .agent_parents
                        .insert(child_id.clone(), parent_id.clone());
                    state
                        .subordinates
                        .entry(parent_id.clone())
                        .or_default()
                        .push(child_id.clone());
                }
            }
        }

        state.network_stats.hierarchy_depth = levels as u32;
        state.network_stats.total_agents = swarm_size;
        state.current_layout = openswarm_hierarchy::PyramidAllocator::new(PyramidConfig {
            branching_factor: k as u32,
            max_depth: openswarm_protocol::MAX_HIERARCHY_DEPTH,
        })
        .compute_layout(swarm_size)
        .ok();

        let my_id = state.agent_id.as_str().to_string();
        if let Some(my_tier) = state.agent_tiers.get(&my_id).copied() {
            state.my_tier = my_tier;
            state.network_stats.my_tier = my_tier;
            state.parent_id = state.agent_parents.get(&my_id).cloned().map(AgentId::new);
            state.network_stats.parent_id = state.parent_id.clone();
            state.network_stats.subordinate_count = state
                .subordinates
                .get(&my_id)
                .map(|s| s.len() as u32)
                .unwrap_or(0);
        }
    }

    fn tier_to_level(tier: Tier) -> Option<u32> {
        match tier {
            Tier::Tier1 => Some(1),
            Tier::Tier2 => Some(2),
            Tier::TierN(n) => Some(n),
            Tier::Executor => None,
        }
    }

    fn level_to_tier(level: u32) -> Tier {
        match level {
            1 => Tier::Tier1,
            2 => Tier::Tier2,
            n => Tier::TierN(n),
        }
    }

    fn member_loop_active(state: &ConnectorState, agent_id: &str, max_staleness: Duration) -> bool {
        let now = chrono::Utc::now();
        // Primary: agent called receive_task recently
        let polled = state
            .member_last_task_poll
            .get(agent_id)
            .and_then(|ts| now.signed_duration_since(*ts).to_std().ok())
            .map(|age| age <= max_staleness)
            .unwrap_or(false);
        if polled {
            return true;
        }
        // Fallback: agent was seen via P2P (proposal, keepalive, vote, etc.)
        state
            .member_last_seen
            .get(agent_id)
            .and_then(|ts| now.signed_duration_since(*ts).to_std().ok())
            .map(|age| age <= max_staleness)
            .unwrap_or(false)
    }

    fn active_participating_members_in_tier(
        state: &ConnectorState,
        tier: Tier,
        member_staleness: Duration,
        poll_staleness: Duration,
    ) -> Vec<String> {
        state
            .active_member_ids(member_staleness)
            .into_iter()
            .filter(|id| state.agent_tiers.get(id).copied().unwrap_or(Tier::Executor) == tier)
            .filter(|id| Self::member_loop_active(state, id, poll_staleness))
            .collect()
    }

    fn is_participating_member_for_task(
        state: &ConnectorState,
        task_id: &str,
        agent_id: &str,
        poll_staleness: Duration,
    ) -> bool {
        let tier_level = state
            .task_details
            .get(task_id)
            .map(|t| t.tier_level)
            .or_else(|| state.task_vote_requirements.get(task_id).map(|r| r.tier_level))
            .unwrap_or(1);
        let expected_tier = Self::level_to_tier(tier_level);
        let agent_tier = state.agent_tiers.get(agent_id).copied().unwrap_or(Tier::Executor);
        // Tier check: match expected tier, OR fall back to any agent when no
        // agents of the expected tier exist (e.g. small swarm where everyone is Executor).
        let tier_ok = agent_tier == expected_tier
            || !state.agent_tiers.values().any(|t| *t == expected_tier);
        tier_ok && Self::member_loop_active(state, agent_id, poll_staleness)
    }

    fn expected_vote_requirement_for_task(state: &ConnectorState, task_id: &str) -> TaskVoteRequirement {
        let tier_level = state
            .task_details
            .get(task_id)
            .map(|t| t.tier_level)
            .or_else(|| state.task_vote_requirements.get(task_id).map(|r| r.tier_level))
            .unwrap_or(1);
        let tier = Self::level_to_tier(tier_level);
        let in_tier = Self::active_participating_members_in_tier(
            state,
            tier,
            Duration::from_secs(ACTIVE_MEMBER_STALENESS_SECS),
            Duration::from_secs(PARTICIPATION_POLL_STALENESS_SECS),
        )
            .len();
        // Fall back to all active agents when no agents of the required tier exist
        let expected = if in_tier > 0 {
            in_tier
        } else {
            state.active_member_ids(Duration::from_secs(ACTIVE_MEMBER_STALENESS_SECS)).len()
        }.max(1);

        TaskVoteRequirement {
            expected_proposers: expected,
            expected_voters: expected,
            tier_level,
        }
    }

    fn dynamic_branching_factor(swarm_size: u64) -> u64 {
        let approx = (swarm_size as f64).sqrt().round() as u64;
        approx.clamp(3, 10)
    }

    /// Get the current network statistics.
    pub async fn get_network_stats(&self) -> NetworkStats {
        let state = self.state.read().await;
        state.network_stats.clone()
    }

    /// Get the shared state for use by the RPC server.
    pub fn shared_state(&self) -> Arc<RwLock<ConnectorState>> {
        Arc::clone(&self.state)
    }

    /// Get the network handle for use by the RPC server.
    pub fn network_handle(&self) -> SwarmHandle {
        self.network_handle.clone()
    }

}

impl Clone for OpenSwarmConnector {
    fn clone(&self) -> Self {
        Self {
            state: Arc::clone(&self.state),
            network_handle: self.network_handle.clone(),
            event_rx: None, // Don't clone the event receiver (consumed by run())
            swarm_host: None, // Don't clone the swarm host (consumed by run())
            config: self.config.clone(),
        }
    }
}

#[cfg(test)]
impl ConnectorState {
    /// Build a minimal ConnectorState for unit tests (no networking required).
    pub fn new_for_test() -> Self {
        let agent_id = AgentId::new("did:swarm:test-self".to_string());
        let current_swarm_id = SwarmId::new("test-swarm".to_string());
        let mut known_swarms = std::collections::HashMap::new();
        known_swarms.insert(
            current_swarm_id.as_str().to_string(),
            SwarmRecord {
                swarm_id: current_swarm_id.clone(),
                name: "test".to_string(),
                is_public: true,
                agent_count: 1,
                joined: true,
                last_seen: chrono::Utc::now(),
            },
        );
        ConnectorState {
            agent_id: agent_id.clone(),
            status: ConnectorStatus::Running,
            epoch_manager: EpochManager::default(),
            pyramid: PyramidAllocator::new(PyramidConfig::default()),
            election: None,
            geo_cluster: GeoCluster::default(),
            succession: SuccessionManager::new(),
            rfp_coordinators: std::collections::HashMap::new(),
            voting_engines: std::collections::HashMap::new(),
            cascade: CascadeEngine::new(),
            task_set: OrSet::new(agent_id.to_string()),
            task_details: std::collections::HashMap::new(),
            task_timelines: std::collections::HashMap::new(),
            agent_set: OrSet::new(agent_id.to_string()),
            member_set: OrSet::new(agent_id.to_string()),
            member_last_seen: {
                let mut m = std::collections::HashMap::new();
                m.insert(agent_id.to_string(), chrono::Utc::now());
                m
            },
            agent_names: {
                let mut m = std::collections::HashMap::new();
                m.insert(agent_id.to_string(), "test-self".to_string());
                m
            },
            agent_activity: {
                let mut m = std::collections::HashMap::new();
                m.insert(agent_id.to_string(), AgentActivity::default());
                m
            },
            task_vote_requirements: std::collections::HashMap::new(),
            member_last_task_poll: std::collections::HashMap::new(),
            member_last_result: std::collections::HashMap::new(),
            task_result_text: std::collections::HashMap::new(),
            pending_plan_reveals: std::collections::HashMap::new(),
            merkle_dag: MerkleDag::new(),
            content_store: ContentStore::new(),
            granularity: GranularityAlgorithm::default(),
            my_tier: Tier::Executor,
            parent_id: None,
            agent_tiers: std::collections::HashMap::new(),
            agent_parents: std::collections::HashMap::new(),
            current_layout: None,
            subordinates: std::collections::HashMap::new(),
            task_results: std::collections::HashMap::new(),
            network_stats: NetworkStats {
                total_agents: 1,
                hierarchy_depth: 1,
                branching_factor: 3,
                current_epoch: 1,
                my_tier: Tier::Executor,
                subordinate_count: 0,
                parent_id: None,
            },
            event_log: Vec::new(),
            message_trace: Vec::new(),
            start_time: chrono::Utc::now(),
            current_swarm_id,
            known_swarms,
            swarm_token: None,
            active_holons: std::collections::HashMap::new(),
            deliberation_messages: std::collections::HashMap::new(),
            ballot_records: std::collections::HashMap::new(),
            irv_rounds: std::collections::HashMap::new(),
            board_acceptances: std::collections::HashMap::new(),
            name_registry: std::collections::HashMap::new(),
            inbox: Vec::new(),
            outbox: Vec::new(),
            inject_rate_limiter: std::collections::HashMap::new(),
            reputation_ledgers: std::collections::HashMap::new(),
            rep_event_rate_limiter: std::collections::HashMap::new(),
            pending_key_rotations: std::collections::HashMap::new(),
            pending_revocations: std::collections::HashMap::new(),
            guardian_designations: std::collections::HashMap::new(),
            guardian_votes: std::collections::HashMap::new(),
            receipts: std::collections::HashMap::new(),
            clarifications: std::collections::HashMap::new(),
            name_file_path: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_bootstrap_peers_valid_multiaddr_with_peer_id() {
        // Use a valid Ed25519 peer ID (base58btc encoded).
        let peer_id = PeerId::random();
        let addr = format!("/ip4/192.168.1.1/tcp/9000/p2p/{}", peer_id);
        let result = OpenSwarmConnector::parse_bootstrap_peers(&[addr]);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, peer_id);
    }

    #[test]
    fn parse_bootstrap_peers_missing_peer_id() {
        let addr = "/ip4/192.168.1.1/tcp/9000".to_string();
        let result = OpenSwarmConnector::parse_bootstrap_peers(&[addr]);
        assert!(result.is_empty(), "Should skip addrs without /p2p/ component");
    }

    #[test]
    fn parse_bootstrap_peers_invalid_multiaddr() {
        let addr = "not-a-valid-multiaddr".to_string();
        let result = OpenSwarmConnector::parse_bootstrap_peers(&[addr]);
        assert!(result.is_empty(), "Should skip unparseable addrs");
    }

    #[test]
    fn parse_bootstrap_peers_empty_and_whitespace_skipped() {
        let addrs = vec![
            "".to_string(),
            "   ".to_string(),
        ];
        let result = OpenSwarmConnector::parse_bootstrap_peers(&addrs);
        assert!(result.is_empty());
    }

    #[test]
    fn parse_bootstrap_peers_multiple_valid() {
        let peer1 = PeerId::random();
        let peer2 = PeerId::random();
        let addrs = vec![
            format!("/ip4/10.0.0.1/tcp/4001/p2p/{}", peer1),
            format!("/ip4/10.0.0.2/tcp/4001/p2p/{}", peer2),
        ];
        let result = OpenSwarmConnector::parse_bootstrap_peers(&addrs);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].0, peer1);
        assert_eq!(result[1].0, peer2);
    }

    #[test]
    fn parse_bootstrap_peers_mixed_valid_and_invalid() {
        let peer1 = PeerId::random();
        let addrs = vec![
            format!("/ip4/10.0.0.1/tcp/4001/p2p/{}", peer1),
            "/ip4/10.0.0.2/tcp/4001".to_string(), // no peer id
            "garbage".to_string(),                  // unparseable
        ];
        let result = OpenSwarmConnector::parse_bootstrap_peers(&addrs);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, peer1);
    }

    #[test]
    fn extract_peer_id_from_valid_addr() {
        let peer_id = PeerId::random();
        let addr = format!("/ip4/127.0.0.1/tcp/8080/p2p/{}", peer_id);
        let extracted = OpenSwarmConnector::extract_peer_id_from_addr(&addr);
        assert_eq!(extracted, Some(peer_id));
    }

    #[test]
    fn extract_peer_id_from_addr_without_p2p() {
        let addr = "/ip4/127.0.0.1/tcp/8080";
        let extracted = OpenSwarmConnector::extract_peer_id_from_addr(addr);
        assert!(extracted.is_none());
    }

    #[tokio::test]
    #[ignore = "Requires networking support"]
    async fn connector_new_with_default_config() {
        let config = ConnectorConfig::default();
        let connector = OpenSwarmConnector::new(config);
        assert!(connector.is_ok(), "Connector should initialize with default config");
    }

    #[tokio::test]
    #[ignore = "Requires networking support"]
    async fn connector_new_passes_bootstrap_peers_to_discovery() {
        let mut config = ConnectorConfig::default();
        let peer_id = PeerId::random();
        config.network.bootstrap_peers = vec![
            format!("/ip4/10.0.0.1/tcp/9000/p2p/{}", peer_id),
        ];
        let connector = OpenSwarmConnector::new(config);
        assert!(connector.is_ok(), "Connector should initialize with bootstrap peers");
    }

    #[tokio::test]
    #[ignore = "Requires networking support"]
    async fn connector_run_connects_to_swarm_on_start() {
        let config = ConnectorConfig::default();
        let connector = OpenSwarmConnector::new(config).unwrap();
        let state = connector.shared_state();

        // Run the connector with a timeout; it will reach Running status
        // within the timeout, then we abort via select.
        let state_clone = state.clone();
        tokio::select! {
            _ = connector.run() => {}
            _ = async {
                loop {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    let s = state_clone.read().await;
                    if matches!(s.status, ConnectorStatus::Running) {
                        break;
                    }
                }
            } => {}
            _ = tokio::time::sleep(Duration::from_secs(5)) => {
                panic!("Timed out waiting for connector to reach Running status");
            }
        }

        let s = state.read().await;
        assert!(
            matches!(s.status, ConnectorStatus::Running),
            "Connector should be Running after start"
        );
        assert!(
            s.event_log.iter().any(|e| e.message.contains("WWS.Connector started")),
            "Should have startup log entry"
        );
    }

    #[test]
    fn test_has_inject_reputation_unknown_agent() {
        let state = ConnectorState::new_for_test();
        assert!(!state.has_inject_reputation("did:swarm:unknown"));
    }

    #[test]
    fn test_has_inject_reputation_self_always_allowed() {
        let state = ConnectorState::new_for_test();
        let self_id = state.agent_id.to_string();
        assert!(state.has_inject_reputation(&self_id));
    }

    #[test]
    fn test_has_inject_reputation_agent_with_completed_task() {
        let mut state = ConnectorState::new_for_test();
        let agent_id = "did:swarm:test-agent-001".to_string();
        // Build up enough reputation score (>= 100) via TaskExecutedVerified events (+10 each).
        // Need at least 10 events to reach score >= 100.
        for _ in 0..10 {
            state.bump_tasks_processed(&agent_id);
        }
        assert!(state.has_inject_reputation(&agent_id));
    }

    #[test]
    fn test_has_inject_reputation_agent_with_no_completed_tasks() {
        let mut state = ConnectorState::new_for_test();
        let agent_id = "did:swarm:test-agent-002".to_string();
        // No events applied — ledger score is 0, below the 100 threshold for simple tasks.
        assert!(!state.has_inject_reputation(&agent_id));
    }

    #[test]
    fn test_inbox_starts_empty() {
        let state = ConnectorState::new_for_test();
        assert!(state.inbox.is_empty());
    }

    #[test]
    fn test_inbox_stores_messages_addressed_to_self() {
        let mut state = ConnectorState::new_for_test();
        let my_id = state.agent_id.to_string();
        // Message addressed to self gets stored.
        state.inbox.push(InboxMessage {
            from: "did:swarm:sender".to_string(),
            to: my_id.clone(),
            content: "hello from the swarm".to_string(),
            timestamp: chrono::Utc::now(),
        });
        assert_eq!(state.inbox.len(), 1);
        assert_eq!(state.inbox[0].from, "did:swarm:sender");
        assert_eq!(state.inbox[0].content, "hello from the swarm");
    }

    #[test]
    fn test_inbox_can_hold_multiple_messages() {
        let mut state = ConnectorState::new_for_test();
        let my_id = state.agent_id.to_string();
        for i in 0..5 {
            state.inbox.push(InboxMessage {
                from: format!("did:swarm:agent-{}", i),
                to: my_id.clone(),
                content: format!("message {}", i),
                timestamp: chrono::Utc::now(),
            });
        }
        assert_eq!(state.inbox.len(), 5);
    }

    #[test]
    fn test_inject_rate_limit_allows_up_to_10() {
        let mut state = ConnectorState::new_for_test();
        let agent_id = "did:swarm:ratelimit-agent";
        for _ in 0..10 {
            assert!(state.check_and_update_inject_rate_limit(agent_id));
        }
        assert!(!state.check_and_update_inject_rate_limit(agent_id));
    }

    #[test]
    fn test_silent_failure_rate_zero_when_no_outcomes() {
        let a = AgentActivity::default();
        assert_eq!(a.silent_failure_rate(), 0.0);
    }

    #[test]
    fn test_silent_failure_rate_computed() {
        let mut a = AgentActivity::default();
        a.total_outcomes_reported = 10;
        a.silent_failure_count = 3;
        assert!((a.silent_failure_rate() - 0.3).abs() < 1e-6);
    }

    #[test]
    fn test_blast_radius_cost() {
        assert_eq!(blast_radius_cost(Some("low")), 1);
        assert_eq!(blast_radius_cost(Some("medium")), 3);
        assert_eq!(blast_radius_cost(Some("high")), 10);
        assert_eq!(blast_radius_cost(None), 0);
        assert_eq!(blast_radius_cost(Some("unknown")), 0);
    }
}
