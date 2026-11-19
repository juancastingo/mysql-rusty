use egui::{Color32, CornerRadius, Margin, RichText, Stroke};

#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub struct Palette {
    pub is_dark: bool,

    // Syntax highlighting tokens
    pub keyword: Color32,
    pub function: Color32,
    pub type_: Color32,
    pub string: Color32,
    pub number: Color32,
    pub comment: Color32,
    pub operator: Color32,
    pub identifier: Color32,
    pub punctuation: Color32,

    // Semantic accent & status tokens
    pub accent: Color32,
    pub accent_bg: Color32,
    pub accent_text: Color32,
    pub success: Color32,
    pub success_bg: Color32,
    pub warning: Color32,
    pub warning_bg: Color32,
    pub danger: Color32,
    pub danger_bg: Color32,
    pub purple: Color32,
    pub purple_bg: Color32,

    // Surface & border tokens
    pub panel_bg: Color32,
    pub surface_bg: Color32,
    pub card_bg: Color32,
    pub card_hover_bg: Color32,
    pub card_border: Color32,
    pub border_subtle: Color32,
    pub border_active: Color32,

    // Text tokens
    pub text_primary: Color32,
    pub text_secondary: Color32,
    pub muted_text: Color32,

    // Chrome, tab & grid tokens
    pub tab_active_bg: Color32,
    pub tab_inactive_bg: Color32,
    pub badge_bg: Color32,
    pub null_badge_bg: Color32,
    pub row_selected_bg: Color32,
    pub row_delete_bg: Color32,
    pub cell_edited_bg: Color32,
}

impl Palette {
    pub fn primary_button(&self, text: impl Into<String>) -> egui::Button<'static> {
        let fill = if self.is_dark {
            Color32::from_rgb(14, 116, 144) // Cyan 700
        } else {
            Color32::from_rgb(2, 132, 199) // Sky 600
        };
        egui::Button::new(RichText::new(text.into()).color(Color32::WHITE).strong())
            .fill(fill)
            .stroke(Stroke::new(1.0, self.accent))
            .corner_radius(CornerRadius::same(6))
    }

    pub fn danger_button(&self, text: impl Into<String>) -> egui::Button<'static> {
        let fill = if self.is_dark {
            Color32::from_rgb(185, 28, 28) // Red 700
        } else {
            Color32::from_rgb(220, 38, 38) // Red 600
        };
        egui::Button::new(RichText::new(text.into()).color(Color32::WHITE).strong())
            .fill(fill)
            .stroke(Stroke::new(1.0, self.danger))
            .corner_radius(CornerRadius::same(6))
    }

    pub fn subtle_button(&self, text: impl Into<String>) -> egui::Button<'static> {
        egui::Button::new(RichText::new(text.into()).color(self.text_primary))
            .fill(self.card_bg)
            .stroke(Stroke::new(1.0, self.card_border))
            .corner_radius(CornerRadius::same(6))
    }

    pub fn card_frame(&self) -> egui::Frame {
        egui::Frame::new()
            .fill(self.card_bg)
            .stroke(Stroke::new(1.0, self.card_border))
            .corner_radius(CornerRadius::same(8))
            .inner_margin(Margin::symmetric(12, 10))
    }

    pub fn compact_card_frame(&self) -> egui::Frame {
        egui::Frame::new()
            .fill(self.card_bg)
            .stroke(Stroke::new(1.0, self.card_border))
            .corner_radius(CornerRadius::same(6))
            .inner_margin(Margin::symmetric(8, 6))
    }

    pub fn banner_frame(&self, bg: Color32, border: Color32) -> egui::Frame {
        egui::Frame::new()
            .fill(bg)
            .stroke(Stroke::new(1.0, border))
            .corner_radius(CornerRadius::same(6))
            .inner_margin(Margin::symmetric(10, 6))
    }

    pub fn badge(&self, ui: &mut egui::Ui, text: &str, fg: Color32, bg: Color32) {
        egui::Frame::new()
            .fill(bg)
            .stroke(Stroke::new(1.0, fg.gamma_multiply(0.35)))
            .corner_radius(CornerRadius::same(4))
            .inner_margin(Margin::symmetric(6, 2))
            .show(ui, |ui| {
                ui.label(RichText::new(text).size(10.5).strong().color(fg));
            });
    }

    pub fn env_badge_colors(&self, r: u8, g: u8, b: u8) -> (Color32, Color32) {
        if self.is_dark {
            (
                Color32::from_rgb(r, g, b),
                Color32::from_rgb(r / 5, g / 5, b / 5),
            )
        } else {
            // Darken bright env colors for legibility on light badge tint
            let fg = Color32::from_rgb(
                (r as u16 * 65 / 100) as u8,
                (g as u16 * 65 / 100) as u8,
                (b as u16 * 65 / 100) as u8,
            );
            let bg = Color32::from_rgb(
                235u8.saturating_add((r / 12).min(20)),
                235u8.saturating_add((g / 12).min(20)),
                235u8.saturating_add((b / 12).min(20)),
            );
            (fg, bg)
        }
    }
}

