//! Traceability Store Module
//!
//! Provides bidirectional traceability linking: Goal→Criteria→Tests→Results→Evidence
//! for complete audit trails and impact analysis.
//!
//! # Traceability Chain
//!
//! ```text
//! Goal ──→ AcceptanceCriteria ──→ TestCase ──→ TestResult ──→ Evidence
//!   ↑           ↑                  ↑            ↑              ↑
//!   └───────────┴──────────────────┴────────────┴──────────────┘
//!                    (bidirectional links)
//! ```
//!
//! # Architecture
//!
//! - [`Goal`] - High-level objective linked to task
//! - [`AcceptanceCriteria`] - Criteria linked to goals  
//! - [`TestCase`] - Test cases linked to criteria
//! - [`TestResult`] - Execution results linked to tests
//! - [`Evidence`] - Evidence artifacts linked to results
//!
//! # Usage
//!
//! ```rust,ignore
//! use swell_validation::traceability::{TraceabilityStore, InMemoryTraceabilityStore};
//!
//! let store = InMemoryTraceabilityStore::new();
//!
//! // Create a goal
//! let goal = store.create_goal(Goal::new("Implement user auth", task_id)).await?;
//!
//! // Link criteria to goal
//! let criteria = store.add_criteria(AcceptanceCriteria::new(
//!     "Users shall login with email/password", &goal.id
//! )).await?;
//!
//! // Navigate: Goal → Criteria
//! let goal_criteria = store.get_criteria_for_goal(goal.id).await?;
//!
//! // Navigate backwards: Criteria → Goals  
//! let criteria_goal = store.get_goal_for_criteria(criteria.id).await?;
//! ```

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use swell_core::ids::TaskId;
use uuid::Uuid;

/// A high-level goal/objective linked to a task
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Goal {
    /// Unique identifier
    pub id: Uuid,
    /// Task ID this goal belongs to
    pub task_id: TaskId,
    /// Goal description
    pub description: String,
    /// When goal was created
    pub created_at: DateTime<Utc>,
    /// When goal was last updated
    pub updated_at: DateTime<Utc>,
    /// Goal status
    pub status: GoalStatus,
    /// Linked criteria IDs (forward links)
    pub criteria_ids: Vec<Uuid>,
}

impl Goal {
    /// Create a new goal
    pub fn new(description: impl Into<String>, task_id: TaskId) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            task_id,
            description: description.into(),
            created_at: now,
            updated_at: now,
            status: GoalStatus::Active,
            criteria_ids: Vec::new(),
        }
    }

    /// Update the goal status
    pub fn with_status(mut self, status: GoalStatus) -> Self {
        self.status = status;
        self.updated_at = Utc::now();
        self
    }
}

/// Goal completion status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GoalStatus {
    /// Goal is active and being worked on
    Active,
    /// Goal has been achieved
    Achieved,
    /// Goal has been abandoned
    Abandoned,
}

/// Acceptance criteria linked to a goal
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcceptanceCriteria {
    /// Unique identifier
    pub id: Uuid,
    /// Parent goal ID (back link)
    pub goal_id: Uuid,
    /// Criteria text
    pub text: String,
    /// Category (functional, security, performance, etc.)
    pub category: String,
    /// Criticality level
    pub criticality: CriteriaCriticality,
    /// Status
    pub status: CriteriaStatus,
    /// When created
    pub created_at: DateTime<Utc>,
    /// Linked test IDs (forward links)
    pub test_ids: Vec<Uuid>,
}

impl AcceptanceCriteria {
    /// Create new acceptance criteria linked to a goal
    pub fn new(text: impl Into<String>, goal_id: Uuid) -> Self {
        Self {
            id: Uuid::new_v4(),
            goal_id,
            text: text.into(),
            category: "general".to_string(),
            criticality: CriteriaCriticality::ShouldHave,
            status: CriteriaStatus::Pending,
            created_at: Utc::now(),
            test_ids: Vec::new(),
        }
    }

    /// Create with category
    pub fn with_category(mut self, category: impl Into<String>) -> Self {
        self.category = category.into();
        self
    }

    /// Create with criticality
    pub fn with_criticality(mut self, criticality: CriteriaCriticality) -> Self {
        self.criticality = criticality;
        self
    }
}

/// Criticality level for acceptance criteria
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CriteriaCriticality {
    /// Must have for release
    MustHave,
    /// Should have for release
    ShouldHave,
    /// Nice to have
    NiceToHave,
}

/// Criteria verification status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CriteriaStatus {
    /// Not yet verified
    Pending,
    /// Verification in progress
    InProgress,
    /// Verified and passing
    Verified,
    /// Failed verification
    Failed,
}

/// Test case linked to acceptance criteria
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestCase {
    /// Unique identifier
    pub id: Uuid,
    /// Parent criteria ID (back link)
    pub criteria_id: Uuid,
    /// Test name
    pub name: String,
    /// Test file path
    pub file_path: Option<String>,
    /// Test type
    pub test_type: TestCaseType,
    /// Status
    pub status: TestCaseStatus,
    /// When created
    pub created_at: DateTime<Utc>,
    /// Linked result IDs (forward links)
    pub result_ids: Vec<Uuid>,
}

impl TestCase {
    /// Create a new test case
    pub fn new(name: impl Into<String>, criteria_id: Uuid) -> Self {
        Self {
            id: Uuid::new_v4(),
            criteria_id,
            name: name.into(),
            file_path: None,
            test_type: TestCaseType::Unit,
            status: TestCaseStatus::Pending,
            created_at: Utc::now(),
            result_ids: Vec::new(),
        }
    }

    /// Create with file path
    pub fn with_file_path(mut self, path: impl Into<String>) -> Self {
        self.file_path = Some(path.into());
        self
    }

    /// Create with test type
    pub fn with_test_type(mut self, test_type: TestCaseType) -> Self {
        self.test_type = test_type;
        self
    }
}

/// Type of test case
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TestCaseType {
    /// Unit test
    Unit,
    /// Integration test
    Integration,
    /// Property-based test
    Property,
    /// End-to-end test
    E2e,
}

/// Test case execution status
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TestCaseStatus {
    /// Not yet executed
    Pending,
    /// Currently executing
    Running,
    /// Passed
    Passed,
    /// Failed
    Failed,
    /// Skipped
    Skipped,
}

/// Test result linked to a test case
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestResult {
    /// Unique identifier
    pub id: Uuid,
    /// Parent test case ID (back link)
    pub test_id: Uuid,
    /// Whether test passed
    pub passed: bool,
    /// Duration in milliseconds
    pub duration_ms: u64,
    /// Failure message if applicable
    pub failure_message: Option<String>,
    /// When executed
    pub executed_at: DateTime<Utc>,
    /// Linked evidence IDs (forward links)
    pub evidence_ids: Vec<Uuid>,
}

impl TestResult {
    /// Create a new test result
    pub fn new(test_id: Uuid, passed: bool) -> Self {
        Self {
            id: Uuid::new_v4(),
            test_id,
            passed,
            duration_ms: 0,
            failure_message: None,
            executed_at: Utc::now(),
            evidence_ids: Vec::new(),
        }
    }

    /// Create with duration
    pub fn with_duration(mut self, duration_ms: u64) -> Self {
        self.duration_ms = duration_ms;
        self
    }

    /// Create with failure message
    pub fn with_failure_message(mut self, message: impl Into<String>) -> Self {
        self.failure_message = Some(message.into());
        self
    }
}

/// Evidence artifact linked to a test result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evidence {
    /// Unique identifier
    pub id: Uuid,
    /// Parent result ID (back link)
    pub result_id: Uuid,
    /// Evidence type
    pub evidence_type: EvidenceType,
    /// Artifact path or URL
    pub artifact_path: String,
    /// Description
    pub description: Option<String>,
    /// When created
    pub created_at: DateTime<Utc>,
}

/// Type of evidence artifact
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvidenceType {
    /// Test output/logs
    TestOutput,
    /// Coverage report
    CoverageReport,
    /// Screenshot/image
    Screenshot,
    /// Video recording
    Video,
    /// Stack trace
    StackTrace,
    /// Memory dump
    MemoryDump,
    /// Custom evidence
    Custom,
}

impl Evidence {
    /// Create new evidence
    pub fn new(
        result_id: Uuid,
        evidence_type: EvidenceType,
        artifact_path: impl Into<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            result_id,
            evidence_type,
            artifact_path: artifact_path.into(),
            description: None,
            created_at: Utc::now(),
        }
    }

    /// Create with description
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }
}

// ============================================================================
// Traceability Store Trait
// ============================================================================

/// Trait for traceability storage with bidirectional links
#[async_trait]
pub trait TraceabilityStore: Send + Sync {
    // --- Goal Operations ---

    /// Create a new goal
    async fn create_goal(&self, goal: Goal) -> Result<Uuid, TraceabilityError>;

    /// Get a goal by ID
    async fn get_goal(&self, id: Uuid) -> Result<Option<Goal>, TraceabilityError>;

    /// Update a goal
    async fn update_goal(&self, goal: &Goal) -> Result<(), TraceabilityError>;

    /// Get all goals for a task
    async fn get_goals_for_task(&self, task_id: TaskId) -> Result<Vec<Goal>, TraceabilityError>;

    // --- Criteria Operations ---

    /// Add acceptance criteria to a goal
    async fn add_criteria(&self, criteria: AcceptanceCriteria) -> Result<Uuid, TraceabilityError>;

    /// Get criteria by ID
    async fn get_criteria(&self, id: Uuid)
        -> Result<Option<AcceptanceCriteria>, TraceabilityError>;

    /// Update criteria
    async fn update_criteria(&self, criteria: &AcceptanceCriteria)
        -> Result<(), TraceabilityError>;

    /// Get criteria linked to a goal (forward: goal → criteria)
    async fn get_criteria_for_goal(
        &self,
        goal_id: Uuid,
    ) -> Result<Vec<AcceptanceCriteria>, TraceabilityError>;

    /// Get the goal that owns this criteria (backward: criteria → goal)
    async fn get_goal_for_criteria(
        &self,
        criteria_id: Uuid,
    ) -> Result<Option<Goal>, TraceabilityError>;

    // --- Test Case Operations ---

    /// Add a test case to criteria
    async fn add_test_case(&self, test_case: TestCase) -> Result<Uuid, TraceabilityError>;

    /// Get test case by ID
    async fn get_test_case(&self, id: Uuid) -> Result<Option<TestCase>, TraceabilityError>;

    /// Update test case
    async fn update_test_case(&self, test_case: &TestCase) -> Result<(), TraceabilityError>;

    /// Get test cases for criteria (forward: criteria → tests)
    async fn get_tests_for_criteria(
        &self,
        criteria_id: Uuid,
    ) -> Result<Vec<TestCase>, TraceabilityError>;

    /// Get the criteria that owns this test (backward: test → criteria)
    async fn get_criteria_for_test(
        &self,
        test_id: Uuid,
    ) -> Result<Option<AcceptanceCriteria>, TraceabilityError>;

    // --- Test Result Operations ---

    /// Add a test result
    async fn add_result(&self, result: TestResult) -> Result<Uuid, TraceabilityError>;

    /// Get result by ID
    async fn get_result(&self, id: Uuid) -> Result<Option<TestResult>, TraceabilityError>;

    /// Get results for test (forward: test → results)
    async fn get_results_for_test(
        &self,
        test_id: Uuid,
    ) -> Result<Vec<TestResult>, TraceabilityError>;

    /// Get the test that produced this result (backward: result → test)
    async fn get_test_for_result(
        &self,
        result_id: Uuid,
    ) -> Result<Option<TestCase>, TraceabilityError>;

    // --- Evidence Operations ---

    /// Add evidence to a result
    async fn add_evidence(&self, evidence: Evidence) -> Result<Uuid, TraceabilityError>;

    /// Get evidence by ID
    async fn get_evidence(&self, id: Uuid) -> Result<Option<Evidence>, TraceabilityError>;

    /// Get evidence for result (forward: result → evidence)
    async fn get_evidence_for_result(
        &self,
        result_id: Uuid,
    ) -> Result<Vec<Evidence>, TraceabilityError>;

    /// Get the result this evidence belongs to (backward: evidence → result)
    async fn get_result_for_evidence(
        &self,
        evidence_id: Uuid,
    ) -> Result<Option<TestResult>, TraceabilityError>;

    // --- Full Traceability Navigation ---

    /// Get full traceability chain from goal (Goal → Criteria → Tests → Results → Evidence)
    async fn get_full_chain(
        &self,
        goal_id: Uuid,
    ) -> Result<Option<TraceabilityChain>, TraceabilityError>;

    /// Get reverse traceability chain from evidence
    async fn get_reverse_chain(
        &self,
        evidence_id: Uuid,
    ) -> Result<Option<TraceabilityChain>, TraceabilityError>;

