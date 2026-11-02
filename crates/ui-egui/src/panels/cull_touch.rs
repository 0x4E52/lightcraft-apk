//! Culling by finger (APK-55, `docs/android-touch-ui.md` §2): the loupe's one- and two-finger gestures
//! that rate, flag and move through photos without opening a panel, and **Cull Mode** (`view.cullMode`), the
//! loupe full screen with the filmstrip, a bar with Next Unflagged and Assisted Culling, and a two-up Compare on
//! a long press of a filmstrip photo.
//!
//! The gestures read the frame's raw [`egui::Event::Touch`] events, so a mouse and the keys are unchanged: a
//! mouse never sends one. Each gesture runs a command (`library.next`, `photo.increaseRating`, …), one undo step
//! where it edits, and the edits show a short HUD over the photo.
//!
//! | Gesture (loupe) | Does |
//! |---|---|
//! | swipe sideways at fit | `library.next` (to the left) / `library.previous` (to the right) |
//! | swipe up / down on the left half at fit | `photo.increaseRating` / `photo.decreaseRating` |
//! | swipe up / down on the right half at fit | `photo.increaseFlag` / `photo.decreaseFlag` (pick, unflagged, reject) |
//! | single tap | hide / show all controls (`view.lightsOut` off / on) |
//! | double tap | zoom to the click-zoom ratio at the point, again to fit |
//! | two-finger tap | `view.infoOverlay` (cycles the info overlays) |
//!
//! Zoomed in, a one-finger drag pans (as before) and is never a swipe.

use egui::{Color32, Pos2, Rect, Vec2, pos2, vec2};
use lightcraft_catalog::{Flag, PhotoId};
use serde_json::{Value, json};

use crate::LightcraftApp;
use crate::state::{LightsOut, ViewMode, Zoom};
use crate::theme::Tokens;
use crate::widgets::register;

/// Farthest a finger may travel (points) and still tap.
pub const TAP_SLOP: f32 = 16.0;
/// Longest a tap lasts (seconds); longer is a press and hold (the context menu).
pub const TAP_SECS: f64 = 0.35;
/// A second tap within this time (and [`DOUBLE_TAP_DIST`]) of the first makes a double tap, so a single tap
/// acts only once this has passed (egui's own double-click delay).
pub const DOUBLE_TAP_SECS: f64 = 0.3;
/// How far apart the two taps of a double tap may be (points).
pub const DOUBLE_TAP_DIST: f32 = 40.0;
/// Shortest swipe (points along its main axis).
pub const SWIPE_MIN: f32 = 56.0;
/// Longest a swipe lasts (seconds): a slower drag is not a flick.
pub const SWIPE_SECS: f64 = 0.8;
/// A swipe's main axis is at least this many times its other one; a diagonal does nothing.
pub const SWIPE_AXIS: f32 = 1.5;
/// How long the HUD of a rating or flag stays (seconds).
pub const HUD_SECS: f64 = 0.9;

/// What the fingers did, once they all lifted.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Gesture {
    /// One finger, briefly, in place (reported once no second tap followed).
    Tap(Pos2),
    /// Two taps in quick succession at about the same place.
    DoubleTap(Pos2),
    /// Two fingers, briefly, in place.
    TwoFingerTap,
    /// One finger flicked from `start` by `delta` (mostly along one axis), at fit.
    Swipe { start: Pos2, delta: Vec2 },
}

#[derive(Clone, Copy, Debug)]
struct Finger {
    id: u64,
    start: Pos2,
    last: Pos2,
    travel: f32,
}

