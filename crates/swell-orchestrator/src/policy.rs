//! Policy engine for evaluating YAML-defined policies against agent actions.
//!
//! Implements deny-first evaluation semantics where:
//! - If any rule denies the action, it's denied
//! - Only if no rule denies AND at least one rule allows, it's permitted
//! - If no rule matches, the default is to deny (safe-by-default)
//!
//! Policy files are YAML documents with the following structure:
//! ```yaml
//! version: "1.0"
//! default_effect: deny  # or allow
//! rules:
//!   - name: "deny dangerous commands"
//!     effect: deny
//!     condition:
//!       type: command_match
//!       data:
//!         pattern: "(rm -rf|DROP|TRUNCATE|--force|--no-verify)"
//!   - name: "allow read operations"
//!     effect: allow
//!     condition:
//!       type: tool_category
//!       data:
//!         category: read
//!   - name: "allow specific paths"
//!     effect: allow
//!     condition:
//!       type: path_prefix
//!       data:
//!         paths: ["/workspace/src", "/workspace/tests"]
//! ```

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;
use tracing::{debug, info, warn};

/// Errors that can occur in policy evaluation
#[derive(Error, Debug, Clone)]
pub enum PolicyError {
    #[error("Failed to parse policy YAML: {0}")]
    ParseError(String),
    #[error("Failed to load policy: {0}")]
    LoadError(String),
    #[error("Invalid policy rule: {0}")]
    InvalidRule(String),
    #[error("No policy loaded")]
    NoPolicyLoaded,
}

/// Result of evaluating an action against policies
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyDecision {
    /// Action is allowed by policy
    Allow,
    /// Action is denied by policy
    Deny,
    /// No rule matched the action
    NoMatch,
}

impl PolicyDecision {
    /// Returns true if the action is permitted
    pub fn is_allowed(&self) -> bool {
        matches!(self, PolicyDecision::Allow)
    }

    /// Returns true if the action is denied
    pub fn is_denied(&self) -> bool {
        matches!(self, PolicyDecision::Deny)
    }

    /// Returns true if no rule matched
    pub fn is_no_match(&self) -> bool {
        matches!(self, PolicyDecision::NoMatch)
    }
}

/// A policy rule condition
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
#[serde(rename_all = "snake_case")]
pub enum PolicyCondition {
    /// Match by command pattern (regex)
    CommandMatch { pattern: String },
    /// Match by tool name
    ToolName { name: String },
    /// Match by tool category (read, write, destructive)
    ToolCategory { category: ToolCategory },
    /// Match by file path prefix
    PathPrefix { paths: Vec<String> },
    /// Match by file path suffix (extension)
    PathSuffix { suffixes: Vec<String> },
    /// Match by exact path
    PathExact { path: String },
    /// Match by risk level
    RiskLevel { level: RiskLevelMatch },
    /// Match by agent role
    AgentRole { role: String },
    /// Always match (for default rules)
    Always,
}

