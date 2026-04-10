//! Hash-chained audit trail for EU AI Act Article 12 compliance.
//!
//! This module provides an append-only, hash-chained audit log that records:
//! - Tool call events with actor, target, arguments, result, duration
//! - External write events when agents modify files
//! - Decision events when agents make significant choices
//! - State transitions (task state changes, mode switches)
//!
//! The log is stored as NDJSON at `audit/swell-actions.ndjson` with sha256 hash chaining
//! for tamper detection.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;

use crate::TaskState;

/// Audit log file name
const AUDIT_FILE: &str = "audit/swell-actions.ndjson";
/// Genesis hash for the first entry
pub const GENESIS_HASH: &str =
    "sha256:0000000000000000000000000000000000000000000000000000000000000000";

/// Event kinds that can be recorded in the audit log
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AuditEventKind {
    /// Any tool invocation
    ToolCall,
    /// Write to external system (file, API, DB)
    ExternalWrite,
    /// Agent decision with reasoning
    Decision,
    /// Safety override applied
    Override,
    /// External input received
    Ingress,
    /// Agent session initialized
    SessionStart,
    /// Agent session terminated
    SessionEnd,
    /// Behavior surface change
    StateTransition,
}

/// Plane label for categorizing events
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuditPlane {
    Ingress,
    Interpretation,
    Decision,
    Action,
}

/// Gate/truth gate applied to the event
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AuditGate {
    None,
    ExternalWrite,
    CredentialAccess,
    InstallExtend,
}

/// An audit log entry with hash chaining
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    /// ISO-8601 timestamp with timezone offset (UTC)
    pub ts: String,
    /// Event type/kind
    pub kind: AuditEventKind,
    /// Agent or component that triggered the event
    pub actor: String,
    /// Domain partition (e.g., "swell", "agent")
    pub domain: String,
    /// Four-plane label
    pub plane: AuditPlane,
    /// Truth gate applied
    pub gate: AuditGate,
    /// Monotonically increasing sequence number
    pub ord: u64,
    /// Source session or external identity
    pub provenance: String,
    /// File, URL, or resource affected
    pub target: String,
    /// Human-readable description of the event
    pub summary: String,
    /// sha256 of the previous log entry (hex, prefixed `sha256:`)
    pub prev_hash: String,
    /// sha256 of this entry excluding the `hash` field itself
    pub hash: String,
    /// Optional: tool call arguments (JSON string)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<String>,
    /// Optional: tool call result (JSON string)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    /// Optional: duration in milliseconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// Optional: previous state (for state transitions)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_state: Option<TaskState>,
    /// Optional: new state (for state transitions)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_state: Option<TaskState>,
}

impl AuditEntry {
    /// Compute the sha256 hash of this entry (excluding the hash field itself)
    fn compute_hash(&self) -> String {
        // Clone without hash for serialization
        let entry_without_hash = Self {
            hash: String::new(),
            ..self.clone()
        };
        let raw = serde_json::to_string(&entry_without_hash).expect("audit entry should serialize");
        let mut hasher = Sha256::new();
        hasher.update(raw.as_bytes());
        format!("sha256:{:x}", hasher.finalize())
    }
}

/// Result of chain integrity verification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChainVerificationResult {
    /// Whether the chain is intact
    pub valid: bool,
    /// Number of entries verified
    pub entries_verified: u64,
    /// Error message if chain is broken
    pub error: Option<String>,
    /// Index of first broken entry if any
    pub broken_at: Option<u64>,
}

/// The audit log manager
#[derive(Debug, Clone)]
pub struct AuditLog {
    /// Path to the audit log file
    log_path: std::path::PathBuf,
    /// Cached last hash
    last_hash: String,
    /// Cached last sequence number
    last_ord: u64,
}

