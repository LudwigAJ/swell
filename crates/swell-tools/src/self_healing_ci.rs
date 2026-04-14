//! Self-healing CI tool for autonomous CI failure fixing.
//!
//! This module provides CI failure analysis, root cause identification,
//! and automated fix proposal and application.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use swell_core::traits::Tool;
use swell_core::{PermissionTier, SwellError, ToolOutput, ToolResultContent, ToolRiskLevel};
use tracing::{info, warn};

/// Tool for self-healing CI failures by analyzing errors and proposing corrections.
#[derive(Debug, Clone)]
pub struct CiHealingTool {
    max_retries: usize,
}

impl CiHealingTool {
    pub fn new() -> Self {
        Self { max_retries: 3 }
    }

    pub fn with_max_retries(mut self, retries: usize) -> Self {
        self.max_retries = retries;
        self
    }

    /// Parse CI output and extract structured failures
    fn parse_ci_output(&self, output: &str) -> Vec<CiFailure> {
        let mut failures = Vec::new();

        // Try to parse as structured JSON first
        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(output) {
            if let Some(errors) = parsed.get("errors").and_then(|e| e.as_array()) {
                for error in errors {
                    if let Some(msg) = error.get("message").and_then(|m| m.as_str()) {
                        let file = error.get("file").and_then(|f| f.as_str()).map(String::from);
                        let line = error.get("line").and_then(|l| l.as_u64()).map(|l| l as u32);

                        failures.push(CiFailure {
                            file,
                            line,
                            message: msg.to_string(),
                            severity: Self::infer_severity(msg),
                            category: Self::categorize_failure(msg),
                        });
                    }
                }
            }
        }

        // Fallback: parse as plain text with common patterns
        if failures.is_empty() {
            failures = self.parse_plain_text_ci_output(output);
        }

        failures
    }

    /// Parse plain text CI output for common error patterns
    fn parse_plain_text_ci_output(&self, output: &str) -> Vec<CiFailure> {
        let mut failures = Vec::new();

        for line in output.lines() {
            let trimmed = line.trim();

            // Rust compiler error patterns
            if trimmed.contains("error[E") {
                let (file, line_num) = Self::extract_file_location(trimmed);
                failures.push(CiFailure {
                    file,
                    line: line_num,
                    message: trimmed.to_string(),
                    severity: CiSeverity::Error,
                    category: FailureCategory::CompilationError,
                });
            }
            // Test failure patterns
            else if trimmed.contains("test result: FAILED")
                || trimmed.contains("FAILED")
                || trimmed.contains("thread '")
                || trimmed.contains("panicked at")
            {
                let (file, line_num) = Self::extract_file_location(trimmed);
                failures.push(CiFailure {
                    file,
                    line: line_num,
                    message: trimmed.to_string(),
                    severity: CiSeverity::Error,
                    category: FailureCategory::TestFailure,
                });
            }
            // Warning patterns
            else if trimmed.contains("warning:") {
                let (file, line_num) = Self::extract_file_location(trimmed);
                failures.push(CiFailure {
                    file,
                    line: line_num,
                    message: trimmed.to_string(),
                    severity: CiSeverity::Warning,
                    category: FailureCategory::LintError,
                });
            }
        }

        failures
    }

    /// Extract file and line number from error messages
    fn extract_file_location(msg: &str) -> (Option<String>, Option<u32>) {
        // Common pattern: /path/to/file.rs:123:45: error message
        // or file.rs:123: error message
        for part in msg.split_whitespace() {
            if let Some(pos) = part.find(":src/") {
                let path_part = &part[..pos + 5]; // Include "src/"
                if let Some(rest) = part.get(pos + 5..) {
                    let segments: Vec<&str> = rest.split(':').collect();
                    if !segments.is_empty() {
                        let file = format!("{}/{}", path_part, segments[0]);
                        let line = segments.get(1).and_then(|s| s.parse::<u32>().ok());
                        return (Some(file), line);
                    }
                }
            }
        }

        // Try simpler pattern: filename.rs:123
        for part in msg.split_whitespace() {
            if part.ends_with(".rs") || part.contains(".rs:") {
                if let Some(colon_pos) = part.find(':') {
                    let file_part = &part[..colon_pos];
                    let line_part = &part[colon_pos + 1..];
                    if let Ok(line) = line_part.parse::<u32>() {
                        return (Some(file_part.to_string()), Some(line));
                    }
                }
            }
        }

        (None, None)
    }

