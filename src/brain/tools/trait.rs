//! Tool trait definition

use super::error::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

/// Execution context for tools
#[derive(Clone)]
pub struct ToolExecutionContext {
    /// Session ID
    pub session_id: Uuid,

    /// Working directory
    pub working_directory: std::path::PathBuf,

    /// Environment variables
    pub env_vars: HashMap<String, String>,

    /// Whether auto-approve is enabled
    pub auto_approve: bool,

    /// Maximum execution timeout in seconds
    pub timeout_secs: u64,

    /// Callback for requesting sudo password from the user (set by TUI)
    pub sudo_callback: Option<crate::brain::agent::SudoCallback>,

    /// Callback for requesting an SSH password from the user (set by TUI).
    /// Wired to the same dialog plumbing as `sudo_callback` but with a
    /// different prompt label so the user knows it's an SSH server.
    pub ssh_callback: Option<crate::brain::agent::SshPasswordCallback>,

    /// Shared working directory handle — tools can mutate this to change the
    /// working directory at runtime (e.g. config_manager set_working_directory).
    pub shared_working_directory: Option<Arc<std::sync::RwLock<std::path::PathBuf>>>,

    /// Service context — tools use this to create SessionService for /usage stats.
    pub service_context: Option<crate::services::ServiceContext>,

    /// Non-blocking progress-event sink. Tools that surface fire-and-forget
    /// UI signals (e.g. `suggest_options`) emit a `ProgressEvent` through
    /// this without awaiting the user. None on surfaces that wire no progress
    /// bridge. Distinct from `question_callback`, which blocks for an answer.
    pub progress_callback: Option<crate::brain::agent::ProgressCallback>,

    /// Background-task manager (#722). When present, bash hands a known-long
    /// command here to run detached and resume the session on completion. `None`
    /// on surfaces without the enqueue producer, where bash runs inline.
    pub background_manager:
        Option<Arc<crate::brain::agent::service::background_tasks::BackgroundTaskManager>>,

    /// Plan-state session override (#908 option A). When set, plan state —
    /// the plan JSON, design `.md`, pre-init and autonomy markers, and the
    /// plan-task goal — resolves against THIS session id instead of
    /// `session_id`. Spawned plan workers carry the parent's session id here
    /// so their `plan` tool operates on the parent's checklist while the
    /// child's own session stays fresh. `None` (the default everywhere except
    /// plan-driven spawns) resolves against the session's own id.
    pub plan_session_override: Option<Uuid>,

    /// Sub-agent manager (#908). When present, tools may spawn child
    /// sessions directly — plan `start` uses this for isolated task
    /// execution. `None` on surfaces without sub-agent support.
    pub subagent_manager: Option<Arc<crate::brain::tools::subagent::SubAgentManager>>,

    /// The session's live tool registry (#908). Handed to spawned children
    /// so their registries inherit the parent's tools (filtered per agent
    /// type). `None` where no registry is wired.
    /// Config key of the provider this session is running on, when known.
    ///
    /// Vision resolution tries the CURRENT provider first (#1318), and a tool
    /// has no handle on `AgentService` to ask. Stamped per execution by the
    /// tool loop, which has both. `None` for surfaces that build a context
    /// without a live provider (cron, tests), where resolution simply starts
    /// at the configured chain instead.
    pub session_provider: Option<String>,

    pub parent_tool_registry: Option<Arc<crate::brain::tools::ToolRegistry>>,

    /// Headless session (#129): the enclosing agent runs on a surface with no
    /// live user (CLI one-shot run, cron execute, sub-agent). Interactive-only
    /// tools read this to hard-error instead of parking a verdict no one will
    /// ever see. `false` by default (interactive).
    pub headless: bool,
}

impl std::fmt::Debug for ToolExecutionContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolExecutionContext")
            .field("session_id", &self.session_id)
            .field("working_directory", &self.working_directory)
            .field("auto_approve", &self.auto_approve)
            .field("timeout_secs", &self.timeout_secs)
            .field("sudo_callback", &self.sudo_callback.is_some())
            .field("ssh_callback", &self.ssh_callback.is_some())
            .finish()
    }
}

impl ToolExecutionContext {
    /// Create a new execution context
    pub fn new(session_id: Uuid) -> Self {
        Self {
            session_id,
            working_directory: std::env::current_dir().unwrap_or_default(),
            env_vars: HashMap::new(),
            auto_approve: false,
            timeout_secs: 120,
            sudo_callback: None,
            ssh_callback: None,
            shared_working_directory: None,
            service_context: None,
            progress_callback: None,
            background_manager: None,
            plan_session_override: None,
            subagent_manager: None,
            session_provider: None,
            parent_tool_registry: None,
            headless: false,
        }
    }

    /// Set working directory
    pub fn with_working_directory(mut self, dir: std::path::PathBuf) -> Self {
        self.working_directory = dir;
        self
    }

    /// Set auto-approve
    pub fn with_auto_approve(mut self, auto_approve: bool) -> Self {
        self.auto_approve = auto_approve;
        self
    }

    /// Set timeout
    pub fn with_timeout(mut self, timeout_secs: u64) -> Self {
        self.timeout_secs = timeout_secs;
        self
    }

    /// Get the effective working directory.
    ///
    /// Reads from the shared lock if available (so `/cd` changes are visible
    /// to tools in the same iteration), falling back to the plain field.
    pub fn working_dir(&self) -> std::path::PathBuf {
        if let Some(ref shared) = self.shared_working_directory {
            shared
                .read()
                .ok()
                .map(|g| (*g).clone())
                .unwrap_or_else(|| self.working_directory.clone())
        } else {
            self.working_directory.clone()
        }
    }
}

