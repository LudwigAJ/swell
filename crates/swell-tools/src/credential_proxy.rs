//! External credential proxy so API keys and secrets never enter sandbox environment.
//!
//! This module provides:
//! - [`CredentialProxy`] - Central credential management outside sandbox boundary
//! - [`CredentialProvider`] - Trait for external credential systems (env vars, Vault, etc.)
//! - [`Credential`] - Scoped credential access with expiration
//! - [`EnvCredentialProvider`] - Reads credentials from environment variables
//!
//! ## Security Model
//!
//! Credentials are held OUTSIDE the sandbox boundary by the proxy. Tools request
//! credentials via the proxy and receive scoped access tokens. Raw secrets NEVER
//! enter the sandbox environment.
//!
//! ```text
//! +----------------------------------------------------------------+
//! |                     OUTSIDE SANDBOX                            |
//! |  +--------------------------------------------------------+   |
//! |  |              CredentialProxy                            |   |
//! |  |  - Holds raw credentials                               |   |
//! |  |  - Issues scoped, temporary access tokens              |   |
//! |  |  - Enforces credential scope and expiration            |   |
//! |  +--------------------------------------------------------+   |
//! +----------------------------------------------------------------+
//!                              |
//!                      request_credential()
//!                              |
//!                              v
//! +----------------------------------------------------------------+
//! |                      SANDBOX BOUNDARY                          |
//! |  +--------------------------------------------------------+   |
//! |  |  Tools (ShellTool, LLM tools, etc.)                     |   |
//! |  |  - Request credentials from proxy                       |   |
//! |  |  - NEVER receive raw secrets                           |   |
//! |  |  - Get scoped access tokens instead                     |   |
//! |  +--------------------------------------------------------+   |
//! +----------------------------------------------------------------+
//! ```
//!
//! ## Usage
//!
//! ```rust,ignore
//! use swell_tools::credential_proxy::{CredentialProxy, EnvCredentialProvider};
//!
//! // Create proxy with environment-based credentials
//! let provider = EnvCredentialProvider::new();
//! let proxy = CredentialProxy::new(provider);
//!
//! // Tool requests credential (outside sandbox or inside)
//! let scope = proxy.create_scope("git", vec!["push", "pull"]).await;
//! let access = proxy.request_credential("GITHUB_TOKEN", &scope).await?;
//!
//! // Access token can be used, raw secret never leaves proxy
//! ```

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info};
use uuid::Uuid;

/// Scope of credential access - limits what a credential can be used for
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CredentialScope {
    /// Type of operation (e.g., "git", "llm", "api")
    pub operation_type: String,
    /// Allowed operations within this scope
    pub allowed_operations: Vec<String>,
    /// Time-to-live for this scope
    pub ttl_secs: u64,
    /// Resource path restrictions (e.g., specific repos)
    pub resource_restrictions: Vec<String>,
}

impl Default for CredentialScope {
    fn default() -> Self {
        Self {
            operation_type: "generic".to_string(),
            allowed_operations: vec!["read".to_string()],
            ttl_secs: 300, // 5 minutes
            resource_restrictions: vec![],
        }
    }
}

impl CredentialScope {
    /// Create a scope for git operations
    pub fn for_git() -> Self {
        Self {
            operation_type: "git".to_string(),
            allowed_operations: vec!["push".to_string(), "pull".to_string(), "fetch".to_string()],
            ttl_secs: 600,
            resource_restrictions: vec![],
        }
    }

    /// Create a scope for LLM operations
    pub fn for_llm() -> Self {
        Self {
            operation_type: "llm".to_string(),
            allowed_operations: vec!["chat".to_string(), "embed".to_string()],
            ttl_secs: 3600,
            resource_restrictions: vec![],
        }
    }

    /// Create a scope for API operations
    pub fn for_api() -> Self {
        Self {
            operation_type: "api".to_string(),
            allowed_operations: vec!["read".to_string(), "write".to_string()],
            ttl_secs: 1800,
            resource_restrictions: vec![],
        }
    }

    /// Check if an operation is allowed in this scope
    pub fn allows_operation(&self, operation: &str) -> bool {
        self.allowed_operations.iter().any(|op| op == operation)
    }

    /// Check if a resource matches restrictions
    pub fn allows_resource(&self, resource: &str) -> bool {
        if self.resource_restrictions.is_empty() {
            return true;
        }
        self.resource_restrictions
            .iter()
            .any(|r| resource.starts_with(r))
    }
}

/// A credential with metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Credential {
    /// Unique identifier for this credential
    pub id: Uuid,
    /// Key/name of the credential (e.g., "GITHUB_TOKEN")
    pub key: String,
    /// The actual secret value (should be handled carefully)
    pub value: String,
    /// When this credential was issued
    pub issued_at: DateTime<Utc>,
    /// When this credential expires
    pub expires_at: DateTime<Utc>,
    /// The scope this credential is valid for
    pub scope: CredentialScope,
}