pub fn dark_palette() -> Palette {
    Palette {
        is_dark: true,

        // Syntax highlighting
        keyword: Color32::from_rgb(56, 189, 248), // Sky 400
        function: Color32::from_rgb(192, 132, 252), // Purple 400
        type_: Color32::from_rgb(45, 212, 191),   // Teal 400
        string: Color32::from_rgb(163, 230, 53),  // Lime 400
        number: Color32::from_rgb(251, 146, 60),  // Orange 400
        comment: Color32::from_rgb(100, 116, 139), // Slate 500
        operator: Color32::from_rgb(148, 163, 184), // Slate 400
        identifier: Color32::from_rgb(226, 232, 240), // Slate 200
        punctuation: Color32::from_rgb(148, 163, 184),

        // Semantic accent & status
        accent: Color32::from_rgb(56, 189, 248),  // Sky 400
        accent_bg: Color32::from_rgb(15, 39, 68), // Deep sky tint
        accent_text: Color32::from_rgb(125, 211, 252), // Sky 300
        success: Color32::from_rgb(52, 211, 153), // Emerald 400
        success_bg: Color32::from_rgb(6, 78, 59), // Emerald 900
        warning: Color32::from_rgb(251, 191, 36), // Amber 400
        warning_bg: Color32::from_rgb(66, 46, 10), // Deep amber
        danger: Color32::from_rgb(248, 113, 113), // Red 400
        danger_bg: Color32::from_rgb(69, 10, 10), // Red 950
        purple: Color32::from_rgb(192, 132, 252), // Purple 400
        purple_bg: Color32::from_rgb(59, 7, 100), // Purple 950

        // Surfaces & borders
        panel_bg: Color32::from_rgb(13, 18, 28),
        surface_bg: Color32::from_rgb(15, 21, 32),
        card_bg: Color32::from_rgb(19, 26, 40),
        card_hover_bg: Color32::from_rgb(26, 36, 54),
        card_border: Color32::from_rgb(35, 46, 68),
        border_subtle: Color32::from_rgb(28, 37, 56),
        border_active: Color32::from_rgb(56, 189, 248),

        // Text
        text_primary: Color32::from_rgb(241, 245, 249), // Slate 100
        text_secondary: Color32::from_rgb(203, 213, 225), // Slate 300
        muted_text: Color32::from_rgb(148, 163, 184),   // Slate 400

        // Chrome, tabs & grid
        tab_active_bg: Color32::from_rgb(15, 39, 68),
        tab_inactive_bg: Color32::from_rgb(17, 23, 35),
        badge_bg: Color32::from_rgb(24, 34, 52),
        null_badge_bg: Color32::from_rgb(24, 32, 48),
        row_selected_bg: Color32::from_rgba_unmultiplied(14, 116, 144, 55),
        row_delete_bg: Color32::from_rgba_unmultiplied(185, 28, 28, 65),
        cell_edited_bg: Color32::from_rgba_unmultiplied(217, 119, 6, 55),
    }
}

