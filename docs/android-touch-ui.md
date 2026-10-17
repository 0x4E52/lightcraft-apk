# Android touch UI: target behaviour

*Last updated: 2026-10-17.* Revision history at the end.

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

### Edit toolbar and panels
- **Toolbar:** one row of icon + label buttons along the bottom, scrolling sideways. Expected tools (order to
  verify on the device): Presets, Auto/Quick actions, Light, Color, Effects, Detail, Optics, Geometry, Masking,
  Remove, Crop; Lightroom also has Lens blur (premium). Versions sit in the top bar, not in the toolbar.
- **Panel:** tapping a tool opens its sliders as a **sheet above the toolbar**; full-width sliders with name and
  value; the sheet collapses to show more of the photo; sub-tools (tone curve, colour mixer, colour grading) are
  buttons at the top of their panel that switch the sheet's content.
- **Tablet landscape:** the same panels in a right-hand column (on Android the panel side is fixed; iOS lets it move
  left) (verify).
- **Premium badge:** in the free app, premium tools stay visible with a badge and open an upgrade screen. LightCraft
  has no tiers: every tool it has is free.

## 2. Gestures

From Adobe's gesture reference for Lightroom on mobile; Android differences are to be verified. The "LightCraft"
column names the existing command or control where there is one; APK-30 maps the rest.

| Where | Gesture | Does | LightCraft |
|---|---|---|---|
| Photo | press and hold | show the original while held | `cmd:view.showOriginal` |
| Photo | single tap | hide / show all controls | lights-out style toggle (APK-30) |
| Photo | double tap | zoom to detail, again to fit | `cmd:view.zoomToggle` |
| Photo | pinch | zoom | already in `panels/detail.rs` |
| Photo (fit) | swipe sideways | next / previous photo | `cmd:view.navigate` |
| Photo, left half | swipe up / down | raise / lower the star rating | `cmd:photo.increaseRating` / `decreaseRating` |
| Photo, right half | swipe up / down | set / clear the flag | flag commands (APK-30) |
| Photo | two-finger tap | cycle info overlays (file, rating/flags, EXIF) | info overlay (APK-30) |
| Slider | double tap the thumb | reset to default | control reset |
| Slider | tap left / right of the thumb | nudge in small steps | control step |
| Slider | drag with two fingers | show clipping while adjusting | clipping overlay |
| Slider + photo | one finger held on the photo while dragging a slider | the desktop Alt/Option view (clipping/mask) | clipping overlay |
| Crop frame | double tap | reset the crop | crop reset |
| Grid | pinch or two-finger swipe | thumbnail size | grid size |
| Grid | tap / long press | open / start multi-select | APK-30b |

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
