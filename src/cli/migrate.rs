//! Migration from other AI agent tools (OpenClaw, Hermes).
//!
//! Instead of building brittle parsers for every config format (JSON5, YAML),
//! we discover source instances on the filesystem, show an interactive picker,
//! then spawn an OpenCrabs agent to handle the actual migration. The agent reads
//! the source files, understands the format, and maps everything to OpenCrabs
//! brain files and config. Zero maintenance when source tools change format.

use anyhow::{Context, Result, bail};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::args::MigrationSource;

/// A discovered instance of a source tool on the filesystem.
struct Instance {
    path: PathBuf,
    label: String,
}

/// Scan the system for instances of the given source tool.
fn discover(source: MigrationSource) -> Vec<Instance> {
    let home = dirs::home_dir().unwrap_or_default();
    let mut found = Vec::new();

    match source {
        MigrationSource::Openclaw => {
            // Default home directory
            let default = home.join(".openclaw");
            if looks_like_openclaw(&default) {
                found.push(Instance {
                    path: default,
                    label: "default".into(),
                });
            }
            // OPENCLAW_CONFIG_PATH env var
            if let Ok(p) = std::env::var("OPENCLAW_CONFIG_PATH") {
                let dir = config_dir_from_path(&p);
                if looks_like_openclaw(&dir) && !found.iter().any(|i| i.path == dir) {
                    found.push(Instance {
                        path: dir,
                        label: "OPENCLAW_CONFIG_PATH".into(),
                    });
                }
            }
            // OPENCLAW_WORKSPACE_DIR env var (workspace parent)
            if let Ok(p) = std::env::var("OPENCLAW_WORKSPACE_DIR") {
                let ws = PathBuf::from(&p);
                let dir = if ws.join("openclaw.json").exists() {
                    ws.clone()
                } else if let Some(parent) = ws.parent() {
                    parent.to_path_buf()
                } else {
                    ws.clone()
                };
                if looks_like_openclaw(&dir) && !found.iter().any(|i| i.path == dir) {
                    found.push(Instance {
                        path: dir,
                        label: "OPENCLAW_WORKSPACE_DIR".into(),
                    });
                }
            }
        }
        MigrationSource::Hermes => {
            // Default home directory
            let default = home.join(".hermes");
            if looks_like_hermes(&default) {
                found.push(Instance {
                    path: default,
                    label: "default".into(),
                });
            }
            // HERMES_HOME env var
            if let Ok(p) = std::env::var("HERMES_HOME") {
                let dir = PathBuf::from(p);
                if looks_like_hermes(&dir) && !found.iter().any(|i| i.path == dir) {
                    found.push(Instance {
                        path: dir,
                        label: "HERMES_HOME".into(),
                    });
                }
            }
        }
    }

    found
}

fn looks_like_openclaw(dir: &Path) -> bool {
    dir.join("openclaw.json").exists()
}

fn looks_like_hermes(dir: &Path) -> bool {
    dir.join("config.yaml").exists() || dir.join("SOUL.md").exists()
}

/// Extract the directory from a config file path (if it's a file, take parent).
fn config_dir_from_path(p: &str) -> PathBuf {
    let pb = PathBuf::from(p);
    if pb.is_file() {
        pb.parent().unwrap_or(&pb).to_path_buf()
    } else {
        pb
    }
}

/// If multiple instances found, show a numbered list and let the user pick one.
fn pick(instances: &[Instance]) -> Result<&Instance> {
    if instances.is_empty() {
        bail!("no instances found");
    }
    if instances.len() == 1 {
        println!("  Found 1 instance: {}", instances[0].path.display());
        return Ok(&instances[0]);
    }

    println!("\n  Found {} instances:\n", instances.len());
    for (i, inst) in instances.iter().enumerate() {
        println!("  [{}] {} ({})", i + 1, inst.path.display(), inst.label);
    }
    print!("\n  Select [1-{}]: ", instances.len());
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .context("failed to read input")?;
    let n: usize = input.trim().parse().context("enter a number")?;
    if n == 0 || n > instances.len() {
        bail!("pick between 1 and {}", instances.len());
    }
    Ok(&instances[n - 1])
}

