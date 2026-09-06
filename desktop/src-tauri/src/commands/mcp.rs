use crate::commands::validation::{bounded_text, identifier};
use serde::Serialize;

#[derive(Serialize)]
pub struct DynamicToolInfo {
    pub name: String,
    pub description: String,
    pub executor: String,
    pub command: Option<String>,
    pub url: Option<String>,
    pub method: Option<String>,
}

#[tauri::command]
pub async fn list_dynamic_tools() -> Result<Vec<DynamicToolInfo>, String> {
    let tools_path = opencrabs::config::opencrabs_home().join("tools.toml");
    if !tools_path.exists() {
        return Ok(vec![]);
    }

    let content = std::fs::read_to_string(&tools_path).map_err(|e| e.to_string())?;
    let tools: opencrabs::brain::tools::dynamic::DynamicToolsConfig =
        toml::from_str(&content).map_err(|e| e.to_string())?;

    Ok(tools
        .tools
        .into_iter()
        .map(|def| DynamicToolInfo {
            name: def.name,
            description: def.description,
            executor: format!("{:?}", def.executor).to_lowercase(),
            command: def.command,
            url: def.url,
            method: def.method,
        })
        .collect())
}

#[tauri::command]
pub async fn add_dynamic_tool(
    name: String,
    description: String,
    executor: String,
    command: String,
) -> Result<(), String> {
    // A dynamic tool is later executed as a shell command or HTTP call by the
    // agent, so its definition is validated as strictly as any identifier the
    // runtime would invoke. Names are bounded identifier tokens; the command
    // body is bounded free-form text with no control characters.
    identifier("tool name", &name, 64)?;
    bounded_text("description", &description, 512)?;
    bounded_text("command", &command, 4096)?;

    let tools_path = opencrabs::config::opencrabs_home().join("tools.toml");
    let mut config = if tools_path.exists() {
        let content = std::fs::read_to_string(&tools_path).map_err(|e| e.to_string())?;
        toml::from_str::<opencrabs::brain::tools::dynamic::DynamicToolsConfig>(&content)
            .map_err(|e| e.to_string())?
    } else {
        opencrabs::brain::tools::dynamic::DynamicToolsConfig::default()
    };

    if config.tools.iter().any(|t| t.name == name) {
        return Err(format!("Dynamic tool already exists: {}", name));
    }

    let exec = match executor.as_str() {
        "shell" => opencrabs::brain::tools::dynamic::ExecutorType::Shell,
        "http" => opencrabs::brain::tools::dynamic::ExecutorType::Http,
        _ => return Err(format!("Unsupported executor: {}", executor)),
    };

    config
        .tools
        .push(opencrabs::brain::tools::dynamic::DynamicToolDef {
            name,
            description,
            executor: exec,
            enabled: true,
            requires_approval: true,
            method: None,
            url: None,
            headers: std::collections::HashMap::new(),
            timeout_secs: 30,
            command: Some(command),
            params: Vec::new(),
        });

    let serialized = toml::to_string_pretty(&config).map_err(|e| e.to_string())?;
    std::fs::write(&tools_path, serialized).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn remove_dynamic_tool(name: String) -> Result<(), String> {
    identifier("tool name", &name, 64)?;
    let tools_path = opencrabs::config::opencrabs_home().join("tools.toml");
    if !tools_path.exists() {
        return Ok(());
    }

    let content = std::fs::read_to_string(&tools_path).map_err(|e| e.to_string())?;
    let mut config: opencrabs::brain::tools::dynamic::DynamicToolsConfig =
        toml::from_str(&content).map_err(|e| e.to_string())?;
    config.tools.retain(|tool| tool.name != name);
    let serialized = toml::to_string_pretty(&config).map_err(|e| e.to_string())?;
    std::fs::write(&tools_path, serialized).map_err(|e| e.to_string())
}
