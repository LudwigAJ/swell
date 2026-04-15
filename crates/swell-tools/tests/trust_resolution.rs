//! Trust Resolution Integration Tests
//!
//! These tests verify that `TrustPolicy` and `TrustResolver` implement
//! allow-list based tool trust enforcement. Untrusted tools are blocked
//! with a clear error message explaining why access was denied.
//!
//! Reference: VAL-OBS-009

#[cfg(test)]
mod trust_resolution_tests {

    // ===================================================================
    // TrustPolicy construction and allow-list tests
    // ===================================================================

    #[test]
    fn test_trust_policy_new_starts_empty() {
        let policy = swell_tools::TrustPolicy::new();
        assert!(!policy.is_trusted("file_read"));
        assert!(!policy.is_trusted("git_status"));
        assert!(!policy.is_trusted("shell"));
    }

    #[test]
    fn test_trust_policy_allow_adds_single_tool() {
        let policy = swell_tools::TrustPolicy::new().allow("file_read");
        assert!(policy.is_trusted("file_read"));
        assert!(!policy.is_trusted("git_status"));
    }

    #[test]
    fn test_trust_policy_allow_is_case_insensitive() {
        let policy = swell_tools::TrustPolicy::new().allow("FileRead");
        assert!(policy.is_trusted("fileread"));
        assert!(policy.is_trusted("FILEREAD"));
        assert!(policy.is_trusted("FileRead"));
    }

    #[test]
    fn test_trust_policy_allow_many_adds_multiple_tools() {
        let policy = swell_tools::TrustPolicy::new()
            .allow_many(["file_read", "git_status", "shell"]);
        assert!(policy.is_trusted("file_read"));
        assert!(policy.is_trusted("git_status"));
        assert!(policy.is_trusted("shell"));
        assert!(!policy.is_trusted("http_request"));
    }

    #[test]
    fn test_trust_policy_trust_all_bypasses_allow_list() {
        let policy = swell_tools::TrustPolicy::trust_all();
        assert!(policy.is_trust_all());
        assert!(policy.is_trusted("any_arbitrary_tool"));
        assert!(policy.is_trusted("dangerous_operation"));
    }

    #[test]
    fn test_trust_policy_trusted_tools_returns_lowercase() {
        let policy = swell_tools::TrustPolicy::new()
            .allow("FILE_READ")
            .allow("GitStatus");
        let trusted = policy.trusted_tools();
        assert!(trusted.contains("file_read"));
        assert!(trusted.contains("gitstatus"));
    }

    // ===================================================================
    // TrustResolver check() tests
    // ===================================================================

    #[test]
    fn test_trust_resolver_check_trusted_for_allowlisted_tool() {
        let policy = swell_tools::TrustPolicy::new()
            .allow("file_read")
            .allow("git_status");
        let resolver = swell_tools::TrustResolver::new(policy);
        assert_eq!(resolver.check("file_read"), swell_tools::TrustStatus::Trusted);
        assert_eq!(resolver.check("git_status"), swell_tools::TrustStatus::Trusted);
    }

    #[test]
    fn test_trust_resolver_check_untrusted_for_unknown_tool() {
        let policy = swell_tools::TrustPolicy::new().allow("file_read");
        let resolver = swell_tools::TrustResolver::new(policy);
        assert_eq!(
            resolver.check("unknown_tool"),
            swell_tools::TrustStatus::Untrusted
        );
    }

    #[test]
    fn test_trust_resolver_check_untrusted_for_empty_policy() {
        let policy = swell_tools::TrustPolicy::new();
        let resolver = swell_tools::TrustResolver::new(policy);
        assert_eq!(
            resolver.check("file_read"),
            swell_tools::TrustStatus::Untrusted
        );
        assert_eq!(
            resolver.check("git_status"),
            swell_tools::TrustStatus::Untrusted
        );
    }

    #[test]
    fn test_trust_resolver_check_trusted_with_trust_all_policy() {
        let policy = swell_tools::TrustPolicy::trust_all();
        let resolver = swell_tools::TrustResolver::new(policy);
        assert_eq!(
            resolver.check("any_tool"),
            swell_tools::TrustStatus::Trusted
        );
        assert_eq!(
            resolver.check("another_tool"),
            swell_tools::TrustStatus::Trusted
        );
    }

    // ===================================================================
    // TrustResolver require_trusted() blocking tests
    // ===================================================================

