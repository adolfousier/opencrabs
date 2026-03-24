//! Dynamic Tool Loader — reads/writes tools.toml, registers into ToolRegistry.

use super::tool::{DynamicTool, DynamicToolDef, DynamicToolsConfig};
use crate::brain::tools::ToolRegistry;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub struct DynamicToolLoader;

impl DynamicToolLoader {
    pub fn default_path() -> Option<PathBuf> {
        dirs::home_dir().map(|h| h.join(".opencrabs").join("tools.toml"))
    }

    pub fn load(path: &Path, registry: &Arc<ToolRegistry>) -> usize {
        let config = Self::read_config(path);
        let mut count = 0;
        for def in config.tools {
            if !def.enabled {
                continue;
            }
            if registry.has_tool(&def.name) {
                continue;
            }
            let name = def.name.clone();
            registry.register(Arc::new(DynamicTool::new(def)));
            count += 1;
            tracing::info!("Registered dynamic tool: {name}");
        }
        if count > 0 {
            tracing::info!("Loaded {count} dynamic tool(s) from {}", path.display());
        }
        count
    }

    pub fn list_tools_detailed(path: &Path) -> Vec<DynamicToolDef> {
        Self::read_config(path).tools
    }

    pub fn add_tool(
        path: &Path,
        def: DynamicToolDef,
        registry: &Arc<ToolRegistry>,
    ) -> anyhow::Result<()> {
        let mut config = Self::read_config(path);
        let name = def.name.clone();
        config.tools.retain(|d| d.name != name);
        let should_register = def.enabled;
        config.tools.push(def.clone());
        Self::write_config(path, &config)?;
        registry.unregister(&name);
        if should_register {
            registry.register(Arc::new(DynamicTool::new(def)));
        }
        tracing::info!("Added dynamic tool: {name}");
        Ok(())
    }

    pub fn remove_tool(
        path: &Path,
        name: &str,
        registry: &Arc<ToolRegistry>,
    ) -> anyhow::Result<bool> {
        let mut config = Self::read_config(path);
        let before = config.tools.len();
        config.tools.retain(|d| d.name != name);
        let removed = config.tools.len() < before;
        if removed {
            Self::write_config(path, &config)?;
            registry.unregister(name);
            tracing::info!("Removed dynamic tool: {name}");
        }
        Ok(removed)
    }

    pub fn set_enabled(
        path: &Path,
        name: &str,
        enabled: bool,
        registry: &Arc<ToolRegistry>,
    ) -> anyhow::Result<bool> {
        let mut config = Self::read_config(path);
        let found = config.tools.iter_mut().find(|d| d.name == name);
        match found {
            Some(def) => {
                def.enabled = enabled;
                let def_clone = def.clone();
                Self::write_config(path, &config)?;
                if enabled {
                    registry.unregister(name);
                    registry.register(Arc::new(DynamicTool::new(def_clone)));
                } else {
                    registry.unregister(name);
                }
                Ok(true)
            }
            None => Ok(false),
        }
    }

    pub fn reload(path: &Path, registry: &Arc<ToolRegistry>) -> usize {
        let config = Self::read_config(path);
        for def in &config.tools {
            registry.unregister(&def.name);
        }
        let mut count = 0;
        for def in config.tools {
            if def.enabled {
                registry.register(Arc::new(DynamicTool::new(def)));
                count += 1;
            }
        }
        tracing::info!("Reloaded {count} dynamic tool(s) from {}", path.display());
        count
    }

    fn read_config(path: &Path) -> DynamicToolsConfig {
        match std::fs::read_to_string(path) {
            Ok(content) => toml::from_str(&content).unwrap_or_default(),
            Err(_) => DynamicToolsConfig::default(),
        }
    }

