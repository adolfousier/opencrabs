//! alacritty-theme TOML parser: `[colors.primary]` bg/fg plus the 16 ANSI
//! colors under `[colors.normal]` / `[colors.bright]` into a
//! [`SourcePalette`]. Optional sections (`[colors.cursor]`,
//! `[colors.selection]`, `[colors.dim]`, indexed colors) are read only where
//! a role benefits (`selection.background`); everything else is ignored, so
//! any file from the `alacritty/alacritty-theme` catalog parses.

use ratatui::style::Color;

use super::SourcePalette;
use crate::tui::render::user_themes::parse_hex;

/// Parse an alacritty-theme TOML document. Errors name the missing or
/// malformed field exactly as spelled in the source file.
pub fn parse(src: &str) -> Result<SourcePalette, String> {
    let doc: toml::Value =
        toml::from_str(src).map_err(|e| format!("alacritty parse error: {e}"))?;
    let colors = doc.get("colors").ok_or("missing [colors] section")?;
    let primary = colors
        .get("primary")
        .ok_or("missing [colors.primary] section")?;
    let bg = hex_field(primary, "colors.primary.background")?;
    let fg = hex_field(primary, "colors.primary.foreground")?;
    let normal = colors
        .get("normal")
        .ok_or("missing [colors.normal] section")?;
    let bright = colors
        .get("bright")
        .ok_or("missing [colors.bright] section")?;
    Ok(SourcePalette {
        bg,
        fg,
        black: hex_field(normal, "colors.normal.black")?,
        red: hex_field(normal, "colors.normal.red")?,
        green: hex_field(normal, "colors.normal.green")?,
        yellow: hex_field(normal, "colors.normal.yellow")?,
        blue: hex_field(normal, "colors.normal.blue")?,
        magenta: hex_field(normal, "colors.normal.magenta")?,
        cyan: hex_field(normal, "colors.normal.cyan")?,
        white: hex_field(normal, "colors.normal.white")?,
        bright_black: hex_field(bright, "colors.bright.black")?,
        bright_red: hex_field(bright, "colors.bright.red")?,
        bright_green: hex_field(bright, "colors.bright.green")?,
        bright_yellow: hex_field(bright, "colors.bright.yellow")?,
        bright_blue: hex_field(bright, "colors.bright.blue")?,
        bright_magenta: hex_field(bright, "colors.bright.magenta")?,
        bright_cyan: hex_field(bright, "colors.bright.cyan")?,
        bright_white: hex_field(bright, "colors.bright.white")?,
        accent: None,
        purple: None,
        info: None,
        link: None,
        muted: None,
        panel: None,
        element: None,
        selection: colors
            .get("selection")
            .and_then(|s| s.get("background"))
            .and_then(|v| v.as_str())
            .and_then(parse_hex),
        border: None,
        border_subtle: None,
    })
}

fn hex_field(table: &toml::Value, path: &str) -> Result<Color, String> {
    let raw = table
        .get(path.rsplit('.').next().unwrap_or(path))
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("{path}: missing or not a string"))?;
    parse_hex(raw).ok_or_else(|| format!("{path}: invalid hex {raw:?} (expected \"#RRGGBB\")"))
}
