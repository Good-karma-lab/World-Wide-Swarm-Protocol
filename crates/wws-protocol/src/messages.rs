use serde::{Deserialize, Serialize};

use crate::constants::JSONRPC_VERSION;
use crate::identity::AgentId;
use crate::types::*;

/// Top-level JSON-RPC 2.0 message envelope.
/// All swarm communications use this format with Ed25519 signatures.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmMessage {
    pub jsonrpc: String,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub params: serde_json::Value,
    /// Ed25519 signature over the canonical JSON of (method + params)
    pub signature: String,
}

impl SwarmMessage {
    pub fn new(method: &str, params: serde_json::Value, signature: String) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION.to_string(),
            method: method.to_string(),
            id: Some(uuid::Uuid::new_v4().to_string()),
            params,
            signature,
        }
    }

    /// Get the canonical bytes for signing: JSON(method + params).
    pub fn signing_payload(method: &str, params: &serde_json::Value) -> Vec<u8> {
        let canonical = serde_json::json!({
            "method": method,
            "params": params,
        });
        serde_json::to_vec(&canonical).unwrap_or_default()
    }
}

/// JSON-RPC response envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmResponse {
    pub jsonrpc: String,
    pub id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

impl SwarmResponse {
    pub fn success(id: Option<String>, result: serde_json::Value) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION.to_string(),
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn error(id: Option<String>, code: i32, message: String) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION.to_string(),
            id,
            result: None,
            error: Some(RpcError {
                code,
                message,
                data: None,
            }),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

// ── Specific Message Payloads ──

/// Handshake message sent on peer connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandshakeParams {
    pub agent_id: AgentId,
    pub pub_key: String,
    pub capabilities: Vec<String>,
    pub resources: crate::identity::AgentResources,
    pub location_vector: crate::identity::VivaldiCoordinates,
    pub proof_of_work: ProofOfWork,
    pub protocol_version: String,
}

/// Candidacy announcement for Tier-1 election.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidacyParams {
    pub agent_id: AgentId,
    pub epoch: u64,
    pub score: crate::identity::NodeScore,
    pub location_vector: crate::identity::VivaldiCoordinates,
}

/// Election vote for a Tier-1 candidate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElectionVoteParams {
    pub voter: AgentId,
    pub epoch: u64,
    pub candidate_rankings: Vec<AgentId>,
}

/// Tier assignment notification from parent to subordinate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TierAssignmentParams {
    pub assigned_agent: AgentId,
    pub tier: Tier,
    pub parent_id: AgentId,
    pub epoch: u64,
    pub branch_size: u64,
}

/// Task injection from external source or parent agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskInjectionParams {
    pub task: Task,
    pub originator: AgentId,
}

/// Commit phase of proposal (hash only, plan hidden).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProposalCommitParams {
    pub task_id: String,
    pub proposer: AgentId,
    pub epoch: u64,
    /// SHA-256 hash of the full plan JSON
    pub plan_hash: String,
}

/// Reveal phase of proposal (full plan disclosed).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProposalRevealParams {
    pub task_id: String,
    pub plan: Plan,
}

/// Ranked Choice Vote for plan selection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsensusVoteParams {
    pub task_id: String,
    pub epoch: u64,
    pub voter: AgentId,
    pub rankings: Vec<String>,
    pub critic_scores: std::collections::HashMap<String, CriticScore>,
}

/// Task assignment from coordinator to subordinate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskAssignmentParams {
    pub task: Task,
    pub assignee: AgentId,
    pub parent_task_id: String,
    pub winning_plan_id: String,
}

/// Result submission from executor to coordinator.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResultSubmissionParams {
    pub task_id: String,
    #[serde(default)]
    pub agent_id: AgentId,
    pub artifact: Artifact,
    #[serde(default)]
    pub merkle_proof: Vec<String>,
    /// Optional flag: marks result as a coordinator synthesis (not raw execution).
    #[serde(default)]
    pub is_synthesis: bool,
}

