use std::io::{self, Write};
use std::time::Duration;
use swell_cli::CliError;
use swell_cli::repl;
use swell_core::{CliCommand, DaemonEvent, Task};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::signal::unix::{signal, SignalKind};
use tokio::time::timeout;
use uuid::Uuid;

/// Print a structured error to stderr
fn print_error(error: &CliError) {
    eprintln!("error: {}", error);
    eprintln!("  Code: {}", error.error_code());
}

/// Default connection timeout
const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// Default request timeout
const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Prompt user for confirmation and return true if they confirm with 'y' or 'yes' (case-insensitive)
fn confirm(prompt: &str) -> bool {
    print!("{} [y/N] ", prompt);
    if let Err(e) = io::stdout().flush() {
        eprintln!("Warning: flush failed: {}", e);
        return false;
    }

    let mut input = String::new();
    match io::stdin().read_line(&mut input) {
        Ok(_) => {
            let trimmed = input.trim().to_lowercase();
            trimmed == "y" || trimmed == "yes"
        }
        Err(e) => {
            eprintln!("Warning: read_line failed: {}", e);
            false
        }
    }
}

#[tokio::main]
async fn main() {
    // Simple CLI parsing for MVP
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        usage();
        std::process::exit(0);
    }

    let socket_path =
        std::env::var("SWELL_SOCKET").unwrap_or_else(|_| "/tmp/swell-daemon.sock".to_string());

    let result = match args[1].as_str() {
        "task" => {
            if args.len() < 3 {
                Err(CliError::MissingArgument("description".to_string()))
            } else {
                let description = args[2..].join(" ");
                let cmd = CliCommand::TaskCreate { description };
                send_command(&socket_path, cmd).await
            }
        }
        "list" => {
            let json_output = args.contains(&"--json".to_string());
            list_tasks(&socket_path, json_output).await
        }
        "watch" => {
            if args.len() < 3 {
                Err(CliError::MissingArgument("task-id".to_string()))
            } else {
                match Uuid::parse_str(&args[2]) {
                    Ok(task_id) => watch_task(&socket_path, task_id).await,
                    Err(e) => Err(CliError::InvalidUuid(e.to_string())),
                }
            }
        }
        "approve" => {
            if args.len() < 3 {
                Err(CliError::MissingArgument("task-id".to_string()))
            } else {
                match Uuid::parse_str(&args[2]) {
                    Ok(task_id) => {
                        // Confirmation prompt before approving
                        if !confirm(&format!(
                            "Are you sure you want to approve task {}?",
                            task_id
                        )) {
                            println!("Approval cancelled.");
                            return;
                        }
                        let cmd = CliCommand::TaskApprove { task_id };
                        send_command(&socket_path, cmd).await
                    }
                    Err(e) => Err(CliError::InvalidUuid(e.to_string())),
                }
            }
        }
        "reject" => {
            if args.len() < 3 {
                Err(CliError::MissingArgument("task-id".to_string()))
            } else {
                match Uuid::parse_str(&args[2]) {
                    Ok(task_id) => {
                        // Parse optional --reason flag
                        let reason = if args.len() > 4 && args[3] == "--reason" {
                            args[4..].join(" ")
                        } else {
                            "User rejected the plan".to_string()
                        };
                        // Confirmation prompt before rejecting
                        if !confirm(&format!(
                            "Are you sure you want to reject task {}?",
                            task_id
                        )) {
                            println!("Rejection cancelled.");
                            return;
                        }
                        let cmd = CliCommand::TaskReject { task_id, reason };
                        send_command(&socket_path, cmd).await
                    }
                    Err(e) => Err(CliError::InvalidUuid(e.to_string())),
                }
            }
        }
        "cancel" => {
            if args.len() < 3 {
                Err(CliError::MissingArgument("task-id".to_string()))
            } else {
                match Uuid::parse_str(&args[2]) {
                    Ok(task_id) => {
                        // Confirmation prompt before cancelling
                        if !confirm(&format!(
                            "Are you sure you want to cancel task {}?",
                            task_id
                        )) {
                            println!("Cancellation aborted.");
                            return;
                        }
                        let cmd = CliCommand::TaskCancel { task_id };
                        send_command(&socket_path, cmd).await
                    }
                    Err(e) => Err(CliError::InvalidUuid(e.to_string())),
                }
            }
        }
        "pause" => {
            if args.len() < 3 {
                Err(CliError::MissingArgument("task-id".to_string()))
            } else {
                match Uuid::parse_str(&args[2]) {
                    Ok(task_id) => {
                        let reason = if args.len() > 4 && args[3] == "--reason" {
                            args[4..].join(" ")
                        } else {
                            "Operator requested pause".to_string()
                        };
                        let cmd = CliCommand::TaskPause { task_id, reason };
                        send_command(&socket_path, cmd).await
                    }
                    Err(e) => Err(CliError::InvalidUuid(e.to_string())),
                }
            }
        }
        "resume" => {
            if args.len() < 3 {
                Err(CliError::MissingArgument("task-id".to_string()))
            } else {
                match Uuid::parse_str(&args[2]) {
                    Ok(task_id) => {
                        let cmd = CliCommand::TaskResume { task_id };
                        send_command(&socket_path, cmd).await
                    }
                    Err(e) => Err(CliError::InvalidUuid(e.to_string())),
                }
            }
        }
        "inject" => {
            if args.len() < 4 {
                Err(CliError::MissingArgument(
                    "task-id and instruction".to_string(),
                ))
            } else {
                match Uuid::parse_str(&args[2]) {
                    Ok(task_id) => {
                        let instruction = args[3..].join(" ");
                        let cmd = CliCommand::TaskInjectInstruction {
                            task_id,
                            instruction,
                        };
                        send_command(&socket_path, cmd).await
                    }
                    Err(e) => Err(CliError::InvalidUuid(e.to_string())),
                }
            }
        }
        "scope" => {
            if args.len() < 3 {
                Err(CliError::MissingArgument("task-id".to_string()))
            } else {
                match Uuid::parse_str(&args[2]) {
                    Ok(task_id) => {
                        // Parse scope arguments: swell scope <task-id> --files file1.rs file2.rs --dirs src tests
                        let mut files = Vec::new();
                        let mut directories = Vec::new();
                        let mut i = 3;
                        while i < args.len() {
                            match args[i].as_str() {
                                "--files" => {
                                    i += 1;
                                    while i < args.len() && !args[i].starts_with("--") {
                                        files.push(args[i].clone());
                                        i += 1;
                                    }
                                }
                                "--dirs" => {
                                    i += 1;
                                    while i < args.len() && !args[i].starts_with("--") {
                                        directories.push(args[i].clone());
                                        i += 1;
                                    }
                                }
                                _ => i += 1,
                            }
                        }
                        let scope = swell_core::TaskScope {
                            files,
                            directories,
                            allowed_operations: vec![],
                        };
                        let cmd = CliCommand::TaskModifyScope { task_id, scope };
                        send_command(&socket_path, cmd).await
                    }
                    Err(e) => Err(CliError::InvalidUuid(e.to_string())),
                }
            }
        }
        "repl" => {
            // Run REPL mode with slash commands
            repl::run_repl()
        }
        unknown => Err(CliError::InvalidCommand(unknown.to_string())),
    };

    if let Err(e) = result {
        print_error(&e);
        std::process::exit(e.exit_code());
    }
}

