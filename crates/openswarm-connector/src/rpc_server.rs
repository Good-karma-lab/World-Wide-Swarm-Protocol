//! JSON-RPC 2.0 server over TCP implementing the Swarm API.
//!
//! Provides the following methods for the local AI agent:
//! - `swarm.connect()` - Connect to a peer by multiaddress
//! - `swarm.get_network_stats()` - Get current network statistics
//! - `swarm.propose_plan()` - Submit a task decomposition plan
//! - `swarm.submit_result()` - Submit a task execution result
//! - `swarm.receive_task()` - Poll for assigned tasks
//! - `swarm.get_task()` - Get full details for a task by ID
//! - `swarm.get_task_timeline()` - Get lifecycle timeline for a task
//! - `swarm.get_status()` - Get connector and agent status
//! - `swarm.register_agent()` - Register an execution agent identity
//! - `swarm.list_swarms()` - List all known swarms with their info
//! - `swarm.create_swarm()` - Create a new private swarm
//! - `swarm.join_swarm()` - Join an existing swarm
//! - `swarm.register_name()` - Bind a human-readable name to a DID
//! - `swarm.resolve_name()` - Resolve a name to a DID
//! - `swarm.send_message()` - Send a direct message to another agent
//! - `swarm.get_messages()` - Retrieve inbox messages
//! - `swarm.get_reputation()` - Get reputation scores for an agent
//! - `swarm.get_reputation_events()` - Get paginated reputation event history
//! - `swarm.submit_reputation_event()` - Submit an observer-weighted reputation event
//! - `swarm.create_receipt()` - Create a commitment receipt at task start
//! - `swarm.fulfill_receipt()` - Agent proposes fulfillment + posts evidence_hash
//! - `swarm.verify_receipt()` - External verifier confirms or disputes receipt
//!
//! The server listens on localhost TCP and speaks JSON-RPC 2.0.
//! Each line received is a JSON-RPC request; each line sent is a response.

use std::sync::Arc;
use std::time::Duration;

use openswarm_consensus::rfp::RfpPhase;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::sync::RwLock;

use openswarm_protocol::*;

use crate::connector::{ConnectorState, SwarmRecord, TaskTimelineEvent, TaskVoteRequirement};

const ACTIVE_MEMBER_STALENESS_SECS: u64 = 45;
const PARTICIPATION_POLL_STALENESS_SECS: u64 = 180;

/// The JSON-RPC 2.0 server.
pub struct RpcServer {
    /// TCP listener address.
    bind_addr: String,
    /// Shared connector state.
    state: Arc<RwLock<ConnectorState>>,
    /// Network handle for network operations.
    network_handle: openswarm_network::SwarmHandle,
    /// Maximum concurrent connections.
    max_connections: usize,
}

impl RpcServer {
    /// Create a new RPC server.
    pub fn new(
        bind_addr: String,
        state: Arc<RwLock<ConnectorState>>,
        network_handle: openswarm_network::SwarmHandle,
        max_connections: usize,
    ) -> Self {
        Self {
            bind_addr,
            state,
            network_handle,
            max_connections,
        }
    }

    /// Start the RPC server, listening for connections.
    pub async fn run(self) -> Result<(), anyhow::Error> {
        let listener = TcpListener::bind(&self.bind_addr).await?;
        tracing::info!(addr = %self.bind_addr, "JSON-RPC server listening");

        let state = Arc::clone(&self.state);
        let network_handle = self.network_handle.clone();
        let semaphore = Arc::new(tokio::sync::Semaphore::new(self.max_connections));

        loop {
            let (stream, peer_addr) = listener.accept().await?;
            tracing::debug!(peer = %peer_addr, "RPC client connected");

            let state = Arc::clone(&state);
            let network_handle = network_handle.clone();
            let permit = semaphore.clone().acquire_owned().await?;

            tokio::spawn(async move {
                if let Err(e) =
                    handle_connection(stream, state, network_handle).await
                {
                    tracing::warn!(
                        peer = %peer_addr,
                        error = %e,
                        "RPC connection error"
                    );
                }
                drop(permit);
            });
        }
    }
}

/// Handle a single RPC client connection.
///
/// Reads newline-delimited JSON-RPC requests and sends back responses.
async fn handle_connection(
    stream: tokio::net::TcpStream,
    state: Arc<RwLock<ConnectorState>>,
    network_handle: openswarm_network::SwarmHandle,
) -> Result<(), anyhow::Error> {
    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();

    while let Some(line) = lines.next_line().await? {
        let response = process_request(&line, &state, &network_handle).await;
        let response_json = serde_json::to_string(&response)?;
        writer.write_all(response_json.as_bytes()).await?;
        writer.write_all(b"\n").await?;
        writer.flush().await?;
    }

    Ok(())
}

/// Process a single JSON-RPC request and return a response.
async fn process_request(
    request_str: &str,
    state: &Arc<RwLock<ConnectorState>>,
    network_handle: &openswarm_network::SwarmHandle,
) -> SwarmResponse {
    // Parse the request.
    let request: SwarmMessage = match serde_json::from_str(request_str) {
        Ok(r) => r,
        Err(e) => {
            return SwarmResponse::error(
                None,
                -32700, // Parse error
                format!("Invalid JSON: {}", e),
            );
        }
    };

    let request_id = request.id.clone();

    // Optional per-session RPC token check.
    if let Ok(required_token) = std::env::var("OPENSWARM_RPC_TOKEN") {
        if !required_token.trim().is_empty() {
            let provided = request.params.get("rpc_token").and_then(|v| v.as_str()).unwrap_or("");
            if provided != required_token.trim() {
                return SwarmResponse::error(
                    request_id.clone(),
                    -32001,
                    "Unauthorized: invalid or missing rpc_token".into(),
                );
            }
        }
    }

    match request.method.as_str() {
        "swarm.connect" => handle_connect(request_id, &request.params, network_handle).await,
        "swarm.get_network_stats" => handle_get_network_stats(request_id, state).await,
        "swarm.propose_plan" => {
            handle_propose_plan(request_id, &request.params, state, network_handle).await
        }
        "swarm.submit_vote" => {
            handle_submit_vote(request_id, &request.params, state, network_handle).await
        }
        "swarm.submit_critique" => {
            handle_submit_critique(request_id, &request.params, state, network_handle).await
        }
        "swarm.get_voting_state" => handle_get_voting_state(request_id, &request.params, state).await,
        "swarm.submit_result" => {
            handle_submit_result(request_id, &request.params, state, network_handle).await
        }
        "swarm.receive_task" => handle_receive_task(request_id, state).await,
        "swarm.get_task" => handle_get_task(request_id, &request.params, state).await,
        "swarm.get_task_timeline" => {
            handle_get_task_timeline(request_id, &request.params, state).await
        }
        "swarm.get_status" => handle_get_status(request_id, state).await,
        "swarm.register_agent" => {
            handle_register_agent(request_id, &request.params, state, network_handle).await
        }
        "swarm.list_swarms" => handle_list_swarms(request_id, state).await,
        "swarm.create_swarm" => {
            handle_create_swarm(request_id, &request.params, state).await
        }
        "swarm.join_swarm" => {
            handle_join_swarm(request_id, &request.params, state).await
        }
        "swarm.inject_task" => {
            handle_inject_task(request_id, &request.params, state, network_handle).await
        }
        "swarm.get_hierarchy" => handle_get_hierarchy(request_id, state).await,
        "swarm.get_board_status" => handle_get_board_status(request_id, state).await,
        "swarm.get_deliberation" => {
            handle_get_deliberation(request_id, &request.params, state).await
        }
        "swarm.get_ballots" => {
            handle_get_ballots(request_id, &request.params, state).await
        }
        "swarm.get_irv_rounds" => {
            handle_get_irv_rounds(request_id, &request.params, state).await
        }
        "swarm.register_name" => {
            handle_register_name(request_id, &request.params, state).await
        }
        "swarm.resolve_name" => {
            handle_resolve_name(request_id, &request.params, state).await
        }
        "swarm.send_message" => {
            handle_send_message(request_id, &request.params, state, network_handle).await
        }
        "swarm.get_messages" => {
            handle_get_messages(request_id, state).await
        }
        "swarm.get_reputation" => {
            handle_get_reputation(request_id, &request.params, state).await
        }
        "swarm.get_reputation_events" => {
            handle_get_reputation_events(request_id, &request.params, state).await
        }
        "swarm.submit_reputation_event" => {
            handle_submit_reputation_event(request_id, &request.params, state).await
        }
        "swarm.rotate_key" => {
            handle_rotate_key(request_id, &request.params, state).await
        }
        "swarm.emergency_revocation" => {
            handle_emergency_revocation(request_id, &request.params, state).await
        }
        "swarm.register_guardians" => {
            handle_register_guardians(request_id, &request.params, state).await
        }
        "swarm.guardian_recovery_vote" => {
            handle_guardian_recovery_vote(request_id, &request.params, state).await
        }
        "swarm.get_identity" => {
            handle_get_identity(request_id, &request.params, state).await
        }
        "swarm.create_receipt" => handle_create_receipt(request_id, &request.params, state).await,
        "swarm.fulfill_receipt" => handle_fulfill_receipt(request_id, &request.params, state).await,
        "swarm.verify_receipt" => handle_verify_receipt(request_id, &request.params, state).await,
        _ => SwarmResponse::error(
            request_id,
            -32601, // Method not found
            format!("Unknown method: {}", request.method),
        ),
    }
}

/// Handle `swarm.submit_vote` - submit a ranked vote for a task.
async fn handle_submit_vote(
    id: Option<String>,
    params: &serde_json::Value,
    state: &Arc<RwLock<ConnectorState>>,
    network_handle: &openswarm_network::SwarmHandle,
) -> SwarmResponse {
    let task_id = match params.get("task_id").and_then(|v| v.as_str()) {
        Some(v) if !v.is_empty() => v.to_string(),
        _ => {
            return SwarmResponse::error(id, -32602, "Missing 'task_id' parameter".to_string());
        }
    };

    // Accept either "rankings" or "ranked_plan_ids" as parameter name
    let rankings: Vec<String> = match params.get("rankings").or_else(|| params.get("ranked_plan_ids")).and_then(|v| v.as_array()) {
        Some(arr) if !arr.is_empty() => arr
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect(),
        _ => {
            return SwarmResponse::error(
                id,
                -32602,
                "Missing or empty 'rankings' or 'ranked_plan_ids' parameter".to_string(),
            );
        }
    };

    let epoch = {
        let state = state.read().await;
        params
            .get("epoch")
            .and_then(|v| v.as_u64())
            .unwrap_or_else(|| state.epoch_manager.current_epoch())
    };

    let (voter, swarm_id, ballot_count, proposal_count, accepted_rankings) = {
        let mut state = state.write().await;
        let voter = state.agent_id.clone();

        let proposals: std::collections::HashMap<String, AgentId> = rankings
            .iter()
            .map(|plan_id| {
                (
                    plan_id.clone(),
                    AgentId::new(format!("did:swarm:proposal-owner:{}", plan_id)),
                )
            })
            .collect();

        let (ballot_count, proposal_count, accepted_rankings) = {
            let voting = state.voting_engines.entry(task_id.clone()).or_insert_with(|| {
                let engine = openswarm_consensus::VotingEngine::new(
                    openswarm_consensus::voting::VotingConfig::default(),
                    task_id.clone(),
                    epoch,
                );
                engine
            });
            voting.set_proposals(proposals.clone());

            let mut accepted_rankings = rankings.clone();
            let mut attempts_left = accepted_rankings.len().max(1);
            loop {
                let ranked_vote = RankedVote {
                    voter: voter.clone(),
                    task_id: task_id.clone(),
                    epoch,
                    rankings: accepted_rankings.clone(),
                    critic_scores: std::collections::HashMap::new(),
                };

                match voting.record_vote(ranked_vote) {
                    Ok(()) => break,
                    Err(openswarm_consensus::ConsensusError::SelfVoteProhibited(_))
                        if accepted_rankings.len() > 1 && attempts_left > 1 =>
                    {
                        accepted_rankings.rotate_left(1);
                        attempts_left -= 1;
                    }
                    Err(e) => {
                        return SwarmResponse::error(
                            id,
                            -32000,
                            format!("Failed to record vote: {}", e),
                        );
                    }
                }
            }

            (
                voting.ballot_count(),
                voting.proposal_count(),
                accepted_rankings,
            )
        };

        state.push_task_timeline_event(
            &task_id,
            "vote_recorded",
            format!("Vote submitted via RPC: {}", accepted_rankings.join(" > ")),
            Some(voter.to_string()),
        );
        state.bump_votes_cast(voter.as_str());
        state.push_log(
            crate::tui::LogCategory::Vote,
                format!(
                    "Vote submitted for task {} by {} ({})",
                    task_id,
                    voter,
                    accepted_rankings.join(" > ")
                ),
        );
        // Record own ballot immediately (P2P won't echo back to sender)
        state.ballot_records.entry(task_id.clone()).or_default().push(BallotRecord {
            task_id: task_id.clone(),
            voter: voter.clone(),
            rankings: accepted_rankings.clone(),
            critic_scores: std::collections::HashMap::new(),
            timestamp: chrono::Utc::now(),
            irv_round_when_eliminated: None,
        });

        (
            voter,
            state.current_swarm_id.as_str().to_string(),
            ballot_count,
            proposal_count,
            accepted_rankings,
        )
    };

    let vote_msg = SwarmMessage::new(
        ProtocolMethod::ConsensusVote.as_str(),
        serde_json::json!({
            "task_id": task_id,
            "voter": voter,
            "epoch": epoch,
            "rankings": accepted_rankings,
            "critic_scores": {},
        }),
        String::new(),
    );

    if let Ok(data) = serde_json::to_vec(&vote_msg) {
        let topic = SwarmTopics::voting_for(&swarm_id, &task_id);
        let _ = network_handle.publish(&topic, data).await;
    }

    SwarmResponse::success(
        id,
        serde_json::json!({
            "task_id": task_id,
            "accepted": true,
            "ballot_count": ballot_count,
            "proposal_count": proposal_count,
        }),
    )
}

