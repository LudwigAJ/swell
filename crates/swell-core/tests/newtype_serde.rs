//! Round-trip every newtype identifier through serde_json.
//!
//! All newtype IDs in `swell_core::ids` use `#[serde(transparent)]`, so this
//! suite catches accidental wrapper changes (`#[serde(transparent)]` removed,
//! a struct field renamed, a variant added) at test time rather than at the
//! daemon boundary.

use swell_core::ids::{
    AgentId, BranchName, CheckpointId, CommitSha, FeatureLeadId, SessionId, SocketPath, TaskId,
    WorktreeId,
};

fn round_trip<T>(value: &T)
where
    T: serde::Serialize + serde::de::DeserializeOwned + std::fmt::Debug + PartialEq,
{
    let json = serde_json::to_string(value).expect("serialize");
    let back: T = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(value, &back, "round-trip mismatch via {json}");
}

#[test]
fn uuid_newtypes_round_trip() {
    round_trip(&TaskId::new());
    round_trip(&AgentId::new());
    round_trip(&WorktreeId::new());
    round_trip(&FeatureLeadId::new());
    round_trip(&CheckpointId::new());
    round_trip(&SessionId::new());
}

#[test]
fn string_newtypes_round_trip() {
    round_trip(&BranchName::from_string("feature/foo"));
    round_trip(&CommitSha::from_string("a1b2c3d4e5f6"));
    round_trip(&SocketPath::from_string("/tmp/swell-daemon.sock"));
}

#[test]
fn uuid_newtypes_reject_invalid_strings() {
    assert!(serde_json::from_str::<TaskId>("\"not-a-uuid\"").is_err());
    assert!(serde_json::from_str::<AgentId>("\"\"").is_err());
    assert!(serde_json::from_str::<WorktreeId>("\"123\"").is_err());
    assert!(serde_json::from_str::<FeatureLeadId>("\"???\"").is_err());
    assert!(serde_json::from_str::<CheckpointId>("\"x\"").is_err());
    assert!(serde_json::from_str::<SessionId>("\"y\"").is_err());
}
