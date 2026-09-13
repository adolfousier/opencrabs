//! ACP method dispatch and the ACP-session state map.
//!
//! One `opencrabs acp` process serves one MonoCode thread in practice, but
//! the protocol allows several sessions per process, so state is a map keyed
//! by the ACP session id (the opencrabs session UUID as a string). Each entry
//! owns its model override and the cancel token of its in-flight turn.
//!
//! Dispatch shape: requests are answered inline when they are cheap
//! (initialize/new/load/set_model or set_mode); `session/prompt` spawns a
//! turn task so
//! the loop keeps reading — `session/cancel` must be processable mid-turn.
//! Notifications never get responses, per JSON-RPC.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use anyhow::Result;
use serde_json::{Value, json};
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use crate::brain::agent::{AgentService, QueuedUserMessage};
use crate::brain::provider::create_provider_by_name;
use crate::cli::session_resolve::resolve_or_create_session;
use crate::services::{MessageService, SessionService};
use crate::utils::provider_pair::parse_pair;

use super::catalog;
use super::protocol::{self, ClientMessage};
use super::transport::{Transport, TransportHandle};
use super::turn;

/// Per-session steering queue (`session/steer`): drained by the tool loop's
/// `MessageQueueCallback` between iterations. Shared with the agent service
/// builder, which is why it lives outside the server struct's Mutex.
pub type SteerMap = Arc<Mutex<HashMap<Uuid, VecDeque<QueuedUserMessage>>>>;

pub fn new_steer_map() -> SteerMap {
    Arc::new(Mutex::new(HashMap::new()))
}

/// One ACP session's live state.
pub struct SessionState {
    /// The opencrabs session — same UUID the ACP session id stringifies.
    pub id: Uuid,
    /// Model override from `--model` or `session/set_model`/`session/set_mode`.
    pub model: Mutex<Option<String>>,
    /// Permission policy from `session/set_mode` (default supervised).
    pub mode: Mutex<protocol::AcpMode>,
    /// Cancel token of the in-flight turn; None when idle.
    pub active_cancel: Mutex<Option<CancellationToken>>,
}

/// Everything a dispatch or turn task needs, shared under one Arc.
pub struct ServerState {
    pub handle: TransportHandle,
    pub agent: Arc<AgentService>,
    pub sessions: SessionService,
    pub messages: MessageService,
    pub states: Mutex<HashMap<String, Arc<SessionState>>>,
    pub steer: SteerMap,
    pub default_model: Option<String>,
    pub config: Arc<crate::config::Config>,
}

/// The server: owns the transport, dispatches to shared state.
pub struct AcpServer {
    state: Arc<ServerState>,
    transport: Transport,
}

impl AcpServer {
    pub fn new(
        agent: Arc<AgentService>,
        sessions: SessionService,
        messages: MessageService,
        default_model: Option<String>,
        steer: SteerMap,
        config: Arc<crate::config::Config>,
    ) -> Self {
        let transport = Transport::spawn();
        let state = Arc::new(ServerState {
            handle: transport.handle(),
            agent,
            sessions,
            messages,
            states: Mutex::new(HashMap::new()),
            steer,
            default_model,
            config,
        });
        Self { state, transport }
    }

    /// Read-dispatch loop. Returns when the client closes stdin (EOF), which
    /// is also the process-exit signal — the client owns our lifecycle.
    pub async fn run(mut self) -> Result<()> {
        tracing::info!("acp server: ready");
        while let Some(msg) = self.transport.next().await {
            match msg {
                ClientMessage::Request { id, method, params } => {
                    Self::dispatch_request(self.state.clone(), id, &method, params).await;
                }
                ClientMessage::Notification { method, params } => {
                    Self::dispatch_notification(&self.state, &method, params).await;
                }
                // Responses are resolved inside the transport reader; the
                // dispatch channel never carries them.
                ClientMessage::Response { .. } => {}
            }
        }
        tracing::info!("acp server: stdin closed, exiting");
        Ok(())
    }

    async fn dispatch_request(state: Arc<ServerState>, id: Value, method: &str, params: Value) {
        match method {
            protocol::INITIALIZE => {
                state.handle.respond(id, protocol::initialize_result());
            }
            protocol::SESSION_NEW => {
                Self::session_new(state, id, None).await;
            }
            protocol::SESSION_LOAD => {
                let session_id = params
                    .get("sessionId")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                match session_id {
                    Some(sid) => Self::session_new(state, id, Some(&sid)).await,
                    None => state.handle.respond_error(
                        id,
                        protocol::INVALID_PARAMS,
                        "session/load requires sessionId",
                    ),
                }
            }
            protocol::SESSION_SET_MODEL | protocol::SESSION_SET_MODE => {
                Self::session_set_model(state, id, params).await;
            }
            protocol::SESSION_SET_MODE => {
                Self::session_set_mode(state, id, params).await;
            }
            protocol::SESSION_COMPACT => {
                Self::session_compact(state, id, params).await;
            }
            protocol::SESSION_PROMPT => {
                Self::session_prompt(state, id, params).await;
            }
            other => {
                state.handle.respond_error(
                    id,
                    protocol::METHOD_NOT_FOUND,
                    format!("method not found: {other}"),
                );
            }
        }
    }

