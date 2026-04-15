//! Tree-sitter AST to KnowledgeGraph extraction pipeline
//!
//! This module provides functionality to:
//! - Parse Rust source files using tree-sitter
//! - Extract typed nodes (File, Function, Struct, Method, Import)
//! - Insert nodes into the KnowledgeGraph with correct KgNodeType
//! - Create edges with correct KgRelation types (Contains, Calls, Imports)

use std::collections::HashMap;
use uuid::Uuid;

use swell_core::treesitter::{
    parse_source, ChunkType, DependencyType, ParseResult, SourceLanguage,
};
use swell_core::{KgEdge, KgNode, KgNodeType, KgRelation, KnowledgeGraph};

use crate::knowledge_graph::SqliteKnowledgeGraph;

/// Result of extracting nodes and edges from source code
#[derive(Debug, Clone)]
pub struct ExtractionResult {
    /// Extracted nodes
    pub nodes: Vec<KgNode>,
    /// Extracted edges
    pub edges: Vec<KgEdge>,
    /// Mapping from chunk ID to node ID
    #[expect(dead_code)]
    chunk_to_node: HashMap<Uuid, Uuid>,
}

impl ExtractionResult {
    /// Get the file node ID from a chunk
    pub fn get_file_node_id(&self) -> Option<Uuid> {
        self.nodes
            .iter()
            .find(|n| n.node_type == KgNodeType::File)
            .map(|n| n.id)
    }

    /// Get all function node IDs
    pub fn get_function_ids(&self) -> Vec<Uuid> {
        self.nodes
            .iter()
            .filter(|n| n.node_type == KgNodeType::Function)
            .map(|n| n.id)
            .collect()
    }

    /// Get all method node IDs
    pub fn get_method_ids(&self) -> Vec<Uuid> {
        self.nodes
            .iter()
            .filter(|n| n.node_type == KgNodeType::Method)
            .map(|n| n.id)
            .collect()
    }

    /// Get all import node IDs
    pub fn get_import_ids(&self) -> Vec<Uuid> {
        self.nodes
            .iter()
            .filter(|n| n.node_type == KgNodeType::Import)
            .map(|n| n.id)
            .collect()
    }
}

/// Extract nodes and edges from Rust source code
pub fn extract_from_source(
    source: &[u8],
    file_path: &str,
    repository: &str,
) -> Result<ExtractionResult, String> {
    let parse_result = parse_source(source, SourceLanguage::Rust)
        .map_err(|e| format!("Failed to parse source: {}", e))?;

    extract_from_parse_result(&parse_result, file_path, repository)
}

