# OpenCrabs Desktop v0.1.0-beta (macOS ARM64)

First **verified** desktop GUI release. The Dioxus frontend actually mounts and
the native smoke gate proves it.

## What this is
The OpenCrabs desktop app: a Dioxus 0.7 web frontend running in a Tauri 2
webview, talking to the OpenCrabs backend over Tauri IPC. Sessions, files,
brain, config, tools/skills, cron, channels, diagnostics, usage panels.

## The headline fix
The GUI previously showed an eternal "Starting desktop workspace…" splash. Root
cause: the Dioxus frontend was built with **Trunk**, which left `dioxus::launch`
as a silent no-op (the WASM loaded, `main()` ran, `TrunkApplicationStarted`
fired — but Dioxus rendered nothing; zero console output, no panic). Building
the frontend with the **`dx` CLI (the Dioxus way)** makes it mount. Verified in
a real browser (full UI renders) and in the native Tauri webview (screenshot
shows the mounted UI with 43 sessions loaded via IPC).

## What changed (since the unverified prerelease)
- Frontend build switched Trunk → **`dx build --release`** (resolves the mount
  defect). `tauri.conf.json` `beforeBuildCommand`/`beforeDevCommand` use `dx`;
  `frontendDist` points at dx's release output.
- `index.html` is now a clean Dioxus-native shell (mount root `#main`; dx
  injects its own wasm loader). Removed the obsolete Trunk markup + watchdog.
- `main.rs`: clean `dioxus::launch` + a `console.error` panic hook (surfaces a
  launch panic even though Tauri hides the webview console in release).
- Verification ladder rewritten for the `dx` build with a structure-agnostic
  asset gate.
- Backend hardening (earlier tasks): validated sensitive command boundaries
  (cron/dynamic-tool/config/brain mutations), reconciled the approval-policy
  vocabulary, removed the unsafe Tauri event-stream surface, truthful UI
  contracts.

## Verification (all green, run 2026-07-26T19:12Z, sha `5f2c2dcb`)
`sh desktop/scripts/release-verify.sh` — fail-closed ladder:
- frontend: `cargo fmt --check`, `cargo clippy -D warnings`, `cargo check`,
  `cargo test` (11 integration + 2 lib), `dx build --release`, self-consistent
  release assets.
- native: `cargo fmt --check`, `cargo clippy -D warnings`, `cargo check`,
  `cargo test`, `cargo tauri build` (OpenCrabs.app + DMG).
- native macOS smoke: launch + `backend_ready` (config/db/state/handler up,
  i.e. the IPC layer) + liveness + clean log + mount screenshot. **App mounts.**
- fresh-install from the DMG: mount → copy → launch → **mounts**, IPC works.

## Artifact
- `OpenCrabs_0.1.0_aarch64.dmg` (10 MB)
- sha256: `fccc0eec18e1d8febcc295add8958bdb9bdb9ff8a8238468f025c39e9f5377ba`

## ⚠️ Not signed / not notarized — beta, not for public release
This DMG is **ad-hoc / unsigned**. macOS Gatekeeper will block it. Until it is
code-signed with an Apple Developer ID certificate and notarized, treat it as an
internal beta only.

### Signing + notarization runbook (requires owner Apple Developer credentials)
Run after the release-verify ladder passes, from `desktop/`:
```sh
# 1. Sign the app (Developer ID Application: <Team>)
APP="src-tauri/target/release/bundle/macos/OpenCrabs.app"
codesign --deep --force --options runtime \
  --sign "Developer ID Application: <YOUR NAME> (<TEAMID>)" \
  --entitlements src-tauri/entitlements.plist "$APP"

# 2. Notarize the zipped app (App Store Connect API key or Apple ID)
ditto -c -k --keepParent "$APP" /tmp/OpenCrabs.zip
xcrun notarytool submit /tmp/OpenCrabs.zip \
  --keychain-profile "opencrabs-notary" --wait
xcrun stapler staple "$APP"

# 3. Re-build the DMG from the signed+stapled app, then notarize + staple the DMG
sh src-tauri/target/release/bundle/dmg/bundle_dmg.sh   # or cargo tauri build
xcrun notarytool submit OpenCrabs_0.1.0_aarch64.dmg \
  --keychain-profile "opencrabs-notary" --wait
xcrun stapler staple OpenCrabs_0.1.0_aarch64.dmg
shasum -a 256 OpenCrabs_0.1.0_aarch64.dmg > OpenCrabs_0.1.0_aarch64.dmg.sha256
```
Publication (GitHub Release, etc.) **awaits explicit owner approval**.

## Known issues
- Building requires the `dx` CLI (`cargo install dioxus-cli --version 0.7`).
- The release WASM is size-optimized (`wasm-opt` runs at level `z` with LTO;
  the binaryen DWARF SIGABRT that previously forced an unoptimized fallback is
  resolved by stripping debug info from the cargo wasm build via
  `[profile.release] debug=false + strip=debuginfo`).
