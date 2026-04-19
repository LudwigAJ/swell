//! Subagent spawning system with hierarchical depth management.
//!
//! This module implements sub-agent spawning with strict 2-level maximum depth
//! enforcement to prevent runaway agent trees.
//!
//! # Architecture
//!
//! - [`SubagentSpawner`] - Main spawner for creating sub-agents
//! - [`Subagent`] - Represents a spawned sub-agent with depth tracking
//! - [`SubagentTree`] - Manages the hierarchical agent tree structure
//! - [`AgentTreeNode`] - Individual node in the agent tree
//!
//! # Depth Enforcement
//!
//! Maximum depth is 2 levels:
//! - Level 0: Root orchestrator (depth = 0)
//! - Level 1: First-level sub-agents spawned by root (depth = 1)
//! - Level 2: Second-level sub-agents spawned by first-level (depth = 2, MAX)
//!
//! Agents at depth 2 cannot spawn further sub-agents.

use swell_core::ids::TaskId;
use tracing::{debug, info, warn};
use uuid::Uuid;

/// Maximum depth of sub-agents in the hierarchy (2 levels max)
pub const MAX_SUBAGENT_DEPTH: u32 = 2;

/// Error type for subagent operations
#[derive(Debug, thiserror::Error)]
pub enum SubagentError {
    #[error("Maximum subagent depth ({max}) exceeded for task {task_id}")]
    MaxDepthExceeded { task_id: TaskId, max: u32 },

    #[error("Cannot spawn subagent at depth {depth} for task {task_id}: {reason}")]
    CannotSpawn {
        task_id: TaskId,
        depth: u32,
        reason: String,
    },

    #[error("Subagent not found: {0}")]
    SubagentNotFound(String),

    #[error("Task not found: {0}")]
    TaskNotFound(String),
}

impl SubagentError {
    pub fn subagent_not_found(id: Uuid) -> Self {
        Self::SubagentNotFound(id.to_string())
    }

    pub fn task_not_found(id: Uuid) -> Self {
        Self::TaskNotFound(id.to_string())
    }
}

/// Reason why a subagent spawn was attempted
#[derive(Debug, Clone, PartialEq)]
pub enum SpawnReason {
    /// Subagent spawned for a complex task requiring decomposition
    TaskDecomposition,
    /// Subagent spawned for parallel execution of independent steps
    ParallelExecution,
    /// Subagent spawned for specialized handling (code review, testing, etc.)
    SpecializedHandling,
    /// Subagent spawned due toFeatureLead splitting a large plan
    FeatureLeadSplit,
}

/// A spawned sub-agent with its metadata and state
#[derive(Debug, Clone, PartialEq)]
pub struct Subagent {
    /// Unique identifier for this subagent
    pub id: Uuid,
    /// ID of the parent agent or orchestrator that spawned this
    pub parent_id: Option<Uuid>,
    /// ID of the task this subagent is working on
    pub task_id: TaskId,
    /// Depth in the agent hierarchy (0 = root, 1 = first level, 2 = second level)
    pub depth: u32,
    /// Human-readable name for this subagent
    pub name: String,
    /// Description of what this subagent does
    pub description: String,
    /// Reason why this subagent was spawned
    pub spawn_reason: SpawnReason,
    /// Whether this subagent has completed its work
    pub completed: bool,
    /// Whether this subagent can spawn further subagents
    pub can_spawn: bool,
}

impl Subagent {
    /// Create a new subagent at the root level (depth 0)
    pub fn new_root(task_id: TaskId, name: String, description: String) -> Self {
        Self {
            id: Uuid::new_v4(),
            parent_id: None,
            task_id,
            depth: 0,
            name,
            description,
            spawn_reason: SpawnReason::TaskDecomposition,
            completed: false,
            can_spawn: true, // Root can spawn level 1 subagents
        }
    }

