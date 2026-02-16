//! Task Management Tool
//!
//! Organize and track multi-step workflows and tasks.

use super::error::{Result, ToolError};
use super::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::fs;
use uuid::Uuid;

/// Task management tool
pub struct TaskTool;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
enum TaskStatus {
    Pending,
    InProgress,
    Completed,
    Blocked,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
enum TaskPriority {
    Low,
    Medium,
    High,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Task {
    id: String,
    description: String,
    status: TaskStatus,
    priority: TaskPriority,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    completed_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    blocked_reason: Option<String>,
    #[serde(default)]
    dependencies: Vec<String>,
    #[serde(default)]
    tags: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct TaskStore {
    tasks: HashMap<String, Task>,
}

/// File lock guard that releases the lock when dropped
struct FileLock {
    lock_path: PathBuf,
}

impl FileLock {
    /// Acquire an exclusive lock on the task store file
    async fn acquire(store_path: &Path) -> Result<Self> {
        let lock_path = store_path.with_extension("lock");

        // Ensure parent directory exists
        if let Some(parent) = lock_path.parent() {
            fs::create_dir_all(parent).await.map_err(ToolError::Io)?;
        }

        // Try to acquire lock with retries and exponential backoff
        let max_attempts = 10;
        let mut attempt = 0;
        let mut delay = Duration::from_millis(50);

        loop {
            attempt += 1;

            // Try to create lock file exclusively (fails if exists)
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&lock_path)
                .await
            {
                Ok(mut file) => {
                    // Write PID and timestamp to lock file for debugging
                    use tokio::io::AsyncWriteExt;
                    let lock_info = format!(
                        "pid: {}\ntimestamp: {}\n",
                        std::process::id(),
                        Utc::now().to_rfc3339()
                    );
                    let _ = file.write_all(lock_info.as_bytes()).await;
                    return Ok(Self { lock_path });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    if attempt >= max_attempts {
                        // Check if lock is stale (older than 60 seconds)
                        if let Ok(metadata) = fs::metadata(&lock_path).await
                            && let Ok(modified) = metadata.modified() {
                                let age = std::time::SystemTime::now()
                                    .duration_since(modified)
                                    .unwrap_or_default();
                                if age > Duration::from_secs(60) {
                                    // Stale lock, force remove it
                                    tracing::warn!("Removing stale lock file (age: {:?})", age);
                                    let _ = fs::remove_file(&lock_path).await;
                                    continue;
                                }
                            }

                        return Err(ToolError::Execution(format!(
                            "Failed to acquire lock after {} attempts. \
                             Another process may be using the task store.",
                            max_attempts
                        )));
                    }

                    // Wait before retrying
                    tokio::time::sleep(delay).await;
                    delay = (delay * 2).min(Duration::from_secs(2));
                }
                Err(e) => {
                    return Err(ToolError::Io(e));
                }
            }
        }
    }

    /// Release the lock (called automatically on drop)
    async fn release(&self) -> Result<()> {
        fs::remove_file(&self.lock_path)
            .await
            .map_err(ToolError::Io)
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        // Best-effort synchronous cleanup on drop
        // This handles cases where the lock isn't explicitly released
        let _ = std::fs::remove_file(&self.lock_path);
    }
}

impl TaskStore {
    fn new() -> Self {
        Self {
            tasks: HashMap::new(),
        }
    }

    async fn load(path: &Path) -> Result<Self> {
        if path.exists() {
            let content = fs::read_to_string(path).await.map_err(ToolError::Io)?;
            serde_json::from_str(&content)
                .map_err(|e| ToolError::Execution(format!("Failed to parse task store: {}", e)))
        } else {
            Ok(Self::new())
        }
    }

    async fn save(&self, path: &Path) -> Result<()> {
        let content = serde_json::to_string_pretty(self)
            .map_err(|e| ToolError::Execution(format!("Failed to serialize tasks: {}", e)))?;

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await.map_err(ToolError::Io)?;
        }

        // Atomic write: write to temp file, then rename
        let temp_path = path.with_extension("tmp");
        fs::write(&temp_path, content)
            .await
            .map_err(ToolError::Io)?;
        fs::rename(&temp_path, path).await.map_err(ToolError::Io)?;

        Ok(())
    }