    /// Infer severity from error message
    fn infer_severity(msg: &str) -> CiSeverity {
        if msg.contains("error[E") || msg.contains("error:") {
            CiSeverity::Error
        } else if msg.contains("warning:") {
            CiSeverity::Warning
        } else {
            CiSeverity::Info
        }
    }

    /// Categorize the failure type from the message
    fn categorize_failure(msg: &str) -> FailureCategory {
        let lower = msg.to_lowercase();

        if lower.contains("cannot find")
            || lower.contains("use of undeclared")
            || lower.contains("not found in")
            || lower.contains("cannot find type")
        {
            FailureCategory::MissingImport
        } else if lower.contains("expected")
            || lower.contains("mismatched")
            || lower.contains("type mismatch")
        {
            FailureCategory::TypeError
        } else if lower.contains("syntax")
            || lower.contains("unexpected token")
            || (lower.contains("expected")
                && (lower.contains("{") || lower.contains(";") || lower.contains("]")))
        {
            FailureCategory::SyntaxError
        } else if lower.contains("test")
            || lower.contains("assertion")
            || lower.contains("panicked")
        {
            FailureCategory::TestFailure
        } else if lower.contains("warning") || lower.contains("lint") {
            FailureCategory::LintError
        } else if lower.contains("error[E") {
            FailureCategory::CompilationError
        } else {
            FailureCategory::Other
        }
    }

    /// Analyze failures to identify root causes
    fn analyze_failures(&self, failures: &[CiFailure]) -> Vec<RootCause> {
        let mut root_causes = Vec::new();

        // Group failures by file
        let mut failures_by_file: HashMap<String, Vec<&CiFailure>> = HashMap::new();
        for failure in failures {
            if let Some(ref file) = failure.file {
                failures_by_file
                    .entry(file.clone())
                    .or_default()
                    .push(failure);
            }
        }

        // Analyze patterns
        for (file, file_failures) in failures_by_file {
            let categories: Vec<_> = file_failures.iter().map(|f| f.category).collect();

            // Check for missing import pattern
            if categories.contains(&FailureCategory::MissingImport) {
                root_causes.push(RootCause {
                    description: format!(
                        "Missing imports in {} - need to add use statements",
                        file
                    ),
                    affected_files: vec![file.clone()],
                    confidence: 0.95,
                });
            }

            // Check for type error patterns
            if categories.contains(&FailureCategory::TypeError) {
                root_causes.push(RootCause {
                    description: format!(
                        "Type mismatch in {} - likely incorrect type annotation or value",
                        file
                    ),
                    affected_files: vec![file.clone()],
                    confidence: 0.85,
                });
            }

            // Check for compilation errors
            if categories.contains(&FailureCategory::CompilationError) {
                root_causes.push(RootCause {
                    description: format!(
                        "Compilation errors in {} - syntax or semantic errors",
                        file
                    ),
                    affected_files: vec![file.clone()],
                    confidence: 0.90,
                });
            }

            // Check for test failures
            if categories.contains(&FailureCategory::TestFailure) {
                root_causes.push(RootCause {
                    description: format!(
                        "Test failures in {} - tests may need updating or code needs fixing",
                        file
                    ),
                    affected_files: vec![file.clone()],
                    confidence: 0.80,
                });
            }
        }

        // If no specific patterns found, create a general root cause
        if root_causes.is_empty() && !failures.is_empty() {
            root_causes.push(RootCause {
                description: format!(
                    "{} CI failures detected - requires manual analysis",
                    failures.len()
                ),
                affected_files: failures
                    .iter()
                    .filter_map(|f| f.file.clone())
                    .collect::<std::collections::HashSet<_>>()
                    .into_iter()
                    .collect(),
                confidence: 0.5,
            });
        }

        root_causes
    }

