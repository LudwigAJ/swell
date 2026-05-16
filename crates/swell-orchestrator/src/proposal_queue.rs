//! `ProposalQueue` — F12 follow-up proposer slice of
//! `plan/flow_integration_plan/12_task_generation_failure_and_followup.md`.
//!
//! In-memory thread-safe queue that holds [`FollowUpProposal`]s produced by
//! the `FollowUpProposerTrigger` on the `AfterTask` success path.
//! Proposals sit in `Pending` until an operator (or autonomy gate, when PR
//! `13` lands) calls [`ProposalQueue::approve`] or
//! [`ProposalQueue::reject`]. Approval drains the proposal so the caller
//! can convert it into a real `Task` and feed it back through the
//! orchestrator.
//!
//! This crate owns only the queue; the approval CLI / autonomy gating is
//! the PR `13` concern.

use std::sync::Mutex;

use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::followup_generator::FollowUpProposal;

/// Lifecycle state of a proposal in the queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProposalStatus {
    /// Waiting for operator (or autonomy-gate) decision.
    Pending,
    /// Approved and drained — caller has taken responsibility for
    /// converting the proposal into a `Task`.
    Approved { approved_at: DateTime<Utc> },
    /// Rejected with a reason recorded for auditability.
    Rejected {
        reason: String,
        rejected_at: DateTime<Utc>,
    },
}

/// One queued proposal: the [`FollowUpProposal`] payload plus its current
/// lifecycle status and submission timestamp.
#[derive(Debug, Clone)]
pub struct QueuedProposal {
    pub proposal: FollowUpProposal,
    pub status: ProposalStatus,
    pub submitted_at: DateTime<Utc>,
}

impl QueuedProposal {
    pub fn id(&self) -> Uuid {
        self.proposal.id
    }
}

/// Thread-safe in-memory queue.
///
/// The orchestrator owns a single `Arc<ProposalQueue>`. The
/// `FollowUpProposerTrigger` writes into it on the AfterTask success path;
/// operator-facing CLI (PR `13`) drains it.
#[derive(Default)]
pub struct ProposalQueue {
    entries: Mutex<Vec<QueuedProposal>>,
}

impl std::fmt::Debug for ProposalQueue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let entries = self.entries.lock().expect("proposal queue lock poisoned");
        f.debug_struct("ProposalQueue")
            .field("count", &entries.len())
            .field(
                "pending",
                &entries
                    .iter()
                    .filter(|e| matches!(e.status, ProposalStatus::Pending))
                    .count(),
            )
            .finish()
    }
}

impl ProposalQueue {
    pub fn new() -> Self {
        Self::default()
    }

    /// Submit a proposal. Returns the queued entry's id (mirrors
    /// `FollowUpProposal.id` for cross-reference convenience). Duplicate
    /// `FollowUpProposal.id` values are silently de-duplicated — the
    /// trigger is allowed to re-fire on retries without the queue
    /// growing.
    pub fn submit(&self, proposal: FollowUpProposal) -> Uuid {
        let id = proposal.id;
        let mut entries = self.entries.lock().expect("proposal queue lock poisoned");
        if entries.iter().any(|e| e.proposal.id == id) {
            return id;
        }
        entries.push(QueuedProposal {
            proposal,
            status: ProposalStatus::Pending,
            submitted_at: Utc::now(),
        });
        id
    }

