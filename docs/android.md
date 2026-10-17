# LightCraft on Android: build, install, debug

The port's plan and task cards are in [`android-port.md`](android-port.md); the touch UI spec is in
[`android-touch-ui.md`](android-touch-ui.md). This page is the day-to-day how-to. It grows with the port; what is
not here yet is not built yet.

## Build

Needs: the Rust targets (`rustup target add aarch64-linux-android x86_64-linux-android`), the Android NDK
(`ANDROID_NDK_HOME`, or under `$ANDROID_HOME/ndk`), the SDK with `platforms;android-36` and `build-tools;36.0.0` or
newer (`ANDROID_HOME`), and a JDK (for `keytool`).

```sh
cargo xtask android check                       # clippy -D warnings for every crate the APK links
cargo xtask android so                          # link liblightcraft_android.so (arm64-v8a); --abi x86_64 for the emulator
cargo xtask android apk                         # package target/android/lightcraft-debug.apk
```

`apk` packages every ABI that `so` has built (or only the `--abi` ones named). A stale `x86_64` library left from an
earlier emulator build is packaged next to a fresh `arm64-v8a` one unless `--abi arm64-v8a` is given. It runs, in order:
`cargo xtask android manifest` (debug id `io.github.n0x4e52.lightcraft.debug`), `aapt2 compile` and `aapt2 link
--debug-mode` over `packaging/android/res/`, the `.so` appended Stored and 16 KB-aligned, `zipalign -P 16`,
`apksigner sign` and `zipalign -c -P 16`. No Gradle runs.

The debug key is `~/.android/debug.keystore` (password `android`). It is created with `keytool` when missing and never
lives in the repository; `.gitignore` also keeps `*.jks`, `*.keystore`, `*.p12` and `keystore.properties` out.

## CI

`.github/workflows/android.yml` runs on pushes that touch Android paths (the crates, `packaging/android/`, `xtask/`,
the manifests, and the files the crates embed: `assets/`, `locales/`, `docs/whats-new.md`, the licence files) and on demand. The `apk` job runs `cargo xtask android check`, `so --abi arm64-v8a --abi x86_64` and
`apk` on a GitHub runner (SDK and NDK preinstalled; the library sizes go to the job summary), then `aapt2 dump
badging` must show the application id, minSdk 30, targetSdk 36, `application-label:'LightCraft'` and an
`application-icon-` line. The `emulator` job boots an x86_64 API 35 image and runs `packaging/android/smoke.sh`: it
installs the APK, starts `android.app.NativeActivity`, lets it run for 45 s, and fails when the process is gone or restarted, or the
log shows `panicked` or `fatal`. The APK (`lightcraft-debug-apk`), the screenshot and the log (`lightcraft-smoke`) are
uploaded as artifacts.

## Install and run

```sh
adb install -r target/android/lightcraft-debug.apk
adb shell am start -n io.github.n0x4e52.lightcraft.debug/android.app.NativeActivity
```

## Debugging

The debug build is `debuggable`, so `run-as` works:

```sh
adb shell run-as io.github.n0x4e52.lightcraft.debug cat files/config/panic.log     # the last panic, if any
adb shell run-as io.github.n0x4e52.lightcraft.debug ls files/logs                  # the log file
adb logcat --pid=$(adb shell pidof io.github.n0x4e52.lightcraft.debug)             # the app's log
adb logcat -d -s AndroidRuntime:E DEBUG:F                                          # Java and native crashes
```

## Lifecycle: what happens when LightCraft is left

No frame runs while the app is in the background, so the host saves and trims from the lifecycle callbacks
(`crates/android-host/src/lifecycle_core.rs`):

- **Focus lost** (Home, recents, another app, a system dialog): everything is saved, an open slider drag committed as
  one undo step. A kill after this (`adb shell am kill`, the low-memory killer) loses nothing.
- **Surface gone** (the app is in the background): the decoded photos, the rendered thumbnails in memory, the
  slideshow backdrop and the GPU's recycled buffers are let go (`Session::trim_caches`, `gpu::trim_pool(0)`), so the
  cached process holds little and is less likely to be killed. Only what changed is saved here, and only when no
  focus-loss save ran: the system waits for this callback (surfaceDestroyed), so it stays short.
- **Low memory** (`onLowMemory`): the same trim. NativeActivity does not forward `onTrimMemory`.
- **Activity destroyed** (Developer options ▸ "Don't keep activities", removal from recents, a missed configuration
  change): winit swallows `onDestroy`, which waits for the native thread, so the host asks the Activity
  (`isDestroyed()` / `isFinishing()`) each time the event loop is about to wait while the app is away. When it is
  ending, the host saves, leaves the loop, closes the library and the process exits (`Exit::Destroyed`); the next
  launch starts fresh.
- **Cached-apps freezer.** Android freezes a backgrounded app's process within seconds. A long import or export keeps
  its row in the activity stack but makes no progress while frozen, and continues when LightCraft is opened again;
  the row is not stalled, only paused. There is no foreground service (it would need Java, decision 2).

## Device checklist

What only a device (or a CI run an agent cannot start) can show. Whoever saw it ticks the line.

- [ ] Launcher icon (APK-14): under circle and squircle masks the ear tufts and the shoulder are not cut, and as a
  themed (monochrome) icon it reads as the LightCraft mark.
- [ ] First start with a 20k-photo library (APK-11): no ANR, the "Opening library…" frame is visible, then the grid.
- [ ] Phone limits (APK-11b): the library notice shows once on the first start, and again after a kill before OK was
  pressed; on a phone with less than 8 GB the AI mask command is disabled with the reason; Try Again in the library
  problem window reopens the library behind the "Opening library…" frame without an ANR.
- [ ] `cargo xtask android so` (APK-12): the first real link with the NDK passes the `elf` checks (16 KB `LOAD`
  alignment, entry symbols, system-only `NEEDED`); needs an NDK, so it is the APK-16 workflow's first run.
- [ ] `aapt2 dump badging` (APK-13) on the built APK shows the application id, version code and name, minSdk 30 and
  targetSdk 36 from the manifest; the APK-16 workflow runs it.
- [ ] `cargo xtask android apk` has produced an APK with the SDK build-tools (aapt2 accepts the manifest and
  resources, `zipalign -c -P 16` passes, `apksigner verify` passes): first real run is the APK-16 workflow.
- [ ] `android.yml` run green; the screenshot shows the LightCraft UI (APK-16): the run's `lightcraft-smoke` artifact
  holds `shot.png` and `logcat.txt`. The job fails on a dead process or a panic, but it does not judge the picture.
- [ ] The CI APK (APK-16) installs on an arm64 phone and opens on the demo library.
- [ ] Back (APK-17) with gesture navigation on Android 16 (targetSdk 36): an open menu closes first, then Back walks
  back to the grid, then moves LightCraft to the background; Home → reopen and Back to the root → reopen both return
  to where it was. File has no Quit, and Ctrl+Q on a hardware keyboard closes nothing.
- [ ] APK-16 smoke run still opens the UI with the lifecycle wrapper (APK-18): eframe now runs through
  `eframe::create_native` on the host's own event loop.
- [ ] Lifecycle (APK-18): edit, Home, `adb shell am kill <id>`, reopen: the edit and the view are back; Home during a
  slider drag: no ANR, and the drag is one undo step; rotate; Developer options ▸ "Don't keep activities" on → leave →
  reopen: no ANR, the app reopens (logcat: "the Activity is going away"); remove from recents → reopen; Home with a
  20k-photo library: no ANR.
