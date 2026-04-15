//! Integration tests for REPL slash commands.
//!
//! These tests verify:
//! - VAL-SESS-009: CLI REPL mode accepts slash commands
//! - VAL-SESS-010: SlashCommandSpec registry allows registering custom commands

use std::io::Cursor;
use swell_cli::repl::{
    built_in_registry, format_help, run_repl_with_io, SlashCommandRegistry, SlashCommandSpec,
};

/// Test that built-in registry has all required commands
#[test]
fn test_built_in_registry_has_defaults() {
    let registry = built_in_registry();

    assert!(registry.contains("help"), "Should have /help command");
    assert!(registry.contains("status"), "Should have /status command");
    assert!(registry.contains("config"), "Should have /config command");
    assert!(registry.contains("exit"), "Should have /exit command");
}

/// Test that custom commands can be registered and invoked
#[test]
fn test_custom_command_registration_and_invocation() {
    let mut registry = built_in_registry();
    registry.register(SlashCommandSpec::new("greet", "Say hello", || {
        "Hello, user!".to_string()
    }));

    let cmd = registry.get("greet").expect("Command should exist");
    assert_eq!(cmd.name, "greet");
    assert_eq!(cmd.description, "Say hello");
    assert_eq!(cmd.execute(), "Hello, user!");
}

/// Test /help command output
#[test]
fn test_help_command_output() {
    let registry = built_in_registry();
    let input = "/help\n/exit\n";
    let mut input = Cursor::new(input.as_bytes());
    let mut output = Vec::new();

    run_repl_with_io(&mut input, &mut output, &registry).unwrap();

    let output_str = String::from_utf8(output).unwrap();
    assert!(
        output_str.contains("Available commands"),
        "Should show available commands"
    );
    assert!(output_str.contains("/help"), "Should list /help command");
    assert!(
        output_str.contains("/status"),
        "Should list /status command"
    );
    assert!(
        output_str.contains("/config"),
        "Should list /config command"
    );
}

/// Test /status command output
#[test]
fn test_status_command_output() {
    let registry = built_in_registry();
    let input = "/status\n/exit\n";
    let mut input = Cursor::new(input.as_bytes());
    let mut output = Vec::new();

    run_repl_with_io(&mut input, &mut output, &registry).unwrap();

    let output_str = String::from_utf8(output).unwrap();
    assert!(
        output_str.contains("No active task") || output_str.contains("status"),
        "Should show task status info"
    );
}

/// Test /config command output
#[test]
fn test_config_command_output() {
    let registry = built_in_registry();
    let input = "/config\n/exit\n";
    let mut input = Cursor::new(input.as_bytes());
    let mut output = Vec::new();

    run_repl_with_io(&mut input, &mut output, &registry).unwrap();

    let output_str = String::from_utf8(output).unwrap();
    assert!(
        output_str.contains("Config") || output_str.contains("socket"),
        "Should show config info"
    );
}

/// Test unrecognized command shows error with available commands
#[test]
fn test_unrecognized_command_error() {
    let registry = built_in_registry();
    let input = "/unknown\n/exit\n";
    let mut input = Cursor::new(input.as_bytes());
    let mut output = Vec::new();

    run_repl_with_io(&mut input, &mut output, &registry).unwrap();

    let output_str = String::from_utf8(output).unwrap();
    assert!(
        output_str.contains("Unknown command"),
        "Should show unknown command error"
    );
    assert!(
        output_str.contains("/help") || output_str.contains("/status"),
        "Should list available commands"
    );
}

/// Test that regular text is NOT treated as slash command
#[test]
fn test_regular_text_not_treated_as_slash_command() {
    let registry = built_in_registry();
    let input = "Hello world\n/exit\n";
    let mut input = Cursor::new(input.as_bytes());
    let mut output = Vec::new();

    run_repl_with_io(&mut input, &mut output, &registry).unwrap();

    let output_str = String::from_utf8(output).unwrap();
    assert!(
        output_str.contains("Would create task: Hello world"),
        "Regular text should be treated as task description"
    );
    assert!(
        !output_str.contains("Unknown command"),
        "Should NOT show unknown command error for regular text"
    );
}

