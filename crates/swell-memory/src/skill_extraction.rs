// skill_extraction.rs - Skill extraction from successful task trajectories
//
// This module provides functionality to analyze successful task executions
// and extract reusable procedures as versioned skills stored in .skills directory.

use crate::{golden_sample_testing::GoldenSampleService, SqliteMemoryStore, SwellError};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;

/// Represents a skill extracted from a successful task trajectory
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub version: String,
    pub task_pattern: String,
    pub steps: Vec<SkillStep>,
    pub tools_used: Vec<String>,
    pub conventions: Vec<String>,
    pub confidence: f64,
    pub source_task_id: Uuid,
    pub created_at: DateTime<Utc>,
    pub metadata: serde_json::Value,
}

impl Skill {
    pub fn new(name: String, task_pattern: String) -> Self {
        Self {
            id: Uuid::new_v4(),
            name,
            description: String::new(),
            version: "1.0.0".to_string(),
            task_pattern,
            steps: Vec::new(),
            tools_used: Vec::new(),
            conventions: Vec::new(),
            confidence: 0.0,
            source_task_id: Uuid::nil(),
            created_at: Utc::now(),
            metadata: serde_json::json!({}),
        }
    }
}

/// A single step within a skill procedure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillStep {
    pub order: usize,
    pub description: String,
    pub affected_file_patterns: Vec<String>,
    pub tool_sequence: Vec<String>,
    pub validation_check: Option<String>,
}

/// Metadata about the task trajectory used for skill extraction
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrajectoryData {
    pub task_id: Uuid,
    pub task_description: String,
    pub plan_steps: Vec<TrajectoryStep>,
    pub tool_calls: Vec<ToolCallData>,
    pub files_modified: Vec<String>,
    pub tests_run: Vec<String>,
    pub validation_passed: bool,
    pub iteration_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrajectoryStep {
    pub step_id: Uuid,
    pub description: String,
    pub affected_files: Vec<String>,
    pub risk_level: String,
    pub status: String,
}

/// Tool call data captured during task execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallData {
    pub tool_name: String,
    pub arguments: serde_json::Value,
    pub success: bool,
    pub timestamp: DateTime<Utc>,
}

/// Extracted pattern with frequency and confidence
#[derive(Debug, Clone)]
pub struct ExtractedPattern {
    pub pattern_type: PatternType,
    pub name: String,
    pub frequency: usize,
    pub examples: Vec<String>,
    pub confidence: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PatternType {
    FileOperation,
    GitOperation,
    ShellCommand,
    TestPattern,
    ValidationPattern,
    CodeTransform,
}

/// Configuration for skill extraction
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractionConfig {
    pub min_confidence: f64,
    pub max_skills_per_trajectory: usize,
    pub store_path: String,
    pub version_format: String,
}

impl Default for ExtractionConfig {
    fn default() -> Self {
        Self {
            min_confidence: 0.5,
            max_skills_per_trajectory: 5,
            store_path: ".skills".to_string(),
            version_format: "1.0.0".to_string(),
        }
    }
}

/// Result of skill extraction operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractionResult {
    pub skills_extracted: usize,
    pub patterns_found: usize,
    pub skills: Vec<Skill>,
    pub errors: Vec<String>,
}

/// Trajectory analyzer for processing successful task executions
#[allow(dead_code)]
pub struct TrajectoryAnalyzer {
    store: SqliteMemoryStore,
    config: ExtractionConfig,
}

impl TrajectoryAnalyzer {
    pub fn new(store: SqliteMemoryStore, config: ExtractionConfig) -> Self {
        Self { store, config }
    }

    pub fn with_default_config(store: SqliteMemoryStore) -> Self {
        Self {
            store,
            config: ExtractionConfig::default(),
        }
    }