    fn write_config(path: &Path, config: &DynamicToolsConfig) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, toml::to_string_pretty(config)?)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::tool::{ExecutorType, ParamDef};
    use super::*;
    use std::collections::HashMap;
    use tempfile::TempDir;

    fn tmp_path() -> (TempDir, PathBuf) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("tools.toml");
        (dir, path)
    }

    #[test]
    fn test_load_nonexistent() {
        let reg = Arc::new(ToolRegistry::new());
        assert_eq!(DynamicToolLoader::load(Path::new("/nonexistent"), &reg), 0);
    }

    #[test]
    fn test_add_and_list() {
        let (_dir, path) = tmp_path();
        let reg = Arc::new(ToolRegistry::new());
        let def = DynamicToolDef {
            name: "ping".into(),
            description: "Ping".into(),
            executor: ExecutorType::Shell,
            enabled: true,
            requires_approval: false,
            method: None,
            url: None,
            headers: HashMap::new(),
            timeout_secs: 10,
            command: Some("ping -c 1 {{host}}".into()),
            params: vec![ParamDef {
                name: "host".into(),
                param_type: "string".into(),
                description: "".into(),
                required: true,
                default: None,
            }],
        };
        DynamicToolLoader::add_tool(&path, def, &reg).unwrap();
        assert!(reg.has_tool("ping"));
        assert_eq!(DynamicToolLoader::list_tools_detailed(&path).len(), 1);
    }

    #[test]
    fn test_remove() {
        let (_dir, path) = tmp_path();
        let reg = Arc::new(ToolRegistry::new());
        let def = DynamicToolDef {
            name: "rm_me".into(),
            description: "".into(),
            executor: ExecutorType::Shell,
            enabled: true,
            requires_approval: false,
            method: None,
            url: None,
            headers: HashMap::new(),
            timeout_secs: 10,
            command: Some("echo".into()),
            params: vec![],
        };
        DynamicToolLoader::add_tool(&path, def, &reg).unwrap();
        assert!(DynamicToolLoader::remove_tool(&path, "rm_me", &reg).unwrap());
        assert!(!reg.has_tool("rm_me"));
    }

    #[test]
    fn test_enable_disable() {
        let (_dir, path) = tmp_path();
        let reg = Arc::new(ToolRegistry::new());
        let def = DynamicToolDef {
            name: "tog".into(),
            description: "".into(),
            executor: ExecutorType::Shell,
            enabled: true,
            requires_approval: false,
            method: None,
            url: None,
            headers: HashMap::new(),
            timeout_secs: 10,
            command: Some("echo".into()),
            params: vec![],
        };
        DynamicToolLoader::add_tool(&path, def, &reg).unwrap();
        DynamicToolLoader::set_enabled(&path, "tog", false, &reg).unwrap();
        assert!(!reg.has_tool("tog"));
        DynamicToolLoader::set_enabled(&path, "tog", true, &reg).unwrap();
        assert!(reg.has_tool("tog"));
    }

    #[test]
    fn test_reload() {
        let (_dir, path) = tmp_path();
        let reg = Arc::new(ToolRegistry::new());
        std::fs::write(&path, "[[tools]]\nname = \"disk\"\ndescription = \"From disk\"\nexecutor = \"shell\"\ncommand = \"echo\"").unwrap();
        assert_eq!(DynamicToolLoader::reload(&path, &reg), 1);
        assert!(reg.has_tool("disk"));
    }

    #[test]
    fn test_disabled_not_registered() {
        let (_dir, path) = tmp_path();
        let reg = Arc::new(ToolRegistry::new());
        std::fs::write(&path, "[[tools]]\nname = \"on\"\ndescription = \"\"\nexecutor = \"shell\"\ncommand = \"echo\"\n\n[[tools]]\nname = \"off\"\ndescription = \"\"\nexecutor = \"shell\"\ncommand = \"echo\"\nenabled = false").unwrap();
        assert_eq!(DynamicToolLoader::load(&path, &reg), 1);
        assert!(reg.has_tool("on"));
        assert!(!reg.has_tool("off"));
    }
}
