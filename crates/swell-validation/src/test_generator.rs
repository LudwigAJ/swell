//! Test Generator Module
//!
//! Generates unit, integration, and property-based tests from acceptance criteria.
//!
//! # Test Types
//!
//! - **Unit Tests**: Test individual functions/modules in isolation
//! - **Integration Tests**: Test interactions between modules/components
//! - **Property-Based Tests**: Test general properties/invariants across many inputs
//!
//! # Usage
//!
//! ```rust
//! use swell_validation::test_generator::{TestGenerator, TestGeneratorConfig};
//! use swell_validation::test_planning::{AcceptanceCriterion, CriterionCriticality};
//!
//! let generator = TestGenerator::new(TestGeneratorConfig::default());
//! let criteria = vec![AcceptanceCriterion {
//!     id: "AC-1".to_string(),
//!     text: "The system shall authenticate users with email and password".to_string(),
//!     category: "authentication".to_string(),
//!     criticality: CriterionCriticality::MustHave,
//!     test_hints: vec!["auth".to_string()],
//!     format: None,
//! }];
//!
//! let unit_tests = generator.generate_unit_tests(&criteria, "src/auth.rs");
//! let integration_tests = generator.generate_integration_tests(&criteria);
//! let property_tests = generator.generate_property_tests(&criteria);
//! ```

use crate::test_planning::{AcceptanceCriterion, CriterionCriticality};
use serde::{Deserialize, Serialize};

/// Configuration for test generation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestGeneratorConfig {
    /// Include doc tests
    pub include_doctests: bool,
    /// Include benchmarks
    pub include_benchmarks: bool,
    /// Minimum confidence threshold for generated tests
    pub min_confidence: f64,
    /// Maximum tests per criterion
    pub max_tests_per_criterion: usize,
    /// Template style: "rustdoc" or "business"
    pub template_style: String,
    /// Use proptest for property-based tests (requires proptest dependency)
    pub use_proptest: bool,
    /// Number of iterations for proptest tests
    pub proptest_iterations: usize,
    /// Maximum shrink iterations for proptest (when a counterexample is found)
    pub proptest_max_shrink_iters: usize,
}

impl Default for TestGeneratorConfig {
    fn default() -> Self {
        Self {
            include_doctests: true,
            include_benchmarks: false,
            min_confidence: 0.5,
            max_tests_per_criterion: 5,
            template_style: "rustdoc".to_string(),
            use_proptest: true,
            proptest_iterations: 256,
            proptest_max_shrink_iters: 1000,
        }
    }
}

/// A generated test with metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneratedTest {
    /// Test name
    pub name: String,
    /// Test module/path where this test should be placed
    pub module_path: String,
    /// The generated test code
    pub code: String,
    /// Type of test
    pub test_type: TestType,
    /// Which criteria this test covers
    pub covers_criteria: Vec<String>,
    /// Confidence score (0.0 to 1.0)
    pub confidence: f64,
    /// Tags/labels
    pub tags: Vec<String>,
}

/// Type of generated test
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TestType {
    /// Unit test
    Unit,
    /// Integration test
    Integration,
    /// Property-based test (basic, without proptest)
    Property,
    /// Property-based test using proptest framework
    PropertyProptest,
}

/// Test generation patterns for different criterion types
#[derive(Debug, Clone)]
struct TestPattern {
    /// Pattern name
    pub name: &'static str,
    /// Criterion text patterns that match this pattern
    pub matchers: Vec<&'static str>,
    /// Test type to generate
    pub test_type: TestType,
    /// Template for generating test code (reserved for future use)
    #[allow(dead_code)]
    pub template: &'static str,
}

/// The test generator
#[derive(Debug, Clone)]
pub struct TestGenerator {
    config: TestGeneratorConfig,
    patterns: Vec<TestPattern>,
}

impl Default for TestGenerator {
    fn default() -> Self {
        Self::new(TestGeneratorConfig::default())
    }
}

impl TestGenerator {
    /// Create a new test generator with default configuration
    pub fn new(config: TestGeneratorConfig) -> Self {
        let patterns = Self::default_patterns();
        Self { config, patterns }
    }

    /// Create with default configuration
    pub fn with_defaults() -> Self {
        Self::new(TestGeneratorConfig::default())
    }

    /// Get default test patterns
    fn default_patterns() -> Vec<TestPattern> {
        vec![
            // Authentication patterns -> Unit tests
            TestPattern {
                name: "authentication_unit",
                matchers: vec!["authenticate", "login", "password", "credential"],
                test_type: TestType::Unit,
                template: UNIT_AUTH_TEMPLATE,
            },
            // Validation patterns -> Unit tests
            TestPattern {
                name: "validation_unit",
                matchers: vec!["validate", "validation", "check", "verify"],
                test_type: TestType::Unit,
                template: UNIT_VALIDATION_TEMPLATE,
            },
            // Error handling patterns -> Unit tests
            TestPattern {
                name: "error_handling_unit",
                matchers: vec!["error", "fail", "panic", "exception"],
                test_type: TestType::Unit,
                template: UNIT_ERROR_TEMPLATE,
            },
            // API patterns -> Integration tests
            TestPattern {
                name: "api_integration",
                matchers: vec!["api", "endpoint", "request", "response", "http"],
                test_type: TestType::Integration,
                template: INTEGRATION_API_TEMPLATE,
            },
            // Data patterns -> Integration tests
            TestPattern {
                name: "data_integration",
                matchers: vec!["database", "store", "persist", "query", "data"],
                test_type: TestType::Integration,
                template: INTEGRATION_DATA_TEMPLATE,
            },
            // Concurrency patterns -> Property tests
            TestPattern {
                name: "concurrency_property",
                matchers: vec!["concurrent", "thread", "parallel", "sync", "mutex"],
                test_type: TestType::Property,
                template: PROPERTY_CONCURRENCY_TEMPLATE,
            },
            // Performance patterns -> Property tests
            TestPattern {
                name: "performance_property",
                matchers: vec!["performance", "latency", "throughput", "speed", "fast"],
                test_type: TestType::Property,
                template: PROPERTY_PERFORMANCE_TEMPLATE,
            },
            // Security patterns -> Unit + Integration
            TestPattern {
                name: "security_unit",
                matchers: vec!["security", "encrypt", "decrypt", "hash", "token"],
                test_type: TestType::Unit,
                template: UNIT_SECURITY_TEMPLATE,
            },
        ]
    }

    /// Find matching pattern for a criterion
    fn find_matching_pattern(&self, criterion: &AcceptanceCriterion) -> Option<&TestPattern> {
        let text_lower = criterion.text.to_lowercase();

        for pattern in &self.patterns {
            for matcher in &pattern.matchers {
                if text_lower.contains(matcher) {
                    return Some(pattern);
                }
            }
        }

        // Default to unit test if no pattern matches
        self.patterns.get(1) // validation pattern as default
    }

    /// Generate unit tests from acceptance criteria
    pub fn generate_unit_tests(
        &self,
        criteria: &[AcceptanceCriterion],
        _target_file: &str,
    ) -> Vec<GeneratedTest> {
        let mut tests = Vec::new();

        for criterion in criteria {
            if criterion.criticality == CriterionCriticality::MustHave
                || criterion.criticality == CriterionCriticality::ShouldHave
            {
                if let Some(pattern) = self.find_matching_pattern(criterion) {
                    if pattern.test_type == TestType::Unit
                        || pattern.test_type == TestType::Property
                    {
                        let test = self.generate_unit_test(criterion, pattern);
                        if test.confidence >= self.config.min_confidence {
                            tests.push(test);
                        }
                    }
                }
            }
        }

        // Limit tests per criterion
        if tests.len() > self.config.max_tests_per_criterion {
            tests.truncate(self.config.max_tests_per_criterion);
        }

        tests
    }

    /// Generate a single unit test
    fn generate_unit_test(
        &self,
        criterion: &AcceptanceCriterion,
        pattern: &TestPattern,
    ) -> GeneratedTest {
        let test_name = self.generate_test_name(criterion);
        let module_path = format!("tests/unit_{}", self.sanitize_name(&test_name));

        let code = match pattern.name {
            "authentication_unit" => self.generate_auth_test(criterion, &test_name),
            "validation_unit" => self.generate_validation_test(criterion, &test_name),
            "error_handling_unit" => self.generate_error_test(criterion, &test_name),
            "security_unit" => self.generate_security_test(criterion, &test_name),
            _ => self.generate_generic_unit_test(criterion, &test_name),
        };

        GeneratedTest {
            name: test_name,
            module_path,
            code,
            test_type: TestType::Unit,
            covers_criteria: vec![criterion.id.clone()],
            confidence: 0.85,
            tags: vec![pattern.name.to_string(), criterion.category.clone()],
        }
    }

    /// Generate authentication test
    fn generate_auth_test(&self, criterion: &AcceptanceCriterion, test_name: &str) -> String {
        let criterion_text = &criterion.text;
        let sanitized = self.sanitize_name(test_name);

        format!(
            r#"#[cfg(test)]
mod {sanitized}_tests {{
    use super::*;

    /// Test: {test_name}
    /// Criterion: {criterion_text}
    /// Criticality: {criticality:?}
    #[test]
    fn {sanitized}() {{
        // TODO: Set up test fixtures
        let _ = ();

        // TODO: Execute the behavior under test
        // assert!(result.is_ok());
    }}

    /// Test invalid credentials are rejected
    #[test]
    fn {sanitized}_invalid_credentials() {{
        // Empty credentials should fail
        let result = validate_credentials("", "");
        assert!(result.is_err(), "Empty credentials should be rejected");

        // Invalid format should fail
        let result = validate_credentials("not-email", "weak");
        assert!(result.is_err(), "Invalid credentials should be rejected");
    }}

    /// Test edge cases for {test_name}
    #[test]
    fn {sanitized}_edge_cases() {{
        // Very long input
        let long_input = "a".repeat(10000);
        let result = validate_credentials(&long_input, &long_input);
        assert!(result.is_err() || result.is_ok()); // Should handle gracefully

        // Special characters
        let result = validate_credentials("user@example.com", "p@$$w0rd!");
        // Should handle special characters appropriately
        let _ = result;
    }}
}}"#,
            sanitized = sanitized,
            test_name = test_name,
            criterion_text = criterion_text,
            criticality = criterion.criticality
        )
    }

