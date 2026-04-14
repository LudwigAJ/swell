//! Permission mode system with ordered variants.
//!
//! PermissionMode defines the permission levels for tool execution with strict ordering:
//! - `Deny`: Never allowed (highest restriction)
//! - `Ask`: Requires user confirmation
//! - `Suggest`: Suggested permission level
//! - `Auto`: Always permitted (lowest restriction)
//!
//! The ordering (Deny < Ask < Suggest < Auto) ensures that Deny always wins
//! when comparing permission levels.

use std::fmt;
use std::ops::Not;

/// Ordered permission mode for tool execution.
///
/// Variants are ordered from most restrictive to most permissive:
/// - `Deny` (0): Never allowed without explicit override
/// - `Ask` (1): Requires user confirmation
/// - `Suggest` (2): Suggested permission level
/// - `Auto` (3): Always permitted
///
/// # Derives
/// - `Ord` and `PartialOrd`: Enable comparison operations (Deny < Ask < Suggest < Auto)
/// - `Clone`, `Copy`, `Debug`, `PartialEq`, `Eq`: Standard utility derives
/// - `serde::Serialize` and `serde::Deserialize`: JSON/YAML serialization
///
/// # Examples
///
/// ```
/// use swell_tools::permissions::PermissionMode;
///
/// // Comparison operations work as expected
/// assert!(PermissionMode::Deny < PermissionMode::Ask);
/// assert!(PermissionMode::Ask < PermissionMode::Suggest);
/// assert!(PermissionMode::Suggest < PermissionMode::Auto);
///
/// // Check if a tool can be executed given active mode
/// fn can_execute(required: PermissionMode, active: PermissionMode) -> bool {
///     required <= active
/// }
///
/// assert!(can_execute(PermissionMode::Ask, PermissionMode::Auto));
/// assert!(!can_execute(PermissionMode::Ask, PermissionMode::Deny));
/// ```
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum PermissionMode {
    /// Never allowed without explicit override
    Deny = 0,
    /// Requires user confirmation before execution
    Ask = 1,
    /// Suggested permission level (default for most tools)
    Suggest = 2,
    /// Always permitted (auto-approved)
    Auto = 3,
}

impl PermissionMode {
    /// Returns the default permission mode for tools (Suggest)
    pub fn default_mode() -> Self {
        PermissionMode::Suggest
    }

    /// Returns true if this permission mode allows execution
    /// given the active permission mode.
    ///
    /// The semantics follow the validation contract's dispatch check:
    /// tool can execute if `required_permission <= active_mode`.
    ///
    /// Special case: Deny (being the most restrictive) never allows execution
    /// by any active mode, including Deny itself. This ensures tools requiring
    /// Deny permission cannot run unless explicitly overridden.
    ///
    /// For all other modes (Ask, Suggest, Auto), the check is `self <= active_mode`:
    /// - Ask allows Ask, Suggest, Auto (but not Deny)
    /// - Suggest allows Suggest, Auto (but not Deny, Ask)
    /// - Auto allows only Auto (but not Deny, Ask, Suggest)
    pub fn allows(&self, active_mode: PermissionMode) -> bool {
        match self {
            PermissionMode::Deny => false, // Special case: Deny never allows
            _ => *self <= active_mode,
        }
    }

    /// Returns the display name for this permission mode
    pub fn display_name(&self) -> &'static str {
        match self {
            PermissionMode::Deny => "Deny",
            PermissionMode::Ask => "Ask",
            PermissionMode::Suggest => "Suggest",
            PermissionMode::Auto => "Auto",
        }
    }

    /// Returns a description of this permission mode
    pub fn description(&self) -> &'static str {
        match self {
            PermissionMode::Deny => "Tool is never allowed without explicit override",
            PermissionMode::Ask => "Tool requires user confirmation before execution",
            PermissionMode::Suggest => "Tool is suggested to run with user confirmation",
            PermissionMode::Auto => "Tool is automatically approved",
        }
    }

    /// Parse from string (case-insensitive)
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "deny" => Some(PermissionMode::Deny),
            "ask" => Some(PermissionMode::Ask),
            "suggest" => Some(PermissionMode::Suggest),
            "auto" => Some(PermissionMode::Auto),
            _ => None,
        }
    }
}

impl Default for PermissionMode {
    fn default() -> Self {
        PermissionMode::Suggest
    }
}

impl fmt::Display for PermissionMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.display_name())
    }
}

