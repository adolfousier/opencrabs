use serde::Serialize;
use std::path::{Component, Path, PathBuf};

const MAX_TEXT_FILE_BYTES: u64 = 256 * 1024;
const TEXT_EXTS: &[&str] = &[
    "rs",
    "toml",
    "md",
    "txt",
    "json",
    "yaml",
    "yml",
    "js",
    "ts",
    "tsx",
    "jsx",
    "html",
    "css",
    "py",
    "sh",
    "bash",
    "c",
    "h",
    "cpp",
    "hpp",
    "go",
    "rb",
    "lua",
    "sql",
    "xml",
    "ini",
    "cfg",
    "env",
    "gitignore",
];

#[derive(Serialize)]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: Option<u64>,
    pub extension: Option<String>,
}

#[derive(Serialize)]
pub struct FileContent {
    pub path: String,
    pub content: String,
    pub is_binary: bool,
    pub size: u64,
}

async fn workspace_root_path() -> Result<PathBuf, String> {
    // The desktop app is launched from a project directory; that directory is
    // the workspace root. We intentionally do NOT infer a root from config
    // values like the provider name, which is an identifier and never a path.
    std::env::current_dir().map_err(|e| format!("Unable to resolve workspace root: {e}"))
}

fn contains_parent_dir(path: &Path) -> bool {
    path.components()
        .any(|component| matches!(component, Component::ParentDir))
}

fn normalize_existing_path(path: &Path) -> Result<PathBuf, String> {
    path.canonicalize().map_err(|e| e.to_string())
}

fn ensure_within_workspace(path: &Path, workspace_root: &Path) -> Result<PathBuf, String> {
    if contains_parent_dir(path) {
        return Err("Parent-directory traversal is not allowed".to_string());
    }

    let root = normalize_existing_path(workspace_root)?;
    let target = normalize_existing_path(path)?;
    if target == root || target.starts_with(&root) {
        Ok(target)
    } else {
        Err(format!(
            "Path '{}' is outside the configured workspace '{}'",
            target.display(),
            root.display()
        ))
    }
}

fn is_text_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| TEXT_EXTS.contains(&e))
        .unwrap_or(false)
}

#[tauri::command]
pub async fn list_directory(path: Option<String>) -> Result<Vec<FileEntry>, String> {
    let workspace_root = workspace_root_path().await?;
    let requested = path
        .map(PathBuf::from)
        .unwrap_or_else(|| workspace_root.clone());
    let dir = ensure_within_workspace(&requested, &workspace_root)?;
    if !dir.is_dir() {
        return Err(format!("Not a directory: {}", dir.display()));
    }

    let entries = std::fs::read_dir(&dir).map_err(|e| e.to_string())?;
    let mut files = Vec::new();

    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        let metadata = entry.metadata().map_err(|e| e.to_string())?;
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }

        let extension = if !metadata.is_dir() {
            entry
                .path()
                .extension()
                .map(|e| e.to_string_lossy().to_string())
        } else {
            None
        };

        files.push(FileEntry {
            name,
            path: entry.path().to_string_lossy().to_string(),
            is_dir: metadata.is_dir(),
            size: if metadata.is_dir() {
                None
            } else {
                Some(metadata.len())
            },
            extension,
        });
    }

    files.sort_by(|a, b| {
        a.is_dir
            .cmp(&b.is_dir)
            .reverse()
            .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    Ok(files)
}

#[tauri::command]
pub async fn read_file_content(path: String) -> Result<FileContent, String> {
    let workspace_root = workspace_root_path().await?;
    let p = ensure_within_workspace(Path::new(&path), &workspace_root)?;
    let metadata = std::fs::metadata(&p).map_err(|e| e.to_string())?;
    if metadata.is_dir() {
        return Err(format!("Cannot read directory as file: {}", p.display()));
    }
    let is_binary = !is_text_file(&p);

    let content = if is_binary {
        format!("[Binary file: {} bytes]", metadata.len())
    } else if metadata.len() > MAX_TEXT_FILE_BYTES {
        format!(
            "[File too large to preview: {} bytes > {} bytes limit]",
            metadata.len(),
            MAX_TEXT_FILE_BYTES
        )
    } else {
        std::fs::read_to_string(&p).map_err(|e| e.to_string())?
    };

    Ok(FileContent {
        path: p.to_string_lossy().to_string(),
        content,
        is_binary,
        size: metadata.len(),
    })
}
#[tauri::command]
pub async fn get_workspace_root() -> Result<String, String> {
    Ok(workspace_root_path().await?.to_string_lossy().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time went backwards")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("opencrabs-desktop-{prefix}-{nanos}"));
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn rejects_parent_directory_traversal() {
        assert!(contains_parent_dir(Path::new("../escape.txt")));
        assert!(contains_parent_dir(Path::new("nested/../../escape.txt")));
        assert!(!contains_parent_dir(Path::new("nested/file.txt")));
    }

    #[test]
    fn allows_paths_inside_workspace() {
        let root = unique_temp_dir("files-inside-root");
        let nested = root.join("nested");
        fs::create_dir_all(&nested).unwrap();
        let file = nested.join("note.txt");
        fs::write(&file, "hello").unwrap();

        let checked = ensure_within_workspace(&file, &root).expect("inside root should pass");
        assert_eq!(checked, file.canonicalize().unwrap());

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rejects_paths_outside_workspace() {
        let root = unique_temp_dir("files-root");
        let outside_parent = unique_temp_dir("files-outside-parent");
        let outside = outside_parent.join("secret.txt");
        fs::write(&outside, "nope").unwrap();

        let err = ensure_within_workspace(&outside, &root).expect_err("outside path should fail");
        assert!(err.contains("outside the configured workspace"));

        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(outside_parent).unwrap();
    }

    #[test]
    fn detects_text_extensions_case_sensitively() {
        assert!(is_text_file(Path::new("readme.md")));
        assert!(is_text_file(Path::new("main.rs")));
        assert!(!is_text_file(Path::new("archive.bin")));
        assert!(!is_text_file(Path::new("README.MD")));
    }
}