/// Verification result from coordinator back to subordinate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationResultParams {
    pub task_id: String,
    pub agent_id: AgentId,
    pub accepted: bool,
    pub reason: Option<String>,
}

/// Keep-alive ping.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeepAliveParams {
    pub agent_id: AgentId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_task_poll_at: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_result_at: Option<chrono::DateTime<chrono::Utc>>,
    pub epoch: u64,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// Succession announcement when a leader fails.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuccessionParams {
    pub failed_leader: AgentId,
    pub new_leader: AgentId,
    pub epoch: u64,
    pub branch_agents: Vec<AgentId>,
}

// ── Swarm Identity Messages ──

/// Announce the existence of a swarm to the network (via DHT + GossipSub).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmAnnounceParams {
    pub swarm_id: SwarmId,
    pub name: String,
    pub is_public: bool,
    pub agent_id: AgentId,
    pub agent_count: u64,
    pub description: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// Request to join a swarm. For private swarms, includes token.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmJoinParams {
    pub swarm_id: SwarmId,
    pub agent_id: AgentId,
    /// Token for private swarm authentication (None for public swarms).
    pub token: Option<SwarmToken>,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// Response to a join request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmJoinResponseParams {
    pub swarm_id: SwarmId,
    pub agent_id: AgentId,
    pub accepted: bool,
    pub reason: Option<String>,
}

/// Leave a swarm notification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmLeaveParams {
    pub swarm_id: SwarmId,
    pub agent_id: AgentId,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

// ── Holonic Board Messages ──

/// Board invitation from chair to local cluster.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoardInviteParams {
    pub task_id: String,
    pub task_digest: String,
    pub complexity_estimate: f64,
    pub depth: u32,
    pub required_capabilities: Vec<String>,
    pub capacity: usize,
    pub chair: AgentId,
}

/// Agent accepts a board invitation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoardAcceptParams {
    pub task_id: String,
    pub agent_id: AgentId,
    pub active_tasks: u32,
    pub capabilities: Vec<String>,
    pub affinity_scores: std::collections::HashMap<String, f64>,
}

/// Agent declines a board invitation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoardDeclineParams {
    pub task_id: String,
    pub agent_id: AgentId,
}

/// Board is ready: chair announces final membership.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoardReadyParams {
    pub task_id: String,
    pub chair_id: AgentId,
    pub members: Vec<AgentId>,
    pub adversarial_critic: Option<AgentId>,
}

/// Board dissolves after task completion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoardDissolveParams {
    pub task_id: String,
}

/// Critique message from a board member after proposals are revealed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscussionCritiqueParams {
    pub task_id: String,
    pub voter_id: AgentId,
    pub round: u32,
    pub plan_scores: std::collections::HashMap<String, CriticScore>,
    pub content: String,
}

/// Enumeration of all protocol methods for pattern matching.
#[derive(Debug, Clone)]
pub enum ProtocolMethod {
    Handshake,
    Candidacy,
    ElectionVote,
    TierAssignment,
    TaskInjection,
    ProposalCommit,
    ProposalReveal,
    ConsensusVote,
    TaskAssignment,
    ResultSubmission,
    VerificationResult,
    KeepAlive,
    AgentKeepAlive,
    Succession,
    SwarmAnnounce,
    SwarmJoin,
    SwarmJoinResponse,
    SwarmLeave,
    BoardInvite,
    BoardAccept,
    BoardDecline,
    BoardReady,
    BoardDissolve,
    DiscussionCritique,
    /// Agent-to-agent direct message (broadcast on shared DM topic, filtered by `to` field).
    DirectMessage,
}

