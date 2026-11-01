//! Input the host rewrites before the UI sees it.
//!
//! Android's Back key (and the back gesture, with the manifest's `enableOnBackInvokedCallback`
//! opt-out) reaches egui as `Key::BrowserBack`, which nothing in the UI maps. It becomes Escape,
//! so every Escape path applies first: an open menu, combo box, context menu or date picker
//! closes, a text field gives its text back, and only then does `view.back` (bound to Escape) run
//! its cascade, whose last step moves the app to the background (`Services::move_to_back`).
//!
//! A hardware keyboard's Ctrl+V (APK-28) reaches egui as `Event::Paste` with the text of
//! egui-winit's in-process clipboard, and only while that one holds some text. [`system_pastes`]
//! puts the system clipboard's text in its place; the host seeds the in-process clipboard with
//! [`PASTE_KEY_SEED`] once, so Ctrl+V works before anything was copied in LightCraft.

use egui::{Event, Key};

/// What the host puts in egui-winit's in-process clipboard at the start (a word joiner, which no
/// one copies), so Ctrl+V gives a paste event to replace; never pasted itself.
pub const PASTE_KEY_SEED: &str = "\u{2060}";

/// The text of every `Event::Paste` in `events` becomes the system clipboard's, read once by
/// `read` and only when there is such an event: an empty clipboard drops the events (nothing is
/// pasted); a clipboard that can't be read keeps a paste of LightCraft's own last copy and drops
/// the seed.
pub fn system_pastes(events: &mut Vec<Event>, read: impl FnOnce() -> Result<Option<String>, String>) {
    if !events.iter().any(|e| matches!(e, Event::Paste(_))) {
        return;
    }
    match read() {
        Ok(Some(text)) => {
            for event in events.iter_mut() {
                if let Event::Paste(old) = event {
                    old.clone_from(&text);
                }
            }
        }
        Ok(None) => events.retain(|e| !matches!(e, Event::Paste(_))),
        Err(e) => {
            log::warn!("paste: can't read the system clipboard: {e}");
            events.retain(|e| !matches!(e, Event::Paste(text) if text == PASTE_KEY_SEED));
        }
    }
}

/// Every Back key event in `events` becomes the same event for Escape (pressed, released and
/// repeats alike, modifiers kept); everything else is left as it is.
pub fn browser_back_to_escape(events: &mut [Event]) {
    for event in events {
        if let Event::Key { key, physical_key, .. } = event
            && *key == Key::BrowserBack
        {
            *key = Key::Escape;
            if *physical_key == Some(Key::BrowserBack) {
                *physical_key = Some(Key::Escape);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::Modifiers;

    fn key(key: Key, physical_key: Option<Key>, pressed: bool) -> Event {
        Event::Key { key, physical_key, pressed, repeat: false, modifiers: Modifiers::NONE }
    }

    #[test]
    fn browser_back_becomes_escape() {
        let mut events = vec![
            key(Key::BrowserBack, Some(Key::BrowserBack), true),
            Event::Text("a".into()),
            key(Key::BrowserBack, None, false),
            key(Key::A, Some(Key::A), true),
            Event::Key { key: Key::BrowserBack, physical_key: None, pressed: true, repeat: true, modifiers: Modifiers::SHIFT },
        ];
        browser_back_to_escape(&mut events);
        assert_eq!(
            events,
            vec![
                key(Key::Escape, Some(Key::Escape), true),
                Event::Text("a".into()),
                key(Key::Escape, None, false),
                key(Key::A, Some(Key::A), true),
                Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: true, modifiers: Modifiers::SHIFT },
            ]
        );
        // nothing to do without a Back key
        let mut plain = vec![key(Key::Escape, Some(Key::Escape), true)];
        browser_back_to_escape(&mut plain);
        assert_eq!(plain, vec![key(Key::Escape, Some(Key::Escape), true)]);
    }

    /// Ctrl+V pastes the system clipboard, whatever egui-winit's own held; the clipboard is read
    /// only for a paste, and the seed is never pasted.
    #[test]
    fn hardware_paste_reads_the_system_clipboard() {
        let mut reads = 0;
        let mut events = vec![Event::Text("a".into())];
        system_pastes(&mut events, || {
            reads += 1;
            Ok(Some("x".into()))
        });
        assert_eq!((reads, events.clone()), (0, vec![Event::Text("a".into())]), "no paste, no read");
        let mut events = vec![Event::Paste(PASTE_KEY_SEED.into()), Event::Text("a".into())];
        system_pastes(&mut events, || Ok(Some("Porto".into())));
        assert_eq!(events, vec![Event::Paste("Porto".into()), Event::Text("a".into())]);
        let mut events = vec![Event::Paste("old copy".into())];
        system_pastes(&mut events, || Ok(None));
        assert_eq!(events, vec![], "an empty clipboard pastes nothing");
        let mut events = vec![Event::Paste(PASTE_KEY_SEED.into()), Event::Paste("own copy".into())];
        system_pastes(&mut events, || Err("no focus".into()));
        assert_eq!(events, vec![Event::Paste("own copy".into())], "unreadable: the app's own copy, never the seed");
    }
}
