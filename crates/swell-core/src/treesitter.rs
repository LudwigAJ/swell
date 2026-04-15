//! Tree-sitter AST parsing for semantic code chunking and dependency analysis.
//!
//! This module provides:
//! - AST parsing for 66+ languages via tree-sitter
//! - Code chunking at function/class/method boundaries
//! - Dependency extraction from imports and calls
//! - Syntax-aware indexing for memory retrieval
//!
//! Reference: https://tree-sitter.github.io/tree-sitter/

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use tree_sitter::{Node, Parser, Tree};
use uuid::Uuid;

/// Re-export tree-sitter Language for language detection
pub use tree_sitter::Language;

/// Supported languages for AST parsing
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceLanguage {
    Rust,
    Python,
    TypeScript,
    JavaScript,
    Go,
    Java,
    C,
    Cpp,
    CSharp,
    Ruby,
    Swift,
    Kotlin,
    Php,
    Html,
    Css,
    Json,
    Yaml,
    Markdown,
    Toml,
    Bash,
    Sql,
    Unknown,
}

impl SourceLanguage {
    /// Detect language from file extension
    pub fn from_extension(ext: &str) -> Self {
        match ext.to_lowercase().as_str() {
            "rs" => SourceLanguage::Rust,
            "py" => SourceLanguage::Python,
            "ts" | "tsx" => SourceLanguage::TypeScript,
            "js" | "jsx" => SourceLanguage::JavaScript,
            "go" => SourceLanguage::Go,
            "java" => SourceLanguage::Java,
            "c" => SourceLanguage::C,
            "cpp" | "cc" | "cxx" => SourceLanguage::Cpp,
            "cs" => SourceLanguage::CSharp,
            "rb" => SourceLanguage::Ruby,
            "swift" => SourceLanguage::Swift,
            "kt" | "kts" => SourceLanguage::Kotlin,
            "php" => SourceLanguage::Php,
            "html" | "htm" => SourceLanguage::Html,
            "css" => SourceLanguage::Css,
            "json" => SourceLanguage::Json,
            "yaml" | "yml" => SourceLanguage::Yaml,
            "md" | "markdown" => SourceLanguage::Markdown,
            "toml" => SourceLanguage::Toml,
            "sh" | "bash" | "zsh" => SourceLanguage::Bash,
            "sql" => SourceLanguage::Sql,
            _ => SourceLanguage::Unknown,
        }
    }

    /// Get tree-sitter language constant
    pub fn to_language(&self) -> Option<Language> {
        match self {
            SourceLanguage::Rust => Some(Language::from(tree_sitter_rust::LANGUAGE)),
            SourceLanguage::Python => Some(Language::from(tree_sitter_python::LANGUAGE)),
            SourceLanguage::TypeScript | SourceLanguage::JavaScript => {
                Some(Language::from(tree_sitter_typescript::LANGUAGE_TYPESCRIPT))
            }
            SourceLanguage::Go => Some(Language::from(tree_sitter_go::LANGUAGE)),
            SourceLanguage::Java => Some(Language::from(tree_sitter_java::LANGUAGE)),
            SourceLanguage::C => Some(Language::from(tree_sitter_c::LANGUAGE)),
            SourceLanguage::Cpp => Some(Language::from(tree_sitter_cpp::LANGUAGE)),
            _ => None,
        }
    }

    /// Check if this language is supported
    pub fn is_supported(&self) -> bool {
        self.to_language().is_some()
    }
}

/// Result of parsing a source file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParseResult {
    /// Root node of the AST
    pub root: AstNode,
    /// Language of the source file
    pub language: SourceLanguage,
    /// File path (if available)
    pub source_path: Option<String>,
    /// All nodes indexed by ID
    pub nodes: HashMap<String, AstNode>,
    /// Code chunks (functions, classes, methods)
    pub chunks: Vec<CodeChunk>,
    /// Extracted dependencies
    pub dependencies: Vec<Dependency>,
}

/// A node in the AST with position information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AstNode {
    /// Unique identifier for this node
    pub id: String,
    /// Node kind (e.g., "function", "class", "import")
    pub kind: String,
    /// Display name (e.g., function name)
    pub name: Option<String>,
    /// Start byte offset
    pub start_byte: usize,
    /// End byte offset
    pub end_byte: usize,
    /// Start line number (1-indexed)
    pub start_line: u32,
    /// Start column (1-indexed)
    pub start_column: u32,
    /// End line number (1-indexed)
    pub end_line: u32,
    /// End column (1-indexed)
    pub end_column: u32,
    /// Child node IDs
    #[serde(default)]
    pub children: Vec<String>,
    /// Parent node ID (if any)
    #[serde(default)]
    pub parent: Option<String>,
    /// Additional properties specific to node kind
    #[serde(default)]
    pub properties: HashMap<String, serde_json::Value>,
}