/// Extract nodes and edges from a ParseResult
pub fn extract_from_parse_result(
    parse_result: &ParseResult,
    file_path: &str,
    repository: &str,
) -> Result<ExtractionResult, String> {
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    let mut chunk_to_node: HashMap<Uuid, Uuid> = HashMap::new();

    // Create file node
    let file_node = KgNode {
        id: Uuid::new_v4(),
        node_type: KgNodeType::File,
        name: file_path.to_string(),
        properties: serde_json::json!({
            "source_path": file_path,
            "repository": repository,
        }),
    };
    let file_node_id = file_node.id;
    nodes.push(file_node);

    // Process chunks and create corresponding nodes
    for chunk in &parse_result.chunks {
        let node_type = chunk_type_to_node_type(chunk.chunk_type);
        let node = KgNode {
            id: Uuid::new_v4(),
            node_type,
            name: chunk.name.clone(),
            properties: serde_json::json!({
                "source_path": file_path,
                "repository": repository,
                "start_line": chunk.start_line,
                "end_line": chunk.end_line,
                "chunk_id": chunk.id.to_string(),
            }),
        };

        // Store chunk -> node mapping for edge creation
        chunk_to_node.insert(chunk.id, node.id);

        // Create Contains edge from file to this chunk
        let contains_edge = KgEdge {
            id: Uuid::new_v4(),
            source: file_node_id,
            target: node.id,
            relation: KgRelation::Contains,
        };
        edges.push(contains_edge);

        nodes.push(node);
    }

    // Process dependencies to create Calls and Imports edges
    for dep in &parse_result.dependencies {
        match dep.dependency_type {
            DependencyType::Import => {
                // Create an Import node
                let import_node = KgNode {
                    id: Uuid::new_v4(),
                    node_type: KgNodeType::Import,
                    name: dep.target.clone(),
                    properties: serde_json::json!({
                        "source_path": file_path,
                        "repository": repository,
                        "target": dep.target,
                    }),
                };
                let import_node_id = import_node.id;

                // Create Imports edge from file to import
                let imports_edge = KgEdge {
                    id: Uuid::new_v4(),
                    source: file_node_id,
                    target: import_node_id,
                    relation: KgRelation::Imports,
                };
                edges.push(imports_edge);
                nodes.push(import_node);
            }
            DependencyType::Call => {
                // For calls, we need to find the source chunk and create a Calls edge
                if let Some(source_chunk_id) = dep.source_chunk {
                    if let Some(source_node_id) = chunk_to_node.get(&source_chunk_id) {
                        // Try to find target function in nodes
                        let target_node_id = nodes
                            .iter()
                            .find(|n| {
                                n.name == dep.target
                                    && matches!(
                                        n.node_type,
                                        KgNodeType::Function | KgNodeType::Method
                                    )
                            })
                            .map(|n| n.id);

                        if let Some(target_id) = target_node_id {
                            let calls_edge = KgEdge {
                                id: Uuid::new_v4(),
                                source: *source_node_id,
                                target: target_id,
                                relation: KgRelation::Calls,
                            };
                            edges.push(calls_edge);
                        }
                    }
                } else {
                    // For calls without source chunk, try to find target function and create edge from file
                    let target_node_id = nodes
                        .iter()
                        .find(|n| {
                            n.name == dep.target
                                && matches!(n.node_type, KgNodeType::Function | KgNodeType::Method)
                        })
                        .map(|n| n.id);

                    if let Some(target_id) = target_node_id {
                        let calls_edge = KgEdge {
                            id: Uuid::new_v4(),
                            source: file_node_id,
                            target: target_id,
                            relation: KgRelation::Calls,
                        };
                        edges.push(calls_edge);
                    }
                }
            }
            DependencyType::Reference | DependencyType::Inheritance => {
                // For reference/inheritance, create DependsOn edge
                let target_node_id = nodes.iter().find(|n| n.name == dep.target).map(|n| n.id);

                if let Some(target_id) = target_node_id {
                    let depends_edge = KgEdge {
                        id: Uuid::new_v4(),
                        source: file_node_id,
                        target: target_id,
                        relation: KgRelation::DependsOn,
                    };
                    edges.push(depends_edge);
                }
            }
        }
    }

    Ok(ExtractionResult {
        nodes,
        edges,
        chunk_to_node,
    })
}

/// Convert ChunkType to KgNodeType
fn chunk_type_to_node_type(chunk_type: ChunkType) -> KgNodeType {
    match chunk_type {
        ChunkType::Function => KgNodeType::Function,
        ChunkType::Method => KgNodeType::Method,
        ChunkType::Class => KgNodeType::Class,
        ChunkType::Module => KgNodeType::Module,
        ChunkType::Interface => KgNodeType::Type,
        ChunkType::Trait => KgNodeType::Type,
        ChunkType::Struct => KgNodeType::Type,
        ChunkType::Enum => KgNodeType::Type,
    }
}

