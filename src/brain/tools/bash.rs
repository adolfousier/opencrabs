//! Bash/Shell Command Execution Tool
//!
//! Allows executing shell commands in the system.

use super::error::{Result, ToolError, expand_tilde};
use super::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
use crate::utils::long_command::Detach;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, VecDeque};
use std::sync::{OnceLock, RwLock};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::time::{Duration, timeout};
use uuid::Uuid;

// ── TOML blocklist (runtime-configurable safety rules) ─────────────
// Loaded once from ~/.opencrabs/safety/bash_blocklist.toml on first use.
// Adds patterns on top of the hardcoded blocklist. The hardcoded rules
// are the immovable floor; TOML adds runtime-configurable patterns.

#[derive(Debug, Deserialize)]
pub(crate) struct TomlBlocklist {
    #[allow(dead_code)]
    pub(crate) meta: Option<toml::Value>,
    #[serde(default)]
    pub(crate) rules: Vec<TomlBlockRule>,
    #[serde(default)]
    pub(crate) overrides: Vec<TomlOverride>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct TomlBlockRule {
    #[allow(dead_code)]
    pub(crate) id: String,
    #[allow(dead_code)]
    pub(crate) category: String,
    pub(crate) severity: String,
    pub(crate) reason: String,
    #[serde(default)]
    pub(crate) patterns: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct TomlOverride {
    #[allow(dead_code)]
    pub(crate) id: String,
    #[allow(dead_code)]
    pub(crate) reason: String,
    pub(crate) patterns: Vec<String>,
}

/// The TOML blocklist, reloaded when the file changes on disk.
///
/// Was a `OnceLock`, which read the file once per process and made the
/// "editable at runtime, no rebuild" promise false: a new pattern needed a
/// restart. Keyed on mtime + length now, so an edit takes effect on the next
/// bash call (#851). The hardcoded blocklist is always active; this only adds.
fn toml_blocklist() -> Option<std::sync::Arc<TomlBlocklist>> {
    static BLOCKLIST: OnceLock<Option<super::toml_hot_reload::HotToml<TomlBlocklist>>> =
        OnceLock::new();
    BLOCKLIST
        .get_or_init(|| {
            super::toml_hot_reload::safety_path("bash_blocklist.toml")
                .map(|p| super::toml_hot_reload::HotToml::new(p, "TOML blocklist"))
        })
        .as_ref()?
        .get()
}

/// Check the command against the TOML blocklist. Returns `Some(reason)`
/// if blocked, `None` if allowed. Overrides are checked first: if any
/// override pattern matches, the command is allowed even if a rule also
/// matches. Rules use simple substring matching on normalized (lowercase,
/// whitespace-collapsed) input.
fn check_toml_blocklist(command: &str) -> Option<String> {
    let blocklist = toml_blocklist()?;
    evaluate_blocklist(&blocklist, command)
}

/// Pure match of `command` against an already-loaded blocklist.
///
/// Split from the loader so the precedence rules can be tested without a home
/// directory or a file on disk. Overrides are checked first: an override match
/// allows the command even when a rule also matches. It can only relax TOML
/// rules — the hardcoded blocklist runs before this is ever reached.
///
/// Matching is substring on normalized (lowercased, whitespace-collapsed)
/// input, so a pattern is exactly as precise as the string chosen for it.
pub(crate) fn evaluate_blocklist(blocklist: &TomlBlocklist, command: &str) -> Option<String> {
    let normalized: String = command
        .split_whitespace()
        .collect::<Vec<&str>>()
        .join(" ")
        .to_lowercase();

    for ov in &blocklist.overrides {
        if ov
            .patterns
            .iter()
            .any(|p| normalized.contains(&p.to_lowercase()))
        {
            return None;
        }
    }

    for rule in &blocklist.rules {
        if rule.severity != "block" || rule.patterns.is_empty() {
            continue;
        }
        if rule
            .patterns
            .iter()
            .any(|p| normalized.contains(&p.to_lowercase()))
        {
            return Some(format!("{} [runtime rule: {}]", rule.reason, rule.id));
        }
    }

    None
}

/// Detach a child from the controlling terminal before `exec`.
///
/// Without this, programs that bypass `stdin` and open `/dev/tty` directly
/// (ssh password prompt, sudo's getpass fallback, gnu readline) steal the
/// parent's TTY — when the parent is the OpenCrabs TUI, that means the
/// child reads the user's keystrokes, switches the terminal into no-echo
/// mode, and emits escape sequences into the rendered chat (cursor keys
/// turn into `[[[[[`). Restart fixes it because the TTY mode is reset.
///
/// `setsid()` puts the child in a brand-new session with no controlling
/// TTY, so `open("/dev/tty")` inside the child returns `ENXIO` and the
/// program either errors out cleanly or falls back to a non-TTY path
/// (e.g. SSH consults `SSH_ASKPASS`). The TUI is never touched.
#[cfg(unix)]
fn detach_session_pre_exec(cmd: &mut Command) {
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

#[cfg(not(unix))]
fn detach_session_pre_exec(_cmd: &mut Command) {
    // No-op on Windows — the TTY-bleed problem doesn't apply (different
    // console model) and pre_exec is a Unix-only API.
}

/// First-token of a shell command, normalized for SSH detection.
fn first_token(cmd: &str) -> &str {
    cmd.split([';', '|', '&'])
        .next()
        .unwrap_or(cmd)
        .split_whitespace()
        .find(|tok| !tok.contains('='))
        .unwrap_or("")
}

/// Pretty-print the SSH/scp/rsync target so the password dialog has a
/// recognisable hostname. Returns `None` for non-SSH commands and for
/// any SSH invocation that already carries a non-interactive auth
/// hint (`-o BatchMode=yes`, `-o PreferredAuthentications=publickey`)
/// — in those cases we let the command fail naturally rather than
/// prompting.
pub(crate) fn parse_ssh_invocation(command: &str) -> Option<String> {
    let cmd = command.trim();
    let first = first_token(cmd);
    let is_ssh_like = matches!(first, "ssh" | "scp" | "sftp" | "rsync");
    if !is_ssh_like {
        return None;
    }

    // ssh-keygen / ssh-keyscan / ssh-add never prompt for a remote
    // password — skip them.
    if first == "ssh" {
        let second = cmd.split_whitespace().nth(1).unwrap_or("");
        if second.is_empty() || second.starts_with('-') {
            // ok
        } else if second.contains('@') || !second.contains('-') {
            // looks like a host
        }
    }

    let lower = cmd.to_lowercase();
    if lower.contains("batchmode=yes")
        || lower.contains("preferredauthentications=publickey")
        || lower.contains("passwordauthentication=no")
    {
        return None;
    }

    // Best-effort host extraction: first token that contains '@' or
    // looks like host:path (scp/rsync) and isn't a flag.
    for tok in cmd.split_whitespace().skip(1) {
        if tok.starts_with('-') {
            continue;
        }
        if tok.contains('@') {
            return Some(tok.to_string());
        }
        if tok.contains(':') && !tok.starts_with('/') {
            return Some(tok.to_string());
        }
    }
    // Fallback: just the bare hostname after `ssh`
    if first == "ssh" {
        for tok in cmd.split_whitespace().skip(1) {
            if !tok.starts_with('-') {
                return Some(tok.to_string());
            }
        }
    }
    Some(format!("(unknown {} target)", first))
}

/// Inject `-o BatchMode=yes -o ConnectTimeout=15` after the leading
/// `ssh`/`scp`/`sftp`/`rsync` token so we can probe whether key auth
/// works without ever blocking on a TTY prompt. Returns the rewritten
/// command, leaving the rest of the args untouched.
fn inject_batch_mode(command: &str) -> String {
    let trimmed_start = command.trim_start();
    let leading_ws = &command[..command.len() - trimmed_start.len()];

    // Find the end of the first token (the binary name).
    let first_end = trimmed_start
        .find(char::is_whitespace)
        .unwrap_or(trimmed_start.len());
    let (head, tail) = trimmed_start.split_at(first_end);

    let probe_opts = " -o BatchMode=yes -o ConnectTimeout=15 -o StrictHostKeyChecking=accept-new";
    format!("{}{}{}{}", leading_ws, head, probe_opts, tail)
}

/// Heuristic: did this stderr come from an SSH/scp/rsync auth failure
/// (vs. a network or hostname error)? We only retry the password flow
/// for genuine auth-rejected cases.
fn ssh_stderr_is_auth_failure(stderr: &str) -> bool {
    let s = stderr.to_lowercase();
    s.contains("permission denied")
        || s.contains("password:")
        || s.contains("host key verification failed")
        || s.contains("publickey,password")
        || s.contains("publickey,keyboard-interactive")
        || s.contains("no supported authentication methods")
        || (s.contains("ssh:") && s.contains("authentication"))
}

/// Bash execution tool
pub struct BashTool;

#[derive(Debug, Deserialize, Serialize)]
struct BashInput {
    /// Command to execute
    command: String,

    /// Optional working directory (overrides context)
    #[serde(skip_serializing_if = "Option::is_none")]
    working_dir: Option<String>,

    /// Optional timeout in seconds (overrides context default)
    #[serde(skip_serializing_if = "Option::is_none")]
    timeout_secs: Option<u64>,

    /// Optional flag to run detached in background (true) or force inline (false)
    #[serde(skip_serializing_if = "Option::is_none")]
    background: Option<bool>,
}

#[async_trait]
impl Tool for BashTool {
    fn name(&self) -> &str {
        "bash"
    }

    fn description(&self) -> &str {
        "Execute a shell command. Returns stdout, stderr, and exit code. \
         stdin is /dev/null — interactive commands (git add -p, git rebase -i, \
         vim/nano/less/top, REPLs like python/node with no script) will not work; \
         use non-interactive flags (-A, -m, --no-edit) or pipe input via heredoc/echo. \
         Each call is a fresh shell — `cd` does not persist across calls; chain with \
         `&&` or use `git -C <path> <cmd>` for cross-directory work. Use carefully \
         as this can modify system state. \
         \n\nGITHUB OPERATIONS: use the `gh` CLI via this tool for \
         everything GitHub — issues, PRs, releases, comments, file \
         fetches, repo / code search, workflow runs, checks. `gh` is \
         preinstalled and authenticated; it returns structured JSON \
         (--json flag) and respects --jq for filtering. Never reach \
         for `browser_navigate` to inspect or act on a GitHub URL. \
         Examples: `gh pr view 123 --json title,body,comments`, \
         `gh issue list --label bug --json number,title`, \
         `gh api repos/OWNER/REPO/commits/SHA/check-runs`."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "Shell command to execute"
                },
                "working_dir": {
                    "type": "string",
                    "description": "Optional: Working directory for command execution"
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "Optional: Timeout in seconds (default 120, max 600). Use higher values for builds."
                },
                "background": {
                    "type": "boolean",
                    "description": "Optional: Set to true to run the command in the background (detached), or false to force inline execution. If omitted, uses automatic heuristic detection."
                }
            },
            "required": ["command"]
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![
            ToolCapability::ExecuteShell,
            ToolCapability::SystemModification,
            ToolCapability::Network,
        ]
    }

    fn requires_approval(&self) -> bool {
        true // Shell execution always requires approval
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        let input: BashInput = serde_json::from_value(input.clone())
            .map_err(|e| ToolError::InvalidInput(format!("Invalid input: {}", e)))?;

        if input.command.trim().is_empty() {
            return Err(ToolError::InvalidInput(
                "Command cannot be empty".to_string(),
            ));
        }

        // Hard blocklist — these commands are NEVER allowed, even if the user
        // accidentally approves them. This is a last line of defense against
        // catastrophic, irreversible operations.
        if let Some(reason) = check_blocked_command(&input.command) {
            return Err(ToolError::InvalidInput(format!(
                "Blocked: {}. This command is on the hard blocklist and cannot be executed.",
                reason
            )));
        }

        // Runtime TOML blocklist — additional patterns loaded from
        // ~/.opencrabs/safety/bash_blocklist.toml. Editable at runtime
        // with approval gate. Checked AFTER the hardcoded rules so the
        // compiled floor always applies. Overrides in the TOML can allow
        // commands that would otherwise match a TOML rule (but NOT the
        // hardcoded rules).
        if let Some(reason) = check_toml_blocklist(&input.command) {
            return Err(ToolError::InvalidInput(format!(
                "Blocked: {}. This command matches a runtime blocklist rule.",
                reason
            )));
        }

        Ok(())
    }

    async fn execute(&self, input: Value, context: &ToolExecutionContext) -> Result<ToolResult> {
        let input: BashInput = serde_json::from_value(input)?;

        // Determine working directory
        let working_dir = if let Some(ref dir) = input.working_dir {
            // Expand a leading `~` before the exists-check: PathBuf::from("~")
            // is a relative path named `~` that never exists (#1165).
            let expanded = expand_tilde(dir);
            if dir.starts_with('~') {
                tracing::info!("bash working_dir expanded: {dir} -> {}", expanded.display());
            }
            expanded
        } else {
            context.working_dir()
        };

        // Verify working directory exists
        if !working_dir.exists() {
            let msg = format!(
                "Working directory does not exist: {}",
                working_dir.display()
            );
            record_bash_outcome(
                context.session_id,
                input.command.clone(),
                true,
                Some(msg.clone()),
            );
            return Ok(ToolResult::error(msg));
        }

        // Layer 3: short-circuit if the agent just ran this exact
        // command and it failed. Stops the same-command-twice loop
        // dead with a strong "stop retrying" message instead of
        // letting the underlying error fire a second time.
        if let Some(msg) = check_recent_failure(context.session_id, &input.command) {
            return Ok(ToolResult::error(msg));
        }

        // Layer 1: refuse interactive commands. With stdin=/dev/null
        // (the post-2026-04-23 stdin-detach fix) most of these exit
        // cleanly on EOF — looks like success, did nothing — and the
        // agent gets stuck retrying. Surface the failure clearly with
        // a non-interactive alternative so the loop breaks on attempt 1.
        if let Some(hint) = check_interactive_command(&input.command) {
            record_bash_outcome(
                context.session_id,
                input.command.clone(),
                true,
                Some(hint.to_string()),
            );
            return Ok(ToolResult::error(hint.to_string()));
        }

        // Prepare command for the current platform
        let (shell, shell_arg) = if cfg!(target_os = "windows") {
            ("cmd", "/C")
        } else {
            ("sh", "-c")
        };

        // Determine timeout: use input override if provided, else context default, cap at 600s
        let effective_timeout = input.timeout_secs.unwrap_or(context.timeout_secs).min(600);

        // Genuinely long tasks (builds, test runs, CI waits, renders) run
        // DETACHED and resume the session on completion instead of churning
        // toward the 600s cap (#722). Only when a surface wired the background
        // manager; sudo needs the inline password flow, so it's excluded.
        // Anything else falls through and runs inline exactly as before.
        // #1195: pure workers (allow_nested=false) lose detachment too -
        // their long commands run inline so nothing outlives their verdict.
        let is_sudo = input.command.trim_start().starts_with("sudo ");
        let nesting_ok = context
            .subagent_manager
            .as_ref()
            .map(|m| m.nesting_allowed_for_session(context.session_id))
            .unwrap_or(true);

        // Check explicit background override or heuristic auto-detection
        match input.background {
            Some(false) => {
                tracing::debug!(
                    target: "background_task",
                    "Forced inline execution (background: false) for session {}",
                    context.session_id
                );
            }
            Some(true) => {
                if is_sudo {
                    return Ok(ToolResult::error(
                        "Cannot run sudo commands in the background (password prompt requires inline execution)",
                    ));
                }
                if !nesting_ok {
                    return Ok(ToolResult::error(
                        "Background execution is disabled in pure worker sessions (allow_nested=false)",
                    ));
                }
                let Some(ref mgr) = context.background_manager else {
                    return Ok(ToolResult::error(
                        "Background execution is unavailable on this surface (no background task manager)",
                    ));
                };

                let label =
                    crate::brain::agent::service::background_tasks::short_label(&input.command);
                tracing::info!(
                    target: "background_task",
                    "Explicitly detaching '{label}' for session {} (background: true)",
                    context.session_id
                );
                mgr.clone().spawn_command(
                    context.session_id,
                    context.working_directory.clone(),
                    label.clone(),
                    input.command.clone(),
                );
                return Ok(ToolResult::success(format!(
                    "Started in the background: {label}\n\nThis is a long-running task, so it \
                     is running detached. I'll continue this session automatically when it \
                     finishes — no need to poll or wait. Do other independent work meanwhile \
                     if there is any."
                )));
            }
            None => {
                if let Some(ref mgr) = context.background_manager
                    && !is_sudo
                    && nesting_ok
                {
                    match crate::utils::long_command::classify(&input.command) {
                        Detach::Yes { marker } => {
                            let label = crate::brain::agent::service::background_tasks::short_label(
                                &input.command,
                            );
                            tracing::info!(
                                target: "background_task",
                                "Detaching '{label}' for session {}: '{marker}' starts a command",
                                context.session_id
                            );
                            mgr.clone().spawn_command(
                                context.session_id,
                                context.working_directory.clone(),
                                label.clone(),
                                input.command.clone(),
                            );
                            return Ok(ToolResult::success(format!(
                                "Started in the background: {label}\n\nThis is a long-running task, so it \
                                 is running detached. I'll continue this session automatically when it \
                                 finishes — no need to poll or wait. Do other independent work meanwhile \
                                 if there is any."
                            )));
                        }
                        // Worth a line: the command looks long to a reader, and the
                        // reason it ran inline is not visible anywhere else.
                        Detach::Mentioned { marker } => tracing::debug!(
                            target: "background_task",
                            "Running inline: '{marker}' appears only as data (heredoc body or quoted \
                             argument), not as a command"
                        ),
                        Detach::No => {}
                    }
                }
            }
        }

        // Detect sudo commands and request password via callback
        let sudo_password = if is_sudo {
            if let Some(ref callback) = context.sudo_callback {
                match callback(input.command.clone()).await {
                    Ok(Some(password)) => Some(password),
                    Ok(None) => return Ok(ToolResult::error("Sudo cancelled by user".to_string())),
                    Err(e) => return Ok(ToolResult::error(format!("Sudo prompt failed: {}", e))),
                }
            } else {
                None // No callback — run normally (will fail if password needed)
            }
        } else {
            None
        };

        // Execute command with timeout — use piped stdin for sudo password
        // RTK rewrite tracking (hoisted from normal-execution branch)
        #[cfg(feature = "rtk")]
        let mut rtk_was_rewritten = false;

        let output = if let Some(password) = sudo_password {
            // Rewrite command to read password from stdin via -S flag
            // Use -p "" to suppress sudo's own prompt (we handle it in the TUI)
            let sudo_cmd = if input.command.trim_start().starts_with("sudo -S ") {
                input.command.clone()
            } else {
                input.command.replacen("sudo ", "sudo -S -p \"\" ", 1)
            };

            let command_future = async {
                let mut cmd = Command::new(shell);
                // Reap the child if this future is dropped. A timeout drops it, and
                // tokio leaves the process running unless told otherwise (#1046).
                cmd.kill_on_drop(true);
                cmd.arg(shell_arg)
                    .arg(&sudo_cmd)
                    .current_dir(&working_dir)
                    .stdin(std::process::Stdio::piped())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped());
                detach_session_pre_exec(&mut cmd);
                let mut child = cmd.spawn()?;

                // Write password to stdin and close it
                if let Some(mut stdin) = child.stdin.take() {
                    if let Err(e) = stdin.write_all(format!("{}\n", password).as_bytes()).await {
                        tracing::warn!(error = %e, "failed to write password to bash stdin");
                    }
                    drop(stdin);
                }

                child.wait_with_output().await
            };

            match timeout(Duration::from_secs(effective_timeout), command_future).await {
                Ok(Ok(output)) => output,
                Ok(Err(e)) => {
                    return Ok(ToolResult::error(format!(
                        "Command execution failed: {}",
                        e
                    )));
                }
                Err(_) => {
                    return Err(ToolError::Timeout(effective_timeout));
                }
            }
        } else if let Some(ssh_target) = parse_ssh_invocation(&input.command) {
            // SSH-like commands need special handling: pre_exec/setsid means
            // the child has no TTY, so an interactive password prompt would
            // exit immediately with "no controlling terminal". We probe with
            // BatchMode=yes first (key-only auth) and fall back to the
            // password callback + SSH_ASKPASS if the probe rejects auth.
            let probe_cmd = inject_batch_mode(&input.command);
            let probe_future = async {
                let mut cmd = Command::new(shell);
                // Reap the child if this future is dropped. A timeout drops it, and
                // tokio leaves the process running unless told otherwise (#1046).
                cmd.kill_on_drop(true);
                cmd.arg(shell_arg)
                    .arg(&probe_cmd)
                    .current_dir(&working_dir)
                    .stdin(std::process::Stdio::null());
                detach_session_pre_exec(&mut cmd);
                cmd.output().await
            };

            let probe_output =
                match timeout(Duration::from_secs(effective_timeout), probe_future).await {
                    Ok(Ok(o)) => o,
                    Ok(Err(e)) => {
                        return Ok(ToolResult::error(format!(
                            "Command execution failed: {}",
                            e
                        )));
                    }
                    Err(_) => {
                        return Err(ToolError::Timeout(effective_timeout));
                    }
                };

            let probe_stderr = String::from_utf8_lossy(&probe_output.stderr).to_string();
            let probe_succeeded = probe_output.status.success();
            let auth_failed = !probe_succeeded && ssh_stderr_is_auth_failure(&probe_stderr);

            if probe_succeeded || !auth_failed {
                probe_output
            } else if let Some(ref callback) = context.ssh_callback {
                // Auth failed and a password callback is wired — request the
                // password from the user, then retry with SSH_ASKPASS pointing
                // at a script that emits the password to stdout.
                let prompt = format!(
                    "{} ({})",
                    ssh_target,
                    input.command.split_whitespace().next().unwrap_or("ssh")
                );
                let password = match callback(prompt).await {
                    Ok(Some(p)) => p,
                    Ok(None) => {
                        return Ok(ToolResult::error(
                            "SSH password cancelled by user".to_string(),
                        ));
                    }
                    Err(e) => {
                        return Ok(ToolResult::error(format!(
                            "SSH password prompt failed: {}",
                            e
                        )));
                    }
                };

                let askpass = match SshAskpass::new(&password) {
                    Ok(a) => a,
                    Err(e) => {
                        return Ok(ToolResult::error(format!(
                            "Failed to set up SSH_ASKPASS: {}",
                            e
                        )));
                    }
                };

                let retry_future = async {
                    let mut cmd = Command::new(shell);
                    // Reap the child if this future is dropped. A timeout drops it, and
                    // tokio leaves the process running unless told otherwise (#1046).
                    cmd.kill_on_drop(true);
                    cmd.arg(shell_arg)
                        .arg(&input.command)
                        .current_dir(&working_dir)
                        .stdin(std::process::Stdio::null())
                        .env("SSH_ASKPASS", askpass.script_path())
                        .env("SSH_ASKPASS_REQUIRE", "force")
                        // SSH_ASKPASS_REQUIRE=force on modern OpenSSH ignores
                        // DISPLAY, but older builds (Debian 11, macOS preinstalled)
                        // still gate on it being non-empty. Keep both happy.
                        .env("DISPLAY", ":0");
                    detach_session_pre_exec(&mut cmd);
                    cmd.output().await
                };

                match timeout(Duration::from_secs(effective_timeout), retry_future).await {
                    Ok(Ok(output)) => output,
                    Ok(Err(e)) => {
                        return Ok(ToolResult::error(format!("SSH retry failed: {}", e)));
                    }
                    Err(_) => {
                        return Err(ToolError::Timeout(effective_timeout));
                    }
                }
            } else {
                // No password callback wired (channel session) — return the
                // probe output as-is so the agent sees the auth-failure
                // stderr and can either retry with explicit `-i <key>` or
                // ask the user out of band.
                probe_output
            }
        } else {
            // Normal execution (no sudo password needed).
            // stdin is set to /dev/null to detach from the parent TTY — when
            // the TUI is running with mouse capture enabled, leaving stdin
            // inherited lets subshells (pipes, `read`, `cat`) swallow
            // mouse-report escape sequences off the terminal and emit them
            // on stdout, where they land in the tool-output buffer and
            // leak into the rendered TUI message.
            // setsid() additionally puts the child in a fresh session
            // with no controlling TTY — programs that bypass stdin and
            // open /dev/tty directly (ssh prompt, sudo getpass) cannot
            // steal the user's keyboard or corrupt the TUI.

            // RTK rewriting: if the rtk feature is enabled and RTK is available,
            // try to rewrite the command to save tokens (60-90% reduction).
            #[cfg(feature = "rtk")]
            let execution_command = if crate::rtk::is_rtk_available().await {
                match crate::rtk::rewrite_command(&input.command).await {
                    Some(result) => {
                        tracing::debug!(
                            "RTK rewrote command: '{}' -> '{}'",
                            input.command,
                            result.rewritten_command
                        );
                        rtk_was_rewritten = result.was_rewritten;
                        result.rewritten_command
                    }
                    None => input.command.clone(),
                }
            } else {
                input.command.clone()
            };

            #[cfg(not(feature = "rtk"))]
            let execution_command = input.command.clone();

            let command_future = async {
                let mut cmd = Command::new(shell);
                // Reap the child if this future is dropped. A timeout drops it, and
                // tokio leaves the process running unless told otherwise (#1046).
                cmd.kill_on_drop(true);
                cmd.arg(shell_arg)
                    .arg(&execution_command)
                    .current_dir(&working_dir)
                    .stdin(std::process::Stdio::null());
                detach_session_pre_exec(&mut cmd);
                cmd.output().await
            };

            match timeout(Duration::from_secs(effective_timeout), command_future).await {
                Ok(Ok(output)) => output,
                Ok(Err(e)) => {
                    return Ok(ToolResult::error(format!(
                        "Command execution failed: {}",
                        e
                    )));
                }
                Err(_) => {
                    return Err(ToolError::Timeout(effective_timeout));
                }
            }
        };

        // Convert output to strings
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let exit_code = output.status.code().unwrap_or(-1);

        // RTK metrics tracking: record that the command was rewritten.
        // We can't measure "original" tokens without running the command twice,
        // so we record the filtered output size. RTK's own `gain` command tracks
        // the actual savings vs what the unfiltered output would have been.
        #[cfg(feature = "rtk")]
        {
            if rtk_was_rewritten {
                let filtered_output = format!("{}\n{}", stdout, stderr);
                let filtered_tokens = filtered_output.split_whitespace().count();

                let savings = crate::rtk::TokenSavings {
                    command: input.command.clone(),
                    rewritten_command: format!("rtk {}", input.command),
                    original_tokens: 0,
                    filtered_tokens,
                    tokens_saved: 0,
                    savings_percent: 0.0,
                    timestamp: chrono::Utc::now(),
                };

                crate::rtk::global_tracker().record_savings(savings).await;

                tracing::info!(
                    "RTK rewrote command ({} filtered tokens): {}",
                    filtered_tokens,
                    input.command
                );
            }
        }

        // Build output message
        let mut result_text = String::new();

        if !stdout.is_empty() {
            result_text.push_str("STDOUT:\n");
            result_text.push_str(&stdout);
        }

        if !stderr.is_empty() {
            if !result_text.is_empty() {
                result_text.push_str("\n\n");
            }
            result_text.push_str("STDERR:\n");
            result_text.push_str(&stderr);
        }

        if result_text.is_empty() {
            result_text = "(no output)".to_string();
        }

        // grep/rg exit 1 with nothing on stderr means "no line matched" — a
        // successful search with zero results, not a command failure (real
        // errors surface as exit 2+ and/or stderr output). Treating that as a
        // failure inflated the bash failure rate with search noise (#663).
        let search_no_match = is_search_no_match(&input.command, exit_code, &stderr);
        let success = output.status.success() || search_no_match;

        if search_no_match && result_text == "(no output)" {
            result_text = "(no matches found)".to_string();
        }

        // Record the outcome so a follow-up call with the exact same
        // command can be short-circuited by Layer 3 instead of re-running.
        let error_snippet = if !success {
            Some(result_text.chars().take(300).collect::<String>())
        } else {
            None
        };
        record_bash_outcome(
            context.session_id,
            input.command.clone(),
            !success,
            error_snippet,
        );

        let result = if success {
            ToolResult::success(result_text)
        } else {
            ToolResult {
                success: false,
                output: result_text,
                error: Some(format!("Command exited with code {}", exit_code)),
                metadata: std::collections::HashMap::new(),
                images: Vec::new(),
            }
        };

        Ok(result
            .with_metadata("exit_code".to_string(), exit_code.to_string())
            .with_metadata("working_dir".to_string(), working_dir.display().to_string()))
    }
}

