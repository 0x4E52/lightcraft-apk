//! Access to the photo library: which runtime permissions to ask for on this Android version,
//! whether the user allowed them, and the error a command gives when they didn't (APK-22).
//!
//! The request is `Activity.requestPermissions`, posted to the Java main thread with
//! `AndroidApp::run_on_java_main_thread` and never awaited (module docs of [`crate::jni_env`]).
//! NativeActivity drops `onRequestPermissionsResult`, so the answer is read back with
//! `checkSelfPermission` instead: on every `Focused(true)` (the system dialog closed) and every
//! [`RECHECK_EVERY`] while the request is pending. When Android shows no dialog at all (the user
//! denied twice, or chose "Don't ask again"), the window never loses focus: no `Focused(false)`
//! within [`NO_DIALOG_WITHIN`] of the request means [`Access::Denied`].
//!
//! [`permissions_for_sdk`] and the state machine [`PhotoPermission`] are pure (a clock and the
//! focus events go in, the `checkSelfPermission` answers come from a closure), so every rule is
//! tested on every target; [`check_grants`] and [`request_permissions`] are the JNI calls and exist
//! on Android only. The SDK level is `AndroidApp::sdk_version()` (the `ro.build.version.sdk`
//! property), no JNI `Build.VERSION` read.

use std::time::{Duration, Instant};

/// Reads shared storage on API 30–32 (`android:maxSdkVersion="32"` in the manifest).
pub const READ_EXTERNAL_STORAGE: &str = "android.permission.READ_EXTERNAL_STORAGE";
/// Reads every image in shared storage, from API 33.
pub const READ_MEDIA_IMAGES: &str = "android.permission.READ_MEDIA_IMAGES";
/// The photos the user picked in the system dialog ("Allow limited access"), from API 34.
pub const READ_MEDIA_VISUAL_USER_SELECTED: &str = "android.permission.READ_MEDIA_VISUAL_USER_SELECTED";
/// Unredacted location in a photo's EXIF: without it Android blanks the GPS of files read through
/// shared storage. A runtime permission, so it is asked for with the others.
pub const ACCESS_MEDIA_LOCATION: &str = "android.permission.ACCESS_MEDIA_LOCATION";

/// The first API level with `READ_MEDIA_IMAGES`.
const API_MEDIA_IMAGES: i32 = 33;
/// The first API level with partial photo access (`READ_MEDIA_VISUAL_USER_SELECTED`).
const API_PARTIAL: i32 = 34;

/// No `Focused(false)` this long after a request: Android showed no dialog, so the permission is
/// permanently denied.
pub const NO_DIALOG_WITHIN: Duration = Duration::from_secs(1);
/// How often the permissions are checked again while a request is pending.
pub const RECHECK_EVERY: Duration = Duration::from_millis(500);

/// What a command says when the user denied access to the photos.
pub const DENIED_MESSAGE: &str = "LightCraft needs access to your photos: allow it in Settings ▸ Apps ▸ LightCraft ▸ Permissions";

/// The runtime permissions to ask for on API level `sdk`: the read permission of that version
/// (and the partial one from API 34) plus [`ACCESS_MEDIA_LOCATION`]. Levels under 30 (below the
/// app's minSdk) get the 30–32 set.
pub fn permissions_for_sdk(sdk: i32) -> &'static [&'static str] {
    if sdk >= API_PARTIAL {
        &[READ_MEDIA_IMAGES, READ_MEDIA_VISUAL_USER_SELECTED, ACCESS_MEDIA_LOCATION]
    } else if sdk >= API_MEDIA_IMAGES {
        &[READ_MEDIA_IMAGES, ACCESS_MEDIA_LOCATION]
    } else {
        &[READ_EXTERNAL_STORAGE, ACCESS_MEDIA_LOCATION]
    }
}