/// The loupe's touch gesture recogniser: raw touch events in, [`Gesture`]s out.
#[derive(Debug, Default)]
pub struct Gestures {
    fingers: Vec<Finger>,
    /// Most fingers down at once in this gesture.
    most: usize,
    /// When this gesture's first finger came down.
    began: f64,
    /// The photo was zoomed in when it began (a drag pans then, never swipes).
    began_zoomed: bool,
    /// It began outside the loupe (another layer, a panel, a dialog): ignored until every finger lifts.
    foreign: bool,
    /// A finger was cancelled (the system took the touch): the gesture does nothing.
    cancelled: bool,
    /// The fingers of this gesture that lifted already.
    ended: Vec<Finger>,
    /// The last tap (end time, place), waiting for a possible second one.
    pending_tap: Option<(f64, Pos2)>,
    /// A finger was down, or lifted, in the latest frame fed: the pointer's clicks are the finger's.
    touched: bool,
}

impl Gestures {
    /// Feed one frame's events at time `now`. `on_loupe` says whether a finger that comes down at a point lands
    /// on the loupe; `zoomed` whether the photo is zoomed in. Returns the gestures that ended this frame and a
    /// single tap whose double-tap window has passed.
    pub fn feed(&mut self, events: &[egui::Event], now: f64, on_loupe: impl Fn(Pos2) -> bool, zoomed: bool) -> Vec<Gesture> {
        let mut out = Vec::new();
        self.touched = !self.fingers.is_empty();
        for e in events {
            let egui::Event::Touch { id, phase, pos, .. } = e else { continue };
            self.touched = true;
            let id = id.0;
            match phase {
                egui::TouchPhase::Start => {
                    if self.fingers.is_empty() {
                        // a tap whose double-tap window passed while no frame came: it was a single tap
                        if let Some((t, at)) = self.pending_tap
                            && now - t > DOUBLE_TAP_SECS
                        {
                            self.pending_tap = None;
                            out.push(Gesture::Tap(at));
                        }
                        self.most = 0;
                        self.began = now;
                        self.began_zoomed = zoomed;
                        self.foreign = !pos.x.is_finite() || !pos.y.is_finite() || !on_loupe(*pos);
                        self.cancelled = false;
                        self.ended.clear();
                    }
                    self.fingers.retain(|f| f.id != id);
                    self.fingers.push(Finger { id, start: *pos, last: *pos, travel: 0.0 });
                    self.most = self.most.max(self.fingers.len());
                }
                egui::TouchPhase::Move => {
                    if let Some(f) = self.fingers.iter_mut().find(|f| f.id == id) {
                        f.last = *pos;
                        let d = (*pos - f.start).length();
                        if d.is_finite() {
                            f.travel = f.travel.max(d);
                        }
                    }
                }
                egui::TouchPhase::End | egui::TouchPhase::Cancel => {
                    let Some(i) = self.fingers.iter().position(|f| f.id == id) else { continue };
                    let mut f = self.fingers.remove(i);
                    f.last = *pos;
                    let d = (*pos - f.start).length();
                    if d.is_finite() {
                        f.travel = f.travel.max(d);
                    }
                    if *phase == egui::TouchPhase::Cancel {
                        self.cancelled = true;
                    }
                    self.ended.push(f);
                    if self.fingers.is_empty() {
                        let ended = std::mem::take(&mut self.ended);
                        if !self.foreign && !self.cancelled {
                            out.extend(self.classify(&ended, now, zoomed));
                        }
                    }
                }
            }
        }
        if let Some((t, at)) = self.pending_tap
            && now - t > DOUBLE_TAP_SECS
            && self.fingers.is_empty()
        {
            self.pending_tap = None;
            out.push(Gesture::Tap(at));
        }
        out
    }

