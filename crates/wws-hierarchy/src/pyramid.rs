//! Dynamic Pyramid Allocation: compute depth = ceil(log_k(N)), assign tiers.
//!
//! The pyramid hierarchy organizes N agents into a tree of depth D where
//! each node oversees k subordinates. The structure adapts dynamically
//! as agents join or leave the swarm.
//!
//! Key formulas:
//! - Depth: D = ceil(log_k(N))
//! - Tier-1 leaders: ceil(N / k^(D-1))
//! - Each tier-t node oversees k tier-(t+1) nodes
//! - Executors are at the leaf level (tier D)

use wws_protocol::{Tier, MAX_HIERARCHY_DEPTH};

use crate::HierarchyError;

/// Configuration for the pyramid allocator.
#[derive(Debug, Clone)]
pub struct PyramidConfig {
    /// Branching factor (k): each node oversees this many subordinates.
    pub branching_factor: u32,
    /// Maximum allowed hierarchy depth.
    pub max_depth: u32,
}

impl Default for PyramidConfig {
    fn default() -> Self {
        Self {
            branching_factor: wws_protocol::DEFAULT_BRANCHING_FACTOR,
            max_depth: MAX_HIERARCHY_DEPTH,
        }
    }
}

/// Result of a pyramid allocation computation.
#[derive(Debug, Clone)]
pub struct PyramidLayout {
    /// Total depth of the hierarchy (number of tiers).
    pub depth: u32,
    /// Number of Tier-1 leaders needed.
    pub tier1_count: u32,
    /// Number of agents at each tier (index 0 = tier 1).
    pub agents_per_tier: Vec<u32>,
    /// Total swarm size used for this computation.
    pub swarm_size: u64,
    /// Branching factor used.
    pub branching_factor: u32,
}

/// Tier distribution result from the static `distribute` function.
///
/// Contains the number of agents at each tier level, with index 0
/// representing Tier-1 (leaders).
#[derive(Debug, Clone)]
pub struct TierDistribution {
    /// Number of agents at each tier (index 0 = tier-1).
    pub tiers: Vec<u64>,
}

/// Manages the dynamic pyramid hierarchy allocation.
///
/// Given the current swarm size N and branching factor k, computes
/// the optimal hierarchy structure and assigns agents to tiers.
pub struct PyramidAllocator {
    config: PyramidConfig,
    current_layout: Option<PyramidLayout>,
}

impl PyramidAllocator {
    /// Create a new pyramid allocator with the given configuration.
    pub fn new(config: PyramidConfig) -> Self {
        Self {
            config,
            current_layout: None,
        }
    }

    /// Compute the hierarchy depth for a given swarm size.
    ///
    /// D = ceil(log_k(N)), clamped to [1, max_depth].
    pub fn compute_depth(&self, swarm_size: u64) -> u32 {
        if swarm_size <= 1 {
            return 1;
        }

        let k = self.config.branching_factor as f64;
        let n = swarm_size as f64;
        let depth = n.log(k).ceil() as u32;

        depth.clamp(1, self.config.max_depth)
    }

    /// Compute the full pyramid layout for a given swarm size.
    ///
    /// Returns the number of agents needed at each tier level.
    pub fn compute_layout(&self, swarm_size: u64) -> Result<PyramidLayout, HierarchyError> {
        let depth = self.compute_depth(swarm_size);

        if depth > self.config.max_depth {
            return Err(HierarchyError::MaxDepthExceeded(self.config.max_depth));
        }

        let k = self.config.branching_factor;
        let mut agents_per_tier = Vec::with_capacity(depth as usize);
        let mut remaining = swarm_size;

        // Tier 1 gets ceil(N / k^(D-1)) leaders.
        let tier1_count = if depth <= 1 {
            swarm_size as u32
        } else {
            let divisor = k.pow(depth - 1) as u64;
            ((swarm_size + divisor - 1) / divisor) as u32
        };
        agents_per_tier.push(tier1_count);
        remaining = remaining.saturating_sub(tier1_count as u64);

        // Intermediate tiers: each tier has k times the tier above.
        for tier_idx in 1..depth.saturating_sub(1) {
            let count = (agents_per_tier[tier_idx as usize - 1] * k).min(remaining as u32);
            agents_per_tier.push(count);
            remaining = remaining.saturating_sub(count as u64);
        }

        // Executor tier gets all remaining agents.
        if depth > 1 {
            agents_per_tier.push(remaining as u32);
        }

        Ok(PyramidLayout {
            depth,
            tier1_count,
            agents_per_tier,
            swarm_size,
            branching_factor: k,
        })
    }

    /// Recompute the layout for a new swarm size and store it.
    pub fn recompute(&mut self, swarm_size: u64) -> Result<&PyramidLayout, HierarchyError> {
        let layout = self.compute_layout(swarm_size)?;
        self.current_layout = Some(layout);
        Ok(self.current_layout.as_ref().expect("just set"))
    }

    /// Get the current layout, if computed.
    pub fn current_layout(&self) -> Option<&PyramidLayout> {
        self.current_layout.as_ref()
    }

    /// Determine the tier assignment for an agent given their rank.
    ///
    /// Agents are sorted by composite score (highest first). The rank
    /// determines which tier they are placed in based on the current layout.
    pub fn assign_tier(
        &self,
        rank: usize,
        layout: &PyramidLayout,
    ) -> Tier {
        let mut cumulative = 0u32;
        let last_tier_idx = layout.agents_per_tier.len().saturating_sub(1);
        for (tier_idx, &count) in layout.agents_per_tier.iter().enumerate() {
            cumulative += count;
            if (rank as u32) < cumulative {
                return match tier_idx {
                    0 if layout.agents_per_tier.len() == 1 => Tier::Executor,
                    0 => Tier::Tier0,
                    n if n == last_tier_idx => Tier::Executor,
                    1 => Tier::Tier1,
                    2 => Tier::Tier2,
                    n => Tier::TierN(n as u32),
                };
            }
        }
        Tier::Executor
    }