/// Whether a non-zero bash exit is a benign "search found no matches" rather
/// than a real failure. `grep`/`rg` exit 1 (with nothing on stderr) when no
/// line matches; that is a successful search with zero results, not an error
/// (exit 2+ / stderr output is a genuine error). Counting these as failures
/// inflated the bash failure rate with noise (#663).
pub(crate) fn is_search_no_match(command: &str, exit_code: i32, stderr: &str) -> bool {
    if exit_code != 1 || !stderr.trim().is_empty() {
        return false;
    }
    // Check each pipe / chain segment's leading command token.
    command.split(['|', ';', '&']).any(|segment| {
        let seg = segment.trim();
        let first = seg.split_whitespace().next().unwrap_or("");
        matches!(first, "grep" | "egrep" | "fgrep" | "rg") || seg.starts_with("git grep")
    })
}

/// Hard blocklist check for dangerous commands.
///
/// Returns `Some(reason)` if the command matches a blocked pattern,
/// `None` if the command is allowed to proceed (still requires approval).
///
/// This is intentionally conservative — it blocks patterns that are
/// almost never legitimate in an AI agent context and would cause
/// catastrophic, irreversible damage if executed.
/// Normalize an `rm` target token for blocklist comparison: strip surrounding
/// quotes and map the (already-lowercased) `$HOME` / `${HOME}` env vars to `~`,
/// so `"$HOME"`, `$home`, and `~` all compare equal.
fn normalize_rm_target(tok: &str) -> String {
    let t = tok.trim_matches(|c| c == '"' || c == '\'');
    if let Some(rest) = t
        .strip_prefix("${home}")
        .or_else(|| t.strip_prefix("$home"))
    {
        format!("~{rest}")
    } else {
        t.to_string()
    }
}