async fn send_command(socket_path: &str, cmd: CliCommand) -> Result<(), CliError> {
    // Check if socket file exists before trying to connect
    if !std::path::Path::new(socket_path).exists() {
        return Err(CliError::SocketNotFound(socket_path.to_string()));
    }

    // Connect with timeout
    let connect_result = timeout(DEFAULT_CONNECT_TIMEOUT, UnixStream::connect(socket_path)).await;

    let mut stream = match connect_result {
        Ok(Ok(stream)) => stream,
        Ok(Err(e)) => {
            return Err(CliError::ConnectionFailed(format!(
                "Failed to connect to {}: {}",
                socket_path, e
            )));
        }
        Err(_) => {
            return Err(CliError::ConnectionTimeout(DEFAULT_CONNECT_TIMEOUT));
        }
    };

    // Serialize command
    let cmd_json =
        serde_json::to_string(&cmd).map_err(|e| CliError::JsonParseError(e.to_string()))?;

    // Write with timeout
    let write_result = timeout(DEFAULT_REQUEST_TIMEOUT, async {
        stream.write_all(cmd_json.as_bytes()).await?;
        stream.flush().await
    })
    .await;

    if write_result.is_err() {
        return Err(CliError::RequestTimeout(DEFAULT_REQUEST_TIMEOUT));
    }

    // Read response with timeout
    let mut response_buf = Vec::with_capacity(65536);
    let read_result = timeout(DEFAULT_REQUEST_TIMEOUT, stream.read_buf(&mut response_buf)).await;

    let n = match read_result {
        Ok(Ok(n)) => n,
        Ok(Err(e)) => {
            return Err(CliError::ConnectionFailed(format!(
                "Failed to read response: {}",
                e
            )));
        }
        Err(_) => {
            return Err(CliError::RequestTimeout(DEFAULT_REQUEST_TIMEOUT));
        }
    };

    if n > 0 {
        let response_str = String::from_utf8_lossy(&response_buf[..n]);
        let response: DaemonEvent = serde_json::from_str(&response_str)
            .map_err(|e| CliError::JsonParseError(format!("Response: {}", e)))?;

        handle_event(&response);
    }

    Ok(())
}

