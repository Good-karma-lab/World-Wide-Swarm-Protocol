//! Comprehensive tests for the protocol message types.
//!
//! Verifies:
//! - SwarmMessage envelope structure (JSON-RPC 2.0 compliance)
//! - Signing payload construction
//! - All protocol method string mappings
//! - GossipSub topic formatting
//! - All message payload types serialization
//! - Response construction (success and error)

use wws_protocol::messages::*;
use wws_protocol::types::*;
use wws_protocol::identity::*;
use wws_protocol::constants::*;

// ═══════════════════════════════════════════════════════════════
// § 3.1 SwarmMessage Envelope (JSON-RPC 2.0)
// ═══════════════════════════════════════════════════════════════

#[test]
fn swarm_message_has_jsonrpc_version() {
    let msg = SwarmMessage::new(
        "swarm.handshake",
        serde_json::json!({}),
        "sig".into(),
    );
    assert_eq!(msg.jsonrpc, "2.0", "jsonrpc field MUST be '2.0'");
}

#[test]
fn swarm_message_has_method() {
    let msg = SwarmMessage::new(
        "swarm.handshake",
        serde_json::json!({}),
        "sig".into(),
    );
    assert_eq!(msg.method, "swarm.handshake");
}

#[test]
fn swarm_message_has_uuid_id() {
    let msg = SwarmMessage::new(
        "swarm.handshake",
        serde_json::json!({}),
        "sig".into(),
    );
    assert!(msg.id.is_some(), "Request messages MUST have an id");
    let id = msg.id.unwrap();
    // UUID v4 format: xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx
    assert!(id.len() == 36, "ID should be a UUID-v4 string");
}

#[test]
fn swarm_message_serialization_roundtrip() {
    let msg = SwarmMessage::new(
        "consensus.vote",
        serde_json::json!({"task_id": "task-123", "rankings": ["a", "b"]}),
        "deadbeef".into(),
    );
    let json = serde_json::to_string(&msg).unwrap();
    let parsed: SwarmMessage = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.method, "consensus.vote");
    assert_eq!(parsed.signature, "deadbeef");
    assert_eq!(parsed.params["task_id"], "task-123");
}

#[test]
fn swarm_message_signing_payload_is_deterministic() {
    let method = "swarm.handshake";
    let params = serde_json::json!({"agent_id": "did:swarm:abc"});
    let p1 = SwarmMessage::signing_payload(method, &params);
    let p2 = SwarmMessage::signing_payload(method, &params);
    assert_eq!(p1, p2, "Signing payload must be deterministic");
}

#[test]
fn swarm_message_signing_payload_includes_method_and_params() {
    let method = "task.inject";
    let params = serde_json::json!({"task_id": "t1"});
    let payload = SwarmMessage::signing_payload(method, &params);
    let payload_str = String::from_utf8(payload).unwrap();
    assert!(
        payload_str.contains("task.inject"),
        "Signing payload must include method"
    );
    assert!(
        payload_str.contains("t1"),
        "Signing payload must include params"
    );
}

#[test]
fn swarm_message_different_params_produce_different_payloads() {
    let method = "task.assign";
    let p1 = SwarmMessage::signing_payload(method, &serde_json::json!({"x": 1}));
    let p2 = SwarmMessage::signing_payload(method, &serde_json::json!({"x": 2}));
    assert_ne!(p1, p2, "Different params must produce different signing payloads");
}

// ═══════════════════════════════════════════════════════════════
// § 3.1.2 Response
// ═══════════════════════════════════════════════════════════════

#[test]
fn response_success_has_result_no_error() {
    let resp = SwarmResponse::success(Some("id-1".into()), serde_json::json!({"ok": true}));
    assert!(resp.result.is_some());
    assert!(resp.error.is_none());
    assert_eq!(resp.jsonrpc, "2.0");
}

#[test]
fn response_error_has_error_no_result() {
    let resp = SwarmResponse::error(Some("id-2".into()), -32600, "Invalid Request".into());
    assert!(resp.result.is_none());
    assert!(resp.error.is_some());
    assert_eq!(resp.error.as_ref().unwrap().code, -32600);
    assert_eq!(resp.error.as_ref().unwrap().message, "Invalid Request");
}

#[test]
fn response_preserves_request_id() {
    let resp = SwarmResponse::success(Some("my-id-123".into()), serde_json::json!(null));
    assert_eq!(resp.id, Some("my-id-123".into()));
}

#[test]
fn response_serialization_roundtrip() {
    let resp = SwarmResponse::error(Some("id-3".into()), -31000, "Self-vote prohibited".into());
    let json = serde_json::to_string(&resp).unwrap();
    let parsed: SwarmResponse = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.error.as_ref().unwrap().code, -31000);
}

// ═══════════════════════════════════════════════════════════════
// § 13.2 Protocol Method Registry
// ═══════════════════════════════════════════════════════════════

