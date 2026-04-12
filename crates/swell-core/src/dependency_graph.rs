//! AST-based dependency graph construction with typed nodes and edges.
//!
//! This module provides:
//! - Graph construction from AST (via tree-sitter)
//! - Typed nodes: File, Module, Class, Function, Variable, Test
//! - Typed edges: CALLS, INHERITS_FROM, IMPORTS, DEPENDS_ON, TESTS
//! - Graph query API for dependency analysis
//! - Impact analysis for code changes
//!
//! Reference: `Memory and Learning Architecture.md`

use crate::treesitter::{parse_source, ChunkType, DependencyType, ParseResult, SourceLanguage};
use crate::{KgDirection, KgEdge, KgNode, KgNodeType, KgPath, KgRelation, KgTraversal};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// A dependency graph built from AST analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyGraph {
    /// All nodes indexed by ID
    nodes: HashMap<Uuid, KgNode>,
    /// All edges indexed by source node ID
    edges: HashMap<Uuid, Vec<KgEdge>>,
    /// File path to nodes index
    file_nodes: HashMap<String, Vec<Uuid>>,
    /// Node name to nodes index (for fast lookup)
    name_index: HashMap<String, Vec<Uuid>>,
    /// Chunk ID to node ID mapping
    chunk_to_node: HashMap<Uuid, Uuid>,
}

impl Default for DependencyGraph {
    fn default() -> Self {
        Self::new()
    }
}