impl ProtocolMethod {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Handshake => "swarm.handshake",
            Self::Candidacy => "election.candidacy",
            Self::ElectionVote => "election.vote",
            Self::TierAssignment => "hierarchy.assign_tier",
            Self::TaskInjection => "task.inject",
            Self::ProposalCommit => "consensus.proposal_commit",
            Self::ProposalReveal => "consensus.proposal_reveal",
            Self::ConsensusVote => "consensus.vote",
            Self::TaskAssignment => "task.assign",
            Self::ResultSubmission => "task.submit_result",
            Self::VerificationResult => "task.verification",
            Self::KeepAlive => "swarm.keepalive",
            Self::AgentKeepAlive => "agent.keepalive",
            Self::Succession => "hierarchy.succession",
            Self::SwarmAnnounce => "swarm.announce",
            Self::SwarmJoin => "swarm.join",
            Self::SwarmJoinResponse => "swarm.join_response",
            Self::SwarmLeave => "swarm.leave",
            Self::BoardInvite => "board.invite",
            Self::BoardAccept => "board.accept",
            Self::BoardDecline => "board.decline",
            Self::BoardReady => "board.ready",
            Self::BoardDissolve => "board.dissolve",
            Self::DiscussionCritique => "discussion.critique",
            Self::DirectMessage => "agent.direct_message",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "swarm.handshake" => Some(Self::Handshake),
            "election.candidacy" => Some(Self::Candidacy),
            "election.vote" => Some(Self::ElectionVote),
            "hierarchy.assign_tier" => Some(Self::TierAssignment),
            "task.inject" => Some(Self::TaskInjection),
            "consensus.proposal_commit" => Some(Self::ProposalCommit),
            "consensus.proposal_reveal" => Some(Self::ProposalReveal),
            "consensus.vote" => Some(Self::ConsensusVote),
            "task.assign" => Some(Self::TaskAssignment),
            "task.submit_result" => Some(Self::ResultSubmission),
            "task.verification" => Some(Self::VerificationResult),
            "swarm.keepalive" => Some(Self::KeepAlive),
            "agent.keepalive" => Some(Self::AgentKeepAlive),
            "hierarchy.succession" => Some(Self::Succession),
            "swarm.announce" => Some(Self::SwarmAnnounce),
            "swarm.join" => Some(Self::SwarmJoin),
            "swarm.join_response" => Some(Self::SwarmJoinResponse),
            "swarm.leave" => Some(Self::SwarmLeave),
            "board.invite" => Some(Self::BoardInvite),
            "board.accept" => Some(Self::BoardAccept),
            "board.decline" => Some(Self::BoardDecline),
            "board.ready" => Some(Self::BoardReady),
            "board.dissolve" => Some(Self::BoardDissolve),
            "discussion.critique" => Some(Self::DiscussionCritique),
            "agent.direct_message" => Some(Self::DirectMessage),
            _ => None,
        }
    }
}

/// GossipSub topics used by the protocol.
///
/// All topics are namespaced by swarm_id to isolate communication between
/// different swarms on the same network. The default public swarm uses
/// "public" as its swarm_id.
pub struct SwarmTopics;

impl SwarmTopics {
    /// Global swarm discovery topic (shared across all swarms).
    pub fn swarm_discovery() -> String {
        format!("{}/swarm/discovery", crate::constants::TOPIC_PREFIX)
    }

    /// Swarm-specific announcement topic.
    pub fn swarm_announce(swarm_id: &str) -> String {
        format!("{}/swarm/{}/announce", crate::constants::TOPIC_PREFIX, swarm_id)
    }

    pub fn election_tier1() -> String {
        Self::election_tier1_for(crate::constants::DEFAULT_SWARM_ID)
    }

    pub fn election_tier1_for(swarm_id: &str) -> String {
        format!("{}/s/{}/election/tier1", crate::constants::TOPIC_PREFIX, swarm_id)
    }

    pub fn proposals(task_id: &str) -> String {
        Self::proposals_for(crate::constants::DEFAULT_SWARM_ID, task_id)
    }

