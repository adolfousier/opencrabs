# OpenCrabs Desktop UI

OpenCrabs Desktop is a **Dioxus WebAssembly frontend embedded in a Tauri 2 native shell**. It is a **beta-stable, Apple-Silicon macOS build**: the GUI mounts, the backend IPC works, and the verification ladder passes — but it is still **unsigned** (Gatekeeper will prompt), so it is suitable for internal/owner testing, not yet a signed public release.

## Current state

- The Dioxus frontend **mounts and renders** in the native webview (sessions list, providers, brain/config/cron panels). IPC is verified — `list_sessions` returns real data.
- Built via the **`dx` CLI** (the Dioxus-native toolchain), not Trunk. See [ADR-0001](docs/adr/0001-build-dioxus-frontend-with-dx-cli.md) for why.
- Gated by a **reproducible release ladder + native macOS smoke**. See [ADR-0002](docs/adr/0002-native-frontend-mount-verification.md) and [`STABILITY-EVIDENCE.md`](STABILITY-EVIDENCE.md).
- **Not yet done:** code signing / notarization / stapling (needs Apple Developer credentials), and publishing — see the release runbook.

## Run the native desktop app

The desktop GUI must run inside Tauri. A browser-only `dx serve` preview renders the UI but cannot execute OpenCrabs desktop commands (it will honestly report them unavailable).

```text
cd /Users/moe/Desktop/crabz/desktop/src-tauri
cargo tauri dev
```

Tauri runs the frontend via `beforeDevCommand` (`dx serve --port 8080`) and loads `http://localhost:8080` in the webview. To inspect only the frontend layout:

```text
cd /Users/moe/Desktop/crabz/desktop
dx serve --port 8080
```

## Toolchain

| Tool | Version | Why |
|---|---|---|
| Rust | `1.91.0` (edition 2024, pinned by `rust-toolchain.toml`) | Matches the crate `rust-version` |
| `dioxus-cli` (`dx`) | `0.7.9` | Builds and serves the Dioxus WASM frontend the Dioxus way |
| `tauri-cli` | `2.x` | Provides the `cargo tauri` command |
| Node | not required | This path is Rust + dx |

Install the CLI tools:

```text
rustup toolchain install 1.91.0
rustup override set 1.91.0
cargo install dioxus-cli --version 0.7.9 --locked
cargo install tauri-cli --version ^2 --locked
```

## Active structure

- `src/main.rs` — Dioxus entry point: `dioxus::launch(app::App)` (+ a console-based panic hook)
- `src/app.rs` — routed shell and panel components
- `src/bridge.rs` — native Tauri invoke bridge for the WASM frontend
- `src/models.rs` — frontend DTOs matching Tauri command payloads
- `public/css/app.css` — active desktop stylesheet (linked from `index.html`; `dx` copies `public/` to the output root)
- `index.html` — clean Dioxus-native shell (`<div id="main">` + the stylesheet `<link>`); `dx` injects its own WASM loader
- `Dioxus.toml` / `Trunk.toml` — `Dioxus.toml` is the active config; `Trunk.toml` is retained only for reference and is **not** used by the build
- `src-tauri/` — native Tauri backend and command handlers

The desktop DTO/command contract is documented in [`desktop-command-contract.md`](desktop-command-contract.md).

## Frontend ↔ native bridge

The WASM bridge resolves `window.__TAURI__.core.invoke` at runtime and preserves the `core` receiver when invoking commands. It always forwards the serialized argument object, including named arguments such as `sessionId` for `get_session_messages`.

The default desktop shell deliberately uses **request/response IPC only**. It does not register long-lived wasm-bindgen callbacks against Tauri's JavaScript event API during startup: those closures are an independent lifecycle boundary and must be introduced with an explicit unlisten/ownership design, not leaked into the page for streaming convenience. Chat submission therefore waits for the native command result and then reloads the persisted transcript.

This matters because detached JavaScript method calls can lose their receiver and silently drop or mishandle IPC arguments. The bridge fails with an explicit native-runtime error when launched in a browser preview rather than pretending desktop controls are functional.

