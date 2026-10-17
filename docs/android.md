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