/// A policy rule condition (alternative YAML format with value field)
/// This supports the format:
///   - type: command_match, pattern: "..." (struct format with named field)
///   - type: tool_category, value: "read" (simple value format)
///   - type: path_prefix, value: "crates/" (simple value format)
///   - type: path_prefix, paths: ["a/", "b/"] (array format)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
#[serde(rename_all = "snake_case")]
pub enum PolicyConditionAlt {
    /// Match by command pattern (regex) - struct format: pattern: "..."
    CommandMatch { pattern: String },
    /// Match by tool name - struct format: name: "..."
    ToolName { name: String },
    /// Match by tool category - simple format: value: "read"
    ToolCategory { value: String },
    /// Match by file path prefix - can be value: "..." or paths: ["...", "..."]
    #[serde(alias = "paths")]
    PathPrefix { #[serde(flatten)] value: PathPrefixValue },
    /// Match by file path suffix (extension)
    PathSuffix { suffixes: Vec<String> },
    /// Match by exact path
    PathExact { path: String },
    /// Match by risk level
    RiskLevel { level: String },
    /// Match by agent role
    AgentRole { role: String },
    /// Always match (for default rules)
    Always,
}

/// Helper for path_prefix value which can be a string or object with paths array
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PathPrefixValue {
    /// Single path prefix string
    Single(String),
    /// Array of path prefixes
    Array { paths: Vec<String> },
}

impl PolicyConditionAlt {
    /// Convert to standard PolicyCondition
    pub fn to_policy_condition(&self) -> PolicyCondition {
        match self {
            PolicyConditionAlt::CommandMatch { pattern } => {
                PolicyCondition::CommandMatch { pattern: pattern.clone() }
            }
            PolicyConditionAlt::ToolName { name } => PolicyCondition::ToolName { name: name.clone() },
            PolicyConditionAlt::ToolCategory { value } => {
                PolicyCondition::ToolCategory {
                    category: match value.as_str() {
                        "read" => ToolCategory::Read,
                        "write" => ToolCategory::Write,
                        "destructive" => ToolCategory::Destructive,
                        _ => ToolCategory::Read, // Default to read for unknown categories
                    },
                }
            }
            PolicyConditionAlt::PathPrefix { value } => {
                let paths = match value {
                    PathPrefixValue::Single(s) => vec![s.clone()],
                    PathPrefixValue::Array { paths } => paths.clone(),
                };
                PolicyCondition::PathPrefix { paths }
            }
            PolicyConditionAlt::PathSuffix { suffixes } => {
                PolicyCondition::PathSuffix { suffixes: suffixes.clone() }
            }
            PolicyConditionAlt::PathExact { path } => PolicyCondition::PathExact { path: path.clone() },
            PolicyConditionAlt::RiskLevel { level } => {
                PolicyCondition::RiskLevel {
                    level: match level.as_str() {
                        "low" => RiskLevelMatch::Low,
                        "medium" => RiskLevelMatch::Medium,
                        "high" => RiskLevelMatch::High,
                        _ => RiskLevelMatch::Low,
                    },
                }
            }
            PolicyConditionAlt::AgentRole { role } => {
                PolicyCondition::AgentRole { role: role.clone() }
            }
            PolicyConditionAlt::Always => PolicyCondition::Always,
        }
    }
}

/// Risk level for matching
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevelMatch {
    Low,
    Medium,
    High,
}

/// Tool category
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCategory {
    Read,
    Write,
    Destructive,
}

/// Effect of a policy rule
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyEffect {
    Allow,
    Deny,
}

/// A single policy rule
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyRule {
    /// Optional ID for the rule (e.g., "deny-rm-rf-root")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Human-readable name for the rule
    pub name: String,
    /// The effect of this rule (allow or deny)
    pub effect: PolicyEffect,
    /// The condition that triggers this rule (single condition, legacy format)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition: Option<PolicyCondition>,
    /// Conditions that all must match for the rule to apply (array format)
    /// When multiple conditions are specified, ALL must match (AND logic)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conditions: Vec<PolicyConditionAlt>,
    /// Optional description explaining the rule
    #[serde(default)]
    pub description: Option<String>,
    /// Optional priority (higher numbers evaluated first)
    #[serde(default = "default_priority")]
    pub priority: i32,
}

fn default_priority() -> i32 {
    0
}

/// Policy file format
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyFile {
    /// Policy version for compatibility
    pub version: String,
    /// Default effect when no rule matches
    #[serde(default = "default_default_effect")]
    pub default_effect: PolicyEffect,
    /// List of policy rules evaluated in order
    pub rules: Vec<PolicyRule>,
}

fn default_default_effect() -> PolicyEffect {
    PolicyEffect::Deny
}

/// Represents an action to be evaluated by the policy engine
#[derive(Debug, Clone)]
pub struct PolicyAction {
    /// Type of action
    pub action_type: ActionType,
    /// The command/operation being performed (if applicable)
    pub command: Option<String>,
    /// The tool being used (if applicable)
    pub tool_name: Option<String>,
    /// The tool category (if applicable)
    pub tool_category: Option<ToolCategory>,
    /// File paths involved (if applicable)
    pub paths: Vec<String>,
    /// Risk level of the action
    pub risk_level: Option<RiskLevelMatch>,
    /// Agent role performing the action
    pub agent_role: Option<String>,
    /// Additional context as key-value pairs
    pub context: HashMap<String, String>,
}

impl PolicyAction {
    /// Create a new action with a command (e.g., shell command)
    pub fn command(cmd: String) -> Self {
        Self {
            action_type: ActionType::Command,
            command: Some(cmd),
            tool_name: None,
            tool_category: None,
            paths: Vec::new(),
            risk_level: None,
            agent_role: None,
            context: HashMap::new(),
        }
    }

    /// Create a new action with a tool invocation
    pub fn tool(name: String, category: ToolCategory, paths: Vec<String>) -> Self {
        Self {
            action_type: ActionType::Tool,
            command: None,
            tool_name: Some(name),
            tool_category: Some(category),
            paths,
            risk_level: None,
            agent_role: None,
            context: HashMap::new(),
        }
    }

    /// Create a file access action
    pub fn file_access(paths: Vec<String>) -> Self {
        Self {
            action_type: ActionType::FileAccess,
            command: None,
            tool_name: None,
            tool_category: None,
            paths,
            risk_level: None,
            agent_role: None,
            context: HashMap::new(),
        }
    }

    /// Set the risk level
    pub fn with_risk_level(mut self, level: RiskLevelMatch) -> Self {
        self.risk_level = Some(level);
        self
    }

    /// Set the agent role
    pub fn with_agent_role(mut self, role: String) -> Self {
        self.agent_role = Some(role);
        self
    }

