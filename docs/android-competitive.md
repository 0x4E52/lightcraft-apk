# Android: what users criticise in Lightroom mobile and its rivals

*Last updated: 2026-10-31.* Revision history at the end.

What people who edit photos on a phone or tablet complain about in Lightroom mobile (Android first, iOS where it is
the same app) and in its closest rivals, what they would miss, and what they keep asking for. It feeds the "Beyond
Lightroom" task cards in [`android-beyond.md`](android-beyond.md), which link the section each card answers.

**Clean-room.** This page records *behaviour and complaints* in our own words, from public forum threads, reviews and
press. Nothing is copied: no code, text beyond a few quoted words, icons, screenshots, presets or profiles. RapidRAW,
CalibRaw, darktable, RawTherapee, ART and every other GPL/AGPL editor are named here as products users compare, never
as code references: their source is not read for LightCraft work (`CLAUDE.md` → "Clean-room").

**How it was gathered (2026-10-31).** Web searches, mostly 2024–2026 sources. Reddit, Google Play, Android Police, XDA
and Lightroom Queen could not be fetched from the agent container (proxy), so most evidence is search-result text of
Adobe Community threads, review sites and aggregators; Play Store reviews and Reddit threads could not be counted.
Frequencies are therefore "often" (many independent threads or sources) or "sometimes" (two or three). **(single
source)** marks a claim found in one place only. Lines worth re-checking when those sites are reachable are marked
**(verify)**.

## Complaints about Lightroom mobile, ranked by how often they appear

### C1 Sync is unreliable
Often.
- Uploads stuck for weeks ("20 elements uploading", Oct 2025); Android → iPad sync stops working (Jun 2025); a Samsung
  S25 uploads only part of a library (late 2025); desktop → device sync taking over 24 hours.
- Named versions synced from desktop not showing as edited on Android (fixed in 10.4.4).
- Classic ↔ mobile sync described as "luck or chance" (r/Lightroom, Oct 2024).
- Adobe's usual advice: clear sync errors in Lightroom Web, keep the app open until it finishes.

### C2 Crashes and lost edits
Often.
- Pixel 7, 8 Pro and 9 Pro XL: crash at start or hang on the loading screen; Android 16 QPR1 made the app unusable for
  one user; Adobe suggested starting in flight mode.
- Crop crashes on a Galaxy Tab S7+ and S24 Ultra, with edits lost on reopen; the masking brush closes the app on a Z
  Fold (2025); copy/paste with masks "very often crashes" (v9.2).
- A free-tier user lost about 500 edits made without cloud sync.

### C3 Paywall and subscription
Often.
- Masking, Generative Remove, Lens Blur, premium presets and cloud sync are Premium-only; TechRadar (2024): the best
  tools are always behind a paywall.
- Choosing a tool on a raw opens "UPGRADE NOW" and asks for a payment method; trials that need a card; trials that
  turn into recurring charges users can't downgrade.
- Price rises: mobile Premium £4.99 → £6.99 (2026, single source); the 2025 photography-plan change raised monthly
  prices by up to 68 % and prompted cancellations; a Google Play subscription unlocks mobile only.
- Capterra reviewers call the model "abhorrent" or costly.

### C4 Storage bloat
Often.
- 103 GB on one Android phone, 90 % of another's storage, 36 GB on an iPhone.
- The app's own storage figure disagrees with Android's (97.7 GB in the app vs 6.6 GB in Settings ▸ Apps).
- Card imports copy every raw into app storage instead of reading it in place.

### C5 Raw and card import on Android
Often.
- Since 5.3.1 the app imports only files Android's media database classifies as images, so CR3 and 3FR appear greyed
  out; users rename `.CR3` to `.CR2`; an Oct 2025 update stopped CR3 import from the gallery altogether.
- OTG card readers aren't listed while the file manager sees them; the camera must be set to PTP; a phone with both a
  microSD and an external card gets confused.
- Fujifilm compressed RAF shows no import thumbnails from a card reader (iOS).

