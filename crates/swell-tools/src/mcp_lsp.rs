//! LSP (Language Server Protocol) integration via mcp-language-server.
//!
//! This module provides tools that bridge MCP to LSP servers (rust-analyzer, clangd)
//! through the mcp-language-server MCP server.
//!
//! Architecture:
//! ```text
//! swell-tools (LSP tools) → MCP Client → mcp-language-server → LSP Server
//!                                               ↓
//!                          rust-analyzer ←─────┼──────→ clangd
//! ```
//!
//! Reference: https://github.com/isaacphi/mcp-language-server

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use swell_core::traits::Tool;
use swell_core::{PermissionTier, SwellError, ToolOutput, ToolRiskLevel};

use super::mcp::McpClient;

/// LSP tool category for classification
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LspLanguage {
    Rust,
    Cpp,
    Unknown,
}

impl LspLanguage {
    /// Parse language from string
    pub fn parse(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "rust" | "rs" => LspLanguage::Rust,
            "c" | "cpp" | "c++" | "clangd" => LspLanguage::Cpp,
            _ => LspLanguage::Unknown,
        }
    }
}

/// Location in a source file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspLocation {
    pub uri: String,
    pub range: LspRange,
}

/// Range in a source file (line/column positions)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspRange {
    pub start: LspPosition,
    pub end: LspPosition,
}

/// Position in a source file (0-indexed line and column)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspPosition {
    pub line: u32,
    pub column: u32,
}

/// Symbol information from LSP
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspSymbol {
    pub name: String,
    pub kind: String,
    pub location: LspLocation,
    pub detail: Option<String>,
}

/// Diagnostic severity levels
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticSeverity {
    Error = 1,
    Warning = 2,
    Information = 3,
    Hint = 4,
}

/// A diagnostic (warning, error, etc.) from the language server
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspDiagnostic {
    pub severity: i32,
    pub message: String,
    pub source: String,
    pub range: LspRange,
    pub code: Option<String>,
}

/// Result of a rename operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspRenameResult {
    pub changes: std::collections::HashMap<String, Vec<LspTextEdit>>,
    pub success: bool,
}

/// A text edit to apply to a document
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspTextEdit {
    pub range: LspRange,
    pub new_text: String,
}

/// Result of workspace diagnostics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspWorkspaceDiagnostics {
    pub files: Vec<LspFileDiagnostics>,
    pub total_errors: i32,
    pub total_warnings: i32,
}

/// Diagnostics for a single file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspFileDiagnostics {
    pub uri: String,
    pub diagnostics: Vec<LspDiagnostic>,
}

/// Hover information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspHover {
    pub contents: String,
    pub range: Option<LspRange>,
}

// ============================================================================
// Tool Implementations
// ============================================================================

/// Tool for finding definitions using LSP (rust-analyzer, clangd)
#[derive(Debug, Clone)]
pub struct LspDefinitionTool {
    mcp_client: McpClient,
}

impl LspDefinitionTool {
    pub fn new(mcp_client: McpClient) -> Self {
        Self { mcp_client }
    }
}

#[async_trait]
impl Tool for LspDefinitionTool {
    fn name(&self) -> &str {
        "lsp_definition"
    }

    fn description(&self) -> String {
        "Find definitions of a symbol using LSP (rust-analyzer for Rust, clangd for C/C++). \
         Returns the location where the symbol is defined. \
         Arguments: symbol_name (e.g., 'my_function', 'MyType.my_method')"
            .to_string()
    }

    fn risk_level(&self) -> ToolRiskLevel {
        ToolRiskLevel::Read
    }

