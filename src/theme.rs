//! Shared visual language for the normal, mini and subtitle windows.
//!
//! Tokens follow the supplied design specification: a dark canvas, one raised
//! surface, a single accent, and text roles whose contrast is verified below.
//! Values are logical points, so they scale with the display DPI setting.

use eframe::egui::{self, Color32, RichText, Stroke};

// ---------------------------------------------------------------------------
// Colour: semantic roles, not component-local literals.
// ---------------------------------------------------------------------------

/// Base canvas behind every screen.
pub const CANVAS: Color32 = Color32::from_rgb(17, 20, 25);
/// Cards, panels, menu backgrounds.
pub const SURFACE: Color32 = Color32::from_rgb(23, 28, 34);
/// Raised or selected surfaces (hover, active row).
pub const SURFACE_RAISED: Color32 = Color32::from_rgb(30, 36, 44);
/// Text entry background.
pub const INPUT: Color32 = Color32::from_rgb(13, 16, 20);
/// Default hairline.
pub const BORDER: Color32 = Color32::from_rgb(41, 48, 58);
/// Emphasis border and focus-adjacent chrome.
pub const BORDER_STRONG: Color32 = Color32::from_rgb(102, 114, 127);

/// Primary reading text.
pub const TEXT: Color32 = Color32::from_rgb(242, 245, 249);
/// Supporting labels and metadata.
pub const TEXT_SECONDARY: Color32 = Color32::from_rgb(182, 192, 204);
/// De-emphasised hints that still have to stay readable.
pub const TEXT_MUTED: Color32 = Color32::from_rgb(143, 154, 168);

/// Single accent for selection, links and progress.
pub const ACCENT: Color32 = Color32::from_rgb(82, 211, 178);
/// Muted accent used for fills behind text.
pub const ACCENT_SOFT: Color32 = Color32::from_rgb(28, 74, 65);
/// Text drawn on top of the accent colour.
pub const ON_ACCENT: Color32 = Color32::from_rgb(6, 20, 16);

pub const ERROR: Color32 = Color32::from_rgb(255, 146, 146);
pub const WARNING: Color32 = Color32::from_rgb(244, 196, 124);
pub const SUCCESS: Color32 = Color32::from_rgb(132, 222, 186);

// ---------------------------------------------------------------------------
// Spacing, radius and control sizes (8-point scale with 4-point half steps).
// ---------------------------------------------------------------------------

pub const SPACE_1: f32 = 4.0;
pub const SPACE_2: f32 = 8.0;
pub const SPACE_3: f32 = 12.0;
pub const SPACE_4: f32 = 16.0;
pub const SPACE_6: f32 = 24.0;

/// Navigation rail width. The reading column gets everything else.
pub const RAIL_WIDTH: f32 = 96.0;
/// Outer gutter of the content area.
pub const GUTTER: f32 = 24.0;

/// Upper bound for a single prose column: about 75 Japanese characters at the
/// current reading size, and never narrower than a comfortable minimum.
pub fn prose_width(reading_size: f32) -> f32 {
    (reading_size * 75.0).clamp(900.0, 1800.0)
}
/// Minimum interactive height. Every control is at least this tall.
pub const CONTROL_H: f32 = 32.0;
/// Primary actions and navigation items.
pub const CONTROL_H_LARGE: f32 = 40.0;

pub const RADIUS_CONTROL: u8 = 8;
pub const RADIUS_CARD: u8 = 10;

// ---------------------------------------------------------------------------
// Typography.
// ---------------------------------------------------------------------------

pub const SIZE_TITLE: f32 = 20.0;
pub const SIZE_BODY: f32 = 16.0;
pub const SIZE_LABEL: f32 = 14.0;
pub const SIZE_CAPTION: f32 = 12.0;

/// Translation text: large enough for 20-24 pt Japanese at 125% DPI.
pub const READING_MIN: f32 = 20.0;
pub const READING_DEFAULT: f32 = 22.0;
pub const READING_MAX: f32 = 28.0;

/// Clamp a user-chosen reading size into the supported range.
pub fn reading_size(size: f32) -> f32 {
    size.clamp(READING_MIN, READING_MAX)
}

