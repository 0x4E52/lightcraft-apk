#!/usr/bin/env bash
# Debug control server check on the emulator (.github/workflows/android.yml, docs/android-port.md APK-21).
#
#   packaging/android/control-check.sh <application-id> <output-dir>
#
# Runs after smoke.sh on the same emulator with the APK installed: sets `debug.lightcraft.control`,
# restarts the app, forwards the port, reads the token the app wrote to files/control-token and sends
# one `ui.inspect` over docs/showcase/run.py with it. A request without the token must be refused.
# Writes <output-dir>/control.txt (the token itself is never printed).
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
