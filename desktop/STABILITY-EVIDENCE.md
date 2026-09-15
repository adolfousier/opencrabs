# OpenCrabs Desktop — Stability Milestone Evidence

**Verdict: BETA-STABLE.** The desktop GUI mounts, IPC works, and the fail-closed
release-verify ladder is green and reproducible. Shipped as an **unsigned beta**
(internal), not for public release until code-signed + notarized.

This conclusion is grounded in two independent evidence streams that agree:
**reproducible verification** (the `release-verify.sh` ladder) and **native
evidence** (the macOS launch smoke + a fresh-install from the DMG).

---

## Reproducible verification — `sh desktop/scripts/release-verify.sh`

Fail-closed (`set -eu` + labeled gates). Run **2026-07-26T19:12:06Z**, build sha
`5f2c2dcb`. Result: **ALL GATES PASSED**.

| Gate | Outcome |
|---|---|
| frontend `cargo fmt --check` | pass |
| frontend `cargo clippy --all-targets -D warnings` | pass |
| frontend `cargo check` | pass |
| frontend `cargo test` | 11 integration + 2 lib pass |
| frontend `dx build --release` + self-consistent assets | pass |
| native `cargo fmt --check` | pass |
| native `cargo clippy --all-targets -D warnings` | pass |
| native `cargo check` | pass |
| native `cargo test` | pass |
| native `cargo tauri build` (OpenCrabs.app + DMG) | pass |
| native macOS launch smoke | pass |

Reproduce on any macOS ARM64 machine with `cargo`, `tauri-cli` (v2), and `dx`
(`cargo install dioxus-cli --version 0.7`) installed: run the script.

## Native evidence

**Launch smoke** (`desktop/scripts/native-smoke.sh`): the packaged app launches,
reaches IPC-readiness (`desktop_smoke: backend_ready config_loaded db_open
state_managed` — the config/DB/state/invoke-handler stack is up), stays alive
through the probe window, and the captured log has no panic/fatal pattern.

**Mount confirmation** (the release-blocking question this milestone exists to
answer): the native app's Dioxus frontend renders. Focused screenshot shows the
full UI — `🦀 OpenCrabs Desktop` / `Dioxus + Tauri command center`, all 9 nav
tabs (💬📁🧠⚙️🛠✨⏱📡📊), `Provider status wired` / `Ready`, the Sessions panel
with `+ New` / `Find sessions`, and **43 of 43 sessions loaded via IPC**.

**Fresh-install from the DMG**: mounted the DMG, copied `OpenCrabs.app` to a
clean location, launched → `backend_ready` + the Dioxus UI mounts (same as the
in-tree build). The distribution path works, not just the build output.

## Artifact identity

- Path: `desktop/src-tauri/target/release/bundle/dmg/OpenCrabs_0.1.0_aarch64.dmg`
- Size: 10 MB
- sha256: `fccc0eec18e1d8febcc295add8958bdb9bdb9ff8a8238468f025c39e9f5377ba`
- Sidecar: `OpenCrabs_0.1.0_aarch64.dmg.sha256`
- Built from commit `5f2c2dcb` (dx frontend) + `7f92a2dc` (release notes)
- Branch: `stable-desktop-gui` (not pushed)

## Residual limitations (why this is a beta, not a public release)

1. **Unsigned / unnotarized.** macOS Gatekeeper will block the DMG. Code-signing
   + notarization require the owner's Apple Developer credentials — runbook is
   in `release-notes-v0.1.0-desktop.md`; not executed.
2. **Build requires the `dx` CLI.** Building via Trunk reintroduces the
   `dioxus::launch` silent-no-op mount defect — the ladder's dx gate guards
   against this. The release WASM is now size-optimized
   (995KB via `[profile.release]` debug=false + strip=debuginfo + lto, and
   `[web.wasm_opt]` level=z + keep_names); `wasm-opt` runs clean (the binaryen
   DWARF SIGABRT is gone).
3. **No automated cross-session upgrade test.** No prior versioned release
   exists to upgrade from; the fresh-install test covers the install path.

## Follow-ups (non-blocking)

- Sign + notarize + staple the DMG (owner credentials) → public release.
- Push `stable-desktop-gui` and open a PR (awaits owner approval).
- Re-enable a deterministic loading/empty-state in `index.html` (the clean
  Dioxus-native shell dropped the boot splash; Dioxus mounts fast enough that
  this is cosmetic).

## Post-beta correction: stylesheet bundling (2026-07-26)

The beta-stable verdict above holds for *mounting*, but a follow-up check found
the rendered UI was **unstyled**: `dx build` was emitting **no CSS at all** —
`src/css/app.css` was only referenced by the old Trunk `<link data-trunk>`, which
`dx` ignores. Without CSS the app rendered inline-styles-only, so Dioxus's
adjacent text nodes jammed together (`wiredReady`, `CronUpdated`,
`No workspacecustom:surplus`) — it looked broken despite mounting.

**Fix:** moved the stylesheet to `public/css/app.css` and linked it statically
(`<link rel="stylesheet" href="/css/app.css">` in `index.html`); `dx` copies
`public/` to the output root. Verified natively — focused screenshot shows proper
spacing (titles separated from timestamps, badges separated from labels). Commit
`657332fe`. (Recorded as a consequence in [ADR-0001](docs/adr/0001-build-dioxus-frontend-with-dx-cli.md).)

**dx build determinism (confirmed):** four consecutive `dx build --release` runs
after the DWARF-strip profile produced **0 wasm-opt SIGABRTs** and a consistent
hashed `assets/` output — no flip to the unoptimized `wasm/` fallback. The
earlier intermittent flip was stale cache from a pre-strip binary; fresh builds
are stable.

## Stability conclusion

The desktop GUI's headline defect (the Dioxus frontend never mounted) is
**resolved and reproducibly verified**: building via `dx` makes `dioxus::launch`
mount, the release-verify ladder is green, the release WASM is now size-optimized
(`wasm-opt` runs clean), and native evidence (launch smoke + mount screenshot + DMG
fresh-install) agrees. Declared **beta-stable**, gated from public release only by
signing/notarization (credential-bound).
