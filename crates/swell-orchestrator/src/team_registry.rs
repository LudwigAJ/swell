//! Team Registry for managing grouped tasks with shared fate notification.
//!
//! When tasks are grouped into a team, they share fate: if one task fails,
//! all other team members are notified via a `TeamTaskFailed` event so they
//! can react accordingly (e.g., pause, abort, or adjust strategy).
//!
//! # Example
//!
//! ```ignore
//! let registry = TeamRegistry::new();
//! let team_id = registry.create_team("feature-x").await;
//!
//! registry.add_task_to_team(task1_id, team_id).await;
//! registry.add_task_to_team(task2_id, team_id).await;
//!
//! // When task1 fails, task2 receives TeamTaskFailed notification
//! ```

use dashmap::DashMap;
use swell_core::ids::TaskId;
use std::sync::Arc;
use swell_core::SwellError;
use tokio::sync::broadcast;
use tracing::{info, warn};
use uuid::Uuid;

/// A team is a collection of tasks that share fate.
///
/// When any task in a team fails, all other team members receive
/// a `TeamTaskFailed` notification event.
#[derive(Debug)]
pub struct Team {
    /// Unique team identifier
    pub id: Uuid,
    /// Human-readable team name (optional)
    pub name: String,
    /// All task IDs belonging to this team
    task_ids: std::sync::RwLock<std::collections::HashSet<TaskId>>,
    /// When the team was created
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl Team {
    /// Create a new team with a generated ID
    pub fn new(name: String) -> Self {
        Self {
            id: Uuid::new_v4(),
            name,
            task_ids: std::sync::RwLock::new(std::collections::HashSet::new()),
            created_at: chrono::Utc::now(),
        }
    }

    /// Add a task to this team
    pub fn add_task(&self, task_id: TaskId) -> bool {
        self.task_ids.write().unwrap().insert(task_id)
    }

    /// Remove a task from this team
    pub fn remove_task(&self, task_id: TaskId) -> bool {
        self.task_ids.write().unwrap().remove(&task_id)
    }

    /// Get all task IDs in this team
    pub fn task_ids(&self) -> Vec<TaskId> {
        self.task_ids.read().unwrap().iter().copied().collect()
    }

    /// Get the count of tasks in this team
    pub fn task_count(&self) -> usize {
        self.task_ids.read().unwrap().len()
    }

    /// Check if a task belongs to this team
    pub fn contains_task(&self, task_id: TaskId) -> bool {
        self.task_ids.read().unwrap().contains(&task_id)
    }
}

/// Event emitted when a team task fails.
///
/// This event is sent to all other tasks in the team so they can
/// react accordingly (pause, abort, adjust strategy).
#[derive(Debug, Clone)]
pub struct TeamTaskFailed {
    /// The team ID this event pertains to
    pub team_id: Uuid,
    /// The task ID that failed
    pub failed_task_id: TaskId,
    /// The remaining task IDs in the team (excluding the failed one)
    pub remaining_task_ids: Vec<TaskId>,
    /// Error message describing why the task failed
    pub error_message: String,
}

/// Thread-safe team registry mapping team IDs to teams.
///
/// Uses DashMap for fine-grained concurrent access, allowing multiple
/// operations on different teams without lock contention.
pub struct TeamRegistry {
    /// Maps team ID -> Team
    teams: DashMap<Uuid, Arc<Team>>,
    /// Maps task ID -> team ID (for fast lookup of which team a task belongs to)
    task_to_team: DashMap<TaskId, Uuid>,
    /// Event sender for broadcasting team events
    event_sender: broadcast::Sender<TeamEvent>,
}

/// Team-related events emitted by the registry
#[derive(Debug, Clone)]
pub enum TeamEvent {
    /// Emitted when a team member task fails
    TeamTaskFailed(TeamTaskFailed),
    /// Emitted when a team is created
    TeamCreated { team_id: Uuid, name: String },
    /// Emitted when a team is disbanded
    TeamDisbanded { team_id: Uuid, member_count: usize },
    /// Emitted when a task joins a team
    TaskJoinedTeam { task_id: TaskId, team_id: Uuid },
    /// Emitted when a task leaves a team
    TaskLeftTeam { task_id: TaskId, team_id: Uuid },
}

impl TeamRegistry {
    /// Create a new empty team registry
    pub fn new() -> Self {
        let (tx, _rx) = broadcast::channel(100);
        Self {
            teams: DashMap::new(),
            task_to_team: DashMap::new(),
            event_sender: tx,
        }
    }

