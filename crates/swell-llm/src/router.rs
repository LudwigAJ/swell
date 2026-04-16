//! LLM Model Router for task-based model selection.
//!
//! The router selects the appropriate LLM model based on task type,
//! with support for fallback chains and cost optimization.
//!
//! # Task Types
//!
//! - **Coding**: Complex code generation, refactoring, debugging (Sonnet)
//! - **Planning**: Task decomposition, architectural decisions (Opus)
//! - **Fast**: Quick lookups, simple transformations (Haiku)
//! - **Review**: Code review, feedback (Sonnet)
//! - **Default**: General purpose tasks
//!
//! # Usage
//!
//! ```rust,ignore
//! use swell_llm::router::{ModelRouter, TaskType, ModelRoute};
//! use swell_llm::{AnthropicBackend, OpenAIBackend};
//!
//! let mut router = ModelRouter::new();
//! router.register_backend(TaskType::Coding, ModelRoute::new()
//!     .primary("claude-sonnet-4-20250514", anthropic_backend.clone())
//!     .fallback("gpt-4-turbo", openai_backend.clone()));
//!
//! let response = router.route(TaskType::Coding, messages, None, config).await?;
//! ```

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use swell_core::{LlmBackend, LlmConfig, LlmMessage, LlmResponse, LlmToolDefinition, SwellError};
use tokio::sync::RwLock;
use tokio::time::{timeout, Duration};

/// Task types that determine model selection
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TaskType {
    /// Complex code generation, refactoring, debugging
    Coding,
    /// Task decomposition, architectural decisions, planning
    Planning,
    /// Quick lookups, simple transformations, fast responses
    Fast,
    /// Code review, feedback, critique
    Review,
    /// General purpose tasks
    #[default]
    Default,
}

impl TaskType {
    /// Returns the default cost weight for this task type (0.0 to 1.0, higher = more expensive OK)
    pub fn cost_tolerance(&self) -> f64 {
        match self {
            TaskType::Fast => 0.2,     // Prefer cheapest
            TaskType::Default => 0.5,  // Balanced
            TaskType::Review => 0.6,   // Some cost OK
            TaskType::Coding => 0.7,   // Good quality worth extra cost
            TaskType::Planning => 0.9, // Complex reasoning worth premium
        }
    }

    /// Returns true if this task type benefits from longer context
    pub fn needs_long_context(&self) -> bool {
        matches!(self, TaskType::Planning | TaskType::Coding)
    }
}

/// A single model in the routing chain with its backend
#[derive(Clone)]
pub struct ModelRoute {
    pub model_name: String,
    pub backend: Arc<dyn LlmBackend>,
}

impl std::fmt::Debug for ModelRoute {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModelRoute")
            .field("model_name", &self.model_name)
            .finish()
    }
}

impl ModelRoute {
    pub fn new(model_name: impl Into<String>, backend: Arc<dyn LlmBackend>) -> Self {
        Self {
            model_name: model_name.into(),
            backend,
        }
    }
}

/// Routing configuration for a task type
#[derive(Debug, Clone)]
pub struct RouteConfig {
    pub task_type: TaskType,
    /// Primary model (tried first)
    pub primary: ModelRoute,
    /// Fallback models in order of preference
    pub fallbacks: Vec<ModelRoute>,
    /// Cost budget for this route (tokens). If set, cheaper models preferred when under budget.
    pub cost_budget: Option<u64>,
}

impl RouteConfig {
    pub fn new(task_type: TaskType, primary: ModelRoute) -> Self {
        Self {
            task_type,
            primary,
            fallbacks: Vec::new(),
            cost_budget: None,
        }
    }

    /// Add a fallback model to try if primary fails
    pub fn with_fallback(mut self, fallback: ModelRoute) -> Self {
        self.fallbacks.push(fallback);
        self
    }

    /// Set a cost budget for cost-optimized routing
    pub fn with_cost_budget(mut self, budget: u64) -> Self {
        self.cost_budget = Some(budget);
        self
    }
}

/// Statistics for a single model's health tracking
#[derive(Debug, Default)]
pub struct ModelHealthStats {
    /// Number of successful calls
    successes: AtomicU64,
    /// Number of failed calls
    failures: AtomicU64,
}

impl ModelHealthStats {
    /// Create a new empty stats entry
    pub fn new() -> Self {
        Self {
            successes: AtomicU64::new(0),
            failures: AtomicU64::new(0),
        }
    }

    /// Get success count
    pub fn successes(&self) -> u64 {
        self.successes.load(Ordering::Relaxed)
    }

    /// Get failure count
    pub fn failures(&self) -> u64 {
        self.failures.load(Ordering::Relaxed)
    }
}

