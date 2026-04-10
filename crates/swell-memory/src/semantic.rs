// semantic.rs - Semantic memory for facts, entities, and relationships
//
// This module provides semantic knowledge representation using a property graph
// where entities are nodes with types and relationships are typed edges.
//
// Semantic memory stores "what is true" - facts about the project such as:
// - "ProjectAlpha uses React 18 with Next.js 14"
// - "The payments module depends on stripe-node v12"
// - "Running make test requires PostgreSQL on port 5433"
//
// Unlike the AST-based knowledge graph (KgNode/KgEdge) which represents code
// structure, semantic memory stores general-purpose facts and their relationships.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::sqlite::{SqlitePool, SqliteRow};
use sqlx::Row;
use std::sync::Arc;
use uuid::Uuid;

use swell_core::SwellError;

/// Entity types for semantic memory
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticEntityType {
    Project,
    Module,
    Function,
    Class,
    Method,
    Variable,
    Configuration,
    Dependency,
    Convention,
    Requirement,
    Task,
    Skill,
    Tool,
    File,
    Package,
    Api,
    Error,
    Test,
}

impl SemanticEntityType {
    pub fn as_str(&self) -> &'static str {
        match self {
            SemanticEntityType::Project => "project",
            SemanticEntityType::Module => "module",
            SemanticEntityType::Function => "function",
            SemanticEntityType::Class => "class",
            SemanticEntityType::Method => "method",
            SemanticEntityType::Variable => "variable",
            SemanticEntityType::Configuration => "configuration",
            SemanticEntityType::Dependency => "dependency",
            SemanticEntityType::Convention => "convention",
            SemanticEntityType::Requirement => "requirement",
            SemanticEntityType::Task => "task",
            SemanticEntityType::Skill => "skill",
            SemanticEntityType::Tool => "tool",
            SemanticEntityType::File => "file",
            SemanticEntityType::Package => "package",
            SemanticEntityType::Api => "api",
            SemanticEntityType::Error => "error",
            SemanticEntityType::Test => "test",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "project" => SemanticEntityType::Project,
            "module" => SemanticEntityType::Module,
            "function" => SemanticEntityType::Function,
            "class" => SemanticEntityType::Class,
            "method" => SemanticEntityType::Method,
            "variable" => SemanticEntityType::Variable,
            "configuration" => SemanticEntityType::Configuration,
            "dependency" => SemanticEntityType::Dependency,
            "convention" => SemanticEntityType::Convention,
            "requirement" => SemanticEntityType::Requirement,
            "task" => SemanticEntityType::Task,
            "skill" => SemanticEntityType::Skill,
            "tool" => SemanticEntityType::Tool,
            "file" => SemanticEntityType::File,
            "package" => SemanticEntityType::Package,
            "api" => SemanticEntityType::Api,
            "error" => SemanticEntityType::Error,
            "test" => SemanticEntityType::Test,
            _ => SemanticEntityType::Project,
        }
    }
}

/// A semantic entity (node in the knowledge graph)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticEntity {
    pub id: Uuid,
    pub entity_type: SemanticEntityType,
    pub name: String,
    pub properties: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl SemanticEntity {
    /// Create a new semantic entity
    pub fn new(entity_type: SemanticEntityType, name: String) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            entity_type,
            name,
            properties: serde_json::json!({}),
            created_at: now,
            updated_at: now,
        }
    }

    /// Create with properties
    pub fn with_properties(
        entity_type: SemanticEntityType,
        name: String,
        properties: serde_json::Value,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            entity_type,
            name,
            properties,
            created_at: now,
            updated_at: now,
        }
    }
}

/// Relationship types for semantic memory
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticRelationType {
    DependsOn,
    Implements,
    Contains,
    References,
    Configures,
    Requires,
    Tests,
    Fixes,
    Breaks,
    Documents,
    Calls,
    InheritsFrom,
    Uses,
    Manages,
}