    /// Create a new team and return its ID
    pub fn create_team(&self, name: String) -> Uuid {
        let team = Arc::new(Team::new(name.clone()));
        let team_id = team.id;
        self.teams.insert(team_id, team);

        let _ = self
            .event_sender
            .send(TeamEvent::TeamCreated { team_id, name });

        info!(team_id = %team_id, "Team created");
        team_id
    }

    /// Disband a team, removing it from the registry.
    ///
    /// All tasks in the team will be notified that the team has disbanded.
    /// Returns the number of tasks that were in the team.
    pub fn disband_team(&self, team_id: Uuid) -> Result<usize, SwellError> {
        let team =
            self.teams
                .remove(&team_id)
                .map(|(_, t)| t)
                .ok_or(SwellError::InvalidOperation(format!(
                    "Team {} not found",
                    team_id
                )))?;

        let member_count = team.task_count();

        // Remove all task-to-team mappings
        for task_id in team.task_ids() {
            self.task_to_team.remove(&task_id);
        }

        let _ = self.event_sender.send(TeamEvent::TeamDisbanded {
            team_id,
            member_count,
        });

        info!(team_id = %team_id, member_count = member_count, "Team disbanded");
        Ok(member_count)
    }

    /// Add a task to a team.
    ///
    /// Returns an error if the team doesn't exist or the task is already in another team.
    pub fn add_task_to_team(&self, task_id: TaskId, team_id: Uuid) -> Result<(), SwellError> {
        // Check if task is already in another team
        if let Some(existing_team_id) = self.get_team_id_for_task(task_id) {
            if existing_team_id != team_id {
                return Err(SwellError::InvalidOperation(format!(
                    "Task {} is already in team {}",
                    task_id, existing_team_id
                )));
            }
            // Task already in this team - no-op
            return Ok(());
        }

        let team = self
            .teams
            .get(&team_id)
            .ok_or(SwellError::InvalidOperation(format!(
                "Team {} not found",
                team_id
            )))?;

        team.add_task(task_id);
        self.task_to_team.insert(task_id, team_id);

        let _ = self
            .event_sender
            .send(TeamEvent::TaskJoinedTeam { task_id, team_id });

        info!(task_id = %task_id, team_id = %team_id, "Task joined team");
        Ok(())
    }

    /// Remove a task from its current team.
    ///
    /// If the task is not in any team, this is a no-op.
    pub fn remove_task_from_team(&self, task_id: TaskId) -> Result<(), SwellError> {
        let Some(team_id) = self.get_team_id_for_task(task_id) else {
            // Task not in any team - no-op
            return Ok(());
        };

        let Some(team) = self.teams.get(&team_id) else {
            return Ok(()); // Team already disbanded
        };

        team.remove_task(task_id);
        self.task_to_team.remove(&task_id);

        let _ = self
            .event_sender
            .send(TeamEvent::TaskLeftTeam { task_id, team_id });

        info!(task_id = %task_id, team_id = %team_id, "Task left team");
        Ok(())
    }

    /// Get all task IDs in a team
    pub fn get_team_tasks(&self, team_id: Uuid) -> Result<Vec<TaskId>, SwellError> {
        let team = self
            .teams
            .get(&team_id)
            .ok_or(SwellError::InvalidOperation(format!(
                "Team {} not found",
                team_id
            )))?;

        Ok(team.task_ids())
    }

    /// Get the team ID for a task, if any
    pub fn get_team_id_for_task(&self, task_id: TaskId) -> Option<Uuid> {
        self.task_to_team.get(&task_id).map(|r| *r.value())
    }

    /// Get a team by ID
    pub fn get_team(&self, team_id: Uuid) -> Option<Arc<Team>> {
        self.teams.get(&team_id).map(|r| r.value().clone())
    }

    /// Check if a team exists
    pub fn team_exists(&self, team_id: Uuid) -> bool {
        self.teams.contains_key(&team_id)
    }

    /// Get all teams
    pub fn get_all_teams(&self) -> Vec<Arc<Team>> {
        self.teams.iter().map(|r| r.value().clone()).collect()
    }

