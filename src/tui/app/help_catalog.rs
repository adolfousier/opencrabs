//! The command catalogue behind the help screen (#1530, #1527).
//!
//! The help screen used to build its command list inline in the renderer,
//! which had two costs. It called `load_all_skills()` and `CommandLoader::load()`
//! on EVERY frame — each one a directory walk plus a parse — so holding the
//! help screen open re-read the disk dozens of times a second. And because the
//! rows only existed inside the draw call, nothing outside it knew how many
//! there were, so scroll had nothing to clamp against.
//!
//! Collecting the rows once on entry fixes both: the disk is read a single
//! time per visit, and the row count is available to the key handler.

/// Where a row came from. Decides which header it renders under, and is what
/// keeps a filtered view from printing a section header with nothing beneath.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum HelpSection {
    BuiltIn,
    Channel,
    Skill,
    Custom,
}

impl HelpSection {
    /// Header text, matching what the screen printed before this module
    /// existed so the layout is unchanged when no filter is active.
    pub fn title(self) -> &'static str {
        match self {
            HelpSection::BuiltIn => "SLASH COMMANDS",
            HelpSection::Channel => "CHANNEL COMMANDS (Telegram/Discord/Slack)",
            HelpSection::Skill => "SKILLS",
            HelpSection::Custom => "CUSTOM COMMANDS",
        }
    }

    /// Render order. Fixed rather than derived from the data so a filter
    /// cannot reorder the screen under the reader.
    pub fn all() -> [HelpSection; 4] {
        [
            HelpSection::BuiltIn,
            HelpSection::Channel,
            HelpSection::Skill,
            HelpSection::Custom,
        ]
    }
}

/// One searchable command row.
#[derive(Debug, Clone)]
pub struct HelpRow {
    pub name: String,
    pub description: String,
    pub section: HelpSection,
}

/// Collect every command the help screen can show, in render order.
///
/// Hits the disk for skills and `commands.toml`, so call it on entering the
/// help screen rather than per frame.
pub fn load() -> Vec<HelpRow> {
    let mut rows = Vec::new();

    for cmd in super::SLASH_COMMANDS {
        rows.push(HelpRow {
            name: cmd.name.to_string(),
            description: cmd.description.to_string(),
            section: HelpSection::BuiltIn,
        });
    }

    // Documented here so they are discoverable, but kept out of the TUI
    // autocomplete because the TUI slash dispatcher does not handle them.
    for cmd in super::CHANNEL_COMMANDS {
        rows.push(HelpRow {
            name: cmd.name.to_string(),
            description: cmd.description.to_string(),
            section: HelpSection::Channel,
        });
    }

    for skill in crate::brain::skills::load_all_skills() {
        rows.push(HelpRow {
            name: skill.slash_name.clone(),
            description: skill.description.clone(),
            section: HelpSection::Skill,
        });
    }

    let brain_path = crate::brain::BrainLoader::resolve_path();
    let mut user_cmds = crate::brain::CommandLoader::from_brain_path(&brain_path).load();
    user_cmds.sort_by(|a, b| a.name.cmp(&b.name));
    for cmd in user_cmds {
        rows.push(HelpRow {
            name: cmd.name,
            description: cmd.description,
            section: HelpSection::Custom,
        });
    }

    rows
}

/// Rows belonging to `section`, in catalogue order.
pub fn section_rows(rows: &[HelpRow], section: HelpSection) -> Vec<&HelpRow> {
    rows.iter().filter(|r| r.section == section).collect()
}

/// Largest scroll offset that still leaves content on screen.
///
/// The help screen used to call `saturating_add(1)` with no ceiling, so
/// holding the down key wound the offset past the end and the reader was left
/// staring at blank rows with no cue for how far back to scroll. Clamping to
/// `content - viewport` keeps the last row visible at maximum scroll.
///
/// A viewport at least as tall as the content means there is nothing to
/// scroll, so the answer is 0.
pub fn max_scroll(content_rows: usize, viewport_rows: usize) -> usize {
    content_rows.saturating_sub(viewport_rows)
}