impl SemanticRelationType {
    pub fn as_str(&self) -> &'static str {
        match self {
            SemanticRelationType::DependsOn => "depends_on",
            SemanticRelationType::Implements => "implements",
            SemanticRelationType::Contains => "contains",
            SemanticRelationType::References => "references",
            SemanticRelationType::Configures => "configures",
            SemanticRelationType::Requires => "requires",
            SemanticRelationType::Tests => "tests",
            SemanticRelationType::Fixes => "fixes",
            SemanticRelationType::Breaks => "breaks",
            SemanticRelationType::Documents => "documents",
            SemanticRelationType::Calls => "calls",
            SemanticRelationType::InheritsFrom => "inherits_from",
            SemanticRelationType::Uses => "uses",
            SemanticRelationType::Manages => "manages",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "depends_on" => SemanticRelationType::DependsOn,
            "implements" => SemanticRelationType::Implements,
            "contains" => SemanticRelationType::Contains,
            "references" => SemanticRelationType::References,
            "configures" => SemanticRelationType::Configures,
            "requires" => SemanticRelationType::Requires,
            "tests" => SemanticRelationType::Tests,
            "fixes" => SemanticRelationType::Fixes,
            "breaks" => SemanticRelationType::Breaks,
            "documents" => SemanticRelationType::Documents,
            "calls" => SemanticRelationType::Calls,
            "inherits_from" => SemanticRelationType::InheritsFrom,
            "uses" => SemanticRelationType::Uses,
            "manages" => SemanticRelationType::Manages,
            _ => SemanticRelationType::References,
        }
    }
}

/// A semantic relationship (edge in the knowledge graph)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticRelation {
    pub id: Uuid,
    pub relation_type: SemanticRelationType,
    pub source_id: Uuid,
    pub target_id: Uuid,
    pub properties: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

impl SemanticRelation {
    /// Create a new semantic relation
    pub fn new(relation_type: SemanticRelationType, source_id: Uuid, target_id: Uuid) -> Self {
        Self {
            id: Uuid::new_v4(),
            relation_type,
            source_id,
            target_id,
            properties: serde_json::json!({}),
            created_at: Utc::now(),
        }
    }

    /// Create with properties
    pub fn with_properties(
        relation_type: SemanticRelationType,
        source_id: Uuid,
        target_id: Uuid,
        properties: serde_json::Value,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            relation_type,
            source_id,
            target_id,
            properties,
            created_at: Utc::now(),
        }
    }
}

/// Query for semantic entities
#[derive(Debug, Clone, Default)]
pub struct SemanticEntityQuery {
    pub entity_types: Option<Vec<SemanticEntityType>>,
    pub name_contains: Option<String>,
    pub limit: usize,
    pub offset: usize,
}

impl SemanticEntityQuery {
    pub fn new() -> Self {
        Self {
            entity_types: None,
            name_contains: None,
            limit: 100,
            offset: 0,
        }
    }

    pub fn with_types(mut self, types: Vec<SemanticEntityType>) -> Self {
        self.entity_types = Some(types);
        self
    }

    pub fn with_name_contains(mut self, name: String) -> Self {
        self.name_contains = Some(name);
        self
    }

    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }
}

/// Query for semantic relations
#[derive(Debug, Clone, Default)]
pub struct SemanticRelationQuery {
    pub relation_types: Option<Vec<SemanticRelationType>>,
    pub source_id: Option<Uuid>,
    pub target_id: Option<Uuid>,
    pub limit: usize,
    pub offset: usize,
}

impl SemanticRelationQuery {
    pub fn new() -> Self {
        Self {
            relation_types: None,
            source_id: None,
            target_id: None,
            limit: 100,
            offset: 0,
        }
    }

    pub fn with_relation_types(mut self, types: Vec<SemanticRelationType>) -> Self {
        self.relation_types = Some(types);
        self
    }

    pub fn with_source(mut self, source_id: Uuid) -> Self {
        self.source_id = Some(source_id);
        self
    }

    pub fn with_target(mut self, target_id: Uuid) -> Self {
        self.target_id = Some(target_id);
        self
    }

    pub fn with_limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }
}

/// Result of querying semantic relations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticRelationResult {
    pub relation: SemanticRelation,
    pub source: SemanticEntity,
    pub target: SemanticEntity,
}

/// SQLite-based semantic memory store
#[derive(Clone)]
pub struct SqliteSemanticStore {
    pool: Arc<SqlitePool>,
}

impl SqliteSemanticStore {
    /// Create a new SqliteSemanticStore with the given database URL
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