    /// Load, modify, and save with file locking to prevent race conditions
    async fn with_lock<F, T>(path: &Path, operation: F) -> Result<T>
    where
        F: FnOnce(&mut Self) -> Result<T>,
    {
        // Acquire exclusive lock
        let lock = FileLock::acquire(path).await?;

        // Load current state
        let mut store = Self::load(path).await?;

        // Perform operation
        let result = operation(&mut store)?;

        // Save updated state
        store.save(path).await?;

        // Release lock explicitly (also released on drop)
        let _ = lock.release().await;

        Ok(result)
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "operation")]
enum TaskOperation {
    #[serde(rename = "create")]
    Create {
        description: String,
        #[serde(default)]
        priority: Option<String>,
        #[serde(default)]
        tags: Vec<String>,
        #[serde(default)]
        dependencies: Vec<String>,
    },

    #[serde(rename = "update")]
    Update {
        task_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        status: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        priority: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        blocked_reason: Option<String>,
    },

    #[serde(rename = "list")]
    List {
        #[serde(skip_serializing_if = "Option::is_none")]
        status: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        priority: Option<String>,
        #[serde(default)]
        show_completed: bool,
    },

    #[serde(rename = "delete")]
    Delete { task_id: String },

    #[serde(rename = "get")]
    Get { task_id: String },

    #[serde(rename = "clear_completed")]
    ClearCompleted,
}

#[derive(Debug, Deserialize, Serialize)]
struct TaskInput {
    #[serde(flatten)]
    operation: TaskOperation,
}

fn parse_priority(priority_str: &str) -> Result<TaskPriority> {
    match priority_str.to_lowercase().as_str() {
        "low" => Ok(TaskPriority::Low),
        "medium" => Ok(TaskPriority::Medium),
        "high" => Ok(TaskPriority::High),
        "critical" => Ok(TaskPriority::Critical),
        _ => Err(ToolError::InvalidInput(format!(
            "Invalid priority: {}. Must be low, medium, high, or critical",
            priority_str
        ))),
    }
}

fn parse_status(status_str: &str) -> Result<TaskStatus> {
    match status_str.to_lowercase().as_str() {
        "pending" => Ok(TaskStatus::Pending),
        "in_progress" | "inprogress" => Ok(TaskStatus::InProgress),
        "completed" => Ok(TaskStatus::Completed),
        "blocked" => Ok(TaskStatus::Blocked),
        "cancelled" => Ok(TaskStatus::Cancelled),
        _ => Err(ToolError::InvalidInput(format!(
            "Invalid status: {}. Must be pending, in_progress, completed, blocked, or cancelled",
            status_str
        ))),
    }
}

fn get_store_path(context: &ToolExecutionContext) -> PathBuf {
    let dir = crate::config::opencrabs_home()
        .join("agents")
        .join("session");
    let _ = std::fs::create_dir_all(&dir);
    dir.join(format!("tasks_{}.json", context.session_id))
}

#[async_trait]
impl Tool for TaskTool {
    fn name(&self) -> &str {
        "task_manager"
    }

