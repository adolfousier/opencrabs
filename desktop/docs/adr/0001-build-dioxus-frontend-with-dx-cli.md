# ADR-0001: Build the Dioxus frontend with the `dx` CLI, not Trunk

- **Status:** Accepted (2026-07-26)
- **Deciders:** desktop app maintainer
- **Supersedes:** the original Trunk-based build pipeline

## Context

The desktop app is a **Dioxus 0.7 WebAssembly frontend served inside a Tauri 2
webview**. The original build pipeline drove **Trunk** (`trunk build --release`)
as the WASM bundler — referenced by `tauri.conf.json` (`beforeBuildCommand`),
`release-verify.sh`, and all the docs.

Trunk is a *generic* WASM bundler: it compiles the wasm-bindgen bin, runs
`init()`, and dispatches a `TrunkApplicationStarted` event. That all worked —
`init()` resolved, the event fired. **But Dioxus 0.7's `dioxus::launch` is built
around the `dx` CLI toolchain**, which sets `dioxus-cli-config` build metadata and
the launch prerequisites at build time. When the same frontend was built via
Trunk instead of `dx`, `dioxus::launch()` became a **silent no-op**:

- `main()` ran (proven by a panic-trap test).
- `TrunkApplicationStarted` fired.
- Dioxus produced **zero console output**, never mounted, never panicked, and
  never threw. The app sat on the "Starting desktop workspace…" boot splash
  forever.

This went undetected for a long time because (a) **Tauri swallows the release
webview console**, and (b) **the verification pipeline never launched the
packaged app** — it relied on unit/integration tests and a Trunk release build,
none of which exercise `dioxus::launch` in the actual webview. The defect was
the real reason "the GUI doesn't work" — far more fundamental than the
`wasm-bindgen` closure crash that had been "fixed" earlier.

## Decision

**Build the Dioxus frontend with the `dx` CLI** (`dx build --release`), the
Dioxus-native toolchain, and stop driving Trunk.

Concretely:

- Install `dioxus-cli` **0.7.9** (matching the framework): `cargo install
  dioxus-cli --version 0.7.9 --locked`.
- `tauri.conf.json`: `beforeBuildCommand = "dx build --release"`,
  `beforeDevCommand = "dx serve --port 8080"`,
  `frontendDist = "../target/dx/opencrabs-desktop-ui/release/web/public"` (where
  `dx` writes the release output).
- `release-verify.sh`: the frontend release build is `dx build --release`, not
  `trunk build`.

## Consequences

- **The GUI mounts.** `dioxus::launch` initializes and renders the full UI,
  verified natively (sessions list loaded via IPC).
- **New toolchain dependency.** `dx` must be installed; it's not part of the
  stock Rust toolchain. Pinned to match the framework version.
- **Output location.** `dx` writes to `target/dx/<crate>/release/web/public`, not
  `dist/`; `frontendDist` points there.
- **CSS must be linked the Dioxus way.** Trunk's `<link data-trunk rel="css">`
  is ignored by `dx`. The stylesheet lives in `public/css/app.css` and is linked
  with a plain `<link rel="stylesheet" href="/css/app.css">` in `index.html`,
  which `dx` copies to the output root. (Getting this wrong produces an
  unstyled app — see `STABILITY-EVIDENCE.md`.)
- **wasm-opt/DWARF.** `dx` runs binaryen `wasm-opt` on the release WASM; rustc's
  DWARF v5 debug sections trigger a binaryen `SIGABRT`. Mitigated by stripping
  DWARF in `[profile.release]` (`debug = false`, `strip = "debuginfo"`).

## Alternatives considered

- **Keep Trunk.** Rejected — the silent no-op mount means the app is unusable.
- **`dx` + `manganis::asset!` for CSS.** Tried: the macro bundles the CSS but
  does **not** inject a `<link>` for an unused `const`, so the stylesheet never
  loads. The static `public/` + `<link>` approach is simpler, reliable, and needs
  no extra dependency.

## Compliance

`tauri.conf.json`, `release-verify.sh`, `index.html`, and every doc now name
`dx` (not Trunk) as the frontend builder. A drift back to Trunk would reintroduce
the silent no-op; the native smoke (ADR-0002) would catch it.
