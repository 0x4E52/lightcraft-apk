//! Slider gestures on the photo (APK-61, `docs/android-touch-ui.md` §2): with a panel of sliders open (Edit, or
//! the profile browser's Amount), a finger's **vertical drag** on the photo chooses one of the panel's sliders and
//! a **horizontal drag** changes it, with a readout over the photo, as Snapseed edits.
//!
//! - The sliders are the open panel's own: each enabled `develop` control slider the panel draws
//!   ([`super::edit::control`]) offers itself here with the value it shows, so there is no second list and a
//!   closed section, a disabled slider or a closed panel offers nothing.
//! - A drag changes the value through the slider's own path ([`super::edit::apply_slider_out`]: `develop.set`
//!   between `develop.beginInteraction` and `develop.endInteraction`), so a gesture is one undo step and the
//!   photo renders as drafts while the finger moves, as with the slider.
//! - It reads the frame's raw touch events, so a mouse drag never does it. It is off while a tool owns the photo
//!   (crop, masking and its brush, remove / spot, red eye, a picker), zoomed in (a drag pans), with two fingers
//!   (a pinch) and when `UiState::photo_drag` is off (Settings ▸ Interface). A drag the loupe stops seeing (a view
//!   change mid-drag) ends at the frame's end ([`end_frame`]). While it is armed, a one-finger flick on the photo is this
//!   gesture, not `cull_touch`'s swipe (rate, flag, next photo); taps keep their meaning.

use egui::{Color32, Pos2, Rect, Vec2, pos2, vec2};
use lightcraft_develop::controls;
use serde_json::json;

use crate::LightcraftApp;
use crate::theme::Tokens;
use crate::widgets::{SliderOut, register, shown_value};

/// Travel (points) before a finger's drag picks its axis; less is a tap (`cull_touch::TAP_SLOP`).
pub const LOCK: f32 = super::cull_touch::TAP_SLOP;
/// Vertical travel (points) that moves the choice by one slider.
pub const ROW: f32 = 36.0;
/// A horizontal drag the width of the photo's area moves the value this share of the slider's track.
pub const SPAN: f64 = 0.5;
/// How long the readout stays after the finger lifts (seconds).
pub const HUD_SECS: f64 = 0.9;

/// What the finger is doing, as the recogniser reports it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Drag {
    /// A vertical drag: `rows` slider rows from where it locked (down is positive); `first` on the frame it locked.
    Choose { rows: i32, first: bool },
    /// A horizontal drag: `dx` points sideways from where it locked; `first` on the frame it locked.
    Adjust { dx: f32, first: bool },
    /// The horizontal drag ended (the finger lifted, was cancelled, a second finger came, or the gesture was
    /// disarmed): the value stays as it is and the interaction ends.
    End,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Axis {
    Undecided,
    Vertical,
    Horizontal,
}

#[derive(Clone, Copy, Debug)]
struct Finger {
    id: u64,
    start: Pos2,
    /// Where it locked to an axis.
    origin: Pos2,
    axis: Axis,
    /// Rows last reported (a vertical drag reports only changes).
    rows: i32,
}

/// The recogniser: raw touch events in, [`Drag`]s out. One finger only; a gesture that began off the photo, while
/// disarmed, or that a second finger joined does nothing until every finger lifts.
#[derive(Debug, Default)]
pub struct Gesture {
    finger: Option<Finger>,
    /// Fingers down now.
    down: Vec<u64>,
    /// The gesture under way is not this one's.
    ignored: bool,
}

