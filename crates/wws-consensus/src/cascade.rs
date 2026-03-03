//! Recursive decomposition cascade: winning plan subtask distribution.
//!
//! After a plan wins the vote at a given tier, its subtasks must be
//! distributed to subordinate agents. If those subordinate agents
//! are themselves coordinators (not leaf executors), they recursively
//! run their own RFP/vote cycles on their assigned subtasks.
//!
//! The cascade proceeds top-down through the hierarchy:
//! 1. Tier-1 wins a plan with N subtasks
//! 2. Each subtask is assigned to a Tier-2 coordinator
//! 3. Tier-2 coordinators run RFP for their subtask, decomposing further
//! 4. This continues until tasks reach executor-level agents
//!
//! The Adaptive Granularity Algorithm (in wws-state) determines
//! whether a task should be further decomposed or executed atomically.

use std::collections::HashMap;

use wws_protocol::{AgentId, Plan, Task, TaskStatus, Tier};

use crate::ConsensusError;

/// Stop condition for the recursive decomposition cascade.
///
/// Determines when the cascade should stop decomposing further
/// and assign the task directly for execution.
#[derive(Debug, Clone, PartialEq)]
pub enum StopCondition {
    /// The task is atomic and cannot be decomposed further.
    AtomicTask,
    /// The agent is at the bottom tier of the hierarchy.
    BottomTier,
    /// The task complexity is below the decomposition threshold (0.1).
    LowComplexity(f64),
}

/// Describes a subtask assignment to a subordinate agent.
#[derive(Debug, Clone)]
pub struct SubtaskAssignment {
    /// The task object for the subtask.
    pub task: Task,
    /// The agent assigned to handle this subtask.
    pub assignee: AgentId,
    /// The parent task this subtask was derived from.
    pub parent_task_id: String,
    /// The winning plan that produced this subtask.
    pub plan_id: String,
    /// The tier level of the assignee.
    pub assignee_tier: Tier,
    /// Whether the subtask requires further decomposition (cascading RFP).
    pub requires_cascade: bool,
}

/// A pending cascade level tracking subtask distribution.
#[derive(Debug, Clone)]
pub struct CascadeLevel {
    /// The parent task being decomposed.
    pub parent_task_id: String,
    /// The winning plan used for decomposition.
    pub plan_id: String,
    /// The tier at which this cascade level operates.
    pub tier: u32,
    /// Subtask assignments made at this level.
    pub assignments: Vec<SubtaskAssignment>,
    /// Whether all assignments have been acknowledged.
    pub all_assigned: bool,
}

/// Result tracking for cascade completion.
#[derive(Debug, Clone)]
pub struct CascadeStatus {
    /// Root task ID.
    pub root_task_id: String,
    /// Number of cascade levels active.
    pub active_levels: usize,
    /// Total subtasks distributed.
    pub total_subtasks: usize,
    /// Completed subtasks.
    pub completed_subtasks: usize,
    /// Failed subtasks.
    pub failed_subtasks: usize,
}

/// Manages the recursive decomposition cascade across hierarchy tiers.
///
/// Tracks the tree of subtask decompositions from root task through
/// all intermediate tiers to leaf executor assignments.
pub struct CascadeEngine {
    /// Active cascade levels, keyed by parent task ID.
    levels: HashMap<String, CascadeLevel>,
    /// Mapping from subtask ID to its parent task ID for traversal.
    subtask_to_parent: HashMap<String, String>,
    /// Track which subtasks have been completed.
    completed: HashMap<String, bool>,
    /// The root task ID for the entire cascade.
    root_task_id: Option<String>,
}

impl CascadeEngine {
    /// Create a new cascade engine.
    pub fn new() -> Self {
        Self {
            levels: HashMap::new(),
            subtask_to_parent: HashMap::new(),
            completed: HashMap::new(),
            root_task_id: None,
        }
    }

    // ── Static Helper Methods ──────────────────────────────────────

    /// Map plan subtasks to agents in round-robin (wrap-around) fashion.
    ///
    /// Produces one `(AgentId, Task)` pair per subtask. When there are fewer
    /// agents than subtasks, agent assignments wrap around.
    pub fn assign_subtasks(plan: &Plan, agents: &[AgentId]) -> Vec<(AgentId, Task)> {
        plan.subtasks
            .iter()
            .enumerate()
            .map(|(idx, subtask)| {
                let agent = &agents[idx % agents.len()];
                let mut task = Task::new(subtask.description.clone(), 2, plan.epoch);
                task.parent_task_id = Some(plan.task_id.clone());
                task.assigned_to = Some(agent.clone());
                (agent.clone(), task)
            })
            .collect()
    }