    /// Initialize the database schema for semantic memory
    async fn init_schema(pool: &SqlitePool) -> Result<(), SwellError> {
        // Semantic entities table
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS semantic_entities (
                id TEXT PRIMARY KEY,
                entity_type TEXT NOT NULL,
                name TEXT NOT NULL,
                properties TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            )
            "#,
        )
        .execute(pool)
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        // Index on entity_type for efficient type queries
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_semantic_entity_type ON semantic_entities(entity_type)",
        )
        .execute(pool)
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        // Index on name for search
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_semantic_entity_name ON semantic_entities(name)",
        )
        .execute(pool)
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        // Semantic relations table
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS semantic_relations (
                id TEXT PRIMARY KEY,
                relation_type TEXT NOT NULL,
                source_id TEXT NOT NULL,
                target_id TEXT NOT NULL,
                properties TEXT NOT NULL,
                created_at TEXT NOT NULL,
                FOREIGN KEY (source_id) REFERENCES semantic_entities(id),
                FOREIGN KEY (target_id) REFERENCES semantic_entities(id)
            )
            "#,
        )
        .execute(pool)
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        // Index on relation_type for efficient type queries
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_semantic_relation_type ON semantic_relations(relation_type)",
        )
        .execute(pool)
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        // Index on source_id for outgoing edge queries
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_semantic_relation_source ON semantic_relations(source_id)",
        )
        .execute(pool)
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        // Index on target_id for incoming edge queries
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS idx_semantic_relation_target ON semantic_relations(target_id)",
        )
        .execute(pool)
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        Ok(())
    }

    /// Convert a database row to a SemanticEntity
    fn row_to_entity(row: &SqliteRow) -> Result<SemanticEntity, SwellError> {
        let id_str: String = row.get("id");
        let id = Uuid::parse_str(&id_str)
            .map_err(|e| SwellError::DatabaseError(format!("Invalid UUID: {}", e)))?;
        let entity_type_str: String = row.get("entity_type");
        let name: String = row.get("name");
        let properties_str: String = row.get("properties");
        let created_at_str: String = row.get("created_at");
        let updated_at_str: String = row.get("updated_at");

        let created_at = chrono::DateTime::parse_from_rfc3339(&created_at_str)
            .map_err(|e| SwellError::DatabaseError(format!("Invalid timestamp: {}", e)))?
            .with_timezone(&chrono::Utc);
        let updated_at = chrono::DateTime::parse_from_rfc3339(&updated_at_str)
            .map_err(|e| SwellError::DatabaseError(format!("Invalid timestamp: {}", e)))?
            .with_timezone(&chrono::Utc);
        let properties: serde_json::Value = serde_json::from_str(&properties_str)
            .map_err(|e| SwellError::DatabaseError(format!("Invalid JSON properties: {}", e)))?;

        Ok(SemanticEntity {
            id,
            entity_type: SemanticEntityType::parse(&entity_type_str),
            name,
            properties,
            created_at,
            updated_at,
        })
    }

    /// Convert a database row to a SemanticRelation
    fn row_to_relation(row: &SqliteRow) -> Result<SemanticRelation, SwellError> {
        let id_str: String = row.get("id");
        let id = Uuid::parse_str(&id_str)
            .map_err(|e| SwellError::DatabaseError(format!("Invalid UUID: {}", e)))?;
        let relation_type_str: String = row.get("relation_type");
        let source_id_str: String = row.get("source_id");
        let target_id_str: String = row.get("target_id");
        let properties_str: String = row.get("properties");
        let created_at_str: String = row.get("created_at");

        let source_id = Uuid::parse_str(&source_id_str)
            .map_err(|e| SwellError::DatabaseError(format!("Invalid UUID: {}", e)))?;
        let target_id = Uuid::parse_str(&target_id_str)
            .map_err(|e| SwellError::DatabaseError(format!("Invalid UUID: {}", e)))?;
        let properties: serde_json::Value = serde_json::from_str(&properties_str)
            .map_err(|e| SwellError::DatabaseError(format!("Invalid JSON properties: {}", e)))?;
        let created_at = chrono::DateTime::parse_from_rfc3339(&created_at_str)
            .map_err(|e| SwellError::DatabaseError(format!("Invalid timestamp: {}", e)))?
            .with_timezone(&chrono::Utc);

        Ok(SemanticRelation {
            id,
            relation_type: SemanticRelationType::parse(&relation_type_str),
            source_id,
            target_id,
            properties,
            created_at,
        })
    }
}

