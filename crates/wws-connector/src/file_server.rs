//! HTTP server for onboarding docs, APIs, and web dashboard assets.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path as AxumPath, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use futures_util::stream::Stream;
use serde::Deserialize;
use tokio::sync::RwLock;
use tower_http::services::ServeDir;

use wws_protocol::Tier;

use crate::connector::{ConnectorState, MessageTraceEvent};

const ACTIVE_MEMBER_STALENESS_SECS: u64 = 45;

struct EmbeddedDocs {
    skill_md: &'static str,
    heartbeat_md: &'static str,
    messaging_md: &'static str,
}

static DOCS: EmbeddedDocs = EmbeddedDocs {
    skill_md: include_str!("../../../docs/SKILL.md"),
    heartbeat_md: include_str!("../../../docs/HEARTBEAT.md"),
    messaging_md: include_str!("../../../docs/MESSAGING.md"),
};

#[derive(Clone)]
struct WebState {
    state: Arc<RwLock<ConnectorState>>,
    network_handle: wws_network::SwarmHandle,
    web_root: PathBuf,
}

pub struct FileServer {
    bind_addr: String,
    state: Arc<RwLock<ConnectorState>>,
    network_handle: wws_network::SwarmHandle,
    web_root: PathBuf,
}

impl FileServer {
    pub fn new(
        bind_addr: String,
        state: Arc<RwLock<ConnectorState>>,
        network_handle: wws_network::SwarmHandle,
    ) -> Self {
        Self {
            bind_addr,
            state,
            network_handle,
            web_root: detect_web_root(),
        }
    }

    pub async fn run(self) -> Result<(), anyhow::Error> {
        let web_root = self.web_root.clone();

        let web_state = WebState {
            state: self.state,
            network_handle: self.network_handle,
            web_root: web_root.clone(),
        };

        // Serve static assets from /assets/* directly; everything else falls
        // through to the SPA index handler which returns index.html with 200.
        // Using axum's .fallback() avoids the tower-http ServeDir bug where
        // not_found_service serves the file but still sets status 404.
        let assets_service = ServeDir::new(web_root.join("assets"));

        let app = Router::new()
            .route("/SKILL.md", get(skill_md))
            .route("/HEARTBEAT.md", get(heartbeat_md))
            .route("/MESSAGING.md", get(messaging_md))
            .route("/agent-onboarding.json", get(onboarding))
            .route("/api/health", get(api_health))
            .route("/api/auth-status", get(api_auth_status))
            .route("/api/hierarchy", get(api_hierarchy))
            .route("/api/voting", get(api_voting))
            .route("/api/voting/:task_id", get(api_voting_task))
            .route("/api/messages", get(api_messages))
            .route("/api/messages/:task_id", get(api_messages_task))
            .route("/api/tasks", get(api_tasks).post(api_submit_task))
            .route("/api/tasks/:task_id/timeline", get(api_task_timeline))
            .route("/api/tasks/:task_id/deliberation", get(api_task_deliberation))
            .route("/api/tasks/:task_id/ballots", get(api_task_ballots))
            .route("/api/tasks/:task_id/irv-rounds", get(api_task_irv_rounds))
            .route("/api/tasks/:task_id/receipts", get(api_task_receipts))
            .route("/api/tasks/:task_id/subtask-results", get(api_subtask_results))
            .route("/api/holons", get(api_holons))
            .route("/api/holons/:task_id", get(api_holon_detail))
            .route("/api/agents", get(api_agents))
            .route("/api/topology", get(api_topology))
            .route("/api/flow", get(api_flow))
            .route("/api/audit", get(api_audit))
            .route("/api/ui-recommendations", get(api_ui_recommendations))
            .route("/api/stream", get(api_stream))
            .route("/api/identity", get(api_identity))
            .route("/api/network", get(api_network))
            .route("/api/reputation", get(api_reputation))
            .route("/api/reputation/:did/events", get(api_reputation_events))
            .route("/api/directory", get(api_directory))
            .route("/api/names", get(api_names))
            .route("/api/keys", get(api_keys))
            .route("/api/inbox", get(api_inbox))
            .route("/api/receipts", get(api_receipts))
            .route("/api/receipts/:receipt_id", get(api_receipt_detail))
            .route("/api/clarifications", get(api_clarifications))
            .route("/api/events", get(api_events))
            .nest_service("/assets", assets_service)
            .fallback(spa_index)
            .with_state(web_state);

        if std::env::var("OPENSWARM_WEB_TOKEN").unwrap_or_default().trim().is_empty() {
            tracing::warn!(
                "OPENSWARM_WEB_TOKEN is not set — task injection via HTTP is unauthenticated. \
                Set this env var to require a token for POST /api/tasks."
            );
        }
        let listener = tokio::net::TcpListener::bind(&self.bind_addr).await?;
        tracing::info!(
            addr = %self.bind_addr,
            web_root = %self.web_root.display(),
            "HTTP web dashboard listening"
        );
        axum::serve(listener, app).await?;
        Ok(())
    }
}

fn detect_web_root() -> PathBuf {
    if let Ok(path) = std::env::var("OPENSWARM_WEBAPP_DIR") {
        let p = PathBuf::from(path);
        if p.join("index.html").exists() {
            return p;
        }
    }

    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let candidates = [
        cwd.join("webapp/dist"),
        cwd.join("dist"),
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../webapp/dist")
            .to_path_buf(),
    ];

    for c in candidates {
        if c.join("index.html").exists() {
            return c;
        }
    }

    cwd
}