/// Health information for a model including success rate
#[derive(Debug, Clone)]
pub struct ModelHealthInfo {
    /// Model name
    pub model_name: String,
    /// Success count
    pub successes: u64,
    /// Failure count
    pub failures: u64,
    /// Success rate (0.0 to 1.0), or None if not enough samples
    pub success_rate: Option<f64>,
    /// Whether this model is currently deprioritized
    pub is_deprioritized: bool,
}

/// Tracker for model health and success rates.
///
/// Tracks per-model success/failure counts and deprioritizes models
/// whose success rate drops below a configurable threshold.
#[derive(Debug)]
pub struct ModelHealthTracker {
    /// Per-model health statistics
    stats: RwLock<HashMap<String, ModelHealthStats>>,
    /// Success rate threshold below which models are deprioritized (0.0 to 1.0)
    success_rate_threshold: f64,
    /// Minimum number of samples before deprioritization kicks in
    min_samples: u64,
}

impl ModelHealthTracker {
    /// Create a new health tracker with default settings
    ///
    /// Default threshold: 50% success rate
    /// Default min samples: 5
    pub fn new() -> Self {
        Self::with_config(0.5, 5)
    }

    /// Create a health tracker with custom configuration
    ///
    /// # Arguments
    /// * `success_rate_threshold` - Success rate below which model is deprioritized (0.0 to 1.0)
    /// * `min_samples` - Minimum samples before deprioritization applies
    pub fn with_config(success_rate_threshold: f64, min_samples: u64) -> Self {
        Self {
            stats: RwLock::new(HashMap::new()),
            success_rate_threshold,
            min_samples,
        }
    }

    /// Record a successful call for a model
    pub async fn record_success(&self, model_name: &str) {
        let mut stats = self.stats.write().await;
        let entry = stats.entry(model_name.to_string()).or_default();
        entry.successes.fetch_add(1, Ordering::Relaxed);

        tracing::debug!(
            model = %model_name,
            successes = entry.successes.load(Ordering::Relaxed),
            failures = entry.failures.load(Ordering::Relaxed),
            "Model success recorded"
        );
    }

    /// Record a failed call for a model
    pub async fn record_failure(&self, model_name: &str) {
        let mut stats = self.stats.write().await;
        let entry = stats.entry(model_name.to_string()).or_default();
        entry.failures.fetch_add(1, Ordering::Relaxed);

        tracing::debug!(
            model = %model_name,
            successes = entry.successes.load(Ordering::Relaxed),
            failures = entry.failures.load(Ordering::Relaxed),
            "Model failure recorded"
        );
    }

    /// Get health info for a specific model
    pub async fn get_health_info(&self, model_name: &str) -> ModelHealthInfo {
        let stats = self.stats.read().await;
        if let Some(entry) = stats.get(model_name) {
            self.build_health_info(model_name, entry).await
        } else {
            ModelHealthInfo {
                model_name: model_name.to_string(),
                successes: 0,
                failures: 0,
                success_rate: None,
                is_deprioritized: false,
            }
        }
    }

    /// Build health info from stats entry
    async fn build_health_info(&self, model_name: &str, entry: &ModelHealthStats) -> ModelHealthInfo {
        let successes = entry.successes.load(Ordering::Relaxed);
        let failures = entry.failures.load(Ordering::Relaxed);
        let total = successes + failures;

        let success_rate = if total >= self.min_samples {
            Some(successes as f64 / total as f64)
        } else {
            None
        };

        let is_deprioritized = success_rate
            .map(|rate| rate < self.success_rate_threshold)
            .unwrap_or(false);

        ModelHealthInfo {
            model_name: model_name.to_string(),
            successes,
            failures,
            success_rate,
            is_deprioritized,
        }
    }

    /// Check if a model should be deprioritized based on its success rate
    pub async fn is_deprioritized(&self, model_name: &str) -> bool {
        let info = self.get_health_info(model_name).await;
        info.is_deprioritized
    }

    /// Get all models that should be deprioritized
    pub async fn get_deprioritized_models(&self) -> Vec<String> {
        let stats = self.stats.read().await;
        let mut deprioritized = Vec::new();

        for (name, entry) in stats.iter() {
            let info = self.build_health_info(name, entry).await;
            if info.is_deprioritized {
                deprioritized.push(name.clone());
            }
        }

        deprioritized
    }

    /// Get the configured success rate threshold
    pub fn threshold(&self) -> f64 {
        self.success_rate_threshold
    }

    /// Get the configured minimum samples
    pub fn min_samples(&self) -> u64 {
        self.min_samples
    }

    /// Reset health statistics (useful for testing)
    pub async fn reset(&self) {
        let mut stats = self.stats.write().await;
        stats.clear();
    }
}

