//! REPL mode with slash commands for the SWELL CLI.
//!
//! REPL mode provides an interactive shell where users can:
//! - Use built-in slash commands: `/help`, `/status`, `/config`
//! - Register custom slash commands via `SlashCommandRegistry`
//! - Exit with `/exit` or Ctrl+C
//!
//! Unrecognized slash commands show a helpful error with available commands.

use crate::CliError;
use std::collections::HashMap;
use std::fmt;
use std::io::{self, BufRead, Write};
use std::sync::Arc;

/// A slash command specification with name, description, and handler.
#[derive(Clone)]
pub struct SlashCommandSpec {
    /// The command name (without leading `/`)
    pub name: String,
    /// Brief description shown in `/help`
    pub description: String,
    /// Handler function that produces output text (wrapped in Arc for Clone)
    handler: Arc<dyn Fn() -> String + Send + Sync>,
}

impl SlashCommandSpec {
    pub fn new(
        name: &str,
        description: &str,
        handler: impl Fn() -> String + Send + Sync + 'static,
    ) -> Self {
        Self {
            name: name.to_lowercase(),
            description: description.to_string(),
            handler: Arc::new(handler),
        }
    }

    /// Execute the command and return the output
    pub fn execute(&self) -> String {
        (self.handler)()
    }
}

impl fmt::Debug for SlashCommandSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SlashCommandSpec")
            .field("name", &self.name)
            .field("description", &self.description)
            .finish()
    }
}

/// Registry for slash commands.
///
/// # Example
///
/// ```
/// use swell_cli::repl::{SlashCommandRegistry, SlashCommandSpec};
///
/// let mut registry = SlashCommandRegistry::new();
/// registry.register(SlashCommandSpec::new("greet", "Say hello", || "Hello!".to_string()));
/// assert!(registry.get("greet").is_some());
/// ```
#[derive(Debug, Clone, Default)]
pub struct SlashCommandRegistry {
    commands: HashMap<String, SlashCommandSpec>,
}

impl SlashCommandRegistry {
    /// Create a new empty registry
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a slash command
    ///
    /// If a command with the same name already exists, it will be replaced.
    pub fn register(&mut self, spec: SlashCommandSpec) {
        self.commands.insert(spec.name.clone(), spec);
    }

    /// Get a command by name (case-insensitive)
    pub fn get(&self, name: &str) -> Option<&SlashCommandSpec> {
        self.commands.get(&name.to_lowercase())
    }

    /// List all registered command names
    #[allow(dead_code)]
    pub fn command_names(&self) -> Vec<&str> {
        self.commands.keys().map(|s| s.as_str()).collect()
    }

    /// Check if a command exists
    #[allow(dead_code)]
    pub fn contains(&self, name: &str) -> bool {
        self.commands.contains_key(&name.to_lowercase())
    }

    /// Get all commands sorted by name
    #[allow(dead_code)]
    pub fn all_commands(&self) -> Vec<&SlashCommandSpec> {
        let mut commands: Vec<_> = self.commands.values().collect();
        commands.sort_by_key(|c| &c.name);
        commands
    }
}

/// Build the default slash command registry with built-in commands.
pub fn built_in_registry() -> SlashCommandRegistry {
    let mut registry = SlashCommandRegistry::new();

    // /help - show available commands
    registry.register(SlashCommandSpec::new(
        "help",
        "Show available commands",
        || {
            "Available commands:\n  /help - Show this help message\n  /status - Show current task status\n  /config - Show active configuration\n  /exit - Exit REPL mode".to_string()
        },
    ));

    // /status - placeholder (would connect to daemon in real usage)
    registry.register(SlashCommandSpec::new(
        "status",
        "Show current task status",
        || "No active task.\nUse 'swell task <description>' to create one.".to_string(),
    ));

    // /config - placeholder (would load from config in real usage)
    registry.register(SlashCommandSpec::new(
        "config",
        "Show active configuration",
        || "Config: (not connected to daemon)\n  socket: /tmp/swell-daemon.sock".to_string(),
    ));

    // /exit - exit the REPL
    registry.register(SlashCommandSpec::new("exit", "Exit REPL mode", || {
        std::process::exit(0);
    }));

    registry
}

/// Format help output including custom commands
#[allow(dead_code)]
pub fn format_help(registry: &SlashCommandRegistry) -> String {
    let mut lines = vec!["Available commands:".to_string()];

    for cmd in registry.all_commands() {
        lines.push(format!("  /{} - {}", cmd.name, cmd.description));
    }

    lines.push("\nRegular text input is treated as a task description.".to_string());
    lines.join("\n")
}