/// Handle `swarm.submit_critique` - submit critique scores for proposals after voting.
///
/// Agent calls this after voting to score each proposal on feasibility/parallelism/completeness/risk.
/// Connector records the critique, broadcasts a `discussion.critique` P2P message, and
/// updates the holon status to Voting.
async fn handle_submit_critique(
    id: Option<String>,
    params: &serde_json::Value,
    state: &Arc<RwLock<ConnectorState>>,
    network_handle: &openswarm_network::SwarmHandle,
) -> SwarmResponse {
    let task_id = match params.get("task_id").and_then(|v| v.as_str()) {
        Some(v) if !v.is_empty() => v.to_string(),
        _ => {
            return SwarmResponse::error(id, -32602, "Missing 'task_id' parameter".to_string());
        }
    };

    let content = params
        .get("content")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let round = params
        .get("round")
        .and_then(|v| v.as_u64())
        .unwrap_or(2) as u32;

    let plan_scores: std::collections::HashMap<String, CriticScore> = match params
        .get("plan_scores")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
    {
        Some(scores) => scores,
        None => {
            return SwarmResponse::error(
                id,
                -32602,
                "Missing or invalid 'plan_scores' parameter".to_string(),
            );
        }
    };

    let (voter, swarm_id) = {
        let mut state = state.write().await;
        let voter = state.agent_id.clone();

        // Record critique in RFP coordinator (transition to CritiquePhase first if needed)
        if let Some(rfp) = state.rfp_coordinators.get_mut(&task_id) {
            let _ = rfp.transition_to_critique();
            let _ = rfp.record_critique(voter.clone(), plan_scores.clone(), content.clone());
        }

        // Store as a CritiqueFeedback DeliberationMessage
        let msg = DeliberationMessage {
            id: uuid::Uuid::new_v4().to_string(),
            task_id: task_id.clone(),
            timestamp: chrono::Utc::now(),
            speaker: voter.clone(),
            round,
            message_type: DeliberationType::CritiqueFeedback,
            content: content.clone(),
            referenced_plan_id: None,
            critic_scores: Some(plan_scores.clone()),
        };
        state
            .deliberation_messages
            .entry(task_id.clone())
            .or_default()
            .push(msg);

        // Advance holon status to Voting once critique phase begins
        if let Some(holon) = state.active_holons.get_mut(&task_id) {
            if matches!(
                holon.status,
                HolonStatus::Forming | HolonStatus::Deliberating
            ) {
                holon.status = HolonStatus::Voting;
            }
        }

        // Update own BallotRecord with critic_scores from this critique
        if let Some(ballots) = state.ballot_records.get_mut(&task_id) {
            if let Some(own_ballot) = ballots.iter_mut().find(|b| b.voter == voter) {
                own_ballot.critic_scores = plan_scores.clone();
            }
        }

        state.push_task_timeline_event(
            &task_id,
            "critique_submitted",
            format!(
                "Critique submitted for {} proposals by {}",
                plan_scores.len(),
                voter
            ),
            Some(voter.to_string()),
        );
        state.push_log(
            crate::tui::LogCategory::Vote,
            format!(
                "Critique submitted for task {} by {} ({} plans scored)",
                task_id, voter, plan_scores.len()
            ),
        );

        (voter, state.current_swarm_id.as_str().to_string())
    };

    // Broadcast discussion.critique P2P message so all board members receive it
    let critique_params = DiscussionCritiqueParams {
        task_id: task_id.clone(),
        voter_id: voter.clone(),
        round,
        plan_scores,
        content,
    };
    let msg = SwarmMessage::new(
        ProtocolMethod::DiscussionCritique.as_str(),
        serde_json::to_value(&critique_params).unwrap_or_default(),
        String::new(),
    );
    if let Ok(data) = serde_json::to_vec(&msg) {
        let topic = SwarmTopics::voting_for(&swarm_id, &task_id);
        let _ = network_handle.publish(&topic, data).await;
    }

    SwarmResponse::success(id, serde_json::json!({ "ok": true, "task_id": task_id }))
}