    /// Analyze a successful task trajectory and extract patterns
    pub async fn analyze(
        &self,
        trajectory: TrajectoryData,
    ) -> Result<Vec<ExtractedPattern>, SwellError> {
        let mut patterns = Vec::new();

        // Extract file operation patterns
        let file_patterns = self.extract_file_patterns(&trajectory);
        patterns.extend(file_patterns);

        // Extract tool sequence patterns
        let tool_patterns = self.extract_tool_patterns(&trajectory);
        patterns.extend(tool_patterns);

        // Extract test patterns
        let test_patterns = self.extract_test_patterns(&trajectory);
        patterns.extend(test_patterns);

        // Extract validation patterns
        let validation_patterns = self.extract_validation_patterns(&trajectory);
        patterns.extend(validation_patterns);

        // Deduplicate patterns by name and type
        let unique_patterns = self.deduplicate_patterns(patterns);

        Ok(unique_patterns)
    }

    fn extract_file_patterns(&self, trajectory: &TrajectoryData) -> Vec<ExtractedPattern> {
        let mut patterns = Vec::new();
        let mut file_ops: HashMap<String, Vec<String>> = HashMap::new();

        for tool_call in &trajectory.tool_calls {
            match tool_call.tool_name.as_str() {
                "read_file" | "write_file" | "edit_file" => {
                    if let Some(path) = tool_call.arguments.get("path") {
                        if let Some(path_str) = path.as_str() {
                            let dir = PathBuf::from(path_str)
                                .parent()
                                .map(|p| p.to_string_lossy().to_string())
                                .unwrap_or_default();
                            file_ops
                                .entry(tool_call.tool_name.clone())
                                .or_default()
                                .push(dir);
                        }
                    }
                }
                _ => {}
            }
        }

        for (op, dirs) in file_ops {
            let mut dir_counts: HashMap<String, usize> = HashMap::new();
            for dir in dirs {
                *dir_counts.entry(dir).or_insert(0) += 1;
            }

            for (dir, count) in dir_counts {
                if count >= 2 {
                    patterns.push(ExtractedPattern {
                        pattern_type: PatternType::FileOperation,
                        name: format!("{} in {}", op, dir),
                        frequency: count,
                        examples: vec![dir],
                        confidence: (count as f64 / trajectory.tool_calls.len() as f64).min(1.0),
                    });
                }
            }
        }

        patterns
    }

    fn extract_tool_patterns(&self, trajectory: &TrajectoryData) -> Vec<ExtractedPattern> {
        let mut patterns = Vec::new();
        let mut sequence_counts: HashMap<String, usize> = HashMap::new();

        // Track tool call sequences (pairs)
        for window in trajectory.tool_calls.windows(2) {
            let seq = format!("{} → {}", window[0].tool_name, window[1].tool_name);
            *sequence_counts.entry(seq).or_insert(0) += 1;
        }

        for (seq, count) in sequence_counts {
            if count >= 2 {
                patterns.push(ExtractedPattern {
                    pattern_type: PatternType::CodeTransform,
                    name: seq.clone(),
                    frequency: count,
                    examples: vec![seq],
                    confidence: (count as f64 / (trajectory.tool_calls.len() as f64 - 1.0))
                        .min(1.0),
                });
            }
        }

        patterns
    }

    fn extract_test_patterns(&self, trajectory: &TrajectoryData) -> Vec<ExtractedPattern> {
        let mut patterns = Vec::new();

        for test in &trajectory.tests_run {
            if test.contains("test_") || test.contains("_test") {
                let pattern_name = if test.contains("::") {
                    test.split("::").last().unwrap_or(test).to_string()
                } else {
                    test.to_string()
                };

                patterns.push(ExtractedPattern {
                    pattern_type: PatternType::TestPattern,
                    name: pattern_name,
                    frequency: 1,
                    examples: vec![test.clone()],
                    confidence: 0.7,
                });
            }
        }

        patterns
    }

