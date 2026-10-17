//! The event handler `run` gives winit: eframe's own (`EframeWinitApplication`), wrapped.
//!
//! Every callback goes to eframe first, unchanged; then the ones the host cares about become a
//! [`HostEvent`] for [`on_event`] (`lifecycle_core.rs`, pure and tested): focus lost and gained,
//! resumed, suspended, the memory warning eframe's own wrapper ignores, and `about_to_wait`, where
//! a destroyed Activity is noticed. On [`Action::Exit`] (everything saved by then) the loop is
//! asked to end; winit then calls `exiting`, which eframe turns into the app's `on_exit` (the
//! library closed), and `run` returns [`crate::Exit::Destroyed`].

use eframe::UserEvent;
use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, DeviceId, StartCause, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::window::WindowId;

use crate::app::Slot;
use crate::lifecycle_core::{Action, CoreState, HostEvent, on_event};

/// eframe's handler and the host's lifecycle.
pub struct Lifecycle<'a> {
    eframe: eframe::EframeWinitApplication<'a>,
    slot: Slot,
    core: CoreState,
}

impl<'a> Lifecycle<'a> {
    pub fn new(eframe: eframe::EframeWinitApplication<'a>, slot: Slot, core: CoreState) -> Lifecycle<'a> {
        Lifecycle { eframe, slot, core }
    }

    /// Whether the loop ended because the Activity went away.
    pub fn left(&self) -> bool {
        self.core.leaving()
    }

    fn host(&mut self, event_loop: &ActiveEventLoop, ev: HostEvent) {
        if on_event(ev, &*self.slot, &mut self.core) == Action::Exit {
            event_loop.exit();
        }
    }
}

impl ApplicationHandler<UserEvent> for Lifecycle<'_> {
    fn new_events(&mut self, event_loop: &ActiveEventLoop, cause: StartCause) {
        self.eframe.new_events(event_loop, cause);
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.eframe.resumed(event_loop);
        self.host(event_loop, HostEvent::Resumed);
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        self.eframe.user_event(event_loop, event);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, window_id: WindowId, event: WindowEvent) {
        let focus = match event {
            WindowEvent::Focused(true) => Some(HostEvent::FocusGained),
            WindowEvent::Focused(false) => Some(HostEvent::FocusLost),
            _ => None,
        };
        self.eframe.window_event(event_loop, window_id, event);
        if let Some(ev) = focus {
            self.host(event_loop, ev);
        }
    }

    fn device_event(&mut self, event_loop: &ActiveEventLoop, device_id: DeviceId, event: DeviceEvent) {
        self.eframe.device_event(event_loop, device_id, event);
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.eframe.about_to_wait(event_loop);
        self.host(event_loop, HostEvent::AboutToWait);
    }

    fn suspended(&mut self, event_loop: &ActiveEventLoop) {
        self.eframe.suspended(event_loop);
        self.host(event_loop, HostEvent::Suspended);
    }

    fn exiting(&mut self, event_loop: &ActiveEventLoop) {
        self.eframe.exiting(event_loop);
    }

    fn memory_warning(&mut self, event_loop: &ActiveEventLoop) {
        self.eframe.memory_warning(event_loop);
        self.host(event_loop, HostEvent::MemoryWarning);
    }
}