    /// Generate validation test
    fn generate_validation_test(&self, criterion: &AcceptanceCriterion, test_name: &str) -> String {
        let criterion_text = &criterion.text;
        let sanitized = self.sanitize_name(test_name);

        format!(
            r#"#[cfg(test)]
mod {sanitized}_tests {{
    use super::*;

    /// Test: {test_name}
    /// Criterion: {criterion_text}
    #[test]
    fn {sanitized}_valid_input() {{
        let valid_cases = vec![
            // TODO: Add valid input cases
        ];

        for input in valid_cases {{
            let result = validate_input(input);
            assert!(
                result.is_ok(),
                "Valid input should pass validation: {{:?}}",
                input
            );
        }}
    }}

    #[test]
    fn {sanitized}_invalid_input() {{
        let invalid_cases = vec![
            // TODO: Add invalid input cases
        ];

        for input in invalid_cases {{
            let result = validate_input(input);
            assert!(
                result.is_err(),
                "Invalid input should fail validation: {{:?}}",
                input
            );
        }}
    }}

    #[test]
    fn {sanitized}_boundary_conditions() {{
        // Empty input
        assert!(validate_input("").is_err(), "Empty should be invalid");

        // Whitespace only
        assert!(validate_input("   ").is_err(), "Whitespace should be invalid");

        // Maximum length
        let max_len = 1000;
        let max_input = "a".repeat(max_len);
        let result = validate_input(&max_input);
        assert!(result.is_ok() || result.is_err()); // Depends on requirements
    }}
}}"#,
            sanitized = sanitized,
            test_name = test_name,
            criterion_text = criterion_text
        )
    }

    /// Generate error handling test
    fn generate_error_test(&self, criterion: &AcceptanceCriterion, test_name: &str) -> String {
        let criterion_text = &criterion.text;
        let sanitized = self.sanitize_name(test_name);

        format!(
            r#"#[cfg(test)]
mod {sanitized}_tests {{
    use super::*;

    /// Test: {test_name}
    /// Criterion: {criterion_text}
    #[test]
    fn {sanitized}_error_propagation() {{
        // Verify errors are properly propagated
        let result = operation_that_fails();
        assert!(result.is_err(), "Errors should be properly propagated");

        if let Err(e) = result {{
            // Verify error type is appropriate
            let error_string = format!("{{}}", e);
            assert!(!error_string.is_empty(), "Error should have a message");
        }}
    }}

    #[test]
    fn {sanitized}_no_panic_on_error() {{
        // Verify the system doesn't panic on error conditions
        let inputs = vec![
            // TODO: Add edge case inputs that might cause panics
        ];

        for input in inputs {{
            let result = std::panic::catch_unwind(|| {{
                let _ = process_input(input);
            }});
            assert!(result.is_ok(), "Should not panic on input: {{:?}}", input);
        }}
    }}

    #[test]
    fn {sanitized}_error_recovery() {{
        // Test that the system can recover from errors
        let _ = setup_state();
        let result = operation_that_fails();
        assert!(result.is_err());

        // Should be able to retry after error
        let retry_result = operation_that_fails();
        assert!(retry_result.is_err() || retry_result.is_ok()); // Either is valid
    }}
}}"#,
            sanitized = sanitized,
            test_name = test_name,
            criterion_text = criterion_text
        )
    }

    /// Generate security test
    fn generate_security_test(&self, criterion: &AcceptanceCriterion, test_name: &str) -> String {
        let criterion_text = &criterion.text;
        let sanitized = self.sanitize_name(test_name);

        format!(
            r#"#[cfg(test)]
mod {sanitized}_tests {{
    use super::*;

    /// Test: {test_name}
    /// Criterion: {criterion_text}
    #[test]
    fn {sanitized}_no_sensitive_data_leak() {{
        // Verify sensitive data doesn't leak in error messages
        let sensitive_data = "secret_password_123";

        let result = process_with_sensitive_data(sensitive_data);

        if let Err(e) = &result {{
            let error_msg = format!("{{}}", e);
            assert!(
                !error_msg.contains(sensitive_data),
                "Error message should not contain sensitive data"
            );
        }}
    }}

    #[test]
    fn {sanitized}_proper_encryption() {{
        // Verify data is properly encrypted
        let plaintext = "sensitive data";
        let encrypted = encrypt(plaintext);

        assert_ne!(
            encrypted, plaintext,
            "Encrypted data should differ from plaintext"
        );

        // Should be able to decrypt back
        let decrypted = decrypt(&encrypted);
        assert_eq!(
            decrypted, plaintext,
            "Decrypted data should match original plaintext"
        );
    }}

    #[test]
    fn {sanitized}_timing_attack_resistance() {{
        // Basic timing attack resistance test
        use std::time::Instant;

        let mut times = Vec::new();

        for _ in 0..10 {{
            let start = Instant::now();
            let _ = secure_compare("password1", "password2");
            let elapsed = start.elapsed();
            times.push(elapsed);
        }}

        // Verify times are relatively consistent (no obvious timing leaks)
        let avg: std::time::Duration = times.iter().sum::<std::time::Duration>() / times.len() as u32;
        for t in &times {{
            let diff = if *t > avg {{ *t - avg }} else {{ avg - *t }};
            assert!(diff < std::time::Duration::from_millis(10), "Timing should be consistent");
        }}
    }}
}}"#,
            sanitized = sanitized,
            test_name = test_name,
            criterion_text = criterion_text
        )
    }

    /// Generate generic unit test
    fn generate_generic_unit_test(
        &self,
        criterion: &AcceptanceCriterion,
        test_name: &str,
    ) -> String {
        let criterion_text = &criterion.text;
        let sanitized = self.sanitize_name(test_name);

        format!(
            r#"#[cfg(test)]
mod {sanitized}_tests {{
    use super::*;

    /// Test: {test_name}
    /// Criterion: {criterion_text}
    #[test]
    fn {sanitized}_basic() {{
        // TODO: Implement test
        let result = target_function();
        assert!(result.is_ok() || result.is_err()); // Update assertion
    }}

    #[test]
    fn {sanitized}_with_valid_input() {{
        // TODO: Add valid input test
        todo!("Implement test with valid input")
    }}

    #[test]
    fn {sanitized}_with_invalid_input() {{
        // TODO: Add invalid input test
        todo!("Implement test with invalid input")
    }}
}}"#,
            sanitized = sanitized,
            test_name = test_name,
            criterion_text = criterion_text
        )
    }

    /// Generate integration tests from acceptance criteria
    pub fn generate_integration_tests(
        &self,
        criteria: &[AcceptanceCriterion],
    ) -> Vec<GeneratedTest> {
        let mut tests = Vec::new();

        for criterion in criteria {
            // Integration tests for API, data, and workflow criteria
            let text_lower = criterion.text.to_lowercase();

            if text_lower.contains("api")
                || text_lower.contains("endpoint")
                || text_lower.contains("request")
                || text_lower.contains("database")
                || text_lower.contains("workflow")
            {
                let test = self.generate_integration_test(criterion);
                if test.confidence >= self.config.min_confidence {
                    tests.push(test);
                }
            }
        }

        if tests.len() > self.config.max_tests_per_criterion {
            tests.truncate(self.config.max_tests_per_criterion);
        }

        tests
    }

    /// Generate a single integration test
    fn generate_integration_test(&self, criterion: &AcceptanceCriterion) -> GeneratedTest {
        let test_name = self.generate_test_name(criterion);
        let sanitized = self.sanitize_name(&test_name);
        let module_path = format!("tests/integration_{}", sanitized);

        let code = if criterion.text.to_lowercase().contains("api")
            || criterion.text.to_lowercase().contains("endpoint")
        {
            self.generate_api_integration_test(criterion, &test_name)
        } else if criterion.text.to_lowercase().contains("database")
            || criterion.text.to_lowercase().contains("data")
        {
            self.generate_data_integration_test(criterion, &test_name)
        } else {
            self.generate_workflow_integration_test(criterion, &test_name)
        };

        GeneratedTest {
            name: test_name,
            module_path,
            code,
            test_type: TestType::Integration,
            covers_criteria: vec![criterion.id.clone()],
            confidence: 0.80,
            tags: vec!["integration".to_string(), criterion.category.clone()],
        }
    }

    /// Generate API integration test
    fn generate_api_integration_test(
        &self,
        criterion: &AcceptanceCriterion,
        test_name: &str,
    ) -> String {
        let criterion_text = &criterion.text;
        let sanitized = self.sanitize_name(test_name);

        format!(
            r#"#[cfg(test)]
mod {sanitized}_integration_tests {{
    use super::*; // TODO: Import your API client

    /// Integration Test: {test_name}
    /// Criterion: {criterion_text}
    #[tokio::test]
    async fn {sanitized}_happy_path() {{
        // TODO: Set up test environment
        // let client = setup_client();

        // Execute the workflow
        // let result = client.endpoint().await;

        // Verify success
        // assert!(result.is_ok());
        todo!("Implement happy path integration test")
    }}

    #[tokio::test]
    async fn {sanitized}_error_handling() {{
        // TODO: Test API error handling
        // let client = setup_client();
        // client.set_error_mode(true);

        // let result = client.endpoint().await;
        // assert!(result.is_err());

        todo!("Implement error handling integration test")
    }}

    #[tokio::test]
    async fn {sanitized}_concurrent_requests() {{
        use tokio::task;

        // TODO: Test concurrent API calls
        // let client = setup_client();

        let handles: Vec<_> = (0..10)
            .map(|_| {{
                task::spawn(async {{}})
            }})
            .collect();

        for handle in handles {{
            let _ = handle.await;
        }}

        todo!("Implement concurrent requests test")
    }}

    #[tokio::test]
    async fn {sanitized}_timeout_handling() {{
        // TODO: Test timeout behavior
        todo!("Implement timeout handling test")
    }}
}}"#,
            sanitized = sanitized,
            test_name = test_name,
            criterion_text = criterion_text
        )
    }

    /// Generate data integration test
    fn generate_data_integration_test(
        &self,
        criterion: &AcceptanceCriterion,
        test_name: &str,
    ) -> String {
        let criterion_text = &criterion.text;
        let sanitized = self.sanitize_name(test_name);

        format!(
            r#"#[cfg(test)]
mod {sanitized}_data_integration_tests {{
    use super::*;

    /// Integration Test: {test_name}
    /// Criterion: {criterion_text}
    #[tokio::test]
    async fn {sanitized}_data_persistence() {{
        // TODO: Set up test database
        // let db = TestDatabase::new().await;

        // Create entity
        // let entity = Entity::new("test");
        // let id = db.insert(&entity).await.unwrap();

        // Verify persistence
        // let retrieved = db.get(id).await.unwrap();
        // assert_eq!(retrieved.name, "test");

        todo!("Implement data persistence test")
    }}

    #[tokio::test]
    async fn {sanitized}_data_relationships() {{
        // TODO: Test data relationships
        // let db = TestDatabase::new().await;

        // Create parent and child
        // let parent = Parent::new("parent");
        // let parent_id = db.insert(&parent).await.unwrap();

        // let child = Child::new("child", parent_id);
        // let child_id = db.insert(&child).await.unwrap();

        // Verify relationship
        // let retrieved_child = db.get_child(child_id).await.unwrap();
        // assert_eq!(retrieved_child.parent_id, parent_id);

        todo!("Implement data relationships test")
    }}

    #[tokio::test]
    async fn {sanitized}_data_migration() {{
        // TODO: Test data migration scenarios
        todo!("Implement data migration test")
    }}

    #[tokio::test]
    async fn {sanitized}_concurrent_data_access() {{
        use tokio::task;

        // TODO: Test concurrent data access
        let handles: Vec<_> = (0..10)
            .map(|_| {{
                task::spawn(async {{}})
            }})
            .collect();

        for handle in handles {{
            let _ = handle.await;
        }}

        todo!("Implement concurrent data access test")
    }}
}}"#,
            sanitized = sanitized,
            test_name = test_name,
            criterion_text = criterion_text
        )
    }