/// What `checkSelfPermission` answered for the permissions of [`permissions_for_sdk`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Grants {
    /// Every photo: `READ_MEDIA_IMAGES` (33+) or `READ_EXTERNAL_STORAGE` (30–32).
    pub all: bool,
    /// Only the photos the user picked: `READ_MEDIA_VISUAL_USER_SELECTED` (34+; always `false` below).
    pub selected: bool,
    /// `ACCESS_MEDIA_LOCATION`: GPS in EXIF is read as written.
    pub location: bool,
}

/// The grants on API level `sdk`, from `granted(permission)`; permissions that don't exist on
/// that level are never asked about and count as not granted.
pub fn grants_from(sdk: i32, granted: impl Fn(&str) -> bool) -> Grants {
    let asked = permissions_for_sdk(sdk);
    let has = |name: &str| asked.contains(&name) && granted(name);
    Grants {
        all: if sdk >= API_MEDIA_IMAGES { has(READ_MEDIA_IMAGES) } else { has(READ_EXTERNAL_STORAGE) },
        selected: has(READ_MEDIA_VISUAL_USER_SELECTED),
        location: has(ACCESS_MEDIA_LOCATION),
    }
}

/// Where access to the photo library stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// Not asked yet in this run, and not granted.
    NotAsked,
    /// Asked; the system dialog is up or about to be.
    Pending,
    /// Every photo can be read.
    Granted,
    /// Only the photos the user picked can be read (API 34+).
    Partial,
    /// The user said no, or Android showed no dialog (denied for good): the photos can't be read.
    Denied,
}

impl Access {
    /// Whether photos can be read (all of them, or the picked ones).
    pub fn readable(self) -> bool {
        matches!(self, Access::Granted | Access::Partial)
    }

    /// The error a command that needs the photos gives: `Some` only for [`Access::Denied`], with
    /// where to allow it.
    pub fn error(self) -> Option<&'static str> {
        (self == Access::Denied).then_some(DENIED_MESSAGE)
    }
}

/// The access what was granted gives on API level `sdk` (`None`: nothing readable).
fn access_from(sdk: i32, grants: Grants) -> Option<Access> {
    if grants.all {
        Some(Access::Granted)
    } else if sdk >= API_PARTIAL && grants.selected {
        Some(Access::Partial)
    } else {
        None
    }
}

/// What the state machine is told.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// The window gained (`true`) or lost (`false`) focus. A system dialog takes the focus while
    /// it is up and gives it back when it closes.
    Focused(bool),
    /// A frame: checks again while a request is pending and applies the no-dialog rule.
    Tick,
}

/// Access to the photo library on one device, `NotAsked → Pending → Granted | Partial | Denied`.
/// Pure: the time and the `checkSelfPermission` answers are given to it.
#[derive(Debug, Clone)]
pub struct PhotoPermission {
    sdk: i32,
    access: Access,
    /// When the pending request was posted.
    asked_at: Option<Instant>,
    /// The window lost focus since the request: a system dialog appeared.
    dialog_seen: bool,
    /// When the permissions were last checked.
    checked_at: Option<Instant>,
    /// The last answer of `ACCESS_MEDIA_LOCATION`.
    location: bool,
}

impl PhotoPermission {
    /// The state on API level `sdk` before anything is known: [`Access::NotAsked`]. Call
    /// [`Self::refresh`] with a first check (the user may have allowed access in an earlier run).
    pub fn new(sdk: i32) -> PhotoPermission {
        PhotoPermission { sdk, access: Access::NotAsked, asked_at: None, dialog_seen: false, checked_at: None, location: false }
    }

    /// The device's API level.
    pub fn sdk(&self) -> i32 {
        self.sdk
    }

    /// Where access stands.
    pub fn access(&self) -> Access {
        self.access
    }

    /// Whether location in EXIF is read unredacted (`ACCESS_MEDIA_LOCATION` granted).
    pub fn media_location(&self) -> bool {
        self.location
    }

