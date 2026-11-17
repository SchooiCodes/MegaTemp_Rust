//! Design tokens and global widget styling for the GUI.
//!
//! The palette stays close to MEGA's own identity: near-black surfaces, a
//! single red accent, and muted greys for everything structural. All values
//! live here so screens never hard-code a colour.

use eframe::egui::{
    self, Color32, CornerRadius, FontFamily, FontId, Stroke, TextStyle, Vec2,
};

/// Window background, one step darker than the panels that sit on it.
pub const BG: Color32 = Color32::from_rgb(0x0B, 0x0C, 0x0F);
/// Cards, the rail and the top bar.
pub const SURFACE: Color32 = Color32::from_rgb(0x14, 0x16, 0x1B);
/// Inputs and hovered rows.
pub const RAISED: Color32 = Color32::from_rgb(0x1B, 0x1E, 0x25);
/// Hover on an already raised surface.
pub const RAISED_HOVER: Color32 = Color32::from_rgb(0x23, 0x27, 0x30);
/// Hairline borders.
pub const BORDER: Color32 = Color32::from_rgb(0x27, 0x2B, 0x33);
/// MEGA red.
pub const ACCENT: Color32 = Color32::from_rgb(0xD9, 0x27, 0x2E);
pub const ACCENT_HOVER: Color32 = Color32::from_rgb(0xE8, 0x41, 0x48);
/// Text colours.
pub const TEXT: Color32 = Color32::from_rgb(0xF1, 0xF3, 0xF6);
pub const MUTED: Color32 = Color32::from_rgb(0x8B, 0x93, 0xA1);
pub const FAINT: Color32 = Color32::from_rgb(0x5F, 0x67, 0x73);
/// Status colours.
pub const OK: Color32 = Color32::from_rgb(0x3D, 0xCF, 0x91);
pub const WARN: Color32 = Color32::from_rgb(0xF2, 0xB4, 0x41);
pub const DANGER: Color32 = Color32::from_rgb(0xFF, 0x5B, 0x60);

/// Corner radii, on a 4 px rhythm.
pub const RADIUS_CARD: u8 = 12;
pub const RADIUS_CONTROL: u8 = 9;

/// Standard inner padding for cards and sheets.
pub const PAD: i8 = 16;

/// Install the dark theme and the typographic scale.
pub fn apply(ctx: &egui::Context) {
    ctx.set_theme(egui::ThemePreference::Dark);
    let mut style = (*ctx.style_of(egui::Theme::Dark)).clone();

    let mut visuals = egui::Visuals::dark();
    visuals.override_text_color = Some(TEXT);
    visuals.panel_fill = BG;
    visuals.window_fill = SURFACE;
    visuals.extreme_bg_color = RAISED;
    visuals.faint_bg_color = RAISED;
    visuals.window_stroke = Stroke::new(1.0, BORDER);
    visuals.selection.bg_fill = ACCENT.gamma_multiply(0.35);
    visuals.selection.stroke = Stroke::new(1.0, ACCENT);
    visuals.hyperlink_color = ACCENT;
    visuals.slider_trailing_fill = true;

    // Every widget family gets the same rounded, bordered treatment so nothing
    // falls back to egui's default square look.
    visuals.widgets.noninteractive.bg_fill = SURFACE;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, MUTED);
    visuals.widgets.noninteractive.corner_radius = CornerRadius::same(RADIUS_CONTROL);

    visuals.widgets.inactive.bg_fill = RAISED;
    visuals.widgets.inactive.weak_bg_fill = RAISED;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, BORDER);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.5, TEXT);
    visuals.widgets.inactive.corner_radius = CornerRadius::same(RADIUS_CONTROL);

    visuals.widgets.hovered.bg_fill = RAISED_HOVER;
    visuals.widgets.hovered.weak_bg_fill = RAISED_HOVER;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, ACCENT.gamma_multiply(0.6));
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.5, TEXT);
    visuals.widgets.hovered.corner_radius = CornerRadius::same(RADIUS_CONTROL);

    visuals.widgets.active.bg_fill = ACCENT;
    visuals.widgets.active.weak_bg_fill = ACCENT;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, ACCENT_HOVER);
    visuals.widgets.active.fg_stroke = Stroke::new(1.5, Color32::WHITE);
    visuals.widgets.active.corner_radius = CornerRadius::same(RADIUS_CONTROL);

    visuals.widgets.open.bg_fill = RAISED;
    visuals.widgets.open.bg_stroke = Stroke::new(1.0, BORDER);
    visuals.widgets.open.fg_stroke = Stroke::new(1.5, TEXT);
    visuals.widgets.open.corner_radius = CornerRadius::same(RADIUS_CONTROL);

    style.visuals = visuals;
    style.spacing.item_spacing = Vec2::new(10.0, 8.0);
    style.spacing.button_padding = Vec2::new(14.0, 8.0);
    style.spacing.interact_size.y = 32.0;
    style.spacing.window_margin = egui::Margin::same(16);
    style.spacing.indent = 18.0;

    style.text_styles = [
        (TextStyle::Heading, FontId::new(24.0, FontFamily::Proportional)),
        (TextStyle::Body, FontId::new(14.0, FontFamily::Proportional)),
        (TextStyle::Button, FontId::new(14.0, FontFamily::Proportional)),
        (TextStyle::Small, FontId::new(12.0, FontFamily::Proportional)),
        (TextStyle::Monospace, FontId::new(13.0, FontFamily::Monospace)),
    ]
    .into();

    ctx.set_style_of(egui::Theme::Dark, style);
}

/// A card surface: rounded, hairline border, standard padding.
pub fn card() -> egui::Frame {
    egui::Frame::new()
        .fill(SURFACE)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(RADIUS_CARD))
        .inner_margin(egui::Margin::same(PAD))
}

/// A card that reacts to the pointer, used for clickable tiles.
pub fn card_hover(ui: &egui::Ui) -> egui::Frame {
    let fill = if ui.rect_contains_pointer(ui.max_rect()) {
        RAISED
    } else {
        SURFACE
    };
    egui::Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(RADIUS_CARD))
        .inner_margin(egui::Margin::same(PAD))
}

/// Section label: small, letter-spaced, muted.
pub fn section_label(text: &str) -> egui::RichText {
    egui::RichText::new(text.to_uppercase())
        .size(11.0)
        .color(MUTED)
        .strong()
}

/// Status chip for a run or account state.
pub fn chip(ui: &mut egui::Ui, text: &str, colour: Color32) {
    let frame = egui::Frame::new()
        .fill(colour.gamma_multiply(0.16))
        .stroke(Stroke::new(1.0, colour.gamma_multiply(0.55)))
        .corner_radius(CornerRadius::same(255))
        .inner_margin(egui::Margin::symmetric(10, 3));
    frame.show(ui, |ui| {
        ui.label(egui::RichText::new(text).size(12.0).color(colour).strong());
    });
}
