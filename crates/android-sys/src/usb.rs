//! Cameras plugged in by USB (APK-59): `UsbManager.getDeviceList()` turned into the cameras the
//! device scan lists, and the permission to open one.
//!
//! A camera in PTP / MTP mode is no mounted volume: it shows up as a USB device with a still-image
//! interface (class 6, subclass 1, protocol 1: PTP, which MTP extends) or a vendor interface named
//! "MTP". [`cameras`] is the pure mapper, tested on every target: it keeps those devices, each once
//! by its device name (`/dev/bus/usb/001/002`, what `UsbManager` knows it by), labelled by its
//! manufacturer and product names, at most [`MAX_DEVICES`].
//!
//! The permission: `UsbManager.requestPermission(device, PendingIntent)` with a
//! `PendingIntent.getBroadcast` on an empty package-local intent. Nothing receives that broadcast
//! (NativeActivity has no receiver class, and Java is decision 2), so the answer is read back with
//! `hasPermission(device)`, as the photo permission is (APK-22): on every `Focused(true)` (the
//! system dialog closed) and every [`RECHECK_EVERY`] while the request is pending; no
//! `Focused(false)` within [`NO_DIALOG_WITHIN`] means no dialog came (the camera went away), and
//! the focus coming back without the permission means the user said no. [`UsbPermission`] is that
//! state machine, pure; [`query_cameras`], [`has_permission`] and [`request_permission`] are the
//! JNI calls (Android only).

use std::time::{Duration, Instant};

pub use crate::permissions::{Event, NO_DIALOG_WITHIN, RECHECK_EVERY};

/// `UsbConstants.USB_CLASS_STILL_IMAGE` (PTP, and MTP on top of it).
pub const CLASS_STILL_IMAGE: i32 = 6;
/// `UsbConstants.USB_CLASS_VENDOR_SPEC`: some cameras and phones name such an interface "MTP".
pub const CLASS_VENDOR_SPEC: i32 = 255;

/// At most this many USB devices are read (a broken service can't grow the list forever).
pub const MAX_DEVICES: usize = 32;
/// At most this many interfaces are read per device.
pub const MAX_INTERFACES: usize = 32;

/// The label of a camera that names neither its maker nor its model.
pub const FALLBACK_LABEL: &str = "USB camera";

/// What a camera import says when the user didn't allow LightCraft to use the camera.
pub const DENIED_MESSAGE: &str =
    "LightCraft was not allowed to use the camera. Unplug it, plug it in again, choose Import from Device and allow access.";

/// One `UsbInterface`, as JNI read it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InterfaceRow {
    pub class: i32,
    pub subclass: i32,
    pub protocol: i32,
    /// `getName()` (often null).
    pub name: Option<String>,
}

/// One `UsbDevice`, as JNI read it; a call that failed or answered null is `None` (or empty).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UsbRow {
    /// `getDeviceName()`: `/dev/bus/usb/<bus>/<address>`.
    pub device: Option<String>,
    pub class: i32,
    pub interfaces: Vec<InterfaceRow>,
    pub manufacturer: Option<String>,
    pub product: Option<String>,
}

/// A camera to import from: what `UsbManager` knows it by, and its label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Camera {
    /// `getDeviceName()`.
    pub device: String,
    /// "Canon EOS R5", or [`FALLBACK_LABEL`].
    pub label: String,
}

/// The interface is PTP (still image, 6/1/1) or a vendor interface named MTP.
fn camera_interface(i: &InterfaceRow) -> bool {
    let ptp = i.class == CLASS_STILL_IMAGE && i.subclass == 1 && i.protocol == 1;
    let mtp = i.class == CLASS_VENDOR_SPEC && i.name.as_deref().is_some_and(|n| n.trim().eq_ignore_ascii_case("MTP"));
    ptp || mtp
}

/// A USB device name: `/dev/bus/usb/` and plain components (a row is another process's data).
pub fn plain_device_name(name: &str) -> bool {
    name.strip_prefix("/dev/bus/usb/")
        .is_some_and(|rest| !rest.is_empty() && rest.split('/').all(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_alphanumeric())))
}

/// Trimmed, without control characters, at most 64 chars; `None` when nothing is left.
fn clean_label(s: Option<&str>) -> Option<String> {
    let s: String = s?.chars().filter(|c| !c.is_control()).take(64).collect();
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_owned())
}

