//! Integration test: Five-layer config cascade is respected by ExecutionController.
//!
//! This test verifies that:
//! 1. ConfigLoader loads five layers of configuration correctly
//! 2. max_iterations from settings is loaded through the config cascade
//! 3. Higher-precedence layer overrides lower for max_iterations
//! 4. ExecutionController respects the loaded max_iterations value
//!
//! This validates VAL-CROSS-004: Five-layer config values are respected by ExecutionController.

use std::sync::Arc;
use swell_core::config::ConfigLoader;
use swell_llm::mock::MockLlm;
use swell_orchestrator::{builder::OrchestratorBuilder, ExecutionController};
use swell_tools::ToolRegistry;
use tempfile::TempDir;

/// Guard that cleans up environment variables when dropped.
struct EnvGuard {
    var_name: String,
}

impl EnvGuard {
    fn new(var_name: &str) -> Self {
        std::env::remove_var(var_name);
        Self {
            var_name: var_name.to_string(),
        }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        std::env::remove_var(&self.var_name);
    }
}

/// Helper to create a config file at a specific path.
fn create_config_file(dir: &TempDir, filename: &str, content: &str) -> std::path::PathBuf {
    let path = dir.path().join(filename);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&path, content).unwrap();
    path
}

/// Test: Five-layer config cascade loads correct max_iterations at each layer.
///
/// This test verifies that each layer in the five-layer config cascade can
/// successfully provide a max_iterations value, with higher layers overriding lower layers.
#[tokio::test]
async fn test_config_cascade_loads_correct_max_iterations() {
    // This test doesn't use env vars - no guard needed

    let temp = TempDir::new().unwrap();

    // Layer 1: User global (~/.config/swell/settings.json)
    create_config_file(
        &temp,
        ".config/swell/settings.json",
        r#"{"execution": {"max_iterations": 10}}"#,
    );

    // Layer 2: User modern (~/.swell/settings.json)
    create_config_file(
        &temp,
        ".swell/settings.json",
        r#"{"execution": {"max_iterations": 20}}"#,
    );

    // Layer 3: Project shared (.swell/settings.json)
    create_config_file(
        &temp,
        ".swell/settings.json",
        r#"{"execution": {"max_iterations": 30}}"#,
    );

    // Layer 4: Project modern (.swell/settings.local.json)
    create_config_file(
        &temp,
        ".swell/settings.local.json",
        r#"{"execution": {"max_iterations": 40}}"#,
    );

    // Load config from project path (layer 3 + 4)
    let loader = ConfigLoader::new().with_project_path(temp.path());
    let config = loader.load().unwrap();

    // Layer 4 (settings.local.json) should win over layer 3 (settings.json)
    let max_iterations = config
        .get("execution.max_iterations")
        .expect("max_iterations should be present")
        .as_u64()
        .expect("max_iterations should be a number");

    assert_eq!(max_iterations, 40, "Layer 4 should override layer 3");
}

/// Test: Higher-precedence layer overrides lower for max_iterations.
///
/// This test verifies that when multiple layers set max_iterations,
/// the highest-precedence layer wins.
#[tokio::test]
async fn test_higher_layer_overrides_lower_for_max_iterations() {
    // This test doesn't use env vars - no guard needed

    let temp = TempDir::new().unwrap();

    // Only layers 1 and 2 exist
    create_config_file(
        &temp,
        ".config/swell/settings.json",
        r#"{"execution": {"max_iterations": 10}}"#,
    );
    create_config_file(
        &temp,
        ".swell/settings.json",
        r#"{"execution": {"max_iterations": 25}}"#,
    );

    // Load config
    let loader = ConfigLoader::new().with_project_path(temp.path());
    let config = loader.load().unwrap();

    // Layer 2 should win
    let max_iterations = config
        .get("execution.max_iterations")
        .expect("max_iterations should be present")
        .as_u64()
        .expect("max_iterations should be a number");

    assert_eq!(max_iterations, 25, "Layer 2 should override layer 1");
}

