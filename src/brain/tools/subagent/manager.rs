//! SubAgentManager — tracks all spawned child agents.
//!
//! Shared across the 5 subagent tools via `Arc<SubAgentManager>`.
//! Each child agent has its own session, cancel token, output channel,
//! and input channel for mid-execution messaging.

use std::collections::HashMap;
use std::sync::RwLock;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

/// State of a spawned sub-agent.
#[derive(Debug, Clone, PartialEq)]
pub enum SubAgentState {
    Running,
    Completed,
    Failed(String),
    Cancelled,
}

/// A spawned child agent.
pub struct SubAgent {
    /// Unique identifier for this child
    pub id: String,

    /// Human-readable label (from the prompt summary)
    pub label: String,

    /// Session ID the child operates on
    pub session_id: Uuid,

    /// Current state
    pub state: SubAgentState,

    /// Cancel token — fire to terminate the child
    pub cancel_token: CancellationToken,

    /// Join handle for the background task (None after awaited)
    pub join_handle: Option<JoinHandle<()>>,

    /// Send follow-up input to the running child
    pub input_tx: Option<mpsc::UnboundedSender<String>>,

    /// Final output collected from the child (set on completion)
    pub output: Option<String>,

    /// Timestamp when spawned
    pub spawned_at: chrono::DateTime<chrono::Utc>,
}

/// Manages all sub-agents for a parent agent instance.
pub struct SubAgentManager {
    agents: RwLock<HashMap<String, SubAgent>>,
}

impl SubAgentManager {
    /// Create a new empty manager.
    pub fn new() -> Self {
        Self {
            agents: RwLock::new(HashMap::new()),
        }
    }

    /// Generate a short agent ID (first 8 chars of a UUID).
    pub fn generate_id() -> String {
        Uuid::new_v4().to_string()[..8].to_string()
    }

    /// Register a new sub-agent.
    pub fn insert(&self, agent: SubAgent) {
        let id = agent.id.clone();
        self.agents
            .write()
            .expect("subagent manager lock poisoned")
            .insert(id, agent);
    }

    /// Get a clone of the agent's state.
    pub fn get_state(&self, id: &str) -> Option<SubAgentState> {
        self.agents
            .read()
            .expect("subagent manager lock poisoned")
            .get(id)
            .map(|a| a.state.clone())
    }

    /// Get the agent's output if completed.
    pub fn get_output(&self, id: &str) -> Option<String> {
        self.agents
            .read()
            .expect("subagent manager lock poisoned")
            .get(id)
            .and_then(|a| a.output.clone())
    }

    /// Get the input sender for a running agent.
    pub fn get_input_tx(&self, id: &str) -> Option<mpsc::UnboundedSender<String>> {
        self.agents
            .read()
            .expect("subagent manager lock poisoned")
            .get(id)
            .and_then(|a| a.input_tx.clone())
    }

    /// Cancel a running agent.
    pub fn cancel(&self, id: &str) -> bool {
        let mut agents = self.agents.write().expect("subagent manager lock poisoned");
        if let Some(agent) = agents.get_mut(id)
            && agent.state == SubAgentState::Running
        {
            agent.cancel_token.cancel();
            agent.state = SubAgentState::Cancelled;
            agent.input_tx = None;
            return true;
        }
        false
    }

    /// Take the join handle (for awaiting completion).
    pub fn take_join_handle(&self, id: &str) -> Option<JoinHandle<()>> {
        let mut agents = self.agents.write().expect("subagent manager lock poisoned");
        agents.get_mut(id).and_then(|a| a.join_handle.take())
    }

    /// Update agent state and output after completion.
    pub fn mark_completed(&self, id: &str, output: String) {
        let mut agents = self.agents.write().expect("subagent manager lock poisoned");
        if let Some(agent) = agents.get_mut(id) {
            agent.state = SubAgentState::Completed;
            agent.output = Some(output);
            agent.input_tx = None;
        }
    }

    /// Update agent state after failure.
    pub fn mark_failed(&self, id: &str, error: String) {
        let mut agents = self.agents.write().expect("subagent manager lock poisoned");
        if let Some(agent) = agents.get_mut(id) {
            agent.state = SubAgentState::Failed(error);
            agent.input_tx = None;
        }
    }

    /// Re-register a completed agent for resumption (new handle/token/channels).
    pub fn prepare_resume(
        &self,
        id: &str,
        cancel_token: CancellationToken,
        input_tx: mpsc::UnboundedSender<String>,
    ) -> bool {
        let mut agents = self.agents.write().expect("subagent manager lock poisoned");
        if let Some(agent) = agents.get_mut(id)
            && matches!(
                agent.state,
                SubAgentState::Completed | SubAgentState::Failed(_)
            )
        {
            agent.state = SubAgentState::Running;
            agent.cancel_token = cancel_token;
            agent.input_tx = Some(input_tx);
            agent.output = None;
            return true;
        }
        false
    }

    /// Set the join handle after spawning a resume task.
    pub fn set_join_handle(&self, id: &str, handle: JoinHandle<()>) {
        let mut agents = self.agents.write().expect("subagent manager lock poisoned");
        if let Some(agent) = agents.get_mut(id) {
            agent.join_handle = Some(handle);
        }
    }

    /// List all agents with their states.
    pub fn list(&self) -> Vec<(String, String, SubAgentState)> {
        self.agents
            .read()
            .expect("subagent manager lock poisoned")
            .values()
            .map(|a| (a.id.clone(), a.label.clone(), a.state.clone()))
            .collect()
    }

    /// Check if an agent exists.
    pub fn exists(&self, id: &str) -> bool {
        self.agents
            .read()
            .expect("subagent manager lock poisoned")
            .contains_key(id)
    }

    /// Get the session_id for a sub-agent (needed for resume).
    pub fn get_session_id(&self, id: &str) -> Option<Uuid> {
        self.agents
            .read()
            .expect("subagent manager lock poisoned")
            .get(id)
            .map(|a| a.session_id)
    }

    /// Remove a terminated agent from tracking.
    pub fn remove(&self, id: &str) -> Option<SubAgent> {
        self.agents
            .write()
            .expect("subagent manager lock poisoned")
            .remove(id)
    }
}

impl Default for SubAgentManager {
    fn default() -> Self {
        Self::new()
    }
}