    fn extract_validation_patterns(&self, trajectory: &TrajectoryData) -> Vec<ExtractedPattern> {
        let mut patterns = Vec::new();

        // Check what validation was done
        if !trajectory.tests_run.is_empty() {
            patterns.push(ExtractedPattern {
                pattern_type: PatternType::ValidationPattern,
                name: "run_tests".to_string(),
                frequency: 1,
                examples: trajectory.tests_run.clone(),
                confidence: 0.8,
            });
        }

        // Check for lint/format validation patterns
        let has_lint = trajectory
            .tool_calls
            .iter()
            .any(|t| t.tool_name.contains("lint") || t.tool_name.contains("clippy"));
        if has_lint {
            patterns.push(ExtractedPattern {
                pattern_type: PatternType::ValidationPattern,
                name: "run_lint".to_string(),
                frequency: 1,
                examples: vec!["clippy".to_string()],
                confidence: 0.8,
            });
        }

        patterns
    }

    fn deduplicate_patterns(&self, patterns: Vec<ExtractedPattern>) -> Vec<ExtractedPattern> {
        let mut seen: HashSet<(PatternType, String)> = HashSet::new();
        let mut result = Vec::new();

        for pattern in patterns {
            let key = (pattern.pattern_type, pattern.name.clone());
            if seen.insert(key) {
                result.push(pattern);
            }
        }

        result
    }
}

/// Skill extractor that creates versioned skill files
pub struct SkillExtractor {
    store: SqliteMemoryStore,
    config: ExtractionConfig,
    workspace_path: PathBuf,
    golden_sample_service: Option<GoldenSampleService>,
}

impl SkillExtractor {
    pub fn new(
        store: SqliteMemoryStore,
        config: ExtractionConfig,
        workspace_path: PathBuf,
    ) -> Self {
        Self {
            store,
            config,
            workspace_path,
            golden_sample_service: None,
        }
    }

    /// Create with a golden sample service for auto-application validation
    pub fn with_golden_sample_service(
        store: SqliteMemoryStore,
        config: ExtractionConfig,
        workspace_path: PathBuf,
        golden_sample_service: GoldenSampleService,
    ) -> Self {
        Self {
            store,
            config,
            workspace_path,
            golden_sample_service: Some(golden_sample_service),
        }
    }
    pub async fn extract_from_trajectory(
        &self,
        trajectory: TrajectoryData,
    ) -> Result<ExtractionResult, SwellError> {
        let analyzer = TrajectoryAnalyzer::new(self.store.clone(), self.config.clone());
        let patterns = analyzer.analyze(trajectory.clone()).await?;
        let pattern_count = patterns.len();

        let mut skills = Vec::new();
        let mut errors = Vec::new();

        // Convert patterns to skills
        for pattern in patterns
            .into_iter()
            .take(self.config.max_skills_per_trajectory)
        {
            if pattern.confidence < self.config.min_confidence {
                continue;
            }

            match self.pattern_to_skill(&pattern, &trajectory) {
                Ok(skill) => skills.push(skill),
                Err(e) => errors.push(e.to_string()),
            }
        }

        // Store skills in .skills directory
        if let Err(e) = self.store_skills(&skills).await {
            errors.push(format!("Failed to store skills: {}", e));
        }

        Ok(ExtractionResult {
            skills_extracted: skills.len(),
            patterns_found: pattern_count,
            skills,
            errors,
        })
    }

    fn pattern_to_skill(
        &self,
        pattern: &ExtractedPattern,
        trajectory: &TrajectoryData,
    ) -> Result<Skill, SwellError> {
        let mut skill = Skill::new(pattern.name.clone(), trajectory.task_description.clone());

        skill.description = format!(
            "Extracted from {} successful execution(s) of: {}",
            pattern.frequency, trajectory.task_description
        );

        // Build skill steps from tool calls
        skill.steps = self.build_skill_steps(pattern, trajectory);

        // Collect tools used
        skill.tools_used = trajectory
            .tool_calls
            .iter()
            .map(|t| t.tool_name.clone())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();

        // Extract conventions from file patterns
        skill.conventions = self.extract_conventions(trajectory);

        skill.confidence = pattern.confidence;
        skill.source_task_id = trajectory.task_id;

        Ok(skill)
    }