impl AuditLog {
    /// Create a new AuditLog, creating the directory and file if needed.
    ///
    /// # Arguments
    /// * `log_path` - Path to the audit log file (e.g., "audit/swell-actions.ndjson")
    ///
    /// # Returns
    /// A new AuditLog instance, or an error if the log cannot be created/opened.
    pub fn new<P: AsRef<Path>>(log_path: P) -> io::Result<Self> {
        let log_path = log_path.as_ref();
        let dir = log_path
            .parent()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "log path has no parent"))?;

        // Create audit directory if it doesn't exist
        fs::create_dir_all(dir)?;

        // Create log file if it doesn't exist
        if !log_path.exists() {
            fs::write(log_path, "")?;
        }

        // Initialize with genesis if empty, otherwise read last entry
        let (last_hash, last_ord) = Self::read_last_entry(log_path)?;

        Ok(Self {
            log_path: log_path.to_path_buf(),
            last_hash,
            last_ord,
        })
    }

    /// Create an AuditLog at the default path (audit/swell-actions.ndjson)
    pub fn with_default_path() -> io::Result<Self> {
        Self::new(AUDIT_FILE)
    }

    /// Read the last entry from the log file to get last_hash and last_ord
    fn read_last_entry(path: &Path) -> io::Result<(String, u64)> {
        let file = fs::File::open(path)?;
        let reader = BufReader::new(file);
        let lines = reader.lines();

        // Find the last line
        let mut last_line: Option<String> = None;
        for line in lines {
            last_line = Some(line?);
        }

        match last_line {
            Some(line) if !line.trim().is_empty() => {
                let entry: AuditEntry = serde_json::from_str(&line)
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
                Ok((entry.hash.clone(), entry.ord))
            }
            _ => Ok((GENESIS_HASH.to_string(), 0)),
        }
    }

    /// Append a new entry to the audit log.
    ///
    /// # Arguments
    /// * `entry` - The audit entry to append (hash will be computed automatically)
    ///
    /// # Returns
    /// The appended entry with computed hash, or an error if append fails.
    pub fn append(&mut self, mut entry: AuditEntry) -> io::Result<AuditEntry> {
        // Set chain fields
        entry.prev_hash = self.last_hash.clone();
        entry.ord = self.last_ord + 1;

        // Compute hash of entry without the hash field
        entry.hash = entry.compute_hash();

        // Append to file
        let mut file = OpenOptions::new()
            .create(false)
            .append(true)
            .open(&self.log_path)?;
        let line = serde_json::to_string(&entry)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        writeln!(file, "{}", line)?;

        // Update cached state
        self.last_hash = entry.hash.clone();
        self.last_ord = entry.ord;

        Ok(entry)
    }

    /// Log a tool call event.
    pub fn log_tool_call(
        &mut self,
        actor: &str,
        target: &str,
        arguments: Option<serde_json::Value>,
        result: Option<serde_json::Value>,
        duration_ms: u64,
        provenance: &str,
    ) -> io::Result<AuditEntry> {
        let entry = AuditEntry {
            ts: Utc::now().to_rfc3339(),
            kind: AuditEventKind::ToolCall,
            actor: actor.to_string(),
            domain: "swell".to_string(),
            plane: AuditPlane::Action,
            gate: AuditGate::None,
            ord: 0, // Will be set by append
            provenance: provenance.to_string(),
            target: target.to_string(),
            summary: format!("Tool {} called on {}", actor, target),
            prev_hash: String::new(), // Will be set by append
            hash: String::new(),      // Will be set by append
            arguments: arguments.map(|v| v.to_string()),
            result: result.map(|v| v.to_string()),
            duration_ms: Some(duration_ms),
            from_state: None,
            to_state: None,
        };
        self.append(entry)
    }

    /// Log an external write event when agents modify files.
    pub fn log_external_write(
        &mut self,
        actor: &str,
        target: &str,
        summary: &str,
        provenance: &str,
    ) -> io::Result<AuditEntry> {
        let entry = AuditEntry {
            ts: Utc::now().to_rfc3339(),
            kind: AuditEventKind::ExternalWrite,
            actor: actor.to_string(),
            domain: "swell".to_string(),
            plane: AuditPlane::Action,
            gate: AuditGate::ExternalWrite,
            ord: 0,
            provenance: provenance.to_string(),
            target: target.to_string(),
            summary: summary.to_string(),
            prev_hash: String::new(),
            hash: String::new(),
            arguments: None,
            result: None,
            duration_ms: None,
            from_state: None,
            to_state: None,
        };
        self.append(entry)
    }

    /// Log a decision event when agents make significant choices.
    pub fn log_decision(
        &mut self,
        actor: &str,
        target: &str,
        summary: &str,
        provenance: &str,
    ) -> io::Result<AuditEntry> {
        let entry = AuditEntry {
            ts: Utc::now().to_rfc3339(),
            kind: AuditEventKind::Decision,
            actor: actor.to_string(),
            domain: "swell".to_string(),
            plane: AuditPlane::Decision,
            gate: AuditGate::None,
            ord: 0,
            provenance: provenance.to_string(),
            target: target.to_string(),
            summary: summary.to_string(),
            prev_hash: String::new(),
            hash: String::new(),
            arguments: None,
            result: None,
            duration_ms: None,
            from_state: None,
            to_state: None,
        };
        self.append(entry)
    }

    /// Log a state transition (task state changes, mode switches).
    pub fn log_state_transition(
        &mut self,
        actor: &str,
        target: &str,
        from_state: TaskState,
        to_state: TaskState,
        provenance: &str,
    ) -> io::Result<AuditEntry> {
        let entry = AuditEntry {
            ts: Utc::now().to_rfc3339(),
            kind: AuditEventKind::StateTransition,
            actor: actor.to_string(),
            domain: "swell".to_string(),
            plane: AuditPlane::Decision,
            gate: AuditGate::None,
            ord: 0,
            provenance: provenance.to_string(),
            target: target.to_string(),
            summary: format!("State transition from {:?} to {:?}", from_state, to_state),
            prev_hash: String::new(),
            hash: String::new(),
            arguments: None,
            result: None,
            duration_ms: None,
            from_state: Some(from_state),
            to_state: Some(to_state),
        };
        self.append(entry)
    }

    /// Log a session start event.
    pub fn log_session_start(&mut self, actor: &str, provenance: &str) -> io::Result<AuditEntry> {
        let entry = AuditEntry {
            ts: Utc::now().to_rfc3339(),
            kind: AuditEventKind::SessionStart,
            actor: actor.to_string(),
            domain: "swell".to_string(),
            plane: AuditPlane::Ingress,
            gate: AuditGate::None,
            ord: 0,
            provenance: provenance.to_string(),
            target: "session".to_string(),
            summary: format!("Session started for {}", actor),
            prev_hash: String::new(),
            hash: String::new(),
            arguments: None,
            result: None,
            duration_ms: None,
            from_state: None,
            to_state: None,
        };
        self.append(entry)
    }

    /// Log a session end event.
    pub fn log_session_end(&mut self, actor: &str, provenance: &str) -> io::Result<AuditEntry> {
        let entry = AuditEntry {
            ts: Utc::now().to_rfc3339(),
            kind: AuditEventKind::SessionEnd,
            actor: actor.to_string(),
            domain: "swell".to_string(),
            plane: AuditPlane::Ingress,
            gate: AuditGate::None,
            ord: 0,
            provenance: provenance.to_string(),
            target: "session".to_string(),
            summary: format!("Session ended for {}", actor),
            prev_hash: String::new(),
            hash: String::new(),
            arguments: None,
            result: None,
            duration_ms: None,
            from_state: None,
            to_state: None,
        };
        self.append(entry)
    }

    /// Verify the integrity of the hash chain.
    ///
    /// # Returns
    /// A ChainVerificationResult indicating whether the chain is valid.
    pub fn verify_chain(&self) -> ChainVerificationResult {
        let path = &self.log_path;

        if !path.exists() {
            return ChainVerificationResult {
                valid: true,
                entries_verified: 0,
                error: None,
                broken_at: None,
            };
        }

        let file = match fs::File::open(path) {
            Ok(f) => f,
            Err(e) => {
                return ChainVerificationResult {
                    valid: false,
                    entries_verified: 0,
                    error: Some(format!("Failed to open log file: {}", e)),
                    broken_at: None,
                };
            }
        };

        let reader = BufReader::new(file);
        let lines = reader.lines();
        let mut prev_hash = GENESIS_HASH.to_string();
        let mut entries_verified: u64 = 0;

        for (i, line) in lines.enumerate() {
            let line = match line {
                Ok(l) => l,
                Err(e) => {
                    return ChainVerificationResult {
                        valid: false,
                        entries_verified,
                        error: Some(format!("Failed to read line {}: {}", i, e)),
                        broken_at: Some(i as u64),
                    };
                }
            };

            if line.trim().is_empty() {
                continue;
            }

            let entry: AuditEntry = match serde_json::from_str(&line) {
                Ok(e) => e,
                Err(e) => {
                    return ChainVerificationResult {
                        valid: false,
                        entries_verified,
                        error: Some(format!("Failed to parse entry {}: {}", i, e)),
                        broken_at: Some(i as u64),
                    };
                }
            };

            // Verify prev_hash matches expected
            if entry.prev_hash != prev_hash {
                return ChainVerificationResult {
                    valid: false,
                    entries_verified,
                    error: Some(format!(
                        "PREV_HASH MISMATCH at entry {} (ord={}): expected {}, got {}",
                        i, entry.ord, prev_hash, entry.prev_hash
                    )),
                    broken_at: Some(i as u64),
                };
            }

            // Verify hash is correct (recompute and compare)
            let entry_without_hash = AuditEntry {
                hash: String::new(),
                ..entry.clone()
            };
            let raw = match serde_json::to_string(&entry_without_hash) {
                Ok(r) => r,
                Err(e) => {
                    return ChainVerificationResult {
                        valid: false,
                        entries_verified,
                        error: Some(format!("Failed to serialize entry {}: {}", i, e)),
                        broken_at: Some(i as u64),
                    };
                }
            };
            let mut hasher = Sha256::new();
            hasher.update(raw.as_bytes());
            let computed_hash = format!("sha256:{:x}", hasher.finalize());

            if computed_hash != entry.hash {
                return ChainVerificationResult {
                    valid: false,
                    entries_verified,
                    error: Some(format!(
                        "HASH MISMATCH at entry {} (ord={}): expected {}, got {}",
                        i, entry.ord, computed_hash, entry.hash
                    )),
                    broken_at: Some(i as u64),
                };
            }

            prev_hash = entry.hash.clone();
            entries_verified += 1;
        }

        ChainVerificationResult {
            valid: true,
            entries_verified,
            error: None,
            broken_at: None,
        }
    }

    /// Get the path to the audit log file.
    pub fn log_path(&self) -> &Path {
        &self.log_path
    }

    /// Get the current sequence number (last entry's ord).
    pub fn last_ord(&self) -> u64 {
        self.last_ord
    }

    /// Get the current last hash.
    pub fn last_hash(&self) -> &str {
        &self.last_hash
    }
}

