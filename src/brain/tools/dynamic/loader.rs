//! Dynamic Tool Loader — reads/writes tools.toml, registers into ToolRegistry.

use super::tool::{DynamicTool, DynamicToolDef, DynamicToolsConfig};
use crate::brain::tools::ToolRegistry;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub struct DynamicToolLoader;

impl DynamicToolLoader {
    /// Resolve the tools.toml path: profile-aware first, fallback to global.
    ///
    /// Option C from #157 — backward-compatible with existing global setups
    /// while supporting per-profile isolation.
    pub fn default_path() -> Option<PathBuf> {
        let profile_path = crate::config::opencrabs_home().join("tools.toml");
        if profile_path.exists() {
            return Some(profile_path);
        }
        dirs::home_dir().map(|h| h.join(".opencrabs").join("tools.toml"))
    }

    pub fn load(path: &Path, registry: &Arc<ToolRegistry>) -> usize {
        // A bad tools.toml must not brick startup — load what we can (nothing,
        // on a parse error) and rely on the ERROR already logged by read_config.
        let config = match Self::read_config(path) {
            Ok(config) => config,
            Err(_) => return 0,
        };
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

    pub fn list_tools_detailed(path: &Path) -> anyhow::Result<Vec<DynamicToolDef>> {
        Ok(Self::read_config(path)?.tools)
    }

    pub fn add_tool(
        path: &Path,
        def: DynamicToolDef,
        registry: &Arc<ToolRegistry>,
    ) -> anyhow::Result<()> {
        // Propagate a parse error rather than starting from an empty config —
        // otherwise write_config below would overwrite a valid-but-unparseable
        // tools.toml and destroy every existing tool (issue #235).
        let mut config = Self::read_config(path)?;
        let name = def.name.clone();

        // A dynamic tool must not shadow a core tool (OC-06). `add_tool`
        // unregisters any existing tool of that name before registering, so a
        // dynamic tool named `bash` would replace the real, blocklist-enforcing
        // one with a model-authored, unguarded stand-in. Reserve the names of
        // the security-critical execution and file tools.
        const RESERVED: &[&str] = &[
            "bash",
            "execute_code",
            "read_file",
            "write_file",
            "edit_file",
            "http_request",
            "web_scrape",
            "evolve",
            "rebuild",
            "cron_manage",
            "tool_manage",
        ];
        if RESERVED.contains(&name.as_str()) {
            anyhow::bail!(
                "'{name}' is a reserved core tool name and cannot be redefined as a dynamic tool"
            );
        }

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
        let mut config = Self::read_config(path)?;
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
        let mut config = Self::read_config(path)?;
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

    pub fn reload(path: &Path, registry: &Arc<ToolRegistry>) -> anyhow::Result<usize> {
        // Read BEFORE unregistering anything: on a parse error we return early
        // and leave the currently-loaded tools untouched, so a bad edit can't
        // silently strip the live registry down to zero (issue #235).
        let config = Self::read_config(path)?;
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
        Ok(count)
    }

    /// Read and parse `tools.toml`.
    ///
    /// A MISSING file is not an error — it means "no dynamic tools yet", so we
    /// return an empty config. A PARSE error (e.g. a duplicate key) is fatal to
    /// the whole file: serde drops every `[[tools]]` entry, so a single typo
    /// silently wipes all dynamic tools. That used to be swallowed by
    /// `unwrap_or_default()`; now it is logged at ERROR and returned as `Err`
    /// so callers can refuse to overwrite the file and can tell the user
    /// (issue #235).
    fn read_config(path: &Path) -> anyhow::Result<DynamicToolsConfig> {
        let content = match std::fs::read_to_string(path) {
            Ok(content) => content,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(DynamicToolsConfig::default());
            }
            Err(e) => {
                tracing::error!("tools: cannot read {}: {e}", path.display());
                return Err(anyhow::anyhow!("failed to read {}: {e}", path.display()));
            }
        };
        match toml::from_str(&content) {
            Ok(config) => Ok(config),
            Err(e) => {
                tracing::error!(
                    "tools: {} failed to parse, trying last_good backup: {e}",
                    path.display()
                );
                let backup = path.with_extension("toml.bak");
                if let Ok(backup_content) = std::fs::read_to_string(&backup)
                    && let Ok(backup_config) = toml::from_str::<DynamicToolsConfig>(&backup_content)
                {
                    tracing::warn!(
                        "tools: recovered {} tool(s) from {}",
                        backup_config.tools.len(),
                        backup.display()
                    );
                    return Ok(backup_config);
                }
                Err(anyhow::anyhow!("failed to parse {}: {e}", path.display()))
            }
        }
    }

    fn write_config(path: &Path, config: &DynamicToolsConfig) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Snapshot the current parseable file as last_good backup (#235).
        if path.exists() {
            let backup = path.with_extension("toml.bak");
            if let Ok(current) = std::fs::read_to_string(path)
                && toml::from_str::<DynamicToolsConfig>(&current).is_ok()
            {
                let _ = std::fs::copy(path, &backup);
            }
        }
        std::fs::write(path, toml::to_string_pretty(config)?)?;
        Ok(())
    }
}
