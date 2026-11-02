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

## When the graphics won't start

The window draws with wgpu: Vulkan first, then OpenGL ES (`crates/android-host/src/renderer.rs`). Before eframe starts,
the host writes `files/config/window-init.marker` naming the renderer; it is removed once the window is up. Logcat shows
the choice (`window: renderer vulkan (order ["vulkan", "gl"])`) and, once the window is up, the adapter (`window:
adapter … (Vulkan, …), driver …`).

- **The renderer fails** (an error eframe logs, or a panic): an `AlertDialog` says why and where the log is
  (`files/logs`). The native thread waits until it is closed (at most 10 minutes; the Activity's lifecycle stays
  answered meanwhile), then the process exits. The marker stays behind, so the next start puts that renderer last and
  tries the other one first, and a notice in the app says so.
- **The process dies inside the driver** (no dialog is possible): the marker stays too, so the next start tries the
  other renderer.
- **The GPU compute device died at start, or the GPU was lost while LightCraft ran** (`gpu-init.marker`,
  `gpu-lost.marker`, as on the desktop): the next start renders photos on the CPU and says why;
  Settings ▸ Performance ▸ Use the GPU for rendering turns the GPU back on.

Forcing a backend: `adb shell setprop debug.lightcraft.gpu_backend gl` (see "Diagnostics switches" below). A forced
renderer that the last start died in is not tried first again, so the next tap on the icon opens on the other one even
though the property is still set.

## Diagnostics switches (system properties)

An app gets no environment variables, so the desktop's switches are system properties. Only the shell can set a
`debug.*` property, so other apps cannot. They are read once, when LightCraft starts (before the first log line); set
one, then start LightCraft again (`adb shell am force-stop io.github.n0x4e52.lightcraft.debug`). `adb shell setprop
<name> ""` clears one. A value that means nothing is ignored.

