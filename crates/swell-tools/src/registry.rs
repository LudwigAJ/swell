//! Tool registry for managing available tools with deferred/lazy loading.

use std::collections::HashMap;
use std::sync::Arc;
use swell_core::traits::Tool;
use swell_core::{PermissionTier, ToolRiskLevel};
use tokio::sync::RwLock;

// ============================================================================
// Tool Name Normalization
// ============================================================================

/// Represents a normalized tool name after parsing.
///
/// Tool names can be either:
/// - Standard tools: `file_read`, `shell`, `git_status`, etc.
/// - MCP tools: `mcp__server__tool` format (e.g., `mcp__tree_sitter__parse`)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NormalizedToolName {
    /// A standard tool name (already lowercased)
    Standard(String),
    /// An MCP tool with server and tool name components
    Mcp { server: String, tool: String },
}

impl NormalizedToolName {
    /// Returns the lookup key for this normalized name.
    /// For standard tools, returns the lowercased name.
    /// For MCP tools, returns the full MCP-format name.
    pub fn lookup_key(&self) -> String {
        match self {
            NormalizedToolName::Standard(name) => name.clone(),
            NormalizedToolName::Mcp { server, tool } => {
                format!("mcp__{}__{}", server, tool)
            }
        }
    }
}

impl std::fmt::Display for NormalizedToolName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NormalizedToolName::Standard(name) => write!(f, "{}", name),
            NormalizedToolName::Mcp { server, tool } => write!(f, "mcp__{}__{}", server, tool),
        }
    }
}

/// Aliases mapping alternative names to canonical tool names.
///
/// Common aliases include:
/// - `bash` → `shell`
/// - `sh` → `shell`
/// - `exec` → `shell`
const TOOL_ALIASES: &[(&str, &str)] = &[
    ("bash", "shell"),
    ("sh", "shell"),
    ("exec", "shell"),
    ("terminal", "shell"),
    ("cmd", "shell"),
    ("powershell", "shell"),
];

/// Normalize a tool name to its canonical form.
///
/// This function handles:
/// 1. **Case normalization**: All names are lowercased (`FILE_READ` → `file_read`)
/// 2. **Alias mapping**: Known aliases resolve to canonical names (`bash` → `shell`)
/// 3. **MCP format parsing**: `mcp__server__tool` is parsed into components
///
/// # Arguments
///
/// * `name` - The tool name to normalize (can be any case, with or without MCP prefix)
///
/// # Returns
///
/// * `NormalizedToolName` - The normalized representation
///
/// # Examples
///
/// ```
/// use swell_tools::registry::{normalize_tool_name, NormalizedToolName};
///
/// // Case normalization - underscores are stripped for flexible matching
/// assert_eq!(normalize_tool_name("FILE_READ"), NormalizedToolName::Standard("fileread".to_string()));
/// assert_eq!(normalize_tool_name("FileRead"), NormalizedToolName::Standard("fileread".to_string()));
///
/// // Alias mapping
/// assert_eq!(normalize_tool_name("Bash"), NormalizedToolName::Standard("shell".to_string()));
///
/// // MCP format - underscores in server/tool names are stripped
/// let result = normalize_tool_name("mcp__tree_sitter__parse");
/// assert_eq!(result, NormalizedToolName::Mcp { server: "treesitter".to_string(), tool: "parse".to_string() });
/// ```
pub fn normalize_tool_name(name: &str) -> NormalizedToolName {
    let name = name.trim();

    // Check for MCP format: mcp__server__tool
    if let Some(mcp_result) = try_parse_mcp_format(name) {
        return mcp_result;
    }

    // Normalize: lowercase and strip underscores for flexible matching
    // This allows "FileRead", "file_read", and "FILE_READ" to all match
    let normalized = normalize_for_lookup(name);

    // Check for aliases
    for (alias, canonical) in TOOL_ALIASES {
        if normalized == *alias {
            return NormalizedToolName::Standard(canonical.to_string());
        }
    }

    NormalizedToolName::Standard(normalized)
}

/// Convert a name to normalized form.
/// - If the input contains underscores (e.g., `"FILE_READ"`), preserve them as lowercase.
/// - If the input is pure CamelCase (no underscores, e.g., `"FileRead"`), strip to lowercase without underscores.
///
/// Examples:
/// - `"FileRead"` → `"fileread"` (no underscores for pure CamelCase)
/// - `"FILE_READ"` → `"file_read"` (preserve underscores)
/// - `"file_read"` → `"file_read"` (preserve underscores)
/// - `"HttpResponseCode"` → `"httpresponsecode"` (no underscores for pure CamelCase)
#[allow(dead_code)]
fn normalize_standard_name(name: &str) -> String {
    name.to_lowercase()
}

