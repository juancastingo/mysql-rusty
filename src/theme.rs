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
    pub window_bg: Color32,
    pub panel_bg: Color32,
    pub editor_bg: Color32,
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
    pub row_hover_bg: Color32,
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

    pub fn pill_button(&self, text: impl Into<String>, selected: bool) -> egui::Button<'static> {
        let (fg, bg, border) = if selected {
            (self.accent_text, self.accent_bg, self.border_active)
        } else {
            (self.text_secondary, self.tab_inactive_bg, self.border_subtle)
        };
        let mut rt = RichText::new(text.into()).size(11.5).color(fg);
        if selected {
            rt = rt.strong();
        }
        egui::Button::new(rt)
            .fill(bg)
            .stroke(Stroke::new(1.0, border))
            .corner_radius(CornerRadius::same(12))
    }

    pub fn icon_toggle_button(
        &self,
        icon: impl Into<String>,
        active: bool,
    ) -> egui::Button<'static> {
        let (fg, bg, border) = if active {
            (self.accent_text, self.accent_bg, self.border_active)
        } else {
            (self.text_secondary, self.card_bg, self.card_border)
        };
        egui::Button::new(RichText::new(icon.into()).size(13.0).color(fg))
            .fill(bg)
            .stroke(Stroke::new(1.0, border))
            .corner_radius(CornerRadius::same(6))
    }

    pub fn section_header(&self, ui: &mut egui::Ui, title: &str) {
        ui.add_space(4.0);
        ui.label(
            RichText::new(title.to_uppercase())
                .size(10.5)
                .strong()
                .color(self.muted_text),
        );
        ui.add_space(2.0);
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

    pub fn micro_badge(&self, ui: &mut egui::Ui, text: &str, fg: Color32, bg: Color32) {
        egui::Frame::new()
            .fill(bg)
            .stroke(Stroke::new(1.0, fg.gamma_multiply(0.35)))
            .corner_radius(CornerRadius::same(3))
            .inner_margin(Margin::symmetric(4, 1))
            .show(ui, |ui| {
                ui.label(RichText::new(text).size(9.5).strong().monospace().color(fg));
            });
    }

    pub fn dialect_badge(&self, ui: &mut egui::Ui, dialect: crate::dialect::Dialect) {
        let (label, fg, bg) = match dialect {
            crate::dialect::Dialect::MySql => ("MySQL", self.accent_text, self.accent_bg),
            crate::dialect::Dialect::Postgres => ("PostgreSQL", self.purple, self.purple_bg),
            crate::dialect::Dialect::Sqlite => ("SQLite", self.success, self.success_bg),
        };
        self.badge(ui, label, fg, bg);
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

pub fn palette_for(is_dark: bool, preset: crate::config::DarkThemePreset) -> Palette {
    if !is_dark {
        return light_palette();
    }
    match preset {
        crate::config::DarkThemePreset::Obsidian => dark_palette(),
        crate::config::DarkThemePreset::Nord => nord_palette(),
        crate::config::DarkThemePreset::Dracula => dracula_palette(),
        crate::config::DarkThemePreset::Catppuccin => catppuccin_palette(),
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
        window_bg: Color32::from_rgb(11, 15, 23),
        panel_bg: Color32::from_rgb(13, 18, 28),
        editor_bg: Color32::from_rgb(8, 11, 17),
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
        row_hover_bg: Color32::from_rgba_unmultiplied(56, 189, 248, 22),
        row_delete_bg: Color32::from_rgba_unmultiplied(185, 28, 28, 65),
        cell_edited_bg: Color32::from_rgba_unmultiplied(217, 119, 6, 55),
    }
}

pub fn nord_palette() -> Palette {
    Palette {
        is_dark: true,

        // Syntax highlighting (Nord Frost & Aurora)
        keyword: Color32::from_rgb(129, 161, 193),  // Nord9
        function: Color32::from_rgb(136, 192, 208), // Nord8
        type_: Color32::from_rgb(143, 188, 187),    // Nord7
        string: Color32::from_rgb(163, 190, 140),   // Nord14
        number: Color32::from_rgb(208, 135, 112),   // Nord12
        comment: Color32::from_rgb(124, 136, 158),
        operator: Color32::from_rgb(180, 190, 208),
        identifier: Color32::from_rgb(229, 233, 240), // Nord5
        punctuation: Color32::from_rgb(180, 190, 208),

        // Semantic accent & status
        accent: Color32::from_rgb(136, 192, 208),      // Nord8
        accent_bg: Color32::from_rgb(44, 62, 78),
        accent_text: Color32::from_rgb(168, 218, 232),
        success: Color32::from_rgb(163, 190, 140),     // Nord14
        success_bg: Color32::from_rgb(40, 58, 44),
        warning: Color32::from_rgb(235, 203, 139),     // Nord13
        warning_bg: Color32::from_rgb(72, 60, 34),
        danger: Color32::from_rgb(235, 130, 140),
        danger_bg: Color32::from_rgb(76, 34, 40),
        purple: Color32::from_rgb(198, 160, 206),
        purple_bg: Color32::from_rgb(64, 46, 72),

        // Surfaces & borders (Nord Polar Night)
        window_bg: Color32::from_rgb(38, 43, 54),
        panel_bg: Color32::from_rgb(46, 52, 64),       // Nord0
        editor_bg: Color32::from_rgb(35, 40, 50),
        surface_bg: Color32::from_rgb(52, 59, 73),
        card_bg: Color32::from_rgb(59, 66, 82),        // Nord1
        card_hover_bg: Color32::from_rgb(67, 76, 94),  // Nord2
        card_border: Color32::from_rgb(76, 86, 106),   // Nord3
        border_subtle: Color32::from_rgb(67, 76, 94),
        border_active: Color32::from_rgb(136, 192, 208),

        // Text (Nord Snow Storm)
        text_primary: Color32::from_rgb(236, 239, 244), // Nord6
        text_secondary: Color32::from_rgb(216, 222, 233), // Nord4
        muted_text: Color32::from_rgb(152, 164, 184),

        // Chrome, tabs & grid
        tab_active_bg: Color32::from_rgb(44, 62, 78),
        tab_inactive_bg: Color32::from_rgb(50, 56, 70),
        badge_bg: Color32::from_rgb(67, 76, 94),
        null_badge_bg: Color32::from_rgb(64, 72, 88),
        row_selected_bg: Color32::from_rgba_unmultiplied(136, 192, 208, 48),
        row_hover_bg: Color32::from_rgba_unmultiplied(136, 192, 208, 20),
        row_delete_bg: Color32::from_rgba_unmultiplied(191, 97, 106, 65),
        cell_edited_bg: Color32::from_rgba_unmultiplied(235, 203, 139, 48),
    }
}

pub fn dracula_palette() -> Palette {
    Palette {
        is_dark: true,

        // Syntax highlighting (Dracula)
        keyword: Color32::from_rgb(255, 121, 198),  // Pink
        function: Color32::from_rgb(80, 250, 123),  // Green
        type_: Color32::from_rgb(139, 233, 253),    // Cyan
        string: Color32::from_rgb(241, 250, 140),   // Yellow
        number: Color32::from_rgb(189, 147, 249),   // Purple
        comment: Color32::from_rgb(122, 134, 174),
        operator: Color32::from_rgb(255, 121, 198),
        identifier: Color32::from_rgb(248, 248, 242),
        punctuation: Color32::from_rgb(188, 194, 214),

        // Semantic accent & status
        accent: Color32::from_rgb(189, 147, 249),      // Purple
        accent_bg: Color32::from_rgb(56, 44, 82),
        accent_text: Color32::from_rgb(212, 184, 255),
        success: Color32::from_rgb(80, 250, 123),
        success_bg: Color32::from_rgb(22, 64, 38),
        warning: Color32::from_rgb(255, 184, 108),
        warning_bg: Color32::from_rgb(74, 50, 24),
        danger: Color32::from_rgb(255, 110, 110),
        danger_bg: Color32::from_rgb(78, 22, 28),
        purple: Color32::from_rgb(255, 121, 198),
        purple_bg: Color32::from_rgb(74, 28, 58),

        // Surfaces & borders
        window_bg: Color32::from_rgb(28, 29, 38),
        panel_bg: Color32::from_rgb(33, 34, 44),
        editor_bg: Color32::from_rgb(24, 25, 34),
        surface_bg: Color32::from_rgb(37, 39, 50),
        card_bg: Color32::from_rgb(40, 42, 54),
        card_hover_bg: Color32::from_rgb(50, 53, 68),
        card_border: Color32::from_rgb(68, 71, 90),
        border_subtle: Color32::from_rgb(55, 58, 74),
        border_active: Color32::from_rgb(189, 147, 249),

        // Text
        text_primary: Color32::from_rgb(248, 248, 242),
        text_secondary: Color32::from_rgb(214, 218, 232),
        muted_text: Color32::from_rgb(154, 162, 194),

        // Chrome, tabs & grid
        tab_active_bg: Color32::from_rgb(56, 44, 82),
        tab_inactive_bg: Color32::from_rgb(36, 38, 49),
        badge_bg: Color32::from_rgb(54, 57, 74),
        null_badge_bg: Color32::from_rgb(50, 53, 68),
        row_selected_bg: Color32::from_rgba_unmultiplied(189, 147, 249, 48),
        row_hover_bg: Color32::from_rgba_unmultiplied(189, 147, 249, 22),
        row_delete_bg: Color32::from_rgba_unmultiplied(255, 85, 85, 60),
        cell_edited_bg: Color32::from_rgba_unmultiplied(255, 184, 108, 48),
    }
}

pub fn catppuccin_palette() -> Palette {
    Palette {
        is_dark: true,

        // Syntax highlighting (Catppuccin Mocha)
        keyword: Color32::from_rgb(203, 166, 247),  // Mauve
        function: Color32::from_rgb(137, 180, 250), // Blue
        type_: Color32::from_rgb(148, 226, 213),    // Teal
        string: Color32::from_rgb(166, 227, 161),   // Green
        number: Color32::from_rgb(250, 179, 135),   // Peach
        comment: Color32::from_rgb(127, 132, 156),  // Overlay1
        operator: Color32::from_rgb(137, 220, 235), // Sky
        identifier: Color32::from_rgb(205, 214, 244), // Text
        punctuation: Color32::from_rgb(147, 153, 178), // Overlay2

        // Semantic accent & status
        accent: Color32::from_rgb(137, 180, 250),      // Blue
        accent_bg: Color32::from_rgb(36, 48, 74),
        accent_text: Color32::from_rgb(166, 200, 255),
        success: Color32::from_rgb(166, 227, 161),     // Green
        success_bg: Color32::from_rgb(30, 58, 40),
        warning: Color32::from_rgb(249, 226, 175),     // Yellow
        warning_bg: Color32::from_rgb(68, 56, 30),
        danger: Color32::from_rgb(243, 139, 168),      // Red
        danger_bg: Color32::from_rgb(72, 28, 42),
        purple: Color32::from_rgb(203, 166, 247),      // Mauve
        purple_bg: Color32::from_rgb(56, 38, 76),

        // Surfaces & borders (Catppuccin Mocha Crust/Mantle/Base/Surface0)
        window_bg: Color32::from_rgb(17, 17, 27),      // Crust
        panel_bg: Color32::from_rgb(24, 24, 37),       // Mantle
        editor_bg: Color32::from_rgb(17, 17, 27),      // Crust
        surface_bg: Color32::from_rgb(27, 27, 42),
        card_bg: Color32::from_rgb(30, 30, 46),        // Base
        card_hover_bg: Color32::from_rgb(49, 50, 68),  // Surface0
        card_border: Color32::from_rgb(69, 71, 90),    // Surface1
        border_subtle: Color32::from_rgb(49, 50, 68),  // Surface0
        border_active: Color32::from_rgb(137, 180, 250),

        // Text
        text_primary: Color32::from_rgb(205, 214, 244), // Text
        text_secondary: Color32::from_rgb(186, 194, 222), // Subtext1
        muted_text: Color32::from_rgb(147, 153, 178),   // Overlay2

        // Chrome, tabs & grid
        tab_active_bg: Color32::from_rgb(36, 48, 74),
        tab_inactive_bg: Color32::from_rgb(26, 26, 40),
        badge_bg: Color32::from_rgb(49, 50, 68),
        null_badge_bg: Color32::from_rgb(44, 46, 62),
        row_selected_bg: Color32::from_rgba_unmultiplied(137, 180, 250, 48),
        row_hover_bg: Color32::from_rgba_unmultiplied(137, 180, 250, 22),
        row_delete_bg: Color32::from_rgba_unmultiplied(243, 139, 168, 55),
        cell_edited_bg: Color32::from_rgba_unmultiplied(249, 226, 175, 45),
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
        window_bg: Color32::from_rgb(255, 255, 255),
        panel_bg: Color32::from_rgb(248, 250, 252), // Slate 50
        editor_bg: Color32::from_rgb(255, 255, 255),
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
        row_hover_bg: Color32::from_rgba_unmultiplied(2, 132, 199, 18),
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
    fn all_palettes_have_accessible_text_contrast() {
        let palettes = [
            ("Obsidian", dark_palette()),
            ("Nord", nord_palette()),
            ("Dracula", dracula_palette()),
            ("Catppuccin", catppuccin_palette()),
            ("Light", light_palette()),
        ];
        for (name, pal) in palettes {
            assert!(
                contrast_ratio(pal.text_primary, pal.card_bg) >= 7.0,
                "{name}: text_primary contrast on card_bg should be >= 7:1 (got {:.2})",
                contrast_ratio(pal.text_primary, pal.card_bg)
            );
            assert!(
                contrast_ratio(pal.text_secondary, pal.card_bg) >= 4.5,
                "{name}: text_secondary contrast on card_bg should be >= 4.5:1 (got {:.2})",
                contrast_ratio(pal.text_secondary, pal.card_bg)
            );
            assert!(
                contrast_ratio(pal.accent_text, pal.card_bg) >= 4.5,
                "{name}: accent_text contrast on card_bg should be >= 4.5:1 (got {:.2})",
                contrast_ratio(pal.accent_text, pal.card_bg)
            );
            assert!(
                contrast_ratio(pal.danger, pal.danger_bg) >= 4.5,
                "{name}: danger contrast on danger_bg should be >= 4.5:1 (got {:.2})",
                contrast_ratio(pal.danger, pal.danger_bg)
            );
            assert!(
                contrast_ratio(pal.warning, pal.warning_bg) >= 4.5,
                "{name}: warning contrast on warning_bg should be >= 4.5:1 (got {:.2})",
                contrast_ratio(pal.warning, pal.warning_bg)
            );
            assert!(
                contrast_ratio(pal.success, pal.success_bg) >= 4.5,
                "{name}: success contrast on success_bg should be >= 4.5:1 (got {:.2})",
                contrast_ratio(pal.success, pal.success_bg)
            );
        }
    }
}