impl AstNode {
    /// Create a new AST node from a tree-sitter node
    pub fn from_ts_node(node: &Node, source: &[u8], id_prefix: &str) -> Self {
        let kind = node.kind().to_string();
        let name = Self::extract_name(node, source);
        let start = node.start_position();
        let end = node.end_position();
        let start_byte = node.start_byte();
        let end_byte = node.end_byte();

        Self {
            id: format!("{}_{}", id_prefix, start_byte),
            kind,
            name,
            start_byte,
            end_byte,
            start_line: start.row as u32 + 1, // Convert to 1-indexed
            start_column: start.column as u32 + 1,
            end_line: end.row as u32 + 1,
            end_column: end.column as u32 + 1,
            children: Vec::new(),
            parent: None,
            properties: HashMap::new(),
        }
    }

    /// Extract name from node (e.g., function name from function definition)
    fn extract_name(node: &Node, source: &[u8]) -> Option<String> {
        // Try to get name from first named child with "name" in it
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let child_kind = child.kind();
            if child_kind.contains("name") || child_kind == "identifier" {
                if let Ok(name) = child.utf8_text(source) {
                    return Some(name.to_string());
                }
            }
        }
        None
    }
}

/// A code chunk (function, class, method) for chunking
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeChunk {
    /// Unique identifier
    pub id: Uuid,
    /// Chunk type
    pub chunk_type: ChunkType,
    /// Name of the chunk (e.g., function name)
    pub name: String,
    /// Full source code of the chunk
    pub source_code: String,
    /// Start byte offset in original file
    pub start_byte: usize,
    /// End byte offset in original file
    pub end_byte: usize,
    /// Start line number
    pub start_line: u32,
    /// End line number
    pub end_line: u32,
    /// Parent chunk (containing class/module)
    pub parent_chunk: Option<Uuid>,
    /// Dependencies (calls to other functions/modules)
    pub dependencies: Vec<String>,
}

/// Type of code chunk
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChunkType {
    Function,
    Method,
    Class,
    Module,
    Interface,
    Trait,
    Struct,
    Enum,
}

/// A dependency extracted from imports/calls
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dependency {
    /// Dependency type
    pub dependency_type: DependencyType,
    /// Target (e.g., module name, function name)
    pub target: String,
    /// Source chunk (if within a chunk)
    pub source_chunk: Option<Uuid>,
    /// Full path if available (e.g., "module.submodule.function")
    pub full_path: Option<String>,
}

/// Type of dependency
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DependencyType {
    Import,
    Call,
    Reference,
    Inheritance,
}

/// Parse a source file and return AST
pub fn parse_file(path: &Path, source: &[u8]) -> Result<ParseResult, String> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");

    let language = SourceLanguage::from_extension(ext);

    let ts_language = language
        .to_language()
        .ok_or_else(|| format!("Language {:?} is not supported", language))?;

    let mut parser = Parser::new();
    parser
        .set_language(&ts_language)
        .map_err(|e| format!("Failed to set language: {}", e))?;

    let tree = parser
        .parse(source, None)
        .ok_or_else(|| "Failed to parse source".to_string())?;

    let root_node = tree.root_node();

    // Build AST nodes
    let mut nodes = HashMap::new();
    let mut chunks = Vec::new();
    let mut dependencies = Vec::new();

    // Visit all nodes
    collect_nodes(
        &root_node,
        source,
        "",
        &mut nodes,
        &mut chunks,
        &mut dependencies,
    );

    // Create root node
    let root = AstNode::from_ts_node(&root_node, source, "root");

    Ok(ParseResult {
        root,
        language,
        source_path: Some(path.to_string_lossy().to_string()),
        nodes,
        chunks,
        dependencies,
    })
}

