//! Work Graph data model for task visualization and tracking.
//!
//! This module provides a DAG (Directed Acyclic Graph) implementation for the work graph,
//! which extends the basic TaskGraph with rich metadata for visualization and tracking.
//!
//! Key features:
//! - Nodes carry metadata: status color, complexity weight, spec links, agent session IDs,
//!   code change references, and test result references
//! - Fully serializable to JSON for external visualization tools
//! - Cycle detection prevents invalid DAGs
//!
//! # Example
//!
//! ```rust
//! use swell_orchestrator::work_graph::{WorkGraph, WorkGraphNode, NodeMetadata};
//! use uuid::Uuid;
//!
//! let mut graph = WorkGraph::new();
//! let node_id = Uuid::new_v4();
//!
//! // Add a node with metadata
//! let metadata = NodeMetadata::new("Implement feature X");
//! graph.add_node(node_id, metadata, vec![]).unwrap();
//!
//! // Serialize to JSON
//! let json = serde_json::to_string_pretty(&graph).unwrap();
//! println!("{}", json);
//!
//! // Deserialize back
//! let restored: WorkGraph = serde_json::from_str(&json).unwrap();
//! assert_eq!(graph, restored);
//! ```

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fmt;
use uuid::Uuid;

/// A directed acyclic graph (DAG) for the work graph with rich node metadata.
///
/// The work graph represents tasks as nodes with dependency edges (directed edges
/// from a task to its dependencies). Each node carries metadata for visualization
/// and tracking purposes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkGraph {
    /// Map from task ID to work graph node
    #[serde(serialize_with = "serialize_ordered_map")]
    #[serde(deserialize_with = "deserialize_ordered_map")]
    nodes: HashMap<Uuid, WorkGraphNode>,

    /// Directed edges representing dependencies.
    /// For edge (A, B), A depends on B (B must complete before A).
    #[serde(serialize_with = "serialize_ordered_edges")]
    #[serde(deserialize_with = "deserialize_ordered_edges")]
    edges: Vec<(Uuid, Uuid)>,

    /// Metadata about the graph itself
    #[serde(default)]
    metadata: GraphMetadata,
}

/// Serialization helper for deterministic map output
fn serialize_ordered_map<S>(
    value: &HashMap<Uuid, WorkGraphNode>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    let mut ordered: Vec<_> = value.iter().collect();
    ordered.sort_by(|a, b| a.0.cmp(b.0));
    serde::Serialize::serialize(&ordered, serializer)
}

/// Deserialization helper for map
fn deserialize_ordered_map<'de, D>(
    deserializer: D,
) -> Result<HashMap<Uuid, WorkGraphNode>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let vec: Vec<(Uuid, WorkGraphNode)> = Deserialize::deserialize(deserializer)?;
    Ok(vec.into_iter().collect())
}

/// Serialization helper for deterministic edge output
fn serialize_ordered_edges<S>(value: &[(Uuid, Uuid)], serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    let mut sorted = value.to_vec();
    sorted.sort();
    serde::Serialize::serialize(&sorted, serializer)
}

/// Deserialization helper for edges
fn deserialize_ordered_edges<'de, D>(deserializer: D) -> Result<Vec<(Uuid, Uuid)>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let vec: Vec<(Uuid, Uuid)> = Deserialize::deserialize(deserializer)?;
    Ok(vec)
}

/// Metadata about the graph itself
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct GraphMetadata {
    /// Name of the graph (e.g., project name or sprint name)
    #[serde(default)]
    pub name: Option<String>,

    /// When the graph was created
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,

    /// When the graph was last updated
    #[serde(default)]
    pub updated_at: Option<DateTime<Utc>>,

    /// Version identifier
    #[serde(default)]
    pub version: Option<String>,
}

/// A node in the work graph representing a task with rich metadata.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkGraphNode {
    /// Unique identifier for this node
    pub id: Uuid,

    /// Human-readable title/name of this task
    pub title: String,

    /// Detailed description of the task
    #[serde(default)]
    pub description: String,

    /// Current status of this node
    #[serde(default)]
    pub status: NodeStatus,

    /// Status color for visualization (hex color code)
    /// E.g., "#4CAF50" for green, "#F44336" for red
    #[serde(default)]
    pub status_color: Option<String>,

    /// Complexity weight for scheduling/prioritization (0.0 to 1.0)
    #[serde(default)]
    pub complexity_weight: Option<f64>,

    /// Links to specifications or requirements documents
    #[serde(default)]
    pub spec_links: Vec<SpecLink>,

    /// Agent session IDs that have worked on or are working on this task
    #[serde(default)]
    pub agent_session_ids: Vec<Uuid>,

    /// References to code changes associated with this task
    #[serde(default)]
    pub code_change_refs: Vec<CodeChangeRef>,

    /// References to test results associated with this task
    #[serde(default)]
    pub test_result_refs: Vec<TestResultRef>,

    /// When this node was created
    #[serde(default)]
    pub created_at: DateTime<Utc>,

    /// When this node was last updated
    #[serde(default)]
    pub updated_at: DateTime<Utc>,
}

/// Status of a node in the work graph
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum NodeStatus {
    /// Node is pending and not yet started
    #[default]
    Pending,
    /// Node is currently being worked on
    InProgress,
    /// Node work is complete
    Completed,
    /// Node was blocked by dependencies
    Blocked,
    /// Node failed during execution
    Failed,
    /// Node was skipped
    Skipped,
}

impl fmt::Display for NodeStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NodeStatus::Pending => write!(f, "pending"),
            NodeStatus::InProgress => write!(f, "in_progress"),
            NodeStatus::Completed => write!(f, "completed"),
            NodeStatus::Blocked => write!(f, "blocked"),
            NodeStatus::Failed => write!(f, "failed"),
            NodeStatus::Skipped => write!(f, "skipped"),
        }
    }
}