### C6 No local folders, weak offline use
Often.
- Long-standing requests: open device folders and external drives without importing; a LAN sync instead of the cloud.
- Older Android versions refused to start offline because Google Play's licence check blocked them; photos turn
  blurry when swiping offline (2023).
- Travellers call missing offline editing a "showstopper" and want to cull, rate and lightly edit without internet.

### C7 Export limits and bugs
Sometimes to often.
- Metadata and GPS dropped or inconsistent between share, save and export; batch export to Apple Photos drops GPS
  (Sep 2026); camera and lens EXIF lost when Content Credentials are off.
- Exports fixed at 72 or 96 dpi; Android export limited to 4 images at a time (older); in 2026 every format but
  "Original" failing on one device; a download that is the unedited original.
- Gain-map (Ultra HDR) JPEG export breaks HDR edits (Jan 2026); no DNG export on iPhone; no choice of destination;
  no watermark on shared albums.

### C8 UI regressions and layout
Sometimes to often.
- The 2023 edit-panel redesign: a "restore the previous edit UI" request ran for more than a year; flags and ratings
  buried; the tool list didn't scroll; no way back to an older version.
- Landscape editing removed on iPhone in 9.0 and still requested (Android kept it); left-handed layout requested for
  phones and Android tablets.
- Preset categories removed from the Recommended tab (Aug 2026); the crop tool not smooth.

### C9 Slowness
Sometimes.
- "Laggy and slow" on budget phones (Capterra); applying an edit to 5 photos takes about a minute on iPad; import
  "clunkier than I would like".
- Masks and AI tools slowing the app (reported mostly on desktop).

### C10 AI features online-only or missing
Sometimes.
- Generative Remove needs a stable connection (cloud); AI Denoise is not on mobile, with users asking for a cloud
  version for iPad and one going back to an old PC; how generative credits apply is unclear.

### C11 Batch editing gaps
Sometimes.
- Auto Settings can't be applied to a selection on Android; batch tools missing for AI masks (update), rotation and
  multi-album download; "waiting for settings" after sync finished.

### C12 Privacy and trust
Sometimes, peaking around news.
- The 2024 Terms of Use change ("may analyse content using machine learning") caused an uproar; photographers raised
  client work under NDA; Adobe then pledged not to train on user content. A similar row in 2023.

### C13 Stylus and tablets
Sometimes.
- No confirmation that S Pen pressure drives the brush mask; S Pen masking "frustrating" on a Tab S8 Ultra; a user
  moving from an iPad Pro calls the Android version "terrible"; Android tablets get features later than iPad.

### C14 Battery and heat
Rare. Old (2019), iPhone-only reports; nothing Android-specific found.

### C15 Regional pricing
No evidence found either way.

## Complaints about the rivals

- **VSCO** (often): tools free for years moved behind a subscription ("a trap"); billing and support complaints;
  crashes mid-edit (Play review, Mar 2025); overwriting the original in the camera roll (single source). Raw editing
  itself stays free.
- **Snapseed**: the 3.0 redesign (iOS Jun 2025, Android Jan 2026) called more cumbersome, with familiar tools moved
  or gone, export among them (single source); years without updates before it; no auto-save; a learning curve; no
  catalogue.
- **Darkroom**: Curves and selective colour paid; crashes on an iPad mini; raw handling called less reliable (single
  source); not full resolution on iPad (one review); no selective brushes in older reviews; restoring purchases
  unclear.
- **Capture One mobile**: PetaPixel (2023) found the iPhone app "not worth the wait": no auto, presets,
  white-balance picker, filmstrip or batch editing; an extra subscription on top of desktop, cloud features needing
  desktop; one user dropped it after a single trip.
- **Pixelmator Photo**: users uneasy about its future since Apple bought it (2024–25), with plans undisclosed.
- **Photo Mate R3**: 10–15 s per export (single source); no independent review found.
- **darktable, RawTherapee**: no official Android or iOS app (Futura-Sciences, Oct 2025).

## What users praise and would miss

- **Edits follow the user** between phone, tablet, desktop and web when sync works; quick phone edits; a laptop-free
  trip (cull and edit on the road, finish at home).