    fn permission_tier(&self) -> PermissionTier {
        PermissionTier::Auto
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "symbol_name": {
                    "type": "string",
                    "description": "The name of the symbol to find (e.g., 'mypackage.MyFunction', 'MyType')"
                }
            },
            "required": ["symbol_name"]
        })
    }

    async fn execute(&self, arguments: Value) -> Result<ToolOutput, SwellError> {
        let symbol_name = arguments
            .get("symbol_name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                SwellError::ToolExecutionFailed("Missing symbol_name".to_string())
            })?;

        let params = serde_json::json!({
            "symbolName": symbol_name
        });

        // Call the MCP tool for LSP definitions
        let result = self.mcp_client.call_tool("definition", params).await?;

        // Parse the result into LspLocation format
        let locations: Vec<LspLocation> =
            serde_json::from_str(&result.result).unwrap_or_else(|_| {
                serde_json::from_str::<Vec<Value>>(&result.result)
                    .map(|vals| {
                        vals.into_iter()
                            .filter_map(|v| serde_json::from_value(v).ok())
                            .collect()
                    })
                    .unwrap_or_default()
            });

        Ok(ToolOutput {
            success: result.success,
            result: serde_json::to_string(&locations).unwrap_or_default(),
            error: result.error,
        })
    }
}

/// Tool for finding references using LSP
#[derive(Debug, Clone)]
pub struct LspReferencesTool {
    mcp_client: McpClient,
}

impl LspReferencesTool {
    pub fn new(mcp_client: McpClient) -> Self {
        Self { mcp_client }
    }
}

#[async_trait]
impl Tool for LspReferencesTool {
    fn name(&self) -> &str {
        "lsp_references"
    }

    fn description(&self) -> String {
        "Find all references to a symbol using LSP. Returns all locations where the symbol is used or defined. \
         Arguments: symbol_name (e.g., 'my_function', 'MyType')".to_string()
    }

    fn risk_level(&self) -> ToolRiskLevel {
        ToolRiskLevel::Read
    }

    fn permission_tier(&self) -> PermissionTier {
        PermissionTier::Auto
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "symbol_name": {
                    "type": "string",
                    "description": "The name of the symbol to search for (e.g., 'mypackage.MyFunction', 'MyType')"
                }
            },
            "required": ["symbol_name"]
        })
    }

    async fn execute(&self, arguments: Value) -> Result<ToolOutput, SwellError> {
        let symbol_name = arguments
            .get("symbol_name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                SwellError::ToolExecutionFailed("Missing symbol_name".to_string())
            })?;

        let params = serde_json::json!({
            "symbolName": symbol_name
        });

        let result = self.mcp_client.call_tool("references", params).await?;

        let locations: Vec<LspLocation> =
            serde_json::from_str(&result.result).unwrap_or_else(|_| {
                serde_json::from_str::<Vec<Value>>(&result.result)
                    .map(|vals| {
                        vals.into_iter()
                            .filter_map(|v| serde_json::from_value(v).ok())
                            .collect()
                    })
                    .unwrap_or_default()
            });

        Ok(ToolOutput {
            success: result.success,
            result: serde_json::to_string(&locations).unwrap_or_default(),
            error: result.error,
        })
    }
}

/// Tool for getting hover information using LSP
#[derive(Debug, Clone)]
pub struct LspHoverTool {
    mcp_client: McpClient,
}

impl LspHoverTool {
    pub fn new(mcp_client: McpClient) -> Self {
        Self { mcp_client }
    }
}

#[async_trait]
impl Tool for LspHoverTool {
    fn name(&self) -> &str {
        "lsp_hover"
    }

    fn description(&self) -> String {
        "Get hover information at a position using LSP. Returns type signatures, documentation, and doc comments. \
         Arguments: file_path (file path), line (1-indexed), column (1-indexed)".to_string()
    }

    fn risk_level(&self) -> ToolRiskLevel {
        ToolRiskLevel::Read
    }