    async fn dispatch_notification(state: &Arc<ServerState>, method: &str, params: Value) {
        match method {
            protocol::SESSION_CANCEL => {
                if let Some(st) = Self::lookup(state, &params).await
                    && let Some(token) = st.active_cancel.lock().await.as_ref()
                {
                    token.cancel();
                }
            }
            protocol::SESSION_STEER => {
                if let (Some(st), Some(text)) = (
                    Self::lookup(state, &params).await,
                    protocol::prompt_text(&params),
                ) {
                    state
                        .steer
                        .lock()
                        .await
                        .entry(st.id)
                        .or_default()
                        .push_back(QueuedUserMessage::plain(text));
                }
            }
            other => {
                tracing::debug!("acp server: ignoring unknown notification {other}");
            }
        }
    }

    /// Resolve params.sessionId to live state.
    async fn lookup(state: &Arc<ServerState>, params: &Value) -> Option<Arc<SessionState>> {
        let sid = params.get("sessionId").and_then(Value::as_str)?;
        state.states.lock().await.get(sid).cloned()
    }

    /// `session/new` and `session/load` share one body: the resolver treats
    /// `None` as create and `Some(id)` as resume (prefix or full UUID). The
    /// client's `cwd` is accepted and ignored: the process already runs in
    /// its own working directory, so there is nothing to bind it to.
    async fn session_new(state: Arc<ServerState>, id: Value, resume: Option<&str>) {
        match resolve_or_create_session(&state.sessions, resume, "ACP").await {
            Ok(session) => {
                let acp_id = session.id.to_string();
                let st = Arc::new(SessionState {
                    id: session.id,
                    model: Mutex::new(state.default_model.clone()),
                    mode: Mutex::new(protocol::AcpMode::default()),
                    active_cancel: Mutex::new(None),
                });
                state.states.lock().await.insert(acp_id.clone(), st.clone());
                // The agent service's per-session model maps are in-memory,
                // so a fresh acp process starts blank while the session row
                // still knows the user's pick — rehydrate from the row.
                if resume.is_some() {
                    Self::restore_session_model(&state, &session, &st).await;
                }
                // ACP: an agent advertising loadSession replays the stored
                // transcript as session/update notifications BEFORE answering
                // the load, so clients without their own transcript store
                // (Zed et al.) render history. MonoCode mutes these — it
                // restores its own persisted blocks — but the replay is the
                // protocol contract, not a client favor.
                if resume.is_some()
                    && let Ok(history) = state.messages.list_messages_for_session(session.id).await
                {
                    for update in protocol::replay_updates(&history) {
                        state.handle.send(protocol::session_update(&acp_id, update));
                    }
                }
                let current = st.model.lock().await.clone();
                let models = catalog::models_payload(&state.config, current.as_deref());
                let modes = protocol::modes_payload(*st.mode.lock().await);
                state.handle.respond(
                    id,
                    json!({ "sessionId": acp_id, "models": models, "modes": modes }),
                );
                // Slash-command discovery pushes after the response so the
                // client's picker fills in as soon as the session exists.
                let commands = catalog::commands_payload();
                if !commands.is_empty() {
                    state.handle.send(protocol::session_update(
                        &acp_id,
                        json!({
                            "sessionUpdate": "available_commands_update",
                            "availableCommands": commands,
                        }),
                    ));
                }
            }
            Err(e) => {
                state
                    .handle
                    .respond_error(id, protocol::INVALID_PARAMS, format!("session: {e}"));
            }
        }
    }

    async fn session_set_model(state: Arc<ServerState>, id: Value, params: Value) {
        let model = params
            .get("modelId")
            .or_else(|| params.get("modeId"))
            .and_then(Value::as_str);
        let (Some(st), Some(model)) = (Self::lookup(&state, &params).await, model) else {
            let msg = if model.is_none() {
                "session/set_model requires modelId (modeId accepted)"
            } else {
                "session/set_model: unknown session"
            };
            state
                .handle
                .respond_error(id, protocol::INVALID_PARAMS, msg);
            return;
        };
        // A `provider/model` pair switches the session's provider too; a bare
        // model name re-serves through the session's current provider.
        if let Ok((provider_name, bare_model)) = parse_pair(model) {
            match create_provider_by_name(&state.config, &provider_name).await {
                Ok(provider) => {
                    state
                        .agent
                        .swap_provider_for_session(st.id, provider, bare_model.clone());
                    state.agent.mark_manual_switch(st.id, bare_model);
                    // st.model holds the pair form — models_payload matches
                    // currentModelId against available pair ids.
                    *st.model.lock().await = Some(model.to_string());
                }
                Err(e) => {
                    state.handle.respond_error(
                        id,
                        protocol::INVALID_PARAMS,
                        format!("session/set_model: {e}"),
                    );
                    return;
                }
            }
        } else {
            state.agent.set_session_model(st.id, model.to_string());
            *st.model.lock().await = Some(model.to_string());
        }
        state.handle.respond(id, json!({}));
    }

