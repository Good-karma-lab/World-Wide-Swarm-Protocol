# Reputation System + Identity Security v0.5.0 Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task.

**Goal:** Implement a full reputation system (PnCounter CRDT, event ledger, score tiers, decay, observer weighting), persistent Ed25519 identity (key file, BIP-39 mnemonic, key rotation, emergency revocation, guardian recovery), Moltbook TaskOutcome insights, Docker E2E infrastructure, and release v0.5.0.

**Architecture:** Reputation ledger stored per-agent in `ConnectorState` as `HashMap<String, ReputationLedger>` with lazy decay calculation. Identity key persisted to `~/.config/wws-connector/<name>.key`. New crate-level `reputation.rs` module in `openswarm-connector`. All score changes triggered by existing event handlers (task completion, plan selection, vote cast, etc.).

**Tech Stack:** Rust, `bip39` crate (new), `ed25519-dalek` (existing), `sha2` (existing), `tokio` (existing), Docker multi-stage build, Python 3 for E2E.

---

## Current State

- `crates/openswarm-connector/src/connector.rs` has `AgentActivity` with simple counters
- `has_inject_reputation()` checks `tasks_processed_count >= 5` (MIN_INJECT_TASKS_COMPLETED = 5)
- `crates/openswarm-state/src/crdt.rs` has `OrSet` — need to add `PnCounter`
- `crates/openswarm-protocol/src/types.rs` has `Task`, no `TaskOutcome`
- `crates/openswarm-connector/src/file_server.rs` has `/api/reputation` endpoint with FIRE formula
- `crates/openswarm-connector/src/rpc_server.rs` has all RPC handlers
- Workspace `Cargo.toml` does NOT have `bip39` crate — need to add
- `ed25519-dalek` v2 is in workspace deps with `serde` and `rand_core` features

---

## Task 1: Add PnCounter CRDT

**Files:**
- Modify: `crates/openswarm-state/src/crdt.rs`
- Modify: `crates/openswarm-state/src/lib.rs`

**Step 1: Add PnCounter struct after OrSet in crdt.rs**

```rust
/// PN-Counter CRDT (Positive-Negative Counter) for reputation scoring.
///
/// Uses two G-Counter maps (one for increments, one for decrements).
/// Merge takes the max per node_id in each map.
/// Value = sum(increments) - sum(decrements).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PnCounter {
    pub node_id: String,
    pub increments: std::collections::HashMap<String, u64>,
    pub decrements: std::collections::HashMap<String, u64>,
}

impl PnCounter {
    pub fn new(node_id: String) -> Self {
        Self {
            node_id,
            increments: std::collections::HashMap::new(),
            decrements: std::collections::HashMap::new(),
        }
    }

    pub fn increment(&mut self, amount: u64) {
        *self.increments.entry(self.node_id.clone()).or_insert(0) += amount;
    }

    pub fn decrement(&mut self, amount: u64) {
        *self.decrements.entry(self.node_id.clone()).or_insert(0) += amount;
    }

    pub fn value(&self) -> i64 {
        let pos: u64 = self.increments.values().sum();
        let neg: u64 = self.decrements.values().sum();
        (pos as i64) - (neg as i64)
    }

    /// Merge two PN-Counters: take max per node_id in each map.
    pub fn merge(&mut self, other: &PnCounter) {
        for (node, &val) in &other.increments {
            let entry = self.increments.entry(node.clone()).or_insert(0);
            if val > *entry { *entry = val; }
        }
        for (node, &val) in &other.decrements {
            let entry = self.decrements.entry(node.clone()).or_insert(0);
            if val > *entry { *entry = val; }
        }
    }
}
```

**Step 2: Add tests for PnCounter at end of crdt.rs tests module**

```rust
#[test]
fn test_pncounter_increment_decrement() {
    let mut c = PnCounter::new("n1".into());
    c.increment(10);
    c.decrement(3);
    assert_eq!(c.value(), 7);
}

#[test]
fn test_pncounter_merge() {
    let mut a = PnCounter::new("a".into());
    let mut b = PnCounter::new("b".into());
    a.increment(15);
    b.decrement(5);
    a.merge(&b);
    assert_eq!(a.value(), 10);
}

#[test]
fn test_pncounter_merge_idempotent() {
    let mut a = PnCounter::new("a".into());
    a.increment(10);
    let b = a.clone();
    a.merge(&b);
    assert_eq!(a.value(), 10); // merge with self is idempotent
}

#[test]
fn test_pncounter_never_below_floor_scenario() {
    let mut c = PnCounter::new("n1".into());
    c.increment(100);
    c.decrement(150);
    // value can go negative — caller enforces floor
    assert_eq!(c.value(), -50);
}
```

**Step 3: Export PnCounter from lib.rs**

In `crates/openswarm-state/src/lib.rs`, add `PnCounter` to the existing `pub use crdt::...` line.

**Step 4: Run tests**

```bash
~/.cargo/bin/cargo test -p openswarm-state -- pncounter 2>&1
```

Expected: 4 tests pass.

**Step 5: Commit**

```bash
git add crates/openswarm-state/src/crdt.rs crates/openswarm-state/src/lib.rs
git commit -m "feat(state): add PnCounter CRDT for reputation scoring"
```

---

## Task 2: Reputation Data Structures

**Files:**
- Create: `crates/openswarm-connector/src/reputation.rs`
- Modify: `crates/openswarm-connector/src/lib.rs` (add `pub mod reputation;`)

**Step 1: Create reputation.rs with all types**