/// Test custom command appears in /help output
#[test]
fn test_custom_command_in_help() {
    let mut registry = built_in_registry();
    registry.register(SlashCommandSpec::new("greet", "Say hello", || {
        "Hello!".to_string()
    }));

    let input = "/help\n/exit\n";
    let mut input = Cursor::new(input.as_bytes());
    let mut output = Vec::new();

    run_repl_with_io(&mut input, &mut output, &registry).unwrap();

    let output_str = String::from_utf8(output).unwrap();
    assert!(
        output_str.contains("/greet"),
        "Custom command /greet should appear in help"
    );
    assert!(
        output_str.contains("Say hello"),
        "Custom command description should appear in help"
    );
}

/// Test custom command invocation
#[test]
fn test_custom_command_invocation() {
    let mut registry = built_in_registry();
    registry.register(SlashCommandSpec::new("greet", "Say hello", || {
        "Hello, user!".to_string()
    }));

    let input = "/greet\n/exit\n";
    let mut input = Cursor::new(input.as_bytes());
    let mut output = Vec::new();

    run_repl_with_io(&mut input, &mut output, &registry).unwrap();

    let output_str = String::from_utf8(output).unwrap();
    assert!(
        output_str.contains("Hello, user!"),
        "Custom command /greet should execute and return 'Hello, user!'"
    );
}

/// Test command name is case-insensitive
#[test]
fn test_command_name_case_insensitive() {
    let mut registry = SlashCommandRegistry::new();
    registry.register(SlashCommandSpec::new("Greet", "Say hello", || {
        "Hello!".to_string()
    }));

    // Should be normalized to lowercase
    assert!(
        registry.get("greet").is_some(),
        "Lowercase lookup should work"
    );
    assert!(
        registry.get("GREET").is_some(),
        "Uppercase lookup should work"
    );
    assert!(
        registry.contains("greet"),
        "contains() should work with lowercase"
    );
}

/// Test format_help function
#[test]
fn test_format_help() {
    let registry = built_in_registry();
    let help = format_help(&registry);

    assert!(help.contains("/help"), "Should contain /help");
    assert!(help.contains("/status"), "Should contain /status");
    assert!(help.contains("/config"), "Should contain /config");
    assert!(help.contains("/exit"), "Should contain /exit");
}

/// Test command_names returns all registered names
#[test]
fn test_command_names() {
    let mut registry = built_in_registry();
    registry.register(SlashCommandSpec::new("custom", "A custom command", || {
        "custom".to_string()
    }));

    let names = registry.command_names();
    assert!(names.contains(&"help"), "Should contain help");
    assert!(names.contains(&"status"), "Should contain status");
    assert!(names.contains(&"custom"), "Should contain custom");
}

/// Test all_commands returns commands sorted by name
#[test]
fn test_all_commands_sorted() {
    let mut registry = built_in_registry();
    registry.register(SlashCommandSpec::new("zulu", "Last command", || {
        "zulu".to_string()
    }));
    registry.register(SlashCommandSpec::new("alpha", "First command", || {
        "alpha".to_string()
    }));

    let all = registry.all_commands();
    let names: Vec<_> = all.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["alpha", "config", "exit", "help", "status", "zulu"]
    );
}

/// Test empty command after / shows error
#[test]
fn test_empty_command_shows_error() {
    let registry = built_in_registry();
    let input = "//\n/exit\n";
    let mut input = Cursor::new(input.as_bytes());
    let mut output = Vec::new();

    run_repl_with_io(&mut input, &mut output, &registry).unwrap();

    let output_str = String::from_utf8(output).unwrap();
    assert!(
        output_str.contains("Empty command"),
        "Empty command should show helpful error"
    );
}
