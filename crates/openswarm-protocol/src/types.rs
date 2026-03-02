use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::identity::AgentId;

/// Tier in the dynamic pyramid hierarchy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Tier {
    /// Top-level orchestrators (High Command)
    Tier1,
    /// Mid-level coordinators
    Tier2,
    /// General tier at specified depth
    TierN(u32),
    /// Leaf executors (bottom of hierarchy)
    Executor,
}

impl Tier {
    pub fn depth(&self) -> u32 {
        match self {
            Tier::Tier1 => 1,
            Tier::Tier2 => 2,
            Tier::TierN(n) => *n,
            Tier::Executor => u32::MAX,
        }
    }
}

/// Current status of a task in the swarm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum TaskStatus {
    #[default]
    /// Task has been created but not yet assigned
    Pending,
    /// RFP phase: proposals being collected
    ProposalPhase,
    /// Voting phase: ranked choice voting in progress
    VotingPhase,
    /// Task has been assigned and is being executed
    InProgress,
    /// Task completed successfully
    Completed,
    /// Task failed and may be reassigned
    Failed,
    /// Task was rejected during verification
    Rejected,
    /// Task result submitted but confidence delta exceeded review threshold.
    PendingReview,
}

fn default_confidence_review_threshold() -> f32 {
    1.0
}

/// Tri-state of a spec-anchored deliverable (Moltbook insight #13).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DeliverableState {
    Done,
    Partial { note: String },
    Skipped,
}

/// A single named deliverable item in a task spec (Moltbook insight #13).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Deliverable {
    pub id: String,
    pub description: String,
    pub state: DeliverableState,
}

/// A clarification request from an agent to a task principal (Moltbook insight #20).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClarificationRequest {
    pub id: String,
    pub task_id: String,
    pub requesting_agent: String,
    pub principal_id: String,
    pub question: String,
    pub resolution: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub resolved_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// A task in the swarm hierarchy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub task_id: String,
    pub parent_task_id: Option<String>,
    pub epoch: u64,
    pub status: TaskStatus,
    pub description: String,
    pub assigned_to: Option<AgentId>,
    pub tier_level: u32,
    pub subtasks: Vec<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub deadline: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default)]
    pub task_type: String,
    #[serde(default)]
    pub horizon: String,
    #[serde(default)]
    pub capabilities_required: Vec<String>,
    #[serde(default)]
    pub backtrack_allowed: bool,
    #[serde(default)]
    pub knowledge_domains: Vec<String>,
    #[serde(default)]
    pub tools_available: Vec<String>,
    /// Spec-anchored deliverable checklist (Moltbook insight #13).
    #[serde(default)]
    pub deliverables: Vec<Deliverable>,
    /// Minimum coverage fraction for SucceededPartially to be accepted (0.0 = any, 1.0 = full).
    #[serde(default)]
    pub coverage_threshold: f32,
    /// Confidence delta gate: if pre−post > threshold, task moves to PendingReview.
    #[serde(default = "default_confidence_review_threshold")]
    pub confidence_review_threshold: f32,
}

impl Task {
    pub fn new(description: String, tier_level: u32, epoch: u64) -> Self {
        Self {
            task_id: Uuid::new_v4().to_string(),
            parent_task_id: None,
            epoch,
            status: TaskStatus::Pending,
            description,
            assigned_to: None,
            tier_level,
            subtasks: Vec::new(),
            created_at: chrono::Utc::now(),
            deadline: None,
            task_type: String::new(),
            horizon: String::new(),
            capabilities_required: Vec::new(),
            backtrack_allowed: false,
            knowledge_domains: Vec::new(),
            tools_available: Vec::new(),
            deliverables: Vec::new(),
            coverage_threshold: 0.0,
            confidence_review_threshold: 1.0,
        }
    }
}

impl Default for Task {
    fn default() -> Self {
        Self::new(String::new(), 1, 0)
    }
}

