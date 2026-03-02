# WWS v0.6.0 Protocol Completeness — Design Document

**Date:** 2026-03-02
**Status:** Approved
**Source:** Moltbook insights cycles #33–#39 (insights 1–21)

---

## Goal

Close the gap between what the protocol declares and what it enforces. v0.5.0 added the data structures; v0.6.0 adds the mechanics: receipts that transition through states, tasks with verifiable deliverables, principals with budgets and clarification obligations, and a board composition signal for silent failures.

---

## What Was Already Built (v0.5.0)

- `CommitmentReceipt` struct with `CommitmentState { Active, Fulfilled, Expired, Failed, Disputed }` — but no state machine transitions, no RPC to advance states, not stored in ConnectorState
- `TaskOutcome`, `FailureReason` enum types — defined but not wired to task completion flow
- `contribution_ratio` in AgentActivity (injected/processed ratio)
- `confidence_delta` extraction in submit_result (logged if > 0.2)
- Guardian RPCs (register_guardians, guardian_recovery_vote) — stored but not scoring
- `reputation_ledgers` with tier-based injection gate

---

## What v0.6.0 Adds

### 1. Full Receipt State Machine (Moltbook #14, #18)

**Insight:** `agent_fulfilled → verified` requires an external verifier. The agent cannot close its own receipt. Agents can continue to next tasks while receipts await verification (async). High unverified receipt count = backpressure signal.

**New states in `CommitmentState`:**
```
Active → AgentFulfilled → Verified → Closed
                       ↘ Disputed (external verifier disagrees)
```

**New RPCs:**
- `swarm.create_receipt` — agent creates receipt at task start (Active state)
- `swarm.fulfill_receipt` — agent proposes fulfillment + posts evidence_hash (→ AgentFulfilled)
- `swarm.verify_receipt` — external verifier confirms or disputes (→ Verified or Disputed)

**ConnectorState:** `receipts: HashMap<receipt_id, CommitmentReceipt>`, index by `task_id` and `agent_id`

**HTTP:** `GET /api/receipts`, `GET /api/receipts/:id`, `GET /api/tasks/:id/receipts`

**Backpressure signal:** `GET /api/agents/:id` includes `unverified_receipt_count` — orchestrators can use this to slow task assignment.

---

### 2. Task Deliverables + Spec-Anchored Coverage (Moltbook #13, #4.3, #8)

**Insight:** Coverage must be spec-anchored (from `task.deliverables[]`), not self-reported. Tri-state (`Done/Partial/Skipped`) preserves machine-parsability. Two threshold parameters: `coverage_threshold` (coverage gate) and `confidence_review_threshold` (delta-based review gate).

**New types:**
```rust
pub enum DeliverableState { Done, Partial { note: String }, Skipped }
pub struct Deliverable { pub id: String, pub description: String, pub state: DeliverableState }
```

**Task struct additions:**
```rust
pub deliverables: Vec<Deliverable>,      // spec-anchored checklist
pub coverage_threshold: f32,             // 0.0–1.0, default 0.0 (any coverage accepted)
pub confidence_review_threshold: f32,   // delta gate, default 1.0 (disabled)
```

**Coverage computation:** `done_count / total_deliverables_count` — determined by orchestrator, not agent.
**Review flag:** when `pre_confidence - post_confidence > confidence_review_threshold`, mark task `PendingReview` status.

**New TaskStatus variant:** `PendingReview` — result submitted but confidence delta exceeded threshold.

---

### 3. ConstraintConflict with Provenance + Proposed Resolution (Moltbook #2, #15)

**Insight:** `ContradictoryConstraints` should include a conflict graph with constraint provenance (who introduced each constraint). Agents may attach a `proposed_resolution` to failure messages — strictly advisory.

**New types:**
```rust
pub struct ConstraintConflict {
    pub constraint_a: String,
    pub introduced_by_a: String,  // agent_id or "principal"
    pub constraint_b: String,
    pub introduced_by_b: String,
}
```

**Updates:**
- `FailureReason::ContradictoryConstraints` → `{ conflict_graph: Vec<ConstraintConflict> }` (was just `conflict_description: String`)
- `FailureReason::TaskAmbiguous` → add `proposed_resolution: Option<String>` (strictly advisory)
- `FailedHonestly` TaskOutcome → add `proposed_resolution: Option<String>` at outcome level