#[async_trait]
impl SemanticStore for SqliteSemanticStore {
    /// Store a semantic entity
    async fn store_entity(&self, entity: SemanticEntity) -> Result<Uuid, SwellError> {
        let properties_str = serde_json::to_string(&entity.properties)
            .map_err(|e| SwellError::DatabaseError(e.to_string()))?;
        let created_at_str = entity.created_at.to_rfc3339();
        let updated_at_str = entity.updated_at.to_rfc3339();

        sqlx::query(
            r#"
            INSERT INTO semantic_entities (id, entity_type, name, properties, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(entity.id.to_string())
        .bind(entity.entity_type.as_str())
        .bind(&entity.name)
        .bind(&properties_str)
        .bind(&created_at_str)
        .bind(&updated_at_str)
        .execute(self.pool.as_ref())
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        Ok(entity.id)
    }

    /// Retrieve an entity by ID
    async fn get_entity(&self, id: Uuid) -> Result<Option<SemanticEntity>, SwellError> {
        let row = sqlx::query("SELECT * FROM semantic_entities WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(self.pool.as_ref())
            .await
            .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        match row {
            Some(r) => Ok(Some(Self::row_to_entity(&r)?)),
            None => Ok(None),
        }
    }

    /// Update an entity
    async fn update_entity(&self, entity: SemanticEntity) -> Result<(), SwellError> {
        let properties_str = serde_json::to_string(&entity.properties)
            .map_err(|e| SwellError::DatabaseError(e.to_string()))?;
        let updated_at_str = chrono::Utc::now().to_rfc3339();

        let result = sqlx::query(
            r#"
            UPDATE semantic_entities
            SET entity_type = ?, name = ?, properties = ?, updated_at = ?
            WHERE id = ?
            "#,
        )
        .bind(entity.entity_type.as_str())
        .bind(&entity.name)
        .bind(&properties_str)
        .bind(&updated_at_str)
        .bind(entity.id.to_string())
        .execute(self.pool.as_ref())
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(SwellError::DatabaseError(format!(
                "No entity found with id {}",
                entity.id
            )));
        }

        Ok(())
    }

    /// Delete an entity (and its relations)
    async fn delete_entity(&self, id: Uuid) -> Result<(), SwellError> {
        // First delete all relations involving this entity
        sqlx::query("DELETE FROM semantic_relations WHERE source_id = ? OR target_id = ?")
            .bind(id.to_string())
            .bind(id.to_string())
            .execute(self.pool.as_ref())
            .await
            .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        // Then delete the entity
        let result = sqlx::query("DELETE FROM semantic_entities WHERE id = ?")
            .bind(id.to_string())
            .execute(self.pool.as_ref())
            .await
            .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(SwellError::DatabaseError(format!(
                "No entity found with id {}",
                id
            )));
        }

        Ok(())
    }

    /// Query entities by type and/or name
    async fn query_entities(
        &self,
        query: SemanticEntityQuery,
    ) -> Result<Vec<SemanticEntity>, SwellError> {
        let mut sql = String::from("SELECT * FROM semantic_entities WHERE 1=1");
        let mut params: Vec<String> = Vec::new();

        if let Some(ref types) = query.entity_types {
            if !types.is_empty() {
                let placeholders: Vec<String> = types.iter().map(|_| "?".to_string()).collect();
                sql.push_str(&format!(
                    " AND entity_type IN ({})",
                    placeholders.join(", ")
                ));
                for t in types {
                    params.push(t.as_str().to_string());
                }
            }
        }

        if let Some(ref name_contains) = query.name_contains {
            sql.push_str(" AND name LIKE ?");
            params.push(format!("%{}%", name_contains));
        }

        sql.push_str(&format!(" LIMIT {} OFFSET {}", query.limit, query.offset));

        let mut q = sqlx::query(&sql);
        for param in &params {
            q = q.bind(param);
        }

        let rows = q
            .fetch_all(self.pool.as_ref())
            .await
            .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        let mut entities = Vec::new();
        for row in rows {
            entities.push(Self::row_to_entity(&row)?);
        }

        Ok(entities)
    }

    /// Store a semantic relation
    async fn store_relation(&self, relation: SemanticRelation) -> Result<Uuid, SwellError> {
        let properties_str = serde_json::to_string(&relation.properties)
            .map_err(|e| SwellError::DatabaseError(e.to_string()))?;
        let created_at_str = relation.created_at.to_rfc3339();

        sqlx::query(
            r#"
            INSERT INTO semantic_relations (id, relation_type, source_id, target_id, properties, created_at)
            VALUES (?, ?, ?, ?, ?, ?)
            "#,
        )
        .bind(relation.id.to_string())
        .bind(relation.relation_type.as_str())
        .bind(relation.source_id.to_string())
        .bind(relation.target_id.to_string())
        .bind(&properties_str)
        .bind(&created_at_str)
        .execute(self.pool.as_ref())
        .await
        .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        Ok(relation.id)
    }

    /// Retrieve a relation by ID
    async fn get_relation(&self, id: Uuid) -> Result<Option<SemanticRelation>, SwellError> {
        let row = sqlx::query("SELECT * FROM semantic_relations WHERE id = ?")
            .bind(id.to_string())
            .fetch_optional(self.pool.as_ref())
            .await
            .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        match row {
            Some(r) => Ok(Some(Self::row_to_relation(&r)?)),
            None => Ok(None),
        }
    }

    /// Delete a relation
    async fn delete_relation(&self, id: Uuid) -> Result<(), SwellError> {
        let result = sqlx::query("DELETE FROM semantic_relations WHERE id = ?")
            .bind(id.to_string())
            .execute(self.pool.as_ref())
            .await
            .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        if result.rows_affected() == 0 {
            return Err(SwellError::DatabaseError(format!(
                "No relation found with id {}",
                id
            )));
        }

        Ok(())
    }

    /// Query relations by type and/or endpoints
    async fn query_relations(
        &self,
        query: SemanticRelationQuery,
    ) -> Result<Vec<SemanticRelation>, SwellError> {
        let mut sql = String::from("SELECT * FROM semantic_relations WHERE 1=1");
        let mut params: Vec<String> = Vec::new();

        if let Some(ref types) = query.relation_types {
            if !types.is_empty() {
                let placeholders: Vec<String> = types.iter().map(|_| "?".to_string()).collect();
                sql.push_str(&format!(
                    " AND relation_type IN ({})",
                    placeholders.join(", ")
                ));
                for t in types {
                    params.push(t.as_str().to_string());
                }
            }
        }

        if let Some(ref source_id) = query.source_id {
            sql.push_str(" AND source_id = ?");
            params.push(source_id.to_string());
        }

        if let Some(ref target_id) = query.target_id {
            sql.push_str(" AND target_id = ?");
            params.push(target_id.to_string());
        }

        sql.push_str(&format!(" LIMIT {} OFFSET {}", query.limit, query.offset));

        let mut q = sqlx::query(&sql);
        for param in &params {
            q = q.bind(param);
        }

        let rows = q
            .fetch_all(self.pool.as_ref())
            .await
            .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        let mut relations = Vec::new();
        for row in rows {
            relations.push(Self::row_to_relation(&row)?);
        }

        Ok(relations)
    }

    /// Get all relations for an entity (both incoming and outgoing)
    async fn get_entity_relations(
        &self,
        entity_id: Uuid,
    ) -> Result<Vec<SemanticRelation>, SwellError> {
        let rows =
            sqlx::query("SELECT * FROM semantic_relations WHERE source_id = ? OR target_id = ?")
                .bind(entity_id.to_string())
                .bind(entity_id.to_string())
                .fetch_all(self.pool.as_ref())
                .await
                .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        let mut relations = Vec::new();
        for row in rows {
            relations.push(Self::row_to_relation(&row)?);
        }

        Ok(relations)
    }

    /// Get incoming relations for an entity (relations where entity is target)
    async fn get_incoming_relations(
        &self,
        entity_id: Uuid,
    ) -> Result<Vec<SemanticRelation>, SwellError> {
        let rows = sqlx::query("SELECT * FROM semantic_relations WHERE target_id = ?")
            .bind(entity_id.to_string())
            .fetch_all(self.pool.as_ref())
            .await
            .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        let mut relations = Vec::new();
        for row in rows {
            relations.push(Self::row_to_relation(&row)?);
        }

        Ok(relations)
    }

    /// Get outgoing relations for an entity (relations where entity is source)
    async fn get_outgoing_relations(
        &self,
        entity_id: Uuid,
    ) -> Result<Vec<SemanticRelation>, SwellError> {
        let rows = sqlx::query("SELECT * FROM semantic_relations WHERE source_id = ?")
            .bind(entity_id.to_string())
            .fetch_all(self.pool.as_ref())
            .await
            .map_err(|e: sqlx::Error| SwellError::DatabaseError(e.to_string()))?;

        let mut relations = Vec::new();
        for row in rows {
            relations.push(Self::row_to_relation(&row)?);
        }

        Ok(relations)
    }
}

/// Trait for semantic memory storage operations
#[async_trait]
pub trait SemanticStore: Send + Sync {
    /// Store a semantic entity
    async fn store_entity(&self, entity: SemanticEntity) -> Result<Uuid, SwellError>;