    /// Generate workflow integration test
    fn generate_workflow_integration_test(
        &self,
        criterion: &AcceptanceCriterion,
        test_name: &str,
    ) -> String {
        let criterion_text = &criterion.text;
        let sanitized = self.sanitize_name(test_name);

        format!(
            r#"#[cfg(test)]
mod {sanitized}_workflow_integration_tests {{
    use super::*;

    /// Integration Test: {test_name}
    /// Criterion: {criterion_text}
    #[tokio::test]
    async fn {sanitized}_complete_workflow() {{
        // TODO: Implement complete workflow test
        // Step 1: Initialize
        // let state = initial_state();

        // Step 2: Execute workflow steps
        // let result = execute_workflow(state).await;

        // Step 3: Verify final state
        // assert!(result.is_ok());

        todo!("Implement complete workflow test")
    }}

    #[tokio::test]
    async fn {sanitized}_workflow_with_failures() {{
        // TODO: Test workflow behavior under failure conditions
        todo!("Implement workflow failure test")
    }}

    #[tokio::test]
    async fn {sanitized}_workflow_recovery() {{
        // TODO: Test workflow recovery after failure
        todo!("Implement workflow recovery test")
    }}
}}"#,
            sanitized = sanitized,
            test_name = test_name,
            criterion_text = criterion_text
        )
    }

    /// Generate property-based tests from acceptance criteria
    pub fn generate_property_tests(&self, criteria: &[AcceptanceCriterion]) -> Vec<GeneratedTest> {
        let mut tests = Vec::new();

        for criterion in criteria {
            // Property tests for robustness, concurrency, performance
            let text_lower = criterion.text.to_lowercase();

            if text_lower.contains("concurrent")
                || text_lower.contains("parallel")
                || text_lower.contains("thread")
                || text_lower.contains("performance")
                || text_lower.contains("robust")
                || text_lower.contains("invariant")
                || text_lower.contains("property")
            {
                let test = self.generate_property_test(criterion);
                if test.confidence >= self.config.min_confidence {
                    tests.push(test);
                }
            }
        }

        if tests.len() > self.config.max_tests_per_criterion {
            tests.truncate(self.config.max_tests_per_criterion);
        }

        tests
    }

    /// Generate a single property-based test
    fn generate_property_test(&self, criterion: &AcceptanceCriterion) -> GeneratedTest {
        let test_name = self.generate_test_name(criterion);
        let sanitized = self.sanitize_name(&test_name);
        let module_path = format!("tests/property_{}", sanitized);

        let code = if criterion.text.to_lowercase().contains("concurrent")
            || criterion.text.to_lowercase().contains("thread")
            || criterion.text.to_lowercase().contains("parallel")
        {
            self.generate_concurrency_property_test(criterion, &test_name)
        } else if criterion.text.to_lowercase().contains("performance")
            || criterion.text.to_lowercase().contains("speed")
        {
            self.generate_performance_property_test(criterion, &test_name)
        } else {
            self.generate_invariant_property_test(criterion, &test_name)
        };

        GeneratedTest {
            name: test_name,
            module_path,
            code,
            test_type: TestType::Property,
            covers_criteria: vec![criterion.id.clone()],
            confidence: 0.75,
            tags: vec!["property_based".to_string(), criterion.category.clone()],
        }
    }

    /// Generate concurrency property test
    fn generate_concurrency_property_test(
        &self,
        criterion: &AcceptanceCriterion,
        test_name: &str,
    ) -> String {
        let criterion_text = &criterion.text;
        let sanitized = self.sanitize_name(test_name);

        format!(
            r#"#[cfg(test)]
mod {sanitized}_property_tests {{
    use super::*;
    use std::sync::Arc;
    use tokio::task;

    /// Property Test: {test_name}
    /// Criterion: {criterion_text}
    #[tokio::test]
    async fn {sanitized}_concurrent_safety() {{
        // Property: Multiple concurrent operations should not cause data corruption
        let counter = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let num_tasks = 100;

        let handles: Vec<_> = (0..num_tasks)
            .map(|i| {{
                let counter = Arc::clone(&counter);
                task::spawn(async move {{
                    counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    i
                }})
            }})
            .collect();

        for handle in handles {{
            let _ = handle.await;
        }}

        // Property: Final count should equal number of tasks
        assert_eq!(
            counter.load(std::sync::atomic::Ordering::SeqCst),
            num_tasks,
            "Concurrent operations should all complete"
        );
    }}

    #[tokio::test]
    async fn {sanitized}_no_deadlock() {{
        // Property: Operations should complete without deadlock
        use std::time::Duration;

        let handle = task::spawn(async {{
            // TODO: Replace with actual concurrent operation
            tokio::time::sleep(Duration::from_millis(1)).await;
            42
        }});

        let result = tokio::time::timeout(Duration::from_secs(5), handle).await;
        assert!(result.is_ok(), "Operation should complete without deadlock");
    }}

    #[tokio::test]
    async fn {sanitized}_thread_safety() {{
        // Property: Shared state should be thread-safe
        use std::sync::Mutex;

        let mutex = Arc::new(Mutex::new(0));
        let mut handles = vec![];

        for _ in 0..10 {{
            let m = Arc::clone(&mutex);
            handles.push(task::spawn(async move {{
                let mut guard = m.lock().unwrap();
                *guard += 1;
            }}));
        }}

        for h in handles {{
            let _ = h.await;
        }}

        let final_value = *mutex.lock().unwrap();
        assert_eq!(final_value, 10, "All increments should be recorded");
    }}

    #[test]
    fn {sanitized}_send_sync_bounds() {{
        // Property: Type should be Send and Sync if used across threads
        fn assert_send_sync<T: Send + Sync>() {{}}

        // TODO: Replace with your actual type
        // assert_send_sync::<YourType>();

        // Placeholder assertion
        assert_send_sync::<std::sync::Mutex<i32>>();
    }}
}}"#,
            sanitized = sanitized,
            test_name = test_name,
            criterion_text = criterion_text
        )
    }

    /// Generate performance property test
    fn generate_performance_property_test(
        &self,
        criterion: &AcceptanceCriterion,
        test_name: &str,
    ) -> String {
        let criterion_text = &criterion.text;
        let sanitized = self.sanitize_name(test_name);

        format!(
            r#"#[cfg(test)]
mod {sanitized}_performance_property_tests {{
    use super::*;
    use std::time::Instant;

    /// Property Test: {test_name}
    /// Criterion: {criterion_text}
    #[test]
    fn {sanitized}_performance_bound() {{
        // Property: Operation should complete within expected time bounds
        let max_duration_ms = 100; // TODO: Set appropriate bound

        let start = Instant::now();
        // TODO: Replace with actual operation
        let _result = std::hint::black_box(42);
        let elapsed = start.elapsed();

        assert!(
            elapsed.as_millis() < max_duration_ms as u128,
            "Operation took {{}}ms, expected < {{}}ms",
            elapsed.as_millis(),
            max_duration_ms
        );
    }}

    #[test]
    fn {sanitized}_consistent_performance() {{
        // Property: Performance should be consistent across multiple runs
        let durations: Vec<std::time::Duration> = (0..100)
            .map(|_| {{
                let start = Instant::now();
                // TODO: Replace with actual operation
                std::hint::black_box(42);
                start.elapsed()
            }})
            .collect();

        // Calculate statistics
        let total: std::time::Duration = durations.iter().sum();
        let avg = total / durations.len() as u32;

        // Property: 95% of runs should be within 2x of average
        let outliers = durations.iter().filter(|d| *d > avg * 2).count();
        assert!(
            outliers < 5,
            "{{}} outliers found, expected < 5 (performance should be consistent)",
            outliers
        );
    }}

    #[test]
    fn {sanitized}_no_memory_leak() {{
        // Property: Memory usage should not grow unboundedly
        // Note: This is a basic check - use valgrind/async-std for real leak detection

        let get_memory = || {{
            // On systems that support it, get current memory usage
            // For now, we just ensure no panic occurs
            0
        }};

        let initial_memory = get_memory();

        // Run operation multiple times
        for _ in 0..1000 {{
            // TODO: Replace with actual operation
            std::hint::black_box(42);
        }}

        let final_memory = get_memory();
        let growth = final_memory - initial_memory;

        // Allow some growth but flag excessive growth
        assert!(
            growth < 1_000_000, // 1MB threshold (placeholder)
            "Memory grew by {{}}, possible memory leak",
            growth
        );
    }}

    #[test]
    fn {sanitized}_scalability() {{
        // Property: Operation should scale linearly or better
        // O(n) operations should complete in O(n) time

        let sizes = vec![100, 1000, 10000];
        let times: Vec<std::time::Duration> = sizes
            .iter()
            .map(|size| {{
                let start = Instant::now();
                // TODO: Replace with actual operation that scales with size
                let _ = std::hint::black_box(*size);
                start.elapsed()
            }})
            .collect();

        // Check scaling is not worse than O(n^2)
        // If O(n), time2/time1 ~= size2/size1
        // If O(n^2), time2/time1 ~= (size2/size1)^2
        let ratio_1_to_2 = times[1].as_secs_f64() / times[0].as_secs_f64().max(0.0001);
        let size_ratio = sizes[1] as f64 / sizes[0] as f64;

        assert!(
            ratio_1_to_2 < size_ratio * size_ratio,
            "Scaling appears worse than O(n^2): ratio={{:.2}}, expected < {{:.2}}",
            ratio_1_to_2,
            size_ratio * size_ratio
        );
    }}
}}"#,
            sanitized = sanitized,
            test_name = test_name,
            criterion_text = criterion_text
        )
    }

