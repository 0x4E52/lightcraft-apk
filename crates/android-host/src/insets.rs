//! Safe-area and keyboard insets for egui (APK-31).
//!
//! With targetSdk 36 the window is edge-to-edge: it reaches under the status bar, the display
//! cutout, the gesture bar and the soft keyboard. The Java side (`lightcraft-android-sys`'s
//! `insets`) reports those in pixels over a channel; [`Tracker`] asks for them when something
//! that changes them happened (the window resized or rescaled, focus came back) and, while a text
//! field has focus, every [`TEXT_REFRESH`] (the keyboard opens and closes without any window
//! event), polls the answer with `try_recv` (never a blocking `recv`) and keeps the last one.
//! [`insets_px_to_points`] turns it into the `RawInput::safe_area_insets` egui subtracts from the
//! window to get `content_rect()`. Everything here is pure and tested on every target; the
//! request itself is passed in.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

use lightcraft_android_sys::insets::{Answer, Insets};

/// How often the insets are asked again while a text field has focus (or just lost it).
pub const TEXT_REFRESH: Duration = Duration::from_millis(250);
/// How long after the last focused frame the refreshing goes on (the keyboard's closing
/// animation).
pub const SETTLE: Duration = Duration::from_millis(1000);
/// How soon an answer in flight is looked at again.
const POLL: Duration = Duration::from_millis(50);

/// egui's safe-area insets for `px` at `pixels_per_point` (egui points, the zoom included). A
/// scale that is zero, negative or not finite gives no insets, never NaN.
pub fn insets_px_to_points(px: Insets, pixels_per_point: f32) -> egui::epaint::MarginF32 {
    if !pixels_per_point.is_finite() || pixels_per_point <= 0.0 {
        return egui::epaint::MarginF32::ZERO;
    }
    let points = |v: i32| v.max(0) as f32 / pixels_per_point;
    egui::epaint::MarginF32 { left: points(px.left), right: points(px.right), top: points(px.top), bottom: points(px.bottom) }
}

/// Set by the lifecycle wrapper when the window resized or rescaled or focus came back; the
/// tracker asks again on its next tick.
pub type Dirty = Rc<Cell<bool>>;

/// Asks for the insets: posts the request and returns the channel the answer arrives on.
pub type Request = Box<dyn Fn() -> Receiver<Answer>>;

/// The cached insets and when to refresh them.
pub struct Tracker {
    request: Request,
    in_flight: Option<Receiver<Answer>>,
    cached: Insets,
    dirty: Dirty,
    last_request: Option<Instant>,
    /// Refreshing every [`TEXT_REFRESH`] goes on until here.
    refresh_until: Option<Instant>,
}

impl Tracker {
    /// A tracker that asks at once (the first tick), then as [`Tracker::tick`] describes.
    pub fn new(request: Request, dirty: Dirty) -> Tracker {
        dirty.set(true);
        Tracker { request, in_flight: None, cached: Insets::default(), dirty, last_request: None, refresh_until: None }
    }

    /// The last insets Java reported, in pixels.
    pub fn cached(&self) -> Insets {
        self.cached
    }

