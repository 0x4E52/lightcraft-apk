//! About ▸ About: what this build is, its licence, and where its source lives.

use crate::{LightcraftApp, theme::Tokens};

pub fn about_ui(app: &mut LightcraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    ui.label(egui::RichText::new("LightCraft").font(t.semibold(20.0)).color(t.text));
    ui.label(crate::i18n::tr_format!("Version {} — a clean-room, pure-Rust photo library and raw developer.", env!("CARGO_PKG_VERSION")));
    ui.label(crate::i18n::tr_format!("MIT OR Apache-2.0. Fonts: {} (OFL). Icons: original.", crate::theme::font_credits(app.chinese_font.is_some())));
    // plain text only: a modified version may name the original, not carry its marks
    // (docs/brand/LICENSE-brand.txt)
    ui.label(egui::RichText::new(crate::i18n::tr("A modified version, based on LightCraft by the ArtCraft team.")).color(t.text_dim));
    ui.add_space(10.0);
    for (label, url) in [("Source code on GitHub", crate::links::GITHUB), ("The original LightCraft", crate::links::UPSTREAM)] {
        let r = ui.link(crate::i18n::tr(label)).on_hover_text(url);
        if r.clicked() {
            let _ = crate::links::open(app, url);
        }
    }
}
