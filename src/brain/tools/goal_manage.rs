//! Goal Manage Tool — self-goaling (#307).
//!
//! Exposes the session goal system (`crate::brain::goal`) to the model itself,
//! not just the human-typed `/goal` slash command. With this the agent can set
//! an objective it works toward across turns: after each turn the post-turn
//! judge (`evaluate_after_turn`, wired unconditionally in the tool loop)
//! evaluates whether the goal is met and, if not, injects a continuation prompt
//! so the loop keeps going until the goal is satisfied or the turn budget runs
//! out. That lets the agent drive its own test / review / fix cycles.
//!
//! This is a pure wrapper over `GoalManager`; it stores no state (all goal
//! state lives in the `goal_state` table keyed by `session_id`) and needs no
//! changes to the judge — the judge activates on any active goal row regardless
//! of whether a tool or the slash command created it.

use super::error::Result;
use super::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
use async_trait::async_trait;
use serde_json::Value;

/// Tool letting the agent set and manage its own session goal.
pub struct GoalManageTool;

#[async_trait]
impl Tool for GoalManageTool {
    fn name(&self) -> &str {
        "goal_manage"
    }

    fn description(&self) -> &str {
        "Set and manage an autonomous goal for the current session so you can drive \
         multi-turn work yourself instead of waiting for the user to type /goal. A goal \
         is an objective you work toward across turns: after each turn a judge evaluates \
         whether it's met and, if not, you automatically continue (bounded by a turn \
         budget). Use 'set' with a concrete, checkable objective (e.g. 'all tests pass \
         and the fix is committed', 'the build is green') to start driving toward it, \
         'status' to check progress, 'pause'/'resume' to hold or continue, and 'clear' \
         to stop. Set a goal to run test/review/fix cycles autonomously."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["set", "status", "pause", "resume", "clear"],
                    "description": "Action to perform. 'set' starts a new goal (replaces any existing one), 'status' shows the current goal and progress, 'pause'/'resume' hold or continue it, 'clear' removes it."
                },
                "goal": {
                    "type": "string",
                    "description": "The objective to work toward. Required for 'set'. Make it concrete and checkable, e.g. 'all tests pass and changes committed', not a vague direction."
                },
                "max_turns": {
                    "type": "integer",
                    "description": "Optional maximum number of turns for this goal before auto-pausing. If omitted, uses agent.goal_max_turns from config, or 20 by default."
                }
            },
            "required": ["action"]
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![ToolCapability::SystemModification]
    }

    fn requires_approval_for_input(&self, _input: &Value) -> bool {
        // Goal state is session-local and bounded by the turn budget (no external
        // side effects), so managing it never needs approval.
        false
    }

    async fn execute(&self, input: Value, context: &ToolExecutionContext) -> Result<ToolResult> {
        let svc_ctx = match &context.service_context {
            Some(ctx) => ctx.clone(),
            None => {
                return Ok(ToolResult::error(
                    "Service context not available — cannot manage goals.".into(),
                ));
            }
        };

        let goal_mgr = crate::brain::goal::GoalManager::new(svc_ctx);
        let session_id = context.session_id;
        let action = input
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("status");

        match action {
            "set" => {
                let goal_text = input
                    .get("goal")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .unwrap_or("");
                if goal_text.is_empty() {
                    return Ok(ToolResult::error(
                        "The 'set' action requires a non-empty 'goal' describing what to \
                         work toward."
                            .into(),
                    ));
                }
                let max_turns = input
                    .get("max_turns")
                    .and_then(|v| v.as_u64())
                    .map(|v| v as u32);
                match goal_mgr
                    .set_goal(session_id, goal_text.to_string(), None, None, max_turns)
                    .await
                {
                    Ok(goal) => Ok(ToolResult::success(format!(
                        "🎯 Goal set (ID: {}):\n\n{}\n\nWorking toward this autonomously for up \
                         to {} turns. After each turn a judge checks whether it's met; if not, \
                         you continue automatically. Use action 'status' to check progress, \
                         'pause' to hold, 'clear' to stop.",
                        goal.id, goal.goal_text, goal.max_turns,
                    ))),
                    Err(e) => Ok(ToolResult::error(format!("Failed to set goal: {}", e))),
                }
            }
            "status" => match goal_mgr.get_goal(session_id).await {
                Ok(Some(goal)) => {
                    let elapsed = chrono::Utc::now()
                        .signed_duration_since(
                            chrono::DateTime::parse_from_rfc3339(&goal.created_at)
                                .unwrap_or_default(),
                        )
                        .num_minutes();
                    Ok(ToolResult::success(format!(
                        "🎯 Active goal ({}):\n\n{}\n\nState: {} | Turns: {}/{} | Elapsed: {}m",
                        goal.id,
                        goal.goal_text,
                        goal.state,
                        goal.turns_used,
                        goal.max_turns,
                        elapsed,
                    )))
                }
                Ok(None) => Ok(ToolResult::success(
                    "No active goal for this session. Use action 'set' with a goal to start \
                     driving toward one."
                        .into(),
                )),
                Err(e) => Ok(ToolResult::error(format!("Failed to get goal: {}", e))),
            },
            "pause" => match goal_mgr.pause_goal(session_id).await {
                Ok(()) => Ok(ToolResult::success("Goal paused.".into())),
                Err(e) => Ok(ToolResult::error(format!("Failed to pause goal: {}", e))),
            },
            "resume" => match goal_mgr.resume_goal(session_id).await {
                Ok(()) => Ok(ToolResult::success("Goal resumed.".into())),
                Err(e) => Ok(ToolResult::error(format!("Failed to resume goal: {}", e))),
            },
            "clear" => match goal_mgr.clear_goal(session_id).await {
                Ok(()) => Ok(ToolResult::success("Goal cleared.".into())),
                Err(e) => Ok(ToolResult::error(format!("Failed to clear goal: {}", e))),
            },
            unknown => Ok(ToolResult::error(format!(
                "Unknown action '{unknown}'. Valid: set, status, pause, resume, clear"
            ))),
        }
    }
}