/// The cameras among `rows`, in order, each device once (see the module docs).
pub fn cameras(rows: impl IntoIterator<Item = UsbRow>) -> Vec<Camera> {
    let mut out: Vec<Camera> = Vec::new();
    for row in rows.into_iter().take(MAX_DEVICES) {
        let Some(device) = row.device.as_deref().map(str::trim).filter(|d| plain_device_name(d)) else { continue };
        let is_camera = row.class == CLASS_STILL_IMAGE || row.interfaces.iter().take(MAX_INTERFACES).any(camera_interface);
        if !is_camera || out.iter().any(|c| c.device == device) {
            continue;
        }
        let maker = clean_label(row.manufacturer.as_deref());
        let model = clean_label(row.product.as_deref());
        let label = match (maker, model) {
            // "Canon Inc." + "Canon EOS R5": the model says it already
            (Some(maker), Some(model)) if model.to_lowercase().starts_with(maker.split_whitespace().next().unwrap_or("").to_lowercase().as_str()) => {
                model
            }
            (Some(maker), Some(model)) => format!("{maker} {model}"),
            (None, Some(one)) | (Some(one), None) => one,
            (None, None) => FALLBACK_LABEL.to_owned(),
        };
        out.push(Camera { device: device.to_owned(), label });
    }
    out
}

/// Where the permission to use one camera stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsbAccess {
    /// Not asked yet, and not granted.
    NotAsked,
    /// Asked; the system dialog is up or about to be.
    Pending,
    /// The camera may be opened.
    Granted,
    /// The user said no, or no dialog came.
    Denied,
}

/// The permission to use one camera, `NotAsked → Pending → Granted | Denied`. Pure: the time and
/// the `hasPermission` answers are given to it.
#[derive(Debug, Clone)]
pub struct UsbPermission {
    access: UsbAccess,
    asked_at: Option<Instant>,
    dialog_seen: bool,
    checked_at: Option<Instant>,
}

impl Default for UsbPermission {
    fn default() -> Self {
        UsbPermission { access: UsbAccess::NotAsked, asked_at: None, dialog_seen: false, checked_at: None }
    }
}

impl UsbPermission {
    /// Where the permission stands.
    pub fn access(&self) -> UsbAccess {
        self.access
    }

    /// Ask at `now` unless the camera may be opened already or a request is pending; `has` is
    /// `hasPermission` (asked first: "always allow" from an earlier plug-in needs no dialog).
    /// Answers whether to call `requestPermission`.
    pub fn request(&mut self, now: Instant, has: impl FnOnce() -> bool) -> bool {
        if matches!(self.access, UsbAccess::Granted | UsbAccess::Pending) {
            return false;
        }
        self.checked_at = Some(now);
        if has() {
            self.settle(UsbAccess::Granted);
            return false;
        }
        self.access = UsbAccess::Pending;
        self.asked_at = Some(now);
        self.dialog_seen = false;
        true
    }

    /// One event at `now`; `has` asks `hasPermission` (on `Focused(true)` and, while pending, at
    /// most every [`RECHECK_EVERY`]). Answers when to tick again: `Some` while pending.
    pub fn on_event(&mut self, event: Event, now: Instant, has: impl FnOnce() -> bool) -> Option<Duration> {
        if self.access != UsbAccess::Pending {
            return None;
        }
        match event {
            Event::Focused(false) => self.dialog_seen = true,
            Event::Focused(true) => {
                self.checked_at = Some(now);
                let granted = has();
                if granted {
                    self.settle(UsbAccess::Granted);
                } else if self.dialog_seen {
                    // the dialog closed without the permission
                    self.settle(UsbAccess::Denied);
                }
            }
            Event::Tick => {
                let due = self.checked_at.is_none_or(|at| now.saturating_duration_since(at) >= RECHECK_EVERY);
                if due {
                    self.checked_at = Some(now);
                    if has() {
                        self.settle(UsbAccess::Granted);
                    }
                }
                let silent = !self.dialog_seen && self.asked_at.is_some_and(|at| now.saturating_duration_since(at) >= NO_DIALOG_WITHIN);
                if self.access == UsbAccess::Pending && silent {
                    self.settle(UsbAccess::Denied);
                }
            }
        }
        (self.access == UsbAccess::Pending).then(|| self.next_tick(now))
    }

    /// The camera was unplugged: a later plug-in asks again.
    pub fn forget(&mut self) {
        *self = UsbPermission::default();
    }

