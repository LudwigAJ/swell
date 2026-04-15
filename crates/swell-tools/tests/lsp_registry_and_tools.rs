//! Tests for LSP Registry and Tools
//!
//! Reference: VAL-MCP-006, VAL-MCP-007, VAL-MCP-008

use swell_core::traits::Tool;
use swell_core::ToolResultContent;

/// Test that LspRegistry maps languages to servers correctly
/// Reference: VAL-MCP-006
#[tokio::test]
async fn test_lsp_registry_rust_mapping() {
    use swell_tools::mcp_lsp::{LspLanguage, LspRegistry};

    let mut registry = LspRegistry::new();

    // Add rust-analyzer configuration
    registry
        .add_server(
            LspLanguage::Rust,
            "rust-analyzer".to_string(),
            "npx mcp-language-server --lsp rust-analyzer".to_string(),
        )
        .await
        .unwrap();

    // Query for Rust language should return the correct server config
    let config = registry.get(LspLanguage::Rust);
    assert!(config.is_some());
    let config = config.unwrap();
    assert_eq!(config.server_name, "rust-analyzer");
}

#[tokio::test]
async fn test_lsp_registry_cpp_mapping() {
    use swell_tools::mcp_lsp::{LspLanguage, LspRegistry};

    let mut registry = LspRegistry::new();

    // Add clangd configuration
    registry
        .add_server(
            LspLanguage::Cpp,
            "clangd".to_string(),
            "npx mcp-language-server --lsp clangd".to_string(),
        )
        .await
        .unwrap();

    // Query for Cpp language should return the correct server config
    let config = registry.get(LspLanguage::Cpp);
    assert!(config.is_some());
    let config = config.unwrap();
    assert_eq!(config.server_name, "clangd");
}

#[tokio::test]
async fn test_lsp_registry_unknown_language_returns_none() {
    use swell_tools::mcp_lsp::{LspLanguage, LspRegistry};

    let registry = LspRegistry::new();

    // Query for Unknown language should return None
    let config = registry.get(LspLanguage::Unknown);
    assert!(config.is_none());
}

#[tokio::test]
async fn test_lsp_registry_multiple_languages() {
    use swell_tools::mcp_lsp::{LspLanguage, LspRegistry};

    let mut registry = LspRegistry::new();

    // Add both Rust and Cpp configurations
    registry
        .add_server(
            LspLanguage::Rust,
            "rust-analyzer".to_string(),
            "npx mcp-language-server --lsp rust-analyzer".to_string(),
        )
        .await
        .unwrap();

    registry
        .add_server(
            LspLanguage::Cpp,
            "clangd".to_string(),
            "npx mcp-language-server --lsp clangd".to_string(),
        )
        .await
        .unwrap();

    // Both should be retrievable
    assert!(registry.get(LspLanguage::Rust).is_some());
    assert!(registry.get(LspLanguage::Cpp).is_some());
    assert!(registry.get(LspLanguage::Unknown).is_none());
}

/// Test LSP tool function signatures and behavior
/// Reference: VAL-MCP-007
#[tokio::test]
async fn test_lsp_definition_tool_info() {
    use swell_tools::mcp::McpClient;
    use swell_tools::mcp_lsp::LspDefinitionTool;

    let client = McpClient::new("echo test");
    let tool = LspDefinitionTool::new(client);

    assert_eq!(tool.name(), "lsp_definition");
    assert!(tool.description().contains("definitions"));
    assert_eq!(tool.risk_level(), swell_core::ToolRiskLevel::Read);
    assert_eq!(tool.permission_tier(), swell_core::PermissionTier::Auto);

    // Check input schema has symbol_name
    let schema = tool.input_schema();
    assert_eq!(schema["type"], "object");
    assert!(schema["properties"]
        .as_object()
        .unwrap()
        .contains_key("symbol_name"));
}