/// Normalize a name for lookup (strips underscores for flexible matching).
/// "FileRead" → "fileread", "file_read" → "fileread"
fn normalize_for_lookup(name: &str) -> String {
    name.to_lowercase().replace('_', "")
}

/// Try to parse a name as MCP format.
///
/// MCP format: `mcp__server__tool` (with double underscores)
/// Examples:
/// - `mcp__tree_sitter__parse` → server="tree_sitter", tool="parse"
/// - `MCP__TreeSitter__Parse` → server="treesitter", tool="parse" (lowercased)
///
/// Returns `None` if the name doesn't match MCP format.
fn try_parse_mcp_format(name: &str) -> Option<NormalizedToolName> {
    let name = name.trim();

    // Check for mcp__ prefix (case-insensitive)
    if name.to_lowercase().starts_with("mcp__") {
        // Extract server and tool parts
        let rest = &name[3..]; // Skip "mcp" prefix

        // Split by "__" to get ["", server, tool, ...]
        let parts: Vec<&str> = rest.split("__").collect();

        if parts.len() >= 3 {
            let server = parts[1];
            let tool = parts[2];

            // For MCP names, lowercase and strip underscores from server/tool
            // This ensures "TreeSitter" matches "tree_sitter" and "tree-sitter"
            let server_normalized = server.to_lowercase().replace('_', "");
            let tool_normalized = tool.to_lowercase().replace('_', "");

            if !server_normalized.is_empty() && !tool_normalized.is_empty() {
                return Some(NormalizedToolName::Mcp {
                    server: server_normalized,
                    tool: tool_normalized,
                });
            }
        }
    }

    None
}

/// Tool categories for progressive disclosure and lazy loading
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToolCategory {
    /// File operations: read, write, edit
    File,
    /// Git operations: status, diff, commit, branch
    Git,
    /// Shell execution
    Shell,
    /// Code search: grep, glob, symbol search
    Search,
    /// MCP client tools (external servers)
    Mcp,
    /// Vault credential tools
    Vault,
    /// Other miscellaneous tools
    Misc,
}

impl ToolCategory {
    /// Get all known categories
    pub fn all() -> &'static [ToolCategory] {
        &[
            ToolCategory::File,
            ToolCategory::Git,
            ToolCategory::Shell,
            ToolCategory::Search,
            ToolCategory::Mcp,
            ToolCategory::Vault,
            ToolCategory::Misc,
        ]
    }

    /// Get display name for category
    pub fn display_name(&self) -> &'static str {
        match self {
            ToolCategory::File => "File Operations",
            ToolCategory::Git => "Git Operations",
            ToolCategory::Shell => "Shell Execution",
            ToolCategory::Search => "Code Search",
            ToolCategory::Mcp => "MCP External Tools",
            ToolCategory::Vault => "Vault Credentials",
            ToolCategory::Misc => "Miscellaneous",
        }
    }
}

impl std::fmt::Display for ToolCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display_name())
    }
}

/// A registered tool with metadata
#[derive(Clone)]
pub struct ToolRegistration {
    pub name: String,
    pub description: String,
    pub risk_level: ToolRiskLevel,
    pub permission_tier: PermissionTier,
    pub category: ToolCategory,
    pub tool: Arc<dyn Tool>,
}

impl ToolRegistration {
    /// Create a new tool registration from a tool instance
    fn from_tool<T: Tool + 'static>(tool: T, category: ToolCategory) -> Self {
        Self {
            name: tool.name().to_string(),
            description: tool.description(),
            risk_level: tool.risk_level(),
            permission_tier: tool.permission_tier(),
            category,
            tool: Arc::new(tool),
        }
    }
}

/// Category metadata for lazy loading UI
#[derive(Clone)]
pub struct CategoryInfo {
    pub category: ToolCategory,
    pub tool_count: usize,
    pub is_loaded: bool,
}

