use crate::AppState;
use crate::commands::config_cmd;
use crate::commands::validation::bounded_text;
use serde::Serialize;
use tauri::State;

#[derive(Serialize)]
pub struct ToolInfo {
    pub name: String,
    pub description: String,
    pub capabilities: Vec<String>,
}

#[derive(Serialize)]
pub struct ToolDetail {
    pub name: String,
    pub description: String,
    pub capabilities: Vec<String>,
    pub parameters: serde_json::Value,
}

fn tool_from_dynamic_def(def: &opencrabs::brain::tools::dynamic::tool::DynamicToolDef) -> ToolInfo {
    let mut capabilities = vec![
        "dynamic".to_string(),
        format!("executor:{:?}", def.executor).to_lowercase(),
    ];
    if def.requires_approval {
        capabilities.push("approval".to_string());
    }
    ToolInfo {
        name: def.name.clone(),
        description: def.description.clone(),
        capabilities,
    }
}

fn tool_detail_from_dynamic_def(
    def: &opencrabs::brain::tools::dynamic::tool::DynamicToolDef,
) -> ToolDetail {
    ToolDetail {
        name: def.name.clone(),
        description: def.description.clone(),
        capabilities: tool_from_dynamic_def(def).capabilities,
        parameters: serde_json::json!({
            "executor": format!("{:?}", def.executor).to_lowercase(),
            "enabled": def.enabled,
            "requires_approval": def.requires_approval,
            "method": def.method,
            "url": def.url,
            "command": def.command,
            "timeout_secs": def.timeout_secs,
            "params": def.params,
        }),
    }
}

