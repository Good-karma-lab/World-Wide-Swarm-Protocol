//! Tests for the Dynamic Pyramid Allocation algorithm.
//!
//! Verifies (per S5.2 of the protocol spec):
//! - Hierarchy depth = ceil(log_k(N))
//! - Tier population distribution
//! - Edge cases: N=1, N=k, N=k^2, etc.
//! - Partial last tier

use wws_hierarchy::pyramid::{PyramidAllocator, PyramidConfig};
use wws_protocol::{Tier, MAX_HIERARCHY_DEPTH};

/// Helper: create a default allocator (k=10, max_depth=MAX_HIERARCHY_DEPTH).
fn default_allocator() -> PyramidAllocator {
    PyramidAllocator::default()
}

/// Helper: create an allocator with a custom branching factor.
fn allocator_with_k(k: u32) -> PyramidAllocator {
    PyramidAllocator::new(PyramidConfig {
        branching_factor: k,
        max_depth: MAX_HIERARCHY_DEPTH,
    })
}

// =====================================================================
// S 5.2 Hierarchy Depth Calculation
// =====================================================================

#[test]
fn depth_single_agent() {
    let alloc = default_allocator();
    assert_eq!(alloc.compute_depth(1), 1);
}

#[test]
fn depth_exactly_k_agents() {
    let alloc = default_allocator();
    assert_eq!(alloc.compute_depth(10), 1);
}

#[test]
fn depth_k_plus_one() {
    let alloc = default_allocator();
    assert_eq!(alloc.compute_depth(11), 2);
}

#[test]
fn depth_exactly_k_squared() {
    let alloc = default_allocator();
    // 100 agents, k=10: log_10(100) = 2 => depth 2
    assert_eq!(alloc.compute_depth(100), 2);
}

#[test]
fn depth_k_squared_plus_one() {
    let alloc = default_allocator();
    assert_eq!(alloc.compute_depth(101), 3);
}

#[test]
fn depth_850_agents() {
    let alloc = default_allocator();
    // ceil(log_10(850)) = ceil(2.929) = 3
    assert_eq!(alloc.compute_depth(850), 3);
}

#[test]
fn depth_1000_agents() {
    let alloc = default_allocator();
    // log_10(1000) = 3 exactly
    assert_eq!(alloc.compute_depth(1000), 3);
}

#[test]
fn depth_1001_agents() {
    let alloc = default_allocator();
    assert_eq!(alloc.compute_depth(1001), 4);
}

#[test]
fn depth_10000_agents() {
    let alloc = default_allocator();
    assert_eq!(alloc.compute_depth(10000), 4);
}

#[test]
fn depth_with_k_2() {
    let alloc = allocator_with_k(2);
    // Binary tree
    assert_eq!(alloc.compute_depth(1), 1);
    assert_eq!(alloc.compute_depth(2), 1);
    assert_eq!(alloc.compute_depth(3), 2);
    assert_eq!(alloc.compute_depth(4), 2);
    assert_eq!(alloc.compute_depth(5), 3);
    assert_eq!(alloc.compute_depth(8), 3);
    assert_eq!(alloc.compute_depth(9), 4);
}

// =====================================================================
// Layout Distribution
// =====================================================================

#[test]
fn layout_10_agents_k10() {
    let alloc = default_allocator();
    let layout = alloc.compute_layout(10).unwrap();
    assert_eq!(layout.agents_per_tier.len(), 1);
    assert_eq!(layout.agents_per_tier[0], 10, "All 10 agents should be at tier-1");
}

#[test]
fn layout_100_agents_k10() {
    let alloc = default_allocator();
    let layout = alloc.compute_layout(100).unwrap();
    assert_eq!(layout.agents_per_tier.len(), 2);
    assert_eq!(layout.agents_per_tier[0], 10, "Tier-1 must have ceil(100/10) = 10 agents");
    assert_eq!(layout.agents_per_tier[1], 90, "Tier-2 gets remaining agents");
}

#[test]
fn layout_850_agents_k10() {
    let alloc = default_allocator();
    let layout = alloc.compute_layout(850).unwrap();
    assert_eq!(layout.agents_per_tier.len(), 3);
    // depth=3, tier1 = ceil(850 / 10^2) = ceil(8.5) = 9
    assert_eq!(layout.agents_per_tier[0], 9, "Tier-1: ceil(850/100) = 9 agents");
    // tier2 = min(9 * 10, 841) = 90
    assert_eq!(layout.agents_per_tier[1], 90, "Tier-2: 9 * 10 = 90 agents");
    // tier3 = remaining: 850 - 9 - 90 = 751
    assert_eq!(layout.agents_per_tier[2], 751, "Tier-3: remaining agents");
}

#[test]
fn layout_total_matches_n() {
    let alloc = default_allocator();
    for n in [1u64, 5, 10, 50, 100, 500, 850, 1000, 5000] {
        let layout = alloc.compute_layout(n).unwrap();
        let total: u64 = layout.agents_per_tier.iter().map(|&c| c as u64).sum();
        assert_eq!(total, n, "Total agents in layout must equal N={}", n);
    }
}

#[test]
fn layout_tier1_never_exceeds_k() {
    let alloc = default_allocator();
    for n in [1u64, 5, 10, 100, 1000, 10000] {
        let layout = alloc.compute_layout(n).unwrap();
        assert!(
            layout.agents_per_tier[0] <= 10,
            "Tier-1 must never exceed k agents for N={}",
            n
        );
    }
}

