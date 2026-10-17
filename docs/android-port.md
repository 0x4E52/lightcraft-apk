# Android port (APK)

*Last updated: 2026-10-17 (after APK-13).* Revision history at the end.

How LightCraft becomes an Android app (APK for sideloading, AAB for app stores) without giving up what the
desktop build stands for: pure Rust, never crash, everything is a command, clean-room, permissive licences.
The tasks below are written to be executed one after another by agents (Sonnet or Opus) in a cloud container
without an Android device; what only a device (or a CI run the agent can't start) can show is listed separately for
the maintainer.

## Where we stand

| Step | State |
|---|---|
| Licence fits an APK (MIT OR Apache-2.0; every dependency permissive or MPL-2.0) | ✅ checked by `cargo xtask licences` (in `ci`) |
| Licence notices inside the app (About ▸ Licences: NOTICE, both licences, Inter's OFL, all 600 linked crates) | ✅ APK-2 (embedded craft-fonts faces still missing: APK-38) |
| ArtCraft marks out of the app (docs/brand/LICENSE-brand.txt, "Forks and modified versions") | ✅ APK-3: no Discord/website links or labels; About names the original in plain text only |
| `lightcraft-engine` (also `--features sam,denoise`) and `lightcraft-ui-egui` compile for `aarch64-linux-android` | ✅ `cargo check` passes, unchanged |
| An eframe host (`wgpu` + `android-native-activity` + `default_fonts`) compiles for `aarch64-linux-android` | ✅ `crates/android-host` (APK-10) |
| Container limits and agent CI access measured (disk need, network, targets, `gh`) | ✅ APK-4 ("Executing in this container"): no SDK/NDK download; agents read CI runs through `gh api`, starting one needs decision 0 |
| Full `cargo xtask ci` on GitHub for every push (`.github/workflows/ci.yml`), fast gate per task here | ✅ maintainer decision 2026-10-17 |
| `cargo xtask android check`: clippy `-D warnings` for every crate the APK links, for `aarch64-linux-android`, a step of `ci` (skipped, and reported as skipped, without the target) | ✅ APK-5 |
| A host without `HOME` can place settings (`engine::config::use_dir`), the export folder (`Services::default_export_dir`) and the Local sidebar roots (`Services::local_roots`) | ✅ APK-6 |
| Desktop host code the Android host also needs is shared (`lightcraft_ui_egui::host`: settings file, session opening, log file with the last error, flush, GPU markers; `lightcraft_engine::gpu::window`) | ✅ APK-7 |
| A host without environment variables sets the diagnostics switches before first use (`lightcraft_profile::set_enabled` for `LIGHTCRAFT_PROFILE`, `lightcraft_engine::gpu::backend::set_choice` for `LIGHTCRAFT_GPU_BACKEND`) | ✅ APK-8 |
| `safe_file` temp names keep the target's extension (`.lightcraft-<pid>-<n>-tmp.<ext>`, `.tmp` without one or with one over 16 bytes), as MediaProvider requires in `Pictures/` (device check: APK-27) | ✅ APK-9 |
| The three Android crates exist and lint clean for `aarch64-linux-android` (`lightcraft-android` entry symbol with a process exit after `run`, `lightcraft-android-host` start guard and `Exit`, `lightcraft-android-sys` `with_activity` and Java error mapping); `cargo xtask android check` derives its set from `lightcraft-android` | ✅ APK-10 (no `.so` linked or run yet: APK-12, APK-16) |
| The Android host opens the UI (eframe, wgpu) on the procedural demo library in internal storage (`files/{config,library,logs,inbox}`), the session opened on a worker behind an "Opening library…" frame, with a phone memory budget (MemTotal / 8, 256–768 MiB, also what Settings' "Automatic" means: `engine::memory::set_default_budget`), the device's RAM injected for export/merge fit checks (`engine::memory::set_total_ram`), the log file, the panic log and file/PNG writers | ✅ APK-11, compiled and linted for `aarch64-linux-android` (first run on a device or emulator: APK-16) |
| The first start tells once that uninstalling or Clear data deletes the library (`Services::library_notice`, flag `UiState::library_notice_shown`); Object and Describe masks are off with "Needs a device with 8 GB of memory" below `Services::min_ram_for_sam` (7 GiB MemTotal, which is what an 8 GB phone reads); Try Again in the library problem window reopens the library on the loader's worker behind the "Opening library…" frame (`Services::reopen_library`); export's and merge's memory messages say "this device" on a phone (`memory::set_handheld`) | ✅ APK-11b (the device checks stay in the card, unticked) |
| `cargo xtask android so [--abi arm64-v8a\|x86_64]…` links `liblightcraft_android.so` with the NDK's clang (no `cargo ndk`; the 16 KB page flag below NDK r28), copies it to `target/android/jniLibs/<abi>/` and checks `LOAD` alignment, the `android_main` / `ANativeActivity_onCreate` exports and system-only `NEEDED` libraries | ✅ APK-12, unit-tested on canned `llvm-readelf` output; no NDK here, so the first real link is the APK-16 workflow |
| `packaging/android/AndroidManifest.xml` (NativeActivity, `hasCode=false`, `allowBackup=false`, `extractNativeLibs=false`, the Back opt-out, full `configChanges`, minSdk 30 / targetSdk 36) filled by `cargo xtask android manifest [--release]` (`.debug` application id suffix for debug); `version_code` from the workspace version (alpha < beta < rc < release) | ✅ APK-13, unit-tested; `aapt2` has not read it yet (APK-15, APK-16 `aapt2 dump badging`) |
| Host UI, packaging, CI, touch UI, release | ⏳ APK-14 … APK-43 below |

## Ground rules for the port

These come from `CLAUDE.md` and apply to every task below.

- **No god scripts, no god modules.** One concern per file. `cargo xtask android <step>` lives in
  `xtask/src/android/` with one module per step (`mod.rs` only dispatches); `xtask/src/main.rs` (already ~2800 lines)
  gains only the `android` arm, the USAGE line and the `ci` step. The manifest and resources are declarative.
- **The host is thin, shared code is shared.** Android code lives in three new crates (below). Shared crates get
  Android behaviour by *injection* (`ui_egui::Services` fields, explicit setters, function parameters), not by
  `#[cfg(target_os = "android")]` in `engine` or `ui-egui`. Desktop host code the Android host also needs (prefs,
  session opening, log file, flush, GPU start markers, control server) moves into shared code first (APK-7, APK-21)
  instead of being copied.
- **Pure Rust.** NativeActivity (`android-activity`'s `native-activity` feature: no Java, no C++), not GameActivity
  (its `build.rs` compiles C++ with `cc`). The APK carries `android:hasCode="false"`. Java shims are a maintainer
  decision (decision 2), never added by an agent.
- **`unsafe` stays isolated.** android-activity calls a Rust symbol `android_main` (declared
  `extern "Rust" { fn android_main(app: AndroidApp); }`, glue.rs:663) on a fresh thread before any of our code runs,
  so nothing can "register" an entry function at run time, and defining the symbol needs `#[unsafe(no_mangle)]`,
  which `#![forbid(unsafe_code)]` rejects. So the symbol lives in a tiny entry crate, `apps/lightcraft-android`
  (≤ 30 lines). JNI's remaining unsafe (the `AndroidApp::activity_as_ptr()` → `JObject` cast) lives in
  `crates/android-sys` behind a safe `with_activity(|env, activity| …) -> Result` API. Both crates use `[lints]
  workspace = true` (the workspace already sets `unsafe_code = "deny"`, root Cargo.toml:112-113, and keeps the
  `type_complexity`/`too_many_arguments` allows that JNI closures need) and put `#[allow(unsafe_code)]` with a
  `// SAFETY:` comment only on the one item that needs it. sysmem's own `[lints.*]` tables are historical, not a
  pattern to copy. jni 0.22.4's `JavaVM::singleton()` (safe, initialised by
  android-activity before `android_main`) replaces `JavaVM::from_raw`, which is unsafe and `assert!`s. Content URIs
  are copied through JNI streams (`ContentResolver.openInputStream` → `InputStream.read(byte[])`), never by reopening
  `/proc/self/fd/<n>`: a reopen runs the kernel's full open permission check against LightCraft's UID and SELinux
  domain and fails with EACCES for most shared files. Both crates get a line in `CLAUDE.md` **and** `AGENTS.md` next
  to `crates/sysmem`. A `macro_rules!` that expands `no_mangle` into a `forbid(unsafe_code)` crate is not used: it
  compiles only because lints are not reported inside external macro expansions, which hides the unsafe instead of
  isolating it.
- **Never crash.** A missing permission, a revoked `content://` URI, a GPU that won't start, low memory, a system
  Activity destroy or a second `android_main` each produce an error the user can act on. Android code gets the same
  `#![deny(clippy::unwrap_used, …)]` crate attribute as every production crate, and `cargo xtask android check`
  lints it with `--target aarch64-linux-android` (on x86_64 the gated modules compile empty and are never linted).
  The panic hook (`engine::guard::install_hook`) writes to internal storage; stderr already reaches logcat
  (android-activity `init.rs`: `forward_stdio_to_logcat`). Two failure modes are native aborts the guard can't
  catch, so the code prevents them: the low-memory killer (the host sets a phone-sized memory budget, APK-11) and
  ART's local-reference-table overflow (every JNI loop over Java objects runs each iteration inside
  `env.with_local_frame(16, |env| …)`, jni 0.22 env.rs:2138; APK-10 states this in `jni_env.rs`'s API doc).
- **Never block android_main on the Java main thread.** While Java handshakes with native code, the Java main thread
  waits until android_main handles the command: onStart/onResume/onPause/onStop (`set_activity_state`,
  android-activity glue.rs:511-525), surface create/destroy (`set_window`, glue.rs:482-503), the input queue
  (`set_input`, glue.rs:461-480) and save state (glue.rs:528-537). So: (1) before the event loop's first poll,
  android_main does only quick work (APK-11); (2) inside winit callbacks it does no long I/O (`suspended` runs inside
  the surfaceDestroyed handshake, APK-18); (3) it never waits on a `run_on_java_main_thread` result (android-activity
  lib.rs:922-925 warns of the deadlock): results come back by `try_recv` polled in `logic()`, or the call runs
  directly on the calling thread where the API allows it (ClipboardManager, `startActivity`, `isShowing`);
  (4) `with_activity` is not re-entrant: a thread-local flag makes a nested call return `Err(Error::Reentrant)`
  instead of reaching jni 0.22's runtime panic for a non-top `Env` (`assert_top`, env.rs:199-236, 417-433), and
  inside `with_local_frame` only the closure's `env` is used. Long work (copies, queries, imports) runs on a worker
  thread that attaches with `JavaVM::singleton()?.attach_current_thread` and holds a `TaskGuard`.
- **Testable on x86_64.** Every Android module is split into a pure part that compiles and is unit-tested on every
  target (intent specs, MediaStore projections and row mapping, permission state machine, URI de-duplication, file
  name sanitising, inset maths, exit reasons, memory budget, lifecycle events, start-up loader) and a thin
  `#[cfg(target_os = "android")]` JNI part
  whose check is listed under "Maintainer checks on a device".
- **Everything is a command.** An Android action is a `Services` field, a command id in `crates/ui-egui/src/menus.rs`
  (dispatch + `enabled()` = the field is `Some`), its menu entry in `menubar.rs`, and a headless test with a fake
  service — the `file.backupLibrary` pattern (menus.rs:774, 2168; menubar.rs:278). No `android.` id namespace.
  `Services` is `#[derive(Default)]` (crates/ui-egui/src/lib.rs:233): every new field's `Default` must mean "desktop
  behaviour unchanged" (so `quit_disabled: bool`, not `can_quit: bool`).
- **Licences, layers, brand.** New crates are registered in `layers::TABLE` (else `cargo xtask layers` fails with
  "unregistered"), `lightcraft-android` joins `SHIPPED` in `xtask/src/licences.rs`, and the same commit runs
  `cargo xtask licences --write` and commits `assets/licences/third-party.json`. Every image (launcher icons, store
  screenshots, feature graphic) gets an `assets/ATTRIBUTION.md` row. The store listing, icon and name never use the
  ArtCraft name or anything from `docs/brand/`. Embedded craft-fonts faces ship with their OFL text (APK-38).

## Architecture

```
apps/lightcraft-android     package lightcraft-android, cdylib, [lib] name = "lightcraft_android"
  └─ lib.rs                 #[unsafe(no_mangle)] fn android_main(app) → lightcraft_android_host::run(app); logs Err
crates/android-host         package lightcraft-android-host, rlib, #![forbid(unsafe_code)]
  ├─ run.rs                 start guard, quick start work, event loop, Exit { Destroyed, Closed }      (APK-10/11)
  ├─ paths.rs               internal_data_path (= …/files) → config/library/logs/inbox dirs            (APK-11)
  ├─ memory.rs              pure budget_for(/proc/meminfo), MemTotal, sam_allowed                      (APK-11/11b)
  ├─ loader.rs              opens the Session on a worker; hands it over to the UI thread              (APK-11)
  ├─ state.rs               HostState { app, prefs }: flush_leaving/flush_dirty/trim (impl Host)      (APK-11/18)
  ├─ app.rs                 eframe::App wrapper: loading frame, logic, raw_input_hook, ui, on_exit    (APK-11 …)
  ├─ services.rs            fills ui_egui::Services (writer, png, android-sys services)               (APK-11 …)
  ├─ lifecycle_core.rs      pure: HostEvent, trait Host, on_event() → Continue | Exit                  (APK-18)
  ├─ lifecycle.rs           android only: ApplicationHandler forwarding to EframeWinitApplication      (APK-18)
  ├─ renderer.rs            renderer order + start markers, exit_reason(), failure dialog wait         (APK-19)
  ├─ incoming.rs            worker that copies received content:// files and imports them              (APK-25)
  └─ insets.rs              px → points, RawInput::safe_area_insets                                    (APK-31)
crates/android-sys          package lightcraft-android-sys, rlib, the JNI unsafe behind a safe API
  ├─ jni_env.rs             JavaVM::singleton(), with_activity() (not re-entrant), Java error mapping
  ├─ activity.rs            activity_state() (isFinishing/isDestroyed), move_task_to_back             (APK-17/18)
  ├─ dialog.rs              AlertDialog (message; text-input variant for APK-35)                       (APK-19)
  ├─ props.rs               debug.lightcraft.* system properties                                       (APK-20)
  ├─ permissions.rs         pure state machine + JNI checkSelfPermission / requestPermissions          (APK-22)
  ├─ media_store.rs         pure projection/row→path mapping + JNI query                               (APK-23)
  ├─ content.rs             copy a content:// URI into a file via InputStream                          (APK-25)
  ├─ intents.rs             pure IntentSpec parse/build + JNI getIntent / startActivity                (APK-25/26)
  ├─ storage.rs             getExternalStoragePublicDirectory                                          (APK-27)
  ├─ clipboard.rs           ClipboardManager get/set                                                    (APK-28)
  └─ insets.rs              getRootWindowInsets (system bars, cutout, IME)                              (APK-31)
crates/profile              package lightcraft-profile, L0: one LIGHTCRAFT_PROFILE switch              (APK-8)
packaging/android/          AndroidManifest.xml template, res/ (icons), icons.sh, no Java/Kotlin
xtask/src/android/          mod.rs (dispatch), check.rs, sdk.rs, so.rs, elf.rs, manifest.rs, zip_lib.rs,
                            signing.rs, apk.rs, aab.rs, listing.rs
.github/workflows/android.yml          builds the APK, boots an x86_64 emulator, smoke-tests (APK-16)
.github/workflows/android-release.yml  signed APK + AAB (APK-40)
```

- Layers (`xtask/src/layers.rs` TABLE): `("android-sys", Class::Standalone)` (L0, no workspace dependencies; its pure
  modules compile on wasm32 and it joins `wasm_set()`), `("android-host", Class::Exempt)`, `("android",
  Class::Exempt)`, `("profile", Class::Layer(0))`.
- Dependencies: workspace `android-activity = { version = "=0.6.1", features = ["native-activity"] }` and
  `jni = "=0.22.4"` (the versions winit 0.30 already pulls in, so `AndroidApp` and the JVM singleton are the same
  types and instance as eframe's). **Every Android-only dependency sits under
  `[target.'cfg(target_os = "android")'.dependencies]`**: android-host's `eframe = { workspace = true, features =
  ["wgpu", "android-native-activity", "default_fonts"] }`, `winit = { version = "0.30", default-features = false,
  features = ["android-native-activity"] }` (eframe does not re-export winit) and `android-activity`; android-sys's
  `jni` and `android-activity`. android-activity has a `compile_error!` on every other OS, and winit without
  `x11`/`wayland` hits `compile_error!("Please select a feature to build for unix: x11, wayland")`
  (winit-0.30.13 `platform_impl/linux/mod.rs:4`) when android-host is tested alone on Linux. Ordinary dependencies
  are only the shared crates (`lightcraft-ui-egui`, `lightcraft-engine`, `lightcraft-codecs`, `lightcraft-raster`,
  `log`). Never `accesskit` (eframe lib.rs:147 `compile_error!` with native-activity; so TalkBack is unsupported) and
  no `glow`: eframe's `wgpu` feature turns on `egui-wgpu/default` → `wgpu/default`, which includes the `gles`
  backend, so the window renderer is Vulkan with a GLES fallback. The engine ships with `sam` (decision 10);
  `denoise` follows the desktop's optional feature.
- `NativeOptions { android_app: Some(app.clone()), renderer: eframe::Renderer::Wgpu, .. }`. The host keeps its own
  `AndroidApp` clone for `internal_data_path()` and JNI.
- The web app (`apps/lightcraft-web`) is the precedent for a second host filling `Services` and saving on a timer
  (`SAVE_EVERY_MS`, web.rs:35, 541). The desktop's `rfd`, `zbus`, `muda`, Windows renderer probe, relaunch chain and
  process respawn don't come to Android (one process, one event loop).

### Platform facts this plan relies on (verified in the crate sources, 2026-10-17)

| Fact | Source | Consequence |
|---|---|---|
| One winit `EventLoop` per process; recreation returns `RecreationAttempt`, reset only on web | winit-0.30.13 `event_loop.rs:118-133` | a second `android_main` can't start the UI again (start guard, APK-10) |
| `onDestroy` blocks the Java main thread until `android_main` returns (`notify_destroyed`); winit swallows `MainEvent::Destroy` with a `warn!` | android-activity glue.rs:400-410; winit `android/mod.rs:289-293` | the next `onCreate` (and a second `android_main`) runs only after the first returns: the host must notice the destroy itself and leave the loop (APK-18), or it ANRs |
| When `android_main` returns, android-activity calls `ANativeActivity_finish` | glue.rs:985-993 | a dialog shown just before returning dies with the Activity: wait for its dismissal first (APK-19); Android usually keeps the process cached, so the entry crate exits the process whenever `run()` returns (APK-10) |
| Start/Resume/SaveState/Pause/Stop, ContentRectChanged and InsetsChanged are logged as TODO | winit `android/mod.rs:222-296` | no pause/resume or insets hooks; use focus, `Suspended`, `memory_warning` and JNI |
| `TerminateWindow` → `Suspended`; no redraws while suspended | winit `android/mod.rs:217-218, 331, 580` | no eframe frame (no `logic()`) runs in the background: the wrapper flushes itself (APK-18) |
| Java lifecycle callbacks wait for android_main: `set_activity_state`, `set_window`, `set_input` block until android_main handles the command; `TermWindow` is acknowledged by `post_exec_cmd` only after winit's whole callback (incl. `suspended` and `about_to_wait`) returns. Focus changes don't wait (`notify_focus_changed` only writes a command) | glue.rs:461-525, 645-651, 781-790; winit `native_activity` single_iteration | quick start before the first poll (APK-11); full flush on `Focused(false)`, only dirty state in `suspended` (APK-18) |
| `LowMemory` → `ApplicationHandler::memory_warning`; eframe's own wrapper ignores it. NativeActivity doesn't forward `onTrimMemory` (android-activity has only `MainEvent::LowMemory`) | winit mod.rs:265; eframe `native/run.rs:541`; glue.rs:34, 126 | the host wraps `eframe::create_native` (eframe lib.rs:376) to receive it, and trims when backgrounded (APK-18) |
| `AndroidApp::internal_data_path()` is NativeActivity's `internalDataPath` = `getFilesDir()`; `AndroidApp::sdk_version()` exists | android-activity lib.rs:1261, 1272 | `run-as <id>` paths start with `files/` (APK-11, APK-21); no JNI `Build.VERSION` read (APK-22) |
| NativeActivity doesn't override `onNewIntent` and `Activity.onNewIntent` doesn't call `setIntent`, so `getIntent()` never changes while the Activity lives | Android framework | `getIntent()` read once per start; a second share while running is not received (APK-25) |
| Scoped storage: the photo permission opens only *media* files of other apps; non-media files other apps created in `Download/` or `Documents/` are hidden from `readdir` and can't be opened by path | MediaProvider FUSE | presets, GPX, keyword lists and models arrive by share into an Inbox (APK-24, APK-25) |
| `READ_MEDIA_VISUAL_USER_SELECTED` exists from API 34 | Android 14 | partial photo access only on 34+ (APK-22) |
| `create_native` returns `EframeWinitApplication`; renderer errors are only logged and kept in a private `return_result` | eframe `native/run.rs:83-164` | detect failure as "the app creator never ran" plus the last logged error (APK-19) |
| `App::save` runs only with eframe's `persistence` feature (not enabled) | eframe `native/epi_integration.rs:403-408` | flushing is the host's job, not `App::save` |
| `set_ime_allowed` shows/hides the soft keyboard; `set_ime_cursor_area` / `set_ime_purpose` are no-ops; text arrives as key events | winit `android/mod.rs:916-926` | no IME composition (CJK), autocorrect or swipe (APK-35) |
| egui-winit's clipboard is in-process on Android | egui-winit `clipboard.rs` | system clipboard via JNI (APK-28) |
| egui fills `safe_area_insets` only on iOS; android-activity has no insets API in native-activity mode; with targetSdk ≥ 35 the content view is edge-to-edge, so `content_rect()` equals the window | egui-winit `safe_area.rs`; android-activity `src/lib.rs` | the host reads `WindowInsets` through JNI (APK-31) |
| KEYCODE_BACK reaches egui as `Key::BrowserBack` (winit `NamedKey::BrowserBack`), marked handled; from targetSdk 36 only with `enableOnBackInvokedCallback="false"` | winit `keycodes.rs:236`, mod.rs:374 | the manifest always opts out (APK-13); the host rewrites Back to Escape, so popups close first (APK-17) |
| `memory::total_ram()` reads `/proc/meminfo` only under `cfg(target_os = "linux")`, so it is `None` on Android: the budget falls back to the 1.5 GiB cap and export's `too_big_for_memory` (export.rs:1654) and merge's `fit_in_memory` (merge.rs:155, 200) are skipped | crates/engine/src/memory.rs:29-57 | the host calls `memory::set_budget` (memory.rs:77) and an injected `memory::set_total_ram` at start (APK-11) |
| Scoped storage: in `Pictures/` an app may only create files whose extension maps to an image MIME type | MediaProvider FUSE | `safe_file`'s `.tmp` temp name fails there; temp names keep the extension (APK-9); sidecars and non-image exports go elsewhere (APK-27) |
| `lightcraft-cli snapshot --size` is in points, `--scale` is pixels per point | apps/lightcraft-cli/src/main.rs:81-82, 787; `headless::viewport_pixels` | phone shots: `--size 412x915 --scale 2.625` |

## Executing in this container

Facts (measured by APK-4, 2026-10-17; each line says how to re-check it):
- **Machine.** linux x86_64, 4 CPUs, 16 GB RAM (`nproc`, `free -g`); runs as root (`id -u` = 0: chmod-based read-only
  tests are vacuous here); `/bin/sh` is dash; JDK 21.0.12 (`java -version`) and `/opt/gradle-8.14.3` installed;
  `xvfb-run` installed; `resvg` and `lightcraft-cli` not on PATH (use `cargo install resvg --locked` and `cargo run -p
  lightcraft-cli --`); no `/dev/kvm`, no GPU (no `/dev/dri`, no Vulkan ICD).
- **Disk.** `df -h /` showed 11 GB free at the start (the volume is 252 GB, but free space is what counts and it
  moved by several GB while builds ran; after trap (b) below wiped `target/`, 28 GB were free and the cold `ci`
  started from there). `target/` was 19 GB: `target/debug/deps` 15 GB, `target/debug/examples`
  3.0 GB (from `clippy --workspace --all-targets`), `target/debug/build` 0.58 GB, `target/debug/incremental`
  0.1 GB, `target/wasm32-unknown-unknown` 0.25 GB. **The Android need is 0.6 GB**: `target/aarch64-linux-android`
  is 590 MB after `cargo clippy --target aarch64-linux-android -p lightcraft-engine -p lightcraft-ui-egui
  --features lightcraft-engine/sam,lightcraft-engine/denoise` (143 s cold with
  `CARGO_PROFILE_DEV_DEBUG=line-tables-only`, 160 s without), and the figure is the same with and without the variable
  because clippy writes metadata only; a card that links a `.so` (APK-12 on) must re-measure its own need. Two traps
  found on the way: (a) exporting `CARGO_PROFILE_DEV_DEBUG=line-tables-only` against a `target/` built without it rebuilds every crate and keeps
  both sets (`deps` grew from 15 GB to 21.5 GB in one `ci` run until the disk filled: linker "Bus error",
  rustc "No space left on device"), so start the first `ci` of a session with the variable already exported, or with
  `target/debug` removed, or with at least 15 GB free; (b) `cargo clean --target aarch64-linux-android` (cargo 1.97.0)
  also removed `target/ci`, `target/wasm32-unknown-unknown` and most of `target/debug` here, so use `rm -rf
  target/aarch64-linux-android` to drop the Android output only.
- **Network** (`curl -sS -o /dev/null -w '%{http_code}' -I <url>`): `dl.google.com/android/repository/
  repository2-3.xml` fails (the proxy refuses the CONNECT with 403: no SDK, NDK, build-tools or emulator images);
  `maven.google.com` 301; `services.gradle.org` 200; `repo.maven.apache.org` 200.
- **Rust targets** (`rustup target list --installed`): `aarch64-linux-android`, `wasm32-unknown-unknown`,
  `x86_64-linux-android` (added by APK-4 for the CI emulator ABI, decision 5) and `x86_64-unknown-linux-gnu`.
- **GitHub** (re-checked after the maintainer enabled Actions, 2026-10-17). `gh api repos/0x4E52/lightcraft-apk/
  actions/workflows` lists the workflows as active and `gh api …/actions/runs` and `…/actions/runs/<id>/jobs` answer,
  so **agents can read runs and their jobs** through `gh api` (the proxy authenticates the call although `gh auth
  status` reports the session's `GH_TOKEN` invalid). The proxy refuses some paths (`/actions/permissions`) and the
  GitHub MCP server. Starting a run (`gh workflow run`) was refused by the session's permission check as an external
  write, not by the proxy: it needs decision 0 (a token in a new session) or the maintainer's go-ahead. Pushes go
  through the session's git access, and every push starts `.github/workflows/ci.yml`.
- **`cargo xtask ci`** (fmt, clippy, heif, test, GPU probe, parity, layers, assets, licences, wasm; since APK-5 also android): from a cold
  `target/` with `CARGO_PROFILE_DEV_DEBUG=line-tables-only` it took 3037 s (about 51 min) and ended green. `df` fell
  from 28 GB free to 1.4 GB during the run (`target/` peaked near 21 GB and was 17 GB after deleting
  `target/debug/examples` and `target/debug/incremental`). A warm run was not timed: per-task runs here are replaced
  by the fast gate (Rules), so the full run's warm time matters only on GitHub. It ends with a skip report (corpus, GPU, craft-fonts and
  model tests skip here).
- So **here** an agent can run `cargo check` / `cargo clippy --target aarch64-linux-android` (no linker needed;
  engine + ui-egui about 2.5 min cold), every x86_64 test and read CI runs, but cannot link the `.so`, build an APK
  or boot an emulator. Linking, packaging and the emulator smoke test run in `.github/workflows/android.yml` (APK-16)
  on GitHub's ubuntu runners (SDK/NDK preinstalled, KVM available); a push starts them.

Rules for every task:
- Run tasks sequentially in the main checkout, default `target/` dir (no parallel worktrees, no second target tree).
  Agent sessions export `CARGO_PROFILE_DEV_DEBUG=line-tables-only` (session environment only, never in the repo).
- Disk, graded, as a precondition: before a build step, `df -h /` must show the step's need plus 2 GB (Android
  `clippy`/`check`: 0.6 GB; a full local `cargo xtask ci`: about 27 GB from cold). Clean only while no cargo runs
  (`pgrep -x cargo` prints nothing; deleting under a running cargo fails with "failed to move dependency graph"); if
  space runs low mid-run, stop the command first. (1) Delete `target/debug/examples` and `target/debug/incremental`;
  (2) still short: `rm -rf target/aarch64-linux-android` or `cargo clean -p <crates the card touches>` (not `cargo
  clean --target`, see Facts); never delete files from `target/debug/deps` by age (cargo then fails to link); (3) a
  full `cargo clean` only as the last step (a cold rebuild exceeds the shell's foreground limit: run it in the
  background).
- Before the first `cargo xtask ci` of a session, `rustup target list --installed` must list both
  `wasm32-unknown-unknown` and `aarch64-linux-android`; if not, `rustup target add` them.
- `cargo xtask ci` takes longer than the shell tool's 10-minute foreground limit: start it with `run_in_background`
  (output to a log file in the scratchpad) and wait with Monitor or by polling the log; never a foreground call.
- `cargo test` takes one positional filter; several filters go after `--` (`cargo test -p lightcraft-ui-egui -- back
  quit`).
- `Needs:` says what a task requires: **none** (x86_64 only), **rust-target** (`aarch64-linux-android` installed),
  **ndk**, **sdk**, **ci** (the GitHub workflow), **device**. A task whose `Needs` is ndk/sdk/ci verifies here with
  the parts that run here (unit tests, YAML parse with `python3 -c 'import yaml,sys; yaml.safe_load(open(sys.argv[1]))'
  <file>`, `actionlint` if available), and its NDK/SDK/CI steps are proven by a workflow run, never assumed.
- **CI runs.** Every push starts `ci.yml` (the full `cargo xtask ci`) and, from APK-16, `android.yml`. After pushing,
  a task reads the run for its commit with `gh api repos/0x4E52/lightcraft-apk/actions/runs?head_sha=<sha>` and its
  jobs, and records the run URL in its result. It doesn't wait for the run to finish before the next task starts;
  whoever sees a red run first fixes it in the next commit (fix forward), before any new card. Starting a run by hand
  (`gh workflow run`) needs decision 0; until then the push trigger is the way.
- "Maintainer checks on a device" (and CI lines the agent can't run) are never ticked by an agent. An agent adds them
  to `docs/android.md` → "Device checklist" (created by APK-15) unticked.
- **Gate per task (maintainer decision 2026-10-17, replaces a local `cargo xtask ci` per task).** Before its one
  commit `APK-<n>: …`, a task runs here: `cargo fmt --all -- --check`; `cargo clippy --all-targets -p <each touched
  crate> -- -D warnings` (and `--target aarch64-linux-android` for crates the APK links); `cargo test -p <each touched
  crate>`; and the xtask checks its change touches (`cargo xtask parity` for docs/tracker, `layers` for new crates or
  dependencies, `assets` for assets, `licences` for `Cargo.lock`, `wasm` for wasm-safe crates, `android check` from
  APK-5). Then it pushes; the full `cargo xtask ci` runs on GitHub (`ci.yml`). A red run is fixed forward first. No
  task waits for a CI run or a device check to be ticked: "Depends on" lists only code dependencies.

## Task card format

Each card has: **Goal**; **Files** (exact paths, new/changed); **Steps**; **Tests** (the test that fails without the
change); **Verify** (commands runnable in this container); **Depends on** (the cards whose code it uses; each task
depends only on earlier ones); **Needs**; **Model** (sonnet or opus, with the reason); **Done when**; and, where it
applies, **Maintainer checks on a device**. Decisions that block a task are named in it; until decided, the agent
uses the default in "Decisions for the maintainer". Where a card's **Verify** ends with "`cargo xtask ci` green",
that means the gate per task here plus the GitHub `ci.yml` run (Rules). IDs with a letter suffix (APK-11b, APK-30b, APK-32a–c) run
right after their number, in letter order.

## Phase 0: foundation (x86_64, no NDK)

#### APK-4 Environment probe
- **Goal:** record what this container and the GitHub repo allow, so later cards don't guess.
- **Files:** changed `docs/android-port.md` ("Executing in this container").
- **Steps:** 1. Record `df -h /`, `du -sh target target/debug/{deps,examples,incremental}`, `ls /dev/kvm`,
  `java -version`. 2. `curl -sS -o /dev/null -w '%{http_code}' -I` for
  `dl.google.com/android/repository/repository2-3.xml`, `maven.google.com`, `services.gradle.org`,
  `repo.maven.apache.org`. 3. `rustup target add wasm32-unknown-unknown x86_64-linux-android`, then record `rustup
  target list --installed`. 4. With `gh`: `gh auth status`, `gh workflow list`, `gh run list -L 3`; if auth works,
  whether `gh workflow run packaging-lint.yml` can be started and its log read with `gh run view --log` (cancel the
  run afterwards). 5. Run `cargo xtask ci` warm (background + Monitor) and record its duration. 6. Run
  `cargo clippy --target aarch64-linux-android -p lightcraft-engine -p lightcraft-ui-egui --features
  lightcraft-engine/sam,lightcraft-engine/denoise` (APK-5's command is not there yet) once with and once without
  `CARGO_PROFILE_DEV_DEBUG=line-tables-only` (clean that target dir in between) and record `du -sh
  target/aarch64-linux-android` for both: the larger figure is "the Android need" of the disk rule. 7. Replace the
  facts line and the assumed 6 GB in the disk rule with the measured figures and today's date.
- **Tests:** none (documentation only).
- **Verify:** `cargo xtask parity` (docs still consistent); `cargo xtask ci` green.
- **Depends on:** —  **Needs:** rust-target  **Model:** sonnet — command runs and a doc edit.
- **Done when:** the section lists each fact with its date (disk need included) and states whether agents can trigger
  and read CI runs (decision 0).

#### APK-5 `cargo xtask android check`
- **Goal:** lint every crate the APK links for `aarch64-linux-android`, inside `ci`.
- **Files:** new `xtask/src/android/mod.rs` (dispatch `check`), `xtask/src/android/check.rs`; changed
  `xtask/src/main.rs` (`Some("android")` arm, USAGE, `steps.push(("android", …))` after `wasm` in `cmd_ci`, step type
  `Fn() -> Result<StepOutcome, String>` with `enum StepOutcome { Ran, Skipped(String) }` and the status computed from
  it at main.rs:381-383), `xtask/src/ci_report.rs` (status `skipped` with its reason in `summary_text` /
  `summary_md`).
- **Steps:** 1. `fn android_set(metadata: &Value) -> Vec<String>`, derived from `cargo metadata` like `wasm_set`
  (main.rs:245): every workspace package reachable over normal (non-dev, non-build) dependencies from the root
  `lightcraft-android`; while that package doesn't exist yet (until APK-10), the roots are `lightcraft-engine` and
  `lightcraft-ui-egui`. 2. One invocation: `cargo clippy --target aarch64-linux-android -p a -p b … --features
  lightcraft-engine/sam,lightcraft-engine/denoise -- -D warnings`. Never `--workspace`: it pulls `apps/lightcraft`,
  whose eframe `accesskit` feature is a compile error with `android-native-activity`. 3. Parse `rustup target list
  --installed`; without the target the step returns `Skipped("aarch64-linux-android not installed")`, not a failure
  and not a testkit `skip!` (that macro is test-only; xtask doesn't depend on testkit). 4. Every existing step
  returns `Ok(StepOutcome::Ran)`.
- **Tests:** `android_set_follows_normal_deps` (canned metadata JSON with a dev-dependency that must be left out),
  `android_set_falls_back_to_engine_and_ui`, `missing_target_is_skipped` (injected rustup output) in check.rs;
  `skipped_step_is_reported` (ci_report.rs).
- **Verify:** `cargo test -p xtask -- android skipped && cargo xtask android check`; by hand (not committed): add
  `let _ = None::<u8>.unwrap();` inside a `#[cfg(target_os = "android")]` fn in `crates/gpu` and see the step fail
  (proves crates beyond engine/ui-egui are linted).
- **Depends on:** APK-4  **Needs:** rust-target  **Model:** sonnet — mirrors `cmd_wasm` with a typed step outcome.
- **Done when:** `cargo xtask ci` runs the android step and reports it ok (or skipped without the target).

#### APK-6 Host paths: settings and export folder
- **Goal:** the host can place settings and exports without `HOME` and without `std::env::set_var` (unsafe in
  edition 2024): on Android `HOME`/`XDG_*` don't name usable folders.
- **Files:** changed `crates/engine/src/config.rs`, `crates/ui-egui/src/lib.rs` (`Services::default_export_dir:
  Option<String>`, `Services::local_roots: Vec<(String, PathBuf)>`), `crates/ui-egui/src/control.rs` and the callers
  of `default_export_dir()`, `crates/ui-egui/src/panels/left.rs` (`local_now`, :806).
- **Steps:** 1. `pub fn use_dir(path: PathBuf) -> Result<(), PathBuf>` = `use_dir_in(&INJECTED, path)`, where the
  pure `fn use_dir_in(cell: &OnceLock<PathBuf>, path) -> Result<(), PathBuf>` returns `Ok` for the same path again and
  `Err(existing)` for a different one; never panics. 2. Give the pure resolver a parameter:
  `config_dir_with(platform, env, injected: Option<&Path>, portable)`, order **injected > `LIGHTCRAFT_CONFIG_DIR` >
  portable > platform**. 3. Exports: callers of `control::default_export_dir()` prefer
  `services.default_export_dir` when set (APK-27 replaces this field by `export_dir_for`). 4. The library dir needs no
  global: the host passes it explicitly to the shared session opener (APK-7); `engine::library::default_dir`
  (library.rs:67, HOME-based) has only the desktop caller main.rs:911. 5. Audit production uses of
  `std::env::temp_dir()` (`/data/local/tmp`, not writable by apps): list them in the commit message; route those
  reachable on Android through a `config`-relative cache dir; leave test uses alone. 6. Audit `grep -rn '"HOME"'
  crates --include=*.rs` (non-test; known: config.rs, control.rs:475, library.rs:67, panels/left.rs:806; headless.rs
  uses are test helpers). The Library sidebar's Local section (available everywhere but wasm32, sidebar.rs:64-66)
  builds Pictures/Desktop/Downloads/Home from `HOME`: `local_now` uses `services.local_roots` instead when it is
  non-empty (empty = today's list). APK-27 fills it on Android.
- **Tests:** `config_tests::use_dir_in_same_path_ok_other_path_err` (on a local `OnceLock`, never the global, so no
  later engine test sees an injected dir), `config_tests::injected_dir_wins_over_env_portable_and_platform`;
  ui-egui `export_without_folder_uses_service_default_dir`, `local_section_uses_injected_roots` (headless).
- **Verify:** `cargo test -p lightcraft-engine -- config_tests && cargo test -p lightcraft-ui-egui -- export
  local_section`
- **Depends on:** APK-5  **Needs:** none  **Model:** sonnet — small, well-specified injection.
- **Done when:** desktop behaviour is unchanged (existing `config_path_tests` pass) and the override is tested.

#### APK-7 Shared host code
- **Goal:** move the platform-neutral parts of `apps/lightcraft` that the Android host also needs into shared code,
  so the Android host doesn't copy them.
- **Files:** new `crates/ui-egui/src/host/{mod.rs,prefs.rs,session_open.rs,log_file.rs,flush.rs,gpu_markers.rs}`,
  `crates/gpu/src/window.rs`; changed `apps/lightcraft/src/{main.rs,logging.rs,window_gpu.rs}`,
  `crates/ui-egui/src/lib.rs` (`#[cfg(not(target_arch = "wasm32"))] pub mod host;`, precedent lib.rs:1199),
  `crates/gpu/src/lib.rs`, `docs/showcase/run.py`.
- **Steps:** 1. `host/prefs.rs`: `prefs_path`/`load_prefs`/`load_prefs_at`/`decode_saved_ui`/`write_atomic`/
  `PrefsWriter` (main.rs 241–440), config dir as a parameter; `std::time::Instant` → `web_time::Instant` (already a
  ui-egui dependency) so the module stays wasm-safe if the cfg is ever lifted. 2. `host/session_open.rs`:
  `open_session` / `open_library_session` (main.rs 722–790), library dir explicit. 3. `host/log_file.rs`: the file
  logger and `record_panics` core of `logging.rs`, plus a shared "last error-level record" slot
  (`last_error() -> Option<String>`) for APK-19; `pub fn install_with(spec: &str) -> Option<&'static AppLogger>`, and
  `install()` = `install_with(&filter_spec(env LIGHTCRAFT_LOG, env RUST_LOG))` (logging.rs:313-323), so a host
  without environment variables passes its own spec (APK-20); desktop-only parts stay. 4. `host/flush.rs`, two
  functions. `flush_periodic(app, prefs)` = `session.persist_if_dirty()` (library.rs:662) + `session.save_view()`
  (library.rs:730) + `prefs.save()`; it never touches an open interaction: a slider drag is begin → `develop.set` per
  frame → end (panels/edit.rs:19-37, "a drag is one undo step"), and committing it mid-drag would split the drag into
  one undo step per frame (the web timer does the same, web.rs:541-549: no `end_interaction`). `flush_leaving(app,
  prefs) -> Result<(), String>` = `session.end_interaction()` (engine lib.rs:945) + `session.persist()`
  (library.rs:610) + `save_view()` + `prefs.save()`; each failure logged, the first returned (`save_view` logs its
  own errors). Desktop `on_exit` calls `flush_leaving`. 5. `host/gpu_markers.rs`: `window_gpu::Files` (marker /
  remembered / `take_failed` / `arm` / `started` / `armed`, window_gpu.rs:35-87), `gpu_crash_check_at`,
  `gpu_lost_check_at`, `record_gpu_lost` (main.rs:131-208) and their tests, config dir as a parameter (they use only
  `lightcraft_engine::gpu::backend`, no eframe). The notice functions (`gpu_crash_notice`, main.rs:160-166) take a
  `how_to_force: &str` hint: the desktop passes today's `LIGHTCRAFT_GPU_BACKEND=…` sentence, Android (APK-19/20)
  "Settings ▸ Performance, or adb shell setprop debug.lightcraft.gpu_backend gl".
  6. `crates/gpu/src/window.rs`: `install_error_handler` and
  `check_pipeline` (window_gpu.rs:199-215; wgpu + pollster only, both already gpu dependencies; wgpu is one version,
  so `wgpu::Device` is eframe's), reached as `lightcraft_engine::gpu::window`. 7. Stays in the app: `choose`/the
  Windows probe, `relaunch_after_failure`, Windows renderer memory, XWayland, `rfd`. 8. Anything needing a dependency
  ui-egui lacks stays in the app; note it in the commit. 9. `docs/showcase/run.py` (still one file, argparse):
  `--port N` (default env `LIGHTCRAFT_CONTROL_PORT`, else 7980; today the port is hard-coded, run.py:5) and `--token T`
  (default env `LIGHTCRAFT_CONTROL_TOKEN`; when set, added to every request as `"token"`).
- **Tests:** the moved tests (main.rs `mod tests` at :1116, which includes the gpu-marker tests, `config_path_tests`
  at :1557, window_gpu tests, logging tests) move with their code and pass unchanged; new
  `flush_leaving_commits_open_interaction_and_view` (ui-egui headless: begin a slider interaction, call
  `flush_leaving` with no frame in between, reopen: the edit and `view.json` are there),
  `periodic_flush_keeps_open_interaction` (begin an interaction, call `flush_periodic`: `session.interaction` is
  still `Some` and the undo stack is unchanged), `install_with_uses_given_spec` (pure `Filter::parse(spec)
  .level_for(target)`, never `log::set_logger`), `crash_notice_uses_host_hint`.
- **Verify:** `cargo test -p lightcraft -p lightcraft-ui-egui -p lightcraft-gpu && cargo xtask wasm && python3 -m
  py_compile docs/showcase/run.py && cargo xtask ci`; look at the desktop through the headless path (this container
  has no libEGL and no Vulkan ICD): `cargo run -p lightcraft-cli -- snapshot --demo -o target/apk7.png`, then Read
  the PNG. Optional windowed run: `xvfb-run -a cargo run -p lightcraft -- --control 18555` and `python3
  docs/showcase/run.py --port 18555 <script>` with `{"method":"ui.screenshot","params":{"path":"<scratch>/apk7.png",
  "headless":true}}`.
- **Depends on:** APK-6  **Needs:** none  **Model:** opus — refactor across a 1500-line host with behaviour to keep.
- **Done when:** no moved function remains duplicated; the desktop app runs as before.

#### APK-8 Diagnostics switches without the environment
- **Goal:** every `LIGHTCRAFT_PROFILE` and `LIGHTCRAFT_GPU_BACKEND` read can be set by a host that has no
  environment variables, before first use (`LIGHTCRAFT_LOG` is covered by APK-7's `host::log_file::install_with`; the
  engine's CLI/MCP `StderrLog`, engine logging.rs:74, isn't used by the Android host and stays as it is).
- **Files:** new `crates/profile/{Cargo.toml,src/lib.rs}` (package `lightcraft-profile`, L0, no dependencies);
  changed root `Cargo.toml` (workspace dep), `xtask/src/layers.rs` (`("profile", Class::Layer(0))`), the nine
  readers: `crates/catalog/src/journal.rs:173`, `crates/pipeline/src/lib.rs:709`, `crates/gpu/src/render.rs:575`,
  `crates/denoise/src/cpu.rs:186`, `crates/denoise-core/src/run.rs:213`, `crates/faces/src/net.rs:274,287`,
  `crates/engine/src/denoise.rs:963`, `crates/engine/src/camera_preview.rs:433,469` and their `Cargo.toml`s;
  `crates/gpu/src/backend.rs` (`env_choice`, :101-108).
- **Steps:** 1. `lightcraft_profile::{enabled() -> bool, set_enabled(bool)}`: an `AtomicU8` (unset / off / on);
  unset reads `LIGHTCRAFT_PROFILE` once. No common lower crate exists (faces and denoise-core share nothing with
  catalog), hence the tiny crate. 2. Replace the nine reads (and the three local `OnceLock`s) by `enabled()`.
  3. `backend::set_choice(BackendChoice) -> Result<(), BackendChoice>` = `set_choice_in(&C, c)`, with `env_choice`'s
  `OnceLock` moved to a module static `C`; the pure `fn set_choice_in(cell: &OnceLock<BackendChoice>, c) -> Result<(),
  BackendChoice>` fills the cell or returns `Err(existing)` when it was already read.
- **Tests:** process-wide statics are never touched by tests (every GPU test that calls `compute_backends()` fills
  the same `OnceLock`, so order would decide the outcome): `profile_state_overrides_env` (profile, pure over a local
  `AtomicU8` and an injected env reader), `set_choice_in_fills_empty_cell` and `set_choice_in_after_read_errs` (gpu,
  `let cell = OnceLock::new()`).
- **Verify:** `cargo test -p lightcraft-profile -p lightcraft-gpu -- profile_state set_choice_in && cargo xtask layers
  && cargo xtask ci`; `grep -rn 'var_os("LIGHTCRAFT_PROFILE")' crates --include=*.rs` shows only `crates/profile`
  and tests.
- **Depends on:** APK-5  **Needs:** none  **Model:** opus — seven crates on four layers, OnceLock ordering.
- **Done when:** one profile switch; desktop behaviour unchanged.

#### APK-9 Temp files keep the target's extension
- **Goal:** atomic writes work in Android's shared `Pictures/` folder, where MediaProvider refuses to create a file
  whose extension maps to a non-image MIME type (`.tmp` → application/octet-stream → EPERM).
- **Files:** changed `crates/catalog/src/safe_file.rs` (`create_temp`, :111-125).
- **Steps:** temp name `.lightcraft-<pid>-<n>-tmp.<ext>` when the target has an extension, `.lightcraft-<pid>-<n>.tmp`
  otherwise; same `create_new` + rename + removal on error. Check that nothing else matches `.tmp`
  (`grep -rn '\.tmp"' crates apps --include=*.rs`).
- **Tests:** `temp_name_keeps_final_extension`; `write_atomic_into_image_only_dir` (a fake check that rejects names
  whose extension isn't an image one, wrapping the temp-name fn).
- **Verify:** `cargo test -p lightcraft-catalog -- safe_file && cargo xtask ci`
- **Depends on:** APK-5  **Needs:** none  **Model:** sonnet — one function and two tests.
- **Done when:** tests pass; desktop exports unchanged.

## Phase 1: host crates, first APK, CI

#### APK-10 Crates, entry point and start guard
- **Goal:** the three crates exist, are registered, compile everywhere and lint for Android.
- **Files:** new `apps/lightcraft-android/{Cargo.toml,src/lib.rs}`, `crates/android-host/{Cargo.toml,src/lib.rs,
  src/run.rs}`, `crates/android-sys/{Cargo.toml,src/lib.rs,src/jni_env.rs}`; changed root `Cargo.toml` (workspace
  deps above), `xtask/src/layers.rs` (TABLE rows), `xtask/src/licences.rs` (`SHIPPED` += `lightcraft-android`),
  `assets/licences/third-party.json`, `CLAUDE.md`, `AGENTS.md` (unsafe bullet names both crates).
- **Steps:** 1. Entry crate as in Architecture; `[lints] workspace = true`; one `#[allow(unsafe_code)]` on
  `android_main` with `// SAFETY:`; body under `#[cfg(target_os = "android")]` so the crate is empty elsewhere.
  **Whenever `run()` returns** (`Ok(Exit::Destroyed)`, `Ok(Exit::Closed)`, `Err` after the failure dialog was
  dismissed, or a caught panic), the entry crate logs the outcome, flushes the log and calls
  `std::process::exit(exit_code(&result))`: winit can't build a second `EventLoop` in this process
  (event_loop.rs:118-133), and when android_main returns android-activity calls `ANativeActivity_finish`
  (glue.rs:985-993) while Android usually keeps the process cached, so without the exit the next tap on the icon would
  hit the start guard and appear to do nothing. Documented as the one exception to android-activity's no-exit advice.
  2. All Android-only dependencies under `[target.'cfg(target_os = "android")'.dependencies]` (Architecture).
  3. `android-host::run(app: AndroidApp) -> Result<Exit, String>` (android only) with `pub enum Exit { Destroyed,
  Closed }` and a pure `pub fn exit_code(&Result<Exit, String>) -> i32` (both compiled everywhere): a process-wide
  `static STARTED: AtomicBool`; the guard is a pure fn `try_start(&AtomicBool) -> Result<(), String>`; a second call
  returns `Err("LightCraft was reopened in the same process; …")` and the entry crate exits (backstop).
  4. `android-sys::jni_env`: `JavaVM::singleton()?`, `with_activity(app, |env, activity| …) -> Result<T, Error>` with
  the one unsafe cast (`env.as_cast_raw`), `#[allow(unsafe_code)]` on that fn only, and `vm.attach_current_thread(|env|
  …)` (jni 0.22's closure API). No `ndk-context`. A thread-local depth flag behind a pure `ReentryGuard` makes a
  nested `with_activity` return `Err(Error::Reentrant)`. API doc: every loop over Java objects runs each iteration
  inside `env.with_local_frame(16, …)`; one `Env` in scope: always shadow `env`, inside `with_local_frame` use only
  the closure's `env` (jni 0.22 env.rs:417 panics otherwise), never call `with_activity` inside a JNI closure;
  android-sys fns called from inside a `with_activity` closure take `&mut Env`; code on android_main never waits on a
  `run_on_java_main_thread` result (Ground rules). 5. Java errors: map `jni::errors::Error::CaughtJavaException {
  name, msg, .. }` (errors.rs:158-170; `attach_current_thread` returns it for a left-over exception,
  vm/java_vm.rs:479-481) to `Error::Java { class: name, message: msg }`, and `Error::SecurityViolation`
  (errors.rs:181-183) to `Error::Java { class: "java.lang.SecurityException", message: "" }`; inside a closure,
  `env.exception_catch()?` (env.rs:1181) gives the same. No hand-written `getClass`/`getMessage` calls. A pure `fn
  user_message(class: &str, message: &str, action: &str) -> String` turns known classes into actionable text
  (`ActivityNotFoundException` → "No app can open this", `SecurityException` → "Access was withdrawn",
  `FileNotFoundException` → "The file is gone"). 6. Production crate attributes (`#![deny(clippy::unwrap_used, …)]`;
  `#![forbid(unsafe_code)]` in android-host). 7. `cargo xtask licences --write`.
- **Tests:** `second_start_is_refused`, `exit_code_for_each_outcome` (android-host, x86_64);
  `java_exception_maps_to_actionable_error`, `security_violation_maps_to_security_exception`,
  `nested_with_activity_is_an_error` (pure `ReentryGuard`) (android-sys); `cargo xtask layers` fails "unregistered"
  without the rows.
- **Verify:** `cargo test -p lightcraft-android-host` **alone** (proves no winit/android-activity leaks into the
  Linux build) `&& cargo test -p lightcraft-android && cargo test -p lightcraft-android-sys && cargo xtask layers &&
  cargo xtask licences && cargo xtask android check && cargo xtask wasm && cargo xtask ci`
- **Depends on:** APK-7  **Needs:** rust-target  **Model:** opus — unsafe placement, JNI API, workspace wiring.
- **Done when:** all three crates lint clean for `aarch64-linux-android` (APK-5 now derives the set from
  `lightcraft-android`); android-sys passes `cargo xtask wasm` (it is `Standalone`, so `wasm_set()` includes it;
  android-host and lightcraft-android are `Exempt` and not checked there); android-host and lightcraft-android
  compile empty on x86_64 (`cargo test -p …` alone).

#### APK-11 First pixel: budget, Session, Services, eframe
- **Goal:** `run()` opens the UI with the procedural demo library in internal storage, inside a memory budget a
  phone can hold, with exports and screenshots able to write files.
- **Files:** new `crates/android-host/src/{paths.rs,memory.rs,loader.rs,state.rs,app.rs,services.rs}`; changed
  `crates/android-host/src/{lib.rs,run.rs}`, `crates/android-host/Cargo.toml`, `crates/engine/src/memory.rs`
  (`set_total_ram`, `set_default_budget`, `total_ram()` made `pub`), `crates/ui-egui/src/lib.rs`
  (`LightcraftApp::fonts_ready()`).
- **Steps:** 0. **Start order.** Everything `run()` does before the event loop's first poll blocks the Java main
  thread (onStart/onResume, `set_input`, surfaceCreated wait for android_main; Ground rules), so a slow start is a
  black screen and a tap during it is an input-dispatch ANR. Before the loop `run()` does only quick work, so
  android_main reaches its first poll within about 1 s: paths, memory budget, log, panic hook (and, from later cards,
  properties and GPU markers). Opening the session and seeding the demo library run on a worker (`loader.rs`); the
  desktop order (open the session, then `run_native`) does not carry over. 1. `paths.rs` (pure): `internal_data_path()`
  is `/data/user/0/<id>/files` (so `run-as <id>` paths start with `files/`); derive `config/`, `library/`, `logs/`,
  `inbox/`; `engine::config::use_dir(config)`. 2. `memory.rs` (pure): `mem_total(meminfo: &str) -> Option<u64>` and
  `budget_for(meminfo) -> usize` = MemTotal / 8 clamped to [256 MiB, 768 MiB] (256 MiB when unreadable); `run` gives
  it to the new `lightcraft_engine::memory::set_default_budget` (so Settings ▸ Performance ▸ Memory set to Automatic,
  a cleared `memory_mb` in `ui.json` and the bug report use the phone budget, not a quarter of RAM up to 1.5 GiB),
  applies it with `set_budget` and calls the new `lightcraft_engine::memory::set_total_ram(bytes)` (an injected
  `OnceLock` that `total_ram()` consults first; no android cfg in engine), so export's `too_big_for_memory`
  (export.rs:1654) and merge's `fit_in_memory` (merge.rs:155, 200) work on Android. (`ActivityManager.getMemoryClass()`
  is not used: it bounds the Java heap, not native allocations.) 3. Audit `grep -rn 'target_os = "linux"' crates
  --include=*.rs` (known: engine memory.rs, ui-egui lib.rs, panels/dialogs.rs, panels/settings.rs): list in the
  commit what else Android silently skips and whether it matters. 4. Log file (`host::log_file`) under `logs/`, plus
  stderr (already logcat); `engine::guard::install_hook(<config>/panic.log)`. 5. `loader.rs`: `Loader::start(open:
  impl FnOnce() -> (Session, Option<LibraryProblem>) + Send + 'static) -> Loader` spawns a thread running
  `host::session_open` with `library/` (seeding the demo library on first start) and returns at once; `poll(&self) ->
  Option<…>` is a `try_recv`, never a blocking `recv`. First step: a compile-time `const _: fn() = || { fn
  is_send<T: Send>() {} is_send::<Session>(); };`; if `Session` is not `Send`, stop and record why in the commit
  instead of opening it on android_main. No session exists yet, so there is no activity stack for a `TaskGuard`: the
  loading frame is the progress indicator. 6. `services.rs`: `fn base_services() -> Services` (compiles on x86_64)
  with `write: Some(Box::new(lightcraft_engine::export::write_file))`, `write_shared: Some(Arc::new(…write_file))`,
  `png` from `lightcraft_codecs::encode_png` (as main.rs:708-711), `log_file`; no pickers yet. 7. `state.rs`:
  `HostState { app: LightcraftApp, prefs: PrefsWriter }` with `flush_leaving()`, `flush_dirty()` (=
  `host::flush::flush_periodic`) and `trim()` (APK-18). 8. `app.rs` (android only): the eframe::App wrapper holding
  `Rc<RefCell<Option<HostState>>>` (shared with APK-18's lifecycle wrapper). Until the loader answers, `ui` draws one
  centred "Opening library…" label and `logic` polls the loader with `ctx.request_repaint_after(100 ms)`; on `Some`
  it builds `LightcraftApp` from the session on the UI thread and fills the slot (the label stays until
  `app.fonts_ready()`, so the hand-over frames are never blank). Then `logic` → `app.logic` + `PrefsWriter::tick` +
  `flush_periodic` every 30 s (web precedent; never `flush_leaving`, which would cut an open slider drag into one
  undo step per frame), `raw_input_hook`, `ui`, `on_exit` → `flush_leaving` + `session.close_library()` +
  `shutdown(EXIT_WAIT)` (as main.rs:60-108). A failed `try_borrow_mut` is logged and
  skipped, never a panic. 9. `eframe::run_native("LightCraft", NativeOptions { android_app: Some(app.clone()),
  renderer: Wgpu, .. }, …)`; returns `Ok(Exit::Closed)` or `Err` with the reason (the entry crate logs it and exits).
- **Tests:** `paths_from_internal_dir`, `budget_for_phone_meminfo` (4 GB → 512 MiB, 12 GB → 768 MiB, garbage → 256
  MiB), `startup_work_is_deferred` (`Loader::start` with an opener blocked on a barrier returns at once and `poll()`
  is `None`), `loader_hands_over_session` (after the barrier, `poll()` yields the session),
  `services_have_writer_and_png`, `state_flush_without_a_frame` (all x86_64); engine
  `injected_total_ram_drives_fit_checks` (`too_big_for_memory` refuses a large export with 4 GB injected, through a
  pure `total_ram_in(cell, probe)` so the global stays untouched), `injected_default_budget_is_automatic` (the host's
  budget wins over RAM / 4, `LIGHTCRAFT_MEMORY_MB` over both, through a pure `default_budget_from`).
- **Verify:** `cargo test -p lightcraft-android-host && cargo test -p lightcraft-engine -- total_ram default_budget
  && cargo xtask android check && cargo xtask ci`
- **Depends on:** APK-7, APK-10  **Needs:** rust-target  **Model:** opus — eframe/Android start-up wiring and
  thread hand-over.
- **Done when:** the host compiles and lints for Android; first real run happens in the APK-16 workflow.
- **Maintainer checks on a device:** first start with a 20k-photo library: no ANR, the "Opening library…" frame is
  visible, then the grid.

#### APK-11b Phone limits: library notice and SAM by memory
- **Goal:** decisions 9 and 10 are implemented: the user learns once that uninstalling deletes the library, and AI
  masking is off on phones that can't hold the SAM model (segment/worker.rs:9) instead of being killed by the
  low-memory killer.
- **Files:** changed `crates/ui-egui/src/lib.rs` (`Services::library_notice: Option<String>`,
  `Services::min_ram_for_sam: Option<u64>`), `crates/ui-egui/src/menus.rs` (`enabled()` and tooltip of the AI-mask
  commands; SAM availability today hangs off `session.segmenter.dir`, menus.rs:2159), the SAM setup dialog,
  `crates/ui-egui/src/host/prefs.rs` (a "library notice shown" flag), `crates/ui-egui/src/panels/library_problem.rs`
  (Try Again through `Services`), `crates/engine/src/export.rs` and `crates/ui-egui/src/merge.rs` (wording),
  `crates/ui-egui/locales/*.json`, `crates/android-host/src/{services.rs,memory.rs,app.rs}`.
- **Steps:** 1. `library_notice`: when `Some` and the prefs flag is unset, push it once through the existing notices
  panel and set the flag. Android text: "Your library is stored inside LightCraft. Uninstalling it or Clear data
  deletes the library; back it up with File ▸ Export…". 2. `min_ram_for_sam`: when `memory::total_ram()` (injected by
  APK-11) is below it, the AI-mask commands' `enabled()` is false with the tooltip "Needs a device with 8 GB of
  memory", and the setup dialog shows the same reason. Default `None` = desktop unchanged. 3. Android: pure
  `sam_allowed(mem_total) -> bool` in memory.rs and `min_ram_for_sam = Some(SAM_MIN_RAM)`, where `SAM_MIN_RAM` is 7 GiB (an interpretation of decision 10's "8 GB", flagged there for the maintainer): a phone sold with 8 GB reads 7.2 to 7.9 GiB in `MemTotal` (a literal 8 GiB would exclude every 8 GB phone) and a 6 GB phone about 5.5 GiB; the text still says "8 GB". A device that cannot tell its memory counts as below the limit. 4. Try Again off
  android_main: the library problem window's Try Again (`panels/library_problem.rs`) runs `app.openLibrary` on the
  UI thread, which on Android is android_main (Ground rules (2): a large library's replay there blocks the event
  loop, the ANR APK-11 avoided for the first open); give `Services` a reopen hook the Android host fills with a
  `Loader` behind the "Opening library…" frame (desktop unchanged when `None`). 5. Wording on a phone: export's
  `too_big_for_memory` ("more than this computer has") and the merge note ("than this computer has", ui-egui
  merge.rs) say "this device" (new catalog keys in `crates/ui-egui/locales/*-formats.json`), and "Opening library…"
  gets its `locales/*.json` entries with the next translation update (as do the new notice, reason and dialog strings).
- **Tests:** headless `library_notice_shown_once`, `sam_disabled_below_min_ram` (fake service, injected RAM through
  the pure check, not the global), `try_again_uses_reopen_hook` (a fake hook is called and `app.openLibrary` is
  not); android-host `sam_allowed_for_phone_meminfo`.
- **Verify:** `cargo test -p lightcraft-ui-egui -- library_notice sam_disabled && cargo test -p
  lightcraft-android-host -- sam_allowed && cargo xtask android check && cargo xtask ci`
- **Depends on:** APK-11  **Needs:** rust-target  **Model:** sonnet — three `Services` fields on existing patterns.
- **Done when:** tests pass; desktop unchanged.
- **Maintainer checks on a device:** first start shows the notice once (and again after a kill before OK was pressed);
  on a phone with < 8 GB the AI mask command is disabled with the reason; Try Again in the library problem window
  reopens the library behind the "Opening library…" frame without an ANR.

#### APK-12 `cargo xtask android so`: link the library
- **Goal:** build `liblightcraft_android.so` with the NDK and check its symbols and page alignment.
- **Files:** new `xtask/src/android/{sdk.rs,so.rs,elf.rs}`; changed `xtask/src/android/mod.rs`.
- **Steps:** 1. `sdk.rs`: find `ANDROID_HOME`/`ANDROID_SDK_ROOT` and `ANDROID_NDK_HOME`/`ANDROID_NDK_LATEST_HOME`,
  read the NDK version from `source.properties`; clear error when missing. 2. `so.rs`: set
  `CARGO_TARGET_<TRIPLE>_LINKER=<ndk>/toolchains/llvm/prebuilt/linux-x86_64/bin/<triple><minSdk>-clang` (no
  `cargo ndk`), add `-C link-arg=-Wl,-z,max-page-size=16384` when the NDK is older than r28, build `-p
  lightcraft-android --target aarch64-linux-android` (and `x86_64-linux-android` with `--abi x86_64`), copy to
  `target/android/jniLibs/<abi>/`. 3. `elf.rs`: run the NDK's `llvm-readelf -lW -d --dyn-syms`; pure parsers assert
  every `LOAD` alignment is a power of two ≥ 0x4000, that `android_main` and `ANativeActivity_onCreate` are exported,
  and (`needed_libs_are_system`) that every `NEEDED` entry is one of libc, libm, libdl, liblog, libandroid,
  libnativewindow, libvulkan, libEGL, libGLESv2: a dependency that links C++ (`libc++_shared.so`) or a non-NDK library
  would otherwise pass here and fail only on a device with `dlopen failed: library … not found` (the lock already has
  `cc` users such as ring and wayland-backend, not on the Android path today).
- **Tests:** `sdk::ndk_version_parses`, `so::linker_path_for_abi`, `elf::rejects_4k_load_alignment`,
  `elf::accepts_64k_load_alignment`, `elf::requires_entry_symbols`, `elf::rejects_libcxx_shared` (canned readelf
  output).
- **Verify:** `cargo test -p xtask -- android::`
- **Depends on:** APK-5  **Needs:** ndk (unit tests: none)  **Model:** sonnet — path/argument building with tests.
- **Done when:** unit tests pass here; the APK-16 workflow runs the command.

#### APK-13 Android manifest
- **Goal:** a complete, declarative manifest and a reproducible version code.
- **Files:** new `packaging/android/AndroidManifest.xml` (template with `${APPLICATION_ID}`, `${APP_LABEL}`,
  `${VERSION_CODE}`, `${VERSION_NAME}`, `${MIN_SDK}`, `${TARGET_SDK}`), `xtask/src/android/manifest.rs` (fills it).
- **Steps:** 1. `<application>`: `android:label="${APP_LABEL}"` (decision 1, default "LightCraft"; a literal string,
  so no resource is needed; the icon follows in APK-14 because `@mipmap/ic_launcher` must exist when aapt2 links),
  `android:hasCode="false"`, `android:allowBackup="false"` (Auto Backup would copy
  the catalog to Drive; see decision 9), `android:extractNativeLibs="false"` (keeps the stored, 16 KB-aligned `.so`
  mapped in place), `android:enableOnBackInvokedCallback="false"` **unconditionally** (with a comment: Back reaches
  native code only as KEYCODE_BACK, which targetSdk 36 stops dispatching without this opt-out). 2. Activity
  `android.app.NativeActivity`: `android:exported="true"`, `android:launchMode="singleTask"`,
  `android:theme="@android:style/Theme.DeviceDefault.NoActionBar"` (framework style, no resources),
  `android:configChanges="mcc|mnc|locale|touchscreen|keyboard|keyboardHidden|navigation|screenLayout|fontScale|uiMode|orientation|density|screenSize|smallestScreenSize|layoutDirection|colorMode|fontWeightAdjustment|grammaticalGender"`,
  `<meta-data android:name="android.app.lib_name" android:value="lightcraft_android"/>`, MAIN/LAUNCHER filter.
  Permissions arrive with the tasks that need them (APK-21, APK-22). minSdk 30, targetSdk 36 (decision 3).
  3. `fn version_code(version: &str) -> Result<u32, String>` from `version::read` (xtask/src/version.rs):
  `major*1_000_000 + minor*1_000 + patch*10 + suffix`, suffix 9 for a release, `alpha.N` → N−1, `beta.N` → 2+N,
  `rc.N` → 5+N for N in 1..=3 (so alpha < beta < rc < release). `Err` for any other pre-release that
  `version::validate` accepts (`1.0.0-alpha`, `rc.12`, `dev.1`), for minor > 999 and for patch > 99 (they would
  collide). `${VERSION_NAME}` is the same version string. 4. A debug build fills `${APPLICATION_ID}` with
  `<id>.debug` (decision 1), a release build with `<id>`.
- **Tests:** `manifest_has_required_keys` (parses the filled template; asserts each attribute above individually,
  the label, each `configChanges` value, the theme, `extractNativeLibs`, the back opt-out);
  `version_code_is_monotonic`, `prerelease_sorts_before_release`, `alpha_before_beta_before_rc`,
  `unsupported_prerelease_is_error`, `debug_build_uses_debug_suffix`.
- **Verify:** `cargo test -p xtask -- manifest version_code`
- **Depends on:** APK-5  **Needs:** none  **Model:** sonnet — declarative config with key tests.
- **Done when:** the tests pass; the APK-16 workflow's `aapt2 dump badging` shows the values.

#### APK-14 Launcher icons
- **Goal:** Android launcher and adaptive icons from our own `assets/app-icon/lightcraft.svg`.
- **Files:** new `packaging/android/icons.sh` (single purpose, bash like `packaging/icons.sh`: starts with
  `#!/usr/bin/env bash` and `set -euo pipefail`),
  `packaging/android/res/mipmap-{mdpi,hdpi,xhdpi,xxhdpi,xxxhdpi}/{ic_launcher.png,ic_launcher_foreground.png,
  ic_launcher_monochrome.png}`, `packaging/android/res/mipmap-anydpi-v26/ic_launcher.xml`,
  `packaging/android/res/values/ic_launcher_background.xml`; changed `assets/ATTRIBUTION.md`,
  `packaging/android/AndroidManifest.xml` (`<application android:icon="@mipmap/ic_launcher"
  android:roundIcon="@mipmap/ic_launcher">`), `xtask/src/android/manifest.rs` (`manifest_has_required_keys` asserts
  both).
- **Steps:** install `cargo install resvg --locked`. The SVG is a 512 tile clipped to `rx=112` with a full-bleed
  `#f2a516` field and one paper `<path>`. Background = colour resource `#f2a516`. Foreground = the SVG with the
  `clip-path` group attribute and the background `<rect>` removed and the viewBox widened by `sed` to `-128 -128 768
  768` (the paper fits the 72/108 safe zone), each `sed` followed by a `grep -q` guard as in `packaging/icons.sh:22-23`.
  Monochrome = the same path filled `#000000`. Legacy `ic_launcher.png` = the original tile. Sizes 48/72/96/144/192
  px (foreground layers 108/162/216/324/432 px). ATTRIBUTION row for `packaging/android/res/` ("derived from
  assets/app-icon/lightcraft.svg", MIT OR Apache-2.0, original work). Nothing from `docs/brand/`, no
  `ai.storyteller.lightcraft` id. A 512 px store icon goes in APK-42.
  Add `android:icon` / `android:roundIcon` to the manifest.
- **Tests:** `cargo xtask assets` fails without the ATTRIBUTION row; `manifest_has_required_keys` fails without the
  icon attributes.
- **Verify:** `bash packaging/android/icons.sh && cargo xtask assets && cargo test -p xtask -- manifest`; Read the
  xxxhdpi foreground PNG to look at it.
- **Depends on:** APK-13  **Needs:** none  **Model:** sonnet — asset generation with a spelled-out recipe.
- **Done when:** icons committed with attribution.

#### APK-15 `cargo xtask android apk`: package a debug APK
- **Goal:** one command turns the `.so`, manifest and resources into an installable, 16 KB-aligned debug APK.
- **Files:** new `xtask/src/android/{zip_lib.rs,signing.rs,apk.rs}`, `docs/android.md` (build, install, debugging,
  Device checklist); changed `xtask/Cargo.toml` (`zip = { version = "=7.2.0", default-features = false }`, no
  features: the locked 7.2.0 (through candle-core) needs only crc32fast/indexmap/memchr/typed-path, while its
  `deflate` feature would add zopfli; nothing is compressed here: aapt2's entries are raw-copied and the `.so` is
  Stored; MIT; xtask is not in `SHIPPED`), `xtask/src/android/mod.rs`, `.gitignore` (`*.jks`, `*.keystore`, `*.p12`,
  `keystore.properties`, `local.properties`, `.gradle/`, `build/`).
- **Steps (route per decision 6, default: SDK build-tools ≥ 36, no Gradle):** 1. `aapt2 compile` res → `aapt2 link`
  with `platforms/android-36/android.jar`, the filled manifest and `--debug-mode` for debug builds (so `adb shell
  run-as <id>` works). 2. `zip_lib.rs`: copy every entry of aapt2's output with `ZipWriter::raw_copy_file`, then
  append `lib/<abi>/liblightcraft_android.so` with `CompressionMethod::Stored` and `with_alignment(16384)`.
  3. `signing.rs` (one concern, shared by apk.rs and APK-39's aab.rs; APK-37 adds release signing to it): the debug
  key `~/.android/debug.keystore` is created when missing (no Gradle/AGP runs on this route, so a fresh runner
  doesn't have it) with a pure `debug_keystore_args(path) -> Vec<String>` for `keytool -genkeypair -keystore <path>
  -storepass android -keypass android -alias androiddebugkey -dname "CN=Android Debug,O=Android,C=US" -keyalg RSA
  -keysize 2048 -validity 10000`, never inside the repo; apksigner gets `--ks-pass pass:android` (debug only).
  4. `zipalign -P 16 -f 4` → `apksigner sign` → `zipalign -c -P 16 -v 4`. Pure argument builders for each tool.
  5. `docs/android.md`: install, `adb shell run-as <id> cat files/config/panic.log`, logs, and the Device checklist
  heading.
- **Tests:** `aapt2_link_args_debug_mode` (includes linking `res/`), `zipalign_args_use_16k_pages`,
  `debug_keystore_created_when_missing_args`, `so_entry_is_stored_uncompressed` and `so_entry_data_is_16k_aligned`
  (build a zip from a fake aapt2 output in a temp dir, read it back), `copied_entries_unchanged`.
- **Verify:** `cargo test -p xtask -- apk zip_lib signing`; `git diff --stat Cargo.lock` shows only xtask's new
  dependency line, no new `[[package]]` entries.
- **Depends on:** APK-12, APK-13, APK-14  **Needs:** sdk + ndk (unit tests: none)  **Model:** sonnet — tool
  invocation with tests; the zip step is named.
- **Done when:** unit tests pass here; the APK-16 workflow produces the APK.

#### APK-16 Android CI workflow
- **Goal:** prove linking, packaging and start-up on every push, since this container can't.
- **Files:** new `.github/workflows/android.yml`; changed `docs/android.md` (Device checklist line for the run).
- **Steps:** ubuntu-latest; `rustup target add aarch64-linux-android x86_64-linux-android`; `cargo xtask android
  check`; `sdkmanager "platforms;android-36" "build-tools;36.0.0"`; `CARGO_PROFILE_DEV_DEBUG=line-tables-only cargo
  xtask android so --abi arm64-v8a --abi x86_64` (record the `.so` sizes in the job summary); `cargo xtask android
  apk`; `aapt2 dump badging`, failing unless it shows `application-label:'LightCraft'` and an `application-icon-`
  line. KVM udev step before the emulator: `echo 'KERNEL=="kvm", GROUP="kvm", MODE="0666",
  OPTIONS+="static_node=kvm"' | sudo tee /etc/udev/rules.d/99-kvm4all.rules && sudo udevadm control --reload-rules
  && sudo udevadm trigger --name-match=kvm`. `reactivecircus/android-emulator-runner` (pinned by SHA), x86_64 API 35
  image, `emulator-options: -no-window -gpu swiftshader_indirect -no-snapshot -noaudio -no-boot-anim`, script: `adb
  install`, `adb shell am start -n <id>/android.app.NativeActivity`, wait, `adb exec-out screencap -p > shot.png`,
  `adb logcat -d --pid=$(adb shell pidof <id>)` plus `AndroidRuntime`/`DEBUG` lines naming `<id>`; fail on
  `panicked` / `FATAL`. Upload APK, screenshot and logcat. Triggers: `workflow_dispatch` and pushes touching Android
  paths.
- **Tests:** the workflow run itself (maintainer or agent per decision 0).
- **Verify:** here: YAML parse (rule above), `actionlint` if available, `cargo test -p xtask`; then the CI-run rule.
- **Depends on:** APK-15  **Needs:** ci  **Model:** sonnet — workflow YAML.
- **Done when:** the workflow is committed and checked here, and `docs/android.md` → Device checklist has "android.yml
  run green; screenshot shows the LightCraft UI" (ticked by whoever saw the run).
- **Maintainer checks on a device:** install the CI APK on an arm64 phone; it opens on the demo library.

#### APK-17 Back key and Quit
- **Goal:** Back navigates inside the app; nothing ends the one-shot event loop by accident.
- **Files:** changed `crates/ui-egui/src/menus.rs` (`view.back` arm :615, `enabled("app.quit")`),
  `crates/ui-egui/src/control.rs` (`app.quit` arm :461), `crates/ui-egui/src/lib.rs` (`Services::move_to_back:
  Option<HostAction>`, `Services::quit_disabled: bool`), `crates/ui-egui/src/menubar.rs` (Quit hidden when disabled),
  `crates/ui-egui/src/panels/library_problem.rs` (Quit choice, :157), `crates/ui-egui/src/panels/notices.rs` (quit
  prompts, :137, 153, 159), new `crates/android-sys/src/activity.rs` (`move_task_to_back`), changed
  `crates/android-host/src/{services.rs,app.rs}`.
- **Steps:** 1. Append one final branch to the existing `view.back` cascade (menus.rs:615-643: dialog, lights out,
  keyword painter, fullscreen, tool, crop, person page, compare/survey, detail → grid): when nothing else applied and
  `services.move_to_back` is `Some`, call it. Keep the existing order and the Escape binding. 2. Back is Escape: the
  android-host `raw_input_hook` (app.rs) rewrites `Event::Key { key: Key::BrowserBack, .. }` into the same event with
  `key: Key::Escape` (pure `browser_back_to_escape(&mut Vec<Event>)`). Every existing Escape path then applies first:
  the shortcut dispatcher keeps Escape from commands while a popup is open (shortcuts.rs:390, 399), egui menus,
  combo boxes and context menus close on Escape, and so do album_picker.rs:333, date_picker.rs:346 and TextField's
  Esc-restores-text; `view.back` (bound to Escape) runs only when nothing else took it, and `move_to_back` is its last
  step. Nothing in ui-egui maps `BrowserBack`. 3. `quit_disabled` (default false = desktop unchanged):
  `enabled("app.quit")` is false, so dispatch, Cmd+Q, the menubar and control `app.quit` all refuse; the control arm
  checks it before sending `ViewportCommand::Close`, and `ui.quit` (lib.rs:1433) too. The other buttons that set
  `app.ui.quit` follow: library_problem drops its Quit choice (Retry and Continue Without Saving stay; Choose comes
  with APK-24), so a corrupt catalog or full disk never shows a Quit button that does nothing; the quit prompts
  ("Quit Anyway" / "Try Saving Again", notices.rs) can't open, because `app.quit` is refused before `may_close` runs.
  4. Android: `quit_disabled = true`, `move_to_back` = JNI `Activity.moveTaskToBack(true)` posted with
  `run_on_java_main_thread` (fire and forget, no result awaited).
- **Tests:** `back_at_root_moves_to_back` (fake service), `back_order_unchanged_on_desktop` (existing arms still
  first; the existing Escape/back tests stay green), `back_closes_open_context_menu_first` (headless: open a grid
  context menu, send Escape: the menu closes, the view is unchanged), `quit_refused_when_disabled` (command, control
  and the quit-prompt path), `library_problem_has_no_quit_when_disabled` (headless: no widget with the Quit id, and
  `ui.clickWidget` on it errors); android-host `browser_back_becomes_escape` (pure).
- **Verify:** `cargo test -p lightcraft-ui-egui -- back quit library_problem && cargo test -p lightcraft-android-host
  -- browser_back && cargo xtask android check && cargo xtask ci`
- **Depends on:** APK-11  **Needs:** rust-target  **Model:** opus — navigation order and command gating.
- **Done when:** tests pass; desktop unchanged.
- **Maintainer checks on a device:** Back with gesture navigation on Android 16 (targetSdk 36) walks back to the grid,
  then moves the app to the background; Home → reopen; Back to root → reopen.

#### APK-18 Lifecycle: flush, trim and system destroy
- **Goal:** work survives backgrounding and process death, the background process holds little memory, and a
  system-initiated Activity destroy ends the process cleanly instead of hanging, all without blocking the Java main
  thread.
- **Files:** new `crates/android-host/src/lifecycle_core.rs` (pure, compiled everywhere),
  `crates/android-host/src/lifecycle.rs` (android only); changed `crates/android-host/src/{run.rs,state.rs,app.rs}`
  (switch to `eframe::create_native` with our own `EventLoop::with_user_event().with_android_app(app)`; the wrapper
  and the eframe::App share the `Rc<RefCell<Option<HostState>>>` slot of APK-11, created before `create_native` and
  filled when the loader hands over), `crates/android-sys/src/activity.rs` (`activity_state() ->
  Result<ActivityState { finishing, destroyed }, Error>`), `crates/engine/src/lib.rs` (`Session::trim_caches()`).
- **Steps:** 1. `lifecycle_core.rs` (pure): `enum HostEvent { Resumed, Suspended, FocusLost, FocusGained,
  MemoryWarning, AboutToWait }`; `trait Host { fn flush_leaving(&mut self); fn flush_dirty(&mut self); fn trim(&mut
  self); fn activity_state(&self) -> Result<ActivityState, String>; }` (implemented by `HostState` and by a test
  fake); `fn on_event<H: Host>(ev, slot: &RefCell<Option<H>>, core: &mut CoreState) -> Action { Continue, Exit }`.
  Events before the slot is filled are no-ops; a failed `try_borrow_mut` is logged and skipped. Mapping:
  - `FocusLost` → `flush_leaving()` (end interaction, persist, view, prefs). It comes before surfaceDestroyed when the
    app goes to the background, and focus changes don't block the Java main thread (`notify_focus_changed` only
    writes a command, glue.rs:781-790).
  - `Suspended` → `trim()` (memory only, no I/O: `Session::trim_caches()` drops the preview/decode caches, each named
    in its doc comment, plus `lightcraft_engine::gpu::trim_pool(0)`), and `flush_dirty()` (`persist_if_dirty` +
    `save_view`) only when no `FocusLost` flush ran since the last `FocusGained`. Why so little: `suspended` comes
    from TerminateWindow, and `set_window(None)` keeps the Java main thread waiting until `post_exec_cmd(TermWindow)`,
    which runs only after winit's whole callback returns (glue.rs:482-503, 645-651), so a full flush here would hold
    surfaceDestroyed for its whole duration (ANR risk on a large catalog). Trimming on background is the real guard
    against the low-memory killer: NativeActivity doesn't forward `onTrimMemory`, only the rare `onLowMemory`.
  - `MemoryWarning` → `trim()`.
  - `AboutToWait` after `Suspended` or `FocusLost` and until the next `Resumed`/`FocusGained` → `activity_state()`
    (JNI `Activity.isFinishing()` / `isDestroyed()`; `performDestroy` sets `mDestroyed` before `onDestroy`); a pure
    `should_leave(state, suspended) -> bool` decides; `Exit` when true. A JNI error is logged and counts as "stay".
  2. `lifecycle.rs` (android only): a thin `ApplicationHandler<UserEvent>` that forwards every method to
  `EframeWinitApplication`, maps the winit callbacks to `HostEvent`, calls `on_event`, and on `Exit` calls
  `flush_leaving` and `event_loop.exit()`. 3. When `run_app` returns because of that, `run` returns
  `Ok(Exit::Destroyed)` and the entry crate exits the process (APK-10), so the next launch starts fresh. 4. Long jobs
  keep their `TaskGuard` rows; `docs/android.md` notes the cached-apps freezer (Risks).
- **Tests:** engine `trim_caches_empties_preview_cache`; android-host (pure, x86_64, fake `Host`):
  `core_flushes_on_focus_loss`, `suspend_after_focus_loss_does_not_reflush`, `core_trims_when_backgrounded`,
  `core_maps_memory_warning_to_trim`, `core_leaves_when_destroyed_while_suspended`, `focus_loss_alone_keeps_loop`,
  `events_before_app_created_are_noops`. The android-only forwarding is proven by the APK-16 smoke run and the device
  checks.
- **Verify:** `cargo test -p lightcraft-engine -- trim && cargo test -p lightcraft-android-host -- core_ focus_loss
  events_before && cargo xtask android check && cargo xtask ci`
- **Depends on:** APK-11, APK-17  **Needs:** rust-target  **Model:** opus — event-loop wrapping and lifecycle
  reasoning.
- **Done when:** tests pass; Device checklist line "APK-16 smoke run still opens the UI with the lifecycle wrapper".
- **Maintainer checks on a device:** edit, Home, `adb shell am kill <id>`, reopen: edits and UI state are back; Home
  during a slider drag: no ANR, one undo step; rotate; Developer options ▸ "Don't keep activities" on → leave →
  reopen: no ANR, the app reopens; remove from recents → reopen; Home with a 20k-photo library: no ANR.

#### APK-19 Renderer failure and GPU start markers
- **Goal:** a GPU that won't start (error, panic or driver crash) gives a message the user can read, and the next
  launch tries the other renderer.
- **Files:** new `crates/android-host/src/renderer.rs`, `crates/android-sys/src/dialog.rs`; changed
  `crates/android-host/src/run.rs`.
- **Steps:** 1. Order: a pure `renderer::order(forced: Option<WindowRenderer>, failed: Option<WindowRenderer>,
  compiled: Backends) -> Vec<WindowRenderer>` over `backend::window_renderer_order(forced, None, failed, "android",
  compiled, false)` (backend.rs:313; yields Vulkan then GL for android; `remembered` is Windows-only on desktop too),
  with `failed` from `host::gpu_markers::Files::take_failed()` under `<config>`. `run.rs` passes `forced = None` (APK-20
  wires the property in) and reads `compiled` from `wgpu::Instance::enabled_backend_features()`; renderer.rs never
  reads it itself, because `cargo test -p lightcraft-android-host` alone builds wgpu without `gles` on x86_64 and
  would drop GL. The first entry's backends go into `NativeOptions::wgpu_options` as desktop's non-Windows path does
  (window_gpu.rs:88-136). 2. `files.arm(r)` before `create_native`,
  `files.started(r)` in the app creator, which also sets an `app_created` flag and calls
  `lightcraft_engine::gpu::window::install_error_handler` on the render state's device. 3. `gpu_crash_check_at(
  Some(<config>/gpu-init.marker), false)` / `gpu_lost_check_at` → `lightcraft_engine::gpu::set_enabled(false)` plus a
  notice (as main.rs:131-208) with the Android `how_to_force` hint (APK-7).
  4. `std::panic::catch_unwind` around the run, mapping a panic to `Err` (main.rs:956-1045).
  5. After `run_app` returns, a pure `exit_reason(app_created, exit_requested, last_error) -> Option<String>`
  (`last_error` from `host::log_file::last_error()`, which also catches eframe's "Exiting because of error") decides
  whether to show the failure. 6. `dialog.rs` posts an `AlertDialog` (reason + log path; buttons with `null`
  listeners) via `run_on_java_main_thread`; then android_main keeps running: a pure `wait_until(dismissed: impl
  FnMut() -> Result<bool, String>, tick, limit)` drives a closure that calls `app.poll_events(Some(200 ms), …)`
  (keeps Pause/Stop acknowledged, no ANR; a Destroy ends the wait) and polls `isShowing()` through `with_activity`; at
  most 10 minutes; a JNI error ends the wait at once (logged). Only then does `run` return `Err`, and the entry crate
  exits the process (APK-10), so the next tap on the icon starts a fresh process that tries the other renderer. The
  panic path waits the same way.
- **Tests:** `failed_vulkan_start_tries_gl_next` (marker left behind → GL first; passes `compiled = Backends::VULKAN
  | Backends::GL` explicitly), `exit_without_app_shows_last_error`, `normal_exit_shows_nothing`,
  `waits_for_dismissal_then_returns`, `jni_error_ends_wait` (fake closures, x86_64).
- **Verify:** `cargo test -p lightcraft-android-host -- renderer && cargo test -p lightcraft-android-host -- exit_
  waits_for jni_error && cargo xtask android check && cargo xtask ci`
- **Depends on:** APK-7, APK-8, APK-18  **Needs:** rust-target  **Model:** opus — GPU fallback and failure design.
- **Done when:** tests pass; Device checklist line "CI logcat shows the adapter line".
- **Maintainer checks on a device:** see APK-20 (the checks need the property).

#### APK-20 Diagnostics switches via system properties
- **Goal:** `debug.lightcraft.profile`, `.log`, `.gpu_backend`, `.control` stand in for environment variables (apps
  get none).
- **Files:** new `crates/android-sys/src/props.rs`; changed `crates/android-host/src/{run.rs,renderer.rs}` (test),
  `docs/android.md`.
- **Steps:** read the properties with `android-properties` (already in `Cargo.lock` through android-activity; licence
  checked by `cargo xtask licences`; target-gated). A pure `fn settings_from(props: impl Fn(&str) -> Option<String>)
  -> DiagSettings`. `run` applies them first thing, before any record is logged or any switch is read:
  `host::log_file::install_with(&filter_spec(props.log, None))` (APK-7), `lightcraft_profile::set_enabled`,
  `backend::set_choice` (APK-8); `run.rs` passes `settings.gpu_backend.and_then(WindowRenderer::parse)` as `forced` to
  `renderer::order` (APK-19); `.control` feeds APK-21. All of it is quick (Ground rules: before the first poll).
- **Tests:** `props_map_to_settings`, `unknown_values_are_ignored` (pure, x86_64, android-sys);
  `forced_backend_from_props_goes_first` (android-host, pure `renderer::order`).
- **Verify:** `cargo test -p lightcraft-android-sys -- props && cargo test -p lightcraft-android-host -- forced_backend
  && cargo xtask android check && cargo xtask ci`
- **Depends on:** APK-8, APK-11, APK-19  **Needs:** rust-target  **Model:** sonnet — mapping onto setters that exist.
- **Done when:** documented in `docs/android.md`.
- **Maintainer checks on a device:** `adb shell setprop debug.lightcraft.gpu_backend gl`, restart: the UI runs on GL;
  `setprop … vulkan` again. Renderer failure (APK-19): make the first renderer fail (e.g. an emulator image without
  Vulkan with `debug.lightcraft.gpu_backend vulkan`), the failure dialog shows, dismiss it, tap the icon **once**: the
  app opens on GL. `setprop debug.lightcraft.log debug`: logcat shows debug lines.

#### APK-21 Debug control server with a token
- **Goal:** drive and screenshot the app on a device with the usual JSON-lines scripts, without letting other apps on
  the device (which share loopback) drive it.
- **Files:** move `apps/lightcraft/src/control_server.rs` → `crates/ui-egui/src/host/control_server.rs` (plain
  `std::net`, 127.0.0.1); changed `apps/lightcraft/src/main.rs`, `crates/android-host/src/run.rs`,
  `packaging/android/AndroidManifest.xml` (`android.permission.INTERNET`, also needed for model downloads through
  `crates/fetch`), `xtask/src/android/manifest.rs` (test), `crates/android-host/Cargo.toml` (`getrandom = "0.3"`
  under the android target table; the locked 0.3.4), `.github/workflows/android.yml` (token step), `docs/android.md`.
- **Steps:** 1. `start(port, ctx, token: Option<String>)`: with a token every request must carry a matching `"token"`
  field, else `{"ok": false, "error": "bad token"}`; desktop passes `None` (unchanged). 2. Android starts the server
  only when `debug.lightcraft.control=<port>` is set (shell-only, `adb shell setprop`); no default port. It writes a
  random 128-bit token (`getrandom`) to `<internal_data_path>/control-token` (= `files/control-token`, APK-11).
  3. Document: `adb shell setprop debug.lightcraft.control 18555`, restart, `adb forward tcp:18555 tcp:18555`, `adb
  shell run-as <id> cat files/control-token`, then `python3 docs/showcase/run.py --port 18555 --token <token>
  <script>` (APK-7's options) with `ui.screenshot`. 4. `android.yml`: after the smoke test, set the property, restart,
  forward the port, read the token and run a one-line `ui.inspect` script that way.
- **Tests:** the existing control_server tests move with it and pass; `rejects_request_without_token`,
  `no_server_without_property` (pure start decision); manifest test asserts INTERNET.
- **Verify:** `cargo test -p lightcraft-ui-egui -- control_server && cargo test -p xtask -- manifest && python3 -m
  py_compile docs/showcase/run.py && python3 -c 'import yaml,sys; yaml.safe_load(open(sys.argv[1]))'
  .github/workflows/android.yml && cargo xtask ci`
- **Depends on:** APK-7, APK-13, APK-16, APK-20  **Needs:** rust-target  **Model:** sonnet — move plus a start
  condition and a check.
- **Done when:** desktop `--control` unchanged; the APK-16 workflow has the token step (Device checklist line "CI
  `ui.inspect` over the forwarded port answers").
- **Maintainer checks on a device:** the documented recipe returns a screenshot; without the token it is refused.

## Phase 2: real photos

#### APK-22 Photo permissions
- **Goal:** ask for and track access to the photo library without blocking the UI thread.
- **Files:** new `crates/android-sys/src/permissions.rs`; changed `packaging/android/AndroidManifest.xml`,
  `xtask/src/android/manifest.rs` (test), `crates/android-host/src/{services.rs,app.rs}`.
- **Steps:** 1. Manifest: `READ_MEDIA_IMAGES`, `READ_MEDIA_VISUAL_USER_SELECTED`, `READ_EXTERNAL_STORAGE`
  `android:maxSdkVersion="32"`, `ACCESS_MEDIA_LOCATION`. 2. Pure `permissions_for_sdk(sdk) -> &[&str]`, three cases:
  30–32 `[READ_EXTERNAL_STORAGE, ACCESS_MEDIA_LOCATION]`; 33 `[READ_MEDIA_IMAGES, ACCESS_MEDIA_LOCATION]`; 34+
  `[READ_MEDIA_IMAGES, READ_MEDIA_VISUAL_USER_SELECTED, ACCESS_MEDIA_LOCATION]` (`READ_MEDIA_VISUAL_USER_SELECTED`
  exists from API 34; `ACCESS_MEDIA_LOCATION` is a runtime permission: unless requested, EXIF GPS read through FUSE is
  redacted). `sdk` = `AndroidApp::sdk_version()` (android-activity lib.rs:1261), no JNI `Build.VERSION` read.
  3. Pure state machine `NotAsked → Pending → Granted | Partial | Denied` (`Partial` only on 34+) with a clock and
  focus events: after a request, no `Focused(false)` within 1 s (no system dialog appeared: permanently denied, and
  NativeActivity drops `onRequestPermissionsResult`) or `Focused(true)` while still not granted → `Denied`.
  4. JNI: `Activity.requestPermissions` posted with `run_on_java_main_thread`; `checkSelfPermission` re-checked on
  each `Focused(true)` and every 500 ms while `Pending`, from the android-host App wrapper's `logic` (app.rs), which
  calls `ctx.request_repaint_after(500 ms)` while Pending. 5. Denied → the command's error: "LightCraft needs access
  to your photos: allow it in Settings ▸ Apps ▸ LightCraft ▸ Permissions".
- **Tests:** `pending_then_granted`, `partial_access_is_reported`, `api33_never_partial`, `no_dialog_means_denied`,
  `denied_gives_actionable_error`, `request_includes_media_location` (pure); manifest test extended.
- **Verify:** `cargo test -p lightcraft-android-sys -- permissions && cargo test -p xtask -- manifest && cargo xtask
  android check && cargo xtask ci`
- **Depends on:** APK-11, APK-13  **Blocked by:** decision 2 (default: MediaStore route)  **Needs:** rust-target
  **Model:** opus — JNI and threading.
- **Done when:** tests pass.
- **Maintainer checks on a device:** allow all / allow selected / deny / deny twice then import (the Settings error
  shows) on API 33+, and allow/deny on an API 30–32 device.

#### APK-23 Import from the photo library (`file.importMediaLibrary`)
- **Goal:** import device photos by **path**, so the catalog and every reader stay unchanged.
- **Files:** new `crates/android-sys/src/media_store.rs`; changed `crates/ui-egui/src/lib.rs`
  (`Services::import_media: Option<…>`, `Services::prefer_copy_import: bool`), `crates/ui-egui/src/menus.rs`,
  `menubar.rs`, `crates/ui-egui/src/panels/import.rs` (bucket list as a folder source; Copy default; Add-mode note),
  `crates/android-host/src/services.rs`, `docs/parity-checklist.md` (row), `docs/android.md` (limitation).
- **Steps:** 1. On API 30+ an app holding the read permission opens shared media through ordinary paths
  (`/storage/emulated/<user>/DCIM/…`, FUSE). Query `MediaStore.Images.Media.EXTERNAL_CONTENT_URI` for `_data`,
  `RELATIVE_PATH`, `DISPLAY_NAME`, `BUCKET_DISPLAY_NAME`, `MIME_TYPE`, `SIZE`; read the column indexes once; each row
  runs inside `env.with_local_frame(16, …)`. The pure mapper builds the path from `_data` or volume root +
  `RELATIVE_PATH` + `DISPLAY_NAME`. 2. The command lists buckets (a bucket is a directory path) and feeds the chosen
  one to the existing folder-import source of the import dialog, exactly as `pick_folder` results are; no new dialog.
  3. On Android the dialog defaults to Copy (`prefer_copy_import`): photos added in place from DCIM are read-only to
  LightCraft (no write access to other apps' files; FUSE refuses `.xmp` there), so XMP write, rename, move and delete
  fail on them; Add stays available with a note. 4. The query runs off the UI thread: the ui-egui command starts a
  worker that holds `session.activity.start("import", "Reading the photo library", Cancel::No)` and calls the service
  there (so `import_media` is `Arc<dyn Fn() -> Result<Vec<Bucket>, String> + Send + Sync>`; the JNI side attaches the
  worker with `JavaVM::singleton()?.attach_current_thread`); the result comes back through a channel polled in
  `logic`. Raws MediaStore doesn't classify as images arrive through APK-25.
- **Tests:** `row_to_path_prefers_data_column`, `row_to_path_from_relative_path`, `rejects_path_outside_volume`
  (pure); headless `import_media_disabled_without_service`, `import_media_feeds_import_dialog` (fake service),
  `media_query_shows_activity_row` (a fake service that waits on a barrier: the activity row exists while it runs and
  is gone after), `prefer_copy_import_defaults_to_copy`, `add_mode_write_beside_original_gives_actionable_error`
  (through an injected writer that returns `io::ErrorKind::PermissionDenied`, or on the pure error-mapping function:
  this container runs tests as root, which ignores `0o555`, so a chmod-based test would pass without the change, see
  tests_move_to_share.rs:102).
- **Verify:** `cargo test -p lightcraft-android-sys -- media_store && cargo test -p lightcraft-ui-egui --
  import_media media_query copy_import add_mode && cargo xtask android check && cargo xtask parity && cargo xtask ci`
- **Depends on:** APK-22  **Needs:** rust-target  **Model:** opus — JNI cursor handling and import integration.
- **Done when:** tests pass; parity row added.
- **Maintainer checks on a device:** import a JPEG and a DNG folder; previews and edits work; GPS shows on the map;
  import a 20k-photo library (no local-reference abort, the activity row shows progress).

#### APK-24 File pickers on Android
- **Goal:** every command that needs a file dialog either works on a phone or says why it doesn't, and parity rows
  don't claim what the APK lacks.
- **Files:** new `crates/ui-egui/src/pick_browser.rs`; changed `crates/ui-egui/src/lib.rs` (`Services::pick_roots:
  Vec<(String, PathBuf)>`), `crates/ui-egui/src/pick.rs`, `crates/ui-egui/src/menus.rs` (`enabled()` / tooltips),
  `crates/android-host/src/services.rs`, `docs/android.md` (supported / unsupported table).
- **Steps:** 1. Audit every command that reaches a picker (`picker`, `pick_files`, `pick_folder`,
  `pick_preset_files`, `pick_tracklog`, `pick_keyword_list`, `pick_denoise_model`, `pick_model_file`,
  `pick_lightroom_catalog`, the `save_*` dialogs; lib.rs:239-282, pick.rs) and list it in `docs/android.md`.
  2. `pick_browser.rs`: an in-app egui `Picker` that browses only `pick_roots`, used when `Services::picker` is
  `None` and `pick_roots` is non-empty (desktop and web: empty, unchanged). Android roots: an `Inbox`
  (`<config>/inbox/`, filled by shares in APK-25) and the library; APK-27 adds the app's own export folders (files
  LightCraft created stay readable to it). Not `Download` or `Documents`: under scoped storage the photo permission
  opens only media files of other apps, so the presets (.xmp/.lcpreset/.zip), .gpx, keyword .txt, .onnx and .lrcat
  files a browser downloaded are hidden from `readdir` and can't be opened by path (reaching them needs SAF, which is
  Java, decision 2, or `MANAGE_EXTERNAL_STORAGE`). `docs/android.md`: "share the file to LightCraft from Files or the
  browser, then pick it from Inbox". 3. A command whose target can't live under those roots is disabled with a
  tooltip ("Not available on Android: …"). 4. The library-problem window (APK-17) offers Choose through this picker.
- **Tests:** headless `presets_import_uses_in_app_picker`, `in_app_picker_stays_inside_roots` (`..` and symlinks
  rejected), `disabled_without_picker_explains`.
- **Verify:** `cargo test -p lightcraft-ui-egui -- in_app_picker presets_import disabled_without_picker && cargo
  xtask parity && cargo xtask ci`
- **Depends on:** APK-17, APK-23  **Needs:** none  **Model:** opus — audit across many commands and a new UI
  component.
- **Done when:** tests pass; the docs table and parity notes match.
- **Maintainer checks on a device:** after APK-25, share a `.xmp` preset from Files to LightCraft, then File ▸ Import
  Presets ▸ Inbox imports it.

#### APK-25 "Open with LightCraft" and shares
- **Goal:** files sent from other apps (view, share one, share several) are imported when they are photos and land
  in the Inbox when they are presets, track logs, keyword lists or models, without blocking the UI thread.
- **Files:** new `crates/android-sys/src/{content.rs,intents.rs}`, `crates/android-host/src/incoming.rs`; changed
  `packaging/android/AndroidManifest.xml`, `xtask/src/android/manifest.rs` (test),
  `crates/android-host/src/{services.rs,app.rs}`, `docs/android.md`.
- **Steps:** 1. Intent filters, category DEFAULT: `ACTION_VIEW` (URI in `getData()`), `ACTION_SEND`
  (`Intent.EXTRA_STREAM` via `getParcelableExtra(String)`, deprecated but working; the typed overload only on API
  33+), `ACTION_SEND_MULTIPLE` (`getParcelableArrayListExtra`). Photo mimeTypes `image/*`, `image/x-adobe-dng`,
  `image/x-canon-cr2`, `image/x-canon-cr3`, `image/x-nikon-nef`, `image/x-sony-arw`, `application/octet-stream`; for
  SEND/SEND_MULTIPLE also the non-image types the pickers need: `application/zip`, `application/xml`,
  `application/rdf+xml`, `application/gpx+xml`, `text/plain`. A pure `IntentSpec` parse from (action, data, extras).
  2. `getIntent()` is read **once per android_main start** (NativeActivity has no `onNewIntent`, and
  `Activity.onNewIntent` doesn't call `setIntent`, so it never changes while the Activity lives); `getIntent()` and
  the `IntentSpec` parse run on android_main (fast). 3. The read grant ends with the Activity, so copy at once, zero
  unsafe, **on a worker thread** (`incoming.rs`): it attaches with `JavaVM::singleton()?.attach_current_thread` and
  holds `session.activity.start("import", "Receiving N files", Cancel::Flag(..))` for the whole job. Per URI: query
  `OpenableColumns.DISPLAY_NAME` / `SIZE` and the MIME type; a pure `route_received(name, mime) -> Route { Import,
  Inbox }` (photo or raw extension → Import, else Inbox); target `<library>/.incoming/<name>` or `<config>/inbox/<name>`
  where a pure `sanitize_display_name` keeps only the final component, drops `/`, `\`, NUL and leading dots, caps it at
  120 bytes on a char boundary, adds an extension from the MIME type when missing and de-duplicates; refuse files over
  2 GiB (by `SIZE`, and again while copying). `ContentResolver.openInputStream(uri)` → loop `InputStream.read(byte[1
  MiB])` + `env.get_byte_array_region` into the file through a pure `copy_chunks(reader, writer, cap, cancel)` that
  checks the cancel flag between chunks → `close()`, each step inside `with_local_frame`, Java exceptions mapped by
  APK-10's `user_message`. The worker hands the finished photo paths to the existing Copy import (the catalog stores
  the path under `Originals/`) through a channel polled in app.rs `logic`; Inbox files are only copied. `incoming.rs`
  takes the content reader as a trait, so the job is tested on x86_64 with a fake. 4. Errors (revoked, unreadable,
  half-read, too big, cancelled) become per-file import errors; partial files are removed. Limitation (decision 2):
  a second "Open with" / share while LightCraft runs only brings it to the front; the file is not received (no
  `onNewIntent` without Java). `docs/android.md` says so and explains the Inbox (APK-24).
- **Tests:** `same_uri_handled_once`, `view_reads_data`, `send_reads_extra_stream`, `send_multiple_reads_list`,
  `display_name_cannot_escape_dir`, `oversized_name_is_truncated_at_char_boundary`, `non_photo_goes_to_inbox`,
  `copy_removes_partial_file_on_read_error` (reader that fails halfway, temp dir), `copy_stops_at_size_cap`,
  `cancel_removes_partial_file` (pure, android-sys); `received_files_show_activity_row` (android-host, headless
  session with a fake content reader: the activity row exists while copying and is gone after); manifest test asserts
  the three filters and the non-image types.
- **Verify:** `cargo test -p lightcraft-android-sys -- content intents && cargo test -p lightcraft-android-host --
  received_files && cargo test -p xtask -- manifest && cargo xtask android check && cargo xtask ci`
- **Depends on:** APK-10, APK-23, APK-24  **Needs:** rust-target  **Model:** opus — URI grants, hostile names,
  threads and error paths.
- **Done when:** tests pass; the limitation and the Inbox are in `docs/android.md`.
- **Maintainer checks on a device:** share from Google Photos (one and several), from Files (Downloads) and from an
  app using a FileProvider; open a DNG from Files with "Open with"; all import; share a 1 GB file: the UI stays
  responsive and the activity row shows it; share a `.xmp` preset: it appears in the Inbox.

#### APK-26 Open links (`Services::open_url`)
- **Goal:** Help links and About open in the browser.
- **Files:** changed `crates/android-sys/src/intents.rs`, `crates/android-host/src/services.rs`.
- **Steps:** pure `IntentSpec::view(url)` (only `https://`), JNI `startActivity` through `with_activity` on the
  calling thread (it works off the Java main thread, so no hop and no waiting on one; the exception comes back as the
  result); `ActivityNotFoundException` → "No app can open links".
- **Tests:** `view_spec_rejects_non_https`, `view_spec_fields` (pure).
- **Verify:** `cargo test -p lightcraft-android-sys -- intents && cargo xtask android check && cargo xtask ci`
- **Depends on:** APK-25  **Needs:** rust-target  **Model:** sonnet — follows APK-25's JNI pattern.
- **Done when:** tests pass.  **Maintainer checks on a device:** Help ▸ a link opens the browser.

#### APK-27 Export to Pictures and share (`file.share`)
- **Goal:** exports land where other apps see them and can be shared.
- **Files:** new `crates/android-sys/src/storage.rs`; changed `crates/ui-egui/src/lib.rs` (`Services::share:
  Option<ShareFn>`, `Services::export_dir_for: Option<Box<dyn Fn(ExportKind) -> String>>`, which **replaces** APK-6's
  `default_export_dir` so `Services` keeps one export-folder injection; `None` = desktop unchanged), the callers of
  `default_export_dir`, `menus.rs`, `menubar.rs`, the export dialog's sidecar option,
  `crates/android-sys/src/{intents.rs,media_store.rs}`, `crates/android-host/src/{paths.rs,services.rs}`,
  `docs/parity-checklist.md`.
- **Steps:** 1. Folders from JNI `Environment.getExternalStoragePublicDirectory(DIRECTORY_PICTURES)` /
  `DIRECTORY_DOCUMENTS` + `/LightCraft` (right for secondary users and work profiles; pure fallback to
  `/storage/emulated/0/…` when JNI fails). A pure `export_dir_for(kind)`: images (JPEG, PNG, TIFF, DNG, HEIF, …) →
  Pictures; PDF, video and other non-image output → Documents. The exporter writes by path (APK-9 makes its temp names
  acceptable to MediaProvider). The same JNI call fills `Services::local_roots` (APK-6) with DCIM, Pictures and
  Download (readable in place through FUSE with the photo permission of APK-22), and the two export folders join
  `pick_roots` (APK-24). 2. XMP sidecars are never written into Pictures: the sidecar option is disabled there
  with an explanation, and a sidecar write that can't be placed gives an actionable error. 3. `file.share`: after
  export, look up the MediaStore content URI of each file (query by `RELATIVE_PATH` + `DISPLAY_NAME`, local frame per
  row) on a worker like APK-25's (attached thread, `session.activity.start("export", "Preparing to share",
  Cancel::Flag(..))`), then start `Intent.createChooser(ACTION_SEND / ACTION_SEND_MULTIPLE)` with `EXTRA_STREAM` and
  `FLAG_GRANT_READ_URI_PERMISSION` (no FileProvider: that is Java). Enabled only with the service.
- **Tests:** `export_dir_for_images_and_documents`, `share_spec_single_and_multiple`, `uri_lookup_selection_for_path`
  (pure); headless `share_disabled_without_service`, `share_calls_service_with_exported_paths`,
  `share_lookup_shows_activity_row`, `sidecar_into_pictures_is_refused_with_reason`,
  `export_without_folder_uses_export_dir_for` (APK-6's test, moved to the new field).
- **Verify:** `cargo test -p lightcraft-android-sys -- share export_dir && cargo test -p lightcraft-ui-egui -- share
  sidecar export_without_folder local_section && cargo xtask parity && cargo xtask ci`
- **Depends on:** APK-6, APK-9, APK-22, APK-24, APK-25  **Needs:** rust-target  **Model:** opus — MediaStore and
  grants.
- **Done when:** tests pass; parity row added.
- **Maintainer checks on a device:** export a JPEG and a TIFF to Pictures/LightCraft (no stray temp file left), a
  Contact Sheet PDF to Documents/LightCraft; share to a messenger; the photo arrives.

#### APK-28 System clipboard
- **Goal:** every copy (TextField, dialogs, egui's own shortcuts) and every paste reach other apps, without ever
  blocking the UI thread.
- **Files:** changed `crates/ui-egui/src/lib.rs` (`Services::clipboard: Option<ClipboardFns>` get/set),
  `crates/ui-egui/src/text_field.rs` (`Action::Paste`, :189), new `crates/android-sys/src/clipboard.rs`, changed
  `crates/android-host/src/{services.rs,app.rs}`.
- **Steps:** 1. JNI: `ClipboardManager.setPrimaryClip(ClipData.newPlainText(…))` and `getPrimaryClip()` →
  `getItemAt(0).coerceToText(ctx)`, called directly from android_main through `with_activity` (binder calls that work
  from any thread; the Android 10+ read restriction depends on window focus, not on the thread). No main-thread hop,
  no channel: android_main waiting on the Java main thread deadlocks whenever Java waits on android_main (Ground
  rules). Reuse APK-25's JNI template (local frame, error mapping). 2. Copy: the android-host App wrapper takes every
  `OutputCommand::CopyText` from `ctx.output` at the end of `ui` (precedent lib.rs:1388-1394) and sets the system
  clip, which covers TextField, `ctx.copy_text` (panels/dialogs.rs:397) and egui's Ctrl+C on a hardware keyboard
  (otherwise they stay in egui-winit's in-process clipboard). 3. Paste: `ClipboardFns { set, request_paste,
  take_paste }`. With the service, TextField's `Action::Paste` calls `request_paste()` (returns at once) instead of
  `ViewportCommand::RequestPaste`; `LightcraftApp::raw_input_hook` calls `take_paste() -> Option<String>` each frame
  and pushes `Event::Paste(text)` when it is there, with `request_repaint` while one is outstanding. The Android
  implementation reads the clip inside `request_paste` through `with_activity`, so the text is there on the next
  frame; the interface also allows a late answer. The android-host `raw_input_hook` replaces the text of any other
  `Event::Paste` (hardware Ctrl+V, filled by egui-winit from its in-process clipboard) with the system clip. Without
  the service, everything is as today.
- **Tests:** headless `text_field_copy_uses_clipboard_service`, `paste_reads_clipboard_service`,
  `dialog_copy_reaches_clipboard_service` (fake), `paste_never_blocks_ui_thread` (a fake service that answers late:
  the frame completes and the text arrives in a later frame, never a blocking `recv`).
- **Verify:** `cargo test -p lightcraft-ui-egui -- clipboard paste_never && cargo xtask android check && cargo xtask
  ci`
- **Depends on:** APK-25  **Needs:** rust-target  **Model:** opus — new JNI calls testable only on a device.
- **Done when:** tests pass.  **Maintainer checks on a device:** copy a keyword into another app and back.

## Phase 3: touch UI

The target behaviour (layout, gestures, tools) follows Lightroom on a phone and is specified in
[`android-touch-ui.md`](android-touch-ui.md); each Phase 3 card implements its part and says where it deviates.

#### APK-29 `ui.touch` control request
- **Goal:** tests can express pinch, long press and multi-finger pans (`ui.pointer` only synthesises mouse events,
  control.rs:268-286).
- **Files:** changed `crates/ui-egui/src/control.rs`, `crates/ui-egui/src/lib.rs` (synthetic clock in the input
  path), `docs/control-protocol.md`.
- **Steps:** `ui.touch {events: [{id, phase: start|move|end|cancel, x, y, t?}]}` (normalized coordinates as
  `ui.pointer`). Events with `t` (seconds) are queued and released in later frames; while the queue is non-empty the
  input hook sets `raw.time` from a synthetic clock that advances to each event's `t`, so egui's long-touch and
  double-tap timing (which compares `RawInput.time` between frames) sees real gaps. Each event becomes
  `egui::Event::Touch` plus the pointer events egui-winit emits for the primary finger. Headless tests step frames
  until the queue is empty.
- **Tests:** headless `touch_two_fingers_reports_multi_touch`, `touch_hold_reports_long_touch`.
- **Verify:** `cargo test -p lightcraft-ui-egui -- touch && cargo xtask ci`
- **Depends on:** APK-5  **Needs:** none  **Model:** sonnet — protocol addition; the timing mechanism is spelled out.
- **Done when:** documented and tested.

#### APK-30 Gestures under touch
- **Goal:** confirm and complete gestures. Pinch zoom (detail.rs:183), double tap = Fit/1:1 (`double_clicked`,
  detail.rs:1461), drag-pan when zoomed and context menus (detail.rs:900, grid) already exist.
- **Files:** changed `crates/ui-egui/src/panels/{detail.rs,grid.rs}` only where a test fails.
- **Steps:** headless tests through `ui.touch` for each gesture; add only what is missing (e.g. long press → grid
  cell context menu if egui's `long_touched` path doesn't reach it).
- **Tests:** `pinch_zooms_loupe`, `double_tap_toggles_fit`, `one_finger_pan_when_zoomed`,
  `long_press_opens_grid_menu`.
- **Verify:** `cargo test -p lightcraft-ui-egui -- gesture && cargo xtask ci`
- **Depends on:** APK-29  **Needs:** none  **Model:** sonnet — tests first, small fixes.
- **Done when:** all four tests pass.  **Maintainer checks on a device:** the four gestures by finger.

#### APK-30b Touch scrolling and multi-select in the grid
- **Goal:** a finger scrolls the grid instead of dragging photos, and several photos can be selected without
  modifier keys (APK-27's `ACTION_SEND_MULTIPLE` and batch export depend on it).
- **Files:** changed `crates/ui-egui/src/panels/grid.rs` (cell sense :608 inside the `ScrollArea` :245, selection
  :771-780, drag start :786-790), the filmstrip panel, `crates/ui-egui/src/state.rs` (`touch_mode`, select mode),
  `crates/ui-egui/src/menus.rs` (`view.back` arm for select mode), `crates/ui-egui/src/lib.rs` (`Services::touch:
  bool`).
- **Steps:** today every cell senses `click_and_drag`, so a swipe starts a photo drag-and-drop, and selection is
  modifier-only (shift = range, command = toggle, else replace). 1. `touch_mode` is on when `Services::touch` is set
  (Android) or an `egui::Event::Touch` was seen. 2. Under touch: a one-finger vertical swipe scrolls the grid and the
  filmstrip; a photo drag starts only after a long press. 3. Long press on a cell enters a select mode in which a tap
  toggles; a Done/Cancel bar exits it, and so does Back (Escape, APK-17) as a new `view.back` arm placed before the
  existing ones. 4. Mouse behaviour is unchanged.
- **Tests:** through `ui.touch`: `swipe_scrolls_grid_not_drags_photo`, `long_press_enters_select_mode_and_tap_toggles`,
  `back_leaves_select_mode`; `desktop_grid_drag_still_drags` (mouse).
- **Verify:** `cargo test -p lightcraft-ui-egui -- swipe_scrolls select_mode desktop_grid_drag && cargo xtask ci`
- **Depends on:** APK-17, APK-30  **Needs:** none  **Model:** opus — touch interaction design in a busy panel.
- **Done when:** tests pass; desktop drag and selection unchanged.
- **Maintainer checks on a device:** scroll a 1000-photo grid by finger; long press, tap three photos, share them.

#### APK-31 Safe-area and keyboard insets
- **Goal:** with targetSdk 36 (edge-to-edge enforced) nothing sits under the status bar, cutout, gesture bar or
  soft keyboard.
- **Files:** new `crates/android-sys/src/insets.rs`, `crates/android-host/src/insets.rs`; changed
  `crates/android-host/src/{app.rs,lifecycle.rs}`, `crates/ui-egui/src/lib.rs` and the top-level layout code that uses
  `ctx.screen_rect()`/`viewport_rect()`.
- **Steps:** 1. android-sys: `getWindow().getDecorView().getRootWindowInsets().getInsets(WindowInsets.Type
  .systemBars() | Type.displayCutout() | Type.ime())` (API 30+), run on the Java main thread via
  `run_on_java_main_thread` (views belong to it); the answer comes back over a channel that `logic()` polls with
  `try_recv` (never a blocking `recv`, Ground rules) into the cached `Insets`. 2. Refresh on `Resized`,
  `ScaleFactorChanged`, `Focused(true)`, and every 250 ms (`request_repaint_after`) while a TextField has focus.
  3. android-host `insets.rs` (pure): `insets_px_to_points(px, pixels_per_point)`; `raw_input_hook` sets
  `RawInput::safe_area_insets` (nothing in the repo sets it yet; main.rs:76 and web.rs:657 are precedents only for
  forwarding `raw_input_hook`; egui applies the insets at context.rs:427).
  4. ui-egui lays out its outer frame inside `ctx.content_rect()` and scrolls a focused TextField above the bottom
  inset. `content_rect()` comparison is not used (equals the window under edge-to-edge).
- **Tests:** `insets_px_to_points` (pure); headless `no_widget_in_inset_band` with fake insets at `--size 412x915
  --scale 2.625`, `focused_field_stays_above_keyboard_inset`.
- **Verify:** `cargo test -p lightcraft-android-host -- insets && cargo test -p lightcraft-ui-egui -- inset && cargo
  xtask android check && cargo xtask ci`
- **Depends on:** APK-18, APK-30b  **Needs:** rust-target  **Model:** opus — JNI on the main thread plus layout.
- **Done when:** tests pass.  **Maintainer checks on a device:** portrait and landscape on a phone with a cutout;
  keyboard open over the keyword field.

#### APK-32a Touch density
- **Goal:** one switch that makes hit areas finger-sized under touch, leaving the desktop layout untouched. egui's
  default `interact_size.y` is 18 pt and LightCraft's icon buttons, slider rows, checkboxes and grid badges are sized
  for a mouse, so 48 dp everywhere is a global density change, not per-panel tuning.
- **Files:** changed `crates/ui-egui/src/theme.rs` (density-scaled `spacing.interact_size`, `button_padding`, slider
  and checkbox hit areas), `crates/ui-egui/src/state.rs` (`touch_density`, set from `touch_mode` of APK-30b); new
  `crates/ui-egui/tests/fixtures/layout-1600x1000.json` (desktop fingerprint, text).
- **Steps:** 1. `touch_density` is on when `touch_mode` is (host `Services::touch` or a first `Event::Touch`).
  2. `theme` applies the scaled sizes only then. 3. Record the desktop fingerprint (widget rects at 1600×1000 from
  `app.widgets` / `ui.inspect`) through the CPU headless harness (`headless::Headless`, headless.rs:226), not
  `egui_kittest` snapshots (wgpu, no adapter here) or committed PNG baselines.
- **Tests:** `touch_density_scales_hit_sizes`, `desktop_layout_fingerprint_unchanged` (rects at 1600×1000 vs the
  committed JSON).
- **Verify:** `cargo test -p lightcraft-ui-egui -- touch_density desktop_layout && cargo xtask ci`
- **Depends on:** APK-30b, APK-31  **Needs:** none  **Model:** opus — theme-wide change behind one switch.
- **Done when:** tests pass; desktop fingerprint unchanged.

#### APK-32b Hit sizes ≥ 48 dp, panel by panel
- **Goal:** every interactive rect is at least 48 dp (Material) under touch density at 1280×800 pt.
- **Files:** new `crates/ui-egui/tests/fixtures/small-hit-allow.txt` (widget ids still below 48 dp); changed the
  panel modules named by the failing test, one panel family per commit (`APK-32b: <panel>`).
- **Steps:** 1. A test lists every widget below 48 pt at 1280×800 under touch density and fails for any id not in the
  allow-list; the first commit records today's list. 2. Each later commit fixes one panel family and shrinks the
  allow-list; the card is done when the list is empty or holds only ids with a written reason.
- **Tests:** `min_hit_size_48dp_at_tablet` (allow-list aware); `desktop_layout_fingerprint_unchanged` stays green.
- **Verify:** `cargo test -p lightcraft-ui-egui -- min_hit_size desktop_layout && cargo xtask ci`
- **Depends on:** APK-32a  **Needs:** none  **Model:** sonnet — repetitive, test-driven fixes.
- **Done when:** the allow-list is empty or every remaining line carries a reason.

#### APK-32c Tablet landscape breakpoint
- **Goal:** usable at 1280×800 pt with touch.
- **Files:** changed `crates/ui-egui/src/` layout modules named by the failing test.
- **Steps:** a width breakpoint in points; panels resizable by touch (drag handles sized by touch density).
- **Tests:** `tablet_breakpoint_layout` (headless rects at 1280×800: panels fit, no overlap);
  `desktop_layout_fingerprint_unchanged` stays green.
- **Verify:** `cargo test -p lightcraft-ui-egui -- tablet_breakpoint desktop_layout && cargo run -p lightcraft-cli --
  snapshot --demo --size 1280x800 --scale 2 -o target/android/tablet.png`, then Read the PNG.
- **Depends on:** APK-32b  **Needs:** none  **Model:** sonnet — layout tuning with structural tests.
- **Done when:** tests pass; desktop fingerprint unchanged.

#### APK-33 Phone portrait: side panels as sheets
- **Goal:** usable at 412×915 pt (2.625 px/pt); side panels become bottom sheets, no second UI.
- **Files:** changed `crates/ui-egui/src/` panel host modules, `crates/ui-egui/src/menus.rs` (`view.back` arm);
  new `crates/ui-egui/src/sheet.rs`.
- **Steps:** below the phone breakpoint, Library/Develop panels render inside a sheet with a drag handle; `view.back`
  (APK-17) closes the sheet first (a new arm at the front of the cascade, desktop unaffected).
- **Tests:** `side_panels_become_sheet_below_breakpoint`, `back_closes_sheet_first`, `min_hit_size_48dp_at_phone`
  (APK-32b's allow-list check at 412×915 under touch density; new small widgets are fixed, not allow-listed);
  `desktop_layout_fingerprint_unchanged` stays green.
- **Verify:** `cargo test -p lightcraft-ui-egui -- sheet min_hit_size desktop_layout && cargo run -p lightcraft-cli --
  snapshot --demo --size 412x915 --scale 2.625 -o target/android/phone.png`, then Read the PNG.
- **Depends on:** APK-17, APK-32c  **Needs:** none  **Model:** opus — UI structure change across panels.
- **Done when:** tests pass; desktop fingerprint unchanged.

#### APK-34 Menu bar → overflow menu
- **Goal:** all menu commands reachable on a phone.
- **Files:** changed `crates/ui-egui/src/menubar.rs`.
- **Steps:** below the phone breakpoint the menu bar collapses to one overflow button listing the same menus
  (same ids, same `enabled()`).
- **Tests:** `overflow_lists_every_menu_command` (compares with the desktop menu tree).
- **Verify:** `cargo test -p lightcraft-ui-egui -- overflow && cargo xtask parity && cargo xtask ci`
- **Depends on:** APK-33  **Needs:** none  **Model:** sonnet — reuses the menu tree.
- **Done when:** tests pass.

#### APK-35 Soft keyboard and text input spike
- **Goal:** the facts the maintainer needs for a go/no-go on text input under NativeActivity (the verdict itself
  needs a device).
- **Files:** changed `docs/android-port.md` (result), `crates/android-sys/src/dialog.rs` (text-input variant),
  `crates/ui-egui/src/text_field.rs` only if needed.
- **Steps:** 1. Confirm the keyboard shows and hides with `TextField` focus (egui IME output → winit
  `set_ime_allowed`). 2. Document what works: Latin key events; what doesn't: composition (CJK), autocorrect, swipe
  (android-activity `text_input_state`/`set_ime_editor_info` are no-ops in native-activity mode). 3. Fallback
  without Java: an `AlertDialog` with an `EditText`, built through JNI on the Java main thread, `null` button
  listeners, `isShowing()` / `getText()` polled each frame through `with_activity` on the calling thread (no waiting
  on the Java main thread). 4. Feed decision 8.
- **Tests:** headless `focus_requests_ime` (IME output set while a TextField has focus).
- **Verify:** `cargo test -p lightcraft-ui-egui -- ime && cargo xtask android check && cargo xtask ci`
- **Depends on:** APK-19, APK-31  **Needs:** rust-target (device for the result)  **Model:** opus — open-ended spike.
- **Done when:** the doc lists what the code supports, the expected limits with their sources (winit
  `android/mod.rs:916-926`, android-activity's native-activity no-ops) and unticked Device checklist lines; its
  go/no-go field reads "pending maintainer" until the device check. An agent never writes a result it didn't observe.
- **Maintainer checks on a device:** type ASCII, an umlaut and Chinese with Gboard into the keyword field.

## Phase 4: quality and release

#### APK-36 Android release profile and size
- **Goal:** a small, unwind-safe release `.so`.
- **Files:** changed root `Cargo.toml` (`[profile.android]` inherits `dist`, `strip = true`, `panic = "unwind"`),
  `xtask/src/android/so.rs` (`--release` uses `--profile android`).
- **Steps:** 1. Add the profile block. 2. `so.rs`: `--release` → `--profile android` and the output path
  `target/aarch64-linux-android/android/`. 3. `docs/android.md`: where CI records the sizes.
- **Tests:** `release_uses_android_profile` (args builder and output path).
- **Verify:** `cargo test -p xtask -- android:: && cargo xtask ci`
- **Depends on:** APK-12  **Needs:** none (size measured in CI)  **Model:** sonnet — config.
- **Done when:** tests pass; Device checklist line "CI artefact sizes recorded".

#### APK-37 Signing without secrets in the repo
- **Goal:** release signing reads only environment variables; key files can't be committed.
- **Files:** changed `xtask/src/assets.rs` (fail on tracked `.jks`, `.keystore`, `.p12`, `keystore.properties`),
  `xtask/src/android/signing.rs` (release signing; apk.rs only chooses debug or release).
- **Steps:** 1. `assets.rs`: add the extensions/names to the rejected list with a message pointing at the env
  variables. 2. `signing.rs`: release signing reads `ANDROID_KEYSTORE_B64`, `ANDROID_KEYSTORE_PASSWORD`,
  `ANDROID_KEY_ALIAS`, `ANDROID_KEY_PASSWORD`; decodes the keystore to a temp file outside the repo (`$RUNNER_TEMP`
  or the OS temp dir), removed at the end even on error; `apksigner --ks-pass env:ANDROID_KEYSTORE_PASSWORD
  --key-pass env:ANDROID_KEY_PASSWORD` (never passwords on argv); a missing variable is a clear error.
- **Tests:** `assets_rejects_keystore_files`, `release_signing_requires_env`, `signing_args_never_contain_passwords`.
- **Verify:** `cargo test -p xtask -- assets signing && cargo xtask assets`
- **Depends on:** APK-15  **Needs:** none  **Model:** sonnet — checks and env handling.
- **Done when:** tests pass.

#### APK-38 Embedded font licences in the app
- **Goal:** an APK built with `CRAFT_FONTS_DIR` carries each embedded face's OFL text (APKs have no user-visible
  files).
- **Files:** changed `crates/engine/build.rs`, `crates/engine/src/fonts.rs` (`CraftFont::licence: &'static str`),
  `crates/ui-egui/src/licences.rs` (`own_entries`, :69-81).
- **Steps:** build.rs embeds each manifest font's licence file next to its bytes; `own_entries` lists one entry per
  embedded family (name, OFL text).
- **Tests:** `embedded_fonts_have_licence_entries` (`lightcraft_testkit::skip!` without `CRAFT_FONTS_DIR`); an
  always-run test that an empty `CRAFT_FONTS` adds no entries.
- **Verify:** `cargo test -p lightcraft-ui-egui -- licences && cargo xtask ci`
- **Depends on:** APK-5  **Needs:** none  **Model:** sonnet — build-script data plus a list.
- **Done when:** tests pass (the fonts test runs where craft-fonts is cloned).

#### APK-39 AAB builder (`cargo xtask android aab`)
- **Goal:** a valid, signed Android App Bundle from the same inputs as the APK.
- **Files:** new `xtask/src/android/aab.rs`; changed `xtask/src/android/{mod.rs,signing.rs}`.
- **Steps:** 1. `aapt2 link --proto-format` (same res and manifest as APK-15) → restructure with `zip_lib.rs` into
  `base.zip` with `ZipWriter::raw_copy_file_rename` (no recompression): `manifest/AndroidManifest.xml`, `res/`,
  `resources.pb`, `lib/arm64-v8a/liblightcraft_android.so`.
  2. bundletool jar pinned by version and sha256 from maven.google.com
  (`com/android/tools/build/bundletool/<v>/bundletool-<v>.jar`), cached under `target/android/tools/`.
  3. `java -jar bundletool.jar build-bundle --modules=base.zip --output=app.aab` → `validate`. 4. Sign with
  `jarsigner -storepass:env ANDROID_KEYSTORE_PASSWORD -keypass:env ANDROID_KEY_PASSWORD` (apksigner doesn't sign
  AABs), keystore from APK-37's temp file; the jarsigner argument builder lives in `signing.rs` (aab.rs never reaches
  into apk.rs). 5. Check: `bundletool build-apks --mode=universal` then `zipalign -c -P 16`
  on the extracted APK.
- **Tests:** `proto_link_args`, `base_module_layout` (zip built from fake aapt2 output, entry names checked),
  `bundletool_args`, `aab_signed_with_jarsigner_env_passwords` (pure).
- **Verify:** `cargo test -p xtask -- aab`; here also the bundletool download and `bundletool version` (maven.google.com
  is reachable).
- **Depends on:** APK-15, APK-37  **Needs:** none for the tests (sdk for a real bundle)  **Model:** sonnet — every
  step named, pure argument builders.
- **Done when:** tests pass.

#### APK-40 Android release workflow
- **Goal:** signed AAB and APK from CI. The main `release.yml` calls the shared
  `storytold/craft-common-actions/.github/workflows/release.yml@main`, so Android gets its own workflow.
- **Files:** new `.github/workflows/android-release.yml`; changed `docs/android.md`.
- **Steps:** `workflow_dispatch` + release tags; build release `.so` (APK-36), `cargo xtask android apk --release`
  and `cargo xtask android aab` with secrets as env (APK-37), keystore decoded to `$RUNNER_TEMP` and deleted in an
  `if: always()` step; 16 KB checks (APK-12/15/39); upload to the draft release. With craft-fonts (decision 11:
  CJK faces optional for APK size) the build depends on APK-38.
- **Tests:** the run itself (maintainer or agent per decision 0).
- **Verify:** YAML parse + `actionlint` if available; `cargo test -p xtask -- aab apk`.
- **Depends on:** APK-36, APK-38, APK-39  **Blocked by:** decisions 1, 5, 6  **Needs:** ci  **Model:** sonnet —
  workflow over existing commands.
- **Done when:** the workflow is committed and checked here; Device checklist line "release dispatch produced
  signed APK and AAB".
- **Maintainer checks on a device:** install the signed release APK; it opens and imports.

#### APK-41 Performance on a phone
- **Goal:** desktop budgets (slider ≤ 16 ms draft, loupe ≤ 60 ms on ~2.5 MP) measured on a mid-range phone.
- **Files:** new `docs/showcase/android-perf.jsonl` (control script), changed `docs/android.md` (results table).
- **Steps:** a script that opens the demo library, moves sliders and reads `ui.inspect` → `perf`, run through the
  debug control server (APK-21) with `debug.lightcraft.profile=1`.
- **Tests:** the script runs headless on desktop.
- **Verify:** `cargo run -p lightcraft-cli -- snapshot --demo --script docs/showcase/android-perf.jsonl -o
  target/android/perf.png`
- **Depends on:** APK-21  **Needs:** device for numbers  **Model:** sonnet — script.
- **Done when:** the script is committed; numbers come from the maintainer.
- **Maintainer checks on a device:** run the script, paste the numbers into `docs/android.md`.

#### APK-42 Store metadata
- **Goal:** listing texts and images from LightCraft itself.
- **Files:** new `fastlane/metadata/android/en-US/{title.txt,short_description.txt,full_description.txt,
  changelogs/<versionCode>.txt,images/icon.png,images/featureGraphic.png,images/phoneScreenshots/*.png}`,
  `docs/showcase/store-tour.jsonl`, `xtask/src/android/listing.rs`; changed `assets/ATTRIBUTION.md`.
- **Steps:** screenshots with `cargo run -p lightcraft-cli -- snapshot --demo --script docs/showcase/store-tour.jsonl
  --size 360x640 --scale 3` (1080×1920 px, procedural photos only, never Adobe material); 512 px icon and a
  1024×500 feature graphic rendered from the app icon SVG with resvg (APK-14's pipeline; original work, ATTRIBUTION
  rows); name per decision 1, no ArtCraft name; `<versionCode>` from APK-13's `version_code`.
- **Tests:** `listing_files_exist_and_fit_limits` (title ≤ 30, short ≤ 80, full ≤ 4000 chars; PNG headers:
  screenshots 320–3840 px with long side ≤ 2 × short side, icon 512×512, feature graphic 1024×500).
- **Verify:** `cargo test -p xtask -- listing && cargo xtask assets`
- **Depends on:** APK-13, APK-14, APK-33  **Blocked by:** decision 1  **Needs:** none  **Model:** sonnet — content
  plus checks.
- **Done when:** tests and assets check pass.

#### APK-43 F-Droid recipe and store checklist
- **Goal:** what the maintainer needs to submit.
- **Files:** new `docs/android-distribution.md`.
- **Steps:** draft fdroiddata recipe (rustup + targets in `prebuild`, pinned NDK, craft-fonts as a pinned srclib,
  `--remap-path-prefix` + `SOURCE_DATE_EPOCH`, versionCode from APK-13), anti-feature review (INTERNET for model
  downloads: NonFreeNet?), Play checklist (targetSdk rule as of the day — verify on Play's "Target API level
  requirements" page, 16 KB pages, AAB, Play App Signing, privacy policy, Data safety, Photo & Video permissions
  declaration).
- **Tests:** none (docs).  **Verify:** `cargo xtask parity`
- **Depends on:** APK-40, APK-42  **Needs:** none  **Model:** sonnet — documentation.
- **Done when:** the doc exists.  **Maintainer checks:** submit the F-Droid RFP/MR and the Play console forms.

## Decisions for the maintainer

Agents use the default until the maintainer decides (execution-plan §7).

| # | Decision | Default an agent uses | Blocks |
|---|---|---|---|
| 0 | **CI access for agents.** Enable Actions on `0x4E52/lightcraft-apk` and give the container a fine-grained token (actions read/write, contents write) as `GH_TOKEN`. Without it agents commit workflows and the maintainer starts and reads the runs. | maintainer runs CI | CI lines of APK-16, 18, 19, 21, 36, 40 (never a card's code) |
| 1 | **App name and application id.** "LightCraft" is allowed (the brand licence covers only the ArtCraft marks), but a distinct store name avoids confusion with official builds. Never `ai.storyteller.lightcraft`. Each id segment must start with a letter, so the owner `0x4E52` can't be used as is (nor `_0x4e52`); `x4e52` would be someone else's GitHub namespace. | id `io.github.n0x4e52.lightcraft` (`.debug` suffix for debug), label "LightCraft" | APK-13, 40, 42 |
| 2 | **Java shims.** Without Java: MediaStore + "Open with", no SAF picker (`onActivityResult`; in-app picker, APK-24), no `onNewIntent` (a second "Open with" while running is ignored), no IME composition, no foreground service for background jobs. A few small Java classes (hasCode=true) would fix each, bending "pure Rust in the product"; a Java SAF shim would also remove the Inbox step for presets, GPX, keyword lists and models (APK-24). | no Java | APK-22, 24, 25, 35 |
| 3 | **minSdk / targetSdk.** minSdk 30: shared media by path (FUSE), settled scoped storage, `WindowInsets.Type` API; devices launching with Android 10+ (64-bit) ship Vulkan 1.1 (upgraded devices may not: GLES fallback). targetSdk tracks Play's rule: 36 for new apps and updates since 2026-08-31 (re-check on the day). | minSdk 30, targetSdk 36 (android-36 platform, build-tools ≥ 36) | APK-13, 15 |
| 4 | **Repository branding.** The README header, `docs/brand/` and desktop packaging metadata still name ArtCraft; fine while mirroring upstream, to be replaced before this fork publishes builds. | unchanged | — |
| 5 | **ABIs.** arm64-v8a for release; x86_64 only for the CI emulator. | as stated | APK-12, 16, 40 |
| 6 | **Packaging route.** (a) SDK build-tools driven by xtask (aapt2 → zip → zipalign → apksigner; bundletool + jarsigner for AAB): no Gradle, no wrapper jar. (b) A declarative Gradle/AGP project: familiar to F-Droid but heavier, needs the `gradle-wrapper.jar` blob and pinned AGP/Gradle/JDK 17+. | (a) | APK-15, 39, 40 |
| 7 | **System destroy and re-entry.** When the Activity is destroyed while the process lives ("Don't keep activities", a missed configChange, removal from recents), the host detects it (APK-18), leaves the event loop and exits the process, so the next launch is fresh. Option "restart": before exit, `startActivity(Intent.makeRestartActivityTask(launchComponent))` via JNI (no Java class) so one tap reopens when the destroy came from a relaunch. | exit when the destroy is detected; start guard exits as backstop | APK-18 |
| 8 | **Text input.** After APK-35: NativeActivity's limits, GameActivity (full IME, but C++ via `cc`), or a small Java InputConnection (decision 2). | NativeActivity + dialog fallback | phase 3 follow-ups |
| 9 | **Library location.** Internal storage (private, `allowBackup="false"`, **deleted on uninstall or "Clear data"** with every edit and Copy-imported original) vs app-specific external or `Pictures/` (survives less, visible to others). | internal, with a first-start notice saying uninstall deletes the library and how to back it up | APK-11b |
| 10 | **SAM on Android.** The SAM worker model holds several GB (engine/src/segment/worker.rs:9). | ship `sam`; the command is enabled only when MemTotal ≥ 8 GB, otherwise disabled with an explanation. *Implemented as 7 GiB MemTotal (the agent's reading of "8 GB": an 8 GB phone reads 7.2 to 7.9 GiB; the text says 8 GB): maintainer to confirm (APK-11b).* | APK-11b |
| 11 | **craft-fonts in the APK.** CJK faces add tens of MB. | release builds embed them (as desktop) with their licences (APK-38) | APK-40 |

## Risks

| Risk | Mitigation |
|---|---|
| A system-initiated Activity destroy while the process lives: `onDestroy` waits for `android_main`, which winit never ends (ANR) | full `configChanges` + `singleTask` + Back → `moveTaskToBack` make it rare; when it happens the host sees `isDestroyed()`/`isFinishing()`, flushes, leaves the loop and exits (APK-18, decision 7); maintainer check with "Don't keep activities"; report upstream to winit |
| Cached-apps freezer: a backgrounded process is frozen within seconds, so long imports/exports pause and their `TaskGuard` row looks stalled | `docs/android.md` explains it; jobs resume when the app returns; optional follow-up: `AndroidApp::set_window_flags(KEEP_SCREEN_ON)` while a long job runs, behind a Services hook (a foreground service needs Java, decision 2) |
| Low-memory killer ends the process (a native kill, not catchable) | phone budget from `/proc/meminfo` via `memory::set_budget` and injected `set_total_ram` so export/merge fit checks run (APK-11), trim when backgrounded and on `memory_warning` (APK-18; `onTrimMemory` isn't forwarded by NativeActivity), flush on focus loss, SAM gated by RAM (APK-11b) |
| ANR or deadlock from the Java/native handshake: Java lifecycle and surface callbacks wait for android_main | quick start with the session opened on a worker (APK-11), full flush on focus loss and only dirty state in `suspended` (APK-18), android_main never waits on the Java main thread and `with_activity` is not re-entrant (Ground rules, APK-10), long JNI copies and queries on attached workers with a `TaskGuard` (APK-23, 25, 27) |
| JNI local-reference overflow aborts on large libraries | `with_local_frame` per iteration (APK-10 rule, APK-23/25/27) and a 20k-photo device check |
| No IME composition under NativeActivity (CJK, autocorrect, swipe) | APK-35 spike, dialog fallback, decision 8 |
| GPU drivers vary (Vulkan bugs, crashes in device creation) | wgpu Vulkan with GLES compiled in, start markers so the next launch tries GL first, compute-GPU crash sentinel, CPU develop pipeline, failure dialog that waits (APK-19) |
| Shared storage rules (MIME per folder, read-only DCIM originals, other apps' non-media files invisible) | extension-keeping temp names (APK-9), image/document export folders, no sidecars in Pictures (APK-27), Copy import default (APK-23), Inbox fed by shares (APK-24, 25) |
| The container can't link, package, run Android code or start CI | CI workflows (APK-16, APK-40) run by the maintainer until decision 0; device checks listed for the maintainer, never claimed by agents |
| TalkBack unsupported (eframe `accesskit` requires GameActivity) | known limitation in `docs/android.md`; revisit with decision 8 |
| Upstream merges touch the same files | Android code stays in its own crates; shared-crate changes are injections and the APK-7 move |

## Revision history

- 2026-10-17: first version: licence check, in-app licences, brand removal done; phases 0–4 planned.
- 2026-10-17 (review round 1): checked against android-activity 0.6.1, winit 0.30.13, eframe 0.36.2, jni 0.22.4 and
  this container. Entry point moved to a tiny `apps/lightcraft-android` cdylib with the host in
  `crates/android-host`; start guard, Back and Quit for the one-event-loop limit; lifecycle through a
  `create_native` wrapper; photo import by path (minSdk 30) and copy-on-receive for "Open with"; export to Pictures
  and share via MediaStore URIs; insets, clipboard, `ui.touch`, debug control server; container limits and the CI
  workflow; every task rewritten as a card (APK-4 … APK-38) with files, tests, verification, needs and model.
- 2026-10-17 (review round 2): cards renumbered APK-4 … APK-43 with real dependencies (no card waits on CI or a
  device; decision 0 for agent CI access). New cards: diagnostics setters with a `lightcraft-profile` crate (APK-8),
  extension-keeping temp files for scoped storage (APK-9), file pickers (APK-24), embedded font licences (APK-38),
  AAB builder split from the release workflow (APK-39/40). System destroy detected through `isDestroyed()` and ended
  with a process exit; flush and trim run synchronously in the lifecycle wrapper over a shared `HostState`; renderer
  failure via start markers, `exit_reason` and a dialog that waits before `android_main` returns; host eframe::App
  wrapper, writers and a phone memory budget in APK-11; content URIs copied through JNI streams with sanitised names;
  ACTION_SEND/SEND_MULTIPLE; insets through JNI `WindowInsets`; control server behind a property and a token;
  targetSdk 36 with an unconditional back opt-out, NoActionBar theme, full `configChanges`, `extractNativeLibs`,
  versionCode; zip crate for stored 16 KB-aligned `.so`; Android-only deps target-gated; wasm target and background
  `ci` in the container rules; snapshot sizes in points; `cargo test` filters after `--`.
- 2026-10-17 (review round 3): checked against android-activity's Java/native handshake (glue.rs), jni 0.22.4's
  errors and top-frame checks, zip 7.2.0's features and this container (disk 11 GB free, `target/` 19 GB, root user,
  wasm32 installed). New ground rule: android_main never blocks on the Java main thread; `with_activity` not
  re-entrant. APK-11 opens the session on a worker behind a loading frame and injects `set_total_ram`; new cards
  APK-11b (library notice, SAM by RAM), APK-30b (touch scrolling, multi-select) and APK-32a/b/c (touch density,
  48 dp allow-list, breakpoint) replace APK-32. Flush split into `flush_periodic` / `flush_leaving`; lifecycle split
  into a pure `lifecycle_core` and an android-only forwarder, full flush on focus loss, trim when backgrounded; the
  entry crate exits whenever `run()` returns (`Exit { Destroyed, Closed }`). Back becomes Escape in the host; Quit
  buttons gated everywhere. Inbox + share filters instead of browsing `Download`; content copies, media queries and
  share lookups on attached workers with a `TaskGuard`; clipboard without main-thread waits and covering every
  `CopyText`; `local_roots` for the Local sidebar; one export-folder injection. Packaging: label and icons in the
  manifest, `version_code` rules, NEEDED-library check, debug keystore and shared `signing.rs`, zip without
  features. Tests no longer touch process-wide statics; renderer order, forced backend and log spec are parameters
  (APK-19 no longer needs APK-20); `run.py --port/--token`; graded disk rule; corrected code references.
- 2026-10-17 (APK-4): environment probed; the facts in "Executing in this container" are now measured (Android need
  0.6 GB, network, targets, `gh` unusable for agents, `ci` duration) and the disk rule names the traps found:
  a profile change doubles `deps`, `cargo clean --target` wipes more than the Android output.
- 2026-10-17 (after APK-4, maintainer decision): fast gate per task here (fmt, clippy and tests of the touched crates,
  the xtask checks the change touches), the full `cargo xtask ci` on GitHub for every push (`ci.yml`), red runs
  fixed forward. Review of APK-4 applied: agents can read CI runs through `gh api` now that Actions is enabled
  (starting one still needs decision 0); the disk rule is a precondition, cleaning only while no cargo runs.
- 2026-10-17 (APK-5): `cargo xtask android check` (`xtask/src/android/`) lints the workspace crates the APK links (derived
  from `cargo metadata`, normal dependencies only) for `aarch64-linux-android` and runs in `ci` after `wasm`; `ci`
  steps return a `StepOutcome`, so a missing target shows as `skipped` with its reason in the report and summary.
- 2026-10-17 (APK-6): `engine::config::use_dir` (set once, wins over `LIGHTCRAFT_CONFIG_DIR`, portable mode and the
  platform folder) and `config::scratch_dir` (`<settings>/cache` for a host that injected a folder, else
  `temp_dir()`); `Services::default_export_dir` and `Services::local_roots` replace the `HOME`-based export folder
  and Local list when set. Audit: of the production `temp_dir()` uses only `profile.import`'s no-library copy was
  reachable on Android (now `scratch_dir`); the MCP server's and the desktop panic log's stay.
- 2026-10-17 (APK-7): `lightcraft_ui_egui::host` (`prefs`, `session_open`, `log_file`, `flush`, `gpu_markers`;
  native only) and `lightcraft_engine::gpu::window` (`install_error_handler`, `check_pipeline`) hold what
  `apps/lightcraft` had and the Android host needs, every folder a parameter; `flush_periodic` never ends an open
  interaction, `flush_leaving` commits it first (desktop `on_exit` calls it before `close_library`);
  `log_file::install_with(spec)` and `last_error()`; the GPU crash notice takes the host's `how_to_force` sentence.
  `docs/showcase/run.py` takes `--port` and `--token`.
- 2026-10-17 (APK-8): new L0 crate `lightcraft-profile` (`enabled()`, `set_enabled(bool)`: an `AtomicU8`
  unset/off/on, the variable read once while unset, the setter always wins) replaces the nine `LIGHTCRAFT_PROFILE`
  reads and three `OnceLock`s in catalog, pipeline, gpu, denoise, denoise-core, faces and engine;
  `gpu::backend::set_choice` fills the process's backend choice before first use and returns `Err(the choice in
  force)` after it was read.
- 2026-10-17 (APK-9): `safe_file` temp names keep the target's extension (`.lightcraft-<pid>-<n>-tmp.<ext>`, `.tmp`
  without one or with one over 16 bytes) so MediaProvider accepts them in `Pictures/`; the other `.tmp` writers
  (catalog store, prefs, camera profiles, lightroom archive, preview cache, denoise models) write into app-private
  folders.
- 2026-10-17 (APK-10): new crates `apps/lightcraft-android` (cdylib; `android_main` catches a panic, logs the
  outcome and exits the process with `exit_code`), `crates/android-host` (`run` with the start guard `try_start`,
  `Exit { Destroyed, Closed }`; until APK-11 the loop only drains input and waits for the destroy) and
  `crates/android-sys` (`jni_env::with_activity`, not re-entrant through `ReentryGuard`; `Error::Java { class,
  message }` from caught exceptions and `SecurityViolation`; `user_message`). Registered in `layers::TABLE`
  (android-sys L0 standalone, the other two exempt) and `licences::SHIPPED`; CLAUDE.md and AGENTS.md name both
  unsafe crates.
- 2026-10-17 (APK-11): `android-host` gains `paths` (`files/config`, `library`, `logs`, `inbox`; the settings folder
  through `config::use_dir`), `memory` (`mem_total`, `budget_for`), `loader` (the session opened on a worker, polled
  with `try_recv`; a worker that stops without an answer is an error), `services` (`base_services`: writers, PNG,
  log file), `state` (`HostState` with `flush_leaving`/`flush_dirty`/`close`; a failed open shows the library
  problem window) and the Android-only eframe `app` (loading frame, then the UI, `flush_periodic` every 30 s);
  `run` does only quick work before `eframe::run_native`. Engine: `memory::total_ram()` is public and consults
  `memory::set_total_ram` first, and `memory::default_budget()` ("Automatic") the host's `set_default_budget` (the
  phone budget; before, Settings ▸ Performance ▸ Memory set back to Automatic or a cleared `memory_mb` applied a
  quarter of RAM up to 1.5 GiB, enough for the low-memory killer). The loading label stays until the app's fonts
  are in. `target_os = "linux"` audit: besides the RAM probe Android skips the
  removable-device mount points (engine `devices.rs`, no card readers there), the NixOS GPU hint, the Linux
  model-folder opener and the `~/.local/share/icc` hint, none of which applies on a phone. Found and given to
  APK-11b: Try Again in the library problem window opens the library on android_main, and export's and merge's
  memory messages say "this computer".
- 2026-10-17 (APK-11b): `Services` gains `library_notice` (shown once through the notices panel when a library is
  open and no problem window is up, in its own "Your Library" window and not among the settings-file problems;
  `UiState::library_notice_shown` is set when the user presses OK, so a kill before that tells it again, and is saved with `ui.json`; the text goes through `i18n::tr` when shown, so a `locales/*.json` entry for it is used), `min_ram_for_sam`
  (the Object and Describe tiles and the component menu entries are disabled with a tooltip, `start_object` /
  `start_describe` start nothing, the SAM dialog shows the reason and confirms nothing, and `run_typed` refuses
  `mask.add` / `mask.addComponent` of kind object or prompt (even with a precomputed `seg`), `mask.objectPoint`, `mask.refineDetail`,
  `segment.prepare` and `segment.model.download` with the reason, which the command lists report as disabled; the
  reason text names 8 GB, so another limit changes it with the limit) and `reopen_library` (Try Again
  hands the path to the host). The Android host fills them: the notice text, `SAM_MIN_RAM` (7 GiB, the agent's reading of decision 10's "8 GB", left for the maintainer to confirm; a literal
  8 GiB would exclude 8 GB phones) and a `Reopen` request that `HostApp` answers by closing the
  problem session and starting a new `Loader` behind the loading frame, with the settings as they are. Engine:
  `memory::set_handheld()` (called by `run`) makes `memory::this_device()` say "this device"; export's
  `too_big_for_memory` takes the noun and the merge note has a second catalog key (`locales/*-formats.json`, all 13
  languages). "Opening library…", the notice text, "Needs a device with 8 GB of memory", the SAM dialog's body and its title
  "Object and Describe Masks" have no `locales/*.json` entry yet (next translation update). Try Again carries no
  pending command-line import (the phone has none). The device checks in the card stay unticked.
- 2026-10-17 (APK-12): `cargo xtask android so` (`xtask/src/android/{sdk,so,elf}.rs`). `sdk.rs` finds the NDK
  (`ANDROID_NDK_HOME`, `ANDROID_NDK_LATEST_HOME`, else the newest `$ANDROID_HOME/ndk/<version>`, with
  `ANDROID_SDK_ROOT` as the older name of `ANDROID_HOME`) and reads its revision from `source.properties`; a missing
  SDK or NDK is an error naming the variable to set. `so.rs` points `CARGO_TARGET_<TRIPLE>_LINKER` at
  `<ndk>/toolchains/llvm/prebuilt/<host>/bin/<triple>30-clang` (`MIN_SDK` 30, decision 3), appends
  `-C link-arg=-Wl,-z,max-page-size=16384` to `RUSTFLAGS` for NDKs older than r28, builds `-p lightcraft-android` in
  the dev profile for each `--abi` (arm64-v8a by default, `x86_64` for the CI emulator) and copies the result. `elf.rs`
  runs the NDK's `llvm-readelf -lW -d --dyn-syms` and fails unless every `LOAD` is aligned to a power of two of at
  least 0x4000, `android_main` and `ANativeActivity_onCreate` are defined global symbols, and every `NEEDED` entry is
  libc, libm, libdl, liblog, libandroid, libnativewindow, libvulkan, libEGL or libGLESv2. Not run here (no NDK:
  `dl.google.com` is refused); the APK-16 workflow runs it and is the first real link. A release profile build is not
  part of this card (APK-40).
- 2026-10-17 (APK-13): `packaging/android/AndroidManifest.xml` (template with `${APPLICATION_ID}`, `${APP_LABEL}`,
  `${VERSION_CODE}`, `${VERSION_NAME}`, `${MIN_SDK}`, `${TARGET_SDK}`) and `xtask/src/android/manifest.rs`: `fill`,
  `version_code` and a small `cargo xtask android manifest [--release]` step that writes the filled file to
  `target/android/AndroidManifest.xml` (APK-15 calls `fill` directly; the step makes the module reachable and lets
  the maintainer look at the result). The template adds `package`, `versionCode`, `versionName` and `<uses-sdk>`
  (`TARGET_SDK` 36 is a new constant next to `MIN_SDK` in `sdk.rs`). `version_code`: alpha, beta and rc take N in
  1..=3 (the card names the range for rc only; the same slots keep the stages from overlapping), a number with a
  leading zero is an error, and a code over Google Play's 2_100_000_000 is an error. The label is escaped as an XML
  attribute. Tests parse the filled template with a small attribute scanner (xtask has no XML dependency). Not yet
  checked by `aapt2` (APK-15/APK-16).