    /// Notify all team members that a task has failed.
    ///
    /// This emits a `TeamTaskFailed` event to all other tasks in the team
    /// (excluding the failed task itself).
    ///
    /// Returns the number of team members notified.
    pub fn notify_task_failed(
        &self,
        failed_task_id: TaskId,
        error_message: String,
    ) -> Result<usize, SwellError> {
        let Some(team_id) = self.get_team_id_for_task(failed_task_id) else {
            // Task not in any team - nothing to notify
            return Ok(0);
        };

        let team = self
            .teams
            .get(&team_id)
            .ok_or(SwellError::InvalidOperation(format!(
                "Team {} not found",
                team_id
            )))?;

        let remaining: Vec<TaskId> = team
            .task_ids()
            .into_iter()
            .filter(|&id| id != failed_task_id)
            .collect();

        let notification = TeamTaskFailed {
            team_id,
            failed_task_id,
            remaining_task_ids: remaining.clone(),
            error_message,
        };

        let _ = self
            .event_sender
            .send(TeamEvent::TeamTaskFailed(notification));

        warn!(
            team_id = %team_id,
            failed_task_id = %failed_task_id,
            notified_count = remaining.len(),
            "Team task failed - notifying team members"
        );

        Ok(remaining.len())
    }

    /// Subscribe to team events.
    ///
    /// Returns a receiver that will receive all subsequent team events.
    pub fn subscribe(&self) -> broadcast::Receiver<TeamEvent> {
        self.event_sender.subscribe()
    }

    /// Get the number of teams
    pub fn team_count(&self) -> usize {
        self.teams.len()
    }

    /// Get the total number of tasks across all teams
    pub fn total_task_count(&self) -> usize {
        self.teams.iter().map(|r| r.task_count()).sum()
    }
}

impl Default for TeamRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_team(registry: &TeamRegistry) -> Uuid {
        registry.create_team("test-team".to_string())
    }

    // --- Team Creation Tests ---

    #[test]
    fn test_create_team_returns_team_id() {
        let registry = TeamRegistry::new();
        let team_id = registry.create_team("test".to_string());

        assert_ne!(team_id, Uuid::nil());
        assert!(registry.team_exists(team_id));
    }

    #[test]
    fn test_create_team_increments_team_count() {
        let registry = TeamRegistry::new();
        assert_eq!(registry.team_count(), 0);

        registry.create_team("team1".to_string());
        assert_eq!(registry.team_count(), 1);

        registry.create_team("team2".to_string());
        assert_eq!(registry.team_count(), 2);
    }

    #[test]
    fn test_create_team_emits_event() {
        let registry = TeamRegistry::new();
        let mut rx = registry.subscribe();

        let team_id = registry.create_team("test".to_string());

        let event = rx.try_recv().unwrap();
        match event {
            TeamEvent::TeamCreated { team_id: id, name } => {
                assert_eq!(id, team_id);
                assert_eq!(name, "test");
            }
            _ => panic!("Expected TeamCreated event"),
        }
    }

    // --- Add Task to Team Tests ---

    #[test]
    fn test_add_task_to_team() {
        let registry = TeamRegistry::new();
        let team_id = create_test_team(&registry);
        let task_id = TaskId::new();

        registry.add_task_to_team(task_id, team_id).unwrap();

        let tasks = registry.get_team_tasks(team_id).unwrap();
        assert_eq!(tasks.len(), 1);
        assert!(tasks.contains(&task_id));
    }

    #[test]
    fn test_add_multiple_tasks_to_team() {
        let registry = TeamRegistry::new();
        let team_id = create_test_team(&registry);
        let task1 = TaskId::new();
        let task2 = TaskId::new();
        let task3 = TaskId::new();

        registry.add_task_to_team(task1, team_id).unwrap();
        registry.add_task_to_team(task2, team_id).unwrap();
        registry.add_task_to_team(task3, team_id).unwrap();

        let tasks = registry.get_team_tasks(team_id).unwrap();
        assert_eq!(tasks.len(), 3);
    }

    #[test]
    fn test_add_same_task_to_team_twice_is_noop() {
        let registry = TeamRegistry::new();
        let team_id = create_test_team(&registry);
        let task_id = TaskId::new();

        registry.add_task_to_team(task_id, team_id).unwrap();
        registry.add_task_to_team(task_id, team_id).unwrap(); // no-op

        let tasks = registry.get_team_tasks(team_id).unwrap();
        assert_eq!(tasks.len(), 1);
    }

