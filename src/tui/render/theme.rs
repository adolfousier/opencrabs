//! Theme core: semantic color roles + runtime theme resolution.
//!
//! S2 of the theme system (#1364). Render sites resolve colors through
//! [`role`] instead of reading palette consts directly, so `/theme` and
//! `[tui.theme]` can switch the whole UI at runtime. The default
//! `crab-dark` theme is byte-identical to the post-S1 palette by
//! construction: every field IS the palette const.
//!
//! Granularity note: one Role per palette const. Collapsing roles would
//! change crab-dark rendering and break the #1363 byte-identical
//! contract; presets that don't distinguish two roles simply map them
//! to the same upstream hex.
//!
//! Performance: `role()` takes an uncontended RwLock read per call and
//! ratatui calls it a few hundred times per frame, nanoseconds each.
//! Revisit only if a profile says so.
//!
//! Decorative cycles (project badge rotation in `palette`) are
//! deliberately NOT roles: they are preset-agnostic by design.

/// The theme name as configured, tolerating a value that carries its own
/// quote characters.
///
/// `/theme set` and the picker used to hand `write_key_string` a value they
/// had already wrapped in quotes, and `write_key_string` produces the TOML
/// string itself, so the quotes ended up inside the value: `theme = '"dracula"'`
/// (#1428). The write sites are fixed, but a config poisoned before that fix
/// would never match a preset again, and the failure is silent because the
/// lookup simply falls through to the default.
///
/// Only a matching leading and trailing pair is stripped. A name that merely
/// contains a quote is left alone, since that is a lookup miss rather than
/// this bug.
pub fn configured_name(raw: &str) -> &str {
    let trimmed = raw.trim();
    trimmed
        .strip_prefix('"')
        .and_then(|r| r.strip_suffix('"'))
        .unwrap_or(trimmed)
}

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{OnceLock, RwLock};

use ratatui::style::Color;

use super::palette;

/// Semantic color roles, one per themeable palette const.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    Accent,         // palette::ORANGE
    AccentTeal,     // palette::TEAL (ANSI Cyan)
    AccentSoft,     // palette::WHITE
    TextPrimary,    // palette::TEXT_PRIMARY
    TextSecondary,  // palette::TEXT_SECONDARY
    TextMuted,      // palette::TEXT_MUTED
    TextDim,        // palette::TEXT_DIM
    Gray,           // palette::GRAY
    GrayMid,        // palette::GRAY_MID
    GrayDetail,     // palette::GRAY_DETAIL
    GrayDim,        // palette::GRAY_DIM
    GrayDark,       // palette::GRAY_DARK
    GrayBase,       // palette::GRAY_BASE
    GrayLight,      // palette::GRAY_LIGHT
    GraySoft,       // palette::GRAY_SOFT
    GrayMuted,      // palette::GRAY_MUTED
    Success,        // palette::SUCCESS
    AnalyticsGreen, // palette::ANALYTICS_GREEN
    GreenCheck,     // palette::GREEN_CHECK
    Error,          // palette::ERROR
    ErrorSoft,      // palette::ERROR_SOFT
    ErrorFaded,     // palette::ERROR_FADED
    Warning,        // palette::WARNING
    WarningMuted,   // palette::WARNING_MUTED
    AmberMuted,     // palette::AMBER_MUTED
    TealVivid,      // palette::TEAL_VIVID
    TealBright,     // palette::TEAL_BRIGHT
    TealMuted,      // palette::TEAL_MUTED
    TealCalm,       // palette::TEAL_CALM
    BlueSlate,      // palette::BLUE_SLATE
    BlueSteel,      // palette::BLUE_STEEL
    BlueLink,       // palette::BLUE_LINK
    BlueSky,        // palette::BLUE_SKY
    BlueSoft,       // palette::BLUE_SOFT
    BlueVivid,      // palette::BLUE_VIVID
    BlueCode,       // palette::BLUE_CODE
    SelectionBg,    // palette::SELECTION_BG
    SurfacePanel,   // palette::SURFACE_PANEL
    SurfaceQr,      // palette::SURFACE_QR
    SurfaceCode,    // palette::SURFACE_CODE
    SurfaceCodeAlt, // palette::SURFACE_CODE_ALT
    Ink,            // palette::INK
    PurpleSoft,     // palette::PURPLE_SOFT
}

