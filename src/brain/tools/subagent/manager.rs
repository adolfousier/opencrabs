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
///
/// `AwaitingInput` is the "round complete, paused for follow-up" state.
/// Without it, `wait_agent` couldn't distinguish "still working" from
/// "done with this round, output ready" — both looked like `Running`,
/// so `handle.await` always timed out (sub-agent task only terminates
/// on input channel close or cancel, never on round boundary).
#[derive(Debug, Clone, PartialEq)]
pub enum SubAgentState {
    Running,
    AwaitingInput,
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

    /// Session that spawned this child (#1183). The manager is process-global
    /// (one instance wired into every channel agent), so without this field a
    /// per-session surface — the Telegram settle card — cannot tell which
    /// children belong to the chat that just settled. Drives
    /// [`SubAgentManager::alive_counts_for`]. Not the child's session: that
    /// one is fresh per agent and nothing is listening on it.
    pub parent_session_id: Uuid,

    /// Whether this child was spawned with a read-restricted tool registry.
    ///
    /// Frozen for the agent's lifetime (#1173): resume must rebuild the same
    /// restriction, never widen it. External code should query via the
    /// manager's `get_read_only` instead of reaching through the lock.
    pub read_only: bool,

    /// Whether this child was spawned with workspace brain files pre-injected (#145).
    ///
    /// Frozen for the agent's lifetime: resume re-attaches system brain when true.
    pub include_brain: bool,

    /// Whether this child may spawn further sub-agents or background tasks
    /// (#1195). Frozen for the agent's lifetime like [`SubAgent::read_only`]:
    /// enforcement live-queries the manager by the caller's session id, so
    /// resumes inherit the restriction without rebuilding anything. Plan
    /// workers are spawned with this false (worker purity); root sessions
    /// are never registered here and are always unrestricted.
    pub allow_nested: bool,

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

    /// How many `wait_agent` calls are currently blocked on this agent.
    ///
    /// Decides whether a finished agent pushes its result into the parent
    /// session. A waiter already receives the output as its own tool result,
    /// so pushing as well would deliver it twice; with no waiter there is
    /// nothing to receive it and the output would otherwise be dropped
    /// (#1036). Read and written only under the manager's write lock, so the
    /// terminal transition and the decision cannot interleave.
    pub waiters: usize,
}