impl Gesture {
    /// Feed one frame's events. `on_photo` says whether a finger that comes down at a point lands on the photo;
    /// `armed` whether the gesture may act now.
    pub fn feed(&mut self, events: &[egui::Event], on_photo: impl Fn(Pos2) -> bool, armed: bool) -> Vec<Drag> {
        let mut out = Vec::new();
        if !armed {
            self.stop(&mut out);
        }
        for e in events {
            let egui::Event::Touch { id, phase, pos, .. } = e else { continue };
            let id = id.0;
            match phase {
                egui::TouchPhase::Start => {
                    if self.down.is_empty() {
                        self.ignored = !armed || !pos.x.is_finite() || !pos.y.is_finite() || !on_photo(*pos);
                        self.finger = (!self.ignored).then_some(Finger { id, start: *pos, origin: *pos, axis: Axis::Undecided, rows: 0 });
                    } else {
                        // a second finger: a pinch or a two-finger tap, never this gesture
                        self.stop(&mut out);
                    }
                    if !self.down.contains(&id) {
                        self.down.push(id);
                    }
                }
                egui::TouchPhase::Move => {
                    if self.ignored || !pos.x.is_finite() || !pos.y.is_finite() {
                        continue;
                    }
                    let Some(f) = self.finger.as_mut().filter(|f| f.id == id) else { continue };
                    match f.axis {
                        Axis::Undecided => {
                            let d = *pos - f.start;
                            if d.length() >= LOCK {
                                f.origin = *pos;
                                if d.y.abs() > d.x.abs() {
                                    f.axis = Axis::Vertical;
                                    out.push(Drag::Choose { rows: 0, first: true });
                                } else {
                                    f.axis = Axis::Horizontal;
                                    out.push(Drag::Adjust { dx: 0.0, first: true });
                                }
                            }
                        }
                        Axis::Vertical => {
                            let rows = ((pos.y - f.origin.y) / ROW).trunc();
                            let rows = if rows.is_finite() { rows.clamp(-1000.0, 1000.0) as i32 } else { 0 };
                            if rows != f.rows {
                                f.rows = rows;
                                out.push(Drag::Choose { rows, first: false });
                            }
                        }
                        Axis::Horizontal => out.push(Drag::Adjust { dx: pos.x - f.origin.x, first: false }),
                    }
                }
                egui::TouchPhase::End | egui::TouchPhase::Cancel => {
                    self.down.retain(|d| *d != id);
                    if self.finger.is_some_and(|f| f.id == id) {
                        if let Some(Finger { axis: Axis::Horizontal, origin, .. }) = self.finger
                            && !self.ignored
                            && *phase == egui::TouchPhase::End
                            && pos.x.is_finite()
                        {
                            out.push(Drag::Adjust { dx: pos.x - origin.x, first: false });
                        }
                        self.stop(&mut out);
                    }
                    if self.down.is_empty() {
                        self.ignored = false;
                        self.finger = None;
                    }
                }
            }
        }
        out
    }

    /// The gesture under way stops: a horizontal drag ends where it is.
    fn stop(&mut self, out: &mut Vec<Drag>) {
        if let Some(f) = self.finger.take()
            && f.axis == Axis::Horizontal
            && !self.ignored
        {
            out.push(Drag::End);
        }
        if !self.down.is_empty() {
            self.ignored = true;
        }
    }

    /// A finger's drag has locked to an axis and is under way.
    pub fn locked(&self) -> bool {
        !self.ignored && self.finger.is_some_and(|f| f.axis != Axis::Undecided)
    }
}

/// The app's side: the sliders the open panel offered, the drag under way, the readout.
#[derive(Debug, Default)]
pub struct PhotoDrag {
    pub(crate) gesture: Gesture,
    /// The pass the panel last offered its sliders in, and those sliders (control id, value shown), in order.
    offered_pass: u64,
    offered: Vec<(String, f64)>,
    /// The sliders as they were when the gesture's vertical drag locked, and the index chosen then.
    choose_base: Option<usize>,
    /// The horizontal drag under way: its control and the track position (0..1) it started from.
    adjusting: Option<(String, f64)>,
    /// Until when (app time) the readout stays after the finger lifted.
    hud_until: f64,
    /// The pass the loupe last fed the gesture in: a pass without the loupe ends what was under way ([`end_frame`]).
    loupe_pass: u64,
    /// Whether the gesture was armed in that pass (a one-finger drag on the photo was this gesture, not a swipe).
    pub(crate) armed: bool,
}

impl PhotoDrag {
    /// The sliders the open panel offered last (control ids, in the panel's order).
    #[cfg(test)]
    pub(crate) fn offered_ids(&self) -> Vec<&str> {
        self.offered.iter().map(|(c, _)| c.as_str()).collect()
    }

    /// The sliders the open panel offered in this pass or the one before (a panel drawn after the photo offers
    /// them a pass later); none once the panel closed.
    fn sliders(&self, pass: u64) -> &[(String, f64)] {
        if pass.saturating_sub(self.offered_pass) <= 1 { &self.offered } else { &[] }
    }
}