| Property | Values | Same as on the desktop |
|---|---|---|
| `debug.lightcraft.profile` | `1`, `true`, `on` or `yes` | `LIGHTCRAFT_PROFILE`: per-stage pipeline timings in logcat |
| `debug.lightcraft.log` | `debug`, `info` or `warn` | `LIGHTCRAFT_LOG`: the level of LightCraft's own crates |
| `debug.lightcraft.gpu_backend` | `vulkan`, `gl`, `auto`, `off`, or a list such as `vulkan,gl` | `LIGHTCRAFT_GPU_BACKEND`: the window uses the one renderer named (a list, `auto` and `off` leave the window's Vulkan, then GL order alone); GPU compute uses the backends listed, or none for `off` |
| `debug.lightcraft.control` | a port, 1 to 65535 | `--control <port>`: starts the debug control server, with a token (next section) |

The code is `crates/android-sys/src/props.rs` (`settings_from` is pure and tested; `read` asks the system) and the
start of `run` in `crates/android-host/src/run.rs`; the log line `diagnostics from system properties: …` shows what was
applied.

## Debug control server

Drive and screenshot the app on a device with the usual JSON-lines scripts ([`control-protocol.md`](control-protocol.md)).
Every app on a device shares loopback, so the server is off by default, only the shell can switch it on, and every
request must carry a token:

```sh
adb shell setprop debug.lightcraft.control 18555          # the port; no default, nothing listens without it
adb shell am force-stop io.github.n0x4e52.lightcraft.debug
adb shell am start -n io.github.n0x4e52.lightcraft.debug/android.app.NativeActivity
adb forward tcp:18555 tcp:18555
token=$(adb shell run-as io.github.n0x4e52.lightcraft.debug cat files/control-token | tr -d '\r')
python3 docs/showcase/run.py --port 18555 --token "$token" script.jsonl     # e.g. {"method":"ui.screenshot","params":{"path":"/data/user/0/<id>/files/shot.png"}}
```

The app writes a new random 128-bit token (32 hex digits) to `files/control-token` at each start with the property set,
readable by the app alone (`run-as` on a debug build), and deletes a stale one when the property is not set. A request
without the matching top-level `"token"` string gets `{"ok": false, "error": "bad token"}` and the connection is
closed; a token that cannot be made or written means no server at all. `adb shell setprop debug.lightcraft.control ""`
and a restart turn it off. The server listens on 127.0.0.1 only; it shares its code with the desktop's `--control`
(`crates/ui-egui/src/host/control_server.rs`), which takes no token. The manifest holds `android.permission.INTERNET`
for this and for model downloads (`crates/fetch`). The `emulator` job runs `packaging/android/control-check.sh` after
the smoke test: it does exactly this and sends one `ui.inspect`, then one without the token (must be refused);
`smoke/control.txt` in the `lightcraft-smoke` artifact is its output. To pull a screenshot, write it under `files/` and
`adb exec-out run-as <id> cat files/shot.png > shot.png`.

## Continuing on a Mac

Everything the cloud agents did is on the branch `ccr-9ce32afe-lqdb0r` of `0x4E52/lightcraft-apk`, one commit per
task (`APK-<n>: …`); the next task is the first open card in [`android-port.md`](android-port.md) → "Order of the
remaining work". A Mac, unlike the agent container, can download the SDK and NDK, so it builds, installs and runs the
APK itself.

### One-time setup (Apple Silicon or Intel)

```sh
# Rust and the targets
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup target add aarch64-linux-android x86_64-linux-android wasm32-unknown-unknown

# JDK (keytool for the debug key) and the Android command-line tools
brew install openjdk@21
brew install --cask android-commandlinetools android-platform-tools   # android-platform-tools = adb
export ANDROID_HOME="$(brew --prefix)/share/android-commandlinetools"
sdkmanager --sdk_root="$ANDROID_HOME" "platforms;android-36" "build-tools;36.0.0" "ndk;28.2.13676358"
export ANDROID_NDK_HOME="$ANDROID_HOME/ndk/28.2.13676358"
# put the two exports in ~/.zshrc; Android Studio's SDK (~/Library/Android/sdk) works the same way
```

`sdkmanager --sdk_root="$ANDROID_HOME" --list | grep ndk` shows the NDK versions on offer. Any NDK r28 or newer works (`xtask/src/android/so.rs` adds the 16 KB page flag itself for older ones); if sdkmanager
lists a newer r28+ revision, use that and adjust `ANDROID_NDK_HOME`.

### Get the code and check it

```sh
git clone https://github.com/0x4E52/lightcraft-apk && cd lightcraft-apk
git checkout ccr-9ce32afe-lqdb0r
cargo xtask android check        # lint everything the APK links
cargo xtask ci                   # the full gate (as GitHub's ci.yml runs it); a Mac is faster than the container
```

### Build, install, run on your phone

```sh
cargo xtask android so --abi arm64-v8a
cargo xtask android apk --abi arm64-v8a
adb install -r target/android/lightcraft-debug.apk   # phone with USB debugging on
adb shell am start -n io.github.n0x4e52.lightcraft.debug/android.app.NativeActivity
```

On an Apple Silicon Mac the Android emulator runs arm64 images, so use `--abi arm64-v8a` there too (the CI emulator
is x86_64 because GitHub's runners are). Headless snapshots of the phone layout need no device:
`cargo run -p lightcraft-cli -- snapshot --demo --size 412x915 --scale 2.625 -o phone.png`.

### Continue with Claude Code on the Mac

The rules the agents followed are in the repository: `CLAUDE.md` (fast gate per Android task), `docs/android-port.md`
(cards, "Executing in this container", decisions) and `docs/android-touch-ui.md` (the Lightroom-like touch UI). On
a Mac the container limits don't apply: the SDK, NDK and a device are there, so a card's NDK, SDK and device steps run
locally instead of only in CI. To go on task by task, ask Claude Code in the repository:

> Continue the Android port: take the next open card from docs/android-port.md → "Order of the remaining work",
> implement only that card, run its fast gate, commit it as `APK-<n>: …`, and push.

For several cards in a row with an implementer, an adversarial reviewer and a fix round per card, run the workflow
in [`tools/agents/android-runner.js`](../tools/agents/android-runner.js) (see the comment at its top).

## Importing from the photo library

File ▸ Import from Photo Library… (`file.importMediaLibrary`) asks for the photo permission the first time (allow
all photos, or only the ones you pick on Android 14+), then reads the phone's photo library (MediaStore) in the
background, with "Reading the photo library" in the activity stack. The import review lists the library's folders
(Camera, Screenshots, …); tap one and its photos appear in the review, as for a folder chosen in Import from
Folder…, except that only the folder's own photos are scanned: its subfolders are folders of their own in the list. LightCraft reads the photos by their path in shared storage (`/storage/emulated/0/DCIM/…`), so previews,
edits, metadata and GPS (with the media location permission) work as for any file.

Limitations:
- **Copy is the default, and Add in place is read-only.** Android lets LightCraft read other apps' photos but not
  change them: for photos added in place, writing XMP sidecars next to them, renaming, moving and deleting fail
  (the error says to import with Copy). Copy puts the photos in the library's own folder, where all of that works.
- The photo library is the phone's main storage: an SD card or USB drive is imported from File ▸ Import from
  Device (next section).
- Raws that the system does not list as images (many `.CR3`, `.ARW`, `.NEF` files) don't appear: share them to
  LightCraft instead (APK-25).
- Denied for good: the import says "allow it in Settings ▸ Apps ▸ LightCraft ▸ Permissions"; after allowing it
  there, choose Import from Photo Library again.

## SD cards and USB drives

A microSD card in the phone, or a card reader or USB drive on the USB-C port (OTG), appears under File ▸ Import from
Device and at the end of the Library sidebar's Local section, under the name Android gives it ("SanDisk SD card",
"USB drive"), as soon as it is mounted: LightCraft asks Android's `StorageManager` for the mounted removable volumes
every two seconds while a menu or the Local section shows them, and again when it comes back to the front. Only a
volume with a `DCIM` folder (what every camera writes) is listed; the phone's own storage is in Local already (DCIM,
Pictures, Download).

