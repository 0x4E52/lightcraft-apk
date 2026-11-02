#!/usr/bin/env bash
# Debug control server check on the emulator (.github/workflows/android.yml, docs/android-port.md APK-21).
#
#   packaging/android/control-check.sh <application-id> <output-dir>
#
# Runs after smoke.sh on the same emulator with the APK installed: sets `debug.lightcraft.control`,
# restarts the app, forwards the port, reads the token the app wrote to files/control-token and sends
# one `ui.inspect` over docs/showcase/run.py with it. A request without the token must be refused.
# Then runs the edit smoke session (docs/showcase/android-edit-smoke.jsonl, APK-53: crop, brush mask by
# touch, copy/paste with masks, two exports) over the same port: any request answering ok:false, a
# process that died or changed pid, a `panicked` line in the log or an entry in the panic log fails the check.
# Writes <output-dir>/control.txt (the token itself is never printed) and <output-dir>/edit-smoke.txt.
set -euo pipefail

app_id=${1:?usage: control-check.sh <application-id> <output-dir>}
out=${2:?usage: control-check.sh <application-id> <output-dir>}
port=18555
mkdir -p "$out"
here=$(cd "$(dirname "$0")" && pwd)
run_py="$here/../../docs/showcase/run.py"
script=$(mktemp)
echo '{"method":"ui.inspect","params":{}}' >"$script"
: >"$out/control.txt"
log() { echo "$*" | tee -a "$out/control.txt"; }

adb shell setprop debug.lightcraft.control "$port"
adb shell am force-stop "$app_id"
adb shell am start -n "$app_id/android.app.NativeActivity"
adb forward "tcp:$port" "tcp:$port"

# The token appears once the window is up (the server starts with the first frame's app creator).
token=
for _ in $(seq 1 60); do
  token=$(adb shell run-as "$app_id" cat files/control-token 2>/dev/null | tr -d '\r\n' || true)
  [ "${#token}" -eq 32 ] && break
  token=
  sleep 2
done
if [ -z "$token" ]; then
  log "::error::$app_id wrote no files/control-token within 120 s"
  exit 1
fi
log "token read: ${#token} characters"

# With the token: ui.inspect answers (a refused connection while the server binds is retried).
answer=
for _ in $(seq 1 30); do
  answer=$(python3 -I "$run_py" --port "$port" --token "$token" "$script" 2>&1 || true)
  case "$answer" in *'"ok": true'*) break ;; esac
  sleep 3
done
log "with token: $answer"
case "$answer" in *'"ok": true'*) ;; *) log "::error::ui.inspect with the token was not answered"; exit 1 ;; esac

# Without it: refused.
refused=$(python3 -I "$run_py" --port "$port" --token "" "$script" 2>&1 || true)
log "without token: $refused"
case "$refused" in *'bad token'*) ;; *) log "::error::a request without the token was not refused"; exit 1 ;; esac
log "control check passed: $app_id answered ui.inspect on port $port with the token and refused it without"

# The edit smoke session. The script is the desktop's headless one with the pieces only a snapshot has
# replaced: ui.settle becomes a pause, ui.screenshot is dropped, the export goes to the app's cache.
smoke_script=$(mktemp)
python3 -I - "$here/../../docs/showcase/android-edit-smoke.jsonl" "$app_id" >"$smoke_script" <<'PY'
import json, sys
src, app_id = sys.argv[1], sys.argv[2]
for line in open(src):
    line = line.strip()
    if not line or line.startswith("#"):
        continue
    method = json.loads(line)["method"]
    if method == "ui.settle":
        print(json.dumps({"sleep": 3}))
    elif method != "ui.screenshot":
        print(line.replace("target/android/smoke-export", f"/data/user/0/{app_id}/cache/smoke-export"))
PY
pid_before=$(adb shell pidof "$app_id" | tr -d '\r' || true)
adb logcat -c
if python3 -I "$run_py" --port "$port" --token "$token" --strict "$smoke_script" >"$out/edit-smoke.txt" 2>&1; then
  log "edit smoke: every request answered ok"
else
  log "::error::the edit smoke session had a request that failed (see edit-smoke.txt)"
  tail -n 5 "$out/edit-smoke.txt" | tee -a "$out/control.txt"
  exit 1
fi
pid_after=$(adb shell pidof "$app_id" | tr -d '\r' || true)
if [ -z "$pid_after" ] || [ "$pid_after" != "$pid_before" ]; then
  log "::error::the app died or restarted during the edit smoke session (pid ${pid_before:-none} -> ${pid_after:-none})"
  exit 1
fi
adb logcat -d >"$out/edit-smoke-logcat.txt" 2>&1 || true
# the default Rust hook prints `thread 'x' panicked at crates/...` with no app name on the line, so
# no second filter: any such line in the log fails the check
if grep -iE 'panicked' "$out/edit-smoke-logcat.txt" >/dev/null; then
  log "::error::the log shows a panic during the edit smoke session (see edit-smoke-logcat.txt)"
  exit 1
fi
# the engine hook appends every panic to the panic log (emptied at start), whatever logcat's format
panic_log=$(adb shell run-as "$app_id" cat files/config/panic.log 2>/dev/null | tr -d '\r' || true)
if [ -n "$(printf '%s' "$panic_log" | tr -d '[:space:]')" ]; then
  printf '%s\n' "$panic_log" >"$out/edit-smoke-panic.log"
  log "::error::the panic log has entries after the edit smoke session (see edit-smoke-panic.log)"
  exit 1
fi
exported=$(adb shell run-as "$app_id" ls "cache/smoke-export" 2>/dev/null | tr -d '\r' | grep -c '\.jpg$' || true)
if [ "${exported:-0}" -lt 2 ]; then
  log "::error::the edit smoke session exported ${exported:-0} JPEGs, not 2"
  exit 1
fi
log "edit smoke passed: crop, brush mask, copy/paste with masks and $exported exports on pid $pid_after"