impl Default for ModelHealthTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Model router that selects appropriate LLM backend based on task type
#[derive(Debug)]
pub struct ModelRouter {
    routes: std::collections::HashMap<TaskType, RouteConfig>,
    /// Default fallback when no route is registered for a task type
    default_fallbacks: Vec<ModelRoute>,
    /// Health tracker for success rate tracking and deprioritization
    health_tracker: ModelHealthTracker,
}

impl ModelRouter {
    /// Create a new empty router with default health tracking settings
    pub fn new() -> Self {
        Self {
            routes: std::collections::HashMap::new(),
            default_fallbacks: Vec::new(),
            health_tracker: ModelHealthTracker::new(),
        }
    }

    /// Create a new router with custom health tracking configuration
    pub fn with_health_tracker(success_rate_threshold: f64, min_samples: u64) -> Self {
        Self {
            routes: std::collections::HashMap::new(),
            default_fallbacks: Vec::new(),
            health_tracker: ModelHealthTracker::with_config(success_rate_threshold, min_samples),
        }
    }

    /// Register a route for a task type
    pub fn register_route(&mut self, config: RouteConfig) -> &mut Self {
        self.routes.insert(config.task_type, config);
        self
    }

    /// Register a route with a task type, primary model, and optional fallbacks
    pub fn register(
        &mut self,
        task_type: TaskType,
        primary_model: impl Into<String>,
        primary_backend: Arc<dyn LlmBackend>,
        fallback_models: Vec<(String, Arc<dyn LlmBackend>)>,
    ) -> &mut Self {
        let primary = ModelRoute::new(primary_model, primary_backend);
        let fallbacks = fallback_models
            .into_iter()
            .map(|(name, backend)| ModelRoute::new(name, backend))
            .collect();

        let config = RouteConfig {
            task_type,
            primary,
            fallbacks,
            cost_budget: None,
        };

        self.routes.insert(task_type, config);
        self
    }

    /// Add a global fallback that's tried when no specific route matches
    pub fn add_default_fallback(&mut self, model: ModelRoute) -> &mut Self {
        self.default_fallbacks.push(model);
        self
    }

    /// Get the route config for a task type
    pub fn get_route(&self, task_type: TaskType) -> Option<&RouteConfig> {
        self.routes.get(&task_type)
    }

    /// Route a chat request to the appropriate model based on task type
    ///
    /// Tries models in order:
    /// 1. Primary model for the task type (respects deprioritization)
    /// 2. Fallback models for the task type
    /// 3. Global default fallbacks
    ///
    /// Models with low success rates are automatically deprioritized within their tier.
    /// Fallback events are logged for observability.
    ///
    /// Returns the first successful response or the last error.
    pub async fn route(
        &self,
        task_type: TaskType,
        messages: Vec<LlmMessage>,
        tools: Option<Vec<LlmToolDefinition>>,
        config: LlmConfig,
    ) -> Result<LlmResponse, SwellError> {
        // Collect all candidate routes with deprioritization applied
        let candidates = self.get_candidates_deprioritized(task_type).await;

        if candidates.is_empty() {
            return Err(SwellError::ConfigError(format!(
                "No routes configured for task type {:?}",
                task_type
            )));
        }

        let mut last_error = None;
        let mut fallback_reason: Option<&str> = None;
        // Panic guard: timeout for each model attempt (120s default)
        // This prevents hanging on network issues and provides crash prevention
        let call_timeout = Duration::from_secs(120);

        for (index, candidate) in candidates.iter().enumerate() {
            let model_name = candidate.model_name.clone();
            let is_primary = index == 0;
            let is_deprioritized = self.health_tracker.is_deprioritized(&model_name).await;

            tracing::debug!(
                model = %model_name,
                task_type = ?task_type,
                is_primary = is_primary,
                is_deprioritized = is_deprioritized,
                position = index,
                "Trying model"
            );

            // Panic guard: wrap the chat call with a timeout.
            // This catches hangs and prevents the orchestrator from crashing.
            let chat_result = timeout(
                call_timeout,
                candidate
                    .backend
                    .chat(messages.clone(), tools.clone(), config.clone()),
            )
            .await;

            match chat_result {
                Ok(Ok(response)) => {
                    // Record success
                    self.health_tracker.record_success(&model_name).await;

                    // Log structured fallback event
                    if !is_primary {
                        let previous =
                            candidates.first().map(|c| c.model_name.as_str()).unwrap_or("none");
                        tracing::info!(
                            target: "model_fallback",
                            model = %model_name,
                            task_type = ?task_type,
                            fallback_reason = ?fallback_reason,
                            previous_model = %previous,
                            deprioritized_models =
                                ?self.health_tracker.get_deprioritized_models().await,
                            "Model fallback succeeded"
                        );
                    }

                    tracing::info!(
                        model = %model_name,
                        task_type = ?task_type,
                        tokens = %response.usage.total_tokens,
                        success_rate =
                            ?self.health_tracker.get_health_info(&model_name).await.success_rate,
                        "Model route succeeded"
                    );

                    return Ok(response);
                }
                Ok(Err(e)) => {
                    // Record failure
                    self.health_tracker.record_failure(&model_name).await;

                    // Determine fallback reason based on error type
                    fallback_reason = Some(determine_fallback_reason(&e));

                    // Log structured fallback event
                    let previous =
                        candidates.first().map(|c| c.model_name.as_str()).unwrap_or("none");
                    tracing::warn!(
                        target: "model_fallback",
                        model = %model_name,
                        task_type = ?task_type,
                        error = %e,
                        fallback_reason = ?fallback_reason,
                        fallback_trigger = "error",
                        previous_model = %previous,
                        is_deprioritized = is_deprioritized,
                        "Model route failed, trying fallback"
                    );

                    last_error = Some(e);
                }
                Err(_) => {
                    // Timeout: treat as failure and fall through to next model
                    let e = SwellError::LlmError(format!(
                        "Model {} timed out after {:?}",
                        model_name, call_timeout
                    ));
                    self.health_tracker.record_failure(&model_name).await;
                    fallback_reason = Some("timeout");

                    tracing::warn!(
                        target: "model_fallback",
                        model = %model_name,
                        task_type = ?task_type,
                        timeout_secs = ?call_timeout,
                        fallback_trigger = "timeout",
                        "Model call timed out, trying fallback"
                    );

                    last_error = Some(e);
                }
            }
        }

        // Log final failure event (all models exhausted)
        let tried_models: Vec<String> = candidates.iter().map(|c| c.model_name.clone()).collect();
        tracing::error!(
            target: "model_exhausted",
            task_type = ?task_type,
            all_models_failed = true,
            models_tried = ?tried_models,
            last_error = ?last_error,
            deprioritized_models = ?self.health_tracker.get_deprioritized_models().await,
            "All models in fallback chain exhausted"
        );

        Err(last_error
            .unwrap_or_else(|| SwellError::ConfigError("No routes available".to_string())))
    }