```rust
//! Reputation ledger types and score calculations.

use serde::{Deserialize, Serialize};

/// Score tier from the spec (Section 1.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScoreTier {
    /// score < 0: cannot participate
    Suspended,
    /// 0–99: can execute, no injection
    Newcomer,
    /// 100–499: can inject simple tasks (complexity ≤ 1)
    Member,
    /// 500–999: can inject medium tasks (complexity ≤ 5)
    Trusted,
    /// 1000–4999: can inject any task, preferred coordinator
    Established,
    /// 5000+: all permissions, priority Tier1 candidate
    Veteran,
}

impl ScoreTier {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Suspended => "Suspended",
            Self::Newcomer => "Newcomer",
            Self::Member => "Member",
            Self::Trusted => "Trusted",
            Self::Established => "Established",
            Self::Veteran => "Veteran",
        }
    }

    /// Minimum score required to inject a task of given complexity.
    /// Returns None if tier cannot inject at all.
    pub fn min_inject_score(complexity: f64) -> i64 {
        if complexity <= 1.0 { 100 }
        else if complexity <= 5.0 { 500 }
        else { 1000 }
    }
}

/// Compute score tier from effective score.
pub fn score_tier(score: i64) -> ScoreTier {
    match score {
        s if s < 0 => ScoreTier::Suspended,
        s if s < 100 => ScoreTier::Newcomer,
        s if s < 500 => ScoreTier::Member,
        s if s < 1000 => ScoreTier::Trusted,
        s if s < 5000 => ScoreTier::Established,
        _ => ScoreTier::Veteran,
    }
}

/// Compute effective score with lazy decay (Section 1.6).
///
/// - 0.5% daily decay after 48h inactivity
/// - Floor at 50% of lifetime peak
pub fn effective_score(raw: i64, last_active: chrono::DateTime<chrono::Utc>, peak: i64) -> i64 {
    let now = chrono::Utc::now();
    let days_total = (now - last_active).num_hours().max(0) as f64 / 24.0;
    // 2-day grace period
    let days_inactive = (days_total - 2.0).max(0.0);
    let decay_factor = (1.0_f64 - 0.005_f64).powf(days_inactive);
    let decayed = (raw as f64 * decay_factor) as i64;
    let floor = peak / 2;
    decayed.max(floor)
}

/// A single reputation event (positive or negative).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepEvent {
    pub event_type: RepEventType,
    pub base_points: i64,
    pub observer: String,
    pub observer_score: i64,
    pub effective_points: i64, // base_points * observer_weight, clamped
    pub task_id: Option<String>,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub evidence: Option<String>,
}

/// All reputation event types per the spec (Section 1.4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RepEventType {
    // Positive events
    TaskExecutedVerified,        // +10 (objective)
    HighQualityResult,           // +5  (observer weighted)
    PlanSelectedByIrv,           // +15 (objective)
    AccurateCritique,            // +8  (objective)
    VoteCastInIrv,               // +2  (objective)
    RedundantExecutionMatch,     // +5  (objective)
    HelpedNewAgent,              // +5  (observer weighted)
    OnlineFor24h,                // +3  (objective)
    FirstToJoinBoard,            // +1  (objective)
    // Negative events
    TaskAcceptedNotDelivered,    // -10
    WrongResultHash,             // -25
    PlanRejectedUnanimously,     // -15
    ReplayAttackDetected,        // -100
    RpcRateLimitExceeded,        // -20
    SybilFlood,                  // -200
    NameSquatting,               // -50
    WildlyOffCritique,           // -5
    MissingKeepalive,            // -1
}

impl RepEventType {
    /// Base points for this event type.
    pub fn base_points(&self) -> i64 {
        match self {
            Self::TaskExecutedVerified => 10,
            Self::HighQualityResult => 5,
            Self::PlanSelectedByIrv => 15,
            Self::AccurateCritique => 8,
            Self::VoteCastInIrv => 2,
            Self::RedundantExecutionMatch => 5,
            Self::HelpedNewAgent => 5,
            Self::OnlineFor24h => 3,
            Self::FirstToJoinBoard => 1,
            Self::TaskAcceptedNotDelivered => -10,
            Self::WrongResultHash => -25,
            Self::PlanRejectedUnanimously => -15,
            Self::ReplayAttackDetected => -100,
            Self::RpcRateLimitExceeded => -20,
            Self::SybilFlood => -200,
            Self::NameSquatting => -50,
            Self::WildlyOffCritique => -5,
            Self::MissingKeepalive => -1,
        }
    }

    /// Is this an objective event (observer weight = 1.0 regardless of observer score)?
    pub fn is_objective(&self) -> bool {
        matches!(
            self,
            Self::TaskExecutedVerified
                | Self::PlanSelectedByIrv
                | Self::AccurateCritique
                | Self::VoteCastInIrv
                | Self::RedundantExecutionMatch
                | Self::OnlineFor24h
                | Self::FirstToJoinBoard
                | Self::TaskAcceptedNotDelivered
                | Self::WrongResultHash
                | Self::PlanRejectedUnanimously
                | Self::ReplayAttackDetected
                | Self::RpcRateLimitExceeded
                | Self::SybilFlood
                | Self::NameSquatting
                | Self::WildlyOffCritique
                | Self::MissingKeepalive
        )
    }
}

/// Observer-weighted contribution per spec Section 1.3.
pub fn observer_weighted_points(base_points: i64, observer_score: i64, is_objective: bool) -> i64 {
    if is_objective {
        return base_points;
    }
    let weight = (observer_score as f64 / 1000.0).min(1.0).max(0.0);
    (base_points as f64 * weight) as i64
}

/// Per-agent reputation ledger.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReputationLedger {
    /// Running raw score (not decayed).
    pub raw_score: i64,
    /// Lifetime peak score (used for decay floor).
    pub peak_score: i64,
    /// Last active timestamp (used for decay calculation).
    pub last_active: chrono::DateTime<chrono::Utc>,
    /// Full event history (capped at 500 entries).
    pub events: Vec<RepEvent>,
}

impl Default for ReputationLedger {
    fn default() -> Self {
        Self {
            raw_score: 0,
            peak_score: 0,
            last_active: chrono::Utc::now(),
            events: Vec::new(),
        }
    }
}

impl ReputationLedger {
    pub fn effective_score(&self) -> i64 {
        effective_score(self.raw_score, self.last_active, self.peak_score)
    }

    pub fn tier(&self) -> ScoreTier {
        score_tier(self.effective_score())
    }

    /// Apply a reputation event, updating raw_score and peak.
    pub fn apply_event(&mut self, event: RepEvent) {
        self.raw_score += event.effective_points;
        if self.raw_score > self.peak_score {
            self.peak_score = self.raw_score;
        }
        if event.effective_points != 0 {
            self.last_active = event.timestamp;
        }
        if self.events.len() >= 500 {
            self.events.remove(0);
        }
        self.events.push(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_score_tier_boundaries() {
        assert_eq!(score_tier(-1), ScoreTier::Suspended);
        assert_eq!(score_tier(0), ScoreTier::Newcomer);
        assert_eq!(score_tier(99), ScoreTier::Newcomer);
        assert_eq!(score_tier(100), ScoreTier::Member);
        assert_eq!(score_tier(499), ScoreTier::Member);
        assert_eq!(score_tier(500), ScoreTier::Trusted);
        assert_eq!(score_tier(999), ScoreTier::Trusted);
        assert_eq!(score_tier(1000), ScoreTier::Established);
        assert_eq!(score_tier(4999), ScoreTier::Established);
        assert_eq!(score_tier(5000), ScoreTier::Veteran);
    }

    #[test]
    fn test_effective_score_no_decay_within_grace() {
        // Within 48h grace period, no decay
        let last_active = chrono::Utc::now() - chrono::Duration::hours(24);
        let score = effective_score(100, last_active, 100);
        assert_eq!(score, 100); // within grace, no decay
    }

    #[test]
    fn test_effective_score_floor() {
        // Score can't drop below 50% of peak
        let last_active = chrono::Utc::now() - chrono::Duration::days(365);
        let score = effective_score(100, last_active, 200);
        assert_eq!(score, 100); // floor = 200/2 = 100
    }

    #[test]
    fn test_observer_weighted_objective_always_full() {
        assert_eq!(observer_weighted_points(10, 0, true), 10);
        assert_eq!(observer_weighted_points(10, 50, true), 10);
    }

    #[test]
    fn test_observer_weighted_subjective_scales() {
        assert_eq!(observer_weighted_points(10, 0, false), 0);
        assert_eq!(observer_weighted_points(10, 500, false), 5);
        assert_eq!(observer_weighted_points(10, 1000, false), 10);
        assert_eq!(observer_weighted_points(10, 2000, false), 10); // capped at 1.0
    }

    #[test]
    fn test_reputation_ledger_apply_event() {
        let mut ledger = ReputationLedger::default();
        let event = RepEvent {
            event_type: RepEventType::TaskExecutedVerified,
            base_points: 10,
            observer: "self".into(),
            observer_score: 0,
            effective_points: 10,
            task_id: Some("t1".into()),
            timestamp: chrono::Utc::now(),
            evidence: None,
        };
        ledger.apply_event(event);
        assert_eq!(ledger.raw_score, 10);
        assert_eq!(ledger.peak_score, 10);
        assert_eq!(ledger.events.len(), 1);
    }

    #[test]
    fn test_min_inject_score() {
        assert_eq!(ScoreTier::min_inject_score(0.5), 100);
        assert_eq!(ScoreTier::min_inject_score(1.0), 100);
        assert_eq!(ScoreTier::min_inject_score(1.1), 500);
        assert_eq!(ScoreTier::min_inject_score(5.0), 500);
        assert_eq!(ScoreTier::min_inject_score(5.1), 1000);
    }
}
```