- **One editor everywhere:** the same develop controls on phone and tablet, user presets, adaptive presets
  (TechRadar 2024).
- **Masking** (subject, sky, people): a main reason people pay.
- **Ultra HDR editing on Android** (from 9.0, Pixel first), which PetaPixel said "could upend phone photography".
- **The built-in DNG camera**, free.
- **Copy/paste with chosen groups** (All, Modified, Default, None) and batch paste.
- **Works offline** for those who set it up (DPReview); runs on a £150 phone.
- **Watermarks** on Export As and Save to Device.

## Requests that come back

- Edit device folders, SD cards and USB drives in place, without importing.
- A full offline workflow: cull, rate, edit and export with no network.
- A LAN or local sync instead of the cloud.
- AI Denoise and Super Resolution on mobile, on the device.
- Landscape editing on phones, a left-handed layout, the old layout as an option.
- Batch Auto, AI-mask update and rotation across a selection; batch export without a cap.
- Export that keeps metadata and GPS, a destination choice, a DPI setting, DNG export, working HDR gain maps.
- Saved watermarks, and watermarks on shared albums.
- Stylus pressure in brush masks.
- Raw import that doesn't depend on Android's media database (CR3, 3FR, compressed RAF).
- A presets panel that stays open while trying several; folder colours or labels.

## Rivals' strengths users mention

- **Snapseed:** completely free, no ads, account, subscription or watermark; control-point selective edits (up to 8
  points); a strong healing tool; swipe up/down to choose a parameter and sideways to change it (kept in 3.0 next to
  a new arc slider); raws open straight into a Develop tool.
- **Darkroom:** edits the photo library directly with no import; batch editing of any selection plus copy/paste of
  edits; a clean, non-destructive interface; RGB curves; raw and ProRAW; a free core with small one-off purchases.
- **VSCO:** distinctive film looks; free raw import and editing; regular updates.
- **Photo Mate R3:** a library built from device folders, filtered by camera, aperture or ISO; non-destructive raw
  editing; most raw formats; a mobile layout and a full layout to switch between (vendor claims).
- **ON1 Photo RAW Mobile:** free app paired with a one-time-purchase desktop (layers, HDR, focus stacking, AI masks).
- **CalibRaw:** free and open source with presets, curves and masks (Android Police).
- **RapidRAW:** a GPU raw editor that lists Android (vendor claim).
- **Capture One iPad:** strong colour tools, but costly and incomplete.

## Opportunities for LightCraft

Each line names the complaint it answers. [`android-beyond.md`](android-beyond.md) checks them against what
LightCraft already has and turns the real gaps into task cards.

1. No account and no tiers: every tool on the device, free (C3).
2. Folders first: browse and edit device folders, cards and USB drives in place (C4, C5, C6).
3. Raw detection that doesn't depend on Android's media database, and an honest message where Android hides files
   (C5).
4. Camera and card import with verified copies, duplicates skipped, destination chosen (C5).
5. A bounded, visible cache whose figures match Android's (C4).
6. Fully offline, no licence check at start (C6).
7. Sync without a cloud: LAN or a folder the user chooses, with visible per-file state (C1).
8. Crash-proof editing: every change saved as it happens, undo that survives a crash (C2).
9. Export that keeps metadata, any size, every format, no batch cap, destination chosen (C7).
10. AI on the device: denoise, masks and healing without credits or uploads (C10).
11. Layouts the user picks and keeps: portrait and landscape, left-handed, compact or full (C8).
12. Gesture editing that doesn't hide tools, Snapseed-style (C8, rivals).
13. Fast drafts on budget phones, measured (C9).
14. Real batch editing: Auto, masks and rotation across a selection (C11).
15. Stylus pressure and hover in the brush (C13).
16. Saved watermark presets (requests).
17. Privacy that can be checked: no telemetry, every network use listed (C12).
18. A free, fast, offline culling mode (C6).
19. Presets that stay open with live thumbnails (C8, requests).

## Sources

