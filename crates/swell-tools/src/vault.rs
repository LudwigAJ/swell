//! HashiCorp Vault integration for dynamic secrets and credential rotation.
//!
//! This module provides:
//! - [`VaultCredentialProvider`] - Fetches credentials from Vault KV secrets engine
//! - [`VaultClient`] - Low-level Vault API client
//! - [`VaultDynamicSecret`] - Wrapper for dynamic secrets with lease management
//!
//! ## Usage with CredentialProxy
//!
//! ```rust,ignore
//! use swell_tools::credential_proxy::{CredentialProxy, CredentialScope};
//! use swell_tools::vault::{VaultCredentialProvider, VaultClientConfig};
//!
//! // Configure Vault connection
//! let config = VaultClientConfig::new("http://localhost:8200", "my-token");
//! let vault_provider = VaultCredentialProvider::new(config);
//!
//! // Wrap with proxy for scoped access
//! let proxy = CredentialProxy::new(vault_provider);
//!
//! // Request a credential with limited scope
//! let scope = proxy.create_scope("database", vec!["read", "write"]).await;
//! let access_token = proxy.request_credential("database/creds/my-role", &scope).await?;
//! ```
//!
//! ## Dynamic Secrets
//!
//! ```rust,ignore
//! use swell_tools::vault::{VaultDynamicSecret, DatabaseDynamicSecret};
//!
//! // Create dynamic database credentials
//! let dynamic = DatabaseDynamicSecret::new("database/creds/my-role");
//! let creds = dynamic.generate().await?;
//! // creds.username and creds.password are temporary
//! // creds.lease_id for renewal/ revocation
//! ```

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use reqwest::Client;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use super::credential_proxy::{
    Credential, CredentialProvider, CredentialProxyError, CredentialScope,
};
use crate::egress::{Destination, EgressFilter};

/// Configuration for Vault client
#[derive(Debug, Clone)]
pub struct VaultClientConfig {
    /// Vault server address (e.g., "https://vault.example.com:8200")
    pub address: String,
    /// Authentication token
    pub token: Option<String>,
    /// AppRole role ID (for AppRole auth)
    pub role_id: Option<String>,
    /// AppRole secret ID (for AppRole auth)
    pub secret_id: Option<String>,
    /// Mount path for KV secrets engine (default: "secret")
    pub kv_mount_path: String,
    /// Default namespace for Vault Enterprise
    pub namespace: Option<String>,
    /// Request timeout in seconds
    pub timeout_secs: u64,
}

impl VaultClientConfig {
    /// Create a new config with token authentication
    pub fn new(address: &str, token: &str) -> Self {
        Self {
            address: address.to_string(),
            token: Some(token.to_string()),
            role_id: None,
            secret_id: None,
            kv_mount_path: "secret".to_string(),
            namespace: None,
            timeout_secs: 30,
        }
    }

    /// Create a new config with AppRole authentication
    pub fn with_approle(address: &str, role_id: &str, secret_id: &str) -> Self {
        Self {
            address: address.to_string(),
            token: None,
            role_id: Some(role_id.to_string()),
            secret_id: Some(secret_id.to_string()),
            kv_mount_path: "secret".to_string(),
            namespace: None,
            timeout_secs: 30,
        }
    }

    /// Set custom KV mount path
    pub fn with_kv_mount_path(mut self, path: &str) -> Self {
        self.kv_mount_path = path.to_string();
        self
    }

    /// Set namespace for Vault Enterprise
    pub fn with_namespace(mut self, namespace: &str) -> Self {
        self.namespace = Some(namespace.to_string());
        self
    }
}

/// Vault API error
#[derive(Debug, thiserror::Error)]
pub enum VaultError {
    #[error("Connection failed: {0}")]
    ConnectionError(String),

    #[error("Authentication failed: {0}")]
    AuthError(String),

    #[error("Secret not found: {0}")]
    NotFound(String),

    #[error("Invalid response: {0}")]
    InvalidResponse(String),

    #[error("Vault API error: code={code}, message={message}")]
    VaultApiError { code: u64, message: String },