/// Central registry for all available tools with deferred/lazy loading.
///
/// # Progressive Disclosure Design
///
/// - **Tier 1 (Startup)**: Only category metadata is loaded (names, counts)
/// - **Tier 2 (On Category Access)**: Full tool list for that category is materialized
/// - **Tier 3 (On Tool Access)**: Individual tool instances are loaded on first use
///
/// # Category-Level Lazy Loading
///
/// Tools are organized into categories. When a category is accessed for the first time,
/// Factory entry: category and factory function for deferred loading
#[allow(clippy::type_complexity)]
type FactoryEntry = (ToolCategory, Box<dyn ToolFactory>);

/// only then are the tools for that category loaded into memory. This allows the system
/// to present a catalog of available tools without the overhead of instantiating all of them.
pub struct ToolRegistry {
    /// Registered tools by name - loaded on-demand
    tools: Arc<RwLock<HashMap<String, ToolRegistration>>>,
    /// Category tool indexes - built at registration time, used for lazy loading
    category_index: Arc<RwLock<HashMap<ToolCategory, Vec<String>>>>,
    /// Tracks which categories have been fully loaded
    loaded_categories: Arc<RwLock<Vec<ToolCategory>>>,
    /// Tool factory functions for deferred loading
    factories: Arc<RwLock<HashMap<String, FactoryEntry>>>,
    /// Lock for initialization
    initialized: Arc<RwLock<bool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: Arc::new(RwLock::new(HashMap::new())),
            category_index: Arc::new(RwLock::new(HashMap::new())),
            loaded_categories: Arc::new(RwLock::new(Vec::new())),
            factories: Arc::new(RwLock::new(HashMap::new())),
            initialized: Arc::new(RwLock::new(false)),
        }
    }

    /// Register a tool with its category
    pub async fn register<T: Tool + 'static>(&self, tool: T, category: ToolCategory) {
        let registration = ToolRegistration::from_tool(tool, category);
        let original_name = registration.name.clone();

        // Normalize the tool name for consistent lookup
        // This ensures case-insensitive and alias-based lookups work
        let normalized = normalize_tool_name(&original_name);
        let key = normalized.lookup_key();

        // Add to tools map using the normalized key
        let mut tools = self.tools.write().await;
        tools.insert(key.clone(), registration);

        // Update category index
        let mut index = self.category_index.write().await;
        index
            .entry(category)
            .or_insert_with(Vec::new)
            .push(key.clone());

        // Mark category as loaded since we have actual tools in it
        drop(index);
        self.load_category(category).await;
    }

    /// Register a tool with default category (Misc)
    pub async fn register_<T: Tool + 'static>(&self, tool: T) {
        self.register(tool, ToolCategory::Misc).await;
    }

    /// Register a tool factory for deferred loading on first access
    ///
    /// This allows tools to be instantiated only when they're actually needed,
    /// reducing startup memory overhead for large tool libraries.
    pub async fn register_factory<F>(&self, name: String, category: ToolCategory, factory: F)
    where
        F: Fn() -> Arc<dyn Tool> + Send + Sync + 'static,
    {
        // Normalize the factory name for consistent lookup
        let normalized_name = normalize_for_lookup(&name);
        let mut factories = self.factories.write().await;
        factories.insert(normalized_name, (category, Box::new(factory)));
    }

    /// Load a specific category on-demand
    async fn load_category(&self, category: ToolCategory) {
        // Check if already loaded
        {
            let loaded = self.loaded_categories.read().await;
            if loaded.contains(&category) {
                return;
            }
        }

        // Mark as loaded
        {
            let mut loaded = self.loaded_categories.write().await;
            if !loaded.contains(&category) {
                loaded.push(category);
            }
        }
    }

    /// Get a tool by name, loading it on-demand if not already loaded.
    ///
    /// This method performs **case-insensitive lookup** with support for:
    /// - Case normalization: `FILE_READ` → `file_read`
    /// - Alias resolution: `Bash` → `shell`
    /// - MCP format: `mcp__server__tool` is parsed and resolved
    ///
    /// # Arguments
    ///
    /// * `name` - The tool name to look up (case-insensitive)
    ///
    /// # Returns
    ///
    /// The tool if found, `None` otherwise.
    pub async fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        // Normalize the tool name (case-insensitive, aliases, MCP format)
        let normalized = normalize_tool_name(name);
        let lookup_key = normalized.lookup_key();

        // Fast path: already loaded
        {
            let tools = self.tools.read().await;
            if let Some(registration) = tools.get(&lookup_key) {
                return Some(registration.tool.clone());
            }
        }

        // Slow path: try to load from factory
        {
            let factories = self.factories.read().await;
            if let Some((category, factory)) = factories.get(&lookup_key) {
                // Load the category first
                self.load_category(*category).await;

                // Instantiate the tool
                let tool = factory.create();

                // Register it for future access using the normalized key
                let registration = ToolRegistration {
                    name: lookup_key.clone(),
                    description: tool.description(),
                    risk_level: tool.risk_level(),
                    permission_tier: tool.permission_tier(),
                    category: *category,
                    tool: tool.clone(),
                };

                let mut tools = self.tools.write().await;
                tools.insert(lookup_key, registration);

                return Some(tool);
            }
        }

        None
    }

    /// List all registered tool names (does not load unloaded tools)
    pub async fn list_names(&self) -> Vec<String> {
        let tools = self.tools.read().await;
        let mut names: Vec<String> = tools.keys().cloned().collect();

        // Also include factory names that haven't been loaded
        let factories = self.factories.read().await;
        for name in factories.keys() {
            if !names.contains(name) {
                names.push(name.clone());
            }
        }

        names
    }

    /// List all tool registrations (only loaded tools)
    pub async fn list(&self) -> Vec<ToolRegistration> {
        let tools = self.tools.read().await;
        tools.values().cloned().collect()
    }

    /// Get category information without loading tools
    pub async fn list_categories(&self) -> Vec<CategoryInfo> {
        let index = self.category_index.read().await;
        let factories = self.factories.read().await;
        let loaded = self.loaded_categories.read().await;

        ToolCategory::all()
            .iter()
            .map(|category| {
                let is_loaded = loaded.contains(category);
                let tool_count = index.get(category).map(|v| v.len()).unwrap_or(0);

                // Add factory count for unloaded categories
                let factory_count = factories
                    .values()
                    .filter(|(cat, _)| *cat == *category)
                    .count();

                CategoryInfo {
                    category: *category,
                    tool_count: tool_count + if is_loaded { 0 } else { factory_count },
                    is_loaded,
                }
            })
            .collect()
    }

    /// List tools in a specific category, loading category if needed
    pub async fn list_by_category(&self, category: ToolCategory) -> Vec<ToolRegistration> {
        // Ensure category is loaded
        self.load_category(category).await;

        let tools = self.tools.read().await;
        tools
            .values()
            .filter(|r| r.category == category)
            .cloned()
            .collect()
    }

    /// Check if a tool is registered (loaded or in factory).
    ///
    /// This method performs **case-insensitive lookup** with support for:
    /// - Case normalization: `FILE_READ` → `file_read`
    /// - Alias resolution: `Bash` → `shell`
    /// - MCP format: `mcp__server__tool` is parsed and resolved
    pub async fn contains(&self, name: &str) -> bool {
        // Normalize the tool name (case-insensitive, aliases, MCP format)
        let normalized = normalize_tool_name(name);
        let lookup_key = normalized.lookup_key();

        // Check loaded tools
        {
            let tools = self.tools.read().await;
            if tools.contains_key(&lookup_key) {
                return true;
            }
        }

        // Check factories
        {
            let factories = self.factories.read().await;
            factories.contains_key(&lookup_key)
        }
    }

    /// Remove a tool using normalized name lookup.
    ///
    /// This method performs **case-insensitive lookup** with support for:
    /// - Case normalization: `FILE_READ` → `file_read`
    /// - Alias resolution: `Bash` → `shell`
    /// - MCP format: `mcp__server__tool` is parsed and resolved
    pub async fn unregister(&self, name: &str) -> bool {
        let normalized = normalize_tool_name(name);
        let lookup_key = normalized.lookup_key();
        let mut tools = self.tools.write().await;
        tools.remove(&lookup_key).is_some()
    }

    /// Get tools filtered by risk level (only loaded tools)
    pub async fn by_risk_level(&self, level: ToolRiskLevel) -> Vec<ToolRegistration> {
        let tools = self.tools.read().await;
        tools
            .values()
            .filter(|r| r.risk_level == level)
            .cloned()
            .collect()
    }

    /// Get total count of all tools (registered + factories)
    pub async fn count(&self) -> usize {
        let tools = self.tools.read().await;
        let factories = self.factories.read().await;
        tools.len() + factories.len()
    }

    /// Get count of loaded tools only
    pub async fn loaded_count(&self) -> usize {
        let tools = self.tools.read().await;
        tools.len()
    }

    /// Check if a specific category has been loaded
    pub async fn is_category_loaded(&self, category: ToolCategory) -> bool {
        let loaded = self.loaded_categories.read().await;
        loaded.contains(&category)
    }
}