    /// Ask for access at `now`: the permissions to pass to `requestPermissions`, or `None` when
    /// nothing is to be asked (every photo is readable already, or a request is pending). Asking
    /// again after [`Access::Denied`] or [`Access::Partial`] is allowed: Android decides whether a
    /// dialog appears, and the no-dialog rule turns its silence into `Denied`.
    pub fn request(&mut self, now: Instant) -> Option<&'static [&'static str]> {
        if matches!(self.access, Access::Granted | Access::Pending) {
            return None;
        }
        self.access = Access::Pending;
        self.asked_at = Some(now);
        self.dialog_seen = false;
        self.checked_at = None;
        Some(permissions_for_sdk(self.sdk))
    }

    /// A check outside any request (at the start, or when focus came back from Settings).
    pub fn refresh(&mut self, now: Instant, grants: Grants) {
        self.checked_at = Some(now);
        self.location = grants.location;
        if self.access == Access::Pending {
            // only full access ends a request early: partial access may be what the user had
            // before the dialog, and the dialog's own answer arrives with the focus
            if access_from(self.sdk, grants) == Some(Access::Granted) {
                self.settle(Access::Granted);
            }
            return;
        }
        self.access = match (access_from(self.sdk, grants), self.access) {
            (Some(access), _) => access,
            // revoked in Settings (Android normally ends the process then): may be asked again
            (None, Access::Granted | Access::Partial) => Access::NotAsked,
            (None, other) => other,
        };
    }

    /// One event at `now`; `check` asks `checkSelfPermission` (called on `Focused(true)` and, while
    /// pending, at most every [`RECHECK_EVERY`]). Answers when to tick again: `Some` while a
    /// request is pending, `None` otherwise.
    pub fn on_event(&mut self, event: Event, now: Instant, check: impl FnOnce() -> Grants) -> Option<Duration> {
        match event {
            Event::Focused(false) => {
                if self.access == Access::Pending {
                    self.dialog_seen = true;
                }
            }
            Event::Focused(true) => {
                let grants = check();
                self.refresh(now, grants);
                // the dialog closed and full access didn't come with it
                if self.access == Access::Pending && self.dialog_seen {
                    self.settle(access_from(self.sdk, grants).unwrap_or(Access::Denied));
                }
            }
            Event::Tick => {
                if self.access == Access::Pending {
                    let due = self.checked_at.is_none_or(|at| now.saturating_duration_since(at) >= RECHECK_EVERY);
                    let grants = due.then(check);
                    if let Some(grants) = grants {
                        self.refresh(now, grants);
                    }
                    let silent = !self.dialog_seen && self.asked_at.is_some_and(|at| now.saturating_duration_since(at) >= NO_DIALOG_WITHIN);
                    if self.access == Access::Pending && silent {
                        // no dialog appeared: denied for good (or the partial access from before)
                        let known = grants.and_then(|g| access_from(self.sdk, g));
                        self.settle(known.unwrap_or(Access::Denied));
                    }
                }
            }
        }
        (self.access == Access::Pending).then(|| self.next_tick(now))
    }

    fn settle(&mut self, access: Access) {
        self.access = access;
        self.asked_at = None;
        self.dialog_seen = false;
    }

    /// How long until the next check or the no-dialog deadline, whichever comes first.
    fn next_tick(&self, now: Instant) -> Duration {
        let recheck = self.checked_at.map_or(Duration::ZERO, |at| RECHECK_EVERY.saturating_sub(now.saturating_duration_since(at)));
        let deadline = match (self.dialog_seen, self.asked_at) {
            (false, Some(at)) => NO_DIALOG_WITHIN.saturating_sub(now.saturating_duration_since(at)),
            _ => RECHECK_EVERY,
        };
        recheck.min(deadline)
    }
}

