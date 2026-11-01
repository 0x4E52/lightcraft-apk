//! The phone layout (APK-33): below [`crate::state::PHONE_WIDTH`] the side panels don't sit beside
//! the photo. The right-hand tool strip becomes the **edit toolbar** along the bottom (one row of
//! icon + label buttons that scrolls sideways), and a tool opens its panel as a **sheet** above
//! it, full width, with a drag handle that makes it taller or shorter
//! (`docs/android-touch-ui.md` §1). The panels are the desktop's own (`panels::right`, `left`,
//! `presets`): no second UI. Back (`view.back`) closes the sheet first.
//!
//! The sheet only shows what [`crate::UiState`] already says is open (the right panel, the
//! presets column, the left sidebar). Opening a tool in the sheet switches that panel on through
//! its command, as the desktop's tool strip does, so a window widened back to a desktop width shows
//! it there too; whether the sheet is up, which panel it shows and its height are the sheet's own,
//! so folding the sheet (its tool's button, Back) leaves the desktop layout's choices alone.

use std::ops::Range;

use egui::{Rect, Sense, pos2, vec2};
use serde_json::json;

use crate::LightcraftApp;
use crate::icons::{Icon, paint};
use crate::state::RightPanel;
use crate::theme::Tokens;
use crate::widgets::register;

/// What the sheet shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Content {
    /// The right panel (`UiState::right`): Edit, Crop, Masking, Info …
    #[default]
    Tools,
    /// The presets column (`UiState::presets`).
    Presets,
    /// The left sidebar (`UiState::left_panel`): sources, albums, folders.
    Library,
}

/// The phone layout's state for this session (not saved).
#[derive(Clone, Debug)]
pub struct SheetState {
    /// The window is narrower than [`crate::state::PHONE_WIDTH`] (set every frame).
    pub phone: bool,
    /// The sheet is up.
    pub open: bool,
    pub content: Content,
    /// The sheet's height as a share of the space below the top bar (resolution independent).
    pub fraction: f32,
    /// The panel switches as the last phone frame saw them, to open the sheet when a command
    /// (a shortcut, the menu, an agent) turns one on.
    last: Option<(RightPanel, bool, bool)>,
    /// The widgets registered by the sheet's content in the last frame: their indexes in
    /// `LightcraftApp::widgets` (tests check none reaches past the sheet).
    pub drawn: Range<usize>,
    /// The sheet's rect in the last frame.
    pub rect: Option<Rect>,
    /// The rect its panel's content took in the last frame (the space its widgets allocated; text
    /// and shapes drawn straight with the painter don't grow it, see `cut`).
    pub body: Option<Rect>,
    /// What the panel painted in the last frame that a clip rect cut off at its left or right side:
    /// each shape's bounds and the clip that cut it. The sheet's body clips to the sheet, so text
    /// painted past its edge (a value at a fixed offset, a long EXIF line) is lost unseen; tests
    /// check this stays empty.
    pub cut: Vec<(Rect, Rect)>,
}

impl Default for SheetState {
    fn default() -> Self {
        Self {
            phone: false,
            open: false,
            content: Content::Tools,
            fraction: DEFAULT_FRACTION,
            last: None,
            drawn: 0..0,
            rect: None,
            body: None,
            cut: Vec::new(),
        }
    }
}

/// The sheet's height as it first opens, and its limits (shares of the space it shares with the photo).
const DEFAULT_FRACTION: f32 = 0.45;
const MIN_FRACTION: f32 = 0.2;
const MAX_FRACTION: f32 = 0.85;
/// The drag handle's height (a finger-sized band under touch density).
const HANDLE_H: f32 = 28.0;
const HANDLE_TOUCH_H: f32 = 48.0;
/// One toolbar button (icon over label): finger-sized in both directions.
const TOOL_W: f32 = 68.0;
const TOOL_H: f32 = 56.0;

/// The toolbar's tools, in the order of the desktop's tool strip, the Library first.
const TOOLS: [(&str, Icon, &str); 11] = [
    ("library", Icon::Sidebar, "Library"),
    ("presets", Icon::Presets, "Presets"),
    ("edit", Icon::Sliders, "Edit"),
    ("crop", Icon::Crop, "Crop"),
    ("remove", Icon::Eraser, "Remove"),
    ("masking", Icon::Mask, "Masking"),
    ("redeye", Icon::Eye, "Red Eye"),
    ("versions", Icon::Versions, "Versions"),
    ("activity", Icon::Activity, "History"),
    ("keywords", Icon::Tag, "Keywords"),
    ("info", Icon::Info, "Info"),
];

/// The right panel a toolbar tool opens (`None` for Library and Presets).
fn tool_panel(id: &str) -> Option<RightPanel> {
    Some(match id {
        "edit" => RightPanel::Edit,
        "crop" => RightPanel::Crop,
        "remove" => RightPanel::Remove,
        "masking" => RightPanel::Masking,
        "redeye" => RightPanel::RedEye,
        "versions" => RightPanel::Versions,
        "activity" => RightPanel::Activity,
        "keywords" => RightPanel::Keywords,
        "info" => RightPanel::Info,
        _ => return None,
    })
}