/// Watch a task and stream events until Ctrl+C or terminal state
async fn watch_task(socket_path: &str, task_id: Uuid) -> Result<(), CliError> {
    // Check if socket file exists before trying to connect
    if !std::path::Path::new(socket_path).exists() {
        return Err(CliError::SocketNotFound(socket_path.to_string()));
    }

    // Connect with timeout
    let connect_result = timeout(DEFAULT_CONNECT_TIMEOUT, UnixStream::connect(socket_path)).await;

    let mut stream = match connect_result {
        Ok(Ok(stream)) => stream,
        Ok(Err(e)) => {
            return Err(CliError::ConnectionFailed(format!(
                "Failed to connect to {}: {}",
                socket_path, e
            )));
        }
        Err(_) => {
            return Err(CliError::ConnectionTimeout(DEFAULT_CONNECT_TIMEOUT));
        }
    };

    let cmd = CliCommand::TaskWatch { task_id };
    let cmd_json =
        serde_json::to_string(&cmd).map_err(|e| CliError::JsonParseError(e.to_string()))?;

    // Write with timeout
    let write_result = timeout(DEFAULT_REQUEST_TIMEOUT, async {
        stream.write_all(cmd_json.as_bytes()).await?;
        stream.flush().await
    })
    .await;

    if write_result.is_err() {
        return Err(CliError::RequestTimeout(DEFAULT_REQUEST_TIMEOUT));
    }

    // Set up Ctrl+C handler
    let mut sigint = match signal(SignalKind::interrupt()) {
        Ok(sig) => sig,
        Err(e) => {
            return Err(CliError::ConnectionFailed(format!(
                "Failed to setup signal handler: {}",
                e
            )));
        }
    };

    println!("Watching task {}... (Press Ctrl+C to stop)", task_id);
    println!("{}", "-".repeat(60));

    // Use a buffered reader for line-by-line reading
    let mut reader = tokio::io::BufReader::new(&mut stream);
    let mut line = String::new();

    loop {
        tokio::select! {
            result = reader.read_line(&mut line) => {
                let n = match result {
                    Ok(n) => n,
                    Err(e) => {
                        eprintln!("\nRead error: {}", e);
                        break;
                    }
                };
                if n == 0 {
                    // EOF - connection closed
                    println!("\nConnection closed by server.");
                    break;
                }

                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }

                // Parse and display the event
                match serde_json::from_str::<DaemonEvent>(trimmed) {
                    Ok(event) => {
                        handle_event(&event);

                        // Check if this is a terminal state event
                        if is_terminal_event(&event) {
                            println!("\nTask reached terminal state. Goodbye!");
                            break;
                        }
                    }
                    Err(e) => {
                        eprintln!("Failed to parse event: {}", e);
                    }
                }

                line.clear();
            }
            _ = sigint.recv() => {
                println!("\nInterrupted. Stopping watch...");
                break;
            }
        }
    }

    Ok(())
}

