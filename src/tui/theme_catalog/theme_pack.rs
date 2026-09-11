//! Curated theme pack (#1461): generated TOML files under `pack/`, embedded
//! into the binary and served as built-in themes alongside the hand-built
//! presets in `render::presets`.
//!
//! The pack is built once through the EXISTING user-theme validator
//! (`user_themes::build_theme`), so embedded themes are held to exactly the
//! standard a `~/.opencrabs/themes/` drop-in file is: all 43 keys, valid
//! hex, 2.5:1 WCAG contrast floor on the 8 readability pairs. Regeneration
//! instructions live in `pack/README.md`; per-theme provenance in each
//! generated file's `# source:` header.
//!
//! Two invariants keep the wiring safe:
//! - `build_theme` rejects stems colliding with `presets::by_name`, and
//!   `by_name` consults this pack. During pack initialization
//!   `PACK_THEMES.get()` is `None` (`OnceLock::get` never forces), so the
//!   collision check sees the pack's own names as not-yet-built-in and
//!   cannot re-enter the initializer. This is why the pack is an
//!   `OnceLock` + non-forcing `themes_if_built()`, NOT a `LazyLock`, and
//!   why `presets::by_name` must never call the forcing `built_ins()`.
//! - `src/tests/theme_pack_test.rs` proves every embedded file passes
//!   `build_theme`, so the runtime skip branch below is unreachable; a
//!   corrupted embed degrades to fewer themes, never a panic in the TUI.

use std::sync::OnceLock;

use crate::tui::render::theme::Theme;
use crate::tui::render::user_themes::build_theme;

/// (pack name, embedded TOML). Names equal the `pack/` file stems and the
/// `/theme <name>` handles. Order = `/theme list` order after the hand-
/// built presets: opencode-sourced themes alphabetically, then the
/// alacritty complements alphabetically. `lucent-orng` is deliberately
/// absent (transparent background has no `ink` mapping — see pack/README).
const PACK_SOURCES: &[(&str, &str)] = &[
    ("aura", include_str!("pack/aura.toml")),
    ("ayu", include_str!("pack/ayu.toml")),
    ("carbonfox", include_str!("pack/carbonfox.toml")),
    (
        "catppuccin-macchiato",
        include_str!("pack/catppuccin-macchiato.toml"),
    ),
    ("cobalt2", include_str!("pack/cobalt2.toml")),
    ("cursor", include_str!("pack/cursor.toml")),
    ("everforest", include_str!("pack/everforest.toml")),
    ("flexoki", include_str!("pack/flexoki.toml")),
    ("github", include_str!("pack/github.toml")),
    ("gruvbox", include_str!("pack/gruvbox.toml")),
    ("kanagawa", include_str!("pack/kanagawa.toml")),
    ("material", include_str!("pack/material.toml")),
    ("matrix", include_str!("pack/matrix.toml")),
    ("mercury", include_str!("pack/mercury.toml")),
    ("nightowl", include_str!("pack/nightowl.toml")),
    ("nord", include_str!("pack/nord.toml")),
    ("one-dark", include_str!("pack/one-dark.toml")),
    ("opencode", include_str!("pack/opencode.toml")),
    ("orng", include_str!("pack/orng.toml")),
    ("osaka-jade", include_str!("pack/osaka-jade.toml")),
    ("palenight", include_str!("pack/palenight.toml")),
    ("rosepine", include_str!("pack/rosepine.toml")),
    ("synthwave84", include_str!("pack/synthwave84.toml")),
    ("tokyonight", include_str!("pack/tokyonight.toml")),
    ("vercel", include_str!("pack/vercel.toml")),
    ("vesper", include_str!("pack/vesper.toml")),
    ("zenburn", include_str!("pack/zenburn.toml")),
    (
        "catppuccin-frappe",
        include_str!("pack/catppuccin-frappe.toml"),
    ),
    ("gruvbox-light", include_str!("pack/gruvbox-light.toml")),
    ("rosepine-dawn", include_str!("pack/rosepine-dawn.toml")),
    (
        "tokyonight-storm",
        include_str!("pack/tokyonight-storm.toml"),
    ),
];

static PACK_THEMES: OnceLock<Vec<Theme>> = OnceLock::new();

/// Every embedded pack theme, built once through the validator.
pub fn themes() -> &'static [Theme] {
    PACK_THEMES.get_or_init(|| {
        PACK_SOURCES
            .iter()
            .filter_map(|(name, toml)| match build_theme(name, toml) {
                Ok(theme) => Some(theme),
                // Unreachable while the pack round-trip test is green; a
                // broken embed must degrade, never panic the TUI.
                Err(e) => {
                    tracing::warn!(
                        theme = name,
                        error = %e,
                        "embedded pack theme failed validation; skipping"
                    );
                    None
                }
            })
            .collect()
    })
}

/// Non-forcing view for `presets::by_name`: `Some` once the pack is built,
/// `None` during initialization — which is exactly what keeps the
/// validator's collision check from re-entering `themes()`.
pub fn themes_if_built() -> Option<&'static [Theme]> {
    PACK_THEMES.get().map(Vec::as_slice)
}

/// The raw (name, TOML) table, for the pack round-trip test.
pub fn sources() -> &'static [(&'static str, &'static str)] {
    PACK_SOURCES
}
