use super::types::*;
use crate::brain::provider::Provider;
use crate::brain::tools::ToolRegistry;
use crate::services::ServiceContext;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use uuid::Uuid;

/// Maximum number of recently-touched paths surfaced to the agent in
/// the system prompt for any one project. ~12 entries × ~30 tokens
/// each ≈ 400 tokens worst-case — small enough that the win on
/// cross-session continuity dominates the cost.
pub(super) const RECENT_PATHS_CAP: usize = 12;

/// A captured manual provider/model switch: `(epoch, provider, model)`.
/// Used to restore the user's pick after an in-flight turn took a fallback.
type ManualSwitchPin = (u64, Arc<dyn Provider>, String);

/// Live-rebuild handle for the system brain (#213).
///
/// The system brain was historically built once at startup and cached as a
/// static string, so edits to brain files (manual, `write_opencrabs_file`,
/// `self_improve`) were invisible until the process restarted. This handle
/// rebuilds the brain from disk on the next turn whenever a brain file's
/// mtime advances, and otherwise returns the byte-identical cached render so
/// the provider prompt cache stays warm when nothing changed.
///
/// Cheap to clone (it's an `Arc`), so a provider/model rebuild can carry the
/// same handle (and its warm cache) forward without re-reading disk.
#[derive(Clone)]
pub struct BrainRebuild {
    inner: Arc<BrainRebuildInner>,
}

struct BrainRebuildInner {
    loader: crate::brain::prompt_builder::BrainLoader,
    runtime_info: Option<crate::brain::prompt_builder::RuntimeInfo>,
    /// `true` → `build_core_brain` (TUI/channels), `false` → `build_system_brain`.
    core: bool,
    /// Append `LAZY_TOOLS_PROMPT` after the brain, matching startup assembly.
    lazy_tools: bool,
    /// Live working-directory handle shared with tool execution. `/cd` mutates
    /// it, so reading it each render lets the project-directive scan follow the
    /// current directory instead of the frozen startup path baked into
    /// `runtime_info`. `None` disables the follow (brain rebuild without a
    /// working-dir source, e.g. some tests).
    live_cwd: Option<Arc<std::sync::RwLock<std::path::PathBuf>>>,
    cache: std::sync::RwLock<BrainCache>,
}

struct BrainCache {
    mtime: std::time::SystemTime,
    /// The working directory this render was built for. A `/cd` changes it and
    /// forces a rebuild so the directive index follows the current project.
    cwd: Option<std::path::PathBuf>,
    /// Newest mtime across the directive files under `cwd`. Advances when a
    /// directive file is added/edited/removed, forcing a rebuild so the index
    /// never goes stale.
    dir_mtime: Option<std::time::SystemTime>,
    rendered: String,
}

impl BrainRebuild {
    /// Build a handle seeded with the brain already assembled at startup.
    /// The seed is returned verbatim until a brain file changes on disk, so
    /// no work happens and the prompt cache stays warm on the common path.
    pub fn new(
        loader: crate::brain::prompt_builder::BrainLoader,
        runtime_info: Option<crate::brain::prompt_builder::RuntimeInfo>,
        core: bool,
        lazy_tools: bool,
        seed: String,
        live_cwd: Option<Arc<std::sync::RwLock<std::path::PathBuf>>>,
    ) -> Self {
        let mtime = loader.brain_files_mtime();
        let cwd = live_cwd
            .as_ref()
            .and_then(|h| h.read().ok().map(|p| p.clone()));
        let dir_mtime = cwd
            .as_deref()
            .and_then(crate::brain::directives::directives_mtime);
        Self {
            inner: Arc::new(BrainRebuildInner {
                loader,
                runtime_info,
                core,
                lazy_tools,
                live_cwd,
                cache: std::sync::RwLock::new(BrainCache {
                    mtime,
                    cwd,
                    dir_mtime,
                    rendered: seed,
                }),
            }),
        }
    }

    /// The system brain for this turn. Returns the cached render unless a
    /// brain file changed, the working directory changed (`/cd`), or a project
    /// directive file was added/edited/removed. In any of those cases it
    /// rebuilds from disk and updates the cache; otherwise it returns the
    /// byte-identical cached render so the provider prompt cache stays warm.
    pub fn render(&self) -> String {
        let i = &self.inner;
        let latest = i.loader.brain_files_mtime();
        let cwd = i
            .live_cwd
            .as_ref()
            .and_then(|h| h.read().ok().map(|p| p.clone()));
        let dir_mtime = cwd
            .as_deref()
            .and_then(crate::brain::directives::directives_mtime);
        {
            let cache = i.cache.read().expect("brain cache lock poisoned");
            if latest <= cache.mtime && cwd == cache.cwd && dir_mtime <= cache.dir_mtime {
                return cache.rendered.clone();
            }
        }
        let runtime_info = self.effective_runtime_info(cwd.as_deref());
        let mut brain = if i.core {
            i.loader.build_core_brain(runtime_info.as_ref())
        } else {
            i.loader.build_system_brain(runtime_info.as_ref())
        };
        if i.lazy_tools {
            brain.push_str(&crate::brain::tools::catalog::tool_access_prompt());
        }
        let mut cache = i.cache.write().expect("brain cache lock poisoned");
        *cache = BrainCache {
            mtime: latest,
            cwd,
            dir_mtime,
            rendered: brain.clone(),
        };
        brain
    }

    /// The frozen `runtime_info` with its `working_directory` overridden by the
    /// live cwd (tilde-collapsed for display and cache-key stability), so the
    /// directive scan and the "Working directory" line follow `/cd`. When no
    /// live cwd is available the frozen value is used unchanged.
    fn effective_runtime_info(
        &self,
        cwd: Option<&std::path::Path>,
    ) -> Option<crate::brain::prompt_builder::RuntimeInfo> {
        match (self.inner.runtime_info.clone(), cwd) {
            (Some(mut ri), Some(path)) => {
                ri.working_directory = Some(crate::brain::tools::error::collapse_home(path));
                Some(ri)
            }
            (other, _) => other,
        }
    }
}

/// Agent Service for managing AI conversations
pub struct AgentService {
    /// Default LLM provider — used for brand-new sessions that haven't
    /// had an explicit provider choice yet, and for channels / callers
    /// that invoke the agent without a session_id.
    pub(super) provider: std::sync::RwLock<Arc<dyn Provider>>,

    /// Per-session provider isolation. Every session that has ever been
    /// seen (via `/models` pick, `load_session`, or first agent turn)
    /// gets its own `Arc<dyn Provider>` here. In-flight agent turns
    /// read their session's entry via `provider_for_session(id)` so a
    /// foreground pane-switch or model-pick on a DIFFERENT session
    /// can't yank the active provider out from under a background
    /// turn. Before this map, `self.provider` was a single shared
    /// pointer — opening `/sessions` during a 47s cargo-clippy on one
    /// pane silently rewrote the running turn's endpoint to whatever
    /// the other session had saved (2026-04-17 17:01 logs).
    pub(super) session_providers: std::sync::RwLock<HashMap<Uuid, Arc<dyn Provider>>>,

    /// Per-session model name overrides. `swap_provider_for_session`
    /// installs a fresh provider whose `default_model()` reflects the
    /// global config rather than the model the session actually wants
    /// (e.g. the user switched from `qwen-3.7-plus` to `qwen-3.7-max`
    /// in Telegram). The actual LLM request reads the right model from
    /// the session DB row via `tool_loop`, but every "current model"
    /// display surface goes through `provider_model_for_session()`,
    /// which used to surface the provider default instead. This map
    /// captures the per-session pick so the display stays in sync with
    /// what's actually being sent on the wire.
    pub(super) session_models: std::sync::RwLock<HashMap<Uuid, String>>,

    /// Captures a USER's manual provider/model switch so an in-flight turn's
    /// automatic fallback can't permanently overwrite it. Maps session →
    /// `(epoch, provider, model)`. A turn snapshots the epoch at start; if it
    /// changed by the time the turn finishes, the user switched mid-turn and
    /// `run_tool_loop_inner` RESTORES this pinned pair AFTER the turn
    /// completes. Crucially this happens off the completion path — the turn
    /// always runs to a full response first, so honoring the switch can never
    /// drop or contaminate the request (the 2026-06-08 regression came from
    /// suppressing the fallback's model-sync event mid-turn; this never does).
    pub(super) manual_switch: std::sync::RwLock<HashMap<Uuid, ManualSwitchPin>>,

    /// What a plan-mode provider override replaced, so the session can be put
    /// back when the plan ends (#792).
    ///
    /// Without this the override would be permanent. `ensure_session_provider_restored`
    /// early-returns whenever the session already has a `session_providers`
    /// entry, so a swap left in the map is never undone by the normal restore
    /// path: the session would silently keep running on the planning model
    /// long after the plan archived, with the footer showing it as if the user
    /// had chosen it. That is the #704/#705 silent-switch failure exactly.
    pub(super) plan_mode_swap:
        std::sync::RwLock<HashMap<Uuid, super::plan_mode_provider::PlanModeSwap>>,