/// The single command-safety floor for every execution tool (OC-06).
///
/// `bash` documents its hard blocklist as applying even under approval, but
/// `execute_code`, dynamic shell executors, and cron-embedded shell ran their
/// commands without it, so `rm -rf ~` (and fork bombs, `/etc/shadow` reads, …)
/// through those tools was a complete bypass of the "immovable floor". This
/// runs the hardcoded blocklist and the runtime TOML blocklist, the same pair
/// `bash` itself uses, and is what those other tools now call. Returns the
/// reason a command is refused, or None when it is allowed.
pub(crate) fn assert_command_allowed(command: &str) -> Option<String> {
    if let Some(reason) = check_blocked_command(command) {
        return Some(reason.to_string());
    }
    check_toml_blocklist(command)
}

pub(crate) fn check_blocked_command(command: &str) -> Option<&'static str> {
    check_blocked_inner(command, 0)
}

/// Strip ONE matching pair of surrounding quotes from a string. Used to peel
/// the code argument out of `bash -c '<code>'` / `echo '<code>' | sh` before
/// re-checking it. Leaves inner quotes alone.
fn unquote(s: &str) -> &str {
    let s = s.trim();
    for q in ['\'', '"'] {
        if s.len() >= 2 && s.starts_with(q) && s.ends_with(q) {
            return &s[1..s.len() - 1];
        }
    }
    s
}