    /// Generate fixes for the identified root causes
    fn generate_fixes(&self, root_causes: &[RootCause], failures: &[CiFailure]) -> Vec<CiFix> {
        let mut fixes = Vec::new();

        for root_cause in root_causes {
            match root_cause.description.contains("Missing imports") {
                true => {
                    // Extract what might be missing from error messages
                    for failure in failures {
                        if let Some(ref file) = failure.file {
                            if let Some(import) = Self::extract_missing_import(&failure.message) {
                                fixes.push(CiFix {
                                    description: format!(
                                        "Add missing import: {} to {}",
                                        import, file
                                    ),
                                    fix_type: FixType::AddImport,
                                    confidence: 0.90,
                                    code_change: Some(CodeChange {
                                        file: file.clone(),
                                        old_str: String::new(), // Will be filled by actual file content
                                        new_str: import,
                                        is_addition: true,
                                    }),
                                });
                            }
                        }
                    }
                }
                false => {
                    // For other root causes, suggest investigation
                    fixes.push(CiFix {
                        description: format!("Investigate and fix: {}", root_cause.description),
                        fix_type: FixType::Other,
                        confidence: root_cause.confidence * 0.7,
                        code_change: None,
                    });
                }
            }
        }

        fixes
    }

    /// Extract missing import from error message
    fn extract_missing_import(msg: &str) -> Option<String> {
        // Pattern: "use `module::Item`"
        if let Some(start) = msg.find("use `") {
            if let Some(end) = msg[start + 4..].find('`') {
                let candidate = &msg[start + 4..start + 4 + end];
                if candidate.contains("::") {
                    return Some(format!("use {};", candidate));
                }
            }
        }

        // Pattern: "cannot find `Item` in `module`" - with backticks
        if msg.contains("cannot find") && msg.contains(" in `") {
            if let Some(in_pos) = msg.find(" in `") {
                let remainder = &msg[in_pos + 5..];
                if let Some(end) = remainder.find('`') {
                    let module_path = &remainder[..end];
                    if module_path.contains("::") || module_path.contains('.') {
                        if let Some(before_in) = msg.find(" cannot find `") {
                            let item_start = before_in + 13;
                            if let Some(item_end) = msg[item_start..].find('`') {
                                let item_name = &msg[item_start..item_start + item_end];
                                return Some(format!("use {}::{};", module_path, item_name));
                            }
                        }
                        return Some(format!("use {};", module_path));
                    }
                }
            }
        }

        // Pattern: "cannot find `Item` in module" - without backticks on module
        if msg.contains("cannot find `") {
            // Find the item between backticks
            if let Some(item_start) = msg.find("cannot find `") {
                let start = item_start + 12;
                if let Some(item_end) = msg[start..].find('`') {
                    let item_name = &msg[start..start + item_end];

                    // Look for " in module_name" without backticks
                    let after_item = &msg[item_start + 13 + item_end..];
                    if let Some(in_pos) = after_item.find(" in ") {
                        let module_part = &after_item[in_pos + 4..];
                        // Get the module name (up to whitespace or end)
                        let module_name: String = module_part
                            .chars()
                            .take_while(|c| !c.is_whitespace() && *c != ',')
                            .collect();

                        if !module_name.is_empty()
                            && (module_name.contains("::") || module_name.contains('.'))
                        {
                            return Some(format!("use {}::{};", module_name, item_name));
                        }
                    }
                }
            }
        }

        // Pattern: "cannot find type `X` in module `Y`"
        if msg.contains("cannot find type") {
            if let Some(start) = msg.find("`") {
                if let Some(end) = msg[start + 1..].find('`') {
                    let type_name = &msg[start + 1..start + 1 + end];
                    return Some(format!("use /* module:: */ {};", type_name));
                }
            }
        }

        // Pattern: "module `crate::types`" - backtick content with ::
        for part in msg.split_whitespace() {
            if part.starts_with('`') && part.ends_with('`') && part.contains("::") {
                let inner = &part[1..part.len() - 1];
                return Some(format!("use {};", inner));
            }
        }

        // Pattern: backtick-quoted item that might be importable
        if let Some(start) = msg.find("`") {
            if let Some(end) = msg[start + 1..].find('`') {
                let candidate = &msg[start + 1..start + 1 + end];
                // If it's a capitalized identifier, might be a type to import
                if !candidate.contains("::") && !candidate.contains('.') && !candidate.is_empty() {
                    let first_char = candidate.chars().next().unwrap_or(' ');
                    if first_char.is_uppercase() {
                        return Some(format!("use /* module:: */ {};", candidate));
                    }
                }
            }
        }

        None
    }

