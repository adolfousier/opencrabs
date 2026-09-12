# Verification notes

This document captures the current verification pyramid for the desktop frontend and native Tauri shell.

## Scope

The desktop app consists of:

- a Dioxus WebAssembly frontend crate at `desktop/`;
- a native Tauri 2 shell/backend crate at `desktop/src-tauri/`.

Verification is meaningful only when commands run from the relevant crate directory.

## Automated coverage

### Frontend (`desktop/`)

- DTO contract smoke tests in `tests/models_contract.rs`
- integration smoke checks in `tests/integration_smoke.rs`
- bridge tests covering serialized named Tauri arguments and browser-preview failures

### Native shell (`desktop/src-tauri/`)

- file/path containment
- Brain-file guards
- config allowlists
- chat/cancellation
- onboarding/channel/update/voice truthfulness
- diagnostics redaction and persisted-pane state

## Commands

Frontend, from `desktop/`:

```text
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo check --message-format short
cargo test --message-format short
dx build --release
```

Native shell, from `desktop/src-tauri/`:

```text
cargo fmt --check
cargo check --message-format short
cargo test --message-format short
cargo tauri build
```

Complete release gate, from `desktop/`:

```text
./scripts/release-verify.sh
```

## Runtime smoke expectations

A native desktop smoke pass must use:

```text
cd /Users/moe/Desktop/crabz/desktop/src-tauri
cargo tauri dev
```

Verify that the Tauri window renders the Dioxus shell, session selection loads messages, and a desktop action either succeeds or surfaces an actionable error. A browser-only `dx serve` preview is not valid IPC verification because `window.__TAURI__` is absent by design.

## Verified local evidence

The macOS ARM64 preview bundle is produced at:

```text
src-tauri/target/release/bundle/macos/OpenCrabs.app
src-tauri/target/release/bundle/dmg/OpenCrabs_0.1.0_aarch64.dmg
```

Before publishing an artifact, rerun the complete release gate and record the exact command output, platform, artifact checksum, and signing/notarization evidence in the release notes. Do not treat old test counts or a build run from `/Users/moe` as current release evidence.

## What this verification does not claim

- macOS signing, notarization, or stapling is complete;
- every supported platform was verified;
- accessibility review is complete;
- every loading, empty, and offline state has been UX-reviewed;
- desktop auto-install updates are supported.

## External release gates

- sign, notarize, and staple macOS artifacts before stable release;
- verify fresh install and upgrade behavior;
- run the release gate on each target platform;
- publish SHA-256 checksums and release notes with each artifact;
- preserve release logs/evidence alongside release notes.