    fn build_skill_steps(
        &self,
        _pattern: &ExtractedPattern,
        trajectory: &TrajectoryData,
    ) -> Vec<SkillStep> {
        let mut steps = Vec::new();

        // Group tool calls by logical operation
        let mut current_step: Option<(usize, String, Vec<String>, Vec<String>)> = None;

        for (idx, tool_call) in trajectory.tool_calls.iter().enumerate() {
            let file = tool_call
                .arguments
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            match current_step {
                Some((order, mut desc, mut files, mut tools)) => {
                    // Check if this continues the same logical step
                    if tools.last().is_some_and(|last| {
                        matches!(
                            (last.as_str(), tool_call.tool_name.as_str()),
                            ("read_file", "edit_file")
                                | ("edit_file", "edit_file")
                                | ("shell", "shell")
                        )
                    }) {
                        if !file.is_empty() && !files.contains(&file) {
                            files.push(file);
                        }
                        tools.push(tool_call.tool_name.clone());

                        // Update description based on tool sequence
                        if tools.len() == 2 {
                            desc = "Read and modify files".to_string();
                        } else if tools.len() == 3 {
                            desc = "Read, modify, and validate".to_string();
                        }

                        current_step = Some((order, desc, files, tools));
                    } else {
                        // Save current step and start new one
                        steps.push(SkillStep {
                            order,
                            description: desc,
                            affected_file_patterns: files,
                            tool_sequence: tools,
                            validation_check: None,
                        });

                        let desc = match tool_call.tool_name.as_str() {
                            "read_file" => "Read files to understand structure",
                            "write_file" => "Create or overwrite files",
                            "edit_file" => "Make targeted modifications",
                            "shell" => "Execute shell commands",
                            _ => "Execute operations",
                        };

                        current_step = Some((
                            idx,
                            desc.to_string(),
                            if file.is_empty() { vec![] } else { vec![file] },
                            vec![tool_call.tool_name.clone()],
                        ));
                    }
                }
                None => {
                    let desc = match tool_call.tool_name.as_str() {
                        "read_file" => "Read files to understand structure",
                        "write_file" => "Create or overwrite files",
                        "edit_file" => "Make targeted modifications",
                        "shell" => "Execute shell commands",
                        _ => "Execute operations",
                    };

                    current_step = Some((
                        idx,
                        desc.to_string(),
                        if file.is_empty() { vec![] } else { vec![file] },
                        vec![tool_call.tool_name.clone()],
                    ));
                }
            }
        }

        // Don't forget the last step
        if let Some((order, desc, files, tools)) = current_step {
            steps.push(SkillStep {
                order,
                description: desc,
                affected_file_patterns: files,
                tool_sequence: tools,
                validation_check: None,
            });
        }

        // Re-number steps starting from 1
        for (i, step) in steps.iter_mut().enumerate() {
            step.order = i + 1;
        }

        steps
    }

    fn extract_conventions(&self, trajectory: &TrajectoryData) -> Vec<String> {
        let mut conventions = Vec::new();

        // Extract file naming conventions
        let file_extensions: HashSet<String> = trajectory
            .files_modified
            .iter()
            .filter_map(|f| {
                PathBuf::from(f)
                    .extension()
                    .map(|e| e.to_string_lossy().to_string())
            })
            .collect();

        if file_extensions.len() == 1 {
            if let Some(ext) = file_extensions.iter().next() {
                conventions.push(format!("File extension: {}", ext));
            }
        }

        // Extract directory conventions
        let mut dir_counts: HashMap<String, usize> = HashMap::new();
        for file in &trajectory.files_modified {
            if let Some(parent) = PathBuf::from(file).parent() {
                let dir = parent.to_string_lossy().to_string();
                *dir_counts.entry(dir).or_insert(0) += 1;
            }
        }

        if let Some((dir, count)) = dir_counts.iter().max_by_key(|(_, c)| *c) {
            if *count > 1 {
                conventions.push(format!("Primary directory: {}", dir));
            }
        }

        // Extract test naming conventions
        for test in &trajectory.tests_run {
            if test.contains("_test") || test.ends_with("Test") {
                if test.contains('_') {
                    conventions.push("Uses snake_case for test names".to_string());
                } else {
                    conventions.push("Uses PascalCase for test names".to_string());
                }
                break;
            }
        }

        conventions
    }