/// SPA fallback: serve index.html with 200 for any unmatched path.
/// This enables client-side routing without browser 404 errors.
async fn spa_index(State(web): State<WebState>) -> impl IntoResponse {
    let index = web.web_root.join("index.html");
    match tokio::fs::read(&index).await {
        Ok(bytes) => (
            StatusCode::OK,
            [("content-type", "text/html; charset=utf-8")],
            bytes,
        )
            .into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn skill_md() -> impl IntoResponse {
    ([("content-type", "text/markdown; charset=utf-8")], DOCS.skill_md)
}

async fn heartbeat_md() -> impl IntoResponse {
    (
        [("content-type", "text/markdown; charset=utf-8")],
        DOCS.heartbeat_md,
    )
}

async fn messaging_md() -> impl IntoResponse {
    (
        [("content-type", "text/markdown; charset=utf-8")],
        DOCS.messaging_md,
    )
}

async fn onboarding() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "name": "WWS.Connector",
        "version": env!("CARGO_PKG_VERSION"),
        "protocol": "JSON-RPC 2.0",
        "rpc_default_port": 9370,
        "files_default_port": 9371,
        "dashboard": "/",
        "methods": [
            "swarm.get_status",
            "swarm.register_agent",
            "swarm.receive_task",
            "swarm.get_task",
            "swarm.get_task_timeline",
            "swarm.propose_plan",
            "swarm.submit_vote",
            "swarm.submit_critique",
            "swarm.get_voting_state",
            "swarm.submit_result",
            "swarm.connect",
            "swarm.get_network_stats",
            "swarm.inject_task",
            "swarm.get_hierarchy",
            "swarm.list_swarms",
            "swarm.create_swarm",
            "swarm.join_swarm",
            "swarm.get_board_status",
            "swarm.get_deliberation",
            "swarm.get_ballots",
            "swarm.get_irv_rounds",
            "swarm.register_name",
            "swarm.resolve_name",
            "swarm.send_message",
            "swarm.get_messages"
        ]
    }))
}

async fn api_health() -> Json<serde_json::Value> {
    Json(serde_json::json!({"ok": true, "service": "wws-connector", "version": env!("CARGO_PKG_VERSION")}))
}

async fn api_auth_status() -> Json<serde_json::Value> {
    let token_required = std::env::var("OPENSWARM_WEB_TOKEN")
        .ok()
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false);
    Json(serde_json::json!({"token_required": token_required}))
}

async fn api_hierarchy(State(web): State<WebState>) -> Json<serde_json::Value> {
    let s = web.state.read().await;
    let active = collect_known_members(&s);

    let mut nodes = Vec::new();
    for agent_id in active {
        let tier = s
            .agent_tiers
            .get(&agent_id)
            .cloned()
            .unwrap_or(if agent_id == s.agent_id.to_string() {
                s.my_tier
            } else {
                Tier::Executor
            });
        let parent_id = s.agent_parents.get(&agent_id).cloned();
        let task_count = s
            .task_details
            .values()
            .filter(|t| t.assigned_to.as_ref().map(|a| a.to_string()) == Some(agent_id.clone()))
            .count();

        let last_seen_secs = s.member_last_seen.get(&agent_id).map(|ts| {
            chrono::Utc::now()
                .signed_duration_since(*ts)
                .num_seconds()
                .max(0)
        });

        nodes.push(serde_json::json!({
            "agent_id": agent_id,
            "agent_name": s.agent_names.get(&agent_id).cloned().unwrap_or_else(|| short_agent_label(&agent_id)),
            "tier": format!("{:?}", tier),
            "parent_id": parent_id,
            "task_count": task_count,
            "last_seen_secs": last_seen_secs,
            "is_self": agent_id == s.agent_id.to_string(),
        }));
    }

    Json(serde_json::json!({
        "generated_at": chrono::Utc::now(),
        "self_agent": s.agent_id.to_string(),
        "nodes": nodes,
    }))
}

async fn api_voting(State(web): State<WebState>) -> Json<serde_json::Value> {
    Json(voting_payload(&web.state, None).await)
}

async fn api_voting_task(
    State(web): State<WebState>,
    AxumPath(task_id): AxumPath<String>,
) -> Json<serde_json::Value> {
    Json(voting_payload(&web.state, Some(task_id)).await)
}