/// Handle `swarm.get_voting_state` - inspect voting and proposal state.
async fn handle_get_voting_state(
    id: Option<String>,
    params: &serde_json::Value,
    state: &Arc<RwLock<ConnectorState>>,
) -> SwarmResponse {
    let maybe_task_id = params
        .get("task_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let state = state.read().await;

    let voting_entries: Vec<serde_json::Value> = state
        .voting_engines
        .iter()
        .filter(|(task_id, _)| maybe_task_id.as_ref().map(|t| t == *task_id).unwrap_or(true))
        .map(|(task_id, voting)| {
            serde_json::json!({
                "task_id": task_id,
                "proposal_count": voting.proposal_count(),
                "ballot_count": voting.ballot_count(),
                "quorum_reached": voting.ballot_count() >= voting.proposal_count() && voting.ballot_count() > 0,
            })
        })
        .collect();

    let rfp_entries: Vec<serde_json::Value> = state
        .rfp_coordinators
        .iter()
        .filter(|(task_id, _)| maybe_task_id.as_ref().map(|t| t == *task_id).unwrap_or(true))
        .map(|(task_id, rfp)| {
            let plan_ids: Vec<String> = rfp
                .reveals
                .values()
                .map(|r| r.plan.plan_id.clone())
                .collect();
            serde_json::json!({
                "task_id": task_id,
                "phase": format!("{:?}", rfp.phase()),
                "commit_count": rfp.commit_count(),
                "reveal_count": rfp.reveal_count(),
                "plan_ids": plan_ids,
            })
        })
        .collect();

    SwarmResponse::success(
        id,
        serde_json::json!({
            "voting_engines": voting_entries,
            "rfp_coordinators": rfp_entries,
        }),
    )
}

/// Handle `swarm.connect` - connect to a peer by multiaddress.
async fn handle_connect(
    id: Option<String>,
    params: &serde_json::Value,
    network_handle: &openswarm_network::SwarmHandle,
) -> SwarmResponse {
    let addr_str = match params.get("addr").and_then(|v| v.as_str()) {
        Some(a) => a,
        None => {
            return SwarmResponse::error(
                id,
                -32602,
                "Missing 'addr' parameter".into(),
            );
        }
    };

    let addr: openswarm_network::Multiaddr = match addr_str.parse() {
        Ok(a) => a,
        Err(e) => {
            return SwarmResponse::error(
                id,
                -32602,
                format!("Invalid multiaddress: {}", e),
            );
        }
    };

    match network_handle.dial(addr).await {
        Ok(()) => SwarmResponse::success(id, serde_json::json!({"connected": true})),
        Err(e) => SwarmResponse::error(id, -32000, format!("Dial failed: {}", e)),
    }
}

/// Handle `swarm.get_network_stats` - return current network statistics.
async fn handle_get_network_stats(
    id: Option<String>,
    state: &Arc<RwLock<ConnectorState>>,
) -> SwarmResponse {
    let state = state.read().await;
    let mut stats = state.network_stats.clone();
    stats.total_agents =
        state.active_member_count(Duration::from_secs(ACTIVE_MEMBER_STALENESS_SECS)) as u64;

    SwarmResponse::success(
        id,
        serde_json::to_value(&stats).unwrap_or_default(),
    )
}

/// Handle `swarm.propose_plan` - submit a task decomposition plan.
pub(crate) async fn handle_propose_plan(
    id: Option<String>,
    params: &serde_json::Value,
    state: &Arc<RwLock<ConnectorState>>,
    network_handle: &openswarm_network::SwarmHandle,
) -> SwarmResponse {
    let mut plan: Plan = match serde_json::from_value(params.clone()) {
        Ok(p) => p,
        Err(e) => {
            return SwarmResponse::error(
                id,
                -32602,
                format!("Invalid plan: {}", e),
            );
        }
    };

    {
        let state = state.read().await;
        plan.proposer = state.agent_id.clone();
    }

    if plan.subtasks.is_empty() {
        return SwarmResponse::error(
            id,
            -32013,
            "Plan must include at least one subtask".to_string(),
        );
    }

    let plan_hash = match openswarm_consensus::RfpCoordinator::compute_plan_hash(&plan) {
        Ok(h) => h,
        Err(e) => {
            return SwarmResponse::error(
                id,
                -32000,
                format!("Hash computation failed: {}", e),
            );
        }
    };

    let (swarm_id, has_task, subtask_count, reveals_to_publish) = {
        let mut state = state.write().await;

        let task = state.task_details.get(&plan.task_id).cloned().unwrap_or_else(|| Task {
            task_id: plan.task_id.clone(),
            parent_task_id: None,
            epoch: plan.epoch,
            status: TaskStatus::ProposalPhase,
            description: "Task proposed for decomposition".to_string(),
            assigned_to: Some(plan.proposer.clone()),
            tier_level: 1,
            subtasks: plan
                .subtasks
                .iter()
                .map(|s| format!("{}:{}", s.index, s.description))
                .collect(),
            created_at: chrono::Utc::now(),
            deadline: None,
            ..Default::default()
        });

        let task_tier_level = state
            .task_details
            .get(&plan.task_id)
            .map(|t| t.tier_level)
            .unwrap_or(1);
        let tier = tier_from_level(task_tier_level);
        let in_tier = active_members_in_tier(&state, tier, Duration::from_secs(ACTIVE_MEMBER_STALENESS_SECS)).len();
        let expected_proposers = if in_tier > 0 {
            in_tier
        } else {
            // No agents of the required tier (small swarm); fall back to all active agents
            state.active_member_ids(Duration::from_secs(ACTIVE_MEMBER_STALENESS_SECS)).len()
        }.max(1);

        let commit = ProposalCommitParams {
            task_id: plan.task_id.clone(),
            proposer: plan.proposer.clone(),
            epoch: plan.epoch,
            plan_hash: plan_hash.clone(),
        };

        let reveal_phase_ready = {
            let coordinator = state
                .rfp_coordinators
                .entry(plan.task_id.clone())
                .or_insert_with(|| {
                    openswarm_consensus::RfpCoordinator::new(
                        plan.task_id.clone(),
                        plan.epoch,
                        expected_proposers,
                    )
                });

            if matches!(coordinator.phase(), RfpPhase::Idle) {
                if let Err(e) = coordinator.inject_task(&task) {
                    return SwarmResponse::error(
                        id,
                        -32000,
                        format!("Failed to initialize RFP: {}", e),
                    );
                }
            }

            if let Err(e) = coordinator.record_commit(&commit) {
                return SwarmResponse::error(
                    id,
                    -32000,
                    format!("Failed to record proposal commit: {}", e),
                );
            }

            matches!(coordinator.phase(), RfpPhase::RevealPhase)
        };

        state
            .pending_plan_reveals
            .entry(plan.task_id.clone())
            .or_default()
            .insert(plan.proposer.to_string(), plan.clone());

        let mut reveals_to_publish = Vec::new();
        if reveal_phase_ready {
            let mut pending_items = state
                .pending_plan_reveals
                .remove(&plan.task_id)
                .unwrap_or_default()
                .into_iter()
                .collect::<Vec<(String, Plan)>>();
            pending_items.sort_by(|a, b| a.0.cmp(&b.0));

            let mut reveal_errors = Vec::new();
            if let Some(coordinator) = state.rfp_coordinators.get_mut(&plan.task_id) {
                for (_, pending_plan) in pending_items {
                    let reveal = ProposalRevealParams {
                        task_id: plan.task_id.clone(),
                        plan: pending_plan,
                    };
                    if let Err(e) = coordinator.record_reveal(&reveal) {
                        reveal_errors.push(format!(
                            "Failed to record deferred proposal reveal for task {}: {}",
                            plan.task_id, e
                        ));
                    } else {
                        reveals_to_publish.push(reveal);
                    }
                }
            }
            for err in reveal_errors {
                state.push_log(crate::tui::LogCategory::Error, err);
            }
        }

        state.push_log(
            crate::tui::LogCategory::Task,
            format!(
                "Plan proposed for task {}: {} subtasks (plan {}) -> {}",
                plan.task_id,
                plan.subtasks.len(),
                plan.plan_id,
                plan
                    .subtasks
                    .iter()
                    .map(|s| format!("{}:{}", s.index, s.description))
                    .collect::<Vec<_>>()
                    .join(" | ")
            ),
        );
        state.push_task_timeline_event(
            &plan.task_id,
            "proposed",
            format!("Plan {} proposed with {} subtasks", plan.plan_id, plan.subtasks.len()),
            Some(plan.proposer.to_string()),
        );
        state.push_log(
            crate::tui::LogCategory::System,
            format!(
                "AUDIT plan.propose actor={} task_id={} plan_id={} subtasks={}",
                plan.proposer,
                plan.task_id,
                plan.plan_id,
                plan.subtasks.len()
            ),
        );

        (
            state.current_swarm_id.as_str().to_string(),
            state.task_details.contains_key(&plan.task_id),
            plan.subtasks.len(),
            reveals_to_publish,
        )
    };

    if !has_task {
        let mut state = state.write().await;
        state.task_details.insert(
            plan.task_id.clone(),
            Task {
                task_id: plan.task_id.clone(),
                parent_task_id: None,
                epoch: plan.epoch,
                status: TaskStatus::ProposalPhase,
                description: "Task proposed for decomposition".to_string(),
                assigned_to: Some(plan.proposer.clone()),
                tier_level: 1,
                subtasks: plan
                    .subtasks
                    .iter()
                    .map(|s| format!("{}:{}", s.index, s.description))
                    .collect(),
                created_at: chrono::Utc::now(),
                deadline: None,
                ..Default::default()
            },
        );
    }

    {
        let mut state = state.write().await;
        let task_tier_level = state
            .task_details
            .get(&plan.task_id)
            .map(|t| t.tier_level)
            .unwrap_or(1);
        let tier = tier_from_level(task_tier_level);
        let tier_members = active_members_in_tier(&state, tier, Duration::from_secs(ACTIVE_MEMBER_STALENESS_SECS));
        let expected = if !tier_members.is_empty() {
            tier_members.len()
        } else {
            state.active_member_ids(Duration::from_secs(ACTIVE_MEMBER_STALENESS_SECS)).len()
        }.max(1);
        state.task_vote_requirements.insert(
            plan.task_id.clone(),
            crate::connector::TaskVoteRequirement {
                expected_proposers: expected,
                expected_voters: expected,
                tier_level: task_tier_level,
            },
        );

        let proposal_owners: std::collections::HashMap<String, AgentId> = state
            .rfp_coordinators
            .get(&plan.task_id)
            .map(|rfp| {
                rfp.reveals
                    .values()
                    .map(|r| (r.plan.plan_id.clone(), r.plan.proposer.clone()))
                    .collect()
            })
            .unwrap_or_default();

        let has_proposals = !proposal_owners.is_empty();
        let voting = state.voting_engines.entry(plan.task_id.clone()).or_insert_with(|| {
            openswarm_consensus::VotingEngine::new(
                openswarm_consensus::voting::VotingConfig::default(),
                plan.task_id.clone(),
                plan.epoch,
            )
        });
        voting.set_proposals(proposal_owners);

        if let Some(task) = state.task_details.get_mut(&plan.task_id) {
            task.status = if has_proposals {
                TaskStatus::VotingPhase
            } else {
                TaskStatus::ProposalPhase
            };
        }
    }

    let proposals_topic = SwarmTopics::proposals_for(&swarm_id, &plan.task_id);
    let voting_topic = SwarmTopics::voting_for(&swarm_id, &plan.task_id);
    let results_topic = SwarmTopics::results_for(&swarm_id, &plan.task_id);

    if let Err(e) = network_handle.subscribe(&proposals_topic).await {
        tracing::debug!(error = %e, topic = %proposals_topic, "Failed to subscribe proposals topic");
    }
    if let Err(e) = network_handle.subscribe(&voting_topic).await {
        tracing::debug!(error = %e, topic = %voting_topic, "Failed to subscribe voting topic");
    }
    if let Err(e) = network_handle.subscribe(&results_topic).await {
        tracing::debug!(error = %e, topic = %results_topic, "Failed to subscribe results topic");
    }

    let commit_params = ProposalCommitParams {
        task_id: plan.task_id.clone(),
        proposer: plan.proposer.clone(),
        epoch: plan.epoch,
        plan_hash: plan_hash.clone(),
    };
    let commit_msg = SwarmMessage::new(
        ProtocolMethod::ProposalCommit.as_str(),
        serde_json::to_value(&commit_params).unwrap_or_default(),
        String::new(),
    );
    let commit_data = match serde_json::to_vec(&commit_msg) {
        Ok(data) => data,
        Err(e) => {
            return SwarmResponse::error(
                id,
                -32000,
                format!("Failed to serialize proposal commit: {}", e),
            );
        }
    };
    let commit_published = match network_handle.publish(&proposals_topic, commit_data).await {
        Ok(()) => true,
        Err(e) => {
            tracing::debug!(error = %e, topic = %proposals_topic, "Failed to publish proposal commit");
            false
        }
    };

    let current_reveal = ProposalRevealParams {
        task_id: plan.task_id.clone(),
        plan: plan.clone(),
    };
    let current_reveal_msg = SwarmMessage::new(
        ProtocolMethod::ProposalReveal.as_str(),
        serde_json::to_value(&current_reveal).unwrap_or_default(),
        String::new(),
    );
    let current_reveal_data = match serde_json::to_vec(&current_reveal_msg) {
        Ok(data) => data,
        Err(e) => {
            return SwarmResponse::error(
                id,
                -32000,
                format!("Failed to serialize proposal reveal: {}", e),
            );
        }
    };

    let mut reveal_published = match network_handle.publish(&proposals_topic, current_reveal_data).await {
        Ok(()) => true,
        Err(e) => {
            tracing::debug!(error = %e, topic = %proposals_topic, "Failed to publish proposal reveal");
            false
        }
    };

    for reveal_params in reveals_to_publish {
        let reveal_msg = SwarmMessage::new(
            ProtocolMethod::ProposalReveal.as_str(),
            serde_json::to_value(&reveal_params).unwrap_or_default(),
            String::new(),
        );
        let reveal_data = match serde_json::to_vec(&reveal_msg) {
            Ok(data) => data,
            Err(e) => {
                return SwarmResponse::error(
                    id,
                    -32000,
                    format!("Failed to serialize proposal reveal: {}", e),
                );
            }
        };
        match network_handle.publish(&proposals_topic, reveal_data).await {
            Ok(()) => {
                reveal_published = true;
            }
            Err(e) => {
                tracing::debug!(error = %e, topic = %proposals_topic, "Failed to publish proposal reveal");
            }
        }
    }

    {
        let mut state = state.write().await;
        state.bump_plans_proposed(plan.proposer.as_str());
        state.push_log(
            crate::tui::LogCategory::Task,
            format!(
                "Plan {} published for task {} (subtasks: {}, commit: {}, reveal: {})",
                plan.plan_id,
                plan.task_id,
                subtask_count,
                commit_published,
                reveal_published
            ),
        );
        state.push_task_timeline_event(
            &plan.task_id,
            "published",
            format!(
                "Plan {} published (commit={}, reveal={})",
                plan.plan_id, commit_published, reveal_published
            ),
            Some(plan.proposer.to_string()),
        );

    }

    SwarmResponse::success(
        id,
        serde_json::json!({
            "plan_id": plan.plan_id,
            "plan_hash": plan_hash,
            "task_id": plan.task_id,
            "accepted": true,
            "commit_published": commit_published,
            "reveal_published": reveal_published,
            "subtasks_created": subtask_count,
            "assignments_published": 0,
        }),
    )
}

/// Aggregate results from all subtasks of a parent task.
fn aggregate_subtask_results(state: &ConnectorState, parent_task_id: &str) -> Artifact {
    use sha2::{Digest, Sha256};

    let parent_task = state.task_details.get(parent_task_id);
    let subtask_ids = parent_task
        .map(|t| t.subtasks.clone())
        .unwrap_or_default();

    // Collect all subtask results
    let mut subtask_results = Vec::new();
    for subtask_id in &subtask_ids {
        if let Some(result) = state.task_results.get(subtask_id) {
            subtask_results.push(result.clone());
        }
    }

    // Create aggregated content (concatenate content CIDs)
    let aggregated_content = subtask_results
        .iter()
        .map(|r| format!("subtask:{} -> cid:{}", r.task_id, r.content_cid))
        .collect::<Vec<_>>()
        .join("\n");

    // Compute content-addressed ID for aggregated result
    let mut hasher = Sha256::new();
    hasher.update(aggregated_content.as_bytes());
    let content_cid = format!("{:x}", hasher.finalize());

    // Compute Merkle hash (aggregate subtask merkle hashes)
    let mut merkle_hasher = Sha256::new();
    for result in &subtask_results {
        merkle_hasher.update(result.merkle_hash.as_bytes());
    }
    let merkle_hash = format!("{:x}", merkle_hasher.finalize());

    Artifact {
        artifact_id: format!("{}-aggregated", parent_task_id),
        task_id: parent_task_id.to_string(),
        producer: state.agent_id.clone(),
        content_cid,
        merkle_hash,
        content_type: "application/json; aggregated".to_string(),
        size_bytes: aggregated_content.len() as u64,
        created_at: chrono::Utc::now(),
        content: aggregated_content,
    }
}

/// Handle `swarm.submit_result` - submit a task execution result.
pub(crate) async fn handle_submit_result(
    id: Option<String>,
    params: &serde_json::Value,
    state: &Arc<RwLock<ConnectorState>>,
    network_handle: &openswarm_network::SwarmHandle,
) -> SwarmResponse {
    let mut submission: ResultSubmissionParams = match serde_json::from_value(params.clone()) {
        Ok(s) => s,
        Err(e) => {
            return SwarmResponse::error(
                id,
                -32602,
                format!("Invalid result submission: {}", e),
            );
        }
    };

    {
        let state = state.read().await;
        submission.agent_id = state.agent_id.clone();
        submission.artifact.producer = state.agent_id.clone();
    }

    // Add to Merkle DAG and update task state.
    let (dag_nodes, parent_propagation_info) = {
        let mut state = state.write().await;

        if let Some(task) = state.task_details.get(&submission.task_id) {
            // Allow submission when:
            //  (a) task is assigned to this agent, OR
            //  (b) task has no assignee (root/coordinator task) — connector synthesizes on behalf
            //  (c) is_synthesis=true — coordinator synthesizing subtask results (any agent allowed)
            let is_synthesis = params.get("is_synthesis").and_then(|v| v.as_bool()).unwrap_or(false);
            let assignee_ok = is_synthesis
                || task.assigned_to.is_none()
                || task.assigned_to.as_ref() == Some(&submission.agent_id);
            if !assignee_ok {
                return SwarmResponse::error(
                    id,
                    -32012,
                    format!(
                        "Result submission ignored for {}: assignee {} is no longer current",
                        submission.task_id, submission.agent_id
                    ),
                );
            }
            if task.parent_task_id.is_none() && task.subtasks.is_empty() {
                return SwarmResponse::error(
                    id,
                    -32011,
                    format!(
                        "Root result submission blocked for {}: no decomposed subtasks",
                        submission.task_id
                    ),
                );
            }

            if !task.subtasks.is_empty() {
                let all_subtasks_done = task.subtasks.iter().all(|sub_id| {
                    state
                        .task_details
                        .get(sub_id)
                        .map(|t| t.status == TaskStatus::Completed)
                        .unwrap_or(false)
                });
                if !all_subtasks_done {
                    return SwarmResponse::error(
                        id,
                        -32010,
                        format!(
                            "Cannot submit aggregated result for {} before all subtasks are completed",
                            submission.task_id
                        ),
                    );
                }
            }
        }

        let parent_task_id = state
            .task_details
            .get(&submission.task_id)
            .and_then(|t| t.parent_task_id.clone());

        if let Some(task) = state.task_details.get_mut(&submission.task_id) {
            task.status = TaskStatus::Completed;
            task.assigned_to = Some(submission.agent_id.clone());
        }
        state.task_set.remove(&submission.task_id);
        state.bump_tasks_processed(submission.agent_id.as_str());
        state.mark_member_submitted_result(submission.agent_id.as_str());
        state.mark_member_seen(submission.agent_id.as_str());
        state.merkle_dag.add_leaf(
            submission.task_id.clone(),
            submission.artifact.content_cid.as_bytes(),
        );
        let nodes = state.merkle_dag.node_count();
        state.push_task_timeline_event(
            &submission.task_id,
            "result_submitted",
            format!("Artifact {} (dag_nodes={})", submission.artifact.artifact_id, nodes),
            Some(submission.agent_id.to_string()),
        );
        state.push_log(
            crate::tui::LogCategory::Task,
            format!(
                "Result submitted for task {} by {} (artifact {}, dag_nodes={})",
                submission.task_id,
                submission.agent_id,
                submission.artifact.artifact_id,
                nodes
            ),
        );
        state.push_log(
            crate::tui::LogCategory::System,
            format!(
                "AUDIT result.submit actor={} task_id={} artifact={}",
                submission.agent_id, submission.task_id, submission.artifact.artifact_id
            ),
        );

        let confidence_delta = params.get("confidence_delta").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let _task_outcome_str = params.get("task_outcome").and_then(|v| v.as_str()).unwrap_or("succeeded_fully").to_string();

        if confidence_delta > 0.2 {
            state.push_log(
                crate::tui::LogCategory::Swarm,
                format!("Agent confidence dropped {:.2} during task — review suggested", confidence_delta),
            );
        }

        // Store the result for potential aggregation
        state.task_results.insert(submission.task_id.clone(), submission.artifact.clone());
        let content_text = params
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if !content_text.trim().is_empty() {
            state
                .task_result_text
                .insert(submission.task_id.clone(), content_text.clone());
        }

        // If is_synthesis flag is set, record a SynthesisResult deliberation message
        // so it appears in the deliberation panel alongside critiques and proposals.
        if params
            .get("is_synthesis")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            let synth_msg = DeliberationMessage {
                id: uuid::Uuid::new_v4().to_string(),
                task_id: submission.task_id.clone(),
                timestamp: chrono::Utc::now(),
                speaker: submission.agent_id.clone(),
                round: 3,
                message_type: DeliberationType::SynthesisResult,
                content: content_text.clone(),
                referenced_plan_id: None,
                critic_scores: None,
            };
            state
                .deliberation_messages
                .entry(submission.task_id.clone())
                .or_default()
                .push(synth_msg);
        }

        let propagation_info = if let Some(parent_id) = parent_task_id {
            let parent_completed = state
                .task_details
                .get(&parent_id)
                .map(|parent| {
                    !parent.subtasks.is_empty()
                        && parent.subtasks.iter().all(|sub_id| {
                            state
                                .task_details
                                .get(sub_id)
                                .map(|t| t.status == TaskStatus::Completed)
                                .unwrap_or(false)
                        })
                })
                .unwrap_or(false);

            if parent_completed {
                // Aggregate results from all subtasks
                let aggregated_artifact = aggregate_subtask_results(&state, &parent_id);

                if let Some(parent) = state.task_details.get_mut(&parent_id) {
                    parent.status = TaskStatus::Completed;
                }
                state.task_set.remove(&parent_id);

                // Store aggregated result
                state.task_results.insert(parent_id.clone(), aggregated_artifact.clone());

                state.push_task_timeline_event(
                    &parent_id,
                    "aggregated",
                    format!("All subtasks completed; parent {} marked completed", parent_id),
                    Some(submission.agent_id.to_string()),
                );
                state.push_log(
                    crate::tui::LogCategory::Task,
                    format!("Parent task {} completed via subtask aggregation", parent_id),
                );

                // Get grandparent for recursive propagation
                let grandparent_id = state
                    .task_details
                    .get(&parent_id)
                    .and_then(|p| p.parent_task_id.clone());

                Some((parent_id, aggregated_artifact, grandparent_id))
            } else {
                None
            }
        } else {
            None
        };

        (nodes, propagation_info)
    };

    // Publish result to the results topic.
    let swarm_id = {
        let state = state.read().await;
        state.current_swarm_id.as_str().to_string()
    };
    let topic = SwarmTopics::results_for(&swarm_id, &submission.task_id);
    let msg = SwarmMessage::new(
        ProtocolMethod::ResultSubmission.as_str(),
        serde_json::to_value(&submission).unwrap_or_default(),
        String::new(),
    );
    if let Ok(data) = serde_json::to_vec(&msg) {
        if let Err(e) = network_handle.publish(&topic, data).await {
            tracing::warn!(error = %e, "Failed to publish result");
        }
    }

    // Hierarchical propagation: if parent was aggregated, submit aggregated result
    // for the parent task as a normal result event. If a grandparent exists,
    // recursive propagation will continue in the nested call.
    if let Some((parent_id, aggregated_artifact, grandparent_id)) = parent_propagation_info {
        let my_agent_id = {
            let state = state.read().await;
            state.agent_id.clone()
        };

        tracing::info!(
            parent_task_id = %parent_id,
            grandparent_task_id = ?grandparent_id,
            "Propagating aggregated result up hierarchy"
        );

        // Recursively submit aggregated result to grandparent
        let propagation_submission = ResultSubmissionParams {
            task_id: parent_id.clone(),
            agent_id: my_agent_id.clone(),
            artifact: aggregated_artifact,
            merkle_proof: vec![], // TODO: proper merkle proof
            is_synthesis: true,
        };

        // Recursively call handle_submit_result for the parent task
        let _ = Box::pin(handle_submit_result(
            None,
            &serde_json::to_value(&propagation_submission).unwrap_or_default(),
            state,
            network_handle,
        ))
        .await;
    }

    SwarmResponse::success(
        id,
        serde_json::json!({
            "task_id": submission.task_id,
            "artifact_id": submission.artifact.artifact_id,
            "accepted": true,
            "dag_nodes": dag_nodes,
        }),
    )
}