    #[test]
    fn test_add_task_to_nonexistent_team_returns_error() {
        let registry = TeamRegistry::new();
        let task_id = TaskId::new();
        let fake_team_id = Uuid::new_v4();

        let result = registry.add_task_to_team(task_id, fake_team_id);
        assert!(result.is_err());
    }

    #[test]
    fn test_add_task_to_team_emits_event() {
        let registry = TeamRegistry::new();
        let team_id = create_test_team(&registry);
        let task_id = TaskId::new();
        let mut rx = registry.subscribe();

        registry.add_task_to_team(task_id, team_id).unwrap();

        let event = rx.try_recv().unwrap();
        match event {
            TeamEvent::TaskJoinedTeam {
                task_id: id,
                team_id: tid,
            } => {
                assert_eq!(id, task_id);
                assert_eq!(tid, team_id);
            }
            _ => panic!("Expected TaskJoinedTeam event"),
        }
    }

    // --- Remove Task from Team Tests ---

    #[test]
    fn test_remove_task_from_team() {
        let registry = TeamRegistry::new();
        let team_id = create_test_team(&registry);
        let task_id = TaskId::new();

        registry.add_task_to_team(task_id, team_id).unwrap();
        registry.remove_task_from_team(task_id).unwrap();

        let tasks = registry.get_team_tasks(team_id).unwrap();
        assert!(tasks.is_empty());
    }

    #[test]
    fn test_remove_nonexistent_task_is_noop() {
        let registry = TeamRegistry::new();
        let team_id = create_test_team(&registry);
        let fake_task_id = TaskId::new();

        // Should not error even though task doesn't exist
        registry.remove_task_from_team(fake_task_id).unwrap();

        let tasks = registry.get_team_tasks(team_id).unwrap();
        assert!(tasks.is_empty());
    }

    // --- Disband Team Tests ---

    #[test]
    fn test_disband_team() {
        let registry = TeamRegistry::new();
        let team_id = create_test_team(&registry);
        let task_id = TaskId::new();

        registry.add_task_to_team(task_id, team_id).unwrap();
        let count = registry.disband_team(team_id).unwrap();

        assert_eq!(count, 1);
        assert!(!registry.team_exists(team_id));
    }

    #[test]
    fn test_disband_nonexistent_team_returns_error() {
        let registry = TeamRegistry::new();
        let fake_team_id = Uuid::new_v4();

        let result = registry.disband_team(fake_team_id);
        assert!(result.is_err());
    }

    #[test]
    fn test_disband_team_emits_event() {
        let registry = TeamRegistry::new();
        let team_id = create_test_team(&registry);
        let mut rx = registry.subscribe();

        registry.disband_team(team_id).unwrap();

        let event = rx.try_recv().unwrap();
        match event {
            TeamEvent::TeamDisbanded { team_id: id, .. } => {
                assert_eq!(id, team_id);
            }
            _ => panic!("Expected TeamDisbanded event"),
        }
    }

    // --- Get Team ID for Task Tests ---

    #[test]
    fn test_get_team_id_for_task() {
        let registry = TeamRegistry::new();
        let team_id = create_test_team(&registry);
        let task_id = TaskId::new();

        assert!(registry.get_team_id_for_task(task_id).is_none());

        registry.add_task_to_team(task_id, team_id).unwrap();

        assert_eq!(registry.get_team_id_for_task(task_id), Some(team_id));
    }

    #[test]
    fn test_task_cannot_join_multiple_teams() {
        let registry = TeamRegistry::new();
        let team1_id = create_test_team(&registry);
        let team2_id = create_test_team(&registry);
        let task_id = TaskId::new();

        registry.add_task_to_team(task_id, team1_id).unwrap();
        let result = registry.add_task_to_team(task_id, team2_id);

        assert!(result.is_err());
    }

    // --- Notify Task Failed Tests ---

