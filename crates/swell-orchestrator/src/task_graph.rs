//! Task dependency graph for tracking and ordering task execution.
//!
//! This module provides a DAG (Directed Acyclic Graph) implementation for managing
//! task dependencies, ensuring:
//! - Circular dependency detection
//! - Topological sorting for execution ordering
//! - Ready task identification (all dependencies satisfied)

use std::collections::{HashMap, HashSet};
use swell_core::ids::TaskId;
use swell_core::{SwellError, TaskState};
use tracing::{info, warn};

/// A directed acyclic graph (DAG) for managing task dependencies.
#[derive(Debug, Clone)]
pub struct TaskGraph {
    /// Map from task ID to its dependency IDs
    dependencies: HashMap<TaskId, HashSet<TaskId>>,
    /// Map from task ID to tasks that depend on it (reverse lookup)
    dependents: HashMap<TaskId, HashSet<TaskId>>,
}

impl TaskGraph {
    /// Create a new empty task graph
    pub fn new() -> Self {
        Self {
            dependencies: HashMap::new(),
            dependents: HashMap::new(),
        }
    }

    /// Add a task to the graph with its dependencies
    ///
    /// Returns an error if adding the task would create a circular dependency.
    pub fn add_task(&mut self, task_id: TaskId, dependencies: Vec<TaskId>) -> Result<(), SwellError> {
        // Check for circular dependency before adding
        if self.would_create_cycle(&task_id, &dependencies) {
            warn!(task_id = %task_id, ?dependencies, "Circular dependency detected");
            return Err(SwellError::InvalidStateTransition(format!(
                "Adding task {} would create circular dependency",
                task_id
            )));
        }

        // Add the task's dependencies
        let deps_set: HashSet<TaskId> = dependencies.into_iter().collect();
        self.dependencies.insert(task_id, deps_set);

        // Ensure task exists in dependents map (with empty set if no dependents)
        self.dependents.entry(task_id).or_default();

        // Update reverse lookup (dependents)
        if let Some(deps) = self.dependencies.get(&task_id) {
            for dep_id in deps {
                self.dependents.entry(*dep_id).or_default().insert(task_id);
            }
        }

        info!(task_id = %task_id, "Task added to graph");
        Ok(())
    }

    /// Remove a task from the graph
    ///
    /// Also cleans up any references to this task from other tasks' dependencies.
    pub fn remove_task(&mut self, task_id: TaskId) -> Result<(), SwellError> {
        if !self.dependencies.contains_key(&task_id) {
            return Err(SwellError::TaskNotFound(task_id.as_uuid()));
        }

        // Find all tasks that depend on this task and remove this task from their dependency lists
        if let Some(dependents_set) = self.dependents.get(&task_id) {
            // Collect a copy since we'll be modifying
            let dependent_tasks: Vec<TaskId> = dependents_set.iter().cloned().collect();
            for dependent_id in dependent_tasks {
                if let Some(deps) = self.dependencies.get_mut(&dependent_id) {
                    deps.remove(&task_id);
                }
            }
        }

        // Remove from dependents map
        self.dependents.remove(&task_id);

        // Remove from dependencies map
        self.dependencies.remove(&task_id);

        info!(task_id = %task_id, "Task removed from graph");
        Ok(())
    }

    /// Check if adding a task with given dependencies would create a cycle
    fn would_create_cycle(&self, task_id: &TaskId, new_dependencies: &[TaskId]) -> bool {
        // If the new task depends on itself, that's a cycle
        if new_dependencies.contains(task_id) {
            return true;
        }

        // Check if any of the new dependencies transitively depend on this task
        // by following the dependency chain
        for dep_id in new_dependencies {
            if self.transitively_depends_on(dep_id, task_id) {
                return true;
            }
        }

        false
    }

    /// Check if task_a transitively depends on task_b
    /// (i.e., there exists a path from task_a to task_b through dependencies)
    fn transitively_depends_on(&self, task_id: &TaskId, target: &TaskId) -> bool {
        let mut visited = HashSet::new();
        self.has_path(task_id, target, &mut visited)
    }

    /// DFS to check if there's a path from source to target
    fn has_path(&self, source: &TaskId, target: &TaskId, visited: &mut HashSet<TaskId>) -> bool {
        if source == target {
            return true;
        }

        if visited.contains(source) {
            return false;
        }

        visited.insert(*source);

        if let Some(deps) = self.dependencies.get(source) {
            for dep_id in deps {
                if self.has_path(dep_id, target, visited) {
                    return true;
                }
            }
        }

        false
    }