    /// Once per frame: takes in an answer that arrived, posts a new request when one is due, and
    /// says when to look again (`None`: nothing to wait for). `text_focus`: a text field has the
    /// focus, so the keyboard may be moving.
    pub fn tick(&mut self, now: Instant, text_focus: bool) -> Option<Duration> {
        if let Some(rx) = &self.in_flight {
            match rx.try_recv() {
                Ok(Ok(insets)) => {
                    self.cached = insets;
                    self.in_flight = None;
                }
                Ok(Err(e)) => {
                    log::warn!("window insets: {e}");
                    self.in_flight = None;
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => self.in_flight = None,
            }
        }
        if text_focus {
            self.refresh_until = Some(now + SETTLE);
        }
        let refreshing = self.refresh_until.is_some_and(|until| now < until);
        if !refreshing {
            self.refresh_until = None;
        }
        let due = self.dirty.get() || (refreshing && self.last_request.is_none_or(|t| now.saturating_duration_since(t) >= TEXT_REFRESH));
        if self.in_flight.is_none() && due {
            self.dirty.set(false);
            self.last_request = Some(now);
            self.in_flight = Some((self.request)());
        }
        match (self.in_flight.is_some(), refreshing) {
            (true, _) => Some(POLL),
            (false, true) => Some(TEXT_REFRESH),
            (false, false) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::sync::mpsc::Sender;

    #[test]
    fn pixels_become_points_at_the_scale() {
        let m = insets_px_to_points(Insets::new(0, 105, 21, 210), 2.625);
        assert_eq!((m.left, m.top, m.right, m.bottom), (0.0, 40.0, 8.0, 80.0));
        for bad in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            assert_eq!(insets_px_to_points(Insets::new(1, 2, 3, 4), bad), egui::epaint::MarginF32::ZERO, "scale {bad}");
        }
    }

    /// A fake Java side: every request is kept so the test answers it when it likes.
    fn fake() -> (Tracker, Rc<RefCell<Vec<Sender<Answer>>>>, Dirty) {
        let sent = Rc::new(RefCell::new(Vec::new()));
        let log = sent.clone();
        let request: Request = Box::new(move || {
            let (tx, rx) = std::sync::mpsc::channel();
            log.borrow_mut().push(tx);
            rx
        });
        let dirty = Dirty::default();
        (Tracker::new(request, dirty.clone()), sent, dirty)
    }

    #[test]
    fn asks_at_once_and_never_blocks_on_the_answer() {
        let (mut t, sent, _) = fake();
        let t0 = Instant::now();
        assert_eq!(t.tick(t0, false), Some(POLL), "an answer in flight is polled soon");
        assert_eq!(sent.borrow().len(), 1);
        assert!(t.cached().is_zero(), "nothing arrived yet");
        // still in flight: no second request however often it ticks
        t.tick(t0 + Duration::from_secs(1), false);
        assert_eq!(sent.borrow().len(), 1);
        sent.borrow()[0].send(Ok(Insets::new(0, 100, 0, 63))).unwrap();
        assert_eq!(t.tick(t0 + Duration::from_secs(2), false), None);
        assert_eq!(t.cached(), Insets::new(0, 100, 0, 63));
    }

    #[test]
    fn a_window_event_asks_again_and_a_failed_answer_keeps_the_old_insets() {
        let (mut t, sent, dirty) = fake();
        let t0 = Instant::now();
        t.tick(t0, false);
        sent.borrow()[0].send(Ok(Insets::new(0, 100, 0, 63))).unwrap();
        t.tick(t0, false);
        dirty.set(true);
        t.tick(t0, false);
        assert_eq!(sent.borrow().len(), 2);
        sent.borrow()[1].send(Err(lightcraft_android_sys::Error::Reentrant)).unwrap();
        t.tick(t0, false);
        assert_eq!(t.cached(), Insets::new(0, 100, 0, 63));
        // nothing is asked without a reason
        t.tick(t0 + Duration::from_secs(60), false);
        assert_eq!(sent.borrow().len(), 2);
    }

    #[test]
    fn a_focused_field_refreshes_every_250_ms_and_a_second_after() {
        let (mut t, sent, _) = fake();
        let t0 = Instant::now();
        let answer = |i: usize, t: &mut Tracker, at: Instant, focus: bool| {
            sent.borrow()[i].send(Ok(Insets::default())).unwrap();
            t.tick(at, focus)
        };
        t.tick(t0, true);
        answer(0, &mut t, t0, true);
        let ms = |n| t0 + Duration::from_millis(n);
        assert_eq!(t.tick(ms(100), true), Some(TEXT_REFRESH));
        assert_eq!(sent.borrow().len(), 1, "too early");
        t.tick(ms(260), true);
        assert_eq!(sent.borrow().len(), 2);
        answer(1, &mut t, ms(260), true);
        // focus lost: the keyboard is still closing, so refreshing goes on for SETTLE
        t.tick(ms(600), false);
        assert_eq!(sent.borrow().len(), 3);
        answer(2, &mut t, ms(600), false);
        assert_eq!(t.tick(ms(1300), false), None, "settled");
        t.tick(ms(5000), false);
        assert_eq!(sent.borrow().len(), 3, "no more requests");
    }
}