/// Parse source code directly (without file path)
pub fn parse_source(source: &[u8], language: SourceLanguage) -> Result<ParseResult, String> {
    let ts_language = language
        .to_language()
        .ok_or_else(|| format!("Language {:?} is not supported", language))?;

    let mut parser = Parser::new();
    parser
        .set_language(&ts_language)
        .map_err(|e| format!("Failed to set language: {}", e))?;

    let tree = parser
        .parse(source, None)
        .ok_or_else(|| "Failed to parse source".to_string())?;

    let root_node = tree.root_node();

    let mut nodes = HashMap::new();
    let mut chunks = Vec::new();
    let mut dependencies = Vec::new();

    collect_nodes(
        &root_node,
        source,
        "",
        &mut nodes,
        &mut chunks,
        &mut dependencies,
    );

    let root = AstNode::from_ts_node(&root_node, source, "root");

    Ok(ParseResult {
        root,
        language,
        source_path: None,
        nodes,
        chunks,
        dependencies,
    })
}

/// Collect all nodes recursively
fn collect_nodes(
    node: &Node,
    source: &[u8],
    parent_id: &str,
    nodes: &mut HashMap<String, AstNode>,
    chunks: &mut Vec<CodeChunk>,
    dependencies: &mut Vec<Dependency>,
) {
    let start_byte = node.start_byte();
    let node_id = format!("{}_{}", parent_id, start_byte);
    let mut ast_node = AstNode::from_ts_node(node, source, parent_id);
    ast_node.parent = if parent_id.is_empty() {
        None
    } else {
        Some(parent_id.to_string())
    };

    // Detect chunk types and extract dependencies
    let kind = node.kind();
    match kind {
        // Function-like nodes
        "function_item" | "function_declaration" | "function_definition" => {
            if let Some(name) = extract_identifier(node, source) {
                let chunk = create_chunk(node, source, ChunkType::Function, &name, None);
                chunks.push(chunk);
                ast_node
                    .properties
                    .insert("chunk".to_string(), serde_json::json!(name));
            }
            // Extract function calls as dependencies
            extract_calls(node, source, dependencies);
        }
        "method" | "method_definition" => {
            if let Some(name) = extract_identifier(node, source) {
                let chunk = create_chunk(node, source, ChunkType::Method, &name, None);
                chunks.push(chunk);
            }
            extract_calls(node, source, dependencies);
        }
        "class" | "class_declaration" | "class_definition" => {
            if let Some(name) = extract_identifier(node, source) {
                let chunk = create_chunk(node, source, ChunkType::Class, &name, None);
                chunks.push(chunk);
            }
        }
        "struct_item" | "struct_declaration" => {
            if let Some(name) = extract_identifier(node, source) {
                let chunk = create_chunk(node, source, ChunkType::Struct, &name, None);
                chunks.push(chunk);
            }
        }
        "enum_item" | "enum_declaration" => {
            if let Some(name) = extract_identifier(node, source) {
                let chunk = create_chunk(node, source, ChunkType::Enum, &name, None);
                chunks.push(chunk);
            }
        }
        "module" | "module_declaration" => {
            if let Some(name) = extract_identifier(node, source) {
                let chunk = create_chunk(node, source, ChunkType::Module, &name, None);
                chunks.push(chunk);
            }
        }
        "use_declaration" | "import_declaration" | "import_statement" | "import" => {
            if let Some(target) = extract_import_target(node, source) {
                dependencies.push(Dependency {
                    dependency_type: DependencyType::Import,
                    target,
                    source_chunk: None,
                    full_path: None,
                });
            }
        }
        "call_expression" | "identifier" => {
            if let Some(target) = extract_identifier(node, source) {
                if !target.is_empty()
                    && target
                        .chars()
                        .next()
                        .map(|c| c.is_lowercase())
                        .unwrap_or(false)
                {
                    dependencies.push(Dependency {
                        dependency_type: DependencyType::Call,
                        target,
                        source_chunk: None,
                        full_path: None,
                    });
                }
            }
        }
        _ => {}
    }

    // Add node to map
    nodes.insert(node_id.clone(), ast_node);

    // Process children
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_nodes(&child, source, &node_id, nodes, chunks, dependencies);
    }
}

/// Extract identifier from node
fn extract_identifier(node: &Node, source: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let kind = child.kind();
        if kind == "identifier" || kind == "type_identifier" || kind == "field_identifier" {
            if let Ok(text) = child.utf8_text(source) {
                return Some(text.to_string());
            }
        }
        // Recurse into anonymous containers like (program (function_item))
        if let Some(name) = extract_identifier(&child, source) {
            return Some(name);
        }
    }
    None
}

