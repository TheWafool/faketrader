//! Colours and type. Brass marks anything that is *yours*: orders, fills,
//! position lines, your equity. Market data never uses it.

use egui::{Color32, FontData, FontDefinitions, FontFamily, FontId, Stroke, TextStyle};
use std::sync::Arc;

#[derive(Clone, Copy)]
pub struct Pal {
    pub bg: Color32,
    pub panel: Color32,
    pub raised: Color32,
    pub line: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub up: Color32,
    pub down: Color32,
    pub brass: Color32,
    pub sma_fast: Color32,
    pub sma_slow: Color32,
    pub ema: Color32,
    pub band: Color32,
    pub vwap: Color32,
}

const fn hex(v: u32) -> Color32 {
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

pub const DARK: Pal = Pal {
    bg: hex(0x1D242C),
    panel: hex(0x232B35),
    raised: hex(0x2C3641),
    line: hex(0x3A4553),
    text: hex(0xDCE2E8),
    muted: hex(0x8C97A4),
    up: hex(0x3FA87C),
    down: hex(0xD9604F),
    brass: hex(0xD4A64A),
    sma_fast: hex(0x6FA3D8),
    sma_slow: hex(0xA98BD6),
    ema: hex(0x5FC4C4),
    band: hex(0x7C8794),
    vwap: hex(0xE08DB4),
};

pub const LIGHT: Pal = Pal {
    bg: hex(0xE9EDF1),
    panel: hex(0xF5F7F9),
    raised: hex(0xFFFFFF),
    line: hex(0xC9D1DA),
    text: hex(0x1E2731),
    muted: hex(0x5A6573),
    up: hex(0x1C8556),
    down: hex(0xC0402F),
    brass: hex(0x9A6E12),
    sma_fast: hex(0x2F6FB3),
    sma_slow: hex(0x7B55B8),
    ema: hex(0x16888A),
    band: hex(0x8A94A0),
    vwap: hex(0xB84A80),
};

pub fn pal(ctx: &egui::Context) -> Pal {
    if ctx.global_style().visuals.dark_mode { DARK } else { LIGHT }
}

pub fn heading() -> FontFamily {
    FontFamily::Name("heading".into())
}

pub fn mono(size: f32) -> FontId {
    FontId::new(size, FontFamily::Monospace)
}

pub fn install_fonts(ctx: &egui::Context) {
    let mut f = FontDefinitions::default();
    let add = |f: &mut FontDefinitions, name: &str, bytes: &'static [u8]| {
        f.font_data.insert(name.into(), Arc::new(FontData::from_static(bytes)));
    };
    add(&mut f, "barlow", include_bytes!("../../assets/fonts/BarlowSemiCondensed-Regular.ttf"));
    add(&mut f, "barlow-semi", include_bytes!("../../assets/fonts/BarlowSemiCondensed-SemiBold.ttf"));
    add(&mut f, "plex", include_bytes!("../../assets/fonts/IBMPlexMono-Regular.ttf"));
    f.families.get_mut(&FontFamily::Proportional).unwrap().insert(0, "barlow".into());
    f.families.get_mut(&FontFamily::Monospace).unwrap().insert(0, "plex".into());
    let mut head = vec!["barlow-semi".to_string()];
    head.extend(f.families[&FontFamily::Proportional].iter().skip(1).cloned());
    f.families.insert(heading(), head);
    ctx.set_fonts(f);
}

pub fn apply(ctx: &egui::Context, dark: bool, scale: f32) {
    let p = if dark { DARK } else { LIGHT };
    let mut v = if dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    v.panel_fill = p.panel;
    v.window_fill = p.raised;
    v.extreme_bg_color = p.bg;
    v.faint_bg_color = p.raised;
    v.window_stroke = Stroke::new(1.0, p.line);
    v.override_text_color = Some(p.text);
    v.hyperlink_color = p.brass;
    v.selection.bg_fill = p.brass.gamma_multiply(0.35);
    v.selection.stroke = Stroke::new(1.0, p.brass);
    let w = &mut v.widgets;
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.line);
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    for (wv, fill) in [(&mut w.inactive, p.raised), (&mut w.hovered, p.line), (&mut w.active, p.line), (&mut w.open, p.raised)] {
        wv.bg_fill = fill;
        wv.weak_bg_fill = fill;
        wv.corner_radius = 3.into();
        wv.fg_stroke = Stroke::new(1.0, p.text);
    }
    w.inactive.bg_stroke = Stroke::new(1.0, p.line);
    w.hovered.bg_stroke = Stroke::new(1.0, p.muted);
    w.active.bg_stroke = Stroke::new(1.0, p.brass);
    v.window_corner_radius = 6.into();
    ctx.set_visuals(v);

    ctx.all_styles_mut(|s| {
        use FontFamily::*;
        s.text_styles = [
            (TextStyle::Small, FontId::new(12.5, Proportional)),
            (TextStyle::Body, FontId::new(15.0, Proportional)),
            (TextStyle::Button, FontId::new(15.0, Proportional)),
            (TextStyle::Heading, FontId::new(21.0, heading())),
            (TextStyle::Monospace, FontId::new(13.5, Monospace)),
        ]
        .into();
        s.spacing.item_spacing = egui::vec2(8.0, 6.0);
        s.spacing.button_padding = egui::vec2(10.0, 4.0);
        s.spacing.interact_size.y = 26.0;
    });
    ctx.set_zoom_factor(scale);
}
