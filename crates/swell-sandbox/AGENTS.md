# swell-sandbox AGENTS.md

## Purpose

`swell-sandbox` provides sandboxed execution environments for the SWELL autonomous coding engine. It offers multiple isolation technologies including gVisor (user-space syscall interception) and Firecracker microVMs (hardware virtualization), enabling secure tool execution in isolated containers.

This crate handles:
- **gVisor (runsc)** — User-space syscall interception via Docker container runtime
- **Firecracker microVMs** — Hardware virtualization via KVM for production workloads
- **Process isolation** — Token privilege-based isolation on macOS/Windows
- **Network configuration** — gVisor network modes (inet, filtered, none)
- **Resource limits** — Memory, vCPU, and startup time constraints

**Depends on:** `swell-core` (for `Sandbox` trait, `SwellError`)

## Public API

### gVisor Sandbox

```rust
pub struct GvisorConfig {
    pub container_id: String,
    pub network_mode: GvisorNetworkMode,
    pub auto_start: bool,
    pub startup_timeout_ms: u64,
}

pub enum GvisorNetworkMode {
    /// No networking - completely isolated
    None,
    /// Filtered networking via iptables
    Filtered,
    /// Full networking
    Inet,
}

pub struct GvisorSandbox {
    config: GvisorConfig,
    state: GvisorState,
}

pub enum GvisorState {
    Created,
    Starting,
    Running,
    Stopping,
    Stopped,
}

impl GvisorSandbox {
    pub fn new(config: GvisorConfig) -> Self;
    pub async fn start(&self) -> Result<(), SwellError>;
    pub async fn stop(&self) -> Result<(), SwellError>;
    pub async fn execute(&self, cmd: Vec<String>) -> Result<SandboxOutput, SwellError>;
    pub fn is_running(&self) -> bool;
}
```

### Firecracker Sandbox

```rust
pub struct FirecrackerConfig {
    /// Unique identifier for this VM
    pub vm_id: String,
    /// Memory in MB (default: 512)
    pub memory_mb: u64,
    /// Number of vCPUs (default: 4)
    pub vcpu_count: u8,
    /// Path to kernel image (default: built-in)
    pub kernel_image: Option<PathBuf>,
    /// Path to rootfs image (default: built-in)
    pub rootfs_image: Option<PathBuf>,
    /// Enable direct kernel boot (skip initrd)
    pub direct_kernel_boot: bool,
    /// Jailer directory (default: /var/lib/firecracker)
    pub jailer_directory: PathBuf,
    /// Socket path for API (default: /tmp/firecracker-{vm_id}.socket)
    pub socket_path: Option<PathBuf>,
    /// Enable vsock (default: false)
    pub vsock: bool,
    /// Enable snapshotting (default: false)
    pub enable_snapshot: bool,
    /// OpenTelemetry service name for tracing
    pub otel_service_name: Option<String>,
}

pub struct FirecrackerSandbox {
    config: FirecrackerConfig,
    state: FirecrackerState,
}

pub enum FirecrackerState {
    Created,
    Starting,
    Running,
    Stopping,
    Stopped,
}

impl FirecrackerSandbox {
    pub fn new(config: FirecrackerConfig) -> Self;
    pub async fn start(&self) -> Result<(), SwellError>;
    pub async fn stop(&self) -> Result<(), SwellError>;
    pub async fn execute(&self, cmd: Vec<String>) -> Result<SandboxOutput, SwellError>;
    pub fn is_running(&self) -> bool;
}
```

### Sandbox Output

```rust
pub struct SandboxOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub duration_ms: u64,
}
```

### Key Re-exports

```rust
pub use gvisor::{GvisorConfig, GvisorNetworkMode, GvisorSandbox};
pub use firecracker::{FirecrackerConfig, FirecrackerSandbox};
```

## Architecture

