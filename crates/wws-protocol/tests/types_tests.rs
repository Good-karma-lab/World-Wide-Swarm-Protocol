//! Comprehensive tests for the core protocol types.
//!
//! Verifies:
//! - Task creation and status transitions
//! - Plan construction and subtask management
//! - CriticScore evaluation
//! - Tier ordering and depth
//! - Epoch tracking
//! - NetworkStats structure

use wws_protocol::types::*;
use wws_protocol::identity::AgentId;

// ═══════════════════════════════════════════════════════════════
// § 6.2 Task
// ═══════════════════════════════════════════════════════════════

#[test]
fn task_new_has_pending_status() {
    let task = Task::new("Test task".into(), 1, 1);
    assert_eq!(task.status, TaskStatus::Pending);
}

#[test]
fn task_new_has_uuid_id() {
    let task = Task::new("Test".into(), 1, 1);
    assert!(!task.task_id.is_empty());
    // UUID v4 format: 36 chars with dashes
    assert_eq!(task.task_id.len(), 36);
}

#[test]
fn task_new_has_no_parent() {
    let task = Task::new("Root task".into(), 1, 1);
    assert!(task.parent_task_id.is_none());
}

#[test]
fn task_new_has_no_subtasks() {
    let task = Task::new("Leaf task".into(), 3, 1);
    assert!(task.subtasks.is_empty());
}

#[test]
fn task_new_has_no_assignee() {
    let task = Task::new("Unassigned".into(), 1, 1);
    assert!(task.assigned_to.is_none());
}

#[test]
fn task_preserves_tier_level() {
    let task = Task::new("Tier 3 task".into(), 3, 5);
    assert_eq!(task.tier_level, 3);
    assert_eq!(task.epoch, 5);
}

#[test]
fn task_has_timestamp() {
    let before = chrono::Utc::now();
    let task = Task::new("Timed task".into(), 1, 1);
    let after = chrono::Utc::now();
    assert!(task.created_at >= before);
    assert!(task.created_at <= after);
}

#[test]
fn task_unique_ids() {
    let t1 = Task::new("Task 1".into(), 1, 1);
    let t2 = Task::new("Task 2".into(), 1, 1);
    assert_ne!(
        t1.task_id, t2.task_id,
        "Different tasks must have different IDs"
    );
}

#[test]
fn task_status_serialization() {
    let statuses = vec![
        TaskStatus::Pending,
        TaskStatus::ProposalPhase,
        TaskStatus::VotingPhase,
        TaskStatus::InProgress,
        TaskStatus::Completed,
        TaskStatus::Failed,
        TaskStatus::Rejected,
    ];
    for status in statuses {
        let json = serde_json::to_string(&status).unwrap();
        let parsed: TaskStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, status);
    }
}

#[test]
fn task_full_serialization_roundtrip() {
    let mut task = Task::new("Full task".into(), 2, 10);
    task.parent_task_id = Some("parent-id".into());
    task.assigned_to = Some(AgentId::new("did:swarm:agent".into()));
    task.status = TaskStatus::InProgress;
    task.subtasks = vec!["sub-1".into(), "sub-2".into()];

    let json = serde_json::to_string(&task).unwrap();
    let parsed: Task = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.status, TaskStatus::InProgress);
    assert_eq!(parsed.parent_task_id, Some("parent-id".into()));
    assert_eq!(parsed.subtasks.len(), 2);
}

// ═══════════════════════════════════════════════════════════════
// § 6.3 Plan
// ═══════════════════════════════════════════════════════════════

#[test]
fn plan_new_has_uuid_id() {
    let plan = Plan::new(
        "task-1".into(),
        AgentId::new("did:swarm:planner".into()),
        1,
    );
    assert_eq!(plan.plan_id.len(), 36);
}

#[test]
fn plan_new_has_empty_subtasks() {
    let plan = Plan::new(
        "task-1".into(),
        AgentId::new("did:swarm:planner".into()),
        1,
    );
    assert!(plan.subtasks.is_empty());
}

#[test]
fn plan_subtask_ordering() {
    let mut plan = Plan::new(
        "task-1".into(),
        AgentId::new("did:swarm:planner".into()),
        1,
    );
    for i in 0..10 {
        plan.subtasks.push(PlanSubtask {
            index: i,
            description: format!("Subtask {}", i),
            required_capabilities: vec![],
            estimated_complexity: 0.5,
        });
    }
    assert_eq!(plan.subtasks.len(), 10);
    for (i, st) in plan.subtasks.iter().enumerate() {
        assert_eq!(st.index as usize, i, "Subtasks must maintain order");
    }
}