    /// Store skills in .skills directory as versioned files
    async fn store_skills(&self, skills: &[Skill]) -> Result<(), SwellError> {
        let skills_dir = self.workspace_path.join(&self.config.store_path);

        // Create .skills directory if it doesn't exist
        if !skills_dir.exists() {
            fs::create_dir_all(&skills_dir).map_err(|e| {
                SwellError::DatabaseError(format!("Failed to create .skills directory: {}", e))
            })?;
        }

        for skill in skills {
            let filename = format!("{}.v{}.json", skill.name.replace(' ', "_"), skill.version);
            let filepath = skills_dir.join(&filename);

            let json = serde_json::to_string_pretty(skill).map_err(|e| {
                SwellError::DatabaseError(format!("Failed to serialize skill: {}", e))
            })?;

            fs::write(&filepath, json).map_err(|e| {
                SwellError::DatabaseError(format!("Failed to write skill file: {}", e))
            })?;
        }

        Ok(())
    }

    /// Load all skills from the .skills directory
    pub async fn load_skills(&self) -> Result<Vec<Skill>, SwellError> {
        let skills_dir = self.workspace_path.join(&self.config.store_path);

        if !skills_dir.exists() {
            return Ok(Vec::new());
        }

        let mut skills = Vec::new();

        for entry in fs::read_dir(&skills_dir).map_err(|e| {
            SwellError::DatabaseError(format!("Failed to read .skills directory: {}", e))
        })? {
            let entry = entry.map_err(|e| {
                SwellError::DatabaseError(format!("Failed to read directory entry: {}", e))
            })?;

            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "json") {
                let content = fs::read_to_string(&path).map_err(|e| {
                    SwellError::DatabaseError(format!("Failed to read skill file: {}", e))
                })?;

                let skill: Skill = serde_json::from_str(&content).map_err(|e| {
                    SwellError::DatabaseError(format!("Failed to parse skill JSON: {}", e))
                })?;

                skills.push(skill);
            }
        }

        // Sort by name and version
        skills.sort_by(|a, b| a.name.cmp(&b.name).then(a.version.cmp(&b.version)));