/// Link to a specification or requirements document
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SpecLink {
    /// Type of specification (e.g., "req", "spec", "ticket")
    #[serde(rename = "type", default)]
    pub spec_type: String,

    /// URL or path to the specification
    #[serde(default)]
    pub url: String,

    /// Optional human-readable label
    #[serde(default)]
    pub label: Option<String>,
}

/// Reference to a code change (commit, PR, or file modification)
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CodeChangeRef {
    /// Type of change (e.g., "commit", "pr", "file")
    #[serde(rename = "type", default)]
    pub change_type: String,

    /// Identifier (commit hash, PR number, or file path)
    #[serde(default)]
    pub identifier: String,

    /// URL to the change (optional)
    #[serde(default)]
    pub url: Option<String>,

    /// When this change was made
    #[serde(default)]
    pub timestamp: Option<DateTime<Utc>>,
}

/// Reference to a test result
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TestResultRef {
    /// Test name or pattern
    #[serde(default)]
    pub test_name: String,

    /// Result status (e.g., "passed", "failed", "skipped")
    #[serde(default)]
    pub status: String,

    /// URL to test report (optional)
    #[serde(default)]
    pub report_url: Option<String>,

    /// When the test was run
    #[serde(default)]
    pub run_at: Option<DateTime<Utc>>,

    /// Duration in milliseconds (optional)
    #[serde(default)]
    pub duration_ms: Option<u64>,
}

/// Metadata for creating a new node
#[derive(Debug, Clone)]
pub struct NodeMetadata {
    /// Human-readable title/name
    pub title: String,

    /// Detailed description
    pub description: String,

    /// Status color for visualization
    pub status_color: Option<String>,

    /// Complexity weight (0.0 to 1.0)
    pub complexity_weight: Option<f64>,

    /// Links to specifications
    pub spec_links: Vec<SpecLink>,

    /// Agent session IDs
    pub agent_session_ids: Vec<Uuid>,

    /// Code change references
    pub code_change_refs: Vec<CodeChangeRef>,

    /// Test result references
    pub test_result_refs: Vec<TestResultRef>,
}

impl NodeMetadata {
    /// Create new node metadata with required fields
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            description: String::new(),
            status_color: None,
            complexity_weight: None,
            spec_links: Vec::new(),
            agent_session_ids: Vec::new(),
            code_change_refs: Vec::new(),
            test_result_refs: Vec::new(),
        }
    }

    /// Set the description
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = description.into();
        self
    }

    /// Set the status color
    pub fn with_status_color(mut self, color: impl Into<String>) -> Self {
        self.status_color = Some(color.into());
        self
    }

    /// Set the complexity weight
    pub fn with_complexity_weight(mut self, weight: f64) -> Self {
        self.complexity_weight = Some(weight);
        self
    }

    /// Add a spec link
    pub fn with_spec_link(mut self, spec_type: impl Into<String>, url: impl Into<String>) -> Self {
        self.spec_links.push(SpecLink {
            spec_type: spec_type.into(),
            url: url.into(),
            label: None,
        });
        self
    }

    /// Add an agent session ID
    pub fn with_agent_session_id(mut self, session_id: Uuid) -> Self {
        self.agent_session_ids.push(session_id);
        self
    }

    /// Add a code change reference
    pub fn with_code_change_ref(
        mut self,
        change_type: impl Into<String>,
        identifier: impl Into<String>,
    ) -> Self {
        self.code_change_refs.push(CodeChangeRef {
            change_type: change_type.into(),
            identifier: identifier.into(),
            url: None,
            timestamp: None,
        });
        self
    }

    /// Add a test result reference
    pub fn with_test_result_ref(
        mut self,
        test_name: impl Into<String>,
        status: impl Into<String>,
    ) -> Self {
        self.test_result_refs.push(TestResultRef {
            test_name: test_name.into(),
            status: status.into(),
            report_url: None,
            run_at: None,
            duration_ms: None,
        });
        self
    }

    /// Build a WorkGraphNode from this metadata
    fn build_node(self, id: Uuid) -> WorkGraphNode {
        let now = Utc::now();
        WorkGraphNode {
            id,
            title: self.title,
            description: self.description,
            status: NodeStatus::Pending,
            status_color: self.status_color,
            complexity_weight: self.complexity_weight,
            spec_links: self.spec_links,
            agent_session_ids: self.agent_session_ids,
            code_change_refs: self.code_change_refs,
            test_result_refs: self.test_result_refs,
            created_at: now,
            updated_at: now,
        }
    }
}