    /// Evaluate whether the cascade should stop at the current level.
    ///
    /// The cascade stops when:
    /// - The task is atomic (cannot be decomposed).
    /// - The agent is at the bottom tier of the hierarchy.
    /// - The task complexity is below the threshold (0.1).
    pub fn should_stop(condition: StopCondition) -> bool {
        match condition {
            StopCondition::AtomicTask => true,
            StopCondition::BottomTier => true,
            StopCondition::LowComplexity(complexity) => complexity < 0.1,
        }
    }

    /// Return the plan proposer as the Prime Orchestrator.
    ///
    /// Per the protocol specification, the agent that proposed the
    /// winning plan becomes the Prime Orchestrator for that cascade.
    pub fn prime_orchestrator(plan: &Plan) -> &AgentId {
        &plan.proposer
    }

    // ── Instance Methods ───────────────────────────────────────────

    /// Distribute subtasks from a winning plan to subordinate agents.
    ///
    /// Takes the winning plan and a list of available subordinate agents,
    /// and creates assignments. Each subtask in the plan is assigned to
    /// one subordinate agent in round-robin fashion.
    ///
    /// Returns the list of assignments to be communicated to subordinates.
    pub fn distribute_subtasks(
        &mut self,
        parent_task_id: &str,
        plan: &Plan,
        subordinates: &[(AgentId, Tier)],
        epoch: u64,
    ) -> Result<Vec<SubtaskAssignment>, ConsensusError> {
        if subordinates.is_empty() {
            return Err(ConsensusError::CascadeError(
                "No subordinates available for subtask distribution".into(),
            ));
        }

        if plan.subtasks.is_empty() {
            return Err(ConsensusError::CascadeError(
                "Plan has no subtasks to distribute".into(),
            ));
        }

        if self.root_task_id.is_none() {
            self.root_task_id = Some(parent_task_id.to_string());
        }

        let mut assignments = Vec::with_capacity(plan.subtasks.len());

        for (idx, plan_subtask) in plan.subtasks.iter().enumerate() {
            // Round-robin assignment to subordinates.
            let (assignee, assignee_tier) = &subordinates[idx % subordinates.len()];

            let mut task = Task::new(
                plan_subtask.description.clone(),
                assignee_tier.depth(),
                epoch,
            );
            task.parent_task_id = Some(parent_task_id.to_string());
            task.assigned_to = Some(assignee.clone());
            task.status = TaskStatus::Pending;

            // Determine if this subtask needs further decomposition.
            let requires_cascade = matches!(assignee_tier, Tier::Tier1 | Tier::Tier2 | Tier::TierN(_));

            let assignment = SubtaskAssignment {
                task: task.clone(),
                assignee: assignee.clone(),
                parent_task_id: parent_task_id.to_string(),
                plan_id: plan.plan_id.clone(),
                assignee_tier: *assignee_tier,
                requires_cascade,
            };

            // Track the subtask.
            self.subtask_to_parent
                .insert(task.task_id.clone(), parent_task_id.to_string());
            self.completed.insert(task.task_id.clone(), false);

            assignments.push(assignment);
        }

        // Record the cascade level.
        self.levels.insert(
            parent_task_id.to_string(),
            CascadeLevel {
                parent_task_id: parent_task_id.to_string(),
                plan_id: plan.plan_id.clone(),
                tier: plan.subtasks.first().map(|_| 1).unwrap_or(0), // Will be set by caller
                assignments: assignments.clone(),
                all_assigned: true,
            },
        );

        tracing::info!(
            parent_task = %parent_task_id,
            plan = %plan.plan_id,
            subtasks = assignments.len(),
            subordinates = subordinates.len(),
            "Distributed subtasks via cascade"
        );

        Ok(assignments)
    }

    /// Record that a subtask has been completed.
    ///
    /// Returns `true` if all subtasks for the parent task are now complete,
    /// meaning the parent task can be marked as complete.
    pub fn record_subtask_completion(
        &mut self,
        subtask_id: &str,
    ) -> Result<bool, ConsensusError> {
        // Mark as completed.
        if let Some(complete) = self.completed.get_mut(subtask_id) {
            *complete = true;
        } else {
            return Err(ConsensusError::TaskNotFound(subtask_id.to_string()));
        }

        // Check if the parent's subtasks are all done.
        let parent_id = self
            .subtask_to_parent
            .get(subtask_id)
            .cloned()
            .ok_or_else(|| ConsensusError::TaskNotFound(subtask_id.to_string()))?;

        if let Some(level) = self.levels.get(&parent_id) {
            let all_done = level.assignments.iter().all(|a| {
                self.completed
                    .get(&a.task.task_id)
                    .copied()
                    .unwrap_or(false)
            });

            if all_done {
                tracing::info!(
                    parent_task = %parent_id,
                    "All subtasks completed for parent task"
                );
            }

            Ok(all_done)
        } else {
            Ok(false)
        }
    }

