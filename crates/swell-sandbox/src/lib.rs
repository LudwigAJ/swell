//! swell-sandbox - Sandbox management (gVisor, Firecracker stub)
//!
//! This crate provides sandboxed execution environments using various isolation
//! technologies:
//! - **gVisor (runsc)** - ✅ **FULLY IMPLEMENTED** - User-space syscall interception via Docker
//! - **Firecracker microVMs** - ❌ **STUB ONLY** - Requires KVM, not available on all platforms
//!
//! ## Current Status
//!
//! ### gVisor (Recommended)
//! ```rust,ignore
//! use swell_sandbox::GvisorSandbox;
//!
//! let config = GvisorConfig::default();
//! let sandbox = GvisorSandbox::new(config);
//! sandbox.start().await?;
//! ```
//!
//! ### Firecracker (STUB - NOT FUNCTIONAL)
//! ```rust,ignore
//! use swell_sandbox::FirecrackerSandbox; // DEPRECATED - returns mock responses only!
//!
//! // WARNING: This creates a STUB, NOT a real microVM
//! let firecracker = FirecrackerSandbox::with_params("vm-1".to_string(), 256);
//! // firecracker.start() will NOT create an actual microVM
//! ```
//!
//! Firecracker requires KVM (Kernel-based Virtual Machine) which is not available on:
//! - macOS (no native KVM support)
//! - Most CI environments
//! - Containers without nested virtualization
//!
//! For actual sandboxed execution, use [`GvisorSandbox`] which works via Docker.

mod gvisor;

pub use gvisor::{GvisorConfig, GvisorNetworkMode, GvisorSandbox};

use async_trait::async_trait;
use swell_core::{Sandbox, SandboxCommand, SandboxOutput, SwellError};

/// Configuration for a Firecracker sandbox instance
#[derive(Debug, Clone)]
pub struct SandboxConfig {
    /// Unique identifier for this VM
    pub vm_id: String,
    /// Memory in megabytes
    pub memory_mb: u64,
    /// Number of virtual CPUs
    pub vcpu_count: u32,
    /// Execution timeout in seconds
    pub timeout_secs: u64,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            vm_id: uuid::Uuid::new_v4().to_string(),
            memory_mb: 256,
            vcpu_count: 2,
            timeout_secs: 300,
        }
    }
}

/// A Firecracker-based sandbox implementation (STUB - NOT FUNCTIONAL).
///
/// # ⚠️ Important Limitations
///
/// This is a **STUB implementation** that does NOT create actual Firecracker microVMs.
/// All methods return mock responses or log operations without real isolation.
///
/// ## What This Stub Does
/// - Logs sandbox operations (start, stop, execute, file operations)
/// - Returns mock responses for all operations
/// - Does NOT provide actual microVM isolation
///
/// ## Requirements for Real Implementation
/// To implement actual Firecracker microVM support, the following are required:
/// - **KVM access**: Firecracker requires Linux with Kernel-based Virtual Machine (KVM)
///   - Not available on macOS natively (requires nested virtualization)
///   - Not available in most CI environments
/// - **Firecracker binary**: Must be installed on the host system
/// - **VM images**: Kernel and root filesystem images for the guest VM
/// - **Guest agent**: A running agent inside the VM to handle commands
///
/// ## Performance Targets (for real implementation)
/// - Startup time: <125ms (per spec)
/// - Memory overhead: <5 MiB per microVM
/// - Hardware virtualization isolation via KVM
///
/// ## Current Status
/// - **NOT IMPLEMENTED**: This is a stub for API compatibility
/// - **USE gVisorSandbox INSTEAD**: For actual sandboxed execution
///
/// # Example
///
/// ```rust,ignore
/// use swell_sandbox::FirecrackerSandbox; // This is a STUB!
///
/// // This creates a STUB instance, NOT a real microVM
/// let sandbox = FirecrackerSandbox::with_params("vm-1".to_string(), 256);
/// // sandbox.start() will NOT create a real microVM
/// ```
///
/// For actual sandboxed execution, use [`GvisorSandbox`] which provides
/// working containerized isolation via Docker with gVisor runtime.
pub struct FirecrackerSandboxStub {
    config: SandboxConfig,
    /// Whether to return errors instead of mock responses.
    /// When true, all operations return an error indicating stub status.
    return_errors: bool,
}