    fn permission_tier(&self) -> PermissionTier {
        PermissionTier::Auto
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Path to the file (e.g., /path/to/file.rs)"
                },
                "line": {
                    "type": "integer",
                    "description": "Line number (1-indexed)"
                },
                "column": {
                    "type": "integer",
                    "description": "Column number (1-indexed)"
                }
            },
            "required": ["file_path", "line", "column"]
        })
    }

    async fn execute(&self, arguments: Value) -> Result<ToolOutput, SwellError> {
        let file_path = arguments
            .get("file_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                SwellError::ToolExecutionFailed("Missing file_path".to_string())
            })?;

        let line = arguments
            .get("line")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| SwellError::ToolExecutionFailed("Missing line".to_string()))? as u32;

        let column = arguments
            .get("column")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| SwellError::ToolExecutionFailed("Missing column".to_string()))? as u32;

        let params = serde_json::json!({
            "filePath": file_path,
            "line": line,
            "column": column
        });

        let result = self.mcp_client.call_tool("hover", params).await?;

        let hover: LspHover = serde_json::from_str(&result.result).unwrap_or_else(|_| LspHover {
            contents: result.result.clone(),
            range: None,
        });

        Ok(ToolOutput {
            success: result.success,
            result: serde_json::to_string(&hover).unwrap_or_default(),
            error: result.error,
        })
    }
}

/// Tool for getting diagnostics using LSP
#[derive(Debug, Clone)]
pub struct LspDiagnosticsTool {
    mcp_client: McpClient,
}

impl LspDiagnosticsTool {
    pub fn new(mcp_client: McpClient) -> Self {
        Self { mcp_client }
    }
}

#[async_trait]
impl Tool for LspDiagnosticsTool {
    fn name(&self) -> &str {
        "lsp_diagnostics"
    }

    fn description(&self) -> String {
        "Get diagnostics (errors, warnings) for a document using LSP. \
         Arguments: file_path (file path)"
            .to_string()
    }

    fn risk_level(&self) -> ToolRiskLevel {
        ToolRiskLevel::Read
    }

    fn permission_tier(&self) -> PermissionTier {
        PermissionTier::Auto
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Path to the file"
                }
            },
            "required": ["file_path"]
        })
    }

    async fn execute(&self, arguments: Value) -> Result<ToolOutput, SwellError> {
        let file_path = arguments
            .get("file_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                SwellError::ToolExecutionFailed("Missing file_path".to_string())
            })?;

        let params = serde_json::json!({
            "filePath": file_path
        });

        let result = self.mcp_client.call_tool("diagnostics", params).await?;

        let diagnostics: Vec<LspDiagnostic> =
            serde_json::from_str(&result.result).unwrap_or_else(|_| {
                serde_json::from_str::<Vec<Value>>(&result.result)
                    .map(|vals| {
                        vals.into_iter()
                            .filter_map(|v| serde_json::from_value(v).ok())
                            .collect()
                    })
                    .unwrap_or_default()
            });

        Ok(ToolOutput {
            success: result.success,
            result: serde_json::to_string(&diagnostics).unwrap_or_default(),
            error: result.error,
        })
    }
}

/// Tool for renaming symbols using LSP
#[derive(Debug, Clone)]
pub struct LspRenameTool {
    mcp_client: McpClient,
}

impl LspRenameTool {
    pub fn new(mcp_client: McpClient) -> Self {
        Self { mcp_client }
    }
}

#[async_trait]
impl Tool for LspRenameTool {
    fn name(&self) -> &str {
        "lsp_rename"
    }

    fn description(&self) -> String {
        "Rename a symbol across the project using LSP. Returns all locations that need to be changed. \
         Arguments: file_path (file path), line (1-indexed), column (1-indexed), new_name (string)".to_string()
    }

    fn risk_level(&self) -> ToolRiskLevel {
        ToolRiskLevel::Write
    }

