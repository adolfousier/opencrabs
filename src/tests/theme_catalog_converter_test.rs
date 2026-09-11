//! Converter tests for the theme catalog (#1461): both source formats
//! round-trip through the fixed 43-role mapping into TOML that the existing
//! validator accepts UNCHANGED (the hard invariant), semantic tokens and
//! defs/variants/ANSI ints resolve, `#RGB` shorthand and `#RRGGBBAA` alpha
//! hex normalize, malformed sources are rejected (builtin-name collisions
//! stay the loader's gate), and low-contrast derivations are repaired
//! before certification instead of failing the validator.

use ratatui::style::Color;

use crate::tui::render::user_themes::{build_theme, parse_hex};
use crate::tui::theme_catalog::{Variant, convert_alacritty_toml, convert_opencode_json, mix};

/// Byte-identical copy of alacritty/alacritty-theme master
/// themes/tokyo_night.toml (see fixtures/SOURCES.md).
const ALCRITTY_TOKYO_NIGHT: &str =
    include_str!("../tui/theme_catalog/fixtures/tokyo_night.alacritty.toml");

/// Byte-identical copy of sst/opencode dev
/// packages/tui/src/theme/assets/tokyonight.json (see fixtures/SOURCES.md).
const OPENCODE_TOKYONIGHT: &str =
    include_str!("../tui/theme_catalog/fixtures/tokyonight.opencode.json");

fn rgb(hex: &str) -> Color {
    parse_hex(hex).unwrap_or_else(|| panic!("test fixture hex {hex}"))
}

/// Pull one emitted `key = "#rrggbb"` line back out of generated TOML.
fn emitted_color(text: &str, key: &str) -> Color {
    let line = text
        .lines()
        .find(|l| l.starts_with(&format!("{key} = ")))
        .unwrap_or_else(|| panic!("emitted TOML lacks key {key}"));
    let hex = line
        .split_once("= \"")
        .map(|(_, v)| v.trim_end_matches('"'))
        .unwrap();
    parse_hex(hex).unwrap_or_else(|| panic!("emitted value for {key}: {hex}"))
}

/// The alacritty fixture converts, the validator accepts the output
/// unchanged, and direct-mapped roles carry the exact upstream colors.
#[test]
fn alacritty_round_trip_passes_validator_unchanged() {
    let text = convert_alacritty_toml("tokyo-night", ALCRITTY_TOKYO_NIGHT, "source: test fixture")
        .expect("alacritty conversion should certify");
    let theme = build_theme("tokyo-night", &text).expect("validator must accept unchanged");
    assert_eq!(theme.colors.ink, rgb("#1a1b26"), "ink = primary.background");
    assert_eq!(
        theme.colors.text_primary,
        rgb("#a9b1d6"),
        "text = primary.foreground"
    );
    assert_eq!(theme.colors.error, rgb("#f7768e"), "error = normal.red");
    assert_eq!(
        theme.colors.success,
        rgb("#9ece6a"),
        "success = normal.green"
    );
    assert_eq!(
        theme.colors.warning,
        rgb("#e0af68"),
        "warning = normal.yellow"
    );
    assert_eq!(
        theme.colors.teal_vivid,
        rgb("#0db9d7"),
        "teal_vivid = bright.cyan"
    );
    assert_eq!(
        theme.colors.accent,
        rgb("#ff9e64"),
        "accent = bright.yellow"
    );
    assert_eq!(
        theme.colors.purple_soft,
        rgb("#bb9af7"),
        "purple = bright.magenta"
    );
}

/// The opencode fixture (dark variant) converts with defs references
/// resolved, and semantic tokens win over ANSI-slot fallbacks.
#[test]
fn opencode_dark_round_trip_passes_validator_unchanged() {
    let text = convert_opencode_json(
        "tokyonight-opencode",
        OPENCODE_TOKYONIGHT,
        Variant::Dark,
        "source: test fixture",
    )
    .expect("opencode dark conversion should certify");
    let theme = build_theme("tokyonight-opencode", &text).expect("validator must accept");
    assert_eq!(theme.colors.ink, rgb("#1a1b26"), "ink = darkStep1");
    assert_eq!(
        theme.colors.text_primary,
        rgb("#c8d3f5"),
        "text = darkStep12"
    );
    assert_eq!(
        theme.colors.accent,
        rgb("#ff966c"),
        "accent = darkOrange token"
    );
    assert_eq!(theme.colors.error, rgb("#ff757f"), "error = darkRed token");
    assert_eq!(
        theme.colors.purple_soft,
        rgb("#c099ff"),
        "purple = darkPurple (secondary)"
    );
    assert_eq!(
        theme.colors.surface_panel,
        rgb("#1e2030"),
        "panel = darkStep2"
    );
    assert_eq!(
        theme.colors.surface_code,
        rgb("#222436"),
        "code = darkStep3"
    );
}