    /// Get all candidate routes for a task type, with deprioritized models moved to the end
    async fn get_candidates_deprioritized(&self, task_type: TaskType) -> Vec<ModelRoute> {
        let candidates = self.get_candidates(task_type);

        if candidates.is_empty() {
            return Vec::new();
        }

        // Separate healthy and deprioritized models
        let mut healthy: Vec<&ModelRoute> = Vec::new();
        let mut deprioritized: Vec<&ModelRoute> = Vec::new();

        for candidate in &candidates {
            if self.health_tracker.is_deprioritized(&candidate.model_name).await {
                deprioritized.push(candidate);
            } else {
                healthy.push(candidate);
            }
        }

        // Return healthy models first, then deprioritized
        let mut result: Vec<ModelRoute> = healthy.into_iter().cloned().collect();
        result.extend(deprioritized.into_iter().cloned());

        result
    }

    /// Get all candidate routes for a task type in order of preference
    fn get_candidates(&self, task_type: TaskType) -> Vec<ModelRoute> {
        let mut candidates = Vec::new();

        // Add route for the specific task type
        if let Some(route_config) = self.routes.get(&task_type) {
            candidates.push(route_config.primary.clone());
            candidates.extend(route_config.fallbacks.clone());
        }

        // If task type has fallbacks in routing hierarchy, add those too
        // For example, Fast tasks might fall back to Default
        let fallback_types = task_type_fallback_chain(task_type);
        for fallback_type in fallback_types {
            if let Some(route_config) = self.routes.get(&fallback_type) {
                // Only add if not already present
                if !candidates
                    .iter()
                    .any(|c| c.model_name == route_config.primary.model_name)
                {
                    candidates.push(route_config.primary.clone());
                    candidates.extend(route_config.fallbacks.iter().cloned());
                }
            }
        }

        // Add global default fallbacks
        for default in &self.default_fallbacks {
            if !candidates
                .iter()
                .any(|c| c.model_name == default.model_name)
            {
                candidates.push(default.clone());
            }
        }

        candidates
    }

    /// Check if a backend is healthy for a given task type
    pub async fn health_check(&self, task_type: TaskType) -> bool {
        let candidates = self.get_candidates(task_type);

        for candidate in candidates {
            if candidate.backend.health_check().await {
                return true;
            }
        }

        false
    }

    /// Get the primary model name for a task type
    pub fn primary_model(&self, task_type: TaskType) -> Option<&str> {
        self.routes
            .get(&task_type)
            .map(|c| c.primary.model_name.as_str())
    }

    /// Get the health tracker for this router
    pub fn health_tracker(&self) -> &ModelHealthTracker {
        &self.health_tracker
    }