/// Handle `swarm.receive_task` - poll for assigned tasks.
async fn handle_receive_task(
    id: Option<String>,
    state: &Arc<RwLock<ConnectorState>>,
) -> SwarmResponse {
    let mut state = state.write().await;
    let my_id = state.agent_id.clone();
    state.mark_member_polled_tasks(my_id.as_str());
    let my_tier = state.my_tier;
    let my_tier_level = my_tier.depth();

    let mut tasks: Vec<&Task> = state
        .task_details
        .values()
        .filter(|task| {
            if !state.task_set.contains(&task.task_id) {
                return false;
            }

            if matches!(
                task.status,
                TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Rejected
            ) {
                return false;
            }

            match my_tier {
                Tier::Executor => task.assigned_to.as_ref() == Some(&my_id),
                _ => {
                    task.tier_level == my_tier_level
                        && (task.assigned_to.is_none() || task.assigned_to.as_ref() == Some(&my_id))
                }
            }
        })
        .collect();
    tasks.sort_by(|a, b| a.created_at.cmp(&b.created_at));
    let tasks: Vec<String> = tasks.into_iter().map(|t| t.task_id.clone()).collect();

    SwarmResponse::success(
        id,
        serde_json::json!({
            "pending_tasks": tasks,
            "agent_id": state.agent_id.to_string(),
            "tier": format!("{:?}", state.my_tier),
        }),
    )
}

/// Handle `swarm.get_task` - fetch full metadata for a task by ID.
async fn handle_get_task(
    id: Option<String>,
    params: &serde_json::Value,
    state: &Arc<RwLock<ConnectorState>>,
) -> SwarmResponse {
    let task_id = match params.get("task_id").and_then(|v| v.as_str()) {
        Some(t) if !t.trim().is_empty() => t,
        _ => {
            return SwarmResponse::error(
                id,
                -32602,
                "Missing 'task_id' parameter".into(),
            );
        }
    };

    let state = state.read().await;
    let task = match state.task_details.get(task_id) {
        Some(task) => task,
        None => {
            return SwarmResponse::error(
                id,
                -32004,
                format!("Task not found: {}", task_id),
            );
        }
    };

    SwarmResponse::success(
        id,
        serde_json::json!({
            "task": task,
            "is_pending": state.task_set.contains(&task.task_id),
        }),
    )
}

/// Handle `swarm.get_task_timeline` - fetch lifecycle events for a task.
async fn handle_get_task_timeline(
    id: Option<String>,
    params: &serde_json::Value,
    state: &Arc<RwLock<ConnectorState>>,
) -> SwarmResponse {
    let task_id = match params.get("task_id").and_then(|v| v.as_str()) {
        Some(t) if !t.trim().is_empty() => t,
        _ => {
            return SwarmResponse::error(
                id,
                -32602,
                "Missing 'task_id' parameter".into(),
            );
        }
    };

    let limit = params
        .get("limit")
        .and_then(|v| v.as_u64())
        .map(|v| v as usize)
        .unwrap_or(200)
        .min(1000);

    let state = state.read().await;
    let timeline: Vec<TaskTimelineEvent> = state
        .task_timelines
        .get(task_id)
        .cloned()
        .unwrap_or_default();
    let total = timeline.len();
    let start = total.saturating_sub(limit);
    let events = timeline.into_iter().skip(start).collect::<Vec<_>>();

    SwarmResponse::success(
        id,
        serde_json::json!({
            "task_id": task_id,
            "events": events,
            "event_count": total,
        }),
    )
}

/// Handle `swarm.get_status` - get connector and agent status.
async fn handle_get_status(
    id: Option<String>,
    state: &Arc<RwLock<ConnectorState>>,
) -> SwarmResponse {
    let state = state.read().await;
    let known_agents = state.active_member_count(Duration::from_secs(ACTIVE_MEMBER_STALENESS_SECS));

    SwarmResponse::success(
        id,
        serde_json::json!({
            "agent_id": state.agent_id.to_string(),
            "status": format!("{:?}", state.status),
            "tier": format!("{:?}", state.my_tier),
            "epoch": state.epoch_manager.current_epoch(),
            "parent_id": state.parent_id.as_ref().map(|p| p.to_string()),
            "active_tasks": state.task_set.len(),
            "known_agents": known_agents,
            "content_items": state.content_store.item_count(),
        }),
    )
}

