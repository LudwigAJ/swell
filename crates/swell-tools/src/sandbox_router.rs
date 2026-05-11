//! Router for the lower-level `swell-sandbox` backends.
//!
//! `ToolExecutor` still owns the final shell execution decision, but this
//! router makes the previously orphaned `swell-sandbox` crate part of the
//! production sandbox path. It prefers stronger isolation when fully
//! provisioned, then falls back to the OS sandbox path that already exists in
//! `swell-tools`.

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};

use swell_core::SwellError;
use swell_sandbox::{FirecrackerConfig, FirecrackerSandbox, GvisorSandbox};

static ROUTE_INVOCATIONS: AtomicUsize = AtomicUsize::new(0);

/// Sandbox backend selected for a shell command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxBackend {
    /// Command is a read-only host allowlist fast path.
    HostAllowlist,
    /// Firecracker is fully provisioned.
    Firecracker,
    /// gVisor/runsc is available through Docker.
    Gvisor,
    /// Use the process-level OS sandbox fallback.
    OsSandbox,
    /// No sandbox backend is available.
    HostFallback,
}

/// Result of routing a sandbox request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxRoute {
    pub backend: SandboxBackend,
    pub reason: String,
}

impl SandboxRoute {
    fn new(backend: SandboxBackend, reason: impl Into<String>) -> Self {
        Self {
            backend,
            reason: reason.into(),
        }
    }
}

/// Routes shell commands to the strongest available sandbox backend.
#[derive(Debug, Clone, Default)]
pub struct SandboxRouter;

impl SandboxRouter {
    pub fn new() -> Self {
        Self
    }

    pub async fn route(&self) -> SandboxRoute {
        ROUTE_INVOCATIONS.fetch_add(1, Ordering::SeqCst);

        match self.firecracker_route() {
            Ok(Some(route)) => return route,
            Ok(None) => {}
            Err(e) => {
                tracing::debug!(error = %e, "Firecracker sandbox probe failed");
            }
        }

        match GvisorSandbox::is_runsc_available().await {
            Ok(true) => {
                return SandboxRoute::new(
                    SandboxBackend::Gvisor,
                    "gVisor runsc is registered with Docker",
                );
            }
            Ok(false) => {}
            Err(e) => {
                tracing::debug!(error = %e, "gVisor sandbox probe failed");
            }
        }

        let availability = swell_sandbox::detect_available_sandbox_sync();
        if availability.is_available {
            return SandboxRoute::new(SandboxBackend::OsSandbox, availability.description);
        }

        SandboxRoute::new(
            SandboxBackend::HostFallback,
            "no swell-sandbox backend available; falling back to existing tool execution",
        )
    }

    /// Route a specific shell command.
    ///
    /// Read-only allowlisted commands bypass sandbox probing entirely. Mutating
    /// or unknown commands still route to the strongest available backend.
    pub async fn route_for_command(&self, cmd: &str, args: &[String]) -> SandboxRoute {
        if Self::is_host_allowlisted_command(cmd, args) {
            return SandboxRoute::new(
                SandboxBackend::HostAllowlist,
                "read-only host allowlist fast path",
            );
        }

        self.route().await
    }

    /// Return true when a shell command is safe to run on the host fast path.
    ///
    /// This intentionally accepts only narrow read-only forms and rejects
    /// absolute paths, parent traversal, home expansion, and non-status git.
    pub fn is_host_allowlisted_command(cmd: &str, args: &[String]) -> bool {
        let command = Path::new(cmd)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(cmd);

        match command {
            "ls" | "cat" | "rg" => args_are_workspace_relative(args),
            "git" => {
                args.first().is_some_and(|arg| arg == "status")
                    && args_are_workspace_relative(&args[1..])
            }
            "cargo" => {
                args.first().is_some_and(|arg| arg == "check")
                    && args.iter().any(|arg| {
                        arg == "--message-format=json"
                            || arg
                                .strip_prefix("--message-format=")
                                .is_some_and(|format| format == "json")
                    })
                    && args_are_workspace_relative(&args[1..])
            }
            _ => false,
        }
    }

    fn firecracker_route(&self) -> Result<Option<SandboxRoute>, SwellError> {
        if !FirecrackerSandbox::is_kvm_available()? {
            return Ok(None);
        }

        let config = FirecrackerConfig::default();
        if !Path::new(&config.firecracker_path).exists() {
            return Ok(None);
        }
        if !config.kernel_image.exists() || !config.rootfs_image.exists() {
            return Ok(None);
        }

        let sandbox = FirecrackerSandbox::new(config);
        if sandbox.is_firecracker_available()? {
            Ok(Some(SandboxRoute::new(
                SandboxBackend::Firecracker,
                "Firecracker binary, KVM, kernel, and rootfs are available",
            )))
        } else {
            Ok(None)
        }
    }
}

fn args_are_workspace_relative(args: &[String]) -> bool {
    args.iter().all(|arg| {
        if arg == "--" || arg.starts_with('-') {
            return true;
        }
        let path = Path::new(arg);
        !path.is_absolute()
            && !arg.starts_with('~')
            && !path
                .components()
                .any(|component| matches!(component, std::path::Component::ParentDir))
    })
}

/// Reset the sandbox router probe count.
///
/// This is intentionally lightweight production code rather than a mock-only
/// test hook: wiring tests use it to prove the daemon path crossed the router.
pub fn reset_sandbox_router_probe_count() {
    ROUTE_INVOCATIONS.store(0, Ordering::SeqCst);
}

/// Return how many times the sandbox router has been consulted.
pub fn sandbox_router_probe_count() -> usize {
    ROUTE_INVOCATIONS.load(Ordering::SeqCst)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn router_always_returns_a_route() {
        let route = SandboxRouter::new().route().await;
        assert!(
            matches!(
                route.backend,
                SandboxBackend::HostAllowlist
                    | SandboxBackend::Firecracker
                    | SandboxBackend::Gvisor
                    | SandboxBackend::OsSandbox
                    | SandboxBackend::HostFallback
            ),
            "unexpected route: {route:?}"
        );
        assert!(!route.reason.is_empty());
    }

    #[tokio::test]
    async fn host_allowlist_bypasses_sandbox_probe_for_trivial_reads() {
        reset_sandbox_router_probe_count();

        let route = SandboxRouter::new()
            .route_for_command(
                "cargo",
                &[
                    "check".to_string(),
                    "--message-format=json".to_string(),
                    "--quiet".to_string(),
                ],
            )
            .await;

        assert_eq!(route.backend, SandboxBackend::HostAllowlist);
        assert_eq!(
            sandbox_router_probe_count(),
            0,
            "host allowlisted commands should bypass backend probes"
        );
    }

    #[test]
    fn host_allowlist_rejects_mutating_or_escape_forms() {
        assert!(SandboxRouter::is_host_allowlisted_command(
            "git",
            &["status".to_string(), "--short".to_string()]
        ));
        assert!(!SandboxRouter::is_host_allowlisted_command(
            "git",
            &["checkout".to_string(), "main".to_string()]
        ));
        assert!(!SandboxRouter::is_host_allowlisted_command(
            "cat",
            &["/etc/passwd".to_string()]
        ));
        assert!(!SandboxRouter::is_host_allowlisted_command(
            "rg",
            &["needle".to_string(), "../other".to_string()]
        ));
    }
}