/// Check if an event represents a terminal state
fn is_terminal_event(event: &DaemonEvent) -> bool {
    matches!(
        event,
        DaemonEvent::TaskCompleted { .. } | DaemonEvent::TaskFailed { .. }
    )
}

/// Handle and display a daemon event
fn handle_event(event: &DaemonEvent) {
    match event {
        DaemonEvent::TaskCreated {
            id,
            correlation_id: _,
        } => {
            println!("Task created: {}", id);
        }
        DaemonEvent::TaskStateChanged {
            id,
            state,
            correlation_id: _,
        } => {
            println!("[{}] State changed to: {}", id, state);
        }
        DaemonEvent::TaskCompleted {
            id,
            pr_url,
            correlation_id: _,
        } => {
            if *id == Uuid::nil() {
                // This is a list response
                if let Some(json) = pr_url {
                    println!("{}", json);
                }
            } else {
                println!("[{}] Task completed!", id);
                if let Some(url) = pr_url {
                    println!("PR: {}", url);
                }
            }
        }
        DaemonEvent::TaskFailed {
            id,
            error,
            failure_class: _,
            correlation_id: _,
        } => {
            eprintln!("[{}] Task failed: {}", id, error);
        }
        DaemonEvent::TaskProgress {
            id,
            message,
            correlation_id: _,
        } => {
            println!("[{}] {}", id, message);
        }
        DaemonEvent::Error {
            message,
            failure_class: _,
            correlation_id: _,
        } => {
            eprintln!("Error: {}", message);
        }
        DaemonEvent::ToolInvocationStarted {
            id,
            tool_name,
            turn_number,
            correlation_id: _,
            ..
        } => {
            println!("[{}] Turn {}: Invoking tool '{}'", id, turn_number, tool_name);
        }
        DaemonEvent::ToolInvocationCompleted {
            id,
            tool_name,
            success,
            duration_ms,
            turn_number,
            correlation_id: _,
            ..
        } => {
            let status = if *success { "success" } else { "failed" };
            println!(
                "[{}] Turn {}: Tool '{}' completed ({} in {}ms)",
                id, turn_number, tool_name, status, duration_ms
            );
        }
        DaemonEvent::AgentTurnStarted {
            id,
            agent_role,
            turn_number,
            correlation_id: _,
        } => {
            println!(
                "[{}] Turn {}: Agent '{}' starting turn",
                id, turn_number, agent_role
            );
        }
        DaemonEvent::AgentTurnCompleted {
            id,
            agent_role,
            turn_number,
            action_taken,
            tools_invoked,
            duration_ms,
            correlation_id: _,
        } => {
            let tools_str = if tools_invoked.is_empty() {
                "no tools".to_string()
            } else {
                tools_invoked.join(", ")
            };
            println!(
                "[{}] Turn {}: Agent '{}' completed - {} ({} invoked, {}ms)",
                id, turn_number, agent_role, action_taken, tools_str, duration_ms
            );
        }
        DaemonEvent::ValidationStepStarted {
            id,
            step_name,
            correlation_id: _,
        } => {
            println!("[{}] Validation: Starting '{}'", id, step_name);
        }
        DaemonEvent::ValidationStepCompleted {
            id,
            step_name,
            passed,
            duration_ms,
            correlation_id: _,
        } => {
            let status = if *passed { "passed" } else { "failed" };
            println!(
                "[{}] Validation: '{}' {} ({}ms)",
                id, step_name, status, duration_ms
            );
        }
    }
}