    /// Get all tasks that are ready to execute (all dependencies satisfied)
    ///
    /// A task is ready if:
    /// - It exists in the graph
    /// - All its dependencies have been completed (are in Accepted state)
    pub fn get_ready_tasks(&self, task_states: &HashMap<TaskId, TaskState>) -> Vec<TaskId> {
        self.dependencies
            .keys()
            .filter(|task_id| {
                // Check if all dependencies are satisfied (Accepted state)
                if let Some(deps) = self.dependencies.get(task_id) {
                    deps.iter().all(|dep_id| {
                        task_states
                            .get(dep_id)
                            .map(|state| *state == TaskState::Accepted)
                            .unwrap_or(false)
                    })
                } else {
                    false
                }
            })
            .cloned()
            .collect()
    }

    /// Get topological ordering of all tasks in the graph
    ///
    /// Uses Kahn's algorithm for topological sorting.
    /// Returns tasks in order such that dependencies come before dependents.
    ///
    /// Returns an error if the graph contains a cycle (which shouldn't happen
    /// if add_task properly validated).
    pub fn topological_sort(&self) -> Result<Vec<TaskId>, SwellError> {
        // Kahn's algorithm
        let mut in_degree: HashMap<TaskId, usize> = HashMap::new();
        let mut result: Vec<TaskId> = Vec::new();

        // Initialize in-degrees
        for task_id in self.dependencies.keys() {
            *in_degree.entry(*task_id).or_insert(0) += 0;
        }

        // Count incoming edges for each node
        for deps in self.dependencies.values() {
            for dep_id in deps {
                *in_degree.entry(*dep_id).or_insert(0) += 1;
            }
        }

        // Start with tasks that have no dependencies (in_degree = 0)
        // But first, we need to identify root tasks (tasks no one depends on)
        // Actually, let's use a different approach: tasks with no incoming edges

        // A task with no incoming edges means no one depends on it being done first
        // But that's the wrong way - we want tasks whose dependencies are done

        // Let's use a simpler approach: iterative removal of "ready" tasks
        let mut ready: Vec<TaskId> = Vec::new();
        let mut completed: HashSet<TaskId> = HashSet::new();

        // Find tasks that have no dependencies
        for (task_id, deps) in &self.dependencies {
            if deps.is_empty() {
                ready.push(*task_id);
            }
        }

        // If no tasks have no dependencies, we have a cycle
        if ready.is_empty() && !self.dependencies.is_empty() {
            return Err(SwellError::InvalidStateTransition(
                "Cycle detected in task graph".to_string(),
            ));
        }

        while !ready.is_empty() {
            // Take a ready task
            let task_id = ready.remove(0);

            if completed.contains(&task_id) {
                continue;
            }

            completed.insert(task_id);
            result.push(task_id);

            // Find all tasks that depend on the completed task and check if they're now ready
            if let Some(dependents) = self.dependents.get(&task_id) {
                for dependent_id in dependents {
                    if completed.contains(dependent_id) {
                        continue;
                    }

                    // Check if all dependencies of this dependent are completed
                    if let Some(deps) = self.dependencies.get(dependent_id) {
                        if deps.iter().all(|d| completed.contains(d)) {
                            ready.push(*dependent_id);
                        }
                    }
                }
            }
        }

        // If we haven't processed all tasks, there's a cycle
        if result.len() != self.dependencies.len() {
            return Err(SwellError::InvalidStateTransition(
                "Cycle detected in task graph".to_string(),
            ));
        }

        Ok(result)
    }