    fn permission_tier(&self) -> PermissionTier {
        PermissionTier::Ask
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Path to the file containing the symbol"
                },
                "line": {
                    "type": "integer",
                    "description": "Line number (1-indexed)"
                },
                "column": {
                    "type": "integer",
                    "description": "Column number (1-indexed)"
                },
                "new_name": {
                    "type": "string",
                    "description": "The new name for the symbol"
                }
            },
            "required": ["file_path", "line", "column", "new_name"]
        })
    }

    async fn execute(&self, arguments: Value) -> Result<ToolOutput, SwellError> {
        let file_path = arguments
            .get("file_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                SwellError::ToolExecutionFailed("Missing file_path".to_string())
            })?;

        let line = arguments
            .get("line")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| SwellError::ToolExecutionFailed("Missing line".to_string()))? as u32;

        let column = arguments
            .get("column")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| SwellError::ToolExecutionFailed("Missing column".to_string()))? as u32;

        let new_name = arguments
            .get("new_name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| SwellError::ToolExecutionFailed("Missing new_name".to_string()))?;

        let params = serde_json::json!({
            "filePath": file_path,
            "line": line,
            "column": column,
            "newName": new_name
        });

        let result = self.mcp_client.call_tool("rename_symbol", params).await?;

        let rename_result: LspRenameResult =
            serde_json::from_str(&result.result).unwrap_or_else(|_| LspRenameResult {
                changes: std::collections::HashMap::new(),
                success: result.success,
            });

        Ok(ToolOutput {
            success: result.success,
            result: serde_json::to_string(&rename_result).unwrap_or_default(),
            error: result.error,
        })
    }
}

// ============================================================================
// MCP-LSP Bridge Manager
// ============================================================================

/// Manages LSP tools connected via mcp-language-server
#[derive(Debug, Clone)]
pub struct LspBridgeManager {
    mcp_manager: Arc<super::mcp::McpManager>,
    language_servers: std::collections::HashMap<LspLanguage, String>,
}

impl LspBridgeManager {
    /// Create a new LSP bridge manager
    pub fn new() -> Self {
        Self {
            mcp_manager: Arc::new(super::mcp::McpManager::new()),
            language_servers: std::collections::HashMap::new(),
        }
    }

    /// Add an LSP server configuration
    ///
    /// # Arguments
    /// * `language` - The programming language (rust, cpp, c)
    /// * `server_command` - The command to start the mcp-language-server with LSP args
    ///   e.g., "npx mcp-language-server --lsp rust-analyzer" for Rust
    ///   e.g., "npx mcp-language-server --lsp clangd" for C/C++
    pub async fn add_language_server(
        &mut self,
        language: LspLanguage,
        server_command: String,
    ) -> Result<(), SwellError> {
        let server_name = match language {
            LspLanguage::Rust => "rust-analyzer",
            LspLanguage::Cpp => "clangd",
            LspLanguage::Unknown => {
                return Err(SwellError::ConfigError("Unknown language".to_string()))
            }
        };

        self.mcp_manager
            .add_server(server_name.to_string(), server_command)
            .await?;

        self.language_servers
            .insert(language, server_name.to_string());
        Ok(())
    }

    /// Register all LSP tools for a specific language with a ToolRegistry
    pub async fn register_with_registry(
        &self,
        registry: &super::registry::ToolRegistry,
        language: LspLanguage,
    ) -> Result<(), SwellError> {
        let server_name = self.language_servers.get(&language).ok_or_else(|| {
            SwellError::ConfigError(format!("No server configured for {:?}", language))
        })?;

        let client = self
            .mcp_manager
            .get_client(server_name)
            .await
            .ok_or_else(|| {
                SwellError::ToolExecutionFailed(format!("MCP client '{}' not found", server_name))
            })?;

        // Directly register each LSP tool with the concrete type
        match language {
            LspLanguage::Rust | LspLanguage::Cpp => {
                registry
                    .register(
                        LspDefinitionTool::new(client.clone()),
                        super::registry::ToolCategory::Mcp,
                    )
                    .await;
                registry
                    .register(
                        LspReferencesTool::new(client.clone()),
                        super::registry::ToolCategory::Mcp,
                    )
                    .await;
                registry
                    .register(
                        LspHoverTool::new(client.clone()),
                        super::registry::ToolCategory::Mcp,
                    )
                    .await;
                registry
                    .register(
                        LspDiagnosticsTool::new(client.clone()),
                        super::registry::ToolCategory::Mcp,
                    )
                    .await;
                registry
                    .register(
                        LspRenameTool::new(client.clone()),
                        super::registry::ToolCategory::Mcp,
                    )
                    .await;
            }
            LspLanguage::Unknown => {
                return Err(SwellError::ConfigError("Unknown language".to_string()))
            }
        }

        Ok(())
    }