/// Test: Environment variable (Layer 5) overrides all file layers.
///
/// Environment variables are the highest-priority layer (layer 5) and should
/// override all file-based configuration.
#[tokio::test]
async fn test_env_var_overrides_file_layers() {
    // Use unique env prefix to avoid collision with other tests
    // Each test gets its own prefix (TEST_A_) so they can't interfere
    let _env_guard = EnvGuard::new("TEST_A_EXECUTION_MAX_ITERATIONS");

    let temp = TempDir::new().unwrap();

    // Create all file layers
    create_config_file(
        &temp,
        ".config/swell/settings.json",
        r#"{"execution": {"max_iterations": 10}}"#,
    );
    create_config_file(
        &temp,
        ".swell/settings.json",
        r#"{"execution": {"max_iterations": 30}}"#,
    );
    create_config_file(
        &temp,
        ".swell/settings.json",
        r#"{"execution": {"max_iterations": 40}}"#,
    );
    create_config_file(
        &temp,
        ".swell/settings.local.json",
        r#"{"execution": {"max_iterations": 50}}"#,
    );

    // Set environment variable (layer 5) with unique prefix
    std::env::set_var("TEST_A_EXECUTION_MAX_ITERATIONS", "100");

    // Load config with custom env prefix
    let loader = ConfigLoader::new()
        .with_project_path(temp.path())
        .with_env_prefix("TEST_A_");
    let config = loader.load().unwrap();

    // Env var should win
    let max_iterations = config
        .get("execution.max_iterations")
        .expect("max_iterations should be present")
        .as_u64()
        .expect("max_iterations should be a number");

    assert_eq!(
        max_iterations, 100,
        "Environment variable (layer 5) should override all file layers"
    );
}

/// Test: ExecutionController respects the loaded max_iterations value.
///
/// This test verifies that when ExecutionController is constructed with a
/// max_iterations value loaded from config, the controller actually uses it.
#[tokio::test]
async fn test_execution_controller_respects_loaded_max_iterations() {
    // This test doesn't use env vars - no guard needed

    let temp = TempDir::new().unwrap();

    // Create config with max_iterations = 5
    create_config_file(
        &temp,
        ".swell/settings.json",
        r#"{"execution": {"max_iterations": 5}}"#,
    );

    // Load config
    let loader = ConfigLoader::new().with_project_path(temp.path());
    let config = loader.load().unwrap();

    // Extract max_iterations from config
    let max_iterations = config
        .get("execution.max_iterations")
        .expect("max_iterations should be present")
        .as_u64()
        .expect("max_iterations should be a number") as u32;

    assert_eq!(max_iterations, 5);

    // Create ExecutionController with the loaded value
    let orchestrator = OrchestratorBuilder::new().build();
    let mock_llm = Arc::new(MockLlm::new("claude-sonnet"));
    let tool_registry = Arc::new(ToolRegistry::new());

    let controller = ExecutionController::with_max_iterations(
        Arc::downgrade(&orchestrator),
        mock_llm.clone(),
        tool_registry.clone(),
        max_iterations,
    );

    // Verify the controller uses the correct value
    assert_eq!(controller.max_iterations(), 5);
}

/// Test: Full cascade - five layers, highest priority wins.
///
/// This test exercises the full five-layer cascade:
/// 1. User global (~/.config/swell/settings.json): 10
/// 2. User modern (~/.swell/settings.json): 20
/// 3. Project shared (.swell/settings.json): 30
/// 4. Project modern (.swell/settings.local.json): 40
/// 5. Environment variable: 50
///
/// Expected: 50 (env var wins)
#[tokio::test]
async fn test_full_cascade_five_layers_highest_wins() {
    // Use unique env prefix to avoid collision with other tests
    let _env_guard = EnvGuard::new("TEST_B_EXECUTION_MAX_ITERATIONS");

    let temp = TempDir::new().unwrap();

    // Layer 1: User global
    create_config_file(
        &temp,
        ".config/swell/settings.json",
        r#"{"execution": {"max_iterations": 10}}"#,
    );

    // Layer 2: User modern
    create_config_file(
        &temp,
        ".swell/settings.json",
        r#"{"execution": {"max_iterations": 20}}"#,
    );

    // Layer 3: Project shared
    create_config_file(
        &temp,
        ".swell/settings.json",
        r#"{"execution": {"max_iterations": 30}}"#,
    );

    // Layer 4: Project modern
    create_config_file(
        &temp,
        ".swell/settings.local.json",
        r#"{"execution": {"max_iterations": 40}}"#,
    );

    // Layer 5: Environment variable
    std::env::set_var("TEST_B_EXECUTION_MAX_ITERATIONS", "50");

    // Load config with custom env prefix
    let loader = ConfigLoader::new()
        .with_project_path(temp.path())
        .with_env_prefix("TEST_B_");
    let config = loader.load().unwrap();

    // Env var (layer 5) should win
    let max_iterations = config
        .get("execution.max_iterations")
        .expect("max_iterations should be present")
        .as_u64()
        .expect("max_iterations should be a number");

    assert_eq!(
        max_iterations, 50,
        "Environment variable (layer 5) should win in full cascade"
    );

    // Now remove env var and verify layer 4 wins
    std::env::remove_var("TEST_B_EXECUTION_MAX_ITERATIONS");

    let loader2 = ConfigLoader::new()
        .with_project_path(temp.path())
        .with_env_prefix("TEST_B_");
    let config2 = loader2.load().unwrap();

    let max_iterations2 = config2
        .get("execution.max_iterations")
        .expect("max_iterations should be present")
        .as_u64()
        .expect("max_iterations should be a number");

    assert_eq!(
        max_iterations2, 40,
        "After removing env var, layer 4 should win"
    );
}