Lightroom mobile, crashes, speed, pricing:
- https://community.adobe.com/questions-680/p-lightroom-on-android-crashes-on-pixel-devices-908857
- https://community.adobe.com/t5/lightroom-ecosystem-cloud-based-discussions/p-lightroom-on-android-crashes-on-pixel-devices/m-p/15478436
- https://community.adobe.com/questions-680/lightroom-mobile-on-android-samsung-tab-s7-and-s24ultra-crashes-when-cropping-1640198
- https://community.adobe.com/questions-680/subject-500-lightroom-edits-gone-no-trial-used-no-cloud-sync-all-work-lost-908242
- https://www.capterra.com/p/233631/Adobe-Photoshop-Lightroom/reviews/
- https://www.techradar.com/pro/software-services/adobe-lightroom-mobile-2024-review
- https://www.picturecorrect.com/exploring-lightroom-mobile-free-features-vs-premium/
- https://community.adobe.com/questions-6/price-increase-of-lr-mobile-premium-1628243
- https://www.digitalcameraworld.com/tech/software/adobe-lightroom-and-photoshop-cost-increasing-for-2025
- https://community.adobe.com/t5/lightroom-classic-discussions/p-information-amp-discussion-regarding-the-recent-january-2025-photography-plan-changes/m-p/15128630
- https://helpx.adobe.com/vn_vi/lightroom-cc/kb/lightroom-1tb-plan-faq.html
- https://www.nikonforums.com/forums/topic/13135-lightroom-mobile-app/
- https://community.adobe.com/t5/lightroom-ecosystem-cloud-based-discussions/is-there-a-free-version-of-lightroom-mobile-it-automatically-signed-me-up-for-a-recurring-charge/m-p/13686375
- https://community.adobe.com/questions-680/extremely-slow-batch-edit-on-ipad-compared-to-android-908248
- https://community.adobe.com/questions-675/lightroom-last-version-slow-using-ai-masking-and-ediitng-969237

Sync:
- https://community.adobe.com/t5/lightroom-ecosystem-cloud-based-discussions/synch-from-android-to-ipad-no-longer-working-correctly/m-p/15379377/highlight/true
- https://community.adobe.com/questions-680/lightroom-mobile-android-20-elements-are-being-uploaded-for-several-weeks-now-908758
- https://community.adobe.com/bug-reports-679/p-lrm-android-named-versions-synced-from-desktop-don-t-appear-as-edited-662276
- https://community.adobe.com/t5/lightroom-ecosystem-cloud-based-discussions/lightroom-cloud-not-syncing-all-photos-how-to-fix-upload-amp-sync-issues-on-samsung-s25/m-p/15603268/highlight/true
- https://community.adobe.com/questions-680/syncing-from-desktop-to-device-takes-a-long-time-24-hours-922176
- https://cal1.lr.ggtyler.dev/r/Lightroom/comments/1gdmmql/syncing_is_atrocious (r/Lightroom mirror)

