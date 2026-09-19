//! #1634 phase 6: no render site in the TUI may name a raw ANSI colour.
//!
//! Every `Color::Gray`, `Color::DarkGray`, `Color::White`, `Color::Cyan` and
//! friend is an index into whatever 16-colour palette the terminal happens to
//! carry. `theme::set` cannot move one, so each is a pixel that stays put while
//! the rest of the screen repaints. 622 of them survived the S1/S2 migrations
//! and made `/theme` look like it changed almost nothing.
//!
//! The migrations left no test behind, which is exactly how the gap survived
//! them, so this scans the source instead of trusting a one-off sweep.
//! `Color::Rgb` and `Color::Indexed` are fine: those carry their own value and
//! are what `theme::role` itself returns.

use std::fs;
use std::path::{Path, PathBuf};

/// The only file allowed to name raw ANSI colours.
///
/// `palette.rs` is where crab-dark's constants are DEFINED, and one of them
/// (`TEAL`) is deliberately `Color::Cyan` so the brand accent follows a
/// terminal's own configured cyan. Themes override it like any other role.
const PALETTE: &str = "render/palette.rs";

const RAW: [&str; 17] = [
    "Color::Black",
    "Color::Red",
    "Color::Green",
    "Color::Yellow",
    "Color::Blue",
    "Color::Magenta",
    "Color::Cyan",
    "Color::Gray",
    "Color::DarkGray",
    "Color::LightRed",
    "Color::LightGreen",
    "Color::LightYellow",
    "Color::LightBlue",
    "Color::LightMagenta",
    "Color::LightCyan",
    "Color::White",
    "Color::Reset",
];

fn tui_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src/tui")
}

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("src/tui must be readable") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// `Color::Gray` must not match inside `Color::GrayDim`; nothing in ratatui is
/// named that way today, but the guard costs one check and a false negative
/// here is a silently unthemed widget.
fn names_raw_variant(line: &str, variant: &str) -> bool {
    let mut from = 0;
    while let Some(at) = line[from..].find(variant) {
        let end = from + at + variant.len();
        let next = line[end..].chars().next();
        if !next.is_some_and(|c| c.is_alphanumeric() || c == '_') {
            return true;
        }
        from = end;
    }
    false
}

/// Comment lines are prose: several explain which raw colour a site used to
/// name, and rewriting history to satisfy a grep would be the wrong fix.
fn is_comment(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("//") || t.starts_with("*") || t.starts_with("/*")
}

#[test]
fn no_tui_render_site_names_a_raw_ansi_colour() {
    let root = tui_root();
    let mut files = Vec::new();
    rust_files(&root, &mut files);
    assert!(
        files.len() > 20,
        "walk found only {} files, so a pass here would prove nothing",
        files.len()
    );

    let mut offenders: Vec<String> = Vec::new();
    for path in &files {
        let rel = path
            .strip_prefix(&root)
            .expect("under src/tui")
            .to_string_lossy()
            .replace('\\', "/");
        if rel == PALETTE {
            continue;
        }
        let source = fs::read_to_string(path).expect("source must be readable");
        for (i, line) in source.lines().enumerate() {
            if is_comment(line) {
                continue;
            }
            for variant in RAW {
                if names_raw_variant(line, variant) {
                    offenders.push(format!("{rel}:{}: {}", i + 1, line.trim()));
                }
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "{} raw ANSI colour site(s) in src/tui. These resolve against the \
         terminal's own 16-colour palette, so no theme can move them (#1634). \
         Use theme::role(Role::…) instead:\n{}",
        offenders.len(),
        offenders.join("\n")
    );
}

/// The allowlist is one file, and it has to stay that way to mean anything.
#[test]
fn only_the_palette_definition_is_exempt() {
    let path = tui_root().join(PALETTE);
    let source = fs::read_to_string(&path).expect("palette.rs must be readable");
    assert!(
        source.contains("pub const TEAL: Color = Color::Cyan;"),
        "palette.rs is exempt because it DEFINES the constants; if it no longer \
         names a raw colour the exemption should be deleted, not kept"
    );
}