    /// Count items in the traceability chain
    async fn count_chain_items(&self, goal_id: Uuid) -> Result<ChainCounts, TraceabilityError>;

    // --- Evidence Pack Assembly ---

    /// Assemble an evidence pack for a requirement (goal).
    ///
    /// An evidence pack bundles all linked tests, their results, the code they cover,
    /// and any artifacts for a given requirement. Used for merge decisions and audit trails.
    async fn assemble_evidence_pack(
        &self,
        goal_id: Uuid,
    ) -> Result<TraceabilityEvidencePack, TraceabilityError>;

    /// Detect missing links in the traceability chain.
    ///
    /// Returns a list of broken or missing links that prevent full traceability.
    /// For example: criteria with no tests, tests with no results, results with no evidence.
    async fn detect_missing_links(
        &self,
        goal_id: Uuid,
    ) -> Result<Vec<MissingLink>, TraceabilityError>;
}

/// An evidence pack for a requirement (goal).
///
/// Bundles all linked tests, their results, the code they cover,
/// and any artifacts for a given requirement. Used for merge decisions
/// and audit trails.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceabilityEvidencePack {
    /// Unique identifier
    pub id: Uuid,
    /// The goal/requirement this pack is for
    pub requirement_id: Uuid,
    /// When the pack was assembled
    pub assembled_at: chrono::DateTime<Utc>,
    /// Linked criteria with their tests and results
    pub criteria: Vec<CriteriaEvidence>,
    /// Total test count
    pub total_tests: usize,
    /// Tests that passed
    pub passed_tests: usize,
    /// Tests that failed
    pub failed_tests: usize,
    /// Code locations covered by tests
    pub code_locations: Vec<CodeLocation>,
    /// Evidence artifacts
    pub artifacts: Vec<EvidenceArtifact>,
}

/// Evidence for a single criteria
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CriteriaEvidence {
    /// The criteria
    pub criteria: AcceptanceCriteria,
    /// Test evidence items
    pub tests: Vec<TestCaseEvidence>,
}

/// Evidence for a single test case
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestCaseEvidence {
    /// The test case
    pub test_case: TestCase,
    /// All results for this test (newest first)
    pub results: Vec<TestResult>,
    /// Evidence artifacts for this test
    pub evidence: Vec<Evidence>,
    /// Code location for this test
    pub code_location: Option<CodeLocation>,
}

/// A code location covered by a test
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeLocation {
    /// File path
    pub file_path: String,
    /// Line number (start)
    pub line_start: u32,
    /// Line number (end)
    pub line_end: u32,
    /// Description of what's being tested
    pub description: Option<String>,
}

/// An evidence artifact from a test result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceArtifact {
    /// Evidence ID
    pub id: Uuid,
    /// Artifact type
    pub evidence_type: EvidenceType,
    /// Path to the artifact
    pub artifact_path: String,
    /// Description
    pub description: Option<String>,
    /// Created at timestamp
    pub created_at: chrono::DateTime<Utc>,
}

/// A missing link in the traceability chain
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissingLink {
    /// The type of missing link
    pub link_type: MissingLinkType,
    /// The parent entity that has the missing link
    pub parent_id: Uuid,
    /// Description of what's missing
    pub description: String,
}

/// Type of missing link
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MissingLinkType {
    /// Criteria has no tests linked
    CriteriaHasNoTests,
    /// Test has no results linked
    TestHasNoResults,
    /// Result has no evidence linked
    ResultHasNoEvidence,
    /// Test has no file path (code location)
    TestHasNoCodeLocation,
    /// Goal has no criteria linked
    GoalHasNoCriteria,
}

/// Complete traceability chain from a goal
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceabilityChain {
    /// The root goal
    pub goal: Goal,
    /// Criteria items in order
    pub criteria: Vec<CriteriaInChain>,
}

/// Criteria with its linked tests and results
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CriteriaInChain {
    /// The criteria itself
    pub criteria: AcceptanceCriteria,
    /// Test cases for this criteria
    pub tests: Vec<TestInChain>,
}

/// Test case with its results and evidence
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestInChain {
    /// The test case
    pub test_case: TestCase,
    /// Execution results (most recent first)
    pub results: Vec<TestResult>,
    /// Evidence for the latest result
    pub evidence: Vec<Evidence>,
}

/// Counts of items in a traceability chain
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChainCounts {
    pub goals: usize,
    pub criteria: usize,
    pub tests: usize,
    pub results: usize,
    pub evidence: usize,
}

/// Errors from traceability operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TraceabilityError {
    /// Item not found
    NotFound(Uuid),
    /// Storage error
    StorageError(String),
    /// Serialization error
    SerializationError(String),
    /// Link error (e.g., linking to non-existent item)
    LinkError(String),
}

impl std::fmt::Display for TraceabilityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TraceabilityError::NotFound(id) => write!(f, "Item not found: {}", id),
            TraceabilityError::StorageError(msg) => write!(f, "Storage error: {}", msg),
            TraceabilityError::SerializationError(msg) => write!(f, "Serialization error: {}", msg),
            TraceabilityError::LinkError(msg) => write!(f, "Link error: {}", msg),
        }
    }
}

impl std::error::Error for TraceabilityError {}

impl From<TraceabilityError> for crate::SwellError {
    fn from(err: TraceabilityError) -> Self {
        match err {
            TraceabilityError::NotFound(_) => crate::SwellError::TaskNotFound(uuid::Uuid::nil()),
            TraceabilityError::StorageError(_) => crate::SwellError::DatabaseError(err.to_string()),
            TraceabilityError::SerializationError(_) => {
                crate::SwellError::ConfigError(err.to_string())
            }
            TraceabilityError::LinkError(_) => crate::SwellError::InvalidOperation(err.to_string()),
        }
    }
}

// ============================================================================
// In-Memory Store Implementation
// ============================================================================

/// In-memory traceability store for testing
#[derive(Debug, Default)]
pub struct InMemoryTraceabilityStore {
    goals: std::sync::RwLock<HashMap<Uuid, Goal>>,
    criteria: std::sync::RwLock<HashMap<Uuid, AcceptanceCriteria>>,
    tests: std::sync::RwLock<HashMap<Uuid, TestCase>>,
    results: std::sync::RwLock<HashMap<Uuid, TestResult>>,
    evidence: std::sync::RwLock<HashMap<Uuid, Evidence>>,
}