impl WorkGraph {
    /// Create a new empty work graph
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            edges: Vec::new(),
            metadata: GraphMetadata::default(),
        }
    }

    /// Create a new work graph with metadata
    pub fn with_metadata(name: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            nodes: HashMap::new(),
            edges: Vec::new(),
            metadata: GraphMetadata {
                name: Some(name.into()),
                created_at: Some(now),
                updated_at: Some(now),
                version: None,
            },
        }
    }

    /// Add a node to the graph with its dependencies.
    ///
    /// Returns an error if:
    /// - `node_id` already exists in the graph
    /// - Any dependency ID doesn't exist in the graph (except self-reference)
    /// - Adding the node would create a circular dependency
    pub fn add_node(
        &mut self,
        node_id: Uuid,
        metadata: NodeMetadata,
        dependencies: Vec<Uuid>,
    ) -> Result<(), WorkGraphError> {
        // Check if node already exists
        if self.nodes.contains_key(&node_id) {
            return Err(WorkGraphError::NodeAlreadyExists(node_id));
        }

        // Check for self-reference BEFORE validating dependencies
        // Self-reference is a cycle regardless of whether the node exists
        for dep_id in &dependencies {
            if *dep_id == node_id {
                return Err(WorkGraphError::CircularDependency {
                    node_id,
                    involved: vec![node_id],
                });
            }
        }

        // Validate all dependencies exist (self-reference already handled above)
        for dep_id in &dependencies {
            if !self.nodes.contains_key(dep_id) {
                return Err(WorkGraphError::DependencyNotFound(*dep_id));
            }
        }

        // Check for circular dependencies (from other sources)
        if self.would_create_cycle(&node_id, &dependencies) {
            return Err(WorkGraphError::CircularDependency {
                node_id,
                involved: self.find_cycle_nodes(&node_id, &dependencies),
            });
        }

        // Create the node
        let node = metadata.build_node(node_id);
        self.nodes.insert(node_id, node);

        // Add edges (dependency -> node_id means node_id depends on dependency)
        for dep_id in dependencies {
            self.edges.push((node_id, dep_id));
        }

        // Update metadata
        self.metadata.updated_at = Some(Utc::now());

        Ok(())
    }

    /// Update an existing node's metadata
    pub fn update_node(
        &mut self,
        node_id: Uuid,
        update: impl FnOnce(&mut WorkGraphNode),
    ) -> Result<(), WorkGraphError> {
        let node = self
            .nodes
            .get_mut(&node_id)
            .ok_or(WorkGraphError::NodeNotFound(node_id))?;

        update(node);
        node.updated_at = Utc::now();
        self.metadata.updated_at = Some(Utc::now());

        Ok(())
    }

    /// Remove a node from the graph.
    ///
    /// Also removes all edges involving this node.
    pub fn remove_node(&mut self, node_id: Uuid) -> Result<WorkGraphNode, WorkGraphError> {
        let node = self
            .nodes
            .remove(&node_id)
            .ok_or(WorkGraphError::NodeNotFound(node_id))?;

        // Remove all edges involving this node
        self.edges.retain(|(a, b)| *a != node_id && *b != node_id);

        self.metadata.updated_at = Some(Utc::now());

        Ok(node)
    }

    /// Check if adding a node with given dependencies would create a cycle
    fn would_create_cycle(&self, node_id: &Uuid, new_dependencies: &[Uuid]) -> bool {
        // Self-reference is a cycle (node depends on itself)
        // Use explicit iteration to avoid type coercion issues
        for dep in new_dependencies {
            if dep == node_id {
                return true;
            }
        }

        // Check if any new dependency already has an existing edge from this node
        // Adding the same edge twice would be a cycle (self-dependency)
        for dep_id in new_dependencies {
            if self
                .edges
                .iter()
                .any(|(n, d)| *n == *node_id && *d == *dep_id)
            {
                return true;
            }
        }

        // For each new dependency, check if adding edge (node_id -> dep_id) would complete a cycle.
        // A cycle exists if dep_id can reach node_id via existing edges.
        // Edge (A, B) means A depends on B. So "dep_id can reach node_id" means
        // there exists path: dep_id -> ... -> node_id in the existing graph.
        // This would create cycle: node_id -> dep_id -> ... -> node_id
        for dep_id in new_dependencies {
            if self.can_reach(dep_id, node_id) {
                return true;
            }
        }

        // Check if any new dependency can reach another new dependency
        // via existing edges. If Di can reach Dj, then adding edges
        // (N -> Di) and (N -> Dj) creates a cycle: Di -> ... -> Dj -> N -> Di
        for i in 0..new_dependencies.len() {
            for j in 0..new_dependencies.len() {
                if i != j && self.can_reach(&new_dependencies[i], &new_dependencies[j]) {
                    return true;
                }
            }
        }

        false
    }

    /// Check if source can reach target through existing edges (following dependencies)
    fn can_reach(&self, source: &Uuid, target: &Uuid) -> bool {
        // Edge (A, B) means A depends on B (A's dependencies include B).
        // To find "what does A depend on?", look for edges (A, X).
        // Then recursively check if any of those X can reach target.
        let mut visited = HashSet::new();
        self.can_reach_dfs(source, target, &mut visited)
    }

    fn can_reach_dfs(&self, source: &Uuid, target: &Uuid, visited: &mut HashSet<Uuid>) -> bool {
        if source == target {
            return true;
        }

        if visited.contains(source) {
            return false;
        }

        visited.insert(*source);

        // Find all nodes that source depends on (source -> deps -> target)
        // Edge (source, X) means source depends on X
        for &(node, dep) in &self.edges {
            if node == *source && self.can_reach_dfs(&dep, target, visited) {
                return true;
            }
        }

        false
    }

    /// Check if task_a transitively depends on task_b
    fn transitively_depends_on(&self, task_id: &Uuid, target: &Uuid) -> bool {
        // Build adjacency list from edges
        // Edge (A, B) means A depends on B
        // So we need to follow edges from task_id to find if target is reachable
        let mut visited = HashSet::new();
        self.has_path(task_id, target, &mut visited)
    }

    /// DFS to check if there's a path from source to target
    fn has_path(&self, source: &Uuid, target: &Uuid, visited: &mut HashSet<Uuid>) -> bool {
        if source == target {
            return true;
        }

        if visited.contains(source) {
            return false;
        }

        visited.insert(*source);

        // Find all nodes that depend on source (source is a dependency of these nodes)
        for &(node, dep) in &self.edges {
            if dep == *source && self.has_path(&node, target, visited) {
                return true;
            }
        }

        false
    }

    /// Find nodes involved in a potential cycle
    fn find_cycle_nodes(&self, node_id: &Uuid, dependencies: &[Uuid]) -> Vec<Uuid> {
        let mut cycle_nodes = vec![*node_id];

        for dep_id in dependencies {
            if self.transitively_depends_on(dep_id, node_id) {
                cycle_nodes.push(*dep_id);
            }
        }

        cycle_nodes
    }

    /// Get a node by ID
    pub fn get_node(&self, node_id: &Uuid) -> Option<&WorkGraphNode> {
        self.nodes.get(node_id)
    }

    /// Get all nodes as a vector
    pub fn nodes(&self) -> Vec<&WorkGraphNode> {
        self.nodes.values().collect()
    }

    /// Get all edges
    pub fn edges(&self) -> &[(Uuid, Uuid)] {
        &self.edges
    }

    /// Get dependencies (incoming edges) for a node
    pub fn get_dependencies(&self, node_id: &Uuid) -> Vec<Uuid> {
        self.edges
            .iter()
            .filter(|(n, _)| *n == *node_id)
            .map(|(_, dep)| *dep)
            .collect()
    }

    /// Get dependents (nodes that depend on this node) for a node
    pub fn get_dependents(&self, node_id: &Uuid) -> Vec<Uuid> {
        self.edges
            .iter()
            .filter(|(_, dep)| *dep == *node_id)
            .map(|(n, _)| *n)
            .collect()
    }

    /// Get nodes that have no dependencies (root nodes).
    /// These are nodes that nothing depends on (sink nodes in the dependency graph).
    pub fn root_nodes(&self) -> Vec<Uuid> {
        // Collect all nodes that appear as dependents (first element of edge tuple)
        // These are nodes that have outgoing edges (they depend on others)
        let dependents: HashSet<Uuid> = self.edges.iter().map(|(n, _)| *n).collect();
        // Root nodes are those that never appear as dependents (nothing depends on them)
        let mut roots: Vec<Uuid> = self
            .nodes
            .keys()
            .filter(|id| !dependents.contains(id))
            .cloned()
            .collect();
        roots.sort(); // Deterministic order
        roots
    }

    /// Get nodes that have no dependents (leaf nodes).
    /// These are nodes that nothing depends on (source nodes in the dependency graph).
    pub fn leaf_nodes(&self) -> Vec<Uuid> {
        // Collect all nodes that appear as dependencies (second element of edge tuple)
        // These are nodes that have incoming edges (other nodes depend on them)
        let dependencies: HashSet<Uuid> = self.edges.iter().map(|(_, dep)| *dep).collect();
        // Leaf nodes are those that never appear as dependencies (no one depends on them)
        let mut leaves: Vec<Uuid> = self
            .nodes
            .keys()
            .filter(|id| !dependencies.contains(id))
            .cloned()
            .collect();
        leaves.sort(); // Deterministic order
        leaves
    }

    /// Get topological ordering of all nodes.
    ///
    /// Returns nodes in order such that dependencies come before dependents.
    pub fn topological_sort(&self) -> Result<Vec<Uuid>, WorkGraphError> {
        if self.nodes.is_empty() {
            return Ok(Vec::new());
        }

        // Build adjacency list: node -> list of nodes that depend on it
        let mut dependents: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
        for (node, dep) in &self.edges {
            dependents.entry(*dep).or_default().push(*node);
        }

        // Calculate in-degree for each node (number of dependencies)
        let mut in_degree: HashMap<Uuid, usize> = HashMap::new();
        for node_id in self.nodes.keys() {
            let deps = self.get_dependencies(node_id);
            in_degree.insert(*node_id, deps.len());
        }

        // Start with nodes that have no dependencies
        let mut queue: Vec<Uuid> = in_degree
            .iter()
            .filter(|(_, &count)| count == 0)
            .map(|(&id, _)| id)
            .collect();

        queue.sort(); // Deterministic order

        let mut result: Vec<Uuid> = Vec::new();

        while let Some(node_id) = queue.pop() {
            result.push(node_id);

            // Decrease in-degree for all dependents
            if let Some(deps_on_this) = dependents.get(&node_id) {
                for &dependent_id in deps_on_this {
                    if let Some(degree) = in_degree.get_mut(&dependent_id) {
                        *degree = degree.saturating_sub(1);
                        if *degree == 0 {
                            queue.push(dependent_id);
                            queue.sort(); // Keep deterministic
                        }
                    }
                }
            }
        }

        // If we processed all nodes, return in order
        if result.len() == self.nodes.len() {
            Ok(result)
        } else {
            // There's a cycle (shouldn't happen with our validation)
            Err(WorkGraphError::CycleDetected)
        }
    }

    /// Validate the graph has no cycles
    pub fn validate_no_cycles(&self) -> Result<(), WorkGraphError> {
        // Check for self-references in edges
        for (node, dep) in &self.edges {
            if node == dep {
                return Err(WorkGraphError::CircularDependency {
                    node_id: *node,
                    involved: vec![*node, *dep],
                });
            }
        }

        // Use topological sort to detect cycles
        self.topological_sort()?;
        Ok(())
    }

    /// Get the number of nodes in the graph
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Check if the graph is empty
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Get a node's status
    pub fn node_status(&self, node_id: &Uuid) -> Option<NodeStatus> {
        self.nodes.get(node_id).map(|n| n.status)
    }

    /// Set a node's status
    pub fn set_node_status(
        &mut self,
        node_id: &Uuid,
        status: NodeStatus,
    ) -> Result<(), WorkGraphError> {
        let node = self
            .nodes
            .get_mut(node_id)
            .ok_or(WorkGraphError::NodeNotFound(*node_id))?;
        node.status = status;
        node.updated_at = Utc::now();
        self.metadata.updated_at = Some(Utc::now());
        Ok(())
    }

    /// Get nodes filtered by status
    pub fn nodes_by_status(&self, status: NodeStatus) -> Vec<&WorkGraphNode> {
        self.nodes.values().filter(|n| n.status == status).collect()
    }

    /// Get total complexity weight of all nodes
    pub fn total_complexity(&self) -> f64 {
        self.nodes
            .values()
            .filter_map(|n| n.complexity_weight)
            .sum()
    }

    /// Get graph metadata
    pub fn metadata(&self) -> &GraphMetadata {
        &self.metadata
    }

    /// Update graph metadata
    pub fn update_metadata(&mut self, name: Option<String>, version: Option<String>) {
        if let Some(n) = name {
            self.metadata.name = Some(n);
        }
        if let Some(v) = version {
            self.metadata.version = Some(v);
        }
        self.metadata.updated_at = Some(Utc::now());
    }
}

