//! swell-cli library crate
//!
//! This library provides the CLI functionality for the SWELL autonomous coding engine.

pub mod repl;

use std::time::Duration;
use thiserror::Error;

/// CLI-specific errors with user-friendly messages
#[derive(Error, Debug)]
pub enum CliError {
    #[error("Connection failed: {0}")]
    ConnectionFailed(String),

    #[error("Daemon not running. Start with: swell-daemon")]
    DaemonNotRunning,

    #[error("Socket not found at {0}")]
    SocketNotFound(String),

    #[error("Connection timeout after {0:?}")]
    ConnectionTimeout(Duration),

    #[error("Request timeout after {0:?}")]
    RequestTimeout(Duration),

    #[error("Invalid UUID format: {0}")]
    InvalidUuid(String),

    #[error("Invalid command: {0}")]
    InvalidCommand(String),

    #[error("Missing required argument: {0}")]
    MissingArgument(String),

    #[error("Server error: {0}")]
    ServerError(String),

    #[error("Unexpected response format")]
    UnexpectedResponse,

    #[error("JSON parse error: {0}")]
    JsonParseError(String),
}

impl CliError {
    /// Returns the appropriate exit code for this error type
    pub fn exit_code(&self) -> i32 {
        match self {
            CliError::ConnectionFailed(_) => 10,
            CliError::DaemonNotRunning => 10,
            CliError::SocketNotFound(_) => 10,
            CliError::ConnectionTimeout(_) => 11,
            CliError::RequestTimeout(_) => 11,
            CliError::InvalidUuid(_) => 2,
            CliError::InvalidCommand(_) => 2,
            CliError::MissingArgument(_) => 2,
            CliError::ServerError(_) => 1,
            CliError::UnexpectedResponse => 1,
            CliError::JsonParseError(_) => 1,
        }
    }

    /// Returns a short error code for scripts
    pub fn error_code(&self) -> &'static str {
        match self {
            CliError::ConnectionFailed(_) => "CONNECTION_FAILED",
            CliError::DaemonNotRunning => "DAEMON_NOT_RUNNING",
            CliError::SocketNotFound(_) => "SOCKET_NOT_FOUND",
            CliError::ConnectionTimeout(_) => "CONNECTION_TIMEOUT",
            CliError::RequestTimeout(_) => "REQUEST_TIMEOUT",
            CliError::InvalidUuid(_) => "INVALID_UUID",
            CliError::InvalidCommand(_) => "INVALID_COMMAND",
            CliError::MissingArgument(_) => "MISSING_ARGUMENT",
            CliError::ServerError(_) => "SERVER_ERROR",
            CliError::UnexpectedResponse => "UNEXPECTED_RESPONSE",
            CliError::JsonParseError(_) => "JSON_PARSE_ERROR",
        }
    }
}