impl FirecrackerSandboxStub {
    /// Create a new FirecrackerSandboxStub with the given configuration
    ///
    /// # Warning
    ///
    /// This creates a **STUB** instance, not a real Firecracker microVM.
    /// The returned sandbox does NOT provide actual isolation.
    pub fn new(config: SandboxConfig) -> Self {
        Self {
            config,
            return_errors: false,
        }
    }

    /// Create a new FirecrackerSandboxStub with simple parameters
    ///
    /// # Warning
    ///
    /// This creates a **STUB** instance, not a real Firecracker microVM.
    pub fn with_params(vm_id: String, memory_mb: u64) -> Self {
        Self {
            config: SandboxConfig {
                vm_id,
                memory_mb,
                ..Default::default()
            },
            return_errors: false,
        }
    }

    /// Get the configuration for this sandbox
    pub fn config(&self) -> &SandboxConfig {
        &self.config
    }

    /// Set whether to return errors instead of mock responses.
    ///
    /// When enabled, all operations will return a [`SwellError::SandboxError`]
    /// indicating that Firecracker is not implemented.
    ///
    /// Default is false (returns mock responses for backward compatibility).
    pub fn set_return_errors(&mut self, return_errors: bool) {
        self.return_errors = return_errors;
    }

    fn stub_error(&self, operation: &str) -> SwellError {
        SwellError::SandboxError(format!(
            "FirecrackerSandbox is a STUB - {} not implemented. \
             Use GvisorSandbox for actual sandboxed execution. \
             Firecracker requires KVM which is not available on this platform.",
            operation
        ))
    }
}

/// Type alias for backward compatibility.
/// In previous versions, this was a seemingly-functional sandbox.
/// Now it is explicitly a stub - see [`FirecrackerSandboxStub`] for details.
#[deprecated(
    since = "0.1.0",
    note = "FirecrackerSandbox is a STUB and does not provide actual microVM isolation. \
            Use GvisorSandbox for actual sandboxed execution."
)]
pub type FirecrackerSandbox = FirecrackerSandboxStub;

#[async_trait]
impl Sandbox for FirecrackerSandboxStub {
    fn id(&self) -> &str {
        &self.config.vm_id
    }

    async fn start(&self) -> Result<(), SwellError> {
        tracing::warn!(
            vm_id = %self.config.vm_id,
            memory_mb = %self.config.memory_mb,
            vcpu_count = %self.config.vcpu_count,
            "FirecrackerSandbox: STUB - not starting real microVM"
        );

        if self.return_errors {
            return Err(self.stub_error("start"));
        }

        // STUB: In real implementation, this would:
        // 1. Check KVM availability
        // 2. Download/create VM image if needed
        // 3. Start Firecracker process with KVM
        // 4. Wait for guest agent to be ready
        // 5. Achieve <125ms startup time
        Ok(())
    }

    async fn stop(&self) -> Result<(), SwellError> {
        tracing::warn!(
            vm_id = %self.config.vm_id,
            "FirecrackerSandbox: STUB - not stopping real microVM"
        );

        if self.return_errors {
            return Err(self.stub_error("stop"));
        }

        // STUB: In real implementation, this would:
        // 1. Send graceful shutdown to guest
        // 2. Force kill if needed
        // 3. Clean up resources
        Ok(())
    }

