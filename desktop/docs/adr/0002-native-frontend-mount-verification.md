# ADR-0002: Verify desktop releases with a reproducible gate ladder + native smoke

- **Status:** Accepted (2026-07-26)
- **Deciders:** desktop app maintainer
- **Related:** [ADR-0001](0001-build-dioxus-frontend-with-dx-cli.md)

## Context

The silent mount defect (ADR-0001) shipped undetected because verification
**never launched the packaged app**. The old checks were all static or compile-time:

- `cargo fmt` / `cargo clippy` / `cargo test` for both crates — these verify code,
  not runtime behaviour.
- A Trunk release build + a check that `dist/index.html` referenced a hashed
  asset pair — this proves the bundler ran, **not** that the frontend renders.

None of those exercise `dioxus::launch` in the actual Tauri webview. And Tauri
swallows the release webview console, so a failed mount was indistinguishable
from "still loading" — an eternal boot splash, no error, no panic.

## Decision

**Gate desktop releases on two agreeing evidence streams:**

1. **A reproducible, fail-closed ladder** — `desktop/scripts/release-verify.sh`,
   which runs (each gate stops the run on failure with a clear label):
   - frontend: `cargo fmt --check`, `cargo clippy -- -D warnings`,
     `cargo check`, `cargo test`, `dx build --release`, and a self-consistent
     release-asset check (index.html references a JS bundle that exists + a WASM
     bundle exists).
   - native: the same four gates for the `src-tauri` crate, then
     `cargo tauri build` (produces the `.app` + `.dmg`).
2. **A native macOS smoke** — `desktop/scripts/native-smoke.sh`, which launches
   the packaged `.app`, verifies `backend_ready` (config + DB + state + handler
   up), confirms the process survived the probe window, scans the captured log
   for fatal patterns, captures a screenshot, and emits a JSON manifest. A
   mount-failure investigation supplements this with a browser-served
   **unswallowed console** check (serve the release assets to a real browser and
   read the actual Dioxus console).

## Consequences

- **Silent mount failures are caught before shipping.** The ladder surfaced the
  Trunk no-op; it would surface any future regression of the same class.
- **The smoke needs a window-server session** (it launches a GUI app + takes a
  screenshot), so it runs on the dev Mac, not headless CI. CI runs the
  non-smoke gates.
- **Mount evidence is screenshot-based.** The webview DOM isn't directly
  inspectable from a shell, so "did Dioxus render the UI?" is judged from the
  captured screenshot (or the browser console during investigation).
- **`backend_ready` is an env-gated stderr marker** emitted by the native `run()`
  when `OPENCRABS_DESKTOP_SMOKE=1`, so the smoke can deterministically confirm
  the IPC layer came up without relying on UI automation.

## Alternatives considered

- **WebKit remote inspector.** Requires building Tauri with the `devtools`
  feature and driving an interactive Safari session — heavyweight and not
  automatable headlessly.
- **WebDriver.** macOS WebKit lacks the WebDriver support that exists on other
  platforms, so it can't drive the Tauri webview.
- **Trust the unit/integration tests alone.** Rejected — that's exactly the gap
  that let the mount defect ship.

## Compliance

A release is not promotable past `beta` unless both streams pass on the target
platform, with the smoke manifest + screenshot recorded as evidence
(`STABILITY-EVIDENCE.md`).
