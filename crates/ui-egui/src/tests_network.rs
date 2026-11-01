//! Settings ▸ Network: every network use is listed, and "Never use the network" reaches the downloads.

use serde_json::json;

use crate::headless::Headless;
use crate::state::Dialog;
use crate::{LightcraftApp, Services};

#[test]
fn network_section_lists_uses() {
    let session = lightcraft_engine::Session::new();
    let mut h = Headless::new(LightcraftApp::new(session, Services::default()), [1000.0, 900.0], 1.0);
    h.app.ui.dialog = Some(Dialog::Settings { tab: "network".into() });
    for _ in 0..5 {
        h.step();
    }
    let has = |h: &Headless, id: &str| h.app.widgets.iter().any(|(w, _)| w == id);
    for u in lightcraft_engine::config::NETWORK_USES {
        assert!(has(&h, &format!("networkUse-{}", u.id)), "{} is listed", u.id);
    }
    assert!(has(&h, "button:settingsTab-network") && has(&h, "check:settings.networkOff"));
    // the switch is saved in the settings and reaches the engine on the next frame
    assert!(!h.app.ui.settings.network_off);
    assert_eq!(h.request("ui.clickWidget", json!({"id": "check:settings.networkOff"}), std::time::Duration::from_secs(20))["ok"], true);
    for _ in 0..3 {
        h.step();
    }
    // (the switch is process-wide and other tests' apps run in parallel: check what this app sent)
    assert!(h.app.ui.settings.network_off);
    assert_eq!(h.app.network_applied, Some(true));
    h.app.ui.settings.network_off = false;
    h.step();
    h.step();
    assert_eq!(h.app.network_applied, Some(false));
}
