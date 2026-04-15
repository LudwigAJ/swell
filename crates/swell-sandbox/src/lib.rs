//! swell-sandbox - Sandbox management (gVisor, Firecracker)
//!
//! This crate provides sandboxed execution environments using various isolation
//! technologies:
//! - **gVisor (runsc)** - ✅ **FULLY IMPLEMENTED** - User-space syscall interception via Docker
//! - **Firecracker microVMs** - ✅ **IMPLEMENTED** - Hardware virtualization via KVM
//!
//! ## Current Status
//!
//! ### gVisor (Recommended for containerized isolation)
//! ```rust,ignore
//! use swell_sandbox::GvisorSandbox;
//!
//! let config = GvisorConfig::default();
//! let sandbox = GvisorSandbox::new(config);
//! sandbox.start().await?;
//! ```
//!
//! ### Firecracker (For KVM-capable systems)
//! ```rust,ignore
//! use swell_sandbox::FirecrackerSandbox;
//!
//! let config = FirecrackerConfig {
//!     vm_id: "task-123".to_string(),
//!     memory_mb: 512,
//!     vcpu_count: 4,
//!     ..Default::default()
//! };
//! let sandbox = FirecrackerSandbox::new(config);
//! sandbox.start().await?;
//! ```
//!
//! Firecracker requires:
//! - Linux with KVM support (/dev/kvm)
//! - Firecracker binary installed
//! - Kernel and rootfs images
//!
//! **Startup time target: <125ms**

mod firecracker;
mod gvisor;
mod os_sandbox;

pub use firecracker::{FirecrackerConfig, FirecrackerSandbox};
pub use gvisor::{GvisorConfig, GvisorNetworkMode, GvisorSandbox};
pub use os_sandbox::{
    detect_available_sandbox, detect_available_sandbox_sync, FilesystemPermission, NetworkPolicy,
    OsSandboxConfig, SandboxAvailability, SandboxType, SeatbeltSandbox,
};