async fn list_tasks(socket_path: &str, json_output: bool) -> Result<(), CliError> {
    // Check if socket file exists before trying to connect
    if !std::path::Path::new(socket_path).exists() {
        return Err(CliError::SocketNotFound(socket_path.to_string()));
    }

    // Connect with timeout
    let connect_result = timeout(DEFAULT_CONNECT_TIMEOUT, UnixStream::connect(socket_path)).await;

    let mut stream = match connect_result {
        Ok(Ok(stream)) => stream,
        Ok(Err(e)) => {
            return Err(CliError::ConnectionFailed(format!(
                "Failed to connect to {}: {}",
                socket_path, e
            )));
        }
        Err(_) => {
            return Err(CliError::ConnectionTimeout(DEFAULT_CONNECT_TIMEOUT));
        }
    };

    let cmd = CliCommand::TaskList;
    let cmd_json =
        serde_json::to_string(&cmd).map_err(|e| CliError::JsonParseError(e.to_string()))?;

    // Write with timeout
    let write_result = timeout(DEFAULT_REQUEST_TIMEOUT, async {
        stream.write_all(cmd_json.as_bytes()).await?;
        stream.flush().await
    })
    .await;

    if write_result.is_err() {
        return Err(CliError::RequestTimeout(DEFAULT_REQUEST_TIMEOUT));
    }

    // Read response with timeout
    let mut response_buf = Vec::with_capacity(65536);
    let read_result = timeout(DEFAULT_REQUEST_TIMEOUT, stream.read_buf(&mut response_buf)).await;

    let n = match read_result {
        Ok(Ok(n)) => n,
        Ok(Err(e)) => {
            return Err(CliError::ConnectionFailed(format!(
                "Failed to read response: {}",
                e
            )));
        }
        Err(_) => {
            return Err(CliError::RequestTimeout(DEFAULT_REQUEST_TIMEOUT));
        }
    };

    if n > 0 {
        let response_str = String::from_utf8_lossy(&response_buf[..n]);
        let response: DaemonEvent = serde_json::from_str(&response_str)
            .map_err(|e| CliError::JsonParseError(format!("Response: {}", e)))?;

        match response {
            DaemonEvent::TaskCompleted { id, pr_url, .. } => {
                if id == Uuid::nil() {
                    if let Some(json) = pr_url {
                        if json_output {
                            // Raw JSON output
                            println!("{}", json);
                        } else {
                            // Formatted table output
                            let tasks: Vec<Task> = serde_json::from_str(&json).map_err(|e| {
                                CliError::JsonParseError(format!("Task list: {}", e))
                            })?;
                            print_task_table(&tasks);
                        }
                    }
                }
            }
            DaemonEvent::Error { message, .. } => {
                return Err(CliError::ServerError(message));
            }
            other => {
                eprintln!("Unexpected response: {:?}", other);
            }
        }
    }

    Ok(())
}

fn print_task_table(tasks: &[Task]) {
    if tasks.is_empty() {
        println!("No tasks found.");
        return;
    }

    // Table header
    println!("{:36} | {:12} | {:40}", "ID", "STATE", "DESCRIPTION");
    println!(
        "{} | {:12} | {}",
        "-".repeat(36),
        "-".repeat(12),
        "-".repeat(40)
    );

    for task in tasks {
        let description = if task.description.len() > 40 {
            format!("{}...", &task.description[..37])
        } else {
            task.description.clone()
        };
        println!("{:36} | {:12} | {}", task.id, task.state, description);
    }
}

fn usage() {
    eprintln!(
        "swell - Autonomous Coding Engine CLI

Usage:
    swell task <description>      Create a new task
    swell list [--json]           List all tasks (--json for raw JSON)
    swell watch <task-id>        Watch task status
    swell approve <task-id>      Approve task plan
    swell reject <task-id> [--reason <reason>]   Reject task plan
    swell cancel <task-id>        Cancel a task
    swell pause <task-id> [--reason <reason>]   Pause a running task
    swell resume <task-id>       Resume a paused task
    swell inject <task-id> <instruction>   Inject instructions into a task
    swell scope <task-id> [--files <files>] [--dirs <dirs>]   Modify task scope
    swell repl                   Enter REPL mode with slash commands

Environment:
    SWELL_SOCKET                  Socket path (default: /tmp/swell-daemon.sock)

Exit codes:
    0   Success
    1   Server error or internal error
    2   Invalid command or arguments
    10  Connection failed (daemon not running)
    11  Timeout
"
    );
}
