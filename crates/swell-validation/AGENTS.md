# swell-validation AGENTS.md

## Purpose

`swell-validation` provides validation gates and test pipelines for the SWELL autonomous coding engine. It offers a multi-layered validation system that runs quality assurance checks on code changes, including linting, testing, security scanning, and AI-powered code review.

This crate handles:
- **LintGate** — Runs linters (clippy, rustfmt) on changed files
- **TestGate** — Runs test suites with failure classification
- **SecurityGate** — Runs security scans (Semgrep, CodeQL)
- **AiReviewGate** — AI-powered code review with Evaluator agent
- **ValidationPipeline** — Orchestrates all gates in order
- **ResultInterpreter** — Classifies test failures into 5 categories with confidence scoring
- **ConfidenceScorer** — Computes confidence scores from validation signals
- **EvidencePackBuilder** — Creates comprehensive evidence packs for PR review
- **FlakinessDetector** — Detects and handles flaky tests
- **StagedTestExecutor** — Multi-stage test execution with adaptive retries
- **MultiSignalValidator** — Combines multiple validation signals
- **TestPlanningEngine** — Generates test plans from acceptance criteria
- **PredictiveSelectionEngine** — ML-based intelligent test prioritization
- **AutonomousCoverageEngine** — Mutation testing and coverage gap detection
- **TestCollaborationOrchestrator** — Multi-agent collaborative testing

**Depends on:** `swell-core` (for `ValidationGate` trait, `SwellError`), `swell-llm` (for AI review)

## Public API

### Validation Gates

```rust
// Lint Gate - runs clippy on workspace
pub struct LintGate;
impl ValidationGate for LintGate { /* ... */ }

// Test Gate - runs cargo test with failure classification
pub struct TestGate;
impl ValidationGate for TestGate { /* ... */ }

// Security Gate - runs Semgrep/CodeQL scans
pub struct SecurityGate {
    scanners: Vec<SecurityScannerType>,
    block_on_high: bool,
}
impl ValidationGate for SecurityGate { /* ... */ }

// AI Review Gate - LLM-powered code review
pub struct AiReviewGate {
    llm: Option<Arc<dyn LlmBackend>>,
    prompt_template: String,
}
impl ValidationGate for AiReviewGate { /* ... */ }
```

### Result Interpretation

```rust
pub enum TestFailureClassification {
    ImplementationDefect,
    TestDefect,
    EnvironmentDefect,
    Unknown,
}

pub struct TestFailure {
    pub name: String,
    pub message: String,
    pub file: Option<String>,
    pub line: Option<u32>,
    pub classification: TestFailureClassification,
}

pub struct ParsedTestOutput {
    pub total: usize,
    pub passed: usize,
    pub failed: usize,
    pub skipped: usize,
    pub duration_ms: u64,
    pub failures: Vec<TestFailure>,
}
```

### Security Scanning

```rust
pub enum SecurityScannerType {
    Semgrep,
    CodeQL,
}

pub enum FindingSeverity {
    Error = 0,
    Warning = 1,
    Info = 2,
}

pub struct Vulnerability {
    pub id: String,
    pub cwe: Option<String>,
    pub severity: FindingSeverity,
    pub title: String,
    pub file: Option<String>,
    pub line: Option<u32>,
    pub description: String,
    pub link: Option<String>,
    pub scanner: SecurityScannerType,
}

pub struct SecurityScanResults {
    pub findings: Vec<Vulnerability>,
    pub scanner: Option<SecurityScannerType>,
    pub duration_ms: u64,
    pub scan_success: bool,
    pub error_message: Option<String>,
}
```

### Confidence Scoring

```rust
pub enum ConfidenceLevel { Low, Medium, High }

pub struct ConfidenceScore {
    pub level: ConfidenceLevel,
    pub score: f64,
    pub signals: Vec<ConfidenceSignal>,
}

pub struct ConfidenceThresholds {
    pub low_threshold: f64,
    pub medium_threshold: f64,
}

pub enum ValidationSignal {
    TestPassRate,
    LintClean,
    SecurityScan,
    AiReviewConfidence,
    Coverage,
}
```

### Evidence Pack

```rust
pub struct EvidencePack {
    pub test_evidence: TestEvidence,
    pub coverage_evidence: CoverageEvidence,
    pub flakiness_evidence: FlakinessEvidence,
    pub security_evidence: SecurityEvidence,
    pub ai_review_evidence: Option<AiReviewEvidence>,
    pub confidence_evidence: ConfidenceEvidence,
}

pub struct EvidencePackBuilder { /* ... */ }
```