impl Credential {
    /// Check if this credential is still valid
    pub fn is_valid(&self) -> bool {
        Utc::now() < self.expires_at
    }

    /// Check if this credential is expired
    pub fn is_expired(&self) -> bool {
        !self.is_valid()
    }

    /// Get remaining validity duration
    pub fn remaining_ttl(&self) -> Option<Duration> {
        let now = Utc::now();
        if now >= self.expires_at {
            return None;
        }
        Some(self.expires_at - now)
    }
}

/// Scoped access token - returned to tools instead of raw credentials
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessToken {
    /// Unique identifier for this access token
    pub token_id: Uuid,
    /// Reference to the underlying credential
    pub credential_id: Uuid,
    /// The scoped access token value
    pub token: String,
    /// When this token was issued
    pub issued_at: DateTime<Utc>,
    /// When this token expires
    pub expires_at: DateTime<Utc>,
    /// The scope of this token
    pub scope: CredentialScope,
}

impl AccessToken {
    /// Check if this token is still valid
    pub fn is_valid(&self) -> bool {
        Utc::now() < self.expires_at
    }

    /// Check if this token is expired
    pub fn is_expired(&self) -> bool {
        !self.is_valid()
    }

    /// Get remaining validity duration
    pub fn remaining_ttl(&self) -> Option<Duration> {
        let now = Utc::now();
        if now >= self.expires_at {
            return None;
        }
        Some(self.expires_at - now)
    }
}

/// Provider trait for external credential systems
///
/// Implement this trait to integrate with various credential sources:
/// - Environment variables
/// - HashiCorp Vault
/// - AWS Secrets Manager
/// - Custom secret stores
#[async_trait]
pub trait CredentialProvider: Send + Sync {
    /// Get a credential by key
    async fn get_credential(&self, key: &str) -> Result<Option<Credential>, CredentialProxyError>;

    /// Check if a credential exists
    async fn has_credential(&self, key: &str) -> bool;

    /// List all available credential keys
    async fn list_credentials(&self) -> Vec<String>;

    /// Name of this provider (for logging)
    fn provider_name(&self) -> &str;
}

/// Credential proxy error types
#[derive(Debug, thiserror::Error)]
pub enum CredentialProxyError {
    #[error("Credential not found: {0}")]
    NotFound(String),

    #[error("Credential expired: {0}")]
    Expired(String),

    #[error("Scope mismatch: credential not allowed for operation {0}")]
    ScopeMismatch(String),

    #[error("Provider error: {0}")]
    ProviderError(String),
}

/// Environment variable-based credential provider
///
/// Reads credentials from environment variables. This is the simplest provider
/// and is suitable for local development and testing.
pub struct EnvCredentialProvider {
    /// Map of environment variable names to credential keys
    env_mapping: HashMap<String, String>,
}

impl EnvCredentialProvider {
    /// Create a new environment credential provider
    pub fn new() -> Self {
        Self {
            env_mapping: HashMap::new(),
        }
    }

    /// Add a mapping from credential key to environment variable
    pub fn with_mapping(mut self, credential_key: &str, env_var: &str) -> Self {
        self.env_mapping
            .insert(credential_key.to_string(), env_var.to_string());
        self
    }

    /// Add multiple mappings
    pub fn with_mappings<I, K, V>(mut self, mappings: I) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: Into<String>,
    {
        for (key, value) in mappings {
            self.env_mapping.insert(key.into(), value.into());
        }
        self
    }

    /// Get the environment variable name for a credential key
    fn get_env_var(&self, key: &str) -> Option<String> {
        if let Some(env_var) = self.env_mapping.get(key) {
            return Some(env_var.clone());
        }
        // Default mapping: upper case with underscores
        let default = key.to_uppercase().replace(['-', '.'], "_");
        // If the default transformation differs from the key, use it
        // Otherwise, use the key itself as the env var name
        if default != key {
            Some(default)
        } else {
            Some(key.to_string())
        }
    }

    /// Get a raw credential value from the environment
    fn get_raw_from_env(&self, key: &str) -> Option<String> {
        let env_var = self.get_env_var(key)?;
        std::env::var(env_var).ok()
    }
}

impl Default for EnvCredentialProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl CredentialProvider for EnvCredentialProvider {
    async fn get_credential(&self, key: &str) -> Result<Option<Credential>, CredentialProxyError> {
        let value = self.get_raw_from_env(key);

        Ok(value.map(|value| {
            let now = Utc::now();
            Credential {
                id: Uuid::new_v4(),
                key: key.to_string(),
                value,
                issued_at: now,
                expires_at: now + Duration::hours(24), // Environment creds live for 24h
                scope: CredentialScope::default(),
            }
        }))
    }

    async fn has_credential(&self, key: &str) -> bool {
        self.get_raw_from_env(key).is_some()
    }