    /// Generate invariant property test
    fn generate_invariant_property_test(
        &self,
        criterion: &AcceptanceCriterion,
        test_name: &str,
    ) -> String {
        let criterion_text = &criterion.text;
        let sanitized = self.sanitize_name(test_name);

        format!(
            r#"#[cfg(test)]
mod {sanitized}_invariant_property_tests {{
    use super::*;

    /// Property Test: {test_name}
    /// Criterion: {criterion_text}

    /// Property: Identity function should preserve value
    #[test]
    fn {sanitized}_identity_preservation() {{
        let values = vec![
            // TODO: Add representative values
            1i32, 0, -1, 42, i32::MAX, i32::MIN,
        ];

        for value in values {{
            let result = std::hint::black_box(value);
            assert_eq!(
                result, value,
                "Identity operation should preserve value for {{}}", value
            );
        }}
    }}

    /// Property: Operation should be reversible
    #[test]
    fn {sanitized}_reversibility() {{
        // TODO: Implement reversibility test
        // let original = create_test_value();
        // let transformed = transform(original);
        // let recovered = reverse(transform);
        // assert_eq!(original, recovered);
        todo!("Implement reversibility test")
    }}

    /// Property: Composition of inverse operations should be identity
    #[test]
    fn {sanitized}_inverse_composition() {{
        // Property: f(g(x)) = x for inverse operations
        // TODO: Implement inverse composition test
        todo!("Implement inverse composition test")
    }}

    /// Property: Operation should be deterministic
    #[test]
    fn {sanitized}_determinism() {{
        let test_values = vec![
            // TODO: Add test values
        ];

        for value in test_values {{
            let result1 = operation(std::hint::black_box(value));
            let result2 = operation(std::hint::black_box(value));

            assert_eq!(
                result1, result2,
                "Operation should be deterministic for input {{}}", value
            );
        }}
    }}

    /// Property: Monoid identity laws
    #[test]
    fn {sanitized}_monoid_identity() {{
        // Property: combine(value, identity) == value
        // Property: combine(identity, value) == value
        let value = std::hint::black_box(42); // TODO: Replace with actual type

        // TODO: Implement monoid identity tests
        // assert_eq!(combine(value, identity), value);
        // assert_eq!(combine(identity, value), value);

        todo!("Implement monoid identity tests")
    }}
}}"#,
            sanitized = sanitized,
            test_name = test_name,
            criterion_text = criterion_text
        )
    }

    /// Generate proptest-based property tests from acceptance criteria
    ///
    /// These tests use the proptest framework to define invariants over input spaces.
    /// For example: "for all valid inputs, output length ≤ input length + N"
    ///
    /// # Invariant Patterns Generated
    ///
    /// - **Length bounds**: `output.len() <= input.len() + N`
    /// - **Value bounds**: `result >= min && result <= max`
    /// - **Reversibility**: `reverse(forward(x)) == x`
    /// - **Monoid laws**: `combine(a, combine(b, c)) == combine(combine(a, b), c)`
    /// - **Determinism**: `f(x) == f(x)` across multiple calls
    ///
    /// # Configuration
    ///
    /// The `use_proptest`, `proptest_iterations`, and `proptest_max_shrink_iters`
    /// config fields control the generated test behavior.
    pub fn generate_proptest_tests(
        &self,
        criteria: &[AcceptanceCriterion],
    ) -> Vec<GeneratedTest> {
        let mut tests = Vec::new();

        // If proptest is disabled, return empty
        if !self.config.use_proptest {
            return tests;
        }

        for criterion in criteria {
            // Proptest tests for invariant-based criteria
            let text_lower = criterion.text.to_lowercase();

            // Match criteria that suggest invariant-based testing
            if text_lower.contains("invariant")
                || text_lower.contains("property")
                || text_lower.contains("for all")
                || text_lower.contains("always")
                || text_lower.contains("never")
                || text_lower.contains("must")
                || text_lower.contains("shall")
                || text_lower.contains("length")
                || text_lower.contains("bounds")
                || text_lower.contains("limit")
                || text_lower.contains("size")
                || text_lower.contains("revers")
                || text_lower.contains("deterministic")
            {
                let test = self.generate_proptest_invariant_test(criterion);
                if test.confidence >= self.config.min_confidence {
                    tests.push(test);
                }
            }
        }

        if tests.len() > self.config.max_tests_per_criterion {
            tests.truncate(self.config.max_tests_per_criterion);
        }

        tests
    }

    /// Generate a single proptest-based invariant test
    fn generate_proptest_invariant_test(
        &self,
        criterion: &AcceptanceCriterion,
    ) -> GeneratedTest {
        let test_name = self.generate_test_name(criterion);
        let sanitized = self.sanitize_name(&test_name);
        let module_path = format!("tests/proptest_{}", sanitized);

        let code = self.generate_proptest_invariant_code(criterion, &test_name);

        GeneratedTest {
            name: test_name,
            module_path,
            code,
            test_type: TestType::PropertyProptest,
            covers_criteria: vec![criterion.id.clone()],
            confidence: 0.80,
            tags: vec!["proptest".to_string(), "invariant".to_string(), criterion.category.clone()],
        }
    }

    /// Generate the actual proptest test code with invariant assertions
    fn generate_proptest_invariant_code(
        &self,
        criterion: &AcceptanceCriterion,
        test_name: &str,
    ) -> String {
        let criterion_text = &criterion.text;
        let sanitized = self.sanitize_name(test_name);
        let iterations = self.config.proptest_iterations;
        let max_shrink = self.config.proptest_max_shrink_iters;

        // Determine the type of invariant to test based on criterion text
        let text_lower = criterion.text.to_lowercase();

        if text_lower.contains("length") || text_lower.contains("size") || text_lower.contains("bound") {
            // Length/size invariant
            self.generate_length_invariant_test(sanitized, criterion_text, iterations, max_shrink)
        } else if text_lower.contains("revers") || text_lower.contains("inverse") {
            // Reversibility invariant
            self.generate_reversibility_invariant_test(sanitized, criterion_text, iterations, max_shrink)
        } else if text_lower.contains("determin") || text_lower.contains("consistent") {
            // Determinism invariant
            self.generate_determinism_invariant_test(sanitized, criterion_text, iterations, max_shrink)
        } else if text_lower.contains("combin") || text_lower.contains("associat") {
            // Monoid/associativity invariant
            self.generate_monoid_invariant_test(sanitized, criterion_text, iterations, max_shrink)
        } else if text_lower.contains("idempot") {
            // Idempotence invariant
            self.generate_idempotence_invariant_test(sanitized, criterion_text, iterations, max_shrink)
        } else {
            // Default: generic bounds invariant
            self.generate_bounds_invariant_test(sanitized, criterion_text, iterations, max_shrink)
        }
    }

    /// Generate a proptest test for length/size invariants
    ///
    /// Example invariant: "output.len() <= input.len() + N"
    fn generate_length_invariant_test(
        &self,
        sanitized: String,
        criterion_text: &str,
        iterations: usize,
        _max_shrink: usize,
    ) -> String {
        format!(
            r#"// This file was generated by swell-validation.
// Edit carefully - this test uses proptest for property-based testing.

#[cfg(test)]
mod {sanitized}_proptest_tests {{
    use proptest::{{prelude::*, collection::vec}};
    use std::collections::HashMap;

    /// Proptest Property: {test_name}
    /// Criterion: {criterion_text}
    ///
    /// # Invariant
    /// For all valid inputs, output length ≤ input length + N
    ///
    /// This test uses proptest to verify the length invariant holds
    /// across {iterations} randomly generated test cases.
    proptest! {{
        #![proptest_config(ProptestConfig::with_cases({iterations}))]

        /// Property: Output length should not exceed input length by more than fixed overhead
        #[test]
        fn {sanitized}_output_length_invariant(input in ".*") {{
            // TODO: Replace with actual function under test
            // The invariant: output.len() <= input.len() + N (where N is the max fixed overhead)
            let max_overhead = 100; // TODO: Set appropriate bound based on your function

            // Example: This is how you would test a function that transforms strings
            // let output = transform_with_padding(&input);
            // prop_assert!(output.len() <= input.len() + max_overhead);

            // Placeholder assertion - replace with actual invariant check
            let output = input.len(); // Replace with: transform_with_padding(&input).len()
            prop_assert!(output <= input.len() + max_overhead,
                "Output length {{}} exceeds input length {{}} + overhead {{}}",
                output, input.len(), max_overhead
            );
        }}

        /// Property: Empty input should produce valid output
        #[test]
        fn {sanitized}_empty_input_invariant() {{
            let input = "";

            // TODO: Replace with actual function under test
            // let output = transform_with_padding(&input);
            // prop_assert!(output.is_ok() || output.len() <= max_overhead);

            // Placeholder - verify empty string handling
            let len = input.len();
            prop_assert!(len == 0, "Empty input should have length 0");
        }}

        /// Property: Output length should be non-negative
        #[test]
        fn {sanitized}_non_negative_length(input in vec(0u8..100, 0..10000)) {{
            // TODO: Replace with actual function under test
            // let output = process_bytes(&input);
            // prop_assert!(output.len() >= 0);

            // Placeholder - verify length is non-negative
            let len = input.len() as isize;
            prop_assert!(len >= 0, "Length should be non-negative, got {{}}", len);
        }}

        /// Property: Maximum input size should not cause overflow
        #[test]
        fn {sanitized}_max_size_handling(input in vec(0u8..100, 9000..10000)) {{
            // TODO: Replace with actual function under test
            // This verifies that very large inputs are handled gracefully

            // Placeholder - verify max size handling
            prop_assert!(input.len() <= 10000, "Input should respect max size limit");
        }}
    }}

    /// Run with: cargo test {sanitized}_proptest -- --test-threads=1
    /// Or with more iterations:
    /// PROPTEST_MAX_SHRINK_ITERS={max_shrink} cargo test {sanitized}_proptest
}}

// =============================================================================
// QuickCheck-compatible version (alternative to proptest)
// =============================================================================
//
// If you prefer quickcheck over proptest, the equivalent tests would be:
//
// #[cfg(test)]
// mod {sanitized}_quickcheck_tests {{
//     use quickcheck::{{Arbitrary, Gen, TestResult}};
//     use quickcheck::Test;
//
//     quickcheck! {{
//         fn {sanitized}_length_invariant(input: String) -> TestResult {{
//             let max_overhead = 100;
//             let output = input.len(); // Replace with actual transform
//
//             if output <= input.len() + max_overhead {{
//                 TestResult::passed()
//             }} else {{
//                 TestResult::failed()
//             }}
//         }}
//     }}
// }}"#,
            sanitized = sanitized,
            test_name = "length_invariant",
            criterion_text = criterion_text,
            iterations = iterations,
            max_shrink = _max_shrink
        )
    }