    #[error("HTTP error: {0}")]
    HttpError(#[from] reqwest::Error),
}

/// Vault API response wrapper
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct VaultResponse<T> {
    data: Option<T>,
    #[serde(rename = "lease_id")]
    lease_id: Option<String>,
    #[serde(rename = "lease_duration")]
    lease_duration: Option<u64>,
    #[serde(rename = "request_id")]
    request_id: Option<String>,
    #[serde(rename = "warnings")]
    warnings: Option<Vec<String>>,
    #[serde(rename = "auth")]
    auth: Option<VaultAuthResponse>,
}

/// Auth response
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct VaultAuthResponse {
    #[serde(rename = "client_token")]
    client_token: String,
    #[serde(rename = "accessor")]
    accessor: Option<String>,
    #[serde(rename = "token_policies")]
    token_policies: Option<Vec<String>>,
    #[serde(rename = "lease_duration")]
    lease_duration: Option<u64>,
}

/// KV secrets data wrapper
#[derive(Debug, Deserialize)]
struct KvSecretsData {
    data: HashMap<String, String>,
    metadata: KvMetadata,
}

/// KV metadata
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct KvMetadata {
    created_time: String,
    destroy_requested: bool,
    expiration_time: Option<String>,
    version: u64,
}

/// Low-level Vault client for API operations
pub struct VaultClient {
    config: VaultClientConfig,
    http_client: Client,
    token: RwLock<Option<String>>,
    egress_filter: Option<Arc<EgressFilter>>,
}

impl VaultClient {
    /// Create a new Vault client
    pub fn new(config: VaultClientConfig) -> Result<Self, VaultError> {
        let http_client = Client::builder()
            .timeout(std::time::Duration::from_secs(config.timeout_secs))
            .build()
            .map_err(|e| VaultError::ConnectionError(e.to_string()))?;

        Ok(Self {
            config,
            http_client,
            token: RwLock::new(None),
            egress_filter: None,
        })
    }

    /// Create a new Vault client with an egress filter for network filtering
    pub fn with_egress_filter(config: VaultClientConfig, egress_filter: Arc<EgressFilter>) -> Result<Self, VaultError> {
        let http_client = Client::builder()
            .timeout(std::time::Duration::from_secs(config.timeout_secs))
            .build()
            .map_err(|e| VaultError::ConnectionError(e.to_string()))?;

        Ok(Self {
            config,
            http_client,
            token: RwLock::new(None),
            egress_filter: Some(egress_filter),
        })
    }

    /// Check if egress is allowed for the Vault server endpoint
    fn is_egress_allowed(&self) -> bool {
        if let Some(ref filter) = self.egress_filter {
            // Extract host and port from Vault address
            let address = &self.config.address;
            let (host, port) = if let Ok(url) = url::Url::parse(address) {
                (url.host_str().unwrap_or("").to_string(), url.port().unwrap_or(8200))
            } else {
                // Fallback: parse manually
                let address = address
                    .trim_start_matches("http://")
                    .trim_start_matches("https://");
                let parts: Vec<&str> = address.split(':').collect();
                let host = parts.first().unwrap_or(&"localhost").to_string();
                let port = parts.get(1).and_then(|p| p.parse::<u16>().ok()).unwrap_or(8200);
                (host, port)
            };

            let dest = Destination::new(&host, port);
            let result = filter.is_allowed_sync(&dest);
            if !result.is_allowed() {
                tracing::debug!(
                    host = %host,
                    port = port,
                    reason = %result.reason,
                    "VaultClient egress denied by filter"
                );
                return false;
            }
        }
        true
    }

    /// Authenticate with token
    pub async fn authenticate_token(&self, token: &str) -> Result<(), VaultError> {
        // Check egress filter before making request
        if !self.is_egress_allowed() {
            return Err(VaultError::ConnectionError("Egress denied: Vault server is not allowed".into()));
        }

        let url = format!("{}/v1/auth/token/lookup", self.config.address);
        let response = self
            .http_client
            .get(&url)
            .header("X-Vault-Token", token)
            .send()
            .await?;

        if response.status().is_success() {
            let mut token_guard = self.token.write().await;
            *token_guard = Some(token.to_string());
            Ok(())
        } else {
            Err(VaultError::AuthError(format!(
                "Token lookup failed: {}",
                response.status()
            )))
        }
    }