    /// The fingers of the gesture that just ended (each finger as it lifted).
    fn classify(&mut self, ended: &[Finger], now: f64, zoomed: bool) -> Option<Gesture> {
        let secs = now - self.began;
        let still = ended.iter().all(|f| f.travel < TAP_SLOP);
        match (self.most, ended) {
            (1, [f]) if still && secs <= TAP_SECS => match self.pending_tap.take() {
                Some((t, at)) if self.began - t <= DOUBLE_TAP_SECS && (f.start - at).length() <= DOUBLE_TAP_DIST => Some(Gesture::DoubleTap(f.start)),
                _ => {
                    self.pending_tap = Some((now, f.start));
                    None
                }
            },
            (1, [f]) if !self.began_zoomed && !zoomed && secs <= SWIPE_SECS => {
                self.pending_tap = None;
                let delta = f.last - f.start;
                let (ax, ay) = (delta.x.abs(), delta.y.abs());
                let along = ax.max(ay);
                (delta.is_finite() && along >= SWIPE_MIN && along >= SWIPE_AXIS * ax.min(ay)).then_some(Gesture::Swipe { start: f.start, delta })
            }
            (2, [_, _]) if still && secs <= TAP_SECS => {
                self.pending_tap = None;
                Some(Gesture::TwoFingerTap)
            }
            _ => {
                self.pending_tap = None;
                None
            }
        }
    }

    /// A finger is down, or lifted this frame: the pointer's click is a finger's (the gestures decide).
    pub fn touched(&self) -> bool {
        self.touched
    }

    /// A single tap waits to see whether a second follows.
    pub fn waiting(&self) -> bool {
        self.pending_tap.is_some()
    }

    /// The gesture under way is another widget's (egui gave the click to a button drawn over the photo): it does
    /// nothing when it ends. Call before [`Self::feed`] for the frame.
    pub fn disown(&mut self) {
        if !self.fingers.is_empty() {
            self.foreign = true;
        }
    }
}

/// The loupe's gestures, called by `detail::show` each frame with the loupe's own `resp`. `act` is false while a
/// tool owns the photo (crop, masking, remove, red eye, a picker): the recogniser keeps up but does nothing.
/// Returns whether a finger owns this frame's pointer, so the loupe's mouse click (click zoom) stays the mouse's.
///
/// A gesture counts only when it starts on the loupe widget itself: the widgets drawn over the photo on its layer
/// (the filter pill, face-region labels and their x, the navigator, the render-failed pill) keep their taps, and
/// a tap egui gives to one of them as a click is not also a tap of the loupe.
pub(crate) fn loupe(app: &mut LightcraftApp, ui: &egui::Ui, resp: &egui::Response, canvas: Rect, img: Rect, act: bool) -> bool {
    let zoomed = img.width() > canvas.width() + 1.0 || img.height() > canvas.height() + 1.0;
    let layer = ui.layer_id();
    let ctx = ui.ctx().clone();
    let (events, now) = ui.input(|i| (i.events.clone(), i.time));
    let clicked = ctx.interaction_snapshot(|i| i.clicked);
    // pressed on the loupe (a widget over it that senses drags, the navigator or a region handle, takes the press),
    // or pressed and released on it within this frame
    let pressed_here = resp.is_pointer_button_down_on() || clicked == Some(resp.id);
    let on_loupe = |p: Pos2| pressed_here && canvas.contains(p) && ctx.layer_id_at(p).is_none_or(|l| l == layer);
    // a click-only widget over the photo (the filter pill, a region's label or x) shares the press with the loupe's
    // drag: the click egui gives it when the finger lifts says the gesture was the widget's
    if clicked.is_some_and(|c| c != resp.id) {
        app.loupe_gestures.disown();
    }
    // with a panel of sliders open, a one-finger drag on the photo chooses and changes them (APK-61): a flick is
    // that gesture then, not a swipe
    let sliders = super::photo_drag::loupe(app, &ctx, &events, on_loupe, canvas, act, zoomed);
    let gestures = app.loupe_gestures.feed(&events, now, on_loupe, zoomed);
    if app.loupe_gestures.waiting() {
        ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(DOUBLE_TAP_SECS));
    }
    if act {
        for g in gestures.into_iter().filter(|g| !(sliders && matches!(g, Gesture::Swipe { .. }))) {
            run(app, ui.ctx(), g, canvas, img, zoomed);
        }
    }
    app.loupe_gestures.touched()
}