/// Inverse permission mode (for negation in rules)
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct InversePermissionMode {
    mode: PermissionMode,
}

impl InversePermissionMode {
    pub fn new(mode: PermissionMode) -> Self {
        Self { mode }
    }

    /// Returns the inverse mode where Deny becomes Auto and vice versa,
    /// Ask becomes Ask and Suggest becomes Suggest (self-inverse for middle values)
    pub fn inverse(&self) -> PermissionMode {
        match self.mode {
            PermissionMode::Deny => PermissionMode::Auto,
            PermissionMode::Ask => PermissionMode::Ask,
            PermissionMode::Suggest => PermissionMode::Suggest,
            PermissionMode::Auto => PermissionMode::Deny,
        }
    }
}

impl Not for PermissionMode {
    type Output = InversePermissionMode;

    fn not(self) -> Self::Output {
        InversePermissionMode::new(self)
    }
}

impl From<PermissionMode> for PermissionTier {
    fn from(mode: PermissionMode) -> Self {
        match mode {
            PermissionMode::Deny => PermissionTier::Deny,
            PermissionMode::Ask => PermissionTier::Ask,
            PermissionMode::Suggest | PermissionMode::Auto => PermissionTier::Auto,
        }
    }
}

impl From<PermissionTier> for PermissionMode {
    fn from(tier: PermissionTier) -> Self {
        match tier {
            PermissionTier::Deny => PermissionMode::Deny,
            PermissionTier::Ask => PermissionMode::Ask,
            PermissionTier::Auto => PermissionMode::Auto,
        }
    }
}

// Re-export PermissionTier from swell_core for compatibility
use swell_core::PermissionTier;

// ============================================================================
// Tool Specification
// ============================================================================

/// Specification for a tool's metadata and permission requirements.
///
/// This struct is used to describe a tool's capabilities and requirements
/// without requiring the full tool implementation. Useful for tool registry,
/// policy evaluation, and documentation generation.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ToolSpec {
    /// Unique name of the tool
    pub name: String,
    /// Human-readable description
    pub description: String,
    /// Required permission mode for execution
    ///
    /// The tool can only be executed when the active permission mode
    /// is greater than or equal to this value (i.e., `required_permission <= active_mode`).
    ///
    /// Default: `PermissionMode::Ask`
    #[serde(default = "PermissionMode::default_mode")]
    pub required_permission: PermissionMode,
    /// JSON Schema for input parameters
    pub input_schema: serde_json::Value,
    /// JSON Schema for output (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<serde_json::Value>,
    /// Risk level of the tool
    #[serde(skip_serializing_if = "Option::is_none")]
    pub risk_level: Option<swell_core::ToolRiskLevel>,
    /// Whether this tool is read-only (safe to retry)
    #[serde(default = "default_bool_false")]
    pub read_only: bool,
    /// Whether this tool is destructive (permanent changes)
    #[serde(default = "default_bool_false")]
    pub destructive: bool,
}

fn default_bool_false() -> bool {
    false
}

impl ToolSpec {
    /// Create a new ToolSpec with the given name and description
    pub fn new(name: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            required_permission: PermissionMode::default_mode(),
            input_schema: serde_json::json!({}),
            output_schema: None,
            risk_level: None,
            read_only: false,
            destructive: false,
        }
    }

    /// Set the required permission mode
    pub fn with_permission(mut self, permission: PermissionMode) -> Self {
        self.required_permission = permission;
        self
    }

    /// Set the risk level
    pub fn with_risk_level(mut self, risk_level: swell_core::ToolRiskLevel) -> Self {
        self.risk_level = Some(risk_level);
        self
    }

    /// Set as read-only tool
    pub fn read_only(mut self) -> Self {
        self.read_only = true;
        self
    }

    /// Set as destructive tool
    pub fn destructive(mut self) -> Self {
        self.destructive = true;
        self
    }

    /// Check if this tool can be executed with the given active permission mode
    pub fn can_execute(&self, active_mode: PermissionMode) -> bool {
        self.required_permission.allows(active_mode)
    }
}

impl Default for ToolSpec {
    fn default() -> Self {
        Self {
            name: "unnamed".to_string(),
            description: "No description".to_string(),
            required_permission: PermissionMode::Ask,
            input_schema: serde_json::json!({}),
            output_schema: None,
            risk_level: None,
            read_only: false,
            destructive: false,
        }
    }
}