impl InMemoryTraceabilityStore {
    /// Create a new in-memory store
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl TraceabilityStore for InMemoryTraceabilityStore {
    // --- Goal Operations ---

    async fn create_goal(&self, goal: Goal) -> Result<Uuid, TraceabilityError> {
        let id = goal.id;
        self.goals.write().unwrap().insert(id, goal);
        Ok(id)
    }

    async fn get_goal(&self, id: Uuid) -> Result<Option<Goal>, TraceabilityError> {
        Ok(self.goals.read().unwrap().get(&id).cloned())
    }

    async fn update_goal(&self, goal: &Goal) -> Result<(), TraceabilityError> {
        let mut goals = self.goals.write().unwrap();
        if let std::collections::hash_map::Entry::Occupied(mut e) = goals.entry(goal.id) {
            e.insert(goal.clone());
            Ok(())
        } else {
            Err(TraceabilityError::NotFound(goal.id))
        }
    }

    async fn get_goals_for_task(&self, task_id: TaskId) -> Result<Vec<Goal>, TraceabilityError> {
        let goals = self.goals.read().unwrap();
        Ok(goals
            .values()
            .filter(|g| g.task_id == task_id)
            .cloned()
            .collect())
    }

    // --- Criteria Operations ---

    async fn add_criteria(&self, criteria: AcceptanceCriteria) -> Result<Uuid, TraceabilityError> {
        let id = criteria.id;

        // Update goal's criteria list
        let mut goals = self.goals.write().unwrap();
        if let Some(goal) = goals.get_mut(&criteria.goal_id) {
            goal.criteria_ids.push(id);
            goal.updated_at = Utc::now();
        } else {
            return Err(TraceabilityError::LinkError(format!(
                "Goal {} not found",
                criteria.goal_id
            )));
        }

        self.criteria.write().unwrap().insert(id, criteria);
        Ok(id)
    }

    async fn get_criteria(
        &self,
        id: Uuid,
    ) -> Result<Option<AcceptanceCriteria>, TraceabilityError> {
        Ok(self.criteria.read().unwrap().get(&id).cloned())
    }

    async fn update_criteria(
        &self,
        criteria: &AcceptanceCriteria,
    ) -> Result<(), TraceabilityError> {
        let mut criteria_map = self.criteria.write().unwrap();
        if let std::collections::hash_map::Entry::Occupied(mut e) = criteria_map.entry(criteria.id)
        {
            e.insert(criteria.clone());
            Ok(())
        } else {
            Err(TraceabilityError::NotFound(criteria.id))
        }
    }

    async fn get_criteria_for_goal(
        &self,
        goal_id: Uuid,
    ) -> Result<Vec<AcceptanceCriteria>, TraceabilityError> {
        let goals = self.goals.read().unwrap();
        let criteria_map = self.criteria.read().unwrap();

        if let Some(goal) = goals.get(&goal_id) {
            Ok(goal
                .criteria_ids
                .iter()
                .filter_map(|id| criteria_map.get(id).cloned())
                .collect())
        } else {
            Err(TraceabilityError::NotFound(goal_id))
        }
    }

    async fn get_goal_for_criteria(
        &self,
        criteria_id: Uuid,
    ) -> Result<Option<Goal>, TraceabilityError> {
        let criteria_map = self.criteria.read().unwrap();
        let goals = self.goals.read().unwrap();

        if let Some(criteria) = criteria_map.get(&criteria_id) {
            Ok(goals.get(&criteria.goal_id).cloned())
        } else {
            Err(TraceabilityError::NotFound(criteria_id))
        }
    }

    // --- Test Case Operations ---

    async fn add_test_case(&self, test_case: TestCase) -> Result<Uuid, TraceabilityError> {
        let id = test_case.id;

        // Update criteria' test list
        let mut criteria_map = self.criteria.write().unwrap();
        if let Some(criteria) = criteria_map.get_mut(&test_case.criteria_id) {
            criteria.test_ids.push(id);
        } else {
            return Err(TraceabilityError::LinkError(format!(
                "Criteria {} not found",
                test_case.criteria_id
            )));
        }

        self.tests.write().unwrap().insert(id, test_case);
        Ok(id)
    }

    async fn get_test_case(&self, id: Uuid) -> Result<Option<TestCase>, TraceabilityError> {
        Ok(self.tests.read().unwrap().get(&id).cloned())
    }

    async fn update_test_case(&self, test_case: &TestCase) -> Result<(), TraceabilityError> {
        let mut tests = self.tests.write().unwrap();
        if let std::collections::hash_map::Entry::Occupied(mut e) = tests.entry(test_case.id) {
            e.insert(test_case.clone());
            Ok(())
        } else {
            Err(TraceabilityError::NotFound(test_case.id))
        }
    }

    async fn get_tests_for_criteria(
        &self,
        criteria_id: Uuid,
    ) -> Result<Vec<TestCase>, TraceabilityError> {
        let criteria_map = self.criteria.read().unwrap();
        let tests_map = self.tests.read().unwrap();

        if let Some(criteria) = criteria_map.get(&criteria_id) {
            Ok(criteria
                .test_ids
                .iter()
                .filter_map(|id| tests_map.get(id).cloned())
                .collect())
        } else {
            Err(TraceabilityError::NotFound(criteria_id))
        }
    }

    async fn get_criteria_for_test(
        &self,
        test_id: Uuid,
    ) -> Result<Option<AcceptanceCriteria>, TraceabilityError> {
        let tests = self.tests.read().unwrap();
        let criteria_map = self.criteria.read().unwrap();

        if let Some(test) = tests.get(&test_id) {
            Ok(criteria_map.get(&test.criteria_id).cloned())
        } else {
            Err(TraceabilityError::NotFound(test_id))
        }
    }

    // --- Test Result Operations ---

    async fn add_result(&self, result: TestResult) -> Result<Uuid, TraceabilityError> {
        let id = result.id;

        // Update test's result list
        let mut tests = self.tests.write().unwrap();
        if let Some(test) = tests.get_mut(&result.test_id) {
            test.result_ids.push(id);
        } else {
            return Err(TraceabilityError::LinkError(format!(
                "Test {} not found",
                result.test_id
            )));
        }

        self.results.write().unwrap().insert(id, result);
        Ok(id)
    }

    async fn get_result(&self, id: Uuid) -> Result<Option<TestResult>, TraceabilityError> {
        Ok(self.results.read().unwrap().get(&id).cloned())
    }

    async fn get_results_for_test(
        &self,
        test_id: Uuid,
    ) -> Result<Vec<TestResult>, TraceabilityError> {
        let tests = self.tests.read().unwrap();
        let results = self.results.read().unwrap();

        if let Some(test) = tests.get(&test_id) {
            // Sort by executed_at descending (newest first)
            let mut result_list: Vec<TestResult> = test
                .result_ids
                .iter()
                .filter_map(|id| results.get(id).cloned())
                .collect();
            result_list.sort_by(|a, b| b.executed_at.cmp(&a.executed_at));
            Ok(result_list)
        } else {
            Err(TraceabilityError::NotFound(test_id))
        }
    }

    async fn get_test_for_result(
        &self,
        result_id: Uuid,
    ) -> Result<Option<TestCase>, TraceabilityError> {
        let results = self.results.read().unwrap();
        let tests = self.tests.read().unwrap();

        if let Some(result) = results.get(&result_id) {
            Ok(tests.get(&result.test_id).cloned())
        } else {
            Err(TraceabilityError::NotFound(result_id))
        }
    }

    // --- Evidence Operations ---

    async fn add_evidence(&self, evidence: Evidence) -> Result<Uuid, TraceabilityError> {
        let id = evidence.id;

        // Update result's evidence list
        let mut results = self.results.write().unwrap();
        if let Some(result) = results.get_mut(&evidence.result_id) {
            result.evidence_ids.push(id);
        } else {
            return Err(TraceabilityError::LinkError(format!(
                "Result {} not found",
                evidence.result_id
            )));
        }

        self.evidence.write().unwrap().insert(id, evidence);
        Ok(id)
    }

    async fn get_evidence(&self, id: Uuid) -> Result<Option<Evidence>, TraceabilityError> {
        Ok(self.evidence.read().unwrap().get(&id).cloned())
    }

    async fn get_evidence_for_result(
        &self,
        result_id: Uuid,
    ) -> Result<Vec<Evidence>, TraceabilityError> {
        let results = self.results.read().unwrap();
        let evidence_map = self.evidence.read().unwrap();

        if let Some(result) = results.get(&result_id) {
            Ok(result
                .evidence_ids
                .iter()
                .filter_map(|id| evidence_map.get(id).cloned())
                .collect())
        } else {
            Err(TraceabilityError::NotFound(result_id))
        }
    }

    async fn get_result_for_evidence(
        &self,
        evidence_id: Uuid,
    ) -> Result<Option<TestResult>, TraceabilityError> {
        let evidence_map = self.evidence.read().unwrap();
        let results = self.results.read().unwrap();

        if let Some(evidence) = evidence_map.get(&evidence_id) {
            Ok(results.get(&evidence.result_id).cloned())
        } else {
            Err(TraceabilityError::NotFound(evidence_id))
        }
    }

    // --- Full Traceability Navigation ---

    async fn get_full_chain(
        &self,
        goal_id: Uuid,
    ) -> Result<Option<TraceabilityChain>, TraceabilityError> {
        let goal = self
            .get_goal(goal_id)
            .await?
            .ok_or(TraceabilityError::NotFound(goal_id))?;

        let criteria_list = self.get_criteria_for_goal(goal_id).await?;
        let mut criteria_chain = Vec::new();

        for criteria in criteria_list {
            let test_list = self.get_tests_for_criteria(criteria.id).await?;
            let mut tests_chain = Vec::new();

            for test in test_list {
                let results = self.get_results_for_test(test.id).await?;
                let mut evidence_list = Vec::new();

                // Get evidence for the most recent result
                if let Some(latest_result) = results.first() {
                    evidence_list = self.get_evidence_for_result(latest_result.id).await?;
                }

                tests_chain.push(TestInChain {
                    test_case: test,
                    results,
                    evidence: evidence_list,
                });
            }

            criteria_chain.push(CriteriaInChain {
                criteria,
                tests: tests_chain,
            });
        }

        Ok(Some(TraceabilityChain {
            goal,
            criteria: criteria_chain,
        }))
    }

    async fn get_reverse_chain(
        &self,
        evidence_id: Uuid,
    ) -> Result<Option<TraceabilityChain>, TraceabilityError> {
        // Navigate backwards: Evidence → Result → Test → Criteria → Goal
        let _evidence = self
            .get_evidence(evidence_id)
            .await?
            .ok_or(TraceabilityError::NotFound(evidence_id))?;

        let result = self
            .get_result_for_evidence(evidence_id)
            .await?
            .ok_or(TraceabilityError::NotFound(evidence_id))?;

        let test = self
            .get_test_for_result(result.id)
            .await?
            .ok_or(TraceabilityError::NotFound(result.id))?;

        let criteria = self
            .get_criteria_for_test(test.id)
            .await?
            .ok_or(TraceabilityError::NotFound(test.id))?;

        let goal = self
            .get_goal_for_criteria(criteria.id)
            .await?
            .ok_or(TraceabilityError::NotFound(criteria.id))?;

        // Build forward chain for consistency
        self.get_full_chain(goal.id).await
    }

    async fn count_chain_items(&self, goal_id: Uuid) -> Result<ChainCounts, TraceabilityError> {
        let chain = self.get_full_chain(goal_id).await?;

        match chain {
            Some(c) => Ok(ChainCounts {
                goals: 1,
                criteria: c.criteria.len(),
                tests: c.criteria.iter().map(|cv| cv.tests.len()).sum(),
                results: c
                    .criteria
                    .iter()
                    .flat_map(|cv| cv.tests.iter())
                    .flat_map(|tv| tv.results.iter())
                    .count(),
                evidence: c
                    .criteria
                    .iter()
                    .flat_map(|cv| cv.tests.iter())
                    .flat_map(|tv| tv.evidence.iter())
                    .count(),
            }),
            None => Err(TraceabilityError::NotFound(goal_id)),
        }
    }

    async fn assemble_evidence_pack(
        &self,
        goal_id: Uuid,
    ) -> Result<TraceabilityEvidencePack, TraceabilityError> {
        let chain = self
            .get_full_chain(goal_id)
            .await?
            .ok_or(TraceabilityError::NotFound(goal_id))?;

        let mut criteria_evidence = Vec::new();
        let mut total_tests = 0;
        let mut passed_tests = 0;
        let mut failed_tests = 0;
        let mut code_locations = Vec::new();
        let mut artifacts = Vec::new();

        for criteria_in_chain in &chain.criteria {
            let mut test_evidence_list = Vec::new();

            for test_in_chain in &criteria_in_chain.tests {
                total_tests += 1;
                let passed = test_in_chain
                    .results
                    .first()
                    .map(|r| r.passed)
                    .unwrap_or(false);
                if passed {
                    passed_tests += 1;
                } else if !test_in_chain.results.is_empty() {
                    failed_tests += 1;
                }

                // Collect code location from test case
                let code_location =
                    test_in_chain
                        .test_case
                        .file_path
                        .as_ref()
                        .map(|path| CodeLocation {
                            file_path: path.clone(),
                            line_start: 0,
                            line_end: 0,
                            description: Some(test_in_chain.test_case.name.clone()),
                        });

                // Collect artifacts from evidence
                for ev in &test_in_chain.evidence {
                    artifacts.push(EvidenceArtifact {
                        id: ev.id,
                        evidence_type: ev.evidence_type,
                        artifact_path: ev.artifact_path.clone(),
                        description: ev.description.clone(),
                        created_at: ev.created_at,
                    });
                }

                // Collect code locations
                if let Some(loc) = &code_location {
                    code_locations.push(loc.clone());
                }

                test_evidence_list.push(TestCaseEvidence {
                    test_case: test_in_chain.test_case.clone(),
                    results: test_in_chain.results.clone(),
                    evidence: test_in_chain.evidence.clone(),
                    code_location,
                });
            }

            criteria_evidence.push(CriteriaEvidence {
                criteria: criteria_in_chain.criteria.clone(),
                tests: test_evidence_list,
            });
        }

        Ok(TraceabilityEvidencePack {
            id: Uuid::new_v4(),
            requirement_id: goal_id,
            assembled_at: Utc::now(),
            criteria: criteria_evidence,
            total_tests,
            passed_tests,
            failed_tests,
            code_locations,
            artifacts,
        })
    }

    async fn detect_missing_links(
        &self,
        goal_id: Uuid,
    ) -> Result<Vec<MissingLink>, TraceabilityError> {
        let mut missing = Vec::new();

        // Check if goal exists and has criteria
        let chain = self.get_full_chain(goal_id).await?;
        let chain = match chain {
            Some(c) => c,
            None => return Err(TraceabilityError::NotFound(goal_id)),
        };

        // Check goal has criteria
        if chain.criteria.is_empty() {
            missing.push(MissingLink {
                link_type: MissingLinkType::GoalHasNoCriteria,
                parent_id: goal_id,
                description: "Goal has no acceptance criteria linked".to_string(),
            });
        }

        // Check each criteria for missing tests
        for criteria_in_chain in &chain.criteria {
            if criteria_in_chain.tests.is_empty() {
                missing.push(MissingLink {
                    link_type: MissingLinkType::CriteriaHasNoTests,
                    parent_id: criteria_in_chain.criteria.id,
                    description: format!(
                        "Criteria '{}' has no tests linked",
                        criteria_in_chain.criteria.text
                    ),
                });
            }

            // Check each test for missing results and code location
            for test_in_chain in &criteria_in_chain.tests {
                if test_in_chain.results.is_empty() {
                    missing.push(MissingLink {
                        link_type: MissingLinkType::TestHasNoResults,
                        parent_id: test_in_chain.test_case.id,
                        description: format!(
                            "Test '{}' has no results linked",
                            test_in_chain.test_case.name
                        ),
                    });
                }

                if test_in_chain.test_case.file_path.is_none() {
                    missing.push(MissingLink {
                        link_type: MissingLinkType::TestHasNoCodeLocation,
                        parent_id: test_in_chain.test_case.id,
                        description: format!(
                            "Test '{}' has no code location (file_path)",
                            test_in_chain.test_case.name
                        ),
                    });
                }

                // Check each result for missing evidence
                for result in &test_in_chain.results {
                    let result_evidence = self.get_evidence_for_result(result.id).await?;
                    if result_evidence.is_empty() {
                        missing.push(MissingLink {
                            link_type: MissingLinkType::ResultHasNoEvidence,
                            parent_id: result.id,
                            description: format!(
                                "Result for test '{}' has no evidence linked",
                                test_in_chain.test_case.name
                            ),
                        });
                    }
                }
            }
        }

        Ok(missing)
    }
}

// ============================================================================
// SQLite Store Implementation
// ============================================================================

pub mod sqlite_store {
    use super::*;
    use std::path::Path;

    /// SQLite-based traceability store for production use
    #[derive(Debug, Clone)]
    pub struct SqliteTraceabilityStore {
        pool: sqlx::SqlitePool,
    }

    impl SqliteTraceabilityStore {
        /// Create a new SQLite store
        pub async fn new<P: AsRef<Path>>(db_path: P) -> Result<Self, TraceabilityError> {
            let database_url = format!("sqlite:{}?mode=rwc", db_path.as_ref().display());

            let pool = sqlx::sqlite::SqlitePoolOptions::new()
                .max_connections(1)
                .connect(&database_url)
                .await
                .map_err(|e| TraceabilityError::StorageError(e.to_string()))?;

            let store = Self { pool };
            store.init_schema().await?;
            Ok(store)
        }

        /// Create from connection string
        pub async fn from_connection_string(conn_str: &str) -> Result<Self, TraceabilityError> {
            let pool = sqlx::sqlite::SqlitePoolOptions::new()
                .max_connections(1)
                .connect(conn_str)
                .await
                .map_err(|e| TraceabilityError::StorageError(e.to_string()))?;

            let store = Self { pool };
            store.init_schema().await?;
            Ok(store)
        }

