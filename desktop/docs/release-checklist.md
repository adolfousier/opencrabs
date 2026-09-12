# Release readiness checklist

Use this before promoting OpenCrabs Desktop beyond a preview.

## Native runtime

- [ ] Launch the native shell with `cd desktop/src-tauri && cargo tauri dev`.
- [ ] Confirm the Dioxus shell renders and status reaches its ready state.
- [ ] Select a session and confirm `get_session_messages` receives its named `sessionId` argument.
- [ ] Exercise at least one mutation and confirm success or actionable failure feedback.
- [ ] Inspect the native WebView console when a runtime action fails.

## Verification

- [ ] Run `./scripts/release-verify.sh` from `desktop/` on the release target — all gates (fmt/clippy/test for both crates, `dx build --release`, tauri bundle, native smoke) must pass.
- [ ] Confirm `dx build --release` produced self-consistent assets: `index.html` references a JS bundle that exists on disk and a WASM bundle exists (hashed under `assets/`, or unhashed under `wasm/` if wasm-opt fell back).
- [ ] Confirm the native smoke (`scripts/native-smoke.sh`) launched the packaged `.app` and verified `backend_ready` + survival + a clean log + captured a screenshot.
- [ ] Confirm the packaged app's Dioxus UI actually **mounts** (judge from the smoke screenshot; if uncertain, serve the release assets to a real browser and read the unswallowed console — it must render the UI, not sit on a splash).
- [ ] Record command output, platform, Git commit, and artifact version with release notes.
- [ ] Repeat on every supported target platform before stable distribution.

## Security

- [x] CSP permits only what the build needs (`'self'`, `'wasm-unsafe-eval'` for the Dioxus WASM, `'unsafe-eval'` for the launch panic hook); no broad `unsafe-inline` script allowance.
- [x] untrusted model/provider output is not rendered as raw HTML.
- [x] file read/write/list commands enforce allowlisted roots and containment.
- [x] config writes remain explicitly allowlisted.
- [x] secrets are masked in UI/logs and written only through constrained codepaths.
- [x] unused shell/dialog/fs plugins and broad capabilities were removed.

## Backend truthfulness

- [x] desktop commands are marked ready, partial, or unsupported in `desktop-command-contract.md`.
- [x] unsupported commands fail honestly rather than returning fake success.
- [x] health checks expose observable filesystem/config/tool state.
- [x] voice and update-install surfaces state their unsupported status.
- [ ] Channel UI continues to distinguish credential readiness from live connectivity.

## Distribution

- [ ] Choose the `dev`, `beta`, or `stable` lane.
- [ ] Produce signed artifacts for the target platform.
- [ ] Complete macOS notarization and stapling where applicable.
- [ ] Test fresh installation and upgrade from the prior supported release.
- [ ] Publish release notes and SHA-256 checksums.
- [ ] Document rollback for the selected release lane.

## Observability and UX

- [x] frontend/backend action failures surface enough detail for bounded diagnostics.
- [x] a credential-redacted diagnostics snapshot exists.
- [ ] Verify diagnostics handling against at least one real failure mode.
- [ ] Review loading, empty, and offline states for every panel.
- [ ] Complete keyboard, focus, contrast, and reduced-motion accessibility review.
- [ ] Confirm destructive actions retain explicit confirmation.