/// Scan the source directory for migratable files and return a label inventory.
fn inventory(path: &Path, source: MigrationSource) -> Vec<String> {
    let mut items = Vec::new();

    match source {
        MigrationSource::Openclaw => {
            let config = path.join("openclaw.json");
            if config.exists() {
                items.push("config (openclaw.json)".into());
            }
            let ws = path.join("workspace");
            for name in [
                "SOUL.md",
                "USER.md",
                "MEMORY.md",
                "AGENTS.md",
                "TOOLS.md",
                "IDENTITY.md",
            ] {
                if ws.join(name).exists() {
                    items.push(format!("brain/{name}"));
                }
            }
            let mem = ws.join("memory");
            if mem.exists() {
                let n = std::fs::read_dir(&mem)
                    .map(|rd| {
                        rd.flatten()
                            .filter(|e| e.path().extension().is_some_and(|x| x == "md"))
                            .count()
                    })
                    .unwrap_or(0);
                if n > 0 {
                    items.push(format!("memory/ ({n} daily logs)"));
                }
            }
            let skills = path.join("skills");
            if skills.exists() {
                let n = std::fs::read_dir(&skills)
                    .map(|rd| rd.flatten().count())
                    .unwrap_or(0);
                if n > 0 {
                    items.push(format!("skills/ ({n} skills)"));
                }
            }
        }
        MigrationSource::Hermes => {
            if path.join("config.yaml").exists() {
                items.push("config (config.yaml)".into());
            }
            if path.join(".env").exists() {
                items.push("secrets (.env)".into());
            }
            if path.join("SOUL.md").exists() {
                items.push("brain/SOUL.md".into());
            }
            let mem = path.join("memories");
            for name in ["MEMORY.md", "USER.md"] {
                if mem.join(name).exists() {
                    items.push(format!("brain/{name}"));
                }
            }
            let skills = path.join("skills");
            if skills.exists() {
                let n = std::fs::read_dir(&skills)
                    .map(|rd| rd.flatten().count())
                    .unwrap_or(0);
                if n > 0 {
                    items.push(format!("skills/ ({n} skills)"));
                }
            }
            if path.join("cron").join("jobs.json").exists() {
                items.push("cron/jobs.json".into());
            }
        }
    }

    items
}

/// Build the migration prompt for the agent.
fn build_prompt(source: MigrationSource, src_path: &Path, items: &[String]) -> String {
    let brain_path = crate::brain::prompt_builder::BrainLoader::resolve_path();
    let source_name = match source {
        MigrationSource::Openclaw => "OpenClaw",
        MigrationSource::Hermes => "Hermes",
    };

    let file_list: String = items
        .iter()
        .map(|s| format!("  - {s}"))
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        "You are migrating from {source_name} to OpenCrabs.\n\n\
         ## Source\n\
         Path: {}\n\
         Files found:\n\
         {file_list}\n\n\
         ## Destination (your brain files)\n\
         Brain path: {}\n\n\
         ## Instructions\n\
         1. Read each source file listed above using read_file\n\
         2. Read your current brain files at the destination to see what already exists\n\
         3. Migrate brain files (SOUL.md, USER.md, MEMORY.md, AGENTS.md, TOOLS.md):\n\
            copy content from source to destination. If a file already has content,\n\
            merge or append rather than overwrite.\n\
         4. Migrate config: extract provider/model settings and channel config from the\n\
            source config, then update config.toml using the config tool\n\
         5. Migrate API keys and secrets to keys.toml if found\n\
         6. Copy memory logs and skills if present\n\
         7. Report what you migrated, what you skipped, and any manual steps needed\n\n\
         Use your tools (read_file, write_file, edit_file, config) to do the actual work.\n\
         Be thorough but careful: read before writing, preserve existing OpenCrabs content.",
        src_path.display(),
        brain_path.display(),
    )
}

/// Brain files to track during migration verification.
const BRAIN_FILES: &[&str] = &[
    "SOUL.md",
    "USER.md",
    "MEMORY.md",
    "AGENTS.md",
    "TOOLS.md",
    "CODE.md",
    "SECURITY.md",
];