/// `Context.checkSelfPermission` for each permission of [`permissions_for_sdk`]`(sdk)`, on the
/// calling thread (it only reads the package manager's answer, no Java main thread involved).
#[cfg(target_os = "android")]
pub fn check_grants(app: &android_activity::AndroidApp, sdk: i32) -> Result<Grants, crate::Error> {
    /// `PackageManager.PERMISSION_GRANTED`.
    const PERMISSION_GRANTED: i32 = 0;
    let asked = permissions_for_sdk(sdk);
    let granted = crate::jni_env::with_activity(app, |env, activity| {
        let mut granted = Vec::with_capacity(asked.len());
        for name in asked {
            let is_granted = env.with_local_frame(16, |env| -> Result<bool, crate::Error> {
                let jname = env.new_string(name)?;
                let answer = env.call_method(
                    activity,
                    jni::jni_str!("checkSelfPermission"),
                    jni::jni_sig!("(Ljava/lang/String;)I"),
                    &[jni::JValue::Object(&jname)],
                );
                Ok(crate::dialog::caught(env, answer)?.i()? == PERMISSION_GRANTED)
            })?;
            if is_granted {
                granted.push(*name);
            }
        }
        Ok(granted)
    })?;
    Ok(grants_from(sdk, |name| granted.contains(&name)))
}