    /// Get the underlying MCP manager
    pub fn mcp_manager(&self) -> &Arc<super::mcp::McpManager> {
        &self.mcp_manager
    }
}

impl Default for LspBridgeManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lsp_language_parse() {
        assert_eq!(LspLanguage::parse("rust"), LspLanguage::Rust);
        assert_eq!(LspLanguage::parse("rs"), LspLanguage::Rust);
        assert_eq!(LspLanguage::parse("RUST"), LspLanguage::Rust);

        assert_eq!(LspLanguage::parse("cpp"), LspLanguage::Cpp);
        assert_eq!(LspLanguage::parse("c++"), LspLanguage::Cpp);
        assert_eq!(LspLanguage::parse("clangd"), LspLanguage::Cpp);

        assert_eq!(LspLanguage::parse("python"), LspLanguage::Unknown);
    }

    #[test]
    fn test_lsp_location_serialization() {
        let location = LspLocation {
            uri: "file:///test.rs".to_string(),
            range: LspRange {
                start: LspPosition {
                    line: 10,
                    column: 5,
                },
                end: LspPosition {
                    line: 10,
                    column: 15,
                },
            },
        };

        let json = serde_json::to_string(&location).unwrap();
        let parsed: LspLocation = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.uri, location.uri);
        assert_eq!(parsed.range.start.line, 10);
    }

    #[test]
    fn test_lsp_symbol_serialization() {
        let symbol = LspSymbol {
            name: "my_function".to_string(),
            kind: "Function".to_string(),
            location: LspLocation {
                uri: "file:///test.rs".to_string(),
                range: LspRange {
                    start: LspPosition { line: 1, column: 0 },
                    end: LspPosition {
                        line: 1,
                        column: 12,
                    },
                },
            },
            detail: Some("fn my_function()".to_string()),
        };

        let json = serde_json::to_string(&symbol).unwrap();
        assert!(json.contains("my_function"));
        assert!(json.contains("Function"));
    }

    #[test]
    fn test_lsp_diagnostic_serialization() {
        let diagnostic = LspDiagnostic {
            severity: 1,
            message: "undefined variable".to_string(),
            source: "rust-analyzer".to_string(),
            range: LspRange {
                start: LspPosition { line: 5, column: 3 },
                end: LspPosition {
                    line: 5,
                    column: 10,
                },
            },
            code: Some("E0433".to_string()),
        };

        let json = serde_json::to_string(&diagnostic).unwrap();
        let parsed: LspDiagnostic = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed.severity, 1);
        assert_eq!(parsed.message, "undefined variable");
    }

    #[test]
    fn test_lsp_rename_result_serialization() {
        let mut changes = std::collections::HashMap::new();
        changes.insert(
            "file:///test.rs".to_string(),
            vec![LspTextEdit {
                range: LspRange {
                    start: LspPosition { line: 0, column: 4 },
                    end: LspPosition {
                        line: 0,
                        column: 10,
                    },
                },
                new_text: "new_name".to_string(),
            }],
        );

        let result = LspRenameResult {
            changes,
            success: true,
        };

        let json = serde_json::to_string(&result).unwrap();
        let parsed: LspRenameResult = serde_json::from_str(&json).unwrap();

        assert!(parsed.success);
        assert_eq!(parsed.changes.len(), 1);
    }

    #[test]
    fn test_lsp_workspace_diagnostics_serialization() {
        let diagnostics = LspWorkspaceDiagnostics {
            files: vec![LspFileDiagnostics {
                uri: "file:///error.rs".to_string(),
                diagnostics: vec![LspDiagnostic {
                    severity: 1,
                    message: "type error".to_string(),
                    source: "rust-analyzer".to_string(),
                    range: LspRange {
                        start: LspPosition { line: 0, column: 0 },
                        end: LspPosition { line: 0, column: 5 },
                    },
                    code: None,
                }],
            }],
            total_errors: 1,
            total_warnings: 0,
        };

        let json = serde_json::to_string(&diagnostics).unwrap();
        assert!(json.contains("total_errors"));
        assert!(json.contains("type error"));
    }

    #[test]
    fn test_lsp_hover_serialization() {
        let hover = LspHover {
            contents: "fn main() - Entry point to the program".to_string(),
            range: Some(LspRange {
                start: LspPosition { line: 0, column: 0 },
                end: LspPosition { line: 0, column: 4 },
            }),
        };

        let json = serde_json::to_string(&hover).unwrap();
        let parsed: LspHover = serde_json::from_str(&json).unwrap();

        assert!(parsed.contents.contains("Entry point"));
        assert!(parsed.range.is_some());
    }

    #[test]
    fn test_lsp_bridge_manager_creation() {
        let manager = LspBridgeManager::new();
        assert!(manager.language_servers.is_empty());
    }

    #[test]
    fn test_diagnostic_severity_constants() {
        assert_eq!(DiagnosticSeverity::Error as i32, 1);
        assert_eq!(DiagnosticSeverity::Warning as i32, 2);
        assert_eq!(DiagnosticSeverity::Information as i32, 3);
        assert_eq!(DiagnosticSeverity::Hint as i32, 4);
    }
}