/// Snapshot of a brain file's metadata before migration.
struct FileSnap {
    exists: bool,
    size: u64,
    modified: Option<SystemTime>,
}

/// Record metadata for all brain files before the agent runs.
fn snapshot_brain_files(brain_path: &Path) -> Vec<FileSnap> {
    BRAIN_FILES
        .iter()
        .map(|name| {
            let meta = brain_path.join(name).metadata().ok();
            FileSnap {
                exists: meta.is_some(),
                size: meta.as_ref().map(|m| m.len()).unwrap_or(0),
                modified: meta.and_then(|m| m.modified().ok()),
            }
        })
        .collect()
}

/// Compare before/after snapshots and print verification summary.
fn verify_migration(before: &[FileSnap], brain_path: &Path) {
    println!("\n  📋 Post-migration verification:\n");
    let mut updated = 0u32;
    let mut unchanged = 0u32;
    let mut created = 0u32;

    for (i, name) in BRAIN_FILES.iter().enumerate() {
        let meta = brain_path.join(name).metadata().ok();
        let exists_now = meta.is_some();
        let size_now = meta.as_ref().map(|m| m.len()).unwrap_or(0);
        let modified_now = meta.as_ref().and_then(|m| m.modified().ok());

        let snap = &before[i];
        if exists_now && !snap.exists {
            println!("    ✅ {name} — created ({size_now} bytes)");
            created += 1;
        } else if exists_now
            && (size_now != snap.size || modified_now.is_some_and(|t| Some(t) != snap.modified))
        {
            println!("    ✅ {name} — updated ({size_now} bytes)");
            updated += 1;
        } else if exists_now {
            println!("    ⏭  {name} — unchanged");
            unchanged += 1;
        }
    }

    let total = created + updated;
    println!();
    if total > 0 {
        println!(
            "  ✅ Migration verified: {total} file{} modified ({created} new, {updated} updated, {unchanged} unchanged)",
            if total == 1 { "" } else { "s" }
        );
    } else {
        println!("  ⚠  No brain files changed. Check the agent output above for errors.");
    }
}

/// Entry point for `opencrabs migrate <source>`.
pub(crate) async fn cmd_migrate(source: MigrationSource, dry_run: bool) -> Result<()> {
    let source_name = match source {
        MigrationSource::Openclaw => "OpenClaw",
        MigrationSource::Hermes => "Hermes",
    };
    let source_dir = match source {
        MigrationSource::Openclaw => ".openclaw",
        MigrationSource::Hermes => ".hermes",
    };

    // 1. Discover instances on the filesystem
    let instances = discover(source);
    if instances.is_empty() {
        bail!(
            "No {source_name} instances found on this system.\n\
             Expected ~/{source_dir}/ with a config file."
        );
    }

    // 2. Interactive picker (or auto-select if only one)
    let instance = pick(&instances)?;

    // 3. Scan for migratable files
    let items = inventory(&instance.path, source);
    if items.is_empty() {
        bail!("No migratable files found at {}", instance.path.display());
    }

    // 4. Show preview
    println!("\n🦀 {source_name} → OpenCrabs Migration\n");
    println!("  Source: {}", instance.path.display());
    println!("  Files:");
    for item in &items {
        println!("    • {item}");
    }

    if dry_run {
        println!("\n  (dry run, no changes made)");
        return Ok(());
    }

    // 5. Snapshot brain files before migration
    let brain_path = crate::brain::prompt_builder::BrainLoader::resolve_path();
    let before = snapshot_brain_files(&brain_path);

    // 6. Build prompt and spawn agent to do the actual migration
    let prompt = build_prompt(source, &instance.path, &items);
    println!("\n  Spawning agent to handle migration...\n");

    let config = super::commands::load_config(None).await?;
    let result = super::commands::cmd_run(
        &config,
        prompt,
        true,
        super::args::OutputFormat::Text,
        None,
        false,
    )
    .await;

    // 7. Verify what actually changed (runs even if agent had errors)
    verify_migration(&before, &brain_path);

    result
}