/// Decode a base64 token (standard alphabet) into a UTF-8 string, or `None`.
/// Lets the blocklist see through `echo <b64> | base64 -d | bash`.
fn try_base64_decode(tok: &str) -> Option<String> {
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(tok.trim())
        .ok()?;
    String::from_utf8(bytes).ok()
}

/// Pull out any command strings that `command` would hand to a shell
/// interpreter, so the blocklist can re-check them. Covers the common ways a
/// catastrophic command gets smuggled past a literal scan:
///
/// - `bash -c '<code>'`, `sh -lc "<code>"`, `zsh -c …`  (the `-c` payload)
/// - `eval '<code>'`                                     (the eval payload)
/// - `echo '<code>' | bash`, `printf … | sh`            (pipe to a shell)
/// - `echo <b64> | base64 -d | bash`                    (base64 then shell)
///
/// Returns the decoded inner command(s); the caller recurses on each.
fn extract_interpreter_payloads(command: &str) -> Vec<String> {
    const SHELLS: [&str; 6] = ["bash", "sh", "zsh", "dash", "ash", "ksh"];
    let basename = |t: &str| -> String {
        t.trim_start_matches("./")
            .rsplit('/')
            .next()
            .unwrap_or(t)
            .to_lowercase()
    };
    let mut out: Vec<String> = Vec::new();
    let tokens: Vec<&str> = command.split_whitespace().collect();

    // `<shell> … -c <code>` — the `-c` value (and everything after it) is code.
    for i in 0..tokens.len() {
        if !SHELLS.contains(&basename(tokens[i]).as_str()) {
            continue;
        }
        for j in (i + 1)..tokens.len() {
            let tk = tokens[j];
            // `-c`, or a short cluster ending in c (`-lc`, `-ic`); NOT `--norc`.
            let is_c =
                tk == "-c" || (tk.starts_with('-') && !tk.starts_with("--") && tk.ends_with('c'));
            if is_c {
                out.push(unquote(&tokens[(j + 1)..].join(" ")).to_string());
                break;
            }
            if !tk.starts_with('-') {
                break; // a positional (script name) before -c => not the -c form
            }
        }
    }

    // `eval <code>` — everything after `eval` is code.
    if let Some(idx) = tokens.iter().position(|&t| basename(t) == "eval") {
        let payload = tokens[(idx + 1)..].join(" ");
        if !payload.is_empty() {
            out.push(unquote(&payload).to_string());
        }
    }

    // `<producer> | … | <shell>` — text piped into a shell is executed. Pull the
    // literal that `echo`/`printf` emits (base64-decoded if the pipe decodes it).
    if command.contains('|') {
        let stages: Vec<&str> = command.split('|').collect();
        let feeds_shell = stages.iter().any(|s| {
            s.split_whitespace()
                .next()
                .is_some_and(|w| SHELLS.contains(&basename(w).as_str()))
        });
        if feeds_shell {
            let has_b64 = stages.iter().any(|s| {
                let l = s.to_lowercase();
                l.contains("base64") && (l.contains(" -d") || l.contains("--decode"))
            });
            for s in &stages {
                let st = s.trim();
                let first = st.split_whitespace().next().unwrap_or("");
                if matches!(first, "echo" | "printf") {
                    let arg = unquote(st.split_once(char::is_whitespace).map_or("", |(_, a)| a));
                    if has_b64 {
                        if let Some(dec) = try_base64_decode(arg) {
                            out.push(dec);
                        }
                    } else {
                        out.push(arg.to_string());
                    }
                }
            }
        }
    }

    out
}

