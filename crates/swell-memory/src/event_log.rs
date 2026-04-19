// event_log.rs - Append-only JSONL event log with schema versioning
//
// This module provides an immutable audit trail for system events.
// Events are appended to a JSONL file (one JSON object per line) with
// schema version tracking for forward compatibility and replay capability.
// Supports hot/warm/cold retention tiers for log data lifecycle management.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;
use uuid::Uuid;

use swell_core::ids::{AgentId, SessionId, TaskId};

/// Current schema version for event log entries
const CURRENT_SCHEMA_VERSION: &str = "1.0";

/// Retention tier for event log data
/// Hot = recent events (last 24h), Warm = recent month, Cold = archived
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RetentionTier {
    /// Recently written events (last 24 hours)
    #[default]
    Hot,
    /// Events from the past month
    Warm,
    /// Archived events (older than 1 month)
    Cold,
}

impl RetentionTier {
    /// Get the age threshold in seconds for this tier
    pub fn age_threshold_seconds(&self) -> i64 {
        match self {
            RetentionTier::Hot => 24 * 60 * 60,       // 24 hours
            RetentionTier::Warm => 30 * 24 * 60 * 60, // 30 days
            RetentionTier::Cold => 90 * 24 * 60 * 60, // 90 days (older goes to cold)
        }
    }

    /// Determine tier based on event age
    pub fn from_age_seconds(age_seconds: i64) -> Self {
        if age_seconds < RetentionTier::Hot.age_threshold_seconds() {
            RetentionTier::Hot
        } else if age_seconds < RetentionTier::Warm.age_threshold_seconds() {
            RetentionTier::Warm
        } else {
            RetentionTier::Cold
        }
    }
}

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
    pub task_id: Option<TaskId>,
    /// Optional session ID
    pub session_id: Option<SessionId>,
    /// Optional agent ID
    pub agent_id: Option<AgentId>,
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
    pub fn with_task_id(mut self, task_id: TaskId) -> Self {
        self.task_id = Some(task_id);
        self
    }

    /// Set the session ID
    pub fn with_session_id(mut self, session_id: SessionId) -> Self {
        self.session_id = Some(session_id);
        self
    }

    /// Set the agent ID
    pub fn with_agent_id(mut self, agent_id: AgentId) -> Self {
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
        task_id: TaskId,
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
        task_id: TaskId,
        agent_id: AgentId,
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
        task_id: Option<TaskId>,
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
        task_id: TaskId,
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
        task_id: TaskId,
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
    pub fn replay_for_task(&self, task_id: TaskId) -> io::Result<TaskEventIter> {
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
    task_id: TaskId,
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

/// Session state reconstructed from event log replay
/// Tracks the state of a session as events are replayed
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionState {
    /// Session identifier
    pub session_id: Option<SessionId>,
    /// Current state of the session
    pub current_state: String,
    /// Task states tracked by task ID
    pub task_states: HashMap<TaskId, String>,
    /// Tool invocations by task
    pub tool_invocations: HashMap<TaskId, Vec<ToolInvocationSummary>>,
    /// Decisions made by agent
    pub decisions: Vec<DecisionSummary>,
    /// All events by correlation ID for tracing
    pub events_by_correlation: HashMap<Uuid, Vec<Uuid>>,
    /// LLM calls made
    pub llm_calls: Vec<LlmCallSummary>,
    /// Final outcomes by task
    pub outcomes: HashMap<TaskId, OutcomeSummary>,
    /// Error count
    pub error_count: usize,
    /// Total events processed
    pub events_processed: usize,
}

impl SessionState {
    /// Create a new empty session state
    pub fn new() -> Self {
        Self::default()
    }

    /// Process a single event and update session state
    pub fn process_event(&mut self, entry: &EventLogEntry) {
        self.events_processed += 1;

        // Track correlation
        if let Some(corr_id) = entry.correlation_id {
            self.events_by_correlation
                .entry(corr_id)
                .or_default()
                .push(entry.id);
        }

        // Track session
        if let Some(sid) = entry.session_id {
            self.session_id = Some(sid);
        }

        match entry.event_type {
            EventType::StateTransition => {
                if let Some(payload) = entry.payload.get("to_state").and_then(|v| v.as_str()) {
                    if let Some(tid) = entry.task_id {
                        self.task_states.insert(tid, payload.to_string());
                        self.current_state = payload.to_string();
                    }
                }
            }
            EventType::ToolInvocation => {
                if let Some(tid) = entry.task_id {
                    let summary = ToolInvocationSummary {
                        tool_name: entry
                            .payload
                            .get("tool_name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown")
                            .to_string(),
                        success: entry
                            .payload
                            .get("success")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false),
                        timestamp: entry.timestamp,
                    };
                    self.tool_invocations.entry(tid).or_default().push(summary);
                }
            }
            EventType::Decision => {
                let summary = DecisionSummary {
                    decision: entry
                        .payload
                        .get("decision")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    timestamp: entry.timestamp,
                    agent_id: entry.agent_id,
                };
                self.decisions.push(summary);
            }
            EventType::LlmCall => {
                let summary = LlmCallSummary {
                    model: entry
                        .payload
                        .get("model")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown")
                        .to_string(),
                    input_tokens: entry
                        .payload
                        .get("input_tokens")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0),
                    output_tokens: entry
                        .payload
                        .get("output_tokens")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0),
                    success: entry
                        .payload
                        .get("success")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false),
                    timestamp: entry.timestamp,
                };
                self.llm_calls.push(summary);
            }
            EventType::Outcome => {
                if let Some(tid) = entry.task_id {
                    let summary = OutcomeSummary {
                        success: entry
                            .payload
                            .get("success")
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false),
                        summary: entry
                            .payload
                            .get("summary")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                        timestamp: entry.timestamp,
                    };
                    self.outcomes.insert(tid, summary);
                }
            }
            EventType::Error => {
                self.error_count += 1;
            }
            _ => {}
        }
    }

    /// Replay events from an iterator and reconstruct session state
    pub fn replay_events<I: Iterator<Item = io::Result<EventLogEntry>>>(
        events: I,
    ) -> io::Result<Self> {
        let mut state = Self::new();
        for event_result in events {
            let entry = event_result?;
            state.process_event(&entry);
        }
        Ok(state)
    }
}

