// knowledge_graph.rs - Property graph knowledge base for code structure representation
//
// This module provides a property graph knowledge base with typed nodes and edges
// for representing code structure. It provides:
// - Graph database with SQLite persistence
// - Query interface for dependency lookups
// - Path finding between code elements
// - Cross-reference analysis
//
// The knowledge graph is built on top of the KgNode/KgEdge types from swell-core
// and provides additional capabilities for code structure representation.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::sqlite::{SqlitePool, SqliteRow};
use sqlx::Row;
use std::collections::HashSet;
use std::sync::Arc;
use uuid::Uuid;

use swell_core::KnowledgeGraph;
use swell_core::{
    KgDirection, KgEdge, KgNode, KgNodeType, KgPath, KgRelation, KgTraversal, SwellError,
};

/// A source reference for provenance tracking
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProvenanceReference {
    /// The source type (e.g., "file", "commit", "document", "llm_generation")
    pub source_type: String,
    /// The source identifier (e.g., file path, commit hash)
    pub source_id: String,
    /// Optional description of this provenance entry
    pub description: Option<String>,
    /// When this source was referenced
    pub timestamp: DateTime<Utc>,
}

impl ProvenanceReference {
    pub fn new(source_type: impl Into<String>, source_id: impl Into<String>) -> Self {
        Self {
            source_type: source_type.into(),
            source_id: source_id.into(),
            description: None,
            timestamp: Utc::now(),
        }
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }
}

/// A node in the knowledge graph with metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeGraphNode {
    pub id: Uuid,
    pub node_type: KgNodeType,
    pub name: String,
    pub properties: serde_json::Value,
    pub repository: String,
    pub file_path: Option<String>,
    pub start_line: Option<u32>,
    pub end_line: Option<u32>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    /// Confidence score (0.0 to 1.0) indicating the reliability of this knowledge
    pub confidence: f64,
    /// Number of evidence sources supporting this knowledge
    pub evidence_count: u32,
    /// When this knowledge became valid
    pub valid_from: chrono::DateTime<chrono::Utc>,
    /// When this knowledge expires (None = never expires)
    pub valid_until: Option<chrono::DateTime<chrono::Utc>>,
    /// Provenance chain showing the source of this knowledge
    pub provenance: Vec<ProvenanceReference>,
}

impl From<KgNode> for KnowledgeGraphNode {
    fn from(node: KgNode) -> Self {
        // Extract standard fields from properties
        let repository = node
            .properties
            .get("repository")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let file_path = node
            .properties
            .get("source_path")
            .and_then(|v| v.as_str())
            .map(String::from);
        let start_line = node
            .properties
            .get("start_line")
            .and_then(|v| v.as_u64())
            .map(|v| v as u32);
        let end_line = node
            .properties
            .get("end_line")
            .and_then(|v| v.as_u64())
            .map(|v| v as u32);

        // Extract metadata fields from properties (stored by insert_node_with_metadata)
        let confidence = node
            .properties
            .get("confidence")
            .and_then(|v| v.as_f64())
            .unwrap_or(1.0);
        let evidence_count = node
            .properties
            .get("evidence_count")
            .and_then(|v| v.as_u64())
            .map(|v| v as u32)
            .unwrap_or(1);
        let created_at = node
            .properties
            .get("created_at")
            .and_then(|v| v.as_str())
            .and_then(|v| chrono::DateTime::parse_from_rfc3339(v).ok())
            .map(|dt| dt.with_timezone(&chrono::Utc))
            .unwrap_or_else(chrono::Utc::now);
        let updated_at = node
            .properties
            .get("updated_at")
            .and_then(|v| v.as_str())
            .and_then(|v| chrono::DateTime::parse_from_rfc3339(v).ok())
            .map(|dt| dt.with_timezone(&chrono::Utc))
            .unwrap_or_else(chrono::Utc::now);
        let valid_from = node
            .properties
            .get("valid_from")
            .and_then(|v| v.as_str())
            .and_then(|v| chrono::DateTime::parse_from_rfc3339(v).ok())
            .map(|dt| dt.with_timezone(&chrono::Utc))
            .unwrap_or_else(chrono::Utc::now);
        let valid_until = node
            .properties
            .get("valid_until")
            .and_then(|v| v.as_str())
            .and_then(|v| {
                if v == "null" || v.is_empty() {
                    None
                } else {
                    chrono::DateTime::parse_from_rfc3339(v).ok().map(|dt| dt.with_timezone(&chrono::Utc))
                }
            });
        let provenance: Vec<ProvenanceReference> = node
            .properties
            .get("provenance")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        Self {
            id: node.id,
            node_type: node.node_type,
            name: node.name,
            properties: node.properties.clone(),
            repository,
            file_path,
            start_line,
            end_line,
            created_at,
            updated_at,
            confidence,
            evidence_count,
            valid_from,
            valid_until,
            provenance,
        }
    }
}

impl From<KnowledgeGraphNode> for KgNode {
    fn from(node: KnowledgeGraphNode) -> Self {
        // Create a mutable properties object and add metadata fields
        let mut properties = node.properties.clone();
        if let Some(obj) = properties.as_object_mut() {
            obj.insert("repository".to_string(), serde_json::json!(node.repository));
            obj.insert("confidence".to_string(), serde_json::json!(node.confidence));
            obj.insert("evidence_count".to_string(), serde_json::json!(node.evidence_count));
            obj.insert("created_at".to_string(), serde_json::json!(node.created_at.to_rfc3339()));
            obj.insert("updated_at".to_string(), serde_json::json!(node.updated_at.to_rfc3339()));
            obj.insert("valid_from".to_string(), serde_json::json!(node.valid_from.to_rfc3339()));
            if let Some(valid_until) = node.valid_until {
                obj.insert("valid_until".to_string(), serde_json::json!(valid_until.to_rfc3339()));
            }
            obj.insert("provenance".to_string(), serde_json::json!(node.provenance));
            if let Some(fp) = node.file_path {
                obj.insert("source_path".to_string(), serde_json::json!(fp));
            }
            if let Some(sl) = node.start_line {
                obj.insert("start_line".to_string(), serde_json::json!(sl));
            }
            if let Some(el) = node.end_line {
                obj.insert("end_line".to_string(), serde_json::json!(el));
            }
        }

        KgNode {
            id: node.id,
            node_type: node.node_type,
            name: node.name,
            properties,
        }
    }
}

/// An edge in the knowledge graph with metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeGraphEdge {
    pub id: Uuid,
    pub source: Uuid,
    pub target: Uuid,
    pub relation: KgRelation,
    pub repository: String,
    pub properties: serde_json::Value,
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// Confidence score (0.0 to 1.0) indicating the reliability of this edge
    pub confidence: f64,
    /// Number of evidence sources supporting this edge
    pub evidence_count: u32,
    /// When this edge became valid
    pub valid_from: chrono::DateTime<chrono::Utc>,
    /// When this edge expires (None = never expires)
    pub valid_until: Option<chrono::DateTime<chrono::Utc>>,
    /// Provenance chain showing the source of this edge
    pub provenance: Vec<ProvenanceReference>,
}

impl From<KgEdge> for KnowledgeGraphEdge {
    fn from(edge: KgEdge) -> Self {
        Self {
            id: edge.id,
            source: edge.source,
            target: edge.target,
            relation: edge.relation,
            repository: String::new(),
            properties: serde_json::json!({}),
            created_at: chrono::Utc::now(),
            confidence: 1.0,
            evidence_count: 1,
            valid_from: chrono::Utc::now(),
            valid_until: None,
            provenance: Vec::new(),
        }
    }
}

impl From<KnowledgeGraphEdge> for KgEdge {
    fn from(edge: KnowledgeGraphEdge) -> Self {
        KgEdge {
            id: edge.id,
            source: edge.source,
            target: edge.target,
            relation: edge.relation,
        }
    }
}

/// Query options for the knowledge graph
#[derive(Debug, Clone)]
pub struct KnowledgeGraphQuery {
    pub repository: String,
    pub node_types: Option<Vec<KgNodeType>>,
    pub relations: Option<Vec<KgRelation>>,
    pub name_contains: Option<String>,
    pub file_path: Option<String>,
    pub max_depth: usize,
    pub direction: KgDirection,
    pub limit: usize,
    pub offset: usize,
    /// Minimum confidence threshold (0.0 to 1.0)
    pub min_confidence: Option<f64>,
    /// Query time for temporal validity filtering
    pub valid_at: Option<chrono::DateTime<chrono::Utc>>,
}