- **Import from Device** reviews the card's DCIM folder and copies the photos into the library, verified (each copy
  is read back and checked before it counts), as on the desktop. Card files the library already has are skipped.
- **Browse** (tap the card in Local) shows its photos where they are, without copying; edits are kept in the
  library, never written to the card.
- Taking the card out during an import ends it with an error for each photo that couldn't be read ("2 not
  readable"); the photos copied before stay, nothing is half-copied, and LightCraft keeps running.

What is readable on a card: with the photo permission (asked by the first import), Android shows LightCraft the
card's photos and videos by path, JPEG, HEIC, PNG, DNG and the raws its media database knows. Files it doesn't
classify as media (on many phones `.CR3`, `.ARW`, `.NEF`, and every non-media file) are hidden from the listing, so
they don't appear in the review at all; APK-52 says so in the app. A card that Android has *adopted* as internal
storage is part of the phone's own storage and not listed as a device.

## Open with LightCraft and sharing

LightCraft is offered in "Open with" for photos and raws (JPEG, PNG, HEIC, DNG, CR2, CR3, NEF, ARW, other raws sent
as `application/octet-stream`) and in the share sheet for one or several files, which may also be presets (`.xmp`,
`.zip`), track logs (`.gpx`), keyword lists (`.txt`) and other files (`packaging/android/AndroidManifest.xml`). Once
the library is open, LightCraft copies what it was sent in the background, with "Receiving N files" in the activity
stack (cancellable), because the other app's permission to read the file ends with LightCraft's window
(`crates/android-host/src/incoming.rs`):

- **Photos and raws** are copied into the staging folder `files/incoming/` (next to the library, not inside it) and
  then imported with Move: they end up in the library's own `Originals/` folder (so editing metadata, renaming and
  deleting work), and the received copy is removed once the import verified it. A duplicate stays out of the
  library, and the staging folder is emptied at the next start (never a file the catalog uses).
- **Everything else** goes to the **Inbox** (next section): a notice names the files; open them with the command
  that uses them (Import Profiles & Presets…, Import Keywords…, Auto-Tag from Tracklog…, a face model) and choose
  Inbox in its chooser.
- The sender's file name is kept, made safe: only the name (no folders), no hidden or control characters, at most
  120 bytes, an extension from the file's type when it has none, and `-1`, `-2`… when the name is taken. Nothing in
  the Inbox is overwritten.
- A file that can't be received is named in the notices with why: access withdrawn, the file gone, reading stopped
  part way, the device full, larger than 2 GB, or cancelled. Its partial copy is removed.

Limitation: **a second "Open with" or share while LightCraft is already running only brings LightCraft to the
front; the file is not received.** NativeActivity (LightCraft has no Java code) does not get `onNewIntent`, and the
intent that started the window never changes while it lives. Leave LightCraft (Back to the background, then remove
it from the recent apps) and share again. Starting LightCraft again from the recent apps does not receive the same
files twice.

