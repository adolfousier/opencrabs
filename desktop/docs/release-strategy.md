# Desktop release strategy

This directory documents the current production release stance for the OpenCrabs desktop app. The build/verify decisions are recorded in [`adr/0001-build-dioxus-frontend-with-dx-cli.md`](adr/0001-build-dioxus-frontend-with-dx-cli.md) and [`adr/0002-native-frontend-mount-verification.md`](adr/0002-native-frontend-mount-verification.md).

## Current release posture

The desktop app supports **signed manual bundle distribution first**. It does **not** self-install updates. The build is **beta-stable but unsigned** (ARM64 macOS): the GUI mounts and IPC is verified, but Gatekeeper will prompt until the artifact is signed/notarized.

What exists now:

- Dioxus WASM frontend embedded in Tauri 2, built via the **`dx` CLI**
- reproducible `cargo tauri build` bundle path, wrapped by `./scripts/release-verify.sh` (fmt/clippy/test for both crates, `dx build --release`, tauri bundle)
- a **native macOS smoke** (`scripts/native-smoke.sh`) that launches the packaged `.app` and verifies launch + `backend_ready` + survival + a clean log + a screenshot
- update discovery command (`check_for_updates`)
- explicit non-support for in-app install (`install_update` returns an honest error)
- macOS `.app` and `.dmg` bundle generation verified locally on 2026-07-26

What does **not** exist yet:

- platform-native in-app installer/update application flow
- restart-and-swap lifecycle per platform
- signed updater feed validation path
- code signing, notarization, and stapling (the runbook documents the steps; they need Apple Developer credentials)
- rollback automation

## Release lanes

| Lane | Purpose | Rule |
|---|---|---|
| `dev` | local engineering builds | may be unsigned/local-only |
| `beta` | pre-release manual QA | signed whenever platform requires it |
| `stable` | user-facing production release | signed artifacts, checksums, and platform verification required |

## Distribution path

1. Run `./scripts/release-verify.sh` from `desktop/`; it runs frontend checks/tests/release build, backend checks/tests, and `cargo tauri build` from their correct directories.
2. Compute checksums for generated artifacts, for example: `shasum -a 256 src-tauri/target/release/bundle/dmg/*.dmg`.
3. Sign and notarize artifacts as required by the target platform.
4. Staple macOS notarization tickets before publishing (`xcrun stapler staple <app-or-dmg>`).
5. Test a fresh install and an upgrade from the previous supported version.
6. Publish release notes, signed artifacts, and checksums together.
7. Desktop clients may detect a newer version, but installation happens outside the running app until updater install is implemented for real.

## Guardrails

- Do **not** pretend the desktop app can self-update yet.
- Keep `install_update` unsupported until the full install/restart path is implemented and tested.
- Any UI button for update installation must remain hidden or explicitly disabled until that contract changes.
- If using `/evolve`, treat it as an OpenCrabs runtime upgrade path, not as a generic desktop GUI updater.

## Future upgrade path options

Choose one before claiming desktop auto-update support:

1. **Tauri updater** — signed feed per platform; download, verify, install, and restart semantics; per-platform QA required.
2. **OpenCrabs-managed updater wrapper** — desktop delegates to a controlled OpenCrabs-native runtime path; still requires explicit restart/install behavior.
3. **Remain manual** — simplest and most honest if operational burden is acceptable.

## Release checklist seed

- [ ] `./scripts/release-verify.sh` passes from `desktop/`
- [ ] target-platform signing/notarization/stapling is complete
- [ ] fresh-install and upgrade checks pass
- [ ] release notes are written
- [ ] SHA-256 checksums are generated and published
- [ ] updater/install UX matches actual capability