/// Summary of a tool invocation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolInvocationSummary {
    pub tool_name: String,
    pub success: bool,
    pub timestamp: DateTime<Utc>,
}

/// Summary of a decision
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionSummary {
    pub decision: String,
    pub timestamp: DateTime<Utc>,
    pub agent_id: Option<AgentId>,
}

/// Summary of an LLM call
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmCallSummary {
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub success: bool,
    pub timestamp: DateTime<Utc>,
}

/// Summary of an outcome
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutcomeSummary {
    pub success: bool,
    pub summary: String,
    pub timestamp: DateTime<Utc>,
}

/// Replayable event log with session state reconstruction
/// Organizes events into hot/warm/cold tiers based on age
#[derive(Clone)]
pub struct ReplayableEventLog {
    /// Base path for the event log
    base_path: std::path::PathBuf,
}

impl ReplayableEventLog {
    /// Create a new replayable event log at the given base path
    pub fn new(base_path: impl Into<std::path::PathBuf>) -> Self {
        let base_path = base_path.into();
        Self { base_path }
    }

    /// Initialize the event log with tier directories
    pub fn init(&self) -> io::Result<()> {
        // Create base directory
        fs::create_dir_all(&self.base_path)?;

        // Create tier subdirectories
        for tier in &[RetentionTier::Hot, RetentionTier::Warm, RetentionTier::Cold] {
            let tier_path = self.tier_path(*tier);
            fs::create_dir_all(tier_path)?;
        }

        Ok(())
    }