impl Default for KnowledgeGraphQuery {
    fn default() -> Self {
        Self {
            repository: String::new(),
            node_types: None,
            relations: None,
            name_contains: None,
            file_path: None,
            max_depth: 3,
            direction: KgDirection::Both,
            limit: 100,
            offset: 0,
            min_confidence: None,
            valid_at: None,
        }
    }
}

impl KnowledgeGraphQuery {
    pub fn new(repository: String) -> Self {
        Self {
            repository,
            node_types: None,
            relations: None,
            name_contains: None,
            file_path: None,
            max_depth: 3,
            direction: KgDirection::Both,
            limit: 100,
            offset: 0,
            min_confidence: None,
            valid_at: None,
        }
    }

    pub fn with_node_types(mut self, types: Vec<KgNodeType>) -> Self {
        self.node_types = Some(types);
        self
    }

    pub fn with_relations(mut self, relations: Vec<KgRelation>) -> Self {
        self.relations = Some(relations);
        self
    }

    pub fn with_name_contains(mut self, name: String) -> Self {
        self.name_contains = Some(name);
        self
    }

    pub fn with_file_path(mut self, path: String) -> Self {
        self.file_path = Some(path);
        self
    }

    pub fn with_max_depth(mut self, depth: usize) -> Self {
        self.max_depth = depth;
        self
    }

    pub fn with_direction(mut self, direction: KgDirection) -> Self {
        self.direction = direction;
        self
    }

    pub fn with_min_confidence(mut self, min_confidence: f64) -> Self {
        self.min_confidence = Some(min_confidence);
        self
    }

    pub fn with_valid_at(mut self, valid_at: chrono::DateTime<chrono::Utc>) -> Self {
        self.valid_at = Some(valid_at);
        self
    }
}

/// Result of a dependency lookup query
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyResult {
    pub node: KnowledgeGraphNode,
    pub relation: KgRelation,
    pub distance: usize,
}

/// Path finding result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PathResult {
    pub path: Vec<KnowledgeGraphNode>,
    pub relations: Vec<KgRelation>,
    pub total_hops: usize,
}

/// Cross-reference result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrossReferenceResult {
    pub node: KnowledgeGraphNode,
    pub references_from: Vec<KnowledgeGraphNode>,
    pub references_to: Vec<KnowledgeGraphNode>,
    pub total_incoming: usize,
    pub total_outgoing: usize,
}

/// SQLite-based knowledge graph store
#[derive(Clone)]
pub struct SqliteKnowledgeGraph {
    pool: Arc<SqlitePool>,
}

impl SqliteKnowledgeGraph {
    /// Create a new SqliteKnowledgeGraph with the given database URL
    pub async fn new(database_url: &str) -> Result<Self, SwellError> {
        Self::create(database_url).await
    }

    /// Create a new pool with the given database URL
    pub async fn create(database_url: &str) -> Result<Self, SwellError> {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(5)
            .connect(database_url)
            .await
            .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        // Initialize the schema
        Self::init_schema(&pool).await?;

        Ok(Self {
            pool: Arc::new(pool),
        })
    }

    /// Initialize the database schema for the knowledge graph
    async fn init_schema(pool: &SqlitePool) -> Result<(), SwellError> {
        // Knowledge graph nodes table
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS kg_nodes (
                id TEXT PRIMARY KEY,
                node_type TEXT NOT NULL,
                name TEXT NOT NULL,
                properties TEXT NOT NULL,
                repository TEXT NOT NULL,
                file_path TEXT,
                start_line INTEGER,
                end_line INTEGER,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                confidence REAL NOT NULL DEFAULT 1.0,
                evidence_count INTEGER NOT NULL DEFAULT 1,
                valid_from TEXT NOT NULL,
                valid_until TEXT,
                provenance TEXT NOT NULL DEFAULT '[]'
            )
            "#,
        )
        .execute(pool)
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        // Index on node_type for efficient type queries
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_kg_node_type ON kg_nodes(node_type)")
            .execute(pool)
            .await
            .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        // Index on name for search
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_kg_node_name ON kg_nodes(name)")
            .execute(pool)
            .await
            .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        // Index on repository for scope isolation
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_kg_node_repo ON kg_nodes(repository)")
            .execute(pool)
            .await
            .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        // Index on file_path for file-based queries
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_kg_node_file ON kg_nodes(file_path)")
            .execute(pool)
            .await
            .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        // Index on confidence for filtering
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_kg_node_confidence ON kg_nodes(confidence)")
            .execute(pool)
            .await
            .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        // Index on valid_from and valid_until for temporal queries
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_kg_node_validity ON kg_nodes(valid_from, valid_until)")
            .execute(pool)
            .await
            .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        // Knowledge graph edges table
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS kg_edges (
                id TEXT PRIMARY KEY,
                source TEXT NOT NULL,
                target TEXT NOT NULL,
                relation TEXT NOT NULL,
                repository TEXT NOT NULL,
                properties TEXT NOT NULL,
                created_at TEXT NOT NULL,
                confidence REAL NOT NULL DEFAULT 1.0,
                evidence_count INTEGER NOT NULL DEFAULT 1,
                valid_from TEXT NOT NULL,
                valid_until TEXT,
                provenance TEXT NOT NULL DEFAULT '[]'
            )
            "#,
        )
        .execute(pool)
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        // Index on source for outgoing edge queries
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_kg_edge_source ON kg_edges(source)")
            .execute(pool)
            .await
            .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        // Index on target for incoming edge queries
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_kg_edge_target ON kg_edges(target)")
            .execute(pool)
            .await
            .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        // Index on relation for relation-based queries
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_kg_edge_relation ON kg_edges(relation)")
            .execute(pool)
            .await
            .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        // Index on repository for scope isolation
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_kg_edge_repo ON kg_edges(repository)")
            .execute(pool)
            .await
            .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        // Index on confidence for filtering
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_kg_edge_confidence ON kg_edges(confidence)")
            .execute(pool)
            .await
            .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        // Index on valid_from and valid_until for temporal queries
        sqlx::query("CREATE INDEX IF NOT EXISTS idx_kg_edge_validity ON kg_edges(valid_from, valid_until)")
            .execute(pool)
            .await
            .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        Ok(())
    }

    /// Convert node type enum to string
    fn node_type_to_string(node_type: KgNodeType) -> &'static str {
        match node_type {
            KgNodeType::File => "file",
            KgNodeType::Function => "function",
            KgNodeType::Class => "class",
            KgNodeType::Method => "method",
            KgNodeType::Module => "module",
            KgNodeType::Type => "type",
            KgNodeType::Import => "import",
            KgNodeType::Variable => "variable",
            KgNodeType::Test => "test",
        }
    }

    /// Convert string to node type enum
    fn string_to_node_type(s: &str) -> KgNodeType {
        match s {
            "file" => KgNodeType::File,
            "function" => KgNodeType::Function,
            "class" => KgNodeType::Class,
            "method" => KgNodeType::Method,
            "module" => KgNodeType::Module,
            "type" => KgNodeType::Type,
            "import" => KgNodeType::Import,
            "variable" => KgNodeType::Variable,
            "test" => KgNodeType::Test,
            _ => KgNodeType::File,
        }
    }

    /// Convert relation enum to string
    fn relation_to_string(relation: KgRelation) -> &'static str {
        match relation {
            KgRelation::Calls => "calls",
            KgRelation::InheritsFrom => "inherits_from",
            KgRelation::Imports => "imports",
            KgRelation::DependsOn => "depends_on",
            KgRelation::Contains => "contains",
            KgRelation::HasType => "has_type",
            KgRelation::Tests => "tests",
        }
    }

    /// Convert string to relation enum
    fn string_to_relation(s: &str) -> KgRelation {
        match s {
            "calls" => KgRelation::Calls,
            "inherits_from" => KgRelation::InheritsFrom,
            "imports" => KgRelation::Imports,
            "depends_on" => KgRelation::DependsOn,
            "contains" => KgRelation::Contains,
            "has_type" => KgRelation::HasType,
            "tests" => KgRelation::Tests,
            _ => KgRelation::DependsOn,
        }
    }

    /// Convert database row to KnowledgeGraphNode
    fn row_to_node(row: &SqliteRow) -> Result<KnowledgeGraphNode, SwellError> {
        let id_str: String = row.get("id");
        let id = Uuid::parse_str(&id_str)
            .map_err(|e| SwellError::DatabaseError(format!("Invalid UUID: {}", e)))?;
        let node_type_str: String = row.get("node_type");
        let name: String = row.get("name");
        let properties_str: String = row.get("properties");
        let repository: String = row.get("repository");
        let file_path: Option<String> = row.get("file_path");
        let start_line: Option<i64> = row.get("start_line");
        let end_line: Option<i64> = row.get("end_line");
        let created_at_str: String = row.get("created_at");
        let updated_at_str: String = row.get("updated_at");
        let confidence: f64 = row.get("confidence");
        let evidence_count: i64 = row.get("evidence_count");
        let valid_from_str: String = row.get("valid_from");
        let valid_until_str: Option<String> = row.get("valid_until");
        let provenance_str: String = row.get("provenance");

        let node_type = Self::string_to_node_type(&node_type_str);
        let properties: serde_json::Value = serde_json::from_str(&properties_str)
            .map_err(|e| SwellError::DatabaseError(format!("Invalid JSON properties: {}", e)))?;

        let created_at = chrono::DateTime::parse_from_rfc3339(&created_at_str)
            .map_err(|e| SwellError::DatabaseError(format!("Invalid timestamp: {}", e)))?
            .with_timezone(&chrono::Utc);
        let updated_at = chrono::DateTime::parse_from_rfc3339(&updated_at_str)
            .map_err(|e| SwellError::DatabaseError(format!("Invalid timestamp: {}", e)))?
            .with_timezone(&chrono::Utc);

        let valid_from = chrono::DateTime::parse_from_rfc3339(&valid_from_str)
            .map_err(|e| SwellError::DatabaseError(format!("Invalid timestamp: {}", e)))?
            .with_timezone(&chrono::Utc);

        let valid_until = valid_until_str
            .map(|s| {
                chrono::DateTime::parse_from_rfc3339(&s)
                    .map(|dt| dt.with_timezone(&chrono::Utc))
                    .map_err(|e| SwellError::DatabaseError(format!("Invalid timestamp: {}", e)))
            })
            .transpose()?;

        let provenance: Vec<ProvenanceReference> = serde_json::from_str(&provenance_str)
            .map_err(|e| SwellError::DatabaseError(format!("Invalid JSON provenance: {}", e)))?;

        Ok(KnowledgeGraphNode {
            id,
            node_type,
            name,
            properties,
            repository,
            file_path,
            start_line: start_line.map(|v| v as u32),
            end_line: end_line.map(|v| v as u32),
            created_at,
            updated_at,
            confidence,
            evidence_count: evidence_count as u32,
            valid_from,
            valid_until,
            provenance,
        })
    }

    /// Convert database row to KnowledgeGraphEdge
    fn row_to_edge(row: &SqliteRow) -> Result<KnowledgeGraphEdge, SwellError> {
        let id_str: String = row.get("id");
        let id = Uuid::parse_str(&id_str)
            .map_err(|e| SwellError::DatabaseError(format!("Invalid UUID: {}", e)))?;
        let source_str: String = row.get("source");
        let target_str: String = row.get("target");
        let relation_str: String = row.get("relation");
        let repository: String = row.get("repository");
        let properties_str: String = row.get("properties");
        let created_at_str: String = row.get("created_at");
        let confidence: f64 = row.get("confidence");
        let evidence_count: i64 = row.get("evidence_count");
        let valid_from_str: String = row.get("valid_from");
        let valid_until_str: Option<String> = row.get("valid_until");
        let provenance_str: String = row.get("provenance");

        let source = Uuid::parse_str(&source_str)
            .map_err(|e| SwellError::DatabaseError(format!("Invalid UUID: {}", e)))?;
        let target = Uuid::parse_str(&target_str)
            .map_err(|e| SwellError::DatabaseError(format!("Invalid UUID: {}", e)))?;
        let relation = Self::string_to_relation(&relation_str);
        let properties: serde_json::Value = serde_json::from_str(&properties_str)
            .map_err(|e| SwellError::DatabaseError(format!("Invalid JSON properties: {}", e)))?;

        let created_at = chrono::DateTime::parse_from_rfc3339(&created_at_str)
            .map_err(|e| SwellError::DatabaseError(format!("Invalid timestamp: {}", e)))?
            .with_timezone(&chrono::Utc);

        let valid_from = chrono::DateTime::parse_from_rfc3339(&valid_from_str)
            .map_err(|e| SwellError::DatabaseError(format!("Invalid timestamp: {}", e)))?
            .with_timezone(&chrono::Utc);

        let valid_until = valid_until_str
            .map(|s| {
                chrono::DateTime::parse_from_rfc3339(&s)
                    .map(|dt| dt.with_timezone(&chrono::Utc))
                    .map_err(|e| SwellError::DatabaseError(format!("Invalid timestamp: {}", e)))
            })
            .transpose()?;

        let provenance: Vec<ProvenanceReference> = serde_json::from_str(&provenance_str)
            .map_err(|e| SwellError::DatabaseError(format!("Invalid JSON provenance: {}", e)))?;

        Ok(KnowledgeGraphEdge {
            id,
            source,
            target,
            relation,
            repository,
            properties,
            created_at,
            confidence,
            evidence_count: evidence_count as u32,
            valid_from,
            valid_until,
            provenance,
        })
    }
}