        /// Initialize database schema
        async fn init_schema(&self) -> Result<(), TraceabilityError> {
            // Goals table
            sqlx::query(
                r#"
                CREATE TABLE IF NOT EXISTS traceability_goals (
                    id TEXT PRIMARY KEY,
                    task_id TEXT NOT NULL,
                    description TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL,
                    status TEXT NOT NULL,
                    criteria_ids TEXT NOT NULL DEFAULT '[]'
                )
                "#,
            )
            .execute(&self.pool)
            .await
            .map_err(|e| TraceabilityError::StorageError(e.to_string()))?;

            // Criteria table
            sqlx::query(
                r#"
                CREATE TABLE IF NOT EXISTS traceability_criteria (
                    id TEXT PRIMARY KEY,
                    goal_id TEXT NOT NULL,
                    text TEXT NOT NULL,
                    category TEXT NOT NULL,
                    criticality TEXT NOT NULL,
                    status TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    test_ids TEXT NOT NULL DEFAULT '[]',
                    FOREIGN KEY (goal_id) REFERENCES traceability_goals(id)
                )
                "#,
            )
            .execute(&self.pool)
            .await
            .map_err(|e| TraceabilityError::StorageError(e.to_string()))?;

            // Test cases table
            sqlx::query(
                r#"
                CREATE TABLE IF NOT EXISTS traceability_tests (
                    id TEXT PRIMARY KEY,
                    criteria_id TEXT NOT NULL,
                    name TEXT NOT NULL,
                    file_path TEXT,
                    test_type TEXT NOT NULL,
                    status TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    result_ids TEXT NOT NULL DEFAULT '[]',
                    FOREIGN KEY (criteria_id) REFERENCES traceability_criteria(id)
                )
                "#,
            )
            .execute(&self.pool)
            .await
            .map_err(|e| TraceabilityError::StorageError(e.to_string()))?;

            // Results table
            sqlx::query(
                r#"
                CREATE TABLE IF NOT EXISTS traceability_results (
                    id TEXT PRIMARY KEY,
                    test_id TEXT NOT NULL,
                    passed INTEGER NOT NULL,
                    duration_ms INTEGER NOT NULL,
                    failure_message TEXT,
                    executed_at TEXT NOT NULL,
                    evidence_ids TEXT NOT NULL DEFAULT '[]',
                    FOREIGN KEY (test_id) REFERENCES traceability_tests(id)
                )
                "#,
            )
            .execute(&self.pool)
            .await
            .map_err(|e| TraceabilityError::StorageError(e.to_string()))?;

            // Evidence table
            sqlx::query(
                r#"
                CREATE TABLE IF NOT EXISTS traceability_evidence (
                    id TEXT PRIMARY KEY,
                    result_id TEXT NOT NULL,
                    evidence_type TEXT NOT NULL,
                    artifact_path TEXT NOT NULL,
                    description TEXT,
                    created_at TEXT NOT NULL,
                    FOREIGN KEY (result_id) REFERENCES traceability_results(id)
                )
                "#,
            )
            .execute(&self.pool)
            .await
            .map_err(|e| TraceabilityError::StorageError(e.to_string()))?;

            // Indexes
            sqlx::query(
                "CREATE INDEX IF NOT EXISTS idx_goals_task_id ON traceability_goals(task_id)",
            )
            .execute(&self.pool)
            .await
            .map_err(|e| TraceabilityError::StorageError(e.to_string()))?;

            sqlx::query(
                "CREATE INDEX IF NOT EXISTS idx_criteria_goal_id ON traceability_criteria(goal_id)",
            )
            .execute(&self.pool)
            .await
            .map_err(|e| TraceabilityError::StorageError(e.to_string()))?;

            sqlx::query(
                "CREATE INDEX IF NOT EXISTS idx_tests_criteria_id ON traceability_tests(criteria_id)"
            )
            .execute(&self.pool)
            .await
            .map_err(|e| TraceabilityError::StorageError(e.to_string()))?;

            sqlx::query(
                "CREATE INDEX IF NOT EXISTS idx_results_test_id ON traceability_results(test_id)",
            )
            .execute(&self.pool)
            .await
            .map_err(|e| TraceabilityError::StorageError(e.to_string()))?;

            Ok(())
        }

        fn serialize_ids(ids: &[Uuid]) -> String {
            serde_json::to_string(ids).unwrap_or_else(|_| "[]".to_string())
        }

        fn deserialize_ids(s: &str) -> Vec<Uuid> {
            serde_json::from_str(s).unwrap_or_default()
        }
    }

    #[async_trait]
    impl TraceabilityStore for SqliteTraceabilityStore {
        async fn create_goal(&self, goal: Goal) -> Result<Uuid, TraceabilityError> {
            let id = goal.id.to_string();
            let criteria_ids = Self::serialize_ids(&goal.criteria_ids);

            sqlx::query(
                r#"
                INSERT INTO traceability_goals (id, task_id, description, created_at, updated_at, status, criteria_ids)
                VALUES (?, ?, ?, ?, ?, ?, ?)
                "#,
            )
            .bind(&id)
            .bind(goal.task_id.to_string())
            .bind(&goal.description)
            .bind(goal.created_at.to_rfc3339())
            .bind(goal.updated_at.to_rfc3339())
            .bind(format!("{:?}", goal.status))
            .bind(&criteria_ids)
            .execute(&self.pool)
            .await
            .map_err(|e| TraceabilityError::StorageError(e.to_string()))?;

            Ok(goal.id)
        }

        async fn get_goal(&self, id: Uuid) -> Result<Option<Goal>, TraceabilityError> {
            let id_str = id.to_string();

            let row: Option<(String, String, String, String, String, String, String)> =
                sqlx::query_as(
                    "SELECT id, task_id, description, created_at, updated_at, status, criteria_ids FROM traceability_goals WHERE id = ?"
                )
                .bind(&id_str)
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| TraceabilityError::StorageError(e.to_string()))?;

            match row {
                Some((id, task_id, description, created_at, updated_at, status, criteria_ids)) => {
                    Ok(Some(Goal {
                        id: Uuid::parse_str(&id)
                            .map_err(|e| TraceabilityError::SerializationError(e.to_string()))?,
                        task_id: TaskId::from_uuid(
                            Uuid::parse_str(&task_id).map_err(|e| {
                                TraceabilityError::SerializationError(e.to_string())
                            })?,
                        ),
                        description,
                        created_at: DateTime::parse_from_rfc3339(&created_at)
                            .map_err(|e| TraceabilityError::SerializationError(e.to_string()))?
                            .with_timezone(&Utc),
                        updated_at: DateTime::parse_from_rfc3339(&updated_at)
                            .map_err(|e| TraceabilityError::SerializationError(e.to_string()))?
                            .with_timezone(&Utc),
                        status: serde_json::from_str(&format!("\"{}\"", status))
                            .unwrap_or(GoalStatus::Active),
                        criteria_ids: Self::deserialize_ids(&criteria_ids),
                    }))
                }
                None => Ok(None),
            }
        }

        async fn update_goal(&self, goal: &Goal) -> Result<(), TraceabilityError> {
            let id = goal.id.to_string();
            let criteria_ids = Self::serialize_ids(&goal.criteria_ids);

            let result = sqlx::query(
                r#"
                UPDATE traceability_goals 
                SET task_id = ?, description = ?, updated_at = ?, status = ?, criteria_ids = ?
                WHERE id = ?
                "#,
            )
            .bind(goal.task_id.to_string())
            .bind(&goal.description)
            .bind(goal.updated_at.to_rfc3339())
            .bind(format!("{:?}", goal.status))
            .bind(&criteria_ids)
            .bind(&id)
            .execute(&self.pool)
            .await
            .map_err(|e| TraceabilityError::StorageError(e.to_string()))?;

            if result.rows_affected() == 0 {
                Err(TraceabilityError::NotFound(goal.id))
            } else {
                Ok(())
            }
        }

        async fn get_goals_for_task(
            &self,
            task_id: TaskId,
        ) -> Result<Vec<Goal>, TraceabilityError> {
            let task_id_str = task_id.to_string();

            let rows: Vec<(String, String, String, String, String, String, String)> =
                sqlx::query_as(
                    "SELECT id, task_id, description, created_at, updated_at, status, criteria_ids FROM traceability_goals WHERE task_id = ?"
                )
                .bind(&task_id_str)
                .fetch_all(&self.pool)
                .await
                .map_err(|e| TraceabilityError::StorageError(e.to_string()))?;

            let mut goals = Vec::new();
            for (id, task_id, description, created_at, updated_at, status, criteria_ids) in rows {
                goals.push(Goal {
                    id: Uuid::parse_str(&id)
                        .map_err(|e| TraceabilityError::SerializationError(e.to_string()))?,
                    task_id: TaskId::from_uuid(
                        Uuid::parse_str(&task_id)
                            .map_err(|e| TraceabilityError::SerializationError(e.to_string()))?,
                    ),
                    description,
                    created_at: DateTime::parse_from_rfc3339(&created_at)
                        .map_err(|e| TraceabilityError::SerializationError(e.to_string()))?
                        .with_timezone(&Utc),
                    updated_at: DateTime::parse_from_rfc3339(&updated_at)
                        .map_err(|e| TraceabilityError::SerializationError(e.to_string()))?
                        .with_timezone(&Utc),
                    status: serde_json::from_str(&format!("\"{}\"", status))
                        .unwrap_or(GoalStatus::Active),
                    criteria_ids: Self::deserialize_ids(&criteria_ids),
                });
            }

            Ok(goals)
        }

        async fn add_criteria(
            &self,
            criteria: AcceptanceCriteria,
        ) -> Result<Uuid, TraceabilityError> {
            let id = criteria.id.to_string();
            let test_ids = Self::serialize_ids(&criteria.test_ids);

            // Insert criteria
            sqlx::query(
                r#"
                INSERT INTO traceability_criteria (id, goal_id, text, category, criticality, status, created_at, test_ids)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?)
                "#,
            )
            .bind(&id)
            .bind(criteria.goal_id.to_string())
            .bind(&criteria.text)
            .bind(&criteria.category)
            .bind(format!("{:?}", criteria.criticality))
            .bind(format!("{:?}", criteria.status))
            .bind(criteria.created_at.to_rfc3339())
            .bind(&test_ids)
            .execute(&self.pool)
            .await
            .map_err(|e| TraceabilityError::StorageError(e.to_string()))?;

            // Update goal's criteria list
            let mut goal = self.get_goal(criteria.goal_id).await?.ok_or_else(|| {
                TraceabilityError::LinkError(format!("Goal {} not found", criteria.goal_id))
            })?;
            goal.criteria_ids.push(criteria.id);
            goal.updated_at = Utc::now();
            self.update_goal(&goal).await?;

            Ok(criteria.id)
        }

        async fn get_criteria(
            &self,
            id: Uuid,
        ) -> Result<Option<AcceptanceCriteria>, TraceabilityError> {
            let id_str = id.to_string();

            let row: Option<(String, String, String, String, String, String, String, String)> =
                sqlx::query_as(
                    "SELECT id, goal_id, text, category, criticality, status, created_at, test_ids FROM traceability_criteria WHERE id = ?"
                )
                .bind(&id_str)
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| TraceabilityError::StorageError(e.to_string()))?;