/// What a gesture does in the loupe.
fn run(app: &mut LightcraftApp, ctx: &egui::Context, g: Gesture, canvas: Rect, img: Rect, zoomed: bool) {
    let Some(id) = app.session.active() else { return };
    let result = match g {
        Gesture::Tap(_) => {
            let mode = if app.ui.lights_out == LightsOut::On { "off" } else { "on" };
            app.run("view.lightsOut", json!({"mode": mode})).map(|_| ())
        }
        Gesture::DoubleTap(at) => {
            toggle_zoom(app, img, at, zoomed);
            Ok(())
        }
        Gesture::TwoFingerTap => app.run("view.infoOverlay", json!({})).map(|_| ()),
        Gesture::Swipe { start, delta } => {
            if delta.x.abs() > delta.y.abs() {
                app.run(if delta.x < 0.0 { "library.next" } else { "library.previous" }, json!({})).map(|_| ())
            } else {
                let up = delta.y < 0.0;
                let rating = start.x < canvas.center().x;
                let cmd = match (rating, up) {
                    (true, true) => "photo.increaseRating",
                    (true, false) => "photo.decreaseRating",
                    (false, true) => "photo.increaseFlag",
                    (false, false) => "photo.decreaseFlag",
                };
                let r = app.run(cmd, json!({"ids": [id.0]})).map(|_| ());
                if r.is_ok() {
                    let now = ctx.input(|i| i.time);
                    app.ui.cull_hud = Some((hud_text(app, id, rating), now + HUD_SECS));
                    ctx.request_repaint();
                }
                r
            }
        }
    };
    if let Err(e) = result {
        app.toast(ctx, e);
    }
}

/// Fit ↔ the click-zoom ratio at `at` (a double tap; the mouse's single click does the same).
pub(crate) fn toggle_zoom(app: &mut LightcraftApp, img: Rect, at: Pos2, zoomed: bool) {
    if zoomed || !matches!(app.ui.zoom, Zoom::Fit) {
        app.ui.zoom = Zoom::Fit;
    } else {
        let u = ((at.x - img.left()) / img.width().max(1.0)).clamp(0.0, 1.0);
        let v = ((at.y - img.top()) / img.height().max(1.0)).clamp(0.0, 1.0);
        if u.is_finite() && v.is_finite() {
            app.ui.pan = (u, v);
        }
        app.ui.zoom = Zoom::Percent(app.ui.click_zoom as f32);
    }
    app.ui.zoom_anim = true;
}

/// The HUD after a rating or flag swipe: the photo's new rating or flag.
fn hud_text(app: &LightcraftApp, id: PhotoId, rating: bool) -> String {
    let Some(p) = app.session.catalog.photo(id) else { return String::new() };
    if rating {
        if p.rating == 0 { crate::i18n::tr("No Rating").to_string() } else { "★".repeat(usize::from(p.rating.min(5))) }
    } else {
        crate::i18n::tr(match p.flag {
            Flag::Pick => "Picked",
            Flag::Reject => "Rejected",
            Flag::None => "Unflagged",
        })
        .to_string()
    }
}

/// Draw the rating / flag HUD over the photo while it lasts.
pub(crate) fn hud(app: &mut LightcraftApp, ui: &egui::Ui, canvas: Rect) {
    let now = ui.input(|i| i.time);
    let Some((text, until)) = app.ui.cull_hud.clone() else { return };
    if now >= until || text.is_empty() {
        app.ui.cull_hud = None;
        return;
    }
    ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64((until - now).clamp(0.0, HUD_SECS)));
    let t = Tokens::get(ui.ctx());
    let p = ui.painter_at(canvas);
    let g = p.layout_no_wrap(text, t.semibold(22.0), Color32::WHITE);
    let r = Rect::from_center_size(canvas.center(), g.size() + vec2(36.0, 20.0));
    p.rect_filled(r, 12.0, Color32::from_black_alpha(170));
    p.galley(r.center() - g.size() / 2.0, g, Color32::WHITE);
    register(ui.ctx(), "loupe:cullHud", r);
}