/// ANSI-256 palette: 43 u8 indices (16..=255) paralleling ThemeColors.
/// Derived from the RGB values via `rgb_to_ansi256` — never hand-written.
/// Used by the truecolor-fallback tier: when the terminal cannot render
/// 24-bit color, roles resolve to the nearest ANSI-256 index.
#[derive(Debug)]
pub struct AnsiColors {
    pub accent: u8,
    pub accent_teal: u8,
    pub accent_soft: u8,
    pub text_primary: u8,
    pub text_secondary: u8,
    pub text_muted: u8,
    pub text_dim: u8,
    pub gray: u8,
    pub gray_mid: u8,
    pub gray_detail: u8,
    pub gray_dim: u8,
    pub gray_dark: u8,
    pub gray_base: u8,
    pub gray_light: u8,
    pub gray_soft: u8,
    pub gray_muted: u8,
    pub success: u8,
    pub analytics_green: u8,
    pub green_check: u8,
    pub error: u8,
    pub error_soft: u8,
    pub error_faded: u8,
    pub warning: u8,
    pub warning_muted: u8,
    pub amber_muted: u8,
    pub teal_vivid: u8,
    pub teal_bright: u8,
    pub teal_muted: u8,
    pub teal_calm: u8,
    pub blue_slate: u8,
    pub blue_steel: u8,
    pub blue_link: u8,
    pub blue_sky: u8,
    pub blue_soft: u8,
    pub blue_vivid: u8,
    pub blue_code: u8,
    pub selection_bg: u8,
    pub surface_panel: u8,
    pub surface_qr: u8,
    pub surface_code: u8,
    pub surface_code_alt: u8,
    pub ink: u8,
    pub purple_soft: u8,
    /// Canvas background, quantized. `None` when the theme declares no
    /// background (see [`ThemeColors::background`]).
    pub background: Option<u8>,
}

/// A complete role-to-color mapping for one theme.
#[derive(Debug)]
pub struct ThemeColors {
    pub accent: Color,
    pub accent_teal: Color,
    pub accent_soft: Color,
    pub text_primary: Color,
    pub text_secondary: Color,
    pub text_muted: Color,
    pub text_dim: Color,
    pub gray: Color,
    pub gray_mid: Color,
    pub gray_detail: Color,
    pub gray_dim: Color,
    pub gray_dark: Color,
    pub gray_base: Color,
    pub gray_light: Color,
    pub gray_soft: Color,
    pub gray_muted: Color,
    pub success: Color,
    pub analytics_green: Color,
    pub green_check: Color,
    pub error: Color,
    pub error_soft: Color,
    pub error_faded: Color,
    pub warning: Color,
    pub warning_muted: Color,
    pub amber_muted: Color,
    pub teal_vivid: Color,
    pub teal_bright: Color,
    pub teal_muted: Color,
    pub teal_calm: Color,
    pub blue_slate: Color,
    pub blue_steel: Color,
    pub blue_link: Color,
    pub blue_sky: Color,
    pub blue_soft: Color,
    pub blue_vivid: Color,
    pub blue_code: Color,
    pub selection_bg: Color,
    pub surface_panel: Color,
    pub surface_qr: Color,
    pub surface_code: Color,
    pub surface_code_alt: Color,
    pub ink: Color,
    pub purple_soft: Color,
    /// The canvas the whole UI is drawn on, when the theme declares one
    /// (#1634).
    ///
    /// Deliberately NOT a [`Role`]: [`ThemeColors::get`] returns a plain
    /// `Color`, and "no background" has to stay expressible. A theme that
    /// declares none leaves the terminal's own background showing, which
    /// is what `crab-dark` does and what keeps terminal transparency and
    /// blur working for anyone relying on them.
    pub background: Option<Color>,
    pub ansi: AnsiColors,
}

