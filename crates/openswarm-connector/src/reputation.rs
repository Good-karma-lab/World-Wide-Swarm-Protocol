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
    /// Compute the score tier from an effective score value.
    pub fn from_score(score: i64) -> ScoreTier {
        match score {
            s if s < 0 => ScoreTier::Suspended,
            0..=99 => ScoreTier::Newcomer,
            100..=499 => ScoreTier::Member,
            500..=999 => ScoreTier::Trusted,
            1000..=4999 => ScoreTier::Established,
            _ => ScoreTier::Veteran,
        }
    }

    /// Returns the display name of this tier as a static string.
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
    pub fn min_inject_score(complexity: f64) -> i64 {
        if complexity <= 1.0 {
            100
        } else if complexity <= 5.0 {
            500
        } else {
            1000
        }
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
pub fn effective_score(
    raw: i64,
    last_active: chrono::DateTime<chrono::Utc>,
    peak: i64,
) -> i64 {
    let now = chrono::Utc::now();
    let hours_total = (now - last_active).num_hours().max(0) as f64;
    let days_total = hours_total / 24.0;
    // 2-day grace period before decay starts
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
    /// Actual points applied after observer weighting.
    pub effective_points: i64,
    pub task_id: Option<String>,
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub evidence: Option<String>,
}

/// All reputation event types per the spec (Section 1.4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RepEventType {
    // Positive events
    /// Task executed and verified via Merkle-DAG: +10 (objective)
    TaskExecutedVerified,
    /// High-quality result (quality ≥ 0.8): +5 (observer weighted)
    HighQualityResult,
    /// Proposed plan selected by IRV: +15 (objective)
    PlanSelectedByIrv,
    /// Accurate critique (within ±20% of IRV consensus): +8 (objective)
    AccurateCritique,
    /// Cast vote in IRV: +2 (objective)
    VoteCastInIrv,
    /// Redundant execution matches expected hash: +5 (objective)
    RedundantExecutionMatch,
    /// Helped new agent bootstrap: +5 (observer weighted)
    HelpedNewAgent,
    /// Continuous online 24h (96 keepalives received): +3 (objective)
    OnlineFor24h,
    /// First to join a board (holon formation): +1 (objective)
    FirstToJoinBoard,
    // Negative events
    /// Task accepted but not delivered (timeout): -10
    TaskAcceptedNotDelivered,
    /// Submitted result with wrong hash: -25
    WrongResultHash,
    /// Submitted plan rejected unanimously (0 votes): -15
    PlanRejectedUnanimously,
    /// Replay attack attempt detected: -100
    ReplayAttackDetected,
    /// RPC rate limit exceeded: -20
    RpcRateLimitExceeded,
    /// Sybil registration flood (>3 agents same IP in 1h): -200
    SybilFlood,
    /// Name squatting (Levenshtein ≤ 1 from high-rep name): -50
    NameSquatting,
    /// Critique wildly off consensus (>50% deviation): -5
    WildlyOffCritique,
    /// Missing keepalive 5+ consecutive intervals: -1
    MissingKeepalive,
}

impl RepEventType {
    /// Base points for this event type (positive = earn, negative = penalty).
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
    ///
    /// Objective events are verifiable without observer opinion (Merkle-DAG, PoW, keepalive).
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
                | Self::WildlyOffCritique
                | Self::MissingKeepalive
        )
    }
}

/// Compute observer-weighted contribution per spec Section 1.3.
///
/// `contribution = base_points × min(1.0, observer_score / 1000)`
///
/// For objective events, weight is always 1.0.
/// For new observers (score 0), subjective events contribute 0 points.
pub fn observer_weighted_points(base_points: i64, observer_score: i64, is_objective: bool) -> i64 {
    if is_objective {
        return base_points;
    }
    let weight = (observer_score as f64 / 1000.0).clamp(0.0, 1.0);
    (base_points as f64 * weight) as i64
}

