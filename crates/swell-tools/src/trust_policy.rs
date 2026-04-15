//! Trust policy and resolution for tool execution.
//!
//! This module implements allow-list based trust control for tools. Before a tool
//! is executed, the `TrustResolver` checks whether the tool is in the `TrustPolicy`
//! allow-list. Untrusted tools are blocked with a clear error message.
//!
//! # Example
//!
//! ```rust
//! use swell_tools::trust_policy::{TrustPolicy, TrustResolver, TrustStatus};
//!
//! // Define which tools are trusted
//! let policy = TrustPolicy::new()
//!     .allow("file_read")
//!     .allow("git_status");
//!
//! let resolver = TrustResolver::new(policy);
//!
//! // Trusted tool
//! assert_eq!(resolver.check("file_read"), TrustStatus::Trusted);
//!
//! // Unknown tool is untrusted
//! assert_eq!(resolver.check("unknown_tool"), TrustStatus::Untrusted);
//!
//! // Block untrusted tool with a clear error
//! let result = resolver.require_trusted("dangerous_tool");
//! assert!(result.is_err());
//! ```

use std::collections::HashSet;
use thiserror::Error;

/// The trust status of a tool as determined by `TrustResolver`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrustStatus {
    /// The tool is in the allow-list and may be executed.
    Trusted,
    /// The tool is not in the allow-list and should be blocked.
    Untrusted,
}

/// Error returned when a tool is blocked by the trust resolver.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TrustError {
    /// The named tool is not trusted.
    #[error(
        "Tool '{tool_name}' is not trusted and cannot be executed. \
         Add it to the TrustPolicy allow-list to grant access. \
         Access was denied because: {reason}"
    )]
    UntrustedTool {
        /// The tool that was blocked.
        tool_name: String,
        /// Explanation of why access was denied.
        reason: String,
    },
}

/// Policy that defines which tools/servers are trusted.
///
/// Uses an allow-list approach: only explicitly listed tools are trusted.
/// All other tools are untrusted by default (deny-first).
///
/// # Examples
///
/// ```rust
/// use swell_tools::trust_policy::TrustPolicy;
///
/// // Explicit allow-list
/// let policy = TrustPolicy::new()
///     .allow("file_read")
///     .allow("git_status")
///     .allow("shell");
///
/// assert!(policy.is_trusted("file_read"));
/// assert!(!policy.is_trusted("unknown_tool"));
///
/// // Trust-all mode (wildcard)
/// let open_policy = TrustPolicy::trust_all();
/// assert!(open_policy.is_trusted("anything"));
/// ```
#[derive(Debug, Clone)]
pub struct TrustPolicy {
    /// Set of trusted tool names (stored in lowercase for case-insensitive matching).
    trusted_tools: HashSet<String>,
    /// When true, all tools are trusted regardless of the allow-list.
    trust_all: bool,
}

impl TrustPolicy {
    /// Create a new empty `TrustPolicy`.
    ///
    /// By default, no tools are trusted. Use [`allow`](Self::allow) to add tools.
    pub fn new() -> Self {
        Self {
            trusted_tools: HashSet::new(),
            trust_all: false,
        }
    }

    /// Create a `TrustPolicy` that trusts all tools (wildcard mode).
    ///
    /// Use with caution — this effectively disables trust enforcement.
    pub fn trust_all() -> Self {
        Self {
            trusted_tools: HashSet::new(),
            trust_all: true,
        }
    }

    /// Add a tool to the allow-list.
    ///
    /// Tool names are stored in lowercase for case-insensitive matching.
    pub fn allow(mut self, tool_name: impl Into<String>) -> Self {
        self.trusted_tools.insert(tool_name.into().to_lowercase());
        self
    }

    /// Add multiple tools to the allow-list at once.
    pub fn allow_many(mut self, tool_names: impl IntoIterator<Item = impl Into<String>>) -> Self {
        for name in tool_names {
            self.trusted_tools.insert(name.into().to_lowercase());
        }
        self
    }

    /// Check if a tool is trusted by this policy.
    ///
    /// Returns `true` when:
    /// - The policy is in trust-all mode, OR
    /// - The tool name (case-insensitive) is in the allow-list.
    pub fn is_trusted(&self, tool_name: &str) -> bool {
        if self.trust_all {
            return true;
        }
        self.trusted_tools.contains(&tool_name.to_lowercase())
    }

