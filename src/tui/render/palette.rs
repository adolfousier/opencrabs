//! Shared TUI palette + style helpers.
//!
//! Brand-level colours — orange, teal, white — used by every dialog
//! that wants visual continuity with the canonical OpenCrabs look in
//! `sessions.rs` and `usage/dashboard.rs`. Panel-specific aliases
//! (e.g. `BORDER_INBOX_FOCUS`) live in their owning module's local
//! theme file.

use ratatui::style::{Color, Modifier, Style};

// ── Brand palette ──────────────────────────────────────────────────────────

/// Crab orange — primary brand colour, used for titles and the
/// "active" / "warn" accent in panels that adopt it.
pub const ORANGE: Color = Color::Rgb(215, 100, 20);
/// Teal accent — primary action / "selected" colour.
pub const TEAL: Color = Color::Cyan;
/// Soft white — passive / informational accent.
pub const WHITE: Color = Color::Rgb(220, 220, 220);

// ── Text ────────────────────────────────────────────────────────────────────

pub const TEXT_PRIMARY: Color = Color::Rgb(200, 200, 210);
pub const TEXT_SECONDARY: Color = Color::Rgb(140, 140, 160);
pub const TEXT_MUTED: Color = Color::Rgb(80, 80, 100);
pub const TEXT_DIM: Color = Color::Rgb(60, 60, 80);

// ── Neutral ladder ──────────────────────────────────────────────────────────
//
// One const per distinct gray in the tree. Do NOT merge near-neighbours:
// byte-identical rendering is the S1 contract (#1363); value unification is
// theme work (S2), not refactor work.

/// Neutral chrome — borders, passive labels, `ErrorSeverity::Info`.
pub const GRAY: Color = Color::Rgb(120, 120, 120);
/// Dimmer chrome — secondary borders, quiet panel text.
pub const GRAY_MID: Color = Color::Rgb(100, 100, 100);
/// Detail rows under a primary row (tool list defaults).
pub const GRAY_DETAIL: Color = Color::Rgb(90, 90, 90);
/// Inactive borders (input frame when unfocused).
pub const GRAY_DIM: Color = Color::Rgb(80, 80, 80);
/// Deeper inactive chrome.
pub const GRAY_DARK: Color = Color::Rgb(70, 70, 70);
/// Darkest neutral fg (plan-widget borders).
pub const GRAY_BASE: Color = Color::Rgb(50, 50, 50);
/// Bright neutral text.
pub const GRAY_LIGHT: Color = Color::Rgb(200, 200, 200);
/// Soft label text.
pub const GRAY_SOFT: Color = Color::Rgb(170, 170, 170);
/// Muted body text (plan widget).
pub const GRAY_MUTED: Color = Color::Rgb(160, 160, 160);

// ── Status ──────────────────────────────────────────────────────────────────

/// Success green — active pane borders, "ok" states.
pub const SUCCESS: Color = Color::Rgb(80, 200, 120);
/// Analytics dashboard green — bars, values, panel focus accent.
pub const ANALYTICS_GREEN: Color = Color::Rgb(46, 204, 113);
/// Checked-checkbox marker in markdown rendering.
pub const GREEN_CHECK: Color = Color::Rgb(120, 200, 120);
/// Error red — primary failures.
pub const ERROR: Color = Color::Rgb(220, 80, 80);
/// Error red, softer variant (chat error titles).
pub const ERROR_SOFT: Color = Color::Rgb(220, 70, 70);
/// Error red, faded variant (chat error bodies).
pub const ERROR_FADED: Color = Color::Rgb(220, 130, 130);
/// Warning amber (input hints).
pub const WARNING: Color = Color::Rgb(230, 180, 80);
/// Warning amber, muted variant.
pub const WARNING_MUTED: Color = Color::Rgb(200, 170, 60);
/// Amber for proposed-skill inbox badges.
pub const AMBER_MUTED: Color = Color::Rgb(190, 160, 70);

// ── Teal accents ────────────────────────────────────────────────────────────

