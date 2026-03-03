//! Integration tests for the WWS Protocol.
//!
//! These tests verify end-to-end protocol flows by composing
//! multiple crates together. They test the full lifecycle:
//! - Identity creation -> handshake -> hierarchy formation
//! - Task injection -> RFP -> voting -> cascade -> execution -> verification

use wws_protocol::crypto::{
    compute_cid, derive_agent_id, generate_keypair, sign_message, verify_signature,
};
use wws_protocol::identity::*;
use wws_protocol::messages::*;
use wws_protocol::types::*;
use wws_protocol::constants::*;

// ═══════════════════════════════════════════════════════════════
// End-to-End: Identity and Handshake
// ═══════════════════════════════════════════════════════════════

#[test]
fn e2e_agent_identity_and_handshake_message() {
    // 1. Generate keypair
    let key = generate_keypair();
    let agent_id = derive_agent_id(&key.verifying_key());

    // 2. Construct handshake params
    let params = HandshakeParams {
        agent_id: AgentId::new(agent_id.clone()),
        pub_key: hex_encode(key.verifying_key().as_bytes()),
        capabilities: vec!["gpt-4".into(), "web-search".into()],
        resources: AgentResources {
            cpu_cores: 8,
            ram_gb: 32,
            gpu_vram_gb: None,
            disk_gb: Some(100),
        },
        location_vector: VivaldiCoordinates::origin(),
        proof_of_work: ProofOfWork {
            nonce: 0,
            hash: "placeholder".into(),
            difficulty: 0,
        },
        protocol_version: PROTOCOL_VERSION.to_string(),
    };

    // 3. Serialize params
    let params_json = serde_json::to_value(&params).unwrap();

    // 4. Sign the message
    let payload = SwarmMessage::signing_payload("swarm.handshake", &params_json);
    let signature = sign_message(&key, &payload);
    let sig_hex: String = signature
        .to_bytes()
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect();

    // 5. Construct the full message
    let msg = SwarmMessage::new("swarm.handshake", params_json.clone(), sig_hex.clone());

    // 6. Verify: receiver can verify signature
    let verify_payload = SwarmMessage::signing_payload(&msg.method, &msg.params);
    let sig_bytes: Vec<u8> = (0..sig_hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&sig_hex[i..i + 2], 16).unwrap())
        .collect();
    let sig = ed25519_dalek::Signature::from_bytes(&sig_bytes.try_into().unwrap());
    let result = verify_signature(&key.verifying_key(), &verify_payload, &sig);
    assert!(result.is_ok(), "Handshake message signature must verify");
}

#[test]
fn e2e_task_lifecycle_messages() {
    let key = generate_keypair();
    let agent_id = AgentId::new(derive_agent_id(&key.verifying_key()));

    // 1. Task injection
    let task = Task::new("Analyze market data".into(), 1, 106);
    let injection = TaskInjectionParams {
        task: task.clone(),
        originator: agent_id.clone(),
    };
    let injection_json = serde_json::to_value(&injection).unwrap();
    assert!(injection_json["task"]["status"] == "Pending");

    // 2. Proposal commit
    let plan_json = serde_json::json!({
        "subtasks": [
            {"index": 0, "description": "Gather data"},
            {"index": 1, "description": "Analyze trends"},
        ]
    });
    let plan_hash = compute_cid(serde_json::to_string(&plan_json).unwrap().as_bytes());
    let commit = ProposalCommitParams {
        task_id: task.task_id.clone(),
        proposer: agent_id.clone(),
        epoch: 106,
        plan_hash: plan_hash.clone(),
    };
    let commit_json = serde_json::to_value(&commit).unwrap();
    assert_eq!(commit_json["plan_hash"].as_str().unwrap(), &plan_hash);

    // 3. Proposal reveal
    let plan = Plan::new(task.task_id.clone(), agent_id.clone(), 106);
    let reveal = ProposalRevealParams {
        task_id: task.task_id.clone(),
        plan: plan.clone(),
    };
    let reveal_json = serde_json::to_value(&reveal).unwrap();
    assert!(reveal_json["plan"]["plan_id"].is_string());

    // 4. Vote
    let vote = ConsensusVoteParams {
        task_id: task.task_id.clone(),
        epoch: 106,
        voter: agent_id.clone(),
        rankings: vec![plan.plan_id.clone()],
        critic_scores: std::collections::HashMap::new(),
    };
    let vote_json = serde_json::to_value(&vote).unwrap();
    assert_eq!(vote_json["rankings"].as_array().unwrap().len(), 1);

    // 5. Result submission
    let artifact = Artifact {
        artifact_id: uuid::Uuid::new_v4().to_string(),
        task_id: task.task_id.clone(),
        producer: agent_id.clone(),
        content_cid: compute_cid(b"analysis result"),
        merkle_hash: compute_cid(b"analysis result"),
        content_type: "application/json".into(),
        size_bytes: 2048,
        created_at: chrono::Utc::now(),
        content: "analysis result".into(),
    };
    let result_msg = ResultSubmissionParams {
        task_id: task.task_id.clone(),
        agent_id: agent_id.clone(),
        artifact,
        merkle_proof: vec![],
        is_synthesis: false,
    };
    let result_json = serde_json::to_value(&result_msg).unwrap();
    assert_eq!(result_json["artifact"]["size_bytes"], 2048);
}