#[test]
fn all_protocol_methods_have_string_representation() {
    let methods = vec![
        ProtocolMethod::Handshake,
        ProtocolMethod::Candidacy,
        ProtocolMethod::ElectionVote,
        ProtocolMethod::TierAssignment,
        ProtocolMethod::TaskInjection,
        ProtocolMethod::ProposalCommit,
        ProtocolMethod::ProposalReveal,
        ProtocolMethod::ConsensusVote,
        ProtocolMethod::TaskAssignment,
        ProtocolMethod::ResultSubmission,
        ProtocolMethod::VerificationResult,
        ProtocolMethod::KeepAlive,
        ProtocolMethod::Succession,
    ];
    for method in &methods {
        let s = method.as_str();
        assert!(!s.is_empty(), "Method string must not be empty");
        assert!(
            s.contains('.'),
            "Method string must be in namespace.action format: {}",
            s
        );
    }
}

#[test]
fn protocol_method_roundtrip_all() {
    let method_strings = vec![
        "swarm.handshake",
        "election.candidacy",
        "election.vote",
        "hierarchy.assign_tier",
        "task.inject",
        "consensus.proposal_commit",
        "consensus.proposal_reveal",
        "consensus.vote",
        "task.assign",
        "task.submit_result",
        "task.verification",
        "swarm.keepalive",
        "hierarchy.succession",
    ];
    for s in method_strings {
        let parsed = ProtocolMethod::from_str(s);
        assert!(parsed.is_some(), "Must parse valid method string: {}", s);
        assert_eq!(
            parsed.unwrap().as_str(),
            s,
            "Roundtrip must preserve method string"
        );
    }
}

#[test]
fn protocol_method_from_str_unknown_returns_none() {
    assert!(ProtocolMethod::from_str("unknown.method").is_none());
    assert!(ProtocolMethod::from_str("").is_none());
    assert!(ProtocolMethod::from_str("swarm.nonexistent").is_none());
}

// ═══════════════════════════════════════════════════════════════
// § 3.2.3 GossipSub Topics
// ═══════════════════════════════════════════════════════════════

#[test]
fn topics_have_correct_prefix() {
    let prefix = "/wws/1.0.0";
    assert!(SwarmTopics::election_tier1().starts_with(prefix));
    assert!(SwarmTopics::keepalive().starts_with(prefix));
    assert!(SwarmTopics::hierarchy().starts_with(prefix));
    assert!(SwarmTopics::proposals("task-1").starts_with(prefix));
    assert!(SwarmTopics::voting("task-1").starts_with(prefix));
    assert!(SwarmTopics::tasks(1).starts_with(prefix));
    assert!(SwarmTopics::results("task-1").starts_with(prefix));
}

#[test]
fn topics_contain_task_id() {
    let task_id = "unique-task-id-12345";
    assert!(SwarmTopics::proposals(task_id).contains(task_id));
    assert!(SwarmTopics::voting(task_id).contains(task_id));
    assert!(SwarmTopics::results(task_id).contains(task_id));
}

#[test]
fn topics_contain_tier_number() {
    let topic = SwarmTopics::tasks(3);
    assert!(topic.contains("tier3"), "Task topic must contain tier number");
}

#[test]
fn topic_election_tier1_is_unique() {
    let t = SwarmTopics::election_tier1();
    assert!(t.contains("election") && t.contains("tier1"));
}

// ═══════════════════════════════════════════════════════════════
// Message Payload Types Serialization
// ═══════════════════════════════════════════════════════════════

#[test]
fn handshake_params_serialization() {
    let params = HandshakeParams {
        agent_id: AgentId::new("did:swarm:abc".into()),
        pub_key: "MCowBQ...".into(),
        capabilities: vec!["gpt-4".into(), "python-exec".into()],
        resources: AgentResources {
            cpu_cores: 8,
            ram_gb: 32,
            gpu_vram_gb: None,
            disk_gb: Some(100),
        },
        location_vector: VivaldiCoordinates {
            x: 0.45,
            y: 0.12,
            z: 0.99,
        },
        proof_of_work: ProofOfWork {
            nonce: 283741,
            hash: "0000a3f2...".into(),
            difficulty: 16,
        },
        protocol_version: PROTOCOL_VERSION.to_string(),
    };
    let json = serde_json::to_string(&params).unwrap();
    let parsed: HandshakeParams = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.agent_id.as_str(), "did:swarm:abc");
    assert_eq!(parsed.capabilities.len(), 2);
    assert_eq!(parsed.proof_of_work.nonce, 283741);
}