    /// Apply a fix to a file
    async fn apply_fix(&self, fix: &CiFix, dry_run: bool) -> Result<ToolOutput, SwellError> {
        // Get the file path from the code_change if available
        let file = match fix.code_change.as_ref() {
            Some(cc) => cc.file.clone(),
            None => {
                return Ok(ToolOutput {
                    is_error: true,
                    content: vec![ToolResultContent::Error(
                        "No file specified in fix".to_string(),
                    )],
                });
            }
        };

        let file_path = Path::new(&file);

        if !file_path.exists() {
            return Err(SwellError::ToolExecutionFailed(format!(
                "File not found: {}",
                file
            )));
        }

        // Read current content
        let content = tokio::fs::read_to_string(file_path)
            .await
            .map_err(|e| SwellError::ToolExecutionFailed(e.to_string()))?;

        let new_content = match fix.fix_type {
            FixType::AddImport => {
                // Add import at the top of the file (after use statements or at the beginning)
                let import_str = fix
                    .code_change
                    .as_ref()
                    .and_then(|c| {
                        if c.is_addition {
                            Some(c.new_str.as_str())
                        } else {
                            None
                        }
                    })
                    .unwrap_or("use /* unknown */;");

                // Find a good insertion point (after existing use statements)
                let insertion_point = content
                    .lines()
                    .take_while(|line| line.starts_with("use ") || line.trim().is_empty())
                    .count();

                let mut lines: Vec<&str> = content.lines().collect();
                let insert_pos = if insertion_point > 0 {
                    insertion_point
                } else {
                    0
                };

                let import_line = format!("use {};", import_str.trim_end_matches(';'));
                lines.insert(insert_pos, &import_line);
                lines.join("\n")
            }
            _ => {
                // For other fixes, we cannot automatically apply without more context
                return Ok(ToolOutput {
                    is_error: dry_run,
                    content: vec![ToolResultContent::Json(serde_json::json!({
                        "applied": false,
                        "reason": "Fix type requires manual intervention",
                        "fix_description": fix.description,
                        "file": file
                    }))],
                });
            }
        };

        if dry_run {
            return Ok(ToolOutput {
                is_error: false,
                content: vec![ToolResultContent::Json(serde_json::json!({
                    "dry_run": true,
                    "would_change_file": file,
                    "proposed_content": new_content,
                    "fix_description": fix.description
                }))],
            });
        }

        // Apply the change
        tokio::fs::write(file_path, new_content)
            .await
            .map_err(|e| SwellError::ToolExecutionFailed(e.to_string()))?;

        info!(file = %file, "CI fix applied successfully");
        Ok(ToolOutput {
            is_error: false,
            content: vec![ToolResultContent::Json(serde_json::json!({
                "applied": true,
                "file": file,
                "fix_description": fix.description
            }))],
        })
    }
}

