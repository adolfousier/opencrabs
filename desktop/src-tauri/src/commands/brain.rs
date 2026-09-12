use crate::AppState;
use serde::Serialize;
use tauri::State;

const SAFE_BRAIN_FILES: &[&str] = &[
    "SOUL.md",
    "USER.md",
    "AGENTS.md",
    "CODE.md",
    "TOOLS.md",
    "SECURITY.md",
    "MEMORY.md",
    "BOOT.md",
    "HEARTBEAT.md",
];
const MAX_BRAIN_FILE_BYTES: usize = 200_000;
const PROTECTED_BRAIN_FILES: &[&str] = &[
    "AGENTS.md",
    "SOUL.md",
    "SECURITY.md",
    "MEMORY.md",
    "TOOLS.md",
];

#[derive(Serialize)]
pub struct BrainFile {
    pub name: String,
    pub content: String,
    pub category: String,
}

fn category_for_brain(name: &str) -> &'static str {
    match name {
        "SOUL.md" | "USER.md" => "core",
        "AGENTS.md" => "always",
        _ => "contextual",
    }
}

fn is_safe_brain_file(name: &str) -> bool {
    SAFE_BRAIN_FILES.contains(&name)
}

fn protected_brain_file(name: &str) -> bool {
    PROTECTED_BRAIN_FILES.contains(&name)
}

fn validate_brain_write(name: &str, content: &str) -> Result<(), String> {
    if content.len() > MAX_BRAIN_FILE_BYTES {
        return Err("Brain file write denied: content too large".to_string());
    }
    if content.trim().is_empty() {
        return Err("Brain file write denied: empty content".to_string());
    }
    if protected_brain_file(name) && !content.contains("**Owns:**") {
        return Err(format!(
            "Protected brain file {} must retain ownership header",
            name
        ));
    }
    Ok(())
}

#[tauri::command]
pub async fn list_brain_files(_state: State<'_, AppState>) -> Result<Vec<BrainFile>, String> {
    let brain_path = opencrabs::config::opencrabs_home();
    let mut files = Vec::new();

    for filename in SAFE_BRAIN_FILES {
        let path = brain_path.join(filename);
        if path.exists() {
            let content = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
            files.push(BrainFile {
                name: (*filename).to_string(),
                content,
                category: category_for_brain(filename).to_string(),
            });
        }
    }

    Ok(files)
}

#[tauri::command]
pub async fn read_brain_file(
    _state: State<'_, AppState>,
    name: String,
) -> Result<BrainFile, String> {
    if !is_safe_brain_file(&name) {
        return Err(format!("Unknown brain file: {}", name));
    }

    let brain_path = opencrabs::config::opencrabs_home();
    let path = brain_path.join(&name);

    if !path.exists() {
        return Err(format!("Brain file not found: {}", name));
    }

    let content = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;

    Ok(BrainFile {
        name: name.clone(),
        content,
        category: category_for_brain(&name).to_string(),
    })
}
#[tauri::command]
pub async fn write_brain_file(
    _state: State<'_, AppState>,
    name: String,
    content: String,
) -> Result<(), String> {
    if !is_safe_brain_file(&name) {
        return Err(format!("Unknown brain file: {}", name));
    }
    validate_brain_write(&name, &content)?;

    let brain_path = opencrabs::config::opencrabs_home();
    let path = brain_path.join(&name);
    std::fs::write(&path, content).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unknown_brain_file_name() {
        assert!(!is_safe_brain_file("NOTES.md"));
        assert!(is_safe_brain_file("SOUL.md"));
    }

    #[test]
    fn rejects_empty_or_oversized_brain_writes() {
        let empty = validate_brain_write("SOUL.md", "   ").expect_err("empty content should fail");
        assert!(empty.contains("empty content"));

        let oversized = "x".repeat(MAX_BRAIN_FILE_BYTES + 1);
        let err =
            validate_brain_write("SOUL.md", &oversized).expect_err("oversized content should fail");
        assert!(err.contains("too large"));
    }

    #[test]
    fn protected_brain_files_must_keep_header() {
        let err = validate_brain_write("AGENTS.md", "missing ownership header")
            .expect_err("header required");
        assert!(err.contains("must retain ownership header"));
        validate_brain_write("AGENTS.md", "**Owns:** rules\nrest")
            .expect("header present should pass");
    }
}