    /// Add context
    pub fn with_context(mut self, key: String, value: String) -> Self {
        self.context.insert(key, value);
        self
    }
}

/// Type of action being performed
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionType {
    /// A shell command
    Command,
    /// A tool invocation
    Tool,
    /// File system access
    FileAccess,
    /// Agent registration or management
    AgentManagement,
    /// Task lifecycle action
    TaskLifecycle,
}

/// The policy engine that evaluates actions against policies
#[derive(Debug, Clone)]
pub struct PolicyEngine {
    /// The loaded policy
    policy: Option<PolicyFile>,
    /// Cached regex patterns for performance
    compiled_patterns: HashMap<String, regex::Regex>,
}

impl PolicyEngine {
    /// Create a new policy engine without a loaded policy
    pub fn new() -> Self {
        Self {
            policy: None,
            compiled_patterns: HashMap::new(),
        }
    }

    /// Create a policy engine with a loaded policy
    pub fn with_policy(policy: PolicyFile) -> Result<Self, PolicyError> {
        let mut engine = Self::new();
        engine.load_policy(policy)?;
        Ok(engine)
    }

    /// Load a policy from a PolicyFile
    pub fn load_policy(&mut self, policy: PolicyFile) -> Result<(), PolicyError> {
        // Validate the policy
        Self::validate_policy(&policy)?;

        // Pre-compile regex patterns
        self.compiled_patterns.clear();
        for rule in &policy.rules {
            // Legacy single condition format
            if let Some(PolicyCondition::CommandMatch { pattern }) = &rule.condition {
                match regex::Regex::new(pattern) {
                    Ok(re) => {
                        self.compiled_patterns.insert(rule.name.clone(), re);
                    }
                    Err(e) => {
                        return Err(PolicyError::InvalidRule(format!(
                            "Invalid regex pattern '{}' in rule '{}': {}",
                            pattern, rule.name, e
                        )));
                    }
                }
            }
            // New conditions array format
            for cond in &rule.conditions {
                if let PolicyConditionAlt::CommandMatch { pattern } = cond {
                    match regex::Regex::new(pattern) {
                        Ok(re) => {
                            self.compiled_patterns.insert(format!("{}_{}", rule.name, pattern), re);
                        }
                        Err(e) => {
                            return Err(PolicyError::InvalidRule(format!(
                                "Invalid regex pattern '{}' in rule '{}': {}",
                                pattern, rule.name, e
                            )));
                        }
                    }
                }
            }
        }

        self.policy = Some(policy);
        info!("Policy loaded successfully");
        Ok(())
    }

    /// Load policy from a YAML string
    pub fn load_from_yaml(&mut self, yaml: &str) -> Result<(), PolicyError> {
        let policy: PolicyFile =
            serde_yaml::from_str(yaml).map_err(|e| PolicyError::ParseError(e.to_string()))?;
        self.load_policy(policy)
    }

    /// Load policy from a YAML file
    pub fn load_from_file<P: AsRef<Path>>(&mut self, path: P) -> Result<(), PolicyError> {
        let content =
            std::fs::read_to_string(path).map_err(|e| PolicyError::LoadError(e.to_string()))?;
        self.load_from_yaml(&content)
    }

    /// Validate a policy for correctness
    fn validate_policy(policy: &PolicyFile) -> Result<(), PolicyError> {
        if policy.rules.is_empty() {
            warn!("Policy has no rules - all actions will use default_effect");
        }

        for rule in &policy.rules {
            if rule.name.is_empty() {
                return Err(PolicyError::InvalidRule("Rule name cannot be empty".into()));
            }
        }

        Ok(())
    }

    /// Check if a policy is loaded
    pub fn is_loaded(&self) -> bool {
        self.policy.is_some()
    }

