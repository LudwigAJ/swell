//! MCP Tool Naming Convention Tests
//!
//! These tests verify that MCP tools are registered with the `mcp__<server>__<tool>`
//! naming convention, with all components lowercased for consistent tool identification
//! across the system.
//!
//! Reference: VAL-MCP-004

#[cfg(test)]
mod mcp_tool_naming_tests {

    use swell_core::traits::Tool;
    use swell_tools::mcp::{McpClient, McpToolInfo, McpToolWrapper};

    /// Test that MCP tool wrapper returns normalized mcp__<server>__<tool> name
    #[tokio::test]
    async fn test_mcp_tool_wrapper_name_format() {
        let client = McpClient::new("echo test");

        // Create a tool info with server name "TreeSitter" and tool name "ParseAST"
        let info = McpToolInfo {
            name: "ParseAST".to_string(),
            description: "Parse an AST".to_string(),
            input_schema: None,
            output_schema: None,
            annotations: None,
            server_name: "TreeSitter".to_string(),
        };

        let wrapper = McpToolWrapper::new(info, client);

        // The tool name should be normalized to mcp__<server>__<tool> format
        // with all components lowercased
        assert_eq!(wrapper.name(), "mcp__treesitter__parseast");
    }

    /// Test that MCP tool wrapper name is lowercase for both server and tool
    #[tokio::test]
    async fn test_mcp_tool_wrapper_name_lowercase() {
        let client = McpClient::new("echo test");

        // Test with uppercase server and tool names
        let info = McpToolInfo {
            name: "DO_SOMETHING".to_string(),
            description: "Does something".to_string(),
            input_schema: None,
            output_schema: None,
            annotations: None,
            server_name: "MY_SERVER".to_string(),
        };

        let wrapper = McpToolWrapper::new(info, client);

        // Both server and tool should be lowercased
        assert_eq!(wrapper.name(), "mcp__my_server__do_something");
    }

    /// Test that MCP tool wrapper name preserves underscores in server and tool names
    #[tokio::test]
    async fn test_mcp_tool_wrapper_name_preserves_underscores() {
        let client = McpClient::new("echo test");

        // Server name with underscores (tree_sitter) and tool name with underscores (parse_ast)
        let info = McpToolInfo {
            name: "parse_ast".to_string(),
            description: "Parse an AST".to_string(),
            input_schema: None,
            output_schema: None,
            annotations: None,
            server_name: "tree_sitter".to_string(),
        };

        let wrapper = McpToolWrapper::new(info, client);

        // Underscores should be preserved
        assert_eq!(wrapper.name(), "mcp__tree_sitter__parse_ast");
    }

    /// Test that MCP tool wrapper name handles mixed case
    #[tokio::test]
    async fn test_mcp_tool_wrapper_name_mixed_case() {
        let client = McpClient::new("echo test");

        // Mixed case server and tool names
        let info = McpToolInfo {
            name: "ReadFile".to_string(),
            description: "Read a file".to_string(),
            input_schema: None,
            output_schema: None,
            annotations: None,
            server_name: "FileSystem".to_string(),
        };

        let wrapper = McpToolWrapper::new(info, client);

        // Should be all lowercase
        assert_eq!(wrapper.name(), "mcp__filesystem__readfile");
    }

    /// Test that MCP tool wrapper name handles single word server and tool
    #[tokio::test]
    async fn test_mcp_tool_wrapper_name_single_word() {
        let client = McpClient::new("echo test");

        let info = McpToolInfo {
            name: "greet".to_string(),
            description: "Say hello".to_string(),
            input_schema: None,
            output_schema: None,
            annotations: None,
            server_name: "hello".to_string(),
        };

        let wrapper = McpToolWrapper::new(info, client);

        assert_eq!(wrapper.name(), "mcp__hello__greet");
    }

    /// Test that MCP tool wrapper name handles empty-ish server name edge case
    #[tokio::test]
    async fn test_mcp_tool_wrapper_name_with_empty_server() {
        let client = McpClient::new("echo test");

        let info = McpToolInfo {
            name: "tool".to_string(),
            description: "A tool".to_string(),
            input_schema: None,
            output_schema: None,
            annotations: None,
            server_name: "".to_string(),
        };

        let wrapper = McpToolWrapper::new(info, client);

        // Empty server name should still produce valid format
        assert_eq!(wrapper.name(), "mcp____tool");
    }
}

