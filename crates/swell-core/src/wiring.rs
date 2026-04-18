//! Wiring report trait for runtime diagnostics of subsystem wiring state.
//!
//! This module provides a common interface for all orchestrator subsystems to
//! report their wiring status. This enables runtime diagnostics and debugging
//! of connectivity issues between components.
//!
//! # Example
//!
//! ```ignore
//! use swell_core::wiring::{WiringReport, WiringState};
//!
//! impl WiringReport for MySubsystem {
//!     fn name(&self) -> &'static str { "MySubsystem" }
//!     fn identity(&self) -> String { format!("{:?}", self) }
//!     fn state(&self) -> WiringState { WiringState::Enabled }
//! }
//! ```

use serde::{Deserialize, Serialize};

/// Represents the wiring state of a subsystem.
///
/// Variants indicate progressively less functional states:
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum WiringState {
    /// Subsystem is fully wired and operational.
    Enabled,

    /// Subsystem is wired but operating in a degraded mode.
    ///
    /// The string provides context about what is degraded.
    Degraded(String),

    /// Subsystem is not wired (completely disabled).
    ///
    /// The string provides the reason (e.g., "Tier 2.1 not wired").
    Disabled(String),
}

impl WiringState {
    /// Returns true if the state is [`WiringState::Enabled`].
    pub fn is_enabled(&self) -> bool {
        matches!(self, WiringState::Enabled)
    }

    /// Returns true if the state is [`WiringState::Disabled`].
    pub fn is_disabled(&self) -> bool {
        matches!(self, WiringState::Disabled(_))
    }

    /// Returns true if the state is [`WiringState::Degraded`].
    pub fn is_degraded(&self) -> bool {
        matches!(self, WiringState::Degraded(_))
    }

    /// Returns the reason if the state is [`WiringState::Disabled`] or [`WiringState::Degraded`].
    pub fn reason(&self) -> Option<&str> {
        match self {
            WiringState::Enabled => None,
            WiringState::Degraded(s) | WiringState::Disabled(s) => Some(s),
        }
    }
}

impl std::fmt::Display for WiringState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WiringState::Enabled => write!(f, "Enabled"),
            WiringState::Degraded(reason) => write!(f, "Degraded({reason})"),
            WiringState::Disabled(reason) => write!(f, "Disabled({reason})"),
        }
    }
}

/// Trait for reporting wiring state of a subsystem.
///
/// All wired orchestrator subsystems should implement this trait
/// to enable runtime diagnostics of their connectivity.
pub trait WiringReport: Send + Sync {
    /// Returns the name of the subsystem.
    fn name(&self) -> &'static str;

    /// Returns a unique identifier for this instance.
    ///
    /// This is used for debugging and log correlation.
    fn identity(&self) -> String;

    /// Returns the current wiring state of the subsystem.
    fn state(&self) -> WiringState;
}