// ============================================================================
// LSP Integration Tests
// ============================================================================

#[cfg(test)]
mod mcp_lsp_tests {
    use super::*;

    /// Test that LSP tool definitions are correct
    #[tokio::test]
    async fn test_lsp_definition_tool_info() {
        let client = McpClient::new("echo test");
        let tool = LspDefinitionTool::new(client);

        assert_eq!(tool.name(), "lsp_definition");
        assert!(tool.description().contains("definitions"));
        assert_eq!(tool.risk_level(), ToolRiskLevel::Read);
        assert_eq!(tool.permission_tier(), PermissionTier::Auto);
    }

    #[tokio::test]
    async fn test_lsp_references_tool_info() {
        let client = McpClient::new("echo test");
        let tool = LspReferencesTool::new(client);

        assert_eq!(tool.name(), "lsp_references");
        assert!(tool.description().contains("references"));
        assert_eq!(tool.risk_level(), ToolRiskLevel::Read);
    }

    #[tokio::test]
    async fn test_lsp_hover_tool_info() {
        let client = McpClient::new("echo test");
        let tool = LspHoverTool::new(client);

        assert_eq!(tool.name(), "lsp_hover");
        assert!(tool.description().contains("hover"));
        assert_eq!(tool.risk_level(), ToolRiskLevel::Read);
    }

    #[tokio::test]
    async fn test_lsp_diagnostics_tool_info() {
        let client = McpClient::new("echo test");
        let tool = LspDiagnosticsTool::new(client);

        assert_eq!(tool.name(), "lsp_diagnostics");
        assert!(tool.description().contains("diagnostics"));
        assert_eq!(tool.risk_level(), ToolRiskLevel::Read);
    }

    #[tokio::test]
    async fn test_lsp_rename_tool_info() {
        let client = McpClient::new("echo test");
        let tool = LspRenameTool::new(client);

        assert_eq!(tool.name(), "lsp_rename");
        assert!(tool.description().to_lowercase().contains("rename"));
        // Rename is a write operation
        assert_eq!(tool.risk_level(), ToolRiskLevel::Write);
        assert_eq!(tool.permission_tier(), PermissionTier::Ask);
    }