pub fn light_palette() -> Palette {
    Palette {
        is_dark: false,

        // Syntax highlighting (WCAG AA on light surfaces)
        keyword: Color32::from_rgb(3, 105, 161),   // Sky 700
        function: Color32::from_rgb(109, 40, 217), // Violet 700
        type_: Color32::from_rgb(15, 118, 110),    // Teal 700
        string: Color32::from_rgb(21, 128, 61),    // Green 700
        number: Color32::from_rgb(194, 65, 12),    // Orange 700
        comment: Color32::from_rgb(100, 116, 139), // Slate 500
        operator: Color32::from_rgb(71, 85, 105),  // Slate 600
        identifier: Color32::from_rgb(15, 23, 42), // Slate 900
        punctuation: Color32::from_rgb(71, 85, 105),

        // Semantic accent & status (high contrast on light surfaces)
        accent: Color32::from_rgb(2, 132, 199),      // Sky 600
        accent_bg: Color32::from_rgb(224, 242, 254), // Sky 100
        accent_text: Color32::from_rgb(3, 105, 161), // Sky 700
        success: Color32::from_rgb(21, 128, 61),     // Green 700
        success_bg: Color32::from_rgb(220, 252, 231), // Green 100
        warning: Color32::from_rgb(161, 98, 7),      // Yellow 700
        warning_bg: Color32::from_rgb(254, 249, 195), // Yellow 100
        danger: Color32::from_rgb(185, 28, 28),      // Red 700
        danger_bg: Color32::from_rgb(254, 226, 226), // Red 100
        purple: Color32::from_rgb(126, 34, 206),     // Purple 700
        purple_bg: Color32::from_rgb(243, 232, 255), // Purple 100

        // Surfaces & borders
        panel_bg: Color32::from_rgb(248, 250, 252), // Slate 50
        surface_bg: Color32::from_rgb(241, 245, 249), // Slate 100
        card_bg: Color32::from_rgb(255, 255, 255),  // White
        card_hover_bg: Color32::from_rgb(241, 245, 249),
        card_border: Color32::from_rgb(203, 213, 225), // Slate 300
        border_subtle: Color32::from_rgb(226, 232, 240), // Slate 200
        border_active: Color32::from_rgb(2, 132, 199), // Sky 600

        // Text
        text_primary: Color32::from_rgb(15, 23, 42), // Slate 900
        text_secondary: Color32::from_rgb(51, 65, 85), // Slate 700
        muted_text: Color32::from_rgb(100, 116, 139), // Slate 500

        // Chrome, tabs & grid
        tab_active_bg: Color32::from_rgb(224, 242, 254), // Sky 100
        tab_inactive_bg: Color32::from_rgb(241, 245, 249), // Slate 100
        badge_bg: Color32::from_rgb(226, 232, 240),      // Slate 200
        null_badge_bg: Color32::from_rgb(226, 232, 240),
        row_selected_bg: Color32::from_rgba_unmultiplied(14, 165, 233, 38),
        row_delete_bg: Color32::from_rgba_unmultiplied(220, 38, 38, 35),
        cell_edited_bg: Color32::from_rgba_unmultiplied(234, 179, 8, 55),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn relative_luminance(c: Color32) -> f32 {
        let to_lin = |v: u8| {
            let s = v as f32 / 255.0;
            if s <= 0.03928 {
                s / 12.92
            } else {
                ((s + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * to_lin(c.r()) + 0.7152 * to_lin(c.g()) + 0.0722 * to_lin(c.b())
    }

    fn contrast_ratio(a: Color32, b: Color32) -> f32 {
        let la = relative_luminance(a);
        let lb = relative_luminance(b);
        let (hi, lo) = if la > lb { (la, lb) } else { (lb, la) };
        (hi + 0.05) / (lo + 0.05)
    }

    #[test]
    fn dark_and_light_palettes_have_accessible_text_contrast() {
        for pal in [dark_palette(), light_palette()] {
            assert!(
                contrast_ratio(pal.text_primary, pal.card_bg) >= 7.0,
                "text_primary contrast on card_bg should be >= 7:1 (is_dark={})",
                pal.is_dark
            );
            assert!(
                contrast_ratio(pal.text_secondary, pal.card_bg) >= 4.5,
                "text_secondary contrast on card_bg should be >= 4.5:1 (is_dark={})",
                pal.is_dark
            );
            assert!(
                contrast_ratio(pal.accent_text, pal.card_bg) >= 4.5,
                "accent_text contrast on card_bg should be >= 4.5:1 (is_dark={})",
                pal.is_dark
            );
            assert!(
                contrast_ratio(pal.danger, pal.danger_bg) >= 4.5,
                "danger contrast on danger_bg should be >= 4.5:1 (is_dark={})",
                pal.is_dark
            );
            assert!(
                contrast_ratio(pal.warning, pal.warning_bg) >= 4.5,
                "warning contrast on warning_bg should be >= 4.5:1 (is_dark={})",
                pal.is_dark
            );
            assert!(
                contrast_ratio(pal.success, pal.success_bg) >= 4.5,
                "success contrast on success_bg should be >= 4.5:1 (is_dark={})",
                pal.is_dark
            );
        }
    }
}