/// Run the REPL loop with mock input/output for testing
pub fn run_repl_with_io<I: BufRead, O: Write>(
    input: &mut I,
    output: &mut O,
    registry: &SlashCommandRegistry,
) -> Result<(), CliError> {
    loop {
        // Print prompt
        writeln!(output, "\nswell> ").map_err(|e| CliError::ServerError(e.to_string()))?;
        output
            .flush()
            .map_err(|e| CliError::ServerError(e.to_string()))?;

        // Read line
        let mut line = String::new();
        match input.read_line(&mut line) {
            Ok(0) => break, // EOF
            Ok(_) => {}
            Err(e) => return Err(CliError::ServerError(e.to_string())),
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Check if it's a slash command
        if let Some(cmd_name) = trimmed.strip_prefix('/') {
            if cmd_name.is_empty() {
                writeln!(output, "Empty command. Use /help for available commands.")
                    .map_err(|e| CliError::ServerError(e.to_string()))?;
                continue;
            }

            if let Some(cmd) = registry.get(cmd_name) {
                let result = cmd.execute();
                writeln!(output, "{}", result).map_err(|e| CliError::ServerError(e.to_string()))?;

                // Check if /exit was run
                if cmd.name == "exit" {
                    break;
                }
            } else {
                // Unknown command - show helpful error
                let available = registry
                    .command_names()
                    .iter()
                    .map(|n| format!("/{}", n))
                    .collect::<Vec<_>>()
                    .join(", ");

                writeln!(
                    output,
                    "Unknown command: /{}\n  Available commands: {}",
                    cmd_name, available
                )
                .map_err(|e| CliError::ServerError(e.to_string()))?;
            }
        } else {
            // Regular text - treat as task description
            writeln!(output, "Would create task: {}", trimmed)
                .map_err(|e| CliError::ServerError(e.to_string()))?;
        }
    }

    Ok(())
}

/// Run the REPL loop using stdio
pub fn run_repl() -> Result<(), CliError> {
    let registry = built_in_registry();
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let mut output = io::stdout();
    run_repl_with_io(&mut input, &mut output, &registry)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_slash_command_spec_execute() {
        let spec = SlashCommandSpec::new("test", "A test command", || "output".to_string());
        assert_eq!(spec.execute(), "output");
        assert_eq!(spec.name, "test");
        assert_eq!(spec.description, "A test command");
    }

    #[test]
    fn test_slash_command_registry_register_and_get() {
        let mut registry = SlashCommandRegistry::new();
        assert!(registry.get("greet").is_none());

        registry.register(SlashCommandSpec::new("greet", "Say hello", || {
            "Hello!".to_string()
        }));

        let cmd = registry.get("greet").expect("Command should exist");
        assert_eq!(cmd.name, "greet");
        assert_eq!(cmd.description, "Say hello");
        assert_eq!(cmd.execute(), "Hello!");
    }

    #[test]
    fn test_slash_command_registry_case_insensitive() {
        let mut registry = SlashCommandRegistry::new();
        registry.register(SlashCommandSpec::new("Greet", "Say hello", || {
            "Hello!".to_string()
        }));

        // Should be normalized to lowercase
        assert!(registry.get("greet").is_some());
        assert!(registry.get("GREET").is_some());
        assert!(registry.contains("greet"));
    }

    #[test]
    fn test_built_in_registry_has_defaults() {
        let registry = built_in_registry();

        assert!(registry.contains("help"));
        assert!(registry.contains("status"));
        assert!(registry.contains("config"));
        assert!(registry.contains("exit"));
    }

    #[test]
    fn test_repl_help_command() {
        let mut registry = built_in_registry();
        registry.register(SlashCommandSpec::new("greet", "Say hello", || {
            "Hello!".to_string()
        }));

        let input = "/help\n/exit\n";
        let mut input = Cursor::new(input.as_bytes());
        let mut output = Vec::new();

        run_repl_with_io(&mut input, &mut output, &registry).unwrap();

        let output_str = String::from_utf8(output).unwrap();
        assert!(output_str.contains("Available commands"));
        assert!(output_str.contains("/help"));
        assert!(output_str.contains("/greet"));
    }

    #[test]
    fn test_repl_unknown_command_shows_error() {
        let registry = built_in_registry();
        let input = "/unknown\n/exit\n";
        let mut input = Cursor::new(input.as_bytes());
        let mut output = Vec::new();

        run_repl_with_io(&mut input, &mut output, &registry).unwrap();

        let output_str = String::from_utf8(output).unwrap();
        assert!(output_str.contains("Unknown command"));
        assert!(output_str.contains("/help"));
        assert!(output_str.contains("/status"));
        assert!(output_str.contains("/config"));
    }

    #[test]
    fn test_repl_regular_text_not_treated_as_slash_command() {
        let registry = built_in_registry();
        let input = "Hello world\n/exit\n";
        let mut input = Cursor::new(input.as_bytes());
        let mut output = Vec::new();

        run_repl_with_io(&mut input, &mut output, &registry).unwrap();

        let output_str = String::from_utf8(output).unwrap();
        assert!(output_str.contains("Would create task: Hello world"));
        assert!(!output_str.contains("Unknown command"));
    }

    #[test]
    fn test_repl_custom_command() {
        let mut registry = built_in_registry();
        registry.register(SlashCommandSpec::new("greet", "Say hello", || {
            "Hello, user!".to_string()
        }));

        let input = "/greet\n/exit\n";
        let mut input = Cursor::new(input.as_bytes());
        let mut output = Vec::new();

        run_repl_with_io(&mut input, &mut output, &registry).unwrap();

        let output_str = String::from_utf8(output).unwrap();
        assert!(output_str.contains("Hello, user!"));
    }

    #[test]
    fn test_format_help() {
        let registry = built_in_registry();
        let help = format_help(&registry);

        assert!(help.contains("/help"));
        assert!(help.contains("/status"));
        assert!(help.contains("/config"));
        assert!(help.contains("/exit"));
    }

    #[test]
    fn test_command_names() {
        let mut registry = built_in_registry();
        registry.register(SlashCommandSpec::new("custom", "A custom command", || {
            "custom".to_string()
        }));

        let names = registry.command_names();
        assert!(names.contains(&"help"));
        assert!(names.contains(&"status"));
        assert!(names.contains(&"custom"));
    }
}