    fn settle(&mut self, access: UsbAccess) {
        self.access = access;
        self.asked_at = None;
        self.dialog_seen = false;
    }

    fn next_tick(&self, now: Instant) -> Duration {
        let recheck = self.checked_at.map_or(Duration::ZERO, |at| RECHECK_EVERY.saturating_sub(now.saturating_duration_since(at)));
        let deadline = match (self.dialog_seen, self.asked_at) {
            (false, Some(at)) => NO_DIALOG_WITHIN.saturating_sub(now.saturating_duration_since(at)),
            _ => RECHECK_EVERY,
        };
        recheck.min(deadline)
    }
}

/// The action of the package-local intent the permission's `PendingIntent` carries (nothing
/// receives it).
#[cfg(target_os = "android")]
const PERMISSION_ACTION: &str = "ai.storyteller.lightcraft.USB_PERMISSION";
/// `PendingIntent.FLAG_IMMUTABLE` (required from API 31; an implicit mutable one is refused from 34).
#[cfg(target_os = "android")]
const FLAG_IMMUTABLE: i32 = 0x0400_0000;

/// The connected cameras ([`cameras`] of `getDeviceList()`), on the calling thread (attached
/// through [`crate::jni_env::with_activity`]: meant for the device watch's worker).
#[cfg(target_os = "android")]
pub fn query_cameras(app: &android_activity::AndroidApp) -> Result<Vec<Camera>, crate::Error> {
    let rows = crate::jni_env::with_activity(app, |env, activity| {
        env.with_local_frame(16, |env| -> Result<Vec<UsbRow>, crate::Error> {
            let manager = usb_manager(env, activity)?;
            device_rows(env, &manager)
        })
    })?;
    Ok(cameras(rows))
}

/// `getSystemService("usb")`.
#[cfg(target_os = "android")]
pub(crate) fn usb_manager<'l>(env: &mut jni::Env<'l>, activity: &jni::objects::JObject<'_>) -> Result<jni::objects::JObject<'l>, crate::Error> {
    use crate::dialog::caught;
    use jni::refs::Reference;
    use jni::{JValue, jni_sig, jni_str};
    let service = env.new_string("usb")?;
    let answer =
        env.call_method(activity, jni_str!("getSystemService"), jni_sig!("(Ljava/lang/String;)Ljava/lang/Object;"), &[JValue::Object(&service)]);
    let manager = caught(env, answer)?.l()?;
    if manager.is_null() {
        return Err(crate::Error::Jni("no USB service".to_owned()));
    }
    Ok(manager)
}

/// The `UsbDevice` named `device` (`getDeviceList().get(device)`), or `None` when it is gone.
#[cfg(target_os = "android")]
pub(crate) fn device_named<'l>(
    env: &mut jni::Env<'l>,
    manager: &jni::objects::JObject<'_>,
    device: &str,
) -> Result<Option<jni::objects::JObject<'l>>, crate::Error> {
    use crate::dialog::caught;
    use jni::refs::Reference;
    use jni::{JValue, jni_sig, jni_str};
    let answer = env.call_method(manager, jni_str!("getDeviceList"), jni_sig!("()Ljava/util/HashMap;"), &[]);
    let map = caught(env, answer)?.l()?;
    if map.is_null() {
        return Ok(None);
    }
    let key = env.new_string(device)?;
    let answer = env.call_method(&map, jni_str!("get"), jni_sig!("(Ljava/lang/Object;)Ljava/lang/Object;"), &[JValue::Object(&key)]);
    let found = caught(env, answer)?.l()?;
    Ok((!found.is_null()).then_some(found))
}

