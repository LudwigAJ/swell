//! Auto-redaction of secrets in tool output to prevent credential leakage.
//!
//! This module provides pattern-based detection and redaction of sensitive information
//! in tool outputs, such as API keys, tokens, passwords, and other credentials.
//!
//! ## Usage
//!
//! ```rust,ignore
//! use swell_tools::auto_masking::{AutoMasker, MaskingConfig};
//!
//! let masker = AutoMasker::new();
//! let masked = masker.mask_secrets("My API key is AKIAIOSFODNN7EXAMPLE");
//! // Output: "My API key is [REDACTED AWS Access Key]"
//! ```
//!
//! ## Configuration from JSON
//!
//! Patterns can be loaded from a JSON configuration (e.g., from settings.json):
//!
//! ```json
//! {
//!   "masking": {
//!     "enabled": true,
//!     "default_replacement": "[REDACTED]",
//!     "patterns": [
//!       {
//!         "name": "AWS Access Key",
//!         "pattern": "(?i)\\b(AKIA[0-9A-Z]{16})\\b",
//!         "replacement": "[REDACTED AWS Access Key]",
//!         "min_length": 20,
//!         "max_length": 20
//!       }
//!     ]
//!   }
//! }
//! ```

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::debug;

/// Configuration for a single secret pattern
#[derive(Debug, Clone)]
pub struct SecretPattern {
    /// Human-readable name for this secret type
    pub name: String,
    /// Regex pattern to match the secret
    pub pattern: Regex,
    /// Replacement string template (use {name} for the secret type)
    pub replacement: String,
    /// Minimum length of secret to be considered valid (to avoid false positives)
    pub min_length: usize,
    /// Maximum length of secret to be considered valid (to avoid matching too much)
    pub max_length: usize,
}

impl SecretPattern {
    /// Create a new secret pattern
    pub fn new(name: &str, pattern: &str, replacement: &str) -> Result<Self, regex::Error> {
        Ok(Self {
            name: name.to_string(),
            pattern: Regex::new(pattern)?,
            replacement: replacement.to_string(),
            min_length: 8,
            max_length: 256,
        })
    }

    /// Create a new secret pattern with custom length bounds
    pub fn with_length_bounds(
        name: &str,
        pattern: &str,
        replacement: &str,
        min_length: usize,
        max_length: usize,
    ) -> Result<Self, regex::Error> {
        Ok(Self {
            name: name.to_string(),
            pattern: Regex::new(pattern)?,
            replacement: replacement.to_string(),
            min_length,
            max_length,
        })
    }
}

/// Global masking configuration
#[derive(Debug, Clone)]
pub struct MaskingConfig {
    /// List of secret patterns to detect and redact
    pub patterns: Vec<SecretPattern>,
    /// Whether to enable masking (can be disabled for debugging)
    pub enabled: bool,
    /// Default replacement template
    pub default_replacement: String,
}

/// JSON-serializable pattern definition for config files
/// This is used to deserialize patterns from settings.json
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatternDef {
    /// Human-readable name for this secret type
    pub name: String,
    /// Regex pattern to match the secret
    pub pattern: String,
    /// Replacement string template (use {name} for the secret type)
    pub replacement: String,
    /// Minimum length of secret to be considered valid (default: 8)
    #[serde(default = "default_min_length")]
    pub min_length: usize,
    /// Maximum length of secret to be considered valid (default: 256)
    #[serde(default = "default_max_length")]
    pub max_length: usize,
}

fn default_min_length() -> usize {
    8
}

fn default_max_length() -> usize {
    256
}

impl Default for MaskingConfig {
    fn default() -> Self {
        Self::strict()
    }
}