/// Verify chain integrity for a log file at the given path.
pub fn verify_audit_chain<P: AsRef<Path>>(path: P) -> ChainVerificationResult {
    match AuditLog::new(path) {
        Ok(log) => log.verify_chain(),
        Err(e) => ChainVerificationResult {
            valid: false,
            entries_verified: 0,
            error: Some(format!("Failed to open audit log: {}", e)),
            broken_at: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn create_temp_log() -> (AuditLog, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let log_path = temp_dir.path().join("swell-actions.ndjson");
        let log = AuditLog::new(&log_path).unwrap();
        (log, temp_dir)
    }

    #[test]
    fn test_audit_log_creation() {
        let (log, _temp) = create_temp_log();
        assert_eq!(log.last_hash(), GENESIS_HASH);
        assert_eq!(log.last_ord(), 0);
    }

    #[test]
    fn test_append_tool_call_entry() {
        let (mut log, temp) = create_temp_log();

        let entry = log
            .log_tool_call(
                "planner",
                "file:///workspace/src/main.rs",
                Some(serde_json::json!({"action": "read"})),
                Some(serde_json::json!({"content": "..."})),
                150,
                "session-123",
            )
            .unwrap();

        assert_eq!(entry.ord, 1);
        assert_eq!(entry.prev_hash, GENESIS_HASH);
        assert!(entry.hash.starts_with("sha256:"));
        assert_eq!(entry.kind, AuditEventKind::ToolCall);
        assert_eq!(entry.actor, "planner");
        assert_eq!(entry.duration_ms, Some(150));

        // Verify file was written
        let content = fs::read_to_string(temp.path().join("swell-actions.ndjson")).unwrap();
        assert!(!content.is_empty());
    }

    #[test]
    fn test_hash_chain_integrity() {
        let (mut log, _temp) = create_temp_log();

        // Append multiple entries
        let entry1 = log
            .log_tool_call("actor1", "target1", None, None, 100, "session-1")
            .unwrap();
        let entry2 = log
            .log_tool_call("actor2", "target2", None, None, 200, "session-1")
            .unwrap();
        let entry3 = log
            .log_external_write("actor3", "target3", "wrote file", "session-1")
            .unwrap();

        // Verify chain
        let result = log.verify_chain();
        assert!(result.valid);
        assert_eq!(result.entries_verified, 3);

        // Verify entry2's prev_hash is entry1's hash
        assert_eq!(entry2.prev_hash, entry1.hash);
        // Verify entry3's prev_hash is entry2's hash
        assert_eq!(entry3.prev_hash, entry2.hash);
    }

    #[test]
    fn test_log_state_transition() {
        let (mut log, _temp) = create_temp_log();

        let entry = log
            .log_state_transition(
                "orchestrator",
                "task-123",
                TaskState::Created,
                TaskState::Executing,
                "session-456",
            )
            .unwrap();

        assert_eq!(entry.kind, AuditEventKind::StateTransition);
        assert_eq!(entry.from_state, Some(TaskState::Created));
        assert_eq!(entry.to_state, Some(TaskState::Executing));
        assert_eq!(entry.ord, 1);
    }

    #[test]
    fn test_log_decision() {
        let (mut log, _temp) = create_temp_log();

        let entry = log
            .log_decision(
                "evaluator",
                "task-789",
                "Selected fix方案B for bug #42",
                "session-789",
            )
            .unwrap();

        assert_eq!(entry.kind, AuditEventKind::Decision);
        assert_eq!(entry.actor, "evaluator");
        assert_eq!(entry.ord, 1);
    }

    #[test]
    fn test_tamper_detection() {
        let (mut log, temp) = create_temp_log();

        // Append an entry
        log.log_tool_call("actor", "target", None, None, 100, "session")
            .unwrap();

        // Tamper with the file (change a character)
        let file_path = temp.path().join("swell-actions.ndjson");
        let content = fs::read_to_string(&file_path).unwrap();
        let tampered = content.replace("actor", "ACTTOR");
        fs::write(&file_path, tampered).unwrap();

        // Verify should fail
        let result = log.verify_chain();
        assert!(!result.valid);
        assert!(result.error.is_some());
    }

    #[test]
    fn test_verify_chain_from_file() {
        let (mut log, temp) = create_temp_log();

        log.log_tool_call("actor1", "target1", None, None, 100, "session")
            .unwrap();
        log.log_tool_call("actor2", "target2", None, None, 100, "session")
            .unwrap();

        let file_path = temp.path().join("swell-actions.ndjson");
        let result = verify_audit_chain(&file_path);
        assert!(result.valid);
        assert_eq!(result.entries_verified, 2);
    }

    #[test]
    fn test_session_events() {
        let (mut log, _temp) = create_temp_log();

        let start = log.log_session_start("agent-1", "session-abc").unwrap();
        assert_eq!(start.kind, AuditEventKind::SessionStart);
        assert_eq!(start.ord, 1);

        let end = log.log_session_end("agent-1", "session-abc").unwrap();
        assert_eq!(end.kind, AuditEventKind::SessionEnd);
        assert_eq!(end.ord, 2);
        // Verify chain
        assert_eq!(end.prev_hash, start.hash);
    }

    #[test]
    fn test_audit_gate_serialization() {
        let gate = AuditGate::ExternalWrite;
        let serialized = serde_json::to_string(&gate).unwrap();
        assert_eq!(serialized, "\"external-write\"");

        let deserialized: AuditGate = serde_json::from_str(&serialized).unwrap();
        assert_eq!(deserialized, gate);
    }

    #[test]
    fn test_audit_plane_serialization() {
        let plane = AuditPlane::Action;
        let serialized = serde_json::to_string(&plane).unwrap();
        assert_eq!(serialized, "\"action\"");

        let deserialized: AuditPlane = serde_json::from_str(&serialized).unwrap();
        assert_eq!(deserialized, plane);
    }
}
