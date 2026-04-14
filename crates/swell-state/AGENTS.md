# swell-state AGENTS.md

## Purpose

`swell-state` provides state management and checkpoint persistence for the SWELL autonomous coding engine. It offers a trait-based architecture that supports multiple storage backends (SQLite for MVP, PostgreSQL for production) with migration support and in-memory caching.

This crate handles:
- **CheckpointStore trait** — Persistent storage for task state snapshots
- **StateManager** — High-level state operations
- **SqliteCheckpointStore** — SQLite implementation for MVP
- **PostgresCheckpointStore** — PostgreSQL implementation for production
- **CheckpointManager** — Checkpoint lifecycle management
- **Migration support** — Schema versioning and upgrades

**Depends on:** `swell-core` (for `CheckpointStore` trait, `Checkpoint`, `SwellError`)

## Public API

### Checkpoint Store Traits

```rust
#[async_trait]
pub trait CheckpointStore: Send + Sync {
    async fn save(&self, checkpoint: Checkpoint) -> Result<Uuid, SwellError>;
    async fn load(&self, id: Uuid) -> Result<Option<Checkpoint>, SwellError>;
    async fn list(&self, task_id: Uuid) -> Result<Vec<Checkpoint>, SwellError>;
    async fn delete(&self, id: Uuid) -> Result<(), SwellError>;
    async fn latest(&self, task_id: Uuid) -> Result<Option<Checkpoint>, SwellError>;
}
```

### SQLite Implementation

```rust
pub struct SqliteCheckpointStore {
    pool: Arc<SqlitePool>,
}

impl SqliteCheckpointStore {
    pub async fn new(database_url: &str) -> Result<Self, SwellError>;
    pub async fn create(database_url: &str) -> Result<Self, SwellError>;
    pub async fn run_migrations(&self) -> Result<(), SwellError>;
}

#[async_trait]
impl CheckpointStore for SqliteCheckpointStore {
    async fn save(&self, checkpoint: Checkpoint) -> Result<Uuid, SwellError>;
    async fn load(&self, id: Uuid) -> Result<Option<Checkpoint>, SwellError>;
    async fn list(&self, task_id: Uuid) -> Result<Vec<Checkpoint>, SwellError>;
    async fn delete(&self, id: Uuid) -> Result<(), SwellError>;
    async fn latest(&self, task_id: Uuid) -> Result<Option<Checkpoint>, SwellError>;
}
```

### PostgreSQL Implementation

```rust
pub struct PostgresCheckpointStore {
    pool: Arc<PgPool>,
}

impl PostgresCheckpointStore {
    pub async fn new(database_url: &str) -> Result<Self, SwellError>;
    pub async fn create(database_url: &str) -> Result<Self, SwellError>;
    pub async fn run_migrations(&self) -> Result<(), SwellError>;
}

#[async_trait]
impl CheckpointStore for PostgresCheckpointStore {
    // Same interface as SqliteCheckpointStore
}
```

### In-Memory Store (Testing)

```rust
pub mod in_memory {
    pub struct InMemoryCheckpointStore {
        checkpoints: RwLock<HashMap<Uuid, Checkpoint>>,
        by_task: RwLock<HashMap<Uuid, Vec<Uuid>>>,
    }

    #[async_trait]
    impl CheckpointStore for InMemoryCheckpointStore {
        // Same interface as SQL implementations
    }
}
```

### State Manager

```rust
pub struct StateManager {
    store: Arc<dyn CheckpointStore>,
    cache: Arc<InMemoryCheckpointStore>,
}

impl StateManager {
    pub fn new(store: Arc<dyn CheckpointStore>) -> Self;
    pub async fn save_checkpoint(&self, task_id: Uuid, state: TaskState, snapshot: Value) -> Result<Uuid, SwellError>;
    pub async fn load_checkpoint(&self, id: Uuid) -> Result<Option<Checkpoint>, SwellError>;
    pub async fn list_checkpoints(&self, task_id: Uuid) -> Result<Vec<Checkpoint>, SwellError>;
    pub async fn delete_checkpoint(&self, id: Uuid) -> Result<(), SwellError>;
    pub async fn latest_checkpoint(&self, task_id: Uuid) -> Result<Option<Checkpoint>, SwellError>;
}
```

### Checkpoint Manager

```rust
pub struct CheckpointManager {
    store: Arc<dyn CheckpointStore>,
    config: CheckpointManagerConfig,
}

pub struct CheckpointManagerConfig {
    pub max_checkpoints_per_task: usize,
    pub retention_days: i64,
    pub auto_checkpoint_enabled: bool,
}

pub struct CheckpointMetadata {
    pub id: Uuid,
    pub task_id: Uuid,
    pub created_at: DateTime<Utc>,
    pub size_bytes: usize,
    pub state: TaskState,
}

impl CheckpointManager {
    pub fn new(store: Arc<dyn CheckpointStore>, config: CheckpointManagerConfig) -> Self;
    pub async fn create_checkpoint(&self, task_id: Uuid, state: TaskState, snapshot: Value) -> Result<Uuid, SwellError>;
    pub async fn prune_old_checkpoints(&self, task_id: Uuid) -> Result<usize, SwellError>;
    pub async fn get_metadata(&self, id: Uuid) -> Result<Option<CheckpointMetadata>, SwellError>;
}
```