**Step 2: Register module in lib.rs**

In `crates/openswarm-connector/src/lib.rs`, add:
```rust
pub mod reputation;
```

**Step 3: Run tests**

```bash
~/.cargo/bin/cargo test -p openswarm-connector -- reputation 2>&1
```

Expected: ~10 tests pass.

**Step 4: Commit**

```bash
git add crates/openswarm-connector/src/reputation.rs crates/openswarm-connector/src/lib.rs
git commit -m "feat(connector): add reputation module with ledger, tiers, decay, observer weighting"
```

---

## Task 3: Wire Reputation Into ConnectorState

**Files:**
- Modify: `crates/openswarm-connector/src/connector.rs`

**Step 1: Import reputation module**

At top of connector.rs, add:
```rust
use crate::reputation::{RepEvent, RepEventType, ReputationLedger, observer_weighted_points};
```

**Step 2: Add fields to ConnectorState struct**

After the `inbox: Vec<InboxMessage>` field (line ~210), add:
```rust
    /// Per-agent reputation ledgers (event log + scores).
    pub reputation_ledgers: std::collections::HashMap<String, ReputationLedger>,
    /// Rate limiting for reputation event submission: agent_id -> timestamps.
    pub rep_event_rate_limiter: std::collections::HashMap<String, Vec<chrono::DateTime<chrono::Utc>>>,
```

**Step 3: Initialize new fields in all ConnectorState literal constructors**

Find every place `ConnectorState { ... }` is constructed (connector.rs constructor and operator_console.rs x3) and add:
```rust
reputation_ledgers: std::collections::HashMap::new(),
rep_event_rate_limiter: std::collections::HashMap::new(),
```

**Step 4: Add reputation helper methods to ConnectorState impl**

After the existing `check_and_update_inject_rate_limit` method, add:

```rust
/// Get or create ledger for an agent.
pub fn ledger_mut(&mut self, agent_id: &str) -> &mut ReputationLedger {
    self.reputation_ledgers
        .entry(agent_id.to_string())
        .or_default()
}

/// Apply an objective reputation event (weight = 1.0 always).
pub fn apply_rep_event(&mut self, agent_id: &str, event_type: RepEventType, task_id: Option<String>) {
    let base = event_type.base_points();
    let is_obj = event_type.is_objective();
    let observer_score = self.reputation_ledgers
        .get(&self.agent_id.to_string())
        .map(|l| l.effective_score())
        .unwrap_or(0);
    let effective = observer_weighted_points(base, observer_score, is_obj);
    let event = RepEvent {
        event_type,
        base_points: base,
        observer: self.agent_id.to_string(),
        observer_score,
        effective_points: effective,
        task_id,
        timestamp: chrono::Utc::now(),
        evidence: None,
    };
    let ledger = self.reputation_ledgers
        .entry(agent_id.to_string())
        .or_default();
    ledger.apply_event(event);
}

/// Check if agent can inject a task of given complexity.
/// Self is always allowed (bootstrapping). Others need tier permission.
pub fn can_inject_task(&self, agent_id: &str, complexity: f64) -> bool {
    if self.agent_id.to_string() == agent_id {
        return true;
    }
    let score = self.reputation_ledgers
        .get(agent_id)
        .map(|l| l.effective_score())
        .unwrap_or(0);
    let min_score = crate::reputation::ScoreTier::min_inject_score(complexity);
    score >= min_score
}

/// Check rep event rate limit: max 20 events per agent per hour.
pub fn check_rep_event_rate_limit(&mut self, agent_id: &str) -> bool {
    let now = chrono::Utc::now();
    let window = chrono::Duration::hours(1);
    let max_per_window: usize = 20;
    let timestamps = self.rep_event_rate_limiter
        .entry(agent_id.to_string())
        .or_insert_with(Vec::new);
    timestamps.retain(|&t| now - t < window);
    if timestamps.len() >= max_per_window {
        return false;
    }
    timestamps.push(now);
    true
}
```

**Step 5: Trigger rep events from existing handlers**

In the existing `bump_tasks_processed` method (which is called when a task result is submitted), add a call to `apply_rep_event` for `TaskExecutedVerified`:

Find `pub fn bump_tasks_processed(&mut self, agent_id: &str)` and after the existing counter increment, add:
```rust
self.apply_rep_event(agent_id, RepEventType::TaskExecutedVerified, None);
```

In `bump_plans_proposed` (called when a plan is proposed and selected), trigger `PlanSelectedByIrv` when applicable. For simplicity, we'll trigger in the voting handler when an IRV winner is determined.

In `bump_votes_cast` (called when a vote is submitted), add:
```rust
self.apply_rep_event(agent_id, RepEventType::VoteCastInIrv, None);
```

**Step 6: Update has_inject_reputation to use new tier system**

Replace `has_inject_reputation` method with:
```rust
/// Legacy: use can_inject_task(agent_id, complexity) for tier-aware checks.
pub fn has_inject_reputation(&self, agent_id: &str) -> bool {
    self.can_inject_task(agent_id, 0.5)  // defaults to Member tier requirement
}
```

**Step 7: Run full test suite**

```bash
~/.cargo/bin/cargo test --workspace 2>&1 | tail -20
```

Expected: all existing tests pass.

**Step 8: Commit**

```bash
git add crates/openswarm-connector/src/connector.rs
git commit -m "feat(connector): wire reputation ledgers into ConnectorState, tier-based injection gate"
```

---

## Task 4: New Reputation RPC Methods

**Files:**
- Modify: `crates/openswarm-connector/src/rpc_server.rs`

**Step 1: Add routing in handle_connection**

Find the `match method` block in `handle_connection`. After the `"swarm.get_messages"` arm, add:
```rust
"swarm.get_reputation" => {
    handle_get_reputation(id, params, &state).await
}
"swarm.get_reputation_events" => {
    handle_get_reputation_events(id, params, &state).await
}
"swarm.submit_reputation_event" => {
    handle_submit_reputation_event(id, params, &state).await
}
```

**Step 2: Add handler functions at end of file**

```rust
async fn handle_get_reputation(
    id: String,
    params: serde_json::Value,
    state: &Arc<RwLock<ConnectorState>>,
) -> String {
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

async fn handle_get_reputation_events(
    id: String,
    params: serde_json::Value,
    state: &Arc<RwLock<ConnectorState>>,
) -> String {
    let did = params.get("did")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(20) as usize;
    let offset = params.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let s = state.read().await;
    let target = if did.is_empty() { s.agent_id.to_string() } else { did };
    let events: Vec<serde_json::Value> = s.reputation_ledgers
        .get(&target)
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
    SwarmResponse::success(id, serde_json::json!({ "events": events, "total": events.len() }))
}

async fn handle_submit_reputation_event(
    id: String,
    params: serde_json::Value,
    state: &Arc<RwLock<ConnectorState>>,
) -> String {
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
        return SwarmResponse::error(id, -32602, "missing target_did");
    }

    let mut s = state.write().await;

    // Submitter needs Member tier (score >= 100) to submit events
    let submitter_score = s.reputation_ledgers
        .get(&submitter)
        .map(|l| l.effective_score())
        .unwrap_or(0);
    if submitter_score < 100 {
        return SwarmResponse::error(id, -32603, "insufficient reputation to submit events");
    }

    // Rate limit: max 20 per hour per submitter
    if !s.check_rep_event_rate_limit(&submitter) {
        return SwarmResponse::error(id, -32604, "reputation event rate limit exceeded");
    }

    // Parse event type (only allow subjective positive events from external submitters)
    let event_type = match event_type_str.as_str() {
        "HighQualityResult" => RepEventType::HighQualityResult,
        "HelpedNewAgent" => RepEventType::HelpedNewAgent,
        _ => return SwarmResponse::error(id, -32602, "unsupported event_type for external submission"),
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
```