/// Vivid teal — selection/active accent in input + tools.
pub const TEAL_VIVID: Color = Color::Rgb(60, 185, 185);
/// Bright teal (dialogs).
pub const TEAL_BRIGHT: Color = Color::Rgb(60, 190, 190);
/// Muted teal — completed/skipped plan tasks.
pub const TEAL_MUTED: Color = Color::Rgb(60, 165, 165);
/// Calm teal — plan progress bars.
pub const TEAL_CALM: Color = Color::Rgb(80, 175, 175);

// ── Blues ───────────────────────────────────────────────────────────────────

/// Slate blue — help text, input placeholders.
pub const BLUE_SLATE: Color = Color::Rgb(90, 110, 150);
/// Steel blue — informational accents (sessions, files, projects).
pub const BLUE_STEEL: Color = Color::Rgb(100, 140, 180);
/// Link blue (markdown links).
pub const BLUE_LINK: Color = Color::Rgb(90, 160, 230);
/// Sky blue (projects).
pub const BLUE_SKY: Color = Color::Rgb(80, 160, 220);
/// Soft blue (chat accents).
pub const BLUE_SOFT: Color = Color::Rgb(75, 160, 215);
/// Vivid blue — onboarding voice brand accent.
pub const BLUE_VIVID: Color = Color::Rgb(60, 130, 246);
/// Inline-code blue (markdown).
pub const BLUE_CODE: Color = Color::Rgb(125, 150, 195);
/// Selection background in the input composer.
pub const SELECTION_BG: Color = Color::Rgb(50, 80, 160);

// ── Surfaces & ink ──────────────────────────────────────────────────────────

/// Mission-control panel background.
pub const SURFACE_PANEL: Color = Color::Rgb(30, 30, 45);
/// QR-code panel background (onboarding).
pub const SURFACE_QR: Color = Color::Rgb(18, 18, 18);
/// Code-block background (chat).
pub const SURFACE_CODE: Color = Color::Rgb(40, 45, 55);
/// Code-block background, alternate shade (chat light variant).
pub const SURFACE_CODE_ALT: Color = Color::Rgb(40, 44, 56);
/// Near-black ink for fg on bright badge backgrounds.
pub const INK: Color = Color::Rgb(20, 20, 30);

// ── Misc ────────────────────────────────────────────────────────────────────

/// Soft purple — retired proposed-brain-dedup inbox badges (kept for theme compatibility).
pub const PURPLE_SOFT: Color = Color::Rgb(160, 120, 200);

// ── Session badge rotation ──────────────────────────────────────────────────

/// Distinct hues cycled per project so badges stay distinguishable (#1363).
pub const ACCENT_AMBER: Color = Color::Rgb(235, 160, 70);
pub const ACCENT_CYAN: Color = Color::Rgb(80, 200, 200);
pub const ACCENT_TEAL: Color = Color::Rgb(90, 200, 160);
pub const ACCENT_BLUE: Color = Color::Rgb(90, 160, 220);
pub const ACCENT_BLUE_LIGHT: Color = Color::Rgb(150, 190, 240);
pub const ACCENT_PALE: Color = Color::Rgb(180, 210, 230);

/// Stable badge-colour rotation for project badges (order is load-bearing:
/// project_id hashes index into this slice).
pub const PROJECT_BADGE_COLORS: &[Color] = &[
    ORANGE,
    ACCENT_AMBER,
    ACCENT_CYAN,
    ACCENT_TEAL,
    ACCENT_BLUE,
    ACCENT_BLUE_LIGHT,
    WHITE,
    ACCENT_PALE,
];

// ── Helpers ────────────────────────────────────────────────────────────────

pub fn title_style(accent: Color) -> Style {
    Style::default().fg(accent).add_modifier(Modifier::BOLD)
}

pub fn muted() -> Style {
    Style::default().fg(crate::tui::render::theme::role(
        crate::tui::render::theme::Role::TextMuted,
    ))
}

pub fn dim() -> Style {
    Style::default().fg(crate::tui::render::theme::role(
        crate::tui::render::theme::Role::TextDim,
    ))
}