async fn voting_payload(
    state: &Arc<RwLock<ConnectorState>>,
    task_filter: Option<String>,
) -> serde_json::Value {
    let s = state.read().await;

    let voting = s
        .voting_engines
        .iter()
        .filter(|(task_id, _)| task_filter.as_ref().map(|t| t == *task_id).unwrap_or(true))
        .map(|(task_id, v)| {
            let req = s.task_vote_requirements.get(task_id);
            let tier_level = req.map(|r| r.tier_level).unwrap_or(1);
            let tier = tier_from_level(tier_level);
            let expected_voters = req
                .map(|r| r.expected_voters)
                .unwrap_or_else(|| {
                    collect_known_members(&s)
                        .into_iter()
                        .filter(|id| s.agent_tiers.get(id).copied().unwrap_or(Tier::Executor) == tier)
                        .count()
                });
            let tier_members: Vec<String> = collect_known_members(&s)
                .into_iter()
                .filter(|id| s.agent_tiers.get(id).copied().unwrap_or(Tier::Executor) == tier)
                .collect();
            let voter_ids = v.voter_ids_for_debug();
            let missing_voter_names = tier_members
                .into_iter()
                .filter(|id| !voter_ids.iter().any(|voter| voter == id))
                .map(|id| s.agent_names.get(&id).cloned().unwrap_or_else(|| short_agent_label(&id)))
                .collect::<Vec<_>>();
            serde_json::json!({
                "task_id": task_id,
                "proposal_count": v.proposal_count(),
                "ballot_count": v.ballot_count(),
                "finalized": v.is_finalized(),
                "expected_voters": expected_voters,
                "missing_voter_names": missing_voter_names,
            })
        })
        .collect::<Vec<_>>();

    let rfp = s
        .rfp_coordinators
        .iter()
        .filter(|(task_id, _)| task_filter.as_ref().map(|t| t == *task_id).unwrap_or(true))
        .map(|(task_id, r)| {
            let req = s.task_vote_requirements.get(task_id);
            let tier_level = req.map(|r| r.tier_level).unwrap_or(1);
            let tier = tier_from_level(tier_level);
            let expected_proposers = req
                .map(|r| r.expected_proposers)
                .unwrap_or_else(|| {
                    collect_known_members(&s)
                        .into_iter()
                        .filter(|id| s.agent_tiers.get(id).copied().unwrap_or(Tier::Executor) == tier)
                        .count()
                });
            let tier_members: Vec<String> = collect_known_members(&s)
                .into_iter()
                .filter(|id| s.agent_tiers.get(id).copied().unwrap_or(Tier::Executor) == tier)
                .collect();

            let commit_ids = r
                .commits_for_debug()
                .iter()
                .map(|(agent, _)| agent.clone())
                .collect::<Vec<_>>();
            let missing_proposer_names = tier_members
                .into_iter()
                .filter(|id| !commit_ids.iter().any(|c| c == id))
                .map(|id| s.agent_names.get(&id).cloned().unwrap_or_else(|| short_agent_label(&id)))
                .collect::<Vec<_>>();

            let plans = r
                .reveals
                .values()
                .map(|p| {
                    let proposer_id = p.proposer.to_string();
                    serde_json::json!({
                        "proposer": proposer_id,
                        "proposer_name": s
                            .agent_names
                            .get(&p.proposer.to_string())
                            .cloned()
                            .unwrap_or_else(|| short_agent_label(&p.proposer.to_string())),
                        "plan_id": p.plan.plan_id,
                        "plan_hash": p.plan_hash,
                        "rationale": p.plan.rationale,
                        "subtask_count": p.plan.subtasks.len(),
                        "subtasks": p.plan.subtasks.iter().map(|st| serde_json::json!({
                            "index": st.index,
                            "description": st.description,
                            "required_capabilities": st.required_capabilities,
                            "estimated_complexity": st.estimated_complexity,
                        })).collect::<Vec<_>>(),
                    })
                })
                .collect::<Vec<_>>();

            let (missing_voter_names, expected_voters) = if let Some(v) = s.voting_engines.get(task_id) {
                let req = s.task_vote_requirements.get(task_id);
                let expected_voters = req.map(|rr| rr.expected_voters).unwrap_or(0);
                let tier_members: Vec<String> = collect_known_members(&s)
                    .into_iter()
                    .filter(|id| s.agent_tiers.get(id).copied().unwrap_or(Tier::Executor) == tier)
                    .collect();
                let voter_ids = v.voter_ids_for_debug();
                (
                    tier_members
                        .into_iter()
                        .filter(|id| !voter_ids.iter().any(|vv| vv == id))
                        .map(|id| s.agent_names.get(&id).cloned().unwrap_or_else(|| short_agent_label(&id)))
                        .collect::<Vec<_>>(),
                    expected_voters,
                )
            } else {
                (Vec::new(), 0)
            };
            serde_json::json!({
                "task_id": task_id,
                "phase": format!("{:?}", r.phase()),
                "commit_count": r.commit_count(),
                "reveal_count": r.reveal_count(),
                "commits": r.commits_for_debug(),
                "plans": plans,
                "expected_proposers": expected_proposers,
                "expected_voters": expected_voters,
                "missing_proposer_names": missing_proposer_names,
                "missing_voter_names": missing_voter_names,
            })
        })
        .collect::<Vec<_>>();

    serde_json::json!({ "voting": voting, "rfp": rfp })
}

async fn api_messages(State(web): State<WebState>) -> Json<serde_json::Value> {
    Json(messages_payload(&web.state, None).await)
}

async fn api_messages_task(
    State(web): State<WebState>,
    AxumPath(task_id): AxumPath<String>,
) -> Json<serde_json::Value> {
    Json(messages_payload(&web.state, Some(task_id)).await)
}

async fn messages_payload(
    state: &Arc<RwLock<ConnectorState>>,
    task_filter: Option<String>,
) -> serde_json::Value {
    let s = state.read().await;
    let items: Vec<&MessageTraceEvent> = s
        .message_trace
        .iter()
        .rev()
        .filter(|m| {
            is_business_message(m)
                &&
            task_filter
                .as_ref()
                .map(|t| m.task_id.as_ref().map(|id| id == t).unwrap_or(false))
                .unwrap_or(true)
        })
        .take(1000)
        .collect();
    serde_json::to_value(items).unwrap_or_else(|_| serde_json::json!([]))
}

async fn api_tasks(State(web): State<WebState>) -> Json<serde_json::Value> {
    let s = web.state.read().await;
    let mut tasks = s
        .task_details
        .values()
        .cloned()
        .collect::<Vec<_>>();
    tasks.sort_by(|a, b| b.created_at.cmp(&a.created_at));

    let tasks = tasks
        .into_iter()
        .map(|task| {
            let result = s.task_results.get(&task.task_id);
            let result_text = s.task_result_text.get(&task.task_id).cloned();
            let assigned_to = task.assigned_to.as_ref().map(|a| a.to_string());
            let assigned_to_name = assigned_to
                .as_ref()
                .map(|id| {
                    s.agent_names
                        .get(id)
                        .cloned()
                        .unwrap_or_else(|| short_agent_label(id))
                });
            serde_json::json!({
                "task_id": task.task_id,
                "parent_task_id": task.parent_task_id,
                "description": task.description,
                "status": format!("{:?}", task.status),
                "tier_level": task.tier_level,
                "assigned_to": assigned_to,
                "assigned_to_name": assigned_to_name,
                "subtasks": task.subtasks,
                "created_at": task.created_at,
                "deadline": task.deadline,
                "has_result": result.is_some(),
                "result_artifact": result,
                "result_text": result_text,
            })
        })
        .collect::<Vec<_>>();
    Json(serde_json::json!({"tasks": tasks}))
}