**Step 3: Run tests**

```bash
~/.cargo/bin/cargo test --workspace 2>&1 | tail -10
```

Expected: all tests pass.

**Step 4: Commit**

```bash
git add crates/openswarm-connector/src/rpc_server.rs
git commit -m "feat(rpc): add swarm.get_reputation, swarm.get_reputation_events, swarm.submit_reputation_event"
```

---

## Task 5: Update HTTP API for Reputation

**Files:**
- Modify: `crates/openswarm-connector/src/file_server.rs` (lines ~1145 onwards)

**Step 1: Replace the `api_reputation` handler with full ledger data**

Find `async fn api_reputation` (around line 1145) and replace its body:

```rust
async fn api_reputation(State(web): State<WebState>) -> Json<serde_json::Value> {
    let s = web.state.read().await;
    let reputation: Vec<serde_json::Value> = s.reputation_ledgers.iter().map(|(id, ledger)| {
        let eff = ledger.effective_score();
        let name = s.agent_names.get(id).cloned().unwrap_or_else(|| id.clone());
        serde_json::json!({
            "agent_id": id,
            "name": name,
            "effective_score": eff,
            "raw_score": ledger.raw_score,
            "peak_score": ledger.peak_score,
            "tier": ledger.tier().as_str(),
            "events_count": ledger.events.len(),
            "last_active": ledger.last_active.to_rfc3339(),
        })
    }).collect();
    Json(serde_json::json!({ "reputation": reputation }))
}
```

**Step 2: Add `/api/reputation/:did/events` route**

After the existing `.route("/api/reputation", get(api_reputation))` line, add:
```rust
.route("/api/reputation/:did/events", get(api_reputation_events))
```

**Step 3: Add the handler**

```rust
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
```

**Step 4: Update agents API to use new reputation ledger instead of FIRE formula**

Find the agents API handler (around line 704 — the "Multi-signal reputation" comment block) and replace the FIRE formula block with:
```rust
// Use ledger-based score if available, fall back to FIRE formula for backward compat
let reputation_score = s.reputation_ledgers.get(&id)
    .map(|l| l.effective_score() as f64)
    .unwrap_or_else(|| {
        // FIRE formula fallback for agents not yet in ledger
        let activity = s.agent_activity.get(&id);
        let tasks_done = activity.map(|a| a.tasks_processed_count).unwrap_or(0) as f64;
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
    .unwrap_or_else(|| format!("{:?}", s.agent_tiers.get(&id).copied().unwrap_or(Tier::Executor)));
```
Also update the JSON output to use `tier` variable and `can_inject`.

**Step 5: Run build**

```bash
~/.cargo/bin/cargo build -p openswarm-connector 2>&1 | tail -20
```

Expected: builds successfully.

**Step 6: Commit**

```bash
git add crates/openswarm-connector/src/file_server.rs
git commit -m "feat(http): update /api/reputation with ledger data, add /api/reputation/:did/events"
```

---

## Task 6: Identity Persistence (Persistent Ed25519 Key + BIP-39 Mnemonic)

**Files:**
- Modify: `Cargo.toml` (workspace) — add `bip39` dep
- Modify: `crates/openswarm-connector/Cargo.toml` — add `bip39`
- Create: `crates/openswarm-connector/src/identity_store.rs`
- Modify: `crates/openswarm-connector/src/lib.rs` — add `pub mod identity_store;`
- Modify: `crates/openswarm-connector/src/connector.rs` — add `key_path: Option<PathBuf>` to ConnectorState
- Modify: `crates/openswarm-connector/src/main.rs` — add `--key-file` CLI arg, call load_or_generate_key

**Step 1: Add bip39 to workspace Cargo.toml**

In `Cargo.toml` under `[workspace.dependencies]`, add:
```toml
bip39 = "2"
```

In `crates/openswarm-connector/Cargo.toml` under `[dependencies]`, add:
```toml
bip39 = { workspace = true }
```

**Step 2: Create identity_store.rs**

```rust
//! Persistent Ed25519 identity key management.
//!
//! Key is stored at ~/.config/wws-connector/<agent_name>.key as 32 raw seed bytes.
//! BIP-39 mnemonic (24 words) printed to stdout on first generation — never again.
//! File permissions set to 0600 (owner read/write only).

use std::path::{Path, PathBuf};
use ed25519_dalek::{SigningKey, VerifyingKey};

/// Load or generate an Ed25519 signing key from the given path.
///
/// If the key file exists, load it. If not, generate a new key, print
/// the BIP-39 mnemonic to stdout, and save the key file with mode 0600.
pub fn load_or_generate_key(key_path: &Path) -> anyhow::Result<SigningKey> {
    if key_path.exists() {
        load_key(key_path)
    } else {
        generate_and_save_key(key_path)
    }
}

/// Load an Ed25519 signing key from a 32-byte seed file.
pub fn load_key(key_path: &Path) -> anyhow::Result<SigningKey> {
    let bytes = std::fs::read(key_path)?;
    if bytes.len() != 32 {
        anyhow::bail!("Invalid key file: expected 32 bytes, got {}", bytes.len());
    }
    let seed: [u8; 32] = bytes.try_into().unwrap();
    Ok(SigningKey::from_bytes(&seed))
}

/// Generate a new Ed25519 key, print BIP-39 mnemonic, save to file with 0600 perms.
pub fn generate_and_save_key(key_path: &Path) -> anyhow::Result<SigningKey> {
    use rand::RngCore;

    // Generate 32 bytes of entropy (= Ed25519 seed)
    let mut seed = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut seed);

    let signing_key = SigningKey::from_bytes(&seed);

    // Generate BIP-39 mnemonic from the seed
    let mnemonic = bip39::Mnemonic::from_entropy(&seed)
        .map_err(|e| anyhow::anyhow!("BIP-39 error: {:?}", e))?;

    // Print mnemonic — shown only once
    println!();
    println!("╔══════════════════════════════════════════════════════════════╗");
    println!("║  WWS Identity Mnemonic — write this down, keep it offline   ║");
    println!("╠══════════════════════════════════════════════════════════════╣");
    println!("║                                                              ║");
    let words = mnemonic.to_string();
    // Print in 6-word rows
    let word_list: Vec<&str> = words.split_whitespace().collect();
    for chunk in word_list.chunks(6) {
        println!("║  {:<60}  ║", chunk.join(" "));
    }
    println!("║                                                              ║");
    println!("╠══════════════════════════════════════════════════════════════╣");
    println!("║  WARNING: Anyone with these words can control your identity ║");
    println!("║  WARNING: This is shown ONCE. It cannot be recovered.       ║");
    println!("╚══════════════════════════════════════════════════════════════╝");
    println!();

    // Save key file
    if let Some(parent) = key_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(key_path, &seed)?;

    // Set file permissions to 0600 on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        std::fs::set_permissions(key_path, perms)?;
    }

    tracing::info!(
        key_path = %key_path.display(),
        pubkey = %hex::encode(signing_key.verifying_key().as_bytes()),
        "Generated new Ed25519 identity key"
    );

    Ok(signing_key)
}

/// Compute the default key file path: ~/.config/wws-connector/<agent_name>.key
pub fn default_key_path(agent_name: &str) -> PathBuf {
    let base = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("wws-connector");
    base.join(format!("{}.key", agent_name))
}

/// Derive a recovery verifying key from the same seed (seed bytes 0..32 used as recovery seed).
/// In practice: we hash the primary seed to get a distinct recovery seed.
pub fn recovery_pubkey(primary_seed: &[u8; 32]) -> VerifyingKey {
    use sha2::{Sha256, Digest};
    let mut hasher = Sha256::new();
    hasher.update(b"recovery:");
    hasher.update(primary_seed);
    let recovery_seed: [u8; 32] = hasher.finalize().into();
    SigningKey::from_bytes(&recovery_seed).verifying_key()
}

/// Hash of the recovery pubkey for DHT commitment (stored locally for now).
pub fn recovery_pubkey_hash(primary_seed: &[u8; 32]) -> String {
    use sha2::{Sha256, Digest};
    let rpk = recovery_pubkey(primary_seed);
    let mut hasher = Sha256::new();
    hasher.update(rpk.as_bytes());
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_save_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.key");

        let key1 = generate_and_save_key(&path).unwrap();
        let key2 = load_key(&path).unwrap();

        assert_eq!(
            key1.verifying_key().as_bytes(),
            key2.verifying_key().as_bytes()
        );
    }

    #[test]
    fn test_recovery_pubkey_is_deterministic() {
        let seed = [42u8; 32];
        let rpk1 = recovery_pubkey(&seed);
        let rpk2 = recovery_pubkey(&seed);
        assert_eq!(rpk1.as_bytes(), rpk2.as_bytes());
    }

    #[test]
    fn test_recovery_pubkey_differs_from_primary() {
        let seed = [42u8; 32];
        let primary = SigningKey::from_bytes(&seed).verifying_key();
        let recovery = recovery_pubkey(&seed);
        assert_ne!(primary.as_bytes(), recovery.as_bytes());
    }
}
```