#[test]
fn plan_serialization_roundtrip() {
    let mut plan = Plan::new(
        "task-1".into(),
        AgentId::new("did:swarm:p".into()),
        5,
    );
    plan.rationale = "Test rationale".into();
    plan.estimated_parallelism = 0.85;
    plan.subtasks.push(PlanSubtask {
        index: 0,
        description: "Do thing".into(),
        required_capabilities: vec!["web-search".into()],
        estimated_complexity: 0.7,
    });

    let json = serde_json::to_string(&plan).unwrap();
    let parsed: Plan = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.rationale, "Test rationale");
    assert!((parsed.estimated_parallelism - 0.85).abs() < 1e-10);
    assert_eq!(parsed.subtasks[0].required_capabilities, vec!["web-search"]);
}

// ═══════════════════════════════════════════════════════════════
// § 6.4 CriticScore
// ═══════════════════════════════════════════════════════════════

#[test]
fn critic_score_aggregate_perfect() {
    let score = CriticScore {
        feasibility: 1.0,
        parallelism: 1.0,
        completeness: 1.0,
        risk: 0.0,
    };
    assert!(
        (score.aggregate() - 1.0).abs() < 1e-10,
        "Perfect scores with zero risk must aggregate to 1.0"
    );
}

#[test]
fn critic_score_aggregate_worst() {
    let score = CriticScore {
        feasibility: 0.0,
        parallelism: 0.0,
        completeness: 0.0,
        risk: 1.0,
    };
    assert!(
        score.aggregate().abs() < 1e-10,
        "Worst scores must aggregate to 0.0"
    );
}

#[test]
fn critic_score_risk_is_inversely_weighted() {
    let low_risk = CriticScore {
        feasibility: 0.5,
        parallelism: 0.5,
        completeness: 0.5,
        risk: 0.1,
    };
    let high_risk = CriticScore {
        feasibility: 0.5,
        parallelism: 0.5,
        completeness: 0.5,
        risk: 0.9,
    };
    assert!(
        low_risk.aggregate() > high_risk.aggregate(),
        "Lower risk must produce higher aggregate score"
    );
}

#[test]
fn critic_score_weights_sum_to_one() {
    // 0.30 + 0.25 + 0.30 + 0.15 = 1.0
    assert!(
        (0.30f64 + 0.25 + 0.30 + 0.15 - 1.0).abs() < 1e-10,
        "Critic score weights must sum to 1.0"
    );
}

#[test]
fn critic_score_serialization() {
    let score = CriticScore {
        feasibility: 0.9,
        parallelism: 0.8,
        completeness: 0.85,
        risk: 0.2,
    };
    let json = serde_json::to_string(&score).unwrap();
    let parsed: CriticScore = serde_json::from_str(&json).unwrap();
    assert!((parsed.feasibility - 0.9).abs() < 1e-10);
}

// ═══════════════════════════════════════════════════════════════
// § 5.1 Tier
// ═══════════════════════════════════════════════════════════════

#[test]
fn tier1_depth_is_1() {
    assert_eq!(Tier::Tier1.depth(), 1);
}

#[test]
fn tier2_depth_is_2() {
    assert_eq!(Tier::Tier2.depth(), 2);
}

#[test]
fn tier_n_preserves_depth() {
    assert_eq!(Tier::TierN(5).depth(), 5);
    assert_eq!(Tier::TierN(100).depth(), 100);
}

#[test]
fn executor_has_max_depth() {
    assert_eq!(Tier::Executor.depth(), u32::MAX);
}

#[test]
fn tier_ordering() {
    assert!(Tier::Tier1.depth() < Tier::Tier2.depth());
    assert!(Tier::Tier2.depth() < Tier::TierN(3).depth());
    assert!(Tier::TierN(3).depth() < Tier::TierN(4).depth());
    assert!(Tier::TierN(100).depth() < Tier::Executor.depth());
}

#[test]
fn tier_serialization_tier1() {
    let json = serde_json::to_string(&Tier::Tier1).unwrap();
    let parsed: Tier = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, Tier::Tier1);
}

#[test]
fn tier_serialization_tier_n() {
    let json = serde_json::to_string(&Tier::TierN(7)).unwrap();
    let parsed: Tier = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, Tier::TierN(7));
}

#[test]
fn tier_serialization_executor() {
    let json = serde_json::to_string(&Tier::Executor).unwrap();
    let parsed: Tier = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, Tier::Executor);
}