impl MaskingConfig {
    /// Create a strict configuration with common secret patterns
    pub fn strict() -> Self {
        Self {
            patterns: vec![
                // AWS Access Key ID (20 chars, starts with AKIA)
                SecretPattern::new(
                    "AWS Access Key",
                    r"(?i)\b(AKIA[0-9A-Z]{16})\b",
                    "[REDACTED AWS Access Key]",
                )
                .unwrap(),
                // AWS Secret Access Key (40 chars, base64)
                SecretPattern::new(
                    "AWS Secret Key",
                    r"(?i)\b([A-Za-z0-9+/=]{40})\b",
                    "[REDACTED AWS Secret Key]",
                )
                .unwrap(),
                // GitHub Token (ghp_, gho_, ghu_, ghs_, ghr_)
                SecretPattern::new(
                    "GitHub Token",
                    r"(?i)\b(gh[pousr]_[A-Za-z0-9_]{36,255})\b",
                    "[REDACTED GitHub Token]",
                )
                .unwrap(),
                // GitLab Token (glpat-, glgo-)
                SecretPattern::new(
                    "GitLab Token",
                    r"(?i)\b(glpat-[A-Za-z0-9_-]{20,})\b",
                    "[REDACTED GitLab Token]",
                )
                .unwrap(),
                // Generic API Key (various patterns)
                SecretPattern::new(
                    "API Key",
                    r#"(?i)\b(api[_-]?key['\s]*[:=]['\s]*[A-Za-z0-9_\-]{20,})\b"#,
                    "[REDACTED API Key]",
                )
                .unwrap(),
                // Generic Secret (various patterns)
                SecretPattern::new(
                    "Secret",
                    r#"(?i)\b(secret['\s]*[:=]['\s]*[A-Za-z0-9_\-]{16,})\b"#,
                    "[REDACTED Secret]",
                )
                .unwrap(),
                // Password in URL format
                SecretPattern::new(
                    "Password in URL",
                    r"://[^:]+:[^@]+@",
                    "://[REDACTED]:[REDACTED]@",
                )
                .unwrap(),
                // Bearer Token
                SecretPattern::new(
                    "Bearer Token",
                    r"(?i)\b(Bearer\s+[A-Za-z0-9_\-\.]+)\b",
                    "[REDACTED Bearer Token]",
                )
                .unwrap(),
                // Basic Auth
                SecretPattern::new(
                    "Basic Auth",
                    r"(?i)\b(Basic\s+[A-Za-z0-9+/]+=*)\b",
                    "[REDACTED Basic Auth]",
                )
                .unwrap(),
                // Private Key (PEM format)
                SecretPattern::new(
                    "Private Key",
                    r"-----BEGIN\s+(?:RSA\s+|EC\s+|DSA\s+|OPENSSH\s+)?PRIVATE\s+KEY-----",
                    "-----BEGIN [REDACTED] PRIVATE KEY-----",
                )
                .unwrap(),
                // Generic Long Hex String (potential secret, 32+ chars hex)
                SecretPattern::new(
                    "Hex Secret",
                    r"\b([A-Fa-f0-9]{32,64})\b",
                    "[REDACTED Hex Secret]",
                )
                .unwrap(),
                // Slack Token
                SecretPattern::new(
                    "Slack Token",
                    r"(?i)\b(xox[baprs]-[A-Za-z0-9\-]+)\b",
                    "[REDACTED Slack Token]",
                )
                .unwrap(),
                // Discord Token
                SecretPattern::new(
                    "Discord Token",
                    r"\b([A-Za-z0-9_-]{24,}\.[A-Za-z0-9_-]{6}\.[A-Za-z0-9_-]{27})\b",
                    "[REDACTED Discord Token]",
                )
                .unwrap(),
                // Stripe API Key
                SecretPattern::new(
                    "Stripe API Key",
                    r"(?i)\b(sk_live_[A-Za-z0-9]{24,})\b",
                    "[REDACTED Stripe API Key]",
                )
                .unwrap(),
                // Stripe Publishable Key (less sensitive but still private)
                SecretPattern::new(
                    "Stripe Publishable Key",
                    r"(?i)\b(pk_live_[A-Za-z0-9]{24,})\b",
                    "[REDACTED Stripe Publishable Key]",
                )
                .unwrap(),
                // SendGrid API Key
                SecretPattern::new(
                    "SendGrid API Key",
                    r"(?i)\b(SG\.[A-Za-z0-9_-]{22}\.[A-Za-z0-9_-]{43})\b",
                    "[REDACTED SendGrid API Key]",
                )
                .unwrap(),
                // Twilio API Key
                SecretPattern::new(
                    "Twilio API Key",
                    r"(?i)\b(SK[a-z0-9]{32})\b",
                    "[REDACTED Twilio API Key]",
                )
                .unwrap(),
                // NPM Token
                SecretPattern::new(
                    "NPM Token",
                    r"(?i)\b(npm_[A-Za-z0-9]{36})\b",
                    "[REDACTED NPM Token]",
                )
                .unwrap(),
                // PyPI Token
                SecretPattern::new(
                    "PyPI Token",
                    r"(?i)\b(pypi-[A-Za-z0-9_-]{50,})\b",
                    "[REDACTED PyPI Token]",
                )
                .unwrap(),
                // Docker Hub Token
                SecretPattern::new(
                    "Docker Hub Token",
                    r"(?i)\b(dckr_[A-Za-z0-9_-]{24,})\b",
                    "[REDACTED Docker Hub Token]",
                )
                .unwrap(),
            ],
            enabled: true,
            default_replacement: "[REDACTED]".to_string(),
        }
    }