    pub fn proposals_for(swarm_id: &str, task_id: &str) -> String {
        format!("{}/s/{}/proposals/{}", crate::constants::TOPIC_PREFIX, swarm_id, task_id)
    }

    pub fn voting(task_id: &str) -> String {
        Self::voting_for(crate::constants::DEFAULT_SWARM_ID, task_id)
    }

    pub fn voting_for(swarm_id: &str, task_id: &str) -> String {
        format!("{}/s/{}/voting/{}", crate::constants::TOPIC_PREFIX, swarm_id, task_id)
    }

    pub fn tasks(tier: u32) -> String {
        Self::tasks_for(crate::constants::DEFAULT_SWARM_ID, tier)
    }

    pub fn tasks_for(swarm_id: &str, tier: u32) -> String {
        format!("{}/s/{}/tasks/tier{}", crate::constants::TOPIC_PREFIX, swarm_id, tier)
    }

    pub fn results(task_id: &str) -> String {
        Self::results_for(crate::constants::DEFAULT_SWARM_ID, task_id)
    }

    pub fn results_for(swarm_id: &str, task_id: &str) -> String {
        format!("{}/s/{}/results/{}", crate::constants::TOPIC_PREFIX, swarm_id, task_id)
    }

    pub fn keepalive() -> String {
        Self::keepalive_for(crate::constants::DEFAULT_SWARM_ID)
    }

    pub fn keepalive_for(swarm_id: &str) -> String {
        format!("{}/s/{}/keepalive", crate::constants::TOPIC_PREFIX, swarm_id)
    }

    pub fn hierarchy() -> String {
        Self::hierarchy_for(crate::constants::DEFAULT_SWARM_ID)
    }

    pub fn hierarchy_for(swarm_id: &str) -> String {
        format!("{}/s/{}/hierarchy", crate::constants::TOPIC_PREFIX, swarm_id)
    }

    pub fn board(task_id: &str) -> String {
        Self::board_for(crate::constants::DEFAULT_SWARM_ID, task_id)
    }

    pub fn board_for(swarm_id: &str, task_id: &str) -> String {
        format!("{}/s/{}/board/{}", crate::constants::TOPIC_PREFIX, swarm_id, task_id)
    }

    /// Shared direct-message topic for a swarm.
    /// All agents subscribe to this topic; each filters messages addressed to itself.
    pub fn dm_for(swarm_id: &str) -> String {
        format!("{}/s/{}/dm", crate::constants::TOPIC_PREFIX, swarm_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_swarm_message_serialization() {
        let msg = SwarmMessage::new(
            "swarm.handshake",
            serde_json::json!({"agent_id": "did:swarm:abc"}),
            "sig_placeholder".to_string(),
        );
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("swarm.handshake"));

        let parsed: SwarmMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.method, "swarm.handshake");
    }

    #[test]
    fn test_protocol_method_roundtrip() {
        let methods = vec![
            ProtocolMethod::Handshake,
            ProtocolMethod::Candidacy,
            ProtocolMethod::ConsensusVote,
            ProtocolMethod::ResultSubmission,
            ProtocolMethod::AgentKeepAlive,
        ];
        for method in methods {
            let s = method.as_str();
            let parsed = ProtocolMethod::from_str(s);
            assert!(parsed.is_some(), "Failed to parse: {}", s);
        }
    }

    #[test]
    fn test_response_success() {
        let resp = SwarmResponse::success(Some("id-1".into()), serde_json::json!({"ok": true}));
        assert!(resp.error.is_none());
        assert!(resp.result.is_some());
    }

    #[test]
    fn test_response_error() {
        let resp = SwarmResponse::error(Some("id-2".into()), -32600, "Invalid Request".into());
        assert!(resp.result.is_none());
        assert_eq!(resp.error.as_ref().unwrap().code, -32600);
    }