impl DependencyGraph {
    /// Create a new empty dependency graph
    pub fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            edges: HashMap::new(),
            file_nodes: HashMap::new(),
            name_index: HashMap::new(),
            chunk_to_node: HashMap::new(),
        }
    }

    /// Add a node to the graph
    pub fn add_node(&mut self, node: KgNode) -> Uuid {
        let id = node.id;
        self.nodes.insert(id, node.clone());

        // Index by name for fast lookup
        let name = node.name.clone();
        self.name_index.entry(name).or_default().push(id);

        id
    }

    /// Add an edge to the graph
    pub fn add_edge(&mut self, edge: KgEdge) {
        self.edges.entry(edge.source).or_default().push(edge);
    }

    /// Get a node by ID
    pub fn get_node(&self, id: Uuid) -> Option<&KgNode> {
        self.nodes.get(&id)
    }

    /// Query nodes by name (exact match)
    pub fn query_by_name(&self, name: &str) -> Vec<&KgNode> {
        self.name_index
            .get(name)
            .map(|ids| ids.iter().filter_map(|id| self.nodes.get(id)).collect())
            .unwrap_or_default()
    }

    /// Query nodes by type
    pub fn query_by_type(&self, node_type: KgNodeType) -> Vec<&KgNode> {
        self.nodes
            .values()
            .filter(|n| n.node_type == node_type)
            .collect()
    }

    /// Get all nodes of a given type
    pub fn get_nodes_by_type(&self, node_type: KgNodeType) -> Vec<&KgNode> {
        self.query_by_type(node_type)
    }

    /// Get outgoing edges from a node
    pub fn get_outgoing_edges(&self, node_id: Uuid) -> Vec<&KgEdge> {
        self.edges
            .get(&node_id)
            .map(|e| e.as_slice())
            .unwrap_or(&[])
            .iter()
            .collect()
    }

    /// Get incoming edges to a node
    pub fn get_incoming_edges(&self, node_id: Uuid) -> Vec<&KgEdge> {
        self.edges
            .values()
            .flat_map(|e| e.iter())
            .filter(|e| e.target == node_id)
            .collect()
    }

    /// Get all edges of a specific relation type
    pub fn get_edges_by_relation(&self, relation: KgRelation) -> Vec<&KgEdge> {
        self.edges
            .values()
            .flat_map(|e| e.iter())
            .filter(|e| e.relation == relation)
            .collect()
    }

    /// Build a graph from a parse result (AST)
    pub fn from_parse_result(&mut self, parse: &ParseResult) {
        // Create file node
        let file_name = parse
            .source_path
            .clone()
            .unwrap_or_else(|| "unknown".to_string());
        let file_node = KgNode {
            id: Uuid::new_v4(),
            node_type: KgNodeType::File,
            name: file_name.clone(),
            properties: serde_json::json!({
                "language": parse.language,
            }),
        };
        let file_id = self.add_node(file_node);

        // Index file
        self.file_nodes.entry(file_name).or_default().push(file_id);

        // Process chunks and create nodes
        for chunk in &parse.chunks {
            let node_type = match chunk.chunk_type {
                ChunkType::Function => KgNodeType::Function,
                ChunkType::Method => KgNodeType::Method,
                ChunkType::Class => KgNodeType::Class,
                ChunkType::Struct => KgNodeType::Type,
                ChunkType::Enum => KgNodeType::Type,
                ChunkType::Module => KgNodeType::Module,
                ChunkType::Interface | ChunkType::Trait => KgNodeType::Type,
            };

            let node = KgNode {
                id: Uuid::new_v4(),
                node_type,
                name: chunk.name.clone(),
                properties: serde_json::json!({
                    "source_path": parse.source_path,
                    "start_line": chunk.start_line,
                    "end_line": chunk.end_line,
                    "source_code_length": chunk.source_code.len(),
                }),
            };

            let node_id = self.add_node(node);

            // Map chunk to node for later reference
            self.chunk_to_node.insert(chunk.id, node_id);

            // Add CONTAINS edge from file to chunk
            let contains_edge = KgEdge {
                id: Uuid::new_v4(),
                source: file_id,
                target: node_id,
                relation: KgRelation::Contains,
            };
            self.add_edge(contains_edge);
        }

        // Process dependencies and create edges
        for dep in &parse.dependencies {
            let (relation, target_name) = match dep.dependency_type {
                DependencyType::Import => (KgRelation::Imports, Some(dep.target.clone())),
                DependencyType::Call => (KgRelation::Calls, Some(dep.target.clone())),
                DependencyType::Reference => (KgRelation::DependsOn, Some(dep.target.clone())),
                DependencyType::Inheritance => (KgRelation::InheritsFrom, Some(dep.target.clone())),
            };

            // Determine source node
            let source_id = if let Some(chunk_id) = &dep.source_chunk {
                self.chunk_to_node.get(chunk_id).copied()
            } else {
                // If no source chunk, use file node as source
                Some(file_id)
            };

            if let Some(source) = source_id {
                // Find or create target node
                if let Some(ref name) = target_name {
                    // Try to find existing node with this name
                    let targets: Vec<Uuid> = self.name_index.get(name).cloned().unwrap_or_default();

                    if let Some(&target_id) = targets.first() {
                        let edge = KgEdge {
                            id: Uuid::new_v4(),
                            source,
                            target: target_id,
                            relation,
                        };
                        self.add_edge(edge);
                    }
                    // Note: Don't create nodes for external dependencies
                    // They would need to be resolved from other files
                }
            }
        }
    }

    /// Build a graph from source code
    pub fn from_source(source: &[u8], language: SourceLanguage) -> Result<Self, String> {
        let parse = parse_source(source, language)?;
        let mut graph = Self::new();
        graph.from_parse_result(&parse);
        Ok(graph)
    }

    /// Traverse the graph from a starting node
    pub fn traverse(&self, traversal: &KgTraversal) -> Vec<KgPath> {
        let mut paths = Vec::new();
        let mut visited = HashSet::new();
        let start_nodes = if traversal.start_node == Uuid::nil() {
            // Start from all nodes
            self.nodes.keys().copied().collect()
        } else {
            vec![traversal.start_node]
        };

        for start in start_nodes {
            self.traverse_from(
                start,
                traversal.relation,
                traversal.max_depth,
                traversal.direction,
                &mut visited,
                &mut KgPath {
                    nodes: Vec::new(),
                    edges: Vec::new(),
                },
                &mut paths,
            );
        }

        paths
    }

    #[allow(clippy::too_many_arguments)]
    fn traverse_from(
        &self,
        node_id: Uuid,
        relation: Option<KgRelation>,
        max_depth: usize,
        direction: KgDirection,
        visited: &mut HashSet<Uuid>,
        current_path: &mut KgPath,
        paths: &mut Vec<KgPath>,
    ) {
        // Base case: depth exhausted or already visiting this node
        if max_depth == 0 || visited.contains(&node_id) {
            return;
        }

        visited.insert(node_id);

        // Add current node to path
        if let Some(node) = self.nodes.get(&node_id) {
            current_path.nodes.push(node.clone());
        }

        // Get edges based on direction
        let edges: Vec<_> = match direction {
            KgDirection::Outgoing => self.get_outgoing_edges(node_id),
            KgDirection::Incoming => self.get_incoming_edges(node_id),
            KgDirection::Both => {
                let mut both = self.get_outgoing_edges(node_id);
                both.extend(self.get_incoming_edges(node_id));
                both
            }
        };

        // Filter edges by relation if specified
        let filtered_edges: Vec<_> = edges
            .into_iter()
            .filter(|edge| relation.is_none_or(|rel| edge.relation == rel))
            .collect();

        // If no edges to follow, this is a leaf node - record the path
        if filtered_edges.is_empty() {
            if !current_path.nodes.is_empty() {
                paths.push(current_path.clone());
            }
            current_path.nodes.pop();
            visited.remove(&node_id);
            return;
        }

        for edge in filtered_edges {
            current_path.edges.push(edge.clone());
            let next_id = if direction == KgDirection::Outgoing {
                edge.target
            } else {
                edge.source
            };
            self.traverse_from(
                next_id,
                relation,
                max_depth - 1,
                direction,
                visited,
                current_path,
                paths,
            );
            current_path.edges.pop();
        }

        current_path.nodes.pop();
        visited.remove(&node_id);
    }

    /// Find all nodes that depend on (or are called by) a given node
    pub fn find_dependents(&self, node_id: Uuid) -> Vec<Uuid> {
        self.get_incoming_edges(node_id)
            .iter()
            .map(|e| e.source)
            .collect()
    }

    /// Find all nodes that a given node depends on (or calls)
    pub fn find_dependencies(&self, node_id: Uuid) -> Vec<Uuid> {
        self.get_outgoing_edges(node_id)
            .iter()
            .map(|e| e.target)
            .collect()
    }

    /// Perform impact analysis for a change to a node
    /// Returns all nodes potentially affected by changes to the given node
    pub fn impact_analysis(&self, node_id: Uuid) -> ImpactResult {
        let mut impacted_files = HashSet::new();
        let mut impacted_nodes = HashSet::new();
        let mut test_nodes = HashSet::new();

        // Direct dependents
        let direct_dependents: Vec<_> = self.find_dependents(node_id);
        for dep_id in &direct_dependents {
            if let Some(node) = self.nodes.get(dep_id) {
                impacted_nodes.insert(*dep_id);
                if let Some(path) = node.properties.get("source_path").and_then(|v| v.as_str()) {
                    impacted_files.insert(path.to_string());
                }
                // Check if this is a test node
                if node.node_type == KgNodeType::Test {
                    test_nodes.insert(*dep_id);
                }
            }
        }

        // Transitive dependents (cascading impact)
        let mut queue: Vec<_> = direct_dependents.clone();
        let mut visited: HashSet<Uuid> = HashSet::new();
        visited.insert(node_id);

        while let Some(current_id) = queue.pop() {
            let transitive: Vec<_> = self
                .find_dependents(current_id)
                .into_iter()
                .filter(|id| !visited.contains(id))
                .collect();

            for dep_id in transitive {
                visited.insert(dep_id);
                if let Some(node) = self.nodes.get(&dep_id) {
                    impacted_nodes.insert(dep_id);
                    if let Some(path) = node.properties.get("source_path").and_then(|v| v.as_str())
                    {
                        impacted_files.insert(path.to_string());
                    }
                    if node.node_type == KgNodeType::Test {
                        test_nodes.insert(dep_id);
                    }
                    queue.push(dep_id);
                }
            }
        }

        let direct_count = direct_dependents.len();
        let total_count = impacted_nodes.len();

        ImpactResult {
            impacted_nodes: impacted_nodes.into_iter().collect(),
            impacted_files: impacted_files.into_iter().collect(),
            test_nodes: test_nodes.into_iter().collect(),
            direct_dependents: direct_count,
            total_impacted: total_count,
        }
    }

    /// Get statistics about the graph
    pub fn stats(&self) -> GraphStats {
        let mut node_counts = HashMap::new();
        for node in self.nodes.values() {
            *node_counts.entry(node.node_type).or_insert(0) += 1;
        }

        let mut edge_counts = HashMap::new();
        for edges in self.edges.values() {
            for edge in edges {
                *edge_counts.entry(edge.relation).or_insert(0) += 1;
            }
        }

        GraphStats {
            total_nodes: self.nodes.len(),
            total_edges: self.edges.values().map(|e| e.len()).sum(),
            node_counts,
            edge_counts,
            files: self.file_nodes.len(),
        }
    }

    /// Merge another graph into this one
    pub fn merge(&mut self, other: &DependencyGraph) {
        for (id, node) in &other.nodes {
            if !self.nodes.contains_key(id) {
                self.add_node(node.clone());
            }
        }
        for (source, edges) in &other.edges {
            for edge in edges {
                // Avoid duplicates by checking if this exact edge already exists
                let exists = self
                    .edges
                    .get(source)
                    .map(|e| e.iter().any(|existing| existing.id == edge.id))
                    .unwrap_or(false);
                if !exists {
                    self.add_edge(edge.clone());
                }
            }
        }
    }

    /// Clear all nodes and edges
    pub fn clear(&mut self) {
        self.nodes.clear();
        self.edges.clear();
        self.file_nodes.clear();
        self.name_index.clear();
        self.chunk_to_node.clear();
    }

    /// Get all nodes
    pub fn all_nodes(&self) -> Vec<&KgNode> {
        self.nodes.values().collect()
    }

    /// Get all edges
    pub fn all_edges(&self) -> Vec<&KgEdge> {
        self.edges.values().flat_map(|e| e.iter()).collect()
    }

    /// Check if a node exists
    pub fn contains_node(&self, node_id: Uuid) -> bool {
        self.nodes.contains_key(&node_id)
    }
}