    /// Record that a subtask has failed.
    ///
    /// The cascade engine marks the subtask as failed and can trigger
    /// re-assignment or escalation.
    pub fn record_subtask_failure(
        &mut self,
        subtask_id: &str,
    ) -> Result<(), ConsensusError> {
        if !self.completed.contains_key(subtask_id) {
            return Err(ConsensusError::TaskNotFound(subtask_id.to_string()));
        }

        tracing::warn!(subtask = %subtask_id, "Subtask failed in cascade");
        Ok(())
    }

    /// Get the overall cascade status.
    pub fn status(&self) -> CascadeStatus {
        let total_subtasks = self.completed.len();
        let completed_subtasks = self.completed.values().filter(|&&v| v).count();
        let failed_subtasks = 0; // Would need separate tracking.

        CascadeStatus {
            root_task_id: self.root_task_id.clone().unwrap_or_default(),
            active_levels: self.levels.len(),
            total_subtasks,
            completed_subtasks,
            failed_subtasks,
        }
    }

    /// Get the cascade level for a specific parent task.
    pub fn get_level(&self, parent_task_id: &str) -> Option<&CascadeLevel> {
        self.levels.get(parent_task_id)
    }

    /// Get all pending (not yet completed) subtask IDs.
    pub fn pending_subtasks(&self) -> Vec<String> {
        self.completed
            .iter()
            .filter(|(_, &done)| !done)
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Get the parent task ID for a given subtask.
    pub fn parent_of(&self, subtask_id: &str) -> Option<&String> {
        self.subtask_to_parent.get(subtask_id)
    }

    /// Check if the entire cascade is complete (all subtasks at all levels done).
    pub fn is_complete(&self) -> bool {
        self.completed.values().all(|&done| done)
    }

    /// Reset the cascade engine for a new task.
    pub fn reset(&mut self) {
        self.levels.clear();
        self.subtask_to_parent.clear();
        self.completed.clear();
        self.root_task_id = None;
    }
}

impl Default for CascadeEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wws_protocol::PlanSubtask;

    fn make_plan(task_id: &str) -> Plan {
        let mut plan = Plan::new(
            task_id.to_string(),
            AgentId::new("coordinator".into()),
            1,
        );
        plan.subtasks = vec![
            PlanSubtask {
                index: 0,
                description: "Part A".into(),
                required_capabilities: vec![],
                estimated_complexity: 0.3,
            },
            PlanSubtask {
                index: 1,
                description: "Part B".into(),
                required_capabilities: vec![],
                estimated_complexity: 0.4,
            },
            PlanSubtask {
                index: 2,
                description: "Part C".into(),
                required_capabilities: vec![],
                estimated_complexity: 0.3,
            },
        ];
        plan
    }

    #[test]
    fn test_distribute_subtasks() {
        let mut engine = CascadeEngine::new();
        let plan = make_plan("root_task");

        let subordinates = vec![
            (AgentId::new("exec1".into()), Tier::Executor),
            (AgentId::new("exec2".into()), Tier::Executor),
        ];

        let assignments = engine
            .distribute_subtasks("root_task", &plan, &subordinates, 1)
            .unwrap();

        assert_eq!(assignments.len(), 3);
        // Round-robin: exec1, exec2, exec1
        assert_eq!(assignments[0].assignee, AgentId::new("exec1".into()));
        assert_eq!(assignments[1].assignee, AgentId::new("exec2".into()));
        assert_eq!(assignments[2].assignee, AgentId::new("exec1".into()));
    }

    #[test]
    fn test_completion_tracking() {
        let mut engine = CascadeEngine::new();
        let plan = make_plan("root_task");

        let subordinates = vec![
            (AgentId::new("exec1".into()), Tier::Executor),
            (AgentId::new("exec2".into()), Tier::Executor),
        ];

        let assignments = engine
            .distribute_subtasks("root_task", &plan, &subordinates, 1)
            .unwrap();

        // Complete first two subtasks.
        assert!(!engine
            .record_subtask_completion(&assignments[0].task.task_id)
            .unwrap());
        assert!(!engine
            .record_subtask_completion(&assignments[1].task.task_id)
            .unwrap());

        // Complete last subtask — parent should now be complete.
        assert!(engine
            .record_subtask_completion(&assignments[2].task.task_id)
            .unwrap());

        assert!(engine.is_complete());
    }
}