fn default_confidence_review_threshold_http() -> f32 {
    1.0
}

#[derive(Deserialize)]
struct TaskSubmitRequest {
    description: String,
    #[serde(default)]
    injector_agent_id: Option<String>,
    #[serde(default)]
    deliverables: Vec<wws_protocol::Deliverable>,
    #[serde(default)]
    coverage_threshold: f32,
    #[serde(default = "default_confidence_review_threshold_http")]
    confidence_review_threshold: f32,
}

async fn api_submit_task(
    State(web): State<WebState>,
    headers: HeaderMap,
    Json(req): Json<TaskSubmitRequest>,
) -> impl IntoResponse {
    if let Ok(required) = std::env::var("OPENSWARM_WEB_TOKEN") {
        if !required.trim().is_empty() {
            let provided = headers
                .get("x-ops-token")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            if provided != required {
                return (
                    StatusCode::UNAUTHORIZED,
                    Json(serde_json::json!({"ok": false, "error": "invalid_operator_token"})),
                );
            }
        }
    }

    if req.description.trim().is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok": false, "error": "missing_description"})),
        );
    }

    // Web UI (operator interface): if no injector_agent_id provided, use the connector's own
    // identity — it is always trusted and bypasses the reputation gate.
    let injector_agent_id = match req.injector_agent_id.as_deref().filter(|s| !s.is_empty()) {
        Some(id) => {
            let s = web.state.read().await;
            if !s.has_inject_reputation(id) {
                return (
                    StatusCode::FORBIDDEN,
                    Json(serde_json::json!({
                        "ok": false,
                        "error": format!("insufficient_reputation: agent '{}' must complete at least 1 task first", id)
                    })),
                );
            }
            id.to_string()
        }
        None => web.state.read().await.agent_id.to_string(),
    };

    let deliverables_val = serde_json::to_value(&req.deliverables).unwrap_or(serde_json::json!([]));
    let params = serde_json::json!({
        "description": req.description,
        "injector_agent_id": injector_agent_id,
        "deliverables": deliverables_val,
        "coverage_threshold": req.coverage_threshold,
        "confidence_review_threshold": req.confidence_review_threshold,
    });
    let response = crate::rpc_server::handle_inject_task(
        Some("web-submit-task".to_string()),
        &params,
        &web.state,
        &web.network_handle,
    )
    .await;

    if let Some(err) = response.error {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"ok": false, "error": err.message})),
        );
    }

    let result = response.result.unwrap_or_else(|| serde_json::json!({"ok": true}));
    (StatusCode::OK, Json(result))
}

async fn api_task_timeline(
    State(web): State<WebState>,
    AxumPath(task_id): AxumPath<String>,
) -> Json<serde_json::Value> {
    let s = web.state.read().await;

    let timeline = s.task_timelines.get(&task_id).cloned().unwrap_or_default();
    let task = s.task_details.get(&task_id).cloned().map(|t| {
        let assigned_to = t.assigned_to.as_ref().map(|a| a.to_string());
        let assigned_to_name = assigned_to
            .as_ref()
            .map(|id| {
                s.agent_names
                    .get(id)
                    .cloned()
                    .unwrap_or_else(|| short_agent_label(id))
            });
        serde_json::json!({
            "task_id": t.task_id,
            "parent_task_id": t.parent_task_id,
            "description": t.description,
            "status": format!("{:?}", t.status),
            "tier_level": t.tier_level,
            "assigned_to": assigned_to,
            "assigned_to_name": assigned_to_name,
            "subtasks": t.subtasks,
            "created_at": t.created_at,
            "deadline": t.deadline,
        })
    });
    let task_result = s.task_results.get(&task_id).cloned();
    let task_result_text = s.task_result_text.get(&task_id).cloned();
    let messages = s
        .message_trace
        .iter()
        .filter(|m| m.task_id.as_ref().map(|id| id == &task_id).unwrap_or(false))
        .cloned()
        .collect::<Vec<_>>();

    let descendants = collect_task_descendants(&task_id, &s.task_details)
        .into_iter()
        .map(|t| {
            let result = s.task_results.get(&t.task_id).cloned();
            let result_text = s.task_result_text.get(&t.task_id).cloned();
            let assigned_to = t.assigned_to.as_ref().map(|a| a.to_string());
            let assigned_to_name = assigned_to
                .as_ref()
                .map(|id| {
                    s.agent_names
                        .get(id)
                        .cloned()
                        .unwrap_or_else(|| short_agent_label(id))
                });
            serde_json::json!({
                "task_id": t.task_id,
                "parent_task_id": t.parent_task_id,
                "description": t.description,
                "status": format!("{:?}", t.status),
                "tier_level": t.tier_level,
                "assigned_to": assigned_to,
                "assigned_to_name": assigned_to_name,
                "subtasks": t.subtasks,
                "created_at": t.created_at,
                "deadline": t.deadline,
                "has_result": result.is_some(),
                "result_artifact": result,
                "result_text": result_text,
            })
        })
        .collect::<Vec<_>>();

    Json(serde_json::json!({
        "task": task,
        "result_artifact": task_result,
        "result_text": task_result_text,
        "timeline": timeline,
        "descendants": descendants,
        "messages": messages,
    }))
}

