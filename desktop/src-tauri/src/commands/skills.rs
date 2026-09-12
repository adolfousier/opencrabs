use crate::AppState;
use opencrabs::brain::{CommandLoader, UserCommand};
use serde::Serialize;
use std::collections::HashSet;
use tauri::State;

#[derive(Serialize)]
pub struct SkillInfo {
    pub name: String,
    pub description: String,
    pub source: String,
    pub review_gate: bool,
    pub enabled: bool,
}

#[derive(Serialize)]
pub struct SkillDetail {
    pub name: String,
    pub description: String,
    pub body: String,
    pub source: String,
    pub review_gate: bool,
    pub enabled: bool,
}

fn desktop_skill_loader() -> CommandLoader {
    CommandLoader::from_brain_path(&opencrabs::config::opencrabs_home())
}

fn desktop_skill_command_name(name: &str) -> String {
    format!("/skill-{}", name.trim_start_matches('/'))
}

fn load_disabled_skills() -> HashSet<String> {
    desktop_skill_loader()
        .load()
        .into_iter()
        .filter(|cmd| cmd.action == "system" && cmd.prompt == "desktop-skill-disabled")
        .map(|cmd| cmd.name.trim_start_matches("/skill-").to_string())
        .collect()
}

#[tauri::command]
pub async fn list_skills(_state: State<'_, AppState>) -> Result<Vec<SkillInfo>, String> {
    let disabled = load_disabled_skills();
    let skills = opencrabs::brain::skills::load_all_skills();
    Ok(skills
        .into_iter()
        .map(|s| SkillInfo {
            enabled: !disabled.contains(&s.name),
            name: s.name,
            description: s.description,
            source: match s.source {
                opencrabs::brain::skills::SkillSource::Builtin => "builtin".into(),
                opencrabs::brain::skills::SkillSource::User => "user".into(),
            },
            review_gate: s.review_gate,
        })
        .collect())
}

#[tauri::command]
pub async fn get_skill_details(
    _state: State<'_, AppState>,
    name: String,
) -> Result<SkillDetail, String> {
    let disabled = load_disabled_skills();
    let skills = opencrabs::brain::skills::load_all_skills();
    let skill = skills
        .into_iter()
        .find(|s| s.name == name)
        .ok_or_else(|| format!("Skill not found: {}", name))?;

    Ok(SkillDetail {
        enabled: !disabled.contains(&skill.name),
        name: skill.name,
        description: skill.description,
        body: skill.body,
        source: match skill.source {
            opencrabs::brain::skills::SkillSource::Builtin => "builtin".into(),
            opencrabs::brain::skills::SkillSource::User => "user".into(),
        },
        review_gate: skill.review_gate,
    })
}

#[tauri::command]
pub async fn toggle_skill(
    _state: State<'_, AppState>,
    name: String,
    enabled: bool,
) -> Result<(), String> {
    let loader = desktop_skill_loader();
    let command_name = desktop_skill_command_name(&name);
    if enabled {
        loader
            .remove_command(&command_name)
            .map_err(|e| e.to_string())?;
    } else {
        loader
            .add_command(UserCommand {
                name: command_name,
                description: format!("Desktop override: disable skill {}", name),
                action: "system".to_string(),
                prompt: "desktop-skill-disabled".to_string(),
            })
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}
