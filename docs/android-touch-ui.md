# Android touch UI: target behaviour

*Last updated: 2026-11-02.* Revision history at the end.

What the Android build should feel like: someone who knows Lightroom on a phone finds every tool where they expect
it and every gesture does what they expect. Phase 3 of [`android-port.md`](android-port.md) (APK-29 … APK-35)
builds this.

**Clean-room.** This page describes *layout and behaviour* in our own words, from Adobe's public help pages, release
notes and third-party reviews (sources at the end). Nothing is copied: no icons, artwork, screenshots, presets or
profiles; the icons are our own vector drawings (`crates/ui-egui/src/icons.rs`). Lines marked **(verify)** come
from a single or secondary source and are checked by the maintainer on the free Lightroom app, read-only (look, never
edit, import or delete in the personal library). Screenshots of Adobe apps never enter the repository.

## 1. Structure

### Home and library
- **Sources of photos:** photos on the device (LightCraft: the Local section, `Services::local_roots`, APK-6/27) and
  the library (all photos, albums, folders of albums). Lightroom's third source, a community feed, is out of scope.
- **Top bar:** search, select (multi-select mode), filter and sort, settings in an overflow menu.
- **Grid:** square or justified layout; segmented by date (or location); an info overlay on thumbnails that a
  two-finger tap cycles (verify on Android); a slideshow entry.
- **Albums** open from a side drawer on phones (a chevron or the back gesture closes it) (verify).

### One photo (loupe)
- **Top bar:** back, undo and redo, versions, share, overflow menu (copy/paste settings, info, delete …) (verify the
  order).
- **Middle:** the photo; a single tap hides all controls, a second tap brings them back.
- **Bottom:** the filmstrip (toggle) and, in edit mode, the **edit toolbar**.
  LightCraft: below 500 pt window height (a phone held sideways, also a short desktop window) the filmstrip starts
  folded and the bottom bar's Filmstrip button pulls it out; the choice is remembered per window class (short/tall).

### Edit toolbar and panels
- **Toolbar:** one row of icon + label buttons along the bottom, scrolling sideways. Expected tools (order to
  verify on the device): Presets, Auto/Quick actions, Light, Color, Effects, Detail, Optics, Geometry, Masking,
  Remove, Crop; Lightroom also has Lens blur (premium). Versions sit in the top bar, not in the toolbar.
- **Panel:** tapping a tool opens its sliders as a **sheet above the toolbar**; full-width sliders with name and
  value; the sheet collapses to show more of the photo; sub-tools (tone curve, colour mixer, colour grading) are
  buttons at the top of their panel that switch the sheet's content.
  LightCraft (APK-33, `crates/ui-egui/src/sheet.rs`): below 600 pt window width (`state::PHONE_WIDTH`, any platform)
  the tool strip becomes this toolbar (Library, Presets, Edit, Crop, Remove, Masking, Red Eye, Versions, History,
  Keywords, Info; 68×56 pt buttons) and a tool opens the desktop's own panel as a full-width sheet above it, with a
  drag handle for its height. Opening a tool there switches its desktop panel on (as the tool strip does); tapping
  the shown tool or Back puts the sheet down and the panel choice stays.
- **Tablet landscape:** the same panels in a right-hand column (on Android the panel side is fixed; iOS lets it move
  left) (verify).
- **Premium badge:** in the free app, premium tools stay visible with a badge and open an upgrade screen. LightCraft
  has no tiers: every tool it has is free.
- **Dialogs:** LightCraft (APK-33b, `panels::dialogs::width`): every dialog is at most the content width (the window
  less its safe-area insets) less 12 pt at each side; below 600 pt it takes all of that. Its rows of controls stack
  there (label above, controls wrapping below), as do two-column bodies (About ▸ Licences, Settings ▸ Slideshow), and
  the tab rows wrap.