    /// Get the set of explicitly trusted tool names (lowercase).
    pub fn trusted_tools(&self) -> &HashSet<String> {
        &self.trusted_tools
    }

    /// Returns `true` if this policy is in trust-all (wildcard) mode.
    pub fn is_trust_all(&self) -> bool {
        self.trust_all
    }
}

impl Default for TrustPolicy {
    fn default() -> Self {
        Self::new()
    }
}

/// Resolves trust for tools before execution.
///
/// `TrustResolver` wraps a [`TrustPolicy`] and exposes:
/// - [`check`](Self::check): returns [`TrustStatus`] for a tool name.
/// - [`require_trusted`](Self::require_trusted): returns `Ok(())` for trusted tools
///   and an informative [`TrustError`] for untrusted ones.
///
/// # Examples
///
/// ```rust
/// use swell_tools::trust_policy::{TrustPolicy, TrustResolver, TrustStatus};
///
/// let policy = TrustPolicy::new().allow("file_read");
/// let resolver = TrustResolver::new(policy);
///
/// assert_eq!(resolver.check("file_read"), TrustStatus::Trusted);
/// assert_eq!(resolver.check("shell"), TrustStatus::Untrusted);
///
/// assert!(resolver.require_trusted("file_read").is_ok());
/// assert!(resolver.require_trusted("shell").is_err());
/// ```
#[derive(Debug, Clone)]
pub struct TrustResolver {
    policy: TrustPolicy,
}

impl TrustResolver {
    /// Create a new `TrustResolver` backed by the given `policy`.
    pub fn new(policy: TrustPolicy) -> Self {
        Self { policy }
    }

    /// Check whether a tool is trusted according to the policy.
    ///
    /// Returns `TrustStatus::Trusted` if the tool is in the allow-list (or the policy
    /// is in trust-all mode), and `TrustStatus::Untrusted` otherwise.
    pub fn check(&self, tool_name: &str) -> TrustStatus {
        if self.policy.is_trusted(tool_name) {
            TrustStatus::Trusted
        } else {
            TrustStatus::Untrusted
        }
    }

    /// Check trust and return an error if the tool is untrusted.
    ///
    /// This is a convenience method that combines `check()` with a descriptive
    /// error message explaining why access was denied.
    ///
    /// # Errors
    ///
    /// Returns [`TrustError::UntrustedTool`] when the tool is not in the allow-list.
    pub fn require_trusted(&self, tool_name: &str) -> Result<(), TrustError> {
        match self.check(tool_name) {
            TrustStatus::Trusted => Ok(()),
            TrustStatus::Untrusted => {
                let reason = if self.policy.is_trust_all() {
                    // This branch is unreachable in practice, but handle it gracefully.
                    "trust-all policy is active but trust check failed unexpectedly".to_string()
                } else if self.policy.trusted_tools().is_empty() {
                    "no tools have been added to the trust allow-list".to_string()
                } else {
                    format!(
                        "this tool is not in the allow-list of {} trusted tool(s): [{}]",
                        self.policy.trusted_tools().len(),
                        {
                            let mut names: Vec<&str> = self
                                .policy
                                .trusted_tools()
                                .iter()
                                .map(String::as_str)
                                .collect();
                            names.sort_unstable();
                            names.join(", ")
                        }
                    )
                };

                Err(TrustError::UntrustedTool {
                    tool_name: tool_name.to_string(),
                    reason,
                })
            }
        }
    }

