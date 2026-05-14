//! Daemon-bootstrap loader for `.swell/triggers.json`.
//!
//! Implements the config-driven half of the PR `02` integration slice from
//! `plan/flow_integration_plan/02_trigger_registry.md`: the daemon reads
//! `.swell/triggers.json` at startup, resolves named triggers against a
//! [`TriggerFactoryRegistry`], and installs the resulting [`Trigger`]
//! instances on the live [`Orchestrator`] via [`Orchestrator::install_trigger`].
//!
//! Forward-compat semantics (per the plan):
//!
//! - Unknown trigger names *warn* — they don't error. This lets a future
//!   release add `git_commit` to the config without breaking daemons running
//!   an older binary that doesn't yet ship that factory.
//! - Unknown stage strings inside an entry *warn*. Known stages on the same
//!   entry are still honored.
//! - Entries with `"enabled": false` are silently skipped.
//! - A missing `.swell/triggers.json` file is not an error — the daemon
//!   boots with an empty registry, preserving the legacy linear pipeline.
//!
//! No built-in factories are registered yet; the three built-ins
//! (`GitCommitTrigger`, `ValidatorGateTrigger`, `MemoryWriteTrigger`) land
//! in PRs `07` / `08` / `09`. Today the loader is exercised by tests that
//! register their own factories — proving the pipe is open before the
//! built-ins arrive.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use serde::Deserialize;
use tracing::{info, warn};

use crate::triggers::{Stage, Trigger};

/// Trigger entry as deserialized from `.swell/triggers.json`.
#[derive(Debug, Clone, Deserialize)]
pub struct TriggerEntry {
    /// Stage names this trigger should fire on. Unknown strings warn.
    #[serde(default)]
    pub stages: Vec<String>,
    /// Disable a trigger without removing the config entry.
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// Optional opaque config blob a factory can interpret.
    #[serde(default)]
    pub config: serde_json::Value,
}

fn default_enabled() -> bool {
    true
}

/// Top-level shape of `.swell/triggers.json`: a name-keyed map of entries.
pub type TriggerConfig = HashMap<String, TriggerEntry>;

/// Function that builds a [`Trigger`] from the resolved stage list and
/// per-entry config blob. Returning `None` means "config is malformed for
/// this trigger" and is treated as a warning by the loader (skipped, not
/// fatal) so a partially broken config can't strand the daemon.
pub type TriggerFactoryFn =
    dyn Fn(&[Stage], &serde_json::Value) -> Option<Arc<dyn Trigger>> + Send + Sync;

/// Registry of name → factory, populated at daemon construction by the
/// crates that own the built-in triggers. Empty by default.
#[derive(Default)]
pub struct TriggerFactoryRegistry {
    factories: HashMap<String, Arc<TriggerFactoryFn>>,
}

impl TriggerFactoryRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register<F>(&mut self, name: impl Into<String>, factory: F)
    where
        F: Fn(&[Stage], &serde_json::Value) -> Option<Arc<dyn Trigger>> + Send + Sync + 'static,
    {
        self.factories.insert(name.into(), Arc::new(factory));
    }

    pub fn known_names(&self) -> Vec<&str> {
        self.factories.keys().map(String::as_str).collect()
    }

    fn get(&self, name: &str) -> Option<&Arc<TriggerFactoryFn>> {
        self.factories.get(name)
    }
}

/// Result of a single-shot config resolve. Triggers in `built` are ready to
/// install in registration order; `warnings` carries operator-visible
/// diagnostics surfaced via `tracing::warn` and returned for tests.
#[derive(Default)]
pub struct LoadedTriggers {
    pub built: Vec<Arc<dyn Trigger>>,
    pub warnings: Vec<String>,
}

impl std::fmt::Debug for LoadedTriggers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names: Vec<&'static str> = self.built.iter().map(|t| t.name()).collect();
        f.debug_struct("LoadedTriggers")
            .field("built", &names)
            .field("warnings", &self.warnings)
            .finish()
    }
}

/// Parse a `Stage` from its `serde` representation. Unknown strings are
/// reported as `Err(name)` so the caller can attach the trigger context to
/// the warning.
fn parse_stage(s: &str) -> Result<Stage, &str> {
    // Round-trip through serde so the canonical string set is the same
    // strings the registry serializes to. Avoids drift with the enum.
    let json = format!("\"{}\"", s);
    serde_json::from_str::<Stage>(&json).map_err(|_| s)
}

/// Build triggers from a parsed [`TriggerConfig`] and a factory registry.
///
/// Pure function; performs no I/O. See [`load_triggers_from_dir`] for the
/// daemon-bootstrap entry point that reads `.swell/triggers.json`.
pub fn build_triggers(
    config: &TriggerConfig,
    factories: &TriggerFactoryRegistry,
) -> LoadedTriggers {
    let mut out = LoadedTriggers::default();

    // Stable iteration order: sort by name. JSON object iteration order is
    // not guaranteed across runs and registration order is fire order, so
    // sorting keeps daemon restarts deterministic.
    let mut names: Vec<&String> = config.keys().collect();
    names.sort();

    for name in names {
        let entry = &config[name];
        if !entry.enabled {
            info!(trigger = %name, "trigger disabled in config; skipping");
            continue;
        }

        let mut stages = Vec::with_capacity(entry.stages.len());
        for raw in &entry.stages {
            match parse_stage(raw) {
                Ok(stage) => stages.push(stage),
                Err(unknown) => {
                    let msg = format!(
                        "trigger '{name}' references unknown stage '{unknown}'; ignoring stage"
                    );
                    warn!("{msg}");
                    out.warnings.push(msg);
                }
            }
        }

        if stages.is_empty() {
            let msg = format!("trigger '{name}' has no recognized stages; skipping");
            warn!("{msg}");
            out.warnings.push(msg);
            continue;
        }

        let Some(factory) = factories.get(name) else {
            let msg = format!(
                "trigger '{name}' has no registered factory; skipping (forward-compat warning)"
            );
            warn!("{msg}");
            out.warnings.push(msg);
            continue;
        };

        match factory(&stages, &entry.config) {
            Some(trigger) => {
                info!(
                    trigger = %name,
                    stages = ?stages,
                    "installing trigger from .swell/triggers.json"
                );
                out.built.push(trigger);
            }
            None => {
                let msg = format!("trigger '{name}' factory rejected its config; skipping");
                warn!("{msg}");
                out.warnings.push(msg);
            }
        }
    }

    out
}