Storage, import, raw, local files, offline:
- https://community.adobe.com/questions-680/lightroom-android-hogging-phone-storage-space-908095
- https://community.adobe.com/questions-680/lr-android-uses-90-of-phone-storage-why-916534
- https://community.adobe.com/questions-680/lightroom-app-taking-up-36-gb-on-iphone-918861
- https://community.adobe.com/t5/lightroom-ecosystem-cloud-based-discussions/lightroom-mobile-how-to-import-raw-files-in-place-from-internal-sd-card-without-copying/m-p/13432735
- https://community.adobe.com/feature-requests-681/p-android-import-from-external-sd-card-directly-661248
- https://community.adobe.com/bug-reports-679/p-mobile-android-gets-confused-with-both-microsd-and-an-external-sd-cards-662738
- https://community.adobe.com/t5/lightroom-ecosystem-cloud-based-discussions/p-lightroom-mobile-import-screen-does-not-display-my-image-thumbs-like-lr-desktop/m-p/15404478/highlight/true
- https://helpx.adobe.com/ee/lightroom/mobile/add-and-capture-photos/add-and-import-photos/import-photos-from-card-or-cameras.html
- https://community.adobe.com/t5/lightroom-ecosystem-cloud-based-discussions/android-doesn-t-import-cr3-files/m-p/12631391
- https://community.adobe.com/questions-680/lr-mobile-nao-reconhece-arquivos-raw-cr3-canon-em-tablet-samsung-908718
- https://community.adobe.com/questions-680/lightroom-mobile-not-importing-cr3-files-922828
- https://community.adobe.com/feature-requests-681/p-enable-access-to-local-files-folders-on-device-660945
- https://community.adobe.com/t5/lightroom-ecosystem-cloud-based-ideas/p-add-support-local-file-browsing-on-ipad-tablet-just-like-on-desktop/idi-p/14328355
- https://community.adobe.com/t5/lightroom-ecosystem-cloud-based-ideas/p-add-ability-to-edit-locally-in-the-mobile-version/idi-p/14817576
- https://community.adobe.com/bug-reports-679/p-with-airplane-mode-android-lightroom-mobile-does-not-work-due-to-google-play-blocking-662138
- https://community.adobe.com/questions-680/lightroom-mobile-android-photos-are-blurry-in-review-mode-and-offline-927798
- https://community.adobe.com/questions-680/run-lightroom-on-ipad-in-offline-mode-932354
- https://www.dpreview.com/forums/threads/opinions-about-lightroom-on-android.4809118/
- https://cameragx.com/2024/12/09/lightroom-for-mobile-premium-is-a-laptop-free-workflow-really-a-possibility/

Export, UI, batch, AI, stylus, HDR:
- https://community.adobe.com/bug-reports-679/p-metadata-stripping-on-export-from-lightroom-for-mobile-662768
- https://community.adobe.com/questions-680/jpeg-export-loses-camera-lens-exif-when-content-credentials-are-off-on-lightroom-for-iphone-1640926
- https://community.adobe.com/questions-680/missing-geo-data-1641301
- https://community.adobe.com/questions-680/issues-with-download-export-on-android-1553222
- https://community.adobe.com/questions-680/light-room-mobile-cant-export-more-than-4-at-once-and-sometimes-2-914813
- https://community.adobe.com/feature-requests-681/p-choose-destination-when-exporting-from-lightroom-mobile-660867
- https://community.adobe.com/questions-680/lr-on-iphone-no-dng-export-926453
- https://community.adobe.com/questions-680/lightroom-mobile-gainmap-problem-1617235
- https://community.adobe.com/t5/lightroom-ecosystem-cloud-based-discussions/how-to-watermark-images-in-an-album-in-lightroom-before-sharing-it-with-a-client/m-p/15381074
- https://community.adobe.com/feature-requests-681/p-save-and-recall-multiple-watermarks-661931
- https://community.adobe.com/feature-requests-681/p-restore-the-previous-edit-ui-as-an-option-on-lightroom-mobile-661916
- https://community.adobe.com/feature-requests-681/p-restore-editing-in-landscape-mode-on-iphones-661941
- https://community.adobe.com/feature-requests-681/p-add-a-left-handed-option-for-android-iphone-users-661329/index3.html
- https://community.adobe.com/questions-680/p-mobile-presets-panel-under-recommended-i-do-not-see-the-preset-categories-warm-dark-bright-cinematic-hdr-etc-1635499/index4.html
- https://community.adobe.com/t5/lightroom-ecosystem-cloud-based-discussions/crop-tool-is-not-smooth/m-p/15279486/highlight/true
- https://community.adobe.com/t5/lightroom-ecosystem-cloud-based-ideas/p-missing-batch-editing-tools-update-ai-masks-rotation-auto-settings-download-multiple-albums/idi-p/13988913
- https://community.adobe.com/t5/lightroom-ecosystem-cloud-based-discussions/mobile-app-batch-auto-settings/td-p/13927918
- https://helpx.adobe.com/au/lightroom/desktop/using/generative-remove-faq.html
- https://community.adobe.com/feature-requests-681/p-enhance-super-res-raw-details-denoise-to-devices-661372/index3.html
- https://community.adobe.com/feature-requests-681/p-offer-ai-denoise-in-the-cloud-so-that-lightroom-for-ipad-users-could-access-it-1554549
- https://community.adobe.com/questions-680/issues-with-s-pen-on-lightroom-for-samsung-while-using-the-s8-ultra-8gb-128gb-924280
- https://community.adobe.com/questions-680/function-brush-in-masking-mobile-lr-on-android-phone-907852
- https://community.adobe.com/t5/lightroom-ecosystem-cloud-based-discussions/what-device-works-best-tablet-running-android-or-microsoft-surface-pro-11/td-p/15579701
- https://petapixel.com/2023/10/11/lightroom-on-android-now-supports-ultra-hdr-photo-editing/
- https://9to5google.com/2023/10/11/lightroom-android-hdr-support/