impl Default for CiHealingTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for CiHealingTool {
    fn name(&self) -> &str {
        "ci_healing"
    }

    fn description(&self) -> String {
        "Analyze CI failures, identify root causes, and propose automated fixes".to_string()
    }

    fn risk_level(&self) -> ToolRiskLevel {
        ToolRiskLevel::Write
    }

    fn permission_tier(&self) -> PermissionTier {
        PermissionTier::Ask
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "description": "Operation to perform: analyze, fix, auto_heal",
                    "enum": ["analyze", "fix", "auto_heal"]
                },
                "ci_output": {
                    "type": "string",
                    "description": "CI output to analyze (JSON or plain text)"
                },
                "max_retries": {
                    "type": "integer",
                    "description": "Maximum fix retry attempts",
                    "default": 3
                },
                "dry_run": {
                    "type": "boolean",
                    "description": "If true, show proposed fixes without applying them",
                    "default": false
                }
            },
            "required": ["operation"]
        })
    }

    async fn execute(&self, arguments: serde_json::Value) -> Result<ToolOutput, SwellError> {
        #[derive(Deserialize)]
        struct Args {
            operation: String,
            ci_output: Option<String>,
            max_retries: Option<usize>,
            dry_run: Option<bool>,
        }

        let args: Args = serde_json::from_value(arguments)
            .map_err(|e| SwellError::ToolExecutionFailed(e.to_string()))?;

        let dry_run = args.dry_run.unwrap_or(false);
        let max_retries = args.max_retries.unwrap_or(self.max_retries);

        match args.operation.as_str() {
            "analyze" => {
                let ci_output = args.ci_output.unwrap_or_default();

                let failures = self.parse_ci_output(&ci_output);
                let root_causes = self.analyze_failures(&failures);
                let fixes = self.generate_fixes(&root_causes, &failures);

                let analysis = CiFailureAnalysis {
                    failures,
                    root_causes,
                    suggested_fixes: fixes,
                };

                Ok(ToolOutput {
                    is_error: false,
                    content: vec![ToolResultContent::Text(
                        serde_json::to_string_pretty(&analysis)
                            .unwrap_or_else(|_| "{}".to_string()),
                    )],
                })
            }
            "fix" => {
                let ci_output = args.ci_output.unwrap_or_default();

                let failures = self.parse_ci_output(&ci_output);
                let root_causes = self.analyze_failures(&failures);
                let fixes = self.generate_fixes(&root_causes, &failures);

                let mut applied = Vec::new();
                let mut failed = Vec::new();

                for fix in fixes.iter().take(max_retries) {
                    match self.apply_fix(fix, dry_run).await {
                        Ok(result) => {
                            let has_error = result.is_error;
                            if !has_error {
                                applied.push(fix.description.clone());
                            }
                        }
                        Err(e) => {
                            warn!(error = %e, "Failed to apply fix: {}", fix.description);
                            failed.push(fix.description.clone());
                        }
                    }
                }

                Ok(ToolOutput {
                    is_error: false,
                    content: vec![ToolResultContent::Json(serde_json::json!({
                        "applied_count": applied.len(),
                        "failed_count": failed.len(),
                        "applied": applied,
                        "failed": failed,
                        "dry_run": dry_run
                    }))],
                })
            }
            "auto_heal" => {
                let ci_output = args.ci_output.unwrap_or_default();

                let failures = self.parse_ci_output(&ci_output);
                let root_causes = self.analyze_failures(&failures);
                let fixes = self.generate_fixes(&root_causes, &failures);

                let mut results = Vec::new();
                let mut retry_count = 0;
                let mut all_succeeded = true;

                while retry_count < max_retries && !fixes.is_empty() {
                    for fix in &fixes {
                        let result = self.apply_fix(fix, false).await;
                        let is_success = result.is_ok();
                        results.push(CiHealingResult {
                            fix_description: fix.description.clone(),
                            success: is_success,
                            error: result.err().map(|e| e.to_string()),
                        });

                        if !is_success {
                            all_succeeded = false;
                        }
                    }

                    if all_succeeded {
                        break;
                    }

                    retry_count += 1;
                }

                let success_count = results.iter().filter(|r| r.success).count();

                let (is_error, error_content) = if !all_succeeded {
                    (
                        true,
                        Some(ToolResultContent::Error(format!(
                            "{} fixes failed after {} retries",
                            fixes.len() - success_count,
                            retry_count
                        ))),
                    )
                } else {
                    (false, None)
                };

                let mut content_vec = vec![ToolResultContent::Json(serde_json::json!({
                    "total_fixes": fixes.len(),
                    "successful": success_count,
                    "failed": fixes.len() - success_count,
                    "retries": retry_count,
                    "results": results
                }))];
                if let Some(err) = error_content {
                    content_vec.push(err);
                }

                Ok(ToolOutput {
                    is_error,
                    content: content_vec,
                })
            }
            _ => Err(SwellError::ToolExecutionFailed(format!(
                "Unknown operation: {} (expected: analyze, fix, auto_heal)",
                args.operation
            ))),
        }
    }
}

