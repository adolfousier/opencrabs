# Production release runbook

This runbook makes the OpenCrabs desktop build repeatable instead of vibes-based.
It assumes the decisions in [`adr/0001-build-dioxus-frontend-with-dx-cli.md`](adr/0001-build-dioxus-frontend-with-dx-cli.md) (build via `dx`) and [`adr/0002-native-frontend-mount-verification.md`](adr/0002-native-frontend-mount-verification.md) (verify via reproducible ladder + native smoke).

## Supported shape

- Frontend: Dioxus WebAssembly built via the **`dx` CLI** (the Dioxus-native toolchain)
- Native shell: Tauri 2
- Distribution: manual bundles first
- Current build: ARM64 macOS, **beta-stable but unsigned**
- Update installation: intentionally unsupported

## Toolchain contract

| Tool | Required version |
|---|---|
| Rust | `1.91.0` (edition 2024; pinned by `desktop/rust-toolchain.toml`) |
| `dioxus-cli` (`dx`) | `0.7.9` |
| `tauri-cli` | `2.x` |

Bootstrap from `desktop/`:

```text
rustup toolchain install 1.91.0
rustup override set 1.91.0
cargo install dioxus-cli --version 0.7.9 --locked
cargo install tauri-cli --version ^2 --locked
```

The installable package is `dioxus-cli`; the command it provides is `dx`. The Tauri package is `tauri-cli`; its command is `cargo tauri`.

## Native development

Run the GUI inside Tauri, not only in a browser preview:

```text
cd /Users/moe/Desktop/crabz/desktop/src-tauri
cargo tauri dev
```

Tauri runs the frontend through `beforeDevCommand` (`dx serve --port 8080`) and loads `http://localhost:8080` in the webview. A browser-only preview (open `http://localhost:8080` yourself) is useful for layout inspection but cannot execute desktop IPC commands — it honestly reports them unavailable.

## Release verification

From `desktop/`, run the single, fail-closed gate:

```text
./scripts/release-verify.sh
```

It executes, from the correct crate directories:

```text
# frontend
cargo fmt --check && cargo clippy --all-targets -- -D warnings
cargo check && cargo test
dx build --release
# native
cd src-tauri && cargo fmt --check && cargo clippy --all-targets -- -D warnings
cd src-tauri && cargo check && cargo test
cd src-tauri && cargo tauri build
# native macOS smoke (launches the packaged .app, verifies backend_ready + survived + log-clean + screenshot)
scripts/native-smoke.sh
```

A Cargo error from a parent directory without the relevant manifest is an invocation error, not a build verdict. Preserve successful command output with the release notes, target platform, artifact version, and signing evidence.

## Frontend build contract (the parts that bite)

`dx build --release` writes to `target/dx/opencrabs-desktop-ui/release/web/public`; `tauri.conf.json`'s `frontendDist` points there. Before packaging, sanity-check three things that silently break the app if wrong:

```text
DXOUT=target/dx/opencrabs-desktop-ui/release/web/public
# 1. dx actually produced the output
test -f "$DXOUT/index.html"
# 2. index.html references a JS bundle that exists on disk
# 3. a WASM bundle exists (hashed under assets/ or unhashed under wasm/)
ls "$DXOUT"/assets/*.wasm "$DXOUT"/wasm/*.wasm 2>/dev/null
```

Common silent breakages:

- **Built via Trunk instead of `dx`.** The same frontend built via Trunk leaves `dioxus::launch` a silent no-op — the app never mounts, just an eternal boot splash. Always build with `dx`. (ADR-0001.)
- **CSS not linked.** The stylesheet lives in `public/css/app.css` and is referenced by `<link rel="stylesheet" href="/css/app.css">` in `index.html`. `dx` copies `public/` to the output root. If the link is missing, the app renders unstyled — adjacent text runs together (`wiredReady`, `CronUpdated`).
- **wasm-opt/DWARF SIGABRT.** `dx` runs binaryen `wasm-opt` on the release WASM; rustc's DWARF v5 debug sections trigger a binaryen abort that drops `dx` into an unoptimized fallback. `[profile.release]` strips debug info (`debug = false`, `strip = "debuginfo"`) so wasm-opt runs clean.

## CSP and bridge contract

The Tauri CSP permits `wasm-unsafe-eval` (required for the Dioxus WASM) and `'unsafe-eval'` (used by the launch panic hook). If a build changes the inline startup content, update the CSP in `src-tauri/tauri.conf.json`; do not weaken it past what the build actually needs.

The WASM bridge resolves `window.__TAURI__.core.invoke` at runtime, preserves `core` as the JavaScript receiver, and always passes the serialized arguments object. Named Tauri arguments, including `sessionId`, therefore reach native commands reliably. Chat uses request/response IPC only — no long-lived streaming event closures.

## Package and publish

1. Run `./scripts/release-verify.sh` from `desktop/` — all gates must pass.
2. Compute checksums, for example:
   ```text
   shasum -a 256 src-tauri/target/release/bundle/dmg/*.dmg
   ```
3. Sign and notarize platform artifacts (see signing tasks below).
4. Staple macOS notarization tickets:
   ```text
   xcrun stapler staple <app-or-dmg>
   ```
5. Test a fresh install and (when a prior version exists) an upgrade.
6. Publish release notes, artifacts, and checksums together.

## Release lanes

| Lane | Purpose | Rule |
|---|---|---|
| `dev` | local engineering builds | may be unsigned/local-only |
| `beta` | pre-release QA | signed whenever the platform requires it |
| `stable` | user-facing production release | signed artifacts, checksums, and platform verification required |

## Current updater policy

- `check_for_updates`: partial; it may report available releases.
- `install_update`: unsupported; no in-app native installer/restart path exists.
- Do not expose an enabled Install button until the full download, verification, install, and restart lifecycle is implemented and tested.
- `/evolve` is an OpenCrabs runtime upgrade path, not generic desktop-GUI updater UX.

## Platform signing tasks

### macOS

- produce `.app` and `.dmg`
- sign with Developer ID: `codesign --deep --force --options runtime --entitlements src-tauri/entitlements.plist --sign "Developer ID Application: <Name>" <app>`
- notarize: `xcrun notarytool submit <dmg> --keychain-profile "<profile>" --wait`
- staple the notarization ticket: `xcrun stapler staple <app-or-dmg>`
- verify: `xcrun stapler validate <app-or-dmg>`

### Linux

- produce intended package formats (`.AppImage`, `.deb`, etc.)
- sign where the distribution channel requires it

### Windows

- produce the intended installer/bundle
- Authenticode-sign before distribution