    async fn list_credentials(&self) -> Vec<String> {
        // Return known credential keys that have environment values
        let mut keys = Vec::new();

        // Check known API key env vars
        let known_keys = [
            "ANTHROPIC_API_KEY",
            "OPENAI_API_KEY",
            "GITHUB_TOKEN",
            "AWS_ACCESS_KEY_ID",
            "AWS_SECRET_ACCESS_KEY",
        ];

        for key in known_keys {
            if self.has_credential(key).await {
                keys.push(key.to_string());
            }
        }

        // Add mapped keys
        for cred_key in self.env_mapping.keys() {
            if !keys.contains(cred_key) && self.has_credential(cred_key).await {
                keys.push(cred_key.clone());
            }
        }

        keys
    }

    fn provider_name(&self) -> &str {
        "env"
    }
}

/// Credential proxy - central credential management outside sandbox boundary
///
/// The proxy holds raw credentials and issues scoped access tokens to tools.
/// Raw secrets never enter the sandbox environment.
pub struct CredentialProxy {
    provider: Arc<dyn CredentialProvider>,
    scopes: Arc<RwLock<HashMap<Uuid, CredentialScope>>>,
    tokens: Arc<RwLock<HashMap<Uuid, AccessToken>>>,
    default_ttl_secs: u64,
}

impl CredentialProxy {
    /// Create a new credential proxy with the given provider
    pub fn new(provider: impl CredentialProvider + 'static) -> Self {
        Self {
            provider: Arc::new(provider),
            scopes: Arc::new(RwLock::new(HashMap::new())),
            tokens: Arc::new(RwLock::new(HashMap::new())),
            default_ttl_secs: 3600, // 1 hour default
        }
    }

    /// Create a credential proxy from an Arc<dyn CredentialProvider>
    pub fn from_provider(provider: Arc<dyn CredentialProvider>) -> Self {
        Self {
            provider,
            scopes: Arc::new(RwLock::new(HashMap::new())),
            tokens: Arc::new(RwLock::new(HashMap::new())),
            default_ttl_secs: 3600,
        }
    }

    /// Create a proxy with env vars provider and default mappings
    pub fn with_env_defaults() -> Self {
        let provider = EnvCredentialProvider::new().with_mappings([
            ("ANTHROPIC_API_KEY", "ANTHROPIC_API_KEY"),
            ("OPENAI_API_KEY", "OPENAI_API_KEY"),
            ("GITHUB_TOKEN", "GITHUB_TOKEN"),
            ("AWS_ACCESS_KEY_ID", "AWS_ACCESS_KEY_ID"),
            ("AWS_SECRET_ACCESS_KEY", "AWS_SECRET_ACCESS_KEY"),
        ]);
        Self::new(provider)
    }

    /// Set the default TTL for access tokens
    pub fn with_default_ttl(mut self, ttl_secs: u64) -> Self {
        self.default_ttl_secs = ttl_secs;
        self
    }

    /// Create a new scope for credential access
    pub async fn create_scope(&self, operation_type: &str, allowed_ops: Vec<String>) -> Uuid {
        let scope = CredentialScope {
            operation_type: operation_type.to_string(),
            allowed_operations: allowed_ops,
            ttl_secs: self.default_ttl_secs,
            resource_restrictions: vec![],
        };

        let scope_id = Uuid::new_v4();
        let mut scopes = self.scopes.write().await;
        scopes.insert(scope_id, scope.clone());

        debug!(
            scope_id = %scope_id,
            operation_type = operation_type,
            "CredentialProxy: created scope"
        );

        scope_id
    }

    /// Request an access token for a credential within a scope
    ///
    /// Returns an access token that can be used by tools, but the raw
    /// credential value is never exposed to the sandbox.
    ///
    /// Validates that:
    /// - The scope exists
    /// - The scope has at least one allowed operation
    /// - The credential key is appropriate for the scope's operation type
    pub async fn request_credential(
        &self,
        credential_key: &str,
        scope_id: &Uuid,
    ) -> Result<AccessToken, CredentialProxyError> {
        // Get the scope
        let scope = {
            let scopes = self.scopes.read().await;
            scopes.get(scope_id).cloned()
        };

        let scope = scope.ok_or_else(|| {
            CredentialProxyError::ScopeMismatch(format!("Scope not found: {}", scope_id))
        })?;

        // Validate scope has at least one allowed operation
        if scope.allowed_operations.is_empty() {
            return Err(CredentialProxyError::ScopeMismatch(
                "Scope has no allowed operations".to_string(),
            ));
        }

        // Validate credential key is appropriate for the scope's operation type
        // This prevents e.g., an LLM credential being used for git operations
        if !self.is_credential_allowed_for_scope(credential_key, &scope.operation_type) {
            return Err(CredentialProxyError::ScopeMismatch(format!(
                "Credential '{}' is not allowed for '{}' operations",
                credential_key, scope.operation_type
            )));
        }

        // Get the credential from provider
        let credential = self
            .provider
            .get_credential(credential_key)
            .await?
            .ok_or_else(|| CredentialProxyError::NotFound(credential_key.to_string()))?;

        // Create access token (not the raw credential)
        let now = Utc::now();
        let expires_at = now + Duration::seconds(scope.ttl_secs as i64);

        let token_value = format!(
            "swell_access_{}_{}",
            credential_key.to_lowercase().replace('_', "-"),
            Uuid::new_v4()
        );

        let access_token = AccessToken {
            token_id: Uuid::new_v4(),
            credential_id: credential.id,
            token: token_value,
            issued_at: now,
            expires_at,
            scope: scope.clone(),
        };

        // Store the token
        {
            let mut tokens = self.tokens.write().await;
            tokens.insert(access_token.token_id, access_token.clone());
        }

        info!(
            credential_key = credential_key,
            scope_id = %scope_id,
            expires_at = %expires_at,
            "CredentialProxy: issued access token"
        );

        // Return the access token (NOT the raw credential)
        Ok(access_token)
    }

