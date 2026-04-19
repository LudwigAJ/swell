pub mod commands;
pub mod dashboard;
pub mod error;
pub mod events;
pub mod server;

pub use dashboard::{DashboardEvent, DashboardState};
pub use error::{
    BudgetClass, ConfigError, DaemonError, DaemonErrorWire, GitError, LlmError,
    ValidationReason, WorktreeError,
};
pub use events::{EventEmitter, ImmutableEventLog};
pub use server::Daemon;