## Build the frontend the Dioxus way

The frontend is built with `dx`, which writes to `target/dx/opencrabs-desktop-ui/release/web/public` for a release build (this is what `tauri.conf.json`'s `frontendDist` points at). Key facts that bite if forgotten:

- **Build via `dx build --release`, not Trunk.** Building the same frontend via Trunk leaves `dioxus::launch` as a silent no-op — the app never mounts. (ADR-0001.)
- **CSS is linked statically.** The stylesheet lives in `public/css/app.css` and is referenced by a plain `<link rel="stylesheet" href="/css/app.css">` in `index.html`. `dx` copies `public/` to the output root. Trunk's `<link data-trunk rel="css">` is ignored by `dx` — leaving the CSS unlinked produces an unstyled app where adjacent text runs together.
- **wasm-opt/DWARF.** `dx` runs binaryen `wasm-opt` on the release WASM; rustc's DWARF v5 debug sections trigger a binaryen `SIGABRT`. `[profile.release]` strips debug info (`debug = false`, `strip = "debuginfo"`) so wasm-opt runs clean and produces size-optimized output.

## CSP and assets

The Tauri CSP permits `wasm-unsafe-eval` (required for the Dioxus WASM) and `'unsafe-eval'` (used by the launch panic hook). All JavaScript and WASM are self-hosted and content-hashed by `dx` in release builds.

## Chat trace and tool activity

Assistant replies follow the TUI's **progressive-disclosure** hierarchy rather than rendering provider markers as transcript text:

- Final response text remains visible.
- Reasoning is a collapsed **Reasoning trace** disclosure.
- Tool activity is a collapsed count with named success/failure rows; each row exposes raw input and output only when opened.
- Persisted provider thinking and CLI markers (`<!-- reasoning -->`, `<think>`, `<antThinking>`, `<mm:think>`, and `<!-- tools[-v2] -->`) are normalized before rendering.

This keeps the conversation readable while retaining local debugging access. The interface uses native keyboard-accessible `details` controls; no motion is required to inspect activity.

## Verification

Run the complete, fail-closed gate from `desktop/`:

```text
./scripts/release-verify.sh
```

It runs, from the correct crate directories:

```text
# frontend
cargo fmt --check && cargo clippy --all-targets -- -D warnings
cargo check && cargo test
dx build --release
# native
cd src-tauri && cargo fmt --check && cargo clippy --all-targets -- -D warnings
cd src-tauri && cargo check && cargo test
cd src-tauri && cargo tauri build
# smoke
scripts/native-smoke.sh
```

Individual crates:

```text
# frontend, from desktop/
cargo fmt --check && cargo clippy -- -D warnings && cargo test

# native, from desktop/src-tauri/
cargo fmt --check && cargo clippy -- -D warnings && cargo test && cargo tauri build
```

A Cargo command launched from a parent directory without the relevant `Cargo.toml` is an invocation error, not verification evidence.

## Release posture

Desktop distribution is **manual bundles first**:

- update discovery may be available;
- in-app update installation is intentionally unsupported;
- stable public releases require signed artifacts, checksums, and target-platform verification;
- the current macOS build is ARM64 and **unsigned**, so Gatekeeper may require explicit approval (right-click → Open the first time).

See [`docs/release-strategy.md`](docs/release-strategy.md), [`docs/release-runbook.md`](docs/release-runbook.md), [`docs/release-checklist.md`](docs/release-checklist.md), and [`docs/diagnostics.md`](docs/diagnostics.md) for release and support details, and [`docs/adr/`](docs/adr/) for the architecture decisions.

## Known limitations

- **Unsigned.** macOS code signing, notarization, and stapling remain external release gates (the runbook documents the steps; they need Apple Developer credentials).
- Channel status reports configuration/credential readiness, not a live channel connection.
- Provider cancellation is asynchronous and provider-specific.
- Voice and native in-app update installation are explicitly unsupported.
- Session rename/delete, cron creation/deletion/history, dynamic-tool removal, and privileged Brain/config edits use explicit confirmation, but their UX is still being polished before stable distribution.