### Key Re-exports

```rust
pub use calibrated_confidence::{CalibratedConfidence, DefectTracker, RiskLevel};
pub use confidence::{ConfidenceLevel, ConfidenceScore, ConfidenceScorer, ConfidenceSignal};
pub use evidence::{EvidencePack, EvidencePackBuilder, EvidenceStore, InMemoryEvidenceStore, SqliteEvidenceStore};
pub use flakiness::{FlakinessConfig, FlakinessDetector, FlakinessReport, QuarantinePool};
pub use result_interpreter::{FailureCategory, ResultInterpreter, TestResultInfo};
pub use staged_executor::{Stage0Config, Stage1Config, Stage2Config, Stage3Config, Stage4Config, StagedTestExecutor};
pub use test_planning::{DiffContextExtractor, RiskScorer, TestCase, TestPlan, TestPlanningEngine};
pub use test_generator::{GeneratedTest, TestGenerator, TestGeneratorConfig};
pub use multi_signal::{MultiSignalValidator, SignalSeverity, SignalWeights};
pub use predictive_selection::{ChangeImpact, ChangeImpactAnalyzer, PredictiveSelectionEngine};
pub use autonomous_coverage::{AutonomousCoverageEngine, CoverageGap, GapSeverity};
pub use multi_agent_test_roles::{TestCollaborationOrchestrator, TestAgentRole};
pub use self_improving_tests::{TestValueGate, TestValueTracker, RetirementCandidate};
```

## Architecture

```
┌─────────────────────────────────────────────────────────────────────┐
│                      swell-validation                              │
├─────────────────────────────────────────────────────────────────────┤
│                                                                      │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐  ┌───────────┐ │
│  │   Lint     │  │    Test     │  │  Security   │  │ AI Review │ │
│  │   Gate     │  │    Gate     │  │    Gate     │  │   Gate    │ │
│  │  lint_gate │  │  test_gate  │  │security_gate│  │ ai_review │ │
│  └──────┬──────┘  └──────┬──────┘  └──────┬──────┘  └─────┬─────┘ │
│         │                 │                 │              │         │
│         └─────────────────┼─────────────────┼──────────────┘         │
│                           │                 │                         │
│                    ┌─────▼─────────────────▼──────┐                  │
│                    │   ValidationPipeline          │                  │
│                    │  (orchestrates all gates)     │                  │
│                    └───────────────┬────────────────┘                  │
│                                │                                     │
│  ┌─────────────────────────────┼─────────────────────────────┐       │
│  │                             │                             │       │
│  ▼                             ▼                             ▼       │
│ ┌──────────────┐      ┌────────────────┐      ┌──────────────────┐  │
│ │   Result     │      │   Confidence   │      │     Evidence      │  │
│ │Interpreter   │      │    Scorer      │      │   Pack Builder   │  │
│ │ (5-class     │      │ (multi-signal  │      │ (comprehensive   │  │
│ │  classif.)   │      │  aggregation)  │      │  evidence for    │  │
│ └──────────────┘      └────────────────┘      │  PR review)      │  │
│                                                └──────────────────┘  │
│                                                                      │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐  ┌───────────┐ │
│  │  Flakiness  │  │   Staged    │  │    Multi    │  │  Test     │ │
│  │  Detector   │  │   Test      │  │   Signal    │  │ Planning  │ │
│  │            │  │  Executor   │  │  Validator  │  │  Engine   │ │
│  └─────────────┘  └─────────────┘  └─────────────┘  └───────────┘ │
│                                                                      │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐                  │
│  │ Predictive │  │Autonomous   │  │   Multi     │                  │
│  │ Selection │  │  Coverage   │  │    Agent    │                  │
│  │  Engine   │  │   Engine    │  │   Test      │                  │
│  └─────────────┘  └─────────────┘  └─────────────┘                  │
│                                                                      │
└─────────────────────────────────────────────────────────────────────┘
                           │ used by
                           ▼
              ┌────────────────────────┐
              │  swell-orchestrator    │
              └────────────────────────┘
```