/// Per-agent reputation ledger (event log + running scores).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReputationLedger {
    /// Running raw score (not decayed; decay applied lazily on read).
    pub raw_score: i64,
    /// Lifetime peak score (used as floor for decay: cannot decay below peak/2).
    pub peak_score: i64,
    /// Last active timestamp (used to compute decay period).
    pub last_active: chrono::DateTime<chrono::Utc>,
    /// Full event history (capped at 500 entries; oldest removed first).
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
    /// Compute the effective (decay-adjusted) score.
    pub fn effective_score(&self) -> i64 {
        effective_score(self.raw_score, self.last_active, self.peak_score)
    }

    /// Compute the current reputation tier.
    pub fn tier(&self) -> ScoreTier {
        score_tier(self.effective_score())
    }

    /// Apply a reputation event, updating raw_score, peak_score, last_active, and event log.
    pub fn apply_event(&mut self, event: RepEvent) {
        self.raw_score += event.effective_points;
        if self.raw_score > self.peak_score {
            self.peak_score = self.raw_score;
        }
        // Update last_active whenever a non-zero event occurs
        if event.effective_points != 0 {
            self.last_active = event.timestamp;
        }
        // Cap event history at 500 entries
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
    fn test_effective_score_within_grace_no_decay() {
        // Within 48h grace period, no decay applied
        let last_active = chrono::Utc::now() - chrono::Duration::hours(24);
        let score = effective_score(100, last_active, 100);
        assert_eq!(score, 100);
    }

    #[test]
    fn test_effective_score_floor() {
        // After a long absence, score cannot drop below 50% of peak
        let last_active = chrono::Utc::now() - chrono::Duration::days(365);
        let score = effective_score(100, last_active, 200);
        assert_eq!(score, 100); // floor = 200/2 = 100
    }

    #[test]
    fn test_effective_score_decay_after_grace() {
        // After 48h grace + 100 days, significant decay should occur
        // but floor (peak/2) should apply if decay goes too far
        let last_active = chrono::Utc::now() - chrono::Duration::days(100);
        // 98 days of actual decay: 1000 * (0.995^98) ≈ 613
        // peak = 1000, floor = 500
        let score = effective_score(1000, last_active, 1000);
        assert!(score >= 500, "score {} should be >= floor 500", score);
        assert!(score < 1000, "score {} should have decayed below 1000", score);
    }

    #[test]
    fn test_observer_weighted_objective_always_full() {
        // Objective events always get full base points regardless of observer score
        assert_eq!(observer_weighted_points(10, 0, true), 10);
        assert_eq!(observer_weighted_points(10, 50, true), 10);
        assert_eq!(observer_weighted_points(-10, 0, true), -10);
    }

    #[test]
    fn test_observer_weighted_subjective_scales_with_score() {
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
        assert_eq!(ledger.tier(), ScoreTier::Newcomer); // 10 < 100
    }

    #[test]
    fn test_reputation_ledger_peak_tracks_max() {
        let mut ledger = ReputationLedger::default();
        // Earn 200 points
        ledger.apply_event(RepEvent {
            event_type: RepEventType::PlanSelectedByIrv,
            base_points: 200,
            observer: "self".into(),
            observer_score: 0,
            effective_points: 200,
            task_id: None,
            timestamp: chrono::Utc::now(),
            evidence: None,
        });
        assert_eq!(ledger.peak_score, 200);
        // Apply a penalty
        ledger.apply_event(RepEvent {
            event_type: RepEventType::WrongResultHash,
            base_points: -25,
            observer: "other".into(),
            observer_score: 0,
            effective_points: -25,
            task_id: None,
            timestamp: chrono::Utc::now(),
            evidence: None,
        });
        assert_eq!(ledger.raw_score, 175);
        assert_eq!(ledger.peak_score, 200); // peak unchanged by penalty
    }

    #[test]
    fn test_min_inject_score() {
        assert_eq!(ScoreTier::min_inject_score(0.5), 100);
        assert_eq!(ScoreTier::min_inject_score(1.0), 100);
        assert_eq!(ScoreTier::min_inject_score(1.1), 500);
        assert_eq!(ScoreTier::min_inject_score(5.0), 500);
        assert_eq!(ScoreTier::min_inject_score(5.1), 1000);
        assert_eq!(ScoreTier::min_inject_score(100.0), 1000);
    }

    #[test]
    fn test_all_event_types_have_base_points() {
        // Ensure every event type has a defined base_points value
        let types = [
            RepEventType::TaskExecutedVerified,
            RepEventType::HighQualityResult,
            RepEventType::PlanSelectedByIrv,
            RepEventType::AccurateCritique,
            RepEventType::VoteCastInIrv,
            RepEventType::RedundantExecutionMatch,
            RepEventType::HelpedNewAgent,
            RepEventType::OnlineFor24h,
            RepEventType::FirstToJoinBoard,
            RepEventType::TaskAcceptedNotDelivered,
            RepEventType::WrongResultHash,
            RepEventType::PlanRejectedUnanimously,
            RepEventType::ReplayAttackDetected,
            RepEventType::RpcRateLimitExceeded,
            RepEventType::SybilFlood,
            RepEventType::NameSquatting,
            RepEventType::WildlyOffCritique,
            RepEventType::MissingKeepalive,
        ];
        // All positive events should have positive base_points
        assert!(RepEventType::TaskExecutedVerified.base_points() > 0);
        assert!(RepEventType::PlanSelectedByIrv.base_points() > 0);
        // All negative events should have negative base_points
        assert!(RepEventType::WrongResultHash.base_points() < 0);
        assert!(RepEventType::ReplayAttackDetected.base_points() < 0);
        // Ensure none return 0 (every event should have a defined effect)
        for t in &types {
            assert_ne!(t.base_points(), 0, "{:?} has zero base_points", t);
        }
    }

    #[test]
    fn test_reputation_ledger_event_log_cap() {
        let mut ledger = ReputationLedger::default();
        for _ in 0..502 {
            ledger.apply_event(RepEvent {
                event_type: RepEventType::MissingKeepalive,
                base_points: -1,
                observer: "self".into(),
                observer_score: 0,
                effective_points: -1,
                task_id: None,
                timestamp: chrono::Utc::now(),
                evidence: None,
            });
        }
        assert_eq!(ledger.events.len(), 500);
    }
}