#[test]
fn layout_tier1_uses_min_n_k() {
    let alloc = default_allocator();
    // If N < k, tier-1 has N agents (depth=1, all agents in one tier)
    let layout = alloc.compute_layout(5).unwrap();
    assert_eq!(layout.agents_per_tier[0], 5);
    assert_eq!(layout.agents_per_tier.len(), 1);
}

// =====================================================================
// Tier Assignment
// =====================================================================

#[test]
fn tier_assignment_100_agents() {
    let alloc = default_allocator();
    let layout = alloc.compute_layout(100).unwrap();
    // First 10 agents (rank 0..9) -> Tier0 (task initiators)
    assert_eq!(alloc.assign_tier(0, &layout), Tier::Tier0);
    assert_eq!(alloc.assign_tier(9, &layout), Tier::Tier0);
    // Remaining 90 agents (rank 10..99) -> Executor (depth=2, no intermediate tiers)
    assert_eq!(alloc.assign_tier(10, &layout), Tier::Executor);
    assert_eq!(alloc.assign_tier(99, &layout), Tier::Executor);
}

#[test]
fn tier_assignment_deep_hierarchy() {
    let alloc = default_allocator();
    let layout = alloc.compute_layout(850).unwrap();
    // Tier-0: ranks 0..8 (9 agents — task initiators)
    assert_eq!(alloc.assign_tier(0, &layout), Tier::Tier0);
    assert_eq!(alloc.assign_tier(8, &layout), Tier::Tier0);
    // Tier-1: ranks 9..98 (90 agents — board members)
    assert_eq!(alloc.assign_tier(9, &layout), Tier::Tier1);
    assert_eq!(alloc.assign_tier(98, &layout), Tier::Tier1);
    // Executor: ranks 99..849 (751 agents)
    assert_eq!(alloc.assign_tier(99, &layout), Tier::Executor);
    assert_eq!(alloc.assign_tier(849, &layout), Tier::Executor);
}

// =====================================================================
// Parent Index Computation
// =====================================================================

#[test]
fn parent_index_k10() {
    let alloc = default_allocator();
    assert_eq!(alloc.compute_parent_index(0), 0);
    assert_eq!(alloc.compute_parent_index(9), 0);
    assert_eq!(alloc.compute_parent_index(10), 1);
    assert_eq!(alloc.compute_parent_index(25), 2);
}

// =====================================================================
// Edge Cases
// =====================================================================

#[test]
fn depth_zero_agents() {
    let alloc = default_allocator();
    // Implementation treats N<=1 as depth 1
    assert_eq!(alloc.compute_depth(0), 1);
}

#[test]
fn layout_zero_agents() {
    let alloc = default_allocator();
    let layout = alloc.compute_layout(0).unwrap();
    let total: u64 = layout.agents_per_tier.iter().map(|&c| c as u64).sum();
    assert_eq!(total, 0);
}

#[test]
fn layout_k_equals_1() {
    // Degenerate case: k=1 causes infinite depth, clamped to max_depth.
    // All agents end up in tier-1 since k^(D-1) = 1^(D-1) = 1, ceil(N/1) = N.
    let alloc = allocator_with_k(1);
    let layout = alloc.compute_layout(5).unwrap();
    assert_eq!(layout.agents_per_tier[0], 5);
    let total: u64 = layout.agents_per_tier.iter().map(|&c| c as u64).sum();
    assert_eq!(total, 5, "Total must still equal N");
}

#[test]
fn max_depth_capped() {
    let alloc = default_allocator();
    // With very large N, depth should be capped at MAX_HIERARCHY_DEPTH
    let depth = alloc.compute_depth(u64::MAX);
    assert!(
        depth <= MAX_HIERARCHY_DEPTH,
        "Depth must be capped at MAX_HIERARCHY_DEPTH"
    );
}

// =====================================================================
// Recompute and Stateful API
// =====================================================================

#[test]
fn recompute_stores_layout() {
    let mut alloc = default_allocator();
    assert!(alloc.current_layout().is_none(), "No layout before recompute");
    let layout = alloc.recompute(100).unwrap();
    assert_eq!(layout.depth, 2);
    assert_eq!(layout.swarm_size, 100);
    // current_layout should now be populated
    let stored = alloc.current_layout().expect("Layout should be stored after recompute");
    assert_eq!(stored.depth, 2);
    assert_eq!(stored.swarm_size, 100);
}

#[test]
fn recompute_updates_on_new_size() {
    let mut alloc = default_allocator();
    alloc.recompute(100).unwrap();
    assert_eq!(alloc.current_layout().unwrap().swarm_size, 100);
    alloc.recompute(1000).unwrap();
    assert_eq!(alloc.current_layout().unwrap().swarm_size, 1000);
    assert_eq!(alloc.current_layout().unwrap().depth, 3);
}

#[test]
fn layout_metadata_fields() {
    let alloc = default_allocator();
    let layout = alloc.compute_layout(100).unwrap();
    assert_eq!(layout.swarm_size, 100);
    assert_eq!(layout.branching_factor, 10);
    assert_eq!(layout.depth, 2);
    assert_eq!(layout.tier1_count, 10);
}