/// Handle `swarm.register_agent` - register an execution agent identity.
async fn handle_register_agent(
    id: Option<String>,
    params: &serde_json::Value,
    state: &Arc<RwLock<ConnectorState>>,
    network_handle: &openswarm_network::SwarmHandle,
) -> SwarmResponse {
    let requested_agent_id = match params.get("agent_id").and_then(|v| v.as_str()) {
        Some(v) if !v.trim().is_empty() => v.trim().to_string(),
        _ => {
            return SwarmResponse::error(
                id,
                -32602,
                "Missing 'agent_id' parameter".into(),
            );
        }
    };

    let (known_agents, canonical_agent_id, swarm_id, epoch, hierarchy_assignments, announced_name) = {
        let mut state = state.write().await;
        let canonical_agent_id = state.agent_id.to_string();
        let requested_name = if requested_agent_id.starts_with("did:swarm:") {
            None
        } else {
            Some(requested_agent_id.as_str())
        };
        state.mark_member_seen_with_name(&canonical_agent_id, requested_name);
        state.push_log(
            crate::tui::LogCategory::System,
            format!(
                "Agent registered: requested={}, canonical={}",
                requested_agent_id, canonical_agent_id
            ),
        );

        let staleness = Duration::from_secs(ACTIVE_MEMBER_STALENESS_SECS);
        let active_members = state.active_member_ids(staleness);
        let swarm_size = active_members.len() as u64;
        let mut hierarchy_assignments = Vec::new();

        if swarm_size >= 2 {
            let mut sorted_agents: Vec<String> = active_members;
            sorted_agents.sort();

            state.agent_tiers.clear();
            state.agent_parents.clear();
            state.subordinates.clear();

            let k = dynamic_branching_factor(swarm_size) as usize;
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
                        state.agent_parents.insert(child_id.clone(), parent_id.clone());
                        state
                            .subordinates
                            .entry(parent_id.clone())
                            .or_default()
                            .push(child_id.clone());
                    }
                }
            }

            for member_id in &sorted_agents {
                let tier = state
                    .agent_tiers
                    .get(member_id)
                    .copied()
                    .unwrap_or(Tier::Executor);
                let parent = state.agent_parents.get(member_id).cloned();
                hierarchy_assignments.push((member_id.clone(), tier, parent));
            }

            let my_agent_id = state.agent_id.as_str().to_string();
            if let Some(my_tier) = state.agent_tiers.get(&my_agent_id).copied() {
                state.my_tier = my_tier;
                state.parent_id = state.agent_parents.get(&my_agent_id).cloned().map(AgentId::new);
                state.network_stats.my_tier = my_tier;
                state.network_stats.parent_id = state.parent_id.clone();
                state.network_stats.subordinate_count = state
                    .subordinates
                    .get(&my_agent_id)
                    .map(|s| s.len())
                    .unwrap_or(0) as u32;
            }
            state.network_stats.hierarchy_depth = levels as u32;
            state.current_layout = openswarm_hierarchy::PyramidAllocator::new(openswarm_hierarchy::pyramid::PyramidConfig {
                branching_factor: k as u32,
                max_depth: openswarm_protocol::MAX_HIERARCHY_DEPTH,
            })
            .compute_layout(swarm_size)
            .ok();
        }

        let announced_name = if let Some(existing) = state.agent_names.get(&canonical_agent_id) {
            if existing.starts_with("did:swarm:") {
                None
            } else {
                Some(existing.clone())
            }
        } else if requested_agent_id.starts_with("did:swarm:") {
            None
        } else {
            Some(requested_agent_id.clone())
        };

        (
            state.active_member_count(staleness),
            canonical_agent_id,
            state.current_swarm_id.as_str().to_string(),
            state.epoch_manager.current_epoch(),
            hierarchy_assignments,
            announced_name,
        )
    };

    // Publish keepalive
    let keepalive = KeepAliveParams {
        agent_id: AgentId::new(canonical_agent_id.clone()),
        agent_name: announced_name,
        last_task_poll_at: None,
        last_result_at: None,
        epoch,
        timestamp: chrono::Utc::now(),
    };
    let msg = SwarmMessage::new(
        ProtocolMethod::AgentKeepAlive.as_str(),
        serde_json::to_value(&keepalive).unwrap_or_default(),
        String::new(),
    );
    if let Ok(data) = serde_json::to_vec(&msg) {
        let topic = SwarmTopics::keepalive_for(&swarm_id);
        let _ = network_handle.publish(&topic, data).await;
    }

    // Broadcast tier assignments if hierarchy was recomputed
    if !hierarchy_assignments.is_empty() {
        let branch_size = dynamic_branching_factor(known_agents as u64);
        for (member_id, tier, parent) in hierarchy_assignments {
            let params = TierAssignmentParams {
                assigned_agent: AgentId::new(member_id),
                tier,
                parent_id: parent.map(|p| AgentId::new(p)).unwrap_or_else(|| AgentId::new("root".to_string())),
                epoch,
                branch_size,
            };

            let msg = SwarmMessage::new(
                ProtocolMethod::TierAssignment.as_str(),
                serde_json::to_value(&params).unwrap_or_default(),
                String::new(),
            );

            if let Ok(data) = serde_json::to_vec(&msg) {
                let topic = SwarmTopics::hierarchy_for(&swarm_id);
                let _ = network_handle.publish(&topic, data).await;
            }
        }
    }

    SwarmResponse::success(
        id,
        serde_json::json!({
            "registered": true,
            "agent_id": canonical_agent_id,
            "requested_agent_id": requested_agent_id,
            "known_agents": known_agents,
        }),
    )
}

fn dynamic_branching_factor(swarm_size: u64) -> u64 {
    let approx = (swarm_size as f64).sqrt().round() as u64;
    approx.clamp(3, 10)
}

fn tier_from_level(level: u32) -> Tier {
    match level {
        1 => Tier::Tier1,
        2 => Tier::Tier2,
        n => Tier::TierN(n),
    }
}

fn active_members_in_tier(
    state: &ConnectorState,
    tier: Tier,
    staleness: Duration,
) -> Vec<String> {
    let now = chrono::Utc::now();
    let poll_staleness = Duration::from_secs(PARTICIPATION_POLL_STALENESS_SECS);
    state
        .active_member_ids(staleness)
        .into_iter()
        .filter(|id| state.agent_tiers.get(id).copied().unwrap_or(Tier::Executor) == tier)
        .filter(|id| {
            state
                .member_last_task_poll
                .get(id)
                .and_then(|ts| now.signed_duration_since(*ts).to_std().ok())
                .map(|age| age <= poll_staleness)
                .unwrap_or(false)
        })
        .collect()
}

/// Handle `swarm.list_swarms` - list all known swarms with their info.
async fn handle_list_swarms(
    id: Option<String>,
    state: &Arc<RwLock<ConnectorState>>,
) -> SwarmResponse {
    let state = state.read().await;

    let swarms: Vec<serde_json::Value> = state
        .known_swarms
        .values()
        .map(|record| {
            serde_json::json!({
                "swarm_id": record.swarm_id.as_str(),
                "name": record.name,
                "is_public": record.is_public,
                "agent_count": record.agent_count,
                "joined": record.joined,
            })
        })
        .collect();

    SwarmResponse::success(
        id,
        serde_json::json!({
            "swarms": swarms,
            "current_swarm": state.current_swarm_id.as_str(),
        }),
    )
}

/// Handle `swarm.create_swarm` - create a new private swarm.
async fn handle_create_swarm(
    id: Option<String>,
    params: &serde_json::Value,
    state: &Arc<RwLock<ConnectorState>>,
) -> SwarmResponse {
    let name = match params.get("name").and_then(|v| v.as_str()) {
        Some(n) => n.to_string(),
        None => {
            return SwarmResponse::error(
                id,
                -32602,
                "Missing 'name' parameter".into(),
            );
        }
    };

    let secret = match params.get("secret").and_then(|v| v.as_str()) {
        Some(s) => s.to_string(),
        None => {
            return SwarmResponse::error(
                id,
                -32602,
                "Missing 'secret' parameter".into(),
            );
        }
    };

    let swarm_id = SwarmId::generate();
    let token = SwarmToken::generate(&swarm_id, &secret);

    let record = SwarmRecord {
        swarm_id: swarm_id.clone(),
        name: name.clone(),
        is_public: false,
        agent_count: 1,
        joined: true,
        last_seen: chrono::Utc::now(),
    };

    {
        let mut state = state.write().await;
        state
            .known_swarms
            .insert(swarm_id.as_str().to_string(), record);
    }

    SwarmResponse::success(
        id,
        serde_json::json!({
            "swarm_id": swarm_id.as_str(),
            "token": token.as_str(),
            "name": name,
        }),
    )
}

/// Handle `swarm.join_swarm` - join an existing swarm.
async fn handle_join_swarm(
    id: Option<String>,
    params: &serde_json::Value,
    state: &Arc<RwLock<ConnectorState>>,
) -> SwarmResponse {
    let swarm_id_str = match params.get("swarm_id").and_then(|v| v.as_str()) {
        Some(s) => s.to_string(),
        None => {
            return SwarmResponse::error(
                id,
                -32602,
                "Missing 'swarm_id' parameter".into(),
            );
        }
    };

    let token = params.get("token").and_then(|v| v.as_str()).map(String::from);

    let mut state = state.write().await;

    let record = match state.known_swarms.get_mut(&swarm_id_str) {
        Some(r) => r,
        None => {
            return SwarmResponse::error(
                id,
                -32001,
                format!("Unknown swarm: {}", swarm_id_str),
            );
        }
    };

    // Private swarms require a token.
    if !record.is_public && token.is_none() {
        return SwarmResponse::error(
            id,
            -32602,
            "Token required for private swarm".into(),
        );
    }

    record.joined = true;

    SwarmResponse::success(
        id,
        serde_json::json!({
            "swarm_id": swarm_id_str,
            "joined": true,
        }),
    )
}

