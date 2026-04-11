//! Open Policy Agent (OPA) integration for infrastructure-level policy enforcement.
//!
//! OPA is an open-source policy engine that uses Rego policy language.
//! It provides:
//! - Policy as code approach
//! - Infrastructure-level access control
//! - Declarative policy evaluation
//! - Rich context matching for authorization decisions
//!
//! # Architecture
//!
//! This module provides:
//! - [`OpaPolicyEngine`] - OPA client for policy evaluation
//! - [`OpaInput`] - Input document structure for OPA evaluation
//! - [`OpaResult`] - Authorization result from OPA
//! - [`OpaClient`] - HTTP client for OPA server or WASM module
//!
//! Note: This module reuses [`ToolOperation`] from [`crate::cedar_policy`] for consistency.

use crate::cedar_policy::ToolOperation;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;
use thiserror::Error;
use tracing::debug;

/// OPA policy engine errors
#[derive(Error, Debug, Clone)]
pub enum OpaError {
    #[error("OPA server connection failed: {0}")]
    ConnectionError(String),

    #[error("Policy evaluation failed: {0}")]
    EvaluationError(String),

    #[error("Invalid input document: {0}")]
    InvalidInput(String),

    #[error("Policy not found: {0}")]
    PolicyNotFound(String),

    #[error("OPA returned error: {0}")]
    OpaServerError(String),

    #[error("WASM module error: {0}")]
    WasmError(String),

    #[error("No policies loaded")]
    NoPoliciesLoaded,
}

/// Result of an OPA policy evaluation
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OpaDecision {
    /// Action is permitted by policy
    Allow,
    /// Action is denied by policy
    Deny,
    /// Decision is not applicable (no matching policies)
    NotApplicable,
}

impl OpaDecision {
    /// Returns true if the action is allowed by policy
    pub fn is_allowed(&self) -> bool {
        matches!(self, OpaDecision::Allow)
    }

    /// Returns true if the action is denied by policy
    pub fn is_denied(&self) -> bool {
        matches!(self, OpaDecision::Deny)
    }
}

/// Risk level for tool operations
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum OpaRiskLevel {
    /// Low risk operations (read-only, no side effects)
    Low,
    /// Medium risk operations (write operations, some side effects)
    Medium,
    /// High risk operations (destructive, system-level changes)
    High,
}

impl std::fmt::Display for OpaRiskLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpaRiskLevel::Low => write!(f, "low"),
            OpaRiskLevel::Medium => write!(f, "medium"),
            OpaRiskLevel::High => write!(f, "high"),
        }
    }
}

/// Input document sent to OPA for evaluation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpaInput {
    /// Subject performing the action
    pub subject: OpaSubject,
    /// Action being performed
    pub action: OpaAction,
    /// Resource being accessed
    pub resource: OpaResource,
    /// Contextual information for policy evaluation
    pub context: HashMap<String, serde_json::Value>,
}

impl OpaInput {
    /// Create a new OPA input document
    pub fn new(subject: OpaSubject, action: OpaAction, resource: OpaResource) -> Self {
        Self {
            subject,
            action,
            resource,
            context: HashMap::new(),
        }
    }

    /// Add context to the input document
    pub fn with_context(mut self, key: impl Into<String>, value: serde_json::Value) -> Self {
        self.context.insert(key.into(), value);
        self
    }
}

/// Subject information for OPA input
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpaSubject {
    /// Agent ID performing the action
    pub agent_id: String,
    /// Role of the agent
    pub role: String,
    /// Risk level of the current operation
    pub operation_risk: String,
}

impl OpaSubject {
    /// Create a new subject
    pub fn new(
        agent_id: &str,
        role: &str,
        operation_risk: crate::cedar_policy::CedarRiskLevel,
    ) -> Self {
        Self {
            agent_id: agent_id.to_string(),
            role: role.to_string(),
            operation_risk: operation_risk.to_string(),
        }
    }
}

/// Action information for OPA input
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpaAction {
    /// Type of action (read, write, edit, shell, git, search)
    pub action_type: String,
    /// Additional action parameters
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<HashMap<String, String>>,
}