async fn api_agents(State(web): State<WebState>) -> Json<serde_json::Value> {
    let s = web.state.read().await;
    let now = chrono::Utc::now();
    let members = collect_known_members(&s);

    let agents = members
        .into_iter()
        .map(|id| {
            let seen_secs = s
                .member_last_seen
                .get(&id)
                .and_then(|ts| now.signed_duration_since(*ts).to_std().ok())
                .map(|d| d.as_secs());
            let last_task_poll_secs = s
                .member_last_task_poll
                .get(&id)
                .and_then(|ts| now.signed_duration_since(*ts).to_std().ok())
                .map(|d| d.as_secs());
            let last_result_secs = s
                .member_last_result
                .get(&id)
                .and_then(|ts| now.signed_duration_since(*ts).to_std().ok())
                .map(|d| d.as_secs());

            let activity = s.agent_activity.get(&id);
            let tasks_processed = activity.map(|a| a.tasks_processed_count).unwrap_or(0);
            let silent_failure_rate = activity.map(|a| a.silent_failure_rate()).unwrap_or(0.0);
            let unverified_receipt_count = s.unverified_receipt_count(&id);

            // Use ledger-based score if available, fall back to FIRE formula for backward compat
            let reputation_score = s.reputation_ledgers.get(&id)
                .map(|l| l.effective_score() as f64)
                .unwrap_or_else(|| {
                    // FIRE formula fallback for agents not yet in ledger
                    let tasks_done = tasks_processed as f64;
                    let tasks_got = activity.map(|a| a.tasks_assigned_count).unwrap_or(0) as f64;
                    let injected = activity.map(|a| a.tasks_injected_count).unwrap_or(0) as f64;
                    let proposals = activity.map(|a| a.plans_proposed_count).unwrap_or(0) as f64;
                    let votes = activity.map(|a| a.votes_cast_count).unwrap_or(0) as f64;
                    let sig_r = tasks_done * (tasks_done / (tasks_got + 1.0)) * 0.10;
                    let sig_d = (proposals + votes) * 0.02;
                    let sig_i = injected.sqrt() * 0.05;
                    ((sig_r + sig_d + sig_i) * 100.0).round() / 100.0
                });
            let can_inject = s.can_inject_task(&id, 0.5);
            let tier = s.reputation_ledgers.get(&id)
                .map(|l| l.tier().as_str().to_string())
                .unwrap_or_else(|| {
                    // legacy tier from agent_tiers if not in ledger
                    format!("{:?}", s.agent_tiers.get(&id).copied().unwrap_or(Tier::Executor))
                });
            // The self-agent is always connected — GossipSub doesn't echo messages
            // back to the sender, so seen_secs is unreliable for the local agent.
            let is_self = id == s.agent_id.to_string();
            let connected = is_self || seen_secs.map(|v| v <= 60).unwrap_or(false);
            let loop_active = is_self || last_task_poll_secs.map(|v| v <= 120).unwrap_or(false);
            let not_responding = !is_self && last_task_poll_secs.map(|v| v > 180).unwrap_or(true);
            serde_json::json!({
                "agent_id": id,
                "name": s.agent_names.get(&id).cloned().unwrap_or_else(|| short_agent_label(&id)),
                "tier": tier,
                "seen_secs": seen_secs,
                "last_task_poll_secs": last_task_poll_secs,
                "last_result_secs": last_result_secs,
                "tasks_assigned_count": activity.map(|a| a.tasks_assigned_count).unwrap_or(0),
                "tasks_processed_count": tasks_processed,
                "plans_proposed_count": activity.map(|a| a.plans_proposed_count).unwrap_or(0),
                "plans_revealed_count": activity.map(|a| a.plans_revealed_count).unwrap_or(0),
                "votes_cast_count": activity.map(|a| a.votes_cast_count).unwrap_or(0),
                "tasks_injected_count": activity.map(|a| a.tasks_injected_count).unwrap_or(0),
                "reputation_score": reputation_score,
                "can_inject_tasks": can_inject,
                "silent_failure_rate": silent_failure_rate,
                "unverified_receipt_count": unverified_receipt_count,
                "is_self": is_self,
                "connected": connected,
                "loop_active": loop_active,
                "not_responding": not_responding,
            })
        })
        .collect::<Vec<_>>();

    Json(serde_json::json!({ "agents": agents }))
}

fn collect_task_descendants(
    root: &str,
    details: &HashMap<String, wws_protocol::Task>,
) -> Vec<wws_protocol::Task> {
    let mut out = Vec::new();
    let mut frontier = vec![root.to_string()];
    while let Some(parent) = frontier.pop() {
        for task in details.values() {
            if task.parent_task_id.as_deref() == Some(parent.as_str()) {
                out.push(task.clone());
                frontier.push(task.task_id.clone());
            }
        }
    }
    out
}

async fn api_topology(State(web): State<WebState>) -> Json<serde_json::Value> {
    let s = web.state.read().await;
    let members = collect_known_members(&s);

    let is_flat = s.agent_tiers.is_empty();

    let nodes: Vec<serde_json::Value> = if is_flat {
        // Flat swarm: all agents from member_last_seen, no tier info.
        members
            .iter()
            .map(|id| {
                serde_json::json!({
                    "id": id,
                    "name": s.agent_names.get(id).cloned().unwrap_or_else(|| short_agent_label(id)),
                    "tier": "Flat",
                    "is_self": *id == s.agent_id.to_string(),
                })
            })
            .collect()
    } else {
        let mut nodes: Vec<serde_json::Value> = members
            .iter()
            .map(|id| {
                serde_json::json!({
                    "id": id,
                    "name": s.agent_names.get(id).cloned().unwrap_or_else(|| short_agent_label(id)),
                    "tier": format!("{:?}", s.agent_tiers.get(id).copied().unwrap_or(Tier::Executor)),
                    "is_self": *id == s.agent_id.to_string(),
                })
            })
            .collect();

        // Count Tier1 agents — only show virtual root when multiple Tier1 agents.
        let tier1_agents: Vec<&String> = s
            .agent_tiers
            .iter()
            .filter(|(id, tier)| **tier == Tier::Tier1 && members.iter().any(|m| m == *id))
            .map(|(id, _)| id)
            .collect();

        if tier1_agents.len() > 1 {
            nodes.push(serde_json::json!({
                "id": "zero0",
                "name": "WWS",
                "tier": "Root",
                "is_self": false,
            }));
        }

        nodes
    };

    let mut edges = Vec::new();

    if !is_flat {
        // Hierarchy edges.
        for (child, parent) in &s.agent_parents {
            edges.push(serde_json::json!({"source": parent, "target": child, "kind": "hierarchy"}));
        }

        let tier1_agents: Vec<&String> = s
            .agent_tiers
            .iter()
            .filter(|(id, tier)| **tier == Tier::Tier1 && members.iter().any(|m| m == *id))
            .map(|(id, _)| id)
            .collect();

        if tier1_agents.len() > 1 {
            for id in &tier1_agents {
                edges.push(serde_json::json!({
                    "source": "zero0",
                    "target": id,
                    "kind": "root_hierarchy"
                }));
            }
        }
    }

    // Peer links always shown.
    for peer in s.agent_set.elements() {
        edges.push(serde_json::json!({
            "source": s.agent_id.to_string(),
            "target": format!("did:swarm:{}", peer),
            "kind": "peer_link"
        }));
    }

    Json(serde_json::json!({"nodes": nodes, "edges": edges}))
}