    /// Test input schema for definition tool
    #[tokio::test]
    async fn test_lsp_definition_schema() {
        let client = McpClient::new("echo test");
        let tool = LspDefinitionTool::new(client);

        let schema = tool.input_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]
            .as_object()
            .unwrap()
            .contains_key("symbol_name"));
    }

    /// Test input schema for rename tool
    #[tokio::test]
    async fn test_lsp_rename_schema() {
        let client = McpClient::new("echo test");
        let tool = LspRenameTool::new(client);

        let schema = tool.input_schema();
        assert!(schema["properties"]
            .as_object()
            .unwrap()
            .contains_key("new_name"));
        assert!(schema["properties"]
            .as_object()
            .unwrap()
            .contains_key("file_path"));
        assert!(schema["properties"]
            .as_object()
            .unwrap()
            .contains_key("line"));
        assert!(schema["properties"]
            .as_object()
            .unwrap()
            .contains_key("column"));
    }

    /// Test that LSP tools require specific arguments
    #[tokio::test]
    async fn test_lsp_definition_requires_symbol_name() {
        let client = McpClient::new("echo test");
        let tool = LspDefinitionTool::new(client);

        // Missing symbol_name should fail
        let args = serde_json::json!({});

        let result = tool.execute(args).await;
        assert!(result.is_err());
    }

    /// Test that LSP rename requires new_name
    #[tokio::test]
    async fn test_lsp_rename_requires_new_name() {
        let client = McpClient::new("echo test");
        let tool = LspRenameTool::new(client);

        let args = serde_json::json!({
            "file_path": "/test.rs",
            "line": 1,
            "column": 1
            // Missing new_name
        });

        let result = tool.execute(args).await;
        assert!(result.is_err());
    }

    /// Test tool naming conventions
    #[test]
    fn test_all_lsp_tools_prefixed() {
        let tools = [
            "lsp_definition",
            "lsp_references",
            "lsp_hover",
            "lsp_diagnostics",
            "lsp_rename",
        ];

        for tool_name in tools {
            assert!(
                tool_name.starts_with("lsp_"),
                "Tool '{}' should be prefixed with 'lsp_'",
                tool_name
            );
        }
    }

    /// Test risk levels for different LSP operations
    #[test]
    fn test_lsp_risk_levels() {
        // Read operations should be Read risk
        assert_eq!(ToolRiskLevel::Read, ToolRiskLevel::Read);

        // Write operations (rename) should be Write risk
        assert_eq!(ToolRiskLevel::Write, ToolRiskLevel::Write);
    }

    /// Test that tools have permission tiers set correctly
    #[tokio::test]
    async fn test_lsp_tool_permission_tiers() {
        let client = McpClient::new("echo test");

        // Read-only tools should be Auto
        let definition_tool = LspDefinitionTool::new(client.clone());
        assert_eq!(definition_tool.permission_tier(), PermissionTier::Auto);

        let references_tool = LspReferencesTool::new(client.clone());
        assert_eq!(references_tool.permission_tier(), PermissionTier::Auto);

        let hover_tool = LspHoverTool::new(client.clone());
        assert_eq!(hover_tool.permission_tier(), PermissionTier::Auto);

        let diagnostics_tool = LspDiagnosticsTool::new(client.clone());
        assert_eq!(diagnostics_tool.permission_tier(), PermissionTier::Auto);

        // Rename is a write operation - should require Ask
        let rename_tool = LspRenameTool::new(client);
        assert_eq!(rename_tool.permission_tier(), PermissionTier::Ask);
    }

    /// Test workspace diagnostics summary
    #[test]
    fn test_workspace_diagnostics_summary() {
        let diagnostics = LspWorkspaceDiagnostics {
            files: vec![LspFileDiagnostics {
                uri: "file:///src/main.rs".to_string(),
                diagnostics: vec![
                    LspDiagnostic {
                        severity: 1,
                        message: "error 1".to_string(),
                        source: "rust-analyzer".to_string(),
                        range: LspRange {
                            start: LspPosition { line: 1, column: 0 },
                            end: LspPosition { line: 1, column: 5 },
                        },
                        code: None,
                    },
                    LspDiagnostic {
                        severity: 2,
                        message: "warning 1".to_string(),
                        source: "rust-analyzer".to_string(),
                        range: LspRange {
                            start: LspPosition { line: 2, column: 0 },
                            end: LspPosition { line: 2, column: 5 },
                        },
                        code: None,
                    },
                ],
            }],
            total_errors: 1,
            total_warnings: 1,
        };

        assert_eq!(diagnostics.total_errors, 1);
        assert_eq!(diagnostics.total_warnings, 1);
        assert_eq!(diagnostics.files.len(), 1);
        assert_eq!(diagnostics.files[0].diagnostics.len(), 2);
    }
}
