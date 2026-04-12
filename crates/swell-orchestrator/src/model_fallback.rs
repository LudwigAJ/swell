//! Model fallback chain tests for orchestrator.
//!
//! Tests the model fallback chain: Sonnet → GPT-4o → Haiku for graceful degradation.

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use swell_llm::router::{ModelRouter, ModelRouterBuilder, TaskType};
    use swell_llm::MockLlm;

    /// Creates a mock backend that returns a specific response
    fn create_mock_backend(name: &'static str) -> Arc<dyn swell_llm::LlmBackend> {
        Arc::new(MockLlm::with_response(name, name))
    }

    /// Creates a mock backend that always fails
    fn create_failing_mock(name: &'static str) -> Arc<dyn swell_llm::LlmBackend> {
        Arc::new(MockLlm::failing(name))
    }

    #[tokio::test]
    async fn test_model_fallback_chain_primary_success() {
        // Test that primary model (Sonnet) is used when it succeeds
        let mut router = ModelRouter::new();

        let sonnet = create_mock_backend("claude-sonnet");
        let gpt_4o = create_mock_backend("gpt-4o");
        let haiku = create_mock_backend("claude-haiku");

        // Register with Sonnet → GPT-4o → Haiku chain
        router.register(
            TaskType::Coding,
            "claude-sonnet",
            sonnet.clone(),
            vec![
                ("gpt-4o".to_string(), gpt_4o.clone()),
                ("claude-haiku".to_string(), haiku.clone()),
            ],
        );

        let messages = vec![swell_llm::LlmMessage {
            role: swell_llm::LlmRole::User,
            content: "Write a function".to_string(),
        }];

        let config = swell_llm::LlmConfig {
            temperature: 0.7,
            max_tokens: 4096,
            stop_sequences: None,
        };

        let response = router
            .route(TaskType::Coding, messages, None, config)
            .await
            .unwrap();

        // Primary model should succeed
        assert!(response.content.contains("claude-sonnet"));
    }

    #[tokio::test]
    async fn test_model_fallback_chain_gpt_4o_fallback() {
        // Test that GPT-4o is tried when Sonnet fails
        let mut router = ModelRouter::new();

        let sonnet = create_failing_mock("claude-sonnet");
        let gpt_4o = create_mock_backend("gpt-4o");
        let haiku = create_mock_backend("claude-haiku");

        // Register with Sonnet → GPT-4o → Haiku chain
        router.register(
            TaskType::Coding,
            "claude-sonnet",
            sonnet,
            vec![
                ("gpt-4o".to_string(), gpt_4o.clone()),
                ("claude-haiku".to_string(), haiku.clone()),
            ],
        );

        let messages = vec![swell_llm::LlmMessage {
            role: swell_llm::LlmRole::User,
            content: "Write a function".to_string(),
        }];

        let config = swell_llm::LlmConfig {
            temperature: 0.7,
            max_tokens: 4096,
            stop_sequences: None,
        };

        let response = router
            .route(TaskType::Coding, messages, None, config)
            .await
            .unwrap();

        // GPT-4o should succeed as fallback
        assert!(response.content.contains("gpt-4o"));
    }

    #[tokio::test]
    async fn test_model_fallback_chain_haiku_final_fallback() {
        // Test that Haiku is tried when both Sonnet and GPT-4o fail
        let mut router = ModelRouter::new();

        let sonnet = create_failing_mock("claude-sonnet");
        let gpt_4o = create_failing_mock("gpt-4o");
        let haiku = create_mock_backend("claude-haiku");

        // Register with Sonnet → GPT-4o → Haiku chain
        router.register(
            TaskType::Coding,
            "claude-sonnet",
            sonnet,
            vec![
                ("gpt-4o".to_string(), gpt_4o),
                ("claude-haiku".to_string(), haiku.clone()),
            ],
        );

        let messages = vec![swell_llm::LlmMessage {
            role: swell_llm::LlmRole::User,
            content: "Write a function".to_string(),
        }];

        let config = swell_llm::LlmConfig {
            temperature: 0.7,
            max_tokens: 4096,
            stop_sequences: None,
        };

        let response = router
            .route(TaskType::Coding, messages, None, config)
            .await
            .unwrap();

        // Haiku should succeed as final fallback
        assert!(response.content.contains("claude-haiku"));
    }

    #[tokio::test]
    async fn test_model_fallback_chain_all_fail() {
        // Test that error is returned when all models in chain fail
        let mut router = ModelRouter::new();

        let sonnet = create_failing_mock("claude-sonnet");
        let gpt_4o = create_failing_mock("gpt-4o");
        let haiku = create_failing_mock("claude-haiku");

        // Register with Sonnet → GPT-4o → Haiku chain - all failing
        router.register(
            TaskType::Coding,
            "claude-sonnet",
            sonnet,
            vec![
                ("gpt-4o".to_string(), gpt_4o),
                ("claude-haiku".to_string(), haiku),
            ],
        );

        let messages = vec![swell_llm::LlmMessage {
            role: swell_llm::LlmRole::User,
            content: "Write a function".to_string(),
        }];

        let config = swell_llm::LlmConfig {
            temperature: 0.7,
            max_tokens: 4096,
            stop_sequences: None,
        };

        let result = router.route(TaskType::Coding, messages, None, config).await;

        // All models failed, should return error
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_model_fallback_chain_builder() {
        // Test the ModelRouterBuilder.with_model_fallback_chain() method
        let builder = ModelRouterBuilder::new();
        let router = builder
            .with_model_fallback_chain(
                "sonnet-key".to_string(),
                "gpt-4o-key".to_string(),
                "haiku-key".to_string(),
            )
            .build();

        // Verify that primary model is set for Coding tasks
        assert_eq!(
            router.primary_model(TaskType::Coding),
            Some("claude-sonnet-4-20250514")
        );

        // Verify that primary model is set for Planning tasks
        assert_eq!(
            router.primary_model(TaskType::Planning),
            Some("claude-sonnet-4-20250514")
        );

        // Verify that primary model is set for Fast tasks (Haiku is primary for Fast)
        assert_eq!(
            router.primary_model(TaskType::Fast),
            Some("claude-haiku-4-20250514")
        );

        // Verify that primary model is set for Review tasks
        assert_eq!(
            router.primary_model(TaskType::Review),
            Some("claude-sonnet-4-20250514")
        );

        // Verify that primary model is set for Default tasks
        assert_eq!(
            router.primary_model(TaskType::Default),
            Some("claude-sonnet-4-20250514")
        );
    }

    #[tokio::test]
    async fn test_model_fallback_chain_applies_to_all_task_types() {
        // Verify the fallback chain is properly set up for all task types
        let builder = ModelRouterBuilder::new();
        let router = builder
            .with_model_fallback_chain(
                "sonnet-key".to_string(),
                "gpt-4o-key".to_string(),
                "haiku-key".to_string(),
            )
            .build();

        // For each task type, verify the route exists and has proper fallbacks
        for task_type in [
            TaskType::Coding,
            TaskType::Planning,
            TaskType::Fast,
            TaskType::Review,
            TaskType::Default,
        ] {
            let route = router.get_route(task_type);
            assert!(route.is_some(), "Route for {:?} should exist", task_type);

            let route = route.unwrap();

            // Verify primary model is set
            assert!(
                !route.primary.model_name.is_empty(),
                "Primary model for {:?} should be set",
                task_type
            );

            // Verify fallbacks are set (at least one fallback for cross-provider fallback)
            assert!(
                !route.fallbacks.is_empty(),
                "At least one fallback should be set for {:?}",
                task_type
            );

            // Verify the chain length includes all three models
            let total_models = 1 + route.fallbacks.len();
            assert_eq!(
                total_models, 3,
                "{:?} should have 3 models in chain (primary + 2 fallbacks)",
                task_type
            );
        }
    }

    #[tokio::test]
    async fn test_model_fallback_chain_cross_provider() {
        // Test that the chain correctly handles cross-provider fallback
        // (Anthropic Sonnet → OpenAI GPT-4o → Anthropic Haiku)
        let mut router = ModelRouter::new();

        // Simulate Sonnet failure
        let sonnet = create_failing_mock("claude-sonnet-4-20250514");
        // GPT-4o succeeds
        let gpt_4o = create_mock_backend("gpt-4o");
        // Haiku as final fallback
        let haiku = create_mock_backend("claude-haiku-4-20250514");

        router.register(
            TaskType::Coding,
            "claude-sonnet-4-20250514",
            sonnet,
            vec![
                ("gpt-4o".to_string(), gpt_4o.clone()),
                ("claude-haiku-4-20250514".to_string(), haiku.clone()),
            ],
        );

        let messages = vec![swell_llm::LlmMessage {
            role: swell_llm::LlmRole::User,
            content: "Code review task".to_string(),
        }];

        let config = swell_llm::LlmConfig {
            temperature: 0.7,
            max_tokens: 4096,
            stop_sequences: None,
        };

        let response = router
            .route(TaskType::Coding, messages, None, config)
            .await
            .unwrap();

        // Cross-provider fallback to GPT-4o succeeded
        assert!(response.content.contains("gpt-4o"));
    }

    #[tokio::test]
    async fn test_model_fallback_fast_task_reversal() {
        // For Fast tasks, the priority is reversed: Haiku → Sonnet → GPT-4o
        // because Fast tasks prioritize speed and cost
        let builder = ModelRouterBuilder::new();
        let router = builder
            .with_model_fallback_chain(
                "sonnet-key".to_string(),
                "gpt-4o-key".to_string(),
                "haiku-key".to_string(),
            )
            .build();

        let route = router.get_route(TaskType::Fast).unwrap();

        // Primary for Fast should be Haiku
        assert_eq!(route.primary.model_name, "claude-haiku-4-20250514");

        // First fallback for Fast should be Sonnet
        assert_eq!(route.fallbacks[0].model_name, "claude-sonnet-4-20250514");

        // Second fallback for Fast should be GPT-4o
        assert_eq!(route.fallbacks[1].model_name, "gpt-4o");
    }
}