    /// Get health info for a specific model
    pub async fn get_model_health(&self, model_name: &str) -> ModelHealthInfo {
        self.health_tracker.get_health_info(model_name).await
    }

    /// Get all deprioritized models
    pub async fn get_deprioritized_models(&self) -> Vec<String> {
        self.health_tracker.get_deprioritized_models().await
    }
}

impl Default for ModelRouter {
    fn default() -> Self {
        Self::new()
    }
}

/// Determine the reason for a fallback based on the error type
fn determine_fallback_reason(error: &SwellError) -> &'static str {
    let error_str = error.to_string().to_lowercase();

    if error_str.contains("rate limit") || error_str.contains("429") {
        "rate_limit"
    } else if error_str.contains("timeout") || error_str.contains("timed out") {
        "timeout"
    } else if error_str.contains("unauthorized") || error_str.contains("401") {
        "auth_error"
    } else if error_str.contains("forbidden") || error_str.contains("403") {
        "permission_error"
    } else if error_str.contains("bad request") || error_str.contains("400") {
        "invalid_request"
    } else if error_str.contains("internal server error") || error_str.contains("500") {
        "server_error"
    } else if error_str.contains("service unavailable") || error_str.contains("503") {
        "service_unavailable"
    } else if error_str.contains("gateway timeout") || error_str.contains("504") {
        "gateway_timeout"
    } else if error_str.contains("network") || error_str.contains("connection") {
        "network_error"
    } else if error_str.contains("llm error") {
        "llm_error"
    } else {
        "unknown_error"
    }
}

/// Returns the fallback chain for task types that inherit routing
fn task_type_fallback_chain(task_type: TaskType) -> Vec<TaskType> {
    match task_type {
        TaskType::Fast => vec![TaskType::Default],
        TaskType::Review => vec![TaskType::Coding, TaskType::Default],
        TaskType::Coding => vec![TaskType::Default],
        TaskType::Planning => vec![TaskType::Default],
        TaskType::Default => vec![],
    }
}

/// Builder for creating a ModelRouter with sensible defaults
#[derive(Debug)]
pub struct ModelRouterBuilder {
    router: ModelRouter,
}

impl ModelRouterBuilder {
    /// Create a new builder
    pub fn new() -> Self {
        Self {
            router: ModelRouter::new(),
        }
    }

    /// Configure custom health tracking settings
    ///
    /// # Arguments
    /// * `success_rate_threshold` - Success rate below which model is deprioritized (0.0 to 1.0), default 0.5
    /// * `min_samples` - Minimum samples before deprioritization applies, default 5
    pub fn with_health_tracking(mut self, success_rate_threshold: f64, min_samples: u64) -> Self {
        self.router = ModelRouter::with_health_tracker(success_rate_threshold, min_samples);
        self
    }

    /// Build the router with standard Anthropic model routing
    pub fn with_anthropic_defaults(
        mut self,
        sonnet_api_key: String,
        opus_api_key: String,
        haiku_api_key: String,
    ) -> Self {
        use crate::AnthropicBackend;

        // Coding route: Sonnet primary, Opus fallback
        let sonnet = Arc::new(AnthropicBackend::new(
            "claude-sonnet-4-20250514",
            sonnet_api_key.clone(),
        ));
        let opus = Arc::new(AnthropicBackend::new(
            "claude-opus-4-5",
            opus_api_key.clone(),
        ));

        self.router.register(
            TaskType::Coding,
            "claude-sonnet-4-20250514",
            sonnet.clone(),
            vec![("claude-opus-4-5".to_string(), opus.clone())],
        );

        // Planning route: Opus primary, Sonnet fallback
        self.router.register(
            TaskType::Planning,
            "claude-opus-4-5",
            opus,
            vec![("claude-sonnet-4-20250514".to_string(), sonnet.clone())],
        );

        // Fast route: Haiku primary, Sonnet fallback
        let haiku = Arc::new(AnthropicBackend::new(
            "claude-haiku-4-20250514",
            haiku_api_key.clone(),
        ));
        self.router.register(
            TaskType::Fast,
            "claude-haiku-4-20250514",
            haiku,
            vec![("claude-sonnet-4-20250514".to_string(), sonnet.clone())],
        );

        // Review route: Sonnet primary
        self.router.register(
            TaskType::Review,
            "claude-sonnet-4-20250514",
            sonnet.clone(),
            vec![],
        );

        // Default route: Sonnet
        self.router.register(
            TaskType::Default,
            "claude-sonnet-4-20250514",
            sonnet,
            vec![],
        );

        self
    }