**Note:** This requires `hex`, `dirs`, and `tempfile` (for tests) crates. Add to connector Cargo.toml:
```toml
hex = "0.4"
dirs = "5"

[dev-dependencies]
tempfile = "3"
```

Also add `hex = "0.4"` and `dirs = "5"` to workspace dependencies.

**Step 3: Register module in lib.rs**

```rust
pub mod identity_store;
```

**Step 4: Add --key-file CLI flag to main.rs**

In the `Cli` struct, add:
```rust
/// Path to Ed25519 key file (default: ~/.config/wws-connector/<name>.key).
#[arg(long, value_name = "PATH")]
key_file: Option<PathBuf>,
```

**Step 5: Use key in main.rs before building connector**

Before `let connector = OpenSwarmConnector::new(config.clone())?;`, add:
```rust
// Load or generate persistent identity key
let key_path = cli.key_file.unwrap_or_else(|| {
    openswarm_connector::identity_store::default_key_path(&config.agent.name)
});
let _signing_key = openswarm_connector::identity_store::load_or_generate_key(&key_path)?;
// TODO: pass signing_key to connector for message signing (future work)
tracing::info!(
    key_path = %key_path.display(),
    "Identity key loaded"
);
```

**Step 6: Run tests**

```bash
~/.cargo/bin/cargo test -p openswarm-connector -- identity_store 2>&1
```

Expected: 3 tests pass.

**Step 7: Run full build**

```bash
~/.cargo/bin/cargo build --release --bin wws-connector 2>&1 | tail -10
```

**Step 8: Commit**

```bash
git add Cargo.toml crates/openswarm-connector/Cargo.toml crates/openswarm-connector/src/identity_store.rs crates/openswarm-connector/src/lib.rs crates/openswarm-connector/src/main.rs
git commit -m "feat(identity): persistent Ed25519 key file, BIP-39 mnemonic on first run"
```

---

## Task 7: Key Rotation and Emergency Revocation RPCs

**Files:**
- Modify: `crates/openswarm-connector/src/connector.rs` — add `PendingKeyRotation`, `PendingRevocation`, `GuardianDesignation` structs + fields
- Modify: `crates/openswarm-connector/src/rpc_server.rs` — add rotation/revocation/guardian handlers

**Step 1: Add pending rotation types to connector.rs**

After `InboxMessage` struct, add:
```rust
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
```

Add to `ConnectorState` struct fields:
```rust
    /// Pending key rotation records (agent_did -> rotation).
    pub pending_key_rotations: std::collections::HashMap<String, PendingKeyRotation>,
    /// Pending emergency revocations (agent_did -> revocation).
    pub pending_revocations: std::collections::HashMap<String, PendingRevocation>,
    /// Guardian designations (agent_did -> designation).
    pub guardian_designations: std::collections::HashMap<String, GuardianDesignation>,
    /// Guardian recovery votes (target_did -> Vec<vote>).
    pub guardian_votes: std::collections::HashMap<String, Vec<GuardianVote>>,
```

Initialize all four fields with `std::collections::HashMap::new()` in all constructors.

**Step 2: Add RPC routing in rpc_server.rs**

In the match block, add:
```rust
"swarm.rotate_key" => {
    handle_rotate_key(id, params, &state).await
}
"swarm.emergency_revocation" => {
    handle_emergency_revocation(id, params, &state).await
}
"swarm.register_guardians" => {
    handle_register_guardians(id, params, &state).await
}
"swarm.guardian_recovery_vote" => {
    handle_guardian_recovery_vote(id, params, &state).await
}
"swarm.get_identity" => {
    handle_get_identity(id, params, &state).await
}
```

**Step 3: Add handler implementations at end of rpc_server.rs**

