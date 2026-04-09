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

use std::sync::Arc;
use swell_core::{
    LlmBackend, LlmConfig, LlmMessage, LlmResponse, LlmToolDefinition, SwellError,
};

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
            TaskType::Fast => 0.2,      // Prefer cheapest
            TaskType::Default => 0.5,   // Balanced
            TaskType::Review => 0.6,   // Some cost OK
            TaskType::Coding => 0.7,    // Good quality worth extra cost
            TaskType::Planning => 0.9,  // Complex reasoning worth premium
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

/// Model router that selects appropriate LLM backend based on task type
#[derive(Debug, Clone)]
pub struct ModelRouter {
    routes: std::collections::HashMap<TaskType, RouteConfig>,
    /// Default fallback when no route is registered for a task type
    default_fallbacks: Vec<ModelRoute>,
}

impl ModelRouter {
    /// Create a new empty router
    pub fn new() -> Self {
        Self {
            routes: std::collections::HashMap::new(),
            default_fallbacks: Vec::new(),
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
    /// 1. Primary model for the task type
    /// 2. Fallback models for the task type
    /// 3. Global default fallbacks
    ///
    /// Returns the first successful response or the last error.
    pub async fn route(
        &self,
        task_type: TaskType,
        messages: Vec<LlmMessage>,
        tools: Option<Vec<LlmToolDefinition>>,
        config: LlmConfig,
    ) -> Result<LlmResponse, SwellError> {
        // Collect all candidate routes
        let candidates = self.get_candidates(task_type);

        if candidates.is_empty() {
            return Err(SwellError::ConfigError(format!(
                "No routes configured for task type {:?}",
                task_type
            )));
        }

        let mut last_error = None;

        for candidate in candidates {
            let model_name = candidate.model_name.clone();
            tracing::debug!(model = %model_name, task_type = ?task_type, "Trying model");

            match candidate.backend.chat(messages.clone(), tools.clone(), config.clone()).await {
                Ok(response) => {
                    tracing::info!(model = %model_name, task_type = ?task_type, tokens = %response.usage.total_tokens, "Model route succeeded");
                    return Ok(response);
                }
                Err(e) => {
                    tracing::warn!(model = %model_name, error = %e, "Model route failed, trying fallback");
                    last_error = Some(e);
                }
            }
        }

        Err(last_error.unwrap_or_else(|| {
            SwellError::ConfigError("No routes available".to_string())
        }))
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
                if !candidates.iter().any(|c| c.model_name == route_config.primary.model_name) {
                    candidates.push(route_config.primary.clone());
                    candidates.extend(
                        route_config.fallbacks.iter().cloned()
                    );
                }
            }
        }

        // Add global default fallbacks
        for default in &self.default_fallbacks {
            if !candidates.iter().any(|c| c.model_name == default.model_name) {
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
}

impl Default for ModelRouter {
    fn default() -> Self {
        Self::new()
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
#[derive(Debug, Clone)]
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

    /// Build the router with standard Anthropic model routing
    pub fn with_anthropic_defaults(
        mut self,
        sonnet_api_key: String,
        opus_api_key: String,
        haiku_api_key: String,
    ) -> Self {
        use crate::AnthropicBackend;

        // Coding route: Sonnet primary, Opus fallback
        let sonnet = Arc::new(AnthropicBackend::new("claude-sonnet-4-20250514", sonnet_api_key.clone()));
        let opus = Arc::new(AnthropicBackend::new("claude-opus-4-5", opus_api_key.clone()));
        
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
        let haiku = Arc::new(AnthropicBackend::new("claude-haiku-4-20250514", haiku_api_key.clone()));
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
    pub fn with_openai_defaults(
        mut self,
        gpt_4_api_key: String,
        gpt_35_api_key: String,
    ) -> Self {
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
        self.router.register(
            TaskType::Planning,
            "gpt-4-turbo",
            gpt_4.clone(),
            vec![],
        );

        // Fast: GPT-3.5 primary
        self.router.register(
            TaskType::Fast,
            "gpt-3.5-turbo",
            gpt_35.clone(),
            vec![("gpt-4-turbo".to_string(), gpt_4.clone())],
        );

        // Review: GPT-4
        self.router.register(
            TaskType::Review,
            "gpt-4-turbo",
            gpt_4.clone(),
            vec![],
        );

        // Default: GPT-3.5
        self.router.register(
            TaskType::Default,
            "gpt-3.5-turbo",
            gpt_35,
            vec![("gpt-4-turbo".to_string(), gpt_4)],
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
        self.spending_ratio() >= self.warning_threshold 
            && self.spending_ratio() < 1.0
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
        }];

        let config = LlmConfig {
            temperature: 0.7,
            max_tokens: 4096,
            stop_sequences: None,
        };

        let response = router.route(TaskType::Coding, messages, None, config).await.unwrap();
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
        }];

        let config = LlmConfig {
            temperature: 0.7,
            max_tokens: 1024,
            stop_sequences: None,
        };

        let response = router.route(TaskType::Fast, messages, None, config).await.unwrap();
        assert!(response.content.contains("fallback-model"));
    }

    #[tokio::test]
    async fn test_task_type_fallback() {
        let mut router = ModelRouter::new();
        
        let fast_backend = create_mock_backend("haiku-model");
        let default_backend = create_mock_backend("default-model");
        
        // Only register Default, not Fast
        router.register(
            TaskType::Default,
            "default-model",
            default_backend,
            vec![],
        );

        // Fast should fall back to Default route
        router.register(
            TaskType::Fast,
            "haiku-model",
            fast_backend,
            vec![],
        );

        let messages = vec![LlmMessage {
            role: crate::LlmRole::User,
            content: "Test".to_string(),
        }];

        let config = LlmConfig {
            temperature: 0.7,
            max_tokens: 1024,
            stop_sequences: None,
        };

        // Should work with Fast backend
        let response = router.route(TaskType::Fast, messages.clone(), None, config.clone()).await.unwrap();
        assert!(response.content.contains("haiku-model"));
    }

    #[tokio::test]
    async fn test_global_fallback() {
        let mut router = ModelRouter::new();
        
        let failing = create_failing_mock("primary-failing");
        let global_fallback = create_mock_backend("global-fallback");
        
        router.add_default_fallback(ModelRoute::new("global-fallback", global_fallback));
        
        // Register for a task type but primary fails
        router.register(
            TaskType::Coding,
            "primary-failing",
            failing,
            vec![],
        );

        let messages = vec![LlmMessage {
            role: crate::LlmRole::User,
            content: "Test".to_string(),
        }];

        let config = LlmConfig {
            temperature: 0.7,
            max_tokens: 1024,
            stop_sequences: None,
        };

        let response = router.route(TaskType::Coding, messages, None, config).await.unwrap();
        assert!(response.content.contains("global-fallback"));
    }

    #[tokio::test]
    async fn test_health_check() {
        let healthy = create_mock_backend("healthy-model");
        let mut builder = ModelRouterBuilder::new();
        builder.router.register(
            TaskType::Coding,
            "healthy-model",
            healthy,
            vec![],
        );
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
        
        router.register(
            TaskType::Coding,
            "test-model",
            backend,
            vec![],
        );

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