    /// Alternative topological sort using Kahn's algorithm directly
    ///
    /// This returns tasks in execution order (dependencies first).
    pub fn topological_sort_kahn(&self) -> Result<Vec<TaskId>, SwellError> {
        // Clone the in-degrees
        let mut in_degree: HashMap<TaskId, usize> = HashMap::new();
        let mut result: Vec<TaskId> = Vec::new();

        // Initialize: every task has in-degree 0
        for task_id in self.dependencies.keys() {
            in_degree.insert(*task_id, 0);
        }

        // Calculate in-degrees: for each dependency edge A -> B,
        // B's in-degree increases
        for (task_id, deps) in &self.dependencies {
            for dep_id in deps {
                // dep_id is a dependency of task_id
                // So task_id depends on dep_id
                // In terms of DAG: dep_id -> task_id (dep_id must come first)
                // So task_id's in-degree should include dep_id's contribution
                let _ = task_id; // suppress unused warning
                *in_degree.entry(*dep_id).or_insert(0) += 0;
            }
        }

        // Actually, let's recalculate properly
        // For edge A -> B (A depends on B, so B must execute before A)
        // B is a dependency of A
        // A's in-degree is the count of dependencies it has

        let mut in_degree: HashMap<TaskId, usize> = HashMap::new();
        for task_id in self.dependencies.keys() {
            let count = self.dependencies.get(task_id).map(|d| d.len()).unwrap_or(0);
            in_degree.insert(*task_id, count);
        }

        // Queue of tasks with in-degree 0
        let mut queue: Vec<TaskId> = in_degree
            .iter()
            .filter(|(_, &count)| count == 0)
            .map(|(&id, _)| id)
            .collect();

        while !queue.is_empty() {
            let task_id = queue.remove(0);
            result.push(task_id);

            // For each task that depends on this one
            if let Some(deps_on_this) = self.dependents.get(&task_id) {
                for dependent_id in deps_on_this {
                    if let Some(degree) = in_degree.get_mut(dependent_id) {
                        *degree -= 1;
                        if *degree == 0 {
                            queue.push(*dependent_id);
                        }
                    }
                }
            }
        }

        // If we processed all tasks, return the result
        // The result is in execution order (tasks with no deps first)
        if result.len() == self.dependencies.len() {
            Ok(result)
        } else {
            // There's a cycle
            Err(SwellError::InvalidStateTransition(
                "Cycle detected in task graph".to_string(),
            ))
        }
    }

    /// Get dependencies for a specific task
    pub fn get_dependencies(&self, task_id: &TaskId) -> Option<&HashSet<TaskId>> {
        self.dependencies.get(task_id)
    }

    /// Get dependents (tasks that depend on this one) for a specific task
    pub fn get_dependents(&self, task_id: &TaskId) -> Option<&HashSet<TaskId>> {
        self.dependents.get(task_id)
    }

    /// Check if the graph has a specific task
    pub fn has_task(&self, task_id: &TaskId) -> bool {
        self.dependencies.contains_key(task_id)
    }

    /// Get all task IDs in the graph
    pub fn all_tasks(&self) -> Vec<TaskId> {
        self.dependencies.keys().cloned().collect()
    }

    /// Get the number of tasks in the graph
    pub fn len(&self) -> usize {
        self.dependencies.len()
    }

    /// Check if the graph is empty
    pub fn is_empty(&self) -> bool {
        self.dependencies.is_empty()
    }

    /// Validate the entire graph for cycles
    ///
    /// Returns Ok(()) if no cycles, Err containing the task ID that is part of a cycle.
    pub fn validate_no_cycles(&self) -> Result<(), SwellError> {
        let mut visited: HashSet<TaskId> = HashSet::new();
        let mut recursion_stack: HashSet<TaskId> = HashSet::new();

        for task_id in self.dependencies.keys() {
            if self.has_cycle_from(task_id, &mut visited, &mut recursion_stack) {
                return Err(SwellError::InvalidStateTransition(format!(
                    "Cycle detected involving task {}",
                    task_id
                )));
            }
        }

        Ok(())
    }

    /// DFS helper to detect cycles
    fn has_cycle_from(
        &self,
        task_id: &TaskId,
        visited: &mut HashSet<TaskId>,
        recursion_stack: &mut HashSet<TaskId>,
    ) -> bool {
        if recursion_stack.contains(task_id) {
            return true;
        }

        if visited.contains(task_id) {
            return false;
        }

        visited.insert(*task_id);
        recursion_stack.insert(*task_id);

        if let Some(deps) = self.dependencies.get(task_id) {
            for dep_id in deps {
                if self.has_cycle_from(dep_id, visited, recursion_stack) {
                    return true;
                }
            }
        }

        recursion_stack.remove(task_id);
        false
    }

