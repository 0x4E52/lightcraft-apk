//! The eframe app on Android: a loading frame until the library is open, then LightCraft's UI.
//!
//! The [`HostState`] lives in a shared slot (`Rc<RefCell<Option<HostState>>>`), created by `run`
//! before eframe starts, so the lifecycle wrapper (`lifecycle.rs`) can flush and trim it while no
//! frame runs. Until
//! the [`Loader`] answers, `ui` draws one centred "Opening library…" label and `logic` asks the
//! loader again every 100 ms; once it answers, the app is built on this (the UI) thread and fills
//! the slot, and the label stays until the app's fonts and theme are installed. While the app
//! runs, `logic` saves what changed every 30 s (as the web host's timer; never `flush_leaving`,
//! which would cut an open slider drag into one undo step per frame), and `on_exit` saves
//! everything, closes the library and waits for renders in flight. A slot that is already
//! borrowed (a re-entrant callback) is logged and the call skipped, never a panic.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use eframe::egui;

use lightcraft_ui_egui::ControlRequest;

use crate::insets::{Tracker, insets_px_to_points};
use crate::lifecycle_core::Host;
use crate::loader::Loader;
use crate::state::{HostState, Reopen, Start};

/// The slot the eframe app and the lifecycle wrapper share.
pub type Slot = Rc<RefCell<Option<HostState>>>;

/// How often a running app writes what changed.
pub const FLUSH_EVERY: Duration = Duration::from_secs(30);
/// How long quitting waits for renders in flight (the desktop's `EXIT_WAIT`).
pub const EXIT_WAIT: Duration = Duration::from_secs(2);
/// How often the loading frame asks the loader.
const LOADING_POLL: Duration = Duration::from_millis(100);

/// eframe's app: the loading frame, then [`HostState`]'s app.
pub struct HostApp {
    slot: Slot,
    loader: Loader,
    /// What the next state is built from; refreshed when the library is opened again.
    start: Start,
    /// Try Again in the library problem window asks here (off android_main: the library opens on
    /// the loader's worker behind the loading frame, as at the start).
    reopen: Reopen,
    /// The debug control server's requests, until a state takes them (and again after Try Again
    /// replaced that state).
    control: Option<Receiver<ControlRequest>>,
    last_flush: Instant,
    /// The window's safe-area and keyboard insets (APK-31).
    insets: Tracker,
}

impl HostApp {
    pub fn new(slot: Slot, loader: Loader, start: Start, reopen: Reopen, control: Option<Receiver<ControlRequest>>, insets: Tracker) -> HostApp {
        HostApp { slot, loader, start, reopen, control, last_flush: Instant::now(), insets }
    }

    /// A reopen request: the state over the library problem ends (its session is empty and in
    /// memory) and the library opens on a worker again; the loading frame shows meanwhile.
    fn reopen_if_asked(&mut self, slot: &mut Option<HostState>, ctx: &egui::Context) -> bool {
        let Some(path) = self.reopen.take() else { return false };
        let library = std::path::PathBuf::from(path);
        if let Some(rx) = slot.as_mut().and_then(|state| state.app.take_control()) {
            self.control = Some(rx);
        }
        self.start.reopening(slot.take(), library.clone());
        self.loader = crate::loader::open_library(library);
        ctx.request_repaint();
        true
    }
}

impl eframe::App for HostApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // (polls the answer, never waits; asks again on a resize or while a field has focus)
        if let Some(after) = self.insets.tick(Instant::now(), ctx.egui_wants_keyboard_input()) {
            ctx.request_repaint_after(after);
        }
        // (the slot's own handle: the borrow must not hold `self`, which the reopen changes)
        let shared = self.slot.clone();
        let Ok(mut slot) = shared.try_borrow_mut() else {
            log::warn!("frame skipped: the app state is in use");
            return;
        };
        if self.reopen_if_asked(&mut slot, ctx) {
            return;
        }
        if slot.is_none() {
            let Some(opened) = self.loader.poll() else {
                ctx.request_repaint_after(LOADING_POLL);
                return;
            };
            // the app's first `logic` runs in this frame: its fonts and theme go in at once
            let start = self.start.clone();
            // (said once: not again when the library is opened again)
            self.start.prefs_warning = None;
            self.start.gpu_notice = None;
            self.start.renderer_notice = None;
            let mut state = HostState::new(opened, start);
            if let Some(rx) = self.control.take() {
                state.app.set_control(rx);
            }
            *slot = Some(state);
            self.last_flush = Instant::now();
        }
        let Some(state) = slot.as_mut() else { return };
        state.app.logic(ctx);
        state.prefs.tick(&mut state.app, ctx);
        if self.last_flush.elapsed() >= FLUSH_EVERY {
            self.last_flush = Instant::now();
            if let Err(e) = state.flush_dirty() {
                log::warn!("{e}");
            }
        }
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        // egui subtracts the insets from the window to get `content_rect()`: the points of the
        // scale this frame's window will have (native scale times UI zoom)
        let native = raw.viewports.get(&egui::ViewportId::ROOT).and_then(|v| v.native_pixels_per_point);
        let scale = native.map_or_else(|| ctx.pixels_per_point(), |n| n * ctx.zoom_factor());
        raw.safe_area_insets = Some(egui::SafeAreaInsets(insets_px_to_points(self.insets.cached(), scale)));
        // Back is Escape: popups and text fields take it first, then the Back cascade
        crate::input::browser_back_to_escape(&mut raw.events);
        match self.slot.try_borrow_mut() {
            Ok(mut slot) => {
                if let Some(state) = slot.as_mut() {
                    state.app.raw_input_hook(raw);
                }
            }
            Err(_) => log::warn!("input hook skipped: the app state is in use"),
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let Ok(mut slot) = self.slot.try_borrow_mut() else {
            log::warn!("frame skipped: the app state is in use");
            return;
        };
        match slot.as_mut() {
            Some(state) if state.app.fonts_ready() => {
                state.app.perf.host_frame_ms = frame.info().cpu_usage.map(|s| f64::from(s) * 1000.0);
                state.app.ui(ui);
                if self.reopen.is_pending() {
                    ui.ctx().request_repaint();
                }
            }
            // the loading label until the app's fonts and theme are in (a frame or two after the
            // session arrives), never a blank screen
            open => {
                if open.is_some() {
                    ui.ctx().request_repaint();
                }
                egui::CentralPanel::default().show(ui, |ui| {
                    ui.centered_and_justified(|ui| ui.label(lightcraft_ui_egui::i18n::tr("Opening library…")));
                });
            }
        }
    }

    fn on_exit(&mut self) {
        match self.slot.try_borrow_mut() {
            Ok(mut slot) => {
                if let Some(state) = slot.as_mut() {
                    state.close(EXIT_WAIT);
                }
            }
            Err(_) => log::error!("quit: the app state was in use, so it was not saved"),
        }
    }
}