/// Every `UsbDevice` of `getDeviceList().values()`, each read in its own local frame.
#[cfg(target_os = "android")]
fn device_rows(env: &mut jni::Env<'_>, manager: &jni::objects::JObject<'_>) -> Result<Vec<UsbRow>, crate::Error> {
    use crate::dialog::caught;
    use jni::refs::Reference;
    use jni::{jni_sig, jni_str};
    let answer = env.call_method(manager, jni_str!("getDeviceList"), jni_sig!("()Ljava/util/HashMap;"), &[]);
    let map = caught(env, answer)?.l()?;
    if map.is_null() {
        return Ok(Vec::new());
    }
    let answer = env.call_method(&map, jni_str!("values"), jni_sig!("()Ljava/util/Collection;"), &[]);
    let values = caught(env, answer)?.l()?;
    let answer = env.call_method(&values, jni_str!("iterator"), jni_sig!("()Ljava/util/Iterator;"), &[]);
    let iter = caught(env, answer)?.l()?;
    let mut rows = Vec::new();
    while rows.len() < MAX_DEVICES {
        let answer = env.call_method(&iter, jni_str!("hasNext"), jni_sig!("()Z"), &[]);
        if !crate::activity::answer_bool(env, answer)? {
            break;
        }
        // a device that can't be read is an empty row, never the whole list
        let row = env.with_local_frame(16, |env| -> Result<UsbRow, crate::Error> {
            let answer = env.call_method(&iter, jni_str!("next"), jni_sig!("()Ljava/lang/Object;"), &[]);
            let device = caught(env, answer)?.l()?;
            if device.is_null() {
                return Ok(UsbRow::default());
            }
            Ok(read_device(env, &device))
        });
        match row {
            Ok(row) => rows.push(row),
            Err(_) => {
                let _ = env.exception_catch();
                // `next` itself failed (the map changed under us): stop rather than loop
                break;
            }
        }
    }
    Ok(rows)
}

/// An `int` answer, 0 when the call threw.
#[cfg(target_os = "android")]
fn int_of(env: &mut jni::Env<'_>, answer: jni::errors::Result<jni::objects::JValueOwned<'_>>) -> i32 {
    crate::dialog::caught(env, answer).and_then(|v| Ok(v.i()?)).unwrap_or(0)
}

/// A `String` answer: `None` when the call threw or answered null.
#[cfg(target_os = "android")]
fn text_of<'l>(env: &mut jni::Env<'l>, answer: jni::errors::Result<jni::objects::JValueOwned<'l>>) -> Option<String> {
    let obj = crate::dialog::caught(env, answer).and_then(|v| Ok(v.l()?)).ok()?;
    match crate::content::string_of(env, obj) {
        Ok(text) => text,
        Err(_) => {
            let _ = env.exception_catch();
            None
        }
    }
}

/// One `UsbDevice`'s answers (a call that throws counts as no answer for that field).
#[cfg(target_os = "android")]
fn read_device(env: &mut jni::Env<'_>, device: &jni::objects::JObject<'_>) -> UsbRow {
    use jni::{JValue, jni_sig, jni_str};
    let answer = env.call_method(device, jni_str!("getDeviceName"), jni_sig!("()Ljava/lang/String;"), &[]);
    let name = text_of(env, answer);
    let answer = env.call_method(device, jni_str!("getDeviceClass"), jni_sig!("()I"), &[]);
    let class = int_of(env, answer);
    let answer = env.call_method(device, jni_str!("getManufacturerName"), jni_sig!("()Ljava/lang/String;"), &[]);
    let manufacturer = text_of(env, answer);
    let answer = env.call_method(device, jni_str!("getProductName"), jni_sig!("()Ljava/lang/String;"), &[]);
    let product = text_of(env, answer);
    let answer = env.call_method(device, jni_str!("getInterfaceCount"), jni_sig!("()I"), &[]);
    let count = usize::try_from(int_of(env, answer)).unwrap_or(0).min(MAX_INTERFACES);
    let mut interfaces = Vec::with_capacity(count);
    for i in 0..count {
        let Ok(index) = i32::try_from(i) else { break };
        let row = env.with_local_frame(8, |env| -> Result<InterfaceRow, crate::Error> {
            let answer = env.call_method(device, jni_str!("getInterface"), jni_sig!("(I)Landroid/hardware/usb/UsbInterface;"), &[JValue::Int(index)]);
            let iface = crate::dialog::caught(env, answer)?.l()?;
            let answer = env.call_method(&iface, jni_str!("getInterfaceClass"), jni_sig!("()I"), &[]);
            let class = int_of(env, answer);
            let answer = env.call_method(&iface, jni_str!("getInterfaceSubclass"), jni_sig!("()I"), &[]);
            let subclass = int_of(env, answer);
            let answer = env.call_method(&iface, jni_str!("getInterfaceProtocol"), jni_sig!("()I"), &[]);
            let protocol = int_of(env, answer);
            let answer = env.call_method(&iface, jni_str!("getName"), jni_sig!("()Ljava/lang/String;"), &[]);
            let name = text_of(env, answer);
            Ok(InterfaceRow { class, subclass, protocol, name })
        });
        match row {
            Ok(row) => interfaces.push(row),
            Err(_) => {
                let _ = env.exception_catch();
            }
        }
    }
    UsbRow { device: name, class, interfaces, manufacturer, product }
}