impl Default for WorkGraph {
    fn default() -> Self {
        Self::new()
    }
}

/// Errors that can occur when manipulating the work graph
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkGraphError {
    /// Node already exists in the graph
    NodeAlreadyExists(Uuid),

    /// Node not found in the graph
    NodeNotFound(Uuid),

    /// Dependency not found in the graph
    DependencyNotFound(Uuid),

    /// Adding this node would create a circular dependency
    CircularDependency {
        /// The node that would create the cycle
        node_id: Uuid,
        /// All nodes involved in the cycle
        involved: Vec<Uuid>,
    },

    /// A cycle was detected in the graph
    CycleDetected,
}

impl fmt::Display for WorkGraphError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WorkGraphError::NodeAlreadyExists(id) => {
                write!(f, "Node {} already exists in the graph", id)
            }
            WorkGraphError::NodeNotFound(id) => {
                write!(f, "Node {} not found in the graph", id)
            }
            WorkGraphError::DependencyNotFound(id) => {
                write!(f, "Dependency {} not found in the graph", id)
            }
            WorkGraphError::CircularDependency { node_id, involved } => {
                write!(
                    f,
                    "Adding node {} would create circular dependency involving {:?}",
                    node_id, involved
                )
            }
            WorkGraphError::CycleDetected => {
                write!(f, "A cycle was detected in the graph")
            }
        }
    }
}