#[tokio::test]
async fn test_lsp_references_tool_info() {
    use swell_tools::mcp::McpClient;
    use swell_tools::mcp_lsp::LspReferencesTool;

    let client = McpClient::new("echo test");
    let tool = LspReferencesTool::new(client);

    assert_eq!(tool.name(), "lsp_references");
    assert!(tool.description().contains("references"));
    assert_eq!(tool.risk_level(), swell_core::ToolRiskLevel::Read);
    assert_eq!(tool.permission_tier(), swell_core::PermissionTier::Auto);

    let schema = tool.input_schema();
    assert!(schema["properties"]
        .as_object()
        .unwrap()
        .contains_key("symbol_name"));
}

#[tokio::test]
async fn test_lsp_hover_tool_info() {
    use swell_tools::mcp::McpClient;
    use swell_tools::mcp_lsp::LspHoverTool;

    let client = McpClient::new("echo test");
    let tool = LspHoverTool::new(client);

    assert_eq!(tool.name(), "lsp_hover");
    assert!(tool.description().contains("hover"));
    assert_eq!(tool.risk_level(), swell_core::ToolRiskLevel::Read);
    assert_eq!(tool.permission_tier(), swell_core::PermissionTier::Auto);

    let schema = tool.input_schema();
    let props = schema["properties"].as_object().unwrap();
    assert!(props.contains_key("file_path"));
    assert!(props.contains_key("line"));
    assert!(props.contains_key("column"));
}

#[tokio::test]
async fn test_lsp_diagnostics_tool_info() {
    use swell_tools::mcp::McpClient;
    use swell_tools::mcp_lsp::LspDiagnosticsTool;

    let client = McpClient::new("echo test");
    let tool = LspDiagnosticsTool::new(client);

    assert_eq!(tool.name(), "lsp_diagnostics");
    assert!(tool.description().contains("diagnostics"));
    assert_eq!(tool.risk_level(), swell_core::ToolRiskLevel::Read);
    assert_eq!(tool.permission_tier(), swell_core::PermissionTier::Auto);

    let schema = tool.input_schema();
    assert!(schema["properties"]
        .as_object()
        .unwrap()
        .contains_key("file_path"));
}

/// Test graceful degradation when LSP server is disconnected
/// Reference: VAL-MCP-008
#[tokio::test]
async fn test_lsp_definition_disconnected_returns_error() {
    use swell_tools::mcp::McpClient;
    use swell_tools::mcp_lsp::LspDefinitionTool;

    // Create a disconnected client (using a fake command that won't connect)
    let client = McpClient::new("nonexistent-command-that-will-fail");

    // The client is not connected initially
    assert!(!client.is_connected().await);

    let tool = LspDefinitionTool::new(client);

    // Execute should return an error, not panic
    let args = serde_json::json!({
        "symbol_name": "test_function"
    });

    let result = tool.execute(args).await;

    // Should return Ok with is_error: true, not Err
    assert!(result.is_ok());
    let output = result.unwrap();
    assert!(
        output.is_error,
        "Disconnected LSP should return is_error: true"
    );

    // Error message should mention disconnected/unavailable
    let content_str = match output.content.first() {
        Some(ToolResultContent::Error(e)) => e.clone(),
        Some(ToolResultContent::Text(s)) => s.clone(),
        Some(ToolResultContent::Json(v)) => serde_json::to_string(v).unwrap_or_default(),
        Some(ToolResultContent::Image { data, .. }) => data.clone(),
        None => String::new(),
    };
    assert!(
        content_str.to_lowercase().contains("disconnect")
            || content_str.to_lowercase().contains("unavailable")
            || content_str.to_lowercase().contains("not connected"),
        "Error message should indicate disconnection: {}",
        content_str
    );
}