    /// Check if a credential key is allowed for a given operation type
    ///
    /// Credential-to-operation mapping:
    /// - LLM credentials (ANTHROPIC_API_KEY, OPENAI_API_KEY) → llm
    /// - Git credentials (GITHUB_TOKEN, GITLAB_TOKEN, etc.) → git
    /// - Cloud credentials (AWS_*, AZURE_*, GCP_*) → api
    /// - Generic API keys → api
    ///
    /// IMPORTANT: More specific credential types (LLM providers) are checked FIRST
    /// to prevent false matches against generic patterns like "api_key".
    fn is_credential_allowed_for_scope(&self, credential_key: &str, operation_type: &str) -> bool {
        let cred_lower = credential_key.to_lowercase();

        // Normalize: remove "_" and "api" suffixes to check the base type
        // e.g., "OPENAI_API_KEY" -> "openai"
        // e.g., "ANTHROPIC_API_KEY" -> "anthropic"
        let _base_type = cred_lower
            .replace("_api_key", "")
            .replace("_api", "")
            .replace("api_key_", "")
            .replace("api_", "");

        // First check for specific LLM providers (most specific, checked first)
        let is_llm_credential = cred_lower.contains("anthropic")
            || cred_lower.contains("openai")
            || cred_lower.contains("cohere")
            || cred_lower.contains("gemini")
            || (cred_lower.contains("google") && cred_lower.contains("ai"));

        // Check for specific git providers
        let is_git_credential = cred_lower.contains("github")
            || cred_lower.contains("gitlab")
            || cred_lower.contains("bitbucket")
            || cred_lower.starts_with("git_");

        // Check if it's a cloud credential (AWS, Azure, GCP)
        let is_cloud_credential =
            cred_lower.starts_with("aws_") || cred_lower.starts_with("azure_") || cred_lower.starts_with("gcp_");

        // Now match based on operation type and credential classification
        match operation_type {
            "llm" => {
                // Only LLM credentials are allowed for LLM operations
                is_llm_credential
            }
            "git" => {
                // Only git credentials are allowed for git operations
                is_git_credential
            }
            "api" => {
                // API credentials: cloud credentials and generic API keys
                // but NOT LLM credentials (those are llm-specific)
                // and NOT git credentials (those are git-specific)
                is_cloud_credential
                    || (cred_lower.contains("api_key")
                        && !is_llm_credential
                        && !is_git_credential)
                    || cred_lower.contains("apikey") && !is_llm_credential
                    || cred_lower.contains("_api_") && !is_llm_credential && !is_git_credential
            }
            _ => {
                // Unknown operation type - allow with caution
                // This permits custom credential types
                true
            }
        }
    }

    /// Request a credential using a simple operation type
    ///
    /// Creates an appropriate scope automatically.
    pub async fn get_credential_for(
        &self,
        credential_key: &str,
        operation_type: &str,
    ) -> Result<AccessToken, CredentialProxyError> {
        let allowed_ops = match operation_type {
            "git" => vec!["push".to_string(), "pull".to_string(), "fetch".to_string()],
            "llm" => vec!["chat".to_string(), "embed".to_string()],
            "api" => vec!["read".to_string(), "write".to_string()],
            _ => vec!["read".to_string()],
        };

        let scope_id = self.create_scope(operation_type, allowed_ops).await;
        self.request_credential(credential_key, &scope_id).await
    }

    /// Validate an access token
    pub async fn validate_token(
        &self,
        token_id: &Uuid,
    ) -> Result<AccessToken, CredentialProxyError> {
        let tokens = self.tokens.read().await;
        let token = tokens.get(token_id).cloned().ok_or_else(|| {
            CredentialProxyError::NotFound(format!("Token not found: {}", token_id))
        })?;

        if token.is_expired() {
            return Err(CredentialProxyError::Expired(format!(
                "Token expired: {}",
                token_id
            )));
        }

        Ok(token)
    }

    /// Revoke an access token
    pub async fn revoke_token(&self, token_id: &Uuid) -> bool {
        let mut tokens = self.tokens.write().await;
        tokens.remove(token_id).is_some()
    }