- **Menu bar:** LightCraft (APK-34, `menubar::show_in_window`): below 600 pt (`state::PHONE_WIDTH`) the in-window
  menu is one "Menu" button (`menu:all`, finger-sized) whatever room is left; its popup lists the six menus as
  submenu rows built from the same `menu_bar()` tree as the desktop (same command ids, same enabled state). APK-34b
  (`panels/topbar.rs`): on a phone the search field collapses to a magnifier (`icon:search`) that opens it full-width
  with a close button (`icon:searchClose`, clears the query); the right-hand icons except Share (and the cloud, only
  while saving fails) are left to the overflow menu, where their commands already are.

## 2. Gestures

From Adobe's gesture reference for Lightroom on mobile; Android differences are to be verified. The "LightCraft"
column names the command or control each gesture runs. The loupe's finger gestures live in
`crates/ui-egui/src/panels/cull_touch.rs` (APK-55): they read the raw touch events, so a mouse and the keys behave as
before, and they act only where no tool owns the photo (Crop, Masking, Remove, Red Eye, a picker).

| Where | Gesture | Does | LightCraft |
|---|---|---|---|
| Photo | press and hold | show the original while held | `cmd:view.showOriginal` (by key; a held finger opens the photo's menu for now) |
| Photo | single tap | hide / show all controls | `cmd:view.lightsOut` off / on (APK-55); acts once no second tap followed (0.3 s) |
| Photo | double tap | zoom to detail, again to fit | the click-zoom ratio at the point, as `cmd:view.zoomToggle` (APK-30, APK-55) |
| Photo | pinch | zoom | `cmd:view.navigate` (`navigate_gesture` in `panels/detail.rs`) |
| Photo (fit) | swipe sideways | next / previous photo | `cmd:library.next` (to the left) / `cmd:library.previous` (APK-55) |
| Photo (fit), left half | swipe up / down | raise / lower the star rating | `cmd:photo.increaseRating` / `cmd:photo.decreaseRating`, with a HUD (APK-55) |
| Photo (fit), right half | swipe up / down | set / clear the flag | `cmd:photo.increaseFlag` / `cmd:photo.decreaseFlag`: reject, unflagged, pick (APK-55) |
| Photo (zoomed in) | one-finger drag | pan | `cmd:view.navigate` (APK-30); never a swipe |
| Photo | two-finger tap | cycle info overlays (file, rating/flags, EXIF) | `cmd:view.infoOverlay` (APK-55) |
| Cull Mode filmstrip | press and hold a photo | compare it with the active one | `cmd:view.cullMode`; the two-up Compare of `cmd:view.compare` with the active photo as select and the held one as candidate; Back returns to the select (APK-55) |
| Slider | double tap the thumb | reset to default | `cmd:develop.resetControl` (a double click on any slider); under touch only on the thumb, a finger-sized target (APK-61) |
| Slider | tap left / right of the thumb | nudge in small steps | one `cmd:develop.adjust` step that way, one undo step (`widgets::slider` under the touch layout, APK-61; the layout decides, not the input: at desktop density a click jumps there, under the touch layout a mouse click nudges too) |
| Slider | drag with two fingers | show clipping while adjusting | `cmd:view.clipping` (by key; the gesture is not built yet) |
| Slider + photo | one finger held on the photo while dragging a slider | the desktop Alt/Option view (clipping/mask) | `cmd:view.clipping` (by key; the gesture is not built yet) |
| Photo, a panel of sliders open | vertical drag | choose one of the panel's sliders, with a readout | the open panel's `develop` sliders, one per 36 pt (`panels/photo_drag.rs`, APK-61) |
| Photo, a panel of sliders open | horizontal drag | change the chosen slider, with a readout | `cmd:develop.set` between `develop.beginInteraction` / `endInteraction`, as the slider: one undo step, draft renders (APK-61) |
| Crop frame | double tap | reset the crop | `cmd:crop.reset` in the panel; a double tap inside the frame applies it (`cmd:tool.done`), as a double click does |
| Grid | pinch or two-finger swipe | thumbnail size | `cmd:view.thumbSizeUp` / `cmd:view.thumbSizeDown` (the bottom bar's slider; pinch not yet mapped) |
| Grid | tap / long press | open / start multi-select | `cmd:library.select` (APK-30b) |

**Slider gestures on the photo** (APK-61, `crates/ui-egui/src/panels/photo_drag.rs`, Snapseed's swipe editing):
with the Edit panel (or its sheet, or the profile browser) open, each enabled `develop` slider it shows offers
itself, so the list is the panel's own: a closed section or a disabled slider is not in it. A one-finger vertical
drag on the photo moves the choice one slider per 36 pt (the last one chosen stays chosen; at first the panel's
first slider), and a horizontal drag changes the chosen one: the photo's width moves the thumb half its track.
While it is armed a one-finger flick is this gesture, not a rating, flag or next-photo swipe; taps, double taps,
two-finger taps and pinches keep their meaning. It is off zoomed in (a drag pans), while a tool owns the photo
(Crop, Masking, Remove, Red Eye, a picker), with the panel closed and when `UiState::photo_drag` is off (on by
default; Settings ▸ Interface ▸ "Slider gestures on the photo (touch)", `ui.set {"photoDrag": false}`; a mouse never
does it). A drag the loupe stops drawing (a view change mid-drag) ends where it is at the frame's end.

**Cull Mode** (View ▸ Cull Mode, `cmd:view.cullMode`, APK-55): the loupe full screen with the filmstrip and a bar
(Done, Next Unflagged `cmd:view.nextUnflagged`, Assisted Culling… on the photos in view `cmd:dialog.cull`); the
gestures above work there, holding a filmstrip photo opens the two-up Compare (a mouse's right click still opens
the photo menu), and Back goes from Compare to the loupe on the photo being culled, then out of Cull Mode. A gesture
counts only when it starts on the photo itself: the widgets over it (the Navigator, a face region's label and x, the
filter pill) keep their taps.

## 3. Premium features and where LightCraft stands

Lightroom's paid tier on mobile, from Adobe's premium-features page (2025) and reviews (2025/26). Sources disagree on
a few items (masking brush, batch edit, DNG), and Adobe moves the line often; the column "LR mobile" says what the
sources claim. In LightCraft all of it is free and local (no cloud, no account).

| Feature | LR mobile | LightCraft today | Android |
|---|---|---|---|
| Raw editing (camera raws) | premium (JPEG/HEIC/DNG from the phone camera free) | ✅ the core of LightCraft | ships |
| Heal / clone / remove | premium | ✅ remove and heal (PatchMatch, on-device) | Remove tool, finger brush |
| Generative remove (cloud AI) | premium, online | 🚫 out of scope (cloud); on-device remove instead | — |
| Remove people / distractions | premium | 🟡 face detection + `cmd:photo.removeRegion` | later |
| Blemish removal (2026) | premium | ✅ `cmd:spot.findBlemishes` | ships |
| Selective adjustments: brush, linear, radial | premium (sources disagree on the brush) | ✅ masks | Masking sheet |
| AI masks: subject, sky, background, people, objects | premium | 🟡 subject/sky heuristics; SAM where RAM ≥ 8 GB (decision 10) | Masking sheet |
| Colour / luminance range masks | premium | check `docs/parity-checklist.md` | Masking sheet |
| Geometry, Upright | premium | ✅ `cmd:geometry.upright`, `ctl:geometry.*` | Geometry panel |
| Premium / adaptive presets | premium | presets ✅, adaptive profiles ⬜ | Presets panel |
| Lens blur | premium | ⬜ (settings field reserved, not rendered) | later |
| Quick actions (subject-aware one-tap edits) | premium | ⬜ (Auto exists) | later |
| Batch edit, copy / paste settings | premium (one source: free) | ✅ copy, paste, sync settings | multi-select + paste |
| Versions | — | ✅ `cmd:version.create` | top bar |
| AI denoise | iPad only so far | 🟡 user-supplied models | heavy on a phone: later |
| HDR editing | — | ✅ HDR section | ships (display support varies) |
| Cloud storage, sync, AI search, share to web | premium | 🚫 local-first by design | — |
| Generative expand | 2026 | 🚫 out of scope | — |

## 4. What the maintainer checks on the free app

Read-only, on the maintainer's phone; results go into this page (replace "(verify)"):
- [ ] the edit toolbar's tools in order, which carry a premium badge
- [ ] the loupe's top bar and overflow menu entries, in order
- [ ] panel sheet: height, collapse gesture, how sub-tools switch
- [ ] the gestures in §2 on Android (two-finger tap, edge swipes for rating/flags, single tap to hide)
- [ ] the library: home sections, drawer, grid options, multi-select

## 5. What it changes in the plan

Phase 3 cards follow this page: APK-30 maps §2 (gestures), APK-30b the grid rows, APK-32a–c sizes, APK-33 the
toolbar and sheets of §1, APK-34 the top bar and overflow menu. A card that has to deviate (an egui limit) says why.

## Sources

- Adobe, gesture shortcuts for Lightroom on mobile: https://www.adobe.com/learn/lightroom-cc/web/gesture-shortcuts-lightroom-mobile
- Adobe, gesture controls in Lightroom for mobile: https://helpx.adobe.com/lightroom/mobile/get-started/gesture-controls-in-lightroom-for-mobile.html
- Adobe, workspace overview (mobile): https://helpx.adobe.com/lightroom/mobile/get-started/workspace-overview.html
- Adobe, adjust light (Android edit panel): https://helpx.adobe.com/lightroom-cc/using/edit-panel-android.html
- Adobe, masking in Lightroom for Android: https://helpx.adobe.com/lightroom-cc/using/masking-mobile-android.html
- Adobe, masking tools (mobile): https://helpx.adobe.com/lightroom/mobile/masks-and-selective-adjustments/masking-tools.html
- Adobe, generative remove (mobile): https://helpx.adobe.com/lightroom/mobile/heal-clone-and-remove/remove-objects-with-generative-remove.html
- Adobe, premium features: https://helpx.adobe.com/lightroom/desktop/using/premium-features.html
- Adobe, Lightroom on mobile FAQ: https://helpx.adobe.com/mobile-apps/help/lightroom-mobile-faq.html
- Adobe, mobile release notes: https://helpx.adobe.com/lightroom/mobile/whats-new/adobe-lightroom-on-mobile-release-notes.html
- Adobe Community, panel position on Android: https://community.adobe.com/t5/lightroom-ecosystem-cloud-based-discussions/change-panel-position-on-lightroom-mobile/td-p/10125332
- Adobe Community, Remove tool replaces heal/clone: https://community.adobe.com/questions-680/lightroom-mobile-app-bring-back-clone-stamp-930972
- Shotkit, is Lightroom free: https://shotkit.com/is-lightroom-free/
- PictureCorrect, free vs premium: https://www.picturecorrect.com/exploring-lightroom-mobile-free-features-vs-premium/
- Aperlust, free vs paid review: https://aperlust.com/lightroom-mobile-review-free-vs-paid/

## Revision history

- 2026-10-17: first version from public sources; the maintainer's device checks pending.
- 2026-11-01: the phone layout as built (APK-33): edit toolbar and bottom sheet below 600 pt width.
- 2026-11-01: dialogs fit a phone (APK-33b).
- 2026-11-01: the menu bar collapses to one overflow button on a phone (APK-34); the top bar's search collapses to a magnifier (APK-34b).
- 2026-11-02: §2 as built (APK-55): the loupe's swipes, taps and two-finger tap, Cull Mode; every row names its command.
- 2026-11-02: slider gestures on the photo and touch slider taps as built (APK-61).
- 2026-11-02: APK-61 review: the Settings switch, the layout (not the input) decides slider taps, a drag ends with the loupe.