    /// Update dependencies for an existing task
    ///
    /// Returns an error if the new dependencies would create a cycle.
    pub fn update_dependencies(
        &mut self,
        task_id: TaskId,
        new_dependencies: Vec<TaskId>,
    ) -> Result<(), SwellError> {
        if !self.dependencies.contains_key(&task_id) {
            return Err(SwellError::TaskNotFound(task_id.as_uuid()));
        }

        // Check for cycles with new dependencies
        if self.would_create_cycle_with_removal(&task_id, &new_dependencies) {
            warn!(task_id = %task_id, ?new_dependencies, "Update would create circular dependency");
            return Err(SwellError::InvalidStateTransition(format!(
                "Updating dependencies for {} would create circular dependency",
                task_id
            )));
        }

        // Remove old dependency references
        if let Some(old_deps) = self.dependencies.get(&task_id) {
            for old_dep in old_deps {
                if let Some(deps_dependents) = self.dependents.get_mut(old_dep) {
                    deps_dependents.remove(&task_id);
                }
            }
        }

        // Add new dependencies
        let new_deps_set: HashSet<TaskId> = new_dependencies.into_iter().collect();
        for new_dep in &new_deps_set {
            self.dependents.entry(*new_dep).or_default().insert(task_id);
        }

        self.dependencies.insert(task_id, new_deps_set);

        info!(task_id = %task_id, "Task dependencies updated");
        Ok(())
    }

    /// Check if updating a task's dependencies would create a cycle
    fn would_create_cycle_with_removal(&self, task_id: &TaskId, new_dependencies: &[TaskId]) -> bool {
        // Temporarily remove the task from the graph
        // and check if the new dependencies would create a cycle

        // First check direct self-reference
        if new_dependencies.contains(task_id) {
            return true;
        }

        // Check transitive dependencies
        for dep_id in new_dependencies {
            // If any of the new dependencies transitively depends on this task,
            // it would be a cycle
            if self.transitively_depends_on(dep_id, task_id) {
                return true;
            }
        }

        false
    }

    /// Extract connected subgraphs from the task graph.
    ///
    /// A connected subgraph is a set of tasks where each task is reachable
    /// from every other task in the set via dependency edges.
    ///
    /// This is useful for determining if a task cluster should be managed
    /// by a FeatureLead sub-orchestrator.
    ///
    /// # Returns
    /// A vector of subgraphs, where each subgraph is a vector of task IDs.
    /// Each subgraph is a maximal set of mutually connected tasks.
    pub fn get_connected_subgraphs(&self) -> Vec<Vec<TaskId>> {
        if self.dependencies.is_empty() {
            return vec![];
        }

        let all_tasks: Vec<TaskId> = self.dependencies.keys().cloned().collect();
        let mut visited: HashSet<TaskId> = HashSet::new();
        let mut subgraphs: Vec<Vec<TaskId>> = Vec::new();

        for task_id in all_tasks {
            if visited.contains(&task_id) {
                continue;
            }

            // BFS to find all tasks connected to this one
            let mut subgraph: Vec<TaskId> = Vec::new();
            let mut queue: Vec<TaskId> = vec![task_id];

            while let Some(current) = queue.pop() {
                if visited.contains(&current) {
                    continue;
                }
                visited.insert(current);
                subgraph.push(current);

                // Get all tasks that depend on current (current is a dependency of these)
                if let Some(dependents) = self.dependents.get(&current) {
                    for dep in dependents {
                        if !visited.contains(dep) {
                            queue.push(*dep);
                        }
                    }
                }

                // Get all tasks that current depends on
                if let Some(deps) = self.dependencies.get(&current) {
                    for dep in deps {
                        if !visited.contains(dep) {
                            queue.push(*dep);
                        }
                    }
                }
            }

            if !subgraph.is_empty() {
                subgraphs.push(subgraph);
            }
        }

        subgraphs
    }

    /// Get the size of the largest connected subgraph.
    ///
    /// This is useful for determining if a FeatureLead should be spawned.
    pub fn largest_subgraph_size(&self) -> usize {
        self.get_connected_subgraphs()
            .iter()
            .map(|g| g.len())
            .max()
            .unwrap_or(0)
    }
}

impl Default for TaskGraph {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // Helper to create a simple task state map
    fn task_state_map(tasks: &[(TaskId, TaskState)]) -> HashMap<TaskId, TaskState> {
        tasks.iter().cloned().collect()
    }

    // --- Basic Graph Operations Tests ---

    #[test]
    fn test_new_graph_is_empty() {
        let graph = TaskGraph::new();
        assert!(graph.is_empty());
        assert_eq!(graph.len(), 0);
    }

    #[test]
    fn test_add_single_task() {
        let mut graph = TaskGraph::new();
        let task_id = TaskId::new();

        graph.add_task(task_id, vec![]).unwrap();

        assert!(!graph.is_empty());
        assert_eq!(graph.len(), 1);
        assert!(graph.has_task(&task_id));
    }