impl OpaAction {
    /// Create a new action from a tool operation
    pub fn from_operation(op: &ToolOperation) -> Self {
        let action_type = op.operation_type().to_string();
        let params = match op {
            ToolOperation::Read { path }
            | ToolOperation::Write { path }
            | ToolOperation::Edit { path } => Some(HashMap::from([(
                "path".to_string(),
                path.display().to_string(),
            )])),
            ToolOperation::Shell { command } => {
                Some(HashMap::from([("command".to_string(), command.clone())]))
            }
            ToolOperation::Git { operation } => Some(HashMap::from([(
                "operation".to_string(),
                operation.clone(),
            )])),
            ToolOperation::Search { operation } => Some(HashMap::from([(
                "operation".to_string(),
                operation.clone(),
            )])),
            ToolOperation::ReadOnly { tool_name } | ToolOperation::Destructive { tool_name } => {
                Some(HashMap::from([(
                    "tool_name".to_string(),
                    tool_name.clone(),
                )]))
            }
        };
        Self {
            action_type,
            params,
        }
    }
}

/// Resource information for OPA input
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpaResource {
    /// Type of resource (file, command, git, search)
    pub resource_type: String,
    /// Resource identifier/path
    pub path: String,
    /// Ownership information
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
}

impl OpaResource {
    /// Create a new resource from a tool operation
    pub fn from_operation(op: &ToolOperation) -> Self {
        let (resource_type, path) = match op {
            ToolOperation::Read { path }
            | ToolOperation::Write { path }
            | ToolOperation::Edit { path } => ("file".to_string(), path.display().to_string()),
            ToolOperation::Shell { command } => ("command".to_string(), command.clone()),
            ToolOperation::Git { operation } => ("git".to_string(), operation.clone()),
            ToolOperation::Search { operation } => ("search".to_string(), operation.clone()),
            ToolOperation::ReadOnly { tool_name } | ToolOperation::Destructive { tool_name } => {
                ("tool".to_string(), tool_name.clone())
            }
        };
        Self {
            resource_type,
            path,
            owner: None,
        }
    }
}

/// OPA policy engine for tool access control
#[derive(Debug, Clone)]
pub struct OpaPolicyEngine {
    /// HTTP client for OPA server
    client: Client,
    /// OPA server URL
    server_url: String,
    /// Policy bundle path (for local evaluation)
    bundle_path: Option<String>,
    /// Decision path in OPA (default: "result")
    decision_path: String,
}

impl OpaPolicyEngine {
    /// Create a new OPA policy engine with server URL
    pub fn new(server_url: &str) -> Self {
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(10))
                .build()
                .expect("OPA client builder should not fail"),
            server_url: server_url.to_string(),
            bundle_path: None,
            decision_path: "result".to_string(),
        }
    }

    /// Create with a local bundle path (for offline evaluation)
    pub fn with_bundle_path(mut self, bundle_path: &str) -> Self {
        self.bundle_path = Some(bundle_path.to_string());
        self
    }

    /// Set the decision path (default: "result")
    pub fn with_decision_path(mut self, path: &str) -> Self {
        self.decision_path = path.to_string();
        self
    }

    /// Evaluate an authorization request against OPA policies
    pub async fn evaluate(&self, input: &OpaInput) -> Result<OpaDecision, OpaError> {
        // If we have a bundle path, we could use WASM evaluation
        // For now, we use the HTTP API
        if self.bundle_path.is_some() {
            return Err(OpaError::WasmError(
                "WASM evaluation not yet implemented".to_string(),
            ));
        }

        let url = format!("{}/v1/data{}", self.server_url, self.decision_path);

        debug!(
            server = %self.server_url,
            decision_path = %self.decision_path,
            "Evaluating policy with OPA"
        );

        let response = self
            .client
            .post(&url)
            .json(input)
            .send()
            .await
            .map_err(|e| OpaError::ConnectionError(e.to_string()))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(OpaError::OpaServerError(format!(
                "OPA returned {}: {}",
                status, body
            )));
        }

        let result: serde_json::Value = response
            .json()
            .await
            .map_err(|e| OpaError::EvaluationError(e.to_string()))?;

        // OPA returns { "result": true/false } for boolean decisions
        let decision = if let Some(result) = result.get("result") {
            match result {
                serde_json::Value::Bool(true) => OpaDecision::Allow,
                serde_json::Value::Bool(false) => OpaDecision::Deny,
                serde_json::Value::Null | serde_json::Value::Object(_) => {
                    // For object results, check for allow/deny fields
                    if let Some(obj) = result.as_object() {
                        if obj.contains_key("allow") {
                            if obj["allow"].as_bool() == Some(true) {
                                OpaDecision::Allow
                            } else {
                                OpaDecision::Deny
                            }
                        } else if obj.contains_key("deny") {
                            if obj["deny"].as_bool() == Some(true) {
                                OpaDecision::Deny
                            } else {
                                OpaDecision::Allow
                            }
                        } else {
                            OpaDecision::NotApplicable
                        }
                    } else {
                        OpaDecision::NotApplicable
                    }
                }
                _ => OpaDecision::NotApplicable,
            }
        } else {
            OpaDecision::NotApplicable
        };

        debug!(
            decision = ?decision,
            "OPA evaluation complete"
        );

        Ok(decision)
    }

    /// Check if a tool operation is allowed
    pub async fn is_allowed(
        &self,
        agent_id: &str,
        role: &str,
        operation: &ToolOperation,
    ) -> Result<bool, OpaError> {
        let subject = OpaSubject::new(agent_id, role, operation.risk_level());
        let action = OpaAction::from_operation(operation);
        let resource = OpaResource::from_operation(operation);

        let input = OpaInput::new(subject, action, resource);

        let decision = self.evaluate(&input).await?;
        Ok(matches!(decision, OpaDecision::Allow))
    }

    /// Get health status of OPA server
    pub async fn healthcheck(&self) -> Result<bool, OpaError> {
        let url = format!("{}/health", self.server_url);

        let response = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| OpaError::ConnectionError(e.to_string()))?;

        Ok(response.status().is_success())
    }

    /// Check if the engine is configured for bundle (offline) mode
    pub fn is_bundle_mode(&self) -> bool {
        self.bundle_path.is_some()
    }
}

