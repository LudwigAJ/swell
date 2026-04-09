use swell_core::{CliCommand, DaemonEvent};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use uuid::Uuid;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Simple CLI parsing for MVP
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        usage();
        return Ok(());
    }

    let socket_path =
        std::env::var("SWELL_SOCKET").unwrap_or_else(|_| "/tmp/swell-daemon.sock".to_string());

    match args[1].as_str() {
        "task" => {
            if args.len() < 3 {
                eprintln!("Error: 'task' command requires a description");
                usage();
                return Ok(());
            }
            let description = args[2..].join(" ");
            let cmd = CliCommand::TaskCreate { description };
            send_command(&socket_path, cmd).await?;
        }
        "list" => {
            let cmd = CliCommand::TaskList;
            send_command(&socket_path, cmd).await?;
        }
        "watch" => {
            if args.len() < 3 {
                eprintln!("Error: 'watch' command requires a task ID");
                return Ok(());
            }
            let task_id = Uuid::parse_str(&args[2]).expect("Invalid UUID format");
            let cmd = CliCommand::TaskWatch { task_id };
            send_command(&socket_path, cmd).await?;
        }
        "approve" => {
            if args.len() < 3 {
                eprintln!("Error: 'approve' command requires a task ID");
                return Ok(());
            }
            let task_id = Uuid::parse_str(&args[2]).expect("Invalid UUID format");
            let cmd = CliCommand::TaskApprove { task_id };
            send_command(&socket_path, cmd).await?;
        }
        "cancel" => {
            if args.len() < 3 {
                eprintln!("Error: 'cancel' command requires a task ID");
                return Ok(());
            }
            let task_id = Uuid::parse_str(&args[2]).expect("Invalid UUID format");
            let cmd = CliCommand::TaskCancel { task_id };
            send_command(&socket_path, cmd).await?;
        }
        _ => {
            usage();
        }
    }

    Ok(())
}

async fn send_command(
    socket_path: &str,
    cmd: CliCommand,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut stream = UnixStream::connect(socket_path).await?;

    let cmd_json = serde_json::to_string(&cmd)?;
    stream.write_all(cmd_json.as_bytes()).await?;
    stream.flush().await?;

    let mut response_buf = Vec::with_capacity(65536);
    let n = stream.read_buf(&mut response_buf).await?;

    if n > 0 {
        let response_str = String::from_utf8_lossy(&response_buf[..n]);
        let response: DaemonEvent =
            serde_json::from_str(&response_str).expect("Invalid response format");

        match response {
            DaemonEvent::TaskCreated { id, correlation_id: _ } => {
                println!("Task created: {}", id);
            }
            DaemonEvent::TaskStateChanged { id, state, correlation_id: _ } => {
                println!("Task {} is now: {}", id, state);
            }
            DaemonEvent::TaskCompleted { id, pr_url, correlation_id: _ } => {
                if id == Uuid::nil() {
                    // This is a list response
                    if let Some(json) = pr_url {
                        println!("{}", json);
                    }
                } else {
                    println!("Task {} completed", id);
                    if let Some(url) = pr_url {
                        println!("PR: {}", url);
                    }
                }
            }
            DaemonEvent::TaskFailed { id, error, correlation_id: _ } => {
                eprintln!("Task {} failed: {}", id, error);
            }
            DaemonEvent::TaskProgress { id, message, correlation_id: _ } => {
                println!("[{}] {}", id, message);
            }
            DaemonEvent::Error { message, correlation_id: _ } => {
                eprintln!("Error: {}", message);
            }
        }
    }

    Ok(())
}

fn usage() {
    eprintln!(
        "swell - Autonomous Coding Engine CLI
    "
    );
    eprintln!(
        "Usage:
    swell task <description>     Create a new task
    swell list                     List all tasks
    swell watch <task-id>         Watch task status
    swell approve <task-id>       Approve task plan
    swell cancel <task-id>         Cancel a task
    "
    );
}
