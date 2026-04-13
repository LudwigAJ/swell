# swell-benchmark AGENTS.md

## Purpose

`swell-benchmark` provides a comprehensive benchmark suite for evaluating the SWELL autonomous coding engine. It contains 50 curated benchmark tasks spanning bug fixes, features, refactoring, and tests, along with a runner and metrics collection system.

This crate handles:
- **Benchmark Suite** — Standardized 50-task benchmark covering diverse coding challenges
- **Benchmark Runner** — Execution engine for running benchmarks with configurable timeouts and retries
- **Metrics Collection** — Comprehensive metrics including success rates, timing, and category breakdowns
- **Task Categorization** — Tasks organized by category (BugFix, Feature, Refactoring, Test) and difficulty (Low, Medium, High, VeryHigh)
- **Progress Tracking** — Real-time progress monitoring during benchmark execution

**Depends on:** `swell-core`, `swell-orchestrator`, `swell-llm`, `swell-tools`, `swell-validation`, `swell-state`

## Public API

### Benchmark Types (`lib.rs`)

```rust
pub struct BenchmarkId(pub Uuid);

pub struct BenchmarkSuite {
    pub id: BenchmarkId,
    pub name: String,
    pub description: String,
    pub tasks: Vec<BenchmarkTask>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl BenchmarkSuite {
    pub fn standard() -> Self;  // Create standard 50-task suite
    pub fn tasks_by_category(&self, category: TaskCategory) -> Vec<&BenchmarkTask>;
    pub fn tasks_by_difficulty(&self, difficulty: TaskDifficulty) -> Vec<&BenchmarkTask>;
    pub fn statistics(&self) -> BenchmarkStatistics;
}
```

### Benchmark Task (`task.rs`)

```rust
pub struct TaskId(pub String);

pub enum TaskCategory {
    BugFix,
    Feature,
    Refactoring,
    Test,
}

pub enum TaskDifficulty {
    Low,
    Medium,
    High,
    VeryHigh,
}

pub struct BenchmarkTask {
    pub id: TaskId,
    pub category: TaskCategory,
    pub difficulty: TaskDifficulty,
    pub description: String,
    pub success_criteria: Vec<String>,
    pub affected_files: Vec<String>,
}

pub struct TaskResult {
    pub task_id: TaskId,
    pub category: TaskCategory,
    pub difficulty: TaskDifficulty,
    pub outcome: TaskOutcome,
    pub duration_secs: f64,
    pub retries: u32,
    pub notes: Option<String>,
    pub completed_at: chrono::DateTime<chrono::Utc>,
}
```

### Metrics Types (`metrics.rs`)

```rust
pub enum TaskOutcome {
    Completed,
    Failed,
    Skipped,
    Timeout,
}

pub struct BenchmarkMetrics {
    pub total_tasks: usize,
    pub completed: usize,
    pub failed: usize,
    pub skipped: usize,
    pub timeouts: usize,
    pub total_duration_secs: f64,
    pub avg_duration_secs: f64,
    pub success_rate: f64,
    pub completion_rate: f64,
    pub by_category: HashMap<String, CategoryMetrics>,
    pub by_difficulty: HashMap<String, DifficultyMetrics>,
    pub task_results: Vec<TaskResult>,
    pub recorded_at: chrono::DateTime<chrono::Utc>,
}

impl BenchmarkMetrics {
    pub fn from_results(results: Vec<TaskResult>, total_tasks: usize) -> Self;
    pub fn summary(&self) -> String;
    pub fn to_json(&self) -> Result<String, serde_json::Error>;
}

pub struct ProgressTracker {
    pub total: usize,
    pub completed: usize,
    pub current_task: Option<TaskId>,
    pub start_time: chrono::DateTime<chrono::Utc>,
}

impl ProgressTracker {
    pub fn new(total: usize) -> Self;
    pub fn record_completion(&mut self, task_id: TaskId);
    pub fn progress(&self) -> f64;
    pub fn eta_secs(&self) -> Option<f64>;
}
```

### Runner (`runner.rs`)

```rust
pub struct RunnerConfig {
    pub task_timeout: Duration,
    pub max_retries: u32,
    pub continue_on_failure: bool,
    pub concurrency: usize,
    pub category_filter: Option<Vec<String>>,
    pub difficulty_filter: Option<Vec<String>>,
    pub task_filter: Option<Vec<String>>,
}

pub trait ProgressCallback: Send + Sync {
    fn on_task_start(&self, task: &BenchmarkTask);
    fn on_task_complete(&self, task_id: &TaskId, result: &TaskResult);
    fn on_progress(&self, progress: &ProgressTracker);
}

pub struct BenchmarkRunner {
    // ...
}

impl BenchmarkRunner {
    pub fn standard() -> Self;
    pub fn new(suite: BenchmarkSuite) -> Self;
    pub fn with_config(self, config: RunnerConfig) -> Self;
    pub async fn run<C: ProgressCallback>(&self, callback: &C) -> BenchmarkMetrics;
    pub fn suite(&self) -> &BenchmarkSuite;
    pub fn config(&self) -> &RunnerConfig;
}
```

### Key Re-exports