/// Read `.swell/triggers.json` from `swell_dir` and resolve it against
/// `factories`. A missing file returns an empty [`LoadedTriggers`]; a
/// malformed file is reported as a warning and yields an empty result —
/// never a hard failure, because the daemon would rather boot with no
/// triggers than refuse to start.
pub fn load_triggers_from_dir(
    swell_dir: &Path,
    factories: &TriggerFactoryRegistry,
) -> LoadedTriggers {
    let path = swell_dir.join("triggers.json");
    if !path.exists() {
        return LoadedTriggers::default();
    }

    let raw = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) => {
            let msg = format!(
                "failed to read {} ({e}); booting with no triggers",
                path.display()
            );
            warn!("{msg}");
            return LoadedTriggers {
                built: Vec::new(),
                warnings: vec![msg],
            };
        }
    };

    let config: TriggerConfig = match serde_json::from_str(&raw) {
        Ok(c) => c,
        Err(e) => {
            let msg = format!(
                "failed to parse {} ({e}); booting with no triggers",
                path.display()
            );
            warn!("{msg}");
            return LoadedTriggers {
                built: Vec::new(),
                warnings: vec![msg],
            };
        }
    };

    build_triggers(&config, factories)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::triggers::{TriggerContext, TriggerOutcome};
    use async_trait::async_trait;

    struct NoopTrigger {
        name: &'static str,
        stages: Vec<Stage>,
    }

    #[async_trait]
    impl Trigger for NoopTrigger {
        fn name(&self) -> &'static str {
            self.name
        }
        fn stages(&self) -> &'static [Stage] {
            // Leak once for &'static. Test-only trigger; allocation is
            // bounded by test count.
            Box::leak(self.stages.clone().into_boxed_slice())
        }
        async fn run(&self, _ctx: &TriggerContext) -> TriggerOutcome {
            TriggerOutcome::Continue
        }
    }

    fn factories_with_noop() -> TriggerFactoryRegistry {
        let mut reg = TriggerFactoryRegistry::new();
        reg.register("noop", |stages, _cfg| {
            Some(Arc::new(NoopTrigger {
                name: "noop",
                stages: stages.to_vec(),
            }))
        });
        reg
    }

    #[test]
    fn missing_file_is_empty_not_error() {
        let tmp = tempfile::TempDir::new().unwrap();
        let loaded = load_triggers_from_dir(tmp.path(), &TriggerFactoryRegistry::new());
        assert!(loaded.built.is_empty());
        assert!(loaded.warnings.is_empty());
    }

    #[test]
    fn unknown_trigger_name_warns_not_errors() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("triggers.json"),
            r#"{ "git_commit": { "stages": ["AfterTask"], "enabled": true } }"#,
        )
        .unwrap();

        let loaded = load_triggers_from_dir(tmp.path(), &TriggerFactoryRegistry::new());
        assert!(loaded.built.is_empty());
        assert_eq!(loaded.warnings.len(), 1);
        assert!(loaded.warnings[0].contains("git_commit"));
    }

    #[test]
    fn unknown_stage_warns_other_stages_kept() {
        let cfg: TriggerConfig = serde_json::from_str(
            r#"{ "noop": { "stages": ["AfterTask", "WhenItRains"], "enabled": true } }"#,
        )
        .unwrap();

        let loaded = build_triggers(&cfg, &factories_with_noop());
        assert_eq!(loaded.built.len(), 1);
        assert!(loaded
            .warnings
            .iter()
            .any(|w| w.contains("WhenItRains") && w.contains("noop")));
    }

    #[test]
    fn entry_with_no_recognized_stages_skips_with_warning() {
        let cfg: TriggerConfig =
            serde_json::from_str(r#"{ "noop": { "stages": ["WhenItRains"], "enabled": true } }"#)
                .unwrap();

        let loaded = build_triggers(&cfg, &factories_with_noop());
        assert!(loaded.built.is_empty());
        assert!(loaded.warnings.iter().any(|w| w.contains("noop")));
    }

    #[test]
    fn disabled_entry_skipped_silently() {
        let cfg: TriggerConfig =
            serde_json::from_str(r#"{ "noop": { "stages": ["AfterTask"], "enabled": false } }"#)
                .unwrap();

        let loaded = build_triggers(&cfg, &factories_with_noop());
        assert!(loaded.built.is_empty());
        assert!(loaded.warnings.is_empty());
    }

    #[test]
    fn malformed_json_warns_returns_empty() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::write(tmp.path().join("triggers.json"), "{ not json").unwrap();
        let loaded = load_triggers_from_dir(tmp.path(), &TriggerFactoryRegistry::new());
        assert!(loaded.built.is_empty());
        assert_eq!(loaded.warnings.len(), 1);
    }
}