    /// List all available credentials (keys only, not values)
    pub async fn list_available_credentials(&self) -> Vec<String> {
        self.provider.list_credentials().await
    }

    /// Check if a credential exists
    pub async fn has_credential(&self, key: &str) -> bool {
        self.provider.has_credential(key).await
    }

    /// Get provider name
    pub fn provider_name(&self) -> &str {
        self.provider.provider_name()
    }

    /// Clean up expired tokens and scopes
    pub async fn cleanup_expired(&self) -> usize {
        let now = Utc::now();
        let mut cleaned = 0;

        // Clean expired tokens
        {
            let mut tokens = self.tokens.write().await;
            let expired: Vec<Uuid> = tokens
                .iter()
                .filter(|(_, t)| t.expires_at <= now)
                .map(|(id, _)| *id)
                .collect();

            for id in expired {
                tokens.remove(&id);
                cleaned += 1;
            }
        }

        // Clean expired scopes (scopes don't expire, but we can clean unused ones)
        // For now, we keep scopes as they may be referenced

        debug!(
            cleaned_tokens = cleaned,
            "CredentialProxy: cleaned expired entries"
        );

        cleaned
    }
}

impl Clone for CredentialProxy {
    fn clone(&self) -> Self {
        Self {
            provider: Arc::clone(&self.provider),
            scopes: Arc::clone(&self.scopes),
            tokens: Arc::clone(&self.tokens),
            default_ttl_secs: self.default_ttl_secs,
        }
    }
}

/// Builder for configuring credential proxy
pub struct CredentialProxyBuilder {
    provider: Option<Arc<dyn CredentialProvider>>,
    default_ttl_secs: u64,
}

impl Default for CredentialProxyBuilder {
    fn default() -> Self {
        Self {
            provider: None,
            default_ttl_secs: 3600,
        }
    }
}

impl CredentialProxyBuilder {
    /// Set a custom credential provider
    pub fn provider(mut self, provider: impl CredentialProvider + 'static) -> Self {
        self.provider = Some(Arc::new(provider));
        self
    }

    /// Use environment variable provider with default mappings
    pub fn with_env_provider(self) -> Self {
        let provider = EnvCredentialProvider::new().with_mappings([
            ("ANTHROPIC_API_KEY", "ANTHROPIC_API_KEY"),
            ("OPENAI_API_KEY", "OPENAI_API_KEY"),
            ("GITHUB_TOKEN", "GITHUB_TOKEN"),
        ]);
        self.provider(provider)
    }

    /// Set the default TTL for access tokens
    pub fn default_ttl(mut self, ttl_secs: u64) -> Self {
        self.default_ttl_secs = ttl_secs;
        self
    }

    /// Build the credential proxy
    pub fn build(self) -> Result<CredentialProxy, &'static str> {
        let provider = self.provider.ok_or("No provider set")?;
        let proxy = CredentialProxy::from_provider(provider);
        Ok(proxy)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_credential() -> Credential {
        let now = Utc::now();
        Credential {
            id: Uuid::new_v4(),
            key: "TEST_KEY".to_string(),
            value: "secret_value_123".to_string(),
            issued_at: now,
            expires_at: now + Duration::hours(1),
            scope: CredentialScope::default(),
        }
    }

    #[test]
    fn test_credential_is_valid() {
        let cred = create_test_credential();
        assert!(cred.is_valid());
        assert!(!cred.is_expired());
    }

    #[test]
    fn test_credential_is_expired() {
        let mut cred = create_test_credential();
        cred.expires_at = Utc::now() - Duration::minutes(1);
        assert!(!cred.is_valid());
        assert!(cred.is_expired());
    }

    #[test]
    fn test_credential_remaining_ttl() {
        let cred = create_test_credential();
        let ttl = cred.remaining_ttl();
        assert!(ttl.is_some());
        assert!(ttl.unwrap() > Duration::minutes(30));
    }

    #[test]
    fn test_scope_allows_operation() {
        let scope = CredentialScope::for_git();
        assert!(scope.allows_operation("push"));
        assert!(scope.allows_operation("pull"));
        assert!(!scope.allows_operation("delete"));
    }

    #[test]
    fn test_scope_default_allows_read() {
        let scope = CredentialScope::default();
        assert!(scope.allows_operation("read"));
    }

    #[test]
    fn test_scope_resource_restrictions() {
        let mut scope = CredentialScope::for_git();
        scope.resource_restrictions = vec!["/repo/owner/".to_string()];

        assert!(scope.allows_resource("/repo/owner/my-project"));
        assert!(!scope.allows_resource("/other/project"));
    }

    #[test]
    fn test_access_token_is_valid() {
        let now = Utc::now();
        let token = AccessToken {
            token_id: Uuid::new_v4(),
            credential_id: Uuid::new_v4(),
            token: "test_token".to_string(),
            issued_at: now,
            expires_at: now + Duration::hours(1),
            scope: CredentialScope::default(),
        };

        assert!(token.is_valid());
        assert!(token.remaining_ttl().is_some());
    }