## Choosing files: the Inbox

Android has no file dialog LightCraft can open without Java, and under scoped storage the photo permission opens
only *media* files of other apps: presets (`.xmp`, `.lcpreset`, `.zip`), track logs (`.gpx`), keyword lists
(`.txt`), models (`.onnx`) and Lightroom catalogs a browser saved in `Download/` or `Documents/` are hidden from
LightCraft. So a command that asks for a file opens LightCraft's own chooser (`crates/ui-egui/src/pick_browser.rs`),
which shows only these places: **Inbox** (`files/inbox/`, where files shared to LightCraft land, APK-25),
**Library** (the library's folder), and the export folders **Pictures** (`Pictures/LightCraft`) and **Documents**
(`Documents/LightCraft`, APK-27), which hold only files LightCraft wrote. **Share the file to LightCraft from Files or the browser, then pick it from
Inbox.** The chooser never leaves those places: `..` and symbolic links are refused, hidden files are not listed.
Back closes it, choosing nothing.

| Command | Dialog | On Android |
|---|---|---|
| File ▸ Import Profiles & Presets… (`file.importPresets`; also the mask effects menu's Import Presets…) | `pick_preset_files` | in-app chooser (Inbox) |
| Point Curve ▸ Import Presets… (`file.importCurvePresets`) | `pick_curve_preset_files` | in-app chooser (Inbox) |
| File ▸ Import Keywords… (`file.importKeywords`) | `pick_keyword_list` | in-app chooser (Inbox) |
| Photo ▸ Auto-Tag from Tracklog… (`photo.tagFromTracklog`) | `pick_tracklog` | in-app chooser (Inbox) |
| Settings ▸ Faces ▸ Add a model file… (`dialog.faceModel`) | `pick_model_file` | in-app chooser (Inbox) |
| File ▸ Import Photos… / Import from Folder… (`file.addPhotos`, `file.addFolder`) | `pick_files`, `pick_folder` | in-app chooser; Import from Photo Library… is the usual way |
| Locate Missing File…, File ▸ Find Missing Photos… (`photo.locate`, `file.findMissing`) | `pick_files`, `pick_folder` | in-app chooser |
| Move to Folder… (`photo.moveToFolder`), Choose Layout Image… (`view.chooseLayoutImage`) | `pick_folder`, `pick_files` | in-app chooser |
| Open Library… (`app.openLibrary`), New Library… (`app.newLibrary`) | `pick_folder` | disabled: LightCraft keeps one library, inside the app |
| Settings ▸ General ▸ Open Library…, New Library… | `pick_folder` | disabled with why, as the menu items |
| Library problem window ▸ Choose Another Library… | `pick_folder` | in-app chooser; only a folder that already holds a LightCraft library (not the one that couldn't be opened, not one inside another library), opened behind the "Opening library…" frame; any other folder is refused with why and no library is created there. On a phone this normally finds nothing to open: the Library place is the library that couldn't be opened, its subfolders are inside it, and shares put single files in the Inbox, so until library backup and restore land Try Again is the way out |
| File ▸ Import Lightroom Catalog… (`file.importLightroom`) | `pick_lightroom_catalog` | disabled: the catalog points to originals on the computer it came from |
| Export Presets…, Export Effect Presets…, Point Curve ▸ Export Presets…, Export Keywords… | `save_preset_file`, `save_curve_preset_file`, `save_keyword_list` | disabled: they ask the in-app chooser like the PDFs do, but where their files (`.lcpreset`, `.xmp`, `.txt`) should land in Documents/LightCraft hasn't been decided or tested yet, so `pick_browser::unavailable` still lists them |
| Print to PDF, Export Contact Sheet PDF (`app.print`, `app.contactSheet` and their dialogs) | save dialogs | in-app chooser, opening in Documents/LightCraft (APK-27) |
| Export Slideshow (`app.slideshowExport` and its dialog) | save dialog | disabled, as above |
| Settings ▸ AI Denoise ▸ Install from file… | `pick_denoise_model` | disabled with why; the model list's downloads work |
| Settings ▸ Import ▸ Auto Import ▸ Watched folder Choose… | `pick_folder` | disabled with why: other apps can't save photos into a folder LightCraft can watch; share them or use Import from Photo Library… |
| Settings ▸ Display ▸ Choose Profile… | `pick_display_profile` | disabled with why; previews are shown as sRGB |
| Develop ▸ Soft Proofing profile Other…, Print's profile Other…, Settings ▸ Performance ▸ smart previews Choose Folder…, slideshow templates Import/Export, watermark and overlay images, the Export and Import dialogs' Choose… folder buttons, the Local sidebar's Browse Folder… and Move Folder To… | synchronous dialogs in panels | not shown (desktop only) |

A disabled command's menu item says why ("Not available on Android: …") as its tooltip, and run from the control
channel or MCP it answers the same reason. Known gaps: a touch screen shows that tooltip only where egui gets a hover
(a later Phase 3 card shows the reason on a tap), and the chooser's own texts and these reasons are not yet in the
translation catalogs (`crates/ui-egui/locales/`; they show in English).

## Exports and Share

Exports go where other apps (Gallery, Files, messengers) see them (APK-27): with no folder chosen, images (JPEG, PNG,
TIFF, PSD, WebP, AVIF, JPEG XL, DNG) go to **`Pictures/LightCraft`** and everything else (PDFs, original files with
their XMP sidecars) to **`Documents/LightCraft`**. The folders come from Android
(`Environment.getExternalStoragePublicDirectory`, so a work profile or a second user gets its own), with
`/storage/emulated/0/…` when it can't be asked. Android lets an app create only photos and videos in `Pictures/` and
`DCIM/`: there the Export dialog doesn't offer Original files (it says why), and an export of originals into them is
refused before anything is written. The Library sidebar's Local section lists DCIM, Pictures and Download.

**File ▸ Share…** (`file.share`) exports the selected photos with the previous export's settings (originals as JPEG),
then, under the activity row "Preparing to share" (✕ stops it), looks each file up in MediaStore and opens the
system's share sheet with them (one file: `ACTION_SEND`, several: `ACTION_SEND_MULTIPLE`, with read access granted
to the app chosen). `{"paths": [...]}` shares files already exported there. A file outside shared storage can't be
shared and the message says where to export it.

## Backing up the library before uninstalling

The library lives in LightCraft's private storage (`files/library`), and Android deletes it when LightCraft is
uninstalled or its data is cleared (APK-50). Back it up first:

- **File ▸ Back Up Library…** (also **Back Up Now** in the first-start notice) writes one zip,
  `lightcraft-library-backup-<date>-<time>.zip`, to **`Documents/LightCraft`**, under the activity row "Backing up the
  library" (✕ stops it and leaves no file). It holds the catalog (edits, albums, keywords, history), presets and
  library settings, and every photo copied into the library (Copy imports, received shares). Photos added in place,
  such as a DCIM folder browsed or added without a copy, stay where they are: the backup lists them by path
  (`backup.json`), so they come back as long as the files are still on the phone. When there isn't room the backup is
  refused before anything is written ("needs 3.2 GB, 1.1 GB free"). Once written, the share sheet opens, so the file
  can go to a computer, a card or a cloud folder of your choice: `Documents/` survives an uninstall, but not a reset
  of the phone. The format is the browser build's zip (`lightcraft_engine::backup`), at most 4 GB and 65,535 files per
  backup (no zip64 yet): a larger library is refused with that reason before anything is read or written. Only the
  photos in the library's `Originals` folder are copied; any other photo, also one elsewhere in the library's folder,
  is listed by path like a photo added in place.
- **File ▸ Restore Library from Backup…** opens the in-app chooser. After an uninstall and reinstall, share the zip to
  LightCraft (from Files, a messenger or the cloud app), so it lands in the Inbox, and pick it there: the new install
  no longer owns the zip the old one wrote to `Documents/LightCraft`, and Android's scoped storage generally won't let
  it read that file by path. Picking it in `Documents/LightCraft` works only for the install that wrote it. (Neither
  route has been tried on a device yet.) Under "Restoring the library", every entry is read and its checksum and
  each photo's content hash checked before anything is written; a damaged or altered backup changes nothing. The
  files are then written to `files/library.restoring`; a marker (`files/library.restore.json`) is written before
  anything moves, the current library moves aside to `files/library.before-restore`, the restored one takes its place
  and opens behind the loading frame. Only once it has opened is the previous library deleted; if it doesn't open, the
  previous one is put back and the restored copy removed. A crash anywhere in between is put right at the next start,
  before the library is opened: the previous library is moved back unless the swap had completed. A
  `files/library.before-restore` that no marker accounts for is never deleted: the next restore keeps it as
  `files/library.before-restore-<time>`.

## Clipboard

Copy and paste reach other apps (APK-28): every copy (a text field's Copy or Cut, a dialog's Copy to Clipboard, Ctrl+C
on a hardware keyboard) goes to Android's clipboard, and Paste in a text field, or Ctrl+V, reads it. The calls run on
LightCraft's own thread without waiting for Android's main thread; a paste arrives on the next frame. Android 10 and
later show the clipboard only to the app in front, so a paste works while LightCraft has the focus. Pastes are cut at
1 MiB.

## Privacy

LightCraft collects nothing and sends nothing about you or your photos: no telemetry, no analytics, no crash
reports, no account, no update check, no advertising id. It uses the network only for downloads you start, and the
list of them is code: `crates/fetch/src/uses.rs` (Settings ▸ Network shows it, and the "Never use the network"
switch there refuses every download and the remote denoise connection, in the app, the CLI and MCP (`app.networkOff`), with "Network use is off in Settings ▸ Network"). The test
`every_network_call_is_registered` (`crates/fetch/tests/uses_scan.rs`) fails when a source file opens a socket or
calls the downloader without naming one of those uses.

| Use | Starts with | Contacts |
|---|---|---|
| `denoise-model` | Settings ▸ AI Denoise ▸ Download | github.com (release file, pinned size and SHA-256) |
| `face-model` | Settings ▸ Faces ▸ Download | github.com, huggingface.co (pinned) |
| `sam3-model` | Masking ▸ Download the model | only mirrors you list (none by default) |
| `place-names` | Map ▸ Download place names | download.geonames.org |
| `remote-denoise` | Settings ▸ AI Denoise ▸ Remote computer | a computer you name, on your network |
| `control-server` | `--control` (on Android only in debug builds, with a token) | 127.0.0.1 only |
| `segment-worker` | Masking with a remote SAM endpoint set | 127.0.0.1 only |
| `mcp-connect` | `lightcraft-cli mcp --connect` | the address you give |

Downloads send no identifier and no data from your library: a plain GET with a user agent. The one exception is a Hugging Face token you enter yourself, which is sent only over HTTPS to huggingface.co. Nothing is uploaded, except photo tiles that remote denoise sends to the computer you name, and only when you turn it on.

### Data safety form (Google Play)

Answers, from the list above (APK-43 copies them):

- Does your app collect or share any of the required user data types? **No.** No data is collected, no data is
  shared with third parties.
- Is all of the user data encrypted in transit? **Not applicable** (no user data is transmitted). The downloads
  use HTTPS.
- Do you provide a way for users to request that their data is deleted? **Not applicable** (no data leaves the
  device; everything is in the app's folders, removed by uninstalling, or by Settings ▸ Storage).
- Network use to disclose in the store text: optional downloads of AI models and place names, started by the user
  and switchable off in Settings ▸ Network.

## Device checklist

What only a device (or a CI run an agent cannot start) can show. Whoever saw it ticks the line.

- [x] The debug APK from the Android workflow starts and renders on a real phone (maintainer, 2026-10-17); the
  layout is the desktop one until Phase 3.

- [ ] Privacy (APK-57): Settings ▸ Network lists the eight uses; with Never use the network on, a model download
  on the phone says so and starts nothing.
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
- [ ] Renderer start (APK-19): the CI logcat (`lightcraft-smoke` artifact) shows the adapter line (`window: adapter
  …`) after `window: renderer …`, and no failure dialog.
- [ ] Diagnostics switches (APK-20): `adb shell setprop debug.lightcraft.gpu_backend gl`, restart: logcat shows
  `window: renderer gl` and the UI runs on GL; `setprop … vulkan` again: Vulkan. `setprop debug.lightcraft.log debug`:
  logcat shows debug lines of LightCraft's crates. `setprop debug.lightcraft.profile 1`: per-stage timings appear.
- [ ] Renderer failure (APK-19, needs APK-20): make the first renderer fail (an emulator image without Vulkan with
  `debug.lightcraft.gpu_backend vulkan`), the failure dialog shows, dismiss it, tap the icon **once**: the app opens on
  GL, with the notice that the window could not start with its Vulkan renderer last time.
- [ ] Links (APK-26): Help ▸ LightCraft Help and About ▸ Source code on GitHub open the browser. Only `https` links
  open; with no browser installed the notice says "No app can open links".
- [ ] Shares (APK-25): share from Google Photos (one photo, then several), from Files (Downloads) and from an app
  that uses a FileProvider; "Open with" a DNG from Files: all are imported into `Originals/`. Share a 1 GB file: the
  UI stays responsive and the activity row "Receiving 1 file" shows it until it is done. Share a `.xmp` preset: a
  notice says it is in the Inbox. Share again while LightCraft runs: it only comes to the front.
- [ ] Exports and Share (APK-27): export a JPEG and a TIFF with no folder chosen: both are in
  `Pictures/LightCraft` (Gallery shows them) and no `.lightcraft-…-tmp` file is left; a Contact Sheet PDF saved
  from the chooser's Documents place is in `Documents/LightCraft` (Files shows it). File ▸ Share… one photo, then
  three, to a messenger: the photos arrive. The Export dialog with a folder in Pictures doesn't offer Original files.
- [ ] Library backup (APK-50): back up a 500-photo library (File ▸ Back Up Library…), the zip is in
  `Documents/LightCraft` and the share sheet opens; uninstall, reinstall, share the zip to LightCraft and restore it
  from the Inbox: edits, albums and Copy-imported originals are back.
- [ ] Clipboard (APK-28): copy a keyword from the keyword field (long press or right-click ▸ Copy) and paste it into
  another app; copy text in another app and paste it into the keyword field with the field's Paste; with a hardware
  keyboard, Ctrl+C and Ctrl+V do the same. The UI doesn't stall on a paste.
- [ ] File pickers (APK-24, after APK-25): share a `.xmp` preset from Files to LightCraft, then File ▸ Import
  Profiles & Presets… ▸ Inbox imports it; the chooser's rows are finger-sized and Back closes it.
- [ ] Debug control server (APK-21): the recipe in "Debug control server" returns a screenshot; the same request
  without the token is answered `bad token`.
- [ ] CI `ui.inspect` over the forwarded port answers (APK-21): the `android.yml` `emulator` job's `control-check.sh`
  step passes; `smoke/control.txt` in the `lightcraft-smoke` artifact shows `with token: ui.inspect {… "ok": true …}`
  and `without token: … bad token`.
- [ ] APK-30b: scroll a 1000-photo grid by finger; long press a photo, tap three more, then share them.
- [ ] APK-31: on a phone with a cutout, portrait and landscape: nothing sits under the status bar, cutout or gesture bar; open the keyboard over a text field (e.g. the keyword field) and the field stays visible above it, and the layout returns when the keyboard closes.
- [ ] APK-54: after importing 1,000 photos, Settings ▸ Storage's total agrees with Settings ▸ Apps ▸ LightCraft ▸
  Storage within the system's rounding; Clear on the thumbnail cache, smart previews and Inbox frees the space App
  info then shows; a fresh install starts with the 512 MB thumbnail cache limit.
- [ ] APK-54: with under 1 GB free (fill the phone), the thumbnail and denoise caches shrink to half their limits within a minute and a notice says so (probe: `statvfs` through `storage_report::free_bytes`).
- [ ] APK-51: a USB-C card reader with JPEG + raw folders and a phone with a microSD in it: both appear under File ▸
  Import from Device and in Local with their names within two seconds of mounting; Import verifies and copies; Browse
  shows the photos in place; unplugging mid-import gives "not readable" errors and no crash (logcat: no `panicked`).
- [ ] Import from Photo Library (APK-23): import a JPEG folder and a DNG folder from the library; previews and edits
  work and GPS shows on the map; with Add in place, Save Metadata to File fails with the "Import the photo with Copy"
  error; a 20k-photo library lists its folders without a local-reference abort, the activity row showing while it
  reads.
- [ ] APK-22 (asked from APK-23's import): on API 33+, allow all (the import lists every bucket), allow selected
  photos on 34+ (`photo access: Pending → Partial` in logcat), deny (`→ Denied`), deny twice then import again (no
  dialog appears; within a second the import's error says to allow it in Settings ▸ Apps ▸ LightCraft ▸
  Permissions); allow it there and return: logcat shows `→ Granted`. On an API 30–32 device, allow and deny.