    /// Get the path for a specific tier
    pub fn tier_path(&self, tier: RetentionTier) -> std::path::PathBuf {
        let tier_name = match tier {
            RetentionTier::Hot => "hot",
            RetentionTier::Warm => "warm",
            RetentionTier::Cold => "cold",
        };
        self.base_path.join(tier_name)
    }

    /// Write an event to the appropriate tier based on timestamp
    pub fn write_event(&self, entry: &EventLogEntry) -> io::Result<()> {
        let tier = self.tier_for_event(entry);
        let tier_path = self.tier_path(tier);

        // Create the file if it doesn't exist
        let log_file = tier_path.join(format!("{}.jsonl", entry.timestamp.format("%Y%m%d")));

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_file)?;

        let json = serde_json::to_string(entry)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        writeln!(file, "{}", json)?;
        file.flush()?;

        Ok(())
    }

    /// Determine which tier an event belongs to based on its timestamp
    pub fn tier_for_event(&self, entry: &EventLogEntry) -> RetentionTier {
        let age = Utc::now() - entry.timestamp;
        RetentionTier::from_age_seconds(age.num_seconds())
    }

    /// Replay all events across all tiers in chronological order
    pub fn replay(&self) -> io::Result<impl Iterator<Item = io::Result<EventLogEntry>>> {
        // Collect all JSONL files from all tiers
        let mut all_files: Vec<std::path::PathBuf> = Vec::new();

        for tier in &[RetentionTier::Hot, RetentionTier::Warm, RetentionTier::Cold] {
            let tier_path = self.tier_path(*tier);
            if tier_path.exists() {
                if let Ok(entries) = fs::read_dir(tier_path) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.extension().is_some_and(|e| e == "jsonl") {
                            all_files.push(path);
                        }
                    }
                }
            }
        }

        // Sort files by name (which includes date) for chronological replay
        all_files.sort();

        Ok(ChainedEventIter {
            files: all_files,
            current_reader: None,
            current_file_idx: 0,
        })
    }

    /// Replay events for a specific session
    pub fn replay_for_session(&self, session_id: SessionId) -> io::Result<SessionState> {
        let events = self.replay()?;
        let filtered = events.filter(|e| {
            e.as_ref()
                .map(|entry| entry.session_id == Some(session_id))
                .unwrap_or(false)
        });
        SessionState::replay_events(filtered)
    }

    /// Replay events for a specific task
    pub fn replay_for_task(&self, task_id: TaskId) -> io::Result<SessionState> {
        let events = self.replay()?;
        let filtered = events.filter(|e| {
            e.as_ref()
                .map(|entry| entry.task_id == Some(task_id))
                .unwrap_or(false)
        });
        SessionState::replay_events(filtered)
    }

    /// Get tier statistics
    pub fn tier_stats(&self) -> io::Result<HashMap<RetentionTier, TierStats>> {
        let mut stats = HashMap::new();

        for tier in &[RetentionTier::Hot, RetentionTier::Warm, RetentionTier::Cold] {
            let tier_path = self.tier_path(*tier);
            let mut file_count = 0;
            let mut event_count = 0;

            if tier_path.exists() {
                if let Ok(entries) = fs::read_dir(&tier_path) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.extension().is_some_and(|e| e == "jsonl") {
                            file_count += 1;
                            // Count events in file
                            if let Ok(content) = fs::read_to_string(&path) {
                                event_count +=
                                    content.lines().filter(|l| !l.trim().is_empty()).count();
                            }
                        }
                    }
                }
            }

            stats.insert(
                *tier,
                TierStats {
                    file_count,
                    event_count,
                },
            );
        }

        Ok(stats)
    }
}

/// Statistics for a retention tier
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TierStats {
    pub file_count: usize,
    pub event_count: usize,
}

/// Iterator that chains multiple event log files together
struct ChainedEventIter {
    files: Vec<std::path::PathBuf>,
    current_reader: Option<BufReader<File>>,
    current_file_idx: usize,
}