pub fn title(text: impl Into<String>) -> RichText {
    RichText::new(text).size(SIZE_TITLE).strong().color(TEXT)
}
pub fn body(text: impl Into<String>) -> RichText {
    RichText::new(text).size(SIZE_BODY).color(TEXT)
}
pub fn label(text: impl Into<String>) -> RichText {
    RichText::new(text).size(SIZE_LABEL).color(TEXT_SECONDARY)
}
pub fn caption(text: impl Into<String>) -> RichText {
    RichText::new(text).size(SIZE_CAPTION).color(TEXT_MUTED)
}
pub fn reading(text: impl Into<String>, size: f32) -> RichText {
    RichText::new(text).size(reading_size(size)).color(TEXT)
}

// ---------------------------------------------------------------------------
// Frames and strokes.
// ---------------------------------------------------------------------------

pub fn hairline() -> Stroke {
    Stroke::new(1.0_f32, BORDER)
}

/// Standard grouped surface.
pub fn card() -> egui::Frame {
    egui::Frame::new()
        .fill(SURFACE)
        .stroke(hairline())
        .corner_radius(RADIUS_CARD)
        .inner_margin(egui::Margin::same(SPACE_4 as i8))
}

/// Borderless surface for the reading canvas: no box around prose.
pub fn reading_surface() -> egui::Frame {
    egui::Frame::new()
        .fill(SURFACE)
        .corner_radius(RADIUS_CARD)
        .inner_margin(egui::Margin::symmetric(SPACE_6 as i8, SPACE_4 as i8))
}

// ---------------------------------------------------------------------------
// Style installation.
// ---------------------------------------------------------------------------

pub fn apply(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    style.visuals = egui::Visuals::dark();
    style.text_styles.insert(
        egui::TextStyle::Heading,
        egui::FontId::proportional(SIZE_TITLE),
    );
    style
        .text_styles
        .insert(egui::TextStyle::Body, egui::FontId::proportional(SIZE_BODY));
    style.text_styles.insert(
        egui::TextStyle::Button,
        egui::FontId::proportional(SIZE_LABEL),
    );
    style.text_styles.insert(
        egui::TextStyle::Small,
        egui::FontId::proportional(SIZE_CAPTION),
    );
    style.text_styles.insert(
        egui::TextStyle::Monospace,
        egui::FontId::monospace(SIZE_LABEL),
    );

    style.spacing.item_spacing = egui::vec2(SPACE_2, SPACE_2);
    style.spacing.button_padding = egui::vec2(SPACE_3, SPACE_2);
    style.spacing.interact_size = egui::vec2(CONTROL_H, CONTROL_H);
    style.spacing.menu_margin = egui::Margin::same(SPACE_2 as i8);
    style.spacing.scroll = egui::style::ScrollStyle::solid();
    style.spacing.scroll.bar_width = 8.0;
    style.spacing.scroll.bar_inner_margin = 4.0;

    style.visuals.override_text_color = Some(TEXT);
    style.visuals.panel_fill = CANVAS;
    style.visuals.window_fill = SURFACE;
    style.visuals.extreme_bg_color = INPUT;
    style.visuals.faint_bg_color = SURFACE;
    style.visuals.selection.bg_fill = ACCENT_SOFT;
    style.visuals.selection.stroke = Stroke::new(2.0_f32, ACCENT);
    style.visuals.hyperlink_color = ACCENT;
    style.visuals.window_stroke = Stroke::new(1.0_f32, BORDER_STRONG);
    style.visuals.window_corner_radius = egui::CornerRadius::same(RADIUS_CARD);

    for (widget, fill, stroke) in [
        (&mut style.visuals.widgets.inactive, SURFACE, BORDER),
        (
            &mut style.visuals.widgets.hovered,
            SURFACE_RAISED,
            BORDER_STRONG,
        ),
        (&mut style.visuals.widgets.active, SURFACE_RAISED, ACCENT),
        (
            &mut style.visuals.widgets.open,
            SURFACE_RAISED,
            BORDER_STRONG,
        ),
    ] {
        widget.bg_fill = fill;
        widget.weak_bg_fill = fill;
        widget.bg_stroke = Stroke::new(1.0_f32, stroke);
        widget.fg_stroke = Stroke::new(1.0_f32, TEXT);
        widget.corner_radius = egui::CornerRadius::same(RADIUS_CONTROL);
    }
    // Keyboard focus reuses the active widget visuals, so the ring must read
    // as a focus indicator as well as a press state.
    style.visuals.widgets.active.bg_stroke = Stroke::new(2.0_f32, ACCENT);
    style.visuals.widgets.active.expansion = 0.0;
    style.visuals.widgets.noninteractive.bg_stroke = hairline();
    style.visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, TEXT_SECONDARY);
    style.visuals.widgets.noninteractive.corner_radius = egui::CornerRadius::same(RADIUS_CONTROL);
    ctx.set_style(style);
}