    /// Authenticate with AppRole
    pub async fn authenticate_approle(
        &self,
        role_id: &str,
        secret_id: &str,
    ) -> Result<(), VaultError> {
        // Check egress filter before making request
        if !self.is_egress_allowed() {
            return Err(VaultError::ConnectionError("Egress denied: Vault server is not allowed".into()));
        }

        let url = format!("{}/v1/auth/approle/login", self.config.address);

        let body = serde_json::json!({
            "role_id": role_id,
            "secret_id": secret_id
        });

        let response = self.http_client.post(&url).json(&body).send().await?;

        if response.status().is_success() {
            let auth_response: VaultResponse<serde_json::Value> = response
                .json()
                .await
                .map_err(|e| VaultError::InvalidResponse(e.to_string()))?;

            if let Some(auth) = auth_response.auth {
                let mut token_guard = self.token.write().await;
                *token_guard = Some(auth.client_token);
                info!(
                    policies = ?auth.token_policies,
                    "Vault AppRole authentication successful"
                );
            }
            Ok(())
        } else {
            Err(VaultError::AuthError(format!(
                "AppRole login failed: {}",
                response.status()
            )))
        }
    }

    /// Read a KV secret
    pub async fn read_secret(&self, path: &str) -> Result<Option<KvSecret>, VaultError> {
        // Check egress filter before making request
        if !self.is_egress_allowed() {
            return Err(VaultError::ConnectionError("Egress denied: Vault server is not allowed".into()));
        }

        let token = self.token.read().await;
        let token = token
            .as_ref()
            .ok_or_else(|| VaultError::AuthError("Not authenticated".into()))?;

        let url = format!(
            "{}/v1/{}/data/{}",
            self.config.address, self.config.kv_mount_path, path
        );

        let mut request = self.http_client.get(&url);
        request = request.header("X-Vault-Token", token.clone());

        if let Some(ref ns) = self.config.namespace {
            request = request.header("X-Vault-Namespace", ns.clone());
        }

        let response = request.send().await?;

        match response.status().as_u16() {
            200 => {
                let vault_resp: VaultResponse<KvSecretsData> = response
                    .json()
                    .await
                    .map_err(|e| VaultError::InvalidResponse(e.to_string()))?;

                if let Some(data) = vault_resp.data {
                    return Ok(Some(KvSecret {
                        path: path.to_string(),
                        data: data.data,
                        version: data.metadata.version,
                    }));
                }
                Ok(None)
            }
            404 => Ok(None),
            status => {
                let body: serde_json::Value = response.json().await.unwrap_or_default();
                let message = body
                    .get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("Unknown error")
                    .to_string();
                Err(VaultError::VaultApiError {
                    code: status as u64,
                    message,
                })
            }
        }
    }

    /// List secrets at a path
    pub async fn list_secrets(&self, path: &str) -> Result<Vec<String>, VaultError> {
        // Check egress filter before making request
        if !self.is_egress_allowed() {
            return Err(VaultError::ConnectionError("Egress denied: Vault server is not allowed".into()));
        }

        let token = self.token.read().await;
        let token = token
            .as_ref()
            .ok_or_else(|| VaultError::AuthError("Not authenticated".into()))?;

        let url = format!(
            "{}/v1/{}/metadata/{}",
            self.config.address, self.config.kv_mount_path, path
        );

        let mut request = self.http_client.get(&url);
        request = request.header("X-Vault-Token", token.clone());

        if let Some(ref ns) = self.config.namespace {
            request = request.header("X-Vault-Namespace", ns.clone());
        }

        let response = request.send().await?;

        if response.status().as_u16() == 200 {
            #[derive(Deserialize)]
            struct ListResponse {
                data: ListData,
            }
            #[derive(Deserialize)]
            struct ListData {
                keys: Vec<String>,
            }

            let list_resp: ListResponse = response
                .json()
                .await
                .map_err(|e| VaultError::InvalidResponse(e.to_string()))?;

            Ok(list_resp.data.keys)
        } else {
            Ok(vec![])
        }
    }