/// Handle `swarm.inject_task` - inject a task into the swarm from the operator/external source.
pub(crate) async fn handle_inject_task(
    id: Option<String>,
    params: &serde_json::Value,
    state: &Arc<RwLock<ConnectorState>>,
    network_handle: &openswarm_network::SwarmHandle,
) -> SwarmResponse {
    let description = match params.get("description").and_then(|v| v.as_str()) {
        Some(d) => d.to_string(),
        None => {
            return SwarmResponse::error(
                id,
                -32602,
                "Missing 'description' parameter".into(),
            );
        }
    };

    if description.len() > 4096 {
        return SwarmResponse::error(id, -32602, "Task description too long (max 4096 chars)".into());
    }

    // Reputation gate: require a registered agent with at least 1 completed task.
    let injector_agent_id = params
        .get("injector_agent_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    {
        let s = state.read().await;
        match &injector_agent_id {
            None => {
                return SwarmResponse::error(
                    id,
                    -32602,
                    "Missing 'injector_agent_id': only registered agents with good standing can inject tasks".into(),
                );
            }
            Some(agent_id) => {
                if !s.has_inject_reputation(agent_id) {
                    return SwarmResponse::error(
                        id,
                        -32003,
                        format!(
                            "insufficient_reputation: agent '{}' needs Member tier (score >= 100) to inject tasks",
                            agent_id
                        ),
                    );
                }
            }
        }
    }

    // Rate limit check.
    {
        let agent_id_for_rate = injector_agent_id.as_deref().unwrap_or("");
        let mut s = state.write().await;
        if !s.check_and_update_inject_rate_limit(agent_id_for_rate) {
            return SwarmResponse::error(
                id,
                -32029,
                format!(
                    "rate_limited: agent '{}' has exceeded the task injection rate limit (max 10 per 60s)",
                    agent_id_for_rate
                ),
            );
        }
    }

    let mut state_guard = state.write().await;
    let epoch = state_guard.epoch_manager.current_epoch();
    let mut task = openswarm_protocol::Task::new(description.clone(), 1, epoch);
    // Accept an optional pre-specified task_id (for multi-node injection with same ID)
    if let Some(v) = params.get("task_id").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
        task.task_id = v.to_string();
    }
    // Accept extended holonic task fields if provided
    if let Some(v) = params.get("task_type").and_then(|v| v.as_str()) {
        task.task_type = v.to_string();
    }
    if let Some(v) = params.get("horizon").and_then(|v| v.as_str()) {
        task.horizon = v.to_string();
    }
    if let Some(arr) = params.get("capabilities_required").and_then(|v| v.as_array()) {
        task.capabilities_required = arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect();
    }
    if let Some(v) = params.get("backtrack_allowed").and_then(|v| v.as_bool()) {
        task.backtrack_allowed = v;
    }
    if let Some(arr) = params.get("knowledge_domains").and_then(|v| v.as_array()) {
        task.knowledge_domains = arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect();
    }
    if let Some(arr) = params.get("tools_available").and_then(|v| v.as_array()) {
        task.tools_available = arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect();
    }
    let task_id = task.task_id.clone();

    // Add task to the local task set (CRDT).
    state_guard.task_set.add(task_id.clone());
    state_guard.task_details.insert(task_id.clone(), task.clone());
    let actor = state_guard.agent_id.to_string();
    state_guard.push_task_timeline_event(
        &task_id,
        "injected",
        format!("Task injected via RPC: {}", description),
        Some(actor),
    );

    // Log the injection.
    state_guard.push_log(
        crate::tui::LogCategory::Task,
        format!("Task injected via RPC: {} ({})", task_id, description),
    );
    let audit_actor = state_guard.agent_id.to_string();
    state_guard.push_log(
        crate::tui::LogCategory::System,
        format!(
            "AUDIT task.inject actor={} task_id={} description={}",
            audit_actor, task_id, description
        ),
    );

    let my_tier = state_guard.my_tier;
    let my_level = my_tier.depth();
    if my_tier != Tier::Executor && my_level == task.tier_level {
        let expected_participants = state_guard
            .active_member_ids(Duration::from_secs(ACTIVE_MEMBER_STALENESS_SECS))
            .into_iter()
            .filter(|id| state_guard.agent_tiers.get(id).copied().unwrap_or(Tier::Executor) == my_tier)
            .count()
            .max(1);

        let mut rfp = openswarm_consensus::RfpCoordinator::new(
            task_id.clone(),
            epoch,
            expected_participants,
        );
        if let Err(e) = rfp.inject_task(&task) {
            tracing::warn!(error = %e, task_id = %task_id, "Failed to initialize local RFP on inject");
        } else {
            state_guard.rfp_coordinators.insert(task_id.clone(), rfp);
            state_guard.task_vote_requirements.insert(
                task_id.clone(),
                TaskVoteRequirement {
                    expected_proposers: expected_participants,
                    expected_voters: expected_participants,
                    tier_level: task.tier_level,
                },
            );
            state_guard.push_log(
                crate::tui::LogCategory::Task,
                format!(
                    "Local RFP initialized for injected task {} (tier {:?}, expected participants: {})",
                    task_id, my_tier, expected_participants
                ),
            );
        }
    }

    // Publish task injection to the swarm network.
    let inject_params = TaskInjectionParams {
        task: task.clone(),
        originator: state_guard.agent_id.clone(),
    };

    let msg = SwarmMessage::new(
        ProtocolMethod::TaskInjection.as_str(),
        serde_json::to_value(&inject_params).unwrap_or_default(),
        String::new(),
    );

    let swarm_id = state_guard.current_swarm_id.as_str().to_string();
    drop(state_guard);

    // Fire-and-forget: publish task + subscribe to its topics in the background.
    // This prevents the inject RPC from blocking on swarm event loop replies under load.
    if let Ok(data) = serde_json::to_vec(&msg) {
        let nh = network_handle.clone();
        let task_topic = SwarmTopics::tasks_for(&swarm_id, 1);
        let proposals_topic = SwarmTopics::proposals_for(&swarm_id, &task_id);
        let voting_topic = SwarmTopics::voting_for(&swarm_id, &task_id);
        let results_topic = SwarmTopics::results_for(&swarm_id, &task_id);
        tokio::spawn(async move {
            if let Err(e) = nh.publish(&task_topic, data).await {
                tracing::debug!(error = %e, "Failed to publish task injection");
            }
            if let Err(e) = nh.subscribe(&proposals_topic).await {
                tracing::debug!(error = %e, topic = %proposals_topic, "Failed to subscribe proposals topic");
            }
            if let Err(e) = nh.subscribe(&voting_topic).await {
                tracing::debug!(error = %e, topic = %voting_topic, "Failed to subscribe voting topic");
            }
            if let Err(e) = nh.subscribe(&results_topic).await {
                tracing::debug!(error = %e, topic = %results_topic, "Failed to subscribe results topic");
            }
        });
    }

    SwarmResponse::success(
        id,
        serde_json::json!({
            "task_id": task_id,
            "description": description,
            "epoch": epoch,
            "injected": true,
        }),
    )
}

/// Handle `swarm.get_hierarchy` - return the agent hierarchy tree.
async fn handle_get_hierarchy(
    id: Option<String>,
    state: &Arc<RwLock<ConnectorState>>,
) -> SwarmResponse {
    let state = state.read().await;
    let active_members = state.active_member_ids(Duration::from_secs(ACTIVE_MEMBER_STALENESS_SECS));

    let self_agent = serde_json::json!({
        "agent_id": state.agent_id.to_string(),
        "tier": format!("{:?}", state.my_tier),
        "parent_id": state.parent_id.as_ref().map(|p| p.to_string()),
        "task_count": state.task_set.len(),
        "is_self": true,
    });

    let peers: Vec<serde_json::Value> = active_members
        .iter()
        .filter(|agent_id| *agent_id != &state.agent_id.to_string())
        .map(|peer_id| {
            let tier = state
                .agent_tiers
                .get(peer_id)
                .copied()
                .unwrap_or(Tier::Executor);
            let parent_id = state.agent_parents.get(peer_id).cloned();
            serde_json::json!({
                "agent_id": peer_id,
                "tier": format!("{:?}", tier),
                "parent_id": parent_id,
                "task_count": 0,
                "is_self": false,
            })
        })
        .collect();

    SwarmResponse::success(
        id,
        serde_json::json!({
            "self": self_agent,
            "peers": peers,
            "total_agents": active_members.len(),
            "hierarchy_depth": state.network_stats.hierarchy_depth,
            "branching_factor": state.network_stats.branching_factor,
            "epoch": state.epoch_manager.current_epoch(),
        }),
    )
}

/// Handle `swarm.get_board_status` - returns all active holons.
async fn handle_get_board_status(
    request_id: Option<String>,
    state: &Arc<RwLock<ConnectorState>>,
) -> SwarmResponse {
    let state = state.read().await;
    let holons: Vec<serde_json::Value> = state.active_holons.values().map(|h| {
        serde_json::json!({
            "task_id": h.task_id,
            "chair": h.chair.to_string(),
            "members": h.members.iter().map(|m| m.to_string()).collect::<Vec<_>>(),
            "adversarial_critic": h.adversarial_critic.as_ref().map(|a| a.to_string()),
            "depth": h.depth,
            "parent_holon": h.parent_holon,
            "child_holons": h.child_holons,
            "status": format!("{:?}", h.status),
            "created_at": h.created_at,
        })
    }).collect();
    SwarmResponse::success(request_id, serde_json::json!({ "holons": holons }))
}

/// Handle `swarm.get_deliberation` - returns deliberation messages for a task.
async fn handle_get_deliberation(
    request_id: Option<String>,
    params: &serde_json::Value,
    state: &Arc<RwLock<ConnectorState>>,
) -> SwarmResponse {
    let task_id = match params.get("task_id").and_then(|v| v.as_str()) {
        Some(id) => id.to_string(),
        None => return SwarmResponse::error(request_id, -32602, "task_id required".to_string()),
    };
    let state = state.read().await;
    let messages: Vec<serde_json::Value> = state.deliberation_messages
        .get(&task_id)
        .map(|msgs| msgs.iter().map(|m| serde_json::json!({
            "id": m.id,
            "task_id": m.task_id,
            "timestamp": m.timestamp,
            "speaker": m.speaker.to_string(),
            "round": m.round,
            "message_type": format!("{:?}", m.message_type),
            "content": m.content,
            "referenced_plan_id": m.referenced_plan_id,
            "critic_scores": m.critic_scores,
        })).collect())
        .unwrap_or_default();
    SwarmResponse::success(request_id, serde_json::json!({ "task_id": task_id, "messages": messages }))
}

/// Handle `swarm.get_ballots` - returns ballot records for a task.
async fn handle_get_ballots(
    request_id: Option<String>,
    params: &serde_json::Value,
    state: &Arc<RwLock<ConnectorState>>,
) -> SwarmResponse {
    let task_id = match params.get("task_id").and_then(|v| v.as_str()) {
        Some(id) => id.to_string(),
        None => return SwarmResponse::error(request_id, -32602, "task_id required".to_string()),
    };
    let state = state.read().await;
    let ballots: Vec<serde_json::Value> = state.ballot_records
        .get(&task_id)
        .map(|records| records.iter().map(|b| serde_json::json!({
            "task_id": b.task_id,
            "voter": b.voter.to_string(),
            "rankings": b.rankings,
            "critic_scores": b.critic_scores,
            "timestamp": b.timestamp,
            "irv_round_when_eliminated": b.irv_round_when_eliminated,
        })).collect())
        .unwrap_or_default();
    SwarmResponse::success(request_id, serde_json::json!({ "task_id": task_id, "ballots": ballots }))
}

/// Handle `swarm.get_irv_rounds` - returns IRV round history for a task.
async fn handle_get_irv_rounds(
    request_id: Option<String>,
    params: &serde_json::Value,
    state: &Arc<RwLock<ConnectorState>>,
) -> SwarmResponse {
    let task_id = match params.get("task_id").and_then(|v| v.as_str()) {
        Some(id) => id.to_string(),
        None => return SwarmResponse::error(request_id, -32602, "task_id required".to_string()),
    };
    let state = state.read().await;
    let rounds: Vec<serde_json::Value> = state.irv_rounds
        .get(&task_id)
        .map(|rounds| rounds.iter().map(|r| serde_json::json!({
            "task_id": r.task_id,
            "round_number": r.round_number,
            "tallies": r.tallies,
            "eliminated": r.eliminated,
            "continuing_candidates": r.continuing_candidates,
        })).collect())
        .unwrap_or_default();
    SwarmResponse::success(request_id, serde_json::json!({ "task_id": task_id, "irv_rounds": rounds }))
}

/// Handle `swarm.register_name` - bind a human-readable name to a DID in the local registry.
async fn handle_register_name(
    id: Option<String>,
    params: &serde_json::Value,
    state: &Arc<RwLock<ConnectorState>>,
) -> SwarmResponse {
    let name = match params.get("name").and_then(|v| v.as_str()) {
        Some(v) if !v.trim().is_empty() => v.trim().to_string(),
        _ => return SwarmResponse::error(id, -32602, "Missing 'name' parameter".into()),
    };
    let did = match params.get("did").and_then(|v| v.as_str()) {
        Some(v) if !v.trim().is_empty() => v.trim().to_string(),
        _ => return SwarmResponse::error(id, -32602, "Missing 'did' parameter".into()),
    };
    let mut s = state.write().await;
    s.name_registry.insert(name.clone(), did.clone());
    SwarmResponse::success(id, serde_json::json!({ "registered": true, "name": name, "did": did }))
}

/// Handle `swarm.resolve_name` - look up a DID by human-readable name.
async fn handle_resolve_name(
    id: Option<String>,
    params: &serde_json::Value,
    state: &Arc<RwLock<ConnectorState>>,
) -> SwarmResponse {
    let name = match params.get("name").and_then(|v| v.as_str()) {
        Some(v) if !v.trim().is_empty() => v.trim().to_string(),
        _ => return SwarmResponse::error(id, -32602, "Missing 'name' parameter".into()),
    };
    let s = state.read().await;
    match s.name_registry.get(&name) {
        Some(did) => SwarmResponse::success(id, serde_json::json!({ "name": name, "did": did })),
        None => SwarmResponse::error(id, -32001, format!("Name not found: {}", name)),
    }
}