    #[test]
    fn test_swarm_protocol_methods_roundtrip() {
        let methods = vec![
            ProtocolMethod::SwarmAnnounce,
            ProtocolMethod::SwarmJoin,
            ProtocolMethod::SwarmJoinResponse,
            ProtocolMethod::SwarmLeave,
        ];
        for method in methods {
            let s = method.as_str();
            let parsed = ProtocolMethod::from_str(s);
            assert!(parsed.is_some(), "Failed to parse: {}", s);
        }
    }

    #[test]
    fn test_board_protocol_methods_roundtrip() {
        let methods = vec![
            ProtocolMethod::BoardInvite,
            ProtocolMethod::BoardAccept,
            ProtocolMethod::BoardDecline,
            ProtocolMethod::BoardReady,
            ProtocolMethod::BoardDissolve,
            ProtocolMethod::DiscussionCritique,
        ];
        for method in methods {
            let s = method.as_str();
            let parsed = ProtocolMethod::from_str(s);
            assert!(parsed.is_some(), "Failed to parse: {}", s);
        }
    }

    #[test]
    fn test_swarm_namespaced_topics() {
        let default_keepalive = SwarmTopics::keepalive();
        let custom_keepalive = SwarmTopics::keepalive_for("my-swarm");

        assert!(default_keepalive.contains("/s/public/"));
        assert!(custom_keepalive.contains("/s/my-swarm/"));
        assert_ne!(default_keepalive, custom_keepalive);
    }

    #[test]
    fn test_swarm_discovery_topic() {
        let topic = SwarmTopics::swarm_discovery();
        assert!(topic.contains("swarm/discovery"));
    }

    #[test]
    fn test_board_topics() {
        let board_topic = SwarmTopics::board("task-123");
        assert!(board_topic.contains("/s/public/board/task-123"));

        let custom_board = SwarmTopics::board_for("my-swarm", "task-456");
        assert!(custom_board.contains("/s/my-swarm/board/task-456"));
    }