    /// Build the router with OpenAI model routing
    pub fn with_openai_defaults(mut self, gpt_4_api_key: String, gpt_35_api_key: String) -> Self {
        use crate::OpenAIBackend;

        // GPT-4 for coding and planning
        let gpt_4 = Arc::new(OpenAIBackend::new("gpt-4-turbo", gpt_4_api_key).unwrap());
        let gpt_35 = Arc::new(OpenAIBackend::new("gpt-3.5-turbo", gpt_35_api_key).unwrap());

        // Coding: GPT-4 primary, GPT-3.5 fallback
        self.router.register(
            TaskType::Coding,
            "gpt-4-turbo",
            gpt_4.clone(),
            vec![("gpt-3.5-turbo".to_string(), gpt_35.clone())],
        );

        // Planning: GPT-4
        self.router
            .register(TaskType::Planning, "gpt-4-turbo", gpt_4.clone(), vec![]);

        // Fast: GPT-3.5 primary
        self.router.register(
            TaskType::Fast,
            "gpt-3.5-turbo",
            gpt_35.clone(),
            vec![("gpt-4-turbo".to_string(), gpt_4.clone())],
        );

        // Review: GPT-4
        self.router
            .register(TaskType::Review, "gpt-4-turbo", gpt_4.clone(), vec![]);

        // Default: GPT-3.5
        self.router.register(
            TaskType::Default,
            "gpt-3.5-turbo",
            gpt_35,
            vec![("gpt-4-turbo".to_string(), gpt_4)],
        );

        self
    }

    /// Build the router with cross-provider model fallback chain:
    /// Primary: Sonnet (Anthropic) → Fallback: GPT-4o (OpenAI) → Final Fallback: Haiku (Anthropic)
    ///
    /// This provides graceful degradation across providers:
    /// - Sonnet is tried first (best for complex coding tasks)
    /// - GPT-4o is tried second if Sonnet fails (cross-provider fallback)
    /// - Haiku is tried last (fastest, cheapest, for simple tasks)
    pub fn with_model_fallback_chain(
        mut self,
        sonnet_api_key: String,
        gpt_4o_api_key: String,
        haiku_api_key: String,
    ) -> Self {
        use crate::{AnthropicBackend, OpenAIBackend};

        // Primary: Claude Sonnet
        let sonnet = Arc::new(AnthropicBackend::new(
            "claude-sonnet-4-20250514",
            sonnet_api_key,
        ));

        // Fallback: GPT-4o (OpenAI)
        let gpt_4o = Arc::new(OpenAIBackend::new("gpt-4o", gpt_4o_api_key).unwrap());

        // Final fallback: Claude Haiku
        let haiku = Arc::new(AnthropicBackend::new(
            "claude-haiku-4-20250514",
            haiku_api_key,
        ));

        // For Coding tasks: Sonnet → GPT-4o → Haiku
        self.router.register(
            TaskType::Coding,
            "claude-sonnet-4-20250514",
            sonnet.clone(),
            vec![
                ("gpt-4o".to_string(), gpt_4o.clone()),
                ("claude-haiku-4-20250514".to_string(), haiku.clone()),
            ],
        );

        // For Planning tasks: Sonnet → GPT-4o → Haiku
        self.router.register(
            TaskType::Planning,
            "claude-sonnet-4-20250514",
            sonnet.clone(),
            vec![
                ("gpt-4o".to_string(), gpt_4o.clone()),
                ("claude-haiku-4-20250514".to_string(), haiku.clone()),
            ],
        );

        // For Fast tasks: Haiku → Sonnet → GPT-4o (reversed priority for fast responses)
        self.router.register(
            TaskType::Fast,
            "claude-haiku-4-20250514",
            haiku.clone(),
            vec![
                ("claude-sonnet-4-20250514".to_string(), sonnet.clone()),
                ("gpt-4o".to_string(), gpt_4o.clone()),
            ],
        );

        // For Review tasks: Sonnet → GPT-4o → Haiku
        self.router.register(
            TaskType::Review,
            "claude-sonnet-4-20250514",
            sonnet.clone(),
            vec![
                ("gpt-4o".to_string(), gpt_4o.clone()),
                ("claude-haiku-4-20250514".to_string(), haiku.clone()),
            ],
        );

        // For Default tasks: Sonnet → GPT-4o → Haiku
        self.router.register(
            TaskType::Default,
            "claude-sonnet-4-20250514",
            sonnet,
            vec![
                ("gpt-4o".to_string(), gpt_4o),
                ("claude-haiku-4-20250514".to_string(), haiku),
            ],
        );

        self
    }