    fn description(&self) -> &str {
        "Manage multi-step workflows and tasks. Create, update, list, and track tasks with priorities, statuses, and dependencies."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "description": "Operation to perform",
                    "enum": ["create", "update", "list", "delete", "get", "clear_completed"]
                },
                "description": {
                    "type": "string",
                    "description": "Task description (for create operation)"
                },
                "task_id": {
                    "type": "string",
                    "description": "Task ID (for update, delete, get operations)"
                },
                "status": {
                    "type": "string",
                    "description": "Task status",
                    "enum": ["pending", "in_progress", "completed", "blocked", "cancelled"]
                },
                "priority": {
                    "type": "string",
                    "description": "Task priority",
                    "enum": ["low", "medium", "high", "critical"]
                },
                "blocked_reason": {
                    "type": "string",
                    "description": "Reason why task is blocked (when setting status to blocked)"
                },
                "tags": {
                    "type": "array",
                    "description": "Tags for categorizing tasks",
                    "items": {
                        "type": "string"
                    },
                    "default": []
                },
                "dependencies": {
                    "type": "array",
                    "description": "List of task IDs this task depends on",
                    "items": {
                        "type": "string"
                    },
                    "default": []
                },
                "show_completed": {
                    "type": "boolean",
                    "description": "Include completed tasks in list (default: false)",
                    "default": false
                }
            },
            "required": ["operation"]
        })
    }

    fn capabilities(&self) -> Vec<ToolCapability> {
        vec![ToolCapability::ReadFiles, ToolCapability::WriteFiles]
    }

    fn requires_approval(&self) -> bool {
        false // Task management is safe
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        let _: TaskInput = serde_json::from_value(input.clone())
            .map_err(|e| ToolError::InvalidInput(format!("Invalid input: {}", e)))?;
        Ok(())
    }

    async fn execute(&self, input: Value, context: &ToolExecutionContext) -> Result<ToolResult> {
        let input: TaskInput = serde_json::from_value(input)?;
        let store_path = get_store_path(context);

        // Read-only operations don't need locking
        let result = match input.operation {
            TaskOperation::List {
                status,
                priority,
                show_completed,
            } => {
                // Read-only: acquire lock briefly just for reading
                let lock = FileLock::acquire(&store_path).await?;
                let store = TaskStore::load(&store_path).await?;
                let _ = lock.release().await;

                let mut filtered_tasks: Vec<_> = store
                    .tasks
                    .values()
                    .filter(|t| {
                        if !show_completed && t.status == TaskStatus::Completed {
                            return false;
                        }
                        if let Some(ref s) = status
                            && let Ok(target_status) = parse_status(s)
                                && t.status != target_status {
                                    return false;
                                }
                        if let Some(ref p) = priority
                            && let Ok(target_priority) = parse_priority(p)
                                && t.priority != target_priority {
                                    return false;
                                }
                        true
                    })
                    .collect();

                if filtered_tasks.is_empty() {
                    return Ok(ToolResult::success("No tasks found".to_string()));
                }

                // Sort by priority (Critical > High > Medium > Low) then by created_at
                filtered_tasks.sort_by(|a, b| {
                    let priority_order = |p: &TaskPriority| match p {
                        TaskPriority::Critical => 0,
                        TaskPriority::High => 1,
                        TaskPriority::Medium => 2,
                        TaskPriority::Low => 3,
                    };
                    priority_order(&a.priority)
                        .cmp(&priority_order(&b.priority))
                        .then_with(|| a.created_at.cmp(&b.created_at))
                });

                let mut output = format!("Found {} tasks:\n\n", filtered_tasks.len());
                for task in filtered_tasks {
                    output.push_str(&format!(
                        "[{}] {:?} | {:?}\n",
                        &task.id[..8],
                        task.status,
                        task.priority
                    ));
                    output.push_str(&format!("    {}\n", task.description));
                    if !task.tags.is_empty() {
                        output.push_str(&format!("    Tags: {}\n", task.tags.join(", ")));
                    }
                    if !task.dependencies.is_empty() {
                        output.push_str(&format!(
                            "    Dependencies: {}\n",
                            task.dependencies
                                .iter()
                                .map(|id| &id[..8])
                                .collect::<Vec<_>>()
                                .join(", ")
                        ));
                    }
                    if let Some(reason) = &task.blocked_reason {
                        output.push_str(&format!("    Blocked: {}\n", reason));
                    }
                    output.push('\n');
                }

                output
            }

            TaskOperation::Get { task_id } => {
                // Read-only: acquire lock briefly just for reading
                let lock = FileLock::acquire(&store_path).await?;
                let store = TaskStore::load(&store_path).await?;
                let _ = lock.release().await;

                let task = store.tasks.get(&task_id).ok_or_else(|| {
                    ToolError::InvalidInput(format!("Task not found: {}", task_id))
                })?;

                let mut output = format!("Task: {}\n", task.id);
                output.push_str(&format!("Description: {}\n", task.description));
                output.push_str(&format!("Status: {:?}\n", task.status));
                output.push_str(&format!("Priority: {:?}\n", task.priority));
                output.push_str(&format!(
                    "Created: {}\n",
                    task.created_at.format("%Y-%m-%d %H:%M:%S")
                ));
                output.push_str(&format!(
                    "Updated: {}\n",
                    task.updated_at.format("%Y-%m-%d %H:%M:%S")
                ));

                if let Some(completed) = task.completed_at {
                    output.push_str(&format!(
                        "Completed: {}\n",
                        completed.format("%Y-%m-%d %H:%M:%S")
                    ));
                }

                if !task.tags.is_empty() {
                    output.push_str(&format!("Tags: {}\n", task.tags.join(", ")));
                }

                if !task.dependencies.is_empty() {
                    output.push_str(&format!("Dependencies: {}\n", task.dependencies.join(", ")));
                }

                if let Some(reason) = &task.blocked_reason {
                    output.push_str(&format!("Blocked Reason: {}\n", reason));
                }

                output
            }

            // Write operations use atomic locking
            TaskOperation::Create {
                description,
                priority,
                tags,
                dependencies,
            } => {
                TaskStore::with_lock(&store_path, |store| {
                    let task_priority = if let Some(p) = priority {
                        parse_priority(&p)?
                    } else {
                        TaskPriority::Medium
                    };

                    // Check dependencies exist
                    for dep_id in &dependencies {
                        if !store.tasks.contains_key(dep_id) {
                            return Err(ToolError::InvalidInput(format!(
                                "Dependency task not found: {}",
                                dep_id
                            )));
                        }
                    }

                    let task_id = Uuid::new_v4().to_string();
                    let task = Task {
                        id: task_id.clone(),
                        description: description.clone(),
                        status: TaskStatus::Pending,
                        priority: task_priority,
                        created_at: Utc::now(),
                        updated_at: Utc::now(),
                        completed_at: None,
                        blocked_reason: None,
                        dependencies,
                        tags,
                    };

                    store.tasks.insert(task_id.clone(), task);

                    Ok(format!(
                        "Created task {}\nDescription: {}\nStatus: pending",
                        task_id, description
                    ))
                })
                .await?
            }

            TaskOperation::Update {
                task_id,
                status,
                description,
                priority,
                blocked_reason,
            } => {
                TaskStore::with_lock(&store_path, |store| {
                    // Check if task exists first
                    if !store.tasks.contains_key(&task_id) {
                        return Err(ToolError::InvalidInput(format!(
                            "Task not found: {}",
                            task_id
                        )));
                    }

                    let mut updates = Vec::new();

                    // Check dependencies before updating status
                    if let Some(ref new_status) = status {
                        let parsed_status = parse_status(new_status)?;

                        // Check dependencies before moving to in_progress or completed
                        if matches!(
                            parsed_status,
                            TaskStatus::InProgress | TaskStatus::Completed
                        ) {
                            let task_deps = store
                                .tasks
                                .get(&task_id)
                                .ok_or_else(|| {
                                    ToolError::Internal(format!(
                                        "Task {} not found after check",
                                        task_id
                                    ))
                                })?
                                .dependencies
                                .clone();
                            for dep_id in &task_deps {
                                if let Some(dep_task) = store.tasks.get(dep_id)
                                    && dep_task.status != TaskStatus::Completed {
                                        return Err(ToolError::InvalidInput(format!(
                                            "Cannot update task: dependency {} is not completed",
                                            dep_id
                                        )));
                                    }
                            }
                        }
                    }

                    // Now get mutable reference and update all fields
                    let task = store.tasks.get_mut(&task_id).ok_or_else(|| {
                        ToolError::Internal(format!("Task {} not found after check", task_id))
                    })?;

                    if let Some(new_status) = status {
                        let parsed_status = parse_status(&new_status)?;
                        task.status = parsed_status.clone();
                        updates.push(format!("status: {:?}", parsed_status));

                        if parsed_status == TaskStatus::Completed {
                            task.completed_at = Some(Utc::now());
                        }
                    }

                    if let Some(new_desc) = description {
                        task.description = new_desc.clone();
                        updates.push(format!("description: {}", new_desc));
                    }

                    if let Some(new_priority) = priority {
                        task.priority = parse_priority(&new_priority)?;
                        updates.push(format!("priority: {}", new_priority));
                    }

                    if let Some(reason) = blocked_reason {
                        task.blocked_reason = Some(reason.clone());
                        updates.push(format!("blocked_reason: {}", reason));
                    }

                    task.updated_at = Utc::now();

                    Ok(format!(
                        "Updated task {}\nChanges: {}",
                        task_id,
                        updates.join(", ")
                    ))
                })
                .await?
            }

            TaskOperation::Delete { task_id } => {
                TaskStore::with_lock(&store_path, |store| {
                    // Check if any other tasks depend on this task
                    let dependents: Vec<String> = store
                        .tasks
                        .values()
                        .filter(|t| t.dependencies.contains(&task_id))
                        .map(|t| t.id.clone())
                        .collect();

                    if !dependents.is_empty() {
                        return Err(ToolError::InvalidInput(format!(
                            "Cannot delete task: {} other tasks depend on it: {}",
                            dependents.len(),
                            dependents
                                .iter()
                                .map(|id| &id[..8])
                                .collect::<Vec<_>>()
                                .join(", ")
                        )));
                    }

                    let task = store.tasks.remove(&task_id).ok_or_else(|| {
                        ToolError::InvalidInput(format!("Task not found: {}", task_id))
                    })?;

                    Ok(format!(
                        "Deleted task {}\nDescription: {}",
                        task_id, task.description
                    ))
                })
                .await?
            }

            TaskOperation::ClearCompleted => {
                TaskStore::with_lock(&store_path, |store| {
                    let completed_count = store
                        .tasks
                        .iter()
                        .filter(|(_, t)| t.status == TaskStatus::Completed)
                        .count();

                    store.tasks.retain(|_, t| t.status != TaskStatus::Completed);

                    Ok(format!("Cleared {} completed tasks", completed_count))
                })
                .await?
            }
        };

        Ok(ToolResult::success(result))
    }
}