    /// Create a new first-level subagent (spawned by root)
    pub fn first_level(
        task_id: TaskId,
        parent_id: Uuid,
        name: String,
        description: String,
        reason: SpawnReason,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            parent_id: Some(parent_id),
            task_id,
            depth: 1,
            name,
            description,
            spawn_reason: reason,
            completed: false,
            can_spawn: true, // Level 1 can spawn level 2 subagents
        }
    }

    /// Create a new second-level subagent (spawned by first-level)
    pub fn second_level(
        task_id: TaskId,
        parent_id: Uuid,
        name: String,
        description: String,
        reason: SpawnReason,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            parent_id: Some(parent_id),
            task_id,
            depth: 2,
            name,
            description,
            spawn_reason: reason,
            completed: false,
            can_spawn: false, // Level 2 cannot spawn further subagents
        }
    }

    /// Check if this subagent is at max depth
    pub fn is_at_max_depth(&self) -> bool {
        self.depth >= MAX_SUBAGENT_DEPTH
    }

    /// Check if this subagent can spawn child subagents
    pub fn can_spawn_subagent(&self) -> bool {
        self.can_spawn && !self.is_at_max_depth()
    }

    /// Mark this subagent as completed
    pub fn mark_completed(&mut self) {
        self.completed = true;
        info!(
            subagent_id = %self.id,
            depth = self.depth,
            "Subagent marked as completed"
        );
    }
}

/// A node in the agent tree representing a subagent
#[derive(Debug)]
pub struct AgentTreeNode {
    /// The subagent this node represents
    pub subagent: Subagent,
    /// Child subagents spawned by this one
    children: Vec<Uuid>,
}

impl AgentTreeNode {
    /// Create a new tree node for a subagent
    pub fn new(subagent: Subagent) -> Self {
        Self {
            subagent,
            children: Vec::new(),
        }
    }

    /// Add a child subagent ID
    pub fn add_child(&mut self, child_id: Uuid) {
        self.children.push(child_id);
    }

    /// Get all child subagent IDs
    pub fn children(&self) -> &[Uuid] {
        &self.children
    }
}

/// Manages the hierarchical agent tree structure
#[derive(Debug, Default)]
pub struct SubagentTree {
    /// All subagents indexed by ID
    nodes: std::collections::HashMap<Uuid, AgentTreeNode>,
    /// Root subagent IDs per task
    roots: std::collections::HashMap<TaskId, Uuid>,
}

impl SubagentTree {
    /// Create a new empty agent tree
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a new subagent into the tree
    pub fn insert(&mut self, subagent: Subagent) {
        let id = subagent.id;

        // If this is a root subagent (no parent), register it
        if subagent.parent_id.is_none() {
            self.roots.insert(subagent.task_id, id);
        }

        self.nodes.insert(id, AgentTreeNode::new(subagent));
    }

    /// Connect a child subagent to its parent
    pub fn connect(&mut self, child_id: Uuid, parent_id: Uuid) -> Result<(), SubagentError> {
        // Verify both nodes exist
        if !self.nodes.contains_key(&child_id) {
            return Err(SubagentError::SubagentNotFound(child_id.to_string()));
        }
        if !self.nodes.contains_key(&parent_id) {
            return Err(SubagentError::SubagentNotFound(parent_id.to_string()));
        }

        // Add child reference to parent
        if let Some(parent_node) = self.nodes.get_mut(&parent_id) {
            parent_node.add_child(child_id);

            // Update child's parent_id
            if let Some(child_node) = self.nodes.get_mut(&child_id) {
                child_node.subagent.parent_id = Some(parent_id);
            }
        }

        Ok(())
    }

    /// Get a subagent by ID
    pub fn get(&self, subagent_id: &Uuid) -> Option<&Subagent> {
        self.nodes.get(subagent_id).map(|n| &n.subagent)
    }

    /// Get a mutable subagent by ID
    pub fn get_mut(&mut self, subagent_id: &Uuid) -> Option<&mut Subagent> {
        self.nodes.get_mut(subagent_id).map(|n| &mut n.subagent)
    }

    /// Get the root subagent for a task
    pub fn get_root(&self, task_id: &TaskId) -> Option<&Subagent> {
        self.roots.get(task_id).and_then(|id| self.get(id))
    }

    /// Get all subagents for a task
    pub fn get_all_for_task(&self, task_id: &TaskId) -> Vec<&Subagent> {
        self.nodes
            .values()
            .filter(|n| n.subagent.task_id == *task_id)
            .map(|n| &n.subagent)
            .collect()
    }

    /// Get child subagents of a given subagent
    pub fn get_children(&self, subagent_id: &Uuid) -> Vec<&Subagent> {
        self.nodes
            .get(subagent_id)
            .map(|n| n.children.iter().filter_map(|cid| self.get(cid)).collect())
            .unwrap_or_default()
    }