    #[test]
    fn test_require_trusted_ok_for_trusted_tool() {
        let policy = swell_tools::TrustPolicy::new().allow("file_read");
        let resolver = swell_tools::TrustResolver::new(policy);
        assert!(resolver.require_trusted("file_read").is_ok());
    }

    #[test]
    fn test_require_trusted_err_for_untrusted_tool() {
        let policy = swell_tools::TrustPolicy::new().allow("file_read");
        let resolver = swell_tools::TrustResolver::new(policy);
        let result = resolver.require_trusted("dangerous_tool");
        assert!(result.is_err());
    }

    #[test]
    fn test_require_trusted_error_contains_tool_name() {
        let policy = swell_tools::TrustPolicy::new().allow("file_read");
        let resolver = swell_tools::TrustResolver::new(policy);
        let err = resolver.require_trusted("dangerous_tool").unwrap_err();

        // The error message must mention which tool was blocked
        let msg = err.to_string();
        assert!(
            msg.contains("dangerous_tool"),
            "Error message should contain the blocked tool name, got: {msg}"
        );
    }

    #[test]
    fn test_require_trusted_error_explains_why_denied() {
        let policy = swell_tools::TrustPolicy::new().allow("file_read");
        let resolver = swell_tools::TrustResolver::new(policy);
        let err = resolver.require_trusted("shell").unwrap_err();

        let msg = err.to_string();
        // The error message must explain WHY access was denied
        assert!(
            msg.contains("not trusted")
                || msg.contains("denied")
                || msg.contains("allow-list"),
            "Error message should explain why access was denied, got: {msg}"
        );
    }

    #[test]
    fn test_require_trusted_error_empty_policy_explains_empty_allow_list() {
        let policy = swell_tools::TrustPolicy::new();
        let resolver = swell_tools::TrustResolver::new(policy);
        let err = resolver.require_trusted("any_tool").unwrap_err();

        let msg = err.to_string();
        assert!(
            msg.contains("no tools have been added"),
            "Empty allow-list should be called out in the error, got: {msg}"
        );
    }

    #[test]
    fn test_require_trusted_error_includes_trusted_tools_count() {
        let policy = swell_tools::TrustPolicy::new()
            .allow("file_read")
            .allow("git_status")
            .allow("shell");
        let resolver = swell_tools::TrustResolver::new(policy);
        let err = resolver.require_trusted("http_request").unwrap_err();

        let msg = err.to_string();
        assert!(
            msg.contains("3"),
            "Error should mention the number of trusted tools, got: {msg}"
        );
    }

    // ===================================================================
    // Integration: untrusted tool execution is blocked
    // ===================================================================

    #[test]
    fn test_untrusted_tool_blocked_with_clear_error() {
        // Set up a policy that only trusts safe tools
        let policy = swell_tools::TrustPolicy::new()
            .allow("file_read")
            .allow("git_status")
            .allow("grep");
        let resolver = swell_tools::TrustResolver::new(policy);

        // Attempting to execute an untrusted tool returns a clear error
        let result = resolver.require_trusted("rm_rf_tool");
        assert!(result.is_err());

        let err = result.unwrap_err();
        let msg = err.to_string();

        // Must clearly identify the blocked tool
        assert!(msg.contains("rm_rf_tool"));
        // Must explain the denial reason
        assert!(
            msg.contains("not trusted")
                || msg.contains("denied")
                || msg.contains("allow-list"),
            "Error message must explain why access was denied, got: {msg}"
        );
    }

    #[test]
    fn test_trusted_tool_allowed_with_no_error() {
        let policy = swell_tools::TrustPolicy::new()
            .allow("file_read")
            .allow("git_status");
        let resolver = swell_tools::TrustResolver::new(policy);

        // Trusted tools pass without error
        assert!(resolver.require_trusted("file_read").is_ok());
        assert!(resolver.require_trusted("git_status").is_ok());
    }

    // ===================================================================
    // TrustError variant test
    // ===================================================================

    #[test]
    fn test_trust_error_display_includes_tool_name_and_reason() {
        let err = swell_tools::TrustError::UntrustedTool {
            tool_name: "test_tool".to_string(),
            reason: "this tool is not in the allow-list".to_string(),
        };

        let msg = err.to_string();
        assert!(msg.contains("test_tool"));
        assert!(msg.contains("this tool is not in the allow-list"));
    }
}