    /// `session/set_mode`: validate the advertised id and store it. Unknown
    /// modes are a hard error — silently accepting one would leave client and
    /// server disagreeing about the approval policy.
    async fn session_set_mode(state: Arc<ServerState>, id: Value, params: Value) {
        let mode_id = params.get("modeId").and_then(Value::as_str);
        let (Some(st), Some(mode_id)) = (Self::lookup(&state, &params).await, mode_id) else {
            let msg = if mode_id.is_none() {
                "session/set_mode requires modeId"
            } else {
                "session/set_mode: unknown session"
            };
            state
                .handle
                .respond_error(id, protocol::INVALID_PARAMS, msg);
            return;
        };
        match protocol::AcpMode::parse(mode_id) {
            Some(mode) => {
                *st.mode.lock().await = mode;
                state.handle.respond(id, json!({}));
            }
            None => state.handle.respond_error(
                id,
                protocol::INVALID_PARAMS,
                format!("session/set_mode: unknown modeId '{mode_id}'"),
            ),
        }
    }

    /// Rehydrate the per-session model/provider override from the session row
    /// on `session/load`. Failure degrades down the chain: a provider that no
    /// longer exists in config falls back to a bare model pin, and a session
    /// with no stored pick keeps the default.
    async fn restore_session_model(
        state: &Arc<ServerState>,
        session: &crate::db::models::Session,
        st: &Arc<SessionState>,
    ) {
        let model = session.model.clone().filter(|m| !m.trim().is_empty());
        let provider_name = session
            .provider_name
            .clone()
            .filter(|p| !p.trim().is_empty());
        if let (Some(provider_name), Some(model)) = (provider_name, model.clone()) {
            match create_provider_by_name(&state.config, &provider_name).await {
                Ok(provider) => {
                    state
                        .agent
                        .swap_provider_for_session(session.id, provider, model.clone());
                    state.agent.mark_manual_switch(session.id, model.clone());
                    *st.model.lock().await = Some(format!("{provider_name}/{model}"));
                    return;
                }
                Err(e) => {
                    tracing::debug!("acp: provider restore skipped ({provider_name}): {e}");
                }
            }
        }
        if let Some(model) = model {
            state.agent.set_session_model(session.id, model.clone());
            *st.model.lock().await = Some(model);
        }
    }

    /// Spawn the turn task and return immediately — the response travels with
    /// the task, so the loop stays free to process `session/cancel`.
    async fn session_prompt(state: Arc<ServerState>, id: Value, params: Value) {
        let Some(st) = Self::lookup(&state, &params).await else {
            state.handle.respond_error(
                id,
                protocol::INVALID_PARAMS,
                "session/prompt: unknown session — call session/new first",
            );
            return;
        };
        let Some(text) = protocol::prompt_text(&params) else {
            state.handle.respond_error(
                id,
                protocol::INVALID_PARAMS,
                "session/prompt: empty prompt",
            );
            return;
        };
        // Claim the in-flight slot under the same lock that checks it: the
        // token is created here, before any other prompt can observe the
        // session idle. run_turn used to create it two awaits after this
        // check, leaving a window where two fast prompts both saw None and
        // both spawned turns against one session.
        let cancel = CancellationToken::new();
        let mut guard = st.active_cancel.lock().await;
        if guard.is_some() {
            state.handle.respond_error(
                id,
                protocol::INVALID_REQUEST,
                "turn already in progress for this session",
            );
            return;
        }
        *guard = Some(cancel.clone());
        drop(guard);
        tokio::spawn(turn::run_turn(state, st, id, text, cancel));
    }
    /// `session/compact`: drive the loop's own manual-compaction path — the
    /// `[SYSTEM: Compact context now.]` marker the TUI and channels use. The
    /// turn streams and answers like any other; the marker keeps the magic
    /// string out of the client's chat because prompts are never echoed.
    /// Claims the in-flight slot exactly like `session/prompt` so a compact
    /// cannot race a live turn on the same session.
    async fn session_compact(state: Arc<ServerState>, id: Value, params: Value) {
        let Some(st) = Self::lookup(&state, &params).await else {
            state.handle.respond_error(
                id,
                protocol::INVALID_PARAMS,
                "session/compact: unknown session — call session/new first",
            );
            return;
        };
        let cancel = CancellationToken::new();
        let mut guard = st.active_cancel.lock().await;
        if guard.is_some() {
            state.handle.respond_error(
                id,
                protocol::INVALID_REQUEST,
                "turn already in progress for this session",
            );
            return;
        }
        *guard = Some(cancel.clone());
        drop(guard);
        tokio::spawn(turn::run_turn(
            state,
            st,
            id,
            "[SYSTEM: Compact context now. Summarize this conversation for continuity.]"
                .to_string(),
            cancel,
        ));
    }
}