impl std::error::Error for WorkGraphError {}

// Serialization/deserialization tests are in the test module

#[cfg(test)]
mod tests {
    use super::*;

    // Helper to create a 5-node DAG for testing
    fn create_5_node_dag() -> WorkGraph {
        let mut graph = WorkGraph::new();

        // Create 5 nodes: A (root), B (depends on A), C (depends on A), D (depends on B, C), E (depends on D)
        // Diamond pattern: A -> B -> D -> E
        //                   \-> C /

        let id_a = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let id_b = Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap();
        let id_c = Uuid::parse_str("00000000-0000-0000-0000-000000000003").unwrap();
        let id_d = Uuid::parse_str("00000000-0000-0000-0000-000000000004").unwrap();
        let id_e = Uuid::parse_str("00000000-0000-0000-0000-000000000005").unwrap();

        // Add nodes with rich metadata
        graph
            .add_node(
                id_a,
                NodeMetadata::new("Task A: Setup")
                    .with_description("Initial project setup")
                    .with_status_color("#4CAF50") // Green
                    .with_complexity_weight(0.2)
                    .with_spec_link("req", "SPEC-001"),
                vec![],
            )
            .unwrap();

        graph
            .add_node(
                id_b,
                NodeMetadata::new("Task B: Core Feature")
                    .with_description("Implement core feature")
                    .with_status_color("#2196F3") // Blue
                    .with_complexity_weight(0.5)
                    .with_spec_link("req", "SPEC-002")
                    .with_agent_session_id(Uuid::new_v4())
                    .with_code_change_ref("commit", "abc123"),
                vec![id_a],
            )
            .unwrap();

        graph
            .add_node(
                id_c,
                NodeMetadata::new("Task C: Secondary Feature")
                    .with_description("Implement secondary feature")
                    .with_status_color("#9C27B0") // Purple
                    .with_complexity_weight(0.3)
                    .with_spec_link("req", "SPEC-003"),
                vec![id_a],
            )
            .unwrap();

        graph
            .add_node(
                id_d,
                NodeMetadata::new("Task D: Integration")
                    .with_description("Integrate B and C")
                    .with_status_color("#FF9800") // Orange
                    .with_complexity_weight(0.4)
                    .with_test_result_ref("test_integration", "passed"),
                vec![id_b, id_c],
            )
            .unwrap();

        graph
            .add_node(
                id_e,
                NodeMetadata::new("Task E: Finalization")
                    .with_description("Finalize and deploy")
                    .with_status_color("#F44336") // Red
                    .with_complexity_weight(0.3),
                vec![id_d],
            )
            .unwrap();

        graph
    }

    // ========================================================================
    // Basic Operations Tests
    // ========================================================================

    #[test]
    fn test_new_graph_is_empty() {
        let graph = WorkGraph::new();
        assert!(graph.is_empty());
        assert_eq!(graph.len(), 0);
    }

    #[test]
    fn test_add_single_node() {
        let mut graph = WorkGraph::new();
        let node_id = Uuid::new_v4();

        graph
            .add_node(node_id, NodeMetadata::new("Test Node"), vec![])
            .unwrap();

        assert!(!graph.is_empty());
        assert_eq!(graph.len(), 1);
        assert!(graph.get_node(&node_id).is_some());
    }

    #[test]
    fn test_add_node_with_dependencies() {
        let mut graph = WorkGraph::new();
        let dep_id = Uuid::new_v4();
        let node_id = Uuid::new_v4();

        graph
            .add_node(dep_id, NodeMetadata::new("Dependency"), vec![])
            .unwrap();
        graph
            .add_node(node_id, NodeMetadata::new("Dependent"), vec![dep_id])
            .unwrap();

        assert_eq!(graph.len(), 2);
        assert_eq!(graph.get_dependencies(&node_id), vec![dep_id]);
    }