    /// Per-session context window overrides. When a session's provider
    /// has a custom `configured_context_window()`, it's cached here so
    /// compaction and budget checks use the correct window even when
    /// the global provider changes (e.g. user switches models on another
    /// pane). Mirrors the `session_providers` pattern.
    pub(super) session_context_limits: std::sync::RwLock<HashMap<Uuid, u32>>,

    /// Per-session counter of consecutive primary-provider failures
    /// that needed a successful fallback rescue. Used to delay the
    /// "stick the fallback as session's provider" decision until we
    /// have strong evidence the primary is genuinely broken — not
    /// just temporarily blipping. Resets to 0 on any primary success
    /// (which is the common case for transient outages where the
    /// primary recovers on the very next request).
    ///
    /// When the count reaches `STICKY_FALLBACK_THRESHOLD` (4 — see
    /// the fallback-success commit site in `tool_loop.rs`), the
    /// fallback gets persisted into `session_providers` and the
    /// per-session model override; before then the fallback rescues
    /// only this single request and the primary is restored for the
    /// next one.
    pub(super) session_primary_failure_streak: std::sync::RwLock<HashMap<Uuid, u32>>,

    /// Per-session set of skill names that have been invoked. When a skill
    /// is activated (via `/skill-name`), its name is recorded here so
    /// the tool loop can re-inject the full skill body into the system brain
    /// after compaction (issue #219). Without this, the 120-char clipped
    /// description in `push_commands_and_skills` is all that survives.
    pub(super) active_skills: std::sync::RwLock<HashMap<Uuid, HashSet<String>>>,

    /// Per-session flag: has the pre-compaction context-pressure warning
    /// (#909) already been emitted for the current band entry? Set true when
    /// the warning fires (usage in 55-64% band), cleared when usage drops
    /// below 55% so a fresh entry into the band warns again. Keeps the
    /// transient nudge to once-per-entry instead of every turn.
    pub(super) session_pressure_warned: std::sync::RwLock<HashMap<Uuid, bool>>,
    /// Observed wall-clock duration of each session's last SUCCESSFUL
    /// compaction (#29 E2). Feeds the `predicted` ETA hint on the next
    /// `Compacting` event — grounded in what actually happened instead of a
    /// static guess. Written at CompactionSummary emit, read at Compacting
    /// emit; same per-session map pattern as `session_pressure_warned`.
    pub(super) last_compaction_elapsed: std::sync::RwLock<HashMap<Uuid, std::time::Duration>>,

    /// Mirrors `[agent] background_compaction` from config.toml. Default true.
    pub(super) background_compaction: bool,
    /// Summariser tasks running against a snapshot of a session's context
    /// while that session keeps taking turns. At most one per session: a
    /// second would summarise a conversation the first is about to replace.
    ///
    /// Entries are taken OUT of the map to be awaited, never awaited under
    /// the lock. Nothing in here is ever cancelled to make room — see
    /// `super::compaction::PendingCompaction`.
    pub(super) pending_compactions:
        std::sync::Mutex<HashMap<Uuid, super::compaction::PendingCompaction>>,

    /// Per-session ring buffer of outgoing assistant texts (#957). The
    /// cross-turn announcement guard: near-identical turn-final texts that
    /// repeat within the ring nudge once, then abort. Lives here (not in
    /// the per-turn context) because the context is reloaded from the DB
    /// each turn while the loop spans turns. Restart re-arms the guard —
    /// same accepted semantics as #507's flags.
    pub(super) session_outgoing_text_ring:
        std::sync::RwLock<HashMap<Uuid, super::announcement_loop::OutgoingTextRing>>,

    /// Service context for database operations
    pub(super) context: ServiceContext,

    /// Tool registry for executing tools
    pub(super) tool_registry: Arc<ToolRegistry>,

    /// Maximum tool execution iterations (0 = unlimited, relies on loop detection)
    pub(crate) max_tool_iterations: usize,

    /// System brain template — the brain assembled at startup. Used as the
    /// seed for `brain_rebuild` and by token-estimate floors; the live prompt
    /// sent each turn comes from `live_system_brain()`.
    pub(super) default_system_brain: Option<String>,

    /// When set, the system brain is rebuilt from disk on the next turn
    /// whenever a brain file changes, so edits take effect without a restart
    /// (#213). `None` → the static `default_system_brain` is used as-is.
    pub(super) brain_rebuild: Option<BrainRebuild>,

    /// Whether to auto-approve tool execution
    pub(super) auto_approve_tools: bool,

    /// Headless session (#129): no live user surface (CLI one-shot run, cron
    /// daemon execute, sub-agent spawn). Stamped into every
    /// ToolExecutionContext so interactive-only tools (session_notify,
    /// suggest_options) hard-error if invoked anyway — the belt-and-braces
    /// backstop behind the registry-level exclusion. Default `false`.
    pub(super) headless: bool,

    /// When true, suppress the playful post-compaction narration.
    /// Mirrors `[agent] silent_compaction` from config.toml. Default
    /// is `false` — users have called out the post-compaction
    /// one-liners as a delight feature; corporate / customer-facing
    /// deployments can opt out by setting the flag.
    pub(super) silent_compaction: bool,

    /// When true, ship only CORE tool schemas + `tool_search` per request and
    /// let the agent activate extended tools on demand. Mirrors `[agent]
    /// lazy_tools`. Default false — see `AgentConfig::lazy_tools`.
    pub(super) lazy_tools: bool,

    /// Cap for concurrent tool execution within one turn (`[agent]
    /// max_concurrent`, clamped to at least 1). Copied at construction like
    /// its neighbors; config hot-reload rebuilds channel services.
    pub(super) max_concurrent: usize,

    /// Context window limit in tokens from config
    pub(super) context_limit: u32,

    /// Max output tokens for API calls from config
    pub(super) max_tokens: u32,

    /// Callback for requesting tool approval from user
    pub(super) approval_callback: Option<ApprovalCallback>,

    /// Deprecated stub kept for layout stability.
    /// discrete-choice question and block until they pick an option.
    /// Set by channel handlers during agent service construction
    /// (Telegram inline keyboard, Discord components, etc.); None on
    /// channels with no interactive surface.
    /// Callback for reporting progress during tool execution
    pub(super) progress_callback: Option<ProgressCallback>,

    /// Callback for checking queued user messages between tool iterations
    pub(super) message_queue_callback: Option<MessageQueueCallback>,

    /// Symmetric producer for the message queue (#722): lets a background-task
    /// watcher push a completion into a session's queue so the resume rides the
    /// same drain path. Set per-surface (TUI / channels).
    pub(super) message_enqueue_callback: Option<super::types::MessageEnqueueCallback>,

    /// Background-task manager (#722), created when an enqueue callback is wired.
    /// bash hands long commands here so they run detached and resume the session
    /// on completion. `None` on surfaces without the enqueue producer, in which
    /// case bash just runs inline as before.
    pub(super) background_manager:
        Option<std::sync::Arc<super::background_tasks::BackgroundTaskManager>>,

    /// Callback for requesting sudo password from user
    pub(super) sudo_callback: Option<SudoCallback>,

    /// Callback for requesting SSH password from user (for `ssh`, `scp`,
    /// `rsync` invocations whose key auth fails). Same shape as
    /// `sudo_callback` — wired by the TUI to a password dialog and by
    /// channels (future) to an approval card.
    pub(super) ssh_callback: Option<SshPasswordCallback>,

    /// Global working directory. Retained as the SEED for per-session handles
    /// and a fallback for callers without a session id; per-session isolation
    /// lives in `session_working_dirs` so a background session's `cd` cannot
    /// contaminate the foreground session's prompt or tools (#703).
    pub(super) working_directory: Arc<std::sync::RwLock<std::path::PathBuf>>,

    /// Per-session working directory isolation (#703). Every session that runs
    /// a turn gets its own `Arc<RwLock<PathBuf>>`, seeded from `working_directory`
    /// on first use. Tool execution, the Runtime Info prompt line, recent-paths,
    /// and the request cwd all resolve THIS session's handle — so two sessions in
    /// different directories can run concurrently without leaking each other's
    /// cwd. Mirrors the `session_providers` per-session pattern.
    pub(super) session_working_dirs:
        std::sync::RwLock<HashMap<Uuid, Arc<std::sync::RwLock<std::path::PathBuf>>>>,

    /// Brain path (~/.opencrabs/) for loading brain files
    pub(super) brain_path: Option<std::path::PathBuf>,

    /// Notification channel — fired after every `run_tool_loop` completion so
    /// the TUI can refresh when a remote channel (Telegram/WhatsApp/…) updates
    /// the shared session.
    pub(super) session_updated_tx:
        Option<tokio::sync::mpsc::UnboundedSender<super::types::ChannelSessionEvent>>,