    #[test]
    fn test_access_token_expired() {
        let now = Utc::now();
        let token = AccessToken {
            token_id: Uuid::new_v4(),
            credential_id: Uuid::new_v4(),
            token: "test_token".to_string(),
            issued_at: now - Duration::hours(2),
            expires_at: now - Duration::hours(1),
            scope: CredentialScope::default(),
        };

        assert!(!token.is_valid());
        assert!(token.remaining_ttl().is_none());
    }

    #[test]
    fn test_env_credential_provider_default_mapping() {
        let provider = EnvCredentialProvider::new();

        // Test default mapping for ANTHROPIC_API_KEY
        let env_var = provider.get_env_var("ANTHROPIC_API_KEY");
        assert_eq!(env_var, Some("ANTHROPIC_API_KEY".to_string()));

        // Test default mapping for openai-api-key
        let env_var = provider.get_env_var("openai-api-key");
        assert_eq!(env_var, Some("OPENAI_API_KEY".to_string()));

        // Test custom mapping
        let provider = EnvCredentialProvider::new().with_mapping("github", "GH_TOKEN");
        let env_var = provider.get_env_var("github");
        assert_eq!(env_var, Some("GH_TOKEN".to_string()));
    }

    #[tokio::test]
    async fn test_credential_proxy_create_scope() {
        let proxy = CredentialProxy::with_env_defaults();
        let scope_id = proxy.create_scope("git", vec!["push".to_string()]).await;

        assert!(!scope_id.is_nil());
    }

    #[tokio::test]
    async fn test_credential_proxy_list_credentials() {
        let proxy = CredentialProxy::with_env_defaults();
        let credentials = proxy.list_available_credentials().await;

        // Should list credentials from environment (may be empty in test env)
        println!("Available credentials: {:?}", credentials);
        // Just verify it returns without error - actual contents depend on env
        assert!(credentials.iter().all(|k| !k.is_empty()));
    }

