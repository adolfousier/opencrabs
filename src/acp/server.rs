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
use crate::db::repository::SessionListOptions;
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
                Self::session_new(state, id, None, false).await;
            }
            protocol::SESSION_LOAD => {
                let session_id = params
                    .get("sessionId")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                match session_id {
                    // `session/load` is the replaying half: the transcript is
                    // streamed as session/update frames before the response.
                    Some(sid) => Self::session_new(state, id, Some(&sid), true).await,
                    None => state.handle.respond_error(
                        id,
                        protocol::INVALID_PARAMS,
                        "session/load requires sessionId",
                    ),
                }
            }
            // #1815 F5: the lifecycle half of the v1 session vocabulary. Each
            // was a METHOD_NOT_FOUND here, so a client that trusted our
            // advertised `loadSession` and asked for a sibling got an error.
            protocol::SESSION_RESUME => {
                let session_id = params
                    .get("sessionId")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                match session_id {
                    // `session/resume` is the non-replaying half, per the schema
                    // ("without returning previous messages (unlike
                    // session/load)"), and its response has no sessionId field.
                    Some(sid) => Self::session_new(state, id, Some(&sid), false).await,
                    None => state.handle.respond_error(
                        id,
                        protocol::INVALID_PARAMS,
                        "session/resume requires sessionId",
                    ),
                }
            }
            protocol::SESSION_LIST => {
                Self::session_list(state, id, params).await;
            }
            protocol::SESSION_CLOSE => {
                Self::session_close(state, id, params).await;
            }
            protocol::SESSION_DELETE => {
                Self::session_delete(state, id, params).await;
            }
            protocol::SESSION_SET_CONFIG_OPTION => {
                Self::session_set_config_option(state, id, params).await;
            }
            protocol::SESSION_SET_MODEL | protocol::SESSION_SET_MODEL_LEGACY => {
                Self::session_set_model(state, id, params).await;
            }
            protocol::SESSION_SET_MODE => {
                Self::session_set_mode(state, id, params).await;
            }
            protocol::SESSION_COMPACT | protocol::SESSION_COMPACT_LEGACY => {
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
            protocol::SESSION_STEER | protocol::SESSION_STEER_LEGACY => {
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

    /// `session/new`, `session/load` and `session/resume` share one body: the
    /// resolver treats `None` as create and `Some(id)` as an existing session
    /// (prefix or full UUID). `replay` is the only difference between the two
    /// resume spellings: v1 says `session/load` replays the transcript and
    /// `session/resume` must not (#1815 F5). The client's `cwd` is accepted and
    /// ignored: the process already runs in its own working directory, so there
    /// is nothing to bind it to.
    async fn session_new(state: Arc<ServerState>, id: Value, resume: Option<&str>, replay: bool) {
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
                if replay
                    && let Ok(history) = state.messages.list_messages_for_session(session.id).await
                {
                    for update in protocol::replay_updates(&history) {
                        state.handle.send(protocol::session_update(&acp_id, update));
                    }
                    // Restore the context meter: the provider's own last
                    // measurement, falling back to the session row's
                    // lifetime total. A real count beats silence; both beat
                    // a tokenized estimate of raw content.
                    let used = protocol::replay_usage(&history)
                        .or((session.token_count > 0).then_some(session.token_count));
                    if let Some(used) = used {
                        state.handle.send(protocol::session_update(
                            &acp_id,
                            // `used` comes off a DB column typed i64; the
                            // schema field is uint64 with minimum 0, so a
                            // nonsensical negative reads as an empty meter
                            // instead of a frame a strict client rejects.
                            protocol::usage_update(
                                used.max(0) as u64,
                                state.agent.context_limit_for_session(session.id) as u64,
                            ),
                        ));
                    }
                }
                let current = st.model.lock().await.clone();
                let models = catalog::models_payload(&state.config, current.as_deref());
                let config_options =
                    catalog::config_options_payload(&state.config, current.as_deref());
                let modes = protocol::modes_payload(*st.mode.lock().await);
                // A resumed session reached us through session/load, and
                // LoadSessionResponse has no sessionId field to echo (#1815 F4).
                state.handle.respond(
                    id,
                    protocol::session_response(
                        &acp_id,
                        models,
                        modes,
                        config_options,
                        resume.is_none(),
                    ),
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
        let model = params.get("modelId").and_then(Value::as_str);
        let (Some(st), Some(model)) = (Self::lookup(&state, &params).await, model) else {
            let msg = if model.is_none() {
                "session/set_model requires modelId"
            } else {
                "session/set_model: unknown session"
            };
            state
                .handle
                .respond_error(id, protocol::INVALID_PARAMS, msg);
            return;
        };
        match Self::apply_model_selection(&state, &st, model).await {
            Ok(()) => state.handle.respond(id, json!({})),
            Err(e) => state.handle.respond_error(
                id,
                protocol::INVALID_PARAMS,
                format!("session/set_model: {e}"),
            ),
        }
    }

    /// The one model-switch path. A `provider/model` pair swaps the session's
    /// provider too; a bare model name re-serves through the provider the
    /// session is already on. The pick is written to the session row so a later
    /// process can rehydrate it, since the agent service's maps are in-memory.
    ///
    /// Shared by `_opencrabs/set_model` and the official
    /// `session/set_config_option` (#1815 F5): two spellings of "change the
    /// model" must not grow different behavior.
    async fn apply_model_selection(
        state: &Arc<ServerState>,
        st: &Arc<SessionState>,
        model: &str,
    ) -> Result<(), String> {
        if let Ok((provider_name, bare_model)) = parse_pair(model) {
            let provider = create_provider_by_name(&state.config, &provider_name)
                .await
                .map_err(|e| e.to_string())?;
            state
                .agent
                .swap_provider_for_session(st.id, provider, bare_model.clone());
            state.agent.mark_manual_switch(st.id, bare_model);
            // st.model holds the pair form: models_payload matches
            // currentModelId against the available pair ids.
            *st.model.lock().await = Some(model.to_string());
        } else {
            state.agent.set_session_model(st.id, model.to_string());
            *st.model.lock().await = Some(model.to_string());
        }
        // Persistence failure is a warning, not an error: the switch took
        // effect for this process, and the row write is what a future one
        // needs. Refusing the request would be worse than a lost preference.
        match state.sessions.get_session_required(st.id).await {
            Ok(mut row) => {
                match parse_pair(model) {
                    Ok((provider_name, bare_model)) => {
                        row.provider_name = Some(provider_name);
                        row.model = Some(bare_model);
                    }
                    Err(_) => row.model = Some(model.to_string()),
                }
                if let Err(e) = state.sessions.update_session(&row).await {
                    tracing::warn!("acp: model pick not persisted for {}: {e}", st.id);
                }
            }
            Err(e) => tracing::warn!("acp: model pick not persisted for {}: {e}", st.id),
        }
        Ok(())
    }

    /// `session/list` (`ListSessionsRequest` -> `ListSessionsResponse`).
    ///
    /// Backed by the same session store the TUI list uses, with its defaults
    /// kept: archived rows and `subagent:` rows are not listed, for the same
    /// reason #931 gave for the TUI picker (a sub-agent session is an
    /// implementation detail of one `spawn_agent` call, and a busy turn buries
    /// the sessions a client actually wants). Pagination is opaque to the
    /// client but not to us: the cursor is the row offset in decimal.
    async fn session_list(state: Arc<ServerState>, id: Value, params: Value) {
        const PAGE: usize = 50;
        let cwd_filter = params.get("cwd").and_then(Value::as_str);
        let offset = match params.get("cursor").and_then(Value::as_str) {
            None => 0usize,
            Some(raw) => match raw.parse::<usize>() {
                Ok(offset) => offset,
                Err(_) => {
                    state.handle.respond_error(
                        id,
                        protocol::INVALID_PARAMS,
                        format!("session/list: unparseable cursor '{raw}'"),
                    );
                    return;
                }
            },
        };
        let rows = match state
            .sessions
            .list_sessions(SessionListOptions {
                include_archived: false,
                limit: Some(PAGE),
                offset,
                query: None,
                include_subagents: false,
            })
            .await
        {
            Ok(rows) => rows,
            Err(e) => {
                state.handle.respond_error(
                    id,
                    protocol::INTERNAL_ERROR,
                    format!("session/list: {e}"),
                );
                return;
            }
        };
        // A session with no stored working directory ran in this process's
        // cwd, which is the only honest answer for it.
        let process_cwd = std::env::current_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        let infos: Vec<Value> = rows
            .iter()
            .filter(|row| match cwd_filter {
                // A session with no stored directory ran here, so it matches a
                // `cwd` filter that names this process's directory.
                Some(want) => row.working_directory.as_deref().unwrap_or(&process_cwd) == want,
                None => true,
            })
            .map(|row| {
                protocol::session_info(
                    &row.id.to_string(),
                    row.working_directory.as_deref().unwrap_or(&process_cwd),
                    row.title.as_deref(),
                    Some(&row.updated_at.to_rfc3339()),
                )
            })
            .collect();
        // A full page means more may exist; a short one is the last page, so
        // `nextCursor` is omitted rather than sent as an empty string.
        let next_cursor = if rows.len() == PAGE {
            Some((offset + PAGE).to_string())
        } else {
            None
        };
        state
            .handle
            .respond(id, protocol::list_sessions_payload(infos, next_cursor));
    }

    /// Resolve an ACP `sessionId` to the stored session. Accepts the same
    /// prefix spellings `session/load` does, so a client that got a full UUID
    /// from `session/list` and a human who typed eight characters hit the same
    /// row.
    async fn resolve_session(
        state: &Arc<ServerState>,
        session_id: &str,
    ) -> Result<crate::db::models::Session, String> {
        resolve_or_create_session(&state.sessions, Some(session_id), "ACP")
            .await
            .map_err(|e| e.to_string())
    }

    /// `session/close` (`CloseSessionRequest` -> `{}`).
    ///
    /// The spec wording is a MUST: cancel any ongoing work as if
    /// `session/cancel` had been called, then free the resources. The stored
    /// transcript and the session row survive: close is not delete.
    async fn session_close(state: Arc<ServerState>, id: Value, params: Value) {
        let Some(session_id) = params.get("sessionId").and_then(Value::as_str) else {
            state.handle.respond_error(
                id,
                protocol::INVALID_PARAMS,
                "session/close requires sessionId",
            );
            return;
        };
        match Self::resolve_session(&state, session_id).await {
            Ok(session) => {
                Self::release_live_state(&state, &session.id).await;
                state.handle.respond(id, json!({}));
            }
            Err(e) => state.handle.respond_error(
                id,
                protocol::INVALID_PARAMS,
                format!("session/close: {e}"),
            ),
        }
    }

    /// `session/delete` (`DeleteSessionRequest` -> `{}`).
    ///
    /// Deletes the stored session, so it removes the row from the store the
    /// same way the TUI's own delete does, after closing it. This is the one
    /// lifecycle method that destroys data; it only ever runs because a client
    /// asked for this session id specifically.
    async fn session_delete(state: Arc<ServerState>, id: Value, params: Value) {
        let Some(session_id) = params.get("sessionId").and_then(Value::as_str) else {
            state.handle.respond_error(
                id,
                protocol::INVALID_PARAMS,
                "session/delete requires sessionId",
            );
            return;
        };
        match Self::resolve_session(&state, session_id).await {
            Ok(session) => {
                Self::release_live_state(&state, &session.id).await;
                if let Err(e) = state.sessions.delete_session(session.id).await {
                    state.handle.respond_error(
                        id,
                        protocol::INTERNAL_ERROR,
                        format!("session/delete: {e}"),
                    );
                    return;
                }
                tracing::info!("acp: session/delete removed {}", session.id);
                state.handle.respond(id, json!({}));
            }
            Err(e) => state.handle.respond_error(
                id,
                protocol::INVALID_PARAMS,
                format!("session/delete: {e}"),
            ),
        }
    }

    /// Drop in-process state for a session: cancel its in-flight turn first,
    /// then remove it from the live map so its steering queue and model
    /// override go with it.
    async fn release_live_state(state: &Arc<ServerState>, session_uuid: &Uuid) {
        let acp_id = session_uuid.to_string();
        let live = state.states.lock().await.remove(&acp_id);
        if let Some(st) = live {
            if let Some(token) = st.active_cancel.lock().await.as_ref() {
                token.cancel();
            }
            state.steer.lock().await.remove(session_uuid);
        }
    }

    /// `session/set_config_option` (`SetSessionConfigOptionRequest` ->
    /// `SetSessionConfigOptionResponse`).
    ///
    /// `model` is the only option this agent publishes, and setting it is the
    /// same switch `_opencrabs/set_model` performs, so both go through
    /// `apply_model_selection`. The response must carry the FULL option set
    /// with the new current value, which is what lets a client that never
    /// called `session/new` stay in sync after a write.
    async fn session_set_config_option(state: Arc<ServerState>, id: Value, params: Value) {
        let config_id = params.get("configId").and_then(Value::as_str);
        let Some(st) = Self::lookup(&state, &params).await else {
            state
                .handle
                .respond_error(id, protocol::INVALID_PARAMS, "unknown session");
            return;
        };
        let Some(config_id) = config_id else {
            state.handle.respond_error(
                id,
                protocol::INVALID_PARAMS,
                "session/set_config_option requires configId",
            );
            return;
        };
        if config_id != catalog::CONFIG_OPTION_MODEL {
            state.handle.respond_error(
                id,
                protocol::INVALID_PARAMS,
                format!("session/set_config_option: unknown configId '{config_id}'"),
            );
            return;
        }
        let Some(value) = config_option_value(&params) else {
            state.handle.respond_error(
                id,
                protocol::INVALID_PARAMS,
                "session/set_config_option: value must be a string for the 'model' option",
            );
            return;
        };
        if let Err(e) = Self::apply_model_selection(&state, &st, &value).await {
            state.handle.respond_error(
                id,
                protocol::INVALID_PARAMS,
                format!("session/set_config_option: {e}"),
            );
            return;
        }
        let current = st.model.lock().await.clone();
        let config_options = catalog::config_options_payload(&state.config, current.as_deref());
        state
            .handle
            .respond(id, protocol::config_options_response(config_options));
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

/// Read the `value` of a `session/set_config_option` request.
///
/// v1 carries it as a tagged union: either `{ "value": "<string>" }` (the
/// default when `type` is absent on the wire) or `{ "type": "boolean",
/// "value": true }`. Only the string form can name a model, and the `model`
/// option is the only one this agent publishes, so a boolean is refused rather
/// than coerced: `"true"` is not a model id.
pub(crate) fn config_option_value(params: &Value) -> Option<String> {
    params
        .get("value")
        .and_then(Value::as_str)
        .map(str::to_string)
}