/// The light variant resolves the `light` side of every token object.
#[test]
fn opencode_light_variant_resolves_light_defs() {
    let text = convert_opencode_json(
        "tokyonight-opencode-light",
        OPENCODE_TOKYONIGHT,
        Variant::Light,
        "source: test fixture",
    )
    .expect("opencode light conversion should certify");
    let theme = build_theme("tokyonight-opencode-light", &text).expect("validator must accept");
    assert_eq!(theme.colors.ink, rgb("#e1e2e7"), "ink = lightStep1");
    assert_eq!(
        theme.colors.text_primary,
        rgb("#3760bf"),
        "text = lightStep12"
    );
    assert_eq!(theme.colors.accent, rgb("#b15c00"), "accent = lightOrange");
}

/// Emission shape: provenance header, exactly 43 unique keys, all hex.
#[test]
fn emitted_toml_has_provenance_and_exactly_43_keys() {
    let text = convert_alacritty_toml(
        "emit-shape-check",
        ALCRITTY_TOKYO_NIGHT,
        "source: alacritty/alacritty-theme themes/tokyo_night.toml",
    )
    .expect("conversion should certify");
    assert!(
        text.starts_with("# source: alacritty/alacritty-theme"),
        "provenance header first"
    );
    assert!(text.contains("# generated by tui::theme_catalog::converter"));
    let key_lines: Vec<&str> = text
        .lines()
        .filter(|l| !l.starts_with('#') && l.contains(" = \"#"))
        .collect();
    assert_eq!(key_lines.len(), 43, "exactly the 43 schema keys");
    let mut keys: Vec<&str> = key_lines
        .iter()
        .map(|l| l.split(" = ").next().unwrap())
        .collect();
    let before = keys.len();
    keys.sort_unstable();
    keys.dedup();
    assert_eq!(keys.len(), before, "keys unique");
}

/// defs references, variant objects, ANSI integers and "none" all resolve
/// in one synthetic document; "none" falls back to the xterm default.
#[test]
fn defs_refs_variants_ansi_ints_and_none_resolve() {
    let src = r##"{
        "defs": { "bgdef": "#101820", "fgdef": "#e0e8f0" },
        "theme": {
            "background": { "dark": "bgdef", "light": "#f4f4f4" },
            "text": { "dark": "fgdef", "light": "#111111" },
            "accent": 9,
            "error": "none"
        }
    }"##;
    let text = convert_opencode_json("synthetic", src, Variant::Dark, "synthetic")
        .expect("synthetic conversion should certify");
    assert_eq!(
        emitted_color(&text, "ink"),
        rgb("#101820"),
        "defs ref resolved"
    );
    assert_eq!(
        emitted_color(&text, "text_primary"),
        rgb("#e0e8f0"),
        "dark variant resolved"
    );
    assert_eq!(
        emitted_color(&text, "accent"),
        rgb("#ff0000"),
        "ANSI int 9 = bright red"
    );
    assert_eq!(
        emitted_color(&text, "error"),
        rgb("#cd0000"),
        "\"none\" falls back to xterm 1"
    );
    let light = convert_opencode_json("synthetic-l", src, Variant::Light, "synthetic")
        .expect("light conversion should certify");
    assert_eq!(
        emitted_color(&light, "ink"),
        rgb("#f4f4f4"),
        "light background resolved"
    );
    assert_eq!(
        emitted_color(&light, "text_primary"),
        rgb("#111111"),
        "light variant resolved"
    );
}

/// ANSI indices outside 0-15 have no canonical RGB and are rejected by
/// name, not silently approximated.
#[test]
fn unsupported_ansi_index_rejected() {
    let src = r##"{ "theme": { "background": "#101820", "text": "#e0e8f0", "accent": 42 } }"##;
    let err = convert_opencode_json("bad-ansi", src, Variant::Dark, "x").unwrap_err();
    assert!(err.contains("unsupported ANSI index 42"), "got: {err}");
}

