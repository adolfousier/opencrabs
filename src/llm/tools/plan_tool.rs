//! Plan Management Tool
//!
//! Allows the LLM to create, update, and manage structured plans for complex tasks.

use super::error::{Result, ToolError};
use super::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
use crate::tui::plan::{PlanDocument, PlanStatus, PlanTask, TaskType, ToolCall as PlanToolCall};
use async_trait::async_trait;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

/// Plan management tool
pub struct PlanTool;

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
enum PlanOperation {
    /// Create a new plan
    Create {
        title: String,
        description: String,
        #[serde(default)]
        context: String,
        #[serde(default)]
        risks: Vec<String>,
        #[serde(default)]
        test_strategy: String,
        #[serde(default)]
        technical_stack: Vec<String>,
    },
    /// Add a task to the current plan
    AddTask {
        title: String,
        description: String,
        task_type: String,
        #[serde(default)]
        dependencies: Vec<usize>, // Task order numbers
        #[serde(default = "default_complexity")]
        complexity: u8,
        #[serde(default)]
        acceptance_criteria: Vec<String>,
    },
    /// Update plan metadata
    UpdatePlan {
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        context: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        risks: Option<Vec<String>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        test_strategy: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        technical_stack: Option<Vec<String>>,
    },
    /// Mark plan as ready for review
    Finalize,
    /// Get current plan status
    Status,
    /// Get the next task to execute
    NextTask,
    /// Start executing a specific task
    StartTask { task_order: usize },
    /// Complete a task execution with results
    CompleteTask {
        task_order: usize,
        success: bool,
        output: String,
        #[serde(default)]
        artifacts: Vec<String>,
    },
    /// Add reflection notes after task execution
    Reflect {
        task_order: usize,
        reflection: String,
        #[serde(default)]
        should_retry: bool,
        #[serde(default)]
        adjustment_needed: Option<String>,
    },
    /// Record a tool call for the current task
    RecordToolCall {
        task_order: usize,
        tool_name: String,
        input: serde_json::Value,
        output: Option<String>,
        success: bool,
    },
    /// Skip a task with reason
    SkipTask { task_order: usize, reason: String },
    /// Get execution summary
    Summary,
}

fn default_complexity() -> u8 {
    3
}

/// Validate plan file path for security
/// Prevents symlink attacks and path traversal
fn validate_plan_file_path(path: &Path, base_dir: &Path) -> Result<()> {
    // Check if path is absolute and within the base directory
    if !path.starts_with(base_dir) {
        return Err(ToolError::InvalidInput(
            "Plan file must be within the session directory".to_string(),
        ));
    }

    // Check for symlinks (security risk)
    if path.exists() {
        let metadata = std::fs::symlink_metadata(path).map_err(ToolError::Io)?;
        if metadata.is_symlink() {
            return Err(ToolError::InvalidInput(
                "Plan file cannot be a symlink (security restriction)".to_string(),
            ));
        }
    }

    // Verify filename matches pattern .opencrabs_plan_{uuid}.json (no traversal)
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| ToolError::InvalidInput("Invalid plan filename".to_string()))?;

    if !file_name.starts_with(".opencrabs_plan_") || !file_name.ends_with(".json") {
        return Err(ToolError::InvalidInput(
            "Plan filename must match pattern .opencrabs_plan_{session_id}.json".to_string(),
        ));
    }

    // Extract and validate UUID portion
    let uuid_part = &file_name[16..file_name.len() - 5]; // Remove ".opencrabs_plan_" (16 chars) and ".json" (5 chars)
    uuid::Uuid::parse_str(uuid_part).map_err(|_| {
        ToolError::InvalidInput("Plan filename must contain a valid UUID".to_string())
    })?;

    Ok(())
}

/// Maximum plan file size (10MB)
const MAX_PLAN_FILE_SIZE: u64 = 10 * 1024 * 1024;