// ═══════════════════════════════════════════════════════════════
// End-to-End: Hierarchy Depth and Score
// ═══════════════════════════════════════════════════════════════

#[test]
fn e2e_hierarchy_depth_matches_spec_examples() {
    use wws_hierarchy::PyramidAllocator;

    let allocator = PyramidAllocator::default(); // k=10

    assert_eq!(allocator.compute_depth(1), 1);
    assert_eq!(allocator.compute_depth(10), 1);
    assert_eq!(allocator.compute_depth(11), 2);
    assert_eq!(allocator.compute_depth(100), 2);
    assert_eq!(allocator.compute_depth(101), 3);
    assert_eq!(allocator.compute_depth(850), 3);
    assert_eq!(allocator.compute_depth(1000), 3);
    assert_eq!(allocator.compute_depth(1001), 4);
    assert_eq!(allocator.compute_depth(10000), 4);
    assert_eq!(allocator.compute_depth(10001), 5);
}

#[test]
fn e2e_node_scores_ranked_correctly() {
    let agents: Vec<NodeScore> = vec![
        NodeScore {
            agent_id: AgentId::new("a".into()),
            proof_of_compute: 0.5,
            reputation: 0.5,
            uptime: 0.5,
            stake: None,
        },
        NodeScore {
            agent_id: AgentId::new("b".into()),
            proof_of_compute: 0.9,
            reputation: 0.9,
            uptime: 0.9,
            stake: Some(0.8),
        },
        NodeScore {
            agent_id: AgentId::new("c".into()),
            proof_of_compute: 0.1,
            reputation: 0.1,
            uptime: 0.1,
            stake: None,
        },
    ];

    let mut sorted = agents.clone();
    sorted.sort_by(|a, b| {
        b.composite_score()
            .partial_cmp(&a.composite_score())
            .unwrap()
    });

    assert_eq!(sorted[0].agent_id.as_str(), "b", "Highest score first");
    assert_eq!(sorted[1].agent_id.as_str(), "a");
    assert_eq!(sorted[2].agent_id.as_str(), "c", "Lowest score last");
}

// ═══════════════════════════════════════════════════════════════
// End-to-End: Merkle Verification Chain
// ═══════════════════════════════════════════════════════════════

#[test]
fn e2e_merkle_verification_three_tier() {
    use wws_state::MerkleDag;

    // Simulate 3-tier hierarchy: 1 root, 2 coordinators, 4 executors
    let mut dag = MerkleDag::new();

    // Tier-3 executors produce artifacts
    let exec_results = vec![b"result_1", b"result_2", b"result_3", b"result_4"];
    let leaf_hashes: Vec<String> = exec_results
        .iter()
        .enumerate()
        .map(|(i, data)| {
            let node = dag.add_leaf(format!("exec-{}", i), *data);
            node.hash
        })
        .collect();

    // Tier-2 coordinators aggregate
    let coord_1 = dag.add_branch("coord-1".into(), leaf_hashes[0..2].to_vec());
    let coord_2 = dag.add_branch("coord-2".into(), leaf_hashes[2..4].to_vec());

    // Tier-1 root assembles
    let root = dag.add_branch(
        "root".into(),
        vec![coord_1.hash.clone(), coord_2.hash.clone()],
    );

    // Verify: changing any leaf changes the root
    let mut dag_tampered = MerkleDag::new();
    let tampered_leaf = dag_tampered.add_leaf("exec-0".into(), b"TAMPERED");
    let leaf_1_same = dag_tampered.add_leaf("exec-1".into(), b"result_2");
    let tampered_coord = dag_tampered.add_branch(
        "coord-1".into(),
        vec![tampered_leaf.hash, leaf_1_same.hash],
    );
    let coord_2_same =
        dag_tampered.add_branch("coord-2".into(), leaf_hashes[2..4].to_vec());
    let tampered_root = dag_tampered.add_branch(
        "root".into(),
        vec![tampered_coord.hash, coord_2_same.hash],
    );

    assert_ne!(
        root.hash, tampered_root.hash,
        "Tampering any leaf must change the root hash"
    );
}