impl ChainedEventIter {
    fn open_next_file(&mut self) -> io::Result<()> {
        if self.current_file_idx < self.files.len() {
            let file = File::open(&self.files[self.current_file_idx])?;
            self.current_reader = Some(BufReader::new(file));
            Ok(())
        } else {
            self.current_reader = None;
            Ok(())
        }
    }
}

impl Iterator for ChainedEventIter {
    type Item = io::Result<EventLogEntry>;

    fn next(&mut self) -> Option<Self::Item> {
        // Open next file if needed
        if self.current_reader.is_none() {
            if let Err(e) = self.open_next_file() {
                return Some(Err(e));
            }
            // If still none, we've exhausted all files
            self.current_reader.as_mut()?;
        }

        // Try to read from current file
        loop {
            let reader = self.current_reader.as_mut()?;
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) => {
                    // EOF, move to next file
                    self.current_file_idx += 1;
                    self.current_reader = None;
                    // Recursively call next to open next file
                    return self.next();
                }
                Ok(_) => {
                    let json = line.trim();
                    if json.is_empty() {
                        continue;
                    }
                    match serde_json::from_str(json) {
                        Ok(entry) => return Some(Ok(entry)),
                        Err(e) => return Some(Err(io::Error::new(io::ErrorKind::InvalidData, e))),
                    }
                }
                Err(e) => return Some(Err(e)),
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
mod episodic {
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
        .with_task_id(TaskId::new());

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

        let task_id = TaskId::new();
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

        let task_id = TaskId::new();
        let agent_id = AgentId::new();

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

        let task_id = TaskId::new();
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

        let task_id = TaskId::new();
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

        let task_id = TaskId::new();
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

        let task1 = TaskId::new();
        let task2 = TaskId::new();

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
        .with_task_id(TaskId::new())
        .with_session_id(SessionId::new())
        .with_agent_id(AgentId::new())
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

    // ============================================================
    // Retention tier tests
    // ============================================================

    #[test]
    fn test_retention_tier_age_thresholds() {
        assert_eq!(RetentionTier::Hot.age_threshold_seconds(), 24 * 60 * 60);
        assert_eq!(
            RetentionTier::Warm.age_threshold_seconds(),
            30 * 24 * 60 * 60
        );
        assert_eq!(
            RetentionTier::Cold.age_threshold_seconds(),
            90 * 24 * 60 * 60
        );
    }

    #[test]
    fn test_retention_tier_from_age_seconds() {
        // Hot: less than 24 hours
        assert_eq!(RetentionTier::from_age_seconds(0), RetentionTier::Hot);
        assert_eq!(
            RetentionTier::from_age_seconds(23 * 60 * 60),
            RetentionTier::Hot
        );

        // Warm: between 24h and 30 days
        assert_eq!(
            RetentionTier::from_age_seconds(24 * 60 * 60),
            RetentionTier::Warm
        );
        assert_eq!(
            RetentionTier::from_age_seconds(29 * 24 * 60 * 60),
            RetentionTier::Warm
        );

        // Cold: older than 30 days
        assert_eq!(
            RetentionTier::from_age_seconds(30 * 24 * 60 * 60),
            RetentionTier::Cold
        );
        assert_eq!(
            RetentionTier::from_age_seconds(100 * 24 * 60 * 60),
            RetentionTier::Cold
        );
    }

    #[test]
    fn test_retention_tier_default() {
        let tier = RetentionTier::default();
        assert_eq!(tier, RetentionTier::Hot);
    }

    #[test]
    fn test_replayable_event_log_init() {
        let temp_dir = TempDir::new().unwrap();
        let base_path = temp_dir.path().join("event_log");

        let log = ReplayableEventLog::new(&base_path);
        log.init().unwrap();

        // Check that tier directories were created
        assert!(log.tier_path(RetentionTier::Hot).exists());
        assert!(log.tier_path(RetentionTier::Warm).exists());
        assert!(log.tier_path(RetentionTier::Cold).exists());
    }

    #[test]
    fn test_replayable_event_log_tier_for_event() {
        let temp_dir = TempDir::new().unwrap();
        let base_path = temp_dir.path().join("event_log");
        let log = ReplayableEventLog::new(&base_path);

        // Create a recent event
        let recent_entry = EventLogEntry::new(
            EventType::Observation,
            serde_json::json!({"text": "recent"}),
        );
        assert_eq!(log.tier_for_event(&recent_entry), RetentionTier::Hot);

        // Create an old event (simulate 45 days old)
        let mut old_entry =
            EventLogEntry::new(EventType::Observation, serde_json::json!({"text": "old"}));
        old_entry.timestamp = Utc::now() - chrono::Duration::days(45);
        assert_eq!(log.tier_for_event(&old_entry), RetentionTier::Cold);
    }

    #[test]
    fn test_replayable_event_log_write_and_replay() {
        let temp_dir = TempDir::new().unwrap();
        let base_path = temp_dir.path().join("event_log");

        let log = ReplayableEventLog::new(&base_path);
        log.init().unwrap();

        // Write some events
        let entry1 = EventLogEntry::new(
            EventType::ToolInvocation,
            serde_json::json!({"tool_name": "test_tool"}),
        )
        .with_task_id(TaskId::new());
        log.write_event(&entry1).unwrap();

        let entry2 = EventLogEntry::new(
            EventType::Decision,
            serde_json::json!({"decision": "test_decision"}),
        )
        .with_agent_id(AgentId::new());
        log.write_event(&entry2).unwrap();

        // Replay all events
        let events: Vec<_> = log.replay().unwrap().collect();
        assert_eq!(events.len(), 2);

        let e1 = events[0].as_ref().unwrap();
        assert_eq!(e1.schema_version, "1.0");
        assert!(matches!(e1.event_type, EventType::ToolInvocation));

        let e2 = events[1].as_ref().unwrap();
        assert!(matches!(e2.event_type, EventType::Decision));
    }

    #[test]
    fn test_replayable_event_log_tier_stats() {
        let temp_dir = TempDir::new().unwrap();
        let base_path = temp_dir.path().join("event_log");

        let log = ReplayableEventLog::new(&base_path);
        log.init().unwrap();

        // Write a few events
        for i in 0..3 {
            let entry = EventLogEntry::new(EventType::Observation, serde_json::json!({"index": i}));
            log.write_event(&entry).unwrap();
        }

        let stats = log.tier_stats().unwrap();

        // All recent events should be in hot tier
        assert_eq!(stats[&RetentionTier::Hot].event_count, 3);
        assert_eq!(stats[&RetentionTier::Warm].event_count, 0); // empty
        assert_eq!(stats[&RetentionTier::Cold].event_count, 0); // empty
    }

    // ============================================================
    // Session state reconstruction tests
    // ============================================================

    #[test]
    fn test_session_state_new() {
        let state = SessionState::new();
        assert!(state.session_id.is_none());
        assert_eq!(state.current_state, "");
        assert!(state.task_states.is_empty());
        assert_eq!(state.error_count, 0);
        assert_eq!(state.events_processed, 0);
    }

    #[test]
    fn test_session_state_process_state_transition() {
        let mut state = SessionState::new();

        let entry = EventLogEntry::new(
            EventType::StateTransition,
            serde_json::json!({"from_state": "CREATED", "to_state": "EXECUTING"}),
        )
        .with_task_id(TaskId::new());

        state.process_event(&entry);
        assert_eq!(state.current_state, "EXECUTING");
        assert_eq!(state.events_processed, 1);
    }

    #[test]
    fn test_session_state_process_tool_invocation() {
        let mut state = SessionState::new();
        let task_id = TaskId::new();

        let entry = EventLogEntry::new(
            EventType::ToolInvocation,
            serde_json::json!({
                "tool_name": "file_read",
                "success": true,
                "duration_ms": 100
            }),
        )
        .with_task_id(task_id);

        state.process_event(&entry);

        let invocations = state.tool_invocations.get(&task_id).unwrap();
        assert_eq!(invocations.len(), 1);
        assert_eq!(invocations[0].tool_name, "file_read");
        assert!(invocations[0].success);
    }

    #[test]
    fn test_session_state_process_decision() {
        let mut state = SessionState::new();
        let agent_id = AgentId::new();

        let entry = EventLogEntry::new(
            EventType::Decision,
            serde_json::json!({
                "decision": "use refactoring",
                "reasoning": "code is complex"
            }),
        )
        .with_agent_id(agent_id);

        state.process_event(&entry);

        assert_eq!(state.decisions.len(), 1);
        assert_eq!(state.decisions[0].decision, "use refactoring");
        assert_eq!(state.decisions[0].agent_id, Some(agent_id));
    }

    #[test]
    fn test_session_state_process_llm_call() {
        let mut state = SessionState::new();

        let entry = EventLogEntry::new(
            EventType::LlmCall,
            serde_json::json!({
                "model": "claude-3-5-sonnet",
                "input_tokens": 100,
                "output_tokens": 50,
                "success": true
            }),
        );

        state.process_event(&entry);

        assert_eq!(state.llm_calls.len(), 1);
        assert_eq!(state.llm_calls[0].model, "claude-3-5-sonnet");
        assert_eq!(state.llm_calls[0].input_tokens, 100);
        assert_eq!(state.llm_calls[0].output_tokens, 50);
    }

    #[test]
    fn test_session_state_process_outcome() {
        let mut state = SessionState::new();
        let task_id = TaskId::new();

        let entry = EventLogEntry::new(
            EventType::Outcome,
            serde_json::json!({
                "success": true,
                "summary": "task completed"
            }),
        )
        .with_task_id(task_id);

        state.process_event(&entry);

        let outcome = state.outcomes.get(&task_id).unwrap();
        assert!(outcome.success);
        assert_eq!(outcome.summary, "task completed");
    }

    #[test]
    fn test_session_state_process_error() {
        let mut state = SessionState::new();

        let entry = EventLogEntry::new(
            EventType::Error,
            serde_json::json!({
                "error_type": "FileNotFound",
                "message": "file not found"
            }),
        );

        state.process_event(&entry);

        assert_eq!(state.error_count, 1);
    }

    #[test]
    fn test_session_state_process_multiple_events() {
        let mut state = SessionState::new();
        let task_id = TaskId::new();

        // Process state transition
        let entry1 = EventLogEntry::new(
            EventType::StateTransition,
            serde_json::json!({"to_state": "EXECUTING"}),
        )
        .with_task_id(task_id);
        state.process_event(&entry1);

        // Process tool invocation
        let entry2 = EventLogEntry::new(
            EventType::ToolInvocation,
            serde_json::json!({"tool_name": "shell", "success": true}),
        )
        .with_task_id(task_id);
        state.process_event(&entry2);

        // Process outcome
        let entry3 = EventLogEntry::new(
            EventType::Outcome,
            serde_json::json!({"success": true, "summary": "done"}),
        )
        .with_task_id(task_id);
        state.process_event(&entry3);

        assert_eq!(state.events_processed, 3);
        assert_eq!(state.current_state, "EXECUTING");
        assert_eq!(
            state.task_states.get(&task_id),
            Some(&"EXECUTING".to_string())
        );
        assert_eq!(state.tool_invocations.get(&task_id).unwrap().len(), 1);
        assert!(state.outcomes.get(&task_id).unwrap().success);
    }

    #[test]
    fn test_session_state_replay_events() {
        let entries = vec![
            EventLogEntry::new(
                EventType::StateTransition,
                serde_json::json!({"to_state": "EXECUTING"}),
            )
            .with_task_id(TaskId::new()),
            EventLogEntry::new(
                EventType::ToolInvocation,
                serde_json::json!({"tool_name": "test", "success": true}),
            ),
        ];

        let events = entries.into_iter().map(|e| Ok(e));
        let state = SessionState::replay_events(events).unwrap();

        assert_eq!(state.events_processed, 2);
    }

    #[test]
    fn test_session_state_correlation_tracking() {
        let mut state = SessionState::new();
        let correlation_id = Uuid::new_v4();

        let entry = EventLogEntry::new(EventType::LlmCall, serde_json::json!({"model": "test"}))
            .with_correlation_id(correlation_id);

        state.process_event(&entry);

        assert!(state.events_by_correlation.contains_key(&correlation_id));
        assert_eq!(state.events_by_correlation[&correlation_id].len(), 1);
    }

    #[test]
    fn test_replayable_event_log_replay_for_session() {
        let temp_dir = TempDir::new().unwrap();
        let base_path = temp_dir.path().join("event_log");
        let session_id = SessionId::new();

        let log = ReplayableEventLog::new(&base_path);
        log.init().unwrap();

        // Write events with different session IDs
        let entry1 = EventLogEntry::new(
            EventType::Observation,
            serde_json::json!({"text": "session1_event"}),
        )
        .with_session_id(session_id);
        log.write_event(&entry1).unwrap();

        let entry2 = EventLogEntry::new(
            EventType::Observation,
            serde_json::json!({"text": "other_session"}),
        )
        .with_session_id(SessionId::new()); // Different session
        log.write_event(&entry2).unwrap();

        // Replay for specific session
        let state = log.replay_for_session(session_id).unwrap();
        assert_eq!(state.events_processed, 1);
    }

    #[test]
    fn test_replayable_event_log_replay_for_task() {
        let temp_dir = TempDir::new().unwrap();
        let base_path = temp_dir.path().join("event_log");
        let task_id = TaskId::new();

        let log = ReplayableEventLog::new(&base_path);
        log.init().unwrap();

        // Write events with different task IDs
        let entry1 = EventLogEntry::new(
            EventType::ToolInvocation,
            serde_json::json!({"tool_name": "tool_a"}),
        )
        .with_task_id(task_id);
        log.write_event(&entry1).unwrap();

        let entry2 = EventLogEntry::new(
            EventType::ToolInvocation,
            serde_json::json!({"tool_name": "tool_b"}),
        )
        .with_task_id(TaskId::new()); // Different task
        log.write_event(&entry2).unwrap();

        // Replay for specific task
        let state = log.replay_for_task(task_id).unwrap();
        assert_eq!(state.events_processed, 1);
        let invocations = state.tool_invocations.get(&task_id).unwrap();
        assert_eq!(invocations.len(), 1);
        assert_eq!(invocations[0].tool_name, "tool_a");
    }

    #[test]
    fn test_jsonl_format_with_schema_version() {
        let temp_dir = TempDir::new().unwrap();
        let base_path = temp_dir.path().join("event_log");

        let log = ReplayableEventLog::new(&base_path);
        log.init().unwrap();

        let entry = EventLogEntry::new(EventType::Observation, serde_json::json!({"key": "value"}));
        log.write_event(&entry).unwrap();

        // Read the file and verify JSONL format
        let hot_path = log.tier_path(RetentionTier::Hot);
        let files: Vec<_> = fs::read_dir(hot_path).unwrap().collect();
        assert!(!files.is_empty());

        let content = fs::read_to_string(files[0].as_ref().unwrap().path()).unwrap();
        let line = content.trim();

        // Verify it's valid JSON with schema_version field
        assert!(line.starts_with("{"));
        assert!(line.ends_with("}"));
        assert!(line.contains("\"schema_version\""));
        assert!(line.contains("\"1.0\""));

        // Verify no newlines within the JSON
        assert!(!line.contains("\n"));
    }
}