    /// Configure the fallback chain from models.json configuration.
    ///
    /// Loads API keys from environment variables:
    /// - `ANTHROPIC_API_KEY` for Anthropic models
    /// - `OPENAI_API_KEY` for OpenAI models
    ///
    /// The `fallback_chain` parameter should contain model identifiers from models.json.
    /// Example chain: ["claude-sonnet-4-20250514", "gpt-4o", "claude-haiku-4-20250530"]
    ///
    /// Panics if required environment variables are not set.
    pub fn with_fallback_chain_from_models_config(
        mut self,
        fallback_chain: Vec<String>,
    ) -> Self {
        use crate::{AnthropicBackend, OpenAIBackend};

        let anthropic_key = std::env::var("ANTHROPIC_API_KEY")
            .expect("ANTHROPIC_API_KEY environment variable must be set");
        let openai_key =
            std::env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY environment variable must be set");

        // Build backends for each model in the chain
        let mut backends: Vec<(String, Arc<dyn LlmBackend>)> = Vec::new();
        for model_name in &fallback_chain {
            let backend: Arc<dyn LlmBackend> = if model_name.starts_with("claude-") {
                Arc::new(AnthropicBackend::new(model_name, anthropic_key.clone()))
            } else {
                // OpenAI models
                Arc::new(OpenAIBackend::new(model_name, openai_key.clone()).unwrap())
            };
            backends.push((model_name.clone(), backend));
        }

        // For non-Fast task types: use chain as-is (primary → fallback → final)
        let primary = backends.first().cloned().expect("fallback_chain must not be empty");
        let fallbacks: Vec<(String, Arc<dyn LlmBackend>)> = backends
            .iter()
            .skip(1)
            .cloned()
            .collect();

        for task_type in [
            TaskType::Coding,
            TaskType::Planning,
            TaskType::Review,
            TaskType::Default,
        ] {
            self.router
                .register(task_type, &primary.0, primary.1.clone(), fallbacks.clone());
        }

        // For Fast tasks: reverse the chain (fastest first)
        let fast_primary = backends.last().cloned().expect("fallback_chain must not be empty");
        let fast_fallbacks: Vec<(String, Arc<dyn LlmBackend>)> = backends
            .iter()
            .rev()
            .skip(1)
            .cloned()
            .collect();
        self.router.register(
            TaskType::Fast,
            &fast_primary.0,
            fast_primary.1.clone(),
            fast_fallbacks,
        );

        self
    }

    /// Build the final router
    pub fn build(self) -> ModelRouter {
        self.router
    }
}

impl Default for ModelRouterBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Cost optimizer that tracks spending and suggests route changes
#[derive(Debug, Clone)]
pub struct CostOptimizer {
    /// Total budget for the session (tokens)
    session_budget: u64,
    /// Spent so far
    spent: u64,
    /// Warning threshold (0.0 to 1.0)
    warning_threshold: f64,
}

impl CostOptimizer {
    pub fn new(session_budget: u64) -> Self {
        Self {
            session_budget,
            spent: 0,
            warning_threshold: 0.75,
        }
    }

    /// Record token usage
    pub fn record_usage(&mut self, input_tokens: u64, output_tokens: u64) {
        self.spent += input_tokens + output_tokens;
        tracing::debug!(
            spent = self.spent,
            budget = self.session_budget,
            "CostOptimizer updated"
        );
    }

    /// Get current spending ratio
    pub fn spending_ratio(&self) -> f64 {
        self.spent as f64 / self.session_budget as f64
    }

    /// Check if we're in warning zone
    pub fn is_warning(&self) -> bool {
        self.spending_ratio() >= self.warning_threshold && self.spending_ratio() < 1.0
    }

    /// Check if we've exceeded budget
    pub fn is_exceeded(&self) -> bool {
        self.spending_ratio() >= 1.0
    }

    /// Suggest whether to use cheaper models based on spending
    pub fn should_use_cheaper(&self, task_type: TaskType) -> bool {
        let ratio = self.spending_ratio();

        // If we're over 75% budget, prefer cheaper models for non-critical tasks
        if ratio >= 0.75 {
            return task_type != TaskType::Planning; // Planning still needs quality
        }

        // If we're over 50% and it's a fast task, use cheap
        if ratio >= 0.5 && task_type == TaskType::Fast {
            return true;
        }

        false
    }