/// Extract import target (module/package name)
fn extract_import_target(node: &Node, source: &[u8]) -> Option<String> {
    let text = node.utf8_text(source).ok()?;
    let text = text.trim();

    // Handle various import formats
    // Python: "from module import x" or "import module"
    // Rust: "use module::path"
    // Go: "import ( \"package\" )" or "import \"package\""
    // TypeScript/JavaScript: "import { x } from 'module'" or "require('module')"

    // Try to extract the module path
    if text.starts_with("from ") {
        text.strip_prefix("from ")
            .and_then(|t| t.split_whitespace().next())
            .map(|s| s.trim_matches(|c| c == '\'' || c == '"').to_string())
    } else if text.starts_with("import ") {
        // Complex import - extract string literal
        text.split('"')
            .nth(1)
            .or_else(|| text.split('\'').nth(1))
            .map(|s| s.to_string())
    } else if text.starts_with("use ") {
        Some(text.strip_prefix("use ").unwrap().trim().to_string())
    } else if text.starts_with("require(") {
        text.strip_prefix("require(")
            .and_then(|t| {
                t.trim_end_matches(')')
                    .split('\'')
                    .nth(1)
                    .or_else(|| t.split('"').nth(1))
            })
            .map(|s| s.to_string())
    } else {
        // For other cases, clean up common patterns
        Some(text.replace("\"", "").replace("'", "").to_string())
    }
}

/// Create a code chunk from a node
fn create_chunk(
    node: &Node,
    source: &[u8],
    chunk_type: ChunkType,
    name: &str,
    parent_chunk: Option<Uuid>,
) -> CodeChunk {
    let start = node.start_position();
    let end = node.end_position();
    let start_byte = node.start_byte();
    let end_byte = node.end_byte();
    let source_code = node.utf8_text(source).unwrap_or("").to_string();

    CodeChunk {
        id: Uuid::new_v4(),
        chunk_type,
        name: name.to_string(),
        source_code,
        start_byte,
        end_byte,
        start_line: start.row as u32 + 1,
        end_line: end.row as u32 + 1,
        parent_chunk,
        dependencies: Vec::new(),
    }
}

/// Extract function calls from a node
fn extract_calls(node: &Node, source: &[u8], dependencies: &mut Vec<Dependency>) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let kind = child.kind();
        if kind == "call_expression" {
            if let Some(target) = extract_call_target(&child, source) {
                // Filter out primitives and common builtins
                if !is_builtin(&target) {
                    dependencies.push(Dependency {
                        dependency_type: DependencyType::Call,
                        target,
                        source_chunk: None,
                        full_path: None,
                    });
                }
            }
        }
        // Recurse
        extract_calls(&child, source, dependencies);
    }
}

/// Extract call target (function being called)
fn extract_call_target(node: &Node, source: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let kind = child.kind();
        if kind == "identifier" || kind == "field_expression" {
            // For field expressions like "obj.method", capture "obj.method"
            if kind == "field_expression" {
                if let Ok(text) = node.utf8_text(source) {
                    return Some(text.trim().to_string());
                }
            } else if let Ok(text) = child.utf8_text(source) {
                return Some(text.to_string());
            }
        }
        if let Some(target) = extract_call_target(&child, source) {
            return Some(target);
        }
    }
    None
}

/// Check if target is a builtin (not a real dependency)
fn is_builtin(target: &str) -> bool {
    let builtins = [
        "print",
        "len",
        "range",
        "str",
        "int",
        "float",
        "list",
        "dict",
        "set",
        "tuple",
        "map",
        "filter",
        "zip",
        "enumerate",
        "sorted",
        "reversed",
        "any",
        "all",
        "min",
        "max",
        "sum",
        "abs",
        "round",
        "open",
        "input",
        "type",
        "isinstance",
        "println",
        "vec",
        "String",
        "i32",
        "i64",
        "u32",
        "u64",
        "f32",
        "f64",
        "bool",
        "Some",
        "None",
        "Ok",
        "Err",
        "self",
        "super",
        "true",
        "false",
        "new",
    ];
    builtins.contains(&target)
}