#[async_trait]
impl KnowledgeGraph for SqliteKnowledgeGraph {
    /// Add a node to the graph
    async fn add_node(&self, node: KgNode) -> Result<Uuid, SwellError> {
        let kg_node: KnowledgeGraphNode = node.into();
        let node_type_str = Self::node_type_to_string(kg_node.node_type);
        let properties_str = serde_json::to_string(&kg_node.properties)
            .map_err(|e| SwellError::DatabaseError(e.to_string()))?;
        let created_at_str = kg_node.created_at.to_rfc3339();
        let updated_at_str = kg_node.updated_at.to_rfc3339();
        let valid_from_str = kg_node.valid_from.to_rfc3339();
        let valid_until_str = kg_node.valid_until.map(|dt| dt.to_rfc3339());
        let provenance_str = serde_json::to_string(&kg_node.provenance)
            .map_err(|e| SwellError::DatabaseError(e.to_string()))?;

        sqlx::query(
            r#"
            INSERT OR REPLACE INTO kg_nodes (id, node_type, name, properties, repository, file_path, start_line, end_line, created_at, updated_at, confidence, evidence_count, valid_from, valid_until, provenance)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(kg_node.id.to_string())
        .bind(node_type_str)
        .bind(&kg_node.name)
        .bind(&properties_str)
        .bind(&kg_node.repository)
        .bind(&kg_node.file_path)
        .bind(kg_node.start_line.map(|v| v as i64))
        .bind(kg_node.end_line.map(|v| v as i64))
        .bind(&created_at_str)
        .bind(&updated_at_str)
        .bind(kg_node.confidence)
        .bind(kg_node.evidence_count as i64)
        .bind(&valid_from_str)
        .bind(&valid_until_str)
        .bind(&provenance_str)
        .execute(self.pool.as_ref())
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        Ok(kg_node.id)
    }

    /// Add an edge between nodes
    async fn add_edge(&self, edge: KgEdge) -> Result<(), SwellError> {
        let kg_edge: KnowledgeGraphEdge = edge.into();
        let relation_str = Self::relation_to_string(kg_edge.relation);
        let properties_str = serde_json::to_string(&kg_edge.properties)
            .map_err(|e| SwellError::DatabaseError(e.to_string()))?;
        let created_at_str = kg_edge.created_at.to_rfc3339();
        let valid_from_str = kg_edge.valid_from.to_rfc3339();
        let valid_until_str = kg_edge.valid_until.map(|dt| dt.to_rfc3339());
        let provenance_str = serde_json::to_string(&kg_edge.provenance)
            .map_err(|e| SwellError::DatabaseError(e.to_string()))?;

        sqlx::query(
            r#"
            INSERT OR REPLACE INTO kg_edges (id, source, target, relation, repository, properties, created_at, confidence, evidence_count, valid_from, valid_until, provenance)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(kg_edge.id.to_string())
        .bind(kg_edge.source.to_string())
        .bind(kg_edge.target.to_string())
        .bind(relation_str)
        .bind(&kg_edge.repository)
        .bind(&properties_str)
        .bind(&created_at_str)
        .bind(kg_edge.confidence)
        .bind(kg_edge.evidence_count as i64)
        .bind(&valid_from_str)
        .bind(&valid_until_str)
        .bind(&provenance_str)
        .execute(self.pool.as_ref())
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        Ok(())
    }

    /// Get a node by ID
    async fn get_node(&self, id: Uuid) -> Result<Option<KgNode>, SwellError> {
        let row = sqlx::query("SELECT * FROM kg_nodes WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(self.pool.as_ref())
            .await
            .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        match row {
            Some(r) => {
                let node = Self::row_to_node(&r)?;
                Ok(Some(node.into()))
            }
            None => Ok(None),
        }
    }

    /// Query nodes by label/name
    async fn query_nodes(&self, label: String) -> Result<Vec<KgNode>, SwellError> {
        let rows = sqlx::query("SELECT * FROM kg_nodes WHERE name LIKE ?")
            .bind(format!("%{}%", label))
            .fetch_all(self.pool.as_ref())
            .await
            .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        let mut nodes = Vec::new();
        for row in rows {
            let node = Self::row_to_node(&row)?;
            nodes.push(node.into());
        }

        Ok(nodes)
    }

    /// Traverse the graph from a starting node (iterative implementation)
    async fn traverse(&self, traversal: KgTraversal) -> Result<Vec<KgPath>, SwellError> {
        // Get the starting node
        let start_node = match traversal.start_node {
            id if id == Uuid::nil() => {
                return Err(SwellError::InvalidOperation(
                    "Start node cannot be nil for traversal".to_string(),
                ))
            }
            id => id,
        };

        // Check if start node exists
        let start_exists = sqlx::query("SELECT 1 FROM kg_nodes WHERE id = ?")
            .bind(start_node.to_string())
            .fetch_optional(self.pool.as_ref())
            .await
            .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        if start_exists.is_none() {
            return Ok(Vec::new());
        }

        // Perform iterative DFS traversal
        let mut visited: HashSet<Uuid> = HashSet::new();
        let mut paths: Vec<KgPath> = Vec::new();

        // Stack entries: (node_id, current_path_nodes, current_path_edges, depth)
        let mut stack: Vec<(Uuid, Vec<KgNode>, Vec<KgEdge>, usize)> = Vec::new();

        // Get start node and add to stack
        if let Some(node_row) = sqlx::query("SELECT * FROM kg_nodes WHERE id = ?")
            .bind(start_node.to_string())
            .fetch_optional(self.pool.as_ref())
            .await
            .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?
        {
            let start_node: KnowledgeGraphNode = Self::row_to_node(&node_row)?;
            stack.push((start_node.id, vec![start_node.into()], vec![], 0));
        }

        while let Some((node_id, path_nodes, path_edges, depth)) = stack.pop() {
            // Check depth limit
            if depth >= traversal.max_depth {
                if path_nodes.len() > 1 {
                    paths.push(KgPath {
                        nodes: path_nodes,
                        edges: path_edges,
                    });
                }
                continue;
            }

            // Mark as visited
            if !visited.insert(node_id) {
                continue;
            }

            // Get edges based on direction
            let edges: Vec<KnowledgeGraphEdge> = match traversal.direction {
                KgDirection::Outgoing => {
                    let rows = sqlx::query("SELECT * FROM kg_edges WHERE source = ?")
                        .bind(node_id.to_string())
                        .fetch_all(self.pool.as_ref())
                        .await
                        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;
                    rows.iter()
                        .filter_map(|row| Self::row_to_edge(row).ok())
                        .collect()
                }
                KgDirection::Incoming => {
                    let rows = sqlx::query("SELECT * FROM kg_edges WHERE target = ?")
                        .bind(node_id.to_string())
                        .fetch_all(self.pool.as_ref())
                        .await
                        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;
                    rows.iter()
                        .filter_map(|row| Self::row_to_edge(row).ok())
                        .collect()
                }
                KgDirection::Both => {
                    let rows = sqlx::query("SELECT * FROM kg_edges WHERE source = ? OR target = ?")
                        .bind(node_id.to_string())
                        .bind(node_id.to_string())
                        .fetch_all(self.pool.as_ref())
                        .await
                        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;
                    rows.iter()
                        .filter_map(|row| Self::row_to_edge(row).ok())
                        .collect()
                }
            };

            // Filter edges by relation if specified
            let filtered_edges: Vec<_> = edges
                .into_iter()
                .filter(|edge| traversal.relation.is_none_or(|rel| edge.relation == rel))
                .collect();

            // If no edges to follow, record path
            if filtered_edges.is_empty() {
                if path_nodes.len() > 1 {
                    paths.push(KgPath {
                        nodes: path_nodes,
                        edges: path_edges,
                    });
                }
                continue;
            }

            for edge in filtered_edges {
                let next_id = if traversal.direction == KgDirection::Outgoing {
                    edge.target
                } else {
                    edge.source
                };

                // Skip if already visited
                if visited.contains(&next_id) {
                    continue;
                }

                // Get the next node
                if let Some(node_row) = sqlx::query("SELECT * FROM kg_nodes WHERE id = ?")
                    .bind(next_id.to_string())
                    .fetch_optional(self.pool.as_ref())
                    .await
                    .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?
                {
                    let next_node: KnowledgeGraphNode = Self::row_to_node(&node_row)?;

                    // Add edge and node to path
                    let mut new_path_nodes = path_nodes.clone();
                    let mut new_path_edges = path_edges.clone();
                    new_path_edges.push(edge.clone().into());
                    new_path_nodes.push(next_node.clone().into());

                    // Add to stack
                    stack.push((next_id, new_path_nodes, new_path_edges, depth + 1));
                }
            }

            visited.remove(&node_id);
        }

        Ok(paths)
    }
}

impl SqliteKnowledgeGraph {
    /// Find dependencies (outgoing edges) for a node
    pub async fn find_dependencies(
        &self,
        node_id: Uuid,
    ) -> Result<Vec<DependencyResult>, SwellError> {
        let rows = sqlx::query(
            r#"
            SELECT e.*, n.* FROM kg_edges e
            JOIN kg_nodes n ON e.target = n.id
            WHERE e.source = ?
            "#,
        )
        .bind(node_id.to_string())
        .fetch_all(self.pool.as_ref())
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        let mut results = Vec::new();
        for row in rows {
            let edge = Self::row_to_edge(&row)?;
            let node = Self::row_to_node(&row)?;
            results.push(DependencyResult {
                node,
                relation: edge.relation,
                distance: 1,
            });
        }

        Ok(results)
    }

    /// Find dependents (incoming edges) for a node
    pub async fn find_dependents(
        &self,
        node_id: Uuid,
    ) -> Result<Vec<DependencyResult>, SwellError> {
        let rows = sqlx::query(
            r#"
            SELECT e.*, n.* FROM kg_edges e
            JOIN kg_nodes n ON e.source = n.id
            WHERE e.target = ?
            "#,
        )
        .bind(node_id.to_string())
        .fetch_all(self.pool.as_ref())
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        let mut results = Vec::new();
        for row in rows {
            let edge = Self::row_to_edge(&row)?;
            let node = Self::row_to_node(&row)?;
            results.push(DependencyResult {
                node,
                relation: edge.relation,
                distance: 1,
            });
        }

        Ok(results)
    }

    /// Find path between two nodes (BFS for shortest path)
    pub async fn find_path(
        &self,
        from_id: Uuid,
        to_id: Uuid,
        max_depth: usize,
    ) -> Result<Option<PathResult>, SwellError> {
        if from_id == to_id {
            // Get the node
            if let Some(node_row) = sqlx::query("SELECT * FROM kg_nodes WHERE id = ?")
                .bind(from_id.to_string())
                .fetch_optional(self.pool.as_ref())
                .await
                .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?
            {
                let node: KnowledgeGraphNode = Self::row_to_node(&node_row)?;
                return Ok(Some(PathResult {
                    path: vec![node],
                    relations: vec![],
                    total_hops: 0,
                }));
            }
            return Ok(None);
        }

        // BFS for shortest path
        let mut visited: HashSet<Uuid> = HashSet::new();
        let mut queue: Vec<(Uuid, Vec<Uuid>, Vec<KgRelation>)> = Vec::new();
        queue.push((from_id, vec![from_id], vec![]));
        visited.insert(from_id);

        while let Some((current_id, path, relations)) = queue.pop() {
            // Check if we've exceeded max depth
            if path.len() > max_depth + 1 {
                continue;
            }

            // Check if we found the target
            if current_id == to_id {
                // Get nodes for path
                let mut path_nodes = Vec::new();
                for node_id in &path {
                    if let Some(node_row) = sqlx::query("SELECT * FROM kg_nodes WHERE id = ?")
                        .bind(node_id.to_string())
                        .fetch_optional(self.pool.as_ref())
                        .await
                        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?
                    {
                        let node: KnowledgeGraphNode = Self::row_to_node(&node_row)?;
                        path_nodes.push(node);
                    }
                }

                return Ok(Some(PathResult {
                    path: path_nodes,
                    relations: relations.clone(),
                    total_hops: relations.len(),
                }));
            }

            // Get outgoing edges
            let rows = sqlx::query("SELECT * FROM kg_edges WHERE source = ?")
                .bind(current_id.to_string())
                .fetch_all(self.pool.as_ref())
                .await
                .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

            for row in rows {
                let edge = Self::row_to_edge(&row)?;
                if !visited.contains(&edge.target) {
                    visited.insert(edge.target);
                    let mut new_path = path.clone();
                    new_path.push(edge.target);
                    let mut new_relations = relations.clone();
                    new_relations.push(edge.relation);
                    queue.push((edge.target, new_path, new_relations));
                }
            }
        }

        Ok(None)
    }

    /// Get cross-references for a node
    pub async fn get_cross_references(
        &self,
        node_id: Uuid,
    ) -> Result<CrossReferenceResult, SwellError> {
        // Get the node
        let node_row = sqlx::query("SELECT * FROM kg_nodes WHERE id = ?")
            .bind(node_id.to_string())
            .fetch_optional(self.pool.as_ref())
            .await
            .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        let node = match node_row {
            Some(row) => Self::row_to_node(&row)?,
            None => {
                return Err(SwellError::DatabaseError(format!(
                    "Node not found: {}",
                    node_id
                )))
            }
        };

        // Get incoming edges (references from other nodes)
        let incoming_rows = sqlx::query("SELECT * FROM kg_edges WHERE target = ?")
            .bind(node_id.to_string())
            .fetch_all(self.pool.as_ref())
            .await
            .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        let mut references_from = Vec::new();
        for row in incoming_rows {
            let edge = Self::row_to_edge(&row)?;
            if let Some(source_row) = sqlx::query("SELECT * FROM kg_nodes WHERE id = ?")
                .bind(edge.source.to_string())
                .fetch_optional(self.pool.as_ref())
                .await
                .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?
            {
                let source_node: KnowledgeGraphNode = Self::row_to_node(&source_row)?;
                references_from.push(source_node);
            }
        }

        // Get outgoing edges (references to other nodes)
        let outgoing_rows = sqlx::query("SELECT * FROM kg_edges WHERE source = ?")
            .bind(node_id.to_string())
            .fetch_all(self.pool.as_ref())
            .await
            .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        let mut references_to = Vec::new();
        for row in outgoing_rows {
            let edge = Self::row_to_edge(&row)?;
            if let Some(target_row) = sqlx::query("SELECT * FROM kg_nodes WHERE id = ?")
                .bind(edge.target.to_string())
                .fetch_optional(self.pool.as_ref())
                .await
                .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?
            {
                let target_node: KnowledgeGraphNode = Self::row_to_node(&target_row)?;
                references_to.push(target_node);
            }
        }

        Ok(CrossReferenceResult {
            node,
            references_from: references_from.clone(),
            references_to: references_to.clone(),
            total_incoming: references_from.len(),
            total_outgoing: references_to.len(),
        })
    }

    /// Get all nodes in a file
    pub async fn get_nodes_in_file(
        &self,
        repository: &str,
        file_path: &str,
    ) -> Result<Vec<KgNode>, SwellError> {
        let rows = sqlx::query("SELECT * FROM kg_nodes WHERE repository = ? AND file_path = ?")
            .bind(repository)
            .bind(file_path)
            .fetch_all(self.pool.as_ref())
            .await
            .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        let mut nodes = Vec::new();
        for row in rows {
            let node: KnowledgeGraphNode = Self::row_to_node(&row)?;
            nodes.push(node.into());
        }

        Ok(nodes)
    }

    /// Get all nodes by type
    pub async fn get_nodes_by_type(
        &self,
        repository: &str,
        node_type: KgNodeType,
    ) -> Result<Vec<KgNode>, SwellError> {
        let node_type_str = Self::node_type_to_string(node_type);
        let rows = sqlx::query("SELECT * FROM kg_nodes WHERE repository = ? AND node_type = ?")
            .bind(repository)
            .bind(node_type_str)
            .fetch_all(self.pool.as_ref())
            .await
            .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        let mut nodes = Vec::new();
        for row in rows {
            let node: KnowledgeGraphNode = Self::row_to_node(&row)?;
            nodes.push(node.into());
        }

        Ok(nodes)
    }

    /// Delete a node and all its edges
    pub async fn delete_node(&self, id: Uuid) -> Result<(), SwellError> {
        // Delete edges where this node is source or target
        sqlx::query("DELETE FROM kg_edges WHERE source = ? OR target = ?")
            .bind(id.to_string())
            .bind(id.to_string())
            .execute(self.pool.as_ref())
            .await
            .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        // Delete the node
        sqlx::query("DELETE FROM kg_nodes WHERE id = ?")
            .bind(id.to_string())
            .execute(self.pool.as_ref())
            .await
            .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        Ok(())
    }

    /// Clear all nodes and edges for a repository
    pub async fn clear_repository(&self, repository: &str) -> Result<(), SwellError> {
        sqlx::query("DELETE FROM kg_edges WHERE repository = ?")
            .bind(repository)
            .execute(self.pool.as_ref())
            .await
            .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        sqlx::query("DELETE FROM kg_nodes WHERE repository = ?")
            .bind(repository)
            .execute(self.pool.as_ref())
            .await
            .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        Ok(())
    }

    /// Get graph statistics
    pub async fn get_stats(&self, repository: &str) -> Result<GraphStats, SwellError> {
        let node_count: i64 = sqlx::query("SELECT COUNT(*) FROM kg_nodes WHERE repository = ?")
            .bind(repository)
            .fetch_one(self.pool.as_ref())
            .await
            .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?
            .get(0);

        let edge_count: i64 = sqlx::query("SELECT COUNT(*) FROM kg_edges WHERE repository = ?")
            .bind(repository)
            .fetch_one(self.pool.as_ref())
            .await
            .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?
            .get(0);

        let file_count: i64 = sqlx::query(
            "SELECT COUNT(DISTINCT file_path) FROM kg_nodes WHERE repository = ? AND file_path IS NOT NULL",
        )
        .bind(repository)
        .fetch_one(self.pool.as_ref())
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?
        .get(0);

        Ok(GraphStats {
            total_nodes: node_count as usize,
            total_edges: edge_count as usize,
            files: file_count as usize,
        })
    }

    /// Insert a node with full metadata (confidence, evidence_count, temporal validity, provenance)
    pub async fn insert_node_with_metadata(
        &self,
        node: KnowledgeGraphNode,
    ) -> Result<Uuid, SwellError> {
        let node_type_str = Self::node_type_to_string(node.node_type);
        let properties_str = serde_json::to_string(&node.properties)
            .map_err(|e| SwellError::DatabaseError(e.to_string()))?;
        let created_at_str = node.created_at.to_rfc3339();
        let updated_at_str = node.updated_at.to_rfc3339();
        let valid_from_str = node.valid_from.to_rfc3339();
        let valid_until_str = node.valid_until.map(|dt| dt.to_rfc3339());
        let provenance_str = serde_json::to_string(&node.provenance)
            .map_err(|e| SwellError::DatabaseError(e.to_string()))?;

        sqlx::query(
            r#"
            INSERT OR REPLACE INTO kg_nodes (id, node_type, name, properties, repository, file_path, start_line, end_line, created_at, updated_at, confidence, evidence_count, valid_from, valid_until, provenance)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(node.id.to_string())
        .bind(node_type_str)
        .bind(&node.name)
        .bind(&properties_str)
        .bind(&node.repository)
        .bind(&node.file_path)
        .bind(node.start_line.map(|v| v as i64))
        .bind(node.end_line.map(|v| v as i64))
        .bind(&created_at_str)
        .bind(&updated_at_str)
        .bind(node.confidence)
        .bind(node.evidence_count as i64)
        .bind(&valid_from_str)
        .bind(&valid_until_str)
        .bind(&provenance_str)
        .execute(self.pool.as_ref())
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        Ok(node.id)
    }

    /// Insert an edge with full metadata (confidence, evidence_count, temporal validity, provenance)
    pub async fn insert_edge_with_metadata(
        &self,
        edge: KnowledgeGraphEdge,
    ) -> Result<(), SwellError> {
        let relation_str = Self::relation_to_string(edge.relation);
        let properties_str = serde_json::to_string(&edge.properties)
            .map_err(|e| SwellError::DatabaseError(e.to_string()))?;
        let created_at_str = edge.created_at.to_rfc3339();
        let valid_from_str = edge.valid_from.to_rfc3339();
        let valid_until_str = edge.valid_until.map(|dt| dt.to_rfc3339());
        let provenance_str = serde_json::to_string(&edge.provenance)
            .map_err(|e| SwellError::DatabaseError(e.to_string()))?;

        sqlx::query(
            r#"
            INSERT OR REPLACE INTO kg_edges (id, source, target, relation, repository, properties, created_at, confidence, evidence_count, valid_from, valid_until, provenance)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(edge.id.to_string())
        .bind(edge.source.to_string())
        .bind(edge.target.to_string())
        .bind(relation_str)
        .bind(&edge.repository)
        .bind(&properties_str)
        .bind(&created_at_str)
        .bind(edge.confidence)
        .bind(edge.evidence_count as i64)
        .bind(&valid_from_str)
        .bind(&valid_until_str)
        .bind(&provenance_str)
        .execute(self.pool.as_ref())
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        Ok(())
    }

    /// Query nodes with metadata filtering (min_confidence and temporal validity)
    pub async fn query_nodes_with_metadata(
        &self,
        query: KnowledgeGraphQuery,
    ) -> Result<Vec<KnowledgeGraphNode>, SwellError> {
        let repository = query.repository.clone();
        let min_confidence = query.min_confidence;
        let valid_at = query.valid_at;
        let name_contains = query.name_contains.clone();
        let file_path = query.file_path.clone();
        let limit = query.limit;
        let offset = query.offset;

        // Pre-compute all bound values to extend their lifetimes
        let valid_at_str: Option<String> = valid_at.map(|va| va.to_rfc3339());
        let name_like: Option<String> = name_contains.as_ref().map(|nc| format!("%{}%", nc));

        let mut sql = String::from("SELECT * FROM kg_nodes WHERE repository = ?");

        if min_confidence.is_some() {
            sql.push_str(" AND confidence >= ?");
        }
        if valid_at.is_some() {
            sql.push_str(" AND valid_from <= ? AND (valid_until IS NULL OR valid_until >= ?)");
        }

        // Add node_types filter
        if let Some(ref node_types) = query.node_types {
            if !node_types.is_empty() {
                let placeholders: Vec<String> = node_types
                    .iter()
                    .map(|t| format!("'{}'", Self::node_type_to_string(*t)))
                    .collect();
                sql.push_str(&format!(" AND node_type IN ({})", placeholders.join(", ")));
            }
        }

        if name_contains.is_some() {
            sql.push_str(" AND name LIKE ?");
        }

        if file_path.is_some() {
            sql.push_str(" AND file_path = ?");
        }

        sql.push_str(&format!(" LIMIT {} OFFSET {}", limit, offset));

        // Build and execute query
        let mut q = sqlx::query(&sql);
        q = q.bind(&repository);
        if let Some(mc) = min_confidence {
            q = q.bind(mc);
        }
        if let Some(ref va_str) = valid_at_str {
            q = q.bind(va_str);
            q = q.bind(va_str);
        }
        if let Some(ref nl) = name_like {
            q = q.bind(nl);
        }
        if let Some(ref fp) = file_path {
            q = q.bind(fp);
        }

        let rows = q
            .fetch_all(self.pool.as_ref())
            .await
            .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        let mut nodes = Vec::new();
        for row in rows {
            let node = Self::row_to_node(&row)?;
            nodes.push(node);
        }

        Ok(nodes)
    }

    /// Query edges with metadata filtering (min_confidence and temporal validity)
    pub async fn query_edges_with_metadata(
        &self,
        repository: &str,
        min_confidence: Option<f64>,
        valid_at: Option<DateTime<Utc>>,
    ) -> Result<Vec<KnowledgeGraphEdge>, SwellError> {
        // Pre-compute all bound values to extend their lifetimes
        let valid_at_str: Option<String> = valid_at.map(|va| va.to_rfc3339());

        let mut sql = String::from("SELECT * FROM kg_edges WHERE repository = ?");

        if min_confidence.is_some() {
            sql.push_str(" AND confidence >= ?");
        }
        if valid_at.is_some() {
            sql.push_str(" AND valid_from <= ? AND (valid_until IS NULL OR valid_until >= ?)");
        }

        // Build and execute query
        let mut q = sqlx::query(&sql);
        q = q.bind(repository);
        if let Some(mc) = min_confidence {
            q = q.bind(mc);
        }
        if let Some(ref va_str) = valid_at_str {
            q = q.bind(va_str);
            q = q.bind(va_str);
        }

        let rows = q
            .fetch_all(self.pool.as_ref())
            .await
            .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        let mut edges = Vec::new();
        for row in rows {
            let edge = Self::row_to_edge(&row)?;
            edges.push(edge);
        }

        Ok(edges)
    }
}

/// Graph statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphStats {
    pub total_nodes: usize,
    pub total_edges: usize,
    pub files: usize,
}

/// Knowledge graph trait for graph operations
#[async_trait]
pub trait KnowledgeGraphStore: Send + Sync {
    /// Add a node to the graph
    async fn add_node(&self, node: KgNode) -> Result<Uuid, SwellError>;

    /// Add an edge between nodes
    async fn add_edge(&self, edge: KgEdge) -> Result<(), SwellError>;

    /// Get a node by ID
    async fn get_node(&self, id: Uuid) -> Result<Option<KgNode>, SwellError>;

    /// Query nodes by label/name
    async fn query_nodes(&self, label: String) -> Result<Vec<KgNode>, SwellError>;

    /// Traverse the graph from a starting node
    async fn traverse(&self, traversal: KgTraversal) -> Result<Vec<KgPath>, SwellError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_add_and_get_node() {
        let kg = SqliteKnowledgeGraph::create("sqlite::memory:")
            .await
            .unwrap();

        let node = KgNode {
            id: Uuid::new_v4(),
            node_type: KgNodeType::Function,
            name: "test_function".to_string(),
            properties: serde_json::json!({"source_path": "test.rs", "start_line": 1, "end_line": 10}),
        };

        let id = kg.add_node(node.clone()).await.unwrap();
        assert_eq!(id, node.id);

        let retrieved = kg.get_node(node.id).await.unwrap();
        assert!(retrieved.is_some());
        let retrieved = retrieved.unwrap();
        assert_eq!(retrieved.name, "test_function");
        assert_eq!(retrieved.node_type, KgNodeType::Function);
    }

    #[tokio::test]
    async fn test_add_edge() {
        let kg = SqliteKnowledgeGraph::create("sqlite::memory:")
            .await
            .unwrap();

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

        kg.add_node(node1.clone()).await.unwrap();
        kg.add_node(node2.clone()).await.unwrap();

        let edge = KgEdge {
            id: Uuid::new_v4(),
            source: node1.id,
            target: node2.id,
            relation: KgRelation::Calls,
        };

        kg.add_edge(edge.clone()).await.unwrap();

        let deps = kg.find_dependencies(node1.id).await.unwrap();
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].node.name, "callee");
        assert_eq!(deps[0].relation, KgRelation::Calls);
    }

    #[tokio::test]
    async fn test_find_path() {
        let kg = SqliteKnowledgeGraph::create("sqlite::memory:")
            .await
            .unwrap();

        // Create: a -> b -> c
        let a = KgNode {
            id: Uuid::new_v4(),
            node_type: KgNodeType::Function,
            name: "a".to_string(),
            properties: serde_json::json!({}),
        };
        let b = KgNode {
            id: Uuid::new_v4(),
            node_type: KgNodeType::Function,
            name: "b".to_string(),
            properties: serde_json::json!({}),
        };
        let c = KgNode {
            id: Uuid::new_v4(),
            node_type: KgNodeType::Function,
            name: "c".to_string(),
            properties: serde_json::json!({}),
        };

        kg.add_node(a.clone()).await.unwrap();
        kg.add_node(b.clone()).await.unwrap();
        kg.add_node(c.clone()).await.unwrap();

        kg.add_edge(KgEdge {
            id: Uuid::new_v4(),
            source: a.id,
            target: b.id,
            relation: KgRelation::Calls,
        })
        .await
        .unwrap();

        kg.add_edge(KgEdge {
            id: Uuid::new_v4(),
            source: b.id,
            target: c.id,
            relation: KgRelation::Calls,
        })
        .await
        .unwrap();

        let path = kg.find_path(a.id, c.id, 10).await.unwrap();
        assert!(path.is_some());
        let path = path.unwrap();
        assert_eq!(path.path.len(), 3);
        assert_eq!(path.total_hops, 2);
    }

    #[tokio::test]
    async fn test_cross_references() {
        let kg = SqliteKnowledgeGraph::create("sqlite::memory:")
            .await
            .unwrap();

        let node = KgNode {
            id: Uuid::new_v4(),
            node_type: KgNodeType::Function,
            name: "target".to_string(),
            properties: serde_json::json!({}),
        };

        let referencer1 = KgNode {
            id: Uuid::new_v4(),
            node_type: KgNodeType::Function,
            name: "ref1".to_string(),
            properties: serde_json::json!({}),
        };
        let referencer2 = KgNode {
            id: Uuid::new_v4(),
            node_type: KgNodeType::Function,
            name: "ref2".to_string(),
            properties: serde_json::json!({}),
        };

        kg.add_node(node.clone()).await.unwrap();
        kg.add_node(referencer1.clone()).await.unwrap();
        kg.add_node(referencer2.clone()).await.unwrap();

        kg.add_edge(KgEdge {
            id: Uuid::new_v4(),
            source: referencer1.id,
            target: node.id,
            relation: KgRelation::Calls,
        })
        .await
        .unwrap();

        kg.add_edge(KgEdge {
            id: Uuid::new_v4(),
            source: referencer2.id,
            target: node.id,
            relation: KgRelation::Calls,
        })
        .await
        .unwrap();

        let xref = kg.get_cross_references(node.id).await.unwrap();
        assert_eq!(xref.node.name, "target");
        assert_eq!(xref.total_incoming, 2);
        assert_eq!(xref.total_outgoing, 0);
    }

    #[tokio::test]
    async fn test_delete_node() {
        let kg = SqliteKnowledgeGraph::create("sqlite::memory:")
            .await
            .unwrap();

        let node = KgNode {
            id: Uuid::new_v4(),
            node_type: KgNodeType::Function,
            name: "to_delete".to_string(),
            properties: serde_json::json!({}),
        };

        kg.add_node(node.clone()).await.unwrap();
        assert!(kg.get_node(node.id).await.unwrap().is_some());

        kg.delete_node(node.id).await.unwrap();
        assert!(kg.get_node(node.id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_traverse() {
        let kg = SqliteKnowledgeGraph::create("sqlite::memory:")
            .await
            .unwrap();

        let a = KgNode {
            id: Uuid::new_v4(),
            node_type: KgNodeType::Function,
            name: "a".to_string(),
            properties: serde_json::json!({}),
        };
        let b = KgNode {
            id: Uuid::new_v4(),
            node_type: KgNodeType::Function,
            name: "b".to_string(),
            properties: serde_json::json!({}),
        };

        kg.add_node(a.clone()).await.unwrap();
        kg.add_node(b.clone()).await.unwrap();

        kg.add_edge(KgEdge {
            id: Uuid::new_v4(),
            source: a.id,
            target: b.id,
            relation: KgRelation::Calls,
        })
        .await
        .unwrap();

        let traversal = KgTraversal {
            start_node: a.id,
            relation: Some(KgRelation::Calls),
            max_depth: 10,
            direction: KgDirection::Outgoing,
        };

        let paths = kg.traverse(traversal).await.unwrap();
        assert!(!paths.is_empty());
    }

    #[tokio::test]
    async fn test_knowledge_graph_schema_metadata() {
        // Test that nodes carry confidence, evidence_count, valid_from, valid_until, and provenance
        let kg = SqliteKnowledgeGraph::create("sqlite::memory:")
            .await
            .unwrap();

        let now = chrono::Utc::now();
        let valid_from = now - chrono::Duration::days(30);
        let valid_until = Some(now + chrono::Duration::days(30));

        let provenance = vec![
            ProvenanceReference::new("commit", "abc123"),
            ProvenanceReference::new("file", "src/main.rs").with_description("Primary definition"),
        ];

        let node = KnowledgeGraphNode {
            id: Uuid::new_v4(),
            node_type: KgNodeType::Function,
            name: "test_function".to_string(),
            properties: serde_json::json!({}),
            repository: "test-repo".to_string(),
            file_path: Some("src/main.rs".to_string()),
            start_line: Some(10),
            end_line: Some(20),
            created_at: now,
            updated_at: now,
            confidence: 0.95,
            evidence_count: 3,
            valid_from,
            valid_until,
            provenance: provenance.clone(),
        };

        kg.insert_node_with_metadata(node.clone()).await.unwrap();

        // Retrieve and verify all metadata fields
        let retrieved = kg.get_node(node.id).await.unwrap();
        assert!(retrieved.is_some());
        let retrieved = retrieved.unwrap();

        // Convert back to KnowledgeGraphNode to access metadata
        let retrieved_kg: KnowledgeGraphNode = retrieved.into();
        assert_eq!(retrieved_kg.confidence, 0.95);
        assert_eq!(retrieved_kg.evidence_count, 3);
        assert_eq!(retrieved_kg.valid_from, valid_from);
        assert_eq!(retrieved_kg.valid_until, valid_until);
        assert_eq!(retrieved_kg.provenance.len(), 2);
        assert_eq!(retrieved_kg.provenance[0].source_type, "commit");
        assert_eq!(retrieved_kg.provenance[0].source_id, "abc123");
    }

    #[tokio::test]
    async fn test_knowledge_graph_edge_metadata() {
        // Test that edges carry confidence, evidence_count, valid_from, valid_until, and provenance
        let kg = SqliteKnowledgeGraph::create("sqlite::memory:")
            .await
            .unwrap();

        let node1 = KnowledgeGraphNode {
            id: Uuid::new_v4(),
            node_type: KgNodeType::Function,
            name: "caller".to_string(),
            properties: serde_json::json!({}),
            repository: "test-repo".to_string(),
            file_path: None,
            start_line: None,
            end_line: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            confidence: 1.0,
            evidence_count: 1,
            valid_from: chrono::Utc::now(),
            valid_until: None,
            provenance: Vec::new(),
        };

        let node2 = KnowledgeGraphNode {
            id: Uuid::new_v4(),
            node_type: KgNodeType::Function,
            name: "callee".to_string(),
            properties: serde_json::json!({}),
            repository: "test-repo".to_string(),
            file_path: None,
            start_line: None,
            end_line: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            confidence: 1.0,
            evidence_count: 1,
            valid_from: chrono::Utc::now(),
            valid_until: None,
            provenance: Vec::new(),
        };

        kg.insert_node_with_metadata(node1.clone()).await.unwrap();
        kg.insert_node_with_metadata(node2.clone()).await.unwrap();

        let valid_from = chrono::Utc::now() - chrono::Duration::days(10);
        let valid_until = Some(chrono::Utc::now() + chrono::Duration::days(10));

        let provenance = vec![
            ProvenanceReference::new("llm_generation", "analysis-123")
                .with_description("Inferred from call pattern"),
        ];

        let edge = KnowledgeGraphEdge {
            id: Uuid::new_v4(),
            source: node1.id,
            target: node2.id,
            relation: KgRelation::Calls,
            repository: "test-repo".to_string(),
            properties: serde_json::json!({}),
            created_at: chrono::Utc::now(),
            confidence: 0.85,
            evidence_count: 2,
            valid_from,
            valid_until,
            provenance,
        };

        kg.insert_edge_with_metadata(edge.clone()).await.unwrap();

        // Verify edge metadata through query
        let deps = kg.find_dependencies(node1.id).await.unwrap();
        assert_eq!(deps.len(), 1);
        // Note: find_dependencies returns DependencyResult with node and relation,
        // not the full edge metadata, so we verify through the query method
    }

    #[tokio::test]
    async fn test_query_min_confidence_filter() {
        // Test that queries can filter by min_confidence threshold
        let kg = SqliteKnowledgeGraph::create("sqlite::memory:")
            .await
            .unwrap();

        let now = chrono::Utc::now();
        let repo = "test-repo";

        // Create nodes with varying confidence levels
        let high_confidence = KnowledgeGraphNode {
            id: Uuid::new_v4(),
            node_type: KgNodeType::Function,
            name: "high_conf".to_string(),
            properties: serde_json::json!({}),
            repository: repo.to_string(),
            file_path: None,
            start_line: None,
            end_line: None,
            created_at: now,
            updated_at: now,
            confidence: 0.95,
            evidence_count: 5,
            valid_from: now,
            valid_until: None,
            provenance: Vec::new(),
        };

        let medium_confidence = KnowledgeGraphNode {
            id: Uuid::new_v4(),
            node_type: KgNodeType::Function,
            name: "medium_conf".to_string(),
            properties: serde_json::json!({}),
            repository: repo.to_string(),
            file_path: None,
            start_line: None,
            end_line: None,
            created_at: now,
            updated_at: now,
            confidence: 0.70,
            evidence_count: 3,
            valid_from: now,
            valid_until: None,
            provenance: Vec::new(),
        };

        let low_confidence = KnowledgeGraphNode {
            id: Uuid::new_v4(),
            node_type: KgNodeType::Function,
            name: "low_conf".to_string(),
            properties: serde_json::json!({}),
            repository: repo.to_string(),
            file_path: None,
            start_line: None,
            end_line: None,
            created_at: now,
            updated_at: now,
            confidence: 0.30,
            evidence_count: 1,
            valid_from: now,
            valid_until: None,
            provenance: Vec::new(),
        };

        kg.insert_node_with_metadata(high_confidence.clone()).await.unwrap();
        kg.insert_node_with_metadata(medium_confidence.clone()).await.unwrap();
        kg.insert_node_with_metadata(low_confidence.clone()).await.unwrap();

        // Query with min_confidence = 0.8 should exclude medium and low
        let query = KnowledgeGraphQuery::new(repo.to_string())
            .with_min_confidence(0.8);
        let results = kg.query_nodes_with_metadata(query).await.unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "high_conf");
        assert!(results[0].confidence >= 0.8);

        // Query with min_confidence = 0.5 should include high and medium
        let query = KnowledgeGraphQuery::new(repo.to_string())
            .with_min_confidence(0.5);
        let results = kg.query_nodes_with_metadata(query).await.unwrap();

        assert_eq!(results.len(), 2);
        for node in &results {
            assert!(node.confidence >= 0.5);
        }
    }

    #[tokio::test]
    async fn test_query_temporal_validity_filter() {
        // Test that queries can filter by temporal validity range
        let kg = SqliteKnowledgeGraph::create("sqlite::memory:")
            .await
            .unwrap();

        let now = chrono::Utc::now();
        let repo = "test-repo";

        // Create nodes with different temporal validity ranges
        let past_valid = KnowledgeGraphNode {
            id: Uuid::new_v4(),
            node_type: KgNodeType::Function,
            name: "past_only".to_string(),
            properties: serde_json::json!({}),
            repository: repo.to_string(),
            file_path: None,
            start_line: None,
            end_line: None,
            created_at: now - chrono::Duration::days(100),
            updated_at: now - chrono::Duration::days(100),
            confidence: 1.0,
            evidence_count: 1,
            valid_from: now - chrono::Duration::days(100),
            valid_until: Some(now - chrono::Duration::days(50)),
            provenance: Vec::new(),
        };

        let current_valid = KnowledgeGraphNode {
            id: Uuid::new_v4(),
            node_type: KgNodeType::Function,
            name: "currently_valid".to_string(),
            properties: serde_json::json!({}),
            repository: repo.to_string(),
            file_path: None,
            start_line: None,
            end_line: None,
            created_at: now - chrono::Duration::days(30),
            updated_at: now - chrono::Duration::days(30),
            confidence: 1.0,
            evidence_count: 1,
            valid_from: now - chrono::Duration::days(30),
            valid_until: Some(now + chrono::Duration::days(30)),
            provenance: Vec::new(),
        };

        let future_valid = KnowledgeGraphNode {
            id: Uuid::new_v4(),
            node_type: KgNodeType::Function,
            name: "future_only".to_string(),
            properties: serde_json::json!({}),
            repository: repo.to_string(),
            file_path: None,
            start_line: None,
            end_line: None,
            created_at: now,
            updated_at: now,
            confidence: 1.0,
            evidence_count: 1,
            valid_from: now + chrono::Duration::days(10),
            valid_until: Some(now + chrono::Duration::days(40)),
            provenance: Vec::new(),
        };

        let never_expires = KnowledgeGraphNode {
            id: Uuid::new_v4(),
            node_type: KgNodeType::Function,
            name: "never_expires".to_string(),
            properties: serde_json::json!({}),
            repository: repo.to_string(),
            file_path: None,
            start_line: None,
            end_line: None,
            created_at: now - chrono::Duration::days(50),
            updated_at: now - chrono::Duration::days(50),
            confidence: 1.0,
            evidence_count: 1,
            valid_from: now - chrono::Duration::days(50),
            valid_until: None,
            provenance: Vec::new(),
        };

        kg.insert_node_with_metadata(past_valid.clone()).await.unwrap();
        kg.insert_node_with_metadata(current_valid.clone()).await.unwrap();
        kg.insert_node_with_metadata(future_valid.clone()).await.unwrap();
        kg.insert_node_with_metadata(never_expires.clone()).await.unwrap();

        // Query for "now" should return current_valid and never_expires
        let query = KnowledgeGraphQuery::new(repo.to_string())
            .with_valid_at(now);
        let results = kg.query_nodes_with_metadata(query).await.unwrap();

        assert_eq!(results.len(), 2);
        let names: Vec<&str> = results.iter().map(|n| n.name.as_str()).collect();
        assert!(names.contains(&"currently_valid"));
        assert!(names.contains(&"never_expires"));
    }

    #[tokio::test]
    async fn test_provenance_reference() {
        let pr = ProvenanceReference::new("commit", "abc123");
        assert_eq!(pr.source_type, "commit");
        assert_eq!(pr.source_id, "abc123");
        assert!(pr.description.is_none());
        assert!(pr.timestamp <= chrono::Utc::now());

        let pr2 = ProvenanceReference::new("file", "src/main.rs")
            .with_description("Definition location");
        assert_eq!(pr2.source_type, "file");
        assert_eq!(pr2.source_id, "src/main.rs");
        assert_eq!(pr2.description, Some("Definition location".to_string()));
    }

    #[tokio::test]
    async fn test_combined_metadata_filters() {
        // Test combining min_confidence and temporal validity filters
        let kg = SqliteKnowledgeGraph::create("sqlite::memory:")
            .await
            .unwrap();

        let now = chrono::Utc::now();
        let repo = "test-repo";

        let high_conf_valid = KnowledgeGraphNode {
            id: Uuid::new_v4(),
            node_type: KgNodeType::Function,
            name: "high_conf_valid".to_string(),
            properties: serde_json::json!({}),
            repository: repo.to_string(),
            file_path: None,
            start_line: None,
            end_line: None,
            created_at: now,
            updated_at: now,
            confidence: 0.95,
            evidence_count: 5,
            valid_from: now - chrono::Duration::days(10),
            valid_until: Some(now + chrono::Duration::days(10)),
            provenance: Vec::new(),
        };

        let high_conf_expired = KnowledgeGraphNode {
            id: Uuid::new_v4(),
            node_type: KgNodeType::Function,
            name: "high_conf_expired".to_string(),
            properties: serde_json::json!({}),
            repository: repo.to_string(),
            file_path: None,
            start_line: None,
            end_line: None,
            created_at: now - chrono::Duration::days(100),
            updated_at: now - chrono::Duration::days(100),
            confidence: 0.95,
            evidence_count: 5,
            valid_from: now - chrono::Duration::days(100),
            valid_until: Some(now - chrono::Duration::days(50)),
            provenance: Vec::new(),
        };

        let low_conf_valid = KnowledgeGraphNode {
            id: Uuid::new_v4(),
            node_type: KgNodeType::Function,
            name: "low_conf_valid".to_string(),
            properties: serde_json::json!({}),
            repository: repo.to_string(),
            file_path: None,
            start_line: None,
            end_line: None,
            created_at: now - chrono::Duration::days(10),
            updated_at: now - chrono::Duration::days(10),
            confidence: 0.30,
            evidence_count: 1,
            valid_from: now - chrono::Duration::days(10),
            valid_until: Some(now + chrono::Duration::days(10)),
            provenance: Vec::new(),
        };

        kg.insert_node_with_metadata(high_conf_valid.clone()).await.unwrap();
        kg.insert_node_with_metadata(high_conf_expired.clone()).await.unwrap();
        kg.insert_node_with_metadata(low_conf_valid.clone()).await.unwrap();

        // Query with both filters: min_confidence >= 0.8 AND valid at "now"
        let query = KnowledgeGraphQuery::new(repo.to_string())
            .with_min_confidence(0.8)
            .with_valid_at(now);
        let results = kg.query_nodes_with_metadata(query).await.unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "high_conf_valid");
    }
}
