//! Regenerates the shipped theme pack (`src/tui/theme_catalog/pack/*.toml`)
//! from upstream sources via the #1461 converter. Dev tooling only; nothing
//! in the binary calls this at runtime.
//!
//! Usage:
//! ```text
//! # 1. fetch upstream sources into a scratch dir (see pack/README.md for
//! #    the exact curl loop and the upstream refs the curation pins)
//! # 2. regenerate:
//! cargo run --example theme_pack_gen -- <sources-dir>
//! ```
//!
//! `<sources-dir>` must contain the files named in [`CURATED`] (opencode
//! JSON as `oc-<upstream-name>.json`, alacritty TOML as
//! `al-<upstream-name>.toml` — the fetch loop in pack/README.md produces
//! exactly these names). Each emitted file is certified through the
//! existing validator by the converter itself; this example additionally
//! refuses to write a file whose conversion failed, and reports a summary.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use opencrabs::tui::theme_catalog::Variant;
use opencrabs::tui::theme_catalog::converter::{convert_alacritty_toml, convert_opencode_json};

enum Format {
    Opencode(Variant),
    Alacritty,
}

/// The curation: (pack name, source file under <sources-dir>, format,
/// provenance tail). Pack names are kebab-case and must not collide with
/// the hand-built presets in `src/tui/render/presets.rs` (dracula, alucard,
/// monokai, catppuccin-mocha, catppuccin-latte, solarized-light,
/// solarized-dark, crab-dark) nor with each other; the pack round-trip
/// test in `src/tests` enforces both.
///
/// lucent-orng (opencode) is deliberately absent: its `background` token is
/// `"transparent"` in both variants, which has no mapping to the required
/// `ink` role.
const CURATED: &[(&str, &str, Format, &str)] = &[
    ("aura", "oc-aura.json", Format::Opencode(Variant::Dark), OC),
    ("ayu", "oc-ayu.json", Format::Opencode(Variant::Dark), OC),
    (
        "carbonfox",
        "oc-carbonfox.json",
        Format::Opencode(Variant::Dark),
        OC,
    ),
    (
        "catppuccin-macchiato",
        "oc-catppuccin-macchiato.json",
        Format::Opencode(Variant::Dark),
        OC,
    ),
    (
        "cobalt2",
        "oc-cobalt2.json",
        Format::Opencode(Variant::Dark),
        OC,
    ),
    (
        "cursor",
        "oc-cursor.json",
        Format::Opencode(Variant::Dark),
        OC,
    ),
    (
        "everforest",
        "oc-everforest.json",
        Format::Opencode(Variant::Dark),
        OC,
    ),
    (
        "flexoki",
        "oc-flexoki.json",
        Format::Opencode(Variant::Dark),
        OC,
    ),
    (
        "github",
        "oc-github.json",
        Format::Opencode(Variant::Dark),
        OC,
    ),
    (
        "gruvbox",
        "oc-gruvbox.json",
        Format::Opencode(Variant::Dark),
        OC,
    ),
    (
        "kanagawa",
        "oc-kanagawa.json",
        Format::Opencode(Variant::Dark),
        OC,
    ),
    (
        "material",
        "oc-material.json",
        Format::Opencode(Variant::Dark),
        OC,
    ),
    (
        "matrix",
        "oc-matrix.json",
        Format::Opencode(Variant::Dark),
        OC,
    ),
    (
        "mercury",
        "oc-mercury.json",
        Format::Opencode(Variant::Dark),
        OC,
    ),
    (
        "nightowl",
        "oc-nightowl.json",
        Format::Opencode(Variant::Dark),
        OC,
    ),
    ("nord", "oc-nord.json", Format::Opencode(Variant::Dark), OC),
    (
        "one-dark",
        "oc-one-dark.json",
        Format::Opencode(Variant::Dark),
        OC,
    ),
    (
        "opencode",
        "oc-opencode.json",
        Format::Opencode(Variant::Dark),
        OC,
    ),
    ("orng", "oc-orng.json", Format::Opencode(Variant::Dark), OC),
    (
        "osaka-jade",
        "oc-osaka-jade.json",
        Format::Opencode(Variant::Dark),
        OC,
    ),
    (
        "palenight",
        "oc-palenight.json",
        Format::Opencode(Variant::Dark),
        OC,
    ),
    (
        "rosepine",
        "oc-rosepine.json",
        Format::Opencode(Variant::Dark),
        OC,
    ),
    (
        "synthwave84",
        "oc-synthwave84.json",
        Format::Opencode(Variant::Dark),
        OC,
    ),
    (
        "tokyonight",
        "oc-tokyonight.json",
        Format::Opencode(Variant::Dark),
        OC,
    ),
    (
        "vercel",
        "oc-vercel.json",
        Format::Opencode(Variant::Dark),
        OC,
    ),
    (
        "vesper",
        "oc-vesper.json",
        Format::Opencode(Variant::Dark),
        OC,
    ),
    (
        "zenburn",
        "oc-zenburn.json",
        Format::Opencode(Variant::Dark),
        OC,
    ),
    (
        "catppuccin-frappe",
        "al-catppuccin_frappe.toml",
        Format::Alacritty,
        AL,
    ),
    (
        "tokyonight-storm",
        "al-tokyo_night_storm.toml",
        Format::Alacritty,
        AL,
    ),
    (
        "rosepine-dawn",
        "al-rose_pine_dawn.toml",
        Format::Alacritty,
        AL,
    ),
    (
        "gruvbox-light",
        "al-gruvbox_light.toml",
        Format::Alacritty,
        AL,
    ),
];

const OC: &str = "sst/opencode @ dev, packages/tui/src/theme/assets/, fetched 2026-09-11";
const AL: &str = "alacritty/alacritty-theme @ master, themes/, fetched 2026-09-11";

fn main() -> ExitCode {
    let Some(dir) = std::env::args().nth(1) else {
        eprintln!("usage: cargo run --example theme_pack_gen -- <sources-dir>");
        return ExitCode::FAILURE;
    };
    let sources = Path::new(&dir);
    let out_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/tui/theme_catalog/pack");
    fs::create_dir_all(&out_dir).expect("create pack dir");
    let mut written = 0;
    let mut failed = 0;
    for (name, src_file, format, upstream) in CURATED {
        let path = sources.join(src_file);
        let Ok(src) = fs::read_to_string(&path) else {
            eprintln!("FAIL {name}: cannot read {}", path.display());
            failed += 1;
            continue;
        };
        let provenance = provenance(name, src_file, format, upstream);
        let result = match format {
            Format::Opencode(variant) => convert_opencode_json(name, &src, *variant, &provenance),
            Format::Alacritty => convert_alacritty_toml(name, &src, &provenance),
        };
        match result {
            Ok(text) => {
                let out = out_dir.join(format!("{name}.toml"));
                fs::write(&out, text).expect("write pack file");
                written += 1;
            }
            Err(e) => {
                eprintln!("FAIL {name}: {e}");
                failed += 1;
            }
        }
    }
    println!(
        "pack generation: {written} written, {failed} failed -> {}",
        out_dir.display()
    );
    if failed > 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn provenance(name: &str, src_file: &str, format: &Format, upstream: &str) -> String {
    let variant = match format {
        Format::Opencode(Variant::Dark) => ", dark variant",
        Format::Opencode(Variant::Light) => ", light variant",
        Format::Alacritty => "",
    };
    format!("source: {upstream} {src_file} -> pack name \"{name}\"{variant}")
}