    #[test]
    fn test_board_invite_params_serialization() {
        let params = BoardInviteParams {
            task_id: "task-abc".to_string(),
            task_digest: "Design a distributed consensus protocol".to_string(),
            complexity_estimate: 0.85,
            depth: 1,
            required_capabilities: vec!["distributed-systems".to_string(), "consensus".to_string()],
            capacity: 5,
            chair: AgentId::new("did:swarm:chair".to_string()),
        };

        let json = serde_json::to_string(&params).unwrap();
        let restored: BoardInviteParams = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.task_id, "task-abc");
        assert!((restored.complexity_estimate - 0.85).abs() < 1e-10);
        assert_eq!(restored.depth, 1);
        assert_eq!(restored.required_capabilities.len(), 2);
        assert_eq!(restored.capacity, 5);
        assert_eq!(restored.chair, AgentId::new("did:swarm:chair".to_string()));
    }

    #[test]
    fn test_board_accept_params_serialization() {
        let mut affinity = std::collections::HashMap::new();
        affinity.insert("consensus".to_string(), 0.9_f64);
        affinity.insert("distributed-systems".to_string(), 0.7_f64);

        let params = BoardAcceptParams {
            task_id: "task-abc".to_string(),
            agent_id: AgentId::new("did:swarm:agent1".to_string()),
            active_tasks: 2,
            capabilities: vec!["consensus".to_string()],
            affinity_scores: affinity,
        };

        let json = serde_json::to_string(&params).unwrap();
        let restored: BoardAcceptParams = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.task_id, "task-abc");
        assert_eq!(restored.active_tasks, 2);
        assert_eq!(restored.capabilities.len(), 1);
        assert!((restored.affinity_scores["consensus"] - 0.9).abs() < 1e-10);
    }

    #[test]
    fn test_board_decline_params_serialization() {
        let params = BoardDeclineParams {
            task_id: "task-busy".to_string(),
            agent_id: AgentId::new("did:swarm:busy-agent".to_string()),
        };

        let json = serde_json::to_string(&params).unwrap();
        let restored: BoardDeclineParams = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.task_id, "task-busy");
        assert_eq!(restored.agent_id, AgentId::new("did:swarm:busy-agent".to_string()));
    }

    #[test]
    fn test_board_ready_params_serialization() {
        let adversarial = AgentId::new("did:swarm:critic".to_string());
        let params = BoardReadyParams {
            task_id: "task-ready".to_string(),
            chair_id: AgentId::new("did:swarm:chair".to_string()),
            members: vec![
                AgentId::new("did:swarm:m1".to_string()),
                AgentId::new("did:swarm:m2".to_string()),
                adversarial.clone(),
            ],
            adversarial_critic: Some(adversarial.clone()),
        };

        let json = serde_json::to_string(&params).unwrap();
        let restored: BoardReadyParams = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.task_id, "task-ready");
        assert_eq!(restored.members.len(), 3);
        assert_eq!(restored.adversarial_critic, Some(adversarial));
    }

    #[test]
    fn test_board_ready_params_no_critic_serialization() {
        let params = BoardReadyParams {
            task_id: "task-small".to_string(),
            chair_id: AgentId::new("did:swarm:chair".to_string()),
            members: vec![AgentId::new("did:swarm:chair".to_string())],
            adversarial_critic: None,
        };

        let json = serde_json::to_string(&params).unwrap();
        let restored: BoardReadyParams = serde_json::from_str(&json).unwrap();
        assert!(restored.adversarial_critic.is_none());
    }

    #[test]
    fn test_board_dissolve_params_serialization() {
        let params = BoardDissolveParams { task_id: "task-done".to_string() };
        let json = serde_json::to_string(&params).unwrap();
        let restored: BoardDissolveParams = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.task_id, "task-done");
    }

    #[test]
    fn test_discussion_critique_params_serialization() {
        let mut plan_scores = std::collections::HashMap::new();
        plan_scores.insert("plan-1".to_string(), CriticScore {
            feasibility: 0.8,
            parallelism: 0.9,
            completeness: 0.7,
            risk: 0.15,
        });
        plan_scores.insert("plan-2".to_string(), CriticScore {
            feasibility: 0.6,
            parallelism: 0.5,
            completeness: 0.65,
            risk: 0.4,
        });

        let params = DiscussionCritiqueParams {
            task_id: "task-critique".to_string(),
            voter_id: AgentId::new("did:swarm:voter".to_string()),
            round: 2,
            plan_scores,
            content: "Plan 1 is superior in parallelism but plan 2 has better completeness coverage".to_string(),
        };

        let json = serde_json::to_string(&params).unwrap();
        let restored: DiscussionCritiqueParams = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.task_id, "task-critique");
        assert_eq!(restored.round, 2);
        assert_eq!(restored.plan_scores.len(), 2);
        assert!((restored.plan_scores["plan-1"].parallelism - 0.9).abs() < 1e-10);
        assert!(restored.content.contains("parallelism"));
    }

    #[test]
    fn test_board_message_in_swarm_message() {
        // Verify board params can be embedded in a SwarmMessage as JSON params
        let invite = BoardInviteParams {
            task_id: "t-1".to_string(),
            task_digest: "Solve cancer".to_string(),
            complexity_estimate: 0.95,
            depth: 0,
            required_capabilities: vec!["oncology".to_string()],
            capacity: 7,
            chair: AgentId::new("did:swarm:root".to_string()),
        };

        let msg = SwarmMessage::new(
            "board.invite",
            serde_json::to_value(&invite).unwrap(),
            "sig".to_string(),
        );

        let json = serde_json::to_string(&msg).unwrap();
        let restored: SwarmMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.method, "board.invite");

        // Extract the params back
        let restored_invite: BoardInviteParams = serde_json::from_value(restored.params).unwrap();
        assert_eq!(restored_invite.task_id, "t-1");
        assert!((restored_invite.complexity_estimate - 0.95).abs() < 1e-10);
    }
}
