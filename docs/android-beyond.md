# Android: beyond Lightroom (task cards APK-50 …)

*Last updated: 2026-11-02.* Revision history at the end.

Task cards that answer what people criticise in Lightroom mobile and its rivals
([`android-competitive.md`](android-competitive.md)). They follow the port plan's rules and card format
([`android-port.md`](android-port.md) → "Ground rules for the port", "Executing in this container", "Task card
format") and its decisions, which stay in force. Every card names the complaint it answers, links its section, and
says whether it **blocks a first public beta** of the APK.

Constraints for every card (from `CLAUDE.md` and the port's ground rules): pure Rust, no Java (a card that needs Java
names decision 2 and uses the no-Java default until decided), no cloud service, `unsafe` only in `crates/android-sys`
(and the entry symbol in `apps/lightcraft-android`), never crash (each card's error paths are tested), everything is
a command (an action is a command id with `enabled()`, a menu entry and a headless test with a fake service).

## What LightCraft already has

Each opportunity in [`android-competitive.md`](android-competitive.md#opportunities-for-lightcraft) was checked
against the code and [`parity-checklist.md`](parity-checklist.md) on 2026-10-31. Built, so no card (the Android cards
only connect them):

| Opportunity | Already there | Where |
|---|---|---|
| 1 No account, no tiers | every tool is free and local; the touch spec says so | `docs/android-touch-ui.md` §3 |
| 6 Offline, no licence check | LR-BEHAV-OFFLINE ✅; the network is used only for downloads the user starts (SAM, face and denoise models, GeoNames) | `crates/fetch/src/lib.rs`, `crates/engine/src/model_download.rs`, `crates/engine/src/cmd/geocode.rs` |
| 8 Every change saved | LR-BEHAV-AUTOSAVE ✅: each change is fsynced before the command returns; a failed write keeps it queued and retries | `crates/catalog/src/journal.rs`, `crates/engine/src/library.rs` |
| 2 Edit in place | LR-IMP-LOCAL ✅ (browse a folder without adding it; edits kept in the library, sidecars optional) and Add-in-place import; sidecar writes that fail wait in a retry queue instead of losing the edit | `crates/engine/src/cmd/browse.rs`, `crates/engine/src/sidecar.rs` (`unsaved_sidecars`), `Services::local_roots` |
| 9 Export | sizes up to 65,535 px with ppi (LR-EXP-DIM), JPEG / PNG / TIFF / PSD / WebP / AVIF / JPEG XL / DNG / Original + XMP, metadata policy with `removeLocation` (LR-EXP-METADATA), gain-map JPEG and PQ / HLG AVIF (LR-EXP-HDR 🟡), destination modes, batch export with no cap in the activity stack | `crates/engine/src/export.rs`, `crates/ui-egui/src/export_task.rs` |
| 16 Watermark presets | LR-EXP-WATERMARK ✅ and LR-PREF-WATERMARK ✅: text or image watermarks saved in export presets | `crates/engine/src/export.rs` (`Watermark`) |
| 14 Batch editing (part) | copy / paste with groups including masks (LR-MASK-SYNC ✅), Sync / Auto Sync (LRC-DEV-AUTOSYNC ✅), batch rating and presets (LR-BEHAV-BATCH ✅) | `crates/engine/src/cmd/develop.rs` |
| 18 Culling (part) | Assisted Culling (LR-LIB-CULL 🟡: focus score, bursts, best of group), Compare and Survey views | `crates/engine/src/cmd/cull.rs`, `crates/ui-egui/src/panels/compare.rs` |
| 19 Presets (part) | hover preview and live thumbnails in the Presets column | `crates/ui-egui/src/panels/presets.rs` |
| 15 Stylus (part) | Brush ▸ Pen Pressure scales size and flow from egui's touch `force` | `crates/ui-egui/src/panels/detail.rs` (`pen_force`, `record_pressure`), `crates/ui-egui/src/brush_live.rs` |
| 10 On-device AI (part) | Remove / heal / blemishes on the device, SAM gated by RAM (APK-11b), AI denoise with user-supplied models | `docs/android-touch-ui.md` §3, `docs/denoise.md` |
| 5 Cache (part) | thumbnail cache limit (512 MB–8 GB) and Clear Cache in Settings ▸ Performance (LR-PREF-LOCALSTORAGE ✅), denoise cache limits | `crates/ui-egui/src/panels/settings.rs`, `crates/ui-egui/src/panels/denoise.rs` |
| 4 Card import (desktop) | LR-IMP-DEVICE ✅: volumes with a DCIM folder, verified copies, duplicates skipped, background rescans | `crates/engine/src/devices.rs` (`devices_in`, `dcim_in`) |
| 3 Raw decoding | CR3 (CRX lossless and C-RAW) and Fujifilm compressed RAF decode (closed 2026-10-08); file type is decided by LightCraft's own import list, not by Android | `crates/raw/`, `lightcraft_engine::import::EXTENSIONS` |

Not built because the decoders are missing, so no Android card can fix them: the CR3 variants `crx.rs` refuses (gap 2),
compressed ORF (gap 6) and compressed SRW (gap 20) still fall back to the embedded preview
([`raw-parity.md`](raw-parity.md#variant-matrix-depth-per-format)). The Android cards below get these files *into*
LightCraft; how they render depends on those gaps.

## Folded into existing cards

Changes to cards in [`android-port.md`](android-port.md), applied when that card runs (its Files, Steps and Tests grow
as stated):

- **APK-23** (opportunity 2, answers [C6](android-competitive.md#c6-no-local-folders-weak-offline-use)): the bucket
  list also offers **Browse** (`library.browse` on the bucket's folder), next to Import, so DCIM photos are edited in
  place without a copy; edits stay in the library. On folders LightCraft can't write (other apps' files), sidecar
  auto-write doesn't queue a retry per edit: the photo is marked "sidecar not possible here" once and
  `unsaved_sidecars` stays empty. Tests: `bucket_browse_opens_local_view`, `readonly_folder_does_not_queue_sidecar_retries`.
- **APK-25** (opportunity 7, answers [C1](android-competitive.md#c1-sync-is-unreliable)): a share that carries a photo
  and an `.xmp` with the same stem routes the sidecar beside the copied photo, so the import reads it (desktop
  Export ▸ Original + Settings, then share to the phone). Test: `photo_and_sidecar_arrive_together`.
- **APK-27** (opportunity 9, answers [C7](android-competitive.md#c7-export-limits-and-bugs)): `file.share` exports
  through the same `app.export` options as the dialog (full size by default, the metadata policy, watermark presets),
  never a downsized copy. Tests: `share_keeps_metadata_and_gps`, `share_is_full_size_by_default`.
- **APK-41** (opportunity 13, answers [C9](android-competitive.md#c9-slowness)): the results table gets a budget-phone
  row (4 GB RAM or less), and the script checks that a slider drag stays on draft renders within 16 ms there.
- **APK-43** (opportunity 17): Play's Data safety answers come from APK-57's network list.

## Cards, ranked by user impact

| Rank | Card | Answers | Beta blocker |
|---:|---|---|---|
| 1 | APK-50 Library backup and restore on a phone | C2 (lost work), decision 9 | yes |
| 2 | APK-51 SD cards and USB drives | C5, C4 | yes |
| 3 | APK-52 Raws Android hides: say so, offer a way | C5 | yes |
| 4 | APK-53 Crash evidence: edit smoke script and last-crash report | C2 | yes |
| 5 | APK-54 Storage on a phone | C4 | yes |
| 6 | APK-55 Culling by swipe and the loupe gestures | C6, C8 | yes |
| 7 | APK-56 Finger strokes at full pressure, pen pressure from a stylus | C13 | yes |
| 8 | APK-57 Privacy you can check | C12 | yes |
| 9 | APK-58 Presets by touch | C8, requests | no |
| 10 | APK-59 Camera import over USB (MTP) | C5 | no |
| 11 | APK-60 Batch Auto and AI-mask update | C11 | no |
| 12 | APK-61 Slider gestures on the photo | C8, rivals | no |
| 13 | APK-62 Phone ↔ computer handoff without a cloud | C1 | no |
| 14 | APK-63 Left-handed and remembered layouts | C8 | no |
| 15 | APK-64 AI denoise on a phone | C10 | no |

### APK-50 Library backup and restore on a phone
- **Answers:** [C2](android-competitive.md#c2-crashes-and-lost-edits) (lost work). On Android the library lives in
  internal storage and is deleted by uninstall or Clear data (decision 9); the first-start notice (APK-11b) says so but
  offers no way out.
- **Goal:** File ▸ Back Up Library… and Restore Library from Backup… work on Android: one zip (catalog plus the
  Copy-imported originals) written to `Documents/LightCraft/` and offered to share; restore from the Inbox.
- **Files:** new `crates/engine/src/backup.rs` (the zip layout, verify and restore helpers moved from
  `apps/lightcraft-web/src/backup.rs`, wasm-safe, so web and Android share one format); changed
  `apps/lightcraft-web/src/{backup.rs,safety.rs}` (use the moved code), `crates/ui-egui/src/lib.rs` (the
  `backup_library` / `restore_library` doc comments: no longer web only), `crates/android-host/src/services.rs`,
  `crates/ui-egui/src/panels/notices.rs` (the first-start notice gains a Back Up Now button), `docs/android.md`.
- **Steps:** 1. Move the format's pure parts into the engine, unchanged on the web (its tests move with them).
  2. Android `backup_library`: a worker with `session.activity.start("export", "Backing up the library",
  Cancel::Flag(..))` streams the zip to `export_dir_for(Document)` (APK-27) through `safe_file`, checking free space
  first ("needs 3.2 GB, 1.1 GB free"), then offers `file.share` on the result. 3. `restore_library`: the in-app picker
  (APK-24) on the Inbox; verify every entry before anything is replaced; the current library is kept beside the
  restored one until the restore has opened. 4. Originals added in place (DCIM) are listed by path, not copied.
- **Tests:** `backup_roundtrip_restores_catalog_and_originals`, `backup_refuses_when_space_is_short`,
  `restore_rejects_tampered_entry` (engine, pure); headless `backup_shows_activity_row_and_offers_share` (fake
  service), `first_start_notice_offers_backup`; the moved web tests stay green.
- **Verify:** `cargo test -p lightcraft-engine -- backup && cargo test -p lightcraft-ui-egui -- backup first_start &&
  cargo xtask wasm && cargo xtask android check && cargo xtask ci`
- **Depends on:** APK-24, APK-25, APK-27  **Needs:** rust-target  **Model:** opus — data safety across two hosts.
- **Done when:** tests pass; `docs/android.md` explains backup before uninstall.
- **Maintainer checks on a device:** back up a 500-photo library, uninstall, reinstall, restore from the shared zip:
  edits, albums and Copy-imported originals are back.
- **As built (2026-11-01):** the format moved to `crates/engine/src/backup.rs` (zip writer and reader, entry names,
  checks, the `backup.json` manifest); the library-folder side is its own file, `crates/engine/src/backup_dir.rs`
  (plan on the UI thread, streamed write in 1 MiB pieces, verify, extract, rebase). The Android flow is shared code in
  `crates/ui-egui/src/library_backup.rs`, which the host installs as `Services::backup_library` /
  `restore_library`; those fields became `LibraryAction` (the app and the command's parameters, so the restore can
  ask the chooser and the backup can start `file.share`), the web's closures adapted. The catalog goes into the zip
  as the files on disk after a save on the UI thread (a compaction resets the log only there). Restore swaps
  `files/library.restoring` in for `files/library`, the previous one waits in `files/library.before-restore` with a
  marker (`files/library.restore.json`) until the restored one opened, then the originals are relinked (when the
  backed-up folder's path differs) and the previous one is deleted; a restored library that doesn't open gives the
  previous one back. The marker is written before the first move and records the phase (moving, swapped, rolled
  back); `library_backup::recover` runs in `open_library_session` before the library is opened, so a crash between
  the moves or a failed rollback moves the previous library back instead of creating an empty one, and a
  `.before-restore` without a marker is kept under a new name, never deleted. A restore writes originals only below
  `Originals/` (a crafted `backup.json` can't overwrite the catalog). A web backup (no `backup.json`) is refused on
  the device with where to restore it. No zip64: a library over 4 GB or 65,535 files is refused with that reason before the space check and before the
  worker starts (`Plan::check_limits`). Only originals below `Originals/` are copied (what a restore writes); any other
  photo inside the library folder is listed by path. The backup's row has kind `export`, the restore's `import`; a
  service that panics is caught in its command and stays installed.

### APK-51 SD cards and USB drives
- **Answers:** [C5](android-competitive.md#c5-raw-and-card-import-on-android) (OTG readers not listed, confused card
  volumes), [C4](android-competitive.md#c4-storage-bloat) (copies of everything).
- **Goal:** a card or USB drive with a DCIM folder appears under Devices and in the Local section the moment it is
  mounted; the user picks Import (verified Copy, the desktop path) or Browse (in place, no copy).
- **Files:** new `crates/android-sys/src/volumes.rs`; changed `crates/engine/src/devices.rs` (the scanner takes the
  host's mount list: `devices_in(parents, extra)` already accepts extra roots), `crates/ui-egui/src/lib.rs`
  (`Services::volumes: Option<VolumesFn>`, `None` = desktop mount parents), `crates/android-host/src/services.rs`,
  `docs/android.md`.
- **Steps:** 1. JNI `StorageManager.getStorageVolumes()` → for each mounted removable volume `getDirectory()`
  (API 30), `getDescription(ctx)` and `isRemovable()`, one local frame per volume, called from the device watcher's
  worker (attached thread). 2. A pure mapper turns the rows into `Device { name, path, root }` with the label
  ("SanDisk SD card", "USB drive") and drops duplicates (the same UUID listed twice). 3. Re-scan on `Focused(true)`
  and on the watcher's interval; a removed volume ends a running import with a per-file error, never a panic. 4. With
  the photo permission (APK-22) image files on the volume are readable by path (FUSE); files Android doesn't classify
  are not visible (APK-52 says so).
- **Tests:** `volume_rows_map_to_devices`, `same_volume_listed_once`, `non_removable_primary_is_not_a_device`
  (pure); headless `device_appears_in_import_and_local` (fake service), `volume_removed_mid_import_gives_file_errors`
  (fake reader that disappears).
- **Verify:** `cargo test -p lightcraft-android-sys -- volumes && cargo test -p lightcraft-engine -- devices && cargo
  test -p lightcraft-ui-egui -- device_appears volume_removed && cargo xtask android check && cargo xtask ci`
- **Depends on:** APK-22, APK-23  **Needs:** rust-target  **Model:** opus — JNI, a hot-plug watcher, error paths.
- **Done when:** tests pass; `docs/android.md` lists what is readable on a card.
- **Maintainer checks on a device:** a USB-C card reader with JPEG + raw folders and a phone with a microSD in it:
  both appear with their names, import verifies, unplugging mid-import gives errors and no crash.
- **As built (2026-11-02):** the device watch moved into the session (`Session::devices`, the desktop's shared
  mount-folder watch by default), so a host's volume list doesn't leak into another session and `library.devices`
  answers from it too. `Services::volumes` (`VolumesFn` = `engine::devices::MountsFn`, labels and mount points)
  makes `LightcraftApp::new` install `DeviceWatch::over_mounts`, where `devices_on` keeps the volumes with a `DCIM`
  folder, each mount point once. The pure mapper is `android-sys` `volumes::volumes` (mounted or read-only,
  removable, not primary, a plain absolute directory, each UUID or path once, at most 64; no description reads
  "Removable storage"); `query_volumes` reads each `StorageVolume` in its own local frame and a call that throws
  counts as no answer for that field. The host's `services::volumes` logs a failed query and lists nothing. The
  rescan on `Focused(true)` is shared code (`DeviceWatch::expire` on egui's `WindowFocused(true)`), so the desktop
  gets it too; Local lists the devices (volume roots) only with a host volume list. While Local or an Import from
  Device menu shows them on such a host, the frame asks to be drawn again after `devices::MAX_AGE`
  (`left::keep_devices_fresh`): the Android host draws only on demand, so without it the interval scan would wait
  for the next touch (headless: `device_appears_in_import_and_local` checks the wake, `desktop_devices_stay_out_of_local`
  that a desktop lists no devices in Local and asks for none). A removed card needs no new
  code: the import re-reads a file whose size changed or that is gone, and a copy that fails is a per-file error
  (the headless test pulls the card out after the import's first batch was copied: those photos stay, the rest
  fail). The watch's background scan runs the JNI query off the UI thread; the explicit queries (`library.devices`,
  `file.addFromDevice` without a path, the control channel's and MCP's forms; the menu always passes a path) scan
  on the calling thread, as on the desktop. The JNI calls have not run on a device.

### APK-52 Raws Android hides: say so, offer a way
- **Answers:** [C5](android-competitive.md#c5-raw-and-card-import-on-android) (CR3 / 3FR greyed out, files "missing").
  Under scoped storage the photo permission shows only files whose type Android's media database knows; on a phone
  whose database doesn't know `.cr3` (or `.3fr`, `.iiq`, …) those files are absent from directory listings, so no
  import code can see them.
- **Goal:** LightCraft tells the user which raw types this phone hides, and how to get them in anyway, instead of
  showing a folder that looks incomplete.
- **Files:** new `crates/android-sys/src/mime.rs`; changed `crates/ui-egui/src/lib.rs` (`Services::hidden_types:
  Vec<String>`, empty = desktop), `crates/ui-egui/src/import.rs` (a note row in the import review),
  `crates/ui-egui/src/panels/left.rs` (the same note in Local), `crates/android-host/src/services.rs`,
  `packaging/android/AndroidManifest.xml` (only under decision 12), `docs/android.md`.
- **Steps:** 1. At start (worker, once): for each raw extension in `lightcraft_engine::import::EXTENSIONS` ask JNI
  `MimeTypeMap.getSingleton().getMimeTypeFromExtension(ext)`; a type that isn't `image/*` is hidden on this phone.
  2. The import review and the Local header show "This phone hides CR3 files from apps. Share them to LightCraft from
  Files, or connect the camera by USB" (APK-25, APK-59), listing the hidden types. 3. **Decision 12 (new, proposed):**
  builds outside Google Play (F-Droid, sideload) may request `MANAGE_EXTERNAL_STORAGE` ("All files access", granted
  in system Settings, no Java) behind a Settings switch, which makes the hidden files visible by path; default: not
  requested. The card implements only the message until the maintainer decides.
- **Tests:** `hidden_types_from_mime_answers` (pure, table of answers), headless `import_review_names_hidden_types`,
  `no_note_when_nothing_is_hidden`, `desktop_never_shows_hidden_note`.
- **Verify:** `cargo test -p lightcraft-android-sys -- mime && cargo test -p lightcraft-ui-egui -- hidden_types
  no_note desktop_never && cargo xtask android check && cargo xtask ci`
- **Depends on:** APK-23, APK-25  **Blocked by:** decision 12 (step 3 only)  **Needs:** rust-target  **Model:** opus —
  scoped-storage facts that differ by Android version.
- **Done when:** tests pass; `docs/android.md` has the per-version facts that were observed, marked as such.
- **Maintainer checks on a device:** on an Android 13 and an Android 15 phone, a folder with CR2, CR3, NEF, ARW, RAF
  and 3FR: record which types the note names and which appear; share a hidden CR3 and it imports.
- **As built (2026-11-02):** decision 12 was decided (never requested), so step 3 adds nothing and the manifest is
  unchanged. The raw extensions are `lightcraft_engine::import::RAW_EXTENSIONS` (the camera raws of `EXTENSIONS`,
  DNG included). `android-sys` `mime::hidden_types` is the pure rule (an answer that isn't `image/*` hides the type;
  upper case, each once, at most 64) and `query_hidden_types` asks `MimeTypeMap.getSingleton()` once, then each
  extension in its own local frame (a call that throws counts as no answer). The host's `services::HiddenTypes` asks
  it once per process on the loader's worker, before the session is handed over (a reopen asks nothing again; a
  failed query is logged and names nothing), and `HostState::new` copies the answer into `Services::hidden_types`.
  The note (`import::hidden_types_note`, widgets `label:importHiddenTypes` and `label:localHiddenTypes`) says "Share
  them to LightCraft from the Files app, or open them with LightCraft"; "connect the camera by USB" waits for APK-59
  (MTP), since a camera in mass-storage mode is a USB drive whose raws are hidden the same way. `docs/android.md`
  "Raws this phone hides" marks the per-version facts as not observed yet; the JNI call has not run on a device.

### APK-53 Crash evidence: edit smoke script and last-crash report
- **Answers:** [C2](android-competitive.md#c2-crashes-and-lost-edits) (crashes on crop and masking, edits lost). The
  never-crash rules, the panic guard and the instant journal exist; what is missing is proof on Android and a way
  for a user to report a crash without any upload.
- **Goal:** the CI emulator runs a real edit session (crop, brush mask, copy/paste with masks, export), and after a
  crash the next start offers the panic text to copy or share.
- **Files:** new `docs/showcase/android-edit-smoke.jsonl`, `crates/ui-egui/src/panels/last_crash.rs`; changed
  `packaging/android/control-check.sh` (runs the script over the forwarded control port), `.github/workflows/android.yml`,
  `crates/ui-egui/src/host/log_file.rs` (`last_panic()` alongside `last_error()`), `crates/android-host/src/run.rs`,
  `docs/android.md`.
- **Steps:** 1. The script (control-protocol requests) opens the demo library, crops and straightens, paints a brush
  mask by `ui.touch`, copies settings with masks and pastes them on five photos, exports two JPEGs, and reads
  `ui.inspect`; it also runs headless on desktop. 2. The emulator job fails on any `panicked` line, a dead process or
  a command error the script doesn't expect. 3. On start, a panic recorded by `engine::guard::install_hook` since the
  last clean exit opens a small window: what happened, Copy Report, Share Report (`file.share` on the log), Dismiss;
  nothing is sent anywhere.
- **Tests:** the script passes headless (`lightcraft-cli snapshot --demo --script …`); headless
  `last_crash_window_after_panic_log`, `no_window_after_clean_exit`, `report_copies_panic_text`.
- **Verify:** `cargo run -p lightcraft-cli -- snapshot --demo --script docs/showcase/android-edit-smoke.jsonl -o
  target/android/smoke.png && cargo test -p lightcraft-ui-egui -- last_crash report_copies && cargo xtask ci`
- **Depends on:** APK-16, APK-21, APK-27  **Needs:** ci (emulator run)  **Model:** sonnet — script plus one window.
- **Done when:** tests pass; the workflow step is committed (its first run is read through `gh api`).
- **Maintainer checks on a device:** run the script on a Pixel and a Samsung phone through the debug control server.
- **As built (2026-11-02):** the script is `docs/showcase/android-edit-smoke.jsonl` (crop and straighten with
  `crop.set` / `crop.straighten`, a brush mask by `panel.masking`, `maskNew:brush` and one `ui.touch` stroke,
  `develop.copy` of light, masks and crop pasted on photos 2 to 6, two 640 px JPEGs by `app.export`, `ui.inspect`); the
  headless `snapshot` already exits non-zero on a failed request. `run.py --strict` does the same over the control
  port, and `control-check.sh` runs the script after the token check (`ui.settle`, which only a snapshot knows,
  becomes a pause; `ui.screenshot` is dropped; the export goes to the app's cache) and fails on a failed request, a
  pid change, a `panicked` line in the log since the script started or fewer than two exported JPEGs. The window is
  `panels::last_crash` (`Services::previous_crash`, `ui.inspect` → `lastCrash`): the host reads the panic log before
  installing the hook, which empties it, so "since the last clean exit" is "since the last start". `log_file::
  last_panic()` keeps this run's latest panic text next to `last_error()`. The workflow's first run has not been seen.

### APK-54 Storage on a phone
- **Answers:** [C4](android-competitive.md#c4-storage-bloat) (tens of GB, figures that disagree with Android).
- **Goal:** one Storage page whose total matches Android's App info figure, with each part listed and clearable, and
  phone-sized defaults.
- **Files:** new `crates/engine/src/storage_report.rs`; changed `crates/engine/src/cmd/library.rs` (a
  `library.storage` command: sizes per part; `library.clearStorage {part}`), `crates/ui-egui/src/panels/settings.rs`
  (Storage section), `crates/ui-egui/src/lib.rs` (`Services::storage_roots`: the host's private dirs to count),
  `crates/android-host/src/services.rs` (phone default thumbnail cache 512 MB), `docs/parity-checklist.md`
  (LR-PREF-LOCALSTORAGE note).
- **Steps:** 1. Parts: library (catalog), Copy-imported originals, thumbnail and preview cache, smart previews,
  denoise cache, models (SAM, faces, denoise), GeoNames data, logs, Inbox, temp files; each measured on a worker
  (activity row, `Cancel::Flag`), walk bounded by depth and entries. 2. The total counts the host's private dirs
  (`files/` and `cache/`), so it agrees with App info within the system's own rounding. 3. Clearable parts: caches,
  smart previews, denoise cache, models (with what stops working), Inbox, logs; originals and the library never.
  4. When free space falls under 1 GB (probe: `storage_report::free_bytes`, `statvfs`), the thumbnail and denoise caches trim to half their limits and a notice says so. Folders to measure or empty come only from the session and what the host registered, never from command parameters; Clear runs on a worker and asks first for models, Inbox and smart previews.
- **Tests:** `storage_report_counts_every_part` (temp dirs), `clear_cache_part_keeps_library_and_originals`,
  `report_walk_is_bounded`, `low_space_trims_caches` (injected free-space probe); headless
  `storage_section_lists_parts`.
- **Verify:** `cargo test -p lightcraft-engine -- storage_report clear_cache_part low_space && cargo test -p
  lightcraft-ui-egui -- storage_section && cargo xtask parity && cargo xtask ci`
- **Depends on:** APK-11  **Needs:** none  **Model:** sonnet — measuring and a settings section.
- **Done when:** tests pass; parity note updated.
- **Maintainer checks on a device:** compare the page's total with Settings ▸ Apps ▸ LightCraft ▸ Storage after
  importing 1,000 photos.

### APK-55 Culling by swipe and the loupe gestures
- **Answers:** [C6](android-competitive.md#c6-no-local-folders-weak-offline-use) (cull and rate offline on a trip),
  [C8](android-competitive.md#c8-ui-regressions-and-layout) (flags and ratings buried). APK-30 covered pinch, double
  tap, pan and long press; the rest of `android-touch-ui.md` §2 is not built.
- **Goal:** in the loupe, a finger rates, flags and moves through photos without opening a panel, and a full-screen
  cull mode reaches Compare and Assisted Culling.
- **Files:** new `crates/ui-egui/src/panels/cull_touch.rs`; changed `crates/ui-egui/src/panels/detail.rs` (gesture
  arms), `crates/ui-egui/src/state.rs`, `crates/ui-egui/src/menus.rs` (`view.cullMode` toggle with `enabled()`),
  `crates/ui-egui/src/menubar.rs`, `docs/android-touch-ui.md` (§2 column filled), `docs/parity-checklist.md`.
- **Steps:** 1. At fit zoom: swipe sideways → `view.navigate`; vertical swipe on the left half → increase / decrease
  rating, on the right half → pick / unflag / reject; each shows a short HUD and is one undo step. 2. Single tap hides
  and shows all controls (the lights-out state); two-finger tap cycles the info overlays. 3. Cull mode: full screen,
  the gestures above plus "next unflagged", a two-up compare on long press of the filmstrip, and an entry to Assisted
  Culling on the visible set. 4. Zoomed in, a one-finger drag still pans (APK-30). 5. Mouse and keys unchanged.
- **Tests:** through `ui.touch`: `swipe_left_half_rates`, `swipe_right_half_flags`, `sideways_swipe_navigates_at_fit`,
  `zoomed_drag_still_pans`, `single_tap_toggles_controls`, `two_finger_tap_cycles_overlay`,
  `cull_mode_next_unflagged`; `desktop_layout_fingerprint_unchanged` stays green.
- **Verify:** `cargo test -p lightcraft-ui-egui -- swipe_ sideways single_tap two_finger cull_mode desktop_layout &&
  cargo xtask parity && cargo xtask ci`
- **Depends on:** APK-30, APK-30b, APK-33  **Needs:** none  **Model:** opus — gesture conflicts in the busiest view.
- **Done when:** tests pass; the touch spec's §2 names a command or control on every row.
- **Maintainer checks on a device:** cull 200 photos one-handed in flight mode; no accidental rating while panning.
- **As built (2026-11-02):** `crates/ui-egui/src/panels/cull_touch.rs` recognises the gestures from the frame's raw
  touch events (a mouse never sends one, so mouse and keys are unchanged) and runs commands: a sideways swipe is
  `library.next` / `library.previous` (the card's `view.navigate` sets zoom and pan; it is the pinch's command), the
  flag swipes `photo.increaseFlag` / `decreaseFlag` (pick, unflagged, reject), on the active photo only (`ids`). A
  single tap is `view.lightsOut` off / on, held back 0.3 s so the first tap of a double tap (zoom, now the touch
  recogniser's, not egui's click) doesn't toggle it; a finger's tap no longer click-zooms. A gesture that starts on
  another layer (a dialog, a menu), outside the loupe or on a widget over the photo (egui gives the press or the
  click to the Navigator, a face region's label or x, the filter pill), a cancelled one and a slow drag (over 0.8 s)
  do nothing.
  Cull Mode (`view.cullMode`, `UiState::cull_mode`, not saved) replaces the top bar, panels and toolbar with a bar
  (Done, Next Unflagged = `view.nextUnflagged`, Assisted Culling… as `dialog.cull {scope: "visible"}`, scoring
  the photos in view and keeping the selection) over the loupe and filmstrip; holding a filmstrip photo there opens
  Compare with the active photo instead of the photo menu (a right click still opens the menu); Back: Compare →
  loupe on the photo being culled, alone → out of Cull Mode; leaving the loupe and Compare any other way (a grid
  shortcut) ends Cull Mode. Tests: `tests_cull_touch.rs` (the card's seven plus `cull_mode_needs_a_photo…`,
  `a_mouse_click_still_zooms…`, `leaving_the_loupe_another_way…`, `assisted_culling_from_the_cull_bar…` and
  `a_tap_on_a_widget_over_the_photo…`) and the recogniser's unit tests in `cull_touch.rs`.

### APK-56 Finger strokes at full pressure, pen pressure from a stylus
- **Answers:** [C13](android-competitive.md#c13-stylus-and-tablets). Brush ▸ Pen Pressure reads `force` from every
  `egui::Event::Touch`; on Android winit reports a force for fingers too (contact pressure), so a finger stroke would
  vary in width by accident (verify on a device).
- **Goal:** fingers paint at full pressure; a stylus (S Pen, USI) scales size and flow; the toggle sits in the
  brush sheet.
- **Files:** changed `crates/ui-egui/src/panels/detail.rs` (`pen_force` takes the pointer kind),
  `crates/ui-egui/src/state.rs` (`brush_pressure` default under `touch_mode`), `crates/ui-egui/src/panels/masking.rs`
  (toggle in the sheet), `crates/android-host/src/input.rs` (a stylus signal if the host can get one),
  `docs/android.md`.
- **Steps:** 1. Find what reaches the host for a stylus: winit 0.30's Android backend and android-activity's
  `MotionEvent` tool type (`ToolType::Stylus`); if the host's input path can see it, `raw_input_hook` keeps `force`
  only for stylus touches and clears it for fingers. 2. If no tool type reaches the host, Pen Pressure starts off under
  `touch_mode` and the brush sheet explains "for a stylus". 3. Desktop behaviour unchanged.
- **Tests:** `finger_touch_paints_full_pressure`, `stylus_force_scales_dab` (fake events), `desktop_pen_pressure_unchanged`.
- **Verify:** `cargo test -p lightcraft-ui-egui -- finger_touch stylus_force desktop_pen && cargo xtask android check
  && cargo xtask ci`
- **Depends on:** APK-30b, APK-33  **Needs:** rust-target  **Model:** sonnet — small, test-driven; step 1 is a
  source read of winit and android-activity.
- **Done when:** tests pass; `docs/android.md` says which path was taken and why.
- **As built (2026-11-02):** step 1 found that winit 0.30.13's Android backend (`platform_impl/android/mod.rs`)
  passes only `pointer.pressure()` as `Force::Normalized` and no tool type, so the host cannot tell a stylus from a
  finger; step 2 was taken. `LightcraftApp::raw_input_hook` (`lib.rs`, not `state.rs`) turns `brush_pressure` off on
  the first frame of a touch-first host (`services.touch`); `pen_force` gained a `stroke_force(enabled, input)` gate
  instead of a pointer-kind argument; `crates/android-host/src/input.rs` is unchanged. The brush sheet shows the
  visible caption "For a stylus; fingers paint at full pressure while off" under touch mode (a touch screen shows no
  tooltip), translated in 7 catalogs. The choice is not persisted across launches. Tests: `finger_touch_paints_full_pressure`,
  `stylus_force_scales_dab`, `desktop_pen_pressure_unchanged`, `touch_brush_sheet_explains_pen_pressure`. The device
  check is still open.
- **Maintainer checks on a device:** a brush mask by finger (even width) and by S Pen (width follows pressure) on a
  Galaxy Tab.

### APK-57 Privacy you can check
- **Answers:** [C12](android-competitive.md#c12-privacy-and-trust). LightCraft has no telemetry and uses the network
  only for downloads the user starts, but nothing proves it or tells the user.
- **Goal:** a registry of every network use, a test that fails when code reaches the network outside it, a Settings
  ▸ Network section listing the uses with an "Never use the network" switch, and the text for the store's Data safety
  form.
- **Files:** new `crates/fetch/src/uses.rs` (the registry: id, purpose, host list, user action that starts it),
  `crates/fetch/tests/uses_scan.rs` (the source scan);
  changed `crates/fetch/src/lib.rs` (`download*` take a registry id), `crates/engine/src/{model_download.rs,
  face_download.rs,cmd/geocode.rs}`, `crates/engine/src/config.rs` (`network_off` pref), `crates/ui-egui/src/panels/settings.rs`,
  `docs/android.md` (Privacy section), `docs/parity-checklist.md` (a "beyond Lightroom" row).
- **Steps:** 1. Every caller of `lightcraft_fetch` passes a registry id; an unknown id is an error. 2. With
  `network_off`, downloads refuse with "Network use is off in Settings ▸ Network". 3. A test greps the workspace's
  non-test sources for `lightcraft_fetch::` calls and HTTP clients and fails on any not tied to a registry id. 4. The
  debug control server is listed too (debug builds only, loopback, token). 5. `docs/android.md` gets the Data safety
  answers: no data collected, no data shared, downloads named.
- **Tests:** `unknown_use_is_refused`, `network_off_refuses_downloads`, `every_network_call_is_registered` (source
  scan), headless `network_section_lists_uses`.
- **Verify:** `cargo test -p lightcraft-fetch && cargo test -p lightcraft-engine -- network_off && cargo test -p
  lightcraft-ui-egui -- network_section && cargo xtask parity && cargo xtask ci`
- **Depends on:** none  **Needs:** none  **Model:** sonnet — registry and checks.
- **Done when:** tests pass; the Privacy section exists.

### APK-58 Presets by touch
- **Answers:** [C8](android-competitive.md#c8-ui-regressions-and-layout) (preset categories removed, re-opening the
  menu for every try), [requests](android-competitive.md#requests-that-come-back). Hover preview needs a pointer that
  rests; a finger has none.
- **Goal:** in the phone and tablet layouts the Presets sheet stays open with live thumbnails; a tap previews on the
  photo, Apply (or a second tap) commits, and the user's groups and favourites stay where they put them.
- **Files:** changed `crates/ui-egui/src/panels/presets.rs`, `crates/ui-egui/src/state.rs` (`preset_previewing`),
  `crates/ui-egui/src/sheet.rs` (created by APK-33).
- **Steps:** under `touch_mode`: tap = the same `HoverPreview` the pointer gives, shown until another tap, Apply,
  Cancel or Back (a new `view.back` arm); the amount slider applies to the previewed preset; thumbnails on by default
  in the sheet, drawn only for rows in view (as today).
- **Tests:** through `ui.touch`: `tap_previews_without_committing`, `second_tap_applies`, `back_cancels_preview`;
  `desktop_hover_preview_unchanged`.
- **Verify:** `cargo test -p lightcraft-ui-egui -- tap_previews second_tap back_cancels desktop_hover && cargo xtask ci`
- **Depends on:** APK-33  **Needs:** none  **Model:** sonnet — one panel.
- **Done when:** tests pass.  **Maintainer checks on a device:** try ten presets on one photo without the sheet
  closing.

### APK-59 Camera import over USB (MTP)
- **Answers:** [C5](android-competitive.md#c5-raw-and-card-import-on-android) (PTP mode, hidden raws). A camera
  connected by USB is not a mounted volume, and MTP reads bypass the media database, so every raw type arrives.
- **Goal:** a camera plugged in by USB shows under Devices; Import copies its photos (verified, duplicates skipped)
  into the library.
- **Files:** new `crates/android-sys/src/{usb.rs,mtp.rs}`; changed `packaging/android/AndroidManifest.xml`
  (`USB_DEVICE_ATTACHED` intent filter), `xtask/src/android/manifest.rs` (test), `crates/android-host/src/services.rs`,
  `crates/engine/src/devices.rs` (a device kind whose files come from a reader, not a path), `crates/ui-egui/src/import.rs`,
  `docs/android.md`.
- **Steps:** 1. JNI `UsbManager.getDeviceList()`; permission via `requestPermission(device, PendingIntent)` with a
  `PendingIntent.getBroadcast` on an empty package-local intent (no receiver class), then `hasPermission` polled on
  `Focused(true)` and every 500 ms as APK-22 does. 2. `android.mtp.MtpDevice.open(UsbDeviceConnection)`,
  `getStorageIds`, `getObjectHandles`, `getObjectInfo` (name, size, date) for the review list, one local frame per
  object. 3. Import copies with `MtpDevice.importFile(handle, path)` into the staging folder `files/incoming/` (outside the library: the importer adds a file inside it where it is) on an attached worker
  (`TaskGuard`, `Cancel::Flag` checked between files), then the existing Copy import with its content-hash check;
  duplicates skipped by name + size + date before copying. 4. Unplugging mid-import gives per-file errors.
- **Tests:** `object_rows_map_to_review_items`, `duplicate_objects_skipped`, `permission_poll_states` (pure);
  headless `camera_device_import_shows_activity_row`, `camera_unplugged_mid_import_gives_errors` (fake reader);
  manifest test asserts the intent filter.
- **Verify:** `cargo test -p lightcraft-android-sys -- usb mtp && cargo test -p lightcraft-ui-egui -- camera_device
  camera_unplugged && cargo test -p xtask -- manifest && cargo xtask android check && cargo xtask ci`
- **Depends on:** APK-22, APK-23, APK-25, APK-51  **Needs:** rust-target  **Model:** opus — USB permission flow and
  MTP through JNI, testable only on a device.
- **Done when:** tests pass; `docs/android.md` lists cameras tried (maintainer).
- **Maintainer checks on a device:** a Canon (CR3), a Fujifilm (compressed RAF) and a Sony camera by USB-C: they
  appear, import, and render as their decoders allow (raw-parity gaps 2 and 6 still apply).

### APK-60 Batch Auto and AI-mask update
- **Answers:** [C11](android-competitive.md#c11-batch-editing-gaps). `develop.auto` acts on the active photo only, and
  LR-MASK-UPDATE (recompute AI masks) is ⬜, so pasted Subject / Sky masks keep the source photo's shape.
- **Goal:** Auto Settings on a selection (each photo its own analysis, one undo step) and Update AI Masks on a
  selection; paste with masks offers "recompute subject and sky on each photo".
- **Files:** changed `crates/engine/src/cmd/develop.rs` (`develop.auto` with `ids`), `crates/engine/src/cmd/masks.rs`
  (`mask.updateAi {ids}`), `crates/ui-egui/src/panels/dialogs.rs` (paste option), `crates/ui-egui/src/menus.rs`,
  `crates/ui-egui/src/menubar.rs`, `docs/parity-checklist.md` (LR-MASK-UPDATE, LR-EDIT-AUTO notes).
- **Steps:** 1. Both run on a worker holding an activity row with `Cancel::Flag`; thumbnails analysed per photo;
  catalog changes applied on the UI thread as one undo step. 2. A photo that can't be read is reported and skipped.
  3. Heuristic masks recompute; SAM masks recompute only where the model is installed and RAM allows (APK-11b),
  otherwise kept and reported.
- **Tests:** `auto_on_selection_analyses_each_photo`, `auto_on_selection_is_one_undo_step`,
  `update_ai_masks_recomputes_subject`, `unreadable_photo_is_skipped_with_reason`; headless
  `paste_with_recompute_option`.
- **Verify:** `cargo test -p lightcraft-engine -- auto_on_selection update_ai_masks unreadable_photo && cargo test -p
  lightcraft-ui-egui -- paste_with_recompute && cargo xtask parity && cargo xtask ci`
- **Depends on:** none (engine and ui-egui; desktop gains it too)  **Needs:** none  **Model:** opus — undo and
  worker split across two commands.
- **Done when:** tests pass; parity rows updated.

### APK-61 Slider gestures on the photo
- **Answers:** [C8](android-competitive.md#c8-ui-regressions-and-layout) (small sliders),
  [rivals](android-competitive.md#rivals-strengths-users-mention) (Snapseed's swipe editing).
- **Goal:** with a panel open, a vertical drag on the photo chooses one of the panel's controls and a horizontal drag
  changes it, with a value readout; on sliders, a tap beside the thumb nudges and a double tap resets
  (`android-touch-ui.md` §2 slider rows).
- **Files:** new `crates/ui-egui/src/panels/photo_drag.rs`; changed `crates/ui-egui/src/panels/detail.rs`,
  `crates/ui-egui/src/widgets.rs` (slider tap and double tap), `crates/ui-egui/src/state.rs` (on/off, default on under
  touch), `docs/android-touch-ui.md`.
- **Steps:** 1. The control list comes from the open panel's `develop` control specs (no second list). 2. A drag
  changes the value through the same control path as the slider (one undo step per gesture, draft renders during the
  drag). 3. Off when a tool owns the canvas (crop, brush, masks, spot).
- **Tests:** through `ui.touch`: `vertical_drag_selects_control`, `horizontal_drag_changes_value_one_undo`,
  `tool_owns_canvas_no_drag_edit`, `tap_beside_thumb_nudges`, `double_tap_thumb_resets`.
- **Verify:** `cargo test -p lightcraft-ui-egui -- vertical_drag horizontal_drag tool_owns tap_beside double_tap_thumb
  && cargo xtask ci`
- **Depends on:** APK-33, APK-55  **Needs:** none  **Model:** opus — gesture arbitration with the tools.
- **Done when:** tests pass.

### APK-62 Phone ↔ computer handoff without a cloud
- **Answers:** [C1](android-competitive.md#c1-sync-is-unreliable) (sync that stalls), the LAN-sync request.
- **Goal:** photos edited on the phone reach the desktop library with their edits, and back, through files the user
  moves (share, USB, a folder synced by their own tool), with nothing silently lost.
- **Files:** new `crates/engine/src/handoff.rs`; changed `crates/engine/src/cmd/export.rs` (a `handoff` export: Original
  + XMP for the selection, plus a manifest with content hashes), `crates/engine/src/sync.rs` (reads a handoff folder
  as Synchronize Folder does), `crates/ui-egui/src/menus.rs`, `docs/xmp-interop.md`.
- **Steps:** 1. Export writes originals with sidecars and a manifest (hash, catalog id, edit time). 2. Import of a
  handoff folder matches photos already in the library by content hash and reads only newer settings, listed for the
  user to confirm (Synchronize Folder's rules: never the default to overwrite). 3. On Android the output goes to
  Documents/LightCraft and is offered to share; APK-25's sidecar routing receives it. 4. A LAN sync stays out until a
  maintainer decision (proposed decision 13: listening socket, discovery, pairing).
- **Tests:** `handoff_roundtrip_keeps_edits`, `older_handoff_never_overwrites`, `hash_match_finds_moved_photo`.
- **Verify:** `cargo test -p lightcraft-engine -- handoff && cargo xtask ci`
- **Depends on:** APK-25, APK-27  **Needs:** none  **Model:** opus — matching and conflict rules.
- **Done when:** tests pass; `docs/xmp-interop.md` documents the manifest.
- **Maintainer checks on a device:** edit 20 raws on the phone, hand them to the desktop by USB, edit two there and
  hand them back.

### APK-63 Left-handed and remembered layouts
- **Answers:** [C8](android-competitive.md#c8-ui-regressions-and-layout) (landscape removed, left-handed requests,
  "give us the old layout back").
- **Goal:** Settings ▸ Interface: Left-handed (the phone toolbar's order and the tablet panel side mirror), panel side
  on tablets, compact or full phone toolbar; each choice persists per breakpoint and no update resets it.
- **Files:** changed `crates/ui-egui/src/state.rs`, `crates/ui-egui/src/sheet.rs` (created by APK-33), the tablet
  layout module from APK-32c, `crates/ui-egui/src/panels/settings.rs`.
- **Steps:** prefs in `UiState` keyed by breakpoint; the mirror is a layout parameter, not a second UI; a settings
  version field so a later layout change migrates the choice instead of dropping it.
- **Tests:** `left_handed_mirrors_toolbar_and_panel`, `layout_choice_survives_restart`,
  `layout_pref_migrates_on_version_bump`; `desktop_layout_fingerprint_unchanged`.
- **Verify:** `cargo test -p lightcraft-ui-egui -- left_handed layout_choice layout_pref desktop_layout && cargo xtask
  ci`
- **Depends on:** APK-32c, APK-33  **Needs:** none  **Model:** sonnet — layout parameters with structural tests.
- **Done when:** tests pass.

### APK-64 AI denoise on a phone
- **Answers:** [C10](android-competitive.md#c10-ai-features-online-only-or-missing) (denoise missing on mobile,
  cloud-only AI).
- **Goal:** AI denoise runs on the phone within its memory budget, never in the foreground thread, and says how long
  it will take.
- **Files:** changed `crates/denoise/` (tile size from the memory budget), `crates/engine/src/denoise.rs`,
  `crates/ui-egui/src/panels/denoise.rs` (estimate line), `crates/ui-egui/src/lib.rs` (`Services::min_ram_for_denoise`
  like `min_ram_for_sam`), `crates/android-host/src/services.rs`, `docs/denoise.md`.
- **Steps:** 1. Tile size and worker count from `memory::default_budget()`; refuse with a reason below the minimum.
  2. A first-tile timing gives the estimate; the activity row shows it and can cancel. 3. Out-of-the-box weights
  depend on gap 11 (no shippable model yet): until then the user installs a model as on desktop.
- **Tests:** `tile_size_follows_budget`, `below_minimum_ram_refuses_with_reason`, `estimate_from_first_tile`.
- **Verify:** `cargo test -p lightcraft-denoise -- tile_size && cargo test -p lightcraft-engine -- below_minimum_ram
  estimate_from && cargo xtask android check && cargo xtask ci`
- **Depends on:** APK-11b  **Needs:** rust-target (device for timings)  **Model:** opus — memory and scheduling in
  the denoise path.
- **Done when:** tests pass; `docs/denoise.md` has the phone section.
- **Maintainer checks on a device:** denoise a 24 MP Bayer raw on an 8 GB and a 6 GB phone; record times.

## Proposed decisions

For [`android-port.md`](android-port.md) → "Decisions for the maintainer" (this page doesn't change that table):

| # | Decision | Default an agent uses | Blocks |
|---|---|---|---|
| 12 | **All files access.** Decided 2026-10-31 (android-port.md decision 12): never requested; APK-52 step 3 uses shares and "Open with" in release 1 and the picker shim after it. | not requested | APK-52 step 3 |
| 13 | **LAN sync.** A listening socket with discovery and pairing between phone and desktop, versus file handoff only. | file handoff only (APK-62) | a later sync card |

## Order

The port plan's order stays: Phase 3 first, then Phase 2, then APK-35 and Phase 4. As of 2026-10-31, `git log`
shows APK-29, 30, 30b and 31 done; APK-32a (in progress in the working tree), 32b, 32c, 33, 33b, 34 and 34b remain in
Phase 3.

1. **Alongside the rest of Phase 3** (no dependency on it): APK-54, APK-57, APK-60.
2. **Phase 2** (APK-22 … 28) with the folds above applied to APK-23, 25 and 27.
3. **Right after Phase 2, before APK-35 and Phase 4** (the beta blockers): APK-50, APK-51, APK-52, APK-53, then
   APK-55 and APK-56 (they need Phase 3's APK-33).
4. **Phase 4** (APK-35, 36 … 43), with APK-41's budget-phone row and APK-43 taking APK-57's Data safety answers.
   The first public beta follows APK-43 with every card marked beta blocker done.
5. **After the beta**, in rank order: APK-58, APK-59, APK-61, APK-62, APK-63, APK-64.

## Revision history

- 2026-10-31: first version: opportunities from `android-competitive.md` checked against the code and the parity
  checklist; 15 cards (APK-50 … 64), folds into APK-23, 25, 27, 41 and 43, proposed decisions 12 and 13.
- 2026-10-31: decision 12 settled by the maintainer (no all-files access); decision 2 (no Java in release 1) applies to
  APK-51 and APK-52.
- 2026-11-01 (APK-25): the MTP card stages imports in `files/incoming/`, outside the library (the importer adds a
  file inside the library where it is).
- 2026-11-01 (APK-50): "As built" note on the card.
- 2026-11-02 (APK-51): "As built" note on the card.
- 2026-11-02 (APK-52): "As built" note on the card.
- 2026-11-02 (APK-53): "As built" note on the card.
- 2026-11-02 (APK-55): "As built" note on the card.
- 2026-11-02 (APK-56): "As built" note on the card.
