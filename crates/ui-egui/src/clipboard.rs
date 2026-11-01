//! The system clipboard where egui's host can't reach it (Android, APK-28).
//!
//! egui-winit keeps an in-process clipboard on Android, so without help a copy never leaves
//! LightCraft and a paste never sees what another app copied. A host with a system clipboard
//! injects [`ClipboardFns`] as `Services::clipboard`:
//!
//! - **Copy.** At the end of every frame each `OutputCommand::CopyText` (a [`crate::text_field`]
//!   Copy or Cut, `ctx.copy_text` in a dialog, egui's own Ctrl+C on a hardware keyboard) goes to
//!   [`ClipboardFns::set`]. The commands stay in the output, so egui-winit's in-process clipboard
//!   keeps a copy too: a hardware Ctrl+V only reaches the app while that one holds some text.
//! - **Paste.** A widget asks with [`request_paste`] instead of `ViewportCommand::RequestPaste`;
//!   at the end of the frame the app calls [`ClipboardFns::request_paste`], which returns at once,
//!   and from then on `LightcraftApp::raw_input_hook` asks [`ClipboardFns::take_paste`] every
//!   frame and turns the answer into `Event::Paste`. The UI thread never waits for the clipboard:
//!   an answer may come on the next frame or later (the app asks for frames while one is due, up
//!   to [`PASTE_WAIT_MS`]).
//!
//! Without the service everything is as before: egui's host answers `RequestPaste`.

/// How long the app keeps asking for frames for a paste that has not been answered (an answer
/// that comes later is still pasted, on the frame after it arrives).
pub const PASTE_WAIT_MS: f64 = 5000.0;

/// Puts text on the system clipboard (`Err` says why it could not).
pub type SetClip = Box<dyn FnMut(&str) -> Result<(), String>>;
/// Starts reading the system clipboard for a paste; returns at once.
pub type RequestPaste = Box<dyn FnMut()>;
/// The answer to the last [`RequestPaste`], once it is there: `Some("")` when the clipboard holds
/// no text (nothing is pasted), `None` while there is no answer.
pub type TakePaste = Box<dyn FnMut() -> Option<String>>;

/// The host's system clipboard (`Services::clipboard`).
pub struct ClipboardFns {
    pub set: SetClip,
    pub request_paste: RequestPaste,
    pub take_paste: TakePaste,
}

/// The app has a clipboard service (set by the app each frame).
fn service_id() -> egui::Id {
    egui::Id::new("lightcraft-clipboard-service")
}

/// A widget asked for a paste this frame.
fn request_id() -> egui::Id {
    egui::Id::new("lightcraft-clipboard-paste-request")
}

/// Says whether this context's app has a clipboard service (the app, at the start of each frame).
pub(crate) fn mark_service(ctx: &egui::Context, on: bool) {
    ctx.data_mut(|d| {
        if on {
            d.insert_temp(service_id(), true);
        } else {
            d.remove::<bool>(service_id());
        }
    });
}

/// Paste into the focused field: with a clipboard service the app asks it at the end of the
/// frame and the text comes as `Event::Paste` in a later frame; without one, egui's host answers
/// `ViewportCommand::RequestPaste` the same way.
pub fn request_paste(ctx: &egui::Context) {
    if ctx.data(|d| d.get_temp::<bool>(service_id())).unwrap_or(false) {
        ctx.data_mut(|d| d.insert_temp(request_id(), true));
    } else {
        ctx.send_viewport_cmd(egui::ViewportCommand::RequestPaste);
    }
}

/// Whether a widget asked for a paste since the last call (the app, at the end of a frame).
pub(crate) fn take_request(ctx: &egui::Context) -> bool {
    ctx.data_mut(|d| d.remove_temp::<bool>(request_id())).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::headless::HeadlessView;

    /// Without the service a paste is egui's host's request; with it, a request the app takes.
    #[test]
    fn request_goes_to_the_service_only_when_there_is_one() {
        let mut view = HeadlessView::new();
        let raw = || HeadlessView::raw_input(egui::vec2(200.0, 100.0), 1.0, 0.0, vec![]);
        let commands = view.run(raw(), |ui| request_paste(ui.ctx()));
        assert!(commands.contains(&egui::ViewportCommand::RequestPaste), "{commands:?}");
        assert!(!take_request(&view.ctx));
        mark_service(&view.ctx, true);
        let commands = view.run(raw(), |ui| request_paste(ui.ctx()));
        assert!(!commands.contains(&egui::ViewportCommand::RequestPaste), "{commands:?}");
        assert!(take_request(&view.ctx));
        assert!(!take_request(&view.ctx), "taken once");
        mark_service(&view.ctx, false);
        let commands = view.run(raw(), |ui| request_paste(ui.ctx()));
        assert!(commands.contains(&egui::ViewportCommand::RequestPaste), "{commands:?}");
    }
}