**Key modules:**
- `lib.rs` — Gate implementations (LintGate, TestGate, SecurityGate, AiReviewGate)
- `result_interpreter.rs` — Test failure classification into 5 categories
- `confidence.rs` — Confidence scoring from validation signals
- `evidence.rs` — Evidence pack for comprehensive PR review evidence
- `flakiness.rs` — Flaky test detection, quarantine, and retry handling
- `staged_executor.rs` — Multi-stage test execution with adaptive retries
- `multi_signal.rs` — Multi-signal validation with configurable weights
- `test_planning.rs` — Test plan generation from acceptance criteria
- `test_generator.rs` — Test generation with multiple test types
- `predictive_selection.rs` — ML-based test prioritization
- `autonomous_coverage.rs` — Mutation testing and coverage analysis
- `multi_agent_test_roles.rs` — Collaborative multi-agent testing
- `self_improving_tests.rs` — Test value tracking and retirement
- `calibrated_confidence.rs` — Calibrated validation confidence (V2)
- `traceability.rs` — Full traceability chain (TestCase, TestResult, AcceptanceCriteria)
- `traceability/sqlite_store.rs` — SQLite traceability store

**Concurrency:** Uses `Arc<RwLock<T>>` for shared state. Gates are `Send + Sync`.

## Testing

```bash
# Run tests for swell-validation
cargo test -p swell-validation -- --test-threads=4

# Run with logging
RUST_LOG=debug cargo test -p swell-validation

# Run specific test module
cargo test -p swell-validation -- result_interpreter --nocapture

# Run security gate tests
cargo test -p swell-validation -- security

# Run flakiness tests
cargo test -p swell-validation -- flakiness

# Run test planning tests
cargo test -p swell-validation -- test_planning
```

**Test patterns:**
- Unit tests for each gate implementation
- Failure classification tests with fixture data
- Security scanner availability tests
- Confidence scoring tests
- Flakiness detection tests
- Evidence pack building tests

**Mock patterns:**
```rust
#[tokio::test]
async fn test_test_gate_classification() {
    let gate = TestGate::new();
    let context = ValidationContext {
        workspace_path: PathBuf::from("."),
        changed_files: vec!["src/lib.rs".to_string()],
        plan: None,
    };
    let result = gate.validate(context).await.unwrap();
    // Check result.passed and result.messages
}
```

## Memory Management

### Cargo Test Serialization

`TestGate` and `StagedTestExecutor` use `Command::output()` which buffers the entire
stdout + stderr of `cargo test` in `Vec<u8>` before returning. For a workspace-wide run
this easily reaches hundreds of megabytes. The orchestrator's `execute_batch` can run up
to `MAX_CONCURRENT_AGENTS` (6) agents in parallel, each triggering their own `cargo test`,
multiplying peak memory by up to 6×.

To prevent this, a **global semaphore** (`CARGO_TEST_SEMAPHORE`, 1 permit) in `lib.rs`
serializes all `cargo test` invocations across agents. The permit is acquired before
`spawn_blocking` and moved into the closure so it is held for the full duration of the
subprocess. This ensures only one `cargo test --workspace` runs at a time.

```rust
// lib.rs — used by both TestGate and StagedTestExecutor
pub(crate) fn cargo_test_semaphore() -> Arc<tokio::sync::Semaphore> { ... }

// Acquire before spawning; move permit into closure to hold it
let permit = cargo_test_semaphore().acquire_owned().await?;
let output = task::spawn_blocking(move || {
    let _permit = permit;
    Command::new("cargo").args([...]).output()
}).await??;
```

### Output Truncation

Error messages stored in `ValidationMessage` are capped at 64 KB (`MAX_STORED_OUTPUT_BYTES`)
via `truncate_output()`, keeping the **tail** (most recent output) which contains the
failure details. Raw `output` (the `Vec<u8>` buffers) is explicitly `drop()`-ed immediately
after parsing to release memory before any further async work.

### Eliminated `full_output` Allocation

`parse_test_output` previously allocated a combined `String` via
`format!("{}\n{}", stdout, stderr)`, creating a third copy of the data alongside the two
existing `Vec<u8>` buffers. This was replaced with `stdout.lines().chain(stderr.lines())`
to iterate over both streams without any additional allocation.

## Dependencies

```toml
# swell-validation/Cargo.toml
[dependencies]
swell-core = { path = "../swell-core" }
swell-llm = { path = "../swell-llm" }
tokio.workspace = true
serde.workspace = true
serde_json.workspace = true
thiserror.workspace = true
anyhow.workspace = true
tracing.workspace = true
uuid.workspace = true
chrono.workspace = true
async-trait.workspace = true
futures.workspace = true
glob = "0.3"
which = "6"
itertools = "0.13"
sqlx = { version = "0.8", features = ["runtime-tokio-rustls", "sqlite", "uuid", "chrono"] }
```
