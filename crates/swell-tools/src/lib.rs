//! Tool registry and execution for SWELL.
//!
//! This crate provides:
//! - [`ToolRegistry`] - central registry for all tools
//! - [`ToolExecutor`] - executes tools with permission enforcement
//! - Built-in tools: file I/O, git, shell execution
//! - MCP client for external tool servers

pub mod registry;
pub mod executor;
pub mod tools;
pub mod mcp;

pub use registry::{ToolRegistry, ToolRegistration};
pub use executor::ToolExecutor;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_registry_empty() {
        let registry = ToolRegistry::new();
        assert!(registry.list().await.is_empty());
    }

    #[tokio::test]
    async fn test_registry_registration() {
        let mut registry = ToolRegistry::new();
        registry.register(tools::ReadFileTool::new()).await;
        assert_eq!(registry.list().await.len(), 1);
    }
}