    #[test]
    fn test_add_task_with_dependencies() {
        let mut graph = TaskGraph::new();
        let dep_id = TaskId::new();
        let task_id = TaskId::new();

        // Add dependency first
        graph.add_task(dep_id, vec![]).unwrap();
        graph.add_task(task_id, vec![dep_id]).unwrap();

        assert_eq!(graph.len(), 2);

        let deps = graph.get_dependencies(&task_id);
        assert!(deps.is_some());
        assert!(deps.unwrap().contains(&dep_id));
    }

    #[test]
    fn test_remove_task() {
        let mut graph = TaskGraph::new();
        let task_id = TaskId::new();

        graph.add_task(task_id, vec![]).unwrap();
        assert!(graph.has_task(&task_id));

        graph.remove_task(task_id).unwrap();
        assert!(!graph.has_task(&task_id));
        assert!(graph.is_empty());
    }

    #[test]
    fn test_remove_task_with_dependent() {
        let mut graph = TaskGraph::new();
        let dep_id = TaskId::new();
        let task_id = TaskId::new();

        graph.add_task(dep_id, vec![]).unwrap();
        graph.add_task(task_id, vec![dep_id]).unwrap();

        // Remove the dependency
        graph.remove_task(dep_id).unwrap();

        // Task should still exist but have no dependencies
        assert!(graph.has_task(&task_id));
        assert!(graph.get_dependencies(&task_id).unwrap().is_empty());
    }

    // --- Circular Dependency Detection Tests ---

    #[test]
    fn test_self_dependency_rejected() {
        let mut graph = TaskGraph::new();
        let task_id = TaskId::new();

        let result = graph.add_task(task_id, vec![task_id]);

        assert!(result.is_err());
        match result.unwrap_err() {
            SwellError::InvalidStateTransition(msg) => {
                assert!(msg.contains("circular dependency"));
            }
            _ => panic!("Expected InvalidStateTransition"),
        }
    }

    #[test]
    fn test_direct_cycle_rejected() {
        let mut graph = TaskGraph::new();
        let task_a = TaskId::new();
        let task_b = TaskId::new();

        // A depends on B
        graph.add_task(task_a, vec![task_b]).unwrap();

        // Now try to make B depend on A - should fail
        let result = graph.add_task(task_b, vec![task_a]);

        assert!(result.is_err());
        match result.unwrap_err() {
            SwellError::InvalidStateTransition(msg) => {
                assert!(msg.contains("circular dependency"));
            }
            _ => panic!("Expected InvalidStateTransition"),
        }
    }

    #[test]
    fn test_transitive_cycle_rejected() {
        let mut graph = TaskGraph::new();
        let task_a = TaskId::new();
        let task_b = TaskId::new();
        let task_c = TaskId::new();

        // A depends on B
        graph.add_task(task_a, vec![task_b]).unwrap();

        // B depends on C
        graph.add_task(task_b, vec![task_c]).unwrap();

        // C depends on A - should fail (cycle: A -> B -> C -> A)
        let result = graph.add_task(task_c, vec![task_a]);

        assert!(result.is_err());
        match result.unwrap_err() {
            SwellError::InvalidStateTransition(msg) => {
                assert!(msg.contains("circular dependency"));
            }
            _ => panic!("Expected InvalidStateTransition"),
        }
    }

    #[test]
    fn test_complex_cycle_detection() {
        let mut graph = TaskGraph::new();
        let a = TaskId::new();
        let b = TaskId::new();
        let c = TaskId::new();
        let d = TaskId::new();

        // Diamond dependency: A -> B, A -> C, B -> D, C -> D
        // This should be allowed (it's a diamond, not a cycle)
        graph.add_task(a, vec![]).unwrap();
        graph.add_task(b, vec![a]).unwrap();
        graph.add_task(c, vec![a]).unwrap();
        graph.add_task(d, vec![b, c]).unwrap();

        assert_eq!(graph.len(), 4);
        assert!(graph.validate_no_cycles().is_ok());
    }

    // --- Topological Sort Tests ---

    #[test]
    fn test_topological_sort_single_task() {
        let mut graph = TaskGraph::new();
        let task_id = TaskId::new();

        graph.add_task(task_id, vec![]).unwrap();

        let sorted = graph.topological_sort().unwrap();
        assert_eq!(sorted.len(), 1);
        assert_eq!(sorted[0], task_id);
    }