/// Quantize an RGB color to the nearest ANSI-256 index.
///
/// Uses the 216-color cube (indices 16–231) with 6 steps per channel:
/// `16 + 36*r + 6*g + b` where r,g,b ∈ 0..5. Steps snap to the nearest
/// of `[0, 95, 135, 175, 215, 255]`. Falls back to the 24 grayscale
/// ramp (232–255) only when the cube distance is worse — a pure neutral
/// gray is better served by a gray step than a saturated cube neighbor.
///
/// Output is the raw ANSI-256 index; callers wrap it in `Color::Indexed`
/// when feeding ratatui.
pub fn rgb_to_ansi256(r: u8, g: u8, b: u8) -> u8 {
    const CUBE_STEPS: [u8; 6] = [0, 95, 135, 175, 215, 255];

    let snap = |v: u8| -> u8 {
        let mut best = 0u8;
        let mut best_d = u16::MAX;
        for (i, &step) in CUBE_STEPS.iter().enumerate() {
            let d = (v as i16 - step as i16).unsigned_abs();
            if d < best_d {
                best_d = d;
                best = i as u8;
            }
        }
        best
    };

    let ri = snap(r);
    let gi = snap(g);
    let bi = snap(b);
    let cube_idx = 16 + 36 * ri + 6 * gi + bi;
    let cr = CUBE_STEPS[ri as usize];
    let cg = CUBE_STEPS[gi as usize];
    let cb = CUBE_STEPS[bi as usize];
    let cube_dist = sq_diff(r, cr) + sq_diff(g, cg) + sq_diff(b, cb);

    // 24-step grayscale ramp: 232 = #080808, 255 = #eeeeee, step ≈ 10.25
    let gray_level = ((r as u16 + g as u16 + b as u16) / 3) as u8;
    let gray_idx = ((gray_level as u16).saturating_sub(3) / 10).min(23) as u8;
    let gv = 8u16 + 10u16 * gray_idx as u16;
    let gray_dist = sq_diff(r, gv as u8) + sq_diff(g, gv as u8) + sq_diff(b, gv as u8);

    if gray_dist < cube_dist {
        232 + gray_idx
    } else {
        cube_idx
    }
}

#[inline]
fn sq_diff(a: u8, b: u8) -> u32 {
    let d = a as i16 - b as i16;
    (d * d) as u32
}

impl ThemeColors {
    /// Resolve one role (rgb tier). Panics never: every role has a field.
    pub fn get(&self, role: Role) -> Color {
        match role {
            Role::Accent => self.accent,
            Role::AccentTeal => self.accent_teal,
            Role::AccentSoft => self.accent_soft,
            Role::TextPrimary => self.text_primary,
            Role::TextSecondary => self.text_secondary,
            Role::TextMuted => self.text_muted,
            Role::TextDim => self.text_dim,
            Role::Gray => self.gray,
            Role::GrayMid => self.gray_mid,
            Role::GrayDetail => self.gray_detail,
            Role::GrayDim => self.gray_dim,
            Role::GrayDark => self.gray_dark,
            Role::GrayBase => self.gray_base,
            Role::GrayLight => self.gray_light,
            Role::GraySoft => self.gray_soft,
            Role::GrayMuted => self.gray_muted,
            Role::Success => self.success,
            Role::AnalyticsGreen => self.analytics_green,
            Role::GreenCheck => self.green_check,
            Role::Error => self.error,
            Role::ErrorSoft => self.error_soft,
            Role::ErrorFaded => self.error_faded,
            Role::Warning => self.warning,
            Role::WarningMuted => self.warning_muted,
            Role::AmberMuted => self.amber_muted,
            Role::TealVivid => self.teal_vivid,
            Role::TealBright => self.teal_bright,
            Role::TealMuted => self.teal_muted,
            Role::TealCalm => self.teal_calm,
            Role::BlueSlate => self.blue_slate,
            Role::BlueSteel => self.blue_steel,
            Role::BlueLink => self.blue_link,
            Role::BlueSky => self.blue_sky,
            Role::BlueSoft => self.blue_soft,
            Role::BlueVivid => self.blue_vivid,
            Role::BlueCode => self.blue_code,
            Role::SelectionBg => self.selection_bg,
            Role::SurfacePanel => self.surface_panel,
            Role::SurfaceQr => self.surface_qr,
            Role::SurfaceCode => self.surface_code,
            Role::SurfaceCodeAlt => self.surface_code_alt,
            Role::Ink => self.ink,
            Role::PurpleSoft => self.purple_soft,
        }
    }