    /// Evaluate an action against the loaded policy using deny-first semantics.
    ///
    /// Deny-first evaluation order:
    /// 1. Find all matching rules
    /// 2. If ANY rule denies, the action is denied (deny takes precedence)
    /// 3. If ANY rule allows AND no rule denies, the action is allowed
    /// 4. If NO rule matches, use the default_effect from the policy
    ///
    /// Returns the decision and the rule that caused it (if any).
    pub fn evaluate(&self, action: &PolicyAction) -> (PolicyDecision, Option<String>) {
        let policy = match &self.policy {
            Some(p) => p,
            None => {
                debug!("No policy loaded, using default deny");
                return (PolicyDecision::Deny, None);
            }
        };

        // Collect matching rules and their effects
        let mut has_deny = false;
        let mut has_allow = false;
        let mut deny_rule: Option<&PolicyRule> = None;
        let mut allow_rule: Option<&PolicyRule> = None;

        // Sort rules by priority (higher first), then evaluate
        let mut sorted_rules: Vec<&PolicyRule> = policy.rules.iter().collect();
        sorted_rules.sort_by(|a, b| b.priority.cmp(&a.priority));

        for rule in sorted_rules {
            if self.rule_matches(rule, action) {
                debug!(rule = %rule.name, effect = ?rule.effect, "Rule matched");
                match rule.effect {
                    PolicyEffect::Deny => {
                        has_deny = true;
                        deny_rule = Some(rule);
                        // With deny-first, we could break here, but we continue
                        // to log all matching rules for debugging
                    }
                    PolicyEffect::Allow => {
                        has_allow = true;
                        allow_rule = Some(rule);
                    }
                }
            }
        }

        // Deny-first: if any rule denies, the action is denied
        if has_deny {
            info!(
                action_type = ?action.action_type,
                rule = ?deny_rule.map(|r| r.name.as_str()),
                "Action denied by policy"
            );
            return (PolicyDecision::Deny, deny_rule.map(|r| r.name.clone()));
        }

        // If no rule denies but at least one allows, the action is allowed
        if has_allow {
            info!(
                action_type = ?action.action_type,
                rule = ?allow_rule.map(|r| r.name.as_str()),
                "Action allowed by policy"
            );
            return (PolicyDecision::Allow, allow_rule.map(|r| r.name.clone()));
        }

        // No rule matched - use default effect
        let default = match policy.default_effect {
            PolicyEffect::Deny => {
                debug!("No rule matched, using default deny");
                PolicyDecision::Deny
            }
            PolicyEffect::Allow => {
                debug!("No rule matched, using default allow");
                PolicyDecision::Allow
            }
        };

        (default, None)
    }

    /// Shorthand for evaluating and just getting the decision
    pub fn evaluate_decision(&self, action: &PolicyAction) -> PolicyDecision {
        self.evaluate(action).0
    }

    /// Check if an action is allowed
    pub fn is_allowed(&self, action: &PolicyAction) -> bool {
        self.evaluate(action).0.is_allowed()
    }

    /// Check if an action is denied
    pub fn is_denied(&self, action: &PolicyAction) -> bool {
        self.evaluate(action).0.is_denied()
    }

    /// Check if all conditions in a rule match the action (AND logic)
    /// Supports both single condition (legacy) and conditions array (new YAML format)
    fn rule_matches(&self, rule: &PolicyRule, action: &PolicyAction) -> bool {
        // Check single condition (legacy format)
        if let Some(condition) = &rule.condition {
            return self.condition_matches(condition, action);
        }

        // Check conditions array (new YAML format)
        // All conditions must match for the rule to apply (AND logic)
        if rule.conditions.is_empty() {
            // No conditions specified - rule never matches
            return false;
        }

        for cond in &rule.conditions {
            let std_cond = cond.to_policy_condition();
            if !self.condition_matches(&std_cond, action) {
                return false;
            }
        }
        true
    }

    /// Evaluate a condition against an action
    fn condition_matches(&self, condition: &PolicyCondition, action: &PolicyAction) -> bool {
        match condition {
            // Always matches everything (used for catch-all rules)
            PolicyCondition::Always => true,

            PolicyCondition::CommandMatch { pattern } => {
                if let Some(cmd) = &action.command {
                    if let Some(regex) = self.compiled_patterns.get(&format!("cmd_{}", pattern)) {
                        regex.is_match(cmd)
                    } else {
                        // Fall back to regex on the pattern directly
                        regex::Regex::new(pattern)
                            .map(|re| re.is_match(cmd))
                            .unwrap_or(false)
                    }
                } else {
                    false
                }
            }

            PolicyCondition::ToolName { name } => action
                .tool_name
                .as_ref()
                .map(|n| n == name)
                .unwrap_or(false),

            PolicyCondition::ToolCategory { category } => action
                .tool_category
                .map(|c| c == *category)
                .unwrap_or(false),

            PolicyCondition::PathPrefix { paths } => action
                .paths
                .iter()
                .any(|p| paths.iter().any(|prefix| p.starts_with(prefix))),

            PolicyCondition::PathSuffix { suffixes } => action
                .paths
                .iter()
                .any(|p| suffixes.iter().any(|suffix| p.ends_with(suffix))),

            PolicyCondition::PathExact { path } => action.paths.iter().any(|p| p == path),

            PolicyCondition::RiskLevel { level } => {
                action.risk_level.map(|r| r == *level).unwrap_or(false)
            }

            PolicyCondition::AgentRole { role } => action
                .agent_role
                .as_ref()
                .map(|r| r == role)
                .unwrap_or(false),
        }
    }

    /// Get the loaded policy (if any)
    pub fn get_policy(&self) -> Option<&PolicyFile> {
        self.policy.as_ref()
    }
}

impl Default for PolicyEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// Builder for creating PolicyActions more conveniently
pub mod action {
    use super::*;

    /// Create a command action
    pub fn command<C: Into<String>>(cmd: C) -> PolicyAction {
        PolicyAction::command(cmd.into())
    }