/// Posts `Activity.requestPermissions(permissions, …)` to the Java main thread and returns at
/// once; `done` gets what posting answered there (for the log). The user's answer is not reported
/// (NativeActivity drops `onRequestPermissionsResult`): [`PhotoPermission`] reads it back.
#[cfg(target_os = "android")]
pub fn request_permissions(
    app: &android_activity::AndroidApp,
    permissions: &'static [&'static str],
    done: impl FnOnce(Result<(), crate::Error>) + Send + 'static,
) {
    /// The request code (unused: the answer never reaches native code).
    const REQUEST_CODE: i32 = 0x4c43;
    let posted = app.clone();
    app.run_on_java_main_thread(Box::new(move || {
        let asked = crate::jni_env::with_activity(&posted, |env, activity| {
            let len = i32::try_from(permissions.len()).map_err(|_| crate::Error::Jni("too many permissions".to_owned()))?;
            let array = env.new_object_array(len, jni::jni_str!("java/lang/String"), jni::objects::JObject::null())?;
            for (i, name) in permissions.iter().enumerate() {
                let jname = env.new_string(name)?;
                array.set_element(env, i, &jname)?;
            }
            let answer = env.call_method(
                activity,
                jni::jni_str!("requestPermissions"),
                jni::jni_sig!("([Ljava/lang/String;I)V"),
                &[jni::JValue::Object(&array), jni::JValue::Int(REQUEST_CODE)],
            );
            crate::dialog::caught(env, answer).map(|_| ())
        });
        done(asked);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    fn all() -> Grants {
        Grants { all: true, selected: false, location: true }
    }
    fn selected() -> Grants {
        Grants { all: false, selected: true, location: true }
    }
    fn none() -> Grants {
        Grants::default()
    }

    /// A request, then the dialog takes the focus; the user allows, the 500 ms check sees it
    /// before the focus comes back.
    #[test]
    fn pending_then_granted() {
        let t0 = Instant::now();
        let mut p = PhotoPermission::new(34);
        p.refresh(t0, none());
        assert_eq!(p.access(), Access::NotAsked);
        assert_eq!(p.request(t0), Some(permissions_for_sdk(34)));
        assert_eq!(p.access(), Access::Pending);
        // a second request while one is pending asks nothing
        assert_eq!(p.request(t0), None);
        // the first tick checks at once and says when to look again
        assert_eq!(p.on_event(Event::Tick, t0, none), Some(RECHECK_EVERY));
        assert_eq!(p.on_event(Event::Focused(false), t0 + 100 * MS, || panic!("no check on focus loss")), Some(400 * MS));
        // not due yet: no check
        assert_eq!(p.on_event(Event::Tick, t0 + 300 * MS, || panic!("checked too early")), Some(200 * MS));
        // the dialog is up past the no-dialog deadline: still pending
        assert_eq!(p.on_event(Event::Tick, t0 + 1500 * MS, none), Some(RECHECK_EVERY));
        assert_eq!(p.access(), Access::Pending);
        assert!(p.on_event(Event::Tick, t0 + 2000 * MS, all).is_none());
        assert_eq!(p.access(), Access::Granted);
        assert!(p.media_location());
        // granted: nothing more to ask
        assert_eq!(p.request(t0 + 3000 * MS), None);

        // the dialog closes before a check sees it: the focus brings the answer
        let mut p = PhotoPermission::new(33);
        p.request(t0);
        p.on_event(Event::Focused(false), t0 + 50 * MS, none);
        assert!(p.on_event(Event::Focused(true), t0 + 200 * MS, all).is_none());
        assert_eq!(p.access(), Access::Granted);
        // allowed in an earlier run: granted from the first check, never asked
        let mut p = PhotoPermission::new(36);
        p.refresh(t0, all());
        assert_eq!(p.access(), Access::Granted);
        assert_eq!(p.request(t0), None);
    }

    /// "Allow limited access" on API 34+ is `Partial`, which may be asked again (to pick more).
    #[test]
    fn partial_access_is_reported() {
        let t0 = Instant::now();
        let mut p = PhotoPermission::new(34);
        p.request(t0);
        p.on_event(Event::Tick, t0, none);
        p.on_event(Event::Focused(false), t0 + 100 * MS, none);
        // the checks see the picked photos while the dialog is still up: still pending
        p.on_event(Event::Tick, t0 + 600 * MS, selected);
        assert_eq!(p.access(), Access::Pending);
        assert!(p.on_event(Event::Focused(true), t0 + 900 * MS, selected).is_none());
        assert_eq!(p.access(), Access::Partial);
        assert!(p.access().readable());
        assert_eq!(p.access().error(), None);
        // asking again to pick more photos: the dialog appears, the user allows all
        assert!(p.request(t0 + 2000 * MS).is_some());
        p.on_event(Event::Tick, t0 + 2000 * MS, selected);
        assert_eq!(p.access(), Access::Pending, "partial access from before is no answer");
        p.on_event(Event::Focused(false), t0 + 2100 * MS, selected);
        p.on_event(Event::Focused(true), t0 + 2500 * MS, all);
        assert_eq!(p.access(), Access::Granted);
        // asking again with no dialog appearing keeps the partial access, not Denied
        let mut p = PhotoPermission::new(35);
        p.refresh(t0, selected());
        assert_eq!(p.access(), Access::Partial);
        p.request(t0);
        p.on_event(Event::Tick, t0 + 1000 * MS, selected);
        assert_eq!(p.access(), Access::Partial);
        // from Settings: the picked photos withdrawn, all allowed
        p.on_event(Event::Focused(true), t0 + 5000 * MS, all);
        assert_eq!(p.access(), Access::Granted);
    }

    /// Below API 34 the partial permission doesn't exist: never `Partial`, even if a check said so.
    #[test]
    fn api33_never_partial() {
        let t0 = Instant::now();
        for sdk in [30, 32, 33] {
            let mut p = PhotoPermission::new(sdk);
            p.refresh(t0, selected());
            assert_eq!(p.access(), Access::NotAsked, "sdk {sdk}");
            p.request(t0);
            p.on_event(Event::Focused(false), t0 + 100 * MS, selected);
            p.on_event(Event::Focused(true), t0 + 500 * MS, selected);
            assert_eq!(p.access(), Access::Denied, "sdk {sdk}");
            // nor does the grant mapping report what the version can't ask for
            assert!(!grants_from(sdk, |_| true).selected, "sdk {sdk}");
            assert!(grants_from(sdk, |_| true).all, "sdk {sdk}");
        }
        assert!(grants_from(34, |name| name == READ_MEDIA_VISUAL_USER_SELECTED).selected);
        // the legacy permission doesn't count from 33 on, READ_MEDIA_IMAGES doesn't before
        assert!(!grants_from(33, |name| name == READ_EXTERNAL_STORAGE).all);
        assert!(!grants_from(32, |name| name == READ_MEDIA_IMAGES).all);
    }

    /// Permanently denied: Android shows no dialog, so the focus never goes; after 1 s it's `Denied`.
    /// The dialog answered "Deny": the focus comes back without access, `Denied` at once.
    #[test]
    fn no_dialog_means_denied() {
        let t0 = Instant::now();
        let mut p = PhotoPermission::new(36);
        p.request(t0);
        assert_eq!(p.on_event(Event::Tick, t0, none), Some(RECHECK_EVERY));
        assert_eq!(p.on_event(Event::Tick, t0 + 600 * MS, none), Some(400 * MS), "the deadline comes before the next check");
        assert_eq!(p.access(), Access::Pending);
        assert!(p.on_event(Event::Tick, t0 + 1000 * MS, none).is_none());
        assert_eq!(p.access(), Access::Denied);
        // asked again: still no dialog, still denied
        assert!(p.request(t0 + 2000 * MS).is_some());
        p.on_event(Event::Tick, t0 + 3100 * MS, none);
        assert_eq!(p.access(), Access::Denied);

        // the dialog appeared and the user denied
        let mut p = PhotoPermission::new(33);
        p.request(t0);
        p.on_event(Event::Focused(false), t0 + 200 * MS, none);
        assert!(p.on_event(Event::Focused(true), t0 + 800 * MS, none).is_none());
        assert_eq!(p.access(), Access::Denied);
        // allowed later in Settings: the focus coming back finds it
        p.on_event(Event::Focused(true), t0 + 60_000 * MS, all);
        assert_eq!(p.access(), Access::Granted);
        // and revoked there: may be asked again
        p.on_event(Event::Focused(true), t0 + 90_000 * MS, none);
        assert_eq!(p.access(), Access::NotAsked);
        // a Focused(true) that isn't the dialog closing (no focus loss since the request) is
        // left to the deadline
        let mut p = PhotoPermission::new(34);
        p.request(t0);
        p.on_event(Event::Focused(true), t0 + 10 * MS, none);
        assert_eq!(p.access(), Access::Pending);
    }

    #[test]
    fn denied_gives_actionable_error() {
        assert_eq!(Access::Denied.error(), Some("LightCraft needs access to your photos: allow it in Settings ▸ Apps ▸ LightCraft ▸ Permissions"));
        assert!(!Access::Denied.readable());
        for access in [Access::NotAsked, Access::Pending, Access::Granted, Access::Partial] {
            assert_eq!(access.error(), None, "{access:?}");
        }
        assert!(Access::Granted.readable() && !Access::NotAsked.readable() && !Access::Pending.readable());
    }

    /// Every version asks for ACCESS_MEDIA_LOCATION, else EXIF GPS read through shared storage is
    /// blanked; and each asks for its own read permission.
    #[test]
    fn request_includes_media_location() {
        assert_eq!(permissions_for_sdk(30), [READ_EXTERNAL_STORAGE, ACCESS_MEDIA_LOCATION]);
        assert_eq!(permissions_for_sdk(32), [READ_EXTERNAL_STORAGE, ACCESS_MEDIA_LOCATION]);
        assert_eq!(permissions_for_sdk(33), [READ_MEDIA_IMAGES, ACCESS_MEDIA_LOCATION]);
        assert_eq!(permissions_for_sdk(34), [READ_MEDIA_IMAGES, READ_MEDIA_VISUAL_USER_SELECTED, ACCESS_MEDIA_LOCATION]);
        assert_eq!(permissions_for_sdk(36), permissions_for_sdk(34));
        // below minSdk (never installed there): the oldest set
        assert_eq!(permissions_for_sdk(0), permissions_for_sdk(30));
        for sdk in [i32::MIN, 29, 30, 33, 34, 99, i32::MAX] {
            assert!(PhotoPermission::new(sdk).request(Instant::now()).unwrap().contains(&ACCESS_MEDIA_LOCATION), "sdk {sdk}");
        }
        // location without it: granted access, GPS redacted
        let mut p = PhotoPermission::new(34);
        p.refresh(Instant::now(), Grants { all: true, selected: false, location: false });
        assert_eq!(p.access(), Access::Granted);
        assert!(!p.media_location());
    }
}