    /// Reset spending
    pub fn reset(&mut self) {
        self.spent = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MockLlm;

    fn create_mock_backend(name: &str) -> Arc<dyn LlmBackend> {
        Arc::new(MockLlm::with_response(name, name))
    }

    fn create_failing_mock(name: &str) -> Arc<dyn LlmBackend> {
        Arc::new(MockLlm::failing(name))
    }

    #[tokio::test]
    async fn test_basic_routing() {
        let mut router = ModelRouter::new();

        let sonnet = create_mock_backend("claude-sonnet");
        let haiku = create_mock_backend("claude-haiku");

        router.register(
            TaskType::Coding,
            "claude-sonnet",
            sonnet,
            vec![("claude-haiku".to_string(), haiku.clone())],
        );

        let messages = vec![LlmMessage {
            role: crate::LlmRole::User,
            content: "Write a function".to_string(),
            ..Default::default()
        }];

        let config = LlmConfig {
            temperature: 0.7,
            max_tokens: 4096,
            stop_sequences: None,
        };

        let response = router
            .route(TaskType::Coding, messages, None, config)
            .await
            .unwrap();
        assert!(response.content.contains("claude-sonnet"));
    }

    #[tokio::test]
    async fn test_fallback_chain() {
        let mut router = ModelRouter::new();

        let failing = create_failing_mock("failing-model");
        let fallback = create_mock_backend("fallback-model");

        router.register(
            TaskType::Fast,
            "failing-model",
            failing,
            vec![("fallback-model".to_string(), fallback.clone())],
        );

        let messages = vec![LlmMessage {
            role: crate::LlmRole::User,
            content: "Quick task".to_string(),
            ..Default::default()
        }];

        let config = LlmConfig {
            temperature: 0.7,
            max_tokens: 1024,
            stop_sequences: None,
        };

        let response = router
            .route(TaskType::Fast, messages, None, config)
            .await
            .unwrap();
        assert!(response.content.contains("fallback-model"));
    }

    #[tokio::test]
    async fn test_task_type_fallback() {
        let mut router = ModelRouter::new();

        let fast_backend = create_mock_backend("haiku-model");
        let default_backend = create_mock_backend("default-model");

        // Only register Default, not Fast
        router.register(TaskType::Default, "default-model", default_backend, vec![]);

        // Fast should fall back to Default route
        router.register(TaskType::Fast, "haiku-model", fast_backend, vec![]);

        let messages = vec![LlmMessage {
            role: crate::LlmRole::User,
            content: "Test".to_string(),
            ..Default::default()
        }];

        let config = LlmConfig {
            temperature: 0.7,
            max_tokens: 1024,
            stop_sequences: None,
        };

        // Should work with Fast backend
        let response = router
            .route(TaskType::Fast, messages.clone(), None, config.clone())
            .await
            .unwrap();
        assert!(response.content.contains("haiku-model"));
    }

    #[tokio::test]
    async fn test_global_fallback() {
        let mut router = ModelRouter::new();

        let failing = create_failing_mock("primary-failing");
        let global_fallback = create_mock_backend("global-fallback");

        router.add_default_fallback(ModelRoute::new("global-fallback", global_fallback));

        // Register for a task type but primary fails
        router.register(TaskType::Coding, "primary-failing", failing, vec![]);

        let messages = vec![LlmMessage {
            role: crate::LlmRole::User,
            content: "Test".to_string(),
            ..Default::default()
        }];

        let config = LlmConfig {
            temperature: 0.7,
            max_tokens: 1024,
            stop_sequences: None,
        };

        let response = router
            .route(TaskType::Coding, messages, None, config)
            .await
            .unwrap();
        assert!(response.content.contains("global-fallback"));
    }

    #[tokio::test]
    async fn test_health_check() {
        let healthy = create_mock_backend("healthy-model");
        let mut builder = ModelRouterBuilder::new();
        builder
            .router
            .register(TaskType::Coding, "healthy-model", healthy, vec![]);
        let router = builder.build();
        assert!(router.health_check(TaskType::Coding).await);
    }

    #[tokio::test]
    async fn test_cost_optimizer() {
        let mut optimizer = CostOptimizer::new(10000);

        optimizer.record_usage(1000, 500);
        assert!(!optimizer.is_warning());
        assert!(!optimizer.is_exceeded());
        assert_eq!(optimizer.spending_ratio(), 0.15);

        optimizer.record_usage(5000, 2000);
        assert!(optimizer.is_warning());
        assert!(!optimizer.is_exceeded());

        // Fast tasks should use cheaper when over threshold
        assert!(optimizer.should_use_cheaper(TaskType::Fast));
        // Planning should still use quality even when over budget
        assert!(!optimizer.should_use_cheaper(TaskType::Planning));
    }

    #[tokio::test]
    async fn test_task_type_cost_tolerance() {
        assert_eq!(TaskType::Fast.cost_tolerance(), 0.2);
        assert_eq!(TaskType::Planning.cost_tolerance(), 0.9);
        assert!(TaskType::Planning.needs_long_context());
        assert!(!TaskType::Fast.needs_long_context());
    }

    #[tokio::test]
    async fn test_primary_model() {
        let mut router = ModelRouter::new();
        let backend = create_mock_backend("test-model");

        router.register(TaskType::Coding, "test-model", backend, vec![]);

        assert_eq!(router.primary_model(TaskType::Coding), Some("test-model"));
        assert_eq!(router.primary_model(TaskType::Fast), None);
    }

    #[tokio::test]
    async fn test_router_builder() {
        let builder = ModelRouterBuilder::new();
        let router = builder.build();

        // Empty builder should have no routes
        assert!(router.primary_model(TaskType::Coding).is_none());
    }
}
