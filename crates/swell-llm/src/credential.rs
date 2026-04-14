//! Credential format detection for LLM API keys.
//!
//! This module provides functions to detect and validate API key formats
//! to prevent misconfiguration errors when using the wrong backend with
//! a key intended for another provider.
//!
//! # Key Format Detection
//!
//! - **OpenAI keys**: Start with `sk-` (e.g., `sk-...`, `sk-proj-...`)
//! - **Anthropic keys**: Start with `sk-ant-` (e.g., `sk-ant-api03-...`)
//!
//! # Error Messages
//!
//! When a key format mismatch is detected, an enriched error is returned
//! that identifies the detected format and suggests the correct backend.

use crate::SwellError;

/// Detects if an API key appears to be an OpenAI-format key.
///
/// OpenAI keys typically start with `sk-` and may be followed by
/// `-proj-` for project keys or `-org-` for organization keys.
///
/// Returns `true` if the key looks like an OpenAI key, `false` otherwise.
pub fn is_openai_key(key: &str) -> bool {
    let trimmed = key.trim();
    // OpenAI keys start with "sk-" but not "sk-ant-"
    trimmed.starts_with("sk-") && !trimmed.starts_with("sk-ant-")
}

/// Detects if an API key appears to be an Anthropic-format key.
///
/// Anthropic keys start with `sk-ant-api03-` or similar patterns
/// like `sk-ant-api` followed by additional segments.
///
/// Returns `true` if the key looks like an Anthropic key, `false` otherwise.
pub fn is_anthropic_key(key: &str) -> bool {
    let trimmed = key.trim();
    // Anthropic keys start with "sk-ant-"
    trimmed.starts_with("sk-ant-")
}

/// Validates that an API key is not in OpenAI format when creating an Anthropic backend.
///
/// When a key starting with `sk-` (but not `sk-ant-`) is passed to the Anthropic backend,
/// it indicates a likely misconfiguration where an OpenAI key is being used with
/// the Anthropic API.
///
/// # Arguments
///
/// * `key` - The API key to validate
///
/// # Returns
///
/// Returns `Ok(())` if the key appears valid for Anthropic, or an `Err(SwellError)`
/// with a descriptive message if the key appears to be an OpenAI key.
pub fn validate_anthropic_key(key: &str) -> Result<(), SwellError> {
    if is_openai_key(key) {
        Err(SwellError::LlmError(format!(
            "Credential format mismatch: The API key appears to be an OpenAI-format key (starts with 'sk-' but not 'sk-ant-'). \
             Anthropic API keys typically start with 'sk-ant-api03-' or similar. \
             Please verify you are using the correct key for the Anthropic backend. \
             Detected key prefix: '{}'",
            get_key_prefix(key)
        )))
    } else {
        Ok(())
    }
}

/// Validates that an API key is not in Anthropic format when creating an OpenAI backend.
///
/// When a key starting with `sk-ant-` is passed to the OpenAI backend,
/// it indicates a likely misconfiguration where an Anthropic key is being used with
/// the OpenAI API.
///
/// # Arguments
///
/// * `key` - The API key to validate
///
/// # Returns
///
/// Returns `Ok(())` if the key appears valid for OpenAI, or an `Err(SwellError)`
/// with a descriptive message if the key appears to be an Anthropic key.
pub fn validate_openai_key(key: &str) -> Result<(), SwellError> {
    if is_anthropic_key(key) {
        Err(SwellError::LlmError(format!(
            "Credential format mismatch: The API key appears to be an Anthropic-format key (starts with 'sk-ant-'). \
             OpenAI API keys typically start with 'sk-' (e.g., 'sk-...', 'sk-proj-...', 'sk-org-...'). \
             Please verify you are using the correct key for the OpenAI backend. \
             Detected key prefix: '{}'",
            get_key_prefix(key)
        )))
    } else {
        Ok(())
    }
}

/// Returns the first 10 characters of a key for use in error messages.
/// This provides enough context to identify the key type without exposing the full key.
fn get_key_prefix(key: &str) -> &str {
    &key[..key.len().min(10)]
}

#[cfg(test)]
mod tests {
    use super::*;

    // ===========================================================================
    // OpenAI Key Detection Tests
    // ===========================================================================

    #[test]
    fn test_is_openai_key_standard() {
        assert!(is_openai_key("sk-1234567890abcdef"));
        assert!(is_openai_key("sk-"));
        assert!(is_openai_key("sk-proj-1234567890abcdef"));
        assert!(is_openai_key("sk-org-1234567890abcdef"));
    }

    #[test]
    fn test_is_openai_key_with_whitespace() {
        assert!(is_openai_key("  sk-1234567890abcdef"));
        assert!(is_openai_key("sk-1234567890abcdef  "));
        assert!(is_openai_key("  sk-proj-1234567890abcdef  "));
    }

    #[test]
    fn test_is_openai_key_false_for_anthropic() {
        // Anthropic keys should NOT be detected as OpenAI keys
        assert!(!is_openai_key("sk-ant-api03-1234567890abcdef"));
        assert!(!is_openai_key("sk-ant-1234567890abcdef"));
        assert!(!is_openai_key("sk-ant-api-1234567890abcdef"));
    }