/// Tool result
#[derive(Debug, Clone)]
pub struct ToolResult {
    /// Whether the execution was successful
    pub success: bool,

    /// Output from the tool
    pub output: String,

    /// Error message if unsuccessful
    pub error: Option<String>,

    /// Additional metadata
    pub metadata: HashMap<String, String>,

    /// Optional images to include alongside the text result.
    /// Each entry is (media_type, base64_data) — e.g. ("image/png", "<base64>").
    /// These are sent as ContentBlock::Image blocks following the ToolResult.
    pub images: Vec<(String, String)>,
}

impl ToolResult {
    /// Create a successful result
    pub fn success(output: String) -> Self {
        Self {
            success: true,
            output,
            error: None,
            metadata: HashMap::new(),
            images: Vec::new(),
        }
    }

    /// Create an error result
    pub fn error(error: String) -> Self {
        Self {
            success: false,
            output: String::new(),
            error: Some(error),
            metadata: HashMap::new(),
            images: Vec::new(),
        }
    }

    /// Attach images to the result (sent as ContentBlock::Image alongside the tool result).
    pub fn with_images(mut self, images: Vec<(String, String)>) -> Self {
        self.images = images;
        self
    }

    /// Add metadata
    pub fn with_metadata(mut self, key: String, value: String) -> Self {
        self.metadata.insert(key, value);
        self
    }
}

/// Tool capability flags
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolCapability {
    /// Can read files
    ReadFiles,
    /// Can write files
    WriteFiles,
    /// Can execute shell commands
    ExecuteShell,
    /// Can access network
    Network,
    /// Can modify system state
    SystemModification,
    /// Can manage plans and tasks
    PlanManagement,
}

/// MCP-style behavioral risk annotations for a tool.
///
/// Mirrors the Model Context Protocol's tool annotations: four orthogonal
/// axes describing a tool's behavior, not a flat capability taxonomy.
/// Defaults are deliberately pessimistic (assume the worst until a tool
/// declares otherwise), matching MCP: an unannotated tool is treated as
/// environment-modifying, destructive, non-idempotent, and open-world.
///
/// These hints are the source of truth for the risk decisions in
/// `classify.rs`. Because our tools are first-party (we author them), the
/// hints are trusted contracts the gate and approval policy can enforce,
/// not merely advisory UX hints as in MCP's untrusted-server model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolHints {
    /// Tool does not modify its environment. Default: `false`.
    pub read_only: bool,
    /// If it modifies, may perform destructive (vs additive) updates.
    /// Only meaningful when `read_only == false`. Default: `true`.
    pub destructive: bool,
    /// Safe to call repeatedly with the same args (no extra effect).
    /// Only meaningful when `read_only == false`. Default: `false`.
    pub idempotent: bool,
    /// Interacts with an open world of external entities vs a closed
    /// domain. Default: `true`.
    pub open_world: bool,
}

impl Default for ToolHints {
    fn default() -> Self {
        // Pessimistic / fail-safe defaults, matching MCP.
        Self {
            read_only: false,
            destructive: true,
            idempotent: false,
            open_world: true,
        }
    }
}

impl ToolHints {
    /// Bridge: derive hints from a legacy [`ToolCapability`] set.
    ///
    /// Used by the default [`Tool::hints`] implementation so existing
    /// tools keep their current behavior while migrating to explicit
    /// hints. The destructive trio maps to `!read_only && destructive`;
    /// `Network` maps to `open_world`.
    pub fn from_capabilities(capabilities: &[ToolCapability]) -> Self {
        let destructive = super::classify::is_destructive(capabilities);
        Self {
            read_only: !destructive,
            destructive,
            idempotent: false,
            open_world: capabilities.contains(&ToolCapability::Network),
        }
    }
}

/// Tool trait - defines an executable tool
#[async_trait]
pub trait Tool: Send + Sync {
    /// Get the tool name
    fn name(&self) -> &str;

    /// Get the tool description
    fn description(&self) -> &str;

    /// Get the input schema (JSON Schema format)
    fn input_schema(&self) -> Value;

    /// Get the tool's capabilities
    fn capabilities(&self) -> Vec<ToolCapability>;

    /// Whether executing this tool should end the agent's turn after the
    /// result is flushed (#1178 M1). Only `suggest_options` overrides this:
    /// its options ARE the turn's ending - the user picks one and the next
    /// turn resumes from that choice.
    fn halts_turn(&self) -> bool {
        false
    }

    /// Get the tool's MCP-style behavioral risk hints.
    ///
    /// Defaults to deriving from [`Tool::capabilities`]; override per-tool
    /// for precision (e.g. mark a channel send as `destructive` +
    /// `open_world`, or a pure read as `read_only` + `idempotent`).
    fn hints(&self) -> ToolHints {
        ToolHints::from_capabilities(&self.capabilities())
    }

    /// Check if the tool requires approval before execution
    fn requires_approval(&self) -> bool {
        super::classify::is_destructive_hints(&self.hints())
    }

    /// Check if this specific invocation requires approval.
    /// Override for tools where only certain operations need approval (e.g. plan finalize).
    fn requires_approval_for_input(&self, _input: &Value) -> bool {
        self.requires_approval()
    }

    /// Execute the tool with given input
    async fn execute(&self, input: Value, context: &ToolExecutionContext) -> Result<ToolResult>;

    /// Validate input before execution
    fn validate_input(&self, _input: &Value) -> Result<()> {
        // Default implementation - no validation
        Ok(())
    }
}