            match row {
                Some((id, goal_id, text, category, criticality, status, created_at, test_ids)) => {
                    Ok(Some(AcceptanceCriteria {
                        id: Uuid::parse_str(&id)
                            .map_err(|e| TraceabilityError::SerializationError(e.to_string()))?,
                        goal_id: Uuid::parse_str(&goal_id)
                            .map_err(|e| TraceabilityError::SerializationError(e.to_string()))?,
                        text,
                        category,
                        criticality: serde_json::from_str(&format!("\"{}\"", criticality))
                            .unwrap_or(CriteriaCriticality::ShouldHave),
                        status: serde_json::from_str(&format!("\"{}\"", status))
                            .unwrap_or(CriteriaStatus::Pending),
                        created_at: DateTime::parse_from_rfc3339(&created_at)
                            .map_err(|e| TraceabilityError::SerializationError(e.to_string()))?
                            .with_timezone(&Utc),
                        test_ids: Self::deserialize_ids(&test_ids),
                    }))
                }
                None => Ok(None),
            }
        }

        async fn update_criteria(
            &self,
            criteria: &AcceptanceCriteria,
        ) -> Result<(), TraceabilityError> {
            let id = criteria.id.to_string();
            let test_ids = Self::serialize_ids(&criteria.test_ids);

            let result = sqlx::query(
                r#"
                UPDATE traceability_criteria 
                SET goal_id = ?, text = ?, category = ?, criticality = ?, status = ?, test_ids = ?
                WHERE id = ?
                "#,
            )
            .bind(criteria.goal_id.to_string())
            .bind(&criteria.text)
            .bind(&criteria.category)
            .bind(format!("{:?}", criteria.criticality))
            .bind(format!("{:?}", criteria.status))
            .bind(&test_ids)
            .bind(&id)
            .execute(&self.pool)
            .await
            .map_err(|e| TraceabilityError::StorageError(e.to_string()))?;

            if result.rows_affected() == 0 {
                Err(TraceabilityError::NotFound(criteria.id))
            } else {
                Ok(())
            }
        }

        async fn get_criteria_for_goal(
            &self,
            goal_id: Uuid,
        ) -> Result<Vec<AcceptanceCriteria>, TraceabilityError> {
            let goal_id_str = goal_id.to_string();

            let rows: Vec<(String, String, String, String, String, String, String, String)> =
                sqlx::query_as(
                    "SELECT id, goal_id, text, category, criticality, status, created_at, test_ids FROM traceability_criteria WHERE goal_id = ?"
                )
                .bind(&goal_id_str)
                .fetch_all(&self.pool)
                .await
                .map_err(|e| TraceabilityError::StorageError(e.to_string()))?;

            let mut criteria_list = Vec::new();
            for (id, goal_id, text, category, criticality, status, created_at, test_ids) in rows {
                criteria_list.push(AcceptanceCriteria {
                    id: Uuid::parse_str(&id)
                        .map_err(|e| TraceabilityError::SerializationError(e.to_string()))?,
                    goal_id: Uuid::parse_str(&goal_id)
                        .map_err(|e| TraceabilityError::SerializationError(e.to_string()))?,
                    text,
                    category,
                    criticality: serde_json::from_str(&format!("\"{}\"", criticality))
                        .unwrap_or(CriteriaCriticality::ShouldHave),
                    status: serde_json::from_str(&format!("\"{}\"", status))
                        .unwrap_or(CriteriaStatus::Pending),
                    created_at: DateTime::parse_from_rfc3339(&created_at)
                        .map_err(|e| TraceabilityError::SerializationError(e.to_string()))?
                        .with_timezone(&Utc),
                    test_ids: Self::deserialize_ids(&test_ids),
                });
            }

            Ok(criteria_list)
        }

        async fn get_goal_for_criteria(
            &self,
            criteria_id: Uuid,
        ) -> Result<Option<Goal>, TraceabilityError> {
            if let Some(criteria) = self.get_criteria(criteria_id).await? {
                self.get_goal(criteria.goal_id).await
            } else {
                Err(TraceabilityError::NotFound(criteria_id))
            }
        }

        async fn add_test_case(&self, test_case: TestCase) -> Result<Uuid, TraceabilityError> {
            let id = test_case.id.to_string();
            let result_ids = Self::serialize_ids(&test_case.result_ids);

            sqlx::query(
                r#"
                INSERT INTO traceability_tests (id, criteria_id, name, file_path, test_type, status, created_at, result_ids)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?)
                "#,
            )
            .bind(&id)
            .bind(test_case.criteria_id.to_string())
            .bind(&test_case.name)
            .bind(&test_case.file_path)
            .bind(format!("{:?}", test_case.test_type))
            .bind(format!("{:?}", test_case.status))
            .bind(test_case.created_at.to_rfc3339())
            .bind(&result_ids)
            .execute(&self.pool)
            .await
            .map_err(|e| TraceabilityError::StorageError(e.to_string()))?;

            // Update criteria' test list
            let mut criteria =
                self.get_criteria(test_case.criteria_id)
                    .await?
                    .ok_or_else(|| {
                        TraceabilityError::LinkError(format!(
                            "Criteria {} not found",
                            test_case.criteria_id
                        ))
                    })?;
            criteria.test_ids.push(test_case.id);
            self.update_criteria(&criteria).await?;

            Ok(test_case.id)
        }

        async fn get_test_case(&self, id: Uuid) -> Result<Option<TestCase>, TraceabilityError> {
            let id_str = id.to_string();

            let row: Option<(String, String, String, Option<String>, String, String, String, String)> =
                sqlx::query_as(
                    "SELECT id, criteria_id, name, file_path, test_type, status, created_at, result_ids FROM traceability_tests WHERE id = ?"
                )
                .bind(&id_str)
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| TraceabilityError::StorageError(e.to_string()))?;

            match row {
                Some((
                    id,
                    criteria_id,
                    name,
                    file_path,
                    test_type,
                    status,
                    created_at,
                    result_ids,
                )) => Ok(Some(TestCase {
                    id: Uuid::parse_str(&id)
                        .map_err(|e| TraceabilityError::SerializationError(e.to_string()))?,
                    criteria_id: Uuid::parse_str(&criteria_id)
                        .map_err(|e| TraceabilityError::SerializationError(e.to_string()))?,
                    name,
                    file_path,
                    test_type: serde_json::from_str(&format!("\"{}\"", test_type))
                        .unwrap_or(TestCaseType::Unit),
                    status: serde_json::from_str(&format!("\"{}\"", status))
                        .unwrap_or(TestCaseStatus::Pending),
                    created_at: DateTime::parse_from_rfc3339(&created_at)
                        .map_err(|e| TraceabilityError::SerializationError(e.to_string()))?
                        .with_timezone(&Utc),
                    result_ids: Self::deserialize_ids(&result_ids),
                })),
                None => Ok(None),
            }
        }

        async fn update_test_case(&self, test_case: &TestCase) -> Result<(), TraceabilityError> {
            let id = test_case.id.to_string();
            let result_ids = Self::serialize_ids(&test_case.result_ids);

            let result = sqlx::query(
                r#"
                UPDATE traceability_tests 
                SET criteria_id = ?, name = ?, file_path = ?, test_type = ?, status = ?, result_ids = ?
                WHERE id = ?
                "#,
            )
            .bind(test_case.criteria_id.to_string())
            .bind(&test_case.name)
            .bind(&test_case.file_path)
            .bind(format!("{:?}", test_case.test_type))
            .bind(format!("{:?}", test_case.status))
            .bind(&result_ids)
            .bind(&id)
            .execute(&self.pool)
            .await
            .map_err(|e| TraceabilityError::StorageError(e.to_string()))?;

            if result.rows_affected() == 0 {
                Err(TraceabilityError::NotFound(test_case.id))
            } else {
                Ok(())
            }
        }

        async fn get_tests_for_criteria(
            &self,
            criteria_id: Uuid,
        ) -> Result<Vec<TestCase>, TraceabilityError> {
            let criteria_id_str = criteria_id.to_string();

            let rows: Vec<(String, String, String, Option<String>, String, String, String, String)> =
                sqlx::query_as(
                    "SELECT id, criteria_id, name, file_path, test_type, status, created_at, result_ids FROM traceability_tests WHERE criteria_id = ?"
                )
                .bind(&criteria_id_str)
                .fetch_all(&self.pool)
                .await
                .map_err(|e| TraceabilityError::StorageError(e.to_string()))?;

            let mut tests = Vec::new();
            for (id, criteria_id, name, file_path, test_type, status, created_at, result_ids) in
                rows
            {
                tests.push(TestCase {
                    id: Uuid::parse_str(&id)
                        .map_err(|e| TraceabilityError::SerializationError(e.to_string()))?,
                    criteria_id: Uuid::parse_str(&criteria_id)
                        .map_err(|e| TraceabilityError::SerializationError(e.to_string()))?,
                    name,
                    file_path,
                    test_type: serde_json::from_str(&format!("\"{}\"", test_type))
                        .unwrap_or(TestCaseType::Unit),
                    status: serde_json::from_str(&format!("\"{}\"", status))
                        .unwrap_or(TestCaseStatus::Pending),
                    created_at: DateTime::parse_from_rfc3339(&created_at)
                        .map_err(|e| TraceabilityError::SerializationError(e.to_string()))?
                        .with_timezone(&Utc),
                    result_ids: Self::deserialize_ids(&result_ids),
                });
            }

            Ok(tests)
        }

        async fn get_criteria_for_test(
            &self,
            test_id: Uuid,
        ) -> Result<Option<AcceptanceCriteria>, TraceabilityError> {
            if let Some(test) = self.get_test_case(test_id).await? {
                self.get_criteria(test.criteria_id).await
            } else {
                Err(TraceabilityError::NotFound(test_id))
            }
        }

        async fn add_result(&self, result: TestResult) -> Result<Uuid, TraceabilityError> {
            let id = result.id.to_string();
            let evidence_ids = Self::serialize_ids(&result.evidence_ids);

            sqlx::query(
                r#"
                INSERT INTO traceability_results (id, test_id, passed, duration_ms, failure_message, executed_at, evidence_ids)
                VALUES (?, ?, ?, ?, ?, ?, ?)
                "#,
            )
            .bind(&id)
            .bind(result.test_id.to_string())
            .bind(if result.passed { 1 } else { 0 })
            .bind(result.duration_ms as i64)
            .bind(&result.failure_message)
            .bind(result.executed_at.to_rfc3339())
            .bind(&evidence_ids)
            .execute(&self.pool)
            .await
            .map_err(|e| TraceabilityError::StorageError(e.to_string()))?;

            // Update test's result list
            let mut test = self.get_test_case(result.test_id).await?.ok_or_else(|| {
                TraceabilityError::LinkError(format!("Test {} not found", result.test_id))
            })?;
            test.result_ids.push(result.id);
            self.update_test_case(&test).await?;

            Ok(result.id)
        }

        async fn get_result(&self, id: Uuid) -> Result<Option<TestResult>, TraceabilityError> {
            let id_str = id.to_string();

            let row: Option<(String, String, i64, i64, Option<String>, String, String)> =
                sqlx::query_as(
                    "SELECT id, test_id, passed, duration_ms, failure_message, executed_at, evidence_ids FROM traceability_results WHERE id = ?"
                )
                .bind(&id_str)
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| TraceabilityError::StorageError(e.to_string()))?;

            match row {
                Some((
                    id,
                    test_id,
                    passed,
                    duration_ms,
                    failure_message,
                    executed_at,
                    evidence_ids,
                )) => Ok(Some(TestResult {
                    id: Uuid::parse_str(&id)
                        .map_err(|e| TraceabilityError::SerializationError(e.to_string()))?,
                    test_id: Uuid::parse_str(&test_id)
                        .map_err(|e| TraceabilityError::SerializationError(e.to_string()))?,
                    passed: passed != 0,
                    duration_ms: duration_ms as u64,
                    failure_message,
                    executed_at: DateTime::parse_from_rfc3339(&executed_at)
                        .map_err(|e| TraceabilityError::SerializationError(e.to_string()))?
                        .with_timezone(&Utc),
                    evidence_ids: Self::deserialize_ids(&evidence_ids),
                })),
                None => Ok(None),
            }
        }

        async fn get_results_for_test(
            &self,
            test_id: Uuid,
        ) -> Result<Vec<TestResult>, TraceabilityError> {
            let test_id_str = test_id.to_string();

            let rows: Vec<(String, String, i64, i64, Option<String>, String, String)> =
                sqlx::query_as(
                    "SELECT id, test_id, passed, duration_ms, failure_message, executed_at, evidence_ids FROM traceability_results WHERE test_id = ? ORDER BY executed_at DESC"
                )
                .bind(&test_id_str)
                .fetch_all(&self.pool)
                .await
                .map_err(|e| TraceabilityError::StorageError(e.to_string()))?;

            let mut results = Vec::new();
            for (id, test_id, passed, duration_ms, failure_message, executed_at, evidence_ids) in
                rows
            {
                results.push(TestResult {
                    id: Uuid::parse_str(&id)
                        .map_err(|e| TraceabilityError::SerializationError(e.to_string()))?,
                    test_id: Uuid::parse_str(&test_id)
                        .map_err(|e| TraceabilityError::SerializationError(e.to_string()))?,
                    passed: passed != 0,
                    duration_ms: duration_ms as u64,
                    failure_message,
                    executed_at: DateTime::parse_from_rfc3339(&executed_at)
                        .map_err(|e| TraceabilityError::SerializationError(e.to_string()))?
                        .with_timezone(&Utc),
                    evidence_ids: Self::deserialize_ids(&evidence_ids),
                });
            }

            Ok(results)
        }

        async fn get_test_for_result(
            &self,
            result_id: Uuid,
        ) -> Result<Option<TestCase>, TraceabilityError> {
            if let Some(result) = self.get_result(result_id).await? {
                self.get_test_case(result.test_id).await
            } else {
                Err(TraceabilityError::NotFound(result_id))
            }
        }

        async fn add_evidence(&self, evidence: Evidence) -> Result<Uuid, TraceabilityError> {
            let id = evidence.id.to_string();

            sqlx::query(
                r#"
                INSERT INTO traceability_evidence (id, result_id, evidence_type, artifact_path, description, created_at)
                VALUES (?, ?, ?, ?, ?, ?)
                "#,
            )
            .bind(&id)
            .bind(evidence.result_id.to_string())
            .bind(format!("{:?}", evidence.evidence_type))
            .bind(&evidence.artifact_path)
            .bind(&evidence.description)
            .bind(evidence.created_at.to_rfc3339())
            .execute(&self.pool)
            .await
            .map_err(|e| TraceabilityError::StorageError(e.to_string()))?;

            // Update result's evidence list
            let mut result = self.get_result(evidence.result_id).await?.ok_or_else(|| {
                TraceabilityError::LinkError(format!("Result {} not found", evidence.result_id))
            })?;
            result.evidence_ids.push(evidence.id);
            let result_id = result.id;
            let evidence_ids = Self::serialize_ids(&result.evidence_ids);

            sqlx::query("UPDATE traceability_results SET evidence_ids = ? WHERE id = ?")
                .bind(&evidence_ids)
                .bind(result_id.to_string())
                .execute(&self.pool)
                .await
                .map_err(|e| TraceabilityError::StorageError(e.to_string()))?;

            Ok(evidence.id)
        }

        async fn get_evidence(&self, id: Uuid) -> Result<Option<Evidence>, TraceabilityError> {
            let id_str = id.to_string();

            let row: Option<(String, String, String, String, Option<String>, String)> =
                sqlx::query_as(
                    "SELECT id, result_id, evidence_type, artifact_path, description, created_at FROM traceability_evidence WHERE id = ?"
                )
                .bind(&id_str)
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| TraceabilityError::StorageError(e.to_string()))?;

            match row {
                Some((id, result_id, evidence_type, artifact_path, description, created_at)) => {
                    Ok(Some(Evidence {
                        id: Uuid::parse_str(&id)
                            .map_err(|e| TraceabilityError::SerializationError(e.to_string()))?,
                        result_id: Uuid::parse_str(&result_id)
                            .map_err(|e| TraceabilityError::SerializationError(e.to_string()))?,
                        evidence_type: serde_json::from_str(&format!("\"{}\"", evidence_type))
                            .unwrap_or(EvidenceType::Custom),
                        artifact_path,
                        description,
                        created_at: DateTime::parse_from_rfc3339(&created_at)
                            .map_err(|e| TraceabilityError::SerializationError(e.to_string()))?
                            .with_timezone(&Utc),
                    }))
                }
                None => Ok(None),
            }
        }

        async fn get_evidence_for_result(
            &self,
            result_id: Uuid,
        ) -> Result<Vec<Evidence>, TraceabilityError> {
            let result_id_str = result_id.to_string();

            let rows: Vec<(String, String, String, String, Option<String>, String)> =
                sqlx::query_as(
                    "SELECT id, result_id, evidence_type, artifact_path, description, created_at FROM traceability_evidence WHERE result_id = ?"
                )
                .bind(&result_id_str)
                .fetch_all(&self.pool)
                .await
                .map_err(|e| TraceabilityError::StorageError(e.to_string()))?;

            let mut evidence_list = Vec::new();
            for (id, result_id, evidence_type, artifact_path, description, created_at) in rows {
                evidence_list.push(Evidence {
                    id: Uuid::parse_str(&id)
                        .map_err(|e| TraceabilityError::SerializationError(e.to_string()))?,
                    result_id: Uuid::parse_str(&result_id)
                        .map_err(|e| TraceabilityError::SerializationError(e.to_string()))?,
                    evidence_type: serde_json::from_str(&format!("\"{}\"", evidence_type))
                        .unwrap_or(EvidenceType::Custom),
                    artifact_path,
                    description,
                    created_at: DateTime::parse_from_rfc3339(&created_at)
                        .map_err(|e| TraceabilityError::SerializationError(e.to_string()))?
                        .with_timezone(&Utc),
                });
            }

            Ok(evidence_list)
        }

        async fn get_result_for_evidence(
            &self,
            evidence_id: Uuid,
        ) -> Result<Option<TestResult>, TraceabilityError> {
            if let Some(evidence) = self.get_evidence(evidence_id).await? {
                self.get_result(evidence.result_id).await
            } else {
                Err(TraceabilityError::NotFound(evidence_id))
            }
        }

        async fn get_full_chain(
            &self,
            goal_id: Uuid,
        ) -> Result<Option<TraceabilityChain>, TraceabilityError> {
            let goal = self
                .get_goal(goal_id)
                .await?
                .ok_or(TraceabilityError::NotFound(goal_id))?;

            let criteria_list = self.get_criteria_for_goal(goal_id).await?;
            let mut criteria_chain = Vec::new();

            for criteria in criteria_list {
                let test_list = self.get_tests_for_criteria(criteria.id).await?;
                let mut tests_chain = Vec::new();

                for test in test_list {
                    let results = self.get_results_for_test(test.id).await?;
                    let mut evidence_list = Vec::new();

                    if let Some(latest_result) = results.first() {
                        evidence_list = self.get_evidence_for_result(latest_result.id).await?;
                    }

                    tests_chain.push(TestInChain {
                        test_case: test,
                        results,
                        evidence: evidence_list,
                    });
                }

                criteria_chain.push(CriteriaInChain {
                    criteria,
                    tests: tests_chain,
                });
            }

            Ok(Some(TraceabilityChain {
                goal,
                criteria: criteria_chain,
            }))
        }

        async fn get_reverse_chain(
            &self,
            evidence_id: Uuid,
        ) -> Result<Option<TraceabilityChain>, TraceabilityError> {
            let _evidence = self
                .get_evidence(evidence_id)
                .await?
                .ok_or(TraceabilityError::NotFound(evidence_id))?;

            let result = self
                .get_result_for_evidence(evidence_id)
                .await?
                .ok_or(TraceabilityError::NotFound(evidence_id))?;

            let test = self
                .get_test_for_result(result.id)
                .await?
                .ok_or(TraceabilityError::NotFound(result.id))?;

            let criteria = self
                .get_criteria_for_test(test.id)
                .await?
                .ok_or(TraceabilityError::NotFound(test.id))?;

            let goal = self
                .get_goal_for_criteria(criteria.id)
                .await?
                .ok_or(TraceabilityError::NotFound(criteria.id))?;

            self.get_full_chain(goal.id).await
        }

        async fn count_chain_items(&self, goal_id: Uuid) -> Result<ChainCounts, TraceabilityError> {
            let chain = self.get_full_chain(goal_id).await?;

            match chain {
                Some(c) => Ok(ChainCounts {
                    goals: 1,
                    criteria: c.criteria.len(),
                    tests: c.criteria.iter().map(|cv| cv.tests.len()).sum(),
                    results: c
                        .criteria
                        .iter()
                        .flat_map(|cv| cv.tests.iter())
                        .flat_map(|tv| tv.results.iter())
                        .count(),
                    evidence: c
                        .criteria
                        .iter()
                        .flat_map(|cv| cv.tests.iter())
                        .flat_map(|tv| tv.evidence.iter())
                        .count(),
                }),
                None => Err(TraceabilityError::NotFound(goal_id)),
            }
        }

        async fn assemble_evidence_pack(
            &self,
            goal_id: Uuid,
        ) -> Result<TraceabilityEvidencePack, TraceabilityError> {
            let chain = self
                .get_full_chain(goal_id)
                .await?
                .ok_or(TraceabilityError::NotFound(goal_id))?;

            let mut criteria_evidence = Vec::new();
            let mut total_tests = 0;
            let mut passed_tests = 0;
            let mut failed_tests = 0;
            let mut code_locations = Vec::new();
            let mut artifacts = Vec::new();

            for criteria_in_chain in &chain.criteria {
                let mut test_evidence_list = Vec::new();

                for test_in_chain in &criteria_in_chain.tests {
                    total_tests += 1;
                    let passed = test_in_chain
                        .results
                        .first()
                        .map(|r| r.passed)
                        .unwrap_or(false);
                    if passed {
                        passed_tests += 1;
                    } else if !test_in_chain.results.is_empty() {
                        failed_tests += 1;
                    }

                    // Collect code location from test case
                    let code_location =
                        test_in_chain
                            .test_case
                            .file_path
                            .as_ref()
                            .map(|path| CodeLocation {
                                file_path: path.clone(),
                                line_start: 0,
                                line_end: 0,
                                description: Some(test_in_chain.test_case.name.clone()),
                            });

                    // Collect artifacts from evidence
                    for ev in &test_in_chain.evidence {
                        artifacts.push(EvidenceArtifact {
                            id: ev.id,
                            evidence_type: ev.evidence_type,
                            artifact_path: ev.artifact_path.clone(),
                            description: ev.description.clone(),
                            created_at: ev.created_at,
                        });
                    }

                    // Collect code locations
                    if let Some(loc) = &code_location {
                        code_locations.push(loc.clone());
                    }

                    test_evidence_list.push(TestCaseEvidence {
                        test_case: test_in_chain.test_case.clone(),
                        results: test_in_chain.results.clone(),
                        evidence: test_in_chain.evidence.clone(),
                        code_location,
                    });
                }

                criteria_evidence.push(CriteriaEvidence {
                    criteria: criteria_in_chain.criteria.clone(),
                    tests: test_evidence_list,
                });
            }

            Ok(TraceabilityEvidencePack {
                id: Uuid::new_v4(),
                requirement_id: goal_id,
                assembled_at: Utc::now(),
                criteria: criteria_evidence,
                total_tests,
                passed_tests,
                failed_tests,
                code_locations,
                artifacts,
            })
        }

        async fn detect_missing_links(
            &self,
            goal_id: Uuid,
        ) -> Result<Vec<MissingLink>, TraceabilityError> {
            let mut missing = Vec::new();

            // Check if goal exists and has criteria
            let chain = self.get_full_chain(goal_id).await?;
            let chain = match chain {
                Some(c) => c,
                None => return Err(TraceabilityError::NotFound(goal_id)),
            };

            // Check goal has criteria
            if chain.criteria.is_empty() {
                missing.push(MissingLink {
                    link_type: MissingLinkType::GoalHasNoCriteria,
                    parent_id: goal_id,
                    description: "Goal has no acceptance criteria linked".to_string(),
                });
            }

            // Check each criteria for missing tests
            for criteria_in_chain in &chain.criteria {
                if criteria_in_chain.tests.is_empty() {
                    missing.push(MissingLink {
                        link_type: MissingLinkType::CriteriaHasNoTests,
                        parent_id: criteria_in_chain.criteria.id,
                        description: format!(
                            "Criteria '{}' has no tests linked",
                            criteria_in_chain.criteria.text
                        ),
                    });
                }

                // Check each test for missing results and code location
                for test_in_chain in &criteria_in_chain.tests {
                    if test_in_chain.results.is_empty() {
                        missing.push(MissingLink {
                            link_type: MissingLinkType::TestHasNoResults,
                            parent_id: test_in_chain.test_case.id,
                            description: format!(
                                "Test '{}' has no results linked",
                                test_in_chain.test_case.name
                            ),
                        });
                    }

                    if test_in_chain.test_case.file_path.is_none() {
                        missing.push(MissingLink {
                            link_type: MissingLinkType::TestHasNoCodeLocation,
                            parent_id: test_in_chain.test_case.id,
                            description: format!(
                                "Test '{}' has no code location (file_path)",
                                test_in_chain.test_case.name
                            ),
                        });
                    }

                    // Check each result for missing evidence
                    for result in &test_in_chain.results {
                        let result_evidence = self.get_evidence_for_result(result.id).await?;
                        if result_evidence.is_empty() {
                            missing.push(MissingLink {
                                link_type: MissingLinkType::ResultHasNoEvidence,
                                parent_id: result.id,
                                description: format!(
                                    "Result for test '{}' has no evidence linked",
                                    test_in_chain.test_case.name
                                ),
                            });
                        }
                    }
                }
            }

            Ok(missing)
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod traceability_tests {
    use super::*;

    fn create_test_goal(task_id: TaskId) -> Goal {
        Goal::new("Test goal description", task_id)
    }

    fn create_test_criteria(goal_id: Uuid) -> AcceptanceCriteria {
        AcceptanceCriteria::new("Test criteria text", goal_id)
            .with_category("functional")
            .with_criticality(CriteriaCriticality::MustHave)
    }

    fn create_test_case(criteria_id: Uuid) -> TestCase {
        TestCase::new("test_example", criteria_id)
            .with_file_path("tests/example.rs")
            .with_test_type(TestCaseType::Unit)
    }

    fn create_test_result(test_id: Uuid, passed: bool) -> TestResult {
        let result = TestResult::new(test_id, passed).with_duration(100);
        if !passed {
            result.with_failure_message("Test failed")
        } else {
            result
        }
    }

    fn create_test_evidence(result_id: Uuid) -> Evidence {
        Evidence::new(result_id, EvidenceType::TestOutput, "/tmp/test_output.log")
            .with_description("Test execution output")
    }

    #[tokio::test]
    async fn test_create_and_get_goal() {
        let store = InMemoryTraceabilityStore::new();
        let task_id = TaskId::new();

        let goal = create_test_goal(task_id);
        let id = store.create_goal(goal.clone()).await.unwrap();

        let retrieved = store.get_goal(id).await.unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().description, "Test goal description");
    }

    #[tokio::test]
    async fn test_goal_criteria_linking() {
        let store = InMemoryTraceabilityStore::new();
        let task_id = TaskId::new();

        // Create goal and criteria
        let goal = create_test_goal(task_id);
        let goal_id = store.create_goal(goal).await.unwrap();

        let criteria = create_test_criteria(goal_id);
        let criteria_id = store.add_criteria(criteria).await.unwrap();

        // Get criteria for goal (forward link)
        let goal_criteria = store.get_criteria_for_goal(goal_id).await.unwrap();
        assert_eq!(goal_criteria.len(), 1);
        assert_eq!(goal_criteria[0].id, criteria_id);

        // Get goal for criteria (backward link)
        let criteria_goal = store.get_goal_for_criteria(criteria_id).await.unwrap();
        assert!(criteria_goal.is_some());
        assert_eq!(criteria_goal.unwrap().id, goal_id);
    }

    #[tokio::test]
    async fn test_criteria_test_linking() {
        let store = InMemoryTraceabilityStore::new();
        let task_id = TaskId::new();

        // Create chain: goal → criteria → test
        let goal = create_test_goal(task_id);
        let goal_id = store.create_goal(goal).await.unwrap();

        let criteria = create_test_criteria(goal_id);
        let criteria_id = store.add_criteria(criteria).await.unwrap();

        let test = create_test_case(criteria_id);
        let test_id = store.add_test_case(test).await.unwrap();

        // Forward: criteria → tests
        let criteria_tests = store.get_tests_for_criteria(criteria_id).await.unwrap();
        assert_eq!(criteria_tests.len(), 1);
        assert_eq!(criteria_tests[0].id, test_id);

        // Backward: test → criteria
        let test_criteria = store.get_criteria_for_test(test_id).await.unwrap();
        assert!(test_criteria.is_some());
        assert_eq!(test_criteria.unwrap().id, criteria_id);
    }

    #[tokio::test]
    async fn test_test_result_linking() {
        let store = InMemoryTraceabilityStore::new();
        let task_id = TaskId::new();

        // Build chain
        let goal = create_test_goal(task_id);
        let goal_id = store.create_goal(goal).await.unwrap();

        let criteria = create_test_criteria(goal_id);
        let criteria_id = store.add_criteria(criteria).await.unwrap();

        let test = create_test_case(criteria_id);
        let test_id = store.add_test_case(test).await.unwrap();

        let result = create_test_result(test_id, true);
        let result_id = store.add_result(result).await.unwrap();

        // Forward: test → results
        let test_results = store.get_results_for_test(test_id).await.unwrap();
        assert_eq!(test_results.len(), 1);
        assert_eq!(test_results[0].id, result_id);

        // Backward: result → test
        let result_test = store.get_test_for_result(result_id).await.unwrap();
        assert!(result_test.is_some());
        assert_eq!(result_test.unwrap().id, test_id);
    }

    #[tokio::test]
    async fn test_result_evidence_linking() {
        let store = InMemoryTraceabilityStore::new();
        let task_id = TaskId::new();

        // Build full chain
        let goal = create_test_goal(task_id);
        let goal_id = store.create_goal(goal).await.unwrap();

        let criteria = create_test_criteria(goal_id);
        let criteria_id = store.add_criteria(criteria).await.unwrap();

        let test = create_test_case(criteria_id);
        let test_id = store.add_test_case(test).await.unwrap();

        let result = create_test_result(test_id, false);
        let result_id = store.add_result(result).await.unwrap();

        let evidence = create_test_evidence(result_id);
        let evidence_id = store.add_evidence(evidence).await.unwrap();

        // Forward: result → evidence
        let result_evidence = store.get_evidence_for_result(result_id).await.unwrap();
        assert_eq!(result_evidence.len(), 1);
        assert_eq!(result_evidence[0].id, evidence_id);

        // Backward: evidence → result
        let evidence_result = store.get_result_for_evidence(evidence_id).await.unwrap();
        assert!(evidence_result.is_some());
        assert_eq!(evidence_result.unwrap().id, result_id);
    }

    #[tokio::test]
    async fn test_full_traceability_chain() {
        let store = InMemoryTraceabilityStore::new();
        let task_id = TaskId::new();

        // Build full chain
        let goal = create_test_goal(task_id);
        let goal_id = store.create_goal(goal).await.unwrap();

        let criteria = create_test_criteria(goal_id);
        let criteria_id = store.add_criteria(criteria).await.unwrap();

        let test = create_test_case(criteria_id);
        let test_id = store.add_test_case(test).await.unwrap();

        let result = create_test_result(test_id, true);
        let result_id = store.add_result(result).await.unwrap();

        let evidence = create_test_evidence(result_id);
        store.add_evidence(evidence).await.unwrap();

        // Get full chain from goal
        let chain = store.get_full_chain(goal_id).await.unwrap();
        assert!(chain.is_some());

        let chain = chain.unwrap();
        assert_eq!(chain.goal.id, goal_id);
        assert_eq!(chain.criteria.len(), 1);
        assert_eq!(chain.criteria[0].criteria.id, criteria_id);
        assert_eq!(chain.criteria[0].tests.len(), 1);
        assert_eq!(chain.criteria[0].tests[0].test_case.id, test_id);
        assert_eq!(chain.criteria[0].tests[0].results.len(), 1);
        assert_eq!(chain.criteria[0].tests[0].evidence.len(), 1);
    }

    #[tokio::test]
    async fn test_reverse_chain_from_evidence() {
        let store = InMemoryTraceabilityStore::new();
        let task_id = TaskId::new();

        // Build full chain
        let goal = create_test_goal(task_id);
        let goal_id = store.create_goal(goal).await.unwrap();

        let criteria = create_test_criteria(goal_id);
        let criteria_id = store.add_criteria(criteria).await.unwrap();

        let test = create_test_case(criteria_id);
        let test_id = store.add_test_case(test).await.unwrap();

        let result = create_test_result(test_id, true);
        let result_id = store.add_result(result).await.unwrap();

        let evidence = create_test_evidence(result_id);
        let evidence_id = store.add_evidence(evidence).await.unwrap();

        // Get reverse chain from evidence
        let chain = store.get_reverse_chain(evidence_id).await.unwrap();
        assert!(chain.is_some());

        let chain = chain.unwrap();
        assert_eq!(chain.goal.id, goal_id);
    }

    #[tokio::test]
    async fn test_chain_counts() {
        let store = InMemoryTraceabilityStore::new();
        let task_id = TaskId::new();

        // Build chain
        let goal = create_test_goal(task_id);
        let goal_id = store.create_goal(goal).await.unwrap();

        let criteria = create_test_criteria(goal_id);
        let criteria_id = store.add_criteria(criteria).await.unwrap();

        let test = create_test_case(criteria_id);
        let test_id = store.add_test_case(test).await.unwrap();

        let result = create_test_result(test_id, true);
        let result_id = store.add_result(result).await.unwrap();

        let evidence = create_test_evidence(result_id);
        store.add_evidence(evidence).await.unwrap();

        let counts = store.count_chain_items(goal_id).await.unwrap();

        assert_eq!(counts.goals, 1);
        assert_eq!(counts.criteria, 1);
        assert_eq!(counts.tests, 1);
        assert_eq!(counts.results, 1);
        assert_eq!(counts.evidence, 1);
    }

    #[tokio::test]
    async fn test_multiple_items_in_chain() {
        let store = InMemoryTraceabilityStore::new();
        let task_id = TaskId::new();

        // Create goal with multiple criteria, tests, and results
        let goal = create_test_goal(task_id);
        let goal_id = store.create_goal(goal).await.unwrap();

        // Two criteria
        let criteria1 = create_test_criteria(goal_id);
        let criteria_id1 = store.add_criteria(criteria1).await.unwrap();

        let criteria2 = AcceptanceCriteria::new("Second criteria", goal_id);
        let criteria_id2 = store.add_criteria(criteria2).await.unwrap();

        // Two tests per criteria
        let test1 = create_test_case(criteria_id1);
        store.add_test_case(test1).await.unwrap();
        let test2 = create_test_case(criteria_id1);
        store.add_test_case(test2).await.unwrap();

        let test3 = create_test_case(criteria_id2);
        store.add_test_case(test3).await.unwrap();

        let counts = store.count_chain_items(goal_id).await.unwrap();

        assert_eq!(counts.goals, 1);
        assert_eq!(counts.criteria, 2);
        assert_eq!(counts.tests, 3);
    }

    #[tokio::test]
    async fn test_get_goals_for_task() {
        let store = InMemoryTraceabilityStore::new();
        let task_id = TaskId::new();

        // Create multiple goals for same task
        let goal1 = create_test_goal(task_id);
        store.create_goal(goal1).await.unwrap();

        let goal2 = create_test_goal(task_id);
        store.create_goal(goal2).await.unwrap();

        // Create goal for different task
        let other_task_id = TaskId::new();
        let goal3 = create_test_goal(other_task_id);
        store.create_goal(goal3).await.unwrap();

        let goals = store.get_goals_for_task(task_id).await.unwrap();
        assert_eq!(goals.len(), 2);

        let other_goals = store.get_goals_for_task(other_task_id).await.unwrap();
        assert_eq!(other_goals.len(), 1);
    }

    #[tokio::test]
    async fn test_not_found_error() {
        let store = InMemoryTraceabilityStore::new();
        let random_id = Uuid::new_v4();

        let result = store.get_goal(random_id).await;
        assert!(result.unwrap().is_none());

        let result = store.get_criteria(random_id).await;
        // get_criteria returns Ok(None) for not found, not Err
        assert!(result.unwrap().is_none());
    }
}