```rust
pub use metrics::{BenchmarkMetrics, CategoryMetrics, DifficultyMetrics, ProgressTracker, TaskOutcome};
pub use runner::{BenchmarkRunner, RunnerConfig, ProgressCallback};
pub use task::{BenchmarkTask, BenchmarkStatistics, TaskCategory, TaskDifficulty, TaskId, TaskResult};
```

## Architecture

```
┌─────────────────────────────────────────────────────────────────────┐
│                       swell-benchmark                               │
├─────────────────────────────────────────────────────────────────────┤
│                                                                     │
│  ┌─────────────────────────────────────────────────────────────┐   │
│  │                    BenchmarkSuite                           │   │
│  │  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌──────────┐   │   │
│  │  │ BugFix   │  │ Feature  │  │ Refactor │  │   Test   │   │   │
│  │  │ (12)     │  │  (14)    │  │  (12)    │  │  (12)    │   │   │
│  │  └──────────┘  └──────────┘  └──────────┘  └──────────┘   │   │
│  └─────────────────────────────────────────────────────────────┘   │
│                                                                     │
│  ┌──────────────────────┐    ┌──────────────────────┐             │
│  │   BenchmarkRunner    │    │  BenchmarkMetrics     │             │
│  │  ┌────────────────┐ │    │  ┌────────────────┐  │             │
│  │  │ RunnerConfig   │ │    │  │ TaskOutcome    │  │             │
│  │  │ ProgressCallback│ │    │  │ CategoryMetrics│  │             │
│  │  │ AsyncRunner    │ │    │  │ DifficultyMetrics│ │             │
│  │  └────────────────┘ │    │  └────────────────┘  │             │
│  └──────────────────────┘    └──────────────────────┘             │
│                                                                     │
│  ┌──────────────────────┐    ┌──────────────────────┐             │
│  │ ProgressTracker      │    │   TaskResult         │             │
│  │  - progress()       │    │   - outcome          │             │
│  │  - eta_secs()       │    │   - duration_secs    │             │
│  └──────────────────────┘    └──────────────────────┘             │
│                                                                     │
└─────────────────────────────────────────────────────────────────────┘
                           │ used by
                           ▼
              ┌────────────────────────┐
              │     Evaluation         │
              │   (external tools)      │
              └────────────────────────┘
```

**Key modules:**
- `lib.rs` — `BenchmarkSuite` with 50 standard tasks, `BenchmarkId`
- `task.rs` — `BenchmarkTask`, `TaskResult`, `TaskCategory`, `TaskDifficulty`
- `metrics.rs` — `BenchmarkMetrics`, `CategoryMetrics`, `DifficultyMetrics`, `ProgressTracker`
- `runner.rs` — `BenchmarkRunner`, `RunnerConfig`, `ProgressCallback`

**Task Distribution (50 total):**
- Bug Fixes: 12 tasks (state machine, LLM, tools, validation)
- Features: 14 tasks (orchestrator, memory, tools, validation)
- Refactoring: 12 tasks (orchestrator, LLM, tools)
- Tests: 12 tasks (orchestrator, LLM, tools)

**Concurrency:** Uses `tokio::sync::RwLock` for async runner. Callbacks are `Send + Sync`.

## Testing

```bash
# Run tests for swell-benchmark
cargo test -p swell-benchmark -- --test-threads=4

# Run with logging
RUST_LOG=debug cargo test -p swell-benchmark

# Run specific test
cargo test -p swell-benchmark -- test_standard_suite_has_50_tasks --nocapture

# Run runner tests
cargo test -p swell-benchmark -- runner --nocapture

# Run metrics tests
cargo test -p swell-benchmark -- metrics --nocapture
```

**Test structure:**
- Unit tests in `#[cfg(test)]` modules within each source file
- Tests for suite generation in `lib.rs`
- Tests for metrics calculation in `metrics.rs`
- Tests for runner configuration and execution in `runner.rs`

**Mock patterns:**
```rust
#[tokio::test]
async fn test_run_standard_suite() {
    let runner = BenchmarkRunner::standard();
    let metrics = runner.run(&NoOpCallback).await;
    assert_eq!(metrics.total_tasks, 50);
    assert_eq!(metrics.completed, 50);  // Simulation mode
}

#[test]
fn test_benchmark_metrics_empty() {
    let metrics = BenchmarkMetrics::from_results(vec![], 50);
    assert_eq!(metrics.total_tasks, 50);
    assert_eq!(metrics.success_rate, 0.0);
}
```

## Dependencies

```toml
# swell-benchmark/Cargo.toml
[dependencies]
swell-core = { path = "../swell-core" }
swell-orchestrator = { path = "../swell-orchestrator" }
swell-llm = { path = "../swell-llm" }
swell-tools = { path = "../swell-tools" }
swell-validation = { path = "../swell-validation" }
swell-state = { path = "../swell-state" }
tokio.workspace = true
serde.workspace = true
serde_json.workspace = true
thiserror.workspace = true
anyhow.workspace = true
tracing.workspace = true
uuid.workspace = true
chrono.workspace = true

[dev-dependencies]
tokio-test.workspace = true
mockall.workspace = true
tempfile.workspace = true
```

**Internal workspace dependencies:** All core crates for full benchmark coverage