    /// Resolve one role (ansi tier). Mirrors `get()` but returns from
    /// the `ansi` field. Used on non-truecolor terminals.
    pub fn get_ansi(&self, role: Role) -> Color {
        match role {
            Role::Accent => Color::Indexed(self.ansi.accent),
            Role::AccentTeal => Color::Indexed(self.ansi.accent_teal),
            Role::AccentSoft => Color::Indexed(self.ansi.accent_soft),
            Role::TextPrimary => Color::Indexed(self.ansi.text_primary),
            Role::TextSecondary => Color::Indexed(self.ansi.text_secondary),
            Role::TextMuted => Color::Indexed(self.ansi.text_muted),
            Role::TextDim => Color::Indexed(self.ansi.text_dim),
            Role::Gray => Color::Indexed(self.ansi.gray),
            Role::GrayMid => Color::Indexed(self.ansi.gray_mid),
            Role::GrayDetail => Color::Indexed(self.ansi.gray_detail),
            Role::GrayDim => Color::Indexed(self.ansi.gray_dim),
            Role::GrayDark => Color::Indexed(self.ansi.gray_dark),
            Role::GrayBase => Color::Indexed(self.ansi.gray_base),
            Role::GrayLight => Color::Indexed(self.ansi.gray_light),
            Role::GraySoft => Color::Indexed(self.ansi.gray_soft),
            Role::GrayMuted => Color::Indexed(self.ansi.gray_muted),
            Role::Success => Color::Indexed(self.ansi.success),
            Role::AnalyticsGreen => Color::Indexed(self.ansi.analytics_green),
            Role::GreenCheck => Color::Indexed(self.ansi.green_check),
            Role::Error => Color::Indexed(self.ansi.error),
            Role::ErrorSoft => Color::Indexed(self.ansi.error_soft),
            Role::ErrorFaded => Color::Indexed(self.ansi.error_faded),
            Role::Warning => Color::Indexed(self.ansi.warning),
            Role::WarningMuted => Color::Indexed(self.ansi.warning_muted),
            Role::AmberMuted => Color::Indexed(self.ansi.amber_muted),
            Role::TealVivid => Color::Indexed(self.ansi.teal_vivid),
            Role::TealBright => Color::Indexed(self.ansi.teal_bright),
            Role::TealMuted => Color::Indexed(self.ansi.teal_muted),
            Role::TealCalm => Color::Indexed(self.ansi.teal_calm),
            Role::BlueSlate => Color::Indexed(self.ansi.blue_slate),
            Role::BlueSteel => Color::Indexed(self.ansi.blue_steel),
            Role::BlueLink => Color::Indexed(self.ansi.blue_link),
            Role::BlueSky => Color::Indexed(self.ansi.blue_sky),
            Role::BlueSoft => Color::Indexed(self.ansi.blue_soft),
            Role::BlueVivid => Color::Indexed(self.ansi.blue_vivid),
            Role::BlueCode => Color::Indexed(self.ansi.blue_code),
            Role::SelectionBg => Color::Indexed(self.ansi.selection_bg),
            Role::SurfacePanel => Color::Indexed(self.ansi.surface_panel),
            Role::SurfaceQr => Color::Indexed(self.ansi.surface_qr),
            Role::SurfaceCode => Color::Indexed(self.ansi.surface_code),
            Role::SurfaceCodeAlt => Color::Indexed(self.ansi.surface_code_alt),
            Role::Ink => Color::Indexed(self.ansi.ink),
            Role::PurpleSoft => Color::Indexed(self.ansi.purple_soft),
        }
    }
}

