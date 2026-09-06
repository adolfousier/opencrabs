use crate::AppState;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

const PANE_STATE_FILE: &str = "desktop-panes.toml";
const DEFAULT_ROUTE: &str = "chat";
const ROUTES: &[&str] = &[
    "chat",
    "files",
    "brain",
    "providers",
    "tools",
    "skills",
    "cron",
    "channels",
    "usage",
];

#[derive(Clone, Serialize, Deserialize)]
pub struct PaneLayout {
    pub tree: PaneNode,
    pub focused_id: u32,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum PaneNode {
    #[serde(rename = "leaf")]
    Leaf {
        pane_id: u32,
        session_id: Option<String>,
    },
    #[serde(rename = "split")]
    Split {
        direction: String,
        ratio: f64,
        first: Box<PaneNode>,
        second: Box<PaneNode>,
    },
}

#[derive(Clone, Serialize, Deserialize)]
pub struct DesktopState {
    pub route: String,
    pub selected_session_id: Option<String>,
}

#[derive(Deserialize, Serialize)]
struct PaneStore {
    #[serde(default = "default_next_id")]
    next_id: u32,
    #[serde(default)]
    sessions: HashMap<String, String>,
    #[serde(default)]
    layout: Option<PaneLayout>,
    #[serde(default = "default_route")]
    route: String,
    #[serde(default)]
    selected_session_id: Option<String>,
}

impl Default for PaneStore {
    fn default() -> Self {
        Self {
            next_id: default_next_id(),
            sessions: HashMap::new(),
            layout: None,
            route: default_route(),
            selected_session_id: None,
        }
    }
}

fn default_next_id() -> u32 {
    1
}

fn default_route() -> String {
    DEFAULT_ROUTE.to_string()
}

fn state_path() -> PathBuf {
    opencrabs::config::opencrabs_home().join(PANE_STATE_FILE)
}

/// A corrupt optional UI-state file must never stop the desktop app from opening.
/// Preserve it for diagnosis, then use a known-good empty state for this launch.
fn load_store() -> Result<PaneStore, String> {
    let path = state_path();
    match fs::read_to_string(&path) {
        Ok(contents) => match toml::from_str(&contents) {
            Ok(store) => Ok(store),
            Err(error) => {
                let backup = path.with_extension("toml.corrupt");
                if let Err(backup_error) = fs::rename(&path, &backup) {
                    tracing::warn!(
                        "Invalid desktop state at {} ({error}); failed to preserve it at {}: {backup_error}",
                        path.display(),
                        backup.display()
                    );
                } else {
                    tracing::warn!(
                        "Invalid desktop state at {} preserved at {}; using defaults: {error}",
                        path.display(),
                        backup.display()
                    );
                }
                Ok(PaneStore::default())
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(PaneStore::default()),
        Err(error) => Err(format!("Unable to read desktop state: {error}")),
    }
}

fn save_store(store: &PaneStore) -> Result<(), String> {
    let path = state_path();
    let contents = toml::to_string(store)
        .map_err(|error| format!("Unable to encode desktop state: {error}"))?;
    fs::write(path, contents).map_err(|error| format!("Unable to save desktop state: {error}"))
}

fn pane_key(pane_id: u32) -> String {
    pane_id.to_string()
}

fn leaf(pane_id: u32, store: &PaneStore) -> PaneNode {
    PaneNode::Leaf {
        pane_id,
        session_id: store.sessions.get(&pane_key(pane_id)).cloned(),
    }
}

fn default_layout(store: &PaneStore) -> PaneLayout {
    let id = store.next_id.max(1);
    PaneLayout {
        tree: leaf(id, store),
        focused_id: id,
    }
}

fn current_layout(store: &PaneStore) -> PaneLayout {
    store
        .layout
        .clone()
        .unwrap_or_else(|| default_layout(store))
}

fn remove_pane(node: PaneNode, pane_id: u32) -> Option<PaneNode> {
    match node {
        PaneNode::Leaf {
            pane_id: current, ..
        } if current == pane_id => None,
        leaf @ PaneNode::Leaf { .. } => Some(leaf),
        PaneNode::Split {
            direction,
            ratio,
            first,
            second,
        } => {
            let first = remove_pane(*first, pane_id);
            let second = remove_pane(*second, pane_id);
            match (first, second) {
                (None, None) => None,
                (Some(node), None) | (None, Some(node)) => Some(node),
                (Some(first), Some(second)) => Some(PaneNode::Split {
                    direction,
                    ratio,
                    first: Box::new(first),
                    second: Box::new(second),
                }),
            }
        }
    }
}

fn valid_route(route: &str) -> bool {
    ROUTES.contains(&route)
}

fn validate_desktop_state(state: &DesktopState) -> Result<(), String> {
    if !valid_route(&state.route) {
        return Err("Unknown desktop route".to_string());
    }
    if let Some(session_id) = &state.selected_session_id
        && (session_id.trim().is_empty() || session_id.len() > 256)
    {
        return Err("Desktop session identifier must be between 1 and 256 characters".to_string());
    }
    Ok(())
}

#[tauri::command]
pub async fn get_desktop_state() -> Result<DesktopState, String> {
    let store = load_store()?;
    Ok(DesktopState {
        route: if valid_route(&store.route) {
            store.route
        } else {
            default_route()
        },
        selected_session_id: store.selected_session_id,
    })
}

#[tauri::command]
pub async fn save_desktop_state(state: DesktopState) -> Result<(), String> {
    validate_desktop_state(&state)?;
    let mut store = load_store()?;
    store.route = state.route;
    store.selected_session_id = state.selected_session_id;
    save_store(&store)
}

#[tauri::command]
pub async fn get_pane_layout() -> Result<PaneLayout, String> {
    let store = load_store()?;
    Ok(current_layout(&store))
}

#[tauri::command]
pub async fn split_pane(direction: String) -> Result<PaneLayout, String> {
    if !matches!(direction.as_str(), "horizontal" | "vertical") {
        return Err("Pane split direction must be horizontal or vertical".to_string());
    }
    let mut store = load_store()?;
    let current = current_layout(&store);
    let new_id = store
        .next_id
        .max(1)
        .checked_add(1)
        .ok_or_else(|| "Pane identifier limit reached".to_string())?;
    store.next_id = new_id;
    let layout = PaneLayout {
        tree: PaneNode::Split {
            direction,
            ratio: 0.5,
            first: Box::new(current.tree),
            second: Box::new(leaf(new_id, &store)),
        },
        focused_id: new_id,
    };
    store.layout = Some(layout.clone());
    save_store(&store)?;
    Ok(layout)
}

#[tauri::command]
pub async fn close_pane(pane_id: u32) -> Result<PaneLayout, String> {
    let mut store = load_store()?;
    store.sessions.remove(&pane_key(pane_id));
    let current = current_layout(&store);
    let tree = remove_pane(current.tree, pane_id).unwrap_or_else(|| leaf(1, &store));
    let focused_id = if current.focused_id == pane_id {
        1
    } else {
        current.focused_id
    };
    let layout = PaneLayout { tree, focused_id };
    store.layout = Some(layout.clone());
    save_store(&store)?;
    Ok(layout)
}

#[tauri::command]
pub async fn set_pane_session(
    _state: tauri::State<'_, AppState>,
    pane_id: u32,
    session_id: String,
) -> Result<(), String> {
    if session_id.trim().is_empty() || session_id.len() > 256 {
        return Err("Pane session identifier must be between 1 and 256 characters".to_string());
    }
    let mut store = load_store()?;
    store.next_id = store.next_id.max(pane_id);
    store.sessions.insert(pane_key(pane_id), session_id);
    save_store(&store)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_state_round_trips() {
        let state = DesktopState {
            route: "usage".to_string(),
            selected_session_id: Some("session-3".to_string()),
        };
        let encoded = toml::to_string(&state).expect("encode state");
        let decoded: DesktopState = toml::from_str(&encoded).expect("decode state");
        assert_eq!(decoded.route, "usage");
        assert_eq!(decoded.selected_session_id.as_deref(), Some("session-3"));
    }

    #[test]
    fn invalid_desktop_state_is_rejected() {
        assert!(
            validate_desktop_state(&DesktopState {
                route: "not-a-route".to_string(),
                selected_session_id: None,
            })
            .is_err()
        );
        assert!(
            validate_desktop_state(&DesktopState {
                route: "chat".to_string(),
                selected_session_id: Some(" ".to_string()),
            })
            .is_err()
        );
    }

    #[test]
    fn closing_a_pane_compacts_the_tree() {
        let tree = PaneNode::Split {
            direction: "horizontal".to_string(),
            ratio: 0.5,
            first: Box::new(PaneNode::Leaf {
                pane_id: 1,
                session_id: None,
            }),
            second: Box::new(PaneNode::Leaf {
                pane_id: 2,
                session_id: None,
            }),
        };
        assert!(matches!(
            remove_pane(tree, 2),
            Some(PaneNode::Leaf { pane_id: 1, .. })
        ));
    }
}
