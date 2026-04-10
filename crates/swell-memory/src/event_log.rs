// event_log.rs - Append-only JSONL event log with schema versioning
//
// This module provides an immutable audit trail for system events.
// Events are appended to a JSONL file (one JSON object per line) with
// schema version tracking for forward compatibility and replay capability.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;
use uuid::Uuid;

/// Current schema version for event log entries
const CURRENT_SCHEMA_VERSION: &str = "1.0";

/// Event types that can be logged
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    /// Tool was invoked
    ToolInvocation,
    /// Agent made a decision
    Decision,
    /// Observation recorded
    Observation,
    /// Error occurred
    Error,
    /// Task outcome (success/failure)
    Outcome,
    /// State transition occurred
    StateTransition,
    /// Validation result
    ValidationResult,
    /// LLM call made
    LlmCall,
    /// Memory operation
    MemoryOperation,
}

/// Schema versioned event entry for JSONL storage
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventLogEntry {
    /// Schema version for forward compatibility
    pub schema_version: String,
    /// Unique event ID
    pub id: Uuid,
    /// Event type
    pub event_type: EventType,
    /// When the event occurred
    pub timestamp: DateTime<Utc>,
    /// Optional task ID this event belongs to
    pub task_id: Option<Uuid>,
    /// Optional session ID
    pub session_id: Option<Uuid>,
    /// Optional agent ID
    pub agent_id: Option<Uuid>,
    /// Event-specific payload (serialized as JSON)
    pub payload: serde_json::Value,
    /// Optional correlation ID for tracing
    pub correlation_id: Option<Uuid>,
}

impl EventLogEntry {
    /// Create a new event log entry
    pub fn new(event_type: EventType, payload: serde_json::Value) -> Self {
        Self {
            schema_version: CURRENT_SCHEMA_VERSION.to_string(),
            id: Uuid::new_v4(),
            event_type,
            timestamp: Utc::now(),
            task_id: None,
            session_id: None,
            agent_id: None,
            payload,
            correlation_id: None,
        }
    }

    /// Set the task ID
    pub fn with_task_id(mut self, task_id: Uuid) -> Self {
        self.task_id = Some(task_id);
        self
    }

    /// Set the session ID
    pub fn with_session_id(mut self, session_id: Uuid) -> Self {
        self.session_id = Some(session_id);
        self
    }

    /// Set the agent ID
    pub fn with_agent_id(mut self, agent_id: Uuid) -> Self {
        self.agent_id = Some(agent_id);
        self
    }

    /// Set the correlation ID
    pub fn with_correlation_id(mut self, correlation_id: Uuid) -> Self {
        self.correlation_id = Some(correlation_id);
        self
    }
}

/// Tool invocation payload
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolInvocationPayload {
    pub tool_name: String,
    pub arguments: serde_json::Value,
    pub success: bool,
    pub duration_ms: u64,
    pub error: Option<String>,
}

/// Decision payload
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionPayload {
    pub decision: String,
    pub reasoning: String,
    pub context: serde_json::Value,
}

/// Observation payload
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservationPayload {
    pub observation: String,
    pub significance: String,
    pub source: String,
}

/// Error payload
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorPayload {
    pub error_type: String,
    pub message: String,
    pub stack_trace: Option<String>,
}

/// Outcome payload
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutcomePayload {
    pub success: bool,
    pub summary: String,
    pub details: serde_json::Value,
}

/// State transition payload
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateTransitionPayload {
    pub from_state: String,
    pub to_state: String,
    pub reason: Option<String>,
}

/// Validation result payload
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationResultPayload {
    pub gate: String,
    pub passed: bool,
    pub errors: Vec<String>,
    pub warnings: Vec<String>,
}

/// LLM call payload
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmCallPayload {
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub duration_ms: u64,
    pub success: bool,
}

/// Memory operation payload
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryOperationPayload {
    pub operation: String,
    pub memory_id: Option<Uuid>,
    pub success: bool,
}

/// Event log for append-only JSONL storage with replay capability
#[derive(Clone)]
pub struct EventLog {
    path: std::path::PathBuf,
}

impl EventLog {
    /// Create a new event log at the given path
    pub fn new(path: impl Into<std::path::PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Initialize the event log (creates parent directories if needed)
    pub fn init(&self) -> io::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        // Create the file if it doesn't exist
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        Ok(())
    }

