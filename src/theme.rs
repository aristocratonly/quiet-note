//! Palette, typography and widget styling.

use eframe::egui::{self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Stroke};
use std::sync::Arc;

// Surfaces, darkest to lightest.
pub const SIDEBAR: Color32 = Color32::from_rgb(0x0f, 0x10, 0x12);
pub const BG: Color32 = Color32::from_rgb(0x14, 0x15, 0x18);
pub const SURFACE: Color32 = Color32::from_rgb(0x19, 0x1a, 0x1e);
pub const INPUT: Color32 = Color32::from_rgb(0x1e, 0x20, 0x24);
pub const HOVER: Color32 = Color32::from_rgb(0x1b, 0x1c, 0x20);
pub const SELECTED: Color32 = Color32::from_rgb(0x23, 0x25, 0x2b);
pub const BORDER: Color32 = Color32::from_rgb(0x25, 0x27, 0x2c);

// Text.
pub const TEXT: Color32 = Color32::from_rgb(0xe6, 0xe6, 0xe9);
pub const MUTED: Color32 = Color32::from_rgb(0xa0, 0xa1, 0xa8);
pub const FAINT: Color32 = Color32::from_rgb(0x62, 0x64, 0x6c);

// Accents.
pub const ACCENT: Color32 = Color32::from_rgb(0x93, 0xa4, 0xf5);
pub const ACCENT_BG: Color32 = Color32::from_rgb(0x22, 0x27, 0x3d);
pub const DROP: Color32 = Color32::from_rgb(0x2a, 0x31, 0x4d);
pub const DANGER: Color32 = Color32::from_rgb(0xe0, 0x84, 0x84);
pub const GOOD: Color32 = Color32::from_rgb(0x7f, 0xc4, 0x93);
pub const WARN: Color32 = Color32::from_rgb(0xd9, 0xb2, 0x6a);

pub const SERIF: &str = "serif";

/// A colour at reduced opacity, for tinted backgrounds.
pub fn tint(c: Color32, a: f32) -> Color32 {
    c.gamma_multiply(a)
}

/// Blend two colours, `t` from 0 to 1.
pub fn lerp(a: Color32, b: Color32, t: f32) -> Color32 {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t.clamp(0.0, 1.0)).round() as u8;
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

/// Animation timing for the whole app: soft and short when smooth motion is on, instant when off.
pub fn motion(ctx: &egui::Context, smooth: bool) {
    ctx.style_mut(|s| {
        s.animation_time = if smooth { 0.2 } else { 0.0 };
        s.scroll_animation = if smooth { egui::style::ScrollAnimation::new(1400.0, egui::Rangef::new(0.12, 0.32)) } else { egui::style::ScrollAnimation::none() };
        s.visuals.text_cursor.blink = smooth;
        s.visuals.text_cursor.on_duration = 0.6;
        s.visuals.text_cursor.off_duration = 0.4;
    });
}

pub fn font(size: f32) -> FontId {
    FontId::proportional(size)
}

pub fn family_for(style: crate::TextStyle) -> FontFamily {
    match style {
        crate::TextStyle::Sans => FontFamily::Name(WRITE_SANS.into()),
        crate::TextStyle::Serif => FontFamily::Name(SERIF.into()),
        crate::TextStyle::Mono => FontFamily::Monospace,
    }
}

fn load(fonts: &mut FontDefinitions, name: &str, paths: &[&str]) -> bool {
    for p in paths {
        if let Ok(bytes) = std::fs::read(p) {
            fonts.font_data.insert(name.into(), Arc::new(FontData::from_owned(bytes)));
            return true;
        }
    }
    false
}

/// Fonts shipped inside the binary, so writing looks the same on every system.
/// Each face is split into Latin and Cyrillic files; both go into the family.
macro_rules! embed {
    ($fonts:expr, $name:literal, $lat:literal, $cyr:literal) => {{
        $fonts.font_data.insert(concat!($name, "-lat").into(), Arc::new(FontData::from_static(include_bytes!(concat!("../assets/fonts/", $lat)))));
        $fonts.font_data.insert(concat!($name, "-cyr").into(), Arc::new(FontData::from_static(include_bytes!(concat!("../assets/fonts/", $cyr)))));
        vec![concat!($name, "-lat").to_string(), concat!($name, "-cyr").to_string()]
    }};
}

pub const WRITE_SANS: &str = "write-sans";
pub const WRITE_SANS_BOLD: &str = "write-sans-bold";
pub const SERIF_BOLD: &str = "serif-bold";
pub const SERIF_ITALIC: &str = "serif-italic";

