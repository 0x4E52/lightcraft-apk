//! #189: the in-window menu bar's menus and submenus stay below the bar. On a short window a
//! tall menu (Photo) used to be slid up over the bar by egui, hiding the menu titles, and the
//! pointer left on the title then opened the first row's submenu by itself.

use crate::headless::Headless;
use crate::{LightcraftApp, Services};
use serde_json::json;
use std::time::Duration;

const T: Duration = Duration::from_secs(5);
const SETTLE: Duration = Duration::from_secs(10);

fn app(size: [f32; 2]) -> Headless {
    let app = LightcraftApp::new(lightcraft_engine::Session::with_demo(), Services { png: None, ..Default::default() });
    let mut h = Headless::new(app, size, 1.0);
    h.settle(SETTLE);
    h
}

fn widget(h: &Headless, id: &str) -> Option<egui::Rect> {
    h.app.widgets.iter().find(|(w, _)| w == id).map(|(_, r)| *r)
}

fn widgets_with(h: &Headless, prefix: &str) -> Vec<(String, egui::Rect)> {
    h.app.widgets.iter().filter(|(w, _)| w.starts_with(prefix)).cloned().collect()
}

/// The open level `id` hangs below every menu title and ends inside the window.
fn check_bar_clear(h: &Headless, id: &str, what: &str) {
    let level = widget(h, id).unwrap_or_else(|| panic!("{what}: no {id} on screen"));
    let titles = widgets_with(h, "menu:");
    assert!(!titles.is_empty(), "{what}: no menu titles");
    let bar_bottom = titles.iter().map(|(_, r)| r.bottom()).fold(f32::MIN, f32::max);
    assert!(level.top() >= bar_bottom - 0.5, "{what}: {id} {level:?} starts above the bar's bottom {bar_bottom}");
    assert!(level.bottom() <= h.size.y + 0.5, "{what}: {id} {level:?} runs past the window's bottom {}", h.size.y);
    for (title, r) in &titles {
        assert!(!level.intersects(*r), "{what}: {id} {level:?} covers {title} {r:?}");
    }
}

#[test]
fn no_menu_or_submenu_covers_the_menu_bar() {
    // The reporter's 1280 × 720 display less the title bar, and a shorter window where even a
    // scrolled menu's submenus must open upward.
    let sizes = [[1280.0, 703.0], [1160.0, 560.0]];
    let mut submenus = 0;
    for size in sizes {
        for top in ["Photo", "File", "View"] {
            let what = format!("{size:?} {top}");
            let mut h = app(size);
            assert!(widget(&h, "menu-level:1").is_none(), "{what}: a menu is open before any click");
            let r = h.request("ui.clickWidget", json!({"id": format!("menu:{top}")}), T);
            assert_eq!(r["ok"], true, "{what}: {r}");
            h.settle(SETTLE);
            check_bar_clear(&h, "menu-level:1", &what);
            // Hover each submenu row in view, top to bottom: rows low in the menu open upward.
            let level = widget(&h, "menu-level:1").unwrap_or_else(|| panic!("{what}: menu"));
            let rows: Vec<String> = widgets_with(&h, "menusub:").into_iter().filter(|(_, r)| level.contains_rect(*r)).map(|(id, _)| id).collect();
            for row in rows {
                let r = h.request("ui.hoverWidget", json!({"id": row}), T);
                assert_eq!(r["ok"], true, "{what}: {r}");
                h.settle(SETTLE);
                if widget(&h, "menu-level:2").is_none() {
                    continue;
                }
                submenus += 1;
                check_bar_clear(&h, "menu-level:2", &format!("{what} › {row}"));
            }
        }
    }
    assert!(submenus >= 6, "opened {submenus} submenus");
}