    #[test]
    fn test_topological_sort_linear_dependencies() {
        let mut graph = TaskGraph::new();
        let a = TaskId::new();
        let b = TaskId::new();
        let c = TaskId::new();

        // A -> B -> C (B depends on A, C depends on B)
        graph.add_task(a, vec![]).unwrap();
        graph.add_task(b, vec![a]).unwrap();
        graph.add_task(c, vec![b]).unwrap();

        let sorted = graph.topological_sort().unwrap();

        // A should come before B, B should come before C
        let a_idx = sorted.iter().position(|&id| id == a).unwrap();
        let b_idx = sorted.iter().position(|&id| id == b).unwrap();
        let c_idx = sorted.iter().position(|&id| id == c).unwrap();

        assert!(a_idx < b_idx);
        assert!(b_idx < c_idx);
    }

    #[test]
    fn test_topological_sort_parallel_branches() {
        let mut graph = TaskGraph::new();
        let a = TaskId::new();
        let b = TaskId::new();
        let c = TaskId::new();

        // A is root, B and C both depend on A
        graph.add_task(a, vec![]).unwrap();
        graph.add_task(b, vec![a]).unwrap();
        graph.add_task(c, vec![a]).unwrap();

        let sorted = graph.topological_sort().unwrap();

        // A should come before both B and C
        let a_idx = sorted.iter().position(|&id| id == a).unwrap();
        let b_idx = sorted.iter().position(|&id| id == b).unwrap();
        let c_idx = sorted.iter().position(|&id| id == c).unwrap();

        assert!(a_idx < b_idx);
        assert!(a_idx < c_idx);
    }

    #[test]
    fn test_topological_sort_diamond() {
        let mut graph = TaskGraph::new();
        let a = TaskId::new();
        let b = TaskId::new();
        let c = TaskId::new();
        let d = TaskId::new();

        // Diamond: A -> B, A -> C, B -> D, C -> D
        graph.add_task(a, vec![]).unwrap();
        graph.add_task(b, vec![a]).unwrap();
        graph.add_task(c, vec![a]).unwrap();
        graph.add_task(d, vec![b, c]).unwrap();

        let sorted = graph.topological_sort().unwrap();

        // A should come first, D should come last
        let a_idx = sorted.iter().position(|&id| id == a).unwrap();
        let d_idx = sorted.iter().position(|&id| id == d).unwrap();

        assert!(a_idx < d_idx);

        // B and C should both be after A and before D
        let b_idx = sorted.iter().position(|&id| id == b).unwrap();
        let c_idx = sorted.iter().position(|&id| id == c).unwrap();

        assert!(a_idx < b_idx);
        assert!(a_idx < c_idx);
        assert!(b_idx < d_idx);
        assert!(c_idx < d_idx);
    }

    // --- Get Ready Tasks Tests ---

    #[test]
    fn test_get_ready_tasks_no_dependencies() {
        let mut graph = TaskGraph::new();
        let task_id = TaskId::new();

        graph.add_task(task_id, vec![]).unwrap();

        // All dependencies are "satisfied" (vacuous - there are none)
        let states = task_state_map(&[(task_id, TaskState::Created)]);
        let ready = graph.get_ready_tasks(&states);

        assert!(ready.contains(&task_id));
    }

    #[test]
    fn test_get_ready_tasks_dependency_not_done() {
        let mut graph = TaskGraph::new();
        let dep_id = TaskId::new();
        let task_id = TaskId::new();

        graph.add_task(dep_id, vec![]).unwrap();
        graph.add_task(task_id, vec![dep_id]).unwrap();

        // Dependency is not completed
        let states = task_state_map(&[(dep_id, TaskState::Executing), (task_id, TaskState::Ready)]);
        let ready = graph.get_ready_tasks(&states);

        // Task should NOT be ready because dependency is not Accepted
        assert!(!ready.contains(&task_id));
    }

    #[test]
    fn test_get_ready_tasks_dependency_done() {
        let mut graph = TaskGraph::new();
        let dep_id = TaskId::new();
        let task_id = TaskId::new();

        graph.add_task(dep_id, vec![]).unwrap();
        graph.add_task(task_id, vec![dep_id]).unwrap();

        // Dependency is completed (Accepted state)
        let states = task_state_map(&[(dep_id, TaskState::Accepted), (task_id, TaskState::Ready)]);
        let ready = graph.get_ready_tasks(&states);

        // Task should be ready
        assert!(ready.contains(&task_id));
    }