/// Handle `swarm.send_message` - send a direct message to another agent.
///
/// Publishes an `agent.direct_message` on the shared DM GossipSub topic.
/// The receiving agent's connector filters by the `to` field and stores it
/// in its inbox.
///
/// Required params: `to` (recipient DID), `content` (message text).
async fn handle_send_message(
    id: Option<String>,
    params: &serde_json::Value,
    state: &Arc<RwLock<ConnectorState>>,
    network_handle: &openswarm_network::SwarmHandle,
) -> SwarmResponse {
    let to = match params.get("to").and_then(|v| v.as_str()) {
        Some(v) if !v.trim().is_empty() => v.trim().to_string(),
        _ => return SwarmResponse::error(id, -32602, "Missing 'to' parameter".into()),
    };
    let content = match params.get("content").and_then(|v| v.as_str()) {
        Some(v) if !v.trim().is_empty() => v.trim().to_string(),
        _ => return SwarmResponse::error(id, -32602, "Missing 'content' parameter".into()),
    };

    let (from, swarm_id) = {
        let s = state.read().await;
        (s.agent_id.to_string(), s.current_swarm_id.as_str().to_string())
    };

    let topic = SwarmTopics::dm_for(&swarm_id);
    let msg = SwarmMessage::new(
        ProtocolMethod::DirectMessage.as_str(),
        serde_json::json!({ "from": from, "to": to, "content": content }),
        String::new(),
    );
    // Fire-and-forget: publish in background so this RPC returns immediately under load.
    if let Ok(data) = serde_json::to_vec(&msg) {
        let nh = network_handle.clone();
        tokio::spawn(async move {
            if let Err(e) = nh.publish(&topic, data).await {
                tracing::debug!(error = %e, "Failed to publish direct message");
            }
        });
    }

    SwarmResponse::success(id, serde_json::json!({ "ok": true, "sent": true, "to": to }))
}

/// Handle `swarm.get_messages` - retrieve all messages in the inbox.
///
/// Returns messages addressed to this agent's DID that arrived since startup.
async fn handle_get_messages(
    id: Option<String>,
    state: &Arc<RwLock<ConnectorState>>,
) -> SwarmResponse {
    let s = state.read().await;
    let messages: Vec<serde_json::Value> = s
        .inbox
        .iter()
        .map(|m| {
            serde_json::json!({
                "from": m.from,
                "to": m.to,
                "content": m.content,
                "timestamp": m.timestamp.to_rfc3339(),
            })
        })
        .collect();
    SwarmResponse::success(id, serde_json::json!({ "messages": messages, "count": messages.len() }))
}

/// Handle `swarm.get_reputation` - get reputation scores for an agent.
///
/// Optional param: `did` (agent DID). Defaults to the local agent.
async fn handle_get_reputation(
    id: Option<String>,
    params: &serde_json::Value,
    state: &Arc<RwLock<ConnectorState>>,
) -> SwarmResponse {
    let did = params.get("did")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let s = state.read().await;
    let target = if did.is_empty() { s.agent_id.to_string() } else { did };
    let ledger = s.reputation_ledgers.get(&target);
    let (raw, effective, peak, tier, events_count, last_active) = match ledger {
        Some(l) => {
            let eff = l.effective_score();
            (l.raw_score, eff, l.peak_score,
             l.tier().as_str().to_string(),
             l.events.len(),
             l.last_active.to_rfc3339())
        }
        None => (0, 0, 0, "Newcomer".to_string(), 0, chrono::Utc::now().to_rfc3339()),
    };
    SwarmResponse::success(id, serde_json::json!({
        "did": target,
        "raw_score": raw,
        "effective_score": effective,
        "peak_score": peak,
        "tier": tier,
        "events_count": events_count,
        "last_active": last_active,
    }))
}

/// Handle `swarm.get_reputation_events` - get paginated reputation event history.
///
/// Optional params: `did`, `limit` (default 20), `offset` (default 0).
async fn handle_get_reputation_events(
    id: Option<String>,
    params: &serde_json::Value,
    state: &Arc<RwLock<ConnectorState>>,
) -> SwarmResponse {
    let did = params.get("did")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as usize;
    let offset = params.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let s = state.read().await;
    let target = if did.is_empty() { s.agent_id.to_string() } else { did };
    let ledger = s.reputation_ledgers.get(&target);
    let total = ledger.map(|l| l.events.len()).unwrap_or(0);
    let events: Vec<serde_json::Value> = ledger
        .map(|l| {
            l.events.iter().rev().skip(offset).take(limit)
                .map(|e| serde_json::json!({
                    "event_type": format!("{:?}", e.event_type),
                    "base_points": e.base_points,
                    "effective_points": e.effective_points,
                    "observer": e.observer,
                    "task_id": e.task_id,
                    "timestamp": e.timestamp.to_rfc3339(),
                }))
                .collect()
        })
        .unwrap_or_default();
    SwarmResponse::success(id, serde_json::json!({ "events": events, "total": total }))
}

/// Handle `swarm.submit_reputation_event` - submit an observer-weighted reputation event.
///
/// Required params: `submitter_did`, `target_did`, `event_type`.
/// Optional params: `task_id`, `evidence`.
/// Submitter must have Member tier (score >= 100) and pass rate limiting.
async fn handle_submit_reputation_event(
    id: Option<String>,
    params: &serde_json::Value,
    state: &Arc<RwLock<ConnectorState>>,
) -> SwarmResponse {
    use crate::reputation::{RepEvent, RepEventType, observer_weighted_points};

    let submitter = params.get("submitter_did")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let target_did = params.get("target_did")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let event_type_str = params.get("event_type")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let task_id = params.get("task_id")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let evidence = params.get("evidence")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    if target_did.is_empty() {
        return SwarmResponse::error(id, -32602, "missing target_did".to_string());
    }

    let mut s = state.write().await;

    // Submitter needs Member tier (score >= 100) to submit events
    let submitter_score = s.reputation_ledgers
        .get(&submitter)
        .map(|l| l.effective_score())
        .unwrap_or(0);
    if submitter_score < 100 {
        return SwarmResponse::error(id, -32603, "insufficient reputation to submit events".to_string());
    }

    // Rate limit: max 20 per hour per submitter
    if !s.check_rep_event_rate_limit(&submitter) {
        return SwarmResponse::error(id, -32604, "reputation event rate limit exceeded".to_string());
    }

    // Parse event type (only allow subjective positive events from external submitters)
    let event_type = match event_type_str.as_str() {
        "HighQualityResult" => RepEventType::HighQualityResult,
        "HelpedNewAgent" => RepEventType::HelpedNewAgent,
        _ => return SwarmResponse::error(id, -32602, "unsupported event_type for external submission".to_string()),
    };

    let base = event_type.base_points();
    let is_obj = event_type.is_objective();
    let effective = observer_weighted_points(base, submitter_score, is_obj);

    let event = RepEvent {
        event_type,
        base_points: base,
        observer: submitter.clone(),
        observer_score: submitter_score,
        effective_points: effective,
        task_id,
        timestamp: chrono::Utc::now(),
        evidence,
    };

    let ledger = s.reputation_ledgers.entry(target_did.clone()).or_default();
    ledger.apply_event(event);

    SwarmResponse::success(id, serde_json::json!({ "accepted": true, "effective_points": effective }))
}

async fn handle_rotate_key(
    id: Option<String>,
    params: &serde_json::Value,
    state: &Arc<RwLock<ConnectorState>>,
) -> SwarmResponse {
    use crate::connector::PendingKeyRotation;

    let agent_did = params.get("agent_did").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let old_pubkey_hex = params.get("old_pubkey_hex").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let new_pubkey_hex = params.get("new_pubkey_hex").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let rotation_timestamp = params.get("rotation_timestamp").and_then(|v| v.as_i64()).unwrap_or(0);

    if agent_did.is_empty() || new_pubkey_hex.is_empty() {
        return SwarmResponse::error(id, -32602, "missing required fields".to_string());
    }

    let grace_expires = chrono::Utc::now() + chrono::Duration::hours(48);
    let rotation = PendingKeyRotation {
        agent_did: agent_did.clone(),
        old_pubkey_hex,
        new_pubkey_hex,
        rotation_timestamp,
        grace_expires,
    };

    let mut s = state.write().await;
    s.pending_key_rotations.insert(agent_did, rotation);
    s.push_log(crate::tui::LogCategory::Swarm, "Key rotation registered".into());

    SwarmResponse::success(id, serde_json::json!({
        "accepted": true,
        "grace_expires": grace_expires.to_rfc3339(),
    }))
}

async fn handle_emergency_revocation(
    id: Option<String>,
    params: &serde_json::Value,
    state: &Arc<RwLock<ConnectorState>>,
) -> SwarmResponse {
    use crate::connector::PendingRevocation;

    let agent_did = params.get("agent_did").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let recovery_pubkey_hex = params.get("recovery_pubkey_hex").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let new_primary_pubkey_hex = params.get("new_primary_pubkey_hex").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let revocation_timestamp = params.get("revocation_timestamp").and_then(|v| v.as_i64()).unwrap_or(0);

    if agent_did.is_empty() || recovery_pubkey_hex.is_empty() || new_primary_pubkey_hex.is_empty() {
        return SwarmResponse::error(id, -32602, "missing required fields".to_string());
    }

    let challenge_expires = chrono::Utc::now() + chrono::Duration::hours(24);
    let revocation = PendingRevocation {
        agent_did: agent_did.clone(),
        recovery_pubkey_hex,
        new_primary_pubkey_hex,
        revocation_timestamp,
        challenge_expires,
    };

    let mut s = state.write().await;
    s.pending_revocations.insert(agent_did, revocation);
    s.push_log(crate::tui::LogCategory::Swarm, "Emergency revocation registered (24h challenge window)".into());

    SwarmResponse::success(id, serde_json::json!({
        "accepted": true,
        "challenge_expires": challenge_expires.to_rfc3339(),
    }))
}

async fn handle_register_guardians(
    id: Option<String>,
    params: &serde_json::Value,
    state: &Arc<RwLock<ConnectorState>>,
) -> SwarmResponse {
    use crate::connector::GuardianDesignation;

    let agent_did = params.get("agent_did").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let guardians: Vec<String> = params.get("guardians")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
        .unwrap_or_default();
    let threshold = params.get("threshold").and_then(|v| v.as_u64()).unwrap_or(2) as u32;

    if agent_did.is_empty() || guardians.is_empty() {
        return SwarmResponse::error(id, -32602, "missing agent_did or guardians".to_string());
    }
    if threshold as usize > guardians.len() {
        return SwarmResponse::error(id, -32602, "threshold exceeds guardian count".to_string());
    }

    let designation = GuardianDesignation { agent_did: agent_did.clone(), guardians, threshold };
    let mut s = state.write().await;
    s.guardian_designations.insert(agent_did, designation);

    SwarmResponse::success(id, serde_json::json!({ "registered": true }))
}

async fn handle_guardian_recovery_vote(
    id: Option<String>,
    params: &serde_json::Value,
    state: &Arc<RwLock<ConnectorState>>,
) -> SwarmResponse {
    use crate::connector::GuardianVote;

    let guardian_did = params.get("guardian_did").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let target_did = params.get("target_did").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let new_pubkey = params.get("new_pubkey").and_then(|v| v.as_str()).unwrap_or("").to_string();

    if guardian_did.is_empty() || target_did.is_empty() || new_pubkey.is_empty() {
        return SwarmResponse::error(id, -32602, "missing required fields".to_string());
    }

    // Guardian must have Trusted tier (score >= 500) per spec
    let guardian_score = {
        let s = state.read().await;
        s.reputation_ledgers.get(&guardian_did)
            .map(|l| l.effective_score())
            .unwrap_or(0)
    };
    if guardian_score < 500 {
        return SwarmResponse::error(id, -32603, "guardian needs Trusted tier (score >= 500)".to_string());
    }

    let mut s = state.write().await;

    // Check guardian is in the designated list
    let (threshold, is_guardian) = s.guardian_designations.get(&target_did)
        .map(|d| (d.threshold, d.guardians.contains(&guardian_did)))
        .unwrap_or((2, false));

    if !is_guardian {
        return SwarmResponse::error(id, -32603, "guardian not in designated list for this agent".to_string());
    }

    let vote = GuardianVote {
        guardian_did: guardian_did.clone(),
        target_did: target_did.clone(),
        new_pubkey: new_pubkey.clone(),
        timestamp: chrono::Utc::now().timestamp(),
    };

    let votes = s.guardian_votes.entry(target_did.clone()).or_default();
    // Prevent duplicate votes
    if !votes.iter().any(|v| v.guardian_did == guardian_did) {
        votes.push(vote);
    }

    let vote_count = s.guardian_votes.get(&target_did).map(|v| v.len()).unwrap_or(0);
    let threshold_met = vote_count >= threshold as usize;

    SwarmResponse::success(id, serde_json::json!({
        "accepted": true,
        "votes_collected": vote_count,
        "threshold": threshold,
        "threshold_met": threshold_met,
    }))
}

