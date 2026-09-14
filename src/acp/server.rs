//! ACP method dispatch and the ACP-session state map.
//!
//! One `opencrabs acp` process serves one MonoCode thread in practice, but
//! the protocol allows several sessions per process, so state is a map keyed
//! by the ACP session id (the opencrabs session UUID as a string). Each entry
//! owns its model override and the cancel token of its in-flight turn.
//!
//! Dispatch shape: requests are answered inline when they are cheap
//! (initialize/new/load/set_model); `session/prompt` spawns a turn task so
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
use crate::cli::session_resolve::resolve_or_create_session;
use crate::services::SessionService;

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
    /// Working directory the client bound the session to (informational;
    /// the process was already spawned with this cwd).
    #[allow(dead_code)]
    pub cwd: String,
    /// Model override from `--model` or `session/set_model`.
    pub model: Mutex<Option<String>>,
    /// Cancel token of the in-flight turn; None when idle.
    pub active_cancel: Mutex<Option<CancellationToken>>,
}

/// Everything a dispatch or turn task needs, shared under one Arc.
pub struct ServerState {
    pub handle: TransportHandle,
    pub agent: Arc<AgentService>,
    pub sessions: SessionService,
    pub states: Mutex<HashMap<String, Arc<SessionState>>>,
    pub steer: SteerMap,
    pub default_model: Option<String>,
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
        default_model: Option<String>,
        steer: SteerMap,
    ) -> Self {
        let transport = Transport::spawn();
        let state = Arc::new(ServerState {
            handle: transport.handle(),
            agent,
            sessions,
            states: Mutex::new(HashMap::new()),
            steer,
            default_model,
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
                Self::session_new(state, id, params, None).await;
            }
            protocol::SESSION_LOAD => {
                let session_id = params
                    .get("sessionId")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                match session_id {
                    Some(sid) => Self::session_new(state, id, params, Some(&sid)).await,
                    None => state.handle.respond_error(
                        id,
                        protocol::INVALID_PARAMS,
                        "session/load requires sessionId",
                    ),
                }
            }
            protocol::SESSION_SET_MODEL => {
                Self::session_set_model(state, id, params).await;
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
    /// `None` as create and `Some(id)` as resume (prefix or full UUID).
    async fn session_new(state: Arc<ServerState>, id: Value, params: Value, resume: Option<&str>) {
        let cwd = params
            .get("cwd")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        match resolve_or_create_session(&state.sessions, resume, "ACP").await {
            Ok(session) => {
                let acp_id = session.id.to_string();
                let st = Arc::new(SessionState {
                    id: session.id,
                    cwd,
                    model: Mutex::new(state.default_model.clone()),
                    active_cancel: Mutex::new(None),
                });
                state.states.lock().await.insert(acp_id.clone(), st);
                state.handle.respond(id, json!({ "sessionId": acp_id }));
            }
            Err(e) => {
                state
                    .handle
                    .respond_error(id, protocol::INVALID_PARAMS, format!("session: {e}"));
            }
        }
    }

    async fn session_set_model(state: Arc<ServerState>, id: Value, params: Value) {
        let model = params.get("modelId").and_then(Value::as_str);
        match (Self::lookup(&state, &params).await, model) {
            (Some(st), Some(model)) => {
                *st.model.lock().await = Some(model.to_string());
                state.handle.respond(id, json!({}));
            }
            (None, _) => state.handle.respond_error(
                id,
                protocol::INVALID_PARAMS,
                "session/set_model: unknown session",
            ),
            (_, None) => state.handle.respond_error(
                id,
                protocol::INVALID_PARAMS,
                "session/set_model requires modelId",
            ),
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
        if st.active_cancel.lock().await.is_some() {
            state.handle.respond_error(
                id,
                protocol::INVALID_REQUEST,
                "turn already in progress for this session",
            );
            return;
        }
        tokio::spawn(turn::run_turn(state, st, id, text));
    }
}