// ============================================================================
// Supporting Types
// ============================================================================

/// Result of CI healing operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CiHealingResult {
    pub fix_description: String,
    pub success: bool,
    pub error: Option<String>,
}

/// Complete CI failure analysis result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CiFailureAnalysis {
    pub failures: Vec<CiFailure>,
    pub root_causes: Vec<RootCause>,
    pub suggested_fixes: Vec<CiFix>,
}

/// A single CI failure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CiFailure {
    pub file: Option<String>,
    pub line: Option<u32>,
    pub message: String,
    pub severity: CiSeverity,
    pub category: FailureCategory,
}

/// CI failure severity
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CiSeverity {
    Error,
    Warning,
    Info,
}

/// Category of CI failure
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FailureCategory {
    CompilationError,
    TestFailure,
    LintError,
    TypeError,
    MissingImport,
    SyntaxError,
    Other,
}

/// A root cause identified from failures
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RootCause {
    pub description: String,
    pub affected_files: Vec<String>,
    pub confidence: f32,
}

/// A suggested fix for a root cause
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CiFix {
    pub description: String,
    pub fix_type: FixType,
    pub confidence: f32,
    pub code_change: Option<CodeChange>,
}

/// Type of fix to apply
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FixType {
    AddImport,
    FixSyntax,
    UpdateType,
    FixTest,
    Configuration,
    Other,
}