/// Depth-bounded core of [`check_blocked_command`]. Runs the literal scan on
/// `command`, then recurses into any code it would feed to a shell interpreter
/// so the same gate applies to `bash -c '<blocked>'`, `echo '<blocked>' | sh`,
/// `eval …`, and base64-wrapped variants. Depth is capped so a maliciously
/// self-nesting command can't spin the checker.
fn check_blocked_inner(command: &str, depth: usize) -> Option<&'static str> {
    if depth > 4 {
        return Some("excessively nested interpreter invocation");
    }

    // Normalize: collapse whitespace, lowercase for pattern matching
    let normalized: String = command
        .split_whitespace()
        .collect::<Vec<&str>>()
        .join(" ")
        .to_lowercase();

    // ── Recursive filesystem destruction ──────────────────────────
    // `rm` (optionally `sudo`) with a recursive flag targeting root/home/cwd.
    // Split into command segments first (so `echo x; rm -rf ~`, `... && rm
    // -rf ~`, and `rm -rf ~;echo` are all caught), then per-segment this is
    // robust to flag ORDER (-rf, -fr, -r -f), LONG flags (--recursive --force),
    // QUOTED targets ("$HOME"), and the $HOME/${HOME} env vars.
    let segmented = normalized
        .replace("&&", "\n")
        .replace("||", "\n")
        .replace(['|', ';', '&'], "\n");
    for segment in segmented.split('\n') {
        let tokens: Vec<&str> = segment.split_whitespace().collect();
        let sudo = tokens.first() == Some(&"sudo");
        let rm_idx = if sudo { 1 } else { 0 };
        if tokens.get(rm_idx) != Some(&"rm") {
            continue;
        }
        let mut recursive = false;
        let mut targets: Vec<String> = Vec::new();
        for &tok in tokens.iter().skip(rm_idx + 1) {
            if tok == "--recursive" {
                recursive = true;
            } else if tok.starts_with("--") {
                // other long flag (--force, --verbose, …) — ignore
            } else if tok.starts_with('-') {
                // short flag cluster: -rf, -fr, -r, -rfv … — recursive is what
                // makes a delete catastrophic.
                if tok.contains('r') {
                    recursive = true;
                }
            } else {
                targets.push(normalize_rm_target(tok));
            }
        }
        if recursive {
            for t in &targets {
                if matches!(t.as_str(), "/" | "/*" | "~" | "~/" | "~/*") {
                    return Some("recursive delete on root or home directory");
                }
                if sudo && matches!(t.as_str(), "." | "./" | "./*" | ".." | "../" | "../*") {
                    return Some("sudo recursive delete on current or parent directory");
                }
            }
        }
    }

    // ── Disk/partition destruction ────────────────────────────────
    if normalized.contains("mkfs")
        || normalized.contains("dd if=") && normalized.contains("of=/dev")
    {
        return Some("disk formatting or raw device write");
    }

    // ── Fork bombs ───────────────────────────────────────────────
    if normalized.contains(":(){ :|:& };:") || normalized.contains("./$0|./$0&") {
        return Some("fork bomb");
    }

    // ── /dev/sda or /dev/nvme direct writes ──────────────────────
    if (normalized.contains("> /dev/sd") || normalized.contains("> /dev/nvme"))
        && !normalized.contains("/dev/stderr")
        && !normalized.contains("/dev/stdout")
    {
        return Some("direct write to block device");
    }

    // ── chmod 777 on system dirs ─────────────────────────────────
    if normalized.contains("chmod")
        && normalized.contains("777")
        && normalized.contains("-r")
        && (normalized.contains(" /") && !normalized.contains(" /tmp"))
    {
        return Some("recursive chmod 777 on system directory");
    }

    // ── Overwrite system files ───────────────────────────────────
    if normalized.contains("> /etc/passwd")
        || normalized.contains("> /etc/shadow")
        || normalized.contains("> /etc/sudoers")
    {
        return Some("overwrite critical system file");
    }

    // ── Kernel/system destruction ────────────────────────────────
    if normalized.contains("echo") && normalized.contains("> /proc/") {
        return Some("write to /proc filesystem");
    }
    if normalized.contains("> /dev/null < /dev/sda")
        || normalized.contains("cat /dev/urandom > /dev/sd")
    {
        return Some("device destruction via /dev");
    }

    // ── Network exfiltration of sensitive files ──────────────────
    if (normalized.contains("curl") || normalized.contains("wget") || normalized.contains("nc "))
        && (normalized.contains("/etc/shadow")
            || normalized.contains("/etc/passwd")
            || normalized.contains("id_rsa")
            || normalized.contains(".ssh/"))
    {
        return Some("network exfiltration of sensitive files");
    }

    // ── Crypto mining / known malware patterns ───────────────────
    if normalized.contains("xmrig")
        || normalized.contains("minerd")
        || normalized.contains("cryptonight")
        || normalized.contains("stratum+tcp")
    {
        return Some("cryptocurrency mining");
    }

    // ── iptables flush (locks out remote access) ─────────────────
    if normalized.contains("iptables -f") && normalized.contains("drop") {
        return Some("firewall flush with default DROP (can lock out remote access)");
    }

    // ── Interpreter indirection ──────────────────────────────────
    // A literal scan can't see a blocked command hidden inside a string handed
    // to a shell (`bash -c '…'`, `echo '…' | sh`, `eval …`, base64-then-shell).
    // Pull those payloads back out and re-run the SAME gate on each.
    for payload in extract_interpreter_payloads(command) {
        if let Some(reason) = check_blocked_inner(&payload, depth + 1) {
            return Some(reason);
        }
    }

    None
}