/// A High-Level Decomposition Plan proposed by a Tier-1 agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    pub plan_id: String,
    pub task_id: String,
    /// Set server-side; clients may send empty string.
    #[serde(default)]
    pub proposer: AgentId,
    #[serde(default)]
    pub epoch: u64,
    pub subtasks: Vec<PlanSubtask>,
    #[serde(default)]
    pub rationale: String,
    #[serde(default = "default_parallelism")]
    pub estimated_parallelism: f64,
    /// Set server-side; clients may omit.
    #[serde(default = "chrono::Utc::now")]
    pub created_at: chrono::DateTime<chrono::Utc>,
}

fn default_parallelism() -> f64 {
    1.0
}

impl Plan {
    pub fn new(task_id: String, proposer: AgentId, epoch: u64) -> Self {
        Self {
            plan_id: Uuid::new_v4().to_string(),
            task_id,
            proposer,
            epoch,
            subtasks: Vec::new(),
            rationale: String::new(),
            estimated_parallelism: 1.0,
            created_at: chrono::Utc::now(),
        }
    }
}

/// A subtask within a decomposition plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanSubtask {
    pub index: u32,
    pub description: String,
    #[serde(default)]
    pub required_capabilities: Vec<String>,
    pub estimated_complexity: f64,
}

/// Result artifact from task execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    /// Unique artifact ID; generated server-side if empty.
    #[serde(default)]
    pub artifact_id: String,
    /// The task this artifact belongs to.
    #[serde(default)]
    pub task_id: String,
    /// Producer agent; overwritten server-side.
    #[serde(default)]
    pub producer: AgentId,
    /// Content-addressed ID (SHA-256 hash of content); computed server-side if empty.
    #[serde(default)]
    pub content_cid: String,
    /// Merkle hash for verification chain; computed server-side if empty.
    #[serde(default)]
    pub merkle_hash: String,
    #[serde(default = "default_content_type")]
    pub content_type: String,
    #[serde(default)]
    pub size_bytes: u64,
    #[serde(default = "chrono::Utc::now")]
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// Human-readable content / result text.
    #[serde(default)]
    pub content: String,
}

fn default_content_type() -> String {
    "text/plain".to_string()
}

/// Critic evaluation scores for a plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CriticScore {
    pub feasibility: f64,
    pub parallelism: f64,
    pub completeness: f64,
    pub risk: f64,
}

impl CriticScore {
    /// Compute a weighted aggregate score.
    pub fn aggregate(&self) -> f64 {
        0.30 * self.feasibility + 0.25 * self.parallelism + 0.30 * self.completeness
            + 0.15 * (1.0 - self.risk)
    }
}

/// Ranked choice vote from an agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RankedVote {
    pub voter: AgentId,
    pub task_id: String,
    pub epoch: u64,
    /// Plan IDs ranked from most preferred to least preferred
    pub rankings: Vec<String>,
    pub critic_scores: std::collections::HashMap<String, CriticScore>,
}

/// Epoch metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Epoch {
    pub epoch_number: u64,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub duration_secs: u64,
    pub tier1_leaders: Vec<AgentId>,
    pub estimated_swarm_size: u64,
}

/// Network statistics observable by any agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkStats {
    /// Estimated total agents in the swarm (N)
    pub total_agents: u64,
    /// Current hierarchy depth
    pub hierarchy_depth: u32,
    /// Branching factor (k)
    pub branching_factor: u32,
    /// Current epoch
    pub current_epoch: u64,
    /// This agent's tier assignment
    pub my_tier: Tier,
    /// Number of direct subordinates
    pub subordinate_count: u32,
    /// Parent agent ID (None if Tier-1)
    pub parent_id: Option<AgentId>,
}

/// Proof of Work entry proof submitted during handshake.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProofOfWork {
    pub nonce: u64,
    pub hash: String,
    pub difficulty: u32,
}

// ── Swarm Identity ──

/// Unique identifier for a swarm. The default public swarm uses "public".
/// Private swarms use a generated UUID-based ID.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SwarmId(pub String);

impl SwarmId {
    pub fn new(id: String) -> Self {
        Self(id)
    }

    /// Create the default public swarm ID.
    pub fn default_public() -> Self {
        Self(crate::constants::DEFAULT_SWARM_ID.to_string())
    }