```rust
async fn handle_rotate_key(
    id: String,
    params: serde_json::Value,
    state: &Arc<RwLock<ConnectorState>>,
) -> String {
    use crate::connector::PendingKeyRotation;

    let agent_did = params.get("agent_did").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let old_pubkey_hex = params.get("old_pubkey_hex").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let new_pubkey_hex = params.get("new_pubkey_hex").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let rotation_timestamp = params.get("rotation_timestamp").and_then(|v| v.as_i64()).unwrap_or(0);
    // NOTE: sig_old and sig_new validation requires the full ed25519-dalek verification.
    // For now we store the rotation record and trust the client sent valid sigs.
    // Full verification: verify sig_old over (new_pubkey || timestamp), sig_new over (old_pubkey || timestamp).

    if agent_did.is_empty() || new_pubkey_hex.is_empty() {
        return SwarmResponse::error(id, -32602, "missing required fields");
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
    id: String,
    params: serde_json::Value,
    state: &Arc<RwLock<ConnectorState>>,
) -> String {
    use crate::connector::PendingRevocation;

    let agent_did = params.get("agent_did").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let recovery_pubkey_hex = params.get("recovery_pubkey_hex").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let new_primary_pubkey_hex = params.get("new_primary_pubkey_hex").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let revocation_timestamp = params.get("revocation_timestamp").and_then(|v| v.as_i64()).unwrap_or(0);

    if agent_did.is_empty() || recovery_pubkey_hex.is_empty() || new_primary_pubkey_hex.is_empty() {
        return SwarmResponse::error(id, -32602, "missing required fields");
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
    id: String,
    params: serde_json::Value,
    state: &Arc<RwLock<ConnectorState>>,
) -> String {
    use crate::connector::GuardianDesignation;

    let agent_did = params.get("agent_did").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let guardians: Vec<String> = params.get("guardians")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
        .unwrap_or_default();
    let threshold = params.get("threshold").and_then(|v| v.as_u64()).unwrap_or(2) as u32;

    if agent_did.is_empty() || guardians.is_empty() {
        return SwarmResponse::error(id, -32602, "missing agent_did or guardians");
    }
    if threshold as usize > guardians.len() {
        return SwarmResponse::error(id, -32602, "threshold exceeds guardian count");
    }

    let designation = GuardianDesignation { agent_did: agent_did.clone(), guardians, threshold };
    let mut s = state.write().await;
    s.guardian_designations.insert(agent_did, designation);

    SwarmResponse::success(id, serde_json::json!({ "registered": true }))
}

async fn handle_guardian_recovery_vote(
    id: String,
    params: serde_json::Value,
    state: &Arc<RwLock<ConnectorState>>,
) -> String {
    use crate::connector::GuardianVote;

    let guardian_did = params.get("guardian_did").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let target_did = params.get("target_did").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let new_pubkey = params.get("new_pubkey").and_then(|v| v.as_str()).unwrap_or("").to_string();

    if guardian_did.is_empty() || target_did.is_empty() || new_pubkey.is_empty() {
        return SwarmResponse::error(id, -32602, "missing required fields");
    }

    // Guardian must have Trusted tier (score >= 500) per spec
    let guardian_score = {
        let s = state.read().await;
        s.reputation_ledgers.get(&guardian_did)
            .map(|l| l.effective_score())
            .unwrap_or(0)
    };
    if guardian_score < 500 {
        return SwarmResponse::error(id, -32603, "guardian needs Trusted tier (score >= 500)");
    }

    let mut s = state.write().await;

    // Check guardian is in the designated list
    let threshold = s.guardian_designations.get(&target_did)
        .map(|d| (d.threshold, d.guardians.contains(&guardian_did)))
        .unwrap_or((2, false));

    if !threshold.1 {
        return SwarmResponse::error(id, -32603, "guardian not in designated list for this agent");
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
    let threshold_met = vote_count >= threshold.0 as usize;

    SwarmResponse::success(id, serde_json::json!({
        "accepted": true,
        "votes_collected": vote_count,
        "threshold": threshold.0,
        "threshold_met": threshold_met,
    }))
}

async fn handle_get_identity(
    id: String,
    params: serde_json::Value,
    state: &Arc<RwLock<ConnectorState>>,
) -> String {
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
```

**Step 4: Run full test suite**

```bash
~/.cargo/bin/cargo test --workspace 2>&1 | tail -10
```

Expected: all tests pass.

**Step 5: Commit**

```bash
git add crates/openswarm-connector/src/connector.rs crates/openswarm-connector/src/rpc_server.rs
git commit -m "feat(identity): key rotation, emergency revocation, guardian social recovery RPCs"
```

---

## Task 8: Moltbook Insights — TaskOutcome + CommitmentReceipt

**Files:**
- Modify: `crates/openswarm-protocol/src/types.rs` — add `TaskOutcome`, `FailureReason`, `CommitmentReceipt`
- Modify: `crates/openswarm-connector/src/connector.rs` — add `contribution_ratio` to AgentActivity
- Modify: `crates/openswarm-connector/src/rpc_server.rs` — accept `confidence_delta` in submit_result

**Step 1: Add TaskOutcome types to types.rs**

After the `TaskStatus` enum (around line 49), add:

```rust
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
    ContradictoryConstraints { conflict_description: String },
    InsufficientContext { missing_keys: Vec<String> },
    ResourceExhausted { resource: String },
    ExternalDependencyFailed { dependency: String },
    TaskAmbiguous { ambiguity_description: String },
}
```