#[tokio::test]
async fn test_lsp_references_disconnected_returns_error() {
    use swell_tools::mcp::McpClient;
    use swell_tools::mcp_lsp::LspReferencesTool;

    let client = McpClient::new("nonexistent-command-that-will-fail");
    let tool = LspReferencesTool::new(client);

    let args = serde_json::json!({
        "symbol_name": "test_function"
    });

    let result = tool.execute(args).await;

    assert!(result.is_ok());
    let output = result.unwrap();
    assert!(
        output.is_error,
        "Disconnected LSP should return is_error: true"
    );
}

#[tokio::test]
async fn test_lsp_hover_disconnected_returns_error() {
    use swell_tools::mcp::McpClient;
    use swell_tools::mcp_lsp::LspHoverTool;

    let client = McpClient::new("nonexistent-command-that-will-fail");
    let tool = LspHoverTool::new(client);

    let args = serde_json::json!({
        "file_path": "/test.rs",
        "line": 10,
        "column": 5
    });

    let result = tool.execute(args).await;

    assert!(result.is_ok());
    let output = result.unwrap();
    assert!(
        output.is_error,
        "Disconnected LSP should return is_error: true"
    );
}

#[tokio::test]
async fn test_lsp_diagnostics_disconnected_returns_error() {
    use swell_tools::mcp::McpClient;
    use swell_tools::mcp_lsp::LspDiagnosticsTool;

    let client = McpClient::new("nonexistent-command-that-will-fail");
    let tool = LspDiagnosticsTool::new(client);

    let args = serde_json::json!({
        "file_path": "/test.rs"
    });

    let result = tool.execute(args).await;

    assert!(result.is_ok());
    let output = result.unwrap();
    assert!(
        output.is_error,
        "Disconnected LSP should return is_error: true"
    );
}

/// Test that all four required LSP tools exist and work
/// Reference: VAL-MCP-007
#[tokio::test]
async fn test_all_four_lsp_tools_required() {
    use swell_tools::mcp::McpClient;
    use swell_tools::mcp_lsp::{
        LspDefinitionTool, LspDiagnosticsTool, LspHoverTool, LspReferencesTool,
    };

    let client = McpClient::new("echo test");

    // All four tools must exist
    let definition = LspDefinitionTool::new(client.clone());
    let references = LspReferencesTool::new(client.clone());
    let hover = LspHoverTool::new(client.clone());
    let diagnostics = LspDiagnosticsTool::new(client.clone());

    assert_eq!(definition.name(), "lsp_definition");
    assert_eq!(references.name(), "lsp_references");
    assert_eq!(hover.name(), "lsp_hover");
    assert_eq!(diagnostics.name(), "lsp_diagnostics");
}

/// Test tool naming conventions
#[test]
fn test_all_lsp_tools_prefixed() {
    use swell_tools::mcp::McpClient;
    use swell_tools::mcp_lsp::{
        LspDefinitionTool, LspDiagnosticsTool, LspHoverTool, LspReferencesTool,
    };

    let client = McpClient::new("echo");
    let definition = LspDefinitionTool::new(client.clone());
    let references = LspReferencesTool::new(client.clone());
    let hover = LspHoverTool::new(client.clone());
    let diagnostics = LspDiagnosticsTool::new(client.clone());

    assert_eq!("lsp_definition", definition.name());
    assert_eq!("lsp_references", references.name());
    assert_eq!("lsp_hover", hover.name());
    assert_eq!("lsp_diagnostics", diagnostics.name());
}

/// Test that tools require specific arguments
#[tokio::test]
async fn test_lsp_definition_requires_symbol_name() {
    use swell_tools::mcp::McpClient;
    use swell_tools::mcp_lsp::LspDefinitionTool;

    let client = McpClient::new("echo test");
    let tool = LspDefinitionTool::new(client);

    // Missing symbol_name should fail
    let args = serde_json::json!({});

    let result = tool.execute(args).await;
    // Should return an error about missing symbol_name
    assert!(result.is_err() || result.as_ref().is_ok_and(|o| o.is_error));
}