Privacy:
- https://www.thephoblographer.com/2024/06/07/the-new-adobe-terms-of-use-have-upset-photographers/
- https://alternativeto.net/news/2024/6/adobe-responds-to-tos-controversy-and-reassures-users-it-won-t-train-ai-on-their-work/
- https://mobilesyrup.com/2024/06/06/adobe-terms-access-user-content-improve-services/

Rivals:
- https://kimola.com/reports/unlock-insights-with-our-vsco-user-feedback-analysis-report-app-store-us-147975
- https://uk.trustpilot.com/review/vsco.co
- https://marlvel.ai/apps/vsco-photo-editor/reviews
- https://www.change.org/p/creators-against-vsco-s-paywall
- https://www.iculture.nl/nieuws/snapseed-update-kritiek-gebruikers-boos/
- https://9to5google.com/2026/01/06/snapseed-android-redesign/
- https://www.alternativeto.net/news/2025/6/snapseed-3-0-for-ios-launches-with-new-interface-and-vintage-filters/
- https://www.techradar.com/cameras/photography/this-is-my-go-to-photo-editor-on-mobile-thats-completely-free-no-strings-attached
- https://shotkit.com/snapseed-beginners-guide/
- https://www.sensepose.com/blog/best-free-lightroom-alternatives-android
- https://www.macstories.net/reviews/darkroom-4-6-adds-video-to-its-excellent-photo-editing-workflow/
- https://www.macstories.net/reviews/darkroom-4-0-the-macstories-review/
- https://thesweetsetup.com/apps/best-photo-editing-app-iphone/
- https://justuseapp.com/en/app/953286746/darkroom-photo-video-editor/reviews
- https://www.sikayetvar.com/en/darkroom-us
- https://petapixel.com/2023/06/14/capture-one-on-iphone-review-its-finally-here-but-wasnt-worth-the-wait
- https://www.thephoblographer.com/2022/08/29/edits-great-more-features-needed-capture-one-for-ipad-review/
- https://support.captureone.com/hc/fr/community/posts/9580921316765-C1-for-iPad-field-report
- https://www.macworld.com/article/2508521/its-day-of-the-dead-and-one-of-my-favorite-mac-apps-isnt-long-for-this-world.html
- https://mezha.ua/en/2025/02/12/pixelmator-and-photomator-have-officially-become-part-of-apple-but-further-plans-are-still-undisclosed/amp/
- https://www.apkmirror.com/apk/ts-systems/photo-mate-r3/
- https://www.gsmarena.com/newscomm-16823.php
- https://www.androidpolice.com/calibraw-is-a-free-open-source-lightroom-alternative-saved-me-from-paying-120/ (search snippet only)
- https://www.xda-developers.com/ditched-lightroom-for-snappy-free-alternative-editing-raw-on-phone-has-never-been-easier/ (search snippet only)
- https://www.getrapidraw.com/
- https://on1.com/getandroid
- https://www.futura-sciences.com/tech/telecharger/darktable-19669

## Revision history

- 2026-10-31: first version, from web research on Lightroom mobile and its rivals (Reddit, Google Play and several
  tech sites not reachable from the container; frequencies are qualitative).
