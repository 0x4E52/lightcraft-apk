#!/usr/bin/env bash
# Emulator smoke test for the debug APK (.github/workflows/android.yml, docs/android-port.md APK-16).
#
#   packaging/android/smoke.sh <apk> <application-id> <output-dir>
#
# Installs the APK on the running emulator, starts the NativeActivity, waits, takes a screenshot and
# collects logcat. Fails when the process is gone, or the app's own log or crash lines show
# `panicked` or `fatal` (any case), or the pid changed (a crash followed by a relaunch). Writes <output-dir>/{shot.png,logcat.txt}. It does not judge the picture:
# whoever reads the run looks at shot.png (Device checklist in docs/android.md).
set -euo pipefail

apk=${1:?usage: smoke.sh <apk> <application-id> <output-dir>}
app_id=${2:?usage: smoke.sh <apk> <application-id> <output-dir>}
out=${3:?usage: smoke.sh <apk> <application-id> <output-dir>}
mkdir -p "$out"

# The pid seen after `am start`; collect() uses it, because a dead process has no pid to look up and
# `logcat --pid` still returns the lines of a process that is gone (a panic is only its forwarded stderr).
started_pid=

# Logcat is collected on every exit, so a failed start leaves its log behind. Only this app's lines
# count: other processes on the image (gms, system apps) crash now and then and must not turn the run red.
collect() {
  {
    echo "== logcat of pid ${started_pid:-none} =="
    if [ -n "$started_pid" ]; then
      adb logcat -d --pid="$started_pid" || true
    else
      # the app never appeared: keep the whole log, the cause may be in any process
      adb logcat -d -b main,system,crash || true
    fi
    echo "== AndroidRuntime and DEBUG lines naming $app_id =="
    adb logcat -d -s AndroidRuntime:E DEBUG:F DEBUG:E | grep -F "$app_id" || true
    echo "== crash buffer of this app =="
    if [ -n "$started_pid" ]; then
      adb logcat -d -b crash --pid="$started_pid" || true
    else
      adb logcat -d -b crash | grep -F "$app_id" || true
    fi
    # the panic hook and the log file write to internal storage; the debug APK allows run-as
    echo "== files/config/panic.log =="
    adb shell run-as "$app_id" cat files/config/panic.log 2>&1 || true
    echo "== files/logs =="
    # adb joins its arguments into one device shell line, so the loop is one string and \$f is expanded there
    adb shell "run-as $app_id sh -c 'for f in files/logs/*; do echo -- \$f; tail -n 200 \$f; done'" 2>&1 || true
  } >"$out/logcat.txt" 2>&1
}
trap collect EXIT

adb wait-for-device
until [ "$(adb shell getprop sys.boot_completed | tr -d '\r')" = 1 ]; do sleep 2; done

adb install -r "$apk"
adb logcat -c
adb shell am start -n "$app_id/android.app.NativeActivity"

# The process appears within seconds; the first frames on a software GPU take longer.
for _ in $(seq 1 30); do
  started_pid=$(adb shell pidof "$app_id" | tr -d '\r' || true)
  [ -n "$started_pid" ] && break
  sleep 2
done
[ -n "$started_pid" ] || { echo "::error::$app_id did not start"; exit 1; }
sleep 45

pid=$(adb shell pidof "$app_id" | tr -d '\r' || true)
[ -n "$pid" ] || { echo "::error::$app_id is no longer running (pid $started_pid)"; exit 1; }
# a new pid means the first process died and the system started the app again: its crash is in the
# log of the old pid, which collect() reads
if [ "$pid" != "$started_pid" ]; then
  echo "::error::$app_id restarted (pid $started_pid, now $pid): the first process died"
  exit 1
fi

adb exec-out screencap -p >"$out/shot.png"
collect
if grep -iE 'panicked|fatal' "$out/logcat.txt"; then
  echo "::error::the log shows a panic or a fatal error (see logcat.txt)"
  exit 1
fi
echo "smoke test passed: $app_id ran for 45 s, screenshot $(wc -c <"$out/shot.png") bytes"