/// Maximum number of recent bash commands tracked per session for the
/// retry-loop guard. Five is enough to catch tight back-to-back loops
/// (the actual symptom — same command twice in a row), small enough
/// that a legitimate retry after enough other work won't false-fire.
pub(crate) const RECENT_BASH_WINDOW: usize = 5;

#[derive(Debug, Clone)]
pub(crate) struct RecentBashOutcome {
    pub command: String,
    pub failed: bool,
    pub error_snippet: Option<String>,
}

/// Per-session ring buffer of the most recent bash commands and their
/// outcomes. In-memory only; cleared on process restart. Keyed by
/// `session_id` so different conversations never see each other's
/// retry history.
pub(crate) fn recent_bash_state() -> &'static RwLock<HashMap<Uuid, VecDeque<RecentBashOutcome>>> {
    static STATE: OnceLock<RwLock<HashMap<Uuid, VecDeque<RecentBashOutcome>>>> = OnceLock::new();
    STATE.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Layer 3 of the retry-loop guard: if the agent ran this exact
/// command in the last `RECENT_BASH_WINDOW` calls and it failed,
/// short-circuit with a stronger "you already tried this" message
/// that quotes the prior error. Returns `None` when there's no
/// matching prior failure (the command should be allowed to run).
pub(crate) fn check_recent_failure(session_id: Uuid, cmd: &str) -> Option<String> {
    let normalized = cmd.trim();
    let state = recent_bash_state().read().ok()?;
    let buf = state.get(&session_id)?;
    for prev in buf.iter() {
        if prev.failed && prev.command.trim() == normalized {
            let snippet = prev
                .error_snippet
                .as_deref()
                .unwrap_or("(no detail captured)");
            return Some(format!(
                "You already ran this exact command in the last {} bash calls and it failed. \
                 {}\n\nPrevious error:\n{}",
                RECENT_BASH_WINDOW,
                crate::brain::agent::service::nudge::variation_directive(),
                snippet
            ));
        }
    }
    None
}