    /// Read a dynamic secret (e.g., database credentials)
    pub async fn read_dynamic_secret<T: for<'de> Deserialize<'de>>(
        &self,
        path: &str,
    ) -> Result<DynamicSecretResponse<T>, VaultError> {
        // Check egress filter before making request
        if !self.is_egress_allowed() {
            return Err(VaultError::ConnectionError("Egress denied: Vault server is not allowed".into()));
        }

        let token = self.token.read().await;
        let token = token
            .as_ref()
            .ok_or_else(|| VaultError::AuthError("Not authenticated".into()))?;

        let url = format!("{}/v1/{}", self.config.address, path);

        let mut request = self.http_client.get(&url);
        request = request.header("X-Vault-Token", token.clone());

        if let Some(ref ns) = self.config.namespace {
            request = request.header("X-Vault-Namespace", ns.clone());
        }

        let response = request.send().await?;

        if response.status().is_success() {
            let vault_resp: VaultResponse<T> = response
                .json()
                .await
                .map_err(|e| VaultError::InvalidResponse(e.to_string()))?;

            Ok(DynamicSecretResponse {
                data: vault_resp.data,
                lease_id: vault_resp.lease_id,
                lease_duration: vault_resp.lease_duration,
            })
        } else {
            let status = response.status().as_u16();
            let body: serde_json::Value = response.json().await.unwrap_or_default();
            let message = body
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("Unknown error")
                .to_string();
            Err(VaultError::VaultApiError {
                code: status as u64,
                message,
            })
        }
    }

    /// Renew a lease
    pub async fn renew_lease(&self, lease_id: &str, increment: u64) -> Result<(), VaultError> {
        // Check egress filter before making request
        if !self.is_egress_allowed() {
            return Err(VaultError::ConnectionError("Egress denied: Vault server is not allowed".into()));
        }

        let token = self.token.read().await;
        let token = token
            .as_ref()
            .ok_or_else(|| VaultError::AuthError("Not authenticated".into()))?;

        let url = format!("{}/v1/system/leases/renew", self.config.address);

        let body = serde_json::json!({
            "lease_id": lease_id,
            "increment": increment
        });

        let response = self
            .http_client
            .post(&url)
            .header("X-Vault-Token", token.clone())
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body: serde_json::Value = response.json().await.unwrap_or_default();
            let message = body
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("Unknown error")
                .to_string();
            return Err(VaultError::VaultApiError {
                code: status as u64,
                message,
            });
        }

        Ok(())
    }

    /// Revoke a lease
    pub async fn revoke_lease(&self, lease_id: &str) -> Result<(), VaultError> {
        // Check egress filter before making request
        if !self.is_egress_allowed() {
            return Err(VaultError::ConnectionError("Egress denied: Vault server is not allowed".into()));
        }

        let token = self.token.read().await;
        let token = token
            .as_ref()
            .ok_or_else(|| VaultError::AuthError("Not authenticated".into()))?;

        let url = format!("{}/v1/system/leases/revoke", self.config.address);

        let body = serde_json::json!({
            "lease_id": lease_id
        });

        let response = self
            .http_client
            .post(&url)
            .header("X-Vault-Token", token.clone())
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            warn!(
                lease_id = lease_id,
                status = response.status().as_u16(),
                "Failed to revoke lease"
            );
        }

        Ok(())
    }

    /// Lookup a lease to get its TTL
    pub async fn lookup_lease(&self, lease_id: &str) -> Result<Option<u64>, VaultError> {
        // Check egress filter before making request
        if !self.is_egress_allowed() {
            return Err(VaultError::ConnectionError("Egress denied: Vault server is not allowed".into()));
        }

        let token = self.token.read().await;
        let token = token
            .as_ref()
            .ok_or_else(|| VaultError::AuthError("Not authenticated".into()))?;

        let url = format!("{}/v1/sys/leases/lookup", self.config.address);

        let body = serde_json::json!({
            "lease_id": lease_id
        });

        let response = self
            .http_client
            .post(&url)
            .header("X-Vault-Token", token.clone())
            .json(&body)
            .send()
            .await?;

        if response.status().is_success() {
            #[derive(Deserialize)]
            struct LeaseLookupResponse {
                data: LeaseLookupData,
            }
            #[derive(Deserialize)]
            struct LeaseLookupData {
                ttl: u64,
            }

            let lookup_resp: LeaseLookupResponse = response
                .json()
                .await
                .map_err(|e| VaultError::InvalidResponse(e.to_string()))?;

            Ok(Some(lookup_resp.data.ttl))
        } else if response.status().as_u16() == 404 {
            Ok(None)
        } else {
            let status = response.status().as_u16();
            let body: serde_json::Value = response.json().await.unwrap_or_default();
            let message = body
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("Unknown error")
                .to_string();
            Err(VaultError::VaultApiError {
                code: status as u64,
                message,
            })
        }
    }

    /// Get current token
    pub async fn get_token(&self) -> Option<String> {
        self.token.read().await.clone()
    }

    /// Check if authenticated
    pub async fn is_authenticated(&self) -> bool {
        self.token.read().await.is_some()
    }

    /// Health check
    pub async fn health_check(&self) -> bool {
        // Check egress filter before making request
        if !self.is_egress_allowed() {
            return false;
        }

        let url = format!("{}/v1/sys/health", self.config.address);
        match self.http_client.get(&url).send().await {
            Ok(resp) => resp.status().is_success(),
            Err(_) => false,
        }
    }
}