/// A named theme. `crab_dark` below is the default and must stay
/// byte-identical to the S1 palette (regression-tested in `presets`).
#[derive(Debug)]
pub struct Theme {
    pub name: &'static str,
    pub colors: ThemeColors,
}

/// The default theme: every field IS the post-S1 palette const, so
/// byte-identical rendering is guaranteed at compile time.
pub static CRAB_DARK: Theme = Theme {
    name: "crab-dark",
    colors: ThemeColors {
        accent: palette::ORANGE,
        accent_teal: palette::TEAL,
        accent_soft: palette::WHITE,
        text_primary: palette::TEXT_PRIMARY,
        text_secondary: palette::TEXT_SECONDARY,
        text_muted: palette::TEXT_MUTED,
        text_dim: palette::TEXT_DIM,
        gray: palette::GRAY,
        gray_mid: palette::GRAY_MID,
        gray_detail: palette::GRAY_DETAIL,
        gray_dim: palette::GRAY_DIM,
        gray_dark: palette::GRAY_DARK,
        gray_base: palette::GRAY_BASE,
        gray_light: palette::GRAY_LIGHT,
        gray_soft: palette::GRAY_SOFT,
        gray_muted: palette::GRAY_MUTED,
        success: palette::SUCCESS,
        analytics_green: palette::ANALYTICS_GREEN,
        green_check: palette::GREEN_CHECK,
        error: palette::ERROR,
        error_soft: palette::ERROR_SOFT,
        error_faded: palette::ERROR_FADED,
        warning: palette::WARNING,
        warning_muted: palette::WARNING_MUTED,
        amber_muted: palette::AMBER_MUTED,
        teal_vivid: palette::TEAL_VIVID,
        teal_bright: palette::TEAL_BRIGHT,
        teal_muted: palette::TEAL_MUTED,
        teal_calm: palette::TEAL_CALM,
        blue_slate: palette::BLUE_SLATE,
        blue_steel: palette::BLUE_STEEL,
        blue_link: palette::BLUE_LINK,
        blue_sky: palette::BLUE_SKY,
        blue_soft: palette::BLUE_SOFT,
        blue_vivid: palette::BLUE_VIVID,
        blue_code: palette::BLUE_CODE,
        selection_bg: palette::SELECTION_BG,
        surface_panel: palette::SURFACE_PANEL,
        surface_qr: palette::SURFACE_QR,
        surface_code: palette::SURFACE_CODE,
        surface_code_alt: palette::SURFACE_CODE_ALT,
        ink: palette::INK,
        purple_soft: palette::PURPLE_SOFT,
        // No canvas: the default theme leaves the terminal's own
        // background showing, exactly as it did before #1634.
        background: None,
        ansi: AnsiColors {
            accent: 166,
            accent_teal: 14,
            accent_soft: 253,
            text_primary: 252,
            text_secondary: 246,
            text_muted: 240,
            text_dim: 238,
            gray: 243,
            gray_mid: 241,
            gray_detail: 240,
            gray_dim: 239,
            gray_dark: 238,
            gray_base: 236,
            gray_light: 251,
            gray_soft: 248,
            gray_muted: 247,
            success: 78,
            analytics_green: 41,
            green_check: 114,
            error: 167,
            error_soft: 167,
            error_faded: 174,
            warning: 179,
            warning_muted: 179,
            amber_muted: 143,
            teal_vivid: 73,
            teal_bright: 73,
            teal_muted: 73,
            teal_calm: 73,
            blue_slate: 60,
            blue_steel: 67,
            blue_link: 74,
            blue_sky: 74,
            blue_soft: 74,
            blue_vivid: 69,
            blue_code: 103,
            selection_bg: 61,
            surface_panel: 235,
            surface_qr: 233,
            surface_code: 236,
            surface_code_alt: 236,
            ink: 234,
            purple_soft: 140,
            background: None,
        },
    },
};