    /// Fallback providers for rate-limit recovery, built from
    /// `[providers.fallback]`. When the primary provider hits a rate/account
    /// limit mid-stream, these are tried in order.
    ///
    /// Behind a lock since #1249: the chain used to be built once at
    /// construction and never touched again, so editing `fallback_chain` in
    /// config.toml had no effect until a full restart. The ConfigWatcher
    /// hot-swapped the PRIMARY provider and left this list frozen, which is how
    /// a provider deleted from the chain kept serving turns for hours. Read
    /// through [`AgentService::fallback_chain_snapshot`]; replace through
    /// [`AgentService::reload_fallback_providers`].
    pub(super) fallback_providers: std::sync::RwLock<Vec<Arc<dyn Provider>>>,

    /// Tracks spawned sub-agents. Shared with the subagent tools; stored here
    /// so compaction can inject running agent IDs into the post-compaction
    /// summary (#936). `None` when the agent was constructed without the
    /// manager (child agents, tests that don't need it).
    pub(super) subagent_manager: Option<Arc<crate::brain::tools::subagent::SubAgentManager>>,

    /// Plan-state session override (#908 option A). When set, this agent's
    /// tool contexts resolve plan state (plan JSON, design `.md`, pre-init
    /// and autonomy markers, plan-task goal) against the given session
    /// instead of the session the turn runs in. Spawned plan workers carry
    /// the parent's session id here so they operate on the parent's
    /// checklist while their own session stays fresh. `None` — the normal
    /// case — resolves against the session's own id.
    pub(super) plan_session_override: Option<Uuid>,
}

impl AgentService {
    /// Create a new agent service. Reads agent settings from the provided config.
    pub async fn new(
        provider: Arc<dyn Provider>,
        context: ServiceContext,
        config: &crate::config::Config,
    ) -> Self {
        Self {
            provider: std::sync::RwLock::new(provider),
            session_providers: std::sync::RwLock::new(HashMap::new()),
            session_models: std::sync::RwLock::new(HashMap::new()),
            manual_switch: std::sync::RwLock::new(HashMap::new()),
            plan_mode_swap: std::sync::RwLock::new(HashMap::new()),
            session_context_limits: std::sync::RwLock::new(HashMap::new()),
            session_primary_failure_streak: std::sync::RwLock::new(HashMap::new()),
            active_skills: std::sync::RwLock::new(HashMap::new()),
            session_pressure_warned: std::sync::RwLock::new(HashMap::new()),
            last_compaction_elapsed: std::sync::RwLock::new(HashMap::new()),
            session_outgoing_text_ring: std::sync::RwLock::new(HashMap::new()),
            context,
            tool_registry: Arc::new(ToolRegistry::new()),
            max_tool_iterations: 0, // 0 = unlimited (loop detection is the safety net)
            default_system_brain: None,
            brain_rebuild: None,
            // Resolved from config here, ONCE, so every surface inherits the
            // same answer. It used to default to false and be derived
            // independently at four call sites; the two CLI ones derived it
            // from a flag instead, so a config of `auto-always` was ignored on
            // any path that had no approval callback and every gated tool was
            // denied with "no approval mechanism configured" (#769).
            //
            // Resolving at construction rather than per tool call is
            // deliberate: the policy check that reads config hits the disk, and
            // the tool gate runs for every call.
            auto_approve_tools: crate::utils::approval::policy_auto_approves(
                &config.agent.approval_policy,
            ),
            headless: false,
            silent_compaction: config.agent.silent_compaction,
            background_compaction: config.agent.background_compaction,
            pending_compactions: std::sync::Mutex::new(HashMap::new()),
            lazy_tools: config.agent.lazy_tools,
            max_concurrent: (config.agent.max_concurrent as usize).max(1),
            context_limit: config.agent.context_limit,
            max_tokens: config.agent.max_tokens,
            approval_callback: None,
            progress_callback: None,
            message_queue_callback: None,
            message_enqueue_callback: None,
            background_manager: None,
            sudo_callback: None,
            ssh_callback: None,
            session_working_dirs: std::sync::RwLock::new(HashMap::new()),
            working_directory: Arc::new(std::sync::RwLock::new(
                std::env::current_dir().unwrap_or_default(),
            )),
            brain_path: None,
            session_updated_tx: None,
            fallback_providers: std::sync::RwLock::new(
                Self::build_fallback_providers(config).await,
            ),
            subagent_manager: None,
            plan_session_override: None,
        }
    }

    /// Create an agent service for tests.
    /// Only use in test code where no real user config exists.
    ///
    /// Pins `approval_policy` to a gating value rather than taking the default,
    /// which is `auto-always`. Now that the policy resolves into the tool gate
    /// (#769), inheriting that default would auto-approve every test service
    /// and silently stop the approval tests from reaching the machinery they
    /// exist to cover. Tests that want the grant ask for it explicitly with
    /// `with_auto_approve_tools(true)`, which is what they already did.
    pub async fn new_for_test(provider: Arc<dyn Provider>, context: ServiceContext) -> Self {
        let mut config = crate::config::Config::default();
        config.agent.approval_policy = "ask".to_string();
        Self::new(provider, context, &config).await
    }

    /// Test-only: replace the configured fallback chain after
    /// construction. `Config::default()` carries no fallbacks, so
    /// `new_for_test` produces an empty `fallback_providers` vec —
    /// fine for tests that don't care about cascade behaviour, but
    /// useless for tests that need to verify
    /// `swap_provider_for_session` wraps in a `FallbackProvider`.
    /// Marked `#[doc(hidden)]` because no production caller should
    /// mutate this field after construction.
    #[doc(hidden)]
    pub fn set_fallback_providers_for_test(&mut self, providers: Vec<Arc<dyn Provider>>) {
        *self
            .fallback_providers
            .write()
            .expect("fallback_providers lock poisoned") = providers;
    }

    /// Snapshot of the configured fallback chain.
    ///
    /// Returns a clone (a `Vec` of `Arc`s, so cheap) rather than a guard on
    /// purpose: every caller walks the chain across `.await` points, and a
    /// live guard would both pin a stale list and make the future non-`Send`.
    pub fn fallback_chain_snapshot(&self) -> Vec<Arc<dyn Provider>> {
        self.fallback_providers
            .read()
            .expect("fallback_providers lock poisoned")
            .clone()
    }

    /// Rebuild the fallback chain from a freshly loaded config (#1249).
    ///
    /// Called by the ConfigWatcher next to `swap_provider`, so a
    /// `fallback_chain` edit takes effect on the running process the same way
    /// a primary-provider edit already did. Without this the two halves of the
    /// same config block reloaded at different speeds: the primary
    /// immediately, the chain never — a provider removed from the chain kept
    /// being handed live traffic until the next restart.
    ///
    /// Providers that fail to construct are skipped by
    /// `build_fallback_providers`, exactly as at startup. An empty result is
    /// applied as-is: clearing `fallback_chain` MUST clear the runtime chain,
    /// otherwise removal is unrepresentable.
    pub async fn reload_fallback_providers(&self, config: &crate::config::Config) {
        let rebuilt = Self::build_fallback_providers(config).await;
        let names: Vec<String> = rebuilt.iter().map(|p| p.name().to_string()).collect();
        match self.fallback_providers.write() {
            Ok(mut slot) => {
                let previous: Vec<String> = slot.iter().map(|p| p.name().to_string()).collect();
                *slot = rebuilt;
                if previous == names {
                    tracing::debug!(
                        "ConfigWatcher: fallback chain unchanged ([{}])",
                        names.join(", ")
                    );
                } else {
                    tracing::info!(
                        "ConfigWatcher: fallback chain reloaded — [{}] (was [{}])",
                        names.join(", "),
                        previous.join(", ")
                    );
                }
            }
            Err(e) => tracing::warn!(
                "ConfigWatcher: fallback chain NOT reloaded, lock poisoned: {e} — \
                 the running chain stays [{}]",
                names.join(", ")
            ),
        }
    }

    /// Get the service context
    pub fn context(&self) -> &ServiceContext {
        &self.context
    }

    /// Effective context-window budget. Returns the active provider's
    /// `configured_context_window()` when set (only custom OpenAI-compatible
    /// providers expose one, via `providers.<name>.context_window` in
    /// `config.toml`); otherwise the static `agent.context_limit`.
    ///
    /// Prefer `context_limit_for_session(session_id)` for session-scoped
    /// operations (compaction, budget checks) to avoid cross-session
    /// contamination when the global provider changes.
    pub fn context_limit(&self) -> u32 {
        self.provider()
            .configured_context_window()
            .unwrap_or(self.context_limit)
    }

    /// Per-session context window budget. Mirrors `provider_for_session`:
    /// returns the cached override for this session if one exists (set by
    /// `swap_provider_for_session`), otherwise falls back to the global
    /// `context_limit()`. This ensures compaction and budget checks use
    /// the correct window even when the user switches models on another pane.
    pub fn context_limit_for_session(&self, session_id: Uuid) -> u32 {
        if let Ok(map) = self.session_context_limits.read()
            && let Some(&cw) = map.get(&session_id)
        {
            return cw;
        }
        // Fall back to the session's ACTIVE provider's configured window before
        // the static agent.context_limit default (#609). session_context_limits
        // is only populated on a manual /models swap, so a session that just
        // uses the configured provider would otherwise compact against the 200K
        // default while the flow header shows the provider's real window (e.g.
        // Kimi K3 configured for 1M compacted at ~185K because the budget was
        // silently 200K). Honor the provider window for every session.
        if let Some(cw) = self
            .provider_for_session(session_id)
            .configured_context_window()
        {
            return cw;
        }
        self.context_limit()
    }