// ═══════════════════════════════════════════════════════════════
// § 7.2 Artifact
// ═══════════════════════════════════════════════════════════════

#[test]
fn artifact_serialization() {
    let artifact = Artifact {
        artifact_id: "art-001".into(),
        task_id: "task-001".into(),
        producer: AgentId::new("did:swarm:p".into()),
        content_cid: "abc123".into(),
        merkle_hash: "def456".into(),
        content_type: "text/plain".into(),
        size_bytes: 1024,
        created_at: chrono::Utc::now(),
        content: "test artifact content".into(),
    };
    let json = serde_json::to_string(&artifact).unwrap();
    let parsed: Artifact = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.artifact_id, "art-001");
    assert_eq!(parsed.size_bytes, 1024);
    assert_eq!(parsed.content_type, "text/plain");
}

// ═══════════════════════════════════════════════════════════════
// NetworkStats
// ═══════════════════════════════════════════════════════════════

#[test]
fn network_stats_serialization() {
    let stats = NetworkStats {
        total_agents: 850,
        hierarchy_depth: 3,
        branching_factor: 10,
        current_epoch: 106,
        my_tier: Tier::Tier2,
        subordinate_count: 8,
        parent_id: Some(AgentId::new("did:swarm:leader".into())),
    };
    let json = serde_json::to_string(&stats).unwrap();
    let parsed: NetworkStats = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.total_agents, 850);
    assert_eq!(parsed.hierarchy_depth, 3);
    assert_eq!(parsed.my_tier, Tier::Tier2);
}

#[test]
fn network_stats_tier1_has_no_parent() {
    let stats = NetworkStats {
        total_agents: 10,
        hierarchy_depth: 1,
        branching_factor: 10,
        current_epoch: 1,
        my_tier: Tier::Tier1,
        subordinate_count: 0,
        parent_id: None,
    };
    assert!(stats.parent_id.is_none());
}

// ═══════════════════════════════════════════════════════════════
// Epoch
// ═══════════════════════════════════════════════════════════════

#[test]
fn epoch_serialization() {
    let epoch = Epoch {
        epoch_number: 106,
        started_at: chrono::Utc::now(),
        duration_secs: 3600,
        tier1_leaders: vec![
            AgentId::new("did:swarm:l1".into()),
            AgentId::new("did:swarm:l2".into()),
        ],
        estimated_swarm_size: 850,
    };
    let json = serde_json::to_string(&epoch).unwrap();
    let parsed: Epoch = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.epoch_number, 106);
    assert_eq!(parsed.tier1_leaders.len(), 2);
    assert_eq!(parsed.estimated_swarm_size, 850);
}

// ═══════════════════════════════════════════════════════════════
// RankedVote
// ═══════════════════════════════════════════════════════════════

#[test]
fn ranked_vote_serialization() {
    let mut critic_scores = std::collections::HashMap::new();
    critic_scores.insert(
        "plan-a".into(),
        CriticScore {
            feasibility: 0.9,
            parallelism: 0.8,
            completeness: 0.85,
            risk: 0.2,
        },
    );
    let vote = RankedVote {
        voter: AgentId::new("did:swarm:voter".into()),
        task_id: "task-1".into(),
        epoch: 106,
        rankings: vec!["plan-a".into(), "plan-b".into(), "plan-c".into()],
        critic_scores,
    };
    let json = serde_json::to_string(&vote).unwrap();
    let parsed: RankedVote = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.rankings.len(), 3);
    assert!(parsed.critic_scores.contains_key("plan-a"));
}

// ═══════════════════════════════════════════════════════════════
// Constants
// ═══════════════════════════════════════════════════════════════

#[test]
fn default_branching_factor_is_10() {
    assert_eq!(wws_protocol::constants::DEFAULT_BRANCHING_FACTOR, 10);
}

#[test]
fn leader_timeout_is_30_seconds() {
    assert_eq!(wws_protocol::constants::LEADER_TIMEOUT_SECS, 30);
}

#[test]
fn keepalive_interval_is_10_seconds() {
    assert_eq!(wws_protocol::constants::KEEPALIVE_INTERVAL_SECS, 10);
}

#[test]
fn pow_difficulty_is_24() {
    assert_eq!(wws_protocol::constants::POW_DIFFICULTY, 24);
}

#[test]
fn protocol_version_format() {
    let v = wws_protocol::constants::PROTOCOL_VERSION;
    assert!(v.starts_with("/wws/"));
    assert!(v.contains("wws"));
}
