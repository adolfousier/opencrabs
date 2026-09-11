# Curated theme pack (#1461)

Generated TOML files in this directory ship embedded in the binary as
built-in themes (see `../theme_pack.rs`). Each file carries a `# source:`
provenance header naming the upstream repo, file, and fetch date, and every
file passes the existing user-theme validator (`user_themes::build_theme`)
unchanged — that invariant is enforced at generation time by the converter
and re-checked by `src/tests/theme_pack_test.rs`.

**Do not hand-edit generated files.** Change the curation or mapping and
regenerate.

## Regeneration

```sh
# 1. fetch the pinned upstream sources into a scratch dir
mkdir -p /tmp/theme_sources && cd /tmp/theme_sources
for t in aura ayu carbonfox catppuccin-macchiato cobalt2 cursor everforest \
         flexoki github gruvbox kanagawa material matrix mercury nightowl \
         nord one-dark opencode orng osaka-jade palenight rosepine \
         synthwave84 tokyonight vercel vesper zenburn; do
  curl -sfL "https://raw.githubusercontent.com/sst/opencode/dev/packages/tui/src/theme/assets/$t.json" -o "oc-$t.json" || echo "FAIL oc-$t"
done
for t in catppuccin_frappe tokyo_night_storm rose_pine_dawn gruvbox_light; do
  curl -sfL "https://raw.githubusercontent.com/alacritty/alacritty-theme/master/themes/$t.toml" -o "al-$t.toml" || echo "FAIL al-$t"
done
cd -

# 2. regenerate the pack (refuses to write any theme that fails conversion)
cargo run --example theme_pack_gen -- /tmp/theme_sources

# 3. run the pack tests, review the diff, commit
cargo test --all-features --lib theme_pack
```

Upstream refs at curation time (2026-09-11): `sst/opencode` @ `dev`,
`packages/tui/src/theme/assets/`; `alacritty/alacritty-theme` @ `master`,
`themes/`. If upstream renames or drops a file, update the `CURATED` table
in `examples/theme_pack_gen.rs` and this loop together.

## Curation notes

- 27 opencode themes (dark variants; every `dark` background verified
  genuinely dark, luminance <= 0.034) + 4 alacritty themes covering family
  gaps and light-mode variety (`catppuccin-frappe`, `tokyonight-storm`,
  `rosepine-dawn`, `gruvbox-light`) = 31 themes.
- **lucent-orng excluded**: its `background` token is `"transparent"` in
  both variants; the required `ink` role has no transparent mapping.
- opencode themes colliding with hand-built presets are excluded by the
  curation (dracula, monokai, catppuccin mocha/latte, solarized light/dark
  already ship in `render/presets.rs`).
- alacritty `tokyo_night.toml` and `tokyonight.opencode.json` describe the
  same upstream palette family but different authors' takes; the pack ships
  the opencode variant as `tokyonight` and the alacritty storm variant as
  `tokyonight-storm`. The plain alacritty tokyo_night lives in
  `fixtures/` as the converter's test sample.