/// An open panel's enabled `develop` slider offers itself (with the value it shows) to the photo's drags; called
/// each pass by [`super::edit::control`].
pub(crate) fn offer(app: &mut LightcraftApp, ctx: &egui::Context, id: &str, value: f64) {
    let pass = ctx.cumulative_pass_nr();
    let d = &mut app.photo_drag;
    if d.offered_pass != pass {
        d.offered_pass = pass;
        d.offered.clear();
    }
    if controls::find(id).is_some() && value.is_finite() && !d.offered.iter().any(|(c, _)| c == id) {
        d.offered.push((id.to_string(), value));
    }
}

/// The control chosen now: the last one chosen while the panel still offers it, else the panel's first.
fn chosen(app: &LightcraftApp, pass: u64) -> Option<usize> {
    let sliders = app.photo_drag.sliders(pass);
    let want = app.ui.photo_drag_control.as_deref();
    want.and_then(|w| sliders.iter().position(|(c, _)| c == w)).or((!sliders.is_empty()).then_some(0))
}

/// The photo's slider gestures, called by `cull_touch::loupe` each frame with the frame's `events`. `act` is false
/// while a tool owns the photo. Returns whether the gesture is armed: a one-finger drag on the photo is this
/// gesture then, not a swipe.
pub(crate) fn loupe(
    app: &mut LightcraftApp,
    ctx: &egui::Context,
    events: &[egui::Event],
    on_photo: impl Fn(Pos2) -> bool,
    canvas: Rect,
    act: bool,
    zoomed: bool,
) -> bool {
    let pass = ctx.cumulative_pass_nr();
    let armed = app.ui.photo_drag && act && !zoomed && app.session.active().is_some() && !app.photo_drag.sliders(pass).is_empty();
    app.photo_drag.loupe_pass = pass;
    app.photo_drag.armed = armed;
    let drags = app.photo_drag.gesture.feed(events, on_photo, armed);
    let now = ctx.input(|i| i.time);
    for drag in drags {
        match drag {
            Drag::Choose { rows, first } => {
                if first {
                    app.photo_drag.choose_base = chosen(app, pass);
                }
                let sliders = app.photo_drag.sliders(pass);
                let Some(base) = app.photo_drag.choose_base else { continue };
                let last = sliders.len().saturating_sub(1);
                let at = i64::try_from(base).unwrap_or(0).saturating_add(i64::from(rows)).clamp(0, i64::try_from(last).unwrap_or(0));
                if let Some((c, _)) = usize::try_from(at).ok().and_then(|i| sliders.get(i)) {
                    app.ui.photo_drag_control = Some(c.clone());
                }
            }
            Drag::Adjust { dx, first } => adjust(app, pass, dx, first, canvas),
            Drag::End => end_adjusting(app),
        }
        app.photo_drag.hud_until = now + HUD_SECS;
    }
    if app.photo_drag.gesture.locked() || now < app.photo_drag.hud_until {
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(HUD_SECS / 3.0));
    }
    armed
}

/// The horizontal drag under way ends where it is: the slider's interaction closes (one undo step).
fn end_adjusting(app: &mut LightcraftApp) {
    if let Some((c, _)) = app.photo_drag.adjusting.take()
        && let Some(spec) = controls::find(&c)
    {
        super::edit::apply_slider_out(app, spec, SliderOut { drag_stopped: true, ..Default::default() }, |_, _| Ok(json!(null)));
    }
}

/// Called at the end of every frame: when the loupe wasn't drawn in this pass (a view change by key or Back
/// mid-drag, a Detail view that returned before its gestures), a drag under way ends where it is, so
/// `develop.beginInteraction` doesn't stay open, and the recogniser forgets its fingers, so the next finger down
/// is a first finger again.
pub(crate) fn end_frame(app: &mut LightcraftApp, ctx: &egui::Context) {
    if app.photo_drag.loupe_pass == ctx.cumulative_pass_nr() {
        return;
    }
    app.photo_drag.armed = false;
    end_adjusting(app);
    app.photo_drag.choose_base = None;
    app.photo_drag.gesture = Gesture::default();
}