    /// Generate a proptest test for reversibility/inverse invariants
    ///
    /// Example invariant: "reverse(forward(x)) == x"
    fn generate_reversibility_invariant_test(
        &self,
        sanitized: String,
        criterion_text: &str,
        iterations: usize,
        _max_shrink: usize,
    ) -> String {
        format!(
            r#"// This file was generated by swell-validation.
// Edit carefully - this test uses proptest for property-based testing.

#[cfg(test)]
mod {sanitized}_proptest_tests {{
    use proptest::{{prelude::*, collection::vec}};

    /// Proptest Property: {test_name}
    /// Criterion: {criterion_text}
    ///
    /// # Invariant
    /// Reversibility: reverse(forward(x)) == x for all valid inputs
    ///
    /// This test uses proptest to verify the reversibility invariant holds
    /// across {iterations} randomly generated test cases.
    proptest! {{
        #![proptest_config(ProptestConfig::with_cases({iterations}))]

        /// Property: Forward then reverse should return original value
        #[test]
        fn {sanitized}_roundtrip_invariant(input in "\\PC*{{0..1000}}") {{
            // TODO: Replace with actual functions under test
            // let forward = encode(&input);
            // let reversed = decode(&forward);
            // prop_assert_eq!(reversed, input, "Roundtrip should preserve value");

            // Placeholder - verify the roundtrip concept
            let original = input.clone();
            let processed = format!("processed: {{}}", input); // Replace with actual transform
            let recovered = processed.strip_prefix("processed: ").unwrap_or(&processed).to_string();
            prop_assert!(recovered == original || processed.len() > 0,
                "Roundtrip should either recover original or produce valid output"
            );
        }}

        /// Property: Double reverse should return original
        #[test]
        fn {sanitized}_double_reverse_invariant(input in "\\PC*{{0..500}}") {{
            // TODO: Replace with actual reverse function
            // let once = reverse(&input);
            // let twice = reverse(&once);
            // prop_assert_eq!(twice, input, "Double reverse should return original");

            // Placeholder
            let reversed = input.chars().rev().collect::<String>();
            let double_reversed = reversed.chars().rev().collect::<String>();
            prop_assert_eq!(double_reversed, input,
                "Double reverse should return original value");
        }}

        /// Property: Inverse operations should be consistent
        #[test]
        fn {sanitized}_inverse_consistency(a in 0u32..10000, b in 0u32..10000) {{
            // TODO: Test inverse operation consistency
            // If f(a) = b, then f⁻¹(b) = a

            // Placeholder: test addition/subtraction inverse
            let sum = a.wrapping_add(b);
            let diff = sum.wrapping_sub(b);
            prop_assert_eq!(diff, a,
                "Inverse operations should be consistent: {{}} + {{}} = {{}}, {{}} - {{}} = {{}}",
                a, b, sum, sum, b, diff);
        }}
    }}

    /// Run with: cargo test {sanitized}_proptest -- --test-threads=1
}}

// =============================================================================
// QuickCheck-compatible version (alternative to proptest)
// =============================================================================
//
// If you prefer quickcheck over proptest, the equivalent tests would be:
//
// #[cfg(test)]
// mod {sanitized}_quickcheck_tests {{
//     use quickcheck::{{Arbitrary, Gen, TestResult}};
//
//     quickcheck! {{
//         fn {sanitized}_roundtrip_invariant(input: String) -> TestResult {{
//             let original = input.clone();
//             let processed = format!("processed: {{}}", input);
//             let recovered = processed.strip_prefix(\"processed: \").unwrap_or(&processed).to_string();
//
//             if recovered == original || processed.len() > 0 {{
//                 TestResult::passed()
//             }} else {{
//                 TestResult::failed()
//             }}
//         }}
//     }}
// }}"#,
            sanitized = sanitized,
            test_name = "reversibility_invariant",
            criterion_text = criterion_text,
            iterations = iterations
        )
    }

    /// Generate a proptest test for determinism invariants
    ///
    /// Example invariant: "f(x) == f(x)" for all inputs
    fn generate_determinism_invariant_test(
        &self,
        sanitized: String,
        criterion_text: &str,
        iterations: usize,
        _max_shrink: usize,
    ) -> String {
        format!(
            r#"// This file was generated by swell-validation.
// Edit carefully - this test uses proptest for property-based testing.

#[cfg(test)]
mod {sanitized}_proptest_tests {{
    use proptest::{{prelude::*, collection::vec}};

    /// Proptest Property: {test_name}
    /// Criterion: {criterion_text}
    ///
    /// # Invariant
    /// Determinism: f(x) == f(x) for all valid inputs (same input yields same output)
    ///
    /// This test uses proptest to verify the determinism invariant holds
    /// across {iterations} randomly generated test cases.
    proptest! {{
        #![proptest_config(ProptestConfig::with_cases({iterations}))]

        /// Property: Same input should produce same output (determinism)
        #[test]
        fn {sanitized}_determinism_invariant(input in "\\PC*{{0..1000}}") {{
            // TODO: Replace with actual function under test
            // let result1 = process(&input);
            // let result2 = process(&input);
            // prop_assert_eq!(result1, result2, "Same input should produce same output");

            // Placeholder - verify deterministic behavior
            let result1 = input.to_uppercase(); // Replace with actual function
            let result2 = input.to_uppercase();
            prop_assert_eq!(result1, result2,
                "Determinism violated: same input produced different outputs");
        }}

        /// Property: Multiple calls should be consistent
        #[test]
        fn {sanitized}_multiple_call_consistency(input in vec(0u8..256, 0..500)) {{
            // TODO: Test that multiple calls produce consistent results
            let results: Vec<_> = (0..3)
                .map(|_| {{ // Replace with: process(&input)
                    input.iter().sum::<u8>() as usize
                }})
                .collect();

            prop_assert!(results.iter().all(|r| *r == results[0]),
                "Multiple calls should produce consistent results: {{:?}}", results);
        }}

        /// Property: Identical complex inputs should produce identical outputs
        #[test]
        fn {sanitized}_complex_determinism(key in "\\PC*{{1..50}}", value in "\\PC*{{0..500}}") {{
            // TODO: Test determinism with composite inputs
            // let input1 = (key.clone(), value.clone());
            // let input2 = (key.clone(), value.clone());
            // let result1 = hash(&input1);
            // let result2 = hash(&input2);
            // prop_assert_eq!(result1, result2, "Identical composite inputs should hash identically");

            // Placeholder
            let hash1 = format!("{{}}:{{}}", key, value).len();
            let hash2 = format!("{{}}:{{}}", key, value).len();
            prop_assert_eq!(hash1, hash2,
                "Determinism violated for composite input");
        }}
    }}

    /// Run with: cargo test {sanitized}_proptest -- --test-threads=1
}}

// =============================================================================
// QuickCheck-compatible version (alternative to proptest)
// =============================================================================
//
// If you prefer quickcheck over proptest, the equivalent tests would be:
//
// #[cfg(test)]
// mod {sanitized}_quickcheck_tests {{
//     use quickcheck::{{Arbitrary, Gen, TestResult}};
//
//     quickcheck! {{
//         fn {sanitized}_determinism_invariant(input: String) -> TestResult {{
//             let result1 = input.to_uppercase();
//             let result2 = input.to_uppercase();
//
//             if result1 == result2 {{
//                 TestResult::passed()
//             }} else {{
//                 TestResult::failed()
//             }}
//         }}
//     }}
// }}"#,
            sanitized = sanitized,
            test_name = "determinism_invariant",
            criterion_text = criterion_text,
            iterations = iterations
        )
    }

    /// Generate a proptest test for monoid/associativity invariants
    ///
    /// Example invariant: "combine(a, combine(b, c)) == combine(combine(a, b), c)"
    fn generate_monoid_invariant_test(
        &self,
        sanitized: String,
        criterion_text: &str,
        iterations: usize,
        _max_shrink: usize,
    ) -> String {
        format!(
            r#"// This file was generated by swell-validation.
// Edit carefully - this test uses proptest for property-based testing.

#[cfg(test)]
mod {sanitized}_proptest_tests {{
    use proptest::{{prelude::*, collection::vec}};

    /// Proptest Property: {test_name}
    /// Criterion: {criterion_text}
    ///
    /// # Invariant
    /// Monoid laws: combine(a, combine(b, c)) == combine(combine(a, b), c)
    /// (associativity) and combine(a, identity) == combine(identity, a) == a (identity)
    ///
    /// This test uses proptest to verify the monoid invariants hold
    /// across {iterations} randomly generated test cases.
    proptest! {{
        #![proptest_config(ProptestConfig::with_cases({iterations}))]

        /// Property: Associativity - (a ⊕ b) ⊕ c == a ⊕ (b ⊕ c)
        #[test]
        fn {sanitized}_associativity_invariant(
            a in 0u32..1000,
            b in 0u32..1000,
            c in 0u32..1000
        ) {{
            // TODO: Replace with actual combine/operation function
            // let ab_c = combine(combine(a, b), c);
            // let a_bc = combine(a, combine(b, c));
            // prop_assert_eq!(ab_c, a_bc, "Associativity violated");

            // Placeholder: test with addition (which is associative)
            let ab_c = (a + b) + c;
            let a_bc = a + (b + c);
            prop_assert_eq!(ab_c, a_bc,
                "Associativity violated: ({{}} + {{}}) + {{}} = {{}} ≠ {{}} + ({{}} + {{}}) = {{}}",
                a, b, c, ab_c, a, b, c, a_bc);
        }}

        /// Property: Left identity - identity ⊕ a == a
        #[test]
        fn {sanitized}_left_identity_invariant(a in 0u32..1000) {{
            let identity = 0u32; // TODO: Replace with actual identity element

            // TODO: Replace with actual combine function
            // let result = combine(identity, a);
            // prop_assert_eq!(result, a, "Left identity violated");

            // Placeholder
            let result = identity + a;
            prop_assert_eq!(result, a,
                "Left identity violated: {{}} + {{}} = {{}} ≠ {{}}",
                identity, a, result, a);
        }}

        /// Property: Right identity - a ⊕ identity == a
        #[test]
        fn {sanitized}_right_identity_invariant(a in 0u32..1000) {{
            let identity = 0u32; // TODO: Replace with actual identity element

            // TODO: Replace with actual combine function
            // let result = combine(a, identity);
            // prop_assert_eq!(result, a, "Right identity violated");

            // Placeholder
            let result = a + identity;
            prop_assert_eq!(result, a,
                "Right identity violated: {{}} + {{}} = {{}} ≠ {{}}",
                a, identity, result, a);
        }}

        /// Property: Commutativity (if applicable) - a ⊕ b == b ⊕ a
        #[test]
        fn {sanitized}_commutativity_invariant(a in 0u32..1000, b in 0u32..1000) {{
            // TODO: Replace with actual combine function (if commutative)
            // let ab = combine(a, b);
            // let ba = combine(b, a);
            // prop_assert_eq!(ab, ba, "Commutativity violated");

            // Placeholder: addition is commutative
            let ab = a + b;
            let ba = b + a;
            prop_assert_eq!(ab, ba,
                "Commutativity violated: {{}} + {{}} = {{}} ≠ {{}}",
                a, b, ab, ba);
        }}
    }}

    /// Run with: cargo test {sanitized}_proptest -- --test-threads=1
}}

// =============================================================================
// QuickCheck-compatible version (alternative to proptest)
// =============================================================================
//
// If you prefer quickcheck over proptest, the equivalent tests would be:
//
// #[cfg(test)]
// mod {sanitized}_quickcheck_tests {{
//     use quickcheck::{{Arbitrary, Gen, TestResult}};
//
//     quickcheck! {{
//         fn {sanitized}_associativity_invariant(a: u32, b: u32, c: u32) -> TestResult {{
//             let ab_c = (a + b) + c;
//             let a_bc = a + (b + c);
//
//             if ab_c == a_bc {{
//                 TestResult::passed()
//             }} else {{
//                 TestResult::failed()
//             }}
//         }}
//     }}
// }}"#,
            sanitized = sanitized,
            test_name = "monoid_invariant",
            criterion_text = criterion_text,
            iterations = iterations
        )
    }

