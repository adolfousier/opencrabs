//! opencode theme JSON parser: `defs` + `theme` token maps into a
//! [`SourcePalette`]. Token values come in five shapes per the opencode
//! docs (https://opencode.ai/docs/themes/): hex strings (`"#RRGGBB"`;
//! `#RGB` shorthand and alpha-bearing `#RRGGBBAA` are normalized by the
//! parser, the loader's strict 6-digit `parse_hex` stays untouched), ANSI
//! color integers (0-15, resolved through the xterm table), references to
//! `defs` entries (bare names), the literal `"none"` (token skipped), and
//! `{dark, light}` variant objects wrapping any of the former.
//!
//! opencode tokens are semantic, not ANSI, so the mapping into the classic
//! 16 slots is conventional: `error`->red, `success`->green, `warning`->
//! yellow, `primary`->blue, `secondary`->magenta, `markdownLinkText`/`info`
//! ->cyan, `borderSubtle`->black, `textMuted`->white. Missing tokens fall
//! back to the xterm defaults so every well-formed theme parses. The
//! semantic fields of [`SourcePalette`] carry the tokens verbatim; the role
//! mapping in `converter` prefers them over the ANSI slots.

use std::collections::HashMap;

use ratatui::style::Color;
use serde_json::Value;

use super::{SourcePalette, Variant, XTERM16};
use crate::tui::render::user_themes::parse_hex;

/// Parse an opencode theme JSON document, resolving every token to the
/// requested variant.
pub fn parse(src: &str, variant: Variant) -> Result<SourcePalette, String> {
    let doc: Value = serde_json::from_str(src).map_err(|e| format!("opencode parse error: {e}"))?;
    // Defs stay raw: `#RRGGBBAA` values composite against the variant's
    // background, which is only known once `background` resolves, and
    // `background` may itself be a defs reference. Two phases: bg first
    // (base `None`; a degenerate alpha bg composites over black), then
    // every other token with `Some(bg)` as the alpha base.
    let defs = raw_defs(doc.get("defs"))?;
    let theme = doc.get("theme").ok_or("missing \"theme\" object")?;
    let tok = |name: &str, base: Option<Color>| -> Result<Option<Color>, String> {
        match theme.get(name) {
            Some(v) => resolve_value(v, &defs, variant, base, name),
            None => Ok(None),
        }
    };
    let bg =
        tok("background", None)?.ok_or("token \"background\" missing or \"none\" (required)")?;
    let base = Some(bg);
    let fg = tok("text", base)?.ok_or("token \"text\" missing or \"none\" (required)")?;
    let xterm = |i: usize| Color::Rgb(XTERM16[i].0, XTERM16[i].1, XTERM16[i].2);
    let accent = tok("accent", base)?;
    let purple = tok("secondary", base)?;
    let info = tok("info", base)?;
    let link = tok("markdownLink", base)?;
    let muted = tok("textMuted", base)?;
    let panel = tok("backgroundPanel", base)?;
    let element = tok("backgroundElement", base)?;
    let selection = tok("selectionBackground", base)?;
    let border = tok("border", base)?;
    let border_subtle = tok("borderSubtle", base)?;
    let link_text = tok("markdownLinkText", base)?;
    let red = tok("error", base)?.unwrap_or_else(|| xterm(1));
    let green = tok("success", base)?.unwrap_or_else(|| xterm(2));
    let yellow = tok("warning", base)?.unwrap_or_else(|| xterm(3));
    let blue = tok("primary", base)?.unwrap_or_else(|| xterm(4));
    let magenta = purple.unwrap_or_else(|| xterm(5));
    let cyan = link_text.or(info).unwrap_or_else(|| xterm(6));
    let black = border_subtle.or(border).unwrap_or_else(|| xterm(0));
    let white = muted.unwrap_or(fg);
    Ok(SourcePalette {
        bg,
        fg,
        black,
        red,
        green,
        yellow,
        blue,
        magenta,
        cyan,
        white,
        // opencode has no bright/dim distinction: the semantic tokens above
        // serve the bright slots directly, and the role mapping prefers the
        // explicit Option fields anyway.
        bright_black: black,
        bright_red: red,
        bright_green: green,
        bright_yellow: yellow,
        bright_blue: blue,
        bright_magenta: magenta,
        bright_cyan: cyan,
        bright_white: fg,
        accent,
        purple,
        info,
        link,
        muted,
        panel,
        element,
        selection,
        border,
        border_subtle,
    })
}

/// Defs kept as raw JSON values: resolution is deferred until the alpha
/// base (the variant's background) is known. Upstream never nests defs
/// references, so one lookup level is enough.
fn raw_defs(defs: Option<&Value>) -> Result<HashMap<String, Value>, String> {
    let mut out = HashMap::new();
    let Some(defs) = defs else { return Ok(out) };
    let obj = defs.as_object().ok_or("\"defs\" is not an object")?;
    for (name, v) in obj {
        out.insert(name.clone(), v.clone());
    }
    Ok(out)
}