fn list_dynamic_tool_defs()
-> Result<Vec<opencrabs::brain::tools::dynamic::tool::DynamicToolDef>, String> {
    let path = opencrabs::brain::tools::dynamic::DynamicToolLoader::default_path()
        .unwrap_or_else(|| opencrabs::config::opencrabs_home().join("tools.toml"));
    opencrabs::brain::tools::dynamic::DynamicToolLoader::list_tools_detailed(&path)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn list_tools(_state: State<'_, AppState>) -> Result<Vec<ToolInfo>, String> {
    let mut tools = core_tools();
    if let Ok(dynamic) = list_dynamic_tool_defs() {
        tools.extend(dynamic.iter().map(tool_from_dynamic_def));
    }
    tools.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(tools)
}

#[tauri::command]
pub async fn get_tool_details(
    _state: State<'_, AppState>,
    tool_name: String,
) -> Result<ToolDetail, String> {
    if let Ok(dynamic) = list_dynamic_tool_defs()
        && let Some(def) = dynamic.into_iter().find(|tool| tool.name == tool_name)
    {
        return Ok(tool_detail_from_dynamic_def(&def));
    }

    let tools = core_tools();
    let tool = tools
        .into_iter()
        .find(|t| t.name == tool_name)
        .ok_or_else(|| format!("Tool not found: {}", tool_name))?;

    Ok(ToolDetail {
        name: tool.name,
        description: tool.description,
        capabilities: tool.capabilities,
        parameters: tool_params_for(&tool_name),
    })
}

#[tauri::command]
pub async fn approve_tool(
    state: State<'_, AppState>,
    session_id: String,
    tool_name: String,
    approved: bool,
    always_approve: bool,
) -> Result<(), String> {
    // The gesture originates from a specific tool and session. The preview does
    // not yet persist per-tool approval, but we still validate the identifiers
    // so an empty or oversized payload is rejected before it touches config.
    bounded_text("session id", &session_id, 256)?;
    bounded_text("tool name", &tool_name, 128)?;

    let policy = config_cmd::policy_for_approval(approved, always_approve);
    config_cmd::apply_approval_policy(policy)?;
    let refreshed = opencrabs::config::Config::load().map_err(|e| e.to_string())?;
    *state.config.write().await = refreshed;
    tracing::info!(
        tool_name,
        session_id,
        policy,
        "Desktop tool approval gesture applied to global agent policy"
    );
    Ok(())
}

fn core_tools() -> Vec<ToolInfo> {
    vec![
        ToolInfo {
            name: "read_file".into(),
            description: "Read file contents from the workspace".into(),
            capabilities: vec!["filesystem".into(), "read".into()],
        },
        ToolInfo {
            name: "write_file".into(),
            description: "Write or overwrite a file in the workspace".into(),
            capabilities: vec!["filesystem".into(), "write".into()],
        },
        ToolInfo {
            name: "edit_file".into(),
            description: "Make precise text replacements in files".into(),
            capabilities: vec!["filesystem".into(), "edit".into()],
        },
        ToolInfo {
            name: "bash".into(),
            description: "Execute shell commands".into(),
            capabilities: vec!["shell".into(), "execution".into()],
        },
        ToolInfo {
            name: "ls".into(),
            description: "List directory contents".into(),
            capabilities: vec!["filesystem".into(), "read".into()],
        },
        ToolInfo {
            name: "glob".into(),
            description: "Search for files by glob pattern".into(),
            capabilities: vec!["filesystem".into(), "search".into()],
        },
        ToolInfo {
            name: "grep".into(),
            description: "Search file contents with regex".into(),
            capabilities: vec!["filesystem".into(), "search".into()],
        },
        ToolInfo {
            name: "web_search".into(),
            description: "Search the web".into(),
            capabilities: vec!["web".into(), "search".into()],
        },
        ToolInfo {
            name: "web_scrape".into(),
            description: "Extract content from a URL".into(),
            capabilities: vec!["web".into(), "scrape".into()],
        },
        ToolInfo {
            name: "memory_search".into(),
            description: "Search the vector memory store".into(),
            capabilities: vec!["memory".into(), "search".into()],
        },
        ToolInfo {
            name: "load_brain_file".into(),
            description: "Load a brain file into context".into(),
            capabilities: vec!["brain".into(), "read".into()],
        },
        ToolInfo {
            name: "write_opencrabs_file".into(),
            description: "Write to brain configuration files".into(),
            capabilities: vec!["brain".into(), "write".into()],
        },
        ToolInfo {
            name: "session_search".into(),
            description: "Search across all sessions".into(),
            capabilities: vec!["session".into(), "search".into()],
        },
        ToolInfo {
            name: "generate_image".into(),
            description: "Generate images via AI".into(),
            capabilities: vec!["media".into(), "generation".into()],
        },
        ToolInfo {
            name: "analyze_image".into(),
            description: "Analyze images using vision models".into(),
            capabilities: vec!["media".into(), "vision".into()],
        },
        ToolInfo {
            name: "spawn_agent".into(),
            description: "Spawn a sub-agent for parallel work".into(),
            capabilities: vec!["agent".into(), "parallel".into()],
        },
        ToolInfo {
            name: "a2a_send".into(),
            description: "Send message to another agent via A2A".into(),
            capabilities: vec!["agent".into(), "interop".into()],
        },
        ToolInfo {
            name: "goal_manage".into(),
            description: "Manage goals and track progress".into(),
            capabilities: vec!["planning".into(), "goals".into()],
        },
        ToolInfo {
            name: "plan".into(),
            description: "Create and manage execution plans".into(),
            capabilities: vec!["planning".into()],
        },
    ]
}

fn tool_params_for(name: &str) -> serde_json::Value {
    match name {
        "read_file" => {
            serde_json::json!({"path": "string (required)", "offset": "number (optional)", "limit": "number (optional)"})
        }
        "write_file" => {
            serde_json::json!({"path": "string (required)", "content": "string (required)"})
        }
        "edit_file" => {
            serde_json::json!({"path": "string (required)", "old_text": "string (required)", "new_text": "string (required)"})
        }
        "bash" => {
            serde_json::json!({"command": "string (required)", "timeout_ms": "number (optional)"})
        }
        "glob" => serde_json::json!({"pattern": "string (required)"}),
        "grep" => {
            serde_json::json!({"pattern": "string (required)", "include": "string[] (optional)", "path": "string (optional)"})
        }
        "web_search" => serde_json::json!({"query": "string (required)"}),
        "web_scrape" => serde_json::json!({"url": "string (required)"}),
        "generate_image" => {
            serde_json::json!({"prompt": "string (required)", "model": "string (optional)"})
        }
        "spawn_agent" => {
            serde_json::json!({"prompt": "string (required)", "agent_type": "string (optional)", "tools": "string[] (optional)"})
        }
        "a2a_send" => {
            serde_json::json!({"agent_url": "string (required)", "message": "string (required)"})
        }
        _ => serde_json::json!({}),
    }
}