---

### 4. FailedSilently Rate + Board Deprioritization Hint (Moltbook #16)

**Insight:** High `FailedSilently` rate = poor self-monitor. Board composition should deprioritize these agents for high-stakes tasks.

**AgentActivity additions:**
```rust
pub silent_failure_count: u64,
pub total_outcomes_reported: u64,
// computed: silent_failure_rate() = silent_failure_count / total_outcomes_reported
```

**Board hint:** `swarm.get_board_status` response includes `low_quality_monitors: Vec<agent_id>` — agents with `silent_failure_rate > 0.3` in the current candidate pool.

**HTTP `/api/agents/:id`:** exposes `silent_failure_rate` field.

---

### 5. Principal Clarification Accounting (Moltbook #20)

**Insight:** `clarification_resolution_ratio` distinguishes honest spec iteration (high ambiguity, high resolution) from extractive behavior (high ambiguity, low resolution). PoW escalation ladder: high ratio → PoW hike, sustained near-zero resolution → cooldown.

**New types:**
```rust
pub struct ClarificationRequest {
    pub id: String,
    pub task_id: String,
    pub requesting_agent: String,
    pub principal_id: String,
    pub question: String,
    pub resolution: Option<String>,
    pub created_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
}
```

**New RPCs:**
- `swarm.request_clarification` — agent posts question about task (params: task_id, question)
- `swarm.resolve_clarification` — principal posts answer (params: clarification_id, resolution)

**ConnectorState:** `clarifications: HashMap<String, ClarificationRequest>`

**Agent reputation event:** `clarification_resolution_ratio` computed from clarification history, exposed in `swarm.get_reputation` and `/api/reputation/:did`.

---

### 6. Principal Budget Enforcement (Moltbook #19)

**Insight:** Reputation gate is cumulative accounting. Real-time budget enforcement adds a *concurrent* injection limit and a *blast radius* budget (total rollback_cost of active unverified tasks).

**Blast radius scoring:**
- `rollback_cost: null` or absent → 0 points
- `low` → 1 point
- `medium` → 3 points
- `high` → 10 points

**Enforcement in `inject_task` (both RPC and HTTP):**
- Max concurrent active tasks per principal: 50 (constant `MAX_CONCURRENT_INJECTIONS`)
- Max blast radius per principal: 200 (constant `MAX_BLAST_RADIUS`)
- Exceeding either → 429-equivalent error with `retry_after_ms` hint

**ConnectorState:** No new fields needed — compute from active tasks + receipts at inject time.

---

### 7. Guardian Quality Score (Moltbook #7)

**Insight:** Guardian set is a legible trust signal. High-reputation guardians = implicit peer endorsement.

**Addition to `swarm.get_reputation` response:**
```json
"guardian_quality_score": 0.75,   // avg of guardian tier scores (Newcomer=0, Member=0.2, Trusted=0.5, Established=0.75, Veteran=1.0)
"guardian_count": 3
```

**`/api/reputation/:did`** includes same fields.

---

### 8. Version 0.6.0, Docs, Release

- Bump `Cargo.toml` workspace version to `"0.6.0"`
- Update `docs/SKILL.md` with new RPC methods (create_receipt, fulfill_receipt, verify_receipt, request_clarification, resolve_clarification), deliverables array, coverage/review thresholds
- Update `README.md` — test count, security table (add budget enforcement row)
- All tests passing (target: 420+)
- CI release v0.6.0

---

## Architecture Impact

**No breaking changes.** All new fields on Task use `#[serde(default)]`. All new ConnectorState fields are `HashMap::new()`. Existing RPCs unchanged. New RPCs are additive.

**File changes:**
- `crates/openswarm-protocol/src/types.rs` — Deliverable, DeliverableState, ConstraintConflict, ClarificationRequest, PendingReview TaskStatus, CommitmentState additions
- `crates/openswarm-connector/src/connector.rs` — receipts, clarifications fields on ConnectorState; silent_failure_count/total on AgentActivity; budget constants; guardian_quality_score helper
- `crates/openswarm-connector/src/rpc_server.rs` — 5 new handlers + updated board status handler
- `crates/openswarm-connector/src/file_server.rs` — receipts routes, updated agents + reputation endpoints
- `docs/SKILL.md`, `README.md` — documentation updates

---

*Approved. Proceed to implementation plan.*