    /// Generate a new unique swarm ID.
    pub fn generate() -> Self {
        Self(Uuid::new_v4().to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_public(&self) -> bool {
        self.0 == crate::constants::DEFAULT_SWARM_ID
    }
}

impl std::fmt::Display for SwarmId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Authentication token for joining a private swarm.
/// Generated from HMAC-SHA256(swarm_id, creator_secret).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SwarmToken(pub String);

impl SwarmToken {
    pub fn new(token: String) -> Self {
        Self(token)
    }

    /// Generate a token from a swarm ID and a secret passphrase.
    pub fn generate(swarm_id: &SwarmId, secret: &str) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(swarm_id.as_str().as_bytes());
        hasher.update(b":");
        hasher.update(secret.as_bytes());
        let hash = hasher.finalize();
        Self(hex::encode(hash))
    }

    /// Verify that a token matches a swarm ID and secret.
    pub fn verify(&self, swarm_id: &SwarmId, secret: &str) -> bool {
        let expected = Self::generate(swarm_id, secret);
        self.0 == expected.0
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SwarmToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Only show first 8 chars for security
        if self.0.len() > 8 {
            write!(f, "{}...", &self.0[..8])
        } else {
            write!(f, "{}", self.0)
        }
    }
}

/// Metadata about a swarm, stored in DHT and tracked locally.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmInfo {
    /// Unique swarm identifier.
    pub swarm_id: SwarmId,
    /// Human-readable name of the swarm.
    pub name: String,
    /// Whether the swarm is public (joinable without token).
    pub is_public: bool,
    /// Number of agents currently in this swarm.
    pub agent_count: u64,
    /// The agent who created this swarm.
    pub creator: AgentId,
    /// When the swarm was created.
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// Optional description.
    pub description: String,
}

impl SwarmInfo {
    /// Create a new public swarm info.
    pub fn new_public(creator: AgentId) -> Self {
        Self {
            swarm_id: SwarmId::default_public(),
            name: crate::constants::DEFAULT_SWARM_NAME.to_string(),
            is_public: true,
            agent_count: 1,
            creator,
            created_at: chrono::Utc::now(),
            description: "Default public swarm - open to all agents".to_string(),
        }
    }

    /// Create a new private swarm info.
    pub fn new_private(name: String, creator: AgentId, description: String) -> Self {
        Self {
            swarm_id: SwarmId::generate(),
            name,
            is_public: false,
            agent_count: 1,
            creator,
            created_at: chrono::Utc::now(),
            description,
        }
    }
}

/// A constraint conflict with provenance (Moltbook insights #2, #15).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConstraintConflict {
    pub constraint_a: String,
    pub introduced_by_a: String,
    pub constraint_b: String,
    pub introduced_by_b: String,
}

/// Structured task outcome (Moltbook insight #2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TaskOutcome {
    SucceededFully { artifact_id: String },
    SucceededPartially { artifact_id: String, coverage_spec: String },
    FailedHonestly { reason: FailureReason, duration_ms: u64 },
    FailedSilently,
}

/// Detailed failure reasons for intelligent retry (Moltbook insight #2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FailureReason {
    MissingCapabilities { required: Vec<String>, had: Vec<String> },
    ContradictoryConstraints { conflict_graph: Vec<ConstraintConflict> },
    InsufficientContext { missing_keys: Vec<String> },
    ResourceExhausted { resource: String },
    ExternalDependencyFailed { dependency: String },
    TaskAmbiguous { ambiguity_description: String, proposed_resolution: Option<String> },
}

/// Commitment receipt with rich reversibility info (Moltbook insight #1).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitmentReceipt {
    pub commitment_id: String,
    pub deliverable_type: String,
    pub evidence_hash: String,
    pub confidence_delta: f64,
    pub can_undo: bool,
    pub rollback_cost: Option<String>,
    pub rollback_window: Option<String>,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    pub commitment_state: CommitmentState,
    pub task_id: String,
    pub agent_id: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// State of a commitment receipt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommitmentState {
    Active,
    /// Agent reports completion, awaiting external verification (Moltbook insight #14).
    AgentFulfilled,
    /// External verifier confirmed evidence_hash (Moltbook insight #14).
    Verified,
    /// Finalized, calibration updated (Moltbook insight #14).
    Closed,
    Expired,
    Failed,
    Disputed,
    /// Legacy alias for backward-compat deserialisation.
    #[serde(alias = "Fulfilled")]
    Fulfilled,
}