/// A code change to apply
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeChange {
    pub file: String,
    pub old_str: String,
    pub new_str: String,
    pub is_addition: bool,
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_ci_healing_tool_analyze() {
        let tool = CiHealingTool::new();
        let ci_output = r#"{"errors": [{"message": "cannot find `HttpClient` in `crate::client`", "file": "src/network.rs", "line": 10}]}"#;

        let result = tool
            .execute(serde_json::json!({
                "operation": "analyze",
                "ci_output": ci_output
            }))
            .await
            .unwrap();

        assert!(!result.is_error);

        let content_str = match &result.content[0] {
            ToolResultContent::Text(s) => s,
            _ => panic!("Expected Text content"),
        };
        let analysis: CiFailureAnalysis = serde_json::from_str(content_str).unwrap();
        assert!(!analysis.failures.is_empty());
        assert!(!analysis.root_causes.is_empty());
        assert!(!analysis.suggested_fixes.is_empty());
    }

    #[tokio::test]
    async fn test_parse_rust_compiler_errors() {
        let tool = CiHealingTool::new();
        let output = r#"
error[E0603]: private struct `Config` is not visible here
  --> src/api.rs:42:5
   |
42 |     let config = Config::new();
   |                  ^^^^^ private struct
"#;

        let failures = tool.parse_ci_output(output);
        assert!(!failures.is_empty());
        assert_eq!(failures[0].category, FailureCategory::CompilationError);
        assert_eq!(failures[0].severity, CiSeverity::Error);
    }

    #[tokio::test]
    async fn test_parse_test_failures() {
        let tool = CiHealingTool::new();
        let output = r#"
running 5 tests
test tests::test_one ... FAILED
test tests::test_two ... ok

test result: FAILED. 1 passed, 4 failed;
"#;

        let failures = tool.parse_ci_output(output);
        assert!(!failures.is_empty());
        assert!(failures
            .iter()
            .any(|f| f.category == FailureCategory::TestFailure));
    }

    #[tokio::test]
    async fn test_extract_missing_import() {
        let msg = "cannot find `MyStruct` in module `crate::types`";
        let import = CiHealingTool::extract_missing_import(msg);
        assert!(import.is_some());

        let msg2 = "use `std::collections::HashMap`";
        let import2 = CiHealingTool::extract_missing_import(msg2);
        assert!(import2.is_some());
    }

    #[tokio::test]
    async fn test_analyze_failures_groups_by_file() {
        let tool = CiHealingTool::new();

        let failures = vec![
            CiFailure {
                file: Some("src/lib.rs".to_string()),
                line: Some(10),
                message: "cannot find `HttpClient`".to_string(),
                severity: CiSeverity::Error,
                category: FailureCategory::MissingImport,
            },
            CiFailure {
                file: Some("src/lib.rs".to_string()),
                line: Some(20),
                message: "expected `i32`, found `String`".to_string(),
                severity: CiSeverity::Error,
                category: FailureCategory::TypeError,
            },
        ];

        let root_causes = tool.analyze_failures(&failures);
        assert!(!root_causes.is_empty());

        // Should have identified missing import and type error
        assert!(root_causes
            .iter()
            .any(|r| r.description.contains("Missing imports")));
        assert!(root_causes
            .iter()
            .any(|r| r.description.contains("Type mismatch")));
    }

    #[tokio::test]
    async fn test_generate_fixes_for_missing_import() {
        let tool = CiHealingTool::new();

        let root_causes = vec![RootCause {
            description: "Missing imports in src/lib.rs - need to add use statements".to_string(),
            affected_files: vec!["src/lib.rs".to_string()],
            confidence: 0.95,
        }];

        let failures = vec![CiFailure {
            file: Some("src/lib.rs".to_string()),
            line: Some(10),
            message: "cannot find `HttpClient` in crate::client".to_string(),
            severity: CiSeverity::Error,
            category: FailureCategory::MissingImport,
        }];

        let fixes = tool.generate_fixes(&root_causes, &failures);
        assert!(!fixes.is_empty());
        assert!(fixes.iter().any(|f| f.fix_type == FixType::AddImport));
    }

    #[tokio::test]
    async fn test_dry_run_does_not_modify_files() {
        let tool = CiHealingTool::new();
        let temp_dir = tempfile::tempdir().unwrap();
        let file_path = temp_dir.path().join("test.rs");

        // Create a file
        tokio::fs::write(&file_path, "fn main() {}\n")
            .await
            .unwrap();

        let fix = CiFix {
            description: "Add import".to_string(),
            fix_type: FixType::AddImport,
            confidence: 0.9,
            code_change: Some(CodeChange {
                file: file_path.to_string_lossy().to_string(),
                old_str: String::new(),
                new_str: "use std::io;".to_string(),
                is_addition: true,
            }),
        };

        let result = tool.apply_fix(&fix, true).await.unwrap();
        let parsed = match &result.content[0] {
            ToolResultContent::Json(v) => v.clone(),
            _ => panic!("Expected Json content"),
        };

        assert!(!result.is_error);
        assert!(parsed.get("dry_run").and_then(|v| v.as_bool()).unwrap());

        // File should still be unchanged
        let content = tokio::fs::read_to_string(&file_path).await.unwrap();
        assert_eq!(content, "fn main() {}\n");
    }
}