/// The right panel shows tool `p` (the profile browser is part of Edit).
fn shows(right: RightPanel, p: RightPanel) -> bool {
    right == p || (p == RightPanel::Edit && right == RightPanel::Profiles)
}

/// Switch the phone layout on or off for this frame (`width`: the content rect's, in points).
pub fn set_width(app: &mut LightcraftApp, width: f32) {
    let s = &mut app.ui.sheet;
    s.phone = width < crate::state::PHONE_WIDTH;
    if !s.phone {
        // coming back to a phone width starts with the sheet down, whatever was switched on meanwhile
        s.last = None;
        s.rect = None;
        s.body = None;
        s.cut.clear();
        s.drawn = 0..0;
    }
}

/// Whether `content` is still switched on in the shared UI state.
fn is_on(app: &LightcraftApp, content: Content) -> bool {
    match content {
        Content::Tools => app.ui.right != RightPanel::None,
        Content::Presets => app.ui.presets,
        Content::Library => app.ui.left_panel,
    }
}

/// Follow the panel switches: one a command turned on opens the sheet on it, and a sheet whose
/// panel was turned off closes.
pub fn follow(app: &mut LightcraftApp) {
    let now = (app.ui.right, app.ui.presets, app.ui.left_panel);
    if let Some((right, presets, left)) = app.ui.sheet.last {
        let opened = if now.0 != right && now.0 != RightPanel::None {
            Some(Content::Tools)
        } else if now.1 && !presets {
            Some(Content::Presets)
        } else if now.2 && !left {
            Some(Content::Library)
        } else {
            None
        };
        if let Some(content) = opened {
            app.ui.sheet.content = content;
            app.ui.sheet.open = true;
        }
    }
    app.ui.sheet.last = Some(now);
    if app.ui.sheet.open && !is_on(app, app.ui.sheet.content) {
        app.ui.sheet.open = false;
    }
}

/// Back's first step on a phone: close the sheet. Says whether there was one to close.
pub fn close(app: &mut LightcraftApp) -> bool {
    let s = &mut app.ui.sheet;
    // (on the desktop layout there is no sheet: leave its state for the next phone frame)
    let was = s.phone && s.open;
    if was {
        s.open = false;
    }
    was
}

/// Whether toolbar tool `id` is the one the sheet shows.
fn active(app: &LightcraftApp, id: &str) -> bool {
    let s = &app.ui.sheet;
    if !s.open {
        return false;
    }
    match (id, s.content) {
        ("library", Content::Library) | ("presets", Content::Presets) => true,
        (id, Content::Tools) => tool_panel(id).is_some_and(|p| shows(app.ui.right, p)),
        _ => false,
    }
}

/// A tap on toolbar tool `id`: the tool the sheet shows folds the sheet away; any other opens
/// its panel in the sheet (switching it on through its command, as the tool strip does).
pub(crate) fn tap(app: &mut LightcraftApp, id: &str) {
    if active(app, id) {
        app.ui.sheet.open = false;
        return;
    }
    let content = match (id, tool_panel(id)) {
        ("library", _) => {
            if !app.ui.left_panel {
                let _ = app.run("view.leftPanel", json!({}));
            }
            Content::Library
        }
        ("presets", _) => {
            if !app.ui.presets {
                let _ = app.run("panel.presets", json!({}));
            }
            Content::Presets
        }
        (id, Some(p)) => {
            if !shows(app.ui.right, p) {
                let _ = app.run(&format!("panel.{id}"), json!({}));
            }
            Content::Tools
        }
        _ => return,
    };
    if is_on(app, content) {
        app.ui.sheet.content = content;
        app.ui.sheet.open = true;
        // a command's switch is the tap's own: don't let `follow` read it as another opening
        app.ui.sheet.last = Some((app.ui.right, app.ui.presets, app.ui.left_panel));
    }
}

/// The edit toolbar along the bottom of a phone layout.
pub fn toolbar(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let has_photo = app.session.active().is_some();
    let bar = egui::Panel::bottom("edit_toolbar")
        .exact_size(TOOL_H + 8.0)
        .resizable(false)
        .frame(egui::Frame::NONE.fill(t.chrome).inner_margin(egui::Margin::symmetric(4, 4)).stroke(egui::Stroke::new(1.0, t.divider)))
        .show(ui, |ui| {
            egui::ScrollArea::horizontal().id_salt("edit-toolbar").auto_shrink([false, true]).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    for (id, icon, label) in TOOLS {
                        // as the desktop's tool strip: everything but the Library and History needs a photo
                        let enabled = has_photo || matches!(id, "library" | "activity");
                        if tool_button(ui, &t, id, icon, label, active(app, id), enabled) {
                            tap(app, id);
                        }
                    }
                });
            });
        });
    register(ui.ctx(), "panel:edit_toolbar", bar.response.rect);
}