async fn handle_get_identity(
    id: Option<String>,
    params: &serde_json::Value,
    state: &Arc<RwLock<ConnectorState>>,
) -> SwarmResponse {
    let did = params.get("did").and_then(|v| v.as_str())
        .unwrap_or("").to_string();
    let s = state.read().await;
    let target = if did.is_empty() { s.agent_id.to_string() } else { did };

    let rotation = s.pending_key_rotations.get(&target).map(|r| serde_json::json!({
        "new_pubkey_hex": r.new_pubkey_hex,
        "grace_expires": r.grace_expires.to_rfc3339(),
    }));
    let revocation = s.pending_revocations.get(&target).map(|r| serde_json::json!({
        "challenge_expires": r.challenge_expires.to_rfc3339(),
    }));
    let guardians = s.guardian_designations.get(&target).map(|d| serde_json::json!({
        "guardians": d.guardians,
        "threshold": d.threshold,
    }));

    SwarmResponse::success(id, serde_json::json!({
        "did": target,
        "pending_rotation": rotation,
        "pending_revocation": revocation,
        "guardians": guardians,
    }))
}

/// Handle `swarm.create_receipt` — create a commitment receipt at task start.
async fn handle_create_receipt(
    id: Option<String>,
    params: &serde_json::Value,
    state: &Arc<RwLock<ConnectorState>>,
) -> SwarmResponse {
    let task_id = match params.get("task_id").and_then(|v| v.as_str()) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => return SwarmResponse::error(id, -32602, "Missing 'task_id'".into()),
    };
    let agent_id = match params.get("agent_id").and_then(|v| v.as_str()) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => return SwarmResponse::error(id, -32602, "Missing 'agent_id'".into()),
    };
    let deliverable_type = params
        .get("deliverable_type")
        .and_then(|v| v.as_str())
        .unwrap_or("artifact")
        .to_string();
    let rollback_cost = params
        .get("rollback_cost")
        .and_then(|v| v.as_str())
        .map(String::from);
    let rollback_window = params
        .get("rollback_window")
        .and_then(|v| v.as_str())
        .map(String::from);

    let receipt = openswarm_protocol::CommitmentReceipt {
        commitment_id: uuid::Uuid::new_v4().to_string(),
        deliverable_type,
        evidence_hash: String::new(),
        confidence_delta: 0.0,
        can_undo: rollback_cost.as_deref().map(|c| c != "high").unwrap_or(true),
        rollback_cost,
        rollback_window,
        expires_at: None,
        commitment_state: openswarm_protocol::CommitmentState::Active,
        task_id,
        agent_id,
        created_at: chrono::Utc::now(),
    };
    let receipt_id = receipt.commitment_id.clone();
    let mut s = state.write().await;
    s.receipts.insert(receipt_id.clone(), receipt);
    SwarmResponse::success(id, serde_json::json!({ "receipt_id": receipt_id, "ok": true }))
}

/// Handle `swarm.fulfill_receipt` — agent proposes fulfillment + evidence_hash.
async fn handle_fulfill_receipt(
    id: Option<String>,
    params: &serde_json::Value,
    state: &Arc<RwLock<ConnectorState>>,
) -> SwarmResponse {
    let receipt_id = match params.get("receipt_id").and_then(|v| v.as_str()) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => return SwarmResponse::error(id, -32602, "Missing 'receipt_id'".into()),
    };
    let evidence_hash = params
        .get("evidence_hash")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let confidence_delta = params
        .get("confidence_delta")
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);

    let mut s = state.write().await;
    match s.receipts.get_mut(&receipt_id) {
        Some(r) if r.commitment_state == openswarm_protocol::CommitmentState::Active => {
            r.commitment_state = openswarm_protocol::CommitmentState::AgentFulfilled;
            r.evidence_hash = evidence_hash;
            r.confidence_delta = confidence_delta;
            SwarmResponse::success(id, serde_json::json!({ "ok": true, "state": "AgentFulfilled" }))
        }
        Some(_) => SwarmResponse::error(id, -32600, "Receipt is not in Active state".into()),
        None => SwarmResponse::error(id, -32602, format!("Receipt '{}' not found", receipt_id)),
    }
}

/// Handle `swarm.verify_receipt` — external verifier confirms or disputes.
async fn handle_verify_receipt(
    id: Option<String>,
    params: &serde_json::Value,
    state: &Arc<RwLock<ConnectorState>>,
) -> SwarmResponse {
    let receipt_id = match params.get("receipt_id").and_then(|v| v.as_str()) {
        Some(s) if !s.is_empty() => s.to_string(),
        _ => return SwarmResponse::error(id, -32602, "Missing 'receipt_id'".into()),
    };
    let confirmed = params
        .get("confirmed")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let mut s = state.write().await;
    match s.receipts.get_mut(&receipt_id) {
        Some(r) if r.commitment_state == openswarm_protocol::CommitmentState::AgentFulfilled => {
            r.commitment_state = if confirmed {
                openswarm_protocol::CommitmentState::Verified
            } else {
                openswarm_protocol::CommitmentState::Disputed
            };
            let new_state_str = format!("{:?}", r.commitment_state);
            SwarmResponse::success(id, serde_json::json!({ "ok": true, "state": new_state_str }))
        }
        Some(_) => SwarmResponse::error(
            id,
            -32600,
            "Receipt is not in AgentFulfilled state".into(),
        ),
        None => SwarmResponse::error(id, -32602, format!("Receipt '{}' not found", receipt_id)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openswarm_protocol::CommitmentState;

    fn make_params(pairs: &[(&str, serde_json::Value)]) -> serde_json::Value {
        let mut map = serde_json::Map::new();
        for (k, v) in pairs {
            map.insert(k.to_string(), v.clone());
        }
        serde_json::Value::Object(map)
    }

    fn make_minimal_state() -> Arc<RwLock<ConnectorState>> {
        Arc::new(RwLock::new(ConnectorState::new_for_test()))
    }

    #[tokio::test]
    async fn test_create_receipt_stores_active() {
        let state = make_minimal_state();
        let params = make_params(&[
            ("task_id", serde_json::json!("task-1")),
            ("agent_id", serde_json::json!("agent-1")),
        ]);
        let resp = handle_create_receipt(Some("1".into()), &params, &state).await;
        let body: serde_json::Value = serde_json::from_str(
            &serde_json::to_string(&resp).unwrap(),
        )
        .unwrap();
        let receipt_id = body["result"]["receipt_id"].as_str().unwrap().to_string();
        assert!(body["result"]["ok"].as_bool().unwrap());

        let s = state.read().await;
        let r = s.receipts.get(&receipt_id).unwrap();
        assert_eq!(r.commitment_state, CommitmentState::Active);
        assert_eq!(r.task_id, "task-1");
    }

    #[tokio::test]
    async fn test_fulfill_receipt_advances_to_agent_fulfilled() {
        let state = make_minimal_state();
        // Create
        let params = make_params(&[
            ("task_id", serde_json::json!("task-2")),
            ("agent_id", serde_json::json!("agent-2")),
        ]);
        let resp = handle_create_receipt(Some("1".into()), &params, &state).await;
        let body: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&resp).unwrap()).unwrap();
        let receipt_id = body["result"]["receipt_id"].as_str().unwrap().to_string();

        // Fulfill
        let params2 = make_params(&[
            ("receipt_id", serde_json::json!(receipt_id.clone())),
            ("evidence_hash", serde_json::json!("sha256:abc")),
            ("confidence_delta", serde_json::json!(0.1)),
        ]);
        let resp2 = handle_fulfill_receipt(Some("2".into()), &params2, &state).await;
        let body2: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&resp2).unwrap()).unwrap();
        assert_eq!(body2["result"]["state"].as_str().unwrap(), "AgentFulfilled");

        let s = state.read().await;
        let r = s.receipts.get(&receipt_id).unwrap();
        assert_eq!(r.commitment_state, CommitmentState::AgentFulfilled);
        assert_eq!(r.evidence_hash, "sha256:abc");
    }

    #[tokio::test]
    async fn test_verify_receipt_confirmed_to_verified() {
        let state = make_minimal_state();
        let params = make_params(&[
            ("task_id", serde_json::json!("task-3")),
            ("agent_id", serde_json::json!("agent-3")),
        ]);
        let create_resp = handle_create_receipt(Some("1".into()), &params, &state).await;
        let create_body: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&create_resp).unwrap()).unwrap();
        let receipt_id = create_body["result"]["receipt_id"]
            .as_str()
            .unwrap()
            .to_string();

        let fulfill_params = make_params(&[("receipt_id", serde_json::json!(receipt_id.clone()))]);
        handle_fulfill_receipt(Some("2".into()), &fulfill_params, &state).await;

        let verify_params = make_params(&[
            ("receipt_id", serde_json::json!(receipt_id.clone())),
            ("confirmed", serde_json::json!(true)),
        ]);
        let verify_resp = handle_verify_receipt(Some("3".into()), &verify_params, &state).await;
        let verify_body: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&verify_resp).unwrap()).unwrap();
        assert_eq!(verify_body["result"]["state"].as_str().unwrap(), "Verified");

        let s = state.read().await;
        assert_eq!(
            s.receipts.get(&receipt_id).unwrap().commitment_state,
            CommitmentState::Verified
        );
    }

    #[tokio::test]
    async fn test_verify_receipt_disputed() {
        let state = make_minimal_state();
        let params = make_params(&[
            ("task_id", serde_json::json!("task-4")),
            ("agent_id", serde_json::json!("agent-4")),
        ]);
        let create_resp = handle_create_receipt(Some("1".into()), &params, &state).await;
        let create_body: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&create_resp).unwrap()).unwrap();
        let receipt_id = create_body["result"]["receipt_id"]
            .as_str()
            .unwrap()
            .to_string();

        let fulfill_params = make_params(&[("receipt_id", serde_json::json!(receipt_id.clone()))]);
        handle_fulfill_receipt(Some("2".into()), &fulfill_params, &state).await;

        let verify_params = make_params(&[
            ("receipt_id", serde_json::json!(receipt_id.clone())),
            ("confirmed", serde_json::json!(false)),
        ]);
        let verify_resp = handle_verify_receipt(Some("3".into()), &verify_params, &state).await;
        let verify_body: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&verify_resp).unwrap()).unwrap();
        assert_eq!(verify_body["result"]["state"].as_str().unwrap(), "Disputed");

        let s = state.read().await;
        assert_eq!(
            s.receipts.get(&receipt_id).unwrap().commitment_state,
            CommitmentState::Disputed
        );
    }

    #[tokio::test]
    async fn test_fulfill_receipt_not_found_returns_error() {
        let state = make_minimal_state();
        let params = make_params(&[("receipt_id", serde_json::json!("nonexistent-id"))]);
        let resp = handle_fulfill_receipt(Some("1".into()), &params, &state).await;
        let body: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&resp).unwrap()).unwrap();
        assert!(body.get("error").is_some());
        assert!(body.get("result").is_none());
    }

    #[tokio::test]
    async fn test_fulfill_receipt_wrong_state_returns_error() {
        let state = Arc::new(RwLock::new(ConnectorState::new_for_test()));
        // Create a receipt
        let create_params = serde_json::json!({
            "task_id": "t1", "agent_id": "alice",
            "deliverable_type": "artifact", "rollback_cost": "low"
        });
        let create_resp = handle_create_receipt(Some("1".into()), &create_params, &state).await;
        let receipt_id = create_resp.result.as_ref().unwrap()["receipt_id"].as_str().unwrap().to_string();
        // Fulfill once (succeeds)
        let fulfill_params = serde_json::json!({
            "receipt_id": receipt_id.clone(),
            "evidence_hash": "sha256:abc",
            "confidence_delta": 0.0
        });
        handle_fulfill_receipt(Some("2".into()), &fulfill_params, &state).await;
        // Try to fulfill again (wrong state — already AgentFulfilled)
        let resp = handle_fulfill_receipt(Some("3".into()), &fulfill_params, &state).await;
        assert!(resp.error.is_some(), "double-fulfill should return error");
    }
}