/// Input validation limits
const MAX_TITLE_LENGTH: usize = 200;
const MAX_DESCRIPTION_LENGTH: usize = 5000;
const MAX_CONTEXT_LENGTH: usize = 5000;

/// Validate string input
fn validate_string(s: &str, max_len: usize, field_name: &str) -> Result<()> {
    if s.is_empty() || s.trim().is_empty() {
        return Err(ToolError::InvalidInput(format!(
            "{} cannot be empty",
            field_name
        )));
    }

    if s.len() > max_len {
        return Err(ToolError::InvalidInput(format!(
            "{} exceeds maximum length of {} characters (got {})",
            field_name,
            max_len,
            s.len()
        )));
    }

    Ok(())
}

#[async_trait]
impl Tool for PlanTool {
    fn name(&self) -> &str {
        "plan"
    }

    fn description(&self) -> &str {
        "Manage structured task plans with full plan-and-execute capabilities. Create plans, add tasks, \
         execute them step-by-step, reflect on results, and adjust as needed. Supports dependency tracking, \
         execution history, and automatic retry logic."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "enum": ["create", "add_task", "update_plan", "finalize", "status", "next_task", "start_task", "complete_task", "reflect", "record_tool_call", "skip_task", "summary"],
                    "description": "Operation to perform: create/add_task/update_plan for planning, next_task/start_task/complete_task/reflect for execution, summary for status"
                },
                "title": {
                    "type": "string",
                    "description": "Plan or task title (for create/add_task)"
                },
                "description": {
                    "type": "string",
                    "description": "Plan or task description (for create/add_task/update_plan)"
                },
                "context": {
                    "type": "string",
                    "description": "Context and assumptions (for create/update_plan)"
                },
                "risks": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Identified risks and unknowns (for create/update_plan)"
                },
                "test_strategy": {
                    "type": "string",
                    "description": "Testing strategy and approach for the plan (for create/update_plan)"
                },
                "technical_stack": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Technologies, frameworks, and tools used (for create/update_plan)"
                },
                "task_type": {
                    "type": "string",
                    "enum": ["research", "edit", "create", "delete", "test", "refactor", "documentation", "configuration", "build"],
                    "description": "Type of task (for add_task)"
                },
                "dependencies": {
                    "type": "array",
                    "items": { "type": "integer" },
                    "description": "Task order numbers that must complete first (for add_task)"
                },
                "complexity": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 5,
                    "default": 3,
                    "description": "Task complexity from 1 (simple) to 5 (very complex)"
                },
                "acceptance_criteria": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Acceptance criteria for task completion (for add_task)"
                },
                "task_order": {
                    "type": "integer",
                    "minimum": 1,
                    "description": "Task number to operate on (for start_task/complete_task/reflect/skip_task)"
                },
                "success": {
                    "type": "boolean",
                    "description": "Whether the task execution was successful (for complete_task)"
                },
                "output": {
                    "type": "string",
                    "description": "Output/result of task execution (for complete_task)"
                },
                "artifacts": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "File paths or other artifacts produced (for complete_task)"
                },
                "reflection": {
                    "type": "string",
                    "description": "LLM reflection on task execution results (for reflect)"
                },
                "should_retry": {
                    "type": "boolean",
                    "description": "Whether to retry the task (for reflect)"
                },
                "adjustment_needed": {
                    "type": "string",
                    "description": "Description of plan adjustment needed (for reflect)"
                },
                "tool_name": {
                    "type": "string",
                    "description": "Name of tool that was called (for record_tool_call)"
                },
                "input": {
                    "type": "object",
                    "description": "Input passed to the tool (for record_tool_call)"
                },
                "reason": {
                    "type": "string",
                    "description": "Reason for skipping task (for skip_task)"
                }
            },
            "required": ["operation"]
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![ToolCapability::PlanManagement]
    }

    fn requires_approval(&self) -> bool {
        false // Plan management doesn't require approval, only viewing/creation
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        let _: PlanOperation = serde_json::from_value(input.clone())
            .map_err(|e| ToolError::InvalidInput(format!("Invalid input: {}", e)))?;
        Ok(())
    }

    async fn execute(&self, input: Value, context: &ToolExecutionContext) -> Result<ToolResult> {
        let operation: PlanOperation = serde_json::from_value(input)?;

        // Load or create plan state from context (session-scoped)
        let session_dir = crate::config::opencrabs_home()
            .join("agents")
            .join("session");
        let _ = std::fs::create_dir_all(&session_dir);
        let plan_filename = format!(".opencrabs_plan_{}.json", context.session_id);
        let plan_file = session_dir.join(&plan_filename);

        // Security: Validate plan file path
        validate_plan_file_path(&plan_file, &session_dir)?;

        // Load existing plan with security checks
        let mut plan: Option<PlanDocument> = if plan_file.exists() {
            // Security: Check file size before reading
            let metadata = tokio::fs::metadata(&plan_file)
                .await
                .map_err(ToolError::Io)?;

            if metadata.len() > MAX_PLAN_FILE_SIZE {
                return Err(ToolError::InvalidInput(format!(
                    "Plan file too large: {} bytes (max: {} bytes)",
                    metadata.len(),
                    MAX_PLAN_FILE_SIZE
                )));
            }

            let content = tokio::fs::read_to_string(&plan_file)
                .await
                .map_err(ToolError::Io)?;

            Some(serde_json::from_str(&content).map_err(|e| {
                ToolError::InvalidInput(format!("Failed to parse plan file: {}", e))
            })?)
        } else {
            None
        };

        let result = match operation {
            PlanOperation::Create {
                title,
                description,
                context: ctx,
                risks,
                test_strategy,
                technical_stack,
            } => {
                // Validate inputs
                validate_string(&title, MAX_TITLE_LENGTH, "Plan title")?;
                validate_string(&description, MAX_DESCRIPTION_LENGTH, "Plan description")?;
                if !ctx.is_empty() {
                    validate_string(&ctx, MAX_CONTEXT_LENGTH, "Plan context")?;
                }

                // Check if plan exists
                if let Some(existing_plan) = plan.as_ref() {
                    // Allow replacing empty Draft plans (likely stale/abandoned)
                    if existing_plan.status == PlanStatus::Draft && existing_plan.tasks.is_empty() {
                        tracing::info!(
                            "📝 Replacing empty Draft plan '{}' with new plan '{}'",
                            existing_plan.title,
                            title
                        );
                        // Continue to create new plan (will replace existing)
                    } else {
                        // Don't allow replacing plans with tasks or in other states
                        return Ok(ToolResult::error(format!(
                            "A plan already exists: '{}' ({:?}, {} tasks). Use 'update_plan' to modify it or 'finalize' to complete it.",
                            existing_plan.title,
                            existing_plan.status,
                            existing_plan.tasks.len()
                        )));
                    }
                }

                let mut new_plan =
                    PlanDocument::new(context.session_id, title.clone(), description);
                new_plan.context = ctx;
                new_plan.risks = risks;
                new_plan.test_strategy = test_strategy;
                new_plan.technical_stack = technical_stack;
                new_plan.status = PlanStatus::Draft;

                plan = Some(new_plan.clone());

                format!(
                    "✓ Created new plan: '{}'\n\nNext steps:\n\
                     1. Use 'add_task' to add tasks to the plan\n\
                     2. Use 'finalize' when ready for user review",
                    title
                )
            }

            PlanOperation::AddTask {
                title,
                description,
                task_type,
                dependencies,
                complexity,
                acceptance_criteria,
            } => {
                // Validate inputs
                validate_string(&title, MAX_TITLE_LENGTH, "Task title")?;
                validate_string(&description, MAX_DESCRIPTION_LENGTH, "Task description")?;

                let current_plan = plan.as_mut().ok_or_else(|| {
                    ToolError::InvalidInput(
                        "No active plan. Create a plan first with 'create' operation.".to_string(),
                    )
                })?;

                // Parse task type
                let parsed_type = match task_type.to_lowercase().as_str() {
                    "research" => TaskType::Research,
                    "edit" => TaskType::Edit,
                    "create" => TaskType::Create,
                    "delete" => TaskType::Delete,
                    "test" => TaskType::Test,
                    "refactor" => TaskType::Refactor,
                    "documentation" => TaskType::Documentation,
                    "configuration" => TaskType::Configuration,
                    "build" => TaskType::Build,
                    other => TaskType::Other(other.to_string()),
                };

                let task_order = current_plan.tasks.len() + 1;
                let mut task =
                    PlanTask::new(task_order, title.clone(), description, parsed_type.clone());
                task.complexity = complexity.clamp(1, 5);
                task.acceptance_criteria = acceptance_criteria;

                // Validate and convert dependency order numbers to task IDs
                for dep_order in dependencies {
                    if dep_order == 0 {
                        return Err(ToolError::InvalidInput(
                            "Task numbers start at 1, not 0".to_string(),
                        ));
                    }
                    if dep_order >= task_order {
                        return Err(ToolError::InvalidInput(format!(
                            "Task {} cannot depend on task {} (not yet created or would create a cycle)",
                            task_order, dep_order
                        )));
                    }

                    let dep_task = current_plan.tasks.get(dep_order - 1).ok_or_else(|| {
                        ToolError::InvalidInput(format!(
                            "Invalid dependency: task {} does not exist",
                            dep_order
                        ))
                    })?;

                    task.dependencies.push(dep_task.id);
                }

                current_plan.add_task(task);

                format!(
                    "✓ Added task #{}: '{}'\n  Type: {:?} | Complexity: {}★\n  Total tasks: {}",
                    task_order,
                    title,
                    parsed_type,
                    complexity,
                    current_plan.tasks.len()
                )
            }

            PlanOperation::UpdatePlan {
                title,
                description,
                context: ctx,
                risks,
                test_strategy,
                technical_stack,
            } => {
                let current_plan = plan.as_mut().ok_or_else(|| {
                    ToolError::InvalidInput("No active plan to update.".to_string())
                })?;

                if let Some(t) = title {
                    current_plan.title = t;
                }
                if let Some(d) = description {
                    current_plan.description = d;
                }
                if let Some(c) = ctx {
                    current_plan.context = c;
                }
                if let Some(r) = risks {
                    current_plan.risks = r;
                }
                if let Some(ts) = test_strategy {
                    current_plan.test_strategy = ts;
                }
                if let Some(stack) = technical_stack {
                    current_plan.technical_stack = stack;
                }
                current_plan.updated_at = Utc::now();

                "✓ Plan updated successfully".to_string()
            }

            PlanOperation::Finalize => {
                tracing::info!("🔧 Finalize operation starting...");

                let current_plan = plan.as_mut().ok_or_else(|| {
                    tracing::error!("❌ Finalize failed: No active plan");
                    ToolError::InvalidInput("No active plan to finalize.".to_string())
                })?;

                if current_plan.tasks.is_empty() {
                    tracing::warn!("⚠️ Cannot finalize: Plan has no tasks");
                    return Ok(ToolResult::error(
                        "Cannot finalize plan with no tasks. Add tasks first.".to_string(),
                    ));
                }

                tracing::debug!(
                    "📋 Finalizing plan: title='{}', tasks={}, status={:?}",
                    current_plan.title,
                    current_plan.tasks.len(),
                    current_plan.status
                );

                // Validate dependencies before finalizing
                if let Err(e) = current_plan.validate_dependencies() {
                    tracing::error!("❌ Dependency validation failed: {}", e);
                    return Ok(ToolResult::error(format!(
                        "Cannot finalize plan: {}\n\n\
                         Please fix the dependency issues before finalizing.",
                        e
                    )));
                }

                // Get validation warnings
                let warnings = current_plan.get_validation_warnings();
                let warning_text = if !warnings.is_empty() {
                    let warning_list = warnings
                        .iter()
                        .map(|w| format!("  {}", w))
                        .collect::<Vec<_>>()
                        .join("\n");
                    format!("\n\n📊 Plan Quality Notes:\n{}\n", warning_list)
                } else {
                    String::new()
                };

                // Change status
                let old_status = current_plan.status.clone();
                current_plan.status = PlanStatus::PendingApproval;
                current_plan.updated_at = Utc::now();

                tracing::info!(
                    "✅ Plan status changed: {:?} → {:?}",
                    old_status,
                    current_plan.status
                );

                format!(
                    "✓ Plan finalized and ready for review!\n\n\
                     📋 Plan: {}\n\
                     📝 {} tasks ready for execution{}\n\
                     Press Ctrl+P to review the plan.",
                    current_plan.title,
                    current_plan.tasks.len(),
                    warning_text
                )
            }

            PlanOperation::Status => {
                if let Some(current_plan) = &plan {
                    format!(
                        "📋 Current Plan Status\n\n\
                         Title: {}\n\
                         Status: {:?}\n\
                         Tasks: {}\n\
                         Created: {}\n\
                         Updated: {}",
                        current_plan.title,
                        current_plan.status,
                        current_plan.tasks.len(),
                        current_plan.created_at.format("%Y-%m-%d %H:%M:%S"),
                        current_plan.updated_at.format("%Y-%m-%d %H:%M:%S")
                    )
                } else {
                    "No active plan. Create one with 'create' operation.".to_string()
                }
            }

            PlanOperation::NextTask => {
                let current_plan = plan
                    .as_ref()
                    .ok_or_else(|| ToolError::InvalidInput("No active plan.".to_string()))?;

                if let Some(next_task) = current_plan.next_executable_task() {
                    format!(
                        "🎯 Next Task to Execute\n\n\
                         Task #{}: {}\n\
                         Type: {:?}\n\
                         Complexity: {}\n\
                         Description: {}\n\n\
                         Acceptance Criteria:\n{}\n\n\
                         Use 'start_task' with task_order={} to begin execution.",
                        next_task.order,
                        next_task.title,
                        next_task.task_type,
                        next_task.complexity_stars(),
                        next_task.description,
                        next_task
                            .acceptance_criteria
                            .iter()
                            .map(|c| format!("  • {}", c))
                            .collect::<Vec<_>>()
                            .join("\n"),
                        next_task.order
                    )
                } else {
                    let summary = current_plan.execution_summary();
                    if summary.pending == 0 && summary.in_progress == 0 {
                        "✅ All tasks completed! No more tasks to execute.".to_string()
                    } else if summary.in_progress > 0 {
                        format!(
                            "⏳ {} task(s) currently in progress. Complete them before starting new ones.",
                            summary.in_progress
                        )
                    } else {
                        "⚠️ No tasks ready. Check for blocked dependencies or failed tasks."
                            .to_string()
                    }
                }
            }

            PlanOperation::StartTask { task_order } => {
                let current_plan = plan
                    .as_mut()
                    .ok_or_else(|| ToolError::InvalidInput("No active plan.".to_string()))?;

                // Check if task exists and get its status
                let task_status = current_plan
                    .get_task_by_order(task_order)
                    .ok_or_else(|| {
                        ToolError::InvalidInput(format!("Task #{} not found.", task_order))
                    })?
                    .status
                    .clone();

                if !matches!(task_status, crate::tui::plan::TaskStatus::Pending) {
                    return Ok(ToolResult::error(format!(
                        "Task #{} is not pending (current status: {:?})",
                        task_order, task_status
                    )));
                }

                // Check dependencies
                let deps_satisfied = current_plan
                    .get_task_by_order(task_order)
                    .map(|t| current_plan.dependencies_satisfied(t))
                    .unwrap_or(false);

                if !deps_satisfied {
                    return Ok(ToolResult::error(format!(
                        "Cannot start task #{}: dependencies not satisfied.",
                        task_order
                    )));
                }

                // Now get mutable reference and update (verified to exist above)
                let task = current_plan.get_task_by_order_mut(task_order)
                    .ok_or_else(|| ToolError::InvalidInput(format!("Task #{} not found.", task_order)))?;
                task.start_execution();
                let task_title = task.title.clone();

                current_plan.status = PlanStatus::InProgress;

                format!(
                    "▶️ Started Task #{}: {}\n\n\
                     Now execute the task by:\n\
                     1. Using appropriate tools (read_file, write_file, bash, etc.)\n\
                     2. Recording tool calls with 'record_tool_call'\n\
                     3. Completing with 'complete_task' when done\n\
                     4. Reflecting on results with 'reflect'",
                    task_order, task_title
                )
            }

            PlanOperation::CompleteTask {
                task_order,
                success,
                output,
                artifacts,
            } => {
                let current_plan = plan
                    .as_mut()
                    .ok_or_else(|| ToolError::InvalidInput("No active plan.".to_string()))?;

                let task = current_plan
                    .get_task_by_order_mut(task_order)
                    .ok_or_else(|| {
                        ToolError::InvalidInput(format!("Task #{} not found.", task_order))
                    })?;

                for artifact in artifacts {
                    task.add_artifact(artifact);
                }

                task.complete_execution(output.clone(), success);

                let status_msg = if success {
                    format!(
                        "✅ Task #{} completed successfully!\n\nOutput: {}\n\n\
                         Next: Use 'reflect' to analyze the results, then 'next_task' to continue.",
                        task_order, output
                    )
                } else {
                    let can_retry = task.can_retry();
                    format!(
                        "❌ Task #{} failed (attempt {}/{})\n\nOutput: {}\n\n{}",
                        task_order,
                        task.retry_count,
                        task.max_retries,
                        output,
                        if can_retry {
                            "Next: Use 'reflect' to analyze what went wrong, then retry if appropriate."
                        } else {
                            "Max retries reached. Use 'reflect' to document the failure."
                        }
                    )
                };

                // Check if all tasks are complete
                if current_plan.is_complete() {
                    current_plan.complete();
                }

                status_msg
            }

            PlanOperation::Reflect {
                task_order,
                reflection,
                should_retry,
                adjustment_needed,
            } => {
                let current_plan = plan
                    .as_mut()
                    .ok_or_else(|| ToolError::InvalidInput("No active plan.".to_string()))?;

                let task = current_plan
                    .get_task_by_order_mut(task_order)
                    .ok_or_else(|| {
                        ToolError::InvalidInput(format!("Task #{} not found.", task_order))
                    })?;

                task.add_reflection(reflection.clone());

                let mut response = format!(
                    "🤔 Reflection recorded for Task #{}:\n\n{}\n\n",
                    task_order, reflection
                );

                if should_retry && task.can_retry() {
                    // Reset to pending for retry
                    task.status = crate::tui::plan::TaskStatus::Pending;
                    response.push_str("🔄 Task marked for retry. Use 'start_task' to retry.\n");
                }

                if let Some(adjustment) = adjustment_needed {
                    response.push_str(&format!(
                        "⚙️ Plan adjustment needed: {}\n\
                         Consider using 'add_task' to add corrective tasks or 'update_plan' to revise the plan.",
                        adjustment
                    ));
                }

                response
            }

            PlanOperation::RecordToolCall {
                task_order,
                tool_name,
                input,
                output,
                success,
            } => {
                let current_plan = plan
                    .as_mut()
                    .ok_or_else(|| ToolError::InvalidInput("No active plan.".to_string()))?;

                let task = current_plan
                    .get_task_by_order_mut(task_order)
                    .ok_or_else(|| {
                        ToolError::InvalidInput(format!("Task #{} not found.", task_order))
                    })?;

                let tool_call = PlanToolCall {
                    tool_name: tool_name.clone(),
                    input,
                    output: output.clone(),
                    success,
                    timestamp: Utc::now(),
                };

                task.record_tool_call(tool_call);

                format!(
                    "📝 Recorded tool call: {} ({})",
                    tool_name,
                    if success { "success" } else { "failed" }
                )
            }

            PlanOperation::SkipTask { task_order, reason } => {
                let current_plan = plan
                    .as_mut()
                    .ok_or_else(|| ToolError::InvalidInput("No active plan.".to_string()))?;

                let task = current_plan
                    .get_task_by_order_mut(task_order)
                    .ok_or_else(|| {
                        ToolError::InvalidInput(format!("Task #{} not found.", task_order))
                    })?;

                task.skip(Some(reason.clone()));

                format!(
                    "⏭️ Skipped Task #{}: {}\nReason: {}",
                    task_order, task.title, reason
                )
            }

            PlanOperation::Summary => {
                let current_plan = plan
                    .as_ref()
                    .ok_or_else(|| ToolError::InvalidInput("No active plan.".to_string()))?;

                let summary = current_plan.execution_summary();

                format!(
                    "📊 Execution Summary\n\n\
                     Plan: {}\n\
                     Status: {:?}\n\n\
                     Tasks: {} total\n\
                     ✅ Completed: {}\n\
                     ❌ Failed: {}\n\
                     ▶️ In Progress: {}\n\
                     ⏸️ Pending: {}\n\
                     ⏭️ Skipped: {}\n\
                     🚫 Blocked: {}\n\n\
                     Progress: {:.1}%\n\
                     Success Rate: {:.1}%\n\
                     Total Retries: {}\n\
                     Total Tool Calls: {}",
                    current_plan.title,
                    current_plan.status,
                    summary.total_tasks,
                    summary.completed,
                    summary.failed,
                    summary.in_progress,
                    summary.pending,
                    summary.skipped,
                    summary.blocked,
                    current_plan.progress_percentage(),
                    summary.success_rate,
                    summary.total_retries,
                    summary.total_tool_calls
                )
            }
        };

        // Save plan to file with atomic write
        if let Some(ref current_plan) = plan {
            let json = serde_json::to_string_pretty(current_plan)
                .map_err(|e| ToolError::InvalidInput(format!("Failed to serialize plan: {}", e)))?;

            // Atomic write: write to temp file, then rename
            let temp_file = plan_file.with_extension("tmp");

            // Write to temp file
            tokio::fs::write(&temp_file, &json)
                .await
                .map_err(ToolError::Io)?;

            // Atomic rename (ensures consistency even if interrupted)
            tokio::fs::rename(&temp_file, &plan_file)
                .await
                .map_err(ToolError::Io)?;

            tracing::info!(
                "💾 Plan saved to file: {} (status: {:?})",
                plan_file.display(),
                current_plan.status
            );

            // Verify file was written correctly
            if plan_file.exists() {
                match tokio::fs::read_to_string(&plan_file).await {
                    Ok(content) => match serde_json::from_str::<PlanDocument>(&content) {
                        Ok(saved_plan) => {
                            tracing::debug!(
                                "✅ Verified saved plan: status={:?}, tasks={}",
                                saved_plan.status,
                                saved_plan.tasks.len()
                            );

                            if saved_plan.status != current_plan.status {
                                tracing::error!(
                                    "❌ Status mismatch! Expected {:?}, got {:?}",
                                    current_plan.status,
                                    saved_plan.status
                                );
                            }
                        }
                        Err(e) => {
                            tracing::error!("❌ Failed to parse saved plan: {}", e);
                        }
                    },
                    Err(e) => {
                        tracing::error!("❌ Failed to read saved plan: {}", e);
                    }
                }
            } else {
                tracing::error!("❌ Plan file does not exist after save!");
            }
        }

        Ok(ToolResult::success(result))
    }
}

#[cfg(test)]
#[path = "plan_tool_security_tests.rs"]
mod plan_tool_security_tests;
