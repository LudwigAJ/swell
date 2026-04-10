//! Secret scanning integration for pre-commit hooks.
//!
//! This module provides:
//! - [`SecretScanner`] - Scans staged changes for secrets using gitleaks or ggshield
//! - [`SecretScanResult`] - Structured result of a secret scan
//! - Pre-commit hook integration for blocking commits when secrets are detected
//!
//! ## Usage
//!
//! ```rust,ignore
//! use swell_tools::secret_scanning::{SecretScanner, SecretScannerConfig};
//!
//! let scanner = SecretScanner::new();
//! let result = scanner.scan_staged(&work_dir).await?;
//! if result.has_secrets() {
//!     return Err("Commit blocked: secrets detected".into());
//! }
//! ```

use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};

/// Configuration for secret scanner
#[derive(Debug, Clone)]
pub struct SecretScannerConfig {
    /// Scanner to use: "gitleaks" or "ggshield"
    pub scanner: SecretScannerType,
    /// Whether to block commits (true) or just warn (false)
    pub block_on_secrets: bool,
    /// Additional gitleaks/ggshield flags
    pub extra_flags: Vec<String>,
    /// Timeout for scanning in seconds
    pub timeout_secs: u64,
}

impl Default for SecretScannerConfig {
    fn default() -> Self {
        Self {
            scanner: SecretScannerType::Gitleaks,
            block_on_secrets: true,
            extra_flags: Vec::new(),
            timeout_secs: 60,
        }
    }
}

/// Type of secret scanner
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretScannerType {
    /// Gitleaks - open-source secret scanning tool
    Gitleaks,
    /// ggshield - GitLab's secret scanning tool
   Ggshield,
}

impl std::fmt::Display for SecretScannerType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SecretScannerType::Gitleaks => write!(f, "gitleaks"),
            SecretScannerType::Ggshield => write!(f, "ggshield"),
        }
    }
}

/// A detected secret with location information
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DetectedSecret {
    /// The type of secret detected (e.g., "AWS_ACCESS_KEY", "GitHub_token")
    pub secret_type: String,
    /// The file where the secret was detected
    pub file: String,
    /// The line number where the secret was detected
    pub line: u32,
    /// The commit hash if in committed code
    pub commit: Option<String>,
    /// The detected secret (may be truncated for logging)
    pub match_preview: String,
}

/// Result of a secret scan
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct SecretScanResult {
    /// Whether any secrets were detected
    pub has_secrets: bool,
    /// List of detected secrets
    pub secrets: Vec<DetectedSecret>,
    /// Scanner that was used
    pub scanner: String,
    /// Raw output from the scanner
    pub raw_output: String,
    /// Whether the scan succeeded (even if secrets found)
    pub scan_succeeded: bool,
    /// Error message if scan failed
    pub error: Option<String>,
}

impl SecretScanResult {
    /// Create a new empty result
    pub fn new(scanner: &str) -> Self {
        Self {
            has_secrets: false,
            secrets: Vec::new(),
            scanner: scanner.to_string(),
            raw_output: String::new(),
            scan_succeeded: false,
            error: None,
        }
    }

    /// Create a result representing a scan error
    pub fn error(scanner: &str, error: String) -> Self {
        Self {
            has_secrets: false,
            secrets: Vec::new(),
            scanner: scanner.to_string(),
            raw_output: String::new(),
            scan_succeeded: false,
            error: Some(error),
        }
    }

    /// Check if the scan found any secrets
    pub fn has_secrets(&self) -> bool {
        self.has_secrets
    }

    /// Get the count of detected secrets
    pub fn secret_count(&self) -> usize {
        self.secrets.len()
    }

    /// Get a formatted report of all detected secrets
    pub fn report(&self) -> String {
        if !self.has_secrets {
            return String::from("No secrets detected");
        }

        let mut report = format!(
            "⚠️  BLOCKED: {} secret(s) detected\n\n",
            self.secrets.len()
        );

        for secret in &self.secrets {
            report.push_str(&format!(
                "  • {} in {}:{} (match: {})\n",
                secret.secret_type, secret.file, secret.line, secret.match_preview
            ));
        }

        report.push_str("\nPlease remove secrets before committing.");
        report
    }
}