    #[test]
    fn test_get_ready_tasks_multiple_dependencies() {
        let mut graph = TaskGraph::new();
        let a = TaskId::new();
        let b = TaskId::new();
        let c = TaskId::new();

        graph.add_task(a, vec![]).unwrap();
        graph.add_task(b, vec![]).unwrap();
        graph.add_task(c, vec![a, b]).unwrap();

        // Only A is done
        let states = task_state_map(&[
            (a, TaskState::Accepted),
            (b, TaskState::Executing),
            (c, TaskState::Ready),
        ]);
        let ready = graph.get_ready_tasks(&states);

        // C should NOT be ready because B is not done
        assert!(!ready.contains(&c));

        // Now both are done
        let states = task_state_map(&[
            (a, TaskState::Accepted),
            (b, TaskState::Accepted),
            (c, TaskState::Ready),
        ]);
        let ready = graph.get_ready_tasks(&states);

        // C should now be ready
        assert!(ready.contains(&c));
    }

    // --- Validate No Cycles Tests ---

    #[test]
    fn test_validate_no_cycles_valid_graph() {
        let mut graph = TaskGraph::new();
        let a = TaskId::new();
        let b = TaskId::new();

        graph.add_task(a, vec![]).unwrap();
        graph.add_task(b, vec![a]).unwrap();

        assert!(graph.validate_no_cycles().is_ok());
    }

    #[test]
    fn test_validate_no_cycles_empty_graph() {
        let graph = TaskGraph::new();
        assert!(graph.validate_no_cycles().is_ok());
    }

    // --- Update Dependencies Tests ---

    #[test]
    fn test_update_dependencies() {
        let mut graph = TaskGraph::new();
        let a = TaskId::new();
        let b = TaskId::new();
        let c = TaskId::new();

        graph.add_task(a, vec![]).unwrap();
        graph.add_task(b, vec![]).unwrap();
        graph.add_task(c, vec![a]).unwrap();

        // Update C to depend on B instead of A
        graph.update_dependencies(c, vec![b]).unwrap();

        let deps = graph.get_dependencies(&c).unwrap();
        assert!(deps.contains(&b));
        assert!(!deps.contains(&a));
    }

    #[test]
    fn test_update_dependencies_to_create_cycle_rejected() {
        let mut graph = TaskGraph::new();
        let a = TaskId::new();
        let b = TaskId::new();

        graph.add_task(a, vec![]).unwrap();
        graph.add_task(b, vec![a]).unwrap();

        // Try to make A depend on B - should fail
        let result = graph.update_dependencies(a, vec![b]);

        assert!(result.is_err());
    }

    // --- Get Dependents Tests ---

    #[test]
    fn test_get_dependents() {
        let mut graph = TaskGraph::new();
        let a = TaskId::new();
        let b = TaskId::new();
        let c = TaskId::new();

        graph.add_task(a, vec![]).unwrap();
        graph.add_task(b, vec![a]).unwrap();
        graph.add_task(c, vec![a]).unwrap();

        let dependents = graph.get_dependents(&a);
        assert!(dependents.is_some());
        let deps = dependents.unwrap();
        assert!(deps.contains(&b));
        assert!(deps.contains(&c));
    }

    #[test]
    fn test_get_dependents_no_dependents() {
        let mut graph = TaskGraph::new();
        let a = TaskId::new();

        graph.add_task(a, vec![]).unwrap();

        let dependents = graph.get_dependents(&a);
        assert!(dependents.is_some());
        assert!(dependents.unwrap().is_empty());
    }

    // --- Kahn's Algorithm Tests ---

    #[test]
    fn test_topological_sort_kahn_simple() {
        let mut graph = TaskGraph::new();
        let a = TaskId::new();
        let b = TaskId::new();

        graph.add_task(a, vec![]).unwrap();
        graph.add_task(b, vec![a]).unwrap();

        let sorted = graph.topological_sort_kahn().unwrap();

        // A should come before B
        let a_idx = sorted.iter().position(|&id| id == a).unwrap();
        let b_idx = sorted.iter().position(|&id| id == b).unwrap();

        assert!(a_idx < b_idx);
    }