/// Result of impact analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImpactResult {
    /// All nodes that would be impacted
    pub impacted_nodes: Vec<Uuid>,
    /// All files that would be impacted
    pub impacted_files: Vec<String>,
    /// Specifically test nodes impacted
    pub test_nodes: Vec<Uuid>,
    /// Number of direct dependents
    pub direct_dependents: usize,
    /// Total number of impacted nodes
    pub total_impacted: usize,
}

impl ImpactResult {
    /// Check if the impact is within acceptable limits
    pub fn is_acceptable(&self, max_impact: usize) -> bool {
        self.total_impacted <= max_impact
    }

    /// Check if any tests would be impacted
    pub fn has_test_impact(&self) -> bool {
        !self.test_nodes.is_empty()
    }
}

/// Graph statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphStats {
    pub total_nodes: usize,
    pub total_edges: usize,
    pub node_counts: HashMap<KgNodeType, usize>,
    pub edge_counts: HashMap<KgRelation, usize>,
    pub files: usize,
}

/// Query options for dependency analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyQuery {
    /// Node name to search for
    pub name: Option<String>,
    /// Node types to include
    pub node_types: Option<Vec<KgNodeType>>,
    /// Relation types to follow
    pub relations: Option<Vec<KgRelation>>,
    /// Maximum traversal depth
    pub max_depth: usize,
    /// Direction to traverse
    pub direction: KgDirection,
}