/// Record the outcome of a bash call so a follow-up matching call can
/// be intercepted by `check_recent_failure`. Move-to-front semantics:
/// repeating the same command bumps the existing entry rather than
/// adding a duplicate, so the buffer always reflects the latest
/// outcome per unique command string.
pub(crate) fn record_bash_outcome(
    session_id: Uuid,
    command: String,
    failed: bool,
    error_snippet: Option<String>,
) {
    let Ok(mut state) = recent_bash_state().write() else {
        return;
    };
    let buf = state.entry(session_id).or_default();
    let normalized = command.trim().to_string();
    buf.retain(|o| o.command.trim() != normalized);
    buf.push_back(RecentBashOutcome {
        command,
        failed,
        error_snippet,
    });
    while buf.len() > RECENT_BASH_WINDOW {
        buf.pop_front();
    }
}

/// Detect commands that require an interactive TTY and return a useful
/// non-interactive alternative. Returns `None` when the command is fine
/// to run as-is, `Some(message)` when it should be rejected up-front.
///
/// The Apr 23 stdin-detach fix made the bash tool well-behaved (no more
/// keystroke theft from the TUI), but it also made interactive commands
/// fail *silently*: stdin=/dev/null returns EOF, the program exits with
/// code 0, output looks plausible, and the agent retries the same
/// command thinking it just needs another shot. Cut the loop on attempt
/// 1 with a clear message that names the alternative.
pub(crate) fn check_interactive_command(command: &str) -> Option<&'static str> {
    let normalized: String = command
        .split_whitespace()
        .collect::<Vec<&str>>()
        .join(" ")
        .to_lowercase();

    // Walk each segment of a chain (`a && b`, `a; b`, `a || b`) and
    // check the first token of each — protects against e.g. `cd foo && vim bar`.
    let segments = normalized
        .split([';', '|', '&'])
        .map(str::trim)
        .filter(|s| !s.is_empty());

    for seg in segments {
        // Skip leading env-var assignments (e.g. `EDITOR=vim git commit`)
        let cmd_start = seg
            .split_whitespace()
            .find(|tok| !tok.contains('='))
            .unwrap_or(seg);
        let cmd_seg = &seg[seg.find(cmd_start).unwrap_or(0)..];

        // `git add -p` / `--patch` / `-i` / `--interactive`
        if cmd_seg.starts_with("git add ")
            && (cmd_seg.contains(" -p")
                || cmd_seg.contains(" --patch")
                || cmd_seg.contains(" -i")
                || cmd_seg.contains(" --interactive"))
        {
            return Some(
                "`git add -p` / `git add -i` is interactive and won't work — stdin is /dev/null so it exits silently after printing the first hunk. \
                 Stage specific files with `git add <path>` (or `git add -A` for everything), or apply a precomputed patch with `git apply <patch>`. \
                 If you really need hunk-level granularity, use `git diff > /tmp/x.patch`, edit it, then `git apply --cached /tmp/x.patch`.",
            );
        }

        // `git rebase -i` / `--interactive`
        if cmd_seg.starts_with("git rebase ")
            && (cmd_seg.contains(" -i") || cmd_seg.contains(" --interactive"))
        {
            return Some(
                "`git rebase -i` is interactive and won't work — stdin is /dev/null. \
                 For squashing use `git reset --soft <base>` then `git commit -m`; \
                 for non-interactive autosquash run `GIT_SEQUENCE_EDITOR=: git rebase -i --autosquash <base>`.",
            );
        }

        // `git commit` with no message source — opens an editor.
        // Short-flag combos like `-am` / `-ma` count as a message
        // source; long form `--message=...` too.
        if cmd_seg.starts_with("git commit")
            && !cmd_seg.contains(" -m")
            && !cmd_seg.contains(" -am")
            && !cmd_seg.contains(" -ma")
            && !cmd_seg.contains(" --message")
            && !cmd_seg.contains(" -f ")
            && !cmd_seg.contains(" --file")
            && !cmd_seg.contains(" --no-edit")
            && !cmd_seg.contains(" -c ")
            && !cmd_seg.contains(" --reuse-message")
            && !cmd_seg.contains(" -c=")
            && !cmd_seg.contains(" --fixup")
            && !cmd_seg.contains(" --squash")
        {
            return Some(
                "`git commit` without a message opens an editor (interactive) — stdin is /dev/null. \
                 Pass `-m \"...\"` directly, or use `--no-edit` when amending.",
            );
        }

        // Standalone editors / pagers / TUI viewers — reject when they're
        // the actual command (not in the middle of a longer pipeline).
        let first_word = cmd_seg.split_whitespace().next().unwrap_or("");
        if matches!(
            first_word,
            "vim" | "vi" | "nvim" | "nano" | "emacs" | "pico" | "ed" | "joe" | "micro"
        ) {
            return Some(
                "Editors (vim/nvim/nano/emacs/...) need a TTY — they will hang on /dev/null stdin. \
                 Use the `edit_file` or `write_file` tool to modify a file, or pipe content via heredoc: `cat > file.txt <<'EOF' ... EOF`.",
            );
        }
        if matches!(first_word, "less" | "more" | "most" | "man") {
            return Some(
                "Pagers (less/more/man) need a TTY. Use `cat`, `head`, `tail`, or pipe to `cat` (e.g. `git log | cat`) so the output streams non-interactively.",
            );
        }
        if matches!(first_word, "top" | "htop" | "btop" | "atop" | "iotop") {
            return Some(
                "TUI process viewers (top/htop/btop) need a TTY. Use `ps aux` or `ps -ef` for a snapshot, or `ps aux --sort=-%cpu | head` for top consumers.",
            );
        }
        // REPLs without a script argument
        if matches!(
            first_word,
            "python" | "python3" | "node" | "irb" | "ghci" | "scala"
        ) && cmd_seg
            .split_whitespace()
            .nth(1)
            .map(|t| t.starts_with('-') && t != "-c" && t != "-e" && t != "-m")
            .unwrap_or(true)
        {
            return Some(
                "Bare REPLs (python/node/irb/ghci) hang on /dev/null stdin. \
                 Pass code via `-c \"...\"` (python) / `-e \"...\"` (node), or save to a file and run it.",
            );
        }
        // Database / Redis CLIs without a query
        if first_word == "psql" && !cmd_seg.contains(" -c ") && !cmd_seg.contains(" -f ") {
            return Some(
                "`psql` without `-c` or `-f` opens a REPL — pass `-c \"SQL\"` or `-f script.sql`.",
            );
        }
        if first_word == "mysql" && !cmd_seg.contains(" -e ") {
            return Some(
                "`mysql` without `-e` opens a REPL — pass `-e \"SQL\"` to run a single statement.",
            );
        }
        if first_word == "redis-cli" && cmd_seg.split_whitespace().count() == 1 {
            return Some(
                "`redis-cli` with no command opens a REPL — pass the command directly, e.g. `redis-cli GET key`.",
            );
        }

        // Interactive selectors / TUIs commonly invoked by mistake.
        if matches!(first_word, "fzf" | "gum" | "tmux" | "screen") {
            return Some(
                "Interactive TUI tools (fzf/gum/tmux/screen) need a TTY. They cannot be driven from this tool.",
            );
        }
    }

    None
}

