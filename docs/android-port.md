# Android port (APK)

*Last updated: 2026-10-17 (review round 2).* Revision history at the end.

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
| An eframe host (`wgpu` + `android-native-activity` + `default_fonts`) compiles for `aarch64-linux-android` | ✅ checked in a scratch crate; not in the repo yet |
| Host crates, packaging, CI, touch UI, release | ⏳ APK-4 … APK-43 below |

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
  `crates/android-sys` behind a safe `with_activity(|env, activity| …) -> Result` API. Both crates copy sysmem's lint
  setup (`crates/sysmem/Cargo.toml`: `[lints.rust] unsafe_code = "deny"`, `[lints.clippy] dbg_macro/todo = "warn"`;
  Cargo rejects `[lints] workspace = true` next to own `[lints.*]` tables) and put `#[allow(unsafe_code)]` with a
  `// SAFETY:` comment only on the one item that needs it. jni 0.22.4's `JavaVM::singleton()` (safe, initialised by
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
- **Testable on x86_64.** Every Android module is split into a pure part that compiles and is unit-tested on every
  target (intent specs, MediaStore projections and row mapping, permission state machine, URI de-duplication, file
  name sanitising, inset maths, exit reasons, memory budget) and a thin `#[cfg(target_os = "android")]` JNI part
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
  ├─ run.rs                 start guard, memory budget, Session, Services, event loop, exit handling   (APK-10/11)
  ├─ paths.rs               internal_data_path → config/library/logs dirs                              (APK-11)
  ├─ memory.rs              pure budget_for(/proc/meminfo)                                              (APK-11)
  ├─ state.rs               HostState { app: LightcraftApp, prefs: PrefsWriter }, flush(), trim()      (APK-11)
  ├─ app.rs                 eframe::App wrapper: logic/raw_input_hook/ui/on_exit (android only)       (APK-11)
  ├─ services.rs            fills ui_egui::Services (writer, png, android-sys services)               (APK-11 …)
  ├─ lifecycle.rs           ApplicationHandler wrapper around eframe::create_native; should_leave()    (APK-18)
  ├─ renderer.rs            renderer order + start markers, exit_reason(), failure dialog wait         (APK-19)
  └─ insets.rs              px → points, RawInput::safe_area_insets                                    (APK-31)
crates/android-sys          package lightcraft-android-sys, rlib, the JNI unsafe behind a safe API
  ├─ jni_env.rs             JavaVM::singleton(), with_activity(), run_on_java_main_thread, Java error mapping
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
xtask/src/android/          mod.rs (dispatch), check.rs, sdk.rs, so.rs, elf.rs, manifest.rs, zip_lib.rs, apk.rs,
                            aab.rs, listing.rs
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
| When `android_main` returns, android-activity calls `ANativeActivity_finish` | glue.rs:985-993 | a dialog shown just before returning dies with the Activity: wait for its dismissal first (APK-19) |
| Start/Resume/SaveState/Pause/Stop, ContentRectChanged and InsetsChanged are logged as TODO | winit `android/mod.rs:222-296` | no pause/resume or insets hooks; use focus, `Suspended`, `memory_warning` and JNI |
| `TerminateWindow` → `Suspended`; no redraws while suspended | winit `android/mod.rs:217-218, 331, 580` | no eframe frame (no `logic()`) runs in the background: flush synchronously in the wrapper (APK-18) |
| `LowMemory` → `ApplicationHandler::memory_warning`; eframe's own wrapper ignores it | winit mod.rs:265; eframe `native/run.rs:541` | the host wraps `eframe::create_native` (eframe lib.rs:376) to receive it (APK-18) |
| `create_native` returns `EframeWinitApplication`; renderer errors are only logged and kept in a private `return_result` | eframe `native/run.rs:83-164` | detect failure as "the app creator never ran" plus the last logged error (APK-19) |
| `App::save` runs only with eframe's `persistence` feature (not enabled) | eframe `native/epi_integration.rs:403-408` | flushing is the host's job, not `App::save` |
| `set_ime_allowed` shows/hides the soft keyboard; `set_ime_cursor_area` / `set_ime_purpose` are no-ops; text arrives as key events | winit `android/mod.rs:916-926` | no IME composition (CJK), autocorrect or swipe (APK-35) |
| egui-winit's clipboard is in-process on Android | egui-winit `clipboard.rs` | system clipboard via JNI (APK-28) |
| egui fills `safe_area_insets` only on iOS; android-activity has no insets API in native-activity mode; with targetSdk ≥ 35 the content view is edge-to-edge, so `content_rect()` equals the window | egui-winit `safe_area.rs`; android-activity `src/lib.rs` | the host reads `WindowInsets` through JNI (APK-31) |
| KEYCODE_BACK reaches egui as `Key::BrowserBack` (winit `NamedKey::BrowserBack`), marked handled; from targetSdk 36 only with `enableOnBackInvokedCallback="false"` | winit `keycodes.rs:236`, mod.rs:374 | the manifest always opts out (APK-13); Back maps to `view.back` (APK-17) |
| `memory::total_ram()` reads `/proc/meminfo` only under `cfg(target_os = "linux")`; Android falls back to the 1.5 GiB cap | crates/engine/src/memory.rs:29-57 | the host calls `memory::set_budget` (memory.rs:77) at start (APK-11) |
| Scoped storage: in `Pictures/` an app may only create files whose extension maps to an image MIME type | MediaProvider FUSE | `safe_file`'s `.tmp` temp name fails there; temp names keep the extension (APK-9); sidecars and non-image exports go elsewhere (APK-27) |
| `lightcraft-cli snapshot --size` is in points, `--scale` is pixels per point | apps/lightcraft-cli/src/main.rs:81-82, 787; `headless::viewport_pixels` | phone shots: `--size 412x915 --scale 2.625` |

## Executing in this container

Facts (2026-10-17): linux x86_64, 4 CPUs, 16 GB RAM, 27 GB free disk (`target/` 3.1 GB); JDK 21 and
`/opt/gradle-8.14.3` installed; `xvfb-run` installed; `resvg` and `lightcraft-cli` not on PATH (use `cargo install
resvg --locked` and `cargo run -p lightcraft-cli --`); Rust targets `x86_64-unknown-linux-gnu` and
`aarch64-linux-android` only (**no `wasm32-unknown-unknown`, so `cargo xtask ci`'s final `wasm` step fails until
APK-4 adds it**); no `/dev/kvm`, no GPU (no `/dev/dri`, no Vulkan ICD); the proxy refuses `dl.google.com` (SDK, NDK,
build-tools, emulator images), `maven.google.com` is reachable. GitHub: `gh auth status` reports the `GH_TOKEN`
invalid, the GitHub MCP server is refused by the proxy, the fork `0x4E52/lightcraft-apk` has never run a workflow
(Actions most likely disabled), and `.github/workflows` has no general ci workflow (only freebsd, nix,
packaging-lint, release, windows-arm64). So **here** an agent can run `cargo check` / `cargo clippy --target
aarch64-linux-android` (no linker needed; engine + ui-egui ≈ 4 min cold) and every x86_64 test, but cannot link the
`.so`, build an APK, boot an emulator or start or read a CI run. Linking, packaging and the emulator smoke test run in
`.github/workflows/android.yml` (APK-16) on GitHub's ubuntu runners (SDK/NDK preinstalled, KVM available), started by
the maintainer until decision 0 is done.

Rules for every task:
- Run tasks sequentially in the main checkout, default `target/` dir (no parallel worktrees, no second target tree).
  Before a cold Android check, `df -h /` must show ≥ 10 GB free; otherwise `cargo clean` first.
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
- **CI runs.** If APK-4 recorded that `gh` works (decision 0 done), a ci task pushes its commit to the session's
  working branch (`git branch --show-current`), starts the run with `gh workflow run <file> --ref <branch>` (works only
  once the workflow file exists on the default branch; before that the `push` trigger starts it), watches it with
  `gh run watch`, and records the run URL in the commit or PR. Otherwise the task stops at "workflow committed" and
  adds "CI run green: <what it shows>" to `docs/android.md` → "Device checklist" for the maintainer.