    /// Generate a proptest test for idempotence invariants
    ///
    /// Example invariant: "f(f(x)) == f(x)"
    fn generate_idempotence_invariant_test(
        &self,
        sanitized: String,
        criterion_text: &str,
        iterations: usize,
        _max_shrink: usize,
    ) -> String {
        format!(
            r#"// This file was generated by swell-validation.
// Edit carefully - this test uses proptest for property-based testing.

#[cfg(test)]
mod {sanitized}_proptest_tests {{
    use proptest::{{prelude::*, collection::vec}};

    /// Proptest Property: {test_name}
    /// Criterion: {criterion_text}
    ///
    /// # Invariant
    /// Idempotence: f(f(x)) == f(x) - applying the operation twice
    /// should produce the same result as applying it once
    ///
    /// This test uses proptest to verify the idempotence invariant holds
    /// across {iterations} randomly generated test cases.
    proptest! {{
        #![proptest_config(ProptestConfig::with_cases({iterations}))]

        /// Property: Double application should equal single application
        #[test]
        fn {sanitized}_idempotence_invariant(input in "\\PC*{{0..1000}}") {{
            // TODO: Replace with actual idempotent function under test
            // let once = normalize(&input);      // First application
            // let twice = normalize(&once);      // Second application
            // prop_assert_eq!(once, twice, "Idempotence violated: f(f(x)) != f(x)");

            // Placeholder: test with string normalization concept
            let once = input.trim().to_lowercase(); // Replace with actual function
            let twice = once.trim().to_lowercase();
            prop_assert_eq!(once, twice,
                "Idempotence violated: f(f(x)) = {{}} ≠ f(x) = {{}}", twice, once);
        }}

        /// Property: Multiple applications should equal single application
        #[test]
        fn {sanitized}_multi_idempotence_invariant(input in "\\PC*{{0..500}}") {{
            // TODO: Test that N applications equals 1 application
            let normalized: String = input.trim().to_lowercase(); // Replace with actual function

            // Apply multiple times
            let mut result = normalized.clone();
            for _ in 0..10 {{
                result = result.trim().to_lowercase();
            }}

            prop_assert_eq!(result, normalized,
                "Multi-application idempotence violated");
        }}

        /// Property: Idempotent operation should be stable
        #[test]
        fn {sanitized}_stability_invariant(items in vec(0u32..100, 1..50)) {{
            // TODO: Test idempotence on collections
            // let once = deduplicate(&items);
            // let twice = deduplicate(&once);
            // prop_assert_eq!(once, twice, "Collection idempotence violated");

            // Placeholder: deduplication concept
            let mut once = items.clone();
            once.sort();
            once.dedup();

            let mut twice = once.clone();
            twice.sort();
            twice.dedup();

            prop_assert_eq!(once, twice,
                "Idempotence violated on collection operation");
        }}
    }}

    /// Run with: cargo test {sanitized}_proptest -- --test-threads=1
}}

// =============================================================================
// QuickCheck-compatible version (alternative to proptest)
// =============================================================================
//
// If you prefer quickcheck over proptest, the equivalent tests would be:
//
// #[cfg(test)]
// mod {sanitized}_quickcheck_tests {{
//     use quickcheck::{{Arbitrary, Gen, TestResult}};
//
//     quickcheck! {{
//         fn {sanitized}_idempotence_invariant(input: String) -> TestResult {{
//             let once = input.trim().to_lowercase();
//             let twice = once.trim().to_lowercase();
//
//             if once == twice {{
//                 TestResult::passed()
//             }} else {{
//                 TestResult::failed()
//             }}
//         }}
//     }}
// }}"#,
            sanitized = sanitized,
            test_name = "idempotence_invariant",
            criterion_text = criterion_text,
            iterations = iterations
        )
    }

    /// Generate a proptest test for generic bounds invariants
    ///
    /// Example invariant: "result >= min && result <= max"
    fn generate_bounds_invariant_test(
        &self,
        sanitized: String,
        criterion_text: &str,
        iterations: usize,
        _max_shrink: usize,
    ) -> String {
        format!(
            r#"// This file was generated by swell-validation.
// Edit carefully - this test uses proptest for property-based testing.

#[cfg(test)]
mod {sanitized}_proptest_tests {{
    use proptest::{{prelude::*, collection::vec}};

    /// Proptest Property: {test_name}
    /// Criterion: {criterion_text}
    ///
    /// # Invariant
    /// Bounds: result >= min && result <= max for all valid inputs
    ///
    /// This test uses proptest to verify the bounds invariant holds
    /// across {iterations} randomly generated test cases.
    proptest! {{
        #![proptest_config(ProptestConfig::with_cases({iterations}))]

        /// Property: Output should be within valid range
        #[test]
        fn {sanitized}_bounds_invariant(input in 0u32..10000) {{
            // TODO: Replace with actual function under test
            // TODO: Set appropriate min_val and max_val based on your function's contract

            let min_val: u32 = 0;      // TODO: Set minimum expected value
            let max_val: u32 = 10000; // TODO: Set maximum expected value

            // Example: test a function that transforms u32 values
            // let result = transform(input);
            // prop_assert!(result >= min_val && result <= max_val,
            //     "Bounds violated: {{}} not in [{{}}, {{}}]", result, min_val, max_val);

            // Placeholder - just pass through the value
            let result = input;
            prop_assert!(result >= min_val && result <= max_val,
                "Bounds violated: {{}} not in [{{}}, {{}}]", result, min_val, max_val);
        }}

        /// Property: Output should be non-negative
        #[test]
        fn {sanitized}_non_negative_invariant(input in 0i32..10000) {{
            // TODO: Replace with actual function under test
            let result = input.abs(); // Replace with actual transform

            prop_assert!(result >= 0,
                "Non-negative invariant violated: {{}} < 0", result);
        }}

        /// Property: Output should not exceed maximum
        #[test]
        fn {sanitized}_max_bound_invariant(input in 0u32..10000) {{
            let max_val: u32 = u32::MAX; // TODO: Set appropriate maximum

            // Placeholder
            let result = input;
            prop_assert!(result <= max_val,
                "Max bound violated: {{}} > {{}}", result, max_val);
        }}

        /// Property: String operations should stay within length bounds
        #[test]
        fn {sanitized}_string_length_invariant(input in "\\PC*{{0..1000}}") {{
            let min_len = 0;  // TODO: Set minimum expected length
            let max_len = 1000; // TODO: Set maximum expected length

            // TODO: Replace with actual string-transforming function
            // let result = process_string(&input);
            // prop_assert!(result.len() >= min_len && result.len() <= max_len);

            // Placeholder
            let len = input.len();
            prop_assert!(len >= min_len && len <= max_len,
                "String length bounds violated: {{}} not in [{{}}, {{}}]",
                len, min_len, max_len);
        }}

        /// Property: Collection operations should maintain element bounds
        #[test]
        fn {sanitized}_collection_element_bounds(
            items in vec(0u32..1000, 1..100)
        ) {{
            // TODO: Test that collection operations maintain element bounds
            let min_elem = 0u32;  // TODO: Set minimum element value
            let max_elem = 1000u32; // TODO: Set maximum element value

            // Example: test that sorting doesn't change bounds
            let mut sorted = items.clone();
            sorted.sort();

            // Verify all elements still in bounds
            prop_assert!(sorted.iter().all(|&x| x >= min_elem && x <= max_elem),
                "Collection element bounds violated after sort");

            // Verify order changed (sort actually did something for non-empty, non-sorted input)
            if items.len() > 1 && items != sorted {{
                prop_assert!(true); // Sort changed the order as expected
            }}
        }}
    }}

    /// Run with: cargo test {sanitized}_proptest -- --test-threads=1
}}

// =============================================================================
// QuickCheck-compatible version (alternative to proptest)
// =============================================================================
//
// If you prefer quickcheck over proptest, the equivalent tests would be:
//
// #[cfg(test)]
// mod {sanitized}_quickcheck_tests {{
//     use quickcheck::{{Arbitrary, Gen, TestResult}};
//
//     quickcheck! {{
//         fn {sanitized}_bounds_invariant(input: u32) -> TestResult {{
//             let min_val: u32 = 0;
//             let max_val: u32 = 10000;
//             let result = input; // Replace with actual transform
//
//             if result >= min_val && result <= max_val {{
//                 TestResult::passed()
//             }} else {{
//                 TestResult::failed()
//             }}
//         }}
//     }}
// }}"#,
            sanitized = sanitized,
            test_name = "bounds_invariant",
            criterion_text = criterion_text,
            iterations = iterations
        )
    }

    /// Generate a test name from a criterion
    fn generate_test_name(&self, criterion: &AcceptanceCriterion) -> String {
        let words: Vec<&str> = criterion
            .text
            .split_whitespace()
            .filter(|w| w.len() > 3)
            .take(5)
            .collect();

        let base = words.join("_").to_lowercase();
        let sanitized: String = base
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '_')
            .collect();

        format!("test_{}", sanitized)
    }

    /// Sanitize a name for use in Rust code
    fn sanitize_name(&self, name: &str) -> String {
        name.chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect()
    }

    /// Generate all tests from acceptance criteria
    pub fn generate_all_tests(
        &self,
        criteria: &[AcceptanceCriterion],
        target_file: &str,
    ) -> TestGeneratorOutput {
        let unit_tests = self.generate_unit_tests(criteria, target_file);
        let integration_tests = self.generate_integration_tests(criteria);
        let property_tests = self.generate_property_tests(criteria);

        TestGeneratorOutput {
            unit_tests,
            integration_tests,
            property_tests,
            total_generated: 0, // Will be calculated below
        }
    }
}

/// Output from test generation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestGeneratorOutput {
    /// Generated unit tests
    pub unit_tests: Vec<GeneratedTest>,
    /// Generated integration tests
    pub integration_tests: Vec<GeneratedTest>,
    /// Generated property-based tests
    pub property_tests: Vec<GeneratedTest>,
    /// Total number of tests generated
    pub total_generated: usize,
}

impl TestGeneratorOutput {
    /// Calculate total generated tests
    pub fn calculate_totals(&mut self) {
        self.total_generated =
            self.unit_tests.len() + self.integration_tests.len() + self.property_tests.len();
    }