#[cfg(test)]
mod mcp_tool_info_server_name_tests {

    use swell_tools::mcp::McpToolInfo;

    /// Test that McpToolInfo stores server_name correctly
    #[test]
    fn test_mcp_tool_info_server_name() {
        let info = McpToolInfo {
            name: "test".to_string(),
            description: "Test tool".to_string(),
            input_schema: None,
            output_schema: None,
            annotations: None,
            server_name: "test-server".to_string(),
        };

        assert_eq!(info.server_name, "test-server");
    }
}

#[cfg(test)]
mod mcp_registry_naming_tests {

    use swell_tools::mcp::{McpClient, McpToolInfo, McpToolWrapper};
    use swell_tools::registry::{ToolCategory, ToolLayer};
    use swell_tools::ToolRegistry;

    /// Test that MCP tools registered with registry have normalized names
    #[tokio::test]
    async fn test_mcp_tool_registry_has_normalized_name() {
        let registry = ToolRegistry::new();
        let client = McpClient::new("echo test");

        // Create and register a tool with server "TreeSitter" and tool "ParseAST"
        let info = McpToolInfo {
            name: "ParseAST".to_string(),
            description: "Parse an AST".to_string(),
            input_schema: None,
            output_schema: None,
            annotations: None,
            server_name: "TreeSitter".to_string(),
        };

        let wrapper = McpToolWrapper::new(info, client);

        // Register the tool
        registry
            .register(wrapper, ToolCategory::Mcp, ToolLayer::Plugin)
            .await;

        // The tool should be discoverable by its normalized name
        let retrieved = registry.get("mcp__treesitter__parseast").await;
        assert!(
            retrieved.is_some(),
            "Tool should be registered as 'mcp__treesitter__parseast' but was not found"
        );
    }

    /// Test that MCP tool is discoverable by various name formats
    #[tokio::test]
    async fn test_mcp_tool_discoverable_by_normalized_name() {
        let registry = ToolRegistry::new();
        let client = McpClient::new("echo test");

        let info = McpToolInfo {
            name: "ParseAST".to_string(),
            description: "Parse an AST".to_string(),
            input_schema: None,
            output_schema: None,
            annotations: None,
            server_name: "TreeSitter".to_string(),
        };

        let wrapper = McpToolWrapper::new(info, client);

        registry
            .register(wrapper, ToolCategory::Mcp, ToolLayer::Plugin)
            .await;

        // The tool should be findable by the exact normalized name
        assert!(
            registry.contains("mcp__treesitter__parseast").await,
            "Tool should be discoverable by normalized name 'mcp__treesitter__parseast'"
        );
    }

    /// Test that multiple MCP tools from different servers have distinct names
    #[tokio::test]
    async fn test_multiple_mcp_tools_from_different_servers() {
        let registry = ToolRegistry::new();
        let client1 = McpClient::new("echo test1");
        let client2 = McpClient::new("echo test2");

        // Tool from server "TreeSitter"
        let info1 = McpToolInfo {
            name: "parse".to_string(),
            description: "Parse".to_string(),
            input_schema: None,
            output_schema: None,
            annotations: None,
            server_name: "tree_sitter".to_string(),
        };

        // Tool from server "Eslint"
        let info2 = McpToolInfo {
            name: "lint".to_string(),
            description: "Lint".to_string(),
            input_schema: None,
            output_schema: None,
            annotations: None,
            server_name: "eslint".to_string(),
        };

        let wrapper1 = McpToolWrapper::new(info1, client1);
        let wrapper2 = McpToolWrapper::new(info2, client2);

        registry
            .register(wrapper1, ToolCategory::Mcp, ToolLayer::Plugin)
            .await;
        registry
            .register(wrapper2, ToolCategory::Mcp, ToolLayer::Plugin)
            .await;

        // Each tool should have a unique name based on server + tool
        assert!(
            registry.contains("mcp__tree_sitter__parse").await,
            "TreeSitter parse tool should be registered"
        );
        assert!(
            registry.contains("mcp__eslint__lint").await,
            "Eslint lint tool should be registered"
        );

        // They should be distinct
        assert_ne!(
            registry.get("mcp__tree_sitter__parse").await.unwrap().name(),
            registry.get("mcp__eslint__lint").await.unwrap().name()
        );
    }
}