    #[test]
    fn test_remove_node() {
        let mut graph = WorkGraph::new();
        let node_id = Uuid::new_v4();

        graph
            .add_node(node_id, NodeMetadata::new("Test"), vec![])
            .unwrap();
        let removed = graph.remove_node(node_id).unwrap();

        assert_eq!(removed.id, node_id);
        assert!(graph.is_empty());
    }

    #[test]
    fn test_remove_node_cleans_up_edges() {
        let mut graph = WorkGraph::new();
        let dep_id = Uuid::new_v4();
        let node_id = Uuid::new_v4();

        graph
            .add_node(dep_id, NodeMetadata::new("Dep"), vec![])
            .unwrap();
        graph
            .add_node(node_id, NodeMetadata::new("Node"), vec![dep_id])
            .unwrap();

        graph.remove_node(dep_id).unwrap();

        assert!(graph.get_node(&dep_id).is_none());
        assert!(graph.get_dependencies(&node_id).is_empty());
    }

    // ========================================================================
    // Cycle Detection Tests
    // ========================================================================

    #[test]
    fn test_self_dependency_rejected() {
        let mut graph = WorkGraph::new();
        let node_id = Uuid::new_v4();

        let result = graph.add_node(node_id, NodeMetadata::new("Test"), vec![node_id]);
        assert!(matches!(
            result,
            Err(WorkGraphError::CircularDependency { .. })
        ));
    }

    #[test]
    fn test_direct_cycle_rejected() {
        let mut graph = WorkGraph::new();
        let id_a = Uuid::new_v4();
        let id_b = Uuid::new_v4();

        graph
            .add_node(id_a, NodeMetadata::new("A"), vec![])
            .unwrap();
        graph
            .add_node(id_b, NodeMetadata::new("B"), vec![id_a])
            .unwrap();

        // Try to make A depend on B - should fail because A already exists
        // (we can't add a node with the same ID twice)
        let result = graph.add_node(id_a, NodeMetadata::new("A"), vec![id_b]);
        assert!(matches!(result, Err(WorkGraphError::NodeAlreadyExists(_))));
    }

    #[test]
    fn test_transitive_cycle_rejected() {
        let mut graph = WorkGraph::new();
        let id_a = Uuid::new_v4();
        let id_b = Uuid::new_v4();
        let id_c = Uuid::new_v4();

        graph
            .add_node(id_a, NodeMetadata::new("A"), vec![])
            .unwrap();
        graph
            .add_node(id_b, NodeMetadata::new("B"), vec![id_a])
            .unwrap();
        graph
            .add_node(id_c, NodeMetadata::new("C"), vec![id_b])
            .unwrap();

        // Try to add A again with C as dependency - should fail because A already exists
        let result = graph.add_node(id_a, NodeMetadata::new("A"), vec![id_c]);
        assert!(matches!(result, Err(WorkGraphError::NodeAlreadyExists(_))));
    }

    #[test]
    fn test_diamond_dependency_allowed() {
        let mut graph = WorkGraph::new();
        let id_a = Uuid::new_v4();
        let id_b = Uuid::new_v4();
        let id_c = Uuid::new_v4();
        let id_d = Uuid::new_v4();

        // Diamond: A -> B, A -> C, B -> D, C -> D (no cycle)
        graph
            .add_node(id_a, NodeMetadata::new("A"), vec![])
            .unwrap();
        graph
            .add_node(id_b, NodeMetadata::new("B"), vec![id_a])
            .unwrap();
        graph
            .add_node(id_c, NodeMetadata::new("C"), vec![id_a])
            .unwrap();
        graph
            .add_node(id_d, NodeMetadata::new("D"), vec![id_b, id_c])
            .unwrap();

        assert_eq!(graph.len(), 4);
        assert!(graph.validate_no_cycles().is_ok());
    }

    // ========================================================================
    // Topological Sort Tests
    // ========================================================================

    #[test]
    fn test_topological_sort_linear() {
        let mut graph = WorkGraph::new();
        let id_a = Uuid::new_v4();
        let id_b = Uuid::new_v4();
        let id_c = Uuid::new_v4();

        graph
            .add_node(id_a, NodeMetadata::new("A"), vec![])
            .unwrap();
        graph
            .add_node(id_b, NodeMetadata::new("B"), vec![id_a])
            .unwrap();
        graph
            .add_node(id_c, NodeMetadata::new("C"), vec![id_b])
            .unwrap();

        let sorted = graph.topological_sort().unwrap();

        let a_idx = sorted.iter().position(|&id| id == id_a).unwrap();
        let b_idx = sorted.iter().position(|&id| id == id_b).unwrap();
        let c_idx = sorted.iter().position(|&id| id == id_c).unwrap();

        assert!(a_idx < b_idx);
        assert!(b_idx < c_idx);
    }

    #[test]
    fn test_topological_sort_parallel_branches() {
        let mut graph = WorkGraph::new();
        let id_a = Uuid::new_v4();
        let id_b = Uuid::new_v4();
        let id_c = Uuid::new_v4();

        graph
            .add_node(id_a, NodeMetadata::new("A"), vec![])
            .unwrap();
        graph
            .add_node(id_b, NodeMetadata::new("B"), vec![id_a])
            .unwrap();
        graph
            .add_node(id_c, NodeMetadata::new("C"), vec![id_a])
            .unwrap();

        let sorted = graph.topological_sort().unwrap();

        let a_idx = sorted.iter().position(|&id| id == id_a).unwrap();
        let b_idx = sorted.iter().position(|&id| id == id_b).unwrap();
        let c_idx = sorted.iter().position(|&id| id == id_c).unwrap();

        assert!(a_idx < b_idx);
        assert!(a_idx < c_idx);
    }

    // ========================================================================
    // Root and Leaf Node Tests
    // ========================================================================