    /// Get all generated tests as a flat list
    pub fn all_tests(&self) -> Vec<&GeneratedTest> {
        let mut tests: Vec<&GeneratedTest> = Vec::new();
        tests.extend(self.unit_tests.iter().chain(self.integration_tests.iter()));
        tests.extend(self.property_tests.iter());
        tests
    }
}

// =============================================================================
// Test Templates (used by generators)
// =============================================================================

const UNIT_AUTH_TEMPLATE: &str = r#"
/// Authentication unit test template
#[test]
fn test_authentication() {
    // Test authentication logic
}
"#;

const UNIT_VALIDATION_TEMPLATE: &str = r#"
/// Validation unit test template
#[test]
fn test_validation() {
    // Test validation logic
}
"#;

const UNIT_ERROR_TEMPLATE: &str = r#"
/// Error handling unit test template
#[test]
fn test_error_handling() {
    // Test error handling
}
"#;

const UNIT_SECURITY_TEMPLATE: &str = r#"
/// Security unit test template
#[test]
fn test_security() {
    // Test security properties
}
"#;

const INTEGRATION_API_TEMPLATE: &str = r#"
/// API integration test template
#[tokio::test]
async fn test_api_integration() {
    // Test API integration
}
"#;

const INTEGRATION_DATA_TEMPLATE: &str = r#"
/// Data integration test template
#[tokio::test]
async fn test_data_integration() {
    // Test data integration
}
"#;

const PROPERTY_CONCURRENCY_TEMPLATE: &str = r#"
/// Concurrency property test template
#[tokio::test]
async fn test_concurrency_property() {
    // Test concurrency properties
}
"#;