    #[test]
    fn test_topological_sort_kahn_complex() {
        let mut graph = TaskGraph::new();
        let a = TaskId::new();
        let b = TaskId::new();
        let c = TaskId::new();
        let d = TaskId::new();

        // Diamond: A -> B, A -> C, B -> D, C -> D
        graph.add_task(a, vec![]).unwrap();
        graph.add_task(b, vec![a]).unwrap();
        graph.add_task(c, vec![a]).unwrap();
        graph.add_task(d, vec![b, c]).unwrap();

        let sorted = graph.topological_sort_kahn().unwrap();

        // A should come first, D should come last
        let a_idx = sorted.iter().position(|&id| id == a).unwrap();
        let d_idx = sorted.iter().position(|&id| id == d).unwrap();

        assert!(a_idx < d_idx);
    }

    // --- Connected Subgraphs Tests ---

    #[test]
    fn test_get_connected_subgraphs_single_component() {
        let mut graph = TaskGraph::new();
        let a = TaskId::new();
        let b = TaskId::new();
        let c = TaskId::new();

        // All tasks in one connected component: A -> B -> C
        graph.add_task(a, vec![]).unwrap();
        graph.add_task(b, vec![a]).unwrap();
        graph.add_task(c, vec![b]).unwrap();

        let subgraphs = graph.get_connected_subgraphs();
        assert_eq!(subgraphs.len(), 1);
        assert_eq!(subgraphs[0].len(), 3);
    }

    #[test]
    fn test_get_connected_subgraphs_multiple_components() {
        let mut graph = TaskGraph::new();
        let a = TaskId::new();
        let b = TaskId::new();
        let c = TaskId::new();
        let d = TaskId::new();

        // Two disconnected components: A -> B and C -> D
        graph.add_task(a, vec![]).unwrap();
        graph.add_task(b, vec![a]).unwrap();
        graph.add_task(c, vec![]).unwrap();
        graph.add_task(d, vec![c]).unwrap();

        let subgraphs = graph.get_connected_subgraphs();
        assert_eq!(subgraphs.len(), 2);

        // Each subgraph should have 2 tasks
        for subgraph in &subgraphs {
            assert_eq!(subgraph.len(), 2);
        }
    }

    #[test]
    fn test_get_connected_subgraphs_empty_graph() {
        let graph = TaskGraph::new();
        let subgraphs = graph.get_connected_subgraphs();
        assert!(subgraphs.is_empty());
    }

    #[test]
    fn test_largest_subgraph_size() {
        let mut graph = TaskGraph::new();
        let a = TaskId::new();
        let b = TaskId::new();
        let c = TaskId::new();
        let d = TaskId::new();
        let e = TaskId::new();

        // Component 1: A -> B -> C (size 3)
        // Component 2: D -> E (size 2)
        graph.add_task(a, vec![]).unwrap();
        graph.add_task(b, vec![a]).unwrap();
        graph.add_task(c, vec![b]).unwrap();
        graph.add_task(d, vec![]).unwrap();
        graph.add_task(e, vec![d]).unwrap();

        assert_eq!(graph.largest_subgraph_size(), 3);
    }

    #[test]
    fn test_largest_subgraph_size_empty_graph() {
        let graph = TaskGraph::new();
        assert_eq!(graph.largest_subgraph_size(), 0);
    }

    #[test]
    fn test_get_connected_subgraphs_independent_tasks() {
        let mut graph = TaskGraph::new();
        // Three independent tasks with no dependencies
        let a = TaskId::new();
        let b = TaskId::new();
        let c = TaskId::new();

        graph.add_task(a, vec![]).unwrap();
        graph.add_task(b, vec![]).unwrap();
        graph.add_task(c, vec![]).unwrap();

        // Each should be its own subgraph
        let subgraphs = graph.get_connected_subgraphs();
        assert_eq!(subgraphs.len(), 3);
        for subgraph in &subgraphs {
            assert_eq!(subgraph.len(), 1);
        }
    }

    #[test]
    fn test_get_connected_subgraphs_complex_diamond() {
        let mut graph = TaskGraph::new();
        let a = TaskId::new();
        let b = TaskId::new();
        let c = TaskId::new();
        let d = TaskId::new();

        // Diamond: A -> B, A -> C, B -> D, C -> D
        // All connected in one component
        graph.add_task(a, vec![]).unwrap();
        graph.add_task(b, vec![a]).unwrap();
        graph.add_task(c, vec![a]).unwrap();
        graph.add_task(d, vec![b, c]).unwrap();

        let subgraphs = graph.get_connected_subgraphs();
        assert_eq!(subgraphs.len(), 1);
        assert_eq!(subgraphs[0].len(), 4);
    }
}