        Ok(skills)
    }

    /// Find skills matching a task description
    ///
    /// For high-confidence skills (confidence > 0.9), this method validates them
    /// against golden samples before returning. If golden sample validation fails,
    /// the skill is NOT returned (blocked from auto-application).
    ///
    /// This ensures that only procedures that have been validated against test cases
    /// (golden samples) are auto-applied to future tasks.
    pub async fn find_matching_skills(
        &self,
        task_description: &str,
    ) -> Result<Vec<Skill>, SwellError> {
        let all_skills = self.load_skills().await?;

        let task_lower = task_description.to_lowercase();
        let mut matched_skills: Vec<(Skill, f64)> = Vec::new();

        for skill in all_skills {
            let similarity =
                self.calculate_similarity(&task_lower, &skill.task_pattern.to_lowercase());
            if similarity > 0.3 {
                matched_skills.push((skill, similarity));
            }
        }

        // Sort by similarity and return top matches
        matched_skills.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        // Apply golden sample validation for high-confidence skills
        if let Some(ref golden_service) = self.golden_sample_service {
            let mut validated_skills = Vec::new();
            for (skill, score) in matched_skills {
                // Only validate skills with confidence > 0.9 (HIGH_CONFIDENCE_THRESHOLD)
                // Lower confidence skills are not auto-applied anyway, so skip validation
                if skill.confidence > 0.9 {
                    // Use skill description as the procedure context
                    // Use task pattern as the procedure output to validate
                    let skill_id = skill.id;
                    let skill_name = skill.name.clone();
                    let skill_confidence = skill.confidence;
                    let eligibility = golden_service
                        .validate_procedure_for_auto_application(
                            skill.id,
                            &skill.description,
                            &skill.task_pattern,
                            skill.confidence,
                        )
                        .await?;

                    if eligibility.is_eligible {
                        // Skill passed golden sample validation - include it
                        validated_skills.push((skill, score));
                        tracing::debug!(
                            skill_id = %skill_id,
                            skill_name = %skill_name,
                            confidence = skill_confidence,
                            "Skill passed golden sample validation for auto-application"
                        );
                    } else {
                        // Skill failed golden sample validation - do NOT auto-apply
                        tracing::info!(
                            skill_id = %skill_id,
                            skill_name = %skill_name,
                            confidence = skill_confidence,
                            reason = ?eligibility.reason,
                            "Skill blocked from auto-application due to golden sample validation failure"
                        );
                    }
                } else {
                    // Low-confidence skill (<=0.9) - skip validation and don't auto-apply
                    // These will be filtered out when actually applying the skill
                    tracing::trace!(
                        skill_id = %skill.id,
                        skill_name = %skill.name,
                        confidence = skill.confidence,
                        "Skill confidence below auto-application threshold"
                    );
                }
            }
            Ok(validated_skills.into_iter().map(|(s, _)| s).collect())
        } else {
            // No golden sample service configured - return skills without validation
            // This maintains backward compatibility for tests and simple setups
            Ok(matched_skills.into_iter().map(|(s, _)| s).collect())
        }
    }

    /// Simple similarity calculation using word overlap
    fn calculate_similarity(&self, text1: &str, text2: &str) -> f64 {
        let words1: HashSet<String> = text1.split_whitespace().map(|s| s.to_string()).collect();
        let words2: HashSet<String> = text2.split_whitespace().map(|s| s.to_string()).collect();

        if words1.is_empty() || words2.is_empty() {
            return 0.0;
        }

        let intersection = words1.intersection(&words2).count() as f64;
        let union = words1.union(&words2).count() as f64;

        intersection / union
    }

    /// Update an existing skill with new trajectory data
    pub async fn update_skill(
        &self,
        skill_id: Uuid,
        trajectory: TrajectoryData,
    ) -> Result<Skill, SwellError> {
        let all_skills = self.load_skills().await?;

        let skill = all_skills
            .iter()
            .find(|s| s.id == skill_id)
            .ok_or_else(|| SwellError::DatabaseError(format!("Skill not found: {}", skill_id)))?
            .clone();

        // Update version (increment patch)
        let version_parts: Vec<&str> = skill.version.split('.').collect();
        let major = version_parts
            .first()
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(1);
        let minor = version_parts
            .get(1)
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(0);
        let patch = version_parts
            .get(2)
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(0)
            + 1;
        let new_version = format!("{}.{}.{}", major, minor, patch);

        // Create updated skill
        let mut updated_skill = skill.clone();
        updated_skill.version = new_version;
        updated_skill.confidence = (skill.confidence + 0.1).min(1.0);
        updated_skill.metadata = serde_json::json!({
            "updated_from_task": trajectory.task_id.to_string(),
            "update_count": skill.metadata.get("update_count").and_then(|v| v.as_u64()).unwrap_or(0) + 1
        });

        // Store updated skill
        let skills_dir = self.workspace_path.join(&self.config.store_path);
        let filename = format!(
            "{}.v{}.json",
            updated_skill.name.replace(' ', "_"),
            updated_skill.version
        );
        let filepath = skills_dir.join(&filename);

        let json = serde_json::to_string_pretty(&updated_skill)
            .map_err(|e| SwellError::DatabaseError(format!("Failed to serialize skill: {}", e)))?;

        fs::write(&filepath, json)
            .map_err(|e| SwellError::DatabaseError(format!("Failed to write skill file: {}", e)))?;

        Ok(updated_skill)
    }
}

/// High-level service for skill extraction operations
pub struct SkillExtractionService {
    store: SqliteMemoryStore,
    config: ExtractionConfig,
    workspace_path: PathBuf,
}

impl SkillExtractionService {
    pub fn new(store: SqliteMemoryStore, workspace_path: PathBuf) -> Self {
        Self {
            store,
            config: ExtractionConfig::default(),
            workspace_path,
        }
    }

