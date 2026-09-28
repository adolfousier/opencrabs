//! Session Context Tool
//!
//! Manage conversation context, store session variables, and maintain state.

use super::error::{Result, ToolError};
use super::r#trait::{Tool, ToolCapability, ToolExecutionContext, ToolResult};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::fs;

/// Monotonic counter for unique context-store temp-file names, so concurrent
/// `session_context` saves never race on the same temp path.
static CONTEXT_TMP_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Session context management tool
pub struct ContextTool;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ContextEntry {
    key: String,
    value: Value,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct ContextStore {
    session_id: String,
    variables: HashMap<String, ContextEntry>,
    #[serde(default)]
    pub(crate) facts: Vec<String>,
    #[serde(default)]
    decisions: Vec<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl ContextStore {
    pub(crate) fn new(session_id: String) -> Self {
        let now = Utc::now();
        Self {
            session_id,
            variables: HashMap::new(),
            facts: Vec::new(),
            decisions: Vec::new(),
            created_at: now,
            updated_at: now,
        }
    }

    pub(crate) async fn load(path: &Path, session_id: &str) -> Result<Self> {
        if path.exists() {
            let content = fs::read_to_string(path).await.map_err(ToolError::Io)?;
            let mut store: Self = serde_json::from_str(&content).map_err(|e| {
                ToolError::Execution(format!("Failed to parse context store: {}", e))
            })?;
            store.session_id = session_id.to_string();
            Ok(store)
        } else {
            Ok(Self::new(session_id.to_string()))
        }
    }

    pub(crate) async fn save(&self, path: &Path) -> Result<()> {
        let content = serde_json::to_string_pretty(self)
            .map_err(|e| ToolError::Execution(format!("Failed to serialize context: {}", e)))?;

        // Ensure parent directory exists
        let parent = path.parent().ok_or_else(|| {
            ToolError::Execution("context store path has no parent directory".to_string())
        })?;
        fs::create_dir_all(parent).await.map_err(ToolError::Io)?;

        // Atomic, race-free write — against concurrent WRITERS, not against
        // power loss: there is no fsync before the rename, so a crash can leave
        // the rename applied and the bytes unflushed (#906). Acceptable for a
        // transient context store; stated so the next reader does not assume
        // durability. `fs::write` truncates per call, but two
        // concurrent `session_context` saves in one turn (e.g. several `set`
        // ops fired together) don't serialize: a shorter write landing over a
        // longer one leaves the longer object's tail bytes in place, producing
        // "trailing characters" parse failures on the next read. Write a
        // uniquely-named temp file in the same directory, then atomically
        // rename it over the target — the store is always either the previous
        // complete object or the new one, never a mix. The unique temp name
        // (pid + monotonic counter) prevents two saves from racing on the temp
        // file itself.
        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("context_store.json");
        let seq = CONTEXT_TMP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let tmp = parent.join(format!("{}.{}.{}.tmp", file_name, std::process::id(), seq));
        fs::write(&tmp, content).await.map_err(ToolError::Io)?;
        // A failed rename must not strand its temp file (#906). Without this
        // the `.tmp` stays beside the store indefinitely, accumulating silently
        // in the same directory. Cleanup is best-effort and never masks the
        // rename error, which is the failure the caller needs to see.
        if let Err(e) = fs::rename(&tmp, path).await {
            if let Err(cleanup) = fs::remove_file(&tmp).await {
                tracing::warn!(
                    "context store: rename failed and its temp file could not be \
                     removed either ({cleanup}); leaving {}",
                    tmp.display()
                );
            }
            return Err(ToolError::Io(e));
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "operation")]
enum ContextOperation {
    #[serde(rename = "set")]
    Set {
        key: String,
        value: Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        #[serde(default)]
        tags: Vec<String>,
    },

    #[serde(rename = "get")]
    Get { key: String },

    #[serde(rename = "delete")]
    Delete { key: String },

    #[serde(rename = "list")]
    List {
        #[serde(skip_serializing_if = "Option::is_none")]
        tag: Option<String>,
    },

    #[serde(rename = "add_fact")]
    AddFact { fact: String },

    #[serde(rename = "add_decision")]
    AddDecision { decision: String },

    #[serde(rename = "summary")]
    Summary,

    #[serde(rename = "clear")]
    Clear {
        #[serde(default)]
        confirm: bool,
    },
}

#[derive(Debug, Deserialize, Serialize)]
struct ContextInput {
    #[serde(flatten)]
    operation: ContextOperation,
}

fn get_store_path(context: &ToolExecutionContext) -> PathBuf {
    let dir = crate::config::opencrabs_home()
        .join("agents")
        .join("session");
    let _ = std::fs::create_dir_all(&dir);
    dir.join(format!("context_{}.json", context.session_id))
}

#[async_trait]
impl Tool for ContextTool {
    fn name(&self) -> &str {
        "session_context"
    }

    fn description(&self) -> &str {
        "Manage session context and variables. Store key-value pairs, track important facts and decisions, and maintain state across the conversation."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "operation": {
                    "type": "string",
                    "description": "Operation to perform",
                    "enum": ["set", "get", "delete", "list", "add_fact", "add_decision", "summary", "clear"]
                },
                "key": {
                    "type": "string",
                    "description": "Variable key (for set, get, delete)"
                },
                "value": {
                    "description": "Variable value (for set operation, can be any JSON type)"
                },
                "description": {
                    "type": "string",
                    "description": "Description of the variable (optional)"
                },
                "tags": {
                    "type": "array",
                    "description": "Tags for categorizing variables",
                    "items": {
                        "type": "string"
                    },
                    "default": []
                },
                "tag": {
                    "type": "string",
                    "description": "Filter by tag (for list operation)"
                },
                "fact": {
                    "type": "string",
                    "description": "Important fact to remember (for add_fact)"
                },
                "decision": {
                    "type": "string",
                    "description": "Important decision made (for add_decision)"
                },
                "confirm": {
                    "type": "boolean",
                    "description": "Confirm clear operation (must be true)",
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
        false // Context management is safe
    }

    fn validate_input(&self, input: &Value) -> Result<()> {
        let _: ContextInput = serde_json::from_value(input.clone())
            .map_err(|e| ToolError::InvalidInput(format!("Invalid input: {}", e)))?;
        Ok(())
    }

    async fn execute(&self, input: Value, context: &ToolExecutionContext) -> Result<ToolResult> {
        let input: ContextInput = serde_json::from_value(input)?;
        let store_path = get_store_path(context);

        // Serialise the whole read-modify-write, not just the save (#600).
        // `ContextStore::save` renames a uniquely-named temp file over the
        // target, which makes each write atomic but leaves the window between
        // the load below and that write wide open: two mutations in one
        // parallel batch both read the same baseline, the later save replaces
        // the file outright, and both calls return success carrying their own
        // stale count. The guard is taken BEFORE the read — the #593 lesson
        // that `edit.rs` applies to the same class of defect. It stays
        // advisory (a contended writer waits briefly, then proceeds), so any
        // residual overlap is reported rather than silent. A short hold is
        // NECESSARY BUT NOT SUFFICIENT, and this comment used to claim the
        // former as a reason for the latter: measured, a hold far shorter than
        // the wait still serialised nothing, because the waiter stalled the
        // task the holder needed to finish on. The two paragraphs below are
        // what actually makes it work.
        //
        // The wait must NOT run on this task. A parallel batch is polled
        // cooperatively on one task (`buffered` in `parallel_tools.rs`), so a
        // blocking sleep here stalls every sibling future — including the one
        // already holding the lock, whose critical section then cannot finish
        // and whose guard is never released. Measured with the wait inline:
        // three mutations in one batch, all three read the same baseline, two
        // lost, and the batch took 1.72 s — two 750 ms waits burned against a
        // holder that could not make progress. Offloading the wait to the
        // blocking pool lets the holder complete, and the waiters then contend
        // against a lock that is actually released.
        //
        // But the PATH must be resolved on THIS task, not in the blocking
        // closure. `resolve_profile_home` reads a `tokio::task_local` override,
        // and `spawn_blocking` starts a new task that cannot see it: resolving
        // there keyed the lock to the real profile home while the store stayed
        // in the scoped one, so contending writers took different lock files,
        // each reported `is_held()` true, and the arbitration was silently
        // inert. `a_contended_write_is_reported_not_swallowed` is that defect's
        // receipt — it failed with a plain success where the notice belongs.
        let lock_at = super::path_lock::lock_path(&store_path);
        let write_lock = match lock_at {
            Some(path) => {
                tokio::task::spawn_blocking(move || super::path_lock::acquire_at(&path))
                    .await
                    .unwrap_or(None)
            }
            None => None,
        };
        let contended = write_lock.as_ref().is_some_and(|l| !l.is_held());

        let session_id_str = context.session_id.to_string();
        let mut store = ContextStore::load(&store_path, &session_id_str).await?;

        let mut result = match input.operation {
            ContextOperation::Set {
                key,
                value,
                description,
                tags,
            } => {
                let now = Utc::now();
                let is_update = store.variables.contains_key(&key);

                let entry = ContextEntry {
                    key: key.clone(),
                    value: value.clone(),
                    created_at: if is_update {
                        store
                            .variables
                            .get(&key)
                            .map(|e| e.created_at)
                            .unwrap_or(now)
                    } else {
                        now
                    },
                    updated_at: now,
                    description,
                    tags,
                };

                store.variables.insert(key.clone(), entry);
                store.updated_at = now;
                store.save(&store_path).await?;

                if is_update {
                    format!("Updated variable '{}' = {}", key, value)
                } else {
                    format!("Set variable '{}' = {}", key, value)
                }
            }

            ContextOperation::Get { key } => {
                let entry = store.variables.get(&key).ok_or_else(|| {
                    ToolError::InvalidInput(format!("Variable not found: {}", key))
                })?;

                let mut output = format!("Variable: {}\n", key);
                output.push_str(&format!("Value: {}\n", entry.value));
                if let Some(desc) = &entry.description {
                    output.push_str(&format!("Description: {}\n", desc));
                }
                if !entry.tags.is_empty() {
                    output.push_str(&format!("Tags: {}\n", entry.tags.join(", ")));
                }
                output.push_str(&format!(
                    "Created: {}\n",
                    entry.created_at.format("%Y-%m-%d %H:%M:%S")
                ));
                output.push_str(&format!(
                    "Updated: {}\n",
                    entry.updated_at.format("%Y-%m-%d %H:%M:%S")
                ));

                output
            }

            ContextOperation::Delete { key } => {
                store.variables.remove(&key).ok_or_else(|| {
                    ToolError::InvalidInput(format!("Variable not found: {}", key))
                })?;

                store.updated_at = Utc::now();
                store.save(&store_path).await?;

                format!("Deleted variable '{}'", key)
            }

            ContextOperation::List { tag } => {
                let mut filtered_vars: Vec<_> = store
                    .variables
                    .values()
                    .filter(|e| {
                        if let Some(ref t) = tag {
                            e.tags.contains(t)
                        } else {
                            true
                        }
                    })
                    .collect();

                if filtered_vars.is_empty() {
                    return Ok(ToolResult::success("No variables found".to_string()));
                }

                filtered_vars.sort_by(|a, b| a.key.cmp(&b.key));

                let mut output = format!("Found {} variables:\n\n", filtered_vars.len());
                for entry in filtered_vars {
                    output.push_str(&format!("{} = {}\n", entry.key, entry.value));
                    if let Some(desc) = &entry.description {
                        output.push_str(&format!("  {}\n", desc));
                    }
                    if !entry.tags.is_empty() {
                        output.push_str(&format!("  Tags: {}\n", entry.tags.join(", ")));
                    }
                    output.push('\n');
                }

                output
            }

            ContextOperation::AddFact { fact } => {
                store.facts.push(fact.clone());
                store.updated_at = Utc::now();
                store.save(&store_path).await?;

                format!("Added fact: {}\nTotal facts: {}", fact, store.facts.len())
            }

            ContextOperation::AddDecision { decision } => {
                store.decisions.push(decision.clone());
                store.updated_at = Utc::now();
                store.save(&store_path).await?;

                format!(
                    "Added decision: {}\nTotal decisions: {}",
                    decision,
                    store.decisions.len()
                )
            }

            ContextOperation::Summary => {
                let mut output = "Session Context Summary\n".to_string();
                output.push_str(&format!("Session ID: {}\n", store.session_id));
                output.push_str(&format!(
                    "Created: {}\n",
                    store.created_at.format("%Y-%m-%d %H:%M:%S")
                ));
                output.push_str(&format!(
                    "Last Updated: {}\n\n",
                    store.updated_at.format("%Y-%m-%d %H:%M:%S")
                ));

                output.push_str(&format!("Variables: {}\n", store.variables.len()));
                output.push_str(&format!("Facts: {}\n", store.facts.len()));
                output.push_str(&format!("Decisions: {}\n\n", store.decisions.len()));

                if !store.facts.is_empty() {
                    output.push_str("Key Facts:\n");
                    for (i, fact) in store.facts.iter().enumerate() {
                        output.push_str(&format!("{}. {}\n", i + 1, fact));
                    }
                    output.push('\n');
                }

                if !store.decisions.is_empty() {
                    output.push_str("Key Decisions:\n");
                    for (i, decision) in store.decisions.iter().enumerate() {
                        output.push_str(&format!("{}. {}\n", i + 1, decision));
                    }
                    output.push('\n');
                }

                if !store.variables.is_empty() {
                    output.push_str("Variables:\n");
                    let mut vars: Vec<_> = store.variables.keys().collect();
                    vars.sort();
                    for key in vars {
                        output.push_str(&format!("  {}\n", key));
                    }
                }

                output
            }

            ContextOperation::Clear { confirm } => {
                if !confirm {
                    return Ok(ToolResult::error(
                        "Clear operation requires confirm=true to proceed".to_string(),
                    ));
                }

                let var_count = store.variables.len();
                let fact_count = store.facts.len();
                let decision_count = store.decisions.len();

                store.variables.clear();
                store.facts.clear();
                store.decisions.clear();
                store.updated_at = Utc::now();
                store.save(&store_path).await?;

                format!(
                    "Cleared all context data\nVariables: {}\nFacts: {}\nDecisions: {}",
                    var_count, fact_count, decision_count
                )
            }
        };

        // An overlapping mutation is reported, not swallowed: the counts above
        // were computed from the store we read AND wrote under the lock, so
        // they describe the persisted state — but a contended writer cannot
        // prove no peer interleaved, and only the caller can decide what that
        // means. Same idiom as `edit.rs`.
        if contended {
            result.push_str(&super::path_lock::contention_notice(&store_path));
        }

        Ok(ToolResult::success(result))
    }
}