/// One toolbar button: the icon over its label. Says whether it was tapped.
fn tool_button(ui: &mut egui::Ui, t: &Tokens, id: &str, icon: Icon, label: &str, on: bool, enabled: bool) -> bool {
    let label = crate::i18n::tr(label);
    let (r, resp) = ui.allocate_exact_size(vec2(TOOL_W, TOOL_H), if enabled { Sense::click() } else { Sense::hover() });
    resp.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, enabled, on, label));
    register(ui.ctx(), format!("toolbar:{id}"), r);
    let p = ui.painter();
    if on {
        p.rect_filled(r.shrink(2.0), 6.0, t.tool_active);
    } else if resp.hovered() && enabled {
        p.rect_filled(r.shrink(2.0), 6.0, t.hover.gamma_multiply(0.7));
    }
    let c = if !enabled {
        t.text_disabled
    } else if on {
        t.pick
    } else {
        t.icon
    };
    paint(p, Rect::from_center_size(pos2(r.center().x, r.top() + 19.0), vec2(22.0, 22.0)), icon, c);
    // the label elided to the button's width, so a long translation never spills into its neighbour
    let mut job = egui::text::LayoutJob::simple_singleline(label.to_string(), t.font(11.0), if enabled { t.text_label } else { t.text_disabled });
    job.wrap = egui::text::TextWrapping::truncate_at_width(TOOL_W - 4.0);
    let galley = ui.painter().layout_job(job);
    ui.painter().galley(pos2(r.center().x - galley.size().x / 2.0, r.bottom() - 6.0 - galley.size().y), galley, c);
    enabled && resp.clicked()
}

/// The sheet above the toolbar (nothing while it is down).
pub fn show(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    app.ui.sheet.drawn = 0..0;
    app.ui.sheet.rect = None;
    app.ui.sheet.body = None;
    app.ui.sheet.cut.clear();
    if !app.ui.sheet.open {
        return;
    }
    let t = Tokens::get(ui.ctx());
    let room = ui.available_rect_before_wrap().height().max(1.0);
    let fraction = app.ui.sheet.fraction.clamp(MIN_FRACTION, MAX_FRACTION);
    let height = (fraction * room).round();
    let handle_h = if app.ui.touch_density { HANDLE_TOUCH_H } else { HANDLE_H };
    let content = app.ui.sheet.content;
    let r = egui::Panel::bottom("phone_sheet")
        .exact_size(height)
        .resizable(false)
        .frame(egui::Frame::NONE.fill(t.chrome).stroke(egui::Stroke::new(1.0, t.divider)))
        .show(ui, |ui| {
            let (handle, resp) = ui.allocate_exact_size(vec2(ui.available_width(), handle_h), Sense::click_and_drag());
            crate::widgets::announce(&resp, crate::i18n::tr("Resize panel"));
            register(ui.ctx(), "sheet:handle", handle);
            let grip = Rect::from_center_size(handle.center(), vec2(36.0, 4.0));
            ui.painter().rect_filled(grip, 2.0, if resp.hovered() || resp.dragged() { t.text } else { t.text_dim });
            if resp.dragged() {
                // the handle follows the finger: up makes the sheet taller
                let dy = resp.drag_delta().y;
                app.ui.sheet.fraction = ((height - dy) / room).clamp(MIN_FRACTION, MAX_FRACTION);
            }
            if resp.double_clicked() {
                app.ui.sheet.fraction = DEFAULT_FRACTION;
            }
            let start = crate::widgets::registry_len(ui.ctx());
            let rect = ui.available_rect_before_wrap();
            let mut body = ui.new_child(egui::UiBuilder::new().id_salt("sheet-body").max_rect(rect));
            body.set_clip_rect(rect.intersect(ui.clip_rect()));
            let layer = body.layer_id();
            let first = ui.ctx().graphics(|g| g.get(layer).map_or(0, |l| l.next_idx().0));
            match content {
                Content::Tools => {
                    register(body.ctx(), "panel:right_panel", rect);
                    crate::panels::right::show(app, &mut body);
                }
                Content::Presets => crate::panels::presets::show(app, &mut body),
                Content::Library => {
                    register(body.ctx(), "panel:left_panel", rect);
                    crate::panels::left::show(app, &mut body);
                }
            }
            let cut = ui.ctx().graphics(|g| {
                g.get(layer).map_or_else(Vec::new, |l| {
                    l.all_entries()
                        .skip(first)
                        .filter_map(|c| {
                            let b = c.shape.visual_bounding_rect();
                            let past = b.is_positive() && (b.left() < c.clip_rect.left() - 0.5 || b.right() > c.clip_rect.right() + 0.5);
                            past.then_some((b, c.clip_rect))
                        })
                        .collect()
                })
            });
            (start..crate::widgets::registry_len(ui.ctx()), body.min_rect(), cut)
        });
    register(ui.ctx(), "panel:sheet", r.response.rect);
    (app.ui.sheet.drawn, app.ui.sheet.body, app.ui.sheet.cut) = (r.inner.0, Some(r.inner.1), r.inner.2);
    app.ui.sheet.rect = Some(r.response.rect);
}