    /// Number of entries in the queue (any status).
    pub fn len(&self) -> usize {
        self.entries
            .lock()
            .expect("proposal queue lock poisoned")
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Snapshot all entries currently in `Pending`.
    pub fn pending(&self) -> Vec<QueuedProposal> {
        self.entries
            .lock()
            .expect("proposal queue lock poisoned")
            .iter()
            .filter(|e| matches!(e.status, ProposalStatus::Pending))
            .cloned()
            .collect()
    }

    /// Snapshot all entries (any status). Useful for audit / UI.
    pub fn all(&self) -> Vec<QueuedProposal> {
        self.entries
            .lock()
            .expect("proposal queue lock poisoned")
            .clone()
    }

    pub fn get(&self, id: Uuid) -> Option<QueuedProposal> {
        self.entries
            .lock()
            .expect("proposal queue lock poisoned")
            .iter()
            .find(|e| e.proposal.id == id)
            .cloned()
    }

    /// Approve a pending proposal. Returns the proposal payload so the
    /// caller can convert it into a `Task`. Returns `None` if the id is
    /// unknown or the proposal is not in `Pending`.
    pub fn approve(&self, id: Uuid) -> Option<FollowUpProposal> {
        let mut entries = self.entries.lock().expect("proposal queue lock poisoned");
        let entry = entries.iter_mut().find(|e| e.proposal.id == id)?;
        if !matches!(entry.status, ProposalStatus::Pending) {
            return None;
        }
        entry.status = ProposalStatus::Approved {
            approved_at: Utc::now(),
        };
        Some(entry.proposal.clone())
    }

    /// Reject a pending proposal with a reason. Returns `true` if the
    /// transition happened, `false` if the id is unknown or the proposal
    /// is not in `Pending`.
    pub fn reject(&self, id: Uuid, reason: impl Into<String>) -> bool {
        let mut entries = self.entries.lock().expect("proposal queue lock poisoned");
        let Some(entry) = entries.iter_mut().find(|e| e.proposal.id == id) else {
            return false;
        };
        if !matches!(entry.status, ProposalStatus::Pending) {
            return false;
        }
        entry.status = ProposalStatus::Rejected {
            reason: reason.into(),
            rejected_at: Utc::now(),
        };
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::followup_generator::{FollowUpOpportunityType, FollowUpProposal};
    use swell_core::{RiskLevel, TaskId};

    fn make_proposal(parent: TaskId) -> FollowUpProposal {
        FollowUpProposal {
            id: Uuid::new_v4(),
            parent_task_id: parent,
            description: "test proposal".to_string(),
            rationale: "because reasons".to_string(),
            opportunity_type: FollowUpOpportunityType::TestGap,
            affected_items: vec!["src/lib.rs".to_string()],
            initial_steps: vec!["step 1".to_string()],
            risk_level: RiskLevel::Low,
            priority: 50,
        }
    }

    #[test]
    fn submit_returns_id_and_lists_as_pending() {
        let q = ProposalQueue::new();
        let p = make_proposal(TaskId::new());
        let pid = p.id;
        let returned = q.submit(p);
        assert_eq!(returned, pid);
        let pending = q.pending();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].proposal.id, pid);
        assert!(matches!(pending[0].status, ProposalStatus::Pending));
    }

    #[test]
    fn duplicate_submit_is_deduped() {
        let q = ProposalQueue::new();
        let p = make_proposal(TaskId::new());
        let id = p.id;
        q.submit(p.clone());
        q.submit(p);
        assert_eq!(q.len(), 1, "duplicate submit must not grow the queue");
        assert_eq!(q.pending().len(), 1);
        let _ = id;
    }

    #[test]
    fn approve_drains_proposal_and_transitions_status() {
        let q = ProposalQueue::new();
        let p = make_proposal(TaskId::new());
        let id = q.submit(p);
        let drained = q.approve(id).expect("approve should return proposal");
        assert_eq!(drained.id, id);
        assert!(q.pending().is_empty(), "approved no longer pending");
        assert_eq!(q.len(), 1, "entry retained for audit");
        let entry = q.get(id).expect("entry still present");
        assert!(matches!(entry.status, ProposalStatus::Approved { .. }));
    }

    #[test]
    fn approve_twice_returns_none() {
        let q = ProposalQueue::new();
        let id = q.submit(make_proposal(TaskId::new()));
        assert!(q.approve(id).is_some());
        assert!(
            q.approve(id).is_none(),
            "approving a non-pending entry must yield None"
        );
    }

    #[test]
    fn reject_records_reason_and_blocks_approve() {
        let q = ProposalQueue::new();
        let id = q.submit(make_proposal(TaskId::new()));
        assert!(q.reject(id, "out of scope"));
        let entry = q.get(id).unwrap();
        match entry.status {
            ProposalStatus::Rejected { reason, .. } => assert_eq!(reason, "out of scope"),
            other => panic!("expected Rejected, got {other:?}"),
        }
        assert!(
            q.approve(id).is_none(),
            "rejected entry must not be re-approvable"
        );
    }

    #[test]
    fn approve_unknown_id_returns_none() {
        let q = ProposalQueue::new();
        assert!(q.approve(Uuid::new_v4()).is_none());
        assert!(!q.reject(Uuid::new_v4(), "nope"));
    }

    #[test]
    fn all_returns_every_entry_regardless_of_status() {
        let q = ProposalQueue::new();
        let a = q.submit(make_proposal(TaskId::new()));
        let b = q.submit(make_proposal(TaskId::new()));
        let c = q.submit(make_proposal(TaskId::new()));
        q.approve(a);
        q.reject(b, "no");
        assert_eq!(q.all().len(), 3);
        let pending = q.pending();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].proposal.id, c);
    }
}