fn short_agent_label(agent_id: &str) -> String {
    if let Some(last) = agent_id.split(':').next_back() {
        if last.len() > 12 {
            return last[..12].to_string();
        }
        return last.to_string();
    }
    agent_id.to_string()
}

fn collect_known_members(s: &ConnectorState) -> Vec<String> {
    let mut members: Vec<String> = s
        .agent_tiers
        .keys()
        .cloned()
        .chain(s.member_last_seen.keys().cloned())
        .collect();
    members.push(s.agent_id.to_string());
    members.sort();
    members.dedup();
    members
}

fn tier_from_level(level: u32) -> Tier {
    match level {
        1 => Tier::Tier1,
        2 => Tier::Tier2,
        n => Tier::TierN(n),
    }
}

fn is_business_message(msg: &MessageTraceEvent) -> bool {
    match msg.method.as_deref() {
        Some(method)
            if method.contains("keepalive")
                || method == "swarm.announce"
                || method == "swarm.join"
                || method == "swarm.join_response"
                || method == "swarm.leave"
                || method == "hierarchy.assign_tier" =>
        {
            false
        }
        Some(_) => true,
        None => false,
    }
}

async fn api_flow(State(web): State<WebState>) -> Json<serde_json::Value> {
    let s = web.state.read().await;
    let mut counters: HashMap<String, usize> = HashMap::new();
    for events in s.task_timelines.values() {
        for event in events {
            *counters.entry(event.stage.clone()).or_insert(0) += 1;
        }
    }

    Json(serde_json::json!({
        "counters": counters,
        "active_tasks": s.task_set.len(),
        "voting_engines": s.voting_engines.len(),
        "rfp_rounds": s.rfp_coordinators.len(),
        "message_trace_size": s.message_trace.len(),
    }))
}

async fn api_audit(State(web): State<WebState>) -> Json<serde_json::Value> {
    let s = web.state.read().await;
    let rows = s
        .event_log
        .iter()
        .rev()
        .filter(|e| e.message.starts_with("AUDIT "))
        .take(500)
        .map(|e| {
            serde_json::json!({
                "timestamp": e.timestamp,
                "category": format!("{:?}", e.category),
                "message": e.message,
            })
        })
        .collect::<Vec<_>>();
    Json(serde_json::json!({"events": rows}))
}

async fn api_ui_recommendations() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "recommended_features": [
            "Task SLA panel (stuck task detector + age heatmap)",
            "Election/succession timeline and incident replay",
            "Agent throughput and reliability leaderboard",
            "Topology drift alerts (partition/churn detection)",
            "Task graph playback over time",
            "Exportable forensic bundle per task (plans, votes, logs, artifacts)",
            "Role-based access control and audit log for operator actions"
        ]
    }))
}

async fn api_stream(
    ws: WebSocketUpgrade,
    State(web): State<WebState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| stream_loop(socket, web.state))
}

async fn stream_loop(mut socket: WebSocket, state: Arc<RwLock<ConnectorState>>) {
    let mut interval = tokio::time::interval(Duration::from_secs(2));
    loop {
        interval.tick().await;
        let payload = {
            let s = state.read().await;
            let recent_messages = s
                .message_trace
                .iter()
                .rev()
                .take(40)
                .cloned()
                .collect::<Vec<_>>();
            let recent_events = s
                .event_log
                .iter()
                .rev()
                .take(40)
                .cloned()
                .collect::<Vec<_>>();
            serde_json::json!({
                "type": "snapshot",
                "time": chrono::Utc::now(),
                "active_tasks": s.task_set.len(),
                "known_agents": s.active_member_count(Duration::from_secs(ACTIVE_MEMBER_STALENESS_SECS)),
                "messages": recent_messages,
                "events": recent_events,
            })
            .to_string()
        };

        if socket.send(Message::Text(payload.into())).await.is_err() {
            break;
        }
    }
}

// ── Holonic API Handlers ────────────────────────────────────────────────────

async fn api_holons(State(s): State<WebState>) -> impl IntoResponse {
    let state = s.state.read().await;
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
            "member_count": h.members.len(),
        })
    }).collect();
    Json(serde_json::json!({ "holons": holons }))
}