fn setup_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);

    let onest = embed!(fonts, "onest", "onest-latin-400.ttf", "onest-cyrillic-400.ttf");
    let onest_b = embed!(fonts, "onest-b", "onest-latin-500.ttf", "onest-cyrillic-500.ttf");
    let lit = embed!(fonts, "literata", "literata-latin-400-normal.ttf", "literata-cyrillic-400-normal.ttf");
    let lit_b = embed!(fonts, "literata-b", "literata-latin-600-normal.ttf", "literata-cyrillic-600-normal.ttf");
    let lit_i = embed!(fonts, "literata-i", "literata-latin-400-italic.ttf", "literata-cyrillic-400-italic.ttf");

    // Interface: San Francisco on macOS, Onest elsewhere (system fonts on Linux vary too much).
    let sf = cfg!(target_os = "macos") && load(&mut fonts, "sf", &["/System/Library/Fonts/SFNS.ttf"]);
    let mono = load(&mut fonts, "sysmono", &["/System/Library/Fonts/SFNSMono.ttf", "C:\\Windows\\Fonts\\consola.ttf", "/usr/share/fonts/truetype/jetbrains-mono/JetBrainsMono-Regular.ttf"]);

    let prop = fonts.families.entry(FontFamily::Proportional).or_default();
    let mut head: Vec<String> = if sf { vec!["sf".into()] } else { onest.clone() };
    head.extend(prop.drain(..));
    *prop = head;
    let fallback = prop.clone();
    let with_fb = |mut v: Vec<String>| {
        v.extend(fallback.iter().cloned());
        v
    };
    fonts.families.insert(FontFamily::Name(WRITE_SANS.into()), with_fb(onest));
    fonts.families.insert(FontFamily::Name(WRITE_SANS_BOLD.into()), with_fb(onest_b));
    fonts.families.insert(FontFamily::Name(SERIF.into()), with_fb(lit));
    fonts.families.insert(FontFamily::Name(SERIF_BOLD.into()), with_fb(lit_b));
    fonts.families.insert(FontFamily::Name(SERIF_ITALIC.into()), with_fb(lit_i));

    let monof = fonts.families.entry(FontFamily::Monospace).or_default();
    if mono {
        monof.insert(0, "sysmono".into());
    }
    monof.extend(fallback.iter().cloned());
    ctx.set_fonts(fonts);
}

pub fn apply(ctx: &egui::Context) {
    setup_fonts(ctx);

    let mut v = egui::Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = SURFACE;
    v.faint_bg_color = SURFACE;
    v.extreme_bg_color = INPUT;
    v.code_bg_color = INPUT;
    v.override_text_color = Some(TEXT);
    v.hyperlink_color = ACCENT;
    v.selection.bg_fill = Color32::from_rgb(0x2c, 0x34, 0x55);
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    v.text_cursor.stroke = Stroke::new(2.0, ACCENT);
    v.window_corner_radius = CornerRadius::same(12);
    v.menu_corner_radius = CornerRadius::same(10);
    v.window_stroke = Stroke::new(1.0, BORDER);
    v.window_shadow = egui::Shadow { offset: [0, 10], blur: 30, spread: 0, color: Color32::from_black_alpha(110) };
    v.popup_shadow = egui::Shadow { offset: [0, 6], blur: 18, spread: 0, color: Color32::from_black_alpha(100) };
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, MUTED);
    v.widgets.noninteractive.bg_fill = SURFACE;

    let w = &mut v.widgets;
    for (wv, fill, stroke) in [
        (&mut w.inactive, INPUT, BORDER),
        (&mut w.hovered, SELECTED, Color32::from_rgb(0x33, 0x36, 0x3d)),
        (&mut w.active, Color32::from_rgb(0x2a, 0x2d, 0x34), ACCENT),
        (&mut w.open, SELECTED, BORDER),
    ] {
        wv.corner_radius = CornerRadius::same(7);
        wv.bg_fill = fill;
        wv.weak_bg_fill = fill;
        wv.bg_stroke = Stroke::new(1.0, stroke);
        wv.expansion = 0.0;
    }
    w.inactive.fg_stroke = Stroke::new(1.0, MUTED);
    w.hovered.fg_stroke = Stroke::new(1.0, TEXT);
    w.active.fg_stroke = Stroke::new(1.0, TEXT);
    ctx.set_visuals(v);

    ctx.style_mut(|s| {
        s.spacing.button_padding = egui::vec2(10.0, 5.0);
        s.spacing.item_spacing = egui::vec2(8.0, 6.0);
        s.spacing.menu_margin = Margin::same(6);
        s.spacing.window_margin = Margin::same(20);
        s.spacing.interact_size.y = 28.0;
        s.spacing.combo_width = 120.0;
        s.text_styles.insert(egui::TextStyle::Body, font(13.5));
        s.text_styles.insert(egui::TextStyle::Button, font(13.5));
        s.text_styles.insert(egui::TextStyle::Small, font(11.5));
        s.text_styles.insert(egui::TextStyle::Monospace, FontId::monospace(13.0));
        s.visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, MUTED);
    });
}

/// A quiet icon button (icon only, tooltip on hover).
pub fn icon_button(ui: &mut egui::Ui, icon: &str, tip: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(26.0, 26.0), egui::Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, 6.0, SELECTED);
    }
    let col = if resp.hovered() { TEXT } else { FAINT };
    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, icon, font(15.0), col);
    resp.on_hover_text(tip)
}

/// Small uppercase label above a group.
pub fn caption(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text.to_uppercase()).size(10.5).color(FAINT).extra_letter_spacing(0.8));
}

/// A subtle pill button with an icon and text.
pub fn pill(ui: &mut egui::Ui, icon: &str, text: &str) -> egui::Response {
    let label = format!("{icon}  {text}");
    ui.add(
        egui::Button::new(egui::RichText::new(label).size(12.5).color(MUTED))
            .fill(SURFACE)
            .stroke(Stroke::new(1.0, BORDER))
            .corner_radius(14),
    )
}