    /// Get max tokens from config
    pub fn max_tokens(&self) -> u32 {
        self.max_tokens
    }

    /// Get the tool registry
    pub fn tool_registry(&self) -> &Arc<ToolRegistry> {
        &self.tool_registry
    }

    /// Get the progress callback (for preserving across rebuilds)
    pub fn progress_callback(&self) -> &Option<ProgressCallback> {
        &self.progress_callback
    }

    /// Get the message queue callback (for preserving across rebuilds)
    pub fn message_queue_callback(&self) -> &Option<MessageQueueCallback> {
        &self.message_queue_callback
    }

    /// Get the sudo callback (for preserving across rebuilds)
    pub fn sudo_callback(&self) -> &Option<SudoCallback> {
        &self.sudo_callback
    }

    /// Get the SSH password callback (for preserving across rebuilds)
    pub fn ssh_callback(&self) -> &Option<SshPasswordCallback> {
        &self.ssh_callback
    }

    /// Get the working directory (for preserving across rebuilds)
    pub fn working_directory(&self) -> &Arc<std::sync::RwLock<std::path::PathBuf>> {
        &self.working_directory
    }

    /// Get the brain path (for preserving across rebuilds)
    pub fn brain_path(&self) -> &Option<std::path::PathBuf> {
        &self.brain_path
    }

    /// Set the default system brain
    pub fn with_system_brain(mut self, prompt: String) -> Self {
        self.default_system_brain = Some(prompt);
        self
    }

    /// Enable live brain rebuilding from disk (#213). Call AFTER
    /// `with_system_brain` — the already-assembled brain is used as the seed
    /// so the first turns return it verbatim (warm prompt cache) until a brain
    /// file actually changes on disk. `core` picks `build_core_brain` (TUI /
    /// channels) vs `build_system_brain`; `lazy_tools` re-appends the lazy-tools
    /// prompt to match the startup assembly.
    pub fn with_brain_rebuild(
        mut self,
        loader: crate::brain::prompt_builder::BrainLoader,
        runtime_info: Option<crate::brain::prompt_builder::RuntimeInfo>,
        core: bool,
        lazy_tools: bool,
    ) -> Self {
        let seed = self.default_system_brain.clone().unwrap_or_default();
        // Share the live working-directory handle so the directive scan follows
        // `/cd`. Same Arc that tool execution and `set_working_directory` use,
        // so runtime mutations are visible to `render`.
        let live_cwd = Some(Arc::clone(&self.working_directory));
        self.brain_rebuild = Some(BrainRebuild::new(
            loader,
            runtime_info,
            core,
            lazy_tools,
            seed,
            live_cwd,
        ));
        self
    }

    /// Carry an existing live-brain handle forward across a service rebuild
    /// (e.g. a `/models` provider swap) so live rebuilding and its warm cache
    /// survive without re-reading disk.
    pub fn with_brain_rebuild_handle(mut self, handle: BrainRebuild) -> Self {
        self.brain_rebuild = Some(handle);
        self
    }

    /// The live-brain handle, if live rebuilding is enabled. Cloned cheaply
    /// (it's an `Arc`) so callers can carry it across a rebuild.
    pub fn brain_rebuild(&self) -> Option<BrainRebuild> {
        self.brain_rebuild.clone()
    }

    /// The system brain to send THIS turn. Rebuilds from disk when a brain
    /// file changed (#213); otherwise returns the cached/static brain. This is
    /// the single source of truth for the prompt — `default_system_brain` is
    /// only a seed and a token-estimate floor.
    pub(super) fn live_system_brain(&self) -> Option<String> {
        match &self.brain_rebuild {
            Some(rebuild) => Some(rebuild.render()),
            None => self.default_system_brain.clone(),
        }
    }

    /// [`live_system_brain`](Self::live_system_brain) with the Runtime Info
    /// block's `Model:`/`Provider:` lines resolved for THIS session. The brain
    /// renders from a `RuntimeInfo` frozen at startup (the default provider),
    /// so a session that swapped providers (per-session `/models` pick,
    /// channel provider sync, sticky fallback) would tell the model it runs
    /// on the startup default and it mis-reports itself when asked. Display
    /// surfaces already resolve via `provider_model_for_session()`; the prompt
    /// must resolve the same way at injection time.
    pub(super) fn live_system_brain_for_session(&self, session_id: Uuid) -> Option<String> {
        let brain = self.live_system_brain()?;
        let model = self.provider_model_for_session(session_id);
        let provider = self.provider_name_for_session(session_id);
        let brain = crate::brain::prompt_builder::override_runtime_model_provider(
            &brain, &model, &provider,
        );
        // Working directory is per-session (#703): the brain rendered from the
        // global cwd, so patch the line to THIS session's own directory or a
        // background session's `cd` leaks into this prompt.
        let wd = crate::brain::tools::error::collapse_home(
            &self.get_working_directory_for_session(session_id),
        );
        Some(crate::brain::prompt_builder::override_runtime_working_directory(&brain, &wd))
    }

    /// Set maximum tool iterations
    pub fn with_max_tool_iterations(mut self, max: usize) -> Self {
        self.max_tool_iterations = max;
        self
    }

    /// Set the tool registry
    pub fn with_tool_registry(mut self, registry: Arc<ToolRegistry>) -> Self {
        self.tool_registry = registry;
        self
    }

    /// Set whether to auto-approve tool execution
    /// Whether tools run without an interactive approval, after folding the
    /// config policy together with any explicit caller grant.
    pub fn auto_approves_tools(&self) -> bool {
        self.auto_approve_tools
    }

    /// Grant auto-approval on top of whatever the config policy already allows.
    ///
    /// Widens, never narrows. The baseline comes from `approval_policy` in
    /// [`Self::new`], and a caller passing `false` means "I have nothing to add"
    /// rather than "revoke the policy" — `--auto-approve` absent on the CLI must
    /// not cancel a configured `auto-always` (#769). Callers that need approval
    /// back on change the policy, which is the setting that owns this.
    ///
    /// `true` still forces it on for callers with their own grant, such as
    /// subagents whose spawn the parent already approved.
    pub fn with_auto_approve_tools(mut self, auto_approve: bool) -> Self {
        self.auto_approve_tools |= auto_approve;
        self
    }

    /// Mark this service headless (#129): no live user surface. Stamped into
    /// every ToolExecutionContext so interactive-only tools hard-error on
    /// invocation — the backstop behind the registry-level exclusion at
    /// `register_core_agent_tools(headless = true)`.
    pub fn with_headless(mut self, headless: bool) -> Self {
        self.headless = headless;
        self
    }

    /// Set the approval callback for interactive tool approval
    pub fn with_approval_callback(mut self, callback: Option<ApprovalCallback>) -> Self {
        self.approval_callback = callback;
        self
    }

    /// Set the progress callback for reporting tool execution progress
    pub fn with_progress_callback(mut self, callback: Option<ProgressCallback>) -> Self {
        self.progress_callback = callback;
        self
    }

    /// Set the message queue callback for injecting user messages between tool iterations
    pub fn with_message_queue_callback(mut self, callback: Option<MessageQueueCallback>) -> Self {
        self.message_queue_callback = callback;
        self
    }

    /// Set the enqueue callback (#722): the surface's producer that pushes a
    /// `QueuedUserMessage` into a session's queue, used to resume a session when
    /// a background task finishes.
    pub fn with_message_enqueue_callback(
        mut self,
        callback: Option<super::types::MessageEnqueueCallback>,
    ) -> Self {
        // Spin up the background-task manager (fork #19: it no longer carries
        // its own enqueue route — delivery goes through the one gated route,
        // `deliver_to_session`, which resolves the session's registered route;
        // this callback is still kept on self for #940 claiming).
        self.background_manager = callback
            .clone()
            .map(|_cb| std::sync::Arc::new(super::background_tasks::BackgroundTaskManager::new()));
        self.message_enqueue_callback = callback;
        self
    }

    /// The background-task manager, if an enqueue producer was wired (#722).
    pub fn background_manager(
        &self,
    ) -> Option<std::sync::Arc<super::background_tasks::BackgroundTaskManager>> {
        self.background_manager.clone()
    }

    /// This surface's enqueue producer, so a surface that owns a session can
    /// claim its background-task completions (#940). Without this the
    /// completion follows whichever service executed the command, which for a
    /// channel session driven from the TUI is the wrong one.
    pub fn message_enqueue_callback(&self) -> Option<super::types::MessageEnqueueCallback> {
        self.message_enqueue_callback.clone()
    }