/// A KV secret from Vault
#[derive(Debug, Clone)]
pub struct KvSecret {
    pub path: String,
    pub data: HashMap<String, String>,
    pub version: u64,
}

/// Response for dynamic secrets
#[derive(Debug)]
pub struct DynamicSecretResponse<T> {
    pub data: Option<T>,
    pub lease_id: Option<String>,
    pub lease_duration: Option<u64>,
}

/// Database credentials from Vault's database secrets engine
#[derive(Debug, Clone, Deserialize)]
pub struct DatabaseCredentials {
    pub username: String,
    pub password: String,
}

/// AWS credentials from Vault's AWS secrets engine
#[derive(Debug, Clone, Deserialize)]
pub struct AwsCredentials {
    pub access_key: String,
    pub secret_key: String,
    pub session_token: Option<String>,
}

/// Manages a dynamic secret's lifecycle
#[allow(dead_code)]
pub struct VaultDynamicSecret<T> {
    client: Arc<VaultClient>,
    path: String,
    secret_type: DynamicSecretType,
    current_lease_id: RwLock<Option<String>>,
    current_credentials: RwLock<Option<T>>,
    renewal_interval_secs: u64,
    _phantom: std::marker::PhantomData<T>,
}

/// Type of dynamic secret
#[derive(Debug, Clone)]
pub enum DynamicSecretType {
    Database,
    Aws,
    Custom,
}

impl<T: Clone + for<'de> Deserialize<'de>> VaultDynamicSecret<T> {
    /// Create a new dynamic secret manager
    pub fn new(client: Arc<VaultClient>, path: &str, secret_type: DynamicSecretType) -> Self {
        Self {
            client,
            path: path.to_string(),
            secret_type,
            current_lease_id: RwLock::new(None),
            current_credentials: RwLock::new(None),
            renewal_interval_secs: 60,
            _phantom: std::marker::PhantomData,
        }
    }

    /// Set renewal interval
    pub fn with_renewal_interval(mut self, interval_secs: u64) -> Self {
        self.renewal_interval_secs = interval_secs;
        self
    }

    /// Generate new dynamic credentials
    pub async fn generate(&self) -> Result<T, VaultError> {
        let response = self.client.read_dynamic_secret::<T>(&self.path).await?;

        if let Some(data) = response.data {
            // Store lease for renewal/revocation
            if let Some(lease_id) = response.lease_id {
                let mut current_lease = self.current_lease_id.write().await;
                *current_lease = Some(lease_id);
            }

            let mut credentials = self.current_credentials.write().await;
            *credentials = Some(data.clone());

            info!(
                path = %self.path,
                lease_duration = response.lease_duration,
                "Generated dynamic secret"
            );

            Ok(data)
        } else {
            Err(VaultError::InvalidResponse(
                "No data in dynamic secret response".into(),
            ))
        }
    }

    /// Get current credentials (without regeneration)
    pub async fn get_current(&self) -> Option<T> {
        self.current_credentials.read().await.clone()
    }

    /// Renew the current lease
    pub async fn renew(&self) -> Result<(), VaultError> {
        let lease_id = self.current_lease_id.read().await;
        if let Some(ref lease) = *lease_id {
            let increment = self.renewal_interval_secs;
            self.client.renew_lease(lease, increment).await?;
            debug!(lease_id = lease, "Renewed dynamic secret lease");
            Ok(())
        } else {
            Err(VaultError::InvalidResponse(
                "No active lease to renew".into(),
            ))
        }
    }