    /// Retrieve an entity by ID
    async fn get_entity(&self, id: Uuid) -> Result<Option<SemanticEntity>, SwellError>;

    /// Update an entity
    async fn update_entity(&self, entity: SemanticEntity) -> Result<(), SwellError>;

    /// Delete an entity (and its relations)
    async fn delete_entity(&self, id: Uuid) -> Result<(), SwellError>;

    /// Query entities by type and/or name
    async fn query_entities(
        &self,
        query: SemanticEntityQuery,
    ) -> Result<Vec<SemanticEntity>, SwellError>;

    /// Store a semantic relation
    async fn store_relation(&self, relation: SemanticRelation) -> Result<Uuid, SwellError>;

    /// Retrieve a relation by ID
    async fn get_relation(&self, id: Uuid) -> Result<Option<SemanticRelation>, SwellError>;

    /// Delete a relation
    async fn delete_relation(&self, id: Uuid) -> Result<(), SwellError>;

    /// Query relations by type and/or endpoints
    async fn query_relations(
        &self,
        query: SemanticRelationQuery,
    ) -> Result<Vec<SemanticRelation>, SwellError>;

    /// Get all relations for an entity (both incoming and outgoing)
    async fn get_entity_relations(
        &self,
        entity_id: Uuid,
    ) -> Result<Vec<SemanticRelation>, SwellError>;