    #[test]
    fn test_notify_task_failed_emits_event_to_team_members() {
        let registry = TeamRegistry::new();
        let team_id = create_test_team(&registry);
        let task1 = TaskId::new();
        let task2 = TaskId::new();
        let task3 = TaskId::new();

        registry.add_task_to_team(task1, team_id).unwrap();
        registry.add_task_to_team(task2, team_id).unwrap();
        registry.add_task_to_team(task3, team_id).unwrap();

        let mut rx = registry.subscribe();

        let notified = registry
            .notify_task_failed(task1, "Test error".to_string())
            .unwrap();

        // Should notify task2 and task3 (not task1 which failed)
        assert_eq!(notified, 2);

        let event = rx.try_recv().unwrap();
        match event {
            TeamEvent::TeamTaskFailed(notification) => {
                assert_eq!(notification.team_id, team_id);
                assert_eq!(notification.failed_task_id, task1);
                assert_eq!(notification.remaining_task_ids.len(), 2);
                assert!(notification.remaining_task_ids.contains(&task2));
                assert!(notification.remaining_task_ids.contains(&task3));
                assert_eq!(notification.error_message, "Test error");
            }
            _ => panic!("Expected TeamTaskFailed event"),
        }
    }

    #[test]
    fn test_notify_task_failed_not_in_team_is_noop() {
        let registry = TeamRegistry::new();
        let task_id = TaskId::new();
        let mut rx = registry.subscribe();

        // Task not in any team - should return 0 and not emit event
        let notified = registry
            .notify_task_failed(task_id, "Error".to_string())
            .unwrap();

        assert_eq!(notified, 0);

        // No event should be emitted
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn test_notify_task_failed_last_member() {
        let registry = TeamRegistry::new();
        let team_id = create_test_team(&registry);
        let task_id = TaskId::new();

        registry.add_task_to_team(task_id, team_id).unwrap();

        let notified = registry
            .notify_task_failed(task_id, "Error".to_string())
            .unwrap();

        // Only member failed, no one to notify
        assert_eq!(notified, 0);
    }

    // --- Get All Teams Tests ---

    #[test]
    fn test_get_all_teams() {
        let registry = TeamRegistry::new();
        let team1_id = registry.create_team("team1".to_string());
        let team2_id = registry.create_team("team2".to_string());

        let teams = registry.get_all_teams();
        assert_eq!(teams.len(), 2);

        let team_ids: Vec<_> = teams.iter().map(|t| t.id).collect();
        assert!(team_ids.contains(&team1_id));
        assert!(team_ids.contains(&team2_id));
    }

    // --- Task Count Tests ---

    #[test]
    fn test_team_task_count() {
        let registry = TeamRegistry::new();
        let team_id = create_test_team(&registry);

        let team = registry.get_team(team_id).unwrap();
        assert_eq!(team.task_count(), 0);

        registry.add_task_to_team(TaskId::new(), team_id).unwrap();
        registry.add_task_to_team(TaskId::new(), team_id).unwrap();

        let team = registry.get_team(team_id).unwrap();
        assert_eq!(team.task_count(), 2);
    }

    #[test]
    fn test_total_task_count() {
        let registry = TeamRegistry::new();
        let team1_id = create_test_team(&registry);
        let team2_id = create_test_team(&registry);

        registry.add_task_to_team(TaskId::new(), team1_id).unwrap();
        registry.add_task_to_team(TaskId::new(), team1_id).unwrap();
        registry.add_task_to_team(TaskId::new(), team2_id).unwrap();

        assert_eq!(registry.total_task_count(), 3);
    }

    // --- Contains Task Tests ---

    #[test]
    fn test_team_contains_task() {
        let registry = TeamRegistry::new();
        let team_id = create_test_team(&registry);
        let task_id = TaskId::new();

        let team = registry.get_team(team_id).unwrap();
        assert!(!team.contains_task(task_id));

        registry.add_task_to_team(task_id, team_id).unwrap();

        let team = registry.get_team(team_id).unwrap();
        assert!(team.contains_task(task_id));
    }

    // --- Empty Team Notifications ---

    #[tokio::test]
    async fn test_notify_task_failed_empty_team() {
        let registry = TeamRegistry::new();
        let team_id = create_test_team(&registry);
        let task_id = TaskId::new();

        // Add only task, then remove it - team is now empty
        registry.add_task_to_team(task_id, team_id).unwrap();
        registry.remove_task_from_team(task_id).unwrap();

        // Now task_id is not in any team
        let notified = registry
            .notify_task_failed(task_id, "Error".to_string())
            .unwrap();

        assert_eq!(notified, 0);
    }
}