/// Malformed sources fail with errors that name the format and the field.
#[test]
fn malformed_sources_rejected() {
    let err = convert_alacritty_toml("bad", "colors = [[", "x").unwrap_err();
    assert!(err.contains("alacritty parse error"), "got: {err}");
    let err = convert_alacritty_toml("bad", "[colors.normal]\nblack = '#000000'", "x").unwrap_err();
    assert!(err.contains("colors.primary"), "got: {err}");
    let err = convert_opencode_json("bad", r#"{ "defs": {} }"#, Variant::Dark, "x").unwrap_err();
    assert!(err.contains("missing \"theme\" object"), "got: {err}");
}

/// A near-invisible source (fg a hair off bg) still certifies: the repair
/// loop lifts the failing pair roles toward the contrasting extreme before
/// the validator ever sees the TOML, and the nudge is visible in the
/// output (text_primary no longer equals the raw source fg).
#[test]
fn low_contrast_source_is_repaired_then_certified() {
    let src = r#"
[colors.primary]
background = '#1e2128'
foreground = '#262a33'
[colors.normal]
black   = '#1c1f26'
red     = '#2a2227'
green   = '#222a24'
yellow  = '#2a2822'
blue    = '#22252a'
magenta = '#28222a'
cyan    = '#222a2a'
white   = '#2e323b'
[colors.bright]
black   = '#22252c'
red     = '#33292f'
green   = '#29332b'
yellow  = '#333029'
blue    = '#292d33'
magenta = '#312933'
cyan    = '#293333'
white   = '#383d48'
"#;
    let text = convert_alacritty_toml("low-contrast", src, "synthetic")
        .expect("repair must certify low-contrast sources");
    build_theme("low-contrast", &text).expect("validator accepts repaired output");
    let tp = emitted_color(&text, "text_primary");
    assert_ne!(
        tp,
        rgb("#262a33"),
        "repair moved text_primary off the raw fg"
    );
    let Color::Rgb(r, g, b) = tp else {
        panic!("rgb")
    };
    assert!(
        r > 0x26 || g > 0x2a || b > 0x33,
        "text_primary was brightened toward white: #{r:02x}{g:02x}{b:02x}"
    );
}

/// Name layering: the converter certifies colors under a shadow stem (so
/// regeneration can legitimately reuse shipped pack names), and install-time
/// collision stays the loader's gate — installing generated TOML under a
/// built-in's name is rejected by `build_theme` itself.
#[test]
fn loader_rejects_install_under_builtin_name() {
    let text = convert_alacritty_toml("dracula", ALCRITTY_TOKYO_NIGHT, "x")
        .expect("conversion certifies colors regardless of the name");
    let err = build_theme("dracula", &text).unwrap_err();
    assert!(
        err.contains("collides with a built-in preset"),
        "got: {err}"
    );
}

/// Roles with no source token fall back to the documented deterministic
/// mixes of bg/fg (selection absent from the alacritty fixture).
#[test]
fn absent_selection_falls_back_to_documented_mix() {
    let text = convert_alacritty_toml("fallback-check", ALCRITTY_TOKYO_NIGHT, "x")
        .expect("conversion should certify");
    let bg = rgb("#1a1b26");
    let fg = rgb("#a9b1d6");
    assert_eq!(emitted_color(&text, "selection_bg"), mix(bg, fg, 0.18));
    assert_eq!(emitted_color(&text, "surface_panel"), mix(bg, fg, 0.06));
    assert_eq!(emitted_color(&text, "surface_qr"), mix(bg, fg, 0.03));
}

/// The two hex forms opencode uses beyond `#RRGGBB` (found in the real
/// cursor/mercury/vesper assets): 3-digit `#RGB` shorthand expands per CSS,
/// and 8-digit `#RRGGBBAA` alpha-composites over the variant's background,
/// the surface those colors (borders, separators) are painted on. The
/// loader's `parse_hex` stays strict 6-digit; normalization is the
/// converter's job.
#[test]
fn hex_shorthand_and_rgba_composite_over_background() {
    let src = r##"{
      "defs": { "bgdef": "#101010", "fgshort": "#FFF", "alphaBorder": "#e4e4e413" },
      "theme": {
        "background": "bgdef",
        "text": { "dark": "fgshort", "light": "#111111" },
        "border": { "dark": "alphaBorder", "light": "none" }
      }
    }"##;
    let dark = crate::tui::theme_catalog::opencode::parse(src, Variant::Dark)
        .expect("dark variant parses");
    assert_eq!(dark.fg, Color::Rgb(255, 255, 255), "#FFF shorthand expands");
    // #e4e4e4 at alpha 0x13=19 over #101010:
    // round((228*19 + 16*236 + 127) / 255) = 32 per channel.
    assert_eq!(
        dark.border,
        Some(Color::Rgb(32, 32, 32)),
        "alpha border composites over the background"
    );
    let light = crate::tui::theme_catalog::opencode::parse(src, Variant::Light)
        .expect("light variant parses");
    assert_eq!(light.border, None, "\"none\" stays absent per variant");
    assert_eq!(light.fg, Color::Rgb(0x11, 0x11, 0x11));
    // The full pipeline still certifies through the validator.
    let toml = convert_opencode_json("normalize-check", src, Variant::Dark, "x")
        .expect("normalized palette certifies");
    assert!(toml.contains("ink = \"#101010\""));
    assert!(toml.contains("text_primary = \"#ffffff\""));
}