/// A horizontal drag `dx` points from where it locked: the chosen slider's value, moved as dragging its thumb
/// [`SPAN`] of the track per width of the photo's area would, on the control's steps.
fn adjust(app: &mut LightcraftApp, pass: u64, dx: f32, first: bool, canvas: Rect) {
    if first {
        let Some((c, v)) = chosen(app, pass).and_then(|i| app.photo_drag.sliders(pass).get(i)).cloned() else { return };
        let Some(spec) = controls::find(&c) else { return };
        app.ui.photo_drag_control = Some(c.clone());
        app.photo_drag.adjusting = Some((c, spec.position(v)));
        super::edit::apply_slider_out(app, spec, SliderOut { drag_started: true, ..Default::default() }, |_, _| Ok(json!(null)));
        return;
    }
    let Some((c, from)) = app.photo_drag.adjusting.clone() else { return };
    let Some(spec) = controls::find(&c) else { return };
    let Some(id) = app.session.active() else { return };
    let current = app.session.develop_of(id).and_then(|d| controls::get(&d, &c)).unwrap_or(spec.default);
    let pos = (from + f64::from(dx / canvas.width().max(1.0)) * SPAN).clamp(0.0, 1.0);
    let step = spec.step.max(1e-9);
    let nv = ((spec.value_at(pos) / step).round() * step).clamp(spec.min, spec.max);
    if !nv.is_finite() || (nv - current).abs() <= 1e-12 {
        return;
    }
    let control = c.clone();
    super::edit::apply_slider_out(app, spec, SliderOut { value: Some(nv), ..Default::default() }, move |app, v| {
        app.run("develop.set", json!({"control": control, "value": v}))
    });
}

