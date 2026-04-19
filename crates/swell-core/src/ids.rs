//! Newtype wrappers for domain identifiers.
//!
//! These newtypes provide type safety by wrapping raw UUIDs and Strings with
//! named domain concepts. Each newtype enforces correct usage and prevents
//! mixing up IDs of different domains.
//!
//! # Design
//!
//! - All newtypes use `#[serde(transparent)]` so they serialize as their inner type
//! - Named accessors (`from_uuid`/`as_uuid` for Uuid-backed, `from_str`/`as_str` for String-backed)
//! - No `From` implementations to prevent implicit conversions - use accessors explicitly
//! - Implements `Display` and `FromStr` for ergonomic string handling

use std::fmt::Display;
use std::hash::Hash;
use std::path::PathBuf;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ----------------------------------------------------------------------------
// TaskId
// ----------------------------------------------------------------------------

/// Newtype wrapper for task identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TaskId(Uuid);

impl TaskId {
    /// Create a new TaskId with a random UUID.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Create a TaskId from a UUID.
    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }

    /// Create a nil (zero) TaskId.
    pub fn nil() -> Self {
        Self(Uuid::nil())
    }

    /// Get the underlying UUID.
    pub fn as_uuid(&self) -> Uuid {
        self.0
    }

    /// Check if this is a nil (zero) TaskId.
    pub fn is_nil(&self) -> bool {
        self.0.is_nil()
    }
}

impl Default for TaskId {
    fn default() -> Self {
        Self::new()
    }
}

impl Display for TaskId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for TaskId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self::from_uuid(s.parse()?))
    }
}

// ----------------------------------------------------------------------------
// AgentId
// ----------------------------------------------------------------------------

/// Newtype wrapper for agent identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AgentId(Uuid);

impl AgentId {
    /// Create a new AgentId with a random UUID.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Create an AgentId from a UUID.
    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }

    /// Get the underlying UUID.
    pub fn as_uuid(&self) -> Uuid {
        self.0
    }

    /// Check if this is a nil (zero) AgentId.
    pub fn is_nil(&self) -> bool {
        self.0.is_nil()
    }
}

impl Default for AgentId {
    fn default() -> Self {
        Self::new()
    }
}

impl Display for AgentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for AgentId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self::from_uuid(s.parse()?))
    }
}

// ----------------------------------------------------------------------------
// WorktreeId
// ----------------------------------------------------------------------------

/// Newtype wrapper for worktree identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WorktreeId(Uuid);

impl WorktreeId {
    /// Create a new WorktreeId with a random UUID.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Create a WorktreeId from a UUID.
    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }

    /// Get the underlying UUID.
    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl Default for WorktreeId {
    fn default() -> Self {
        Self::new()
    }
}

impl Display for WorktreeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for WorktreeId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self::from_uuid(s.parse()?))
    }
}

// ----------------------------------------------------------------------------
// BranchName
// ----------------------------------------------------------------------------

/// Newtype wrapper for git branch names.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BranchName(String);

impl BranchName {
    /// Create a new BranchName from a String.
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    /// Create a BranchName from a string slice.
    pub fn from_string(s: &str) -> Self {
        Self(s.to_string())
    }

    /// Get the underlying String.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for BranchName {
    fn default() -> Self {
        Self("main".to_string())
    }
}

impl Display for BranchName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for BranchName {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self::from_string(s))
    }
}

// ----------------------------------------------------------------------------
// CommitSha
// ----------------------------------------------------------------------------

/// Newtype wrapper for git commit SHAs.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CommitSha(String);

impl CommitSha {
    /// Create a new CommitSha from a String.
    pub fn new(sha: impl Into<String>) -> Self {
        Self(sha.into())
    }

    /// Create a CommitSha from a string slice.
    pub fn from_string(s: &str) -> Self {
        Self(s.to_string())
    }

    /// Get the underlying String.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for CommitSha {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for CommitSha {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self::from_string(s))
    }
}

// ----------------------------------------------------------------------------
// FeatureLeadId
// ----------------------------------------------------------------------------

/// Newtype wrapper for feature lead identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct FeatureLeadId(Uuid);

impl FeatureLeadId {
    /// Create a new FeatureLeadId with a random UUID.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Create a FeatureLeadId from a UUID.
    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }

    /// Get the underlying UUID.
    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl Default for FeatureLeadId {
    fn default() -> Self {
        Self::new()
    }
}

impl Display for FeatureLeadId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for FeatureLeadId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self::from_uuid(s.parse()?))
    }
}

// ----------------------------------------------------------------------------
// CheckpointId
// ----------------------------------------------------------------------------

/// Newtype wrapper for checkpoint identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CheckpointId(Uuid);

impl CheckpointId {
    /// Create a new CheckpointId with a random UUID.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Create a CheckpointId from a UUID.
    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }

    /// Get the underlying UUID.
    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl Default for CheckpointId {
    fn default() -> Self {
        Self::new()
    }
}

impl Display for CheckpointId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for CheckpointId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self::from_uuid(s.parse()?))
    }
}

// ----------------------------------------------------------------------------
// SessionId
// ----------------------------------------------------------------------------

/// Newtype wrapper for session identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SessionId(Uuid);