#[test]
fn e2e_multilevel_cascade_decomposition_and_backprop() {
    use wws_consensus::CascadeEngine;

    let mut cascade = CascadeEngine::new();

    // Root plan: split into two coordinator-level subtasks.
    let mut root_plan = Plan::new("root-task".into(), AgentId::new("tier1".into()), 1);
    root_plan.subtasks = vec![
        PlanSubtask {
            index: 0,
            description: "Domain A".into(),
            required_capabilities: vec!["analysis".into()],
            estimated_complexity: 0.6,
        },
        PlanSubtask {
            index: 1,
            description: "Domain B".into(),
            required_capabilities: vec!["analysis".into()],
            estimated_complexity: 0.4,
        },
    ];

    let tier2_nodes = vec![
        (AgentId::new("tier2-a".into()), Tier::Tier1),
        (AgentId::new("tier2-b".into()), Tier::Tier1),
    ];
    let root_assignments = cascade
        .distribute_subtasks("root-task", &root_plan, &tier2_nodes, 1)
        .unwrap();
    assert_eq!(root_assignments.len(), 2);
    assert!(root_assignments.iter().all(|a| a.requires_cascade));

    // First coordinator decomposes further to executors.
    let first_mid_task = root_assignments[0].task.task_id.clone();
    let mut mid_plan = Plan::new(
        first_mid_task.clone(),
        AgentId::new("tier2-a".into()),
        1,
    );
    mid_plan.subtasks = vec![
        PlanSubtask {
            index: 0,
            description: "Leaf A1".into(),
            required_capabilities: vec!["exec".into()],
            estimated_complexity: 0.5,
        },
        PlanSubtask {
            index: 1,
            description: "Leaf A2".into(),
            required_capabilities: vec!["exec".into()],
            estimated_complexity: 0.5,
        },
    ];

    let executors = vec![
        (AgentId::new("exec-1".into()), Tier::Executor),
        (AgentId::new("exec-2".into()), Tier::Executor),
    ];
    let leaf_assignments = cascade
        .distribute_subtasks(&first_mid_task, &mid_plan, &executors, 1)
        .unwrap();
    assert_eq!(leaf_assignments.len(), 2);
    assert!(leaf_assignments.iter().all(|a| !a.requires_cascade));

    // Leaf completions aggregate to their parent.
    assert!(!cascade
        .record_subtask_completion(&leaf_assignments[0].task.task_id)
        .unwrap());
    assert!(cascade
        .record_subtask_completion(&leaf_assignments[1].task.task_id)
        .unwrap());

    // Parent of this branch is now complete, mark the branch subtask itself done.
    assert!(!cascade
        .record_subtask_completion(&root_assignments[0].task.task_id)
        .unwrap());

    // Mark sibling root-level branch done; now root cascade completes.
    assert!(cascade
        .record_subtask_completion(&root_assignments[1].task.task_id)
        .unwrap());

    assert!(cascade.is_complete());
}

#[test]
fn e2e_same_tier_topic_and_method_support() {
    // Same-tier peers should communicate on the same tier topic namespace.
    let tier2_topic = SwarmTopics::tasks_for("public", 2);
    assert!(tier2_topic.contains("/tasks/tier2"));

    // Agent heartbeat protocol method should round-trip.
    let method = ProtocolMethod::AgentKeepAlive;
    let parsed = ProtocolMethod::from_str(method.as_str());
    assert!(parsed.is_some());
    assert_eq!(parsed.unwrap().as_str(), "agent.keepalive");
}

// Helper for hex encoding
fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}