/// Wrap `s` in POSIX single quotes so it survives /bin/sh expansion verbatim.
/// `'` inside the input becomes `'\''` (close, escaped quote, reopen).
pub(crate) fn posix_single_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for ch in s.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

/// Tempfile-backed `SSH_ASKPASS` script.
///
/// SSH consults `$SSH_ASKPASS` (with `SSH_ASKPASS_REQUIRE=force`) when it
/// can't open a TTY. The script we point at must be executable and emit
/// the password on stdout. We materialise both files in the process tempdir,
/// chmod 0700, and clean up on `Drop` so the secret never lingers.
struct SshAskpass {
    _password_file: tempfile::NamedTempFile,
    script_file: tempfile::NamedTempFile,
}

impl SshAskpass {
    fn new(password: &str) -> std::io::Result<Self> {
        use std::io::Write;

        // 1. Password file (mode 0600, owner-only).
        let mut pw_file = tempfile::Builder::new()
            .prefix("opencrabs-ssh-pw-")
            .tempfile()?;
        pw_file.write_all(password.as_bytes())?;
        pw_file.flush()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o600);
            std::fs::set_permissions(pw_file.path(), perms)?;
        }

        // 2. Askpass script (mode 0700, owner-only executable).
        // `cat` is universally available; the script just dumps the
        // password file. The path goes through POSIX single-quote escaping
        // so $TMPDIR containing quotes/$/backticks can't break out.
        let pw_path = pw_file.path().to_string_lossy().to_string();
        let script_body = format!("#!/bin/sh\nexec cat {}\n", posix_single_quote(&pw_path));
        let mut script_file = tempfile::Builder::new()
            .prefix("opencrabs-ssh-askpass-")
            .suffix(".sh")
            .tempfile()?;
        script_file.write_all(script_body.as_bytes())?;
        script_file.flush()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o700);
            std::fs::set_permissions(script_file.path(), perms)?;
        }

        Ok(Self {
            _password_file: pw_file,
            script_file,
        })
    }

    fn script_path(&self) -> &std::path::Path {
        self.script_file.path()
    }
}