/// The readout over the photo while a drag is under way and briefly after: the chosen slider's name and value,
/// with its neighbours above and below while choosing.
pub(crate) fn hud(app: &LightcraftApp, ui: &egui::Ui, canvas: Rect) {
    let now = ui.input(|i| i.time);
    let d = &app.photo_drag;
    if !d.gesture.locked() && now >= d.hud_until {
        return;
    }
    let pass = ui.ctx().cumulative_pass_nr();
    let sliders = d.sliders(pass);
    let Some(i) = chosen(app, pass) else { return };
    let line = |i: usize| -> Option<String> {
        let (c, v) = sliders.get(i)?;
        let spec = controls::find(c)?;
        Some(format!("{}  {}", crate::i18n::tr(spec.label), shown_value(spec, *v)))
    };
    let Some(main) = line(i) else { return };
    let choosing = d.gesture.finger.is_some_and(|f| f.axis == Axis::Vertical);
    let t = Tokens::get(ui.ctx());
    let p = ui.painter_at(canvas);
    let g = p.layout_no_wrap(main, t.semibold(18.0), Color32::WHITE);
    let (above, below) = if choosing { (i.checked_sub(1).and_then(line), line(i.saturating_add(1))) } else { (None, None) };
    let small = |s: Option<String>| s.map(|s| p.layout_no_wrap(s, t.font(13.0), Color32::from_white_alpha(150)));
    let (above, below) = (small(above), small(below));
    let gap = 6.0;
    let w = [Some(&g), above.as_ref(), below.as_ref()].into_iter().flatten().map(|g| g.size().x).fold(0.0_f32, f32::max);
    let h = g.size().y + [above.as_ref(), below.as_ref()].into_iter().flatten().map(|g| g.size().y + gap).sum::<f32>();
    let top = canvas.top() + 24.0;
    let r = Rect::from_min_size(pos2(canvas.center().x - w / 2.0 - 18.0, top), vec2(w + 36.0, h + 20.0));
    p.rect_filled(r, 12.0, Color32::from_black_alpha(170));
    let mut y = r.top() + 10.0;
    for (galley, color) in [(above, Color32::from_white_alpha(150)), (Some(g), Color32::WHITE), (below, Color32::from_white_alpha(150))] {
        let Some(galley) = galley else { continue };
        let size: Vec2 = galley.size();
        p.galley(pos2(r.center().x - size.x / 2.0, y), galley, color);
        y += size.y + gap;
    }
    register(ui.ctx(), "loupe:photoDragHud", r);
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::TouchPhase::{Cancel, End, Move, Start};

    fn ev(id: u64, phase: egui::TouchPhase, x: f32, y: f32) -> egui::Event {
        egui::Event::Touch { device_id: egui::TouchDeviceId(0), id: egui::TouchId(id), phase, pos: pos2(x, y), force: None }
    }

    fn feed(g: &mut Gesture, frames: &[Vec<egui::Event>]) -> Vec<Drag> {
        frames.iter().flat_map(|e| g.feed(e, |_| true, true)).collect()
    }

    #[test]
    fn a_vertical_drag_chooses_by_rows_and_a_horizontal_one_adjusts_then_ends() {
        let mut g = Gesture::default();
        let got = feed(
            &mut g,
            &[vec![ev(1, Start, 100.0, 100.0)], vec![ev(1, Move, 100.0, 120.0)], vec![ev(1, Move, 100.0, 200.0)], vec![ev(1, End, 100.0, 200.0)]],
        );
        assert_eq!(got, vec![Drag::Choose { rows: 0, first: true }, Drag::Choose { rows: 2, first: false }]);
        let got = feed(
            &mut g,
            &[vec![ev(1, Start, 100.0, 100.0)], vec![ev(1, Move, 80.0, 100.0)], vec![ev(1, Move, 40.0, 104.0)], vec![ev(1, End, 30.0, 104.0)]],
        );
        assert_eq!(
            got,
            vec![
                Drag::Adjust { dx: 0.0, first: true },
                Drag::Adjust { dx: -40.0, first: false },
                Drag::Adjust { dx: -50.0, first: false },
                Drag::End
            ]
        );
        // a tap does nothing
        assert!(feed(&mut g, &[vec![ev(1, Start, 100.0, 100.0)], vec![ev(1, End, 104.0, 100.0)]]).is_empty());
    }

    #[test]
    fn a_second_finger_a_cancel_or_disarming_ends_the_drag_and_off_photo_starts_nothing() {
        let mut g = Gesture::default();
        let got = feed(
            &mut g,
            &[
                vec![ev(1, Start, 100.0, 100.0)],
                vec![ev(1, Move, 130.0, 100.0)],
                vec![ev(2, Start, 300.0, 100.0)],
                vec![ev(1, Move, 200.0, 100.0)],
                vec![ev(1, End, 200.0, 100.0), ev(2, End, 300.0, 100.0)],
            ],
        );
        assert_eq!(got, vec![Drag::Adjust { dx: 0.0, first: true }, Drag::End], "a pinch stops it");
        let got = feed(&mut g, &[vec![ev(1, Start, 100.0, 100.0)], vec![ev(1, Move, 130.0, 100.0)], vec![ev(1, Cancel, 200.0, 100.0)]]);
        assert_eq!(got, vec![Drag::Adjust { dx: 0.0, first: true }, Drag::End], "a cancel keeps what was done");
        let mut out = g.feed(&[ev(1, Start, 100.0, 100.0)], |_| true, true);
        out.extend(g.feed(&[ev(1, Move, 130.0, 100.0)], |_| true, true));
        out.extend(g.feed(&[ev(1, Move, 160.0, 100.0)], |_| true, false));
        out.extend(g.feed(&[ev(1, End, 160.0, 100.0)], |_| true, true));
        assert_eq!(out, vec![Drag::Adjust { dx: 0.0, first: true }, Drag::End], "a tool taking the photo ends it");
        let off: Vec<Drag> = [vec![ev(1, Start, 100.0, 100.0)], vec![ev(1, Move, 200.0, 100.0)], vec![ev(1, End, 200.0, 100.0)]]
            .iter()
            .flat_map(|e| g.feed(e, |_| false, true))
            .collect();
        assert!(off.is_empty(), "{off:?}");
        let hostile = feed(&mut g, &[vec![ev(1, Start, f32::NAN, 100.0)], vec![ev(1, Move, f32::INFINITY, 100.0)], vec![ev(1, End, 0.0, 0.0)]]);
        assert!(hostile.is_empty(), "{hostile:?}");
        assert!(feed(&mut g, &[vec![ev(7, End, 0.0, 0.0)], vec![ev(7, Move, 0.0, 0.0)]]).is_empty());
    }
}