### Key Re-exports

```rust
pub use swell_core::{Checkpoint, CheckpointStore};
pub use checkpoint_manager::{CheckpointManager, CheckpointManagerConfig, CheckpointMetadata};
pub use manager::StateManager;
pub use postgres::PostgresCheckpointStore;
pub use sqlite::SqliteCheckpointStore;
pub use traits::*;
```

## Architecture

```
┌─────────────────────────────────────────────────────────────────────┐
│                        swell-state                                 │
├─────────────────────────────────────────────────────────────────────┤
│                                                                      │
│  ┌──────────────────────────────────────────────────────────────┐   │
│  │                      StateManager                             │   │
│  │  (High-level API with in-memory cache)                        │   │
│  └──────────────────────────────────────────────────────────────┘   │
│                              │                                      │
│  ┌───────────────────────────┼───────────────────────────┐          │
│  │                           ▼                           │          │
│  │  ┌────────────────────────────────────────────────┐  │          │
│  │  │             CheckpointStore (trait)              │  │          │
│  │  │  save / load / list / delete / latest          │  │          │
│  │  └────────────────────────────────────────────────┘  │          │
│  │                           │                           │          │
│  │         ┌─────────────────┼─────────────────┐         │          │
│  │         ▼                 ▼                 ▼         │          │
│  │  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐   │          │
│  │  │   SQLite    │  │ PostgreSQL  │  │   In-Mem    │   │          │
│  │  │  Store      │  │   Store     │  │   (Test)    │   │          │
│  │  └─────────────┘  └─────────────┘  └─────────────┘   │          │
│  │                                                      │          │
│  └──────────────────────────────────────────────────────┘          │
│                              │                                      │
│  ┌───────────────────────────▼───────────────────────────┐          │
│  │              CheckpointManager                        │          │
│  │  (Lifecycle: create, prune, metadata)                │          │
│  └──────────────────────────────────────────────────────┘          │
│                                                                      │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐                  │
│  │  Migration  │  │    Trait    │  │    Tests    │                  │
│  │  Support    │  │   Modules   │  │   (inmem)   │                  │
│  └─────────────┘  └─────────────┘  └─────────────┘                  │
│                                                                      │
└─────────────────────────────────────────────────────────────────────┘
                           │ used by
                           ▼
              ┌────────────────────────┐
              │  swell-orchestrator    │
              │   swell-daemon        │
              │  swell-benchmark      │
              └────────────────────────┘
```

**Key modules:**
- `lib.rs` — Main exports (StateManager, CheckpointManager, store implementations)
- `traits/mod.rs` — CheckpointStore trait definition
- `traits/in_memory.rs` — In-memory store for testing
- `sqlite.rs` — SQLite checkpoint store implementation
- `postgres.rs` — PostgreSQL checkpoint store implementation
- `manager.rs` — StateManager implementation
- `checkpoint_manager.rs` — CheckpointManager implementation

**Concurrency:** Uses `Arc<RwLock<T>>` for in-memory cache. SQL stores use connection pools. All types are `Send + Sync`.

## Testing

```bash
# Run tests for swell-state
cargo test -p swell-state -- --test-threads=4

# Run with logging
RUST_LOG=debug cargo test -p swell-state

# Run specific test module
cargo test -p swell-state -- test_in_memory_store --nocapture

# Run SQLite tests (requires SQLite support)
cargo test -p swell-state -- sqlite

# Run PostgreSQL tests (requires PostgreSQL)
cargo test -p swell-state -- postgres
```

**Test patterns:**
- Unit tests for each store implementation
- In-memory store for fast unit tests
- Integration tests for SQLite/PostgreSQL stores
- Checkpoint lifecycle tests (create, list, load, delete)
- Pruning tests for CheckpointManager
- Migration tests

**Mock patterns:**
```rust
#[tokio::test]
async fn test_in_memory_store() {
    use crate::traits::in_memory::InMemoryCheckpointStore;

    let store = InMemoryCheckpointStore::new();
    let checkpoint = swell_core::Checkpoint {
        id: uuid::Uuid::new_v4(),
        task_id: uuid::Uuid::new_v4(),
        state: swell_core::TaskState::Created,
        snapshot: serde_json::json!({"test": true}),
        created_at: chrono::Utc::now(),
        metadata: serde_json::json!({}),
    };

    let id = store.save(checkpoint.clone()).await.unwrap();
    let loaded = store.load(id).await.unwrap().unwrap();
    assert_eq!(loaded.task_id, checkpoint.task_id);
}
```

## Dependencies

```toml
# swell-state/Cargo.toml
[dependencies]
swell-core = { path = "../swell-core" }
tokio.workspace = true
serde.workspace = true
serde_json.workspace = true
thiserror.workspace = true
sqlx = { workspace = true, features = ["runtime-tokio-rustls", "sqlite", "postgres", "uuid", "chrono", "migrate"] }
tracing.workspace = true
chrono.workspace = true
uuid.workspace = true
anyhow.workspace = true
async-trait.workspace = true

[dev-dependencies]
tokio-test.workspace = true
tempfile.workspace = true
```