/// OPA client for managing policies and bundles
#[derive(Debug, Clone)]
pub struct OpaClient {
    /// HTTP client
    client: Client,
    /// Base URL for OPA server
    base_url: String,
}

impl OpaClient {
    /// Create a new OPA client
    pub fn new(server_url: &str) -> Self {
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .expect("OPA client builder should not fail"),
            base_url: server_url.to_string(),
        }
    }

    /// Load policies from a bundle file
    pub async fn load_bundle(&self, bundle_path: &Path) -> Result<(), OpaError> {
        if !bundle_path.exists() {
            return Err(OpaError::PolicyNotFound(bundle_path.display().to_string()));
        }

        let url = format!("{}/v1/bundles/{}", self.base_url, bundle_path.display());

        let response = self
            .client
            .put(&url)
            .send()
            .await
            .map_err(|e| OpaError::ConnectionError(e.to_string()))?;

        if !response.status().is_success() {
            return Err(OpaError::OpaServerError(format!(
                "Failed to load bundle: {}",
                response.status()
            )));
        }

        Ok(())
    }

    /// Check if OPA server is reachable
    pub async fn is_reachable(&self) -> bool {
        let url = format!("{}/health", self.base_url);
        self.client.get(&url).send().await.is_ok()
    }
}

// =============================================================================
// Default Rego Policies
// =============================================================================

/// Generate a default allow-all policy for testing
pub fn default_allow_policy() -> String {
    r#"
package swell.tool.access

default allow = true
"#
    .to_string()
}

/// Generate a default deny-all policy for strict enforcement
pub fn default_deny_policy() -> String {
    r#"
package swell.tool.access

# Deny by default for high-risk operations
default allow = false

# Allow read operations
allow {
    input.action.action_type == "read"
}

# Allow search operations
allow {
    input.action.action_type == "search"
}

# Allow git operations (non-destructive)
allow {
    input.action.action_type == "git"
    not contains_destructive_command(input.resource.path)
}

# Shell commands require high-risk role
allow {
    input.action.action_type == "shell"
    input.subject.role == "admin"
}

# Write operations require explicit permit
allow {
    input.action.action_type == "write"
    startswith(input.resource.path, "/workspace/")
}

# Edit operations require explicit permit
allow {
    input.action.action_type == "edit"
    startswith(input.resource.path, "/workspace/")
}

contains_destructive_command(cmd) {
    contains(cmd, "rm -rf")
}

contains_destructive_command(cmd) {
    contains(cmd, "sudo")
}
"#
    .to_string()
}

/// Generate a development policy with relaxed restrictions
pub fn development_policy() -> String {
    r#"
package swell.tool.access

# Development mode: more permissive
default allow = true

# But still deny destructive commands
deny[msg] {
    input.action.action_type == "shell"
    contains(input.resource.path, "rm -rf")
    msg := "Destructive shell commands are not allowed"
}

deny[msg] {
    input.action.action_type == "shell"
    contains(input.resource.path, "sudo")
    msg := "Sudo commands are not allowed in development mode"
}
"#
    .to_string()
}