    /// Get incoming relations for an entity
    async fn get_incoming_relations(
        &self,
        entity_id: Uuid,
    ) -> Result<Vec<SemanticRelation>, SwellError>;

    /// Get outgoing relations for an entity
    async fn get_outgoing_relations(
        &self,
        entity_id: Uuid,
    ) -> Result<Vec<SemanticRelation>, SwellError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_store_and_get_entity() {
        let store = SqliteSemanticStore::create("sqlite::memory:")
            .await
            .unwrap();

        let entity = SemanticEntity::new(SemanticEntityType::Project, "TestProject".to_string());

        let id = store.store_entity(entity.clone()).await.unwrap();
        assert_eq!(id, entity.id);

        let retrieved = store.get_entity(id).await.unwrap();
        assert!(retrieved.is_some());
        let retrieved = retrieved.unwrap();
        assert_eq!(retrieved.name, "TestProject");
        assert_eq!(retrieved.entity_type, SemanticEntityType::Project);
    }

    #[tokio::test]
    async fn test_query_entities_by_type() {
        let store = SqliteSemanticStore::create("sqlite::memory:")
            .await
            .unwrap();

        // Create entities of different types
        let project = SemanticEntity::new(SemanticEntityType::Project, "MyProject".to_string());
        let func = SemanticEntity::new(SemanticEntityType::Function, "my_function".to_string());
        let class = SemanticEntity::new(SemanticEntityType::Class, "MyClass".to_string());

        store.store_entity(project.clone()).await.unwrap();
        store.store_entity(func.clone()).await.unwrap();
        store.store_entity(class.clone()).await.unwrap();

        // Query only project types
        let query = SemanticEntityQuery::new().with_types(vec![SemanticEntityType::Project]);
        let results = store.query_entities(query).await.unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, project.id);
    }

    #[tokio::test]
    async fn test_query_entities_by_multiple_types() {
        let store = SqliteSemanticStore::create("sqlite::memory:")
            .await
            .unwrap();

        let func = SemanticEntity::new(SemanticEntityType::Function, "my_function".to_string());
        let class = SemanticEntity::new(SemanticEntityType::Class, "MyClass".to_string());

        store.store_entity(func.clone()).await.unwrap();
        store.store_entity(class.clone()).await.unwrap();

        // Query Function and Class types
        let query = SemanticEntityQuery::new().with_types(vec![
            SemanticEntityType::Function,
            SemanticEntityType::Class,
        ]);
        let results = store.query_entities(query).await.unwrap();

        assert_eq!(results.len(), 2);
    }

    #[tokio::test]
    async fn test_query_entities_by_name() {
        let store = SqliteSemanticStore::create("sqlite::memory:")
            .await
            .unwrap();

        let entity1 = SemanticEntity::new(SemanticEntityType::Project, "MyProject".to_string());
        let entity2 = SemanticEntity::new(SemanticEntityType::Module, "MyModule".to_string());
        let entity3 =
            SemanticEntity::new(SemanticEntityType::Function, "other_function".to_string());

        store.store_entity(entity1.clone()).await.unwrap();
        store.store_entity(entity2.clone()).await.unwrap();
        store.store_entity(entity3.clone()).await.unwrap();

        // Query names containing "My"
        let query = SemanticEntityQuery::new().with_name_contains("My".to_string());
        let results = store.query_entities(query).await.unwrap();

        assert_eq!(results.len(), 2);
    }