#[test]
fn a_tall_menu_scrolls_instead_of_growing_past_the_window() {
    let mut h = app([1160.0, 420.0]);
    let r = h.request("ui.clickWidget", json!({"id": "menu:Photo"}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.settle(SETTLE);
    check_bar_clear(&h, "menu-level:1", "420 pt window");
    let level = widget(&h, "menu-level:1").expect("Photo menu");
    // Not every row fits: some are scrolled out of view, none drawn outside the menu.
    let rows = widgets_with(&h, "menusub:");
    assert!(!rows.is_empty(), "no submenu rows registered");
    assert!(rows.iter().any(|(_, r)| !level.contains_rect(*r)), "every row fits a 420 pt window: {rows:?}");
}

// ---- top-level menus switch on hover, like native menu bars

/// The title whose menu is open (the open level hangs from its left edge), or none.
fn open_menu(h: &Headless) -> Option<String> {
    let level = widget(h, "menu-level:1")?;
    widgets_with(h, "menu:").into_iter().filter(|(id, _)| id != "menu:all").find(|(_, r)| (r.left() - level.left()).abs() < 12.0).map(|(id, _)| id)
}

fn go(h: &mut Headless, method: &str, id: &str) {
    let r = h.request(method, json!({"id": id}), T);
    assert_eq!(r["ok"], true, "{method} {id}: {r}");
    h.settle(SETTLE);
}

#[test]
fn hovering_another_title_switches_the_open_menu() {
    let mut h = app([1280.0, 703.0]);
    // hovering with no menu open does nothing
    go(&mut h, "ui.hoverWidget", "menu:Edit");
    assert_eq!(open_menu(&h), None);
    go(&mut h, "ui.clickWidget", "menu:File");
    assert_eq!(open_menu(&h).as_deref(), Some("menu:File"));
    for title in ["Edit", "View", "Photo", "Window", "Help", "File"] {
        go(&mut h, "ui.hoverWidget", &format!("menu:{title}"));
        assert_eq!(open_menu(&h), Some(format!("menu:{title}")), "after hovering {title}");
    }
}

#[test]
fn clicking_another_title_opens_it_in_one_click() {
    let mut h = app([1280.0, 703.0]);
    go(&mut h, "ui.clickWidget", "menu:Edit");
    assert_eq!(open_menu(&h).as_deref(), Some("menu:Edit"));
    // move away from the titles first so only the click can be what switches
    let r = h.request("ui.move", json!({"x": 640.0, "y": 400.0}), T);
    assert_eq!(r["ok"], true);
    h.settle(SETTLE);
    go(&mut h, "ui.clickWidget", "menu:View");
    assert_eq!(open_menu(&h).as_deref(), Some("menu:View"));
    // clicking the open title closes it
    go(&mut h, "ui.clickWidget", "menu:View");
    assert_eq!(open_menu(&h), None);
}

#[test]
fn click_outside_and_escape_close_the_open_menu() {
    let mut h = app([1280.0, 703.0]);
    go(&mut h, "ui.clickWidget", "menu:File");
    assert!(open_menu(&h).is_some());
    let r = h.request("ui.click", json!({"x": 640.0, "y": 600.0}), T);
    assert_eq!(r["ok"], true);
    h.settle(SETTLE);
    assert_eq!(open_menu(&h), None, "click outside");
    go(&mut h, "ui.clickWidget", "menu:Photo");
    assert!(open_menu(&h).is_some());
    let r = h.request("ui.key", json!({"key": "Escape"}), T);
    assert_eq!(r["ok"], true, "{r}");
    h.settle(SETTLE);
    assert_eq!(open_menu(&h), None, "Escape");
}

// ---- a scrolled menu draws no scrollbar over its rows (issue #691)

/// egui's floating scrollbar sat on the rows' right edge, where the shortcut text ends, and in
/// its wide (hovered) state its background hid the last character. With the pointer resting on
/// the menu, no tall narrow rectangle (a bar's handle or track) is painted along the level's
/// right edge, in a menu that scrolls as in one that fits, and the level is as wide in both.
#[test]
fn a_scrolled_menu_draws_no_scrollbar_over_its_shortcuts() {
    let mut widths = Vec::new();
    for (size, scrolls) in [([1600.0, 700.0], true), ([1600.0, 1400.0], false)] {
        let what = format!("{} pt window", size[1]);
        let mut h = app(size);
        go(&mut h, "ui.clickWidget", "menu:View");
        go(&mut h, "ui.hoverWidget", "menu-level:1");
        let level = widget(&h, "menu-level:1").unwrap_or_else(|| panic!("{what}: no View menu"));
        let rows = widgets_with(&h, "menusub:");
        assert!(!rows.is_empty(), "{what}: no submenu rows registered");
        assert_eq!(rows.iter().any(|(_, r)| !level.contains_rect(*r)), scrolls, "{what}: scrolls = {scrolls}, level {level:?}");
        widths.push(level.width());
        // a row's hover highlight spans the level; a separator is one point tall; the ▸ of a
        // submenu row and the text are not rectangles: only a scrollbar is tall and narrow there
        let bars: Vec<egui::Rect> = h
            .view
            .painted_shapes()
            .iter()
            .filter_map(|s| match s {
                egui::epaint::Shape::Rect(r) => Some(r.rect),
                _ => None,
            })
            .filter(|r| r.width() <= 14.0 && r.height() > 30.0 && r.left() >= level.right() - 16.0 && r.right() <= level.right() + 0.5)
            .collect();
        assert!(bars.is_empty(), "{what}: a scrollbar is drawn along the menu's right edge: {bars:?}");
    }
    assert!((widths[0] - widths[1]).abs() < 0.5, "a scrolled menu is as wide as one that fits: {widths:?}");
}

// ---- APK-34: one overflow button below the phone breakpoint

/// Every menu title as a `menusub:` row in the open overflow level, in order.
#[test]
fn overflow_lists_every_menu_command() {
    // the desktop bar: its titles and, per title, the commands its menu offers
    let mut desk = app([1280.0, 703.0]);
    let bar = crate::menubar::menu_bar(&desk.app);
    assert!(widget(&desk, "menu:all").is_none(), "the desktop keeps its titles");
    let titles: Vec<String> = bar.iter().map(|(t, _)| t.clone()).collect();
    for t in &titles {
        assert!(widget(&desk, &format!("menu:{t}")).is_some(), "desktop title {t}");
    }
    desk.settle(SETTLE);

    let svc = Services { png: None, ..Default::default() };
    // 412 pt: the search field still runs over the button (APK-34b), so only look; 590 pt, still
    // below the breakpoint, leaves the button free to tap
    let mut narrow = Headless::new(
        LightcraftApp::new(lightcraft_engine::Session::with_demo(), Services { png: None, ..Default::default() }),
        [412.0, 915.0],
        2.625,
    );
    narrow.settle(SETTLE);
    assert!(narrow.app.ui.sheet.phone && widget(&narrow, "menu:all").is_some());
    let mut h = Headless::new(LightcraftApp::new(lightcraft_engine::Session::with_demo(), svc), [590.0, 900.0], 1.0);
    h.settle(SETTLE);
    assert!(h.app.ui.sheet.phone);
    assert!(widget(&h, "menu:all").is_some(), "one overflow button");
    assert!(titles.iter().all(|t| widget(&h, &format!("menu:{t}")).is_none()), "no titles in the phone bar");
    go(&mut h, "ui.clickWidget", "menu:all");
    h.step();
    h.step();
    let level = widget(&h, "menu-level:1")
        .unwrap_or_else(|| panic!("overflow open: {:?}", h.app.widgets.iter().filter(|w| w.0.starts_with("menu")).collect::<Vec<_>>()));
    for t in &titles {
        let r = widget(&h, &format!("menusub:{t}")).unwrap_or_else(|| panic!("overflow lacks {t}"));
        assert!(level.expand(0.5).contains_rect(r) || r.intersects(level), "{t} in the overflow");
    }
    // the overflow draws the desktop tree: per title, every item row (nested submenus walked by
    // hovering their rows) is registered as `menuitem:<id>` / `menuitem-off:<id>`, and the set of
    // ids with their enabled flags equals the tree's
    fn walk(n: &[crate::menubar::MenuNode], items: &mut Vec<(String, bool)>, subs: &mut Vec<String>) {
        for x in n {
            match x {
                crate::menubar::MenuNode::Item { id, enabled, .. } => items.push((id.clone(), *enabled)),
                crate::menubar::MenuNode::Submenu { label, children } => {
                    subs.push(label.clone());
                    walk(children, items, subs);
                }
                _ => {}
            }
        }
    }
    let drawn = |h: &Headless| -> std::collections::BTreeSet<(String, bool)> {
        h.app
            .widgets
            .iter()
            .filter_map(|(w, _)| {
                w.strip_prefix("menuitem:").map(|i| (i.to_string(), true)).or_else(|| w.strip_prefix("menuitem-off:").map(|i| (i.to_string(), false)))
            })
            .collect()
    };
    // a nested popup can cover its parent's later rows, so every submenu starts from a fresh overflow
    let open_title = |h: &mut Headless, t: &str| {
        h.step();
        if widget(h, "menu-level:1").is_some() {
            go(h, "ui.clickWidget", "menu:all");
            h.step();
        }
        go(h, "ui.clickWidget", "menu:all");
        h.step();
        h.step();
        go(h, "ui.hoverWidget", &format!("menusub:{t}"));
        h.settle(SETTLE);
        assert!(widget(h, "menu-level:2").is_some(), "{t}: its submenu opens");
    };
    let mut total = 0;
    let mut disabled = 0;
    for (t, nodes) in &bar {
        let (mut want, mut subs) = (vec![], vec![]);
        walk(nodes, &mut want, &mut subs);
        let want: std::collections::BTreeSet<(String, bool)> = want.into_iter().collect();
        open_title(&mut h, t);
        let mut got = drawn(&h);
        for s in &subs {
            open_title(&mut h, t);
            go(&mut h, "ui.hoverWidget", &format!("menusub:{s}"));
            h.settle(SETTLE);
            got.extend(drawn(&h));
        }
        assert_eq!(got, want, "{t}: the overflow draws the desktop menu's commands with the same enabled flags");
        total += want.len();
        disabled += want.iter().filter(|(_, e)| !e).count();
    }
    assert!(total > 50, "{total} commands compared");
    assert!(disabled > 0, "some disabled item was compared");
}

/// APK-34b turns this on: at the real portrait-phone width the search field still runs over the
/// Menu button, so a tap lands on the field.
#[test]
#[ignore = "APK-34b: the search field covers menu:all at 412 pt"]
fn menu_button_is_tappable_at_412() {
    let mut h = Headless::new(
        LightcraftApp::new(lightcraft_engine::Session::with_demo(), Services { png: None, ..Default::default() }),
        [412.0, 915.0],
        2.625,
    );
    h.settle(SETTLE);
    let r = widget(&h, "menu:all").expect("menu:all");
    let field = widget(&h, "field:search").expect("field:search");
    assert!(!r.intersects(field), "{r:?} vs {field:?}");
}

/// A phone-sized window never puts the titles in the bar, however much room the bar has (APK-34).
#[test]
fn phone_bar_is_one_button_even_with_room() {
    let mut view = crate::headless::HeadlessView::new();
    let mut run = |phone: bool| {
        let mut app = LightcraftApp::new(lightcraft_engine::Session::with_demo(), Services { png: None, ..Default::default() });
        app.ui.sheet.phone = phone;
        let raw = crate::headless::HeadlessView::raw_input(egui::vec2(1280.0, 703.0), 1.0, 0.0, vec![]);
        let mut ids = vec![];
        for _ in 0..2 {
            let raw = raw.clone();
            view.run(raw, |ui| {
                crate::menubar::show_in_window(&mut app, ui, 5000.0);
                ids = crate::widgets::take_registry(ui.ctx()).into_iter().map(|w| w.0).collect();
            });
        }
        ids
    };
    let desk = run(false);
    assert!(desk.iter().any(|i| i == "menu:File") && !desk.iter().any(|i| i == "menu:all"), "{desk:?}");
    let phone = run(true);
    assert!(phone.iter().any(|i| i == "menu:all"), "{phone:?}");
    assert!(!phone.iter().any(|i| i.starts_with("menu:") && i != "menu:all"), "{phone:?}");
}