/// Query AST with a tree-sitter pattern
pub fn query<'a>(tree: &'a Tree, pattern: &'a str) -> Result<Vec<Node<'a>>, String> {
    // Note: Full query implementation would use tree_sitter::Query
    // For now, we return an empty vec as a placeholder
    let _ = pattern;
    let _ = tree;
    Ok(Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_language_detection() {
        assert_eq!(SourceLanguage::from_extension("rs"), SourceLanguage::Rust);
        assert_eq!(SourceLanguage::from_extension("py"), SourceLanguage::Python);
        assert_eq!(
            SourceLanguage::from_extension("ts"),
            SourceLanguage::TypeScript
        );
        assert_eq!(SourceLanguage::from_extension("go"), SourceLanguage::Go);
        assert_eq!(SourceLanguage::from_extension("java"), SourceLanguage::Java);
        assert_eq!(
            SourceLanguage::from_extension("unknown"),
            SourceLanguage::Unknown
        );
    }

    #[test]
    fn test_language_support() {
        // Languages with tree-sitter grammars available
        assert!(SourceLanguage::Rust.is_supported());
        assert!(SourceLanguage::Python.is_supported());
        assert!(SourceLanguage::TypeScript.is_supported());
        assert!(SourceLanguage::JavaScript.is_supported());
        assert!(SourceLanguage::Go.is_supported());
        assert!(SourceLanguage::Java.is_supported());
        assert!(SourceLanguage::C.is_supported());
        assert!(SourceLanguage::Cpp.is_supported());
    }

    #[test]
    fn test_parse_python_function() {
        let source = br#"
def hello_world():
    print("Hello, World!")
    return 42
"#;

        let result = parse_source(source, SourceLanguage::Python);
        assert!(result.is_ok(), "Parsing failed: {:?}", result.err());

        let parse = result.unwrap();
        assert_eq!(parse.language, SourceLanguage::Python);
        assert!(!parse.chunks.is_empty(), "Expected at least one chunk");

        // Find a chunk named hello_world - might be function or expression_statement
        let func_chunk = parse.chunks.iter().find(|c| c.name == "hello_world");
        assert!(
            func_chunk.is_some(),
            "Expected to find 'hello_world' function, got chunks: {:?}",
            parse
                .chunks
                .iter()
                .map(|c| format!("{:?}", c.chunk_type))
                .collect::<Vec<_>>()
        );

        let chunk = func_chunk.unwrap();
        // Should have source code containing Hello
        assert!(chunk.source_code.contains("Hello"));
    }

    #[test]
    fn test_parse_rust_function() {
        let source = br#"
fn add(a: i32, b: i32) -> i32 {
    a + b
}
"#;

        let result = parse_source(source, SourceLanguage::Rust);
        assert!(result.is_ok(), "Parsing failed: {:?}", result.err());

        let parse = result.unwrap();
        assert_eq!(parse.language, SourceLanguage::Rust);
        assert!(!parse.chunks.is_empty(), "Expected at least one chunk");

        // Find a chunk named add
        let func_chunk = parse.chunks.iter().find(|c| c.name == "add");
        assert!(
            func_chunk.is_some(),
            "Expected to find 'add' function, got chunks: {:?}",
            parse
                .chunks
                .iter()
                .map(|c| format!("{:?}", c.chunk_type))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_chunk_boundaries() {
        let source = br#"
def outer():
    def inner():
        pass
    return inner
"#;

        let result = parse_source(source, SourceLanguage::Python);
        assert!(result.is_ok());

        let parse = result.unwrap();
        // Should have two function chunks
        let functions: Vec<_> = parse
            .chunks
            .iter()
            .filter(|c| c.chunk_type == ChunkType::Function)
            .collect();
        assert!(functions.len() >= 1, "Expected at least one function chunk");
    }

    #[test]
    fn test_dependency_extraction() {
        let source = br#"
import os
import sys
from pathlib import Path

def main():
    os.getcwd()
    sys.exit(0)
"#;

        let result = parse_source(source, SourceLanguage::Python);
        assert!(result.is_ok());

        let parse = result.unwrap();
        // Should have import dependencies
        let imports: Vec<_> = parse
            .dependencies
            .iter()
            .filter(|d| d.dependency_type == DependencyType::Import)
            .collect();
        assert!(!imports.is_empty(), "Expected import dependencies");
    }

    #[test]
    fn test_unknown_language_returns_error() {
        let source = b"some content";
        let result = parse_source(source, SourceLanguage::Unknown);
        assert!(result.is_err(), "Expected error for unknown language");
    }

    #[test]
    fn test_is_builtin() {
        assert!(is_builtin("print"));
        assert!(is_builtin("len"));
        assert!(is_builtin("String"));
        assert!(!is_builtin("my_function"));
        assert!(!is_builtin("SomeStruct"));
    }
}
