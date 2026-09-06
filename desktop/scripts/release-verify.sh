#!/usr/bin/env sh
# Release verification ladder for the OpenCrabs desktop app.
#
# Every gate is fail-closed: the first failure aborts the run with a clear GATE
# label. Deterministic evidence is written under .verification/.
#
# The Dioxus frontend is built with the `dx` CLI (the Dioxus way) — building via
# Trunk left `dioxus::launch` as a silent no-op, so the frontend never mounted.
#
# Gates, in order:
#   frontend: fmt, clippy -D warnings, check, test, dx release build,
#             hashed JS+WASM assets present in the dx output
#   native:   fmt, clippy -D warnings, check, test, tauri bundle (uses dx build)
#   smoke:    native macOS launch + IPC-readiness + liveness + log-clean + mount screenshot
#
# Run from anywhere:  sh desktop/scripts/release-verify.sh
# Needs an interactive macOS session for the final smoke gate (GUI launch).
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$root"
evidence="$root/.verification"
mkdir -p "$evidence"

DX_RELEASE_OUT="$root/target/dx/opencrabs-desktop-ui/release/web/public"

gate() {
  printf '\n=== GATE: %s ===\n' "$1"
}

need() {
  if ! command -v "$1" >/dev/null 2>&1; then
    printf '%s\n' "Missing required release tool: $1" >&2
    exit 1
  fi
}

need dx
need cargo
cargo tauri --version >/dev/null 2>&1 || {
  printf '%s\n' 'Missing required release tool: tauri-cli (install with cargo install tauri-cli --version "^2" --locked)' >&2
  exit 1
}
dx --version >/dev/null 2>&1 || {
  printf '%s\n' 'Missing required release tool: dx (install with cargo install dioxus-cli --version 0.7 --locked)' >&2
  exit 1
}

# ---------------- frontend gates ----------------
gate "frontend: cargo fmt --check"
cargo fmt --check

gate "frontend: cargo clippy -D warnings (all-targets)"
cargo clippy --all-targets -- -D warnings

gate "frontend: cargo check"
cargo check --message-format short

gate "frontend: cargo test"
cargo test --message-format short

gate "frontend: dx build --release + self-consistent assets"
dx build --release
INDEX="$DX_RELEASE_OUT/index.html"
[ -f "$INDEX" ] \
  || { printf '%s\n' "FAIL: dx output index.html missing at $INDEX" >&2; exit 1; }
# index.html references the JS bundle via <script ... src="...">. Extract and
# verify the referenced file exists on disk. This is robust to dx's two release
# output shapes (hashed under assets/ when wasm-opt runs, unhashed under wasm/
# when binaryen's DWARF emitter SIGABRTs and dx falls back) — both are
# self-consistent and mount; we only require internal consistency.
js_rel=$(grep -oE 'src="[^"]*opencrabs-desktop-ui[^"]*\.js"' "$INDEX" | head -1 \
  | sed -E 's/src="([^"]+)"/\1/' | sed -E 's#^/(\./)?##')
[ -n "$js_rel" ] \
  || { printf '%s\n' 'FAIL: no JS bundle referenced in dx output index.html' >&2; exit 1; }
[ -f "$DX_RELEASE_OUT/$js_rel" ] \
  || { printf '%s\n' "FAIL: $DX_RELEASE_OUT/$js_rel missing" >&2; exit 1; }
# A WASM bundle must exist (hashed under assets/ or unhashed under wasm/).
if ! { ls "$DX_RELEASE_OUT"/assets/*.wasm >/dev/null 2>&1 \
    || [ -f "$DX_RELEASE_OUT/wasm/opencrabs-desktop-ui_bg.wasm" ]; }; then
  printf '%s\n' 'FAIL: no WASM bundle in dx output' >&2; exit 1
fi
printf '%s\n' "  dx release assets OK: js=$js_rel"

# ---------------- native gates ----------------
gate "native: cargo fmt --check"
( cd src-tauri && cargo fmt --check )

gate "native: cargo clippy -D warnings (all-targets)"
( cd src-tauri && cargo clippy --all-targets -- -D warnings )

gate "native: cargo check"
( cd src-tauri && cargo check --message-format short )

gate "native: cargo test"
( cd src-tauri && cargo test --message-format short )

gate "native: tauri bundle (release; runs dx build via beforeBuildCommand)"
( cd src-tauri && cargo tauri build )

# ---------------- smoke gate ----------------
gate "native: macOS launch smoke"
sh "$root/scripts/native-smoke.sh"

printf '\n=== ALL GATES PASSED ===\n'
sha=$(git rev-parse --short HEAD 2>/dev/null || echo unknown)
printf '%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ) release-verify: ALL GATES PASSED (sha $sha)" > "$evidence/release-verify.txt"
cat "$evidence/release-verify.txt"