    /// Create a tool action
    pub fn tool<N: Into<String>>(name: N, category: ToolCategory) -> PolicyAction {
        PolicyAction::tool(name.into(), category, Vec::new())
    }

    /// Create a tool action with paths
    pub fn tool_with_paths<N: Into<String>>(
        name: N,
        category: ToolCategory,
        paths: Vec<String>,
    ) -> PolicyAction {
        PolicyAction::tool(name.into(), category, paths)
    }

    /// Create a file access action
    pub fn file_access<P: Into<String>>(path: P) -> PolicyAction {
        PolicyAction::file_access(vec![path.into()])
    }

    /// Create a file access action with multiple paths
    pub fn file_access_many<I, P>(paths: I) -> PolicyAction
    where
        I: IntoIterator<Item = P>,
        P: Into<String>,
    {
        PolicyAction::file_access(paths.into_iter().map(|p| p.into()).collect())
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_policy() -> PolicyFile {
        PolicyFile {
            version: "1.0".to_string(),
            default_effect: PolicyEffect::Deny,
            rules: vec![
                PolicyRule {
                    id: None,
                    name: "deny dangerous commands".to_string(),
                    effect: PolicyEffect::Deny,
                    condition: Some(PolicyCondition::CommandMatch {
                        pattern: r"(rm -rf|DROP TABLE|TRUNCATE)".to_string(),
                    }),
                    conditions: vec![],
                    description: Some("Block dangerous shell commands".to_string()),
                    priority: 100,
                },
                PolicyRule {
                    id: None,
                    name: "allow read tools".to_string(),
                    effect: PolicyEffect::Allow,
                    condition: Some(PolicyCondition::ToolCategory {
                        category: ToolCategory::Read,
                    }),
                    conditions: vec![],
                    description: Some("Allow all read operations".to_string()),
                    priority: 50,
                },
                PolicyRule {
                    id: None,
                    name: "allow workspace files".to_string(),
                    effect: PolicyEffect::Allow,
                    condition: Some(PolicyCondition::PathPrefix {
                        paths: vec!["/workspace/src".to_string(), "/workspace/tests".to_string()],
                    }),
                    conditions: vec![],
                    description: Some("Allow access to workspace files".to_string()),
                    priority: 10,
                },
                PolicyRule {
                    id: None,
                    name: "deny high risk".to_string(),
                    effect: PolicyEffect::Deny,
                    condition: Some(PolicyCondition::RiskLevel {
                        level: RiskLevelMatch::High,
                    }),
                    conditions: vec![],
                    description: Some("Block high risk operations".to_string()),
                    priority: 80,
                },
            ],
        }
    }

    #[test]
    fn test_policy_engine_load_from_yaml() {
        let yaml = r#"
version: "1.0"
default_effect: deny
rules:
  - name: "test rule"
    effect: allow
    condition:
      type: always
"#;
        let mut engine = PolicyEngine::new();
        engine.load_from_yaml(yaml).unwrap();
        assert!(engine.is_loaded());
    }

    #[test]
    fn test_deny_dangerous_command() {
        let policy = create_test_policy();
        let engine = PolicyEngine::with_policy(policy).unwrap();

        // rm -rf should be denied
        let action = PolicyAction::command("rm -rf /workspace".to_string());
        let (decision, rule) = engine.evaluate(&action);
        assert_eq!(decision, PolicyDecision::Deny);
        assert_eq!(rule, Some("deny dangerous commands".to_string()));

        // DROP TABLE should be denied
        let action = PolicyAction::command("DROP TABLE users".to_string());
        let (decision, rule) = engine.evaluate(&action);
        assert_eq!(decision, PolicyDecision::Deny);
        assert_eq!(rule, Some("deny dangerous commands".to_string()));
    }

    #[test]
    fn test_allow_read_tool() {
        let policy = create_test_policy();
        let engine = PolicyEngine::with_policy(policy).unwrap();

        let action = PolicyAction::tool("file_read".to_string(), ToolCategory::Read, vec![]);
        let (decision, rule) = engine.evaluate(&action);
        assert_eq!(decision, PolicyDecision::Allow);
        assert_eq!(rule, Some("allow read tools".to_string()));
    }

    #[test]
    fn test_allow_workspace_path() {
        let policy = create_test_policy();
        let engine = PolicyEngine::with_policy(policy).unwrap();

        let action = PolicyAction::file_access(vec!["/workspace/src/main.rs".to_string()]);
        let (decision, rule) = engine.evaluate(&action);
        assert_eq!(decision, PolicyDecision::Allow);
        assert_eq!(rule, Some("allow workspace files".to_string()));
    }

    #[test]
    fn test_deny_high_risk() {
        let policy = create_test_policy();
        let engine = PolicyEngine::with_policy(policy).unwrap();

        let action = PolicyAction::command("deploy production".to_string())
            .with_risk_level(RiskLevelMatch::High);
        let (decision, rule) = engine.evaluate(&action);
        assert_eq!(decision, PolicyDecision::Deny);
        assert_eq!(rule, Some("deny high risk".to_string()));
    }

    #[test]
    fn test_deny_first_semantics() {
        let yaml = r#"
version: "1.0"
default_effect: allow
rules:
  - name: "allow dangerous"
    effect: allow
    condition:
      type: command_match
      data:
        pattern: "rm -rf"
  - name: "deny dangerous"
    effect: deny
    condition:
      type: command_match
      data:
        pattern: "rm -rf"
"#;
        let mut engine = PolicyEngine::new();
        engine.load_from_yaml(yaml).unwrap();

        // Even though allow matches first (priority), deny should win
        let action = PolicyAction::command("rm -rf /workspace".to_string());
        let (decision, rule) = engine.evaluate(&action);
        assert_eq!(decision, PolicyDecision::Deny);
        assert_eq!(rule, Some("deny dangerous".to_string()));
    }

    #[test]
    fn test_default_effect_deny() {
        let policy = create_test_policy();
        let engine = PolicyEngine::with_policy(policy).unwrap();

        // Unknown action should be denied (default_effect is deny)
        let action = PolicyAction::command("echo hello".to_string());
        let (decision, rule) = engine.evaluate(&action);
        assert_eq!(decision, PolicyDecision::Deny);
        assert!(rule.is_none()); // No matching rule
    }

    #[test]
    fn test_default_effect_allow() {
        let yaml = r#"
version: "1.0"
default_effect: allow
rules: []
"#;
        let mut engine = PolicyEngine::new();
        engine.load_from_yaml(yaml).unwrap();

        // Unknown action should be allowed when default is allow
        let action = PolicyAction::command("echo hello".to_string());
        let (decision, _) = engine.evaluate(&action);
        assert_eq!(decision, PolicyDecision::Allow);
    }

    #[test]
    fn test_no_policy_loaded() {
        let engine = PolicyEngine::new();
        let action = PolicyAction::command("any command".to_string());
        let (decision, _) = engine.evaluate(&action);
        assert_eq!(decision, PolicyDecision::Deny);
    }

    #[test]
    fn test_path_exact_match() {
        let yaml = r#"
version: "1.0"
default_effect: deny
rules:
  - name: "allow specific file"
    effect: allow
    condition:
      type: path_exact
      data:
        path: "/workspace/Cargo.toml"
"#;
        let mut engine = PolicyEngine::new();
        engine.load_from_yaml(yaml).unwrap();

        let action = PolicyAction::file_access(vec!["/workspace/Cargo.toml".to_string()]);
        let (decision, _) = engine.evaluate(&action);
        assert_eq!(decision, PolicyDecision::Allow);

        let action = PolicyAction::file_access(vec!["/workspace/other.toml".to_string()]);
        let (decision, _) = engine.evaluate(&action);
        assert_eq!(decision, PolicyDecision::Deny);
    }

    #[test]
    fn test_path_suffix_match() {
        let yaml = r#"
version: "1.0"
default_effect: deny
rules:
  - name: "allow rust files"
    effect: allow
    condition:
      type: path_suffix
      data:
        suffixes: [".rs", ".toml"]
"#;
        let mut engine = PolicyEngine::new();
        engine.load_from_yaml(yaml).unwrap();

        let action = PolicyAction::file_access(vec!["/workspace/src/lib.rs".to_string()]);
        let (decision, _) = engine.evaluate(&action);
        assert_eq!(decision, PolicyDecision::Allow);

        let action = PolicyAction::file_access(vec!["/workspace/README.md".to_string()]);
        let (decision, _) = engine.evaluate(&action);
        assert_eq!(decision, PolicyDecision::Deny);
    }

    #[test]
    fn test_agent_role_match() {
        let yaml = r#"
version: "1.0"
default_effect: deny
rules:
  - name: "allow planner"
    effect: allow
    condition:
      type: agent_role
      data:
        role: "planner"
"#;
        let mut engine = PolicyEngine::new();
        engine.load_from_yaml(yaml).unwrap();

        let action =
            PolicyAction::command("plan task".to_string()).with_agent_role("planner".to_string());
        let (decision, _) = engine.evaluate(&action);
        assert_eq!(decision, PolicyDecision::Allow);

        let action = PolicyAction::command("generate code".to_string())
            .with_agent_role("generator".to_string());
        let (decision, _) = engine.evaluate(&action);
        assert_eq!(decision, PolicyDecision::Deny);
    }

    #[test]
    fn test_priority_ordering() {
        let yaml = r#"
version: "1.0"
default_effect: deny
rules:
  - name: "low priority allow"
    effect: allow
    condition:
      type: always
    priority: 10
  - name: "high priority deny"
    effect: deny
    condition:
      type: always
    priority: 100
"#;
        let mut engine = PolicyEngine::new();
        engine.load_from_yaml(yaml).unwrap();

        // High priority deny should win even though low priority allow also matches
        let action = PolicyAction::command("any command".to_string());
        let (decision, rule) = engine.evaluate(&action);
        assert_eq!(decision, PolicyDecision::Deny);
        assert_eq!(rule, Some("high priority deny".to_string()));
    }

    #[test]
    fn test_decision_helpers() {
        let yaml = r#"
version: "1.0"
default_effect: deny
rules:
  - name: "allow read"
    effect: allow
    condition:
      type: tool_category
      data:
        category: read
"#;
        let mut engine = PolicyEngine::new();
        engine.load_from_yaml(yaml).unwrap();

        let read_action = PolicyAction::tool("file_read".to_string(), ToolCategory::Read, vec![]);
        assert!(engine.is_allowed(&read_action));
        assert!(!engine.is_denied(&read_action));

        let write_action =
            PolicyAction::tool("file_write".to_string(), ToolCategory::Write, vec![]);
        assert!(!engine.is_allowed(&write_action));
        assert!(engine.is_denied(&write_action));
    }

    #[test]
    fn test_invalid_regex_in_policy() {
        let yaml = r#"
version: "1.0"
default_effect: deny
rules:
  - name: "bad regex"
    effect: allow
    condition:
      type: command_match
      data:
        pattern: "[invalid(regex"
"#;
        let mut engine = PolicyEngine::new();
        let result = engine.load_from_yaml(yaml);
        assert!(result.is_err());
    }

    #[test]
    fn test_policy_action_builder() {
        use super::action;

        let action = action::command("ls -la");
        assert_eq!(action.command, Some("ls -la".to_string()));

        let action = action::tool("file_read", ToolCategory::Read);
        assert_eq!(action.tool_name, Some("file_read".to_string()));
        assert_eq!(action.tool_category, Some(ToolCategory::Read));

        let action = action::file_access("/workspace/src/main.rs");
        assert_eq!(action.paths, vec!["/workspace/src/main.rs".to_string()]);

        let action = action::file_access_many(vec!["/a.rs", "/b.rs"]);
        assert_eq!(action.paths.len(), 2);
    }

    // =========================================================================
    // policy_gates tests - YAML policy loading with deny-first semantics
    // =========================================================================

    #[test]
    fn test_policy_gates_yaml_loading() {
        // Test loading from actual YAML format like default.yaml
        let yaml = r#"
version: "1.0"
default_effect: deny

rules:
  - id: deny-rm-rf
    name: deny rm -rf
    effect: deny
    priority: 100
    conditions:
      - type: command_match
        pattern: "rm\\s+-rf"

  - id: allow-read
    name: allow read operations
    effect: allow
    priority: 10
    conditions:
      - type: tool_category
        value: "read"
"#;
        let mut engine = PolicyEngine::new();
        engine.load_from_yaml(yaml).unwrap();
        assert!(engine.is_loaded());

        let policy = engine.get_policy().unwrap();
        assert_eq!(policy.version, "1.0");
        assert_eq!(policy.default_effect, PolicyEffect::Deny);
        assert_eq!(policy.rules.len(), 2);
    }

    #[test]
    fn test_policy_gates_deny_first_semantics() {
        // Test that deny takes precedence over allow even when both match
        let yaml = r#"
version: "1.0"
default_effect: allow

rules:
  - id: allow-all
    name: allow all
    effect: allow
    priority: 10
    conditions:
      - type: always

  - id: deny-all
    name: deny all
    effect: deny
    priority: 100
    conditions:
      - type: always
"#;
        let mut engine = PolicyEngine::new();
        engine.load_from_yaml(yaml).unwrap();

        let action = PolicyAction::command("any command".to_string());
        let (decision, rule) = engine.evaluate(&action);
        assert_eq!(decision, PolicyDecision::Deny);
        assert_eq!(rule, Some("deny all".to_string()));
    }

    #[test]
    fn test_policy_gates_multi_condition_and_logic() {
        // Test that multiple conditions use AND logic - all must match
        let yaml = r#"
version: "1.0"
default_effect: deny

rules:
  - id: allow-workspace-read
    name: allow workspace read
    effect: allow
    priority: 50
    conditions:
      - type: tool_category
        value: "read"
      - type: path_prefix
        paths: ["/workspace/"]
"#;
        let mut engine = PolicyEngine::new();
        engine.load_from_yaml(yaml).unwrap();

        // Read tool + workspace path = allowed
        let action = PolicyAction::tool(
            "file_read".to_string(),
            ToolCategory::Read,
            vec!["/workspace/src/main.rs".to_string()],
        );
        let (decision, _) = engine.evaluate(&action);
        assert_eq!(decision, PolicyDecision::Allow);

        // Read tool but non-workspace path = denied (second condition fails)
        let action = PolicyAction::tool(
            "file_read".to_string(),
            ToolCategory::Read,
            vec!["/etc/passwd".to_string()],
        );
        let (decision, _) = engine.evaluate(&action);
        assert_eq!(decision, PolicyDecision::Deny);

        // Write tool + workspace path = denied (first condition fails)
        let action = PolicyAction::tool(
            "file_write".to_string(),
            ToolCategory::Write,
            vec!["/workspace/src/main.rs".to_string()],
        );
        let (decision, _) = engine.evaluate(&action);
        assert_eq!(decision, PolicyDecision::Deny);
    }

    #[test]
    fn test_policy_gates_enforcement_on_operations() {
        // Test that policy is actually enforced on operations
        let yaml = r#"
version: "1.0"
default_effect: deny

rules:
  - id: deny-dangerous
    name: deny dangerous commands
    effect: deny
    priority: 100
    conditions:
      - type: command_match
        pattern: "(rm -rf|DROP|DESTROY)"

  - id: allow-safe
    name: allow safe commands
    effect: allow
    priority: 10
    conditions:
      - type: command_match
        pattern: "(ls|echo|pwd)"
"#;
        let mut engine = PolicyEngine::new();
        engine.load_from_yaml(yaml).unwrap();

        // Dangerous command should be denied
        let dangerous_action = PolicyAction::command("rm -rf /workspace".to_string());
        assert!(engine.is_denied(&dangerous_action));
        assert!(!engine.is_allowed(&dangerous_action));

        // Safe command should be allowed
        let safe_action = PolicyAction::command("ls -la".to_string());
        assert!(engine.is_allowed(&safe_action));
        assert!(!engine.is_denied(&safe_action));
    }

    #[test]
    fn test_policy_gates_default_effect_deny() {
        // Test that default deny works when no rule matches
        // This test creates a rule that matches "echo" commands but NOT other commands
        // so we can verify that unmatched commands get default deny
        let yaml = r#"
version: "1.0"
default_effect: deny

rules:
  - id: allow-echo
    name: allow echo
    effect: allow
    priority: 10
    conditions:
      - type: command_match
        pattern: "^echo "
"#;
        let mut engine = PolicyEngine::new();
        engine.load_from_yaml(yaml).unwrap();

        // Echo command should be allowed (rule matches)
        let echo_action = PolicyAction::command("echo hello".to_string());
        let (decision, _) = engine.evaluate(&echo_action);
        assert_eq!(decision, PolicyDecision::Allow);

        // Unknown command should be denied (no rule matched, default deny)
        let unknown_action = PolicyAction::command("unknown command".to_string());
        let (decision, rule) = engine.evaluate(&unknown_action);
        assert_eq!(decision, PolicyDecision::Deny);
        assert!(rule.is_none()); // No rule matched
    }

    #[test]
    fn test_policy_gates_path_prefix_matching() {
        // Test path prefix matching from actual default.yaml
        let yaml = r#"
version: "1.0"
default_effect: deny

rules:
  - id: allow-workspace
    name: allow workspace files
    effect: allow
    priority: 20
    conditions:
      - type: path_prefix
        paths: ["crates/", "src/", "tests/"]
"#;
        let mut engine = PolicyEngine::new();
        engine.load_from_yaml(yaml).unwrap();

        // Files under crates/ should be allowed
        let action = PolicyAction::file_access(vec!["crates/swell-core/src/lib.rs".to_string()]);
        assert!(engine.is_allowed(&action));

        // Files not under allowed paths should be denied
        let action = PolicyAction::file_access(vec!["etc/config.json".to_string()]);
        assert!(engine.is_denied(&action));
    }

    #[test]
    fn test_policy_gates_priority_ordering() {
        // Test that higher priority rules take precedence
        let yaml = r#"
version: "1.0"
default_effect: deny

rules:
  - id: low-priority-allow
    name: low priority allow
    effect: allow
    priority: 10
    conditions:
      - type: always

  - id: high-priority-deny
    name: high priority deny
    effect: deny
    priority: 100
    conditions:
      - type: always
"#;
        let mut engine = PolicyEngine::new();
        engine.load_from_yaml(yaml).unwrap();

        let action = PolicyAction::command("test".to_string());
        let (decision, rule) = engine.evaluate(&action);
        assert_eq!(decision, PolicyDecision::Deny);
        assert_eq!(rule, Some("high priority deny".to_string()));
    }

    #[test]
    fn test_policy_gates_unknown_action_default_deny() {
        // Test that unknown actions use default effect
        let engine = PolicyEngine::new(); // No policy loaded

        let action = PolicyAction::command("any command".to_string());
        let (decision, _) = engine.evaluate(&action);
        assert_eq!(decision, PolicyDecision::Deny); // Default deny when no policy loaded
    }
}