/// Draw an explicit 2 pt focus ring for widgets whose own frame is invisible.
pub fn focus_ring(ui: &egui::Ui, response: &egui::Response) {
    if response.has_focus() {
        ui.painter().rect_stroke(
            response.rect.expand(2.0),
            egui::CornerRadius::same(RADIUS_CONTROL),
            Stroke::new(2.0_f32, ACCENT),
            egui::StrokeKind::Outside,
        );
    }
}

// ---------------------------------------------------------------------------
// Contrast (WCAG 2.2).
// ---------------------------------------------------------------------------

/// sRGB channel to linear light, per WCAG 2.2 relative luminance.
#[cfg(test)]
fn linear(channel: u8) -> f32 {
    let value = channel as f32 / 255.0;
    if value <= 0.040_45 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

#[cfg(test)]
pub fn relative_luminance(color: Color32) -> f32 {
    0.2126 * linear(color.r()) + 0.7152 * linear(color.g()) + 0.0722 * linear(color.b())
}

#[cfg(test)]
pub fn contrast_ratio(a: Color32, b: Color32) -> f32 {
    let (light, dark) = {
        let (a, b) = (relative_luminance(a), relative_luminance(b));
        if a >= b { (a, b) } else { (b, a) }
    };
    (light + 0.05) / (dark + 0.05)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_roles_meet_wcag_aa_on_every_dark_surface() {
        for (name, fg, bg, minimum) in [
            ("body/canvas", TEXT, CANVAS, 4.5),
            ("body/surface", TEXT, SURFACE, 4.5),
            ("secondary/canvas", TEXT_SECONDARY, CANVAS, 4.5),
            ("secondary/surface", TEXT_SECONDARY, SURFACE, 4.5),
            ("muted/canvas", TEXT_MUTED, CANVAS, 4.5),
            ("muted/surface", TEXT_MUTED, SURFACE, 4.5),
            ("accent/canvas", ACCENT, CANVAS, 4.5),
            ("on-accent/accent", ON_ACCENT, ACCENT, 4.5),
            ("error/surface", ERROR, SURFACE, 4.5),
            ("warning/surface", WARNING, SURFACE, 4.5),
            ("success/surface", SUCCESS, SURFACE, 4.5),
            ("border/surface", BORDER_STRONG, SURFACE, 3.0),
            ("focus/canvas", ACCENT, CANVAS, 3.0),
        ] {
            let ratio = contrast_ratio(fg, bg);
            assert!(
                ratio >= minimum,
                "{name}: {ratio:.2}:1 is below {minimum}:1"
            );
        }
    }

    #[test]
    fn applied_style_keeps_a_visible_focus_ring_and_large_targets() {
        let context = egui::Context::default();
        apply(&context);
        let style = context.style();
        assert!(style.spacing.interact_size.y >= 32.0);
        assert_eq!(style.visuals.widgets.active.bg_stroke.color, ACCENT);
        assert_eq!(style.visuals.widgets.active.bg_stroke.width, 2.0);
        assert_eq!(
            style.visuals.widgets.inactive.corner_radius.nw,
            RADIUS_CONTROL
        );
        assert_eq!(
            style
                .text_styles
                .get(&egui::TextStyle::Body)
                .map(|font| font.size),
            Some(SIZE_BODY)
        );
    }

    #[test]
    fn reading_size_stays_within_the_supported_scale() {
        assert_eq!(reading_size(10.0), READING_MIN);
        assert_eq!(reading_size(22.0), 22.0);
        assert_eq!(reading_size(80.0), READING_MAX);
        assert!(reading_size(19.9) >= 20.0);
        assert!(reading_size(READING_DEFAULT) >= 20.0);
        assert!(reading_size(READING_MAX) <= 28.0);
    }
}