    async fn execute(&self, cmd: SandboxCommand) -> Result<SandboxOutput, SwellError> {
        tracing::warn!(
            vm_id = %self.config.vm_id,
            command = %cmd.command,
            args = ?cmd.args,
            "FirecrackerSandbox: STUB - not executing in real microVM"
        );

        if self.return_errors {
            return Err(self.stub_error("execute"));
        }

        // STUB: In real implementation, this would:
        // 1. Serialize command and send to guest agent via vsock
        // 2. Wait for response with timeout
        // 3. Return actual output with real execution time
        Ok(SandboxOutput {
            exit_code: 0,
            stdout: format!(
                "STUB: would execute '{}' with args {:?}",
                cmd.command, cmd.args
            ),
            stderr: String::new(),
            duration_ms: 10,
        })
    }

    async fn write_file(&self, path: &str, content: &[u8]) -> Result<(), SwellError> {
        tracing::warn!(
            vm_id = %self.config.vm_id,
            path = %path,
            size = %content.len(),
            "FirecrackerSandbox: STUB - not writing to real microVM"
        );

        if self.return_errors {
            return Err(self.stub_error("write_file"));
        }

        // STUB: In real implementation, this would send file content via guest agent
        Ok(())
    }

    async fn read_file(&self, path: &str) -> Result<Vec<u8>, SwellError> {
        tracing::warn!(
            vm_id = %self.config.vm_id,
            path = %path,
            "FirecrackerSandbox: STUB - not reading from real microVM"
        );

        if self.return_errors {
            return Err(self.stub_error("read_file"));
        }

        // STUB: In real implementation, this would request file via guest agent
        Ok(b"STUB: file content would be here".to_vec())
    }

    async fn is_running(&self) -> bool {
        // STUB: In real implementation, check guest agent health
        // For stub, we return true if return_errors is false
        // (to maintain backward compatibility with tests)
        !self.return_errors
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_firecracker_sandbox_new() {
        let config = SandboxConfig {
            vm_id: "test-vm-1".to_string(),
            memory_mb: 512,
            vcpu_count: 4,
            timeout_secs: 600,
        };
        let sandbox = FirecrackerSandbox::new(config);
        assert_eq!(sandbox.id(), "test-vm-1");
    }

    #[tokio::test]
    async fn test_firecracker_sandbox_with_params() {
        let sandbox = FirecrackerSandbox::with_params("test-vm-2".to_string(), 1024);
        assert_eq!(sandbox.id(), "test-vm-2");
        assert_eq!(sandbox.config().memory_mb, 1024);
    }

    #[tokio::test]
    async fn test_firecracker_sandbox_start_stop() {
        let sandbox = FirecrackerSandbox::with_params("test-vm-3".to_string(), 256);
        assert!(sandbox.start().await.is_ok());
        assert!(sandbox.stop().await.is_ok());
    }

    #[tokio::test]
    async fn test_firecracker_sandbox_execute() {
        let sandbox = FirecrackerSandbox::with_params("test-vm-4".to_string(), 256);
        let cmd = SandboxCommand {
            command: "echo".to_string(),
            args: vec!["hello".to_string()],
            env: std::collections::HashMap::new(),
            working_dir: None,
            timeout_secs: 30,
        };
        let output = sandbox.execute(cmd).await.unwrap();
        assert_eq!(output.exit_code, 0);
        assert!(output.stdout.contains("STUB"));
    }

    #[tokio::test]
    async fn test_firecracker_sandbox_file_operations() {
        let sandbox = FirecrackerSandbox::with_params("test-vm-5".to_string(), 256);
        assert!(sandbox.write_file("/tmp/test.txt", b"hello").await.is_ok());
        let content = sandbox.read_file("/tmp/test.txt").await.unwrap();
        assert!(!content.is_empty());
    }

    #[tokio::test]
    async fn test_firecracker_sandbox_is_running() {
        let sandbox = FirecrackerSandbox::with_params("test-vm-6".to_string(), 256);
        assert!(sandbox.is_running().await);
    }
}