```
┌─────────────────────────────────────────────────────────────────────┐
│                       swell-sandbox                                │
├─────────────────────────────────────────────────────────────────────┤
│                                                                      │
│  ┌──────────────────────────────────────────────────────────────┐   │
│  │                     Sandbox Traits                            │   │
│  │  (from swell-core: Sandbox trait with start/stop/execute)     │   │
│  └──────────────────────────────────────────────────────────────┘   │
│                              │                                      │
│         ┌────────────────────┼────────────────────┐                 │
│         ▼                    ▼                    ▼                 │
│  ┌─────────────┐      ┌─────────────┐      ┌─────────────┐         │
│  │   gVisor    │      │ Firecracker │      │  Process    │         │
│  │  (runsc)    │      │ (microVM)   │      │  (stub)     │         │
│  │             │      │             │      │             │         │
│  │ User-space  │      │ Hardware    │      │ Token priv  │         │
│  │ syscall     │      │ KVM virt    │      │ isolation   │         │
│  │ interception│      │             │      │             │         │
│  └─────────────┘      └─────────────┘      └─────────────┘         │
│                                                                      │
│  ┌──────────────────────────────────────────────────────────────┐   │
│  │                    Isolation Technologies                    │   │
│  │                                                                  │   │
│  │  gVisor (runsc)          Firecracker          macOS/Windows    │   │
│  │  ├─ No networking       ├─ KVM hypervisor    ├─ Token privileges│  │
│  │  ├─ Filtered (iptables) │  ├─ <125ms startup  │  └─ Process isolation│ │
│  │  └─ Full networking      │  ├─ <5MiB memory    │                     │   │
│  │                         │  └─ Snapshotting   │                     │   │
│  │                         └─────────────────────┘                     │   │
│  └──────────────────────────────────────────────────────────────┘   │
│                                                                      │
└─────────────────────────────────────────────────────────────────────┘
                           │ used by
                           ▼
              ┌────────────────────────┐
              │    swell-tools         │
              │  (shell execution)     │
              └────────────────────────┘
```

**Key modules:**
- `lib.rs` — Main exports and trait implementations
- `gvisor.rs` — gVisor sandbox implementation (user-space syscall interception)
- `firecracker.rs` — Firecracker microVM implementation (hardware virtualization)

**Platform support:**
- Linux: gVisor (runsc) and Firecracker (KVM)
- macOS: Process isolation via token privileges (stub for MVP)
- Windows: Process isolation (stub for MVP)

**Startup time targets:**
- gVisor: <125ms
- Firecracker: <125ms

**Concurrency:** All types are `Send + Sync`.

## Testing

```bash
# Run tests for swell-sandbox
cargo test -p swell-sandbox -- --test-threads=4

# Run with logging
RUST_LOG=debug cargo test -p swell-sandbox

# Run gVisor tests (requires runsc installed)
cargo test -p swell-sandbox -- gvisor

# Run Firecracker tests (requires KVM + firecracker binary)
cargo test -p swell-sandbox -- firecracker
```

**Test patterns:**
- Unit tests for state machine transitions
- Configuration validation tests
- Platform detection tests
- Mock-based tests for CI environments

**Mock patterns:**
```rust
#[tokio::test]
async fn test_gvisor_config_default() {
    let config = GvisorConfig::default();
    assert_eq!(config.network_mode, GvisorNetworkMode::None);
    assert!(config.auto_start);
}

#[tokio::test]
async fn test_firecracker_config_default() {
    let config = FirecrackerConfig::default();
    assert_eq!(config.memory_mb, 512);
    assert_eq!(config.vcpu_count, 4);
    assert!(!config.direct_kernel_boot);
}
```

## Dependencies

```toml
# swell-sandbox/Cargo.toml
[dependencies]
swell-core = { path = "../swell-core" }
tokio.workspace = true
serde.workspace = true
serde_json.workspace = true
thiserror.workspace = true
anyhow.workspace = true
tracing.workspace = true
uuid.workspace = true
chrono.workspace = true
async-trait.workspace = true

[target.'cfg(target_os = "linux")'.dependencies]
libc = "0.2"
```

**Platform-specific dependencies:**
- Linux: `libc` for KVM ioctl calls (Firecracker)
- macOS/Windows: No additional dependencies (stub implementations)