// ── Holonic Swarm Types ──

/// Status of a holonic board.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HolonStatus {
    Forming,
    Deliberating,
    Voting,
    Executing,
    Synthesizing,
    Done,
}

/// State of a dynamic holonic board formed for a task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HolonState {
    pub task_id: String,
    pub chair: AgentId,
    pub members: Vec<AgentId>,
    pub adversarial_critic: Option<AgentId>,
    pub depth: u32,
    pub parent_holon: Option<String>,
    pub child_holons: Vec<String>,
    pub subtask_assignments: std::collections::HashMap<String, AgentId>,
    pub status: HolonStatus,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Type of deliberation message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeliberationType {
    ProposalSubmission,
    CritiqueFeedback,
    Rebuttal,
    SynthesisResult,
}

/// A message in the deliberation thread of a holon board.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeliberationMessage {
    pub id: String,
    pub task_id: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub speaker: AgentId,
    pub round: u32,
    pub message_type: DeliberationType,
    pub content: String,
    pub referenced_plan_id: Option<String>,
    pub critic_scores: Option<std::collections::HashMap<String, CriticScore>>,
}

/// Per-voter ballot record for full deliberation visibility.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BallotRecord {
    pub task_id: String,
    pub voter: AgentId,
    pub rankings: Vec<String>,
    pub critic_scores: std::collections::HashMap<String, CriticScore>,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub irv_round_when_eliminated: Option<u32>,
}