// ------------------------------------------------------------------------ cull mode

/// `view.cullMode {on?}`: the loupe full screen with the filmstrip and the cull bar; no params toggles.
pub(crate) fn set_mode(app: &mut LightcraftApp, p: &Value) -> Result<Value, String> {
    let on = p.get("on").and_then(Value::as_bool).unwrap_or(!app.ui.cull_mode);
    if on {
        if app.session.active().is_none() {
            return Err("Cull Mode needs a photo: choose one in the grid first".into());
        }
        app.ui.cull_mode = true;
        app.ui.fullscreen = false;
        app.ui.slideshow = None;
        app.ui.view = ViewMode::Detail;
        app.ui.zoom = Zoom::Fit;
        app.ui.tool.clear();
        let _ = app.session.end_interaction();
    } else {
        app.ui.cull_mode = false;
    }
    Ok(json!({"cullMode": app.ui.cull_mode}))
}

/// `view.nextUnflagged`: the next photo in view after the active one that has no flag.
pub(crate) fn next_unflagged(app: &mut LightcraftApp) -> Result<Value, String> {
    let vis = app.session.visible_cloned();
    let active = app.session.active();
    let from = active.and_then(|a| vis.iter().position(|x| *x == a)).map_or(0, |i| i.saturating_add(1));
    let next = vis
        .iter()
        .skip(from)
        .copied()
        .find(|id| app.session.catalog.photo(*id).is_some_and(|p| p.flag == Flag::None))
        .ok_or_else(|| "No unflagged photo after this one".to_string())?;
    app.session.execute("library.select", &json!({"ids": [next.0]})).map_err(|e| e.to_string())?;
    Ok(json!({"id": next.0}))
}

/// A long press on filmstrip photo `id` in Cull Mode: the active photo (the select) and it (the candidate, active
/// while compared, as View ▸ Compare makes it) side by side. The view changes only once both are selected.
pub(crate) fn compare_with(app: &mut LightcraftApp, id: PhotoId) -> Result<Value, String> {
    let sel = app.session.active().ok_or("Compare needs an active photo")?;
    if sel == id {
        return Err("Hold another photo of the filmstrip to compare it with this one".into());
    }
    app.session.execute("library.select", &json!({"ids": [sel.0, id.0], "active": id.0})).map_err(|e| e.to_string())?;
    app.ui.compare = Some((sel.0, id.0));
    app.ui.view = ViewMode::Compare;
    Ok(json!({"select": sel.0, "candidate": id.0}))
}

/// Back from Cull Mode's two-up (the bar's Back, `view.back`): the loupe again, on the photo being culled (the
/// select) alone, not on the candidate with both still selected.
pub(crate) fn leave_compare(app: &mut LightcraftApp) -> Result<Value, String> {
    app.ui.view = ViewMode::Detail;
    let select = app.ui.compare.map(|(a, _)| PhotoId(a)).filter(|a| app.session.catalog.photo(*a).is_some_and(|p| !p.deleted));
    if let Some(a) = select {
        app.session.execute("library.select", &json!({"ids": [a.0]})).map_err(|e| e.to_string())?;
    }
    Ok(json!({"active": app.session.active().map(|a| a.0)}))
}

/// Assisted Culling on the photos in view (the dialog scores the selection when more than one is selected).
fn assisted(app: &mut LightcraftApp) -> Result<Value, String> {
    // the photos in view; the user's selection is left as it is
    app.run("dialog.cull", json!({"scope": "visible"}))
}