    /// Get the parent of a subagent
    pub fn get_parent(&self, subagent_id: &Uuid) -> Option<&Subagent> {
        self.nodes
            .get(subagent_id)
            .and_then(|n| n.subagent.parent_id)
            .and_then(|pid| self.get(&pid))
    }

    /// Get the depth of a subagent
    pub fn get_depth(&self, subagent_id: &Uuid) -> Option<u32> {
        self.nodes.get(subagent_id).map(|n| n.subagent.depth)
    }

    /// Count total subagents for a task
    pub fn count(&self, task_id: &TaskId) -> usize {
        self.nodes
            .iter()
            .filter(|(_, n)| n.subagent.task_id == *task_id)
            .count()
    }

    /// Check if a subagent is a leaf (no children)
    pub fn is_leaf(&self, subagent_id: &Uuid) -> bool {
        self.nodes
            .get(subagent_id)
            .map(|n| n.children.is_empty())
            .unwrap_or(true)
    }

    /// Mark a subagent as completed
    pub fn mark_completed(&mut self, subagent_id: &Uuid) -> Result<(), SubagentError> {
        if let Some(node) = self.nodes.get_mut(subagent_id) {
            node.subagent.mark_completed();
            Ok(())
        } else {
            Err(SubagentError::SubagentNotFound(subagent_id.to_string()))
        }
    }

    /// Get all completed subagents for a task
    pub fn get_completed(&self, task_id: &TaskId) -> Vec<&Subagent> {
        self.nodes
            .values()
            .filter(|n| n.subagent.task_id == *task_id && n.subagent.completed)
            .map(|n| &n.subagent)
            .collect()
    }

    /// Get all active (not completed) subagents for a task
    pub fn get_active(&self, task_id: &TaskId) -> Vec<&Subagent> {
        self.nodes
            .values()
            .filter(|n| n.subagent.task_id == *task_id && !n.subagent.completed)
            .map(|n| &n.subagent)
            .collect()
    }

    /// Validate depth constraint - returns true if any subagent exceeds max depth
    pub fn validate_depth_constraint(&self) -> bool {
        self.nodes
            .values()
            .all(|n| n.subagent.depth <= MAX_SUBAGENT_DEPTH)
    }
}

/// Subagent spawner with depth enforcement.
///
/// This is the main entry point for spawning sub-agents with hierarchical
/// depth management.
#[derive(Debug, Default)]
pub struct SubagentSpawner {
    /// The agent tree tracking all spawned subagents
    tree: SubagentTree,
    /// Statistics about spawning
    stats: SpawnStats,
}

#[derive(Debug, Clone, Default)]
pub struct SpawnStats {
    /// Total subagents spawned
    pub total_spawned: usize,
    /// Total subagents completed
    pub total_completed: usize,
    /// Spawns rejected due to max depth
    pub depth_rejections: usize,
}

impl SubagentSpawner {
    /// Create a new subagent spawner
    pub fn new() -> Self {
        Self::default()
    }

    /// Spawn a new root-level subagent (level 0)
    ///
    /// This creates a subagent that can spawn level 1 children.
    pub fn spawn_root(
        &mut self,
        task_id: TaskId,
        name: String,
        description: String,
    ) -> Result<Subagent, SubagentError> {
        info!(
            task_id = %task_id,
            name = %name,
            "Spawning root subagent"
        );

        let subagent = Subagent::new_root(task_id, name, description);
        self.tree.insert(subagent.clone());
        self.stats.total_spawned += 1;

        debug!(
            subagent_id = %subagent.id,
            depth = subagent.depth,
            "Root subagent spawned successfully"
        );

        Ok(subagent)
    }