    /// Get the underlying `TrustPolicy`.
    pub fn policy(&self) -> &TrustPolicy {
        &self.policy
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------
    // TrustPolicy tests
    // -----------------------------------------------------------------

    #[test]
    fn test_new_policy_trusts_nothing() {
        let policy = TrustPolicy::new();
        assert!(!policy.is_trusted("file_read"));
        assert!(!policy.is_trusted("shell"));
        assert!(!policy.is_trusted("git_status"));
    }

    #[test]
    fn test_allow_adds_tool_to_allow_list() {
        let policy = TrustPolicy::new().allow("file_read");
        assert!(policy.is_trusted("file_read"));
        assert!(!policy.is_trusted("shell"));
    }

    #[test]
    fn test_allow_is_case_insensitive() {
        let policy = TrustPolicy::new().allow("FileRead");
        assert!(policy.is_trusted("fileread"));
        assert!(policy.is_trusted("FILEREAD"));
        assert!(policy.is_trusted("FileRead"));
    }

    #[test]
    fn test_trust_all_trusts_everything() {
        let policy = TrustPolicy::trust_all();
        assert!(policy.is_trusted("any_tool"));
        assert!(policy.is_trusted("unknown_tool"));
        assert!(policy.is_trusted("file_read"));
    }

    #[test]
    fn test_allow_many_adds_multiple_tools() {
        let policy = TrustPolicy::new().allow_many(["file_read", "git_status", "shell"]);
        assert!(policy.is_trusted("file_read"));
        assert!(policy.is_trusted("git_status"));
        assert!(policy.is_trusted("shell"));
        assert!(!policy.is_trusted("unknown_tool"));
    }

    #[test]
    fn test_trusted_tools_returns_lowercase_names() {
        let policy = TrustPolicy::new().allow("FileRead").allow("GIT_STATUS");
        let trusted = policy.trusted_tools();
        assert!(trusted.contains("fileread"));
        assert!(trusted.contains("git_status"));
    }

    #[test]
    fn test_is_trust_all_reflects_mode() {
        assert!(!TrustPolicy::new().is_trust_all());
        assert!(TrustPolicy::trust_all().is_trust_all());
    }

    // -----------------------------------------------------------------
    // TrustResolver tests
    // -----------------------------------------------------------------

    #[test]
    fn test_check_returns_trusted_for_allowed_tool() {
        let policy = TrustPolicy::new().allow("file_read");
        let resolver = TrustResolver::new(policy);
        assert_eq!(resolver.check("file_read"), TrustStatus::Trusted);
    }

    #[test]
    fn test_check_returns_untrusted_for_unknown_tool() {
        let policy = TrustPolicy::new().allow("file_read");
        let resolver = TrustResolver::new(policy);
        assert_eq!(resolver.check("unknown_tool"), TrustStatus::Untrusted);
    }

    #[test]
    fn test_check_with_empty_policy_blocks_all() {
        let policy = TrustPolicy::new();
        let resolver = TrustResolver::new(policy);
        assert_eq!(resolver.check("file_read"), TrustStatus::Untrusted);
    }

    #[test]
    fn test_check_with_trust_all_policy_allows_all() {
        let policy = TrustPolicy::trust_all();
        let resolver = TrustResolver::new(policy);
        assert_eq!(resolver.check("any_tool"), TrustStatus::Trusted);
        assert_eq!(resolver.check("unknown_tool"), TrustStatus::Trusted);
    }

    #[test]
    fn test_require_trusted_ok_for_trusted_tool() {
        let policy = TrustPolicy::new().allow("file_read");
        let resolver = TrustResolver::new(policy);
        assert!(resolver.require_trusted("file_read").is_ok());
    }

    #[test]
    fn test_require_trusted_err_for_untrusted_tool() {
        let policy = TrustPolicy::new().allow("file_read");
        let resolver = TrustResolver::new(policy);
        let result = resolver.require_trusted("dangerous_tool");
        assert!(result.is_err());
    }

    #[test]
    fn test_require_trusted_error_message_mentions_tool_name() {
        let policy = TrustPolicy::new().allow("file_read");
        let resolver = TrustResolver::new(policy);
        let err = resolver.require_trusted("dangerous_tool").unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("dangerous_tool"),
            "Error message should mention the blocked tool name, got: {msg}"
        );
    }

    #[test]
    fn test_require_trusted_error_message_explains_why() {
        let policy = TrustPolicy::new().allow("file_read");
        let resolver = TrustResolver::new(policy);
        let err = resolver.require_trusted("shell").unwrap_err();
        let msg = err.to_string();
        // Must contain an explanation of why access was denied
        assert!(
            msg.contains("not trusted") || msg.contains("denied") || msg.contains("allow-list"),
            "Error message should explain why access was denied, got: {msg}"
        );
    }

    #[test]
    fn test_require_trusted_error_message_for_empty_policy() {
        let policy = TrustPolicy::new();
        let resolver = TrustResolver::new(policy);
        let err = resolver.require_trusted("any_tool").unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("no tools have been added"),
            "Empty policy error should mention empty allow-list, got: {msg}"
        );
    }

    #[test]
    fn test_policy_accessor() {
        let policy = TrustPolicy::new().allow("file_read");
        let resolver = TrustResolver::new(policy);
        assert!(resolver.policy().is_trusted("file_read"));
    }
}
