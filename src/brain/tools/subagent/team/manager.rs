//! TeamManager — tracks named teams of sub-agents.
//!
//! A team is a named group of agent IDs that were spawned together.
//! Teams enable batch operations: broadcast a message to all members,
//! cancel all members, or query team status.

use std::collections::HashMap;
use std::sync::RwLock;

/// A named team of sub-agents.
#[derive(Debug, Clone)]
pub struct Team {
    /// Unique team name
    pub name: String,
    /// Agent IDs belonging to this team
    pub agent_ids: Vec<String>,
    /// When the team was created
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Manages named teams of sub-agents.
pub struct TeamManager {
    teams: RwLock<HashMap<String, Team>>,
}

impl TeamManager {
    /// Create a new empty team manager.
    pub fn new() -> Self {
        Self {
            teams: RwLock::new(HashMap::new()),
        }
    }

    /// Create a team with the given name and agent IDs.
    /// Returns false if a team with that name already exists.
    pub fn create_team(&self, name: String, agent_ids: Vec<String>) -> bool {
        let mut teams = self.teams.write().expect("team manager lock poisoned");
        if teams.contains_key(&name) {
            return false;
        }
        teams.insert(
            name.clone(),
            Team {
                name,
                agent_ids,
                created_at: chrono::Utc::now(),
            },
        );
        true
    }

    /// Delete a team by name. Returns the team if it existed.
    pub fn delete_team(&self, name: &str) -> Option<Team> {
        self.teams
            .write()
            .expect("team manager lock poisoned")
            .remove(name)
    }

    /// Get agent IDs for a team.
    pub fn get_agent_ids(&self, name: &str) -> Option<Vec<String>> {
        self.teams
            .read()
            .expect("team manager lock poisoned")
            .get(name)
            .map(|t| t.agent_ids.clone())
    }

    /// List all teams with their agent counts.
    pub fn list_teams(&self) -> Vec<(String, usize)> {
        self.teams
            .read()
            .expect("team manager lock poisoned")
            .values()
            .map(|t| (t.name.clone(), t.agent_ids.len()))
            .collect()
    }

    /// Check if a team exists.
    pub fn exists(&self, name: &str) -> bool {
        self.teams
            .read()
            .expect("team manager lock poisoned")
            .contains_key(name)
    }
}

impl Default for TeamManager {
    fn default() -> Self {
        Self::new()
    }
}
