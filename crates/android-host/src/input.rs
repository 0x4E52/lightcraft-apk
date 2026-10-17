//! Input the host rewrites before the UI sees it.
//!
//! Android's Back key (and the back gesture, with the manifest's `enableOnBackInvokedCallback`
//! opt-out) reaches egui as `Key::BrowserBack`, which nothing in the UI maps. It becomes Escape,
//! so every Escape path applies first: an open menu, combo box, context menu or date picker
//! closes, a text field gives its text back, and only then does `view.back` (bound to Escape) run
//! its cascade, whose last step moves the app to the background (`Services::move_to_back`).

use egui::{Event, Key};

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
}