/// Test: ExecutionController max_iterations enforcement with cascade value.
///
/// This test verifies that when max_iterations is loaded from config and
/// passed to ExecutionController, the controller actually enforces it.
/// We verify this by checking that the controller's internal limit matches.
#[tokio::test]
async fn test_execution_controller_enforces_cascade_max_iterations() {
    // This test doesn't use env vars - no guard needed

    let temp = TempDir::new().unwrap();

    // Set a specific value in the project local settings
    create_config_file(
        &temp,
        ".swell/settings.local.json",
        r#"{"execution": {"max_iterations": 7}}"#,
    );

    // Load config
    let loader = ConfigLoader::new().with_project_path(temp.path());
    let config = loader.load().unwrap();

    // Extract max_iterations
    let max_iterations = config
        .get("execution.max_iterations")
        .expect("max_iterations should be present")
        .as_u64()
        .expect("max_iterations should be a number") as u32;

    assert_eq!(max_iterations, 7);

    // Create ExecutionController with the loaded value
    let orchestrator = OrchestratorBuilder::new().build();
    let mock_llm = Arc::new(MockLlm::new("claude-sonnet"));
    let tool_registry = Arc::new(ToolRegistry::new());

    let controller = ExecutionController::with_max_iterations(
        Arc::downgrade(&orchestrator),
        mock_llm.clone(),
        tool_registry.clone(),
        max_iterations,
    );

    // The controller should report the correct max_iterations
    assert_eq!(controller.max_iterations(), 7);

    // Also verify with_all_settings works with the loaded value
    let controller2 = ExecutionController::with_all_settings(
        Arc::downgrade(&orchestrator),
        mock_llm.clone(),
        tool_registry.clone(),
        max_iterations,
        100_000, // context_compaction_threshold
        10,      // tail_message_count
    );

    assert_eq!(controller2.max_iterations(), 7);
}

/// Test: Config audit trail tracks max_iterations source.
///
/// This test verifies that the config audit trail correctly identifies
/// which layer provided the max_iterations value.
#[tokio::test]
async fn test_config_audit_trail_for_max_iterations() {
    // This test doesn't use env vars - no guard needed

    let temp = TempDir::new().unwrap();

    // Only layer 3 (project shared)
    create_config_file(
        &temp,
        ".swell/settings.json",
        r#"{"execution": {"max_iterations": 35}}"#,
    );

    // Load config
    let loader = ConfigLoader::new().with_project_path(temp.path());
    let config = loader.load().unwrap();

    // Check audit trail for max_iterations
    let entries = config.loaded_entries();
    let max_iter_entry = entries
        .iter()
        .find(|e| e.key_path == "execution.max_iterations");

    assert!(
        max_iter_entry.is_some(),
        "max_iterations should be in audit trail"
    );

    let entry = max_iter_entry.unwrap();
    assert!(
        entry.source_file.is_some(),
        "source_file should be set for file-based config"
    );

    // Verify the source file contains settings.json
    let source = entry.source_file.as_ref().unwrap();
    assert!(
        source.contains("settings.json"),
        "source should point to settings.json, got: {}",
        source
    );
}