/// Factory trait for creating tools on-demand
pub trait ToolFactory: Send + Sync {
    fn create(&self) -> Arc<dyn Tool>;
}

impl<T: Fn() -> Arc<dyn Tool> + Send + Sync + 'static> ToolFactory for T {
    fn create(&self) -> Arc<dyn Tool> {
        self()
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for ToolRegistry {
    fn clone(&self) -> Self {
        Self {
            tools: self.tools.clone(),
            category_index: self.category_index.clone(),
            loaded_categories: self.loaded_categories.clone(),
            factories: self.factories.clone(),
            initialized: self.initialized.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use swell_core::ToolOutput;

    /// A mock tool for testing
    struct MockTool {
        name: String,
        category: ToolCategory,
    }

    impl MockTool {
        fn new(name: &str, category: ToolCategory) -> Self {
            Self {
                name: name.to_string(),
                category,
            }
        }
    }

    #[async_trait::async_trait]
    impl Tool for MockTool {
        fn name(&self) -> &str {
            &self.name
        }
        fn description(&self) -> String {
            format!("Mock tool: {}", self.name)
        }
        fn risk_level(&self) -> ToolRiskLevel {
            ToolRiskLevel::Read
        }
        fn permission_tier(&self) -> PermissionTier {
            PermissionTier::Auto
        }
        fn input_schema(&self) -> serde_json::Value {
            serde_json::json!({})
        }
        async fn execute(
            &self,
            _: serde_json::Value,
        ) -> Result<ToolOutput, swell_core::SwellError> {
            Ok(ToolOutput {
                success: true,
                result: "executed".to_string(),
                error: None,
            })
        }
    }

    #[tokio::test]
    async fn test_tool_category_all() {
        let categories = ToolCategory::all();
        assert_eq!(categories.len(), 7);
        assert!(categories.contains(&ToolCategory::File));
        assert!(categories.contains(&ToolCategory::Git));
        assert!(categories.contains(&ToolCategory::Shell));
        assert!(categories.contains(&ToolCategory::Search));
        assert!(categories.contains(&ToolCategory::Mcp));
        assert!(categories.contains(&ToolCategory::Vault));
        assert!(categories.contains(&ToolCategory::Misc));
    }

    #[tokio::test]
    async fn test_category_display_name() {
        assert_eq!(ToolCategory::File.display_name(), "File Operations");
        assert_eq!(ToolCategory::Git.display_name(), "Git Operations");
        assert_eq!(ToolCategory::Shell.display_name(), "Shell Execution");
        assert_eq!(ToolCategory::Search.display_name(), "Code Search");
        assert_eq!(ToolCategory::Mcp.display_name(), "MCP External Tools");
        assert_eq!(ToolCategory::Vault.display_name(), "Vault Credentials");
        assert_eq!(ToolCategory::Misc.display_name(), "Miscellaneous");
    }

    #[tokio::test]
    async fn test_registry_progressive_disclosure_categories() {
        let registry = ToolRegistry::new();

        // Initially, no categories are loaded
        let categories = registry.list_categories().await;
        assert_eq!(categories.len(), 7);
        for cat in categories {
            assert!(
                !cat.is_loaded,
                "Category {:?} should not be loaded initially",
                cat.category
            );
        }
    }

    #[tokio::test]
    async fn test_registry_register_with_category() {
        let registry = ToolRegistry::new();

        // Register tools in different categories
        registry
            .register(
                MockTool::new("file_tool", ToolCategory::File),
                ToolCategory::File,
            )
            .await;
        registry
            .register(
                MockTool::new("git_tool", ToolCategory::Git),
                ToolCategory::Git,
            )
            .await;
        registry
            .register(
                MockTool::new("shell_tool", ToolCategory::Shell),
                ToolCategory::Shell,
            )
            .await;

        // Verify all tools are registered
        assert_eq!(registry.list().await.len(), 3);
        assert!(registry.contains("file_tool").await);
        assert!(registry.contains("git_tool").await);
        assert!(registry.contains("shell_tool").await);
    }

    #[tokio::test]
    async fn test_registry_list_by_category() {
        let registry = ToolRegistry::new();

        registry
            .register(
                MockTool::new("file_tool_1", ToolCategory::File),
                ToolCategory::File,
            )
            .await;
        registry
            .register(
                MockTool::new("file_tool_2", ToolCategory::File),
                ToolCategory::File,
            )
            .await;
        registry
            .register(
                MockTool::new("git_tool", ToolCategory::Git),
                ToolCategory::Git,
            )
            .await;

        // List by category
        let file_tools = registry.list_by_category(ToolCategory::File).await;
        assert_eq!(file_tools.len(), 2);

        let git_tools = registry.list_by_category(ToolCategory::Git).await;
        assert_eq!(git_tools.len(), 1);
    }

    #[tokio::test]
    async fn test_registry_factory_deferred_loading() {
        let registry = ToolRegistry::new();

        // Register a factory - tool is NOT loaded yet
        let load_count = Arc::new(tokio::sync::Mutex::new(0usize));
        let load_count_clone = load_count.clone();

        registry
            .register_factory("deferred_tool".to_string(), ToolCategory::File, move || {
                let count = load_count_clone.clone();
                Arc::new(MockTool::new("deferred_tool", ToolCategory::File)) as Arc<dyn Tool>
            })
            .await;

        // Tool should be in registry (as a factory) but not yet loaded
        assert!(registry.contains("deferred_tool").await);
        assert_eq!(registry.loaded_count().await, 0);

        // Access the tool - now it should be loaded
        let tool = registry.get("deferred_tool").await;
        assert!(tool.is_some());
        assert_eq!(registry.loaded_count().await, 1);

        // Access again - should still work and not re-load
        let tool2 = registry.get("deferred_tool").await;
        assert!(tool2.is_some());
        assert_eq!(registry.loaded_count().await, 1); // Still 1, not re-loaded
    }

    #[tokio::test]
    async fn test_registry_factory_on_demand() {
        let registry = ToolRegistry::new();

        // Register multiple factories
        for i in 0..5 {
            let name = format!("lazy_tool_{}", i);
            let registry_name = name.clone();
            registry
                .register_factory(name, ToolCategory::Misc, move || {
                    Arc::new(MockTool::new(&registry_name, ToolCategory::Misc)) as Arc<dyn Tool>
                })
                .await;
        }

        // No tools loaded yet
        assert_eq!(registry.loaded_count().await, 0);
        assert_eq!(registry.count().await, 5);

        // All factory tools should be in list_names (since they're registered)
        let names = registry.list_names().await;
        assert_eq!(names.len(), 5);
        // Names are normalized (underscores stripped) for flexible matching
        assert!(names.contains(&"lazytool0".to_string()));

        // Load only one specific tool on-demand
        let tool = registry.get("lazy_tool_2").await;
        assert!(tool.is_some());

        // Only that one tool should be loaded
        assert_eq!(registry.loaded_count().await, 1);
    }

    #[tokio::test]
    async fn test_registry_count() {
        let registry = ToolRegistry::new();

        // Initially empty
        assert_eq!(registry.count().await, 0);
        assert_eq!(registry.loaded_count().await, 0);

        // Register a regular tool
        registry
            .register(
                MockTool::new("regular_tool", ToolCategory::File),
                ToolCategory::File,
            )
            .await;
        assert_eq!(registry.count().await, 1);
        assert_eq!(registry.loaded_count().await, 1);

        // Register a factory
        registry
            .register_factory("factory_tool".to_string(), ToolCategory::Git, || {
                Arc::new(MockTool::new("factory_tool", ToolCategory::Git)) as Arc<dyn Tool>
            })
            .await;
        assert_eq!(registry.count().await, 2); // Total = 1 loaded + 1 factory
        assert_eq!(registry.loaded_count().await, 1); // Still only 1 loaded
    }

    #[tokio::test]
    async fn test_registry_by_risk_level() {
        let registry = ToolRegistry::new();

        // MockTool defaults to ToolRiskLevel::Read
        registry
            .register(
                MockTool::new("read_tool", ToolCategory::File),
                ToolCategory::File,
            )
            .await;
        registry
            .register(
                MockTool::new("another_read_tool", ToolCategory::File),
                ToolCategory::File,
            )
            .await;

        let read_tools = registry.by_risk_level(ToolRiskLevel::Read).await;
        assert_eq!(read_tools.len(), 2); // Both tools are Read risk
    }

    #[tokio::test]
    async fn test_registry_category_loading() {
        let registry = ToolRegistry::new();

        // Register tools
        registry
            .register(
                MockTool::new("tool1", ToolCategory::File),
                ToolCategory::File,
            )
            .await;

        // File category should be marked as loaded after registration
        assert!(registry.is_category_loaded(ToolCategory::File).await);
        assert!(!registry.is_category_loaded(ToolCategory::Git).await);
    }

    #[tokio::test]
    async fn test_registry_tool_registration_has_category() {
        let registry = ToolRegistry::new();

        registry
            .register(
                MockTool::new("my_tool", ToolCategory::Search),
                ToolCategory::Search,
            )
            .await;

        let tools = registry.list_by_category(ToolCategory::Search).await;
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].category, ToolCategory::Search);
    }

    // =============================================================================
    // Tool Name Normalization Tests
    // =============================================================================

    #[test]
    fn test_normalize_tool_name_case_insensitive() {
        // Case normalization tests - underscores are stripped for flexible matching
        assert_eq!(
            normalize_tool_name("FILE_READ"),
            NormalizedToolName::Standard("fileread".to_string())
        );
        assert_eq!(
            normalize_tool_name("file_read"),
            NormalizedToolName::Standard("fileread".to_string())
        );
        assert_eq!(
            normalize_tool_name("FileRead"),
            NormalizedToolName::Standard("fileread".to_string())
        );
        assert_eq!(
            normalize_tool_name("File_Read"),
            NormalizedToolName::Standard("fileread".to_string())
        );
    }

    #[test]
    fn test_normalize_tool_name_alias_bash_to_shell() {
        // bash should resolve to shell
        assert_eq!(
            normalize_tool_name("Bash"),
            NormalizedToolName::Standard("shell".to_string())
        );
        assert_eq!(
            normalize_tool_name("bash"),
            NormalizedToolName::Standard("shell".to_string())
        );
        assert_eq!(
            normalize_tool_name("BASH"),
            NormalizedToolName::Standard("shell".to_string())
        );
    }

    #[test]
    fn test_normalize_tool_name_alias_sh_to_shell() {
        // sh should resolve to shell
        assert_eq!(
            normalize_tool_name("sh"),
            NormalizedToolName::Standard("shell".to_string())
        );
        assert_eq!(
            normalize_tool_name("SH"),
            NormalizedToolName::Standard("shell".to_string())
        );
    }

    #[test]
    fn test_normalize_tool_name_alias_exec_to_shell() {
        // exec should resolve to shell
        assert_eq!(
            normalize_tool_name("exec"),
            NormalizedToolName::Standard("shell".to_string())
        );
        assert_eq!(
            normalize_tool_name("EXEC"),
            NormalizedToolName::Standard("shell".to_string())
        );
    }

    #[test]
    fn test_normalize_tool_name_alias_terminal_to_shell() {
        // terminal should resolve to shell
        assert_eq!(
            normalize_tool_name("terminal"),
            NormalizedToolName::Standard("shell".to_string())
        );
    }

    #[test]
    fn test_normalize_tool_name_no_alias() {
        // Tools without aliases are normalized with underscores stripped
        assert_eq!(
            normalize_tool_name("file_read"),
            NormalizedToolName::Standard("fileread".to_string())
        );
        assert_eq!(
            normalize_tool_name("git_status"),
            NormalizedToolName::Standard("gitstatus".to_string())
        );
        assert_eq!(
            normalize_tool_name("grep"),
            NormalizedToolName::Standard("grep".to_string())
        );
    }

    #[test]
    fn test_normalize_tool_name_mcp_format() {
        // MCP format: mcp__server__tool - underscores in server/tool are stripped
        let result = normalize_tool_name("mcp__tree_sitter__parse");
        assert_eq!(
            result,
            NormalizedToolName::Mcp {
                server: "treesitter".to_string(),
                tool: "parse".to_string()
            }
        );
    }

    #[test]
    fn test_normalize_tool_name_mcp_format_lowercase() {
        // MCP format should have components lowercased
        let result = normalize_tool_name("MCP__TREE_SITTER__PARSE");
        assert_eq!(
            result,
            NormalizedToolName::Mcp {
                server: "treesitter".to_string(),
                tool: "parse".to_string()
            }
        );
    }

    #[test]
    fn test_normalize_tool_name_mcp_format_mixed_case() {
        // MCP format with mixed case
        let result = normalize_tool_name("mcp__TreeSitter__ParseAST");
        assert_eq!(
            result,
            NormalizedToolName::Mcp {
                server: "treesitter".to_string(),
                tool: "parseast".to_string()
            }
        );
    }

    #[test]
    fn test_normalize_tool_name_mcp_lookup_key() {
        // Verify lookup key generation - underscores stripped from server/tool
        let result = normalize_tool_name("mcp__tree_sitter__parse");
        assert_eq!(result.lookup_key(), "mcp__treesitter__parse");
    }

    #[test]
    fn test_normalize_tool_name_mcp_with_extra_parts() {
        // MCP format with more than 3 parts (should use first 3)
        let result = normalize_tool_name("mcp__server__tool__extra");
        assert_eq!(
            result,
            NormalizedToolName::Mcp {
                server: "server".to_string(),
                tool: "tool".to_string()
            }
        );
    }

    #[test]
    fn test_normalize_tool_name_invalid_mcp_format() {
        // Invalid MCP format should fall back to standard normalization
        // Missing double underscore - underscores stripped
        let result = normalize_tool_name("mcp_server_tool");
        assert_eq!(
            result,
            NormalizedToolName::Standard("mcpservertool".to_string())
        );

        // Single underscore
        let result = normalize_tool_name("mcp_server_tool");
        assert_eq!(
            result,
            NormalizedToolName::Standard("mcpservertool".to_string())
        );
    }

    #[test]
    fn test_normalized_tool_name_display() {
        // Test the display of NormalizedToolName
        let standard = NormalizedToolName::Standard("file_read".to_string());
        assert_eq!(format!("{}", standard), "file_read");

        let mcp = NormalizedToolName::Mcp {
            server: "tree_sitter".to_string(),
            tool: "parse".to_string(),
        };
        assert_eq!(format!("{}", mcp), "mcp__tree_sitter__parse");
    }

    #[tokio::test]
    async fn test_registry_get_case_insensitive() {
        let registry = ToolRegistry::new();

        // Register a tool with lowercase name
        registry
            .register(
                MockTool::new("file_read", ToolCategory::File),
                ToolCategory::File,
            )
            .await;

        // Should find with different cases
        assert!(registry.get("file_read").await.is_some());
        assert!(registry.get("FILE_READ").await.is_some());
        assert!(registry.get("FileRead").await.is_some());
        assert!(registry.get("File_Read").await.is_some());
    }

    #[tokio::test]
    async fn test_registry_get_alias_resolution() {
        let registry = ToolRegistry::new();

        // Register shell tool
        registry
            .register(
                MockTool::new("shell", ToolCategory::Shell),
                ToolCategory::Shell,
            )
            .await;

        // Should find with bash (alias)
        assert!(registry.get("bash").await.is_some());
        assert!(registry.get("BASH").await.is_some());
        assert!(registry.get("sh").await.is_some());
        assert!(registry.get("exec").await.is_some());
    }

    #[tokio::test]
    async fn test_registry_contains_case_insensitive() {
        let registry = ToolRegistry::new();

        // Register a tool
        registry
            .register(
                MockTool::new("file_read", ToolCategory::File),
                ToolCategory::File,
            )
            .await;

        // Should return true with different cases
        assert!(registry.contains("file_read").await);
        assert!(registry.contains("FILE_READ").await);
        assert!(registry.contains("FileRead").await);
    }

    #[tokio::test]
    async fn test_registry_unregister_case_insensitive() {
        let registry = ToolRegistry::new();

        // Register a tool
        registry
            .register(
                MockTool::new("file_read", ToolCategory::File),
                ToolCategory::File,
            )
            .await;

        // Should be able to unregister with different cases
        assert!(registry.unregister("FILE_READ").await);
        assert!(!registry.contains("file_read").await); // No longer exists
    }

    #[tokio::test]
    async fn test_registry_get_mcp_format() {
        let registry = ToolRegistry::new();

        // Register an MCP tool with its actual MCP-format name
        registry
            .register(
                MockTool::new("mcp__tree_sitter__parse", ToolCategory::Mcp),
                ToolCategory::Mcp,
            )
            .await;

        // Should find with normalized MCP format
        assert!(registry.get("mcp__tree_sitter__parse").await.is_some());
        assert!(registry.get("MCP__TREE_SITTER__PARSE").await.is_some());
        assert!(registry.get("mcp__TreeSitter__Parse").await.is_some());
    }
}