    /// Spawn a child subagent from a parent
    ///
    /// Returns an error if the parent is at max depth (2).
    pub fn spawn_child(
        &mut self,
        task_id: TaskId,
        parent_id: Uuid,
        name: String,
        description: String,
        reason: SpawnReason,
    ) -> Result<Subagent, SubagentError> {
        // Check if parent exists and can spawn
        let parent = self
            .tree
            .get(&parent_id)
            .ok_or(SubagentError::SubagentNotFound(parent_id.to_string()))?;

        // Check depth constraint
        if parent.is_at_max_depth() {
            self.stats.depth_rejections += 1;
            warn!(
                task_id = %task_id,
                parent_id = %parent_id,
                parent_depth = parent.depth,
                "Spawn rejected: parent at max depth"
            );
            return Err(SubagentError::MaxDepthExceeded {
                task_id,
                max: MAX_SUBAGENT_DEPTH,
            });
        }

        if !parent.can_spawn_subagent() {
            self.stats.depth_rejections += 1;
            return Err(SubagentError::CannotSpawn {
                task_id,
                depth: parent.depth + 1,
                reason: "Parent subagent has can_spawn=false".to_string(),
            });
        }

        // Calculate the child depth
        let child_depth = parent.depth + 1;

        // Create the child subagent based on depth
        let subagent = match child_depth {
            1 => Subagent::first_level(task_id, parent_id, name, description, reason),
            2 => Subagent::second_level(task_id, parent_id, name, description, reason),
            _ => {
                self.stats.depth_rejections += 1;
                return Err(SubagentError::MaxDepthExceeded {
                    task_id,
                    max: MAX_SUBAGENT_DEPTH,
                });
            }
        };

        // Insert and connect
        self.tree.insert(subagent.clone());
        self.tree.connect(subagent.id, parent_id)?;
        self.stats.total_spawned += 1;

        info!(
            subagent_id = %subagent.id,
            parent_id = %parent_id,
            depth = subagent.depth,
            can_spawn = subagent.can_spawn,
            "Child subagent spawned successfully"
        );

        Ok(subagent)
    }

    /// Get a subagent by ID
    pub fn get(&self, subagent_id: &Uuid) -> Option<&Subagent> {
        self.tree.get(subagent_id)
    }

    /// Get all subagents for a task
    pub fn get_all_for_task(&self, task_id: &TaskId) -> Vec<&Subagent> {
        self.tree.get_all_for_task(task_id)
    }

    /// Get children of a subagent
    pub fn get_children(&self, subagent_id: &Uuid) -> Vec<&Subagent> {
        self.tree.get_children(subagent_id)
    }

    /// Get the parent of a subagent
    pub fn get_parent(&self, subagent_id: &Uuid) -> Option<&Subagent> {
        self.tree.get_parent(subagent_id)
    }

    /// Mark a subagent as completed
    pub fn mark_completed(&mut self, subagent_id: &Uuid) -> Result<(), SubagentError> {
        self.tree.mark_completed(subagent_id)?;
        self.stats.total_completed += 1;
        Ok(())
    }

    /// Get all completed subagents for a task
    pub fn get_completed(&self, task_id: &TaskId) -> Vec<&Subagent> {
        self.tree.get_completed(task_id)
    }

    /// Get all active subagents for a task
    pub fn get_active(&self, task_id: &TaskId) -> Vec<&Subagent> {
        self.tree.get_active(task_id)
    }

    /// Check if all subagents for a task are completed
    pub fn is_task_completed(&self, task_id: &TaskId) -> bool {
        let active = self.get_active(task_id);
        active.is_empty()
    }

    /// Get the depth of a subagent
    pub fn get_depth(&self, subagent_id: &Uuid) -> Option<u32> {
        self.tree.get_depth(subagent_id)
    }

    /// Get current statistics
    pub fn stats(&self) -> &SpawnStats {
        &self.stats
    }

    /// Validate that no subagent exceeds max depth
    pub fn validate_depth(&self) -> bool {
        self.tree.validate_depth_constraint()
    }

    /// Get the total count of subagents for a task
    pub fn count(&self, task_id: &TaskId) -> usize {
        self.tree.count(task_id)
    }