    /// Push a message into `session_id`'s queue via the surface enqueue callback,
    /// if one is wired (#722). Returns `true` when enqueued. Used by the
    /// background-task watcher to resume a session on completion; the tool loop
    /// drains it at the next iteration boundary (or it starts a fresh turn).
    pub fn enqueue_session_message(
        &self,
        session_id: Uuid,
        message: super::types::QueuedUserMessage,
    ) -> bool {
        match self.message_enqueue_callback.as_ref() {
            Some(cb) => {
                cb(session_id, message);
                true
            }
            None => {
                tracing::warn!(
                    "enqueue_session_message: no enqueue callback wired for this surface; \
                     dropping resume for session {session_id}"
                );
                false
            }
        }
    }

    /// Set the sudo password callback for interactive sudo prompts
    pub fn with_sudo_callback(mut self, callback: Option<SudoCallback>) -> Self {
        self.sudo_callback = callback;
        self
    }

    /// Set the SSH password callback for interactive ssh/scp/rsync prompts
    pub fn with_ssh_callback(mut self, callback: Option<SshPasswordCallback>) -> Self {
        self.ssh_callback = callback;
        self
    }

    /// Wire the sub-agent manager so compaction can inject running agent IDs
    /// into the post-compaction summary (#936).
    pub fn with_subagent_manager(
        mut self,
        manager: Arc<crate::brain::tools::subagent::SubAgentManager>,
    ) -> Self {
        self.subagent_manager = Some(manager);
        self
    }

    /// Clone the sub-agent manager handle (for carrying across service rebuilds).
    pub fn subagent_manager(&self) -> Option<Arc<crate::brain::tools::subagent::SubAgentManager>> {
        self.subagent_manager.clone()
    }

    /// Point this agent's plan state at another session (#908 option A).
    /// Plan-driven execution passes the parent's session id when spawning a
    /// task worker, so the worker's `plan` tool resolves the parent's
    /// checklist while the worker session itself stays fresh. `None` (the
    /// default) keeps plan state session-local.
    pub fn with_plan_session_override(mut self, plan_session: Option<Uuid>) -> Self {
        self.plan_session_override = plan_session;
        self
    }

    /// Set the working directory for tool execution
    pub fn with_working_directory(self, working_directory: std::path::PathBuf) -> Self {
        *self
            .working_directory
            .write()
            .expect("working_directory lock poisoned") = working_directory;
        self
    }

    /// Get the current working directory
    pub fn get_working_directory(&self) -> std::path::PathBuf {
        self.working_directory
            .read()
            .expect("working_directory lock poisoned")
            .clone()
    }

    /// Change the working directory at runtime (called from /cd or agent tools)
    pub fn set_working_directory(&self, path: std::path::PathBuf) {
        *self
            .working_directory
            .write()
            .expect("working_directory lock poisoned") = path;
    }

    /// Get a shared handle to the working directory (for tools that need to mutate it)
    pub fn shared_working_directory(&self) -> Arc<std::sync::RwLock<std::path::PathBuf>> {
        Arc::clone(&self.working_directory)
    }

    /// The per-session working-directory handle (#703), creating it lazily on
    /// first use seeded from the global `working_directory`. Every session gets
    /// its OWN `Arc`, so a `cd` in one session's tool loop mutates only that
    /// session's cwd — never the global, never another session's. Tool
    /// execution, the Runtime Info prompt line, and recent-paths all resolve
    /// through this so concurrent sessions in different directories stay
    /// isolated.
    pub fn working_dir_handle_for_session(
        &self,
        session_id: Uuid,
    ) -> Arc<std::sync::RwLock<std::path::PathBuf>> {
        if let Some(handle) = self
            .session_working_dirs
            .read()
            .expect("session_working_dirs lock poisoned")
            .get(&session_id)
        {
            return Arc::clone(handle);
        }
        let seed = self.get_working_directory();
        let mut map = self
            .session_working_dirs
            .write()
            .expect("session_working_dirs lock poisoned");
        // Re-check under the write lock: another turn may have inserted it
        // between our read miss and acquiring the write lock.
        Arc::clone(
            map.entry(session_id)
                .or_insert_with(|| Arc::new(std::sync::RwLock::new(seed))),
        )
    }

    /// Current working directory for a specific session (#703). Falls back to
    /// the global value for a session that has not run a turn yet.
    pub fn get_working_directory_for_session(&self, session_id: Uuid) -> std::path::PathBuf {
        self.working_dir_handle_for_session(session_id)
            .read()
            .expect("session working_directory lock poisoned")
            .clone()
    }

    /// Set a session's working directory (#703). Called from a session switch,
    /// `/cd`, or resume. Also updates the global so brand-new sessions seed from
    /// the most recent foreground directory, but background sessions are never
    /// affected because they hold their own handle.
    pub fn set_working_directory_for_session(&self, session_id: Uuid, path: std::path::PathBuf) {
        *self
            .working_dir_handle_for_session(session_id)
            .write()
            .expect("session working_directory lock poisoned") = path.clone();
        self.set_working_directory(path);
    }

    /// True when this session has no cwd handle yet, i.e. the next
    /// `working_dir_handle_for_session` call would seed it from the global
    /// directory. Restore reads this to know whether the persisted
    /// `sessions.working_directory` still has a say, so a `cd` made later in
    /// the same process is never overwritten by a stale DB row.
    pub fn session_working_dir_unset(&self, session_id: Uuid) -> bool {
        !self
            .session_working_dirs
            .read()
            .expect("session_working_dirs lock poisoned")
            .contains_key(&session_id)
    }

    /// Set ONLY this session's working directory, leaving the global untouched.
    ///
    /// `set_working_directory_for_session` also moves the global so a new
    /// foreground session seeds from the last directory the operator picked.
    /// Restoring a background chat's persisted directory must not do that: a
    /// Telegram group waking up in its own repo would otherwise drag the TUI
    /// and every other chat along with it.
    pub fn set_session_only_working_directory(&self, session_id: Uuid, path: std::path::PathBuf) {
        *self
            .working_dir_handle_for_session(session_id)
            .write()
            .expect("session working_directory lock poisoned") = path;
    }

    /// Set the brain path (~/.opencrabs/)
    pub fn with_brain_path(mut self, brain_path: std::path::PathBuf) -> Self {
        self.brain_path = Some(brain_path);
        self
    }

    /// Set the session-updated notification sender.
    ///
    /// When set, `run_tool_loop` fires this after every completed agent response
    /// so the TUI can reload the session in real-time when a remote channel
    /// (Telegram, WhatsApp, Discord, Slack) processes a message.
    pub fn with_session_updated_tx(
        mut self,
        tx: tokio::sync::mpsc::UnboundedSender<super::types::ChannelSessionEvent>,
    ) -> Self {
        self.session_updated_tx = Some(tx);
        self
    }

    /// Get the session-updated sender (for preserving across agent rebuilds).
    pub fn session_updated_tx(
        &self,
    ) -> Option<tokio::sync::mpsc::UnboundedSender<super::types::ChannelSessionEvent>> {
        self.session_updated_tx.clone()
    }

    /// Get the provider name. When a sticky FallbackProvider has swapped to
    /// a fallback, this returns the *active* sub-provider's name so the
    /// footer/splash reflects what's actually serving requests.
    pub fn provider_name(&self) -> String {
        let p = self.provider.read().expect("provider lock poisoned");
        p.active_subprovider_name()
            .unwrap_or_else(|| p.name().to_string())
    }

    /// Get the system brain
    pub fn system_brain(&self) -> Option<&String> {
        self.default_system_brain.as_ref()
    }

    /// Raw cl100k_base estimate of system_brain + tool schemas.
    /// Kept for the few internal call sites that still need a local
    /// floor estimate (e.g. when a provider reports zero input_tokens).
    /// NOT used for the ctx footer — see `base_context_tokens()`.
    pub fn base_context_tokens_raw(&self) -> u32 {
        use crate::brain::tokenizer::count_tokens;
        let system_tokens = self
            .default_system_brain
            .as_deref()
            .map(count_tokens)
            .unwrap_or(0);
        let tool_tokens = self.actual_tool_schema_tokens();
        (system_tokens + tool_tokens) as u32
    }

    /// Baseline for the ctx-footer display BEFORE any API response has
    /// landed for this session. Returns 0 — opencrabs uses ONLY
    /// real-time data from the provider's `usage.input_tokens`. There
    /// is no local tokenizer estimate, no per-provider calibration
    /// ratio, no prediction. On `/new` the footer shows `0/max` until
    /// the first turn completes, then every subsequent footer shows the
    /// provider's actual reported value verbatim.
    ///
    /// History note: 2026-05-24 a calibration system tried to predict
    /// this floor from a learned `real/local` ratio per provider; it
    /// shipped wrong (issue #119) and was ripped out the same week.
    /// Real data only, no guessing.
    pub fn base_context_tokens(&self) -> u32 {
        0
    }

    /// Get the default model for this provider. Mirrors `provider_name()`
    /// — returns the sticky-fallback active model when swapped.
    pub fn provider_model(&self) -> String {
        let p = self.provider.read().expect("provider lock poisoned");
        p.active_subprovider_model()
            .unwrap_or_else(|| p.default_model().to_string())
    }