impl Default for DependencyQuery {
    fn default() -> Self {
        Self {
            name: None,
            node_types: None,
            relations: None,
            max_depth: 3,
            direction: KgDirection::Both,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_graph() {
        let graph = DependencyGraph::new();
        assert_eq!(graph.nodes.len(), 0);
        assert_eq!(graph.all_edges().len(), 0);
    }

    #[test]
    fn test_add_node() {
        let mut graph = DependencyGraph::new();
        let node = KgNode {
            id: Uuid::new_v4(),
            node_type: KgNodeType::Function,
            name: "test_func".to_string(),
            properties: serde_json::json!({}),
        };
        let id = graph.add_node(node);
        assert!(graph.contains_node(id));
        assert_eq!(graph.query_by_name("test_func").len(), 1);
    }

    #[test]
    fn test_add_edge() {
        let mut graph = DependencyGraph::new();
        let node1 = KgNode {
            id: Uuid::new_v4(),
            node_type: KgNodeType::Function,
            name: "caller".to_string(),
            properties: serde_json::json!({}),
        };
        let node2 = KgNode {
            id: Uuid::new_v4(),
            node_type: KgNodeType::Function,
            name: "callee".to_string(),
            properties: serde_json::json!({}),
        };
        let id1 = graph.add_node(node1);
        let id2 = graph.add_node(node2);

        let edge = KgEdge {
            id: Uuid::new_v4(),
            source: id1,
            target: id2,
            relation: KgRelation::Calls,
        };
        graph.add_edge(edge);

        assert_eq!(graph.get_outgoing_edges(id1).len(), 1);
        assert_eq!(graph.get_incoming_edges(id2).len(), 1);
    }

    #[test]
    fn test_from_source_python() {
        let source = br#"
import os
from pathlib import Path

def hello():
    return "Hello"

def world():
    return "World"

def greet():
    return hello() + " " + world()
"#;
        let graph = DependencyGraph::from_source(source, SourceLanguage::Python);
        assert!(graph.is_ok(), "Failed to parse: {:?}", graph.err());

        let graph = graph.unwrap();
        // Should have function nodes
        let funcs = graph.get_nodes_by_type(KgNodeType::Function);
        assert!(!funcs.is_empty(), "Expected function nodes");

        // Should have edges (contains edges at minimum for file -> functions)
        let all_edges = graph.all_edges();
        assert!(!all_edges.is_empty(), "Expected some edges in graph");
    }

    #[test]
    fn test_from_source_rust() {
        let source = br#"
use std::collections::HashMap;

fn add(a: i32, b: i32) -> i32 {
    a + b
}

fn multiply(a: i32, b: i32) -> i32 {
    a * b
}

fn calculate() {
    let _ = add(1, 2);
    let _ = multiply(3, 4);
}
"#;
        let graph = DependencyGraph::from_source(source, SourceLanguage::Rust);
        assert!(graph.is_ok(), "Failed to parse: {:?}", graph.err());

        let graph = graph.unwrap();
        let funcs = graph.get_nodes_by_type(KgNodeType::Function);
        assert!(!funcs.is_empty(), "Expected function nodes");
    }

    #[test]
    fn test_impact_analysis() {
        let mut graph = DependencyGraph::new();

        // Create: main -> helper -> leaf
        let main = graph.add_node(KgNode {
            id: Uuid::new_v4(),
            node_type: KgNodeType::Function,
            name: "main".to_string(),
            properties: serde_json::json!({}),
        });
        let helper = graph.add_node(KgNode {
            id: Uuid::new_v4(),
            node_type: KgNodeType::Function,
            name: "helper".to_string(),
            properties: serde_json::json!({}),
        });
        let leaf = graph.add_node(KgNode {
            id: Uuid::new_v4(),
            node_type: KgNodeType::Function,
            name: "leaf".to_string(),
            properties: serde_json::json!({}),
        });

        // main -> helper
        graph.add_edge(KgEdge {
            id: Uuid::new_v4(),
            source: main,
            target: helper,
            relation: KgRelation::Calls,
        });
        // helper -> leaf
        graph.add_edge(KgEdge {
            id: Uuid::new_v4(),
            source: helper,
            target: leaf,
            relation: KgRelation::Calls,
        });

        // Impact on leaf: main and helper are impacted
        let impact = graph.impact_analysis(leaf);
        assert!(impact.impacted_nodes.contains(&main));
        assert!(impact.impacted_nodes.contains(&helper));
        assert_eq!(impact.direct_dependents, 1); // helper directly calls leaf
        assert!(impact.total_impacted >= 2); // main and helper
    }

    #[test]
    fn test_traverse() {
        let mut graph = DependencyGraph::new();

        let a = graph.add_node(KgNode {
            id: Uuid::new_v4(),
            node_type: KgNodeType::Function,
            name: "a".to_string(),
            properties: serde_json::json!({}),
        });
        let b = graph.add_node(KgNode {
            id: Uuid::new_v4(),
            node_type: KgNodeType::Function,
            name: "b".to_string(),
            properties: serde_json::json!({}),
        });
        let c = graph.add_node(KgNode {
            id: Uuid::new_v4(),
            node_type: KgNodeType::Function,
            name: "c".to_string(),
            properties: serde_json::json!({}),
        });

        graph.add_edge(KgEdge {
            id: Uuid::new_v4(),
            source: a,
            target: b,
            relation: KgRelation::Calls,
        });
        graph.add_edge(KgEdge {
            id: Uuid::new_v4(),
            source: b,
            target: c,
            relation: KgRelation::Calls,
        });

        let traversal = KgTraversal {
            start_node: a,
            relation: Some(KgRelation::Calls),
            max_depth: 10,
            direction: KgDirection::Outgoing,
        };

        let paths = graph.traverse(&traversal);
        assert!(!paths.is_empty());
    }

    #[test]
    fn test_graph_stats() {
        let mut graph = DependencyGraph::new();
        let func = graph.add_node(KgNode {
            id: Uuid::new_v4(),
            node_type: KgNodeType::Function,
            name: "test".to_string(),
            properties: serde_json::json!({}),
        });
        let class = graph.add_node(KgNode {
            id: Uuid::new_v4(),
            node_type: KgNodeType::Class,
            name: "TestClass".to_string(),
            properties: serde_json::json!({}),
        });

        graph.add_edge(KgEdge {
            id: Uuid::new_v4(),
            source: class,
            target: func,
            relation: KgRelation::Contains,
        });

        let stats = graph.stats();
        assert_eq!(stats.total_nodes, 2);
        assert_eq!(stats.total_edges, 1);
        assert_eq!(
            *stats.node_counts.get(&KgNodeType::Function).unwrap_or(&0),
            1
        );
        assert_eq!(*stats.node_counts.get(&KgNodeType::Class).unwrap_or(&0), 1);
    }

    #[test]
    fn test_kg_node_type_and_relation_serialization() {
        // Test that KgNodeType serializes correctly
        let node_type = KgNodeType::Variable;
        let json = serde_json::to_string(&node_type).unwrap();
        assert_eq!(json, "\"variable\"");

        // Test that KgRelation serializes correctly
        let relation = KgRelation::Tests;
        let json = serde_json::to_string(&relation).unwrap();
        assert_eq!(json, "\"tests\"");
    }

    #[test]
    fn test_query_by_type() {
        let mut graph = DependencyGraph::new();

        for i in 0..3 {
            graph.add_node(KgNode {
                id: Uuid::new_v4(),
                node_type: KgNodeType::Function,
                name: format!("func_{}", i),
                properties: serde_json::json!({}),
            });
        }
        graph.add_node(KgNode {
            id: Uuid::new_v4(),
            node_type: KgNodeType::Class,
            name: "MyClass".to_string(),
            properties: serde_json::json!({}),
        });

        let funcs = graph.query_by_type(KgNodeType::Function);
        assert_eq!(funcs.len(), 3);

        let classes = graph.query_by_type(KgNodeType::Class);
        assert_eq!(classes.len(), 1);
    }

    #[test]
    fn test_merge_graphs() {
        let mut graph1 = DependencyGraph::new();
        graph1.add_node(KgNode {
            id: Uuid::new_v4(),
            node_type: KgNodeType::Function,
            name: "func_a".to_string(),
            properties: serde_json::json!({}),
        });

        let mut graph2 = DependencyGraph::new();
        graph2.add_node(KgNode {
            id: Uuid::new_v4(),
            node_type: KgNodeType::Function,
            name: "func_b".to_string(),
            properties: serde_json::json!({}),
        });

        graph1.merge(&graph2);

        let funcs = graph1.get_nodes_by_type(KgNodeType::Function);
        assert_eq!(funcs.len(), 2);
    }

    #[test]
    fn test_impact_result_acceptable() {
        let result = ImpactResult {
            impacted_nodes: vec![],
            impacted_files: vec![],
            test_nodes: vec![],
            direct_dependents: 0,
            total_impacted: 5,
        };

        assert!(result.is_acceptable(10));
        assert!(!result.is_acceptable(3));
    }

    #[test]
    fn test_impact_result_has_test_impact() {
        let node_id = Uuid::new_v4();
        let result = ImpactResult {
            impacted_nodes: vec![node_id],
            impacted_files: vec![],
            test_nodes: vec![node_id],
            direct_dependents: 1,
            total_impacted: 1,
        };

        assert!(result.has_test_impact());
    }
}