impl SubAgent {
    /// Canonical construction: identity plus fresh running state (#1197
    /// review DRY). Per-spawn variation (`read_only`, `allow_nested`,
    /// handles, tokens) is overridden at the call site via struct-update
    /// syntax, so adding a field means touching this one place instead of
    /// once per construction site - the failure mode that turned every
    /// flag addition into a repo-wide fixture sweep.
    pub fn new(
        id: impl Into<String>,
        label: impl Into<String>,
        session_id: Uuid,
        parent_session_id: Uuid,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            session_id,
            parent_session_id,
            read_only: false,
            include_brain: false,
            allow_nested: true,
            state: SubAgentState::Running,
            cancel_token: CancellationToken::new(),
            join_handle: None,
            input_tx: None,
            output: None,
            spawned_at: chrono::Utc::now(),
            waiters: 0,
        }
    }
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

    /// Get the agent's read-only grant (#1173).
    ///
    /// `None` = no such agent; `Some(true)` = restricted for life, including
    /// every resume; `Some(false)` = full (minus always-excluded) registry.
    pub fn get_read_only(&self, id: &str) -> Option<bool> {
        self.agents
            .read()
            .expect("subagent manager lock poisoned")
            .get(id)
            .map(|a| a.read_only)
    }

    /// Whether the agent was spawned with workspace brain files pre-injected (#145).
    pub fn get_include_brain(&self, id: &str) -> Option<bool> {
        self.agents
            .read()
            .expect("subagent manager lock poisoned")
            .get(id)
            .map(|a| a.include_brain)
    }

    /// May the agent operating on `session_id` spawn further sub-agents or
    /// background tasks? (#1195) Unregistered sessions (roots, main chats)
    /// are always unrestricted; registered children answer from their frozen
    /// `allow_nested` grant.
    pub fn nesting_allowed_for_session(&self, session_id: Uuid) -> bool {
        self.agents
            .read()
            .expect("subagent manager lock poisoned")
            .values()
            .find(|a| a.session_id == session_id)
            .map(|a| a.allow_nested)
            .unwrap_or(true)
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

    /// Cancel a running or paused agent.
    pub fn cancel(&self, id: &str) -> bool {
        let mut agents = self.agents.write().expect("subagent manager lock poisoned");
        if let Some(agent) = agents.get_mut(id)
            && matches!(
                agent.state,
                SubAgentState::Running | SubAgentState::AwaitingInput
            )
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

    /// Update output for a running agent without changing state.
    pub fn update_output(&self, id: &str, output: String) {
        let mut agents = self.agents.write().expect("subagent manager lock poisoned");
        if let Some(agent) = agents.get_mut(id) {
            agent.output = Some(output);
        }
    }

    /// Mark the agent as paused, waiting for a follow-up input. Pairs with
    /// `mark_running_again` when input arrives. Only transitions from
    /// `Running`; no-op if the agent already terminated.
    pub fn mark_awaiting_input(&self, id: &str) {
        let mut agents = self.agents.write().expect("subagent manager lock poisoned");
        if let Some(agent) = agents.get_mut(id)
            && agent.state == SubAgentState::Running
        {
            agent.state = SubAgentState::AwaitingInput;
        }
    }

    /// Flip back to Running when follow-up input has been consumed.
    pub fn mark_running_again(&self, id: &str) {
        let mut agents = self.agents.write().expect("subagent manager lock poisoned");
        if let Some(agent) = agents.get_mut(id)
            && agent.state == SubAgentState::AwaitingInput
        {
            agent.state = SubAgentState::Running;
        }
    }

    /// Register a `wait_agent` call as blocked on `id`.
    ///
    /// Paired with [`Self::leave_wait`]. Taken before the first state check so
    /// an agent finishing during the wait still sees the waiter and skips the
    /// push. Returns false when no such agent exists, so the caller does not
    /// leave a decrement outstanding.
    pub fn enter_wait(&self, id: &str) -> bool {
        let mut agents = self.agents.write().expect("subagent manager lock poisoned");
        match agents.get_mut(id) {
            Some(agent) => {
                agent.waiters += 1;
                true
            }
            None => false,
        }
    }

    /// Release a wait registered by [`Self::enter_wait`].
    pub fn leave_wait(&self, id: &str) {
        let mut agents = self.agents.write().expect("subagent manager lock poisoned");
        if let Some(agent) = agents.get_mut(id) {
            agent.waiters = agent.waiters.saturating_sub(1);
        }
    }

    /// Update agent state and output after completion.
    ///
    /// Returns whether the result should be pushed into the parent session:
    /// true when nobody is waiting on this agent, since then no tool result
    /// will ever carry the output and it would simply be dropped. The check
    /// happens under the same write lock as the transition, so a `wait_agent`
    /// arriving concurrently either registers first (and we do not push) or
    /// arrives after and reads the finished state directly.
    pub fn mark_completed(&self, id: &str, output: String) -> bool {
        let mut agents = self.agents.write().expect("subagent manager lock poisoned");
        match agents.get_mut(id) {
            Some(agent) => {
                agent.state = SubAgentState::Completed;
                agent.output = Some(output);
                agent.input_tx = None;
                agent.waiters == 0
            }
            None => false,
        }
    }

    /// Mark the child completed AND deliver its final output to the spawning
    /// session in one step - the single completion path for every spawner
    /// (#1197 review DRY). Delivery fires only when nobody is collecting via
    /// `wait_agent` ([`Self::mark_completed`]'s bool): a waiter reads the
    /// result inline, and pushing anyway would duplicate it in the parent
    /// chat. Returns whether the result was delivered.
    pub fn complete_and_deliver(
        &self,
        id: &str,
        output: String,
        parent_session_id: uuid::Uuid,
        label: &str,
    ) -> bool {
        let should_push = self.mark_completed(id, output.clone());
        if should_push {
            crate::brain::tools::subagent::spawn::push_result(
                parent_session_id,
                label,
                id,
                Ok(&output),
            );
        }
        should_push
    }

    /// Update agent state after failure. Returns whether to push, on the same
    /// terms as [`Self::mark_completed`]: a failure nobody is waiting on is
    /// still a result the parent needs.
    pub fn mark_failed(&self, id: &str, error: String) -> bool {
        let mut agents = self.agents.write().expect("subagent manager lock poisoned");
        match agents.get_mut(id) {
            Some(agent) => {
                agent.state = SubAgentState::Failed(error);
                agent.input_tx = None;
                agent.waiters == 0
            }
            None => false,
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

    /// List agents belonging to one parent session (#191).
    ///
    /// Mirrors [`Self::alive_counts_for`] for the same reason: the manager is
    /// process-global (one instance wired into every channel agent), so an
    /// unfiltered [`Self::list`] reports another chat's fan-out as this chat's
    /// in-flight work. A surface that frames those rows as *the caller's*
    /// sub-agents then makes a lane adopt — or, worse, silently suppress —
    /// work that is not its own.
    ///
    /// Unlike `alive_counts_for`, terminal agents ARE included: a caller
    /// asking what it spawned wants its finished children too.
    pub fn list_for_parent(&self, parent_session_id: Uuid) -> Vec<(String, String, SubAgentState)> {
        self.agents
            .read()
            .expect("subagent manager lock poisoned")
            .values()
            .filter(|a| a.parent_session_id == parent_session_id)
            .map(|a| (a.id.clone(), a.label.clone(), a.state.clone()))
            .collect()
    }

    /// Alive-agent counts for one parent session (#1183): `(working,
    /// awaiting)` — children mid-round (`Running`) vs parked at a round
    /// boundary with output ready to collect (`AwaitingInput`). Terminal
    /// agents never count, and neither do children spawned by OTHER sessions:
    /// the settle card is per-chat while the manager is process-global, so an
    /// unfiltered count would report another chat's fan-out as this chat's
    /// pending work.
    pub fn alive_counts_for(&self, parent_session_id: Uuid) -> (usize, usize) {
        self.agents
            .read()
            .expect("subagent manager lock poisoned")
            .values()
            .filter(|a| a.parent_session_id == parent_session_id)
            .fold((0, 0), |(working, awaiting), a| match a.state {
                SubAgentState::Running => (working + 1, awaiting),
                SubAgentState::AwaitingInput => (working, awaiting + 1),
                _ => (working, awaiting),
            })
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

    /// Get the label for a sub-agent (needed for result delivery on paths
    /// other than spawn — #1197).
    pub fn get_label(&self, id: &str) -> Option<String> {
        self.agents
            .read()
            .expect("subagent manager lock poisoned")
            .get(id)
            .map(|a| a.label.clone())
    }

    /// Get the parent session for a sub-agent (needed for result delivery
    /// on resume/team completion — #1197).
    pub fn get_parent_session_id(&self, id: &str) -> Option<Uuid> {
        self.agents
            .read()
            .expect("subagent manager lock poisoned")
            .get(id)
            .map(|a| a.parent_session_id)
    }

    /// Remove a terminated agent from tracking.
    pub fn remove(&self, id: &str) -> Option<SubAgent> {
        self.agents
            .write()
            .expect("subagent manager lock poisoned")
            .remove(id)
    }

    /// Format running/paused sub-agents as a compaction preamble block.
    ///
    /// Returns `None` when there are no non-terminal agents. The block is
    /// appended to the compaction summary so the post-compaction agent can
    /// still call `wait_agent`, `send_input`, `resume_agent`, and
    /// `close_agent` on them (#936).
    pub fn format_running_for_compaction(&self) -> Option<String> {
        let active: Vec<(String, String, SubAgentState, Uuid)> = {
            let agents = self.agents.read().expect("subagent manager lock poisoned");
            agents
                .values()
                .filter(|a| {
                    matches!(
                        a.state,
                        SubAgentState::Running | SubAgentState::AwaitingInput
                    )
                })
                .map(|a| (a.id.clone(), a.label.clone(), a.state.clone(), a.session_id))
                .collect()
        };
        if active.is_empty() {
            return None;
        }
        let mut block = String::from(
            "## Running Sub-Agents\n\n\
             The following sub-agents were alive when compaction fired. \
             They are still running in the background. Use the IDs below with \
             `wait_agent`, `send_input`, `resume_agent`, or `close_agent`. \
             Do NOT spawn duplicates.\n\n\
             | Agent ID | Label | State | Session ID |\n\
             |----------|-------|-------|------------|\n",
        );
        for (id, label, state, session_id) in &active {
            let state_str = match state {
                SubAgentState::Running => "Running",
                SubAgentState::AwaitingInput => "AwaitingInput",
                _ => continue,
            };
            block.push_str(&format!(
                "| `{id}` | {label} | {state_str} | `{session_id}` |\n",
            ));
        }
        Some(block)
    }
}

impl Default for SubAgentManager {
    fn default() -> Self {
        Self::new()
    }
}