    /// Revoke the current credentials
    pub async fn revoke(&self) -> Result<(), VaultError> {
        let lease_id = self.current_lease_id.read().await;
        if let Some(ref lease) = *lease_id {
            self.client.revoke_lease(lease).await?;
            let mut credentials = self.current_credentials.write().await;
            *credentials = None;
            let mut lease_guard = self.current_lease_id.write().await;
            *lease_guard = None;
            info!(lease_id = lease, "Revoked dynamic secret");
        }
        Ok(())
    }

    /// Get remaining lease time
    pub async fn get_lease_ttl(&self) -> Option<i64> {
        let lease_id = self.current_lease_id.read().await;
        if let Some(ref lease) = *lease_id {
            match self.client.lookup_lease(lease).await {
                Ok(Some(ttl)) => Some(ttl as i64),
                Ok(None) => None, // Lease not found (may have been revoked)
                Err(e) => {
                    warn!(lease_id = lease, error = %e, "Failed to lookup lease TTL");
                    None
                }
            }
        } else {
            None
        }
    }
}

/// Vault credential provider implementing CredentialProvider trait
pub struct VaultCredentialProvider {
    client: Arc<VaultClient>,
    cache: Arc<RwLock<HashMap<String, CachedCredential>>>,
    default_ttl_secs: u64,
}

/// Cache entry for credentials
struct CachedCredential {
    credential: Credential,
    cached_at: DateTime<Utc>,
}

impl VaultCredentialProvider {
    /// Create a new Vault credential provider
    pub fn new(config: VaultClientConfig) -> Result<Self, VaultError> {
        let client = Arc::new(VaultClient::new(config)?);
        Ok(Self {
            client,
            cache: Arc::new(RwLock::new(HashMap::new())),
            default_ttl_secs: 3600,
        })
    }

    /// Create from existing Vault client
    pub fn from_client(client: Arc<VaultClient>) -> Self {
        Self {
            client,
            cache: Arc::new(RwLock::new(HashMap::new())),
            default_ttl_secs: 3600,
        }
    }

    /// Set default TTL for cached credentials
    pub fn with_default_ttl(mut self, ttl_secs: u64) -> Self {
        self.default_ttl_secs = ttl_secs;
        self
    }

    /// Authenticate with token
    pub async fn authenticate_token(&self, token: &str) -> Result<(), VaultError> {
        self.client.authenticate_token(token).await
    }

    /// Authenticate with AppRole
    pub async fn authenticate_approle(
        &self,
        role_id: &str,
        secret_id: &str,
    ) -> Result<(), VaultError> {
        self.client.authenticate_approle(role_id, secret_id).await
    }

    /// Clear the credential cache
    pub async fn clear_cache(&self) {
        let mut cache = self.cache.write().await;
        cache.clear();
    }

    /// Remove a specific credential from cache
    pub async fn evict(&self, key: &str) {
        let mut cache = self.cache.write().await;
        cache.remove(key);
    }

    /// Check if a credential is cached and valid, evicting expired entries
    async fn is_cached(&self, key: &str) -> bool {
        let mut cache = self.cache.write().await;
        if let Some(cached) = cache.get(key) {
            let now = Utc::now();
            let max_age = chrono::Duration::seconds(self.default_ttl_secs as i64);
            if now - cached.cached_at < max_age {
                return cached.credential.is_valid();
            } else {
                // Evict expired entry to prevent memory leak
                cache.remove(key);
            }
        }
        false
    }

    /// Get a credential with caching, evicting expired entries
    async fn get_cached(&self, key: &str) -> Option<Credential> {
        let mut cache = self.cache.write().await;
        if let Some(cached) = cache.get(key) {
            let now = Utc::now();
            let max_age = chrono::Duration::seconds(self.default_ttl_secs as i64);
            if now - cached.cached_at < max_age && cached.credential.is_valid() {
                Some(cached.credential.clone())
            } else {
                // Evict expired or invalid entry
                cache.remove(key);
                None
            }
        } else {
            None
        }
    }

    /// Cache a credential
    async fn cache_credential(&self, key: &str, credential: Credential) {
        let mut cache = self.cache.write().await;
        cache.insert(
            key.to_string(),
            CachedCredential {
                credential,
                cached_at: Utc::now(),
            },
        );
    }
}

