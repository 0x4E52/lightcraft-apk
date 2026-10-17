# Android port (APK)

*Last updated: 2026-10-17.* Revision history at the end.

How LightCraft becomes an Android app (APK for sideloading, AAB for app stores) without giving up what the
desktop build stands for: pure Rust, never crash, everything is a command, clean-room, permissive licences.

## Where we stand

| Step | State |
|---|---|
| Licence fits an APK (MIT OR Apache-2.0; every dependency permissive or MPL-2.0) | ✅ checked by `cargo xtask licences` (in `ci`) |
| Licence notices inside the app (About ▸ Licences: NOTICE, both licences, Inter's OFL, all 600 linked crates) | ✅ APK-2 |
| ArtCraft marks out of the app (docs/brand/LICENSE-brand.txt, "Forks and modified versions") | ✅ APK-3: no Discord/website links or labels; About names the original in plain text only |
| `lightcraft-engine` and `lightcraft-ui-egui` compile for `aarch64-linux-android` | ✅ `cargo check` passes today, unchanged |
| Android host app, packaging, device testing | ⏳ phases 1–4 below |

## Ground rules for the port

These come from `CLAUDE.md` and apply to every task below.

- **No god scripts, no god modules.** One concern per file. Each build step is a small `cargo xtask android <step>`
  function in its own module (`xtask/src/android.rs`), not a shell script that checks, builds, signs and uploads in
  one go. Gradle files and the manifest are declarative configuration, with no build logic in them.
- **The host is thin.** Android-specific code lives in the new host crate and in one helper crate (below). Shared
  crates get Android behaviour by *injection* (`ui_egui::Services` callbacks, explicit setters), not by
  `#[cfg(target_os = "android")]` scattered through `engine` or `ui-egui`. A `cfg` is acceptable only at a leaf that
  already branches per platform (e.g. `engine::config`).
- **Pure Rust.** NativeActivity (`android-activity`'s `native-activity` feature, no Java and no C++), not
  GameActivity (which compiles C++ glue with `cc`). The APK carries `android:hasCode="false"`.
- **`unsafe` stays isolated.** The exported `android_main` symbol (`#[unsafe(no_mangle)]`) and the JNI calls
  (`JavaVM::from_raw` from `ndk-context`) both need `unsafe`. They go in a new crate, `crates/android-sys`, built like
  `crates/sysmem`: `// SAFETY:` on every block, a safe API, tests, and a line naming it in `CLAUDE.md`. Every other
  crate keeps `#![forbid(unsafe_code)]`.
- **Never crash.** A missing permission, a revoked `content://` URI, a GPU that won't start or low memory each produce
  an error the user can act on. The panic hook and `catch_unwind` around command dispatch work as they do on
  desktop, logging to logcat.
- **Everything is a command.** Android actions (import from the media library, share an export, open a link) are
  commands with ids, so the control channel and the tests reach them like any other.
- **Licences and brand.** Add `lightcraft-android` to `SHIPPED` in `xtask/src/licences.rs` so its dependencies
  (`android-activity`, `ndk`, `jni`…) are inventoried and policy-checked. The store listing, icon and name never use
  the ArtCraft name.

## Architecture

```
apps/lightcraft-android   (L6, cdylib liblightcraft_android.so, ~200 lines)
  ├─ builds the Session (library in app storage), fills ui_egui::Services for Android
  └─ eframe::run_native(NativeOptions { android_app, renderer: wgpu (Vulkan), .. })
crates/android-sys        (the isolated unsafe: entry point + JNI behind a safe API)
  ├─ entry.rs        android_main → calls the safe fn the host registers
  ├─ jni_env.rs      attach to the JavaVM from ndk-context
  ├─ intents.rs      ACTION_VIEW (open_url), ACTION_SEND (share export), getIntent() (opened with LightCraft)
  ├─ permissions.rs  check/request READ_MEDIA_IMAGES (polled on resume: NativeActivity gets no callback)
  └─ media_store.rs  query MediaStore images, open content:// URIs as file descriptors
packaging/android/        Gradle project: manifest (NativeActivity, hasCode=false), icons, signing config;
                          no Java/Kotlin sources
xtask/src/android.rs      `cargo xtask android check | build [--release] | run`
```

The web app (`apps/lightcraft-web`) is the precedent: a second host that fills the same `Services` with
platform services (browser file pickers instead of `rfd`). The desktop app's `rfd`, `zbus`, `muda` and the control
server's TCP listener don't come to Android.

## Phases

Each task is one commit (`APK-<n>: …`), green under `cargo xtask ci`, with a test that fails without it.

### Phase 0: foundation (CI-able, no device)

- **APK-4 `cargo xtask android check`**: `cargo check --target aarch64-linux-android` for the crates the port needs,
  like `cargo xtask wasm`. It joins `ci` when the target is installed and otherwise reports a skip
  (`lightcraft_testkit::skip!`). Acceptance: fails on a crate that stops compiling for Android.
- **APK-5 Settings folder injection**: `engine::config` gets an explicit `use_dir(path)` that the host calls with the
  app's internal storage path (`AndroidApp::internal_data_path`). It replaces setting `LIGHTCRAFT_CONFIG_DIR` at
  run time, because `std::env::set_var` is `unsafe` in edition 2024. Test: the override wins over the platform
  default.

### Phase 1: first pixel (emulator)

- **APK-6 `crates/android-sys` entry point + `apps/lightcraft-android`** with the procedural demo library
  (`lightcraft-scenes`). Acceptance: the UI renders on an arm64 emulator; rotating the device and backgrounding the
  app don't crash it (surface lost and recreated).
- **APK-7 `packaging/android` + `cargo xtask android build`**: `cargo ndk` builds the `.so`, then Gradle packages a
  debug APK (arm64-v8a, plus x86_64 for emulators). `docs/android.md` explains `adb install`. Acceptance: a clean
  checkout plus NDK and SDK produce an installable APK with one command.

### Phase 2: real photos

- **APK-8 Media library import** (`android.importMedia`): request permission, query MediaStore, import photos by
  `content://` URI (read through a file descriptor; the catalog stores the URI). Raws whose MIME type MediaStore
  doesn't know are reached through "Open with" (APK-10).
- **APK-9 Lifecycle**: save on `Pause`/`SaveState`, drop preview caches on `LowMemory`, and let long jobs (import,
  export) report progress through the activity stack.
- **APK-10 Intents**: `open_url` through `ACTION_VIEW`, share an export through `ACTION_SEND`, and open the file or
  URI the app was started with (`getIntent()`, no Java needed).

### Phase 3: touch UI

- **APK-11 Gestures**: pinch zoom already exists in the loupe (`panels/detail.rs`). Add pan, double tap = Fit/1:1,
  and long press = context menu; tests drive them through `ui.pointer`.
- **APK-12 Compact layout**: tablet landscape first, then phone. Side panels become sheets, the menu bar becomes an
  overflow menu, and the hit targets are at least 44 pt. It follows the existing layout rules; no second UI.
- **APK-13 Soft keyboard** for `TextField` (show and hide with focus, IME input). This is the riskiest egui piece on
  Android, so prototype it early (it can run in parallel with phase 1).

### Phase 4: quality and release

- **APK-14 Performance** on a mid-range phone: Vulkan through wgpu, with the CPU pipeline as fallback; budgets as on
  desktop (slider ≤ 16 ms draft); `LIGHTCRAFT_PROFILE` output to logcat.
- **APK-15 Release builds**: a signed AAB/APK. The keystore lives outside the repo (CI secret), and
  `release.yml` gets an Android job. The craft-fonts CJK faces are optional (APK size).
- **APK-16 Distribution**: the store listing (own name and icon), and F-Droid, which suits a pure-Rust, permissively
  licensed app without trackers.

## Decisions for the maintainer

1. **App name and application id.** The fork can keep "LightCraft" (the brand licence covers only the ArtCraft marks),
   but a distinct store name avoids confusion with official LightCraft builds. The application id must be ours
   (e.g. `io.github.x4e52.lightcraft`), not `ai.storyteller.lightcraft`.
2. **File access.** The recommended pure-Rust route is MediaStore plus "Open with" intents. The system file picker
   (Storage Access Framework) delivers its result through `onActivityResult`, which needs about 50 lines of
   Java/Kotlin. That is not C/C++, but it bends "pure Rust in the product".
3. **minSdk / targetSdk.** Recommended: minSdk 28 (Vulkan 1.1 widely available) and targetSdk at the current store
   requirement.
4. **Repository branding.** The app is clean (APK-3). The README header, `docs/brand/` and the desktop packaging
   metadata (`packaging/linux/*.metainfo.xml.in`, the Windows installer) still name or show ArtCraft. That is fine
   while this repository only mirrors upstream; before this fork publishes its own builds, remove or replace them.
5. **ABIs.** arm64-v8a only for release (practically every current device), plus x86_64 for emulators in debug.

## Risks

| Risk | Mitigation |
|---|---|
| egui soft keyboard and IME on Android are immature | prototype in phase 1 (APK-13); fall back to a native input dialog via JNI |
| A 24 MP raw decode needs hundreds of MB; Android kills background apps | budgeted previews, `LowMemory` trimming (APK-9), save on pause |
| GPU drivers vary (Vulkan bugs) | CPU pipeline fallback, adapter probe at start (as `window_gpu` does on desktop) |
| NativeActivity gets no activity results | the MediaStore route (decision 2), with permissions polled on resume |
| Upstream merges touch the same files | Android code stays in its own crates; the shared-crate changes are small injections |

## Revision history

- 2026-10-17: first version: licence check, in-app licences, brand removal done; phases 0–4 planned.