impl SessionId {
    /// Create a new SessionId with a random UUID.
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Create a SessionId from a UUID.
    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }

    /// Get the underlying UUID.
    pub fn as_uuid(&self) -> Uuid {
        self.0
    }

    /// Check if this is a nil (zero) SessionId.
    pub fn is_nil(&self) -> bool {
        self.0.is_nil()
    }
}

impl Default for SessionId {
    fn default() -> Self {
        Self::new()
    }
}

impl Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for SessionId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self::from_uuid(s.parse()?))
    }
}

// ----------------------------------------------------------------------------
// SocketPath
// ----------------------------------------------------------------------------

/// Newtype wrapper for Unix socket paths.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SocketPath(PathBuf);

impl SocketPath {
    /// Create a new SocketPath from a PathBuf.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self(path.into())
    }

    /// Create a SocketPath from a string slice.
    pub fn from_string(s: &str) -> Self {
        Self(PathBuf::from(s))
    }

    /// Get the underlying PathBuf.
    pub fn as_path_buf(&self) -> &PathBuf {
        &self.0
    }
}

impl Display for SocketPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0.display())
    }
}

impl FromStr for SocketPath {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self::from_string(s))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_task_id_new_and_accessors() {
        let id = TaskId::new();
        assert!(id.as_uuid() != Uuid::nil());
    }

    #[test]
    fn test_task_id_from_uuid() {
        let uuid = Uuid::new_v4();
        let id = TaskId::from_uuid(uuid);
        assert_eq!(id.as_uuid(), uuid);
    }

    #[test]
    fn test_task_id_display() {
        let id = TaskId::from_uuid(Uuid::nil());
        assert_eq!(format!("{}", id), Uuid::nil().to_string());
    }

    #[test]
    fn test_task_id_from_str() {
        let uuid = Uuid::new_v4();
        let id: TaskId = uuid.to_string().parse().unwrap();
        assert_eq!(id.as_uuid(), uuid);
    }

    #[test]
    fn test_agent_id_new() {
        let id = AgentId::new();
        assert!(id.as_uuid() != Uuid::nil());
    }

    #[test]
    fn test_worktree_id_new() {
        let id = WorktreeId::new();
        assert!(id.as_uuid() != Uuid::nil());
    }

    #[test]
    fn test_feature_lead_id_new() {
        let id = FeatureLeadId::new();
        assert!(id.as_uuid() != Uuid::nil());
    }

    #[test]
    fn test_checkpoint_id_new() {
        let id = CheckpointId::new();
        assert!(id.as_uuid() != Uuid::nil());
    }

    #[test]
    fn test_session_id_new() {
        let id = SessionId::new();
        assert!(id.as_uuid() != Uuid::nil());
    }

    #[test]
    fn test_branch_name_new() {
        let name = BranchName::new("feature/test");
        assert_eq!(name.as_str(), "feature/test");
    }

    #[test]
    fn test_branch_name_from_string() {
        let name = BranchName::from_string("main");
        assert_eq!(name.as_str(), "main");
    }

    #[test]
    fn test_branch_name_display() {
        let name = BranchName::new("develop");
        assert_eq!(format!("{}", name), "develop");
    }

    #[test]
    fn test_commit_sha_new() {
        let sha = CommitSha::new("abc123");
        assert_eq!(sha.as_str(), "abc123");
    }

    #[test]
    fn test_commit_sha_display() {
        let sha = CommitSha::new("def456");
        assert_eq!(format!("{}", sha), "def456");
    }

    #[test]
    fn test_socket_path_new() {
        let path = SocketPath::new("/tmp/swell.sock");
        assert_eq!(path.as_path_buf(), &PathBuf::from("/tmp/swell.sock"));
    }

    #[test]
    fn test_socket_path_display() {
        let path = SocketPath::new("/var/run/sock");
        assert_eq!(format!("{}", path), "/var/run/sock");
    }

    #[test]
    fn test_newtypes_are_copy() {
        fn assert_copy<T: Copy>() {}
        assert_copy::<TaskId>();
        assert_copy::<AgentId>();
        assert_copy::<WorktreeId>();
        assert_copy::<FeatureLeadId>();
        assert_copy::<CheckpointId>();
        assert_copy::<SessionId>();
    }

    #[test]
    fn test_newtypes_are_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<TaskId>();
        assert_send_sync::<AgentId>();
        assert_send_sync::<WorktreeId>();
        assert_send_sync::<BranchName>();
        assert_send_sync::<CommitSha>();
        assert_send_sync::<FeatureLeadId>();
        assert_send_sync::<CheckpointId>();
        assert_send_sync::<SessionId>();
        assert_send_sync::<SocketPath>();
    }

    #[test]
    fn test_serde_transparent_task_id() {
        let id = TaskId::from_uuid(Uuid::nil());
        let json = serde_json::to_string(&id).unwrap();
        assert_eq!(json, "\"00000000-0000-0000-0000-000000000000\"");
        let parsed: TaskId = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, id);
    }

    #[test]
    fn test_serde_transparent_branch_name() {
        let name = BranchName::new("feature/test");
        let json = serde_json::to_string(&name).unwrap();
        assert_eq!(json, "\"feature/test\"");
        let parsed: BranchName = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, name);
    }

    #[test]
    fn test_serde_transparent_socket_path() {
        let path = SocketPath::new("/tmp/sock");
        let json = serde_json::to_string(&path).unwrap();
        assert_eq!(json, "\"/tmp/sock\"");
        let parsed: SocketPath = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, path);
    }
}