async fn api_holon_detail(
    State(s): State<WebState>,
    AxumPath(task_id): AxumPath<String>,
) -> impl IntoResponse {
    let state = s.state.read().await;
    match state.active_holons.get(&task_id) {
        Some(h) => {
            let chair_str = h.chair.to_string();
            let critic_str = h.adversarial_critic.as_ref().map(|a| a.to_string());
            let executor_ids: std::collections::HashSet<String> = h.subtask_assignments
                .values()
                .map(|a| a.to_string())
                .collect();

            // Annotate each member with their role in this holon.
            let members_detail: Vec<serde_json::Value> = h.members.iter().map(|m| {
                let id = m.to_string();
                let role = if id == chair_str {
                    "chair"
                } else if critic_str.as_deref() == Some(id.as_str()) {
                    "critic"
                } else if executor_ids.contains(&id) {
                    "executor"
                } else {
                    "member"
                };
                let name = state.agent_names.get(&id)
                    .cloned()
                    .unwrap_or_else(|| id.chars().rev().take(8).collect::<String>().chars().rev().collect());
                serde_json::json!({ "agent_id": id, "name": name, "role": role })
            }).collect();

            Json(serde_json::json!({
                "task_id": h.task_id,
                "chair": chair_str,
                "members": h.members.iter().map(|m| m.to_string()).collect::<Vec<_>>(),
                "members_detail": members_detail,
                "adversarial_critic": critic_str,
                "depth": h.depth,
                "parent_holon": h.parent_holon,
                "child_holons": h.child_holons,
                "subtask_assignments": h.subtask_assignments.iter()
                    .map(|(k, v)| (k.clone(), v.to_string()))
                    .collect::<std::collections::HashMap<_, _>>(),
                "status": format!("{:?}", h.status),
                "created_at": h.created_at,
            })).into_response()
        },
        None => Json(serde_json::json!({"task_id": task_id, "members": [], "members_detail": [], "status": "none"})).into_response(),
    }
}

async fn api_task_deliberation(
    State(s): State<WebState>,
    AxumPath(task_id): AxumPath<String>,
) -> impl IntoResponse {
    let state = s.state.read().await;
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
    Json(serde_json::json!({ "task_id": task_id, "messages": messages }))
}

async fn api_task_ballots(
    State(s): State<WebState>,
    AxumPath(task_id): AxumPath<String>,
) -> impl IntoResponse {
    let state = s.state.read().await;
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
    Json(serde_json::json!({ "task_id": task_id, "ballots": ballots }))
}

async fn api_task_irv_rounds(
    State(s): State<WebState>,
    AxumPath(task_id): AxumPath<String>,
) -> impl IntoResponse {
    let state = s.state.read().await;
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
    Json(serde_json::json!({ "task_id": task_id, "irv_rounds": rounds }))
}

// ── Identity / Network / Directory API ──────────────────────────────────────

async fn api_identity(State(web): State<WebState>) -> Json<serde_json::Value> {
    let s = web.state.read().await;
    let did = s.agent_id.to_string();
    let peer_id = did.trim_start_matches("did:swarm:").to_string();
    let name = s.agent_names.get(&did).cloned()
        .unwrap_or_else(|| short_agent_label(&did));
    Json(serde_json::json!({
        "did": did,
        "peer_id": peer_id,
        "version": env!("CARGO_PKG_VERSION"),
        "tier": format!("{:?}", s.my_tier),
        "name": name,
    }))
}

async fn api_network(State(web): State<WebState>) -> Json<serde_json::Value> {
    let s = web.state.read().await;
    let peer_count = s.agent_set.elements().len();
    let known_agents = s.active_member_count(Duration::from_secs(ACTIVE_MEMBER_STALENESS_SECS));
    Json(serde_json::json!({
        "peer_count": peer_count,
        "known_agents": known_agents,
        "agent_id": s.agent_id.to_string(),
    }))
}

async fn api_reputation(State(web): State<WebState>) -> Json<serde_json::Value> {
    let s = web.state.read().await;
    let reputation: Vec<serde_json::Value> = s.reputation_ledgers.iter().map(|(id, ledger)| {
        let eff = ledger.effective_score();
        let name = s.agent_names.get(id).cloned().unwrap_or_else(|| id.clone());
        let (guardian_quality_score, guardian_count) = s.guardian_quality_score(id);
        serde_json::json!({
            "agent_id": id,
            "name": name,
            "effective_score": eff,
            "raw_score": ledger.raw_score,
            "peak_score": ledger.peak_score,
            "tier": ledger.tier().as_str(),
            "events_count": ledger.events.len(),
            "last_active": ledger.last_active.to_rfc3339(),
            "guardian_quality_score": guardian_quality_score,
            "guardian_count": guardian_count,
        })
    }).collect();
    Json(serde_json::json!({ "reputation": reputation }))
}

async fn api_reputation_events(
    State(web): State<WebState>,
    axum::extract::Path(did): axum::extract::Path<String>,
) -> Json<serde_json::Value> {
    let s = web.state.read().await;
    let events: Vec<serde_json::Value> = s.reputation_ledgers.get(&did)
        .map(|l| l.events.iter().rev().take(50).map(|e| serde_json::json!({
            "event_type": format!("{:?}", e.event_type),
            "base_points": e.base_points,
            "effective_points": e.effective_points,
            "observer": e.observer,
            "task_id": e.task_id,
            "timestamp": e.timestamp.to_rfc3339(),
        })).collect())
        .unwrap_or_default();
    Json(serde_json::json!({ "did": did, "events": events }))
}

async fn api_directory(State(web): State<WebState>) -> Json<serde_json::Value> {
    let s = web.state.read().await;
    let agents: Vec<serde_json::Value> = s.agent_names.iter().map(|(id, name)| {
        serde_json::json!({
            "did": id,
            "name": name,
            "tier": format!("{:?}", s.agent_tiers.get(id).copied().unwrap_or(Tier::Executor)),
        })
    }).collect();
    Json(serde_json::json!({ "agents": agents }))
}

async fn api_names(State(web): State<WebState>) -> Json<serde_json::Value> {
    let s = web.state.read().await;
    let names: Vec<serde_json::Value> = s.name_registry.iter().map(|(name, did)| {
        serde_json::json!({ "name": name, "did": did })
    }).collect();
    Json(serde_json::json!({ "names": names }))
}