const PROPERTY_PERFORMANCE_TEMPLATE: &str = r#"
/// Performance property test template
#[test]
fn test_performance_property() {
    // Test performance properties
}
"#;

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod test_generator_tests {
    use super::*;

    #[test]
    fn test_test_generator_new() {
        let config = TestGeneratorConfig::default();
        let generator = TestGenerator::new(config);
        assert!(!generator.patterns.is_empty());
    }

    #[test]
    fn test_test_generator_default() {
        let generator = TestGenerator::default();
        assert!(!generator.patterns.is_empty());
    }

    #[test]
    fn test_test_generator_with_defaults() {
        let generator = TestGenerator::with_defaults();
        assert!(!generator.patterns.is_empty());
    }

    #[test]
    fn test_generate_test_name() {
        let generator = TestGenerator::with_defaults();
        let criterion = AcceptanceCriterion {
            id: "AC-1".to_string(),
            text: "The system shall authenticate users with email and password".to_string(),
            category: "authentication".to_string(),
            criticality: CriterionCriticality::MustHave,
            test_hints: vec!["auth".to_string()],
            format: None,
        };

        let name = generator.generate_test_name(&criterion);
        assert!(name.starts_with("test_"));
        assert!(name.contains("authenticate"));
    }

    #[test]
    fn test_sanitize_name() {
        let generator = TestGenerator::with_defaults();

        // Sanitize should keep alphanumeric and underscore, filter out others
        // hyphens and dots become underscores in the output
        assert_eq!(generator.sanitize_name("test-name"), "test_name");
        assert_eq!(generator.sanitize_name("test.name"), "test_name");
        assert_eq!(generator.sanitize_name("test_name"), "test_name");
        assert_eq!(generator.sanitize_name("test name"), "test_name");
    }

    #[test]
    fn test_find_matching_pattern_authentication() {
        let generator = TestGenerator::with_defaults();
        let criterion = AcceptanceCriterion {
            id: "AC-1".to_string(),
            text: "The system shall authenticate users with email and password".to_string(),
            category: "authentication".to_string(),
            criticality: CriterionCriticality::MustHave,
            test_hints: vec![],
            format: None,
        };

        let pattern = generator.find_matching_pattern(&criterion);
        assert!(pattern.is_some());
        assert!(pattern.unwrap().name.contains("authentication"));
    }

    #[test]
    fn test_find_matching_pattern_validation() {
        let generator = TestGenerator::with_defaults();
        let criterion = AcceptanceCriterion {
            id: "AC-2".to_string(),
            text: "The system should validate input format".to_string(),
            category: "validation".to_string(),
            criticality: CriterionCriticality::ShouldHave,
            test_hints: vec![],
            format: None,
        };

        let pattern = generator.find_matching_pattern(&criterion);
        assert!(pattern.is_some());
        assert!(pattern.unwrap().name.contains("validation"));
    }

    #[test]
    fn test_find_matching_pattern_api() {
        let generator = TestGenerator::with_defaults();
        let criterion = AcceptanceCriterion {
            id: "AC-3".to_string(),
            text: "The API shall accept POST requests".to_string(),
            category: "api".to_string(),
            criticality: CriterionCriticality::MustHave,
            test_hints: vec![],
            format: None,
        };

        let pattern = generator.find_matching_pattern(&criterion);
        assert!(pattern.is_some());
        // API patterns should match integration test type
        assert_eq!(pattern.unwrap().test_type, TestType::Integration);
    }

    #[test]
    fn test_generate_unit_tests_basic() {
        let generator = TestGenerator::with_defaults();
        let criteria = vec![AcceptanceCriterion {
            id: "AC-1".to_string(),
            text: "The system shall authenticate users with email and password".to_string(),
            category: "authentication".to_string(),
            criticality: CriterionCriticality::MustHave,
            test_hints: vec!["auth".to_string()],
            format: None,
        }];

        let tests = generator.generate_unit_tests(&criteria, "src/auth.rs");
        assert!(!tests.is_empty());
    }

    #[test]
    fn test_generate_unit_tests_respects_min_confidence() {
        let mut config = TestGeneratorConfig::default();
        config.min_confidence = 0.99; // Very high threshold
        let generator = TestGenerator::new(config);

        let criteria = vec![AcceptanceCriterion {
            id: "AC-1".to_string(),
            text: "The system shall authenticate users".to_string(),
            category: "auth".to_string(),
            criticality: CriterionCriticality::MustHave,
            test_hints: vec![],
            format: None,
        }];

        let tests = generator.generate_unit_tests(&criteria, "src/auth.rs");
        // Should be empty because confidence is below threshold
        assert!(tests.is_empty());
    }

    #[test]
    fn test_generate_integration_tests_api() {
        let generator = TestGenerator::with_defaults();
        let criteria = vec![AcceptanceCriterion {
            id: "AC-1".to_string(),
            text: "The API shall accept POST requests at /api/users".to_string(),
            category: "api".to_string(),
            criticality: CriterionCriticality::MustHave,
            test_hints: vec!["api".to_string()],
            format: None,
        }];

        let tests = generator.generate_integration_tests(&criteria);
        assert!(!tests.is_empty());
        assert_eq!(tests[0].test_type, TestType::Integration);
    }

    #[test]
    fn test_generate_integration_tests_database() {
        let generator = TestGenerator::with_defaults();
        let criteria = vec![AcceptanceCriterion {
            id: "AC-1".to_string(),
            text: "Data shall be persisted to the database".to_string(),
            category: "data".to_string(),
            criticality: CriterionCriticality::MustHave,
            test_hints: vec!["database".to_string()],
            format: None,
        }];

        let tests = generator.generate_integration_tests(&criteria);
        assert!(!tests.is_empty());
    }

    #[test]
    fn test_generate_property_tests_concurrency() {
        let generator = TestGenerator::with_defaults();
        let criteria = vec![AcceptanceCriterion {
            id: "AC-1".to_string(),
            text: "The system shall handle concurrent requests safely".to_string(),
            category: "concurrency".to_string(),
            criticality: CriterionCriticality::MustHave,
            test_hints: vec!["concurrent".to_string()],
            format: None,
        }];

        let tests = generator.generate_property_tests(&criteria);
        assert!(!tests.is_empty());
        assert_eq!(tests[0].test_type, TestType::Property);
    }

    #[test]
    fn test_generate_property_tests_performance() {
        let generator = TestGenerator::with_defaults();
        let criteria = vec![AcceptanceCriterion {
            id: "AC-1".to_string(),
            text: "The system shall have performance that meets requirements".to_string(),
            category: "performance".to_string(),
            criticality: CriterionCriticality::ShouldHave,
            test_hints: vec!["performance".to_string()],
            format: None,
        }];

        let tests = generator.generate_property_tests(&criteria);
        assert!(!tests.is_empty());
    }

    #[test]
    fn test_generate_all_tests() {
        let generator = TestGenerator::with_defaults();
        let criteria = vec![
            AcceptanceCriterion {
                id: "AC-1".to_string(),
                text: "The system shall authenticate users".to_string(),
                category: "auth".to_string(),
                criticality: CriterionCriticality::MustHave,
                test_hints: vec![],
                format: None,
            },
            AcceptanceCriterion {
                id: "AC-2".to_string(),
                text: "The API shall accept requests".to_string(),
                category: "api".to_string(),
                criticality: CriterionCriticality::MustHave,
                test_hints: vec![],
                format: None,
            },
            AcceptanceCriterion {
                id: "AC-3".to_string(),
                text: "The system shall handle concurrent operations".to_string(),
                category: "concurrency".to_string(),
                criticality: CriterionCriticality::MustHave,
                test_hints: vec![],
                format: None,
            },
        ];

        let output = generator.generate_all_tests(&criteria, "src/main.rs");
        assert!(!output.unit_tests.is_empty() || !output.integration_tests.is_empty());
    }

    #[test]
    fn test_test_generator_output_calculate_totals() {
        let mut output = TestGeneratorOutput {
            unit_tests: vec![GeneratedTest {
                name: "test1".to_string(),
                module_path: "tests/test1.rs".to_string(),
                code: "#[test] fn test1() {}".to_string(),
                test_type: TestType::Unit,
                covers_criteria: vec![],
                confidence: 0.9,
                tags: vec![],
            }],
            integration_tests: vec![
                GeneratedTest {
                    name: "test2".to_string(),
                    module_path: "tests/test2.rs".to_string(),
                    code: "#[test] fn test2() {}".to_string(),
                    test_type: TestType::Integration,
                    covers_criteria: vec![],
                    confidence: 0.85,
                    tags: vec![],
                },
                GeneratedTest {
                    name: "test3".to_string(),
                    module_path: "tests/test3.rs".to_string(),
                    code: "#[test] fn test3() {}".to_string(),
                    test_type: TestType::Integration,
                    covers_criteria: vec![],
                    confidence: 0.8,
                    tags: vec![],
                },
            ],
            property_tests: vec![],
            total_generated: 0,
        };

        output.calculate_totals();
        assert_eq!(output.total_generated, 3);
    }

    #[test]
    fn test_test_generator_output_all_tests() {
        let output = TestGeneratorOutput {
            unit_tests: vec![GeneratedTest {
                name: "unit_test".to_string(),
                module_path: "tests/unit.rs".to_string(),
                code: "#[test] fn test() {}".to_string(),
                test_type: TestType::Unit,
                covers_criteria: vec![],
                confidence: 0.9,
                tags: vec![],
            }],
            integration_tests: vec![GeneratedTest {
                name: "integration_test".to_string(),
                module_path: "tests/integration.rs".to_string(),
                code: "#[test] fn test() {}".to_string(),
                test_type: TestType::Integration,
                covers_criteria: vec![],
                confidence: 0.85,
                tags: vec![],
            }],
            property_tests: vec![GeneratedTest {
                name: "property_test".to_string(),
                module_path: "tests/property.rs".to_string(),
                code: "#[test] fn test() {}".to_string(),
                test_type: TestType::Property,
                covers_criteria: vec![],
                confidence: 0.8,
                tags: vec![],
            }],
            total_generated: 3,
        };

        let all = output.all_tests();
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn test_test_generator_config_default() {
        let config = TestGeneratorConfig::default();
        assert!(config.include_doctests);
        assert!(!config.include_benchmarks);
        assert_eq!(config.min_confidence, 0.5);
        assert_eq!(config.max_tests_per_criterion, 5);
        assert_eq!(config.template_style, "rustdoc");
    }

    #[test]
    fn test_test_type_variants() {
        assert_eq!(TestType::Unit, TestType::Unit);
        assert_eq!(TestType::Integration, TestType::Integration);
        assert_eq!(TestType::Property, TestType::Property);
        assert_ne!(TestType::Unit, TestType::Integration);
    }

    #[test]
    fn test_generated_test_structure() {
        let test = GeneratedTest {
            name: "test_example".to_string(),
            module_path: "tests/example.rs".to_string(),
            code: "#[test] fn test_example() {}".to_string(),
            test_type: TestType::Unit,
            covers_criteria: vec!["AC-1".to_string()],
            confidence: 0.85,
            tags: vec!["example".to_string()],
        };

        assert_eq!(test.name, "test_example");
        assert_eq!(test.test_type, TestType::Unit);
        assert_eq!(test.covers_criteria.len(), 1);
        assert!(test.confidence > 0.0);
    }

    #[test]
    fn test_unit_tests_only_must_have_should_have() {
        let generator = TestGenerator::with_defaults();
        let criteria = vec![
            AcceptanceCriterion {
                id: "AC-1".to_string(),
                text: "Must have requirement".to_string(),
                category: "test".to_string(),
                criticality: CriterionCriticality::MustHave,
                test_hints: vec!["auth".to_string()],
                format: None,
            },
            AcceptanceCriterion {
                id: "AC-2".to_string(),
                text: "Should have requirement".to_string(),
                category: "test".to_string(),
                criticality: CriterionCriticality::ShouldHave,
                test_hints: vec!["auth".to_string()],
                format: None,
            },
            AcceptanceCriterion {
                id: "AC-3".to_string(),
                text: "Nice to have requirement".to_string(),
                category: "test".to_string(),
                criticality: CriterionCriticality::NiceToHave,
                test_hints: vec!["auth".to_string()],
                format: None,
            },
        ];

        let tests = generator.generate_unit_tests(&criteria, "src/test.rs");
        // NiceToHave should not generate tests by default
        assert!(tests
            .iter()
            .all(|t| t.covers_criteria.iter().any(|c| c != "AC-3")));
    }

    #[test]
    fn test_integration_tests_contain_api_patterns() {
        let generator = TestGenerator::with_defaults();
        let criteria = vec![AcceptanceCriterion {
            id: "AC-1".to_string(),
            text: "The API shall accept POST requests".to_string(),
            category: "api".to_string(),
            criticality: CriterionCriticality::MustHave,
            test_hints: vec![],
            format: None,
        }];

        let tests = generator.generate_integration_tests(&criteria);
        assert!(!tests.is_empty());
        assert!(tests[0].code.contains("async"));
        assert!(tests[0].code.contains("tokio::test"));
    }

    #[test]
    fn test_property_tests_contain_property_checks() {
        let generator = TestGenerator::with_defaults();
        let criteria = vec![AcceptanceCriterion {
            id: "AC-1".to_string(),
            text: "System shall handle concurrent operations".to_string(),
            category: "concurrency".to_string(),
            criticality: CriterionCriticality::MustHave,
            test_hints: vec![],
            format: None,
        }];

        let tests = generator.generate_property_tests(&criteria);
        assert!(!tests.is_empty());
        assert!(tests[0].code.contains("Arc") || tests[0].code.contains("thread"));
    }

    // =====================================================================
    // Proptest generation tests
    // =====================================================================

    #[test]
    fn test_generate_proptest_tests_basic() {
        let generator = TestGenerator::with_defaults();
        let criteria = vec![AcceptanceCriterion {
            id: "AC-1".to_string(),
            text: "For all valid inputs, output length shall not exceed input length + N".to_string(),
            category: "invariant".to_string(),
            criticality: CriterionCriticality::MustHave,
            test_hints: vec!["length".to_string(), "invariant".to_string()],
            format: None,
        }];

        let tests = generator.generate_proptest_tests(&criteria);
        assert!(!tests.is_empty());
        assert_eq!(tests[0].test_type, TestType::PropertyProptest);
    }

    #[test]
    fn test_generate_proptest_tests_contains_proptest_macro() {
        let generator = TestGenerator::with_defaults();
        let criteria = vec![AcceptanceCriterion {
            id: "AC-1".to_string(),
            text: "The system shall maintain invariant bounds for all inputs".to_string(),
            category: "bounds".to_string(),
            criticality: CriterionCriticality::MustHave,
            test_hints: vec!["invariant".to_string(), "bounds".to_string()],
            format: None,
        }];

        let tests = generator.generate_proptest_tests(&criteria);
        assert!(!tests.is_empty());
        // Verify proptest macro is present
        assert!(tests[0].code.contains("proptest!"));
        assert!(tests[0].code.contains("use proptest"));
    }

    #[test]
    fn test_generate_proptest_tests_length_invariant() {
        let generator = TestGenerator::with_defaults();
        let criteria = vec![AcceptanceCriterion {
            id: "AC-1".to_string(),
            text: "Output length must never exceed input length plus 100".to_string(),
            category: "length".to_string(),
            criticality: CriterionCriticality::MustHave,
            test_hints: vec!["length".to_string()],
            format: None,
        }];

        let tests = generator.generate_proptest_tests(&criteria);
        assert!(!tests.is_empty());
        // Should contain length-related invariant assertions
        assert!(tests[0].code.contains("length") || tests[0].code.contains("len()"));
    }

    #[test]
    fn test_generate_proptest_tests_reversibility_invariant() {
        let generator = TestGenerator::with_defaults();
        let criteria = vec![AcceptanceCriterion {
            id: "AC-1".to_string(),
            text: "reverse(forward(x)) must equal x for all inputs".to_string(),
            category: "reversibility".to_string(),
            criticality: CriterionCriticality::MustHave,
            test_hints: vec!["revers".to_string(), "inverse".to_string()],
            format: None,
        }];

        let tests = generator.generate_proptest_tests(&criteria);
        assert!(!tests.is_empty());
        // Should contain roundtrip/reversibility assertions
        assert!(tests[0].code.contains("roundtrip") || tests[0].code.contains("reverse"));
    }

    #[test]
    fn test_generate_proptest_tests_determinism_invariant() {
        let generator = TestGenerator::with_defaults();
        let criteria = vec![AcceptanceCriterion {
            id: "AC-1".to_string(),
            text: "The function must be deterministic: same input always yields same output".to_string(),
            category: "determinism".to_string(),
            criticality: CriterionCriticality::MustHave,
            test_hints: vec!["deterministic".to_string(), "consistent".to_string()],
            format: None,
        }];

        let tests = generator.generate_proptest_tests(&criteria);
        assert!(!tests.is_empty());
        // Should contain determinism assertions
        assert!(tests[0].code.contains("determinism") || tests[0].code.contains("consistent"));
    }

    #[test]
    fn test_generate_proptest_tests_disabled_when_use_proptest_false() {
        let mut config = TestGeneratorConfig::default();
        config.use_proptest = false;
        let generator = TestGenerator::new(config);

        let criteria = vec![AcceptanceCriterion {
            id: "AC-1".to_string(),
            text: "For all valid inputs, output length shall not exceed input length + N".to_string(),
            category: "invariant".to_string(),
            criticality: CriterionCriticality::MustHave,
            test_hints: vec!["length".to_string()],
            format: None,
        }];

        let tests = generator.generate_proptest_tests(&criteria);
        // Should return empty when proptest is disabled
        assert!(tests.is_empty());
    }

    #[test]
    fn test_generate_proptest_tests_config_iterations() {
        let mut config = TestGeneratorConfig::default();
        config.proptest_iterations = 512;
        let generator = TestGenerator::new(config);

        let criteria = vec![AcceptanceCriterion {
            id: "AC-1".to_string(),
            text: "Output must maintain size bounds".to_string(),
            category: "bounds".to_string(),
            criticality: CriterionCriticality::MustHave,
            test_hints: vec!["bounds".to_string()],
            format: None,
        }];

        let tests = generator.generate_proptest_tests(&criteria);
        assert!(!tests.is_empty());
        // Should use configured iteration count
        assert!(tests[0].code.contains("512"));
    }

    #[test]
    fn test_generate_proptest_tests_module_path() {
        let generator = TestGenerator::with_defaults();
        let criteria = vec![AcceptanceCriterion {
            id: "AC-1".to_string(),
            text: "The function must satisfy the length invariant".to_string(),
            category: "invariant".to_string(),
            criticality: CriterionCriticality::MustHave,
            test_hints: vec!["length".to_string()],
            format: None,
        }];

        let tests = generator.generate_proptest_tests(&criteria);
        assert!(!tests.is_empty());
        // Module path should indicate proptest tests
        assert!(tests[0].module_path.contains("proptest_"));
    }

    #[test]
    fn test_test_type_property_proptest_variant() {
        assert_eq!(TestType::PropertyProptest, TestType::PropertyProptest);
        assert_ne!(TestType::PropertyProptest, TestType::Property);
        assert_ne!(TestType::PropertyProptest, TestType::Unit);
        assert_ne!(TestType::PropertyProptest, TestType::Integration);
    }

    #[test]
    fn test_proptest_tests_have_correct_tags() {
        let generator = TestGenerator::with_defaults();
        let criteria = vec![AcceptanceCriterion {
            id: "AC-1".to_string(),
            text: "Output must satisfy the length invariant".to_string(),
            category: "validation".to_string(),
            criticality: CriterionCriticality::MustHave,
            test_hints: vec!["invariant".to_string()],
            format: None,
        }];

        let tests = generator.generate_proptest_tests(&criteria);
        assert!(!tests.is_empty());
        // Should have proptest and invariant tags
        assert!(tests[0].tags.contains(&"proptest".to_string()));
        assert!(tests[0].tags.contains(&"invariant".to_string()));
    }
}