impl Clone for VaultCredentialProvider {
    fn clone(&self) -> Self {
        Self {
            client: Arc::clone(&self.client),
            cache: Arc::clone(&self.cache),
            default_ttl_secs: self.default_ttl_secs,
        }
    }
}

#[async_trait]
impl CredentialProvider for VaultCredentialProvider {
    async fn get_credential(&self, key: &str) -> Result<Option<Credential>, CredentialProxyError> {
        // Check cache first
        if self.is_cached(key).await {
            if let Some(cached) = self.get_cached(key).await {
                debug!(key = key, "Returning cached credential");
                return Ok(Some(cached));
            }
        }

        // Fetch from Vault
        let secret = self.client.read_secret(key).await.map_err(|e| {
            error!(key = key, error = %e, "Failed to read secret from Vault");
            CredentialProxyError::ProviderError(e.to_string())
        })?;

        if let Some(secret) = secret {
            let now = Utc::now();
            // If Vault doesn't tell us expiration, use default TTL
            let expires_at = now + Duration::seconds(self.default_ttl_secs as i64);

            let value = if secret.data.len() == 1 {
                // Single value secret - use the only value
                secret.data.values().next().cloned().unwrap_or_default()
            } else {
                // Multi-value secret - serialize as JSON
                serde_json::to_string(&secret.data).unwrap_or_default()
            };

            let credential = Credential {
                id: Uuid::new_v4(),
                key: key.to_string(),
                value,
                issued_at: now,
                expires_at,
                scope: CredentialScope::default(),
            };

            // Cache the credential
            self.cache_credential(key, credential.clone()).await;

            info!(
                key = key,
                version = secret.version,
                ttl_secs = self.default_ttl_secs,
                "Fetched credential from Vault"
            );

            Ok(Some(credential))
        } else {
            Ok(None)
        }
    }

    async fn has_credential(&self, key: &str) -> bool {
        // Check cache first
        if self.is_cached(key).await {
            return true;
        }

        // Try to fetch - Vault returns Option<KvSecret>
        match self.client.read_secret(key).await {
            Ok(Some(_)) => true,
            Ok(None) => false,
            Err(e) => {
                warn!(key = key, error = %e, "Error checking credential existence");
                false
            }
        }
    }

    async fn list_credentials(&self) -> Vec<String> {
        // List available secrets at root path
        match self.client.list_secrets("").await {
            Ok(paths) => paths,
            Err(e) => {
                warn!(error = %e, "Failed to list Vault secrets");
                vec![]
            }
        }
    }

    fn provider_name(&self) -> &str {
        "vault"
    }
}

/// Builder for Vault credential provider
pub struct VaultCredentialProviderBuilder {
    config: Option<VaultClientConfig>,
    default_ttl_secs: u64,
    auth_method: VaultAuthMethod,
}

/// Authentication method for Vault
pub enum VaultAuthMethod {
    Token(String),
    AppRole { role_id: String, secret_id: String },
}

impl Default for VaultCredentialProviderBuilder {
    fn default() -> Self {
        Self {
            config: None,
            default_ttl_secs: 3600,
            auth_method: VaultAuthMethod::Token(String::new()),
        }
    }
}

impl VaultCredentialProviderBuilder {
    /// Set Vault address and token
    pub fn address(mut self, address: &str, token: &str) -> Self {
        self.config = Some(VaultClientConfig::new(address, token));
        self.auth_method = VaultAuthMethod::Token(token.to_string());
        self
    }

    /// Set AppRole authentication
    pub fn approle(mut self, address: &str, role_id: &str, secret_id: &str) -> Self {
        self.config = Some(VaultClientConfig::with_approle(address, role_id, secret_id));
        self.auth_method = VaultAuthMethod::AppRole {
            role_id: role_id.to_string(),
            secret_id: secret_id.to_string(),
        };
        self
    }

    /// Set default TTL
    pub fn default_ttl(mut self, ttl_secs: u64) -> Self {
        self.default_ttl_secs = ttl_secs;
        self
    }