    #[test]
    fn test_root_nodes() {
        let graph = create_5_node_dag();
        let roots = graph.root_nodes();

        assert_eq!(roots.len(), 1);
        let root_id = roots[0];
        assert_eq!(root_id.to_string(), "00000000-0000-0000-0000-000000000001");
    }

    #[test]
    fn test_leaf_nodes() {
        let graph = create_5_node_dag();
        let leaves = graph.leaf_nodes();

        assert_eq!(leaves.len(), 1);
        let leaf_id = leaves[0];
        assert_eq!(leaf_id.to_string(), "00000000-0000-0000-0000-000000000005");
    }

    // ========================================================================
    // Serialization Tests (VAL-ORCH-013)
    // ========================================================================

    #[test]
    fn test_5_node_dag_serialization() {
        let graph = create_5_node_dag();

        // Serialize to JSON
        let json = serde_json::to_string_pretty(&graph).unwrap();

        // Verify JSON structure has nodes and edges
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(parsed.get("nodes").is_some());
        assert!(parsed.get("edges").is_some());

        // Verify nodes array has 5 elements
        let nodes = parsed.get("nodes").unwrap().as_array().unwrap();
        assert_eq!(nodes.len(), 5);

        // Verify edges exist
        let edges = parsed.get("edges").unwrap().as_array().unwrap();
        assert!(!edges.is_empty());
    }

    #[test]
    fn test_metadata_fields_in_serialization() {
        let mut graph = WorkGraph::new();
        let node_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();

        graph
            .add_node(
                node_id,
                NodeMetadata::new("Test Task")
                    .with_description("A test task")
                    .with_status_color("#FF0000")
                    .with_complexity_weight(0.75)
                    .with_spec_link("requirement", "REQ-123")
                    .with_agent_session_id(session_id)
                    .with_code_change_ref("commit", "abc123def")
                    .with_test_result_ref("test_foo", "passed"),
                vec![],
            )
            .unwrap();

        let json = serde_json::to_string_pretty(&graph).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        // Check node has all metadata fields (nodes is serialized as array of [key, value] pairs)
        let nodes = parsed.get("nodes").unwrap().as_array().unwrap();
        assert_eq!(nodes.len(), 1);

        // Each entry is [uuid, node_object]
        let node_entry = nodes[0].as_array().expect("nodes entry should be array");
        assert!(
            node_entry.len() >= 2,
            "nodes entry should have [uuid, node]"
        );

        // Access the second element (node object) - handle Option properly
        let node_obj = node_entry
            .get(1)
            .and_then(|v| v.as_object())
            .expect("node object expected");

        // Verify we have the right keys
        assert!(
            node_obj.contains_key("agent_session_ids"),
            "Missing 'agent_session_ids' key. Available keys: {:?}",
            node_obj.keys().collect::<Vec<_>>()
        );

        assert_eq!(
            node_obj.get("title").unwrap().as_str().unwrap(),
            "Test Task"
        );
        assert_eq!(
            node_obj.get("description").unwrap().as_str().unwrap(),
            "A test task"
        );
        assert_eq!(
            node_obj.get("status_color").unwrap().as_str().unwrap(),
            "#FF0000"
        );
        assert!(
            (node_obj.get("complexity_weight").unwrap().as_f64().unwrap() - 0.75).abs()
                < f64::EPSILON
        );

        // Check spec_links - JSON uses "type" (from #[serde(rename = "type")])
        let spec_links = node_obj.get("spec_links").unwrap().as_array().unwrap();
        assert_eq!(spec_links.len(), 1);
        assert_eq!(
            spec_links[0].get("type").unwrap().as_str().unwrap(),
            "requirement"
        );
        assert_eq!(
            spec_links[0].get("url").unwrap().as_str().unwrap(),
            "REQ-123"
        );

        // Check agent_session_ids
        let agent_ids = node_obj
            .get("agent_session_ids")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(agent_ids.len(), 1);

        // Check code_change_refs - JSON uses "type" (from #[serde(rename = "type")])
        let code_refs = node_obj
            .get("code_change_refs")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(code_refs.len(), 1);
        assert_eq!(
            code_refs[0].get("type").unwrap().as_str().unwrap(),
            "commit"
        );
        assert_eq!(
            code_refs[0].get("identifier").unwrap().as_str().unwrap(),
            "abc123def"
        );

        // Check test_result_refs
        let test_refs = node_obj
            .get("test_result_refs")
            .unwrap()
            .as_array()
            .unwrap();
        assert_eq!(test_refs.len(), 1);
        assert_eq!(
            test_refs[0].get("test_name").unwrap().as_str().unwrap(),
            "test_foo"
        );
        assert_eq!(
            test_refs[0].get("status").unwrap().as_str().unwrap(),
            "passed"
        );
    }

    #[test]
    fn test_deserialization_produces_equivalent_graph() {
        let graph = create_5_node_dag();

        // Round-trip through JSON
        let json = serde_json::to_string(&graph).unwrap();
        let restored: WorkGraph = serde_json::from_str(&json).unwrap();

        // Verify same structure
        assert_eq!(graph.len(), restored.len());
        assert_eq!(graph.edges().len(), restored.edges().len());

        // Verify all nodes exist with same data
        for (id, node) in &graph.nodes {
            let restored_node = restored.get_node(id).unwrap();
            assert_eq!(node, restored_node);
        }

        // Verify edges are the same
        let mut orig_edges = graph.edges().to_vec();
        let mut rest_edges = restored.edges().to_vec();
        orig_edges.sort();
        rest_edges.sort();
        assert_eq!(orig_edges, rest_edges);

        // Verify topological sort produces same result
        let orig_sorted = graph.topological_sort().unwrap();
        let rest_sorted = restored.topological_sort().unwrap();
        assert_eq!(orig_sorted, rest_sorted);
    }