/// `hasPermission(device)` on the calling thread; a camera that is gone answers `false`.
#[cfg(target_os = "android")]
pub fn has_permission(app: &android_activity::AndroidApp, device: &str) -> Result<bool, crate::Error> {
    use jni::{JValue, jni_sig, jni_str};
    crate::jni_env::with_activity(app, |env, activity| {
        env.with_local_frame(16, |env| -> Result<bool, crate::Error> {
            let manager = usb_manager(env, activity)?;
            let Some(found) = device_named(env, &manager, device)? else { return Ok(false) };
            let answer =
                env.call_method(&manager, jni_str!("hasPermission"), jni_sig!("(Landroid/hardware/usb/UsbDevice;)Z"), &[JValue::Object(&found)]);
            crate::activity::answer_bool(env, answer)
        })
    })
}

/// `requestPermission(device, PendingIntent)` on the calling thread (a binder call that returns at
/// once; the system dialog answers later, read back with [`has_permission`]).
#[cfg(target_os = "android")]
pub fn request_permission(app: &android_activity::AndroidApp, device: &str) -> Result<(), crate::Error> {
    use crate::dialog::caught;
    use jni::{JValue, jni_sig, jni_str};
    crate::jni_env::with_activity(app, |env, activity| {
        env.with_local_frame(16, |env| -> Result<(), crate::Error> {
            let manager = usb_manager(env, activity)?;
            let Some(found) = device_named(env, &manager, device)? else {
                return Err(crate::Error::Java { class: "java.io.FileNotFoundException".to_owned(), message: "the camera was unplugged".to_owned() });
            };
            let action = env.new_string(PERMISSION_ACTION)?;
            let intent = env.new_object(jni_str!("android/content/Intent"), jni_sig!("(Ljava/lang/String;)V"), &[JValue::Object(&action)]);
            let intent = caught(env, intent)?;
            let answer = env.call_method(activity, jni_str!("getPackageName"), jni_sig!("()Ljava/lang/String;"), &[]);
            let package = caught(env, answer)?.l()?;
            let answer = env.call_method(
                &intent,
                jni_str!("setPackage"),
                jni_sig!("(Ljava/lang/String;)Landroid/content/Intent;"),
                &[JValue::Object(&package)],
            );
            caught(env, answer)?;
            let answer = env.call_static_method(
                jni_str!("android/app/PendingIntent"),
                jni_str!("getBroadcast"),
                jni_sig!("(Landroid/content/Context;ILandroid/content/Intent;I)Landroid/app/PendingIntent;"),
                &[JValue::Object(activity), JValue::Int(0), JValue::Object(&intent), JValue::Int(FLAG_IMMUTABLE)],
            );
            let pending = caught(env, answer)?.l()?;
            let answer = env.call_method(
                &manager,
                jni_str!("requestPermission"),
                jni_sig!("(Landroid/hardware/usb/UsbDevice;Landroid/app/PendingIntent;)V"),
                &[JValue::Object(&found), JValue::Object(&pending)],
            );
            caught(env, answer)?;
            Ok(())
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ptp() -> InterfaceRow {
        InterfaceRow { class: CLASS_STILL_IMAGE, subclass: 1, protocol: 1, name: None }
    }

    fn row(device: &str, maker: &str, model: &str, interfaces: Vec<InterfaceRow>) -> UsbRow {
        UsbRow { device: Some(device.into()), class: 0, interfaces, manufacturer: Some(maker.into()), product: Some(model.into()) }
    }

    #[test]
    fn usb_rows_map_to_cameras() {
        let keyboard = InterfaceRow { class: 3, subclass: 1, protocol: 1, name: None };
        let mtp = InterfaceRow { class: CLASS_VENDOR_SPEC, subclass: 255, protocol: 0, name: Some(" mtp ".into()) };
        let rows = vec![
            row("/dev/bus/usb/001/002", "Canon Inc.", "Canon EOS R5", vec![ptp()]),
            row("/dev/bus/usb/001/003", "Logitech", "Keyboard", vec![keyboard.clone()]),
            row("/dev/bus/usb/001/004", "FUJIFILM", "X-T5", vec![keyboard, mtp]),
            // the same camera twice: once
            row("/dev/bus/usb/001/002", "Canon Inc.", "Canon EOS R5", vec![ptp()]),
            // a device class of still image, no names
            UsbRow { device: Some("/dev/bus/usb/002/001".into()), class: CLASS_STILL_IMAGE, ..UsbRow::default() },
            // a device name that is not a USB device node: never used
            row("/dev/../data", "Sony", "ILCE-7M4", vec![ptp()]),
            row("/dev/bus/usb/", "Sony", "ILCE-7M4", vec![ptp()]),
            UsbRow { device: None, ..row("", "Sony", "ILCE-7M4", vec![ptp()]) },
            row("/dev/bus/usb/003/001", "\u{1}  ", "Sony ILCE-7M4\n", vec![ptp()]),
        ];
        let c = cameras(rows);
        let labels: Vec<(&str, &str)> = c.iter().map(|c| (c.device.as_str(), c.label.as_str())).collect();
        assert_eq!(
            labels,
            [
                ("/dev/bus/usb/001/002", "Canon EOS R5"),
                ("/dev/bus/usb/001/004", "FUJIFILM X-T5"),
                ("/dev/bus/usb/002/001", FALLBACK_LABEL),
                ("/dev/bus/usb/003/001", "Sony ILCE-7M4"),
            ]
        );
        let many = (0..100).map(|i| row(&format!("/dev/bus/usb/001/{i:03}"), "Nikon", "Z 8", vec![ptp()]));
        assert_eq!(cameras(many).len(), MAX_DEVICES);
    }

    /// The permission's states: granted from an earlier "always allow" without a dialog; a dialog
    /// answered yes (on the focus or a tick) or no; no dialog at all; unplugged and plugged in again.
    #[test]
    fn permission_poll_states() {
        let t0 = Instant::now();
        let ms = |n| t0 + Duration::from_millis(n);

        // allowed before: no request, granted at once
        let mut p = UsbPermission::default();
        assert_eq!(p.access(), UsbAccess::NotAsked);
        assert!(!p.request(t0, || true));
        assert_eq!(p.access(), UsbAccess::Granted);
        assert!(!p.request(ms(10), || panic!("granted: not asked again")));
        assert_eq!(p.on_event(Event::Tick, ms(20), || panic!("not pending")), None);

        // the dialog, then Allow: the focus comes back with the permission
        let mut p = UsbPermission::default();
        assert!(p.request(t0, || false));
        assert_eq!(p.access(), UsbAccess::Pending);
        assert!(!p.request(ms(5), || false), "a pending request isn't made twice");
        assert!(p.on_event(Event::Focused(false), ms(100), || panic!("no check on focus loss")).is_some());
        // ticks within 500 ms of the last check don't ask again
        assert!(p.on_event(Event::Tick, ms(200), || panic!("checked too soon")).is_some());
        assert!(p.on_event(Event::Tick, ms(600), || false).is_some(), "checked again after 500 ms");
        assert_eq!(p.on_event(Event::Focused(true), ms(900), || true), None);
        assert_eq!(p.access(), UsbAccess::Granted);

        // granted while the dialog is still up (a tick finds it)
        let mut p = UsbPermission::default();
        assert!(p.request(t0, || false));
        p.on_event(Event::Focused(false), ms(50), || false);
        assert_eq!(p.on_event(Event::Tick, ms(700), || true), None);
        assert_eq!(p.access(), UsbAccess::Granted);

        // the dialog, then Cancel: the focus comes back without it
        let mut p = UsbPermission::default();
        assert!(p.request(t0, || false));
        p.on_event(Event::Focused(false), ms(100), || false);
        assert_eq!(p.on_event(Event::Focused(true), ms(800), || false), None);
        assert_eq!(p.access(), UsbAccess::Denied);
        // asking again is allowed (a new dialog)
        assert!(p.request(ms(900), || false));
        assert_eq!(p.access(), UsbAccess::Pending);

        // no dialog within a second: denied
        let mut p = UsbPermission::default();
        assert!(p.request(t0, || false));
        let next = p.on_event(Event::Tick, ms(100), || false).unwrap();
        assert!(next <= NO_DIALOG_WITHIN, "{next:?}");
        assert!(p.on_event(Event::Tick, ms(500), || false).is_some());
        assert_eq!(p.on_event(Event::Tick, ms(1000), || false), None);
        assert_eq!(p.access(), UsbAccess::Denied);

        // unplugged: forgotten, so the next plug-in asks again
        p.forget();
        assert_eq!(p.access(), UsbAccess::NotAsked);
    }
}