/// Active theme slot. `None` means default (crab-dark); avoids any
/// const-eval constraints on the lock's initializer.
static ACTIVE: RwLock<Option<&'static Theme>> = RwLock::new(None);

/// Bumped by every [`set`] and [`reset`]. Consumers that cache pre-styled
/// output (the chat render cache holds `Line`s whose spans already carry
/// resolved colours) compare this against the generation they cached under
/// and drop their entries when it moves. Without it a live `/theme` switch
/// repaints only the widgets that call [`role`] every frame, and every
/// already-rendered message keeps the previous theme's colours until an
/// unrelated cache-busting event such as a terminal resize (#1634).
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// Truecolor capability cache. `None` means not yet probed; defaults
/// to true on first `role()` call if `init_capability()` wasn't invoked.
static TRUECOLOR: OnceLock<bool> = OnceLock::new();

/// Probe terminal truecolor support once. Call at TUI boot before any
/// render. `crossterm::style::available_color_count()` returns `u16::MAX`
/// for truecolor terminals, 256 for 256-color, 8 for basic.
pub fn init_capability() {
    let count = crossterm::style::available_color_count();
    let _ = TRUECOLOR.set(count == u16::MAX);
}

/// Current truecolor capability. Defaults to true if not yet probed
/// (safe: rgb values render correctly on truecolor terminals, and
/// non-truecolor terminals will just show the rgb as-is).
pub fn is_truecolor() -> bool {
    *TRUECOLOR.get().unwrap_or(&true)
}

/// Currently active theme (crab-dark until [`set`] is called).
pub fn active() -> &'static Theme {
    ACTIVE
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .unwrap_or(&CRAB_DARK)
}

/// Switch the active theme. Callers own validation (preset lookup by
/// name lives in `presets::by_name`).
pub fn set(theme: &'static Theme) {
    *ACTIVE.write().unwrap_or_else(|e| e.into_inner()) = Some(theme);
    GENERATION.fetch_add(1, Ordering::Relaxed);
}

/// Return to the default theme.
pub fn reset() {
    *ACTIVE.write().unwrap_or_else(|e| e.into_inner()) = None;
    GENERATION.fetch_add(1, Ordering::Relaxed);
}

/// Resolve a role against the active theme. The one call render sites
/// make after the S2 codemod. Dispatches rgb vs ansi based on the
/// terminal's truecolor capability (probed once at boot via
/// [`init_capability`]).
pub fn role(role: Role) -> Color {
    let theme = active();
    if is_truecolor() {
        theme.colors.get(role)
    } else {
        theme.colors.get_ansi(role)
    }
}

/// The active theme's canvas background, or `None` when it declares none.
///
/// Separate from [`role`] because the answer is optional: a theme that
/// declares no background must leave the terminal's cells untouched so a
/// transparent or blurred terminal keeps working. Callers paint only on
/// `Some`; see `render::frame`.
pub fn background() -> Option<Color> {
    let theme = active();
    if is_truecolor() {
        theme.colors.background
    } else {
        theme.colors.ansi.background.map(Color::Indexed)
    }
}

/// Monotonic counter of theme switches. Holders of pre-styled output cache
/// the value they rendered under and invalidate when it changes; see
/// [`GENERATION`].
pub fn generation() -> u64 {
    GENERATION.load(Ordering::Relaxed)
}