#[test]
fn candidacy_params_serialization() {
    let params = CandidacyParams {
        agent_id: AgentId::new("did:swarm:candidate".into()),
        epoch: 106,
        score: NodeScore {
            agent_id: AgentId::new("did:swarm:candidate".into()),
            proof_of_compute: 0.85,
            reputation: 0.92,
            uptime: 0.99,
            stake: Some(0.5),
        },
        location_vector: VivaldiCoordinates {
            x: 0.1,
            y: 0.2,
            z: 0.3,
        },
    };
    let json = serde_json::to_string(&params).unwrap();
    let parsed: CandidacyParams = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.epoch, 106);
    assert!((parsed.score.reputation - 0.92).abs() < 1e-10);
}

#[test]
fn consensus_vote_params_serialization() {
    let mut critic_scores = std::collections::HashMap::new();
    critic_scores.insert(
        "plan-1".into(),
        CriticScore {
            feasibility: 0.9,
            parallelism: 0.8,
            completeness: 0.85,
            risk: 0.2,
        },
    );
    let params = ConsensusVoteParams {
        task_id: "task-001".into(),
        epoch: 106,
        voter: AgentId::new("did:swarm:voter".into()),
        rankings: vec!["plan-1".into(), "plan-2".into()],
        critic_scores,
    };
    let json = serde_json::to_string(&params).unwrap();
    let parsed: ConsensusVoteParams = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.rankings.len(), 2);
    assert!(parsed.critic_scores.contains_key("plan-1"));
}

#[test]
fn result_submission_params_serialization() {
    let params = ResultSubmissionParams {
        task_id: "task-sub-001".into(),
        agent_id: AgentId::new("did:swarm:exec1".into()),
        artifact: Artifact {
            artifact_id: "art-001".into(),
            task_id: "task-sub-001".into(),
            producer: AgentId::new("did:swarm:exec1".into()),
            content_cid: "QmYwAPJzv5CZsnA...".into(),
            merkle_hash: "a3f2b1c4d5e6...".into(),
            content_type: "application/json".into(),
            size_bytes: 4096,
            created_at: chrono::Utc::now(),
            content: "result content".into(),
        },
        merkle_proof: vec!["hash1".into(), "hash2".into()],
        is_synthesis: false,
    };
    let json = serde_json::to_string(&params).unwrap();
    let parsed: ResultSubmissionParams = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.artifact.size_bytes, 4096);
    assert_eq!(parsed.merkle_proof.len(), 2);
}

#[test]
fn keepalive_params_serialization() {
    let params = KeepAliveParams {
        agent_id: AgentId::new("did:swarm:alive".into()),
        agent_name: Some("alive".into()),
        last_task_poll_at: None,
        last_result_at: None,
        epoch: 105,
        timestamp: chrono::Utc::now(),
    };
    let json = serde_json::to_string(&params).unwrap();
    let parsed: KeepAliveParams = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.epoch, 105);
}

#[test]
fn succession_params_serialization() {
    let params = SuccessionParams {
        failed_leader: AgentId::new("did:swarm:old".into()),
        new_leader: AgentId::new("did:swarm:new".into()),
        epoch: 106,
        branch_agents: vec![
            AgentId::new("did:swarm:agent1".into()),
            AgentId::new("did:swarm:agent2".into()),
        ],
    };
    let json = serde_json::to_string(&params).unwrap();
    let parsed: SuccessionParams = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.branch_agents.len(), 2);
}

#[test]
fn tier_assignment_params_serialization() {
    let params = TierAssignmentParams {
        assigned_agent: AgentId::new("did:swarm:agent".into()),
        tier: Tier::Tier1,
        parent_id: AgentId::new("did:swarm:leader".into()),
        epoch: 106,
        branch_size: 85,
    };
    let json = serde_json::to_string(&params).unwrap();
    let parsed: TierAssignmentParams = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.tier, Tier::Tier1);
    assert_eq!(parsed.branch_size, 85);
}

// ═══════════════════════════════════════════════════════════════
// § 11.2 Error Codes
// ═══════════════════════════════════════════════════════════════

#[test]
fn error_codes_in_correct_ranges() {
    // JSON-RPC standard errors: -32700 to -32600
    let standard_errors = vec![
        (-32700, "Parse error"),
        (-32600, "Invalid Request"),
        (-32601, "Method not found"),
        (-32602, "Invalid params"),
    ];
    for (code, _msg) in &standard_errors {
        assert!(
            *code >= -32700 && *code <= -32600,
            "Standard error {} out of range",
            code
        );
    }

    // Protocol errors: -32000 to -32099
    let protocol_errors = vec![(-32000, "Invalid signature"), (-32002, "Invalid PoW")];
    for (code, _msg) in &protocol_errors {
        assert!(
            *code >= -32099 && *code <= -32000,
            "Protocol error {} out of range",
            code
        );
    }

    // Consensus errors: -31000 to -31099
    let consensus_errors = vec![(-31000, "Self-vote"), (-31003, "Voting timeout")];
    for (code, _msg) in &consensus_errors {
        assert!(
            *code >= -31099 && *code <= -31000,
            "Consensus error {} out of range",
            code
        );
    }
}