/// IRV round history for debugging and UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IrvRound {
    pub task_id: String,
    pub round_number: u32,
    pub tallies: std::collections::HashMap<String, usize>,
    pub eliminated: Option<String>,
    pub continuing_candidates: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_task_creation() {
        let task = Task::new("Test task".into(), 1, 1);
        assert_eq!(task.status, TaskStatus::Pending);
        assert!(task.parent_task_id.is_none());
        assert!(task.subtasks.is_empty());
    }

    #[test]
    fn test_critic_score_aggregate() {
        let score = CriticScore {
            feasibility: 0.9,
            parallelism: 0.8,
            completeness: 0.85,
            risk: 0.2,
        };
        let expected = 0.30 * 0.9 + 0.25 * 0.8 + 0.30 * 0.85 + 0.15 * 0.8;
        assert!((score.aggregate() - expected).abs() < 1e-10);
    }

    #[test]
    fn test_tier_ordering() {
        assert!(Tier::Tier1.depth() < Tier::Tier2.depth());
        assert!(Tier::Tier2.depth() < Tier::TierN(3).depth());
    }

    #[test]
    fn test_swarm_id_default_public() {
        let id = SwarmId::default_public();
        assert_eq!(id.as_str(), "public");
        assert!(id.is_public());
    }

    #[test]
    fn test_swarm_id_generate() {
        let id1 = SwarmId::generate();
        let id2 = SwarmId::generate();
        assert_ne!(id1, id2);
        assert!(!id1.is_public());
    }

    #[test]
    fn test_swarm_token_generate_and_verify() {
        let swarm_id = SwarmId::new("test-swarm".to_string());
        let secret = "my-secret-passphrase";
        let token = SwarmToken::generate(&swarm_id, secret);

        assert!(token.verify(&swarm_id, secret));
        assert!(!token.verify(&swarm_id, "wrong-secret"));
        assert!(!token.verify(&SwarmId::new("other-swarm".to_string()), secret));
    }

    #[test]
    fn test_swarm_token_deterministic() {
        let swarm_id = SwarmId::new("test-swarm".to_string());
        let secret = "my-secret";
        let token1 = SwarmToken::generate(&swarm_id, secret);
        let token2 = SwarmToken::generate(&swarm_id, secret);
        assert_eq!(token1, token2);
    }

    #[test]
    fn test_swarm_info_public() {
        let creator = AgentId::new("did:swarm:test".to_string());
        let info = SwarmInfo::new_public(creator);
        assert!(info.is_public);
        assert!(info.swarm_id.is_public());
        assert_eq!(info.agent_count, 1);
    }

    #[test]
    fn test_swarm_info_private() {
        let creator = AgentId::new("did:swarm:test".to_string());
        let info = SwarmInfo::new_private("My Swarm".to_string(), creator, "desc".to_string());
        assert!(!info.is_public);
        assert!(!info.swarm_id.is_public());
    }

    #[test]
    fn test_task_new_fields_default() {
        let task = Task::new("Test holonic task".into(), 1, 1);
        assert_eq!(task.task_type, "");
        assert_eq!(task.horizon, "");
        assert!(task.capabilities_required.is_empty());
        assert!(!task.backtrack_allowed);
        assert!(task.knowledge_domains.is_empty());
        assert!(task.tools_available.is_empty());
    }

    #[test]
    fn test_holon_status_variants() {
        let status = HolonStatus::Forming;
        assert_eq!(status, HolonStatus::Forming);
        assert_ne!(status, HolonStatus::Done);
    }

    #[test]
    fn test_holon_state_serialization_roundtrip() {
        use crate::identity::AgentId;
        use std::collections::HashMap;

        let chair = AgentId::new("did:swarm:chair".to_string());
        let member1 = AgentId::new("did:swarm:member1".to_string());
        let critic = AgentId::new("did:swarm:critic".to_string());

        let mut subtask_assignments = HashMap::new();
        subtask_assignments.insert("subtask-1".to_string(), member1.clone());

        let holon = HolonState {
            task_id: "task-abc".to_string(),
            chair: chair.clone(),
            members: vec![member1.clone(), critic.clone()],
            adversarial_critic: Some(critic.clone()),
            depth: 2,
            parent_holon: Some("parent-task-id".to_string()),
            child_holons: vec!["child-1".to_string(), "child-2".to_string()],
            subtask_assignments,
            status: HolonStatus::Deliberating,
            created_at: chrono::Utc::now(),
        };

        let json = serde_json::to_string(&holon).unwrap();
        let restored: HolonState = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.task_id, "task-abc");
        assert_eq!(restored.chair, chair);
        assert_eq!(restored.members.len(), 2);
        assert_eq!(restored.adversarial_critic, Some(critic));
        assert_eq!(restored.depth, 2);
        assert_eq!(restored.parent_holon, Some("parent-task-id".to_string()));
        assert_eq!(restored.child_holons.len(), 2);
        assert_eq!(restored.subtask_assignments.len(), 1);
        assert_eq!(restored.status, HolonStatus::Deliberating);
    }

    #[test]
    fn test_holon_state_no_critic_no_parent() {
        let chair = AgentId::new("did:swarm:chair".to_string());
        let holon = HolonState {
            task_id: "root-task".to_string(),
            chair: chair.clone(),
            members: vec![chair.clone()],
            adversarial_critic: None,
            depth: 0,
            parent_holon: None,
            child_holons: vec![],
            subtask_assignments: std::collections::HashMap::new(),
            status: HolonStatus::Forming,
            created_at: chrono::Utc::now(),
        };

        let json = serde_json::to_string(&holon).unwrap();
        let restored: HolonState = serde_json::from_str(&json).unwrap();
        assert!(restored.adversarial_critic.is_none());
        assert!(restored.parent_holon.is_none());
        assert_eq!(restored.depth, 0);
        assert_eq!(restored.status, HolonStatus::Forming);
    }

    #[test]
    fn test_deliberation_message_serialization_roundtrip() {
        use std::collections::HashMap;

        let speaker = AgentId::new("did:swarm:speaker".to_string());
        let mut scores = HashMap::new();
        scores.insert("plan-1".to_string(), CriticScore {
            feasibility: 0.8,
            parallelism: 0.7,
            completeness: 0.9,
            risk: 0.2,
        });

        let msg = DeliberationMessage {
            id: "msg-uuid-123".to_string(),
            task_id: "task-xyz".to_string(),
            timestamp: chrono::Utc::now(),
            speaker: speaker.clone(),
            round: 2,
            message_type: DeliberationType::CritiqueFeedback,
            content: "Plan 1 has insufficient parallelism for subtask 3".to_string(),
            referenced_plan_id: Some("plan-1".to_string()),
            critic_scores: Some(scores),
        };

        let json = serde_json::to_string(&msg).unwrap();
        let restored: DeliberationMessage = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.id, "msg-uuid-123");
        assert_eq!(restored.task_id, "task-xyz");
        assert_eq!(restored.speaker, speaker);
        assert_eq!(restored.round, 2);
        assert_eq!(restored.message_type, DeliberationType::CritiqueFeedback);
        assert!(restored.content.contains("parallelism"));
        assert_eq!(restored.referenced_plan_id, Some("plan-1".to_string()));
        let scores = restored.critic_scores.unwrap();
        assert!((scores["plan-1"].feasibility - 0.8).abs() < 1e-10);
    }

    #[test]
    fn test_deliberation_message_proposal_submission() {
        let speaker = AgentId::new("did:swarm:proposer".to_string());
        let msg = DeliberationMessage {
            id: "msg-1".to_string(),
            task_id: "task-1".to_string(),
            timestamp: chrono::Utc::now(),
            speaker,
            round: 1,
            message_type: DeliberationType::ProposalSubmission,
            content: "My proposal for decomposing the task".to_string(),
            referenced_plan_id: None,
            critic_scores: None,
        };

        let json = serde_json::to_string(&msg).unwrap();
        let restored: DeliberationMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.message_type, DeliberationType::ProposalSubmission);
        assert!(restored.critic_scores.is_none());
        assert!(restored.referenced_plan_id.is_none());
    }

    #[test]
    fn test_deliberation_type_all_variants_serialize() {
        let types = vec![
            DeliberationType::ProposalSubmission,
            DeliberationType::CritiqueFeedback,
            DeliberationType::Rebuttal,
            DeliberationType::SynthesisResult,
        ];
        for t in types {
            let json = serde_json::to_string(&t).unwrap();
            let restored: DeliberationType = serde_json::from_str(&json).unwrap();
            assert_eq!(restored, t);
        }
    }

    #[test]
    fn test_ballot_record_serialization_roundtrip() {
        use std::collections::HashMap;

        let voter = AgentId::new("did:swarm:voter".to_string());
        let mut critic_scores = HashMap::new();
        critic_scores.insert("plan-A".to_string(), CriticScore {
            feasibility: 0.9,
            parallelism: 0.8,
            completeness: 0.85,
            risk: 0.1,
        });
        critic_scores.insert("plan-B".to_string(), CriticScore {
            feasibility: 0.6,
            parallelism: 0.5,
            completeness: 0.7,
            risk: 0.3,
        });

        let record = BallotRecord {
            task_id: "task-vote".to_string(),
            voter: voter.clone(),
            rankings: vec!["plan-A".to_string(), "plan-B".to_string()],
            critic_scores,
            timestamp: chrono::Utc::now(),
            irv_round_when_eliminated: Some(2),
        };

        let json = serde_json::to_string(&record).unwrap();
        let restored: BallotRecord = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.task_id, "task-vote");
        assert_eq!(restored.voter, voter);
        assert_eq!(restored.rankings, vec!["plan-A", "plan-B"]);
        assert_eq!(restored.critic_scores.len(), 2);
        assert_eq!(restored.irv_round_when_eliminated, Some(2));
        assert!((restored.critic_scores["plan-A"].feasibility - 0.9).abs() < 1e-10);
    }

    #[test]
    fn test_ballot_record_no_elimination() {
        let voter = AgentId::new("did:swarm:winner".to_string());
        let record = BallotRecord {
            task_id: "task-1".to_string(),
            voter,
            rankings: vec!["plan-A".to_string()],
            critic_scores: std::collections::HashMap::new(),
            timestamp: chrono::Utc::now(),
            irv_round_when_eliminated: None,
        };

        let json = serde_json::to_string(&record).unwrap();
        let restored: BallotRecord = serde_json::from_str(&json).unwrap();
        assert!(restored.irv_round_when_eliminated.is_none());
    }

    #[test]
    fn test_irv_round_serialization_roundtrip() {
        use std::collections::HashMap;

        let mut tallies = HashMap::new();
        tallies.insert("plan-A".to_string(), 3usize);
        tallies.insert("plan-B".to_string(), 2usize);
        tallies.insert("plan-C".to_string(), 1usize);

        let round = IrvRound {
            task_id: "task-irv".to_string(),
            round_number: 1,
            tallies,
            eliminated: Some("plan-C".to_string()),
            continuing_candidates: vec!["plan-A".to_string(), "plan-B".to_string()],
        };

        let json = serde_json::to_string(&round).unwrap();
        let restored: IrvRound = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.task_id, "task-irv");
        assert_eq!(restored.round_number, 1);
        assert_eq!(restored.tallies.len(), 3);
        assert_eq!(restored.tallies["plan-A"], 3);
        assert_eq!(restored.eliminated, Some("plan-C".to_string()));
        assert_eq!(restored.continuing_candidates.len(), 2);
    }

    #[test]
    fn test_task_outcome_serialization() {
        let outcome = TaskOutcome::SucceededPartially {
            artifact_id: "art-1".into(),
            coverage_spec: "80% complete".into(),
        };
        let json = serde_json::to_string(&outcome).unwrap();
        let restored: TaskOutcome = serde_json::from_str(&json).unwrap();
        match restored {
            TaskOutcome::SucceededPartially { coverage_spec, .. } => {
                assert_eq!(coverage_spec, "80% complete");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_failure_reason_missing_capabilities() {
        let reason = FailureReason::MissingCapabilities {
            required: vec!["gpu".into()],
            had: vec!["cpu".into()],
        };
        let json = serde_json::to_string(&reason).unwrap();
        assert!(json.contains("MissingCapabilities"));
    }

    #[test]
    fn test_commitment_receipt_serialization() {
        let receipt = CommitmentReceipt {
            commitment_id: "c1".into(),
            deliverable_type: "artifact".into(),
            evidence_hash: "sha256:abc".into(),
            confidence_delta: 0.1,
            can_undo: true,
            rollback_cost: Some("low".into()),
            rollback_window: Some("PT1H".into()),
            expires_at: None,
            commitment_state: CommitmentState::Active,
            task_id: "t1".into(),
            agent_id: "a1".into(),
            created_at: chrono::Utc::now(),
        };
        let json = serde_json::to_string(&receipt).unwrap();
        let restored: CommitmentReceipt = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.commitment_state, CommitmentState::Active);
        assert!((restored.confidence_delta - 0.1).abs() < 1e-10);
    }

    #[test]
    fn test_irv_round_final_no_elimination() {
        use std::collections::HashMap;
        let mut tallies = HashMap::new();
        tallies.insert("plan-A".to_string(), 5usize);

        let round = IrvRound {
            task_id: "task-final".to_string(),
            round_number: 3,
            tallies,
            eliminated: None,
            continuing_candidates: vec!["plan-A".to_string()],
        };

        let json = serde_json::to_string(&round).unwrap();
        let restored: IrvRound = serde_json::from_str(&json).unwrap();
        assert!(restored.eliminated.is_none());
        assert_eq!(restored.round_number, 3);
    }

    #[test]
    fn test_task_serialization_with_new_fields() {
        let mut task = Task::new("Research cold fusion pathways".into(), 1, 5);
        task.task_type = "scientific_research".to_string();
        task.horizon = "long".to_string();
        task.capabilities_required = vec!["physics".to_string(), "chemistry".to_string()];
        task.backtrack_allowed = true;
        task.knowledge_domains = vec!["nuclear-physics".to_string()];
        task.tools_available = vec!["pubmed_search".to_string(), "arxiv_query".to_string()];

        let json = serde_json::to_string(&task).unwrap();
        let restored: Task = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.task_type, "scientific_research");
        assert_eq!(restored.horizon, "long");
        assert_eq!(restored.capabilities_required, vec!["physics", "chemistry"]);
        assert!(restored.backtrack_allowed);
        assert_eq!(restored.knowledge_domains, vec!["nuclear-physics"]);
        assert_eq!(restored.tools_available.len(), 2);
    }

    #[test]
    fn test_task_deserialization_missing_new_fields_defaults() {
        // Old-format task JSON without new fields should deserialize with defaults
        let json = r#"{
            "task_id": "old-task-id",
            "parent_task_id": null,
            "epoch": 1,
            "status": "Pending",
            "description": "Legacy task",
            "assigned_to": null,
            "tier_level": 1,
            "subtasks": [],
            "created_at": "2025-01-01T00:00:00Z",
            "deadline": null
        }"#;

        let task: Task = serde_json::from_str(json).unwrap();
        assert_eq!(task.task_type, "");
        assert_eq!(task.horizon, "");
        assert!(task.capabilities_required.is_empty());
        assert!(!task.backtrack_allowed);
        assert!(task.knowledge_domains.is_empty());
        assert!(task.tools_available.is_empty());
    }

    #[test]
    fn test_deliverable_tri_state_serialises() {
        let d = Deliverable {
            id: "d1".into(),
            description: "Write tests".into(),
            state: DeliverableState::Partial { note: "half done".into() },
        };
        let json = serde_json::to_string(&d).unwrap();
        let back: Deliverable = serde_json::from_str(&json).unwrap();
        match back.state {
            DeliverableState::Partial { note } => assert_eq!(note, "half done"),
            _ => panic!("wrong state"),
        }
    }

    #[test]
    fn test_task_with_deliverables_defaults() {
        let t = Task::new("x".into(), 1, 0);
        assert!(t.deliverables.is_empty());
        assert_eq!(t.coverage_threshold, 0.0);
        assert_eq!(t.confidence_review_threshold, 1.0);
    }

    #[test]
    fn test_constraint_conflict_provenance() {
        let cc = ConstraintConflict {
            constraint_a: "must finish by Friday".into(),
            introduced_by_a: "principal".into(),
            constraint_b: "cannot start until Monday".into(),
            introduced_by_b: "alice".into(),
        };
        let json = serde_json::to_string(&cc).unwrap();
        let back: ConstraintConflict = serde_json::from_str(&json).unwrap();
        assert_eq!(back.introduced_by_b, "alice");
    }

    #[test]
    fn test_failure_reason_contradictory_uses_conflict_graph() {
        let fr = FailureReason::ContradictoryConstraints {
            conflict_graph: vec![ConstraintConflict {
                constraint_a: "A".into(),
                introduced_by_a: "p1".into(),
                constraint_b: "B".into(),
                introduced_by_b: "p2".into(),
            }],
        };
        let json = serde_json::to_string(&fr).unwrap();
        let back: FailureReason = serde_json::from_str(&json).unwrap();
        match back {
            FailureReason::ContradictoryConstraints { conflict_graph } => {
                assert_eq!(conflict_graph.len(), 1);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn test_pending_review_task_status() {
        let s = TaskStatus::PendingReview;
        let json = serde_json::to_string(&s).unwrap();
        let back: TaskStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(back, TaskStatus::PendingReview);
    }

    #[test]
    fn test_clarification_request_serialises() {
        let cr = ClarificationRequest {
            id: "cr-1".into(),
            task_id: "t-1".into(),
            requesting_agent: "alice".into(),
            principal_id: "bob".into(),
            question: "Which format?".into(),
            resolution: None,
            created_at: chrono::Utc::now(),
            resolved_at: None,
        };
        let json = serde_json::to_string(&cr).unwrap();
        let back: ClarificationRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.question, "Which format?");
    }

    #[test]
    fn test_commitment_state_agent_fulfilled() {
        let s = CommitmentState::AgentFulfilled;
        let json = serde_json::to_string(&s).unwrap();
        let back: CommitmentState = serde_json::from_str(&json).unwrap();
        assert_eq!(back, CommitmentState::AgentFulfilled);
    }

    #[test]
    fn test_commitment_state_legacy_fulfilled_compat() {
        // Old "Fulfilled" JSON should deserialise to Fulfilled variant
        let back: CommitmentState = serde_json::from_str("\"Fulfilled\"").unwrap();
        assert_eq!(back, CommitmentState::Fulfilled);
    }

    #[test]
    fn test_holon_status_all_variants_serialize() {
        let statuses = vec![
            HolonStatus::Forming,
            HolonStatus::Deliberating,
            HolonStatus::Voting,
            HolonStatus::Executing,
            HolonStatus::Synthesizing,
            HolonStatus::Done,
        ];
        for status in statuses {
            let json = serde_json::to_string(&status).unwrap();
            let restored: HolonStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(restored, status);
        }
    }
}
