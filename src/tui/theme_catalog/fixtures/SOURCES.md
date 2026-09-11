# Theme catalog fixtures — provenance

Byte-identical copies of upstream theme sources, used by the converter tests
(`src/tests/theme_catalog_converter_test.rs`) and as the generation inputs for
the curated pack. Fetched 2026-09-11.

| File | Upstream | URL |
|---|---|---|
| `tokyo_night.alacritty.toml` | `alacritty/alacritty-theme` @ master, `themes/tokyo_night.toml` | https://raw.githubusercontent.com/alacritty/alacritty-theme/master/themes/tokyo_night.toml |
| `tokyonight.opencode.json` | `sst/opencode` @ dev, `packages/tui/src/theme/assets/tokyonight.json` | https://raw.githubusercontent.com/sst/opencode/dev/packages/tui/src/theme/assets/tokyonight.json |

The opencode JSON token format is documented at https://opencode.ai/docs/themes/
(`defs` named colors + `theme` token map; values are hex strings, ANSI color
integers 0-15, `defs` references, `"none"`, or `{dark, light}` variant objects).