    pub fn with_config(
        store: SqliteMemoryStore,
        config: ExtractionConfig,
        workspace_path: PathBuf,
    ) -> Self {
        Self {
            store,
            config,
            workspace_path,
        }
    }

    /// Extract skills from a successful task trajectory
    pub async fn extract_skills(
        &self,
        trajectory: TrajectoryData,
    ) -> Result<ExtractionResult, SwellError> {
        if !trajectory.validation_passed {
            return Err(SwellError::DatabaseError(
                "Cannot extract skills from failed task".to_string(),
            ));
        }

        let extractor = SkillExtractor::new(
            self.store.clone(),
            self.config.clone(),
            self.workspace_path.clone(),
        );

        extractor.extract_from_trajectory(trajectory).await
    }

    /// Get all stored skills
    pub async fn get_all_skills(&self) -> Result<Vec<Skill>, SwellError> {
        let extractor = SkillExtractor::new(
            self.store.clone(),
            self.config.clone(),
            self.workspace_path.clone(),
        );
        extractor.load_skills().await
    }

    /// Find skills relevant to a task description
    pub async fn find_skills_for_task(
        &self,
        task_description: &str,
    ) -> Result<Vec<Skill>, SwellError> {
        let extractor = SkillExtractor::new(
            self.store.clone(),
            self.config.clone(),
            self.workspace_path.clone(),
        );
        extractor.find_matching_skills(task_description).await
    }

    /// Update an existing skill with new trajectory data
    pub async fn update_skill(
        &self,
        skill_id: Uuid,
        trajectory: TrajectoryData,
    ) -> Result<Skill, SwellError> {
        let extractor = SkillExtractor::new(
            self.store.clone(),
            self.config.clone(),
            self.workspace_path.clone(),
        );
        extractor.update_skill(skill_id, trajectory).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_extraction_config_default() {
        let config = ExtractionConfig::default();
        assert_eq!(config.min_confidence, 0.5);
        assert_eq!(config.max_skills_per_trajectory, 5);
        assert_eq!(config.store_path, ".skills");
    }

    #[tokio::test]
    async fn test_skill_creation() {
        let skill = Skill::new(
            "test_skill".to_string(),
            "Implement a test feature".to_string(),
        );
        assert_eq!(skill.name, "test_skill");
        assert_eq!(skill.task_pattern, "Implement a test feature");
        assert_eq!(skill.version, "1.0.0");
        assert!(skill.steps.is_empty());
    }

    #[tokio::test]
    async fn test_trajectory_data_serialization() {
        let trajectory = TrajectoryData {
            task_id: Uuid::new_v4(),
            task_description: "Add new feature".to_string(),
            plan_steps: vec![],
            tool_calls: vec![ToolCallData {
                tool_name: "read_file".to_string(),
                arguments: serde_json::json!({"path": "src/main.rs"}),
                success: true,
                timestamp: Utc::now(),
            }],
            files_modified: vec!["src/main.rs".to_string()],
            tests_run: vec!["test_main".to_string()],
            validation_passed: true,
            iteration_count: 1,
        };

        let json = serde_json::to_string(&trajectory).unwrap();
        let deserialized: TrajectoryData = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.task_description, trajectory.task_description);
    }