// ============================================================================
// Three-Layer Rule Evaluation
// ============================================================================

/// Result of a three-layer permission rule evaluation
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PermissionResult {
    /// Explicitly denied by a Deny rule
    Denied,
    /// Requires user confirmation due to Ask rule
    Ask,
    /// Explicitly allowed (no Deny/Ask matched)
    Allowed,
}

impl PermissionResult {
    /// Returns true if the action is permitted (Allowed or Ask requires confirmation)
    pub fn is_permitted(&self) -> bool {
        matches!(self, PermissionResult::Allowed | PermissionResult::Ask)
    }

    /// Returns true if the action is denied
    pub fn is_denied(&self) -> bool {
        matches!(self, PermissionResult::Denied)
    }

    /// Returns true if the action requires user confirmation
    pub fn requires_confirmation(&self) -> bool {
        matches!(self, PermissionResult::Ask)
    }
}

/// A permission rule with an effect and target
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PermissionRule {
    /// Rule identifier (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Human-readable name
    pub name: String,
    /// The permission effect
    pub effect: PermissionRuleEffect,
    /// Tool name pattern (glob-style, e.g., "file_*")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_pattern: Option<String>,
    /// Path patterns for file operations
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path_patterns: Option<Vec<String>>,
    /// Optional description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Effect of a permission rule
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PermissionRuleEffect {
    /// Deny the action (highest priority - always wins if matched)
    Deny,
    /// Ask for confirmation
    Ask,
    /// Allow the action (only used if no Deny/Ask matched)
    Allow,
}

impl PermissionRuleEffect {
    /// Get the priority of this effect (higher = evaluated first)
    pub fn priority(&self) -> i32 {
        match self {
            PermissionRuleEffect::Deny => 100,
            PermissionRuleEffect::Ask => 50,
            PermissionRuleEffect::Allow => 10,
        }
    }
}

/// Evaluator for three-layer permission rules (Deny → Ask → Allow)
///
/// This evaluator processes rules in strict priority order:
/// 1. Check all Deny rules - if ANY matches, result is Denied
/// 2. Check all Ask rules - if ANY matches and no Deny matched, result is Ask
/// 3. If neither Deny nor Ask matched, result is Allowed
///
/// This ensures Deny always takes precedence over Ask, and Ask always takes
/// precedence over Allow.
#[derive(Debug, Clone)]
pub struct ThreeLayerEvaluator {
    rules: Vec<PermissionRule>,
}

impl ThreeLayerEvaluator {
    /// Create a new empty evaluator
    pub fn new() -> Self {
        Self { rules: Vec::new() }
    }

    /// Add a rule to the evaluator
    pub fn add_rule(mut self, rule: PermissionRule) -> Self {
        self.rules.push(rule);
        self
    }

    /// Add rules from an iterator
    pub fn add_rules(mut self, rules: impl IntoIterator<Item = PermissionRule>) -> Self {
        self.rules.extend(rules);
        self
    }

    /// Evaluate a tool execution request against all rules.
    ///
    /// Uses three-layer evaluation:
    /// - First pass: Check all Deny rules (highest priority)
    /// - Second pass: Check all Ask rules (if no Deny matched)
    /// - Third pass: If no Deny or Ask matched, return Allowed
    ///
    /// # Arguments
    ///
    /// * `tool_name` - The name of the tool being executed
    /// * `paths` - Optional list of file paths involved in the operation
    ///
    /// # Returns
    ///
    /// * `PermissionResult::Denied` if any Deny rule matches (strict priority)
    /// * `PermissionResult::Ask` if any Ask rule matches and no Deny matched
    /// * `PermissionResult::Allowed` if neither Deny nor Ask matched
    ///
    /// # Examples
    ///
    /// ```
    /// use swell_tools::permissions::{
    ///     ThreeLayerEvaluator, PermissionRule, PermissionRuleEffect, PermissionResult
    /// };
    ///
    /// let evaluator = ThreeLayerEvaluator::new()
    ///     .add_rule(PermissionRule {
    ///         id: Some("deny-rm-rf".to_string()),
    ///         name: "Deny rm -rf".to_string(),
    ///         effect: PermissionRuleEffect::Deny,
    ///         tool_pattern: Some("shell".to_string()),
    ///         path_patterns: Some(vec!["/etc".to_string(), "/root".to_string()]),
    ///         description: None,
    ///     })
    ///     .add_rule(PermissionRule {
    ///         id: Some("allow-read".to_string()),
    ///         name: "Allow read".to_string(),
    ///         effect: PermissionRuleEffect::Allow,
    ///         tool_pattern: Some("file_read".to_string()),
    ///         path_patterns: None,
    ///         description: None,
    ///     });
    ///
    /// // shell with /etc path should be denied
    /// let paths = vec!["/etc/passwd".to_string()];
    /// let result = evaluator.evaluate("shell", Some(&paths));
    /// assert_eq!(result, PermissionResult::Denied);
    ///
    /// // file_read should be allowed
    /// let result = evaluator.evaluate("file_read", None);
    /// assert_eq!(result, PermissionResult::Allowed);
    /// ```
    pub fn evaluate(&self, tool_name: &str, paths: Option<&[String]>) -> PermissionResult {
        // Sort rules by priority (Deny first, then Ask, then Allow)
        let mut sorted_rules: Vec<_> = self.rules.iter().collect();
        sorted_rules.sort_by(|a, b| b.effect.priority().cmp(&a.effect.priority()));

        let mut has_deny = false;
        let mut has_ask = false;

        for rule in sorted_rules {
            if self.rule_matches(rule, tool_name, paths) {
                match rule.effect {
                    PermissionRuleEffect::Deny => {
                        has_deny = true;
                        // Deny always wins - we could break here but continue to detect conflicts
                    }
                    PermissionRuleEffect::Ask => {
                        // Only applies if no Deny matched
                        if !has_deny {
                            has_ask = true;
                        }
                    }
                    PermissionRuleEffect::Allow => {
                        // Only applies if no Deny or Ask matched
                        if !has_deny && !has_ask {
                            // Would return Allowed, but we fall through to the end
                        }
                    }
                }
            }
        }

        if has_deny {
            PermissionResult::Denied
        } else if has_ask {
            PermissionResult::Ask
        } else {
            PermissionResult::Allowed
        }
    }

    /// Check if a rule matches the given tool and paths
    fn rule_matches(
        &self,
        rule: &PermissionRule,
        tool_name: &str,
        paths: Option<&[String]>,
    ) -> bool {
        // Check tool pattern
        if let Some(pattern) = &rule.tool_pattern {
            if !self::pattern_matches(pattern, tool_name) {
                return false;
            }
        }

        // Check path patterns
        if let Some(patterns) = &rule.path_patterns {
            if let Some(paths) = paths {
                if paths.is_empty() {
                    // If rule has path patterns but no paths given, no match
                    return false;
                }
                // Check if any path matches any pattern
                let any_match = paths.iter().any(|p| {
                    patterns
                        .iter()
                        .any(|pattern| self::path_matches(pattern, p))
                });
                if !any_match {
                    return false;
                }
            } else {
                // Rule has path patterns but no paths provided - no match
                return false;
            }
        }

        true
    }

    /// Get all rules
    pub fn rules(&self) -> &[PermissionRule] {
        &self.rules
    }

    /// Clear all rules
    pub fn clear(&mut self) {
        self.rules.clear();
    }

    /// Get count of rules by effect
    pub fn rule_counts(&self) -> (usize, usize, usize) {
        let mut deny = 0;
        let mut ask = 0;
        let mut allow = 0;
        for rule in &self.rules {
            match rule.effect {
                PermissionRuleEffect::Deny => deny += 1,
                PermissionRuleEffect::Ask => ask += 1,
                PermissionRuleEffect::Allow => allow += 1,
            }
        }
        (deny, ask, allow)
    }
}

impl Default for ThreeLayerEvaluator {
    fn default() -> Self {
        Self::new()
    }
}

// Helper function for glob-style pattern matching
fn pattern_matches(pattern: &str, name: &str) -> bool {
    // Simple glob matching: * matches any sequence, ? matches single char
    if pattern.contains('*') {
        let parts: Vec<&str> = pattern.split('*').collect();
        let mut remaining = name;

        for (i, part) in parts.iter().enumerate() {
            if i == 0 && !part.is_empty() {
                // First part must be at the start
                if !remaining.starts_with(part) {
                    return false;
                }
                remaining = &remaining[part.len()..];
            } else if i == parts.len() - 1 {
                // Last part must be at the end
                if !remaining.ends_with(part) {
                    return false;
                }
            } else {
                // Middle parts - find next occurrence
                if let Some(pos) = remaining.find(part) {
                    remaining = &remaining[pos + part.len()..];
                } else {
                    return false;
                }
            }
        }
        true
    } else {
        pattern == name
    }
}

// Helper function for path matching (prefix-based)
fn path_matches(pattern: &str, path: &str) -> bool {
    // Simple prefix matching with * for wildcard
    if pattern.ends_with('*') {
        let prefix = &pattern[..pattern.len() - 1];
        path.starts_with(prefix)
    } else {
        path.starts_with(pattern)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_permission_mode_ordering() {
        // Verify the ordering: Deny < Ask < Suggest < Auto
        assert!(PermissionMode::Deny < PermissionMode::Ask);
        assert!(PermissionMode::Ask < PermissionMode::Suggest);
        assert!(PermissionMode::Suggest < PermissionMode::Auto);

        // Transitivity
        assert!(PermissionMode::Deny < PermissionMode::Suggest);
        assert!(PermissionMode::Deny < PermissionMode::Auto);
        assert!(PermissionMode::Ask < PermissionMode::Auto);
    }

    #[test]
    fn test_permission_mode_allows() {
        // Deny blocks everything below it
        assert!(!PermissionMode::Deny.allows(PermissionMode::Deny));
        assert!(!PermissionMode::Deny.allows(PermissionMode::Ask));
        assert!(!PermissionMode::Deny.allows(PermissionMode::Suggest));
        assert!(!PermissionMode::Deny.allows(PermissionMode::Auto));

        // Ask blocks Ask and below
        assert!(!PermissionMode::Ask.allows(PermissionMode::Deny));
        assert!(PermissionMode::Ask.allows(PermissionMode::Ask));
        assert!(PermissionMode::Ask.allows(PermissionMode::Suggest));
        assert!(PermissionMode::Ask.allows(PermissionMode::Auto));

        // Suggest blocks Suggest and below
        assert!(!PermissionMode::Suggest.allows(PermissionMode::Deny));
        assert!(!PermissionMode::Suggest.allows(PermissionMode::Ask));
        assert!(PermissionMode::Suggest.allows(PermissionMode::Suggest));
        assert!(PermissionMode::Suggest.allows(PermissionMode::Auto));

        // Auto only allows Auto
        assert!(!PermissionMode::Auto.allows(PermissionMode::Deny));
        assert!(!PermissionMode::Auto.allows(PermissionMode::Ask));
        assert!(!PermissionMode::Auto.allows(PermissionMode::Suggest));
        assert!(PermissionMode::Auto.allows(PermissionMode::Auto));
    }

    #[test]
    fn test_permission_mode_parse() {
        assert_eq!(PermissionMode::parse("deny"), Some(PermissionMode::Deny));
        assert_eq!(PermissionMode::parse("Deny"), Some(PermissionMode::Deny));
        assert_eq!(PermissionMode::parse("DENY"), Some(PermissionMode::Deny));
        assert_eq!(PermissionMode::parse("ask"), Some(PermissionMode::Ask));
        assert_eq!(
            PermissionMode::parse("suggest"),
            Some(PermissionMode::Suggest)
        );
        assert_eq!(PermissionMode::parse("auto"), Some(PermissionMode::Auto));
        assert_eq!(PermissionMode::parse("unknown"), None);
    }

    #[test]
    fn test_tool_spec_default() {
        let spec = ToolSpec::default();
        assert_eq!(spec.name, "unnamed");
        assert_eq!(spec.required_permission, PermissionMode::Ask);
    }

    #[test]
    fn test_tool_spec_builder() {
        let spec = ToolSpec::new("my_tool", "A test tool")
            .with_permission(PermissionMode::Auto)
            .with_risk_level(swell_core::ToolRiskLevel::Read)
            .read_only();

        assert_eq!(spec.name, "my_tool");
        assert_eq!(spec.required_permission, PermissionMode::Auto);
        assert_eq!(spec.risk_level, Some(swell_core::ToolRiskLevel::Read));
        assert!(spec.read_only);
        assert!(!spec.destructive);
    }

    #[test]
    fn test_tool_spec_can_execute() {
        let spec = ToolSpec::new("test", "test").with_permission(PermissionMode::Ask);

        assert!(spec.can_execute(PermissionMode::Ask));
        assert!(spec.can_execute(PermissionMode::Suggest));
        assert!(spec.can_execute(PermissionMode::Auto));
        assert!(!spec.can_execute(PermissionMode::Deny));
    }

    #[test]
    fn test_three_layer_deny_wins() {
        // Test that Deny always wins over Allow
        let evaluator = ThreeLayerEvaluator::new()
            .add_rule(PermissionRule {
                id: Some("allow-shell".to_string()),
                name: "Allow shell".to_string(),
                effect: PermissionRuleEffect::Allow,
                tool_pattern: Some("shell".to_string()),
                path_patterns: None,
                description: None,
            })
            .add_rule(PermissionRule {
                id: Some("deny-shell".to_string()),
                name: "Deny shell".to_string(),
                effect: PermissionRuleEffect::Deny,
                tool_pattern: Some("shell".to_string()),
                path_patterns: None,
                description: None,
            });

        let result = evaluator.evaluate("shell", None);
        assert_eq!(result, PermissionResult::Denied);
    }

    #[test]
    fn test_three_layer_ask_wins_over_allow() {
        // Test that Ask wins over Allow (but not over Deny)
        let evaluator = ThreeLayerEvaluator::new()
            .add_rule(PermissionRule {
                id: Some("allow-shell".to_string()),
                name: "Allow shell".to_string(),
                effect: PermissionRuleEffect::Allow,
                tool_pattern: Some("shell".to_string()),
                path_patterns: None,
                description: None,
            })
            .add_rule(PermissionRule {
                id: Some("ask-shell".to_string()),
                name: "Ask shell".to_string(),
                effect: PermissionRuleEffect::Ask,
                tool_pattern: Some("shell".to_string()),
                path_patterns: None,
                description: None,
            });

        let result = evaluator.evaluate("shell", None);
        assert_eq!(result, PermissionResult::Ask);
    }

    #[test]
    fn test_three_layer_no_match_allowed() {
        // Test that when no rule matches, result is Allowed
        let evaluator = ThreeLayerEvaluator::new().add_rule(PermissionRule {
            id: Some("deny-shell".to_string()),
            name: "Deny shell".to_string(),
            effect: PermissionRuleEffect::Deny,
            tool_pattern: Some("shell".to_string()),
            path_patterns: None,
            description: None,
        });

        let result = evaluator.evaluate("file_read", None);
        assert_eq!(result, PermissionResult::Allowed);
    }

    #[test]
    fn test_three_layer_path_pattern_matching() {
        let evaluator = ThreeLayerEvaluator::new()
            .add_rule(PermissionRule {
                id: Some("deny-etc".to_string()),
                name: "Deny /etc".to_string(),
                effect: PermissionRuleEffect::Deny,
                tool_pattern: Some("file_write".to_string()),
                path_patterns: Some(vec!["/etc/".to_string()]),
                description: None,
            })
            .add_rule(PermissionRule {
                id: Some("allow-workspace".to_string()),
                name: "Allow workspace".to_string(),
                effect: PermissionRuleEffect::Allow,
                tool_pattern: Some("file_write".to_string()),
                path_patterns: Some(vec!["/workspace/".to_string()]),
                description: None,
            });

        // Write to /etc should be denied
        let result = evaluator.evaluate("file_write", Some(&["/etc/passwd".to_string()]));
        assert_eq!(result, PermissionResult::Denied);

        // Write to /workspace should be allowed
        let result =
            evaluator.evaluate("file_write", Some(&["/workspace/src/main.rs".to_string()]));
        assert_eq!(result, PermissionResult::Allowed);
    }

    #[test]
    fn test_permission_result_helpers() {
        assert!(PermissionResult::Denied.is_denied());
        assert!(!PermissionResult::Denied.is_permitted());

        assert!(!PermissionResult::Ask.is_denied());
        assert!(PermissionResult::Ask.is_permitted());
        assert!(PermissionResult::Ask.requires_confirmation());

        assert!(!PermissionResult::Allowed.is_denied());
        assert!(PermissionResult::Allowed.is_permitted());
        assert!(!PermissionResult::Allowed.requires_confirmation());
    }

    #[test]
    fn test_pattern_matching_glob() {
        // Test glob patterns
        assert!(pattern_matches("file_*", "file_read"));
        assert!(pattern_matches("file_*", "file_write"));
        assert!(!pattern_matches("file_*", "shell_exec"));

        assert!(pattern_matches("read_file", "read_file"));
        assert!(!pattern_matches("read_file", "write_file"));
    }

    #[test]
    fn test_inverse_permission_mode() {
        assert_eq!(
            !PermissionMode::Deny,
            InversePermissionMode::new(PermissionMode::Deny)
        );

        // Deny inverts to Auto
        let inverse = !PermissionMode::Deny;
        assert_eq!(inverse.inverse(), PermissionMode::Auto);

        // Auto inverts to Deny
        let inverse = !PermissionMode::Auto;
        assert_eq!(inverse.inverse(), PermissionMode::Deny);

        // Ask and Suggest are self-inverse
        let inverse = !PermissionMode::Ask;
        assert_eq!(inverse.inverse(), PermissionMode::Ask);

        let inverse = !PermissionMode::Suggest;
        assert_eq!(inverse.inverse(), PermissionMode::Suggest);
    }

    #[test]
    fn test_conversion_from_permission_tier() {
        assert_eq!(
            PermissionMode::from(PermissionTier::Deny),
            PermissionMode::Deny
        );
        assert_eq!(
            PermissionMode::from(PermissionTier::Ask),
            PermissionMode::Ask
        );
        assert_eq!(
            PermissionMode::from(PermissionTier::Auto),
            PermissionMode::Auto
        );
    }

    #[test]
    fn test_conversion_to_permission_tier() {
        assert_eq!(
            PermissionTier::from(PermissionMode::Deny),
            PermissionTier::Deny
        );
        assert_eq!(
            PermissionTier::from(PermissionMode::Ask),
            PermissionTier::Ask
        );
        assert_eq!(
            PermissionTier::from(PermissionMode::Suggest),
            PermissionTier::Auto
        );
        assert_eq!(
            PermissionTier::from(PermissionMode::Auto),
            PermissionTier::Auto
        );
    }

    #[test]
    fn test_permission_rule_effect_priority() {
        assert!(PermissionRuleEffect::Deny.priority() > PermissionRuleEffect::Ask.priority());
        assert!(PermissionRuleEffect::Ask.priority() > PermissionRuleEffect::Allow.priority());
    }

    #[test]
    fn test_evaluator_rule_counts() {
        let evaluator = ThreeLayerEvaluator::new()
            .add_rule(PermissionRule {
                id: Some("1".to_string()),
                name: "Deny 1".to_string(),
                effect: PermissionRuleEffect::Deny,
                tool_pattern: None,
                path_patterns: None,
                description: None,
            })
            .add_rule(PermissionRule {
                id: Some("2".to_string()),
                name: "Ask 1".to_string(),
                effect: PermissionRuleEffect::Ask,
                tool_pattern: None,
                path_patterns: None,
                description: None,
            })
            .add_rule(PermissionRule {
                id: Some("3".to_string()),
                name: "Allow 1".to_string(),
                effect: PermissionRuleEffect::Allow,
                tool_pattern: None,
                path_patterns: None,
                description: None,
            });

        let (deny, ask, allow) = evaluator.rule_counts();
        assert_eq!(deny, 1);
        assert_eq!(ask, 1);
        assert_eq!(allow, 1);
    }

    #[test]
    fn test_tool_spec_serialization() {
        let spec = ToolSpec::new("test_tool", "A test tool description")
            .with_permission(PermissionMode::Ask)
            .with_risk_level(swell_core::ToolRiskLevel::Write);

        let json = serde_json::to_string(&spec).unwrap();
        let deserialized: ToolSpec = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.name, "test_tool");
        assert_eq!(deserialized.required_permission, PermissionMode::Ask);
        assert_eq!(
            deserialized.risk_level,
            Some(swell_core::ToolRiskLevel::Write)
        );
    }

    #[test]
    fn test_permission_mode_serialization() {
        let mode = PermissionMode::Ask;

        let json = serde_json::to_string(&mode).unwrap();
        assert_eq!(json, "\"ask\"");

        let deserialized: PermissionMode = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, PermissionMode::Ask);
    }

    #[test]
    fn test_evaluator_clear() {
        let mut evaluator = ThreeLayerEvaluator::new().add_rule(PermissionRule {
            id: Some("1".to_string()),
            name: "Test".to_string(),
            effect: PermissionRuleEffect::Deny,
            tool_pattern: None,
            path_patterns: None,
            description: None,
        });

        assert_eq!(evaluator.rules().len(), 1);

        evaluator.clear();
        assert_eq!(evaluator.rules().len(), 0);

        let result = evaluator.evaluate("any_tool", None);
        assert_eq!(result, PermissionResult::Allowed); // No rules = Allowed
    }
}