    /// Append an event to the log (append-only, never modify)
    pub fn append(&self, entry: &EventLogEntry) -> io::Result<()> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;

        let json = serde_json::to_string(entry)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        writeln!(file, "{}", json)?;
        file.flush()?;

        Ok(())
    }

    /// Log a tool invocation event
    pub fn log_tool_invocation(
        &self,
        task_id: Uuid,
        tool_name: &str,
        arguments: serde_json::Value,
        success: bool,
        duration_ms: u64,
        error: Option<String>,
    ) -> io::Result<()> {
        let payload = ToolInvocationPayload {
            tool_name: tool_name.to_string(),
            arguments,
            success,
            duration_ms,
            error,
        };

        let entry = EventLogEntry::new(
            EventType::ToolInvocation,
            serde_json::to_value(payload).unwrap_or_default(),
        )
        .with_task_id(task_id);

        self.append(&entry)
    }

    /// Log a decision event
    pub fn log_decision(
        &self,
        task_id: Uuid,
        agent_id: Uuid,
        decision: &str,
        reasoning: &str,
        context: serde_json::Value,
    ) -> io::Result<()> {
        let payload = DecisionPayload {
            decision: decision.to_string(),
            reasoning: reasoning.to_string(),
            context,
        };

        let entry = EventLogEntry::new(
            EventType::Decision,
            serde_json::to_value(payload).unwrap_or_default(),
        )
        .with_task_id(task_id)
        .with_agent_id(agent_id);

        self.append(&entry)
    }

    /// Log an error event
    pub fn log_error(
        &self,
        task_id: Option<Uuid>,
        error_type: &str,
        message: &str,
        stack_trace: Option<String>,
    ) -> io::Result<()> {
        let payload = ErrorPayload {
            error_type: error_type.to_string(),
            message: message.to_string(),
            stack_trace,
        };

        let mut entry = EventLogEntry::new(
            EventType::Error,
            serde_json::to_value(payload).unwrap_or_default(),
        );

        if let Some(tid) = task_id {
            entry = entry.with_task_id(tid);
        }

        self.append(&entry)
    }

    /// Log an outcome event
    pub fn log_outcome(
        &self,
        task_id: Uuid,
        success: bool,
        summary: &str,
        details: serde_json::Value,
    ) -> io::Result<()> {
        let payload = OutcomePayload {
            success,
            summary: summary.to_string(),
            details,
        };

        let entry = EventLogEntry::new(
            EventType::Outcome,
            serde_json::to_value(payload).unwrap_or_default(),
        )
        .with_task_id(task_id);

        self.append(&entry)
    }

    /// Log a state transition event
    pub fn log_state_transition(
        &self,
        task_id: Uuid,
        from_state: &str,
        to_state: &str,
        reason: Option<String>,
    ) -> io::Result<()> {
        let payload = StateTransitionPayload {
            from_state: from_state.to_string(),
            to_state: to_state.to_string(),
            reason,
        };

        let entry = EventLogEntry::new(
            EventType::StateTransition,
            serde_json::to_value(payload).unwrap_or_default(),
        )
        .with_task_id(task_id);

        self.append(&entry)
    }

    /// Replay events from the log (for debugging and reconstruction)
    /// Returns an iterator over all entries in the log
    pub fn replay(&self) -> io::Result<EventLogReplayIter<fs::File>> {
        let file = File::open(&self.path)?;
        let reader = BufReader::new(file);
        Ok(EventLogReplayIter {
            reader,
            line: 0,
            current_entry: None,
        })
    }

    /// Replay events for a specific task
    pub fn replay_for_task(&self, task_id: Uuid) -> io::Result<TaskEventIter> {
        Ok(TaskEventIter {
            inner: self.replay()?,
            task_id,
        })
    }

    /// Get the path to the log file
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Check if the log file exists
    pub fn exists(&self) -> bool {
        self.path.exists()
    }
}

/// Iterator for replaying all events in the log
pub struct EventLogReplayIter<R: io::Read> {
    reader: BufReader<R>,
    line: usize,
    current_entry: Option<EventLogEntry>,
}

impl<R: io::Read> EventLogReplayIter<R> {
    /// Parse a schema-versioned JSON entry, handling version differences
    fn parse_entry(json: &str) -> io::Result<EventLogEntry> {
        // Try to parse as EventLogEntry directly
        serde_json::from_str(json).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }
}