    /// Compute the parent assignment for an agent based on tier and branch.
    ///
    /// Within a given tier, agents are grouped into branches of size k.
    /// Each branch is overseen by one agent in the tier above.
    pub fn compute_parent_index(
        &self,
        agent_rank_in_tier: usize,
    ) -> usize {
        agent_rank_in_tier / self.config.branching_factor as usize
    }

    /// Get the branching factor.
    pub fn branching_factor(&self) -> u32 {
        self.config.branching_factor
    }

    /// Get the maximum hierarchy depth.
    pub fn max_depth(&self) -> u32 {
        self.config.max_depth
    }

    // ── Static convenience methods (used by tests and protocol layer) ──

    /// Compute hierarchy depth as a static function.
    ///
    /// `D = ceil(log_k(N))`, clamped to `[0, MAX_HIERARCHY_DEPTH]`.
    /// Returns 0 for `n == 0`, 1 for `n <= k`.
    pub fn compute_depth_static(n: u64, k: u64) -> u32 {
        if n == 0 {
            return 0;
        }
        if n <= 1 {
            return 1;
        }
        if k <= 1 {
            // Degenerate: linear chain
            return (n as u32).min(MAX_HIERARCHY_DEPTH);
        }
        let depth = (n as f64).log(k as f64).ceil() as u32;
        depth.clamp(1, MAX_HIERARCHY_DEPTH)
    }

    /// Distribute N agents across tiers with branching factor k.
    ///
    /// Returns a `TierDistribution` whose `tiers` vector contains the
    /// agent count at each tier (index 0 = Tier-1). The sum of all
    /// tiers equals `n`.
    pub fn distribute(n: u64, k: u64) -> TierDistribution {
        if n == 0 {
            return TierDistribution { tiers: vec![] };
        }
        let depth = Self::compute_depth_static(n, k);
        if depth == 0 {
            return TierDistribution { tiers: vec![] };
        }
        if depth == 1 {
            return TierDistribution { tiers: vec![n] };
        }
        let mut tiers: Vec<u64> = Vec::with_capacity(depth as usize);
        let mut remaining = n;

        // Tier-1: min(k, n) leaders
        let tier1 = std::cmp::min(k, n);
        tiers.push(tier1);
        remaining -= tier1;

        // Intermediate tiers: each has k * tier_above, but capped by remaining
        for i in 1..depth.saturating_sub(1) {
            let above = tiers[i as usize - 1];
            let ideal = above * k;
            let count = std::cmp::min(ideal, remaining);
            tiers.push(count);
            remaining -= count;
        }

        // Bottom tier gets all remaining
        if depth > 1 {
            tiers.push(remaining);
        }

        TierDistribution { tiers }
    }
}

impl Default for PyramidAllocator {
    fn default() -> Self {
        Self::new(PyramidConfig::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_depth_single_agent() {
        let allocator = PyramidAllocator::default();
        assert_eq!(allocator.compute_depth(1), 1);
    }

    #[test]
    fn test_depth_small_network() {
        let allocator = PyramidAllocator::default(); // k=10
        // 10 agents: log_10(10) = 1, ceil = 1
        assert_eq!(allocator.compute_depth(10), 1);
        // 11 agents: log_10(11) ≈ 1.04, ceil = 2
        assert_eq!(allocator.compute_depth(11), 2);
        // 100 agents: log_10(100) = 2, ceil = 2
        assert_eq!(allocator.compute_depth(100), 2);
        // 101 agents: log_10(101) ≈ 2.004, ceil = 3
        assert_eq!(allocator.compute_depth(101), 3);
    }

    #[test]
    fn test_depth_large_network() {
        let allocator = PyramidAllocator::default(); // k=10
        // 10000 agents: log_10(10000) = 4
        assert_eq!(allocator.compute_depth(10_000), 4);
        // 1 million: log_10(1M) = 6
        assert_eq!(allocator.compute_depth(1_000_000), 6);
    }

    #[test]
    fn test_layout_100_agents() {
        let allocator = PyramidAllocator::default(); // k=10
        let layout = allocator.compute_layout(100).unwrap();
        assert_eq!(layout.depth, 2);
        assert_eq!(layout.tier1_count, 10); // ceil(100/10) = 10
        assert_eq!(layout.agents_per_tier.len(), 2);
    }

    #[test]
    fn test_tier_assignment() {
        let allocator = PyramidAllocator::default();
        let layout = allocator.compute_layout(100).unwrap();
        // First 10 agents → Tier0 (task initiators)
        assert_eq!(allocator.assign_tier(0, &layout), Tier::Tier0);
        assert_eq!(allocator.assign_tier(9, &layout), Tier::Tier0);
        // Remaining 90 → Executor (depth=2, so no intermediate tiers)
        assert_eq!(allocator.assign_tier(10, &layout), Tier::Executor);
    }

    #[test]
    fn test_parent_index() {
        let allocator = PyramidAllocator::default(); // k=10
        assert_eq!(allocator.compute_parent_index(0), 0);
        assert_eq!(allocator.compute_parent_index(9), 0);
        assert_eq!(allocator.compute_parent_index(10), 1);
        assert_eq!(allocator.compute_parent_index(25), 2);
    }
}