#[cfg(test)]
mod sqlite_traceability_tests {
    use super::sqlite_store::SqliteTraceabilityStore;
    use super::*;

    fn create_test_goal(task_id: TaskId) -> Goal {
        Goal::new("SQLite test goal", task_id)
    }

    fn create_test_criteria(goal_id: Uuid) -> AcceptanceCriteria {
        AcceptanceCriteria::new("SQLite test criteria", goal_id)
    }

    fn create_test_case(criteria_id: Uuid) -> TestCase {
        TestCase::new("test_sqlite_case", criteria_id)
    }

    fn create_test_result(test_id: Uuid) -> TestResult {
        TestResult::new(test_id, true)
    }

    #[tokio::test]
    async fn test_sqlite_goal_operations() {
        let store = SqliteTraceabilityStore::from_connection_string("sqlite::memory:")
            .await
            .unwrap();

        let task_id = TaskId::new();
        let goal = create_test_goal(task_id);
        let goal_id = store.create_goal(goal).await.unwrap();

        let retrieved = store.get_goal(goal_id).await.unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().description, "SQLite test goal");

        let goals = store.get_goals_for_task(task_id).await.unwrap();
        assert_eq!(goals.len(), 1);
    }

    #[tokio::test]
    async fn test_sqlite_full_chain() {
        let store = SqliteTraceabilityStore::from_connection_string("sqlite::memory:")
            .await
            .unwrap();

        let task_id = TaskId::new();
        let goal = create_test_goal(task_id);
        let goal_id = store.create_goal(goal).await.unwrap();

        let criteria = create_test_criteria(goal_id);
        let criteria_id = store.add_criteria(criteria).await.unwrap();

        let test = create_test_case(criteria_id);
        let test_id = store.add_test_case(test).await.unwrap();

        let result = create_test_result(test_id);
        store.add_result(result).await.unwrap();

        let chain = store.get_full_chain(goal_id).await.unwrap();
        assert!(chain.is_some());

        let chain = chain.unwrap();
        assert_eq!(chain.criteria.len(), 1);
        assert_eq!(chain.criteria[0].tests.len(), 1);
    }

    #[tokio::test]
    async fn test_sqlite_reverse_chain() {
        let store = SqliteTraceabilityStore::from_connection_string("sqlite::memory:")
            .await
            .unwrap();

        let task_id = TaskId::new();
        let goal = create_test_goal(task_id);
        let goal_id = store.create_goal(goal).await.unwrap();

        let criteria = create_test_criteria(goal_id);
        let criteria_id = store.add_criteria(criteria).await.unwrap();

        let test = create_test_case(criteria_id);
        let test_id = store.add_test_case(test).await.unwrap();

        let result = create_test_result(test_id);
        let result_id = store.add_result(result).await.unwrap();

        let evidence = Evidence::new(result_id, EvidenceType::TestOutput, "/tmp/output.log");
        let evidence_id = store.add_evidence(evidence).await.unwrap();

        let chain = store.get_reverse_chain(evidence_id).await.unwrap();
        assert!(chain.is_some());
        assert_eq!(chain.unwrap().goal.id, goal_id);
    }

    #[tokio::test]
    async fn test_sqlite_count_items() {
        let store = SqliteTraceabilityStore::from_connection_string("sqlite::memory:")
            .await
            .unwrap();

        let task_id = TaskId::new();
        let goal = create_test_goal(task_id);
        let goal_id = store.create_goal(goal).await.unwrap();

        let criteria = create_test_criteria(goal_id);
        store.add_criteria(criteria).await.unwrap();

        let counts = store.count_chain_items(goal_id).await.unwrap();

        assert_eq!(counts.goals, 1);
        assert_eq!(counts.criteria, 1);
        assert_eq!(counts.tests, 0);
        assert_eq!(counts.results, 0);
        assert_eq!(counts.evidence, 0);
    }

    #[tokio::test]
    async fn test_sqlite_detect_missing_links_per_result_evidence() {
        // Test that ResultHasNoEvidence is detected per-result in SQLite store.
        // When a test has multiple results and only the OLDER result lacks evidence
        // (while the NEWER result HAS evidence), ONLY the older result should be flagged.
        let store = SqliteTraceabilityStore::from_connection_string("sqlite::memory:")
            .await
            .unwrap();

        let task_id = TaskId::new();
        let goal = create_test_goal(task_id);
        let goal_id = store.create_goal(goal).await.unwrap();

        let criteria = create_test_criteria(goal_id);
        let criteria_id = store.add_criteria(criteria).await.unwrap();

        let test = create_test_case(criteria_id)
            .with_file_path("src/lib.rs")
            .with_test_type(TestCaseType::Unit);
        let test_id = store.add_test_case(test).await.unwrap();

        // Create OLDER result (no evidence)
        let old_result = create_test_result(test_id).with_duration(100);
        let old_result_id = store.add_result(old_result).await.unwrap();
        // OLD result has NO evidence linked

        // Create NEWER result (with evidence)
        let new_result = TestResult::new(test_id, false)
            .with_duration(50)
            .with_failure_message("failed");
        let new_result_id = store.add_result(new_result).await.unwrap();
        // NEW result HAS evidence
        let evidence = Evidence::new(
            new_result_id,
            EvidenceType::TestOutput,
            "/tmp/sqlite_output.log",
        );
        store.add_evidence(evidence).await.unwrap();

        // Detect missing links
        let missing = store.detect_missing_links(goal_id).await.unwrap();

        // Should find exactly ONE ResultHasNoEvidence - for the OLD result only
        let result_missing: Vec<_> = missing
            .iter()
            .filter(|m| matches!(m.link_type, MissingLinkType::ResultHasNoEvidence))
            .collect();

        assert_eq!(
            result_missing.len(),
            1,
            "Expected exactly 1 result missing evidence (old one), got {}: {:?}",
            result_missing.len(),
            result_missing
        );

        // The missing link should be for the OLD result (which has no evidence)
        assert_eq!(
            result_missing[0].parent_id, old_result_id,
            "Missing link should be for old result (no evidence), got different result"
        );

        // Should NOT flag the new result as missing evidence (it has evidence!)
        let new_result_missing = missing.iter().any(|m| {
            matches!(m.link_type, MissingLinkType::ResultHasNoEvidence)
                && m.parent_id == new_result_id
        });
        assert!(
            !new_result_missing,
            "New result should NOT be flagged as missing evidence (it has evidence)"
        );
    }

    #[tokio::test]
    async fn test_evidence_pack_assembly_full_chain() {
        // Test VAL-VPIPE-005: evidence pack contains all links and bidirectional traversal works
        // Note: This test uses InMemoryTraceabilityStore from traceability_tests scope,
        // where create_test_result has 2 args (passed: bool)
        let store = InMemoryTraceabilityStore::new();
        let task_id = TaskId::new();

        // Create goal (requirement)
        let goal = Goal::new("Test goal", task_id);
        let goal_id = store.create_goal(goal).await.unwrap();

        // Link 2 tests to criteria
        let criteria = AcceptanceCriteria::new("Test criteria", goal_id)
            .with_category("functional")
            .with_criticality(CriteriaCriticality::MustHave);
        let criteria_id = store.add_criteria(criteria).await.unwrap();

        let test1 = TestCase::new("test_1", criteria_id).with_file_path("tests/1.rs");
        let test_id1 = store.add_test_case(test1).await.unwrap();

        let test2 = TestCase::new("test_2", criteria_id).with_file_path("tests/2.rs");
        let test_id2 = store.add_test_case(test2).await.unwrap();

        // Record results for both tests using 2-arg version
        let result1 = TestResult::new(test_id1, true).with_duration(100);
        store.add_result(result1).await.unwrap();

        let result2 = TestResult::new(test_id2, false)
            .with_duration(50)
            .with_failure_message("failed");
        let result2_id = store.add_result(result2).await.unwrap();

        // Link evidence to results
        let evidence1 = Evidence::new(result2_id, EvidenceType::TestOutput, "/tmp/test_output.log");
        store.add_evidence(evidence1).await.unwrap();

        // Assemble evidence pack
        let pack = store.assemble_evidence_pack(goal_id).await.unwrap();

        // Verify pack contains all links
        assert_eq!(pack.requirement_id, goal_id);
        assert_eq!(pack.total_tests, 2);
        assert_eq!(pack.passed_tests, 1);
        assert_eq!(pack.failed_tests, 1);
        assert_eq!(pack.criteria.len(), 1);
        assert_eq!(pack.criteria[0].tests.len(), 2);

        // Verify bidirectional traversal: requirement→tests
        let criteria_tests = pack.criteria[0].tests.len();
        assert_eq!(criteria_tests, 2);

        // Verify tests have code locations (from create_test_case which has file_path)
        let code_locs = pack.code_locations.len();
        assert!(
            code_locs >= 2,
            "Expected at least 2 code locations, got {}",
            code_locs
        );

        // Verify artifacts are present
        assert_eq!(pack.artifacts.len(), 1, "Expected 1 artifact from evidence");
    }

    #[tokio::test]
    async fn test_detect_missing_links() {
        let store = InMemoryTraceabilityStore::new();
        let task_id = TaskId::new();

        // Create goal with criteria but NO tests (missing link!)
        let goal = Goal::new("Test goal", task_id);
        let goal_id = store.create_goal(goal).await.unwrap();

        let criteria = AcceptanceCriteria::new("Test criteria", goal_id);
        store.add_criteria(criteria).await.unwrap();

        // Detect missing links - should find criteria has no tests
        let missing = store.detect_missing_links(goal_id).await.unwrap();

        assert!(!missing.is_empty(), "Expected missing links to be detected");
        let has_criteria_no_tests = missing
            .iter()
            .any(|m| matches!(m.link_type, MissingLinkType::CriteriaHasNoTests));
        assert!(has_criteria_no_tests, "Should detect criteria has no tests");
    }

    #[tokio::test]
    async fn test_detect_no_missing_links_full_chain() {
        let store = InMemoryTraceabilityStore::new();
        let task_id = TaskId::new();

        // Build complete chain
        let goal = Goal::new("Complete chain goal", task_id);
        let goal_id = store.create_goal(goal).await.unwrap();

        let criteria = AcceptanceCriteria::new("Complete criteria", goal_id);
        let criteria_id = store.add_criteria(criteria).await.unwrap();

        let test = TestCase::new("test_complete", criteria_id)
            .with_file_path("src/lib.rs")
            .with_test_type(TestCaseType::Unit);
        let test_id = store.add_test_case(test).await.unwrap();

        let result = TestResult::new(test_id, true).with_duration(100);
        let result_id = store.add_result(result).await.unwrap();

        let evidence = Evidence::new(result_id, EvidenceType::TestOutput, "/tmp/output.log");
        store.add_evidence(evidence).await.unwrap();

        // Detect missing links - should find none
        let missing = store.detect_missing_links(goal_id).await.unwrap();

        // Note: test has file_path, so no TestHasNoCodeLocation
        // Result has evidence, so no ResultHasNoEvidence
        // Criteria has tests, so no CriteriaHasNoTests
        // Goal has criteria, so no GoalHasNoCriteria
        // The only potential missing: result has evidence (1 found), tests have results (1 found)
        // So missing should be empty
        assert!(
            missing.is_empty(),
            "Expected no missing links for complete chain, got {:?}",
            missing
        );
    }

    #[tokio::test]
    async fn test_evidence_pack_code_locations() {
        let store = InMemoryTraceabilityStore::new();
        let task_id = TaskId::new();

        let goal = Goal::new("Code locations test", task_id);
        let goal_id = store.create_goal(goal).await.unwrap();

        let criteria = AcceptanceCriteria::new("Code location criteria", goal_id);
        let criteria_id = store.add_criteria(criteria).await.unwrap();

        // Create test with code location
        let test = TestCase::new("test_with_location", criteria_id)
            .with_file_path("src/lib.rs")
            .with_test_type(TestCaseType::Unit);
        let test_id = store.add_test_case(test).await.unwrap();

        let result = TestResult::new(test_id, true);
        let result_id = store.add_result(result).await.unwrap();

        let evidence = Evidence::new(
            result_id,
            EvidenceType::CoverageReport,
            "/tmp/coverage.json",
        );
        store.add_evidence(evidence).await.unwrap();

        let pack = store.assemble_evidence_pack(goal_id).await.unwrap();

        // Verify code locations include file path
        assert!(!pack.code_locations.is_empty());
        assert_eq!(pack.code_locations[0].file_path, "src/lib.rs");

        // Verify artifacts include coverage report
        let has_coverage = pack
            .artifacts
            .iter()
            .any(|a| matches!(a.evidence_type, EvidenceType::CoverageReport));
        assert!(has_coverage);
    }

    #[tokio::test]
    async fn test_evidence_pack_bidirectional_traversal() {
        // Test that we can traverse both ways: requirement→tests and test→requirement
        let store = InMemoryTraceabilityStore::new();
        let task_id = TaskId::new();

        let goal = Goal::new("Bidirectional test", task_id);
        let goal_id = store.create_goal(goal).await.unwrap();

        let criteria = AcceptanceCriteria::new("Bidirectional criteria", goal_id);
        let criteria_id = store.add_criteria(criteria).await.unwrap();

        let test = TestCase::new("test_bidir", criteria_id).with_file_path("src/lib.rs");
        let test_id = store.add_test_case(test).await.unwrap();

        let result = TestResult::new(test_id, true);
        store.add_result(result).await.unwrap();

        // Forward: get full chain from goal (requirement→tests)
        let chain = store.get_full_chain(goal_id).await.unwrap().unwrap();
        assert_eq!(chain.criteria[0].tests[0].test_case.id, test_id);

        // Backward: get criteria for test (test→criteria) and then goal for criteria
        let test_criteria = store.get_criteria_for_test(test_id).await.unwrap();
        assert!(test_criteria.is_some());
        assert_eq!(test_criteria.unwrap().id, criteria_id);

        let criteria_goal = store.get_goal_for_criteria(criteria_id).await.unwrap();
        assert!(criteria_goal.is_some());
        assert_eq!(criteria_goal.unwrap().id, goal_id);

        // Both directions should lead to same goal
        let pack = store.assemble_evidence_pack(goal_id).await.unwrap();
        assert_eq!(pack.requirement_id, goal_id);
    }

    #[tokio::test]
    async fn test_detect_missing_links_per_result_evidence() {
        // Test that ResultHasNoEvidence is detected per-result, not from parent test-level evidence.
        // When a test has multiple results and only the OLDER result lacks evidence
        // (while the NEWER result HAS evidence), ONLY the older result should be flagged.
        // This verifies the fix: we check each result's evidence individually, not test-level.
        let store = InMemoryTraceabilityStore::new();
        let task_id = TaskId::new();

        let goal = Goal::new("Per-result evidence test", task_id);
        let goal_id = store.create_goal(goal).await.unwrap();

        let criteria = AcceptanceCriteria::new("Per-result criteria", goal_id);
        let criteria_id = store.add_criteria(criteria).await.unwrap();

        let test = TestCase::new("test_per_result", criteria_id)
            .with_file_path("src/lib.rs")
            .with_test_type(TestCaseType::Unit);
        let test_id = store.add_test_case(test).await.unwrap();

        // Create OLDER result (simulated by inserting it first, then newer)
        let old_result = TestResult::new(test_id, true).with_duration(100);
        let old_result_id = store.add_result(old_result).await.unwrap();
        // OLD result has NO evidence linked

        // Create NEWER result
        let new_result = TestResult::new(test_id, false)
            .with_duration(50)
            .with_failure_message("new failure");
        let new_result_id = store.add_result(new_result).await.unwrap();
        // NEW result HAS evidence
        let evidence = Evidence::new(
            new_result_id,
            EvidenceType::TestOutput,
            "/tmp/new_output.log",
        );
        store.add_evidence(evidence).await.unwrap();

        // Detect missing links
        let missing = store.detect_missing_links(goal_id).await.unwrap();

        // Should find exactly ONE ResultHasNoEvidence - for the OLD result only
        let result_missing: Vec<_> = missing
            .iter()
            .filter(|m| matches!(m.link_type, MissingLinkType::ResultHasNoEvidence))
            .collect();

        assert_eq!(
            result_missing.len(),
            1,
            "Expected exactly 1 result missing evidence (old one), got {}: {:?}",
            result_missing.len(),
            result_missing
        );

        // The missing link should be for the OLD result (which has no evidence)
        assert_eq!(
            result_missing[0].parent_id, old_result_id,
            "Missing link should be for old result (no evidence), got different result"
        );

        // Should NOT flag the new result as missing evidence (it has evidence!)
        let new_result_missing = missing.iter().any(|m| {
            matches!(m.link_type, MissingLinkType::ResultHasNoEvidence)
                && m.parent_id == new_result_id
        });
        assert!(
            !new_result_missing,
            "New result should NOT be flagged as missing evidence (it has evidence)"
        );
    }
}