impl<R: io::Read> Iterator for EventLogReplayIter<R> {
    type Item = io::Result<EventLogEntry>;

    fn next(&mut self) -> Option<Self::Item> {
        let mut line = String::new();

        match self.reader.read_line(&mut line) {
            Ok(0) => None, // EOF
            Ok(_) => {
                self.line += 1;
                let json = line.trim();

                if json.is_empty() {
                    // Skip empty lines
                    return self.next();
                }

                match Self::parse_entry(json) {
                    Ok(entry) => {
                        self.current_entry = Some(entry.clone());
                        Some(Ok(entry))
                    }
                    Err(e) => Some(Err(e)),
                }
            }
            Err(e) => Some(Err(e)),
        }
    }
}

/// Iterator for replaying events filtered by task ID
pub struct TaskEventIter {
    inner: EventLogReplayIter<fs::File>,
    task_id: Uuid,
}

impl Iterator for TaskEventIter {
    type Item = io::Result<EventLogEntry>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match self.inner.next() {
                Some(Ok(entry)) => {
                    if entry.task_id == Some(self.task_id) {
                        return Some(Ok(entry));
                    }
                    // Continue looking for matching task
                }
                Some(Err(e)) => return Some(Err(e)),
                None => return None,
            }
        }
    }
}

/// Helper to read all events as a Vec (for testing/debugging)
pub fn read_all_events(path: &Path) -> io::Result<Vec<EventLogEntry>> {
    let log = EventLog::new(path);
    log.replay()?.collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_temp_log() -> (TempDir, EventLog) {
        let temp_dir = TempDir::new().unwrap();
        let log_path = temp_dir.path().join("events.jsonl");
        let log = EventLog::new(&log_path);
        log.init().unwrap();
        (temp_dir, log)
    }

    #[test]
    fn test_append_and_replay() {
        let (_temp_dir, log) = create_temp_log();

        let entry = EventLogEntry::new(
            EventType::ToolInvocation,
            serde_json::json!({"tool_name": "test", "success": true}),
        )
        .with_task_id(Uuid::new_v4());

        log.append(&entry).unwrap();

        let entries: Vec<_> = log.replay().unwrap().collect();
        assert_eq!(entries.len(), 1);

        let replayed = &entries[0].as_ref().unwrap();
        assert_eq!(replayed.schema_version, "1.0");
        assert!(matches!(replayed.event_type, EventType::ToolInvocation));
    }

    #[test]
    fn test_log_tool_invocation() {
        let (_temp_dir, log) = create_temp_log();

        let task_id = Uuid::new_v4();
        log.log_tool_invocation(
            task_id,
            "file_read",
            serde_json::json!({"path": "/tmp/test.txt"}),
            true,
            15,
            None,
        )
        .unwrap();

        let entries: Vec<_> = log.replay().unwrap().collect();
        assert_eq!(entries.len(), 1);

        let entry = entries[0].as_ref().unwrap();
        assert_eq!(entry.task_id, Some(task_id));

        let payload: ToolInvocationPayload = serde_json::from_value(entry.payload.clone()).unwrap();
        assert_eq!(payload.tool_name, "file_read");
        assert!(payload.success);
        assert_eq!(payload.duration_ms, 15);
    }

    #[test]
    fn test_log_decision() {
        let (_temp_dir, log) = create_temp_log();

        let task_id = Uuid::new_v4();
        let agent_id = Uuid::new_v4();

        log.log_decision(
            task_id,
            agent_id,
            "Use refactoring approach",
            "Based on code complexity analysis",
            serde_json::json!({"complexity": 7}),
        )
        .unwrap();

        let entries: Vec<_> = log.replay().unwrap().collect();
        assert_eq!(entries.len(), 1);

        let entry = entries[0].as_ref().unwrap();
        assert_eq!(entry.agent_id, Some(agent_id));

        let payload: DecisionPayload = serde_json::from_value(entry.payload.clone()).unwrap();
        assert_eq!(payload.decision, "Use refactoring approach");
    }

    #[test]
    fn test_log_error() {
        let (_temp_dir, log) = create_temp_log();

        let task_id = Uuid::new_v4();
        log.log_error(
            Some(task_id),
            "FileNotFound",
            "Could not open /tmp/missing.txt",
            Some("stack trace here".to_string()),
        )
        .unwrap();

        let entries: Vec<_> = log.replay().unwrap().collect();
        assert_eq!(entries.len(), 1);

        let payload: ErrorPayload =
            serde_json::from_value(entries[0].as_ref().unwrap().payload.clone()).unwrap();
        assert_eq!(payload.error_type, "FileNotFound");
        assert!(payload.stack_trace.is_some());
    }

    #[test]
    fn test_log_outcome() {
        let (_temp_dir, log) = create_temp_log();

        let task_id = Uuid::new_v4();
        log.log_outcome(
            task_id,
            true,
            "Task completed successfully",
            serde_json::json!({"files_modified": 3}),
        )
        .unwrap();

        let entries: Vec<_> = log.replay().unwrap().collect();
        assert_eq!(entries.len(), 1);

        let payload: OutcomePayload =
            serde_json::from_value(entries[0].as_ref().unwrap().payload.clone()).unwrap();
        assert!(payload.success);
    }

    #[test]
    fn test_log_state_transition() {
        let (_temp_dir, log) = create_temp_log();

        let task_id = Uuid::new_v4();
        log.log_state_transition(
            task_id,
            "EXECUTING",
            "VALIDATING",
            Some("All steps completed".to_string()),
        )
        .unwrap();

        let entries: Vec<_> = log.replay().unwrap().collect();
        assert_eq!(entries.len(), 1);

        let payload: StateTransitionPayload =
            serde_json::from_value(entries[0].as_ref().unwrap().payload.clone()).unwrap();
        assert_eq!(payload.from_state, "EXECUTING");
        assert_eq!(payload.to_state, "VALIDATING");
    }

    #[test]
    fn test_replay_for_task() {
        let (_temp_dir, log) = create_temp_log();

        let task1 = Uuid::new_v4();
        let task2 = Uuid::new_v4();

        // Log events for task1
        log.log_state_transition(task1, "CREATED", "EXECUTING", None)
            .unwrap();
        log.log_tool_invocation(task1, "shell", serde_json::json!({}), true, 100, None)
            .unwrap();

        // Log events for task2
        log.log_state_transition(task2, "CREATED", "EXECUTING", None)
            .unwrap();

        // Replay only task1
        let task1_entries: Vec<_> = log.replay_for_task(task1).unwrap().collect();
        assert_eq!(task1_entries.len(), 2);

        for entry in task1_entries {
            let e = entry.unwrap();
            assert_eq!(e.task_id, Some(task1));
        }
    }

    #[test]
    fn test_multiple_entries() {
        let (_temp_dir, log) = create_temp_log();

        for i in 0..5 {
            let entry = EventLogEntry::new(EventType::Observation, serde_json::json!({"index": i}));
            log.append(&entry).unwrap();
        }

        let entries: Vec<_> = log.replay().unwrap().collect();
        assert_eq!(entries.len(), 5);
    }

    #[test]
    fn test_event_log_entry_builder() {
        let entry = EventLogEntry::new(
            EventType::LlmCall,
            serde_json::json!({"model": "claude-3-5-sonnet"}),
        )
        .with_task_id(Uuid::new_v4())
        .with_session_id(Uuid::new_v4())
        .with_agent_id(Uuid::new_v4())
        .with_correlation_id(Uuid::new_v4());

        assert!(entry.task_id.is_some());
        assert!(entry.session_id.is_some());
        assert!(entry.agent_id.is_some());
        assert!(entry.correlation_id.is_some());
    }

    #[test]
    fn test_jsonl_format() {
        let (_temp_dir, log) = create_temp_log();

        let entry = EventLogEntry::new(
            EventType::ToolInvocation,
            serde_json::json!({"key": "value"}),
        );
        log.append(&entry).unwrap();

        // Read raw file content
        let content = std::fs::read_to_string(log.path()).unwrap();
        let line = content.trim();

        // Should be valid JSON
        let parsed: EventLogEntry = serde_json::from_str(line).unwrap();
        assert_eq!(parsed.id, entry.id);

        // Should NOT have trailing comma or other JSONL issues
        assert!(!line.contains("\n"));
    }

    #[test]
    fn test_exists_and_path() {
        let temp_dir = TempDir::new().unwrap();
        let log_path = temp_dir.path().join("events.jsonl");

        // Before init
        let log = EventLog::new(&log_path);
        assert!(!log.exists());

        // After init
        log.init().unwrap();
        assert!(log.exists());
        assert_eq!(log.path(), log_path);
    }
}