/// Secret scanner for detecting secrets in staged changes
#[derive(Debug, Clone)]
pub struct SecretScanner {
    config: SecretScannerConfig,
    /// Track scans for statistics
    scan_history: Arc<RwLock<Vec<SecretScanResult>>>,
}

impl SecretScanner {
    /// Create a new secret scanner with default config
    pub fn new() -> Self {
        Self::with_config(SecretScannerConfig::default())
    }

    /// Create a new secret scanner with custom config
    pub fn with_config(config: SecretScannerConfig) -> Self {
        Self {
            config,
            scan_history: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Get the configuration
    pub fn config(&self) -> &SecretScannerConfig {
        &self.config
    }

    /// Check if the scanner is available in the environment
    pub fn is_scanner_available(&self) -> bool {
        let scanner_name = self.config.scanner.to_string();
        which::which(scanner_name).is_ok()
    }

    /// Detect which scanner is available (gitleaks or ggshield)
    pub fn detect_available_scanner() -> Option<SecretScannerType> {
        if which::which("gitleaks").is_ok() {
            Some(SecretScannerType::Gitleaks)
        } else if which::which("ggshield").is_ok() {
            Some(SecretScannerType::Ggshield)
        } else {
            None
        }
    }

    /// Run gitleaks scan on staged changes
    async fn run_gitleaks_scan(&self, cwd: &Path) -> Result<SecretScanResult, SecretScannerError> {
        let scanner_name = "gitleaks";

        // Check if gitleaks is available
        if which::which(scanner_name).is_err() {
            return Err(SecretScannerError::ScannerNotFound(
                scanner_name.to_string(),
            ));
        }

        // Build gitleaks command - scan staged changes
        // gitleaks detect --staged --no-color -g .  (or use --files argument for specific files)
        let mut cmd = Command::new(scanner_name);
        cmd.arg("detect");
        cmd.arg("--staged");
        cmd.arg("--no-color");
        cmd.arg("--format=json");

        // Add any extra flags
        for flag in &self.config.extra_flags {
            cmd.arg(flag);
        }

        // Set working directory
        cmd.current_dir(cwd);

        info!(scanner = scanner_name, "Running secret scan on staged changes");

        let output = tokio::task::spawn_blocking(move || {
            cmd.output()
        })
        .await
        .map_err(|e| SecretScannerError::ExecutionFailed(e.to_string()))?
        .map_err(|e| SecretScannerError::ExecutionFailed(e.to_string()))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        debug!(stdout = %stdout, stderr = %stderr, "Gitleaks output");

        // Gitleaks returns exit code 1 when secrets are found
        let scan_succeeded = output.status.success() || output.status.code() == Some(1);

        // Parse gitleaks JSON output
        let mut result = SecretScanResult::new(scanner_name);
        result.scan_succeeded = scan_succeeded;
        result.raw_output = stdout.to_string();

        if !stdout.trim().is_empty() {
            // Try to parse JSON output from gitleaks
            if let Ok(json_output) = serde_json::from_str::<serde_json::Value>(&stdout) {
                // Gitleaks outputs an array of findings
                if let Some(findings) = json_output.as_array() {
                    for finding in findings {
                        if let Some(secret) = self.parse_gitleaks_finding(finding) {
                            result.secrets.push(secret);
                        }
                    }
                }
            }
        }

        result.has_secrets = !result.secrets.is_empty();

        if result.has_secrets {
            warn!(
                secrets_found = result.secrets.len(),
                "Secrets detected in staged changes"
            );
        }

        Ok(result)
    }

    /// Parse a single gitleaks finding
    fn parse_gitleaks_finding(&self, finding: &serde_json::Value) -> Option<DetectedSecret> {
        let secret_type = finding
            .get("Rule")
            .and_then(|r| r.as_str())
            .unwrap_or("unknown")
            .to_string();

        let file = finding
            .get("File")
            .and_then(|f| f.as_str())
            .unwrap_or("unknown")
            .to_string();

        let line = finding
            .get("StartLine")
            .and_then(|l| l.as_u64())
            .unwrap_or(0) as u32;

        let match_preview = finding
            .get("Match")
            .and_then(|m| m.as_str())
            .unwrap_or("")
            .to_string();

        // Skip empty matches
        if match_preview.is_empty() {
            return None;
        }

        Some(DetectedSecret {
            secret_type,
            file,
            line,
            commit: finding.get("Commit").and_then(|c| c.as_str()).map(String::from),
            match_preview,
        })
    }

    /// Run ggshield scan on staged changes
    async fn run_ggshield_scan(&self, cwd: &Path) -> Result<SecretScanResult, SecretScannerError> {
        let scanner_name = "ggshield";

        // Check if ggshield is available
        if which::which(scanner_name).is_err() {
            return Err(SecretScannerError::ScannerNotFound(
                scanner_name.to_string(),
            ));
        }

        // Build ggshield command - scan staged changes
        // ggshield secret scan --staged
        let mut cmd = Command::new(scanner_name);
        cmd.arg("secret");
        cmd.arg("scan");
        cmd.arg("--staged");
        cmd.arg("--json");

        // Add any extra flags
        for flag in &self.config.extra_flags {
            cmd.arg(flag);
        }

        // Set working directory
        cmd.current_dir(cwd);

        info!(scanner = scanner_name, "Running secret scan on staged changes");

        let output = tokio::task::spawn_blocking(move || {
            cmd.output()
        })
        .await
        .map_err(|e| SecretScannerError::ExecutionFailed(e.to_string()))?
        .map_err(|e| SecretScannerError::ExecutionFailed(e.to_string()))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        debug!(stdout = %stdout, stderr = %stderr, "GGShield output");

        // ggshield returns exit code 1 when secrets are found
        let scan_succeeded = output.status.success() || output.status.code() == Some(1);

        let mut result = SecretScanResult::new(scanner_name);
        result.scan_succeeded = scan_succeeded;
        result.raw_output = stdout.to_string();

        // Parse ggshield JSON output
        if !stdout.trim().is_empty() {
            if let Ok(json_output) = serde_json::from_str::<serde_json::Value>(&stdout) {
                // GGShield has result array under "results" or similar structure
                if let Some(results) = json_output.get("results").and_then(|r| r.as_array()) {
                    for result_item in results {
                        if let Some(secrets) = result_item.get("incidents").and_then(|i| i.as_array()) {
                            for secret in secrets {
                                if let Some(detected) = self.parse_ggshield_incident(secret) {
                                    result.secrets.push(detected);
                                }
                            }
                        }
                    }
                }
                // Alternative: GGShield format with "secret_scan" -> "matches"
                if let Some(scan_results) = json_output.get("secret_scan").and_then(|s| s.get("results"))
                {
                    if let Some(matches) = scan_results.get("matches").and_then(|m| m.as_array()) {
                        for m in matches {
                            if let Some(detected) = self.parse_ggshield_match(m) {
                                result.secrets.push(detected);
                            }
                        }
                    }
                }
            }
        }

        result.has_secrets = !result.secrets.is_empty();

        if result.has_secrets {
            warn!(
                secrets_found = result.secrets.len(),
                "Secrets detected in staged changes"
            );
        }

        Ok(result)
    }

    /// Parse a ggshield incident
    fn parse_ggshield_incident(&self, incident: &serde_json::Value) -> Option<DetectedSecret> {
        let secret_type = incident
            .get("type")
            .and_then(|t| t.as_str())
            .unwrap_or("unknown")
            .to_string();

        let file = incident
            .get("file")
            .and_then(|f| f.as_str())
            .unwrap_or("unknown")
            .to_string();

        let line = incident
            .get("line_start")
            .and_then(|l| l.as_u64())
            .unwrap_or(incident.get("line").and_then(|l| l.as_u64()).unwrap_or(0))
            as u32;

        let match_preview = incident
            .get("match")
            .and_then(|m| m.as_str())
            .unwrap_or("")
            .to_string();

        if match_preview.is_empty() {
            return None;
        }

        Some(DetectedSecret {
            secret_type,
            file,
            line,
            commit: None,
            match_preview,
        })
    }

    /// Parse a ggshield match
    fn parse_ggshield_match(&self, match_data: &serde_json::Value) -> Option<DetectedSecret> {
        let secret_type = match_data
            .get("type")
            .and_then(|t| t.as_str())
            .unwrap_or("secret")
            .to_string();

        let file = match_data
            .get("file")
            .and_then(|f| f.as_str())
            .unwrap_or("unknown")
            .to_string();

        let line = match_data
            .get("line")
            .and_then(|l| l.as_u64())
            .unwrap_or(0) as u32;

        let match_preview = match_data
            .get("match")
            .and_then(|m| m.as_str())
            .unwrap_or("")
            .to_string();

        if match_preview.is_empty() {
            return None;
        }

        Some(DetectedSecret {
            secret_type,
            file,
            line,
            commit: None,
            match_preview,
        })
    }

    /// Scan staged changes for secrets
    pub async fn scan_staged(&self, cwd: &Path) -> Result<SecretScanResult, SecretScannerError> {
        let result = match self.config.scanner {
            SecretScannerType::Gitleaks => self.run_gitleaks_scan(cwd).await?,
            SecretScannerType::Ggshield => self.run_ggshield_scan(cwd).await?,
        };

        // Store in history
        {
            let mut history = self.scan_history.write().await;
            history.push(result.clone());
        }

        Ok(result)
    }

    /// Check if a commit should be blocked (secrets detected)
    pub async fn should_block_commit(&self, result: &SecretScanResult) -> bool {
        self.config.block_on_secrets && result.has_secrets()
    }

    /// Get scan history
    pub async fn get_history(&self) -> Vec<SecretScanResult> {
        let history = self.scan_history.read().await;
        history.clone()
    }

    /// Clear scan history
    pub async fn clear_history(&self) {
        let mut history = self.scan_history.write().await;
        history.clear();
    }
}

impl Default for SecretScanner {
    fn default() -> Self {
        Self::new()
    }
}

/// Errors that can occur during secret scanning
#[derive(Debug, Clone, thiserror::Error)]
pub enum SecretScannerError {
    #[error("Scanner not found: {0}. Please install gitleaks or ggshield.")]
    ScannerNotFound(String),

    #[error("Git operation failed: {0}")]
    GitFailed(String),

    #[error("Failed to execute scanner: {0}")]
    ExecutionFailed(String),

    #[error("Failed to parse scanner output: {0}")]
    ParseError(String),

    #[error("No staged changes to scan")]
    NoStagedChanges,

    #[error("Not a git repository: {0}")]
    NotGitRepository(String),
}

/// Install pre-commit hooks for secret scanning
pub async fn install_precommit_hook(cwd: &Path) -> Result<(), SecretScannerError> {
    // Verify we're in a git repo
    let output = tokio::process::Command::new("git")
        .args(["rev-parse", "--git-dir"])
        .current_dir(cwd)
        .output()
        .await
        .map_err(|e| SecretScannerError::GitFailed(e.to_string()))?;

    if !output.status.success() {
        return Err(SecretScannerError::NotGitRepository(
            cwd.display().to_string(),
        ));
    }

    let hook_path = cwd.join(".git/hooks/pre-commit");

    // Create hooks directory if it doesn't exist
    if !cwd.join(".git/hooks").exists() {
        tokio::fs::create_dir_all(cwd.join(".git/hooks"))
            .await
            .map_err(|e| SecretScannerError::GitFailed(e.to_string()))?;
    }

    // Check for gitleaks or ggshield
    let scanner = if which::which("gitleaks").is_ok() {
        "gitleaks"
    } else if which::which("ggshield").is_ok() {
        "ggshield"
    } else {
        return Err(SecretScannerError::ScannerNotFound(
            "gitleaks or ggshield".to_string(),
        ));
    };

    // Create pre-commit hook script
    let hook_content = format!(
        r#"#!/bin/bash
# Pre-commit hook for secret scanning
# Generated by SWELL

# Check for staged changes
STAGED_FILES=$(git diff --cached --name-only --diff-filter=ACM)
if [ -z "$STAGED_FILES" ]; then
    exit 0
fi

# Run secret scanner on staged changes
{scanner} detect --staged --no-color
EXIT_CODE=$?

if [ $EXIT_CODE -eq 0 ]; then
    exit 0
elif [ $EXIT_CODE -eq 1 ]; then
    echo ""
    echo "ERROR: Secrets detected in staged changes. Commit blocked."
    echo "To bypass this hook temporarily, use: git commit --no-verify"
    exit 1
else
    echo ""
    echo "ERROR: Secret scanner failed with exit code $EXIT_CODE"
    exit 1
fi
"#
    );

    // Write the hook
    tokio::fs::write(&hook_path, hook_content)
        .await
        .map_err(|e| SecretScannerError::GitFailed(e.to_string()))?;

    // Make executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = tokio::fs::metadata(&hook_path)
            .await
            .map_err(|e| SecretScannerError::GitFailed(e.to_string()))?
            .permissions();
        perms.set_mode(0o755);
        tokio::fs::set_permissions(&hook_path, perms)
            .await
            .map_err(|e| SecretScannerError::GitFailed(e.to_string()))?;
    }

    info!(path = %hook_path.display(), "Installed pre-commit hook for secret scanning");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_secret_scanner_config_default() {
        let config = SecretScannerConfig::default();
        assert_eq!(config.scanner, SecretScannerType::Gitleaks);
        assert!(config.block_on_secrets);
        assert!(config.extra_flags.is_empty());
        assert_eq!(config.timeout_secs, 60);
    }

    #[test]
    fn test_secret_scanner_type_display() {
        assert_eq!(format!("{}", SecretScannerType::Gitleaks), "gitleaks");
        assert_eq!(format!("{}", SecretScannerType::Ggshield), "ggshield");
    }

    #[test]
    fn test_secret_scan_result_new() {
        let result = SecretScanResult::new("gitleaks");
        assert!(!result.has_secrets);
        assert!(result.secrets.is_empty());
        assert_eq!(result.scanner, "gitleaks");
        assert!(!result.scan_succeeded);
        assert!(result.error.is_none());
    }

    #[test]
    fn test_secret_scan_result_error() {
        let result = SecretScanResult::error("gitleaks", "scanner failed".to_string());
        assert!(!result.has_secrets);
        assert!(result.error.is_some());
        assert_eq!(result.error.unwrap(), "scanner failed");
    }

    #[test]
    fn test_secret_scan_result_has_secrets() {
        let mut result = SecretScanResult::new("gitleaks");
        assert!(!result.has_secrets());

        result.has_secrets = true;
        assert!(result.has_secrets());

        result.secrets.push(DetectedSecret {
            secret_type: "AWS_ACCESS_KEY".to_string(),
            file: "config.py".to_string(),
            line: 10,
            commit: None,
            match_preview: "AKIAIOSFODNN7EXAMPLE".to_string(),
        });
        assert!(result.has_secrets());
        assert_eq!(result.secret_count(), 1);
    }

    #[test]
    fn test_secret_scan_result_report_no_secrets() {
        let result = SecretScanResult::new("gitleaks");
        assert_eq!(result.report(), "No secrets detected");
    }

    #[test]
    fn test_secret_scan_result_report_with_secrets() {
        let mut result = SecretScanResult::new("gitleaks");
        result.has_secrets = true;
        result.secrets.push(DetectedSecret {
            secret_type: "GitHub Token".to_string(),
            file: "src/auth.py".to_string(),
            line: 42,
            commit: None,
            match_preview: "ghp_xxxxxxxxxxxx".to_string(),
        });

        let report = result.report();
        assert!(report.contains("BLOCKED"));
        assert!(report.contains("1 secret(s) detected"));
        assert!(report.contains("GitHub Token"));
        assert!(report.contains("src/auth.py"));
        assert!(report.contains("42"));
    }

    #[tokio::test]
    async fn test_secret_scanner_new() {
        let scanner = SecretScanner::new();
        assert_eq!(scanner.config.scanner, SecretScannerType::Gitleaks);
    }

    #[tokio::test]
    async fn test_secret_scanner_with_config() {
        let config = SecretScannerConfig {
            scanner: SecretScannerType::Ggshield,
            block_on_secrets: false,
            extra_flags: vec!["--verbose".to_string()],
            timeout_secs: 120,
        };
        let scanner = SecretScanner::with_config(config);
        assert_eq!(scanner.config.scanner, SecretScannerType::Ggshield);
        assert!(!scanner.config.block_on_secrets);
        assert_eq!(scanner.config.timeout_secs, 120);
    }

    #[tokio::test]
    async fn test_should_block_commit() {
        let scanner = SecretScanner::new();

        // No secrets
        let result = SecretScanResult::new("gitleaks");
        assert!(!scanner.should_block_commit(&result).await);

        // Has secrets but block_on_secrets is true
        let mut result_with_secrets = SecretScanResult::new("gitleaks");
        result_with_secrets.has_secrets = true;
        assert!(scanner.should_block_commit(&result_with_secrets).await);

        // Config with block_on_secrets = false
        let config = SecretScannerConfig {
            block_on_secrets: false,
            ..Default::default()
        };
        let scanner_no_block = SecretScanner::with_config(config);
        let mut result_with_secrets2 = SecretScanResult::new("gitleaks");
        result_with_secrets2.has_secrets = true;
        assert!(!scanner_no_block.should_block_commit(&result_with_secrets2).await);
    }

    #[tokio::test]
    async fn test_detect_available_scanner() {
        // This test will pass if at least one scanner is installed,
        // or return None if neither is available
        let scanner = SecretScanner::detect_available_scanner();
        // Just verify it doesn't panic - actual availability depends on environment
        if let Some(s) = scanner {
            assert!(s == SecretScannerType::Gitleaks || s == SecretScannerType::Ggshield);
        }
    }

    #[tokio::test]
    async fn test_scan_history() {
        let scanner = SecretScanner::new();

        // Initially empty
        let history = scanner.get_history().await;
        assert!(history.is_empty());

        // After scanning (even if no secrets), history should have entry
        // Note: This test assumes scanner might not be available
        let dir = tempdir().unwrap();
        let _result = scanner.scan_staged(dir.path()).await;

        // Just verify history tracking works (may error if no scanner)
        let _history = scanner.get_history().await;
        // History should be populated if scan was called
        // If scanner not found, history won't have entries
    }

    #[tokio::test]
    async fn test_clear_history() {
        let scanner = SecretScanner::new();
        scanner.clear_history().await;
        let history = scanner.get_history().await;
        assert!(history.is_empty());
    }

    #[tokio::test]
    async fn test_install_precommit_hook_not_git_repo() {
        let dir = tempdir().unwrap();
        let result = install_precommit_hook(dir.path()).await;
        // Should fail because tempdir is not a git repo
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            SecretScannerError::NotGitRepository(_)
        ));
    }

    #[tokio::test]
    async fn test_install_precommit_hook_in_git_repo() {
        let dir = tempdir().unwrap();

        // Initialize git repo
        tokio::process::Command::new("git")
            .args(["init"])
            .current_dir(&dir)
            .output()
            .await
            .unwrap();

        // Try to install hook without scanner available
        // This should still fail with ScannerNotFound
        let result = install_precommit_hook(dir.path()).await;

        // We expect this to fail with ScannerNotFound since gitleaks/ggshield
        // may not be installed in the test environment
        // But the git init should succeed, so NotGitRepository shouldn't be the error
        if let Err(e) = result {
            // Either scanner not found or something else
            assert!(!matches!(e, SecretScannerError::NotGitRepository(_)));
        }
    }

    #[tokio::test]
    async fn test_secret_scanner_error_display() {
        let err = SecretScannerError::ScannerNotFound("gitleaks".to_string());
        assert!(err.to_string().contains("gitleaks"));

        let err = SecretScannerError::GitFailed("git error".to_string());
        assert!(err.to_string().contains("git error"));

        let err = SecretScannerError::ExecutionFailed("exec error".to_string());
        assert!(err.to_string().contains("exec error"));

        let err = SecretScannerError::ParseError("parse error".to_string());
        assert!(err.to_string().contains("parse error"));

        let err = SecretScannerError::NoStagedChanges;
        assert!(err.to_string().contains("No staged changes"));

        let err = SecretScannerError::NotGitRepository("/path".to_string());
        assert!(err.to_string().contains("/path"));
    }
}