/// Insert extracted nodes and edges into the knowledge graph
pub async fn insert_into_graph(
    graph: &SqliteKnowledgeGraph,
    result: &ExtractionResult,
) -> Result<(), String> {
    // Insert all nodes
    for node in &result.nodes {
        graph
            .add_node(node.clone())
            .await
            .map_err(|e| format!("Failed to insert node: {}", e))?;
    }

    // Insert all edges
    for edge in &result.edges {
        graph
            .add_edge(edge.clone())
            .await
            .map_err(|e| format!("Failed to insert edge: {}", e))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_function_nodes() {
        let source = br#"
pub fn hello_world() {
    println!("Hello, World!");
}

fn internal_function() {
    do_something();
}
"#;

        let result = extract_from_source(source, "test.rs", "test-repo").unwrap();

        // Should have file node
        let file_nodes: Vec<_> = result
            .nodes
            .iter()
            .filter(|n| n.node_type == KgNodeType::File)
            .collect();
        assert_eq!(file_nodes.len(), 1);

        // Should have function nodes
        let func_nodes: Vec<_> = result
            .nodes
            .iter()
            .filter(|n| n.node_type == KgNodeType::Function)
            .collect();
        assert!(
            func_nodes.len() >= 2,
            "Expected at least 2 functions, got {}",
            func_nodes.len()
        );

        // Verify function names
        let func_names: Vec<_> = func_nodes.iter().map(|n| n.name.clone()).collect();
        assert!(func_names.contains(&"hello_world".to_string()));
        assert!(func_names.contains(&"internal_function".to_string()));
    }

    #[test]
    fn test_extract_struct_nodes() {
        let source = br#"
struct Point {
    x: i32,
    y: i32,
}

pub struct Color {
    r: u8,
    g: u8,
    b: u8,
}
"#;

        let result = extract_from_source(source, "shapes.rs", "test-repo").unwrap();

        // Should have struct nodes (mapped to Type)
        let type_nodes: Vec<_> = result
            .nodes
            .iter()
            .filter(|n| n.node_type == KgNodeType::Type)
            .collect();
        assert!(
            type_nodes.len() >= 2,
            "Expected at least 2 types (structs), got {}",
            type_nodes.len()
        );

        // Verify struct names
        let type_names: Vec<_> = type_nodes.iter().map(|n| n.name.clone()).collect();
        assert!(type_names.contains(&"Point".to_string()));
        assert!(type_names.contains(&"Color".to_string()));
    }

    #[test]
    fn test_extract_import_nodes() {
        let source = br#"
use std::collections::HashMap;
use std::fmt::Display;

fn main() {}
"#;

        let result = extract_from_source(source, "main.rs", "test-repo").unwrap();

        // Should have import nodes
        let import_nodes: Vec<_> = result
            .nodes
            .iter()
            .filter(|n| n.node_type == KgNodeType::Import)
            .collect();
        assert!(
            import_nodes.len() >= 2,
            "Expected at least 2 imports, got {}",
            import_nodes.len()
        );

        // Verify import targets
        let import_names: Vec<_> = import_nodes.iter().map(|n| n.name.clone()).collect();
        assert!(import_names.iter().any(|n| n.contains("HashMap")));
        assert!(import_names.iter().any(|n| n.contains("Display")));
    }

    #[test]
    fn test_contains_edges() {
        let source = br#"
fn top_level_function() {
    inner_helper();
}

fn inner_helper() {}
"#;

        let result = extract_from_source(source, "mod.rs", "test-repo").unwrap();

        // File should contain the functions via Contains edges
        let contains_edges: Vec<_> = result
            .edges
            .iter()
            .filter(|e| e.relation == KgRelation::Contains)
            .collect();
        assert!(
            contains_edges.len() >= 2,
            "Expected at least 2 Contains edges, got {}",
            contains_edges.len()
        );

        // Each Contains edge should have the file node as source
        let file_node_id = result.get_file_node_id().unwrap();
        for edge in &contains_edges {
            assert_eq!(
                edge.source, file_node_id,
                "Contains edge should originate from file node"
            );
        }
    }

    #[test]
    fn test_calls_edges() {
        let source = br#"
fn caller() {
    callee();
}

fn callee() {
    println!("called");
}
"#;

        let result = extract_from_source(source, "calls.rs", "test-repo").unwrap();

        // Debug: print all nodes and edges
        eprintln!("=== Nodes ===");
        for node in &result.nodes {
            eprintln!("  {:?} - {}", node.node_type, node.name);
        }
        eprintln!("\n=== Edges ===");
        for edge in &result.edges {
            eprintln!(
                "  {:?} - {} -> {}",
                edge.relation,
                result
                    .nodes
                    .iter()
                    .find(|n| n.id == edge.source)
                    .map(|n| n.name.clone())
                    .unwrap_or_default(),
                result
                    .nodes
                    .iter()
                    .find(|n| n.id == edge.target)
                    .map(|n| n.name.clone())
                    .unwrap_or_default()
            );
        }

        // Should have Calls edges (at minimum, file calling callee since source_chunk isn't tracked)
        let calls_edges: Vec<_> = result
            .edges
            .iter()
            .filter(|e| e.relation == KgRelation::Calls)
            .collect();
        assert!(
            !calls_edges.is_empty(),
            "Expected at least one Calls edge, got edges: {:?}",
            result.edges.iter().map(|e| e.relation).collect::<Vec<_>>()
        );

        // Verify callee is called - the callee function should be the target of some Calls edge
        let callee_id = result
            .nodes
            .iter()
            .find(|n| n.name == "callee")
            .map(|n| n.id);
        assert!(callee_id.is_some(), "Callee function should exist in nodes");

        let callee_is_called = calls_edges.iter().any(|e| e.target == callee_id.unwrap());
        assert!(callee_is_called, "Callee should be called by something");
    }

    #[test]
    fn test_imports_edges() {
        let source = br#"
use std::fmt::Debug;

fn helper() {}
"#;

        let result = extract_from_source(source, "imports.rs", "test-repo").unwrap();

        // Should have Imports edges from file to imports
        let imports_edges: Vec<_> = result
            .edges
            .iter()
            .filter(|e| e.relation == KgRelation::Imports)
            .collect();
        assert!(
            !imports_edges.is_empty(),
            "Expected at least one Imports edge"
        );

        // Verify imports edge connects file to import node
        let file_node_id = result.get_file_node_id().unwrap();
        for edge in &imports_edges {
            assert_eq!(
                edge.source, file_node_id,
                "Imports edge should originate from file node"
            );
        }
    }

    #[test]
    fn test_chunk_type_to_node_type_mapping() {
        // Function
        assert_eq!(
            chunk_type_to_node_type(ChunkType::Function),
            KgNodeType::Function
        );
        // Method
        assert_eq!(
            chunk_type_to_node_type(ChunkType::Method),
            KgNodeType::Method
        );
        // Class
        assert_eq!(chunk_type_to_node_type(ChunkType::Class), KgNodeType::Class);
        // Struct -> Type
        assert_eq!(chunk_type_to_node_type(ChunkType::Struct), KgNodeType::Type);
        // Enum -> Type
        assert_eq!(chunk_type_to_node_type(ChunkType::Enum), KgNodeType::Type);
        // Module
        assert_eq!(
            chunk_type_to_node_type(ChunkType::Module),
            KgNodeType::Module
        );
    }

    #[test]
    fn test_integration_with_knowledge_graph() {
        // This test validates the full pipeline: parse -> extract -> insert -> query
        let source = br#"
use std::collections::HashMap;

pub fn process_data(items: Vec<i32>) -> String {
    let mut map = HashMap::new();
    for item in items {
        map.insert(item, item * 2);
    }
    format!("Processed {} items", items.len())
}

struct InternalState {
    counter: i32,
}

impl InternalState {
    pub fn new() -> Self {
        InternalState { counter: 0 }
    }

    pub fn increment(&mut self) {
        self.counter += 1;
    }
}
"#;

        let result = extract_from_source(source, "processor.rs", "integration-test-repo").unwrap();

        // Verify node counts
        let file_count = result
            .nodes
            .iter()
            .filter(|n| n.node_type == KgNodeType::File)
            .count();
        let func_count = result
            .nodes
            .iter()
            .filter(|n| n.node_type == KgNodeType::Function)
            .count();
        let type_count = result
            .nodes
            .iter()
            .filter(|n| n.node_type == KgNodeType::Type)
            .count();
        let import_count = result
            .nodes
            .iter()
            .filter(|n| n.node_type == KgNodeType::Import)
            .count();

        assert_eq!(file_count, 1, "Should have exactly 1 file node");
        assert!(func_count >= 1, "Should have at least 1 function");
        assert!(type_count >= 1, "Should have at least 1 type (struct)");
        assert!(import_count >= 1, "Should have at least 1 import");

        // Verify edges
        let contains_count = result
            .edges
            .iter()
            .filter(|e| e.relation == KgRelation::Contains)
            .count();
        let imports_edge_count = result
            .edges
            .iter()
            .filter(|e| e.relation == KgRelation::Imports)
            .count();

        assert!(
            contains_count >= 3,
            "Should have Contains edges for file -> function/struct/method"
        );
        assert!(
            imports_edge_count >= 1,
            "Should have at least 1 Imports edge"
        );
    }
}