    #[tokio::test]
    async fn test_store_and_get_relation() {
        let store = SqliteSemanticStore::create("sqlite::memory:")
            .await
            .unwrap();

        // Create two entities
        let entity1 = SemanticEntity::new(SemanticEntityType::Module, "ModuleA".to_string());
        let entity2 =
            SemanticEntity::new(SemanticEntityType::Dependency, "stripe-node".to_string());

        store.store_entity(entity1.clone()).await.unwrap();
        store.store_entity(entity2.clone()).await.unwrap();

        // Create a relation between them
        let relation =
            SemanticRelation::new(SemanticRelationType::DependsOn, entity1.id, entity2.id);

        let id = store.store_relation(relation.clone()).await.unwrap();
        assert_eq!(id, relation.id);

        let retrieved = store.get_relation(id).await.unwrap();
        assert!(retrieved.is_some());
        let retrieved = retrieved.unwrap();
        assert_eq!(retrieved.relation_type, SemanticRelationType::DependsOn);
        assert_eq!(retrieved.source_id, entity1.id);
        assert_eq!(retrieved.target_id, entity2.id);
    }

    #[tokio::test]
    async fn test_query_relations_by_type() {
        let store = SqliteSemanticStore::create("sqlite::memory:")
            .await
            .unwrap();

        let entity1 = SemanticEntity::new(SemanticEntityType::Module, "ModuleA".to_string());
        let entity2 = SemanticEntity::new(SemanticEntityType::Module, "ModuleB".to_string());
        let entity3 = SemanticEntity::new(SemanticEntityType::Configuration, "ConfigA".to_string());

        store.store_entity(entity1.clone()).await.unwrap();
        store.store_entity(entity2.clone()).await.unwrap();
        store.store_entity(entity3.clone()).await.unwrap();

        // Create relations of different types
        let rel1 = SemanticRelation::new(SemanticRelationType::DependsOn, entity1.id, entity2.id);
        let rel2 = SemanticRelation::new(SemanticRelationType::Configures, entity3.id, entity1.id);

        store.store_relation(rel1.clone()).await.unwrap();
        store.store_relation(rel2.clone()).await.unwrap();

        // Query only DependsOn relations
        let query =
            SemanticRelationQuery::new().with_relation_types(vec![SemanticRelationType::DependsOn]);
        let results = store.query_relations(query).await.unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, rel1.id);
    }

    #[tokio::test]
    async fn test_query_relations_by_source() {
        let store = SqliteSemanticStore::create("sqlite::memory:")
            .await
            .unwrap();

        let entity1 = SemanticEntity::new(SemanticEntityType::Module, "ModuleA".to_string());
        let entity2 = SemanticEntity::new(SemanticEntityType::Dependency, "DepA".to_string());
        let entity3 = SemanticEntity::new(SemanticEntityType::Dependency, "DepB".to_string());

        store.store_entity(entity1.clone()).await.unwrap();
        store.store_entity(entity2.clone()).await.unwrap();
        store.store_entity(entity3.clone()).await.unwrap();

        // Create two relations from entity1
        let rel1 = SemanticRelation::new(SemanticRelationType::DependsOn, entity1.id, entity2.id);
        let rel2 = SemanticRelation::new(SemanticRelationType::References, entity1.id, entity3.id);

        store.store_relation(rel1.clone()).await.unwrap();
        store.store_relation(rel2.clone()).await.unwrap();

        // Query relations from entity1
        let query = SemanticRelationQuery::new().with_source(entity1.id);
        let results = store.query_relations(query).await.unwrap();

        assert_eq!(results.len(), 2);
    }

    #[tokio::test]
    async fn test_get_entity_relations() {
        let store = SqliteSemanticStore::create("sqlite::memory:")
            .await
            .unwrap();

        let entity1 = SemanticEntity::new(SemanticEntityType::Module, "ModuleA".to_string());
        let entity2 = SemanticEntity::new(SemanticEntityType::Dependency, "DepA".to_string());
        let entity3 = SemanticEntity::new(SemanticEntityType::Api, "ApiA".to_string());

        store.store_entity(entity1.clone()).await.unwrap();
        store.store_entity(entity2.clone()).await.unwrap();
        store.store_entity(entity3.clone()).await.unwrap();

        // Create relations: entity1 -> entity2 and entity3 -> entity1
        let rel1 = SemanticRelation::new(SemanticRelationType::DependsOn, entity1.id, entity2.id);
        let rel2 = SemanticRelation::new(SemanticRelationType::References, entity3.id, entity1.id);

        store.store_relation(rel1.clone()).await.unwrap();
        store.store_relation(rel2.clone()).await.unwrap();

        // Get all relations for entity1
        let relations = store.get_entity_relations(entity1.id).await.unwrap();

        assert_eq!(relations.len(), 2);
    }

    #[tokio::test]
    async fn test_get_incoming_outgoing_relations() {
        let store = SqliteSemanticStore::create("sqlite::memory:")
            .await
            .unwrap();

        let entity1 = SemanticEntity::new(SemanticEntityType::Module, "ModuleA".to_string());
        let entity2 = SemanticEntity::new(SemanticEntityType::Dependency, "DepA".to_string());
        let entity3 = SemanticEntity::new(SemanticEntityType::Api, "ApiA".to_string());

        store.store_entity(entity1.clone()).await.unwrap();
        store.store_entity(entity2.clone()).await.unwrap();
        store.store_entity(entity3.clone()).await.unwrap();

        // Create relations: entity1 -> entity2 and entity3 -> entity1
        let rel1 = SemanticRelation::new(SemanticRelationType::DependsOn, entity1.id, entity2.id);
        let rel2 = SemanticRelation::new(SemanticRelationType::References, entity3.id, entity1.id);

        store.store_relation(rel1.clone()).await.unwrap();
        store.store_relation(rel2.clone()).await.unwrap();

        // Get outgoing relations (entity1 -> ...)
        let outgoing = store.get_outgoing_relations(entity1.id).await.unwrap();
        assert_eq!(outgoing.len(), 1);
        assert_eq!(outgoing[0].id, rel1.id);

        // Get incoming relations (... -> entity1)
        let incoming = store.get_incoming_relations(entity1.id).await.unwrap();
        assert_eq!(incoming.len(), 1);
        assert_eq!(incoming[0].id, rel2.id);
    }

    #[tokio::test]
    async fn test_delete_entity_cascades_to_relations() {
        let store = SqliteSemanticStore::create("sqlite::memory:")
            .await
            .unwrap();

        let entity1 = SemanticEntity::new(SemanticEntityType::Module, "ModuleA".to_string());
        let entity2 = SemanticEntity::new(SemanticEntityType::Dependency, "DepA".to_string());

        store.store_entity(entity1.clone()).await.unwrap();
        store.store_entity(entity2.clone()).await.unwrap();

        // Create relation between them
        let relation =
            SemanticRelation::new(SemanticRelationType::DependsOn, entity1.id, entity2.id);
        store.store_relation(relation.clone()).await.unwrap();

        // Delete entity1
        store.delete_entity(entity1.id).await.unwrap();

        // Relation should be deleted too
        let retrieved = store.get_relation(relation.id).await.unwrap();
        assert!(retrieved.is_none());

        // entity2 should still exist
        let retrieved = store.get_entity(entity2.id).await.unwrap();
        assert!(retrieved.is_some());
    }

    #[tokio::test]
    async fn test_entity_with_properties() {
        let store = SqliteSemanticStore::create("sqlite::memory:")
            .await
            .unwrap();

        let entity = SemanticEntity::with_properties(
            SemanticEntityType::Dependency,
            "stripe-node".to_string(),
            serde_json::json!({
                "version": "12.0.0",
                "language": "JavaScript",
                "license": "MIT"
            }),
        );

        store.store_entity(entity.clone()).await.unwrap();

        let retrieved = store.get_entity(entity.id).await.unwrap().unwrap();
        assert_eq!(retrieved.properties["version"], "12.0.0");
        assert_eq!(retrieved.properties["language"], "JavaScript");
    }

    #[tokio::test]
    async fn test_update_entity() {
        let store = SqliteSemanticStore::create("sqlite::memory:")
            .await
            .unwrap();

        let mut entity = SemanticEntity::new(SemanticEntityType::Project, "OldName".to_string());

        store.store_entity(entity.clone()).await.unwrap();

        // Update entity
        entity.name = "NewName".to_string();
        entity.properties = serde_json::json!({"updated": true});
        store.update_entity(entity.clone()).await.unwrap();

        let retrieved = store.get_entity(entity.id).await.unwrap().unwrap();
        assert_eq!(retrieved.name, "NewName");
        assert_eq!(retrieved.properties["updated"], true);
    }
}