/// Generate a production policy with strict restrictions
pub fn production_policy() -> String {
    r#"
package swell.tool.access

# Production mode: deny by default
default allow = false

# Allow only in workspace
allow {
    input.action.action_type == "read"
    startswith(input.resource.path, "/workspace/")
}

allow {
    input.action.action_type == "search"
}

# Only admin can run shell
allow {
    input.action.action_type == "shell"
    input.subject.role == "admin"
    startswith(input.resource.path, "/workspace/")
}

# Only admin can run git operations outside workspace
allow {
    input.action.action_type == "git"
    input.subject.role == "admin"
}

# Only admin can write
allow {
    input.action.action_type == "write"
    input.subject.role == "admin"
    startswith(input.resource.path, "/workspace/")
}
"#
    .to_string()
}

// =============================================================================
// Helper Functions
// =============================================================================

/// Create an OPA input from a tool operation and agent context
pub fn create_opa_input(agent_id: &str, role: &str, operation: &ToolOperation) -> OpaInput {
    let subject = OpaSubject::new(agent_id, role, operation.risk_level());
    let action = OpaAction::from_operation(operation);
    let resource = OpaResource::from_operation(operation);

    OpaInput::new(subject, action, resource)
        .with_context(
            "timestamp",
            serde_json::json!(chrono::Utc::now().to_rfc3339()),
        )
        .with_context(
            "operation_risk",
            serde_json::json!(operation.risk_level().to_string()),
        )
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_opa_input_creation() {
        let op = ToolOperation::Read {
            path: std::path::PathBuf::from("/workspace/src/main.rs"),
        };

        let input = create_opa_input("planner", "agent", &op);

        assert_eq!(input.subject.agent_id, "planner");
        assert_eq!(input.subject.role, "agent");
        assert_eq!(input.action.action_type, "read");
        assert_eq!(input.resource.resource_type, "file");
    }

    #[test]
    fn test_tool_operation_risk_level() {
        use crate::cedar_policy::CedarRiskLevel;

        let read_op = ToolOperation::Read {
            path: std::path::PathBuf::from("/workspace/src/main.rs"),
        };
        assert_eq!(read_op.risk_level(), CedarRiskLevel::Low);

        let shell_op = ToolOperation::Shell {
            command: "rm -rf".to_string(),
        };
        assert_eq!(shell_op.risk_level(), CedarRiskLevel::High);
    }

    #[test]
    fn test_opa_subject_creation() {
        use crate::cedar_policy::CedarRiskLevel;

        let subject = OpaSubject::new("agent1", "planner", CedarRiskLevel::Medium);

        assert_eq!(subject.agent_id, "agent1");
        assert_eq!(subject.role, "planner");
        assert_eq!(subject.operation_risk, "medium");
    }

    #[test]
    fn test_opa_action_from_operation() {
        let read_op = ToolOperation::Read {
            path: std::path::PathBuf::from("/workspace/src/lib.rs"),
        };
        let action = OpaAction::from_operation(&read_op);

        assert_eq!(action.action_type, "read");
        assert!(action.params.is_some());
        assert_eq!(
            action.params.as_ref().unwrap().get("path").unwrap(),
            "/workspace/src/lib.rs"
        );
    }

    #[test]
    fn test_opa_resource_from_operation() {
        let edit_op = ToolOperation::Edit {
            path: std::path::PathBuf::from("/workspace/src/lib.rs"),
        };
        let resource = OpaResource::from_operation(&edit_op);

        assert_eq!(resource.resource_type, "file");
        assert_eq!(resource.path, "/workspace/src/lib.rs");
    }

    #[test]
    fn test_default_allow_policy() {
        let policy = default_allow_policy();
        assert!(policy.contains("default allow = true"));
    }

    #[test]
    fn test_default_deny_policy() {
        let policy = default_deny_policy();
        assert!(policy.contains("default allow = false"));
        assert!(policy.contains("input.action.action_type"));
    }

    #[test]
    fn test_development_policy() {
        let policy = development_policy();
        assert!(policy.contains("Development mode"));
    }

    #[test]
    fn test_production_policy() {
        let policy = production_policy();
        assert!(policy.contains("Production mode"));
        assert!(policy.contains("deny by default"));
    }

    #[test]
    fn test_opa_decision_serialization() {
        let allow = OpaDecision::Allow;
        let deny = OpaDecision::Deny;

        assert_eq!(allow.is_allowed(), true);
        assert_eq!(deny.is_allowed(), false);
    }

    #[tokio::test]
    async fn test_opa_engine_creation() {
        let engine = OpaPolicyEngine::new("http://localhost:8181");
        assert!(!engine.is_bundle_mode());
    }

    #[tokio::test]
    async fn test_opa_client_creation() {
        let client = OpaClient::new("http://localhost:8181");
        // Just verify it can be created
        assert!(!client.is_reachable().await);
    }
}