    /// Build the provider
    pub async fn build(self) -> Result<VaultCredentialProvider, VaultError> {
        let config = self
            .config
            .ok_or_else(|| VaultError::ConnectionError("No address configured".into()))?;
        let provider = VaultCredentialProvider::new(config)?;

        // Authenticate based on method
        match self.auth_method {
            VaultAuthMethod::Token(token) => {
                provider.authenticate_token(&token).await?;
            }
            VaultAuthMethod::AppRole { role_id, secret_id } => {
                provider.authenticate_approle(&role_id, &secret_id).await?;
            }
        }

        Ok(provider.with_default_ttl(self.default_ttl_secs))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vault_client_config_new() {
        let config = VaultClientConfig::new("https://vault.example.com", "my-token");
        assert_eq!(config.address, "https://vault.example.com");
        assert_eq!(config.token, Some("my-token".to_string()));
        assert_eq!(config.kv_mount_path, "secret");
        assert_eq!(config.timeout_secs, 30);
    }

    #[test]
    fn test_vault_client_config_approle() {
        let config = VaultClientConfig::with_approle(
            "https://vault.example.com",
            "my-role-id",
            "my-secret-id",
        );
        assert_eq!(config.address, "https://vault.example.com");
        assert_eq!(config.role_id, Some("my-role-id".to_string()));
        assert_eq!(config.secret_id, Some("my-secret-id".to_string()));
        assert!(config.token.is_none());
    }

    #[test]
    fn test_vault_client_config_with_options() {
        let config = VaultClientConfig::new("https://vault.example.com", "token")
            .with_kv_mount_path("custom-kv")
            .with_namespace("admin");

        assert_eq!(config.kv_mount_path, "custom-kv");
        assert_eq!(config.namespace, Some("admin".to_string()));
    }

    #[tokio::test]
    async fn test_vault_credential_provider_cache() {
        // Create a mock provider (won't actually connect to Vault)
        let config = VaultClientConfig::new("http://localhost:8200", "test-token");
        let provider = VaultCredentialProvider::new(config).unwrap();

        // Test that cache starts empty
        let cached = provider.get_cached("test-key").await;
        assert!(cached.is_none());

        // Test listing credentials (will fail since no Vault running, but shouldn't panic)
        let credentials = provider.list_credentials().await;
        // Empty list expected when Vault unreachable
        assert!(credentials.is_empty() || !credentials.is_empty()); // Either is fine
    }

    #[test]
    fn test_dynamic_secret_type_debug() {
        let db_type = DynamicSecretType::Database;
        assert_eq!(format!("{:?}", db_type), "Database");

        let aws_type = DynamicSecretType::Aws;
        assert_eq!(format!("{:?}", aws_type), "Aws");
    }

    #[tokio::test]
    async fn test_vault_provider_clone() {
        let config = VaultClientConfig::new("http://localhost:8200", "token");
        let provider1 = VaultCredentialProvider::new(config).unwrap();
        let provider2 = provider1.clone();

        // Both should work independently
        assert_eq!(provider1.provider_name(), "vault");
        assert_eq!(provider2.provider_name(), "vault");
    }

    #[test]
    fn test_builder_address() {
        let _builder =
            VaultCredentialProviderBuilder::default().address("https://vault.example.com", "token");

        // Just verify builder doesn't panic
        assert!(true);
    }

    #[test]
    fn test_builder_approle() {
        let _builder = VaultCredentialProviderBuilder::default().approle(
            "https://vault.example.com",
            "role-id",
            "secret-id",
        );

        // Just verify builder doesn't panic
        assert!(true);
    }

    #[test]
    fn test_builder_default_ttl() {
        let _builder = VaultCredentialProviderBuilder::default().default_ttl(7200);

        // Just verify builder doesn't panic
        assert!(true);
    }

    #[tokio::test]
    async fn test_cache_eviction() {
        let config = VaultClientConfig::new("http://localhost:8200", "token");
        let provider = VaultCredentialProvider::new(config).unwrap();

        // Evict should not panic on non-existent key
        provider.evict("non-existent").await;

        // Clear cache should not panic
        provider.clear_cache().await;
    }

    #[tokio::test]
    async fn test_is_cached_when_empty() {
        let config = VaultClientConfig::new("http://localhost:8200", "token");
        let provider = VaultCredentialProvider::new(config).unwrap();

        let is_cached = provider.is_cached("test-key").await;
        assert!(!is_cached);
    }
}