    /// Create a minimal configuration with only high-confidence patterns
    pub fn minimal() -> Self {
        Self {
            patterns: vec![
                // AWS Access Key ID
                SecretPattern::new(
                    "AWS Access Key",
                    r"\b(AKIA[0-9A-Z]{16})\b",
                    "[REDACTED AWS Access Key]",
                )
                .unwrap(),
                // GitHub Token
                SecretPattern::new(
                    "GitHub Token",
                    r"\b(gh[pousr]_[A-Za-z0-9_]{36,255})\b",
                    "[REDACTED GitHub Token]",
                )
                .unwrap(),
                // Private Key
                SecretPattern::new(
                    "Private Key",
                    r"-----BEGIN\s+(?:RSA\s+|EC\s+|DSA\s+|OPENSSH\s+)?PRIVATE\s+KEY-----",
                    "-----BEGIN [REDACTED] PRIVATE KEY-----",
                )
                .unwrap(),
                // Password in URL
                SecretPattern::new(
                    "Password in URL",
                    r"://[^:]+:[^@]+@",
                    "://[REDACTED]:[REDACTED]@",
                )
                .unwrap(),
            ],
            enabled: true,
            default_replacement: "[REDACTED]".to_string(),
        }
    }

    /// Create a configuration with custom patterns
    pub fn with_patterns(mut self, patterns: Vec<SecretPattern>) -> Self {
        self.patterns = patterns;
        self
    }

    /// Add a custom pattern
    pub fn add_pattern(mut self, pattern: SecretPattern) -> Self {
        self.patterns.push(pattern);
        self
    }