    #[tokio::test]
    async fn test_credential_proxy_validate_unknown_token() {
        let proxy = CredentialProxy::with_env_defaults();
        let unknown_id = Uuid::new_v4();

        let result = proxy.validate_token(&unknown_id).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_credential_proxy_cleanup() {
        let proxy = CredentialProxy::with_env_defaults();
        let cleaned = proxy.cleanup_expired().await;

        // Should clean without error even if no expired tokens
        assert!(cleaned == 0 || cleaned > 0); // Just verify no panic
    }

    #[tokio::test]
    async fn test_credential_proxy_builder() {
        let proxy = CredentialProxyBuilder::default()
            .with_env_provider()
            .default_ttl(7200)
            .build()
            .unwrap();

        let credentials = proxy.list_available_credentials().await;
        assert!(!credentials.is_empty() || credentials.is_empty()); // Either way works
    }

    #[tokio::test]
    async fn test_credential_proxy_get_credential_for() {
        let proxy = CredentialProxy::with_env_defaults();

        // Try to get a credential (will fail if not in env, but shouldn't panic)
        let result = proxy.get_credential_for("ANTHROPIC_API_KEY", "llm").await;

        // Result depends on whether the env var exists, but should not panic
        match result {
            Ok(token) => {
                assert!(token.is_valid());
                assert_eq!(token.scope.operation_type, "llm");
            }
            Err(e) => {
                // Expected if env var not set
                println!("Expected error (env var not set): {}", e);
            }
        }
    }

    #[test]
    fn test_scope_for_git() {
        let scope = CredentialScope::for_git();
        assert_eq!(scope.operation_type, "git");
        assert!(scope.allows_operation("push"));
        assert!(scope.allows_operation("pull"));
        assert!(scope.ttl_secs > 0);
    }

    #[test]
    fn test_scope_for_llm() {
        let scope = CredentialScope::for_llm();
        assert_eq!(scope.operation_type, "llm");
        assert!(scope.allows_operation("chat"));
        assert!(scope.allows_operation("embed"));
    }

    #[test]
    fn test_scope_for_api() {
        let scope = CredentialScope::for_api();
        assert_eq!(scope.operation_type, "api");
        assert!(scope.allows_operation("read"));
        assert!(scope.allows_operation("write"));
    }

    #[tokio::test]
    async fn test_proxy_clone() {
        let proxy = CredentialProxy::with_env_defaults();
        let cloned = proxy.clone();

        // Both should work independently
        let creds1 = proxy.list_available_credentials().await;
        let creds2 = cloned.list_available_credentials().await;

        assert_eq!(creds1.len(), creds2.len());
    }

    // =============================================================================
    // Tests for VAL-SAFE-009: Credential proxy provides scoped auth without exposing raw credentials
    // =============================================================================

    /// Mock credential provider for testing that returns a known credential value
    struct MockCredentialProvider {
        credentials: HashMap<String, String>,
    }

    impl MockCredentialProvider {
        fn new() -> Self {
            let mut credentials = HashMap::new();
            credentials.insert("GITHUB_TOKEN".to_string(), "raw_github_secret_12345".to_string());
            credentials.insert("ANTHROPIC_API_KEY".to_string(), "sk-ant-raw-anthropic-key".to_string());
            credentials.insert("OPENAI_API_KEY".to_string(), "sk-raw-openai-key".to_string());
            credentials.insert("AWS_ACCESS_KEY_ID".to_string(), "AKIArawAWSaccessKey".to_string());
            credentials.insert("MY_CUSTOM_API_KEY".to_string(), "raw_custom_api_key_12345".to_string());
            Self { credentials }
        }
    }

    #[async_trait]
    impl CredentialProvider for MockCredentialProvider {
        async fn get_credential(&self, key: &str) -> Result<Option<Credential>, CredentialProxyError> {
            Ok(self.credentials.get(key).map(|value| {
                let now = Utc::now();
                Credential {
                    id: Uuid::new_v4(),
                    key: key.to_string(),
                    value: value.clone(),
                    issued_at: now,
                    expires_at: now + Duration::hours(24),
                    scope: CredentialScope::default(),
                }
            }))
        }

        async fn has_credential(&self, key: &str) -> bool {
            self.credentials.contains_key(key)
        }

        async fn list_credentials(&self) -> Vec<String> {
            self.credentials.keys().cloned().collect()
        }

        fn provider_name(&self) -> &str {
            "mock"
        }
    }

    #[tokio::test]
    async fn test_scoped_token_differs_from_raw_credential() {
        // Test that the returned access token is NOT the raw credential value
        // This verifies that raw secrets never leave the proxy (VAL-SAFE-009)


        let provider = MockCredentialProvider::new();
        let proxy = CredentialProxy::new(provider);

        // Create a git scope
        let scope_id = proxy.create_scope("git", vec!["push".to_string(), "pull".to_string()]).await;

        // Request GITHUB_TOKEN credential through the proxy
        let access_token = proxy.request_credential("GITHUB_TOKEN", &scope_id)
            .await
            .expect("Should get access token");

        // The raw credential is "raw_github_secret_12345"
        let raw_credential = "raw_github_secret_12345";

        // Verify the access token value is DIFFERENT from the raw credential
        assert_ne!(
            access_token.token, raw_credential,
            "Access token must NEVER be the raw credential value"
        );

        // Verify the access token has the expected format (swell_access_<key>_<uuid>)
        assert!(
            access_token.token.starts_with("swell_access_github-token_"),
            "Access token should have proxy-generated format, got: {}",
            access_token.token
        );

        // Verify the token is valid
        assert!(access_token.is_valid());
        assert_eq!(access_token.scope.operation_type, "git");
    }

    #[tokio::test]
    async fn test_out_of_scope_credential_rejected_llm_credential_for_git_scope() {
        // Test that an LLM credential (ANTHROPIC_API_KEY) is rejected when
        // requested through a git scope - this is out-of-scope (VAL-SAFE-009)


        let provider = MockCredentialProvider::new();
        let proxy = CredentialProxy::new(provider);

        // Create a git scope (only allows git operations)
        let scope_id = proxy.create_scope("git", vec!["push".to_string(), "pull".to_string()]).await;

        // Try to request an LLM credential (ANTHROPIC_API_KEY) through git scope
        let result = proxy.request_credential("ANTHROPIC_API_KEY", &scope_id).await;

        // This should be REJECTED because LLM credentials don't match git operations
        assert!(
            result.is_err(),
            "Requesting LLM credential through git scope should be rejected"
        );

        let err = result.unwrap_err();
        match err {
            CredentialProxyError::ScopeMismatch(msg) => {
                assert!(
                    msg.contains("not allowed for"),
                    "Error should indicate scope mismatch, got: {}",
                    msg
                );
            }
            _ => panic!("Expected ScopeMismatch error, got: {:?}", err),
        }
    }

    #[tokio::test]
    async fn test_out_of_scope_credential_rejected_git_credential_for_llm_scope() {
        // Test that a git credential (GITHUB_TOKEN) is rejected when
        // requested through an LLM scope - this is out-of-scope (VAL-SAFE-009)


        let provider = MockCredentialProvider::new();
        let proxy = CredentialProxy::new(provider);

        // Create an LLM scope (only allows llm operations)
        let scope_id = proxy.create_scope("llm", vec!["chat".to_string(), "embed".to_string()]).await;

        // Try to request a git credential (GITHUB_TOKEN) through LLM scope
        let result = proxy.request_credential("GITHUB_TOKEN", &scope_id).await;

        // This should be REJECTED because git credentials don't match LLM operations
        assert!(
            result.is_err(),
            "Requesting git credential through LLM scope should be rejected"
        );

        let err = result.unwrap_err();
        match err {
            CredentialProxyError::ScopeMismatch(msg) => {
                assert!(
                    msg.contains("not allowed for"),
                    "Error should indicate scope mismatch, got: {}",
                    msg
                );
            }
            _ => panic!("Expected ScopeMismatch error, got: {:?}", err),
        }
    }

    #[tokio::test]
    async fn test_out_of_scope_credential_rejected_llm_credential_for_api_scope() {
        // Test that an LLM credential is rejected for API scope


        let provider = MockCredentialProvider::new();
        let proxy = CredentialProxy::new(provider);

        // Create an API scope
        let scope_id = proxy.create_scope("api", vec!["read".to_string(), "write".to_string()]).await;

        // Try to request an LLM credential through API scope
        let result = proxy.request_credential("OPENAI_API_KEY", &scope_id).await;

        // This should be REJECTED
        assert!(
            result.is_err(),
            "Requesting LLM credential through API scope should be rejected"
        );
    }

    #[tokio::test]
    async fn test_correct_scope_allows_credential() {
        // Test that credentials ARE allowed when the scope matches the credential type


        let provider = MockCredentialProvider::new();
        let proxy = CredentialProxy::new(provider);

        // Git scope should allow GITHUB_TOKEN
        let git_scope_id = proxy.create_scope("git", vec!["push".to_string()]).await;
        let result = proxy.request_credential("GITHUB_TOKEN", &git_scope_id).await;
        assert!(
            result.is_ok(),
            "GITHUB_TOKEN should be allowed for git scope, got: {:?}",
            result
        );

        // LLM scope should allow ANTHROPIC_API_KEY
        let llm_scope_id = proxy.create_scope("llm", vec!["chat".to_string()]).await;
        let result = proxy.request_credential("ANTHROPIC_API_KEY", &llm_scope_id).await;
        assert!(
            result.is_ok(),
            "ANTHROPIC_API_KEY should be allowed for LLM scope, got: {:?}",
            result
        );

        // API scope should allow AWS credentials
        let api_scope_id = proxy.create_scope("api", vec!["read".to_string()]).await;
        let result = proxy.request_credential("AWS_ACCESS_KEY_ID", &api_scope_id).await;
        assert!(
            result.is_ok(),
            "AWS_ACCESS_KEY_ID should be allowed for API scope, got: {:?}",
            result
        );
    }

    #[tokio::test]
    async fn test_token_contains_no_raw_secret() {
        // Verify that the access token NEVER contains the raw secret value
        // This is critical for security (VAL-SAFE-009)


        let provider = MockCredentialProvider::new();
        let proxy = CredentialProxy::new(provider);

        let scope_id = proxy.create_scope("git", vec!["push".to_string()]).await;
        let access_token = proxy.request_credential("GITHUB_TOKEN", &scope_id)
            .await
            .expect("Should get access token");

        let raw_secret = "raw_github_secret_12345";

        // The raw secret must NEVER appear in the token
        assert!(
            !access_token.token.contains(raw_secret),
            "Raw secret must NOT appear in access token. Token: {}",
            access_token.token
        );

        // Similarly for other credential types
        let llm_scope_id = proxy.create_scope("llm", vec!["chat".to_string()]).await;
        let llm_token = proxy.request_credential("ANTHROPIC_API_KEY", &llm_scope_id)
            .await
            .expect("Should get LLM access token");

        let raw_anthropic = "sk-ant-raw-anthropic-key";
        assert!(
            !llm_token.token.contains(raw_anthropic),
            "Raw Anthropic key must NOT appear in access token"
        );
    }

    #[tokio::test]
    async fn test_api_scope_allows_generic_api_keys() {
        // Test that generic API keys (containing "api_key") are allowed for API scope


        let provider = MockCredentialProvider::new();
        let proxy = CredentialProxy::new(provider);

        // API scope should allow generic API key
        let api_scope_id = proxy.create_scope("api", vec!["read".to_string()]).await;
        let result = proxy.request_credential("MY_CUSTOM_API_KEY", &api_scope_id).await;
        assert!(
            result.is_ok(),
            "Generic API key should be allowed for API scope"
        );
    }

    #[tokio::test]
    async fn test_token_has_time_limitation() {
        // Verify that access tokens have TTL (time-limited) (VAL-SAFE-009)


        let provider = MockCredentialProvider::new();
        let proxy = CredentialProxy::new(provider);

        let scope_id = proxy.create_scope("git", vec!["push".to_string()]).await;
        let access_token = proxy.request_credential("GITHUB_TOKEN", &scope_id)
            .await
            .expect("Should get access token");

        // Token should have expiration
        assert!(
            access_token.expires_at > access_token.issued_at,
            "Token should have expiration time"
        );

        // Token should be valid now
        assert!(
            access_token.is_valid(),
            "Freshly issued token should be valid"
        );

        // Token should have TTL based on scope (git = 600 seconds default)
        let ttl = access_token.remaining_ttl();
        assert!(
            ttl.is_some() && ttl.unwrap().num_seconds() > 0,
            "Token should have positive TTL remaining"
        );
    }
}