/// Resolve one token value for the chosen variant. `Ok(None)` means the
/// token is `"none"` for that variant and callers fall back. `base` is the
/// alpha-compositing background (the variant's `background` token); only
/// the background itself resolves with `None`.
fn resolve_value(
    v: &Value,
    defs: &HashMap<String, Value>,
    variant: Variant,
    base: Option<Color>,
    path: &str,
) -> Result<Option<Color>, String> {
    if let Some(obj) = v.as_object() {
        let key = match variant {
            Variant::Dark => "dark",
            Variant::Light => "light",
        };
        let inner = obj
            .get(key)
            .ok_or_else(|| format!("{path}: variant object without \"{key}\" key"))?;
        return resolve_value(inner, defs, variant, base, path);
    }
    resolve_with_defs(v, defs, base, path)
}

/// Hex string (normalized first: `#RGB` shorthand expanded, `#RRGGBBAA`
/// composited over `base`), ANSI integer, `"none"`, or an error. Bare defs
/// references are handled by [`resolve_with_defs`] before reaching here.
fn scalar_to_color(v: &Value, base: Option<Color>, path: &str) -> Result<Option<Color>, String> {
    if let Some(i) = v.as_i64() {
        let idx = usize::try_from(i)
            .ok()
            .filter(|&i| i < XTERM16.len())
            .ok_or_else(|| {
                format!("{path}: unsupported ANSI index {i} (only 0-15 resolve to RGB)")
            })?;
        let (r, g, b) = XTERM16[idx];
        return Ok(Some(Color::Rgb(r, g, b)));
    }
    let Some(s) = v.as_str() else {
        return Err(format!("{path}: value is neither a string nor an integer"));
    };
    if s.eq_ignore_ascii_case("none") {
        return Ok(None);
    }
    if s.starts_with('#') {
        let normalized = normalize_hex(s, base).map_err(|e| format!("{path}: {e}"))?;
        return parse_hex(&normalized)
            .map(Some)
            .ok_or_else(|| format!("{path}: invalid hex {s:?}"));
    }
    Err(format!(
        "{path}: unresolved value {s:?} (defs references need the defs table)"
    ))
}

/// Normalize upstream hex strings into the 6-digit `#RRGGBB` shape the
/// loader's strict `parse_hex` accepts (the loader stays untouched: #1461
/// scope). opencode themes use two shapes beyond it:
/// - `#RGB` shorthand, expanded per CSS (`#FFF` -> `#FFFFFF`).
/// - `#RRGGBBAA`, alpha-composited over `base`: the variant's background,
///   which is the surface these colors (borders, subtle separators) are
///   painted on. `base` is `None` only while resolving the background
///   itself; a degenerate alpha background composites over black.
fn normalize_hex(s: &str, base: Option<Color>) -> Result<String, String> {
    let h = s
        .strip_prefix('#')
        .ok_or_else(|| format!("invalid hex {s:?} (missing '#')"))?;
    if !h.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!("invalid hex {s:?} (non-hex digits)"));
    }
    match h.len() {
        6 => Ok(s.to_string()),
        3 => Ok(format!(
            "#{}{}{}",
            h[0..1].repeat(2),
            h[1..2].repeat(2),
            h[2..3].repeat(2)
        )),
        8 => {
            let v = u64::from_str_radix(h, 16).map_err(|e| format!("invalid hex {s:?}: {e}"))?;
            let (r, g, b, a) = (
                (v >> 24) as u8,
                ((v >> 16) & 0xff) as u8,
                ((v >> 8) & 0xff) as u8,
                (v & 0xff) as u8,
            );
            let (br, bg_, bb) = match base {
                Some(Color::Rgb(r, g, b)) => (r, g, b),
                _ => (0, 0, 0),
            };
            let a32 = u32::from(a);
            let over = |c: u8, bs: u8| -> u8 {
                ((u32::from(c) * a32 + u32::from(bs) * (255 - a32) + 127) / 255) as u8
            };
            Ok(format!(
                "#{:02x}{:02x}{:02x}",
                over(r, br),
                over(g, bg_),
                over(b, bb)
            ))
        }
        _ => Err(format!(
            "invalid hex {s:?} (expected #RGB, #RRGGBB or #RRGGBBAA)"
        )),
    }
}

/// Resolve a token that may be a defs reference (bare name). Kept separate
/// from `scalar_to_color` so defs stay free of cross-references (upstream
/// files never nest them). The reference resolves through
/// `scalar_to_color`, so `#RRGGBBAA` defs composite over `base` at lookup
/// time.
fn resolve_with_defs(
    v: &Value,
    defs: &HashMap<String, Value>,
    base: Option<Color>,
    path: &str,
) -> Result<Option<Color>, String> {
    if let Some(s) = v.as_str()
        && !s.starts_with('#')
        && !s.eq_ignore_ascii_case("none")
        && let Some(d) = defs.get(s)
    {
        return scalar_to_color(d, base, &format!("{path} (defs.{s})"));
    }
    scalar_to_color(v, base, path)
}