    #[test]
    fn test_deserialization_with_nested_structures() {
        let mut graph = WorkGraph::with_metadata("Test Graph");
        let node_id = Uuid::new_v4();

        graph
            .add_node(
                node_id,
                NodeMetadata::new("Complex Node")
                    .with_description("A node with all fields populated")
                    .with_status_color("#00FF00")
                    .with_complexity_weight(0.5)
                    .with_spec_link("spec", "https://example.com/spec")
                    .with_spec_link("ticket", "JIRA-123")
                    .with_agent_session_id(Uuid::new_v4())
                    .with_agent_session_id(Uuid::new_v4())
                    .with_code_change_ref("pr", "https://github.com/org/repo/pull/456")
                    .with_code_change_ref("commit", "abc789")
                    .with_test_result_ref("test_a", "passed")
                    .with_test_result_ref("test_b", "failed")
                    .with_test_result_ref("test_c", "skipped"),
                vec![],
            )
            .unwrap();

        let json = serde_json::to_string_pretty(&graph).unwrap();
        let restored: WorkGraph = serde_json::from_str(&json).unwrap();

        assert_eq!(graph, restored);

        // Check nested structures specifically
        let node = restored.get_node(&node_id).unwrap();
        assert_eq!(node.spec_links.len(), 2);
        assert_eq!(node.agent_session_ids.len(), 2);
        assert_eq!(node.code_change_refs.len(), 2);
        assert_eq!(node.test_result_refs.len(), 3);
    }

    #[test]
    fn test_empty_graph_serialization() {
        let graph = WorkGraph::new();

        let json = serde_json::to_string(&graph).unwrap();
        let restored: WorkGraph = serde_json::from_str(&json).unwrap();

        assert_eq!(graph, restored);
        assert!(restored.is_empty());
    }

    #[test]
    fn test_graph_with_metadata_serialization() {
        let graph = WorkGraph::with_metadata("Sprint 42");

        let json = serde_json::to_string(&graph).unwrap();
        let restored: WorkGraph = serde_json::from_str(&json).unwrap();

        assert_eq!(graph.metadata.name, restored.metadata.name);
    }

    // ========================================================================
    // Node Status Tests
    // ========================================================================

    #[test]
    fn test_node_status_update() {
        let mut graph = WorkGraph::new();
        let node_id = Uuid::new_v4();

        graph
            .add_node(node_id, NodeMetadata::new("Test"), vec![])
            .unwrap();

        assert_eq!(graph.node_status(&node_id), Some(NodeStatus::Pending));

        graph
            .set_node_status(&node_id, NodeStatus::InProgress)
            .unwrap();
        assert_eq!(graph.node_status(&node_id), Some(NodeStatus::InProgress));

        graph
            .set_node_status(&node_id, NodeStatus::Completed)
            .unwrap();
        assert_eq!(graph.node_status(&node_id), Some(NodeStatus::Completed));
    }

    #[test]
    fn test_nodes_by_status() {
        let mut graph = WorkGraph::new();
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();
        let id3 = Uuid::new_v4();

        graph
            .add_node(id1, NodeMetadata::new("Node 1"), vec![])
            .unwrap();
        graph
            .add_node(id2, NodeMetadata::new("Node 2"), vec![])
            .unwrap();
        graph
            .add_node(id3, NodeMetadata::new("Node 3"), vec![])
            .unwrap();

        graph.set_node_status(&id1, NodeStatus::InProgress).unwrap();
        graph.set_node_status(&id2, NodeStatus::InProgress).unwrap();
        // id3 remains Pending

        let in_progress = graph.nodes_by_status(NodeStatus::InProgress);
        assert_eq!(in_progress.len(), 2);

        let pending = graph.nodes_by_status(NodeStatus::Pending);
        assert_eq!(pending.len(), 1);
    }

    // ========================================================================
    // Complexity and Statistics Tests
    // ========================================================================

    #[test]
    fn test_total_complexity() {
        let graph = create_5_node_dag();

        // 0.2 + 0.5 + 0.3 + 0.4 + 0.3 = 1.7
        let total = graph.total_complexity();
        assert!((total - 1.7).abs() < f64::EPSILON);
    }

    // ========================================================================
    // Update Node Tests
    // ========================================================================

    #[test]
    fn test_update_node() {
        let mut graph = WorkGraph::new();
        let node_id = Uuid::new_v4();

        graph
            .add_node(node_id, NodeMetadata::new("Original"), vec![])
            .unwrap();

        graph
            .update_node(node_id, |node| {
                node.title = "Updated".to_string();
                node.status = NodeStatus::Completed;
            })
            .unwrap();

        let node = graph.get_node(&node_id).unwrap();
        assert_eq!(node.title, "Updated");
        assert_eq!(node.status, NodeStatus::Completed);
    }

    #[test]
    fn test_update_nonexistent_node() {
        let mut graph = WorkGraph::new();
        let fake_id = Uuid::new_v4();

        let result = graph.update_node(fake_id, |_| {});
        assert!(matches!(result, Err(WorkGraphError::NodeNotFound(_))));
    }

    // ========================================================================
    // Dependent/Dependency Query Tests
    // ========================================================================

    #[test]
    fn test_get_dependents() {
        let graph = create_5_node_dag();

        // Node A should have B and C as dependents
        let id_a = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let dependents = graph.get_dependents(&id_a);

        assert_eq!(dependents.len(), 2);
    }

    #[test]
    fn test_get_dependencies_for_leaf() {
        let graph = create_5_node_dag();

        // Leaf node E should depend on D
        let id_e = Uuid::parse_str("00000000-0000-0000-0000-000000000005").unwrap();
        let deps = graph.get_dependencies(&id_e);

        assert_eq!(deps.len(), 1);
        let id_d = Uuid::parse_str("00000000-0000-0000-0000-000000000004").unwrap();
        assert!(deps.contains(&id_d));
    }
}