- "Maintainer checks on a device" (and CI lines the agent can't run) are never ticked by an agent. An agent adds them
  to `docs/android.md` → "Device checklist" (created by APK-15) unticked.
- Every task ends with `cargo xtask ci` green and one commit `APK-<n>: …`. No task waits for a CI run or a device
  check to be ticked: "Depends on" lists only code dependencies.

## Task card format

Each card has: **Goal**; **Files** (exact paths, new/changed); **Steps**; **Tests** (the test that fails without the
change); **Verify** (commands runnable in this container); **Depends on** (the cards whose code it uses; each task
depends only on earlier ones); **Needs**; **Model** (sonnet or opus, with the reason); **Done when**; and, where it
applies, **Maintainer checks on a device**. Decisions that block a task are named in it; until decided, the agent
uses the default in "Decisions for the maintainer".

## Phase 0: foundation (x86_64, no NDK)

#### APK-4 Environment probe
- **Goal:** record what this container and the GitHub repo allow, so later cards don't guess.
- **Files:** changed `docs/android-port.md` ("Executing in this container").
- **Steps:** 1. Record `df -h /`, `du -sh target`, `ls /dev/kvm`, `java -version`. 2. `curl -sS -o /dev/null -w
  '%{http_code}' -I` for `dl.google.com/android/repository/repository2-3.xml`, `maven.google.com`,
  `services.gradle.org`, `repo.maven.apache.org`. 3. `rustup target add wasm32-unknown-unknown x86_64-linux-android`,
  then record `rustup target list --installed`. 4. With `gh`: `gh auth status`, `gh workflow list`, `gh run list -L
  3`; if auth works, whether `gh workflow run packaging-lint.yml` can be started and its log read with `gh run view
  --log` (cancel the run afterwards). 5. Run `cargo xtask ci` cold and again warm (background + Monitor) and record
  both durations.
- **Tests:** none (documentation only).
- **Verify:** `cargo xtask parity` (docs still consistent); `cargo xtask ci` green (the wasm step now runs).
- **Depends on:** —  **Needs:** none  **Model:** sonnet — command runs and a doc edit.
- **Done when:** the section lists each fact with its date and states whether agents can trigger and read CI runs
  (decision 0).

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
  Option<String>`), `crates/ui-egui/src/control.rs` and the callers of `default_export_dir()`.
- **Steps:** 1. `pub fn use_dir(path: PathBuf) -> Result<(), PathBuf>` backed by `OnceLock<PathBuf>`: the same path
  again is `Ok`, a different one returns `Err(existing)`; never panics. 2. Give the pure resolver a parameter:
  `config_dir_with(platform, env, injected: Option<&Path>, portable)`, order **injected > `LIGHTCRAFT_CONFIG_DIR` >
  portable > platform**. 3. Exports: callers of `control::default_export_dir()` prefer
  `services.default_export_dir` when set. 4. The library dir needs no global: the host passes it explicitly to the
  shared session opener (APK-7). 5. Audit production uses of `std::env::temp_dir()` (`/data/local/tmp`, not
  writable by apps): list them in the commit message; route those reachable on Android through a `config`-relative
  cache dir; leave test uses alone.
- **Tests:** `config_tests::injected_dir_wins_over_env_portable_and_platform`,
  `config_tests::use_dir_same_path_ok_other_path_err`; ui-egui `export_without_folder_uses_service_default_dir`.
- **Verify:** `cargo test -p lightcraft-engine -- config_tests && cargo test -p lightcraft-ui-egui -- export`
- **Depends on:** APK-5  **Needs:** none  **Model:** sonnet — small, well-specified injection.
- **Done when:** desktop behaviour is unchanged (existing `config_path_tests` pass) and the override is tested.

#### APK-7 Shared host code
- **Goal:** move the platform-neutral parts of `apps/lightcraft` that the Android host also needs into shared code,
  so the Android host doesn't copy them.
- **Files:** new `crates/ui-egui/src/host/{mod.rs,prefs.rs,session_open.rs,log_file.rs,flush.rs,gpu_markers.rs}`,
  `crates/gpu/src/window.rs`; changed `apps/lightcraft/src/{main.rs,logging.rs,window_gpu.rs}`,
  `crates/ui-egui/src/lib.rs` (`#[cfg(not(target_arch = "wasm32"))] pub mod host;`, precedent lib.rs:1199),
  `crates/gpu/src/lib.rs`.
- **Steps:** 1. `host/prefs.rs`: `prefs_path`/`load_prefs`/`load_prefs_at`/`decode_saved_ui`/`write_atomic`/
  `PrefsWriter` (main.rs 241–440), config dir as a parameter; `std::time::Instant` → `web_time::Instant` (already a
  ui-egui dependency) so the module stays wasm-safe if the cfg is ever lifted. 2. `host/session_open.rs`:
  `open_session` / `open_library_session` (main.rs 722–790), library dir explicit. 3. `host/log_file.rs`: the file
  logger and `record_panics` core of `logging.rs`, plus a shared "last error-level record" slot
  (`last_error() -> Option<String>`) for APK-19; desktop-only parts stay. 4. `host/flush.rs`: `pub fn
  flush(app: &mut LightcraftApp, prefs: &mut PrefsWriter) -> Result<(), String>` = `session.end_interaction()`
  (engine lib.rs:945), `session.persist()` (library.rs:610), `session.save_view()` (library.rs:730), `prefs.save()`;
  each failure logged, the first returned. Desktop `on_exit` calls it. 5. `host/gpu_markers.rs`: `window_gpu::Files`
  (marker / remembered / `take_failed` / `arm` / `started` / `armed`, window_gpu.rs:35-87), `gpu_crash_check_at`,
  `gpu_lost_check_at`, `record_gpu_lost` (main.rs:131-208) and their tests, config dir as a parameter (they use only
  `lightcraft_engine::gpu::backend`, no eframe). 6. `crates/gpu/src/window.rs`: `install_error_handler` and
  `check_pipeline` (window_gpu.rs:199-215; wgpu + pollster only, both already gpu dependencies; wgpu is one version,
  so `wgpu::Device` is eframe's), reached as `lightcraft_engine::gpu::window`. 7. Stays in the app: `choose`/the
  Windows probe, `relaunch_after_failure`, Windows renderer memory, XWayland, `rfd`. 8. Anything needing a dependency
  ui-egui lacks stays in the app; note it in the commit.
- **Tests:** the moved tests (main.rs `mod tests`, `config_path_tests` main.rs:1116, gpu-marker tests main.rs:1557,
  window_gpu tests, logging tests) move with their code and pass unchanged; new `flush_commits_open_interaction_and_view`
  (ui-egui headless: begin a slider interaction, call `host::flush` with no frame in between, reopen: the edit and
  `view.json` are there).
- **Verify:** `cargo test -p lightcraft -p lightcraft-ui-egui -p lightcraft-gpu && cargo xtask wasm && cargo xtask
  ci`; look at the desktop app: `xvfb-run -a cargo run -p lightcraft -- --control 18555` plus a one-line script
  `{"method":"ui.screenshot","params":{"path":"<scratch>/apk7.png","headless":true}}` through
  `docs/showcase/run.py`, or `cargo run -p lightcraft-cli -- snapshot --demo -o target/apk7.png`, then Read the PNG.
- **Depends on:** APK-6  **Needs:** none  **Model:** opus — refactor across a 1500-line host with behaviour to keep.
- **Done when:** no moved function remains duplicated; the desktop app runs as before.

#### APK-8 Diagnostics switches without the environment
- **Goal:** every `LIGHTCRAFT_PROFILE`, `LIGHTCRAFT_LOG` and `LIGHTCRAFT_GPU_BACKEND` read can be set by a host
  that has no environment variables, before first use.
- **Files:** new `crates/profile/{Cargo.toml,src/lib.rs}` (package `lightcraft-profile`, L0, no dependencies);
  changed root `Cargo.toml` (workspace dep), `xtask/src/layers.rs` (`("profile", Class::Layer(0))`), the nine
  readers: `crates/catalog/src/journal.rs:173`, `crates/pipeline/src/lib.rs:709`, `crates/gpu/src/render.rs:575`,
  `crates/denoise/src/cpu.rs:186`, `crates/denoise-core/src/run.rs:213`, `crates/faces/src/net.rs:274,287`,
  `crates/engine/src/denoise.rs:963`, `crates/engine/src/camera_preview.rs:433,469` and their `Cargo.toml`s;
  `crates/gpu/src/backend.rs` (`env_choice`, :101-108); `crates/engine/src/logging.rs` (:74).
- **Steps:** 1. `lightcraft_profile::{enabled() -> bool, set_enabled(bool)}`: an `AtomicU8` (unset / off / on);
  unset reads `LIGHTCRAFT_PROFILE` once. No common lower crate exists (faces and denoise-core share nothing with
  catalog), hence the tiny crate. 2. Replace the nine reads (and the three local `OnceLock`s) by `enabled()`.
  3. `backend::set_choice(BackendChoice) -> Result<(), BackendChoice>` fills `env_choice`'s `OnceLock` first; `Err`
  when it was already read. 4. `logging::install_with_level(prefix, LevelFilter)`; `install` calls it with
  `level_from(env…)`.
- **Tests:** `profile_switch_overrides_env` (profile), `set_choice_before_first_use_wins` and
  `set_choice_after_first_use_errs` (gpu), `install_with_level_uses_given_level` (engine logging).
- **Verify:** `cargo test -p lightcraft-profile -p lightcraft-gpu -p lightcraft-engine -- profile set_choice
  install_with_level && cargo xtask layers && cargo xtask ci`; `grep -rn 'var_os("LIGHTCRAFT_PROFILE")' crates
  --include=*.rs` shows only `crates/profile` and tests.
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
- **Steps:** 1. Entry crate as in Architecture; lint tables copied from `crates/sysmem/Cargo.toml`; one
  `#[allow(unsafe_code)]` on `android_main` with `// SAFETY:`; body under `#[cfg(target_os = "android")]` so the
  crate is empty elsewhere. 2. All Android-only dependencies under `[target.'cfg(target_os = "android")'.dependencies]`
  (Architecture). 3. `android-host::run(app: AndroidApp) -> Result<(), String>` (android only): a process-wide
  `static STARTED: AtomicBool`; the guard is a pure fn `try_start(&AtomicBool) -> Result<(), String>`; a second call
  returns `Err("LightCraft was reopened in the same process; …")` and the entry crate exits (APK-18). 4.
  `android-sys::jni_env`: `JavaVM::singleton()?`, `with_activity(app, |env, activity| …) -> Result<T, Error>` with the
  one unsafe cast (`env.as_cast_raw`), `#[allow(unsafe_code)]` on that fn only, and `vm.attach_current_thread(|env|
  …)` (jni 0.22's closure API). No `ndk-context`. API doc: every loop over Java objects runs each iteration inside
  `env.with_local_frame(16, …)`. 5. Java errors: `Error::Java { class, message }` built from
  `Error::CaughtJavaException` (jni-0.22.4 vm/java_vm.rs:471-481; read class name and message inside the closure via
  `exception_occurred`, `getClass().getName()`, `getMessage()`, then clear). A pure `fn user_message(class: &str,
  message: &str, action: &str) -> String` turns known classes into actionable text
  (`ActivityNotFoundException` → "No app can open this", `SecurityException` → "Access was withdrawn",
  `FileNotFoundException` → "The file is gone"). 6. Production crate attributes (`#![deny(clippy::unwrap_used, …)]`;
  `#![forbid(unsafe_code)]` in android-host). 7. `cargo xtask licences --write`.
- **Tests:** `second_start_is_refused` (android-host, x86_64); `java_exception_maps_to_actionable_error`
  (android-sys, canned class names); `cargo xtask layers` fails "unregistered" without the rows.
- **Verify:** `cargo test -p lightcraft-android-host` **alone** (proves no winit/android-activity leaks into the
  Linux build) `&& cargo test -p lightcraft-android-sys && cargo xtask layers && cargo xtask licences && cargo xtask
  android check && cargo xtask wasm && cargo xtask ci`
- **Depends on:** APK-7  **Needs:** rust-target  **Model:** opus — unsafe placement, JNI API, workspace wiring.
- **Done when:** all three crates lint clean for `aarch64-linux-android` (APK-5 now derives the set from
  `lightcraft-android`) and compile empty on x86_64 and wasm32.

#### APK-11 First pixel: budget, Session, Services, eframe
- **Goal:** `run()` opens the UI with the procedural demo library in internal storage, inside a memory budget a
  phone can hold, with exports and screenshots able to write files.
- **Files:** new `crates/android-host/src/{paths.rs,memory.rs,state.rs,app.rs,services.rs}`; changed
  `crates/android-host/src/{lib.rs,run.rs}`, `crates/android-host/Cargo.toml`.
- **Steps:** 1. `paths.rs` (pure): from `internal_data_path()` derive `config/`, `library/`, `logs/`;
  `engine::config::use_dir(config)`. 2. `memory.rs` (pure): `budget_for(meminfo: &str) -> usize` = MemTotal / 8
  clamped to [256 MiB, 768 MiB] (256 MiB when unreadable); `run` calls `lightcraft_engine::memory::set_budget` before
  opening the session. (`ActivityManager.getMemoryClass()` is not used: it bounds the Java heap, not native
  allocations.) 3. Audit `grep -rn 'target_os = "linux"' crates --include=*.rs` (known: engine memory.rs, ui-egui
  lib.rs, panels/dialogs.rs, panels/settings.rs): list in the commit what Android silently skips and whether it
  matters. 4. Log file (`host::log_file`) under `logs/`, plus stderr (already logcat);
  `engine::guard::install_hook(<config>/panic.log)`. 5. `host::session_open` with `library/`, seeding the demo
  library on first start. 6. `services.rs`: `fn base_services() -> Services` (compiles on x86_64) with `write:
  Some(Box::new(lightcraft_engine::export::write_file))`, `write_shared: Some(Arc::new(…write_file))`, `png` from
  `lightcraft_codecs::encode_png` (as main.rs:708-711), `log_file`; no pickers yet. 7. `state.rs`: `HostState { app:
  LightcraftApp, prefs: PrefsWriter }` with `flush()` (= `host::flush`) and `trim()` (APK-18). 8. `app.rs` (android
  only): the eframe::App wrapper holding `Rc<RefCell<HostState>>` — `logic` → `app.logic` + `PrefsWriter::tick` +
  a periodic `flush` every 30 s (web precedent), `raw_input_hook`, `ui`, `on_exit` → `flush` +
  `session.close_library()` + `shutdown(EXIT_WAIT)` (as main.rs:60-108). A failed `try_borrow_mut` is logged and
  skipped, never a panic. 9. `eframe::run_native("LightCraft", NativeOptions { android_app: Some(app.clone()),
  renderer: Wgpu, .. }, …)`; errors return `Err` with the reason (logged by the entry crate).
- **Tests:** `paths_from_internal_dir`, `budget_for_phone_meminfo` (4 GB → 512 MiB, 12 GB → 768 MiB, garbage → 256
  MiB), `services_have_writer_and_png`, `state_flush_without_a_frame` (all x86_64).
- **Verify:** `cargo test -p lightcraft-android-host && cargo xtask android check && cargo xtask ci`
- **Depends on:** APK-7, APK-10  **Needs:** rust-target  **Model:** opus — eframe/Android start-up wiring.
- **Done when:** the host compiles and lints for Android; first real run happens in the APK-16 workflow.

#### APK-12 `cargo xtask android so`: link the library
- **Goal:** build `liblightcraft_android.so` with the NDK and check its symbols and page alignment.
- **Files:** new `xtask/src/android/{sdk.rs,so.rs,elf.rs}`; changed `xtask/src/android/mod.rs`.
- **Steps:** 1. `sdk.rs`: find `ANDROID_HOME`/`ANDROID_SDK_ROOT` and `ANDROID_NDK_HOME`/`ANDROID_NDK_LATEST_HOME`,
  read the NDK version from `source.properties`; clear error when missing. 2. `so.rs`: set
  `CARGO_TARGET_<TRIPLE>_LINKER=<ndk>/toolchains/llvm/prebuilt/linux-x86_64/bin/<triple><minSdk>-clang` (no
  `cargo ndk`), add `-C link-arg=-Wl,-z,max-page-size=16384` when the NDK is older than r28, build `-p
  lightcraft-android --target aarch64-linux-android` (and `x86_64-linux-android` with `--abi x86_64`), copy to
  `target/android/jniLibs/<abi>/`. 3. `elf.rs`: run the NDK's `llvm-readelf -lW --dyn-syms`; pure parsers assert every
  `LOAD` alignment is a power of two ≥ 0x4000 and that `android_main` and `ANativeActivity_onCreate` are exported.
- **Tests:** `sdk::ndk_version_parses`, `so::linker_path_for_abi`, `elf::rejects_4k_load_alignment`,
  `elf::accepts_64k_load_alignment`, `elf::requires_entry_symbols` (canned readelf output).
- **Verify:** `cargo test -p xtask -- android::`
- **Depends on:** APK-5  **Needs:** ndk (unit tests: none)  **Model:** sonnet — path/argument building with tests.
- **Done when:** unit tests pass here; the APK-16 workflow runs the command.

#### APK-13 Android manifest
- **Goal:** a complete, declarative manifest and a reproducible version code.
- **Files:** new `packaging/android/AndroidManifest.xml` (template with `${APPLICATION_ID}`, `${VERSION_CODE}`,
  `${VERSION_NAME}`, `${MIN_SDK}`, `${TARGET_SDK}`), `xtask/src/android/manifest.rs` (fills it).
- **Steps:** 1. `<application>`: `android:hasCode="false"`, `android:allowBackup="false"` (Auto Backup would copy
  the catalog to Drive; see decision 9), `android:extractNativeLibs="false"` (keeps the stored, 16 KB-aligned `.so`
  mapped in place), `android:enableOnBackInvokedCallback="false"` **unconditionally** (with a comment: Back reaches
  native code only as KEYCODE_BACK, which targetSdk 36 stops dispatching without this opt-out). 2. Activity
  `android.app.NativeActivity`: `android:exported="true"`, `android:launchMode="singleTask"`,
  `android:theme="@android:style/Theme.DeviceDefault.NoActionBar"` (framework style, no resources),
  `android:configChanges="mcc|mnc|locale|touchscreen|keyboard|keyboardHidden|navigation|screenLayout|fontScale|uiMode|orientation|density|screenSize|smallestScreenSize|layoutDirection|colorMode|fontWeightAdjustment|grammaticalGender"`,
  `<meta-data android:name="android.app.lib_name" android:value="lightcraft_android"/>`, MAIN/LAUNCHER filter.
  Permissions arrive with the tasks that need them (APK-21, APK-22). minSdk 30, targetSdk 36 (decision 3).
  3. `fn version_code(version: &str) -> Result<u32, String>` from `version::read` (xtask/src/version.rs):
  `major*1_000_000 + minor*1_000 + patch*10 + (9 for a release, 0..8 for pre-releases by their number)`;
  `${VERSION_NAME}` is the same version string.
- **Tests:** `manifest_has_required_keys` (parses the filled template; asserts each attribute above individually,
  each `configChanges` value, the theme, `extractNativeLibs`, the back opt-out); `version_code_is_monotonic`,
  `prerelease_sorts_before_release`.
- **Verify:** `cargo test -p xtask -- manifest version_code`
- **Depends on:** APK-5  **Needs:** none  **Model:** sonnet — declarative config with key tests.
- **Done when:** the tests pass; the APK-16 workflow's `aapt2 dump badging` shows the values.

#### APK-14 Launcher icons
- **Goal:** Android launcher and adaptive icons from our own `assets/app-icon/lightcraft.svg`.
- **Files:** new `packaging/android/icons.sh` (single purpose, modelled on `packaging/icons.sh`),
  `packaging/android/res/mipmap-{mdpi,hdpi,xhdpi,xxhdpi,xxxhdpi}/{ic_launcher.png,ic_launcher_foreground.png,
  ic_launcher_monochrome.png}`, `packaging/android/res/mipmap-anydpi-v26/ic_launcher.xml`,
  `packaging/android/res/values/ic_launcher_background.xml`; changed `assets/ATTRIBUTION.md`.
- **Steps:** install `cargo install resvg --locked`. The SVG is a 512 tile clipped to `rx=112` with a full-bleed
  `#f2a516` field and one paper `<path>`. Background = colour resource `#f2a516`. Foreground = the SVG with the
  `clip-path` group attribute and the background `<rect>` removed and the viewBox widened by `sed` to `-128 -128 768
  768` (the paper fits the 72/108 safe zone), each `sed` followed by a `grep -q` guard as in `packaging/icons.sh:22-23`.
  Monochrome = the same path filled `#000000`. Legacy `ic_launcher.png` = the original tile. Sizes 48/72/96/144/192
  px (foreground layers 108/162/216/324/432 px). ATTRIBUTION row for `packaging/android/res/` ("derived from
  assets/app-icon/lightcraft.svg", MIT OR Apache-2.0, original work). Nothing from `docs/brand/`, no
  `ai.storyteller.lightcraft` id. A 512 px store icon goes in APK-42.
- **Tests:** `cargo xtask assets` fails without the ATTRIBUTION row.
- **Verify:** `sh packaging/android/icons.sh && cargo xtask assets`; Read the xxxhdpi foreground PNG to look at it.
- **Depends on:** APK-13  **Needs:** none  **Model:** sonnet — asset generation with a spelled-out recipe.
- **Done when:** icons committed with attribution.

#### APK-15 `cargo xtask android apk`: package a debug APK
- **Goal:** one command turns the `.so`, manifest and resources into an installable, 16 KB-aligned debug APK.
- **Files:** new `xtask/src/android/{zip_lib.rs,apk.rs}`, `docs/android.md` (build, install, debugging, Device
  checklist); changed `xtask/Cargo.toml` (`zip = { version = "7", default-features = false, features = ["deflate"]
  }`, already in Cargo.lock at 7.2.0, MIT; xtask is not in `SHIPPED`), `xtask/src/android/mod.rs`, `.gitignore`
  (`*.jks`, `*.keystore`, `*.p12`, `keystore.properties`, `local.properties`, `.gradle/`, `build/`).
- **Steps (route per decision 6, default: SDK build-tools ≥ 36, no Gradle):** 1. `aapt2 compile` res → `aapt2 link`
  with `platforms/android-36/android.jar`, the filled manifest and `--debug-mode` for debug builds (so `adb shell
  run-as <id>` works). 2. `zip_lib.rs`: copy every entry of aapt2's output with `ZipWriter::raw_copy_file`, then
  append `lib/<abi>/liblightcraft_android.so` with `CompressionMethod::Stored` and `with_alignment(16384)`.
  3. `zipalign -P 16 -f 4` → `apksigner sign` with a debug key generated outside the repo
  (`~/.android/debug.keystore`) → `zipalign -c -P 16 -v 4`. Pure argument builders for each tool. 4. `docs/android.md`:
  install, `adb shell run-as <id> cat files/config/panic.log`, logs, and the Device checklist heading.
- **Tests:** `aapt2_link_args_debug_mode`, `zipalign_args_use_16k_pages`, `so_entry_is_stored_uncompressed` and
  `so_entry_data_is_16k_aligned` (build a zip from a fake aapt2 output in a temp dir, read it back),
  `copied_entries_unchanged`.
- **Verify:** `cargo test -p xtask -- apk zip_lib`
- **Depends on:** APK-12, APK-13, APK-14  **Needs:** sdk + ndk (unit tests: none)  **Model:** sonnet — tool
  invocation with tests; the zip step is named.
- **Done when:** unit tests pass here; the APK-16 workflow produces the APK.

#### APK-16 Android CI workflow
- **Goal:** prove linking, packaging and start-up on every push, since this container can't.
- **Files:** new `.github/workflows/android.yml`; changed `docs/android.md` (Device checklist line for the run).
- **Steps:** ubuntu-latest; `rustup target add aarch64-linux-android x86_64-linux-android`; `cargo xtask android
  check`; `sdkmanager "platforms;android-36" "build-tools;36.0.0"`; `CARGO_PROFILE_DEV_DEBUG=line-tables-only cargo
  xtask android so --abi arm64-v8a --abi x86_64` (record the `.so` sizes in the job summary); `cargo xtask android
  apk`; `aapt2 dump badging`. KVM udev step before the emulator: `echo 'KERNEL=="kvm", GROUP="kvm", MODE="0666",
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
  new `crates/android-sys/src/activity.rs` (`move_task_to_back`), changed `crates/android-host/src/services.rs`.
- **Steps:** 1. Append one final branch to the existing `view.back` cascade (menus.rs:615-643: dialog, lights out,
  keyword painter, fullscreen, tool, crop, person page, compare/survey, detail → grid): when nothing else applied and
  `services.move_to_back` is `Some`, call it. Keep the existing order and the Escape binding. 2. Map
  `Key::BrowserBack` to `view.back`. 3. `quit_disabled` (default false = desktop unchanged): `enabled("app.quit")` is
  false, so dispatch, Cmd+Q, the menubar and control `app.quit` all refuse; the control arm checks it before sending
  `ViewportCommand::Close`, and `ui.quit` (lib.rs:1433) too. 4. Android: `quit_disabled = true`, `move_to_back` = JNI
  `Activity.moveTaskToBack(true)` posted with `run_on_java_main_thread`.
- **Tests:** `back_at_root_moves_to_back` (fake service), `back_order_unchanged_on_desktop` (existing arms still
  first; the existing Escape/back tests stay green), `quit_refused_when_disabled` (command + control).
- **Verify:** `cargo test -p lightcraft-ui-egui -- back quit && cargo xtask android check && cargo xtask ci`
- **Depends on:** APK-11  **Needs:** rust-target  **Model:** opus — navigation order and command gating.
- **Done when:** tests pass; desktop unchanged.
- **Maintainer checks on a device:** Back with gesture navigation on Android 16 (targetSdk 36) walks back to the grid,
  then moves the app to the background; Home → reopen; Back to root → reopen.

#### APK-18 Lifecycle: flush, trim and system destroy
- **Goal:** work survives backgrounding and process death, memory warnings shrink caches, and a system-initiated
  Activity destroy ends the process cleanly instead of hanging.
- **Files:** new `crates/android-host/src/lifecycle.rs`; changed `crates/android-host/src/{run.rs,state.rs}` (switch
  to `eframe::create_native` with our own `EventLoop::with_user_event().with_android_app(app)`; the app creator
  hands a clone of the `Rc<RefCell<HostState>>` to the wrapper), `crates/android-sys/src/activity.rs`
  (`activity_state() -> Result<ActivityState { finishing, destroyed }, Error>`), `crates/engine/src/lib.rs`
  (`Session::trim_caches()`), `apps/lightcraft-android/src/lib.rs` (exit after a destroy).
- **Steps:** 1. `lifecycle.rs`: an `ApplicationHandler<UserEvent>` forwarding every method to
  `EframeWinitApplication`. In `suspended` and `WindowEvent::Focused(false)` it runs `HostState::flush()`; in
  `memory_warning` it runs `HostState::trim()` = `Session::trim_caches()` (drop preview/decode caches, each named in
  its doc comment) + `lightcraft_engine::gpu::trim_pool(0)`. Everything runs synchronously on the android_main thread,
  outside eframe's own App calls (no frames run while suspended); `try_borrow_mut` failure → log and skip. 2. Destroy
  detection: after `suspended` / `Focused(false)` and then on each `about_to_wait` until the next `resumed`, call
  `activity_state()` (JNI `Activity.isFinishing()` / `isDestroyed()`; `performDestroy` sets `mDestroyed` before
  `onDestroy`). A pure `should_leave(state, suspended) -> bool` decides; when true: flush, `event_loop.exit()`.
  3. When `run_app` returns because of that, `run` returns `Ok(Exit::Destroyed)`; the entry crate flushes the log and
  calls `std::process::exit(0)`, so the next launch starts a fresh process (the start guard of APK-10 stays as the
  backstop and also exits). Documented as the one exception to android-activity's no-exit advice. 4. Long jobs keep
  their `TaskGuard` rows; `docs/android.md` notes the cached-apps freezer (below, Risks).
- **Tests:** `trim_caches_empties_preview_cache` (engine); `wrapper_flushes_without_a_frame` and
  `wrapper_maps_memory_warning_to_trim` (android-host, fake `HostState` behind a trait);
  `destroyed_while_suspended_leaves_loop`, `focus_loss_alone_keeps_loop` (pure `should_leave`).
- **Verify:** `cargo test -p lightcraft-engine -- trim && cargo test -p lightcraft-android-host && cargo xtask
  android check && cargo xtask ci`
- **Depends on:** APK-11, APK-17  **Needs:** rust-target  **Model:** opus — event-loop wrapping and lifecycle
  reasoning.
- **Done when:** tests pass; the APK-16 workflow's smoke test still opens the UI (Device checklist line).
- **Maintainer checks on a device:** edit, Home, `adb shell am kill <id>`, reopen: edits and UI state are back;
  rotate; Developer options ▸ "Don't keep activities" on → leave → reopen: no ANR, the app reopens; remove from
  recents → reopen.

#### APK-19 Renderer failure and GPU start markers
- **Goal:** a GPU that won't start (error, panic or driver crash) gives a message the user can read, and the next
  launch tries the other renderer.
- **Files:** new `crates/android-host/src/renderer.rs`, `crates/android-sys/src/dialog.rs`; changed
  `crates/android-host/src/run.rs`.
- **Steps:** 1. Order: `backend::window_renderer_order(forced, None, files.take_failed(), "android", compiled,
  false)` (backend.rs:313; yields Vulkan then GL for android; `remembered` is Windows-only on desktop too) with
  `host::gpu_markers::Files` under `<config>`; `forced = backend::forced_window_renderer(<debug.lightcraft.gpu_backend>,
  None)` (APK-20); the first entry's backends go into `NativeOptions::wgpu_options` as desktop's non-Windows path
  does (window_gpu.rs:88-136). 2. `files.arm(r)` before `create_native`,
  `files.started(r)` in the app creator, which also sets an `app_created` flag and calls
  `lightcraft_engine::gpu::window::install_error_handler` on the render state's device. 3. `gpu_crash_check_at(
  Some(<config>/gpu-init.marker), false)` / `gpu_lost_check_at` → `lightcraft_engine::gpu::set_enabled(false)` plus a
  notice (as main.rs:131-208). 4. `std::panic::catch_unwind` around the run, mapping a panic to `Err` (main.rs:956-1045).
  5. After `run_app` returns, a pure `exit_reason(app_created, exit_requested, last_error) -> Option<String>`
  (`last_error` from `host::log_file::last_error()`, which also catches eframe's "Exiting because of error") decides
  whether to show the failure. 6. `dialog.rs` posts an `AlertDialog` (reason + log path; buttons with `null`
  listeners) via `run_on_java_main_thread`; then android_main keeps running: a pure `wait_until(dismissed: impl
  FnMut() -> Result<bool, String>, tick, limit)` drives a closure that calls `app.poll_events(Some(200 ms), …)`
  (keeps Pause/Stop acknowledged, no ANR; a Destroy ends the wait) and polls `isShowing()` through `with_activity`; at
  most 10 minutes; a JNI error ends the wait at once (logged). Only then does `run` return `Err`. The panic path
  waits the same way.
- **Tests:** `failed_vulkan_start_tries_gl_next` (marker left behind → GL first), `exit_without_app_shows_last_error`,
  `normal_exit_shows_nothing`, `waits_for_dismissal_then_returns`, `jni_error_ends_wait` (fake closures, x86_64).
- **Verify:** `cargo test -p lightcraft-android-host -- renderer && cargo xtask android check && cargo xtask ci`
- **Depends on:** APK-7, APK-8, APK-18  **Needs:** rust-target  **Model:** opus — GPU fallback and failure design.
- **Done when:** tests pass; Device checklist line "CI logcat shows the adapter line".
- **Maintainer checks on a device:** after APK-20, `adb shell setprop debug.lightcraft.gpu_backend gl`, restart: the
  UI runs on GL; `setprop … vulkan` again.

#### APK-20 Diagnostics switches via system properties
- **Goal:** `debug.lightcraft.profile`, `.log`, `.gpu_backend`, `.control` stand in for environment variables (apps
  get none).
- **Files:** new `crates/android-sys/src/props.rs`; changed `crates/android-host/src/run.rs`, `docs/android.md`.
- **Steps:** read the properties with `android-properties` (already in `Cargo.lock` through android-activity; licence
  checked by `cargo xtask licences`; target-gated). A pure `fn settings_from(props: impl Fn(&str) -> Option<String>)
  -> DiagSettings`. `run` applies them before anything reads them: `lightcraft_profile::set_enabled`,
  `backend::set_choice`, `logging::install_with_level` / the `host::log_file` filter (APK-8); `.control` feeds APK-21.
- **Tests:** `props_map_to_settings`, `unknown_values_are_ignored` (pure, x86_64).
- **Verify:** `cargo test -p lightcraft-android-sys -- props && cargo xtask android check && cargo xtask ci`
- **Depends on:** APK-8, APK-11  **Needs:** rust-target  **Model:** sonnet — mapping onto setters that exist.
- **Done when:** documented in `docs/android.md`.

#### APK-21 Debug control server with a token
- **Goal:** drive and screenshot the app on a device with the usual JSON-lines scripts, without letting other apps on
  the device (which share loopback) drive it.
- **Files:** move `apps/lightcraft/src/control_server.rs` → `crates/ui-egui/src/host/control_server.rs` (plain
  `std::net`, 127.0.0.1); changed `apps/lightcraft/src/main.rs`, `crates/android-host/src/run.rs`,
  `packaging/android/AndroidManifest.xml` (`android.permission.INTERNET`, also needed for model downloads through
  `crates/fetch`), `xtask/src/android/manifest.rs` (test), `docs/android.md`.
- **Steps:** 1. `start(port, ctx, token: Option<String>)`: with a token every request must carry a matching `"token"`
  field, else `{"ok": false, "error": "bad token"}`; desktop passes `None` (unchanged). 2. Android starts the server
  only when `debug.lightcraft.control=<port>` is set (shell-only, `adb shell setprop`); no default port. It writes a
  random 128-bit token (from `getrandom`, already in the lock) to `<internal>/files/control-token`. 3. Document: `adb
  shell setprop debug.lightcraft.control 18555`, restart, `adb forward tcp:18555 tcp:18555`, `adb shell run-as <id>
  cat files/control-token`, then `docs/showcase/run.py` with the token and `ui.screenshot`.
- **Tests:** the existing control_server tests move with it and pass; `rejects_request_without_token`,
  `no_server_without_property` (pure start decision); manifest test asserts INTERNET.
- **Verify:** `cargo test -p lightcraft-ui-egui -- control_server && cargo test -p xtask -- manifest && cargo xtask ci`
- **Depends on:** APK-7, APK-20  **Needs:** rust-target  **Model:** sonnet — move plus a start condition and a check.
- **Done when:** desktop `--control` unchanged; the APK-16 workflow gains a step that sets the property, forwards
  the port and runs a one-line `ui.inspect` script with the token (Device checklist line).
- **Maintainer checks on a device:** the documented recipe returns a screenshot; without the token it is refused.

## Phase 2: real photos

#### APK-22 Photo permissions
- **Goal:** ask for and track access to the photo library without blocking the UI thread.
- **Files:** new `crates/android-sys/src/permissions.rs`; changed `packaging/android/AndroidManifest.xml`,
  `xtask/src/android/manifest.rs` (test), `crates/android-host/src/{services.rs,app.rs}`.
- **Steps:** 1. Manifest: `READ_MEDIA_IMAGES`, `READ_MEDIA_VISUAL_USER_SELECTED`, `READ_EXTERNAL_STORAGE`
  `android:maxSdkVersion="32"`, `ACCESS_MEDIA_LOCATION`. 2. Pure `permissions_for_sdk(sdk) -> &[&str]`:
  `[READ_MEDIA_IMAGES, READ_MEDIA_VISUAL_USER_SELECTED, ACCESS_MEDIA_LOCATION]` on API 33+, `[READ_EXTERNAL_STORAGE,
  ACCESS_MEDIA_LOCATION]` on 30–32 (`ACCESS_MEDIA_LOCATION` is a runtime permission: unless requested, EXIF GPS read
  through FUSE is redacted). 3. Pure state machine `NotAsked → Pending → Granted | Partial | Denied` with a clock and
  focus events: after a request, no `Focused(false)` within 1 s (no system dialog appeared: permanently denied, and
  NativeActivity drops `onRequestPermissionsResult`) or `Focused(true)` while still not granted → `Denied`.
  4. JNI: `Activity.requestPermissions` posted with `run_on_java_main_thread`; `checkSelfPermission` re-checked on
  each `Focused(true)` and every 500 ms while `Pending`, from the android-host App wrapper's `logic` (app.rs), which
  calls `ctx.request_repaint_after(500 ms)` while Pending. 5. Denied → the command's error: "LightCraft needs access
  to your photos: allow it in Settings ▸ Apps ▸ LightCraft ▸ Permissions".
- **Tests:** `pending_then_granted`, `partial_access_is_reported`, `no_dialog_means_denied`,
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
  fail on them; Add stays available with a note. 4. The query runs off the UI thread and holds a `TaskGuard`
  (`session.activity.start("import", …)`). Raws MediaStore doesn't classify as images arrive through APK-25.
- **Tests:** `row_to_path_prefers_data_column`, `row_to_path_from_relative_path`, `rejects_path_outside_volume`
  (pure); headless `import_media_disabled_without_service`, `import_media_feeds_import_dialog` (fake service),
  `prefer_copy_import_defaults_to_copy`, `add_mode_write_beside_original_gives_actionable_error` (read-only temp
  folder).
- **Verify:** `cargo test -p lightcraft-android-sys -- media_store && cargo test -p lightcraft-ui-egui --
  import_media copy_import add_mode && cargo xtask android check && cargo xtask parity && cargo xtask ci`
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
  2. `pick_browser.rs`: an in-app egui `Picker` that browses only `pick_roots` (Android: the app's external files dir,
  `Pictures/LightCraft`, `Documents/LightCraft`, `Download`, the library), used when `Services::picker` is `None` and
  `pick_roots` is non-empty (desktop and web: empty, unchanged). 3. A command whose target can't live under those
  roots is disabled with a tooltip ("Not available on Android: …").
- **Tests:** headless `presets_import_uses_in_app_picker`, `in_app_picker_stays_inside_roots` (`..` and symlinks
  rejected), `disabled_without_picker_explains`.
- **Verify:** `cargo test -p lightcraft-ui-egui -- in_app_picker presets_import disabled_without_picker && cargo
  xtask parity && cargo xtask ci`
- **Depends on:** APK-23  **Needs:** none  **Model:** opus — audit across many commands and a new UI component.
- **Done when:** tests pass; the docs table and parity notes match.
- **Maintainer checks on a device:** Import Presets from `Download` through the in-app picker.

#### APK-25 "Open with LightCraft" and shares
- **Goal:** files sent from other apps (view, share one, share several) are imported.
- **Files:** new `crates/android-sys/src/{content.rs,intents.rs}`; changed `packaging/android/AndroidManifest.xml`,
  `xtask/src/android/manifest.rs` (test), `crates/android-host/src/services.rs`, `docs/android.md`.
- **Steps:** 1. Intent filters, category DEFAULT, mimeTypes `image/*`, `image/x-adobe-dng`, `image/x-canon-cr2`,
  `image/x-canon-cr3`, `image/x-nikon-nef`, `image/x-sony-arw`, `application/octet-stream`: `ACTION_VIEW` (URI in
  `getData()`), `ACTION_SEND` (`Intent.EXTRA_STREAM` via `getParcelableExtra(String)`, deprecated but working; the
  typed overload only on API 33+), `ACTION_SEND_MULTIPLE` (`getParcelableArrayListExtra`). A pure `IntentSpec` parse
  from (action, data, extras). 2. Read `getIntent()` at start and on each `Focused(true)`; handle URIs not seen
  before. 3. The read grant ends with the Activity, so copy at once, zero unsafe: query `OpenableColumns.DISPLAY_NAME`
  / `SIZE` and the MIME type; target `<library>/.incoming/<name>` where a pure `sanitize_display_name` keeps only the
  final component, drops `/`, `\`, NUL and leading dots, caps it at 120 bytes on a char boundary, adds an extension
  from the MIME type when missing and de-duplicates; refuse files over 2 GiB (by `SIZE`, and again while copying).
  `ContentResolver.openInputStream(uri)` → loop `InputStream.read(byte[1 MiB])` + `env.get_byte_array_region` into
  the file through a pure `copy_chunks(reader, writer, cap)` → `close()`, each step inside `with_local_frame`, Java
  exceptions mapped by APK-10's `user_message`. Then the existing Copy import (the catalog stores the path under
  `Originals/`). 4. Errors (revoked, unreadable, half-read, too big) become per-file import errors; partial files are
  removed. Limitation (decision 2): NativeActivity has no `onNewIntent`, so under `singleTask` a second "Open with"
  while LightCraft runs only brings it to the front (it is picked up on the next `Focused(true)` only if
  `getIntent()` changed).
- **Tests:** `same_uri_handled_once`, `view_reads_data`, `send_reads_extra_stream`, `send_multiple_reads_list`,
  `display_name_cannot_escape_dir`, `oversized_name_is_truncated_at_char_boundary`,
  `copy_removes_partial_file_on_read_error` (reader that fails halfway, temp dir), `copy_stops_at_size_cap` (pure);
  manifest test asserts the three filters.
- **Verify:** `cargo test -p lightcraft-android-sys -- content intents && cargo test -p xtask -- manifest && cargo
  xtask android check && cargo xtask ci`
- **Depends on:** APK-10, APK-23  **Needs:** rust-target  **Model:** opus — URI grants, hostile names, error paths.
- **Done when:** tests pass; the limitation is in `docs/android.md`.
- **Maintainer checks on a device:** share from Google Photos (one and several), from Files (Downloads) and from an
  app using a FileProvider; open a DNG from Files with "Open with"; all import.

#### APK-26 Open links (`Services::open_url`)
- **Goal:** Help links and About open in the browser.
- **Files:** changed `crates/android-sys/src/intents.rs`, `crates/android-host/src/services.rs`.
- **Steps:** pure `IntentSpec::view(url)` (only `https://`), JNI `startActivity` on the Java main thread;
  `ActivityNotFoundException` → "No app can open links".
- **Tests:** `view_spec_rejects_non_https`, `view_spec_fields` (pure).
- **Verify:** `cargo test -p lightcraft-android-sys -- intents && cargo xtask android check && cargo xtask ci`
- **Depends on:** APK-25  **Needs:** rust-target  **Model:** sonnet — follows APK-25's JNI pattern.
- **Done when:** tests pass.  **Maintainer checks on a device:** Help ▸ a link opens the browser.

#### APK-27 Export to Pictures and share (`file.share`)
- **Goal:** exports land where other apps see them and can be shared.
- **Files:** new `crates/android-sys/src/storage.rs`; changed `crates/ui-egui/src/lib.rs` (`Services::share:
  Option<ShareFn>`, `Services::export_dir_for: Option<Box<dyn Fn(ExportKind) -> String>>`), `menus.rs`,
  `menubar.rs`, the export dialog's sidecar option, `crates/android-sys/src/{intents.rs,media_store.rs}`,
  `crates/android-host/{paths.rs,services.rs}`, `docs/parity-checklist.md`.
- **Steps:** 1. Folders from JNI `Environment.getExternalStoragePublicDirectory(DIRECTORY_PICTURES)` /
  `DIRECTORY_DOCUMENTS` + `/LightCraft` (right for secondary users and work profiles; pure fallback to
  `/storage/emulated/0/…` when JNI fails). A pure `export_dir_for(kind)`: images (JPEG, PNG, TIFF, DNG, HEIF, …) →
  Pictures; PDF, video and other non-image output → Documents. The exporter writes by path (APK-9 makes its temp names
  acceptable to MediaProvider). 2. XMP sidecars are never written into Pictures: the sidecar option is disabled there
  with an explanation, and a sidecar write that can't be placed gives an actionable error. 3. `file.share`: after
  export, look up the MediaStore content URI of each file (query by `RELATIVE_PATH` + `DISPLAY_NAME`, local frame per
  row) and start `Intent.createChooser(ACTION_SEND / ACTION_SEND_MULTIPLE)` with `EXTRA_STREAM` and
  `FLAG_GRANT_READ_URI_PERMISSION` (no FileProvider: that is Java). Enabled only with the service.
- **Tests:** `export_dir_for_images_and_documents`, `share_spec_single_and_multiple`, `uri_lookup_selection_for_path`
  (pure); headless `share_disabled_without_service`, `share_calls_service_with_exported_paths`,
  `sidecar_into_pictures_is_refused_with_reason`.
- **Verify:** `cargo test -p lightcraft-android-sys -- share export_dir && cargo test -p lightcraft-ui-egui -- share
  sidecar && cargo xtask parity && cargo xtask ci`
- **Depends on:** APK-6, APK-9, APK-25  **Needs:** rust-target  **Model:** opus — MediaStore and grants.
- **Done when:** tests pass; parity row added.
- **Maintainer checks on a device:** export a JPEG and a TIFF to Pictures/LightCraft (no stray temp file left), a
  Contact Sheet PDF to Documents/LightCraft; share to a messenger; the photo arrives.

#### APK-28 System clipboard
- **Goal:** `TextField` Cut / Copy / Paste reach other apps.
- **Files:** changed `crates/ui-egui/src/lib.rs` (`Services::clipboard: Option<ClipboardFns>` get/set),
  `crates/ui-egui/src/text_field.rs`, new `crates/android-sys/src/clipboard.rs`, changed
  `crates/android-host/src/services.rs`.
- **Steps:** with the service, Copy sets `ClipboardManager.setPrimaryClip(ClipData.newPlainText(…))` and Paste reads
  `getPrimaryClip()` → `getItemAt(0).coerceToText(ctx)` (on the Java main thread, result over a channel); without it,
  egui's clipboard as today. Reuse APK-25's JNI template (local frame, error mapping).
- **Tests:** headless `text_field_copy_uses_clipboard_service`, `paste_reads_clipboard_service` (fake).
- **Verify:** `cargo test -p lightcraft-ui-egui -- clipboard && cargo xtask android check && cargo xtask ci`
- **Depends on:** APK-25  **Needs:** rust-target  **Model:** opus — new JNI calls testable only on a device.
- **Done when:** tests pass.  **Maintainer checks on a device:** copy a keyword into another app and back.

## Phase 3: touch UI

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

#### APK-31 Safe-area and keyboard insets
- **Goal:** with targetSdk 36 (edge-to-edge enforced) nothing sits under the status bar, cutout, gesture bar or
  soft keyboard.
- **Files:** new `crates/android-sys/src/insets.rs`, `crates/android-host/src/insets.rs`; changed
  `crates/android-host/src/{app.rs,lifecycle.rs}`, `crates/ui-egui/src/lib.rs` and the top-level layout code that uses
  `ctx.screen_rect()`/`viewport_rect()`.
- **Steps:** 1. android-sys: `getWindow().getDecorView().getRootWindowInsets().getInsets(WindowInsets.Type
  .systemBars() | Type.displayCutout() | Type.ime())` (API 30+), run on the Java main thread via
  `run_on_java_main_thread`, returned over a channel and cached in an `Arc<Mutex<Insets>>`. 2. Refresh on `Resized`,
  `ScaleFactorChanged`, `Focused(true)`, and every 250 ms (`request_repaint_after`) while a TextField has focus.
  3. android-host `insets.rs` (pure): `insets_px_to_points(px, pixels_per_point)`; `raw_input_hook` sets
  `RawInput::safe_area_insets` (precedents: apps/lightcraft/src/main.rs:76, apps/lightcraft-web/src/web.rs:657).
  4. ui-egui lays out its outer frame inside `ctx.content_rect()` and scrolls a focused TextField above the bottom
  inset. `content_rect()` comparison is not used (equals the window under edge-to-edge).
- **Tests:** `insets_px_to_points` (pure); headless `no_widget_in_inset_band` with fake insets at `--size 412x915
  --scale 2.625`, `focused_field_stays_above_keyboard_inset`.
- **Verify:** `cargo test -p lightcraft-android-host -- insets && cargo test -p lightcraft-ui-egui -- inset && cargo
  xtask android check && cargo xtask ci`
- **Depends on:** APK-18, APK-30  **Needs:** rust-target  **Model:** opus — JNI on the main thread plus layout.
- **Done when:** tests pass.  **Maintainer checks on a device:** portrait and landscape on a phone with a cutout;
  keyboard open over the keyword field.

#### APK-32 Tablet landscape layout
- **Goal:** usable at 1280×800 pt with touch.
- **Files:** changed `crates/ui-egui/src/` layout modules named by the failing test; new
  `crates/ui-egui/tests/fixtures/layout-1600x1000.json` (desktop fingerprint, text).
- **Steps:** breakpoint by width in points; panels resizable by touch; every interactive rect ≥ 48 dp (Material).
  Tests use the widget rects the app already records (`app.widgets` / `ui.inspect`) through the CPU headless harness
  (`headless::Harness`), not `egui_kittest` snapshots (wgpu, no adapter here) or committed PNG baselines.
- **Tests:** `min_hit_size_48dp_at_tablet`; `desktop_layout_fingerprint_unchanged` (rects at 1600×1000 vs the
  committed JSON).
- **Verify:** `cargo test -p lightcraft-ui-egui -- min_hit_size desktop_layout && cargo run -p lightcraft-cli --
  snapshot --demo --size 1280x800 --scale 2 -o target/android/tablet.png`, then Read the PNG.
- **Depends on:** APK-31  **Needs:** none  **Model:** sonnet — layout tuning with structural tests.
- **Done when:** tests pass; desktop fingerprint unchanged.

#### APK-33 Phone portrait: side panels as sheets
- **Goal:** usable at 412×915 pt (2.625 px/pt); side panels become bottom sheets, no second UI.
- **Files:** changed `crates/ui-egui/src/` panel host modules, `crates/ui-egui/src/menus.rs` (`view.back` arm);
  new `crates/ui-egui/src/sheet.rs`.
- **Steps:** below the phone breakpoint, Library/Develop panels render inside a sheet with a drag handle; `view.back`
  (APK-17) closes the sheet first (a new arm at the front of the cascade, desktop unaffected).
- **Tests:** `side_panels_become_sheet_below_breakpoint`, `back_closes_sheet_first`, `min_hit_size_48dp_at_phone`;
  `desktop_layout_fingerprint_unchanged` stays green.
- **Verify:** `cargo test -p lightcraft-ui-egui -- sheet min_hit_size desktop_layout && cargo run -p lightcraft-cli --
  snapshot --demo --size 412x915 --scale 2.625 -o target/android/phone.png`, then Read the PNG.
- **Depends on:** APK-17, APK-32  **Needs:** none  **Model:** opus — UI structure change across panels.
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
- **Goal:** a written go/no-go on text input under NativeActivity.
- **Files:** changed `docs/android-port.md` (result), `crates/android-sys/src/dialog.rs` (text-input variant),
  `crates/ui-egui/src/text_field.rs` only if needed.
- **Steps:** 1. Confirm the keyboard shows and hides with `TextField` focus (egui IME output → winit
  `set_ime_allowed`). 2. Document what works: Latin key events; what doesn't: composition (CJK), autocorrect, swipe
  (android-activity `text_input_state`/`set_ime_editor_info` are no-ops in native-activity mode). 3. Fallback
  without Java: an `AlertDialog` with an `EditText`, built through JNI on the Java main thread, `null` button
  listeners, `isShowing()` / `getText()` polled each frame. 4. Feed decision 8.
- **Tests:** headless `focus_requests_ime` (IME output set while a TextField has focus).
- **Verify:** `cargo test -p lightcraft-ui-egui -- ime && cargo xtask android check && cargo xtask ci`
- **Depends on:** APK-19, APK-31  **Needs:** rust-target (device for the result)  **Model:** opus — open-ended spike.
- **Done when:** the doc states the result and the maintainer decision it needs.
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
  `xtask/src/android/apk.rs` (release signing).
- **Steps:** 1. `assets.rs`: add the extensions/names to the rejected list with a message pointing at the env
  variables. 2. `apk.rs`: release signing reads `ANDROID_KEYSTORE_B64`, `ANDROID_KEYSTORE_PASSWORD`,
  `ANDROID_KEY_ALIAS`, `ANDROID_KEY_PASSWORD`; decodes the keystore to a temp file outside the repo (`$RUNNER_TEMP`
  or the OS temp dir), removed at the end even on error; `apksigner --ks-pass env:ANDROID_KEYSTORE_PASSWORD
  --key-pass env:ANDROID_KEY_PASSWORD` (never passwords on argv); a missing variable is a clear error.
- **Tests:** `assets_rejects_keystore_files`, `release_signing_requires_env`, `signing_args_never_contain_passwords`.
- **Verify:** `cargo test -p xtask -- assets apk && cargo xtask assets`
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
- **Files:** new `xtask/src/android/aab.rs`; changed `xtask/src/android/mod.rs`.
- **Steps:** 1. `aapt2 link --proto-format` (same res and manifest as APK-15) → restructure with `zip_lib.rs` into
  `base.zip`: `manifest/AndroidManifest.xml`, `res/`, `resources.pb`, `lib/arm64-v8a/liblightcraft_android.so`.
  2. bundletool jar pinned by version and sha256 from maven.google.com
  (`com/android/tools/build/bundletool/<v>/bundletool-<v>.jar`), cached under `target/android/tools/`.
  3. `java -jar bundletool.jar build-bundle --modules=base.zip --output=app.aab` → `validate`. 4. Sign with
  `jarsigner -storepass:env ANDROID_KEYSTORE_PASSWORD -keypass:env ANDROID_KEY_PASSWORD` (apksigner doesn't sign
  AABs), keystore from APK-37's temp file. 5. Check: `bundletool build-apks --mode=universal` then `zipalign -c -P 16`
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
| 2 | **Java shims.** Without Java: MediaStore + "Open with", no SAF picker (`onActivityResult`; in-app picker, APK-24), no `onNewIntent` (a second "Open with" while running is ignored), no IME composition, no foreground service for background jobs. A few small Java classes (hasCode=true) would fix each, bending "pure Rust in the product". | no Java | APK-22, 24, 25, 35 |
| 3 | **minSdk / targetSdk.** minSdk 30: shared media by path (FUSE), settled scoped storage, `WindowInsets.Type` API; devices launching with Android 10+ (64-bit) ship Vulkan 1.1 (upgraded devices may not: GLES fallback). targetSdk tracks Play's rule: 36 for new apps and updates since 2026-08-31 (re-check on the day). | minSdk 30, targetSdk 36 (android-36 platform, build-tools ≥ 36) | APK-13, 15 |
| 4 | **Repository branding.** The README header, `docs/brand/` and desktop packaging metadata still name ArtCraft; fine while mirroring upstream, to be replaced before this fork publishes builds. | unchanged | — |
| 5 | **ABIs.** arm64-v8a for release; x86_64 only for the CI emulator. | as stated | APK-12, 16, 40 |
| 6 | **Packaging route.** (a) SDK build-tools driven by xtask (aapt2 → zip → zipalign → apksigner; bundletool + jarsigner for AAB): no Gradle, no wrapper jar. (b) A declarative Gradle/AGP project: familiar to F-Droid but heavier, needs the `gradle-wrapper.jar` blob and pinned AGP/Gradle/JDK 17+. | (a) | APK-15, 39, 40 |
| 7 | **System destroy and re-entry.** When the Activity is destroyed while the process lives ("Don't keep activities", a missed configChange, removal from recents), the host detects it (APK-18), leaves the event loop and exits the process, so the next launch is fresh. Option "restart": before exit, `startActivity(Intent.makeRestartActivityTask(launchComponent))` via JNI (no Java class) so one tap reopens when the destroy came from a relaunch. | exit when the destroy is detected; start guard exits as backstop | APK-18 |
| 8 | **Text input.** After APK-35: NativeActivity's limits, GameActivity (full IME, but C++ via `cc`), or a small Java InputConnection (decision 2). | NativeActivity + dialog fallback | phase 3 follow-ups |
| 9 | **Library location.** Internal storage (private, `allowBackup="false"`, **deleted on uninstall or "Clear data"** with every edit and Copy-imported original) vs app-specific external or `Pictures/` (survives less, visible to others). | internal, with a first-start notice saying uninstall deletes the library and how to back it up | APK-11 |
| 10 | **SAM on Android.** The SAM worker model holds several GB (engine/src/segment/worker.rs:9). | ship `sam`; the command is enabled only when MemTotal ≥ 8 GB, otherwise disabled with an explanation | APK-11 |
| 11 | **craft-fonts in the APK.** CJK faces add tens of MB. | release builds embed them (as desktop) with their licences (APK-38) | APK-40 |

## Risks

| Risk | Mitigation |
|---|---|
| A system-initiated Activity destroy while the process lives: `onDestroy` waits for `android_main`, which winit never ends (ANR) | full `configChanges` + `singleTask` + Back → `moveTaskToBack` make it rare; when it happens the host sees `isDestroyed()`/`isFinishing()`, flushes, leaves the loop and exits (APK-18, decision 7); maintainer check with "Don't keep activities"; report upstream to winit |
| Cached-apps freezer: a backgrounded process is frozen within seconds, so long imports/exports pause and their `TaskGuard` row looks stalled | `docs/android.md` explains it; jobs resume when the app returns; optional follow-up: `AndroidApp::set_window_flags(KEEP_SCREEN_ON)` while a long job runs, behind a Services hook (a foreground service needs Java, decision 2) |
| Low-memory killer ends the process (a native kill, not catchable) | phone budget from `/proc/meminfo` via `memory::set_budget` (APK-11), `memory_warning` → `trim_caches` + GPU pool trim (APK-18), flush on focus loss, SAM gated by RAM (decision 10) |
| JNI local-reference overflow aborts on large libraries | `with_local_frame` per iteration (APK-10 rule, APK-23/25/27) and a 20k-photo device check |
| No IME composition under NativeActivity (CJK, autocorrect, swipe) | APK-35 spike, dialog fallback, decision 8 |
| GPU drivers vary (Vulkan bugs, crashes in device creation) | wgpu Vulkan with GLES compiled in, start markers so the next launch tries GL first, compute-GPU crash sentinel, CPU develop pipeline, failure dialog that waits (APK-19) |
| Shared storage rules (MIME per folder, read-only DCIM originals) | extension-keeping temp names (APK-9), image/document export folders, no sidecars in Pictures (APK-27), Copy import default (APK-23) |
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