async fn api_keys(State(web): State<WebState>) -> Json<serde_json::Value> {
    let s = web.state.read().await;
    let keys: Vec<serde_json::Value> = s.agent_names.keys().map(|id| {
        serde_json::json!({ "agent_id": id })
    }).collect();
    Json(serde_json::json!({ "keys": keys }))
}

async fn api_inbox(State(web): State<WebState>) -> Json<serde_json::Value> {
    let s = web.state.read().await;
    let messages: Vec<serde_json::Value> = s.inbox.iter().map(|m| {
        serde_json::json!({
            "from": m.from,
            "to": m.to,
            "content": m.content,
            "timestamp": m.timestamp.to_rfc3339(),
        })
    }).collect();
    Json(serde_json::json!({ "messages": messages, "count": messages.len() }))
}

async fn api_receipts(State(web): State<WebState>) -> Json<serde_json::Value> {
    let s = web.state.read().await;
    let receipts: Vec<serde_json::Value> = s.receipts.values().map(|r| {
        serde_json::json!({
            "receipt_id": r.commitment_id,
            "task_id": r.task_id,
            "agent_id": r.agent_id,
            "state": format!("{:?}", r.commitment_state),
            "deliverable_type": r.deliverable_type,
            "rollback_cost": r.rollback_cost,
            "evidence_hash": r.evidence_hash,
            "confidence_delta": r.confidence_delta,
            "created_at": r.created_at,
        })
    }).collect();
    let count = receipts.len();
    Json(serde_json::json!({ "receipts": receipts, "count": count }))
}

async fn api_receipt_detail(
    State(web): State<WebState>,
    AxumPath(receipt_id): AxumPath<String>,
) -> impl IntoResponse {
    let s = web.state.read().await;
    match s.receipts.get(&receipt_id) {
        Some(r) => (StatusCode::OK, Json(serde_json::json!({
            "receipt_id": r.commitment_id,
            "task_id": r.task_id,
            "agent_id": r.agent_id,
            "state": format!("{:?}", r.commitment_state),
            "deliverable_type": r.deliverable_type,
            "rollback_cost": r.rollback_cost,
            "rollback_window": r.rollback_window,
            "evidence_hash": r.evidence_hash,
            "confidence_delta": r.confidence_delta,
            "can_undo": r.can_undo,
            "expires_at": r.expires_at,
            "created_at": r.created_at,
        }))).into_response(),
        None => (StatusCode::NOT_FOUND, Json(serde_json::json!({"error": "receipt not found"}))).into_response(),
    }
}

async fn api_task_receipts(
    State(web): State<WebState>,
    AxumPath(task_id): AxumPath<String>,
) -> Json<serde_json::Value> {
    let s = web.state.read().await;
    let receipts: Vec<serde_json::Value> = s.receipts.values()
        .filter(|r| r.task_id == task_id)
        .map(|r| serde_json::json!({
            "receipt_id": r.commitment_id,
            "agent_id": r.agent_id,
            "state": format!("{:?}", r.commitment_state),
            "evidence_hash": r.evidence_hash,
            "deliverable_type": r.deliverable_type,
        }))
        .collect();
    let count = receipts.len();
    Json(serde_json::json!({ "task_id": task_id, "receipts": receipts, "count": count }))
}

async fn api_subtask_results(
    State(web): State<WebState>,
    AxumPath(task_id): AxumPath<String>,
) -> Json<serde_json::Value> {
    let s = web.state.read().await;
    let task = s.task_details.get(&task_id);
    let subtask_ids = task.map(|t| t.subtasks.clone()).unwrap_or_default();

    let all_completed = !subtask_ids.is_empty() && subtask_ids.iter().all(|sub_id| {
        s.task_details
            .get(sub_id)
            .map(|t| t.status == wws_protocol::TaskStatus::Completed || t.status == wws_protocol::TaskStatus::PendingReview)
            .unwrap_or(false)
    });

    let subtask_results: Vec<serde_json::Value> = subtask_ids
        .iter()
        .map(|sub_id| {
            let sub_task = s.task_details.get(sub_id);
            let result_text = s.task_result_text.get(sub_id).cloned()
                .or_else(|| s.task_results.get(sub_id).map(|a| a.content.clone()))
                .unwrap_or_default();
            serde_json::json!({
                "subtask_id": sub_id,
                "description": sub_task.map(|t| t.description.as_str()).unwrap_or(""),
                "result_text": result_text,
                "status": sub_task.map(|t| format!("{:?}", t.status)).unwrap_or_else(|| "Unknown".to_string()),
            })
        })
        .collect();

    Json(serde_json::json!({
        "parent_task_id": task_id,
        "all_completed": all_completed,
        "subtask_results": subtask_results,
    }))
}

async fn api_clarifications(State(web): State<WebState>) -> Json<serde_json::Value> {
    let s = web.state.read().await;
    let items: Vec<serde_json::Value> = s.clarifications.values().map(|c| serde_json::json!({
        "id": c.id,
        "task_id": c.task_id,
        "requesting_agent": c.requesting_agent,
        "principal_id": c.principal_id,
        "question": c.question,
        "resolution": c.resolution,
        "created_at": c.created_at,
        "resolved_at": c.resolved_at,
    })).collect();
    let count = items.len();
    Json(serde_json::json!({ "clarifications": items, "count": count }))
}

async fn api_events(
    State(web): State<WebState>,
) -> Sse<impl Stream<Item = Result<Event, std::convert::Infallible>>> {
    let state = web.state;
    let stream = futures_util::stream::unfold(state, |state| async move {
        tokio::time::sleep(Duration::from_secs(2)).await;
        let payload = {
            let s = state.read().await;
            serde_json::json!({
                "type": "snapshot",
                "active_tasks": s.task_set.len(),
                "known_agents": s.active_member_count(Duration::from_secs(ACTIVE_MEMBER_STALENESS_SECS)),
            })
            .to_string()
        };
        let event = Event::default().event("snapshot").data(payload);
        Some((Ok(event), state))
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}