/// Cull Mode's screen: the bar along the top, then the loupe (or the two-up Compare) with its filmstrip.
pub(crate) fn show(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let touch = app.ui.touch_density;
    let h = if touch { 56.0 } else { 40.0 };
    let full = ui.max_rect();
    let bar = Rect::from_min_size(full.min, vec2(full.width(), h));
    ui.painter().rect_filled(bar, 0.0, Color32::from_gray(12));
    let mut row = ui.new_child(egui::UiBuilder::new().max_rect(bar.shrink2(vec2(8.0, 4.0))).layout(egui::Layout::left_to_right(egui::Align::Center)));
    let min = if touch { vec2(48.0, 48.0) } else { vec2(0.0, 28.0) };
    let mut clicked = None;
    let compare = app.ui.view == ViewMode::Compare;
    let items: [(&str, &str); 3] = [
        (if compare { "cull:back" } else { "cull:done" }, if compare { "Back" } else { "Done" }),
        ("cull:nextUnflagged", "Next Unflagged"),
        ("cull:assisted", "Assisted Culling…"),
    ];
    for (wid, label) in items {
        let b = row.add(egui::Button::new(egui::RichText::new(crate::i18n::tr(label)).font(t.font(13.0)).color(Color32::WHITE)).min_size(min));
        register(ui.ctx(), wid, b.rect);
        if b.clicked() {
            clicked = Some(wid);
        }
    }
    let r = match clicked {
        Some("cull:done") => app.run("view.cullMode", json!({"on": false})),
        Some("cull:back") => leave_compare(app),
        Some("cull:nextUnflagged") => app.run("view.nextUnflagged", json!({})),
        Some("cull:assisted") => assisted(app),
        _ => Ok(Value::Null),
    };
    if let Err(e) = r {
        app.toast(ui.ctx(), e);
    }
    let rest = Rect::from_min_max(pos2(full.left(), bar.bottom()), full.max);
    let mut view = ui.new_child(egui::UiBuilder::new().id_salt("cull-view").max_rect(rest));
    match app.ui.view {
        ViewMode::Compare => {
            crate::panels::compare::prepare(app);
            let t = Tokens::get(view.ctx());
            let film = Rect::from_min_max(pos2(rest.left(), rest.bottom() - t.film_h), rest.max);
            let pane = Rect::from_min_max(rest.min, pos2(rest.right(), film.top()));
            let mut top = view.new_child(egui::UiBuilder::new().id_salt("cull-compare").max_rect(pane));
            crate::panels::compare::show_compare(app, &mut top);
            crate::panels::detail::filmstrip(app, &mut view, film);
        }
        _ => crate::panels::detail::show(app, &mut view),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(id: u64, phase: egui::TouchPhase, x: f32, y: f32) -> egui::Event {
        egui::Event::Touch { device_id: egui::TouchDeviceId(0), id: egui::TouchId(id), phase, pos: pos2(x, y), force: None }
    }
    use egui::TouchPhase::{Cancel, End, Move, Start};

    fn feed(g: &mut Gestures, frames: &[(f64, Vec<egui::Event>)]) -> Vec<Gesture> {
        frames.iter().flat_map(|(t, e)| g.feed(e, *t, |_| true, false)).collect()
    }

    #[test]
    fn a_flick_is_a_swipe_a_slow_drag_and_a_diagonal_are_not() {
        let mut g = Gestures::default();
        let got =
            feed(&mut g, &[(0.0, vec![ev(1, Start, 100.0, 300.0)]), (0.1, vec![ev(1, Move, 100.0, 200.0)]), (0.2, vec![ev(1, End, 100.0, 150.0)])]);
        assert_eq!(got, vec![Gesture::Swipe { start: pos2(100.0, 300.0), delta: vec2(0.0, -150.0) }]);
        let slow = feed(&mut g, &[(1.0, vec![ev(1, Start, 100.0, 300.0)]), (3.0, vec![ev(1, End, 100.0, 150.0)])]);
        assert!(slow.is_empty(), "{slow:?}");
        let diagonal = feed(&mut g, &[(4.0, vec![ev(1, Start, 100.0, 300.0)]), (4.2, vec![ev(1, End, 200.0, 200.0)])]);
        assert!(diagonal.is_empty(), "{diagonal:?}");
    }

    #[test]
    fn a_tap_waits_for_a_second_and_two_make_a_double_tap() {
        let mut g = Gestures::default();
        let first = feed(&mut g, &[(0.0, vec![ev(1, Start, 50.0, 50.0)]), (0.05, vec![ev(1, End, 51.0, 50.0)])]);
        assert!(first.is_empty() && g.waiting());
        let double = feed(&mut g, &[(0.15, vec![ev(2, Start, 52.0, 50.0)]), (0.2, vec![ev(2, End, 52.0, 50.0)])]);
        assert_eq!(double, vec![Gesture::DoubleTap(pos2(52.0, 50.0))]);
        assert!(!g.waiting());
        let single = feed(&mut g, &[(1.0, vec![ev(3, Start, 50.0, 50.0)]), (1.05, vec![ev(3, End, 50.0, 50.0)]), (1.5, vec![])]);
        assert_eq!(single, vec![Gesture::Tap(pos2(50.0, 50.0))]);
    }

    #[test]
    fn two_fingers_in_place_tap_and_a_pinch_does_not() {
        let mut g = Gestures::default();
        let tap = feed(
            &mut g,
            &[(0.0, vec![ev(1, Start, 50.0, 50.0), ev(2, Start, 150.0, 50.0)]), (0.1, vec![ev(1, End, 50.0, 50.0), ev(2, End, 150.0, 50.0)])],
        );
        assert_eq!(tap, vec![Gesture::TwoFingerTap]);
        let pinch = feed(
            &mut g,
            &[
                (1.0, vec![ev(1, Start, 50.0, 50.0), ev(2, Start, 150.0, 50.0)]),
                (1.1, vec![ev(1, Move, 0.0, 50.0), ev(2, Move, 200.0, 50.0)]),
                (1.2, vec![ev(1, End, 0.0, 50.0), ev(2, End, 200.0, 50.0)]),
            ],
        );
        assert!(pinch.is_empty(), "{pinch:?}");
    }

    #[test]
    fn zoomed_foreign_cancelled_and_hostile_touches_do_nothing() {
        let mut g = Gestures::default();
        let zoomed: Vec<Gesture> = [(0.0, vec![ev(1, Start, 100.0, 300.0)]), (0.2, vec![ev(1, End, 100.0, 100.0)])]
            .iter()
            .flat_map(|(t, e)| g.feed(e, *t, |_| true, true))
            .collect();
        assert!(zoomed.is_empty(), "a drag on a zoomed photo pans: {zoomed:?}");
        let foreign: Vec<Gesture> = [(1.0, vec![ev(1, Start, 100.0, 300.0)]), (1.2, vec![ev(1, End, 100.0, 100.0)])]
            .iter()
            .flat_map(|(t, e)| g.feed(e, *t, |_| false, false))
            .collect();
        assert!(foreign.is_empty(), "a drag that began on a dialog: {foreign:?}");
        let cancelled = feed(&mut g, &[(2.0, vec![ev(1, Start, 100.0, 300.0)]), (2.2, vec![ev(1, Cancel, 100.0, 100.0)])]);
        assert!(cancelled.is_empty(), "{cancelled:?}");
        let nan = feed(&mut g, &[(3.0, vec![ev(1, Start, f32::NAN, 300.0)]), (3.1, vec![ev(1, End, f32::INFINITY, 100.0)])]);
        assert!(nan.is_empty(), "{nan:?}");
        // a gesture disowned before it ends (a button over the photo took the click) does nothing
        let _ = feed(&mut g, &[(2.5, vec![ev(1, Start, 100.0, 300.0)])]);
        g.disown();
        let disowned = feed(&mut g, &[(2.6, vec![ev(1, End, 100.0, 300.0)]), (3.0, vec![])]);
        assert!(disowned.is_empty() && !g.waiting(), "{disowned:?}");
        // an end for a finger never seen, and a frame without fingers, are ignored
        assert!(feed(&mut g, &[(4.0, vec![ev(9, End, 0.0, 0.0)]), (4.1, vec![])]).is_empty());
    }
}