    /// Get the list of supported models for this provider (hardcoded fallback)
    pub fn supported_models(&self) -> Vec<String> {
        self.provider
            .read()
            .expect("provider lock poisoned")
            .supported_models()
    }

    /// Fetch available models from the provider API (live)
    pub async fn fetch_models(&self) -> Vec<String> {
        let provider = self
            .provider
            .read()
            .expect("provider lock poisoned")
            .clone();
        provider.fetch_models().await
    }

    /// Get a clone of the underlying LLM provider
    pub fn provider(&self) -> Arc<dyn Provider> {
        self.provider
            .read()
            .expect("provider lock poisoned")
            .clone()
    }

    /// Swap the DEFAULT provider at runtime. Used during bootstrap and by
    /// callers without a session_id. Prefer `swap_provider_for_session` for
    /// anything session-scoped — this does NOT affect sessions that already
    /// have their own entry in `session_providers`.
    pub fn swap_provider(&self, new_provider: Arc<dyn Provider>) {
        *self.provider.write().expect("provider lock poisoned") = new_provider;
    }

    /// Look up the provider a specific session should use. Returns the
    /// session's dedicated entry if one exists; otherwise falls back to
    /// the global default. Read-path hot function — cheap Arc clone,
    /// no allocation beyond lock acquisition.
    pub fn provider_for_session(&self, session_id: Uuid) -> Arc<dyn Provider> {
        if let Ok(map) = self.session_providers.read()
            && let Some(p) = map.get(&session_id)
        {
            return p.clone();
        }
        self.provider
            .read()
            .expect("provider lock poisoned")
            .clone()
    }

    /// Restore a session's saved provider into `session_providers` before a
    /// turn runs, so the turn never falls back to the GLOBAL default (#704).
    ///
    /// After a restart the in-memory `session_providers` map is empty, so
    /// `provider_for_session` returns the global default for every session that
    /// hasn't been explicitly restored yet (resume path, channel turns, and any
    /// session not yet switched-to in the TUI). A turn on the wrong provider
    /// then trips `guard_cross_provider_model_leak`, which silently remaps the
    /// saved model to the wrong provider's default — a switch the user never
    /// made. Creating and registering the saved provider up front closes that
    /// gap for ALL entry points. No-op when the session already has an entry,
    /// has no saved provider, or its saved provider is already the global
    /// default (nothing to restore).
    /// Route this turn onto the provider/model its plan state calls for, and
    /// put the session back once the plan ends (#792).
    ///
    /// Called at turn start, after `ensure_session_provider_restored` so the
    /// session is on its OWN pair before any override is measured against it,
    /// and before the turn reads `provider_for_session`.
    ///
    /// Keying off plan state rather than the `/plan` and `/execute` commands is
    /// what makes this work everywhere at once: the TUI approval, the channel
    /// command, and the agent approving its own plan in prose all converge on
    /// `try_approve`, so all three route identically with no per-surface code.
    ///
    /// A no-op when the config sets no plan-mode keys, which is the default: no
    /// provider is built and no swap happens, so an install that has never
    /// heard of this feature behaves exactly as before.
    pub(crate) async fn apply_plan_mode_provider(&self, session_id: Uuid) {
        let state = crate::utils::plan_files::plan_mode_state(session_id).await;
        let config = match crate::config::Config::load() {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(
                    "Could not load config to resolve plan-mode provider for session {session_id}: {e}"
                );
                return;
            }
        };
        let desired = super::plan_mode_provider::normalized_override_for(state, &config);

        // Cheap: both are in-memory map reads.
        let current_provider = self.provider_for_session(session_id);
        let current_name = current_provider.name().to_string();
        let current_model = self.provider_model_for_session(session_id);

        let Some(over) = desired else {
            self.restore_from_plan_mode(session_id, &config, &current_name, &current_model)
                .await;
            return;
        };

        let target_name = over
            .provider
            .clone()
            .unwrap_or_else(|| current_name.clone());

        if target_name == current_name {
            // Same provider. Only a model change is left to make, and only if
            // one was configured — a provider-only override whose provider is
            // already active has nothing to do.
            let Some(target_model) = over.model.clone() else {
                return;
            };
            if target_model == current_model {
                return;
            }
            self.record_plan_mode_swap(
                session_id,
                &current_name,
                &current_model,
                &target_name,
                &target_model,
            );
            self.set_session_model(session_id, target_model.clone());
            tracing::info!(
                "Plan-mode routing ({state:?}) for session {session_id}: model {current_model} -> {target_model} on {current_name}"
            );
            return;
        }

