#!/usr/bin/env sh
# Native macOS smoke test for the packaged OpenCrabs desktop app.
#
# Proves the BUILT bundle actually launches and reaches IPC-readiness without
# the startup WASM-closure crash that motivated this whole effort. Deterministic
# pass/fail is based on four machine-checkable signals:
#
#   launched      - the packaged binary process started
#   backend_ready - the OPENCRABS_DESKTOP_SMOKE env-gated marker fired in the
#                   log, proving config load + db open + state managed + handler
#                   registered (i.e. the IPC layer is up and ready to answer)
#   survived      - the process stayed alive through the probe window
#   log_clean     - no panic / fatal / closure-exception in the captured log
#
# A screenshot is captured as a mount/render evidence ARTIFACT. It is judged by a
# human or the agent (does the window show the rendered UI?) — never by this
# script — so a missing screenshot does not fail a headless run by itself.
#
# Evidence is written under .verification/native-smoke/.
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
evidence="$root/.verification/native-smoke"
mkdir -p "$evidence"
log="$evidence/app.log"
shot="$evidence/app.png"
manifest="$evidence/manifest.json"

printf '%s\n' '--- native macOS smoke ---'

# Discover the packaged bundle + its Mach-O dynamically. Tauri names the
# executable after the Cargo binary (e.g. opencrabs-desktop), which need not
# match the productName used for the .app directory — so never hardcode it.
bundle=""
for d in "$root/src-tauri/target/release/bundle/macos"/*.app; do
  [ -d "$d" ] && bundle="$d" && break
done
app=""
if [ -n "$bundle" ]; then
  for f in "$bundle/Contents/MacOS/"*; do
    [ -x "$f" ] && app="$f" && break
  done
fi

if [ -z "$app" ] || [ ! -x "$app" ]; then
  printf '%s\n' "FAIL: packaged executable not found under $bundle" >&2
  printf '%s\n' "      run 'cargo tauri build' (or release-verify.sh) first." >&2
  exit 1
fi

rm -f "$log" "$shot" "$manifest"

# Launch the packaged binary with the smoke env var, capturing all output.
OPENCRABS_DESKTOP_SMOKE=1 "$app" >"$log" 2>&1 &
pid=$!

cleanup() {
  if kill -0 "$pid" 2>/dev/null; then
    kill "$pid" 2>/dev/null || true
    sleep 1
    kill -9 "$pid" 2>/dev/null || true
  fi
}
trap cleanup EXIT INT TERM

# Poll up to 25s for the deterministic backend_ready marker.
launched=0
backend_ready=0
if kill -0 "$pid" 2>/dev/null; then
  launched=1
fi
deadline=$(( $(date +%s) + 25 ))
while [ "$(date +%s)" -lt "$deadline" ]; do
  if ! kill -0 "$pid" 2>/dev/null; then
    break
  fi
  if grep -q 'desktop_smoke: backend_ready' "$log" 2>/dev/null; then
    backend_ready=1
    break
  fi
  sleep 0.5
done

# Give the window time to fully mount the WASM frontend (Dioxus load + initial
# IPC fetches). Must exceed the index.html startup-watchdog (10s) so a pending
# init is surfaced into the page before the screenshot is taken.
sleep 14
shot_ok=0
if command -v screencapture >/dev/null 2>&1; then
  if screencapture -x "$shot" 2>/dev/null && [ -s "$shot" ]; then
    shot_ok=1
  fi
fi

# Final liveness check.
survived=0
if kill -0 "$pid" 2>/dev/null; then
  survived=1
fi

# Fatal-pattern scan of the captured log.
log_clean=1
if grep -qiE 'panic|fatal|RUST_BACKTRACE|error while running tauri|closure.*trampoline' "$log" 2>/dev/null; then
  log_clean=0
fi

# Emit a deterministic JSON manifest.
{
  printf '%s\n' "{"
  printf '  "binary": "%s",\n' "$app"
  printf '  "pid": %s,\n' "$pid"
  printf '  "launched": %s,\n' "$launched"
  printf '  "backend_ready": %s,\n' "$backend_ready"
  printf '  "survived": %s,\n' "$survived"
  printf '  "log_clean": %s,\n' "$log_clean"
  printf '  "screenshot": "%s",\n' "$shot"
  printf '  "screenshot_captured": %s\n' "$shot_ok"
  printf '%s\n' "}"
} > "$manifest"

printf '%s %s %s %s %s %s\n' \
  "launched=$launched" "backend_ready=$backend_ready" "survived=$survived" \
  "log_clean=$log_clean" "screenshot=$shot_ok" "pid=$pid"

if [ "$backend_ready" = "1" ] && [ "$survived" = "1" ] && [ "$log_clean" = "1" ]; then
  printf '%s\n' 'PASS: packaged app launched, reached IPC-readiness, and stayed clean.'
  exit 0
fi

printf '%s\n' 'FAIL: see .verification/native-smoke/app.log and manifest.json' >&2
[ "$survived" = "0" ] && printf '%s\n' '  - the process exited during the probe window' >&2
[ "$backend_ready" = "0" ] && printf '%s\n' '  - backend_ready marker never fired (config/db/setup failure?)' >&2
[ "$log_clean" = "0" ] && printf '%s\n' '  - fatal pattern found in app log' >&2
exit 1