Add `CommitmentReceipt` struct for the richer reversibility schema (Moltbook insight #1):

```rust
/// Commitment receipt with rich reversibility info (Moltbook insight #1).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitmentReceipt {
    pub commitment_id: String,
    pub deliverable_type: String, // "artifact" | "decision" | "state_change" | "message"
    pub evidence_hash: String,
    pub confidence_delta: f64,    // pre - post execution confidence
    pub can_undo: bool,
    pub rollback_cost: Option<String>, // "low" | "medium" | "high"
    pub rollback_window: Option<String>, // ISO8601 duration or null
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    pub commitment_state: CommitmentState,
    pub task_id: String,
    pub agent_id: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommitmentState {
    Active,
    Fulfilled,
    Expired,
    Failed,
    Disputed,
}
```

**Step 2: Add contribution_ratio to AgentActivity in connector.rs**

In the `AgentActivity` struct, add field:
```rust
    /// tasks_injected_count / tasks_processed_count (principal accountability).
    pub contribution_ratio: f64,
```

Add update logic in `bump_tasks_injected`:
```rust
pub fn bump_tasks_injected(&mut self, agent_id: &str) {
    let a = self.activity_mut(agent_id);
    a.tasks_injected_count += 1;
    // Update contribution ratio: injected / processed (avoid divide by zero)
    let processed = a.tasks_processed_count.max(1) as f64;
    a.contribution_ratio = a.tasks_injected_count as f64 / processed;
}
```

And in `bump_tasks_processed`, update the ratio:
```rust
let a = self.activity_mut(agent_id);
a.tasks_processed_count += 1;
let injected = a.tasks_injected_count as f64;
a.contribution_ratio = injected / a.tasks_processed_count as f64;
```

**Step 3: Accept confidence_delta in submit_result RPC**

In `handle_submit_result` in rpc_server.rs, after extracting `merkle_proof`, also extract:
```rust
let confidence_delta = params.get("confidence_delta").and_then(|v| v.as_f64()).unwrap_or(0.0);
let task_outcome_str = params.get("task_outcome").and_then(|v| v.as_str()).unwrap_or("succeeded_fully");
```

Log the confidence_delta if it's significant (indicates agent uncertainty):
```rust
if confidence_delta > 0.2 {
    let mut s = state.write().await;
    s.push_log(
        crate::tui::LogCategory::Swarm,
        format!("Agent confidence dropped {:.2} during task {} — review suggested", confidence_delta, task_id_param)
    );
}
```

**Step 4: Add tests for new types in types.rs**

```rust
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
```

**Step 5: Run full test suite**

```bash
~/.cargo/bin/cargo test --workspace 2>&1 | tail -10
```

Expected: all tests pass, including the 3 new tests.

**Step 6: Commit**

```bash
git add crates/openswarm-protocol/src/types.rs crates/openswarm-connector/src/connector.rs crates/openswarm-connector/src/rpc_server.rs
git commit -m "feat(protocol): add TaskOutcome, FailureReason, CommitmentReceipt; contribution_ratio; confidence_delta"
```

---

## Task 9: Docker E2E Infrastructure

**Files:**
- Create: `docker/Dockerfile`
- Create: `docker/docker-compose.yml`
- Create: `tests/e2e/docker-e2e.sh`

**Step 1: Create docker/Dockerfile**

```dockerfile
# Stage 1: Build
FROM rust:1.75-slim AS builder

WORKDIR /app

# Install build dependencies
RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

# Copy workspace files
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/

# Build release binary
RUN cargo build --release --bin wws-connector

# Stage 2: Runtime
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/wws-connector /usr/local/bin/wws-connector

# Default listen port
EXPOSE 9370 9371

ENTRYPOINT ["wws-connector"]
```

**Step 2: Create docker/docker-compose.yml**

A 20-node network where node-1 is the bootstrap node and all others connect to it:

```yaml
version: '3.8'

networks:
  wws-net:
    driver: bridge
    ipam:
      config:
        - subnet: 172.28.0.0/16

services:
  wws-node-1:
    image: wws-connector:local
    build:
      context: ..
      dockerfile: docker/Dockerfile
    networks:
      wws-net:
        ipv4_address: 172.28.0.2
    ports:
      - "9370:9370"
      - "9371:9371"
    command: >
      --agent-name node-1
      --listen /ip4/0.0.0.0/tcp/9000
      --rpc 0.0.0.0:9370
      --files-addr 0.0.0.0:9371
    healthcheck:
      test: ["CMD", "curl", "-f", "http://localhost:9371/api/health"]
      interval: 5s
      timeout: 3s
      retries: 10

  wws-node-2:
    image: wws-connector:local
    networks:
      wws-net:
        ipv4_address: 172.28.0.3
    ports:
      - "9372:9370"
      - "9373:9371"
    command: >
      --agent-name node-2
      --listen /ip4/0.0.0.0/tcp/9000
      --rpc 0.0.0.0:9370
      --files-addr 0.0.0.0:9371
      --bootstrap /ip4/172.28.0.2/tcp/9000/p2p/BOOTSTRAP_PEER_ID
    depends_on:
      wws-node-1:
        condition: service_healthy
```

**NOTE**: The `BOOTSTRAP_PEER_ID` in the compose file is a placeholder. The actual peer ID is dynamic. For Docker E2E, we use a simpler approach: nodes discover each other via mDNS (enable-mdns flag) within the bridge network. Replace the above with mdns-based compose:

```yaml
version: '3.8'

# 20-node WWS network for E2E testing
# All nodes on same Docker bridge network, use mDNS for peer discovery

networks:
  wws-net:
    driver: bridge

x-node-base: &node-base
  image: wws-connector:local
  build:
    context: ..
    dockerfile: docker/Dockerfile
  networks:
    - wws-net
  restart: unless-stopped

services:
  wws-node-1:
    <<: *node-base
    ports: ["9370:9370", "9371:9371"]
    command: --agent-name node-1 --rpc 0.0.0.0:9370 --files-addr 0.0.0.0:9371
    healthcheck:
      test: ["CMD-SHELL", "curl -sf http://localhost:9371/api/health || exit 1"]
      interval: 5s
      timeout: 3s
      retries: 15

  wws-node-2:
    <<: *node-base
    ports: ["9372:9370", "9373:9371"]
    command: --agent-name node-2 --rpc 0.0.0.0:9370 --files-addr 0.0.0.0:9371
    depends_on: [wws-node-1]

  # Nodes 3–20 follow the same pattern with port offsets
  wws-node-3:
    <<: *node-base
    ports: ["9374:9370", "9375:9371"]
    command: --agent-name node-3 --rpc 0.0.0.0:9370 --files-addr 0.0.0.0:9371
    depends_on: [wws-node-1]

  wws-node-4:
    <<: *node-base
    ports: ["9376:9370", "9377:9371"]
    command: --agent-name node-4 --rpc 0.0.0.0:9370 --files-addr 0.0.0.0:9371
    depends_on: [wws-node-1]

  wws-node-5:
    <<: *node-base
    ports: ["9378:9370", "9379:9371"]
    command: --agent-name node-5 --rpc 0.0.0.0:9370 --files-addr 0.0.0.0:9371
    depends_on: [wws-node-1]

  wws-node-6:
    <<: *node-base
    ports: ["9380:9370", "9381:9371"]
    command: --agent-name node-6 --rpc 0.0.0.0:9370 --files-addr 0.0.0.0:9371
    depends_on: [wws-node-1]

  wws-node-7:
    <<: *node-base
    ports: ["9382:9370", "9383:9371"]
    command: --agent-name node-7 --rpc 0.0.0.0:9370 --files-addr 0.0.0.0:9371
    depends_on: [wws-node-1]

  wws-node-8:
    <<: *node-base
    ports: ["9384:9370", "9385:9371"]
    command: --agent-name node-8 --rpc 0.0.0.0:9370 --files-addr 0.0.0.0:9371
    depends_on: [wws-node-1]

  wws-node-9:
    <<: *node-base
    ports: ["9386:9370", "9387:9371"]
    command: --agent-name node-9 --rpc 0.0.0.0:9370 --files-addr 0.0.0.0:9371
    depends_on: [wws-node-1]

  wws-node-10:
    <<: *node-base
    ports: ["9388:9370", "9389:9371"]
    command: --agent-name node-10 --rpc 0.0.0.0:9370 --files-addr 0.0.0.0:9371
    depends_on: [wws-node-1]

  wws-node-11:
    <<: *node-base
    ports: ["9390:9370", "9391:9371"]
    command: --agent-name node-11 --rpc 0.0.0.0:9370 --files-addr 0.0.0.0:9371
    depends_on: [wws-node-1]

  wws-node-12:
    <<: *node-base
    ports: ["9392:9370", "9393:9371"]
    command: --agent-name node-12 --rpc 0.0.0.0:9370 --files-addr 0.0.0.0:9371
    depends_on: [wws-node-1]

  wws-node-13:
    <<: *node-base
    ports: ["9394:9370", "9395:9371"]
    command: --agent-name node-13 --rpc 0.0.0.0:9370 --files-addr 0.0.0.0:9371
    depends_on: [wws-node-1]

  wws-node-14:
    <<: *node-base
    ports: ["9396:9370", "9397:9371"]
    command: --agent-name node-14 --rpc 0.0.0.0:9370 --files-addr 0.0.0.0:9371
    depends_on: [wws-node-1]

  wws-node-15:
    <<: *node-base
    ports: ["9398:9370", "9399:9371"]
    command: --agent-name node-15 --rpc 0.0.0.0:9370 --files-addr 0.0.0.0:9371
    depends_on: [wws-node-1]

  wws-node-16:
    <<: *node-base
    ports: ["9400:9370", "9401:9371"]
    command: --agent-name node-16 --rpc 0.0.0.0:9370 --files-addr 0.0.0.0:9371
    depends_on: [wws-node-1]

  wws-node-17:
    <<: *node-base
    ports: ["9402:9370", "9403:9371"]
    command: --agent-name node-17 --rpc 0.0.0.0:9370 --files-addr 0.0.0.0:9371
    depends_on: [wws-node-1]

  wws-node-18:
    <<: *node-base
    ports: ["9404:9370", "9405:9371"]
    command: --agent-name node-18 --rpc 0.0.0.0:9370 --files-addr 0.0.0.0:9371
    depends_on: [wws-node-1]

  wws-node-19:
    <<: *node-base
    ports: ["9406:9370", "9407:9371"]
    command: --agent-name node-19 --rpc 0.0.0.0:9370 --files-addr 0.0.0.0:9371
    depends_on: [wws-node-1]

  wws-node-20:
    <<: *node-base
    ports: ["9408:9370", "9409:9371"]
    command: --agent-name node-20 --rpc 0.0.0.0:9370 --files-addr 0.0.0.0:9371
    depends_on: [wws-node-1]
```

**NOTE:** The Docker compose uses port mappings to the host. The E2E test will use ports 9370, 9372, 9374, ..., 9408 for RPC (even ports) and 9371, 9373, ..., 9409 for HTTP (odd ports).

**Step 3: Create tests/e2e/docker-e2e.sh**

```bash
#!/usr/bin/env bash
# Docker E2E test: build, spin up 20-node network, run Python E2E test, tear down.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
COMPOSE_FILE="$REPO_ROOT/docker/docker-compose.yml"

echo "=== Docker E2E Test ==="

# 1. Build the Docker image
echo "▶ Building Docker image..."
docker build -t wws-connector:local -f "$REPO_ROOT/docker/Dockerfile" "$REPO_ROOT"

# 2. Start all 20 nodes
echo "▶ Starting 20-node network..."
docker compose -f "$COMPOSE_FILE" up -d

# 3. Wait for node-1 to be healthy
echo "▶ Waiting for nodes to be ready..."
for i in $(seq 1 30); do
    if curl -sf http://127.0.0.1:9371/api/health > /dev/null 2>&1; then
        echo "  ✓ Node 1 ready"
        break
    fi
    sleep 2
done

# 4. Give other nodes time to start
sleep 10

# 5. Run the E2E test with Docker port mapping
echo "▶ Running E2E test against Docker nodes..."
python3 "$REPO_ROOT/tests/e2e/e2e_docker.py"

E2E_EXIT=$?

# 6. Tear down
echo "▶ Tearing down Docker network..."
docker compose -f "$COMPOSE_FILE" down

if [ $E2E_EXIT -eq 0 ]; then
    echo "=== Docker E2E PASSED ==="
else
    echo "=== Docker E2E FAILED ==="
    exit 1
fi
```

**Step 4: Create tests/e2e/e2e_docker.py (Docker port mapping)**

Copy `tests/e2e/e2e_20_agents.py` to `tests/e2e/e2e_docker.py` and change the PORTS definition to use Docker-mapped ports:

```python
# Docker compose maps node-N to host ports:
# node-1: rpc=9370, files=9371
# node-2: rpc=9372, files=9373
# ...
# node-N: rpc=9370+(N-1)*2, files=9371+(N-1)*2
PORTS = [(9370 + i*2, 9371 + i*2) for i in range(20)]
```

(Same as the original — Docker compose uses identical port mapping scheme.)

**Step 5: Make script executable**

```bash
chmod +x tests/e2e/docker-e2e.sh
```

**Step 6: Build Docker image locally to verify Dockerfile**

```bash
cd /Users/aostapenko/Work/OpenSwarm && docker build -t wws-connector:local -f docker/Dockerfile . 2>&1 | tail -20
```

Expected: Successfully built. NOTE: This will take a while (Rust compile inside Docker).

**Step 7: Commit**

```bash
git add docker/Dockerfile docker/docker-compose.yml tests/e2e/docker-e2e.sh tests/e2e/e2e_docker.py
git commit -m "feat(docker): Dockerfile and docker-compose.yml for 20-node E2E infrastructure"
```

---

## Task 10: Version Bump, Full Test Suite, Release v0.5.0

**Files:**
- Modify: `Cargo.toml` (workspace version)
- Modify: `docs/package.json`
- Modify: `README.md` (version references)
- Modify: `MEMORY.md` in the memory directory

**Step 1: Run full test suite before bumping**

```bash
~/.cargo/bin/cargo test --workspace 2>&1 | tail -20
```

Expected: all tests pass.

**Step 2: Bump version in workspace Cargo.toml**

Change line 13: `version = "0.4.0"` → `version = "0.5.0"`

**Step 3: Update docs/package.json**

Change `"version": "0.4.0"` → `"version": "0.5.0"`

**Step 4: Update README.md**

Replace all `0.4.0` references in download URLs and version table with `0.5.0`.
Replace all `0.3.9` references in filenames with `0.4.9` or check the actual release archive filenames match.

**Step 5: Run full test suite after version bump**

```bash
~/.cargo/bin/cargo test --workspace 2>&1 | tail -10
```

Expected: all tests pass.

**Step 6: Build release binary to verify**

```bash
~/.cargo/bin/cargo build --release --bin wws-connector 2>&1 | tail -5
./target/release/wws-connector --version
```

Expected: `wws-connector 0.5.0`

**Step 7: Start 20 local nodes and run native E2E test**

```bash
# Start nodes
for i in $(seq 0 19); do
    rpc_port=$((9370 + i*2))
    files_port=$((9371 + i*2))
    if [ $i -eq 0 ]; then
        ./target/release/wws-connector --agent-name "node-$((i+1))" --rpc 127.0.0.1:$rpc_port --files-addr 127.0.0.1:$files_port &
    else
        ./target/release/wws-connector --agent-name "node-$((i+1))" --rpc 127.0.0.1:$rpc_port --files-addr 127.0.0.1:$files_port &
    fi
done
sleep 8
python3 tests/e2e/e2e_20_agents.py
# Kill all nodes
kill $(lsof -ti:9370,9372,9374,9376,9378,9380,9382,9384,9386,9388,9390,9392,9394,9396,9398,9400,9402,9404,9406,9408 2>/dev/null) 2>/dev/null || true
```

Expected: E2E PASSED

**Step 8: Commit and tag**

```bash
git add Cargo.toml docs/package.json README.md
git commit -m "chore: bump version to v0.5.0"
git tag -a v0.5.0 -m "v0.5.0: Full reputation system, identity persistence, key rotation, TaskOutcome, Docker E2E"
git push origin ci-cd --tags
```

**Step 9: Wait for CI, then run Docker E2E with downloaded binary**

After CI builds release:
```bash
# Download macOS ARM64 binary
curl -LO https://github.com/Good-karma-lab/World-Wide-Swarm-Protocol/releases/download/v0.5.0/wws-connector-0.5.0-macos-arm64.tar.gz
tar xzf wws-connector-0.5.0-macos-arm64.tar.gz
./wws-connector --version
```

Then update Dockerfile to use the downloaded binary instead of building from source:
```dockerfile
# Alternative: use pre-built binary for faster Docker builds
FROM debian:bookworm-slim
COPY wws-connector /usr/local/bin/wws-connector
RUN chmod +x /usr/local/bin/wws-connector
EXPOSE 9370 9371
ENTRYPOINT ["wws-connector"]
```

Run Docker E2E:
```bash
bash tests/e2e/docker-e2e.sh
```

---

## Summary

After all 10 tasks, v0.5.0 includes:

| Feature | Status |
|---------|--------|
| PnCounter CRDT | ✅ |
| Reputation ledger (events, tiers, decay, observer weighting) | ✅ |
| swarm.get_reputation / submit / get_events RPCs | ✅ |
| /api/reputation with full ledger data | ✅ |
| Persistent Ed25519 key (chmod 0600) | ✅ |
| BIP-39 mnemonic on first run | ✅ |
| swarm.rotate_key RPC | ✅ |
| swarm.emergency_revocation RPC | ✅ |
| swarm.register_guardians / guardian_recovery_vote RPCs | ✅ |
| TaskOutcome + FailureReason (Moltbook #2) | ✅ |
| CommitmentReceipt (Moltbook #1) | ✅ |
| contribution_ratio in AgentActivity | ✅ |
| confidence_delta in submit_result | ✅ |
| Docker Dockerfile + docker-compose.yml | ✅ |
| Docker 20-node E2E test | ✅ |
| Release v0.5.0 | ✅ |
