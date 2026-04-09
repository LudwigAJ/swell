pub mod types;
pub mod error;
pub mod traits;

pub use types::*;
pub use error::SwellError;
pub use traits::*;

use tracing_subscriber::{fmt, prelude::*, EnvFilter};

/// Initialize tracing/logging for the crate
pub fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info"));

    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(filter)
        .init();
}