        // A different provider: build it. Reached only when the session is not
        // already on the target, so this does not rebuild every turn.
        let provider =
            match crate::brain::provider::create_provider_by_name(&config, &target_name).await {
                Ok(p) => p,
                Err(e) => {
                    // A dead configured provider must not kill the turn (#469).
                    tracing::warn!(
                        "Plan-mode provider '{target_name}' failed to create ({e:#}) for session \
                         {session_id} — staying on '{current_name}'"
                    );
                    return;
                }
            };
        // Model and provider are set as one unit. Swapping the provider alone
        // would leave the previous model pinned against the new catalogue, and
        // `guard_cross_provider_model_leak` would substitute a default nobody
        // asked for.
        let target_model = over
            .model
            .clone()
            .unwrap_or_else(|| provider.default_model().to_string());
        self.record_plan_mode_swap(
            session_id,
            &current_name,
            &current_model,
            &target_name,
            &target_model,
        );
        self.swap_provider_for_session(session_id, provider, target_model.clone());
        tracing::info!(
            "Plan-mode routing ({state:?}) for session {session_id}: {current_name}/{current_model} \
             -> {target_name}/{target_model}"
        );
    }

    /// Remember what an override replaced, the first time it replaces it.
    ///
    /// Only the FIRST swap records. Moving from drafting to executing changes
    /// which override applies, and the pair worth keeping is the one the
    /// session had before any of it started: recording again would make the
    /// plan model the restore target, so finishing a plan would leave the
    /// session on the planning model instead of the user's own.
    fn record_plan_mode_swap(
        &self,
        session_id: Uuid,
        original_provider: &str,
        original_model: &str,
        applied_provider: &str,
        applied_model: &str,
    ) {
        if let Ok(mut map) = self.plan_mode_swap.write() {
            map.entry(session_id)
                .and_modify(|s| {
                    // Keep the original; update what is currently installed so
                    // the "did the user change this themselves" check stays
                    // accurate across a drafting -> executing transition.
                    s.applied_provider = applied_provider.to_string();
                    s.applied_model = applied_model.to_string();
                })
                .or_insert_with(|| super::plan_mode_provider::PlanModeSwap {
                    original_provider: original_provider.to_string(),
                    original_model: original_model.to_string(),
                    applied_provider: applied_provider.to_string(),
                    applied_model: applied_model.to_string(),
                });
        }
    }

    /// Put the session back on the pair it had before plan-mode routing.
    ///
    /// Runs on the first turn after the plan archives, which is the turn where
    /// the state resolves to no override at all.
    async fn restore_from_plan_mode(
        &self,
        session_id: Uuid,
        config: &crate::config::Config,
        current_name: &str,
        current_model: &str,
    ) {
        let Some(swap) = self
            .plan_mode_swap
            .write()
            .ok()
            .and_then(|mut m| m.remove(&session_id))
        else {
            return;
        };
        // If the pair is no longer the one the override installed, the user
        // switched deliberately while the plan was live. Their pick outranks a
        // stale restore target, so drop the record and leave them alone.
        if !swap.still_applied(current_name, current_model) {
            tracing::info!(
                "Plan-mode routing ended for session {session_id}: provider is now \
                 {current_name}/{current_model}, not the {}/{} that was installed — \
                 leaving the current pick in place",
                swap.applied_provider,
                swap.applied_model
            );
            return;
        }
        match crate::brain::provider::create_provider_by_name(config, &swap.original_provider).await
        {
            Ok(provider) => {
                self.swap_provider_for_session(session_id, provider, swap.original_model.clone());
                tracing::info!(
                    "Plan-mode routing ended for session {session_id}: restored {}/{}",
                    swap.original_provider,
                    swap.original_model
                );
            }
            Err(e) => {
                tracing::warn!(
                    "Plan-mode routing ended for session {session_id} but the original provider \
                     '{}' could not be rebuilt ({e:#}) — session stays on {current_name}",
                    swap.original_provider
                );
            }
        }
    }

    pub async fn ensure_session_provider_restored(
        &self,
        session_id: Uuid,
        saved_provider: Option<&str>,
        saved_model: Option<&str>,
    ) {
        let Some(saved) = saved_provider else {
            return;
        };
        if self
            .session_providers
            .read()
            .map(|m| m.contains_key(&session_id))
            .unwrap_or(false)
        {
            return;
        }
        if saved == self.provider_name() {
            return;
        }
        let config = match crate::config::Config::load() {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(
                    "Could not load config to restore provider '{saved}' for session {session_id}: {e}"
                );
                return;
            }
        };
        match crate::brain::provider::create_provider_by_name(&config, saved).await {
            Ok(provider) => {
                let model = saved_model
                    .map(str::to_string)
                    .unwrap_or_else(|| provider.default_model().to_string());
                self.swap_provider_for_session(session_id, provider, model);
                tracing::info!(
                    "Restored saved provider '{saved}' for session {session_id} before turn (#704)"
                );
            }
            Err(e) => {
                tracing::warn!(
                    "Could not restore saved provider '{saved}' for session {session_id}: {e} — \
                     turn will use the current provider"
                );
            }
        }
    }

    /// Assign a provider specifically to `session_id`. Subsequent agent
    /// turns for that session use this provider; other sessions and the
    /// global default are untouched. Called by `/models` dialog on model
    /// pick and by `load_session` when restoring a session's saved
    /// `provider_name`.
    ///
    /// Wraps the new provider in a `FallbackProvider` (using the
    /// AgentService's configured `fallback_providers`, filtered to
    /// exclude the new primary itself) when it isn't already a
    /// fallback chain. Without this wrapping, per-session swaps
    /// stripped FallbackProvider coverage entirely — a session that
    /// picked a custom provider via `/models` lost the transparent
    /// cascade that the global default sessions kept, and was left
    /// to rely on the in-tool_loop manual fallback paths as its only
    /// safety net. Logs 2026-06-02 02:33:25-29 captured the resulting
    /// regression: with the dialagram primary returning HTTP 530 and
    /// no FallbackProvider in front of it, every "Trying fallback
    /// X/Y..." iteration in the tool loop re-hit dialagram because
    /// the manual loop never swapped the session's provider before
    /// calling stream_complete. Wrapping at swap time restores the
    /// architectural invariant that every active provider in this
    /// service is a fallback chain.
    ///
    /// Also caches the provider's `configured_context_window()` into
    /// `session_context_limits` so compaction uses the correct budget
    /// even if the global provider changes later.
    ///
    /// **Provider+model are a pair.** The `model` is REQUIRED and set
    /// atomically with the provider — you cannot swap a provider without
    /// saying which model it pairs with. The caller always knows the pair:
    /// the user's pick (/models dialog, channel /models), the session's
    /// saved model (restore), or the fallback's remapped model
    /// (ProviderSwitched / sticky fallback). Pass `new_provider.default_model()`
    /// explicitly ONLY when there is genuinely no chosen model (e.g. a
    /// legacy session with an empty model column) — never let this function
    /// invent one. An earlier version silently reset the model to the new
    /// provider's default here, which clobbered the user's explicit pick on
    /// every swap — the footer showed "modelscope / GLM 5.1" right after the
    /// user switched to Qwen3.7-Max (2026-06-07).
    pub fn swap_provider_for_session(
        &self,
        session_id: Uuid,
        new_provider: Arc<dyn Provider>,
        model: impl Into<String>,
    ) {
        let model = model.into();
        let context_window = new_provider.configured_context_window();
        let stored: Arc<dyn Provider> = if new_provider.is_fallback_chain() {
            new_provider
        } else {
            // Exclude any fallback with the same name as the new
            // primary so a chain can't fall back to itself. Common
            // case: user picks "dialagram" as the active provider via
            // /models, and the configured fallback list also contains
            // "dialagram" — the duplicate would just retry the same
            // dead endpoint immediately on cascade.
            let new_name = new_provider.name().to_string();
            let chain: Vec<Arc<dyn Provider>> = self
                .fallback_chain_snapshot()
                .into_iter()
                .filter(|p| p.name() != new_name)
                .collect();
            if chain.is_empty() {
                // No fallbacks configured (or all of them collide with
                // the new primary). Store the raw provider — wrapping
                // it in an empty FallbackProvider would add a pointer
                // hop with no behavioural difference.
                new_provider
            } else {
                Arc::new(crate::brain::provider::FallbackProvider::new(
                    new_provider,
                    chain,
                ))
            }
        };

        self.session_providers
            .write()
            .expect("session_providers lock poisoned")
            .insert(session_id, stored);

        // Cache context window for this session
        if let Some(cw) = context_window {
            self.session_context_limits
                .write()
                .expect("session_context_limits lock poisoned")
                .insert(session_id, cw);
        }
        // Set the paired model atomically with the provider. The caller
        // supplied it (the user's pick / saved / remapped model) — this
        // function never invents a default.
        if let Ok(mut map) = self.session_models.write() {
            map.insert(session_id, model);
        }
    }

    /// Drop a session's provider entry (e.g. session deleted). Noop if
    /// no entry exists. Does NOT affect the global default or other
    /// sessions.
    pub fn remove_session_provider(&self, session_id: Uuid) {
        self.session_providers
            .write()
            .expect("session_providers lock poisoned")
            .remove(&session_id);
        self.session_context_limits
            .write()
            .expect("session_context_limits lock poisoned")
            .remove(&session_id);
        self.session_primary_failure_streak
            .write()
            .expect("session_primary_failure_streak lock poisoned")
            .remove(&session_id);
        self.active_skills
            .write()
            .expect("active_skills lock poisoned")
            .remove(&session_id);
    }

    /// Record one primary-provider failure that was rescued by a
    /// successful fallback. Returns the new streak count.
    ///
    /// Bumped only when the fallback ACTUALLY succeeded — failures
    /// where both primary and fallback errored out don't count, since
    /// no rescue happened and the situation is exceptional rather
    /// than evidence of a chronically broken primary.
    pub fn bump_primary_failure_streak(&self, session_id: Uuid) -> u32 {
        let mut map = self
            .session_primary_failure_streak
            .write()
            .expect("session_primary_failure_streak lock poisoned");
        let entry = map.entry(session_id).or_insert(0);
        *entry += 1;
        *entry
    }

    /// Reset the per-session primary-failure streak. Called after any
    /// successful PRIMARY stream so a single recovery wipes the count
    /// — the threshold meaning becomes "N consecutive rescues with
    /// no primary success in between", which matches the user intent
    /// ("if the fallback runs 3 times in a row successfully, the 4th
    /// it sticks").
    pub fn reset_primary_failure_streak(&self, session_id: Uuid) {
        self.session_primary_failure_streak
            .write()
            .expect("session_primary_failure_streak lock poisoned")
            .remove(&session_id);
    }

    /// Read current streak without mutating. Used by the fallback
    /// commit site to decide between "rescue this request only" vs
    /// "stick the fallback permanently".
    pub fn peek_primary_failure_streak(&self, session_id: Uuid) -> u32 {
        self.session_primary_failure_streak
            .read()
            .expect("session_primary_failure_streak lock poisoned")
            .get(&session_id)
            .copied()
            .unwrap_or(0)
    }

    /// Snapshot of every per-session provider binding. Used by
    /// `rebuild_agent_service` to carry session→provider pins across
    /// the rebuild so live sessions on other panes don't lose their
    /// provider when the user reconfigures via `/models`.
    pub fn session_provider_snapshot(&self) -> Vec<(Uuid, Arc<dyn Provider>)> {
        let map = self
            .session_providers
            .read()
            .expect("session_providers lock poisoned");
        map.iter().map(|(k, v)| (*k, v.clone())).collect()
    }

    /// Snapshot of every explicit per-session model pin. Used by
    /// `rebuild_agent_service` to carry the user's locked model choices
    /// across the rebuild. Only contains models a caller pinned via
    /// `set_session_model` — `swap_provider_for_session` never writes here,
    /// so this carries real picks, not invented defaults.
    pub fn session_model_snapshot(&self) -> Vec<(Uuid, String)> {
        let map = self
            .session_models
            .read()
            .expect("session_models lock poisoned");
        map.iter().map(|(k, v)| (*k, v.clone())).collect()
    }

    /// Provider name for a specific session, including sticky-fallback
    /// active sub-provider.
    pub fn provider_name_for_session(&self, session_id: Uuid) -> String {
        let p = self.provider_for_session(session_id);
        p.active_subprovider_name()
            .unwrap_or_else(|| p.name().to_string())
    }

    /// Default model for a specific session, including sticky-fallback
    /// active sub-model. Resolution order:
    /// 1. The per-session override in `session_models` (set by
    ///    `switch_model` and `sync_provider_for_session`). This is the
    ///    user's actual current pick.
    /// 2. The provider's active sub-model (sticky fallback in flight).
    /// 3. The provider's compiled-in `default_model()` (from config).
    pub fn provider_model_for_session(&self, session_id: Uuid) -> String {
        if let Ok(map) = self.session_models.read()
            && let Some(m) = map.get(&session_id)
        {
            return m.clone();
        }
        let p = self.provider_for_session(session_id);
        p.active_subprovider_model()
            .unwrap_or_else(|| p.default_model().to_string())
    }

    /// Install a per-session model override. Pair with
    /// `swap_provider_for_session` when restoring or switching a
    /// session's pick so display surfaces stay aligned with what the
    /// LLM call will actually use.
    pub fn set_session_model(&self, session_id: Uuid, model: String) {
        if let Ok(mut map) = self.session_models.write() {
            map.insert(session_id, model);
        }
    }

    /// Clear the per-session model override (e.g. when a session ends
    /// or is deleted).
    pub fn clear_session_model(&self, session_id: Uuid) {
        if let Ok(mut map) = self.session_models.write() {
            map.remove(&session_id);
        }
    }

    /// Record that the USER manually switched this session's provider/model.
    /// Call AFTER `swap_provider_for_session` in the /models dialog and
    /// channel /models paths. Captures the just-installed provider+model
    /// pair and bumps a per-session epoch. If a turn that started before
    /// this call later finishes having taken an automatic fallback, it
    /// restores this pair so the user's pick wins (see
    /// `restore_manual_switch_if_changed`).
    pub fn mark_manual_switch(&self, session_id: Uuid, model: String) {
        let provider = self.provider_for_session(session_id);
        let next = self.manual_switch_epoch(session_id).wrapping_add(1);
        if let Ok(mut map) = self.manual_switch.write() {
            map.insert(session_id, (next, provider, model));
        }
    }

    /// Current manual-switch epoch for a session (0 if never switched).
    pub fn manual_switch_epoch(&self, session_id: Uuid) -> u64 {
        self.manual_switch
            .read()
            .ok()
            .and_then(|m| m.get(&session_id).map(|(e, _, _)| *e))
            .unwrap_or(0)
    }

    /// If the user manually switched this session AFTER `since_epoch`,
    /// re-install their pinned provider+model pair (atomically, so the
    /// model can never desync from the provider) and return the model so
    /// the caller can persist it to the session DB row. Returns `None`
    /// when there was no mid-turn switch. Called once, AFTER a turn
    /// completes — never on the completion path — so it cannot affect
    /// whether the turn delivered a response.
    pub fn restore_manual_switch_if_changed(
        &self,
        session_id: Uuid,
        since_epoch: u64,
    ) -> Option<String> {
        let pin = {
            let map = self.manual_switch.read().ok()?;
            let (epoch, provider, model) = map.get(&session_id)?;
            if *epoch == since_epoch {
                return None;
            }
            (provider.clone(), model.clone())
        };
        let (provider, model) = pin;
        self.swap_provider_for_session(session_id, provider, model.clone());
        Some(model)
    }

    /// Record that a sticky-fallback fired for this session. Intentionally a
    /// no-op for persistence: a transient rescue must NOT mutate the user's
    /// chosen provider/model. Earlier this function wrote both
    /// `session_models[sid]` AND `sessions.model` in DB, which converted
    /// every successful fallback into a permanent per-session pin the user
    /// never asked for. Concrete failure mode on 2026-06-04: dialagram
    /// fallback fired earlier in the day, persist_sticky_pair pinned
    /// `qwen-3.7-max-thinking` into the session row; user later set up a new
    /// modelscope-qwen provider via /models; the next turn read the stale
    /// pin and shipped `qwen-3.7-max-thinking` to modelscope-qwen → 400
    /// "Invalid model id". The pin had survived a complete provider change.
    ///
    /// Modern resolution path: tool_loop reads from DB but the cross-
    /// provider leak guard at the request site substitutes the active
    /// provider's default when the pinned model isn't in its catalogue.
    /// Sticky-fallback display is handled per-request via the
    /// `SwapEvent`/`ProviderSwitched` event stream; the user sees the swap
    /// in the footer while the underlying session record stays anchored on
    /// whatever they explicitly picked.
    ///
    /// Kept as a function (rather than deleted) so the dozen+ call sites in
    /// tool_loop.rs don't need a structural change in the same commit, and
    /// so we have a single place to re-add per-session persistence later if
    /// we ever introduce an opt-in "let fallbacks become my new default"
    /// preference.
    pub(crate) fn persist_sticky_pair(
        &self,
        session_id: Uuid,
        provider_name: String,
        model: String,
    ) {
        tracing::debug!(
            "persist_sticky_pair[{}]: fallback to {}/{} — not persisting (transient rescue, \
             user's session pick stays authoritative; tool_loop guards against cross-provider leaks)",
            session_id,
            provider_name,
            model
        );
    }

    /// Get context window size for a given model.
    ///
    /// Delegates to `context_limit()` so custom OpenAI-compatible providers
    /// that declare a `providers.custom.<name>.context_window` are honored
    /// here too. Without this, the TUI header reads the static
    /// `agent.context_limit` fallback (typically 200k) while the actual
    /// budget enforcer uses the provider-configured window — producing a
    /// misleading "202k/200k" when the engine is still safely inside its
    /// real limit.
    pub fn context_window_for_model(&self, _model: &str) -> u32 {
        self.context_limit()
    }

    /// Record that the agent just successfully accessed `raw_path`
    /// while operating under `working_directory`. Persists to the
    /// `recent_paths` table so a later session on the same project
    /// can re-anchor on real paths instead of guessing.
    ///
    /// Fire-and-forget: spawns a task and never blocks the tool loop.
    /// Both the working directory and the path are collapsed to
    /// `~/...` form before storage so the key is stable across
    /// machines and OS user names.
    pub fn record_recent_path(
        &self,
        working_directory: &std::path::Path,
        raw_path: &std::path::Path,
    ) {
        let wd_collapsed = crate::brain::tools::error::collapse_home(working_directory);
        let path_collapsed = crate::brain::tools::error::collapse_home(raw_path);
        if wd_collapsed.is_empty() || path_collapsed.is_empty() {
            return;
        }
        let pool = self.context.pool();
        tokio::spawn(async move {
            let repo = crate::db::repository::RecentPathsRepository::new(pool);
            if let Err(e) = repo.record(&wd_collapsed, &path_collapsed).await {
                tracing::debug!("recent_paths write failed: {e}");
            }
        });
    }

    /// Top recently-accessed paths under the given `working_directory`,
    /// most-recent first, capped at `RECENT_PATHS_CAP`. Returns an empty
    /// Vec when the project has no recorded paths yet (or on DB error).
    /// Stored & returned in `~/...` collapsed form.
    pub async fn recent_paths_for_dir(&self, working_directory: &std::path::Path) -> Vec<String> {
        let wd_collapsed = crate::brain::tools::error::collapse_home(working_directory);
        if wd_collapsed.is_empty() {
            return Vec::new();
        }
        let repo = crate::db::repository::RecentPathsRepository::new(self.context.pool());
        match repo.top_for_dir(&wd_collapsed, RECENT_PATHS_CAP).await {
            Ok(paths) => paths,
            Err(e) => {
                tracing::debug!("recent_paths read failed: {e}");
                Vec::new()
            }
        }
    }

    /// Build fallback providers from config for mid-stream rate limit recovery.
    async fn build_fallback_providers(config: &crate::config::Config) -> Vec<Arc<dyn Provider>> {
        if let Some(fallback) = &config.providers.fallback
            && fallback.enabled
        {
            let chain = crate::brain::provider::factory::fallback_chain(fallback);
            let mut providers = Vec::new();
            let mut skipped: Vec<String> = Vec::new();
            for name in &chain {
                match crate::brain::provider::factory::create_provider_by_name(config, name).await {
                    Ok(p) => {
                        tracing::info!("AgentService: fallback provider '{}' ready", name);
                        providers.push(p);
                    }
                    Err(e) => {
                        tracing::warn!("AgentService: fallback provider '{}' skipped: {}", name, e);
                        skipped.push(name.clone());
                    }
                }
            }
            // Summarise the chain result so operators see the totals in one
            // line instead of reconstructing them from per-provider logs (#260).
            tracing::info!(
                "AgentService: fallback chain built — {} ready, {} skipped (chain: [{}], skipped: [{}])",
                providers.len(),
                skipped.len(),
                chain.join(", "),
                skipped.join(", "),
            );
            providers
        } else {
            Vec::new()
        }
    }

    /// Check if any fallback providers are configured
    pub fn has_fallback_provider(&self) -> bool {
        !self
            .fallback_providers
            .read()
            .expect("fallback_providers lock poisoned")
            .is_empty()
    }

    /// Get the next fallback provider that isn't the currently active one.
    /// Walks the chain until it finds a different provider name.
    pub fn try_get_fallback_provider(&self) -> Option<Arc<dyn Provider>> {
        let active_name = self
            .provider
            .read()
            .ok()
            .map(|p| p.name().to_string())
            .unwrap_or_default();
        self.fallback_chain_snapshot()
            .into_iter()
            .find(|p| p.name() != active_name)
    }

    /// Record that a skill has been activated for a session. Called when
    /// `match_user_command` resolves a skill slash command. The full skill
    /// body is re-injected into the system brain on every turn so it
    /// survives context compaction (#219).
    pub fn register_active_skill(&self, session_id: Uuid, skill_name: &str) {
        let mut map = self
            .active_skills
            .write()
            .expect("active_skills lock poisoned");
        map.entry(session_id)
            .or_default()
            .insert(skill_name.to_string());
    }

    /// Get the set of active skill names for a session. Returns empty set
    /// if no skills have been activated.
    pub fn active_skills_for_session(&self, session_id: Uuid) -> HashSet<String> {
        self.active_skills
            .read()
            .expect("active_skills lock poisoned")
            .get(&session_id)
            .cloned()
            .unwrap_or_default()
    }
}