    #[tokio::test]
    async fn test_skill_serialization() {
        let mut skill = Skill::new("rust_test".to_string(), "Write tests".to_string());
        skill.description = "A skill for writing tests".to_string();
        skill.confidence = 0.85;
        skill.steps.push(SkillStep {
            order: 1,
            description: "Read file".to_string(),
            affected_file_patterns: vec!["src/**/*.rs".to_string()],
            tool_sequence: vec!["read_file".to_string()],
            validation_check: Some("cargo test".to_string()),
        });

        let json = serde_json::to_string_pretty(&skill).unwrap();
        let deserialized: Skill = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.name, "rust_test");
        assert_eq!(deserialized.steps.len(), 1);
    }

    #[tokio::test]
    async fn test_extraction_result() {
        let skill = Skill::new("test".to_string(), "pattern".to_string());
        let result = ExtractionResult {
            skills_extracted: 1,
            patterns_found: 2,
            skills: vec![skill],
            errors: vec![],
        };

        assert_eq!(result.skills_extracted, 1);
        assert_eq!(result.patterns_found, 2);
        assert_eq!(result.skills.len(), 1);
    }

    #[tokio::test]
    async fn test_skill_extractor_similarity() {
        use crate::SqliteMemoryStore;

        let store = SqliteMemoryStore::create("sqlite::memory:").await.unwrap();

        let extractor =
            SkillExtractor::new(store, ExtractionConfig::default(), std::env::temp_dir());

        // Test similarity calculation
        let sim1 = extractor.calculate_similarity("add feature", "add new feature");
        assert!(sim1 > 0.5, "Similar phrases should have high similarity");

        let sim2 = extractor.calculate_similarity("add feature", "remove bug");
        assert!(sim2 < 0.5, "Different phrases should have low similarity");

        let sim3 = extractor.calculate_similarity("", "test");
        assert_eq!(sim3, 0.0, "Empty string should return 0 similarity");
    }

    #[tokio::test]
    async fn test_tool_call_data() {
        let tool_call = ToolCallData {
            tool_name: "edit_file".to_string(),
            arguments: serde_json::json!({
                "path": "src/main.rs",
                "old_str": "old",
                "new_str": "new"
            }),
            success: true,
            timestamp: Utc::now(),
        };

        let json = serde_json::to_string(&tool_call).unwrap();
        let deserialized: ToolCallData = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.tool_name, "edit_file");
        assert!(deserialized.success);
    }

    #[tokio::test]
    async fn test_skill_step() {
        let step = SkillStep {
            order: 1,
            description: "Read and modify file".to_string(),
            affected_file_patterns: vec!["src/*.rs".to_string()],
            tool_sequence: vec!["read_file".to_string(), "edit_file".to_string()],
            validation_check: Some("cargo build".to_string()),
        };

        assert_eq!(step.order, 1);
        assert_eq!(step.tool_sequence.len(), 2);
        assert!(step.validation_check.is_some());
    }

    #[tokio::test]
    async fn test_find_matching_skills_with_golden_sample_service_configured() {
        // Test that SkillExtractor can be configured with a GoldenSampleService
        // This verifies the wiring integration compiles and runs
        use crate::golden_sample_testing::GoldenSampleService;

        let store = SqliteMemoryStore::create("sqlite::memory:").await.unwrap();
        let golden_store =
            crate::golden_sample_testing::SqliteGoldenSampleStore::create("sqlite::memory:")
                .await
                .unwrap();
        let golden_service = GoldenSampleService::new(golden_store);

        // Create extractor with golden sample service - should not panic
        let extractor = SkillExtractor::with_golden_sample_service(
            store,
            ExtractionConfig::default(),
            std::env::temp_dir(),
            golden_service,
        );

        // find_matching_skills should work (return empty since no skills match)
        let matched = extractor.find_matching_skills("any task").await.unwrap();
        assert!(
            matched.is_empty(),
            "Should return empty when no skills exist"
        );

        // Verify golden_sample_service is set
        // We can't directly check private field, but the test passing means it works
    }

    #[tokio::test]
    async fn test_find_matching_skills_without_golden_sample_service() {
        // Test that without golden sample service, skills are returned without validation
        let store = SqliteMemoryStore::create("sqlite::memory:").await.unwrap();
        let extractor =
            SkillExtractor::new(store, ExtractionConfig::default(), std::env::temp_dir());

        // When golden_sample_service is None, find_matching_skills should return
        // skills without validation (backward compatibility)
        // This test verifies the backward compatibility path works
        let matched = extractor.find_matching_skills("any task").await.unwrap();
        // No skills in temp dir, so empty result is expected
        assert!(
            matched.is_empty(),
            "Should return empty when no skills match"
        );
    }
}