    /// Check if spawning is allowed at a given depth
    pub fn can_spawn_at_depth(&self, parent_id: Uuid, _task_id: TaskId) -> bool {
        self.tree
            .get(&parent_id)
            .map(|p| p.can_spawn_subagent())
            .unwrap_or(false)
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // --- Subagent Creation Tests ---

    #[test]
    fn test_subagent_new_root() {
        let task_id = TaskId::new();
        let subagent = Subagent::new_root(
            task_id,
            "Root Agent".to_string(),
            "Main coordinator".to_string(),
        );

        assert_eq!(subagent.depth, 0);
        assert!(subagent.parent_id.is_none());
        assert!(subagent.can_spawn_subagent());
        assert!(!subagent.is_at_max_depth());
        assert!(!subagent.completed);
    }

    #[test]
    fn test_subagent_first_level() {
        let task_id = TaskId::new();
        let parent_id = Uuid::new_v4();
        let subagent = Subagent::first_level(
            task_id,
            parent_id,
            "Worker 1".to_string(),
            "Handles portion A".to_string(),
            SpawnReason::TaskDecomposition,
        );

        assert_eq!(subagent.depth, 1);
        assert_eq!(subagent.parent_id, Some(parent_id));
        assert!(subagent.can_spawn_subagent());
        assert!(!subagent.is_at_max_depth());
    }

    #[test]
    fn test_subagent_second_level() {
        let task_id = TaskId::new();
        let parent_id = Uuid::new_v4();
        let subagent = Subagent::second_level(
            task_id,
            parent_id,
            "Detail Worker".to_string(),
            "Handles specific task".to_string(),
            SpawnReason::ParallelExecution,
        );

        assert_eq!(subagent.depth, 2);
        assert_eq!(subagent.parent_id, Some(parent_id));
        assert!(!subagent.can_spawn_subagent()); // Cannot spawn at max depth
        assert!(subagent.is_at_max_depth());
    }

    #[test]
    fn test_subagent_is_at_max_depth() {
        let task_id = TaskId::new();
        let parent_id = Uuid::new_v4();

        let root = Subagent::new_root(task_id, "Root".to_string(), "".to_string());
        let level1 = Subagent::first_level(
            task_id,
            parent_id,
            "L1".to_string(),
            "".to_string(),
            SpawnReason::TaskDecomposition,
        );
        let level2 = Subagent::second_level(
            task_id,
            parent_id,
            "L2".to_string(),
            "".to_string(),
            SpawnReason::TaskDecomposition,
        );

        assert!(!root.is_at_max_depth());
        assert!(!level1.is_at_max_depth());
        assert!(level2.is_at_max_depth());
    }

    // --- SubagentTree Tests ---

    #[test]
    fn test_tree_insert_and_get() {
        let mut tree = SubagentTree::new();
        let task_id = TaskId::new();

        let root = Subagent::new_root(task_id, "Root".to_string(), "".to_string());
        tree.insert(root.clone());

        assert_eq!(tree.get(&root.id), Some(&root));
        assert_eq!(tree.get_root(&task_id), Some(&root));
    }

    #[test]
    fn test_tree_connect() {
        let mut tree = SubagentTree::new();
        let task_id = TaskId::new();

        let root = Subagent::new_root(task_id, "Root".to_string(), "".to_string());
        let child = Subagent::first_level(
            task_id,
            Uuid::nil(), // placeholder, will be updated
            "Child".to_string(),
            "".to_string(),
            SpawnReason::TaskDecomposition,
        );

        let child_id = child.id;
        tree.insert(root.clone());
        tree.insert(child);
        tree.connect(child_id, root.id).unwrap();

        let child = tree.get(&child_id).unwrap();
        assert_eq!(child.parent_id, Some(root.id));
        assert_eq!(child.depth, 1);

        let children = tree.get_children(&root.id);
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].id, child_id);
    }

    #[test]
    fn test_tree_get_all_for_task() {
        let mut tree = SubagentTree::new();
        let task_id = TaskId::new();
        let other_task_id = TaskId::new();

        tree.insert(Subagent::new_root(
            task_id,
            "Root".to_string(),
            "".to_string(),
        ));
        tree.insert(Subagent::new_root(
            other_task_id,
            "Other Root".to_string(),
            "".to_string(),
        ));

        let all = tree.get_all_for_task(&task_id);
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].name, "Root");
    }

    #[test]
    fn test_tree_mark_completed() {
        let mut tree = SubagentTree::new();
        let task_id = TaskId::new();

        let subagent = Subagent::new_root(task_id, "Root".to_string(), "".to_string());
        let id = subagent.id;
        tree.insert(subagent);

        assert!(!tree.get(&id).unwrap().completed);
        tree.mark_completed(&id).unwrap();
        assert!(tree.get(&id).unwrap().completed);

        let completed = tree.get_completed(&task_id);
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].id, id);
    }

    #[test]
    fn test_tree_validate_depth() {
        let mut tree = SubagentTree::new();
        let task_id = TaskId::new();

        // Insert subagents with various depths
        let root = Subagent::new_root(task_id, "Root".to_string(), "".to_string());
        tree.insert(root.clone());

        let l1 = Subagent::first_level(
            task_id,
            root.id,
            "L1".to_string(),
            "".to_string(),
            SpawnReason::TaskDecomposition,
        );
        let l1_id = l1.id;
        tree.insert(l1);

        let l2 = Subagent::second_level(
            task_id,
            l1_id,
            "L2".to_string(),
            "".to_string(),
            SpawnReason::TaskDecomposition,
        );
        tree.insert(l2);

        // All should be valid (depths 0, 1, 2)
        assert!(tree.validate_depth_constraint());

        // If we tried to add a depth 3, it would be invalid but we prevent that at spawn time
    }

    // --- SubagentSpawner Tests ---

    #[test]
    fn test_spawner_spawn_root() {
        let mut spawner = SubagentSpawner::new();
        let task_id = TaskId::new();

        let root = spawner
            .spawn_root(
                task_id,
                "Root Agent".to_string(),
                "Main coordinator".to_string(),
            )
            .unwrap();

        assert_eq!(root.depth, 0);
        assert!(root.can_spawn_subagent());
        assert_eq!(spawner.stats.total_spawned, 1);
    }

    #[test]
    fn test_spawner_spawn_child() {
        let mut spawner = SubagentSpawner::new();
        let task_id = TaskId::new();

        // Spawn root
        let root = spawner
            .spawn_root(task_id, "Root".to_string(), "".to_string())
            .unwrap();

        // Spawn child (level 1)
        let child = spawner
            .spawn_child(
                task_id,
                root.id,
                "Worker".to_string(),
                "Does work".to_string(),
                SpawnReason::TaskDecomposition,
            )
            .unwrap();

        assert_eq!(child.depth, 1);
        assert!(child.can_spawn_subagent()); // Level 1 can still spawn
        assert_eq!(spawner.stats.total_spawned, 2);
    }

    #[test]
    fn test_spawner_spawn_grandchild() {
        let mut spawner = SubagentSpawner::new();
        let task_id = TaskId::new();

        // Spawn root
        let root = spawner
            .spawn_root(task_id, "Root".to_string(), "".to_string())
            .unwrap();

        // Spawn child (level 1)
        let child = spawner
            .spawn_child(
                task_id,
                root.id,
                "Worker".to_string(),
                "".to_string(),
                SpawnReason::TaskDecomposition,
            )
            .unwrap();

        // Spawn grandchild (level 2 - max depth)
        let grandchild = spawner
            .spawn_child(
                task_id,
                child.id,
                "Detail Worker".to_string(),
                "".to_string(),
                SpawnReason::ParallelExecution,
            )
            .unwrap();

        assert_eq!(grandchild.depth, 2);
        assert!(!grandchild.can_spawn_subagent()); // Cannot spawn at max depth
        assert_eq!(spawner.stats.total_spawned, 3);
    }

    #[test]
    fn test_spawner_rejects_depth_3() {
        let mut spawner = SubagentSpawner::new();
        let task_id = TaskId::new();

        // Spawn root
        let root = spawner
            .spawn_root(task_id, "Root".to_string(), "".to_string())
            .unwrap();

        // Spawn child level 1
        let l1 = spawner
            .spawn_child(
                task_id,
                root.id,
                "L1".to_string(),
                "".to_string(),
                SpawnReason::TaskDecomposition,
            )
            .unwrap();

        // Spawn child level 2
        let l2 = spawner
            .spawn_child(
                task_id,
                l1.id,
                "L2".to_string(),
                "".to_string(),
                SpawnReason::TaskDecomposition,
            )
            .unwrap();

        // Try to spawn level 3 - should fail
        let result = spawner.spawn_child(
            task_id,
            l2.id,
            "L3".to_string(),
            "".to_string(),
            SpawnReason::TaskDecomposition,
        );

        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            SubagentError::MaxDepthExceeded { .. }
        ));
        assert_eq!(spawner.stats.depth_rejections, 1);
    }

    #[test]
    fn test_spawner_get_children() {
        let mut spawner = SubagentSpawner::new();
        let task_id = TaskId::new();

        let root = spawner
            .spawn_root(task_id, "Root".to_string(), "".to_string())
            .unwrap();

        let child1 = spawner
            .spawn_child(
                task_id,
                root.id,
                "Child1".to_string(),
                "".to_string(),
                SpawnReason::TaskDecomposition,
            )
            .unwrap();

        let child2 = spawner
            .spawn_child(
                task_id,
                root.id,
                "Child2".to_string(),
                "".to_string(),
                SpawnReason::TaskDecomposition,
            )
            .unwrap();

        let children = spawner.get_children(&root.id);
        assert_eq!(children.len(), 2);
        let child_ids: Vec<_> = children.iter().map(|c| c.id).collect();
        assert!(child_ids.contains(&child1.id));
        assert!(child_ids.contains(&child2.id));
    }

    #[test]
    fn test_spawner_mark_completed() {
        let mut spawner = SubagentSpawner::new();
        let task_id = TaskId::new();

        let root = spawner
            .spawn_root(task_id, "Root".to_string(), "".to_string())
            .unwrap();

        assert!(!spawner.is_task_completed(&task_id));

        spawner.mark_completed(&root.id).unwrap();
        assert_eq!(spawner.stats.total_completed, 1);
        assert!(spawner.is_task_completed(&task_id));
    }

    #[test]
    fn test_spawner_depth_validation() {
        let mut spawner = SubagentSpawner::new();
        let task_id = TaskId::new();

        let root = spawner
            .spawn_root(task_id, "Root".to_string(), "".to_string())
            .unwrap();

        let l1 = spawner
            .spawn_child(
                task_id,
                root.id,
                "L1".to_string(),
                "".to_string(),
                SpawnReason::TaskDecomposition,
            )
            .unwrap();

        let l2 = spawner
            .spawn_child(
                task_id,
                l1.id,
                "L2".to_string(),
                "".to_string(),
                SpawnReason::TaskDecomposition,
            )
            .unwrap();

        // All spawned subagents are at valid depths
        assert!(spawner.validate_depth());
        assert_eq!(spawner.get_depth(&root.id), Some(0));
        assert_eq!(spawner.get_depth(&l1.id), Some(1));
        assert_eq!(spawner.get_depth(&l2.id), Some(2));
    }

    #[test]
    fn test_spawner_can_spawn_at_depth() {
        let mut spawner = SubagentSpawner::new();
        let task_id = TaskId::new();

        let root = spawner
            .spawn_root(task_id, "Root".to_string(), "".to_string())
            .unwrap();

        assert!(spawner.can_spawn_at_depth(root.id, task_id));

        let l1 = spawner
            .spawn_child(
                task_id,
                root.id,
                "L1".to_string(),
                "".to_string(),
                SpawnReason::TaskDecomposition,
            )
            .unwrap();

        assert!(spawner.can_spawn_at_depth(l1.id, task_id));

        let l2 = spawner
            .spawn_child(
                task_id,
                l1.id,
                "L2".to_string(),
                "".to_string(),
                SpawnReason::TaskDecomposition,
            )
            .unwrap();

        assert!(!spawner.can_spawn_at_depth(l2.id, task_id));
    }

    #[test]
    fn test_spawner_multiple_tasks() {
        let mut spawner = SubagentSpawner::new();
        let task1 = TaskId::new();
        let task2 = TaskId::new();

        let root1 = spawner
            .spawn_root(task1, "Task1 Root".to_string(), "".to_string())
            .unwrap();
        let _root2 = spawner
            .spawn_root(task2, "Task2 Root".to_string(), "".to_string())
            .unwrap();

        // Spawn children for task1
        spawner
            .spawn_child(
                task1,
                root1.id,
                "Task1 Worker".to_string(),
                "".to_string(),
                SpawnReason::TaskDecomposition,
            )
            .unwrap();

        assert_eq!(spawner.count(&task1), 2);
        assert_eq!(spawner.count(&task2), 1);

        let task1_active = spawner.get_active(&task1);
        let task2_active = spawner.get_active(&task2);
        assert_eq!(task1_active.len(), 2);
        assert_eq!(task2_active.len(), 1);
    }

    #[test]
    fn test_spawner_get_parent() {
        let mut spawner = SubagentSpawner::new();
        let task_id = TaskId::new();

        let root = spawner
            .spawn_root(task_id, "Root".to_string(), "".to_string())
            .unwrap();

        let child = spawner
            .spawn_child(
                task_id,
                root.id,
                "Child".to_string(),
                "".to_string(),
                SpawnReason::TaskDecomposition,
            )
            .unwrap();

        let parent = spawner.get_parent(&child.id);
        assert!(parent.is_some());
        assert_eq!(parent.unwrap().id, root.id);
    }
}