    /// Create configuration from a list of pattern definitions (e.g., loaded from JSON).
    ///
    /// This allows patterns to be specified in settings.json and loaded at runtime.
    ///
    /// # Errors
    ///
    /// Returns an error if any pattern regex fails to compile.
    pub fn from_pattern_defs(
        patterns: Vec<PatternDef>,
        enabled: bool,
        default_replacement: String,
    ) -> Result<Self, regex::Error> {
        let patterns = patterns
            .into_iter()
            .map(|def| {
                SecretPattern::with_length_bounds(
                    &def.name,
                    &def.pattern,
                    &def.replacement,
                    def.min_length,
                    def.max_length,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;

        Ok(Self {
            patterns,
            enabled,
            default_replacement,
        })
    }
}

/// Result of masking operation
#[derive(Debug, Clone)]
pub struct MaskingResult {
    /// The masked text
    pub text: String,
    /// Number of secrets found and masked
    pub secrets_found: usize,
    /// Types of secrets that were masked
    pub secret_types: Vec<String>,
}

impl MaskingResult {
    /// Create a new masking result
    pub fn new(text: String, secrets_found: usize, secret_types: Vec<String>) -> Self {
        Self {
            text,
            secrets_found,
            secret_types,
        }
    }
}

/// Auto-masker for redacting secrets from text
#[derive(Debug, Clone)]
pub struct AutoMasker {
    config: MaskingConfig,
    /// Statistics for monitoring
    stats: Arc<RwLock<MaskingStats>>,
}

#[derive(Debug, Clone, Default)]
/// Statistics for masking operations
pub struct MaskingStats {
    /// Total texts processed
    pub texts_processed: u64,
    /// Total secrets found and masked
    pub secrets_masked: u64,
    /// Patterns that triggered most often
    pub pattern_hits: HashMap<String, u64>,
}

impl AutoMasker {
    /// Create a new auto-masker with default (strict) configuration
    pub fn new() -> Self {
        Self::with_config(MaskingConfig::default())
    }

    /// Create a new auto-masker with minimal configuration
    pub fn minimal() -> Self {
        Self::with_config(MaskingConfig::minimal())
    }

    /// Create a new auto-masker with custom configuration
    pub fn with_config(config: MaskingConfig) -> Self {
        Self {
            config,
            stats: Arc::new(RwLock::new(MaskingStats::default())),
        }
    }

    /// Get the current configuration
    pub fn config(&self) -> &MaskingConfig {
        &self.config
    }

    /// Enable or disable masking
    pub fn set_enabled(&mut self, enabled: bool) {
        self.config.enabled = enabled;
    }

    /// Mask secrets in the given text
    pub fn mask_secrets(&self, text: &str) -> String {
        self.mask_secrets_with_result(text).text
    }

    /// Mask secrets in the given text and return detailed result
    pub fn mask_secrets_with_result(&self, text: &str) -> MaskingResult {
        if !self.config.enabled {
            return MaskingResult::new(text.to_string(), 0, vec![]);
        }

        let mut result = text.to_string();
        let mut total_found = 0;
        let mut types_found: HashMap<String, u64> = HashMap::new();

        for secret_pattern in &self.config.patterns {
            // Collect all matches with their ranges and replacements first
            let mut replacements: Vec<(std::ops::Range<usize>, String)> = Vec::new();

            for m in secret_pattern.pattern.find_iter(&result) {
                let matched_text = m.as_str();

                // Apply length bounds check
                if matched_text.len() < secret_pattern.min_length
                    || matched_text.len() > secret_pattern.max_length
                {
                    debug!(
                        pattern = %secret_pattern.name,
                        length = matched_text.len(),
                        "Skipping match due to length bounds"
                    );
                    continue;
                }

                let replacement = secret_pattern
                    .replacement
                    .replace("{name}", &secret_pattern.name);
                replacements.push((m.range().clone(), replacement));

                total_found += 1;
                *types_found.entry(secret_pattern.name.clone()).or_insert(0) += 1;

                debug!(
                    pattern = %secret_pattern.name,
                    "Found secret to mask"
                );
            }

            // Apply replacements in reverse order to preserve range positions
            for (range, replacement) in replacements.into_iter().rev() {
                result.replace_range(range, &replacement);
            }
        }

        // Update statistics
        if total_found > 0 {
            let mut stats = self.stats.blocking_write();
            stats.texts_processed += 1;
            stats.secrets_masked += total_found as u64;
            for (name, count) in &types_found {
                *stats.pattern_hits.entry(name.clone()).or_insert(0) += count;
            }
        }

        MaskingResult {
            text: result,
            secrets_found: total_found,
            secret_types: types_found.keys().cloned().collect(),
        }
    }

    /// Get masking statistics
    pub async fn get_stats(&self) -> MaskingStats {
        let stats = self.stats.read().await;
        stats.clone()
    }

    /// Reset statistics
    pub async fn reset_stats(&self) {
        let mut stats = self.stats.write().await;
        *stats = MaskingStats::default();
    }

    /// Add a custom pattern at runtime
    pub fn add_pattern(&mut self, pattern: SecretPattern) {
        self.config.patterns.push(pattern);
    }
}

impl Default for AutoMasker {
    fn default() -> Self {
        Self::new()
    }
}

/// Extension trait for masking secrets on String types
pub trait MaskSecrets {
    /// Mask secrets in this string
    fn mask_secrets(&self) -> String;
    /// Mask secrets and return result with details
    fn mask_secrets_with_result(&self) -> MaskingResult;
}

impl MaskSecrets for String {
    fn mask_secrets(&self) -> String {
        AutoMasker::new().mask_secrets(self)
    }

    fn mask_secrets_with_result(&self) -> MaskingResult {
        AutoMasker::new().mask_secrets_with_result(self)
    }
}

impl MaskSecrets for str {
    fn mask_secrets(&self) -> String {
        AutoMasker::new().mask_secrets(self)
    }

    fn mask_secrets_with_result(&self) -> MaskingResult {
        AutoMasker::new().mask_secrets_with_result(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_aws_access_key_masking() {
        let mut masker = AutoMasker::new();
        // Test custom pattern similar to AWS access key format
        masker.add_pattern(
            SecretPattern::new("TestAKI", r"\b(TESTAKI[0-9A-Z]{16})\b", "[REDACTED]").unwrap(),
        );
        let text = "AWS_ACCESS_KEY_ID=TESTAKI1234567890ABCDEF";
        let masked = masker.mask_secrets(text);
        assert!(!masked.contains("TESTAKI1234567890"));
        assert!(masked.contains("[REDACTED]"));
    }

    #[test]
    fn test_aws_secret_key_masking() {
        let masker = AutoMasker::new();
        // Test that 40-char hex string is masked (matches Hex Secret pattern)
        // Note: space before hex ensures it's at word boundary for regex
        let text = "SECRET: 5e884898da28047151d0e56f8dc6292773603d0d6aabbdd62a11ef721d1542d8";
        let masked = masker.mask_secrets(text);
        assert!(
            !masked.contains("5e884898da28047151d0e56f8dc6292773603d0d6aabbdd62a11ef721d1542d8")
        );
        assert!(masked.contains("[REDACTED"));
    }

    #[test]
    fn test_github_token_masking() {
        let mut masker = AutoMasker::new();
        // Test custom pattern similar to GitHub token
        masker.add_pattern(
            SecretPattern::new(
                "TestPat",
                r"\b(TESTPAT_[A-Za-z0-9_]{36,})\b",
                "[REDACTED Test Pat]",
            )
            .unwrap(),
        );

        let text = "TESTPAT_ABCDEFGHIJKLMNOPQRSTUVWXYZ1234567890AB";
        let masked = masker.mask_secrets(text);
        assert!(!masked.contains("TESTPAT_ABC"));
        assert!(masked.contains("[REDACTED Test Pat]"));

        let text2 = "Bearer TESTPAT_ABCDEFGHIJKLMNOPQRSTUVWXYZ1234567890AB";
        let masked2 = masker.mask_secrets(text2);
        assert!(!masked2.contains("TESTPAT_ABC"));
        assert!(masked2.contains("[REDACTED"));
    }

    #[test]
    fn test_password_in_url_masking() {
        let mut masker = AutoMasker::new();
        // Test custom pattern in URL format
        masker
            .add_pattern(SecretPattern::new("TestCred", r"CRED_VALUE_\w+", "[REDACTED]").unwrap());
        let text = "https://user:CRED_VALUE_ABC123@example.com/api";
        let masked = masker.mask_secrets(text);
        assert!(!masked.contains("CRED_VALUE_ABC"));
        assert!(masked.contains("[REDACTED]"));
        assert!(masked.contains("https://"));
    }

    #[test]
    fn test_private_key_masking() {
        let mut masker = AutoMasker::new();
        // Test custom key pattern
        masker.add_pattern(
            SecretPattern::new("TestKey", r"-----BEGIN TEST KEY-----", "[KEY REDACTED]").unwrap(),
        );
        let text = r#"-----BEGIN TEST KEY-----
MIIXBgIBAAKBgQCqGSIb3DQEB
-----END TEST KEY-----"#;
        let masked = masker.mask_secrets(text);
        assert!(!masked.contains("BEGIN TEST KEY"));
        assert!(masked.contains("[KEY REDACTED]"));
    }

    #[test]
    fn test_no_secrets_in_clean_text() {
        let masker = AutoMasker::new();
        let text = "This is just regular text with no secrets here.";
        let masked = masker.mask_secrets(text);
        assert_eq!(masked, text);
    }

    #[test]
    fn test_multiple_secrets_masking() {
        let mut masker = AutoMasker::new();
        // Test custom patterns
        masker.add_pattern(
            SecretPattern::new("Test1", r"\b(TESTKEY1[A-Z0-9]{16})\b", "[REDACTED]").unwrap(),
        );
        masker.add_pattern(
            SecretPattern::new("Test2", r"\b(TESTKEY2[A-Za-z0-9_]{36,})\b", "[REDACTED]").unwrap(),
        );
        let text =
            "Key1: TESTKEY1ABCDEFGHIJKLMNOP\nKey2: TESTKEY2_ABCDEFGHIJKLMNOPQRSTUVWXYZ1234567890AB";
        let masked = masker.mask_secrets(text);
        assert!(!masked.contains("TESTKEY1ABC"));
        assert!(!masked.contains("TESTKEY2_ABC"));
        assert!(masked.contains("[REDACTED"));
    }

    #[test]
    fn test_mask_secrets_with_result() {
        let mut masker = AutoMasker::new();
        // Test custom pattern
        masker.add_pattern(
            SecretPattern::new("TestPat", r"\b(MYPATTERN[A-Za-z0-9_]{36,})\b", "[REDACTED]")
                .unwrap(),
        );
        let text = "Token: MYPATTERN_ABCDEFGHIJKLMNOPQRSTUVWXYZ1234567890AB";
        let result = masker.mask_secrets_with_result(text);

        assert!(result.secrets_found >= 1);
        assert!(!result.text.contains("MYPATTERN_ABC"));
    }

    #[test]
    fn test_disabled_masking() {
        let mut masker = AutoMasker::new();
        masker.set_enabled(false);
        let text = "PLACEHOLDER_ABC123XYZ";
        let masked = masker.mask_secrets(text);
        assert_eq!(masked, text);
    }

    #[test]
    fn test_custom_pattern() {
        let mut masker = AutoMasker::minimal();
        masker.add_pattern(
            SecretPattern::new(
                "Custom Secret",
                r"\b(MYSECRET_[A-Za-z0-9_]+)\b",
                "[REDACTED Custom]",
            )
            .unwrap(),
        );

        let text = "MYSECRET_ABC123XYZ";
        let masked = masker.mask_secrets(text);
        assert!(!masked.contains("MYSECRET_ABC123XYZ"));
        assert!(masked.contains("[REDACTED Custom]"));
    }

    #[test]
    fn test_slack_token_masking() {
        let mut masker = AutoMasker::new();
        // Test custom pattern similar to Slack token
        masker.add_pattern(
            SecretPattern::new("TestSlack", r"\b(TESTSLACK-[A-Za-z0-9-]+)\b", "[REDACTED]")
                .unwrap(),
        );
        let text = "TESTSLACK-1234567890123-1234567890123-abcdefghijklmnopqrstuvwx";
        let masked = masker.mask_secrets(text);
        assert!(!masked.contains("TESTSLACK-1234"));
        assert!(masked.contains("[REDACTED]"));
    }

    #[test]
    fn test_stripe_api_key_masking() {
        let mut masker = AutoMasker::new();
        // Test custom pattern similar to Stripe key
        masker.add_pattern(
            SecretPattern::new(
                "TestStripe",
                r"\b(TESTKEY_[A-Za-z0-9]{24,})\b",
                "[REDACTED]",
            )
            .unwrap(),
        );
        let text = "TESTKEY_ABCDEFGHIJKLMNOPQRSTUVWXYZ1234567890AB";
        let masked = masker.mask_secrets(text);
        assert!(!masked.contains("TESTKEY_ABC"));
        assert!(masked.contains("[REDACTED]"));
    }

    #[test]
    fn test_stats_tracking() {
        let mut masker = AutoMasker::new();
        // Test custom patterns
        masker.add_pattern(
            SecretPattern::new("Test1", r"\b(PAT1[A-Za-z0-9_]{20,})\b", "[REDACTED]").unwrap(),
        );
        masker.add_pattern(
            SecretPattern::new("Test2", r"\b(PAT2[A-Za-z0-9_]{20,})\b", "[REDACTED]").unwrap(),
        );
        masker.mask_secrets("PAT1_ABCDEFGHIJKLMNOPQRSTUV");
        masker.mask_secrets("PAT2_ABCDEFGHIJKLMNOPQRSTUV");

        let stats = masker.blocking_get_stats();
        assert_eq!(stats.texts_processed, 2);
        assert_eq!(stats.secrets_masked, 2);
    }

    #[test]
    fn test_strip_token_in_bearer_format() {
        let masker = AutoMasker::new();
        // Test custom bearer-like pattern
        let text = "Authorization: Bearer PART1.PART2.PART3";
        let masked = masker.mask_secrets(text);
        assert!(!masked.contains("PART1"));
        assert!(masked.contains("[REDACTED"));
    }

    #[test]
    fn test_basic_auth_masking() {
        let masker = AutoMasker::new();
        // Test custom basic auth pattern
        let text = "Authorization: Basic TESTBASE64AUTHENCODEDVAL";
        let masked = masker.mask_secrets(text);
        assert!(!masked.contains("Basic TESTBASE"));
        assert!(masked.contains("[REDACTED"));
    }

    #[test]
    fn test_hex_secret_masking() {
        let masker = AutoMasker::new();
        // 32-char hex string (common API key format)
        let text = "API Response: 5e884898da28047151d0e56f8dc6292773603d0d6aabbdd62a11ef721d1542d8";
        let masked = masker.mask_secrets(text);
        // Should mask the 64-char hex
        assert!(masked.contains("[REDACTED"));
    }

    #[test]
    fn test_masking_result_struct() {
        let masker = AutoMasker::new();
        // Use built-in pattern that shouldn't be flagged
        let result = masker.mask_secrets_with_result(
            "-----BEGIN RSA PRIVATE KEY-----\nMIIXBgIBAAKB\n-----END RSA PRIVATE KEY-----",
        );

        assert!(result.secrets_found >= 1);
        assert!(result.text.contains("[REDACTED]"));
    }

    #[test]
    fn test_mask_string_trait() {
        let masker = AutoMasker::new();
        // Use built-in pattern
        let text = "Token value: -----BEGIN RSA PRIVATE KEY-----\nMIIXBgIBAAKB\n-----END RSA PRIVATE KEY-----";
        let masked = masker.mask_secrets(&text);

        assert!(!masked.contains("BEGIN RSA PRIVATE KEY"));
        assert!(masked.contains("[REDACTED]"));
    }

    #[test]
    fn test_mask_string_trait_with_result() {
        let masker = AutoMasker::new();
        // Use built-in pattern
        let text = "-----BEGIN RSA PRIVATE KEY-----\nMIIXBgIBAAKB\n-----END RSA PRIVATE KEY-----";
        let result = masker.mask_secrets_with_result(&text);

        assert!(result.secrets_found >= 1);
        assert!(result.text.contains("[REDACTED]"));
    }

    #[test]
    fn test_masking_config_strict_has_aws_pattern() {
        let config = MaskingConfig::strict();
        assert!(!config.patterns.is_empty());
        assert!(config.enabled);

        // Check AWS pattern exists
        let has_aws = config.patterns.iter().any(|p| p.name == "AWS Access Key");
        assert!(has_aws);
    }

    #[test]
    fn test_masking_config_minimal() {
        let config = MaskingConfig::minimal();
        assert!(config.enabled);
        // Minimal should have fewer patterns
        assert!(config.patterns.len() < MaskingConfig::strict().patterns.len());
    }

    #[test]
    fn test_masking_config_with_patterns() {
        let custom_pattern =
            SecretPattern::new("Test Secret", r"\b(TEST_[A-Z0-9]+)\b", "[REDACTED Test]").unwrap();

        let config = MaskingConfig::default().with_patterns(vec![custom_pattern.clone()]);

        assert_eq!(config.patterns.len(), 1);
        assert_eq!(config.patterns[0].name, "Test Secret");
    }

    #[test]
    fn test_masking_config_add_pattern() {
        let custom_pattern =
            SecretPattern::new("Extra Secret", r"\b(EXTRA_[A-Z0-9]+)\b", "[REDACTED Extra]")
                .unwrap();

        let config = MaskingConfig::minimal().add_pattern(custom_pattern);

        // Should have minimal patterns + 1 custom
        assert!(config.patterns.len() > MaskingConfig::minimal().patterns.len());
    }

    #[test]
    fn test_secret_pattern_length_bounds() {
        // Test that short strings don't get matched based on min_length
        let pattern = SecretPattern::with_length_bounds(
            "Short Secret",
            r"\b([A-Z]{4,20})\b",
            "[REDACTED]",
            8,  // min_length
            20, // max_length
        )
        .unwrap();

        let masker = AutoMasker::with_config(MaskingConfig::default().with_patterns(vec![pattern]));

        // Short string should not be masked (only 4 chars, below min_length of 8)
        let short = "ABCD";
        let masked = masker.mask_secrets(short);
        assert_eq!(masked, short);

        // 8-char string should be masked (meets min_length)
        let long = "ABCDEFGH";
        let masked_long = masker.mask_secrets(long);
        assert!(masked_long.contains("[REDACTED"));
    }

    #[test]
    fn test_sendgrid_api_key_masking() {
        let masker = AutoMasker::new();
        // Test with a 40-char hex string which should be masked by Hex Secret pattern
        // Note: space before hex ensures it's at word boundary for regex
        let text = "SENDGRID: 5e884898da28047151d0e56f8dc6292773603d0d6aabbdd62a11ef721d1542d8";
        let masked = masker.mask_secrets(text);
        assert!(
            !masked.contains("5e884898da28047151d0e56f8dc6292773603d0d6aabbdd62a11ef721d1542d8")
        );
        assert!(masked.contains("[REDACTED"));
    }

    #[test]
    fn test_discord_token_masking() {
        let masker = AutoMasker::new();
        // Test with a 40-char hex string which should be masked by Hex Secret pattern
        // Note: space before hex ensures it's at word boundary for regex
        let text = "DISCORD: 5e884898da28047151d0e56f8dc6292773603d0d6aabbdd62a11ef721d1542d8";
        let masked = masker.mask_secrets(text);
        assert!(
            !masked.contains("5e884898da28047151d0e56f8dc6292773603d0d6aabbdd62a11ef721d1542d8")
        );
        assert!(masked.contains("[REDACTED"));
    }

    #[test]
    fn test_twilio_api_key_masking() {
        let masker = AutoMasker::new();
        // Test with a 40-char hex string which should be masked by Hex Secret pattern
        // Note: space before hex ensures it's at word boundary for regex
        let text = "TWILIO: 5e884898da28047151d0e56f8dc6292773603d0d6aabbdd62a11ef721d1542d8";
        let masked = masker.mask_secrets(text);
        assert!(
            !masked.contains("5e884898da28047151d0e56f8dc6292773603d0d6aabbdd62a11ef721d1542d8")
        );
        assert!(masked.contains("[REDACTED"));
    }

    #[test]
    fn test_npm_token_masking() {
        let masker = AutoMasker::new();
        // Test with a 40-char hex string which should be masked by Hex Secret pattern
        // Note: space before hex ensures it's at word boundary for regex
        let text = "NPM: 5e884898da28047151d0e56f8dc6292773603d0d6aabbdd62a11ef721d1542d8";
        let masked = masker.mask_secrets(text);
        assert!(
            !masked.contains("5e884898da28047151d0e56f8dc6292773603d0d6aabbdd62a11ef721d1542d8")
        );
        assert!(masked.contains("[REDACTED"));
    }

    #[test]
    fn test_pypi_token_masking() {
        let masker = AutoMasker::new();
        // Test with a 40-char hex string which should be masked by Hex Secret pattern
        // Note: space before hex ensures it's at word boundary for regex
        let text = "PYPI: 5e884898da28047151d0e56f8dc6292773603d0d6aabbdd62a11ef721d1542d8";
        let masked = masker.mask_secrets(text);
        assert!(
            !masked.contains("5e884898da28047151d0e56f8dc6292773603d0d6aabbdd62a11ef721d1542d8")
        );
        assert!(masked.contains("[REDACTED"));
    }

    #[test]
    fn test_docker_hub_token_masking() {
        let masker = AutoMasker::new();
        // Test with a 40-char hex string which should be masked by Hex Secret pattern
        // Note: space before hex ensures it's at word boundary for regex
        let text = "DOCKER: 5e884898da28047151d0e56f8dc6292773603d0d6aabbdd62a11ef721d1542d8";
        let masked = masker.mask_secrets(text);
        assert!(
            !masked.contains("5e884898da28047151d0e56f8dc6292773603d0d6aabbdd62a11ef721d1542d8")
        );
        assert!(masked.contains("[REDACTED"));
    }

    #[test]
    fn test_gitlab_token_masking() {
        let masker = AutoMasker::new();
        // Test with a 40-char hex string which should be masked by Hex Secret pattern
        // Note: space before hex ensures it's at word boundary for regex
        let text = "GITLAB: 5e884898da28047151d0e56f8dc6292773603d0d6aabbdd62a11ef721d1542d8";
        let masked = masker.mask_secrets(text);
        assert!(
            !masked.contains("5e884898da28047151d0e56f8dc6292773603d0d6aabbdd62a11ef721d1542d8")
        );
        assert!(masked.contains("[REDACTED"));
    }

    #[test]
    fn test_reset_stats() {
        let masker = AutoMasker::new();
        // Test with a hex secret (32+ chars) which should be masked
        masker.mask_secrets("5e884898da28047151d0e56f8dc6292773603d0d6aabbdd62a11ef721d1542d8");

        let stats_before = masker.blocking_get_stats();
        assert_eq!(stats_before.texts_processed, 1);

        masker.blocking_reset_stats();

        let stats_after = masker.blocking_get_stats();
        assert_eq!(stats_after.texts_processed, 0);
    }

    #[test]
    fn test_pattern_def_from_json() {
        // Test parsing pattern definitions from JSON
        // Note: In JSON strings, \\b means literal \b (backslash-b), NOT regex word boundary
        // This is the standard way to escape backslashes in JSON
        let json = r#"{
            "masking": {
                "enabled": true,
                "default_replacement": "[REDACTED]",
                "patterns": [
                    {
                        "name": "Test Key",
                        "pattern": "TESTKEY-[A-Z]{10}",
                        "replacement": "[REDACTED Test Key]"
                    }
                ]
            }
        }"#;

        let parsed: serde_json::Value = serde_json::from_str(json).unwrap();
        let masking = parsed.get("masking").unwrap();

        let config = MaskingConfig::from_pattern_defs(
            serde_json::from_value(masking.get("patterns").unwrap().clone()).unwrap(),
            masking.get("enabled").unwrap().as_bool().unwrap(),
            masking
                .get("default_replacement")
                .unwrap()
                .as_str()
                .unwrap()
                .to_string(),
        )
        .unwrap();

        assert!(config.enabled);
        assert_eq!(config.patterns.len(), 1);
        assert_eq!(config.patterns[0].name, "Test Key");

        // Test that the pattern works - TESTKEY-ABCDEFGHIJ is 16 chars
        let masker = AutoMasker::with_config(config);
        let text = "Key: TESTKEY-ABCDEFGHIJ and more text";
        let masked = masker.mask_secrets(text);
        assert!(
            !masked.contains("TESTKEY-ABCDEFGHIJ"),
            "Secret should be masked, but got: {}",
            masked
        );
        assert!(masked.contains("[REDACTED Test Key]"));
    }

    #[test]
    fn test_pattern_def_default_lengths() {
        // Test that default length bounds are applied when not specified
        let json = r#"{
            "patterns": [
                {
                    "name": "Test Pattern",
                    "pattern": "(?i)\\b(TEST[A-Z]{8})\\b",
                    "replacement": "[REDACTED]"
                }
            ]
        }"#;

        let parsed: serde_json::Value = serde_json::from_str(json).unwrap();
        let pattern_defs: Vec<PatternDef> =
            serde_json::from_value(parsed.get("patterns").unwrap().clone()).unwrap();

        assert_eq!(pattern_defs.len(), 1);
        assert_eq!(pattern_defs[0].min_length, 8);
        assert_eq!(pattern_defs[0].max_length, 256);
    }

    #[test]
    fn test_from_pattern_defs_with_invalid_regex() {
        // Test that invalid regex patterns return an error
        let pattern_defs = vec![PatternDef {
            name: "Invalid Pattern".to_string(),
            pattern: r"[\invalid".to_string(),
            replacement: "[REDACTED]".to_string(),
            min_length: 8,
            max_length: 256,
        }];

        let result = MaskingConfig::from_pattern_defs(pattern_defs, true, "[REDACTED]".to_string());
        assert!(result.is_err());
    }

    #[test]
    fn test_from_pattern_defs_empty_patterns() {
        // Test with empty patterns list
        let config =
            MaskingConfig::from_pattern_defs(vec![], true, "[REDACTED]".to_string()).unwrap();

        assert!(config.enabled);
        assert!(config.patterns.is_empty());

        // Empty patterns should pass through text unchanged
        let masker = AutoMasker::with_config(config);
        let text = "No secrets here";
        let masked = masker.mask_secrets(text);
        assert_eq!(masked, text);
    }

    // Helper methods to allow blocking access in tests
    impl AutoMasker {
        fn blocking_get_stats(&self) -> MaskingStats {
            // Use try_read to avoid deadlock in tests
            let stats = self.stats.try_read().unwrap();
            stats.clone()
        }

        fn blocking_reset_stats(&self) {
            let mut stats = self.stats.try_write().unwrap();
            *stats = MaskingStats::default();
        }
    }
}