    #[test]
    fn test_is_openai_key_false_for_non_sk_keys() {
        assert!(!is_openai_key("anthropic-api-key-12345"));
        assert!(!is_openai_key("1234567890abcdef"));
        assert!(!is_openai_key(""));
        assert!(!is_openai_key("claude-api-key-12345"));
    }

    // ===========================================================================
    // Anthropic Key Detection Tests
    // ===========================================================================

    #[test]
    fn test_is_anthropic_key_standard() {
        assert!(is_anthropic_key("sk-ant-api03-1234567890abcdef"));
        assert!(is_anthropic_key("sk-ant-api03-abcd"));
        assert!(is_anthropic_key("sk-ant-1234567890abcdef"));
        assert!(is_anthropic_key("sk-ant-api-1234567890abcdef"));
    }

    #[test]
    fn test_is_anthropic_key_with_whitespace() {
        assert!(is_anthropic_key("  sk-ant-api03-1234567890abcdef"));
        assert!(is_anthropic_key("sk-ant-api03-1234567890abcdef  "));
    }

    #[test]
    fn test_is_anthropic_key_false_for_openai() {
        // OpenAI keys should NOT be detected as Anthropic keys
        assert!(!is_anthropic_key("sk-1234567890abcdef"));
        assert!(!is_anthropic_key("sk-proj-1234567890abcdef"));
        assert!(!is_anthropic_key("sk-org-1234567890abcdef"));
    }

    #[test]
    fn test_is_anthropic_key_false_for_non_ant_keys() {
        assert!(!is_anthropic_key("openai-key-12345"));
        assert!(!is_anthropic_key("1234567890abcdef"));
        assert!(!is_anthropic_key(""));
        assert!(!is_anthropic_key("claude-some-key"));
    }

    // ===========================================================================
    // Anthropic Key Validation Tests
    // ===========================================================================

    #[test]
    fn test_validate_anthropic_key_accepts_valid_keys() {
        // Valid Anthropic keys
        assert!(validate_anthropic_key("sk-ant-api03-1234567890abcdef").is_ok());
        assert!(validate_anthropic_key("sk-ant-1234567890abcdef").is_ok());
        assert!(validate_anthropic_key("sk-ant-api-1234567890abcdef").is_ok());

        // Non-sk keys (could be other valid formats)
        assert!(validate_anthropic_key("1234567890abcdef").is_ok());
        assert!(validate_anthropic_key("").is_ok());
    }

    #[test]
    fn test_validate_anthropic_key_rejects_openai_keys() {
        // Standard OpenAI key
        let result = validate_anthropic_key("sk-1234567890abcdef");
        assert!(result.is_err());
        let error = result.unwrap_err();
        let error_msg = error.to_string();
        assert!(error_msg.contains("OpenAI-format key"));
        assert!(error_msg.contains("sk-ant-"));

        // Project key
        let result = validate_anthropic_key("sk-proj-1234567890abcdef");
        assert!(result.is_err());
        let error = result.unwrap_err();
        let error_msg = error.to_string();
        assert!(error_msg.contains("OpenAI-format key"));

        // Organization key
        let result = validate_anthropic_key("sk-org-1234567890abcdef");
        assert!(result.is_err());
    }

    // ===========================================================================
    // OpenAI Key Validation Tests
    // ===========================================================================

    #[test]
    fn test_validate_openai_key_accepts_valid_keys() {
        // Valid OpenAI keys
        assert!(validate_openai_key("sk-1234567890abcdef").is_ok());
        assert!(validate_openai_key("sk-proj-1234567890abcdef").is_ok());
        assert!(validate_openai_key("sk-org-1234567890abcdef").is_ok());

        // Non-sk keys
        assert!(validate_openai_key("1234567890abcdef").is_ok());
        assert!(validate_openai_key("").is_ok());
    }

    #[test]
    fn test_validate_openai_key_rejects_anthropic_keys() {
        // Anthropic keys should be rejected
        let result = validate_openai_key("sk-ant-api03-1234567890abcdef");
        assert!(result.is_err());
        let error = result.unwrap_err();
        let error_msg = error.to_string();
        assert!(error_msg.contains("Anthropic-format key"));
        assert!(error_msg.contains("sk-"));

        let result = validate_openai_key("sk-ant-1234567890abcdef");
        assert!(result.is_err());
        let error = result.unwrap_err();
        let error_msg = error.to_string();
        assert!(error_msg.contains("Anthropic-format key"));
    }

    // ===========================================================================
    // Key Prefix Extraction Tests
    // ===========================================================================

    #[test]
    fn test_get_key_prefix_short_key() {
        assert_eq!(get_key_prefix("sk-123"), "sk-123");
    }

    #[test]
    fn test_get_key_prefix_long_key() {
        assert_eq!(get_key_prefix("sk-1234567890abcdefghij"), "sk-1234567");
    }

    #[test]
    fn test_get_key_prefix_exactly_10_chars() {
        assert_eq!(get_key_prefix("sk-1234567"), "sk-1234567");
    }
}
