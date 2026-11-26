use super::*;

impl MysqlRustyApp {
    pub(crate) fn parse_hex_color(&self, hex: &str) -> Color32 {
        let clean = hex.trim_start_matches('#');
        if clean.len() == 6
            && clean.is_ascii()
            && let (Ok(r), Ok(g), Ok(b)) = (
                u8::from_str_radix(&clean[0..2], 16),
                u8::from_str_radix(&clean[2..4], 16),
                u8::from_str_radix(&clean[4..6], 16),
            )
        {
            return Color32::from_rgb(r, g, b);
        }
        Color32::from_rgb(56, 189, 248)
    }
}

pub(crate) fn configure_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let prop_candidates = [
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
        "/System/Library/Fonts/Supplemental/Arial.ttf",
    ];
    for path in prop_candidates {
        if let Ok(font_bytes) = std::fs::read(path) {
            fonts.font_data.insert(
                "system_prop_fallback".to_owned(),
                std::sync::Arc::new(egui::FontData::from_owned(font_bytes)),
            );
            fonts
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default()
                .push("system_prop_fallback".to_owned());
            break;
        }
    }
    let mono_candidates = [
        "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
        "/usr/share/fonts/truetype/liberation/LiberationMono-Regular.ttf",
        "/System/Library/Fonts/Menlo.ttc",
    ];
    for path in mono_candidates {
        if let Ok(font_bytes) = std::fs::read(path) {
            fonts.font_data.insert(
                "system_mono_fallback".to_owned(),
                std::sync::Arc::new(egui::FontData::from_owned(font_bytes)),
            );
            fonts
                .families
                .entry(egui::FontFamily::Monospace)
                .or_default()
                .push("system_mono_fallback".to_owned());
            break;
        }
    }
    ctx.set_fonts(fonts);
}

fn apply_density_spacing(style: &mut egui::Style, density: crate::config::UiDensity) {
    match density {
        crate::config::UiDensity::Compact => {
            style.spacing.button_padding = egui::vec2(7.0, 3.0);
            style.spacing.item_spacing = egui::vec2(6.0, 4.0);
            style.spacing.window_margin = egui::Margin::same(10);
            style.spacing.interact_size = egui::vec2(24.0, 20.0);
            style.spacing.combo_width = 165.0;
        }
        crate::config::UiDensity::Comfortable => {
            style.spacing.button_padding = egui::vec2(10.0, 5.0);
            style.spacing.item_spacing = egui::vec2(8.0, 6.0);
            style.spacing.window_margin = egui::Margin::same(14);
            style.spacing.interact_size = egui::vec2(28.0, 24.0);
            style.spacing.combo_width = 180.0;
        }
        crate::config::UiDensity::Spacious => {
            style.spacing.button_padding = egui::vec2(13.0, 7.0);
            style.spacing.item_spacing = egui::vec2(10.0, 8.0);
            style.spacing.window_margin = egui::Margin::same(18);
            style.spacing.interact_size = egui::vec2(32.0, 28.0);
            style.spacing.combo_width = 195.0;
        }
    }
}

pub(crate) fn configure_theme_visuals(
    ctx: &egui::Context,
    pal: crate::theme::Palette,
    density: crate::config::UiDensity,
) {
    let mut visuals = if pal.is_dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };

    visuals.window_fill = pal.window_bg;
    visuals.panel_fill = pal.panel_bg;
    visuals.extreme_bg_color = pal.editor_bg;
    visuals.faint_bg_color = pal.surface_bg;
    visuals.selection.bg_fill = pal.accent_bg;
    visuals.selection.stroke = Stroke::new(1.0, pal.border_active);

    visuals.window_corner_radius = egui::CornerRadius::same(10);
    visuals.menu_corner_radius = egui::CornerRadius::same(8);
    visuals.window_stroke = Stroke::new(1.0, pal.card_border);

    let subtle_border = Stroke::new(1.0, pal.card_border);
    let hover_border = Stroke::new(1.0, pal.border_active);
    let active_border = Stroke::new(1.2, pal.accent);

    visuals.widgets.noninteractive.corner_radius = egui::CornerRadius::same(6);
    visuals.widgets.noninteractive.bg_fill = pal.surface_bg;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, pal.border_subtle);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, pal.text_secondary);

    visuals.widgets.inactive.corner_radius = egui::CornerRadius::same(6);
    visuals.widgets.inactive.bg_fill = pal.card_bg;
    visuals.widgets.inactive.weak_bg_fill = pal.tab_inactive_bg;
    visuals.widgets.inactive.bg_stroke = subtle_border;
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, pal.text_primary);

    visuals.widgets.hovered.corner_radius = egui::CornerRadius::same(6);
    visuals.widgets.hovered.bg_fill = pal.card_hover_bg;
    visuals.widgets.hovered.weak_bg_fill = pal.card_hover_bg;
    visuals.widgets.hovered.bg_stroke = hover_border;
    visuals.widgets.hovered.fg_stroke = Stroke::new(
        1.0,
        if pal.is_dark {
            Color32::WHITE
        } else {
            pal.text_primary
        },
    );

    visuals.widgets.active.corner_radius = egui::CornerRadius::same(6);
    visuals.widgets.active.bg_fill = pal.accent_bg;
    visuals.widgets.active.weak_bg_fill = pal.accent_bg;
    visuals.widgets.active.bg_stroke = active_border;
    visuals.widgets.active.fg_stroke = Stroke::new(
        1.0,
        if pal.is_dark {
            Color32::WHITE
        } else {
            pal.text_primary
        },
    );

    visuals.widgets.open.corner_radius = egui::CornerRadius::same(6);
    visuals.widgets.open.bg_fill = pal.card_hover_bg;
    visuals.widgets.open.bg_stroke = hover_border;

    let target = if pal.is_dark {
        egui::Theme::Dark
    } else {
        egui::Theme::Light
    };
    ctx.set_theme(target);
    ctx.set_visuals(visuals.clone());
    ctx.style_mut_of(target, |style| {
        style.visuals = visuals;
        apply_density_spacing(style, density);
    });
}

impl super::MysqlRustyApp {
    pub(crate) fn resolved_egui_theme(&self, ctx: &egui::Context) -> egui::Theme {
        match self.config.ui.theme {
            crate::config::ThemePreference::Dark => egui::Theme::Dark,
            crate::config::ThemePreference::Light => egui::Theme::Light,
            crate::config::ThemePreference::System => {
                ctx.system_theme().unwrap_or(egui::Theme::Dark)
            }
        }
    }

    pub(crate) fn effective_palette(&self, ctx: &egui::Context) -> crate::theme::Palette {
        let is_dark = self.resolved_egui_theme(ctx) == egui::Theme::Dark;
        crate::theme::palette_for(is_dark, self.config.ui.dark_preset)
    }

    pub(crate) fn apply_ui_theme_and_scale(&mut self, ctx: &egui::Context) {
        let target_theme = self.resolved_egui_theme(ctx);
        let key = (
            target_theme,
            self.config.ui.dark_preset,
            self.config.ui.density,
        );
        if self.last_applied_theme != Some(key) {
            let pal = crate::theme::palette_for(
                target_theme == egui::Theme::Dark,
                self.config.ui.dark_preset,
            );
            configure_theme_visuals(ctx, pal, self.config.ui.density);
            self.last_applied_theme = Some(key);
        }
        let scale = self.config.ui.ui_scale.clamp(0.75, 2.0);
        if (ctx.zoom_factor() - scale).abs() > 0.001 {
            ctx.set_zoom_factor(scale);
        }
    }
}

pub(crate) fn format_uptime_secs(uptime_str: Option<&String>) -> String {
    let Some(s) = uptime_str else {
        return "—".to_string();
    };
    let Ok(secs) = s.parse::<u64>() else {
        return s.clone();
    };
    let days = secs / 86400;
    let hours = (secs % 86400) / 3600;
    let mins = (secs % 3600) / 60;
    let rem_secs = secs % 60;
    if days > 0 {
        format!("{days}d {hours}h {mins}m")
    } else if hours > 0 {
        format!("{hours}h {mins}m {rem_secs}s")
    } else {
        format!("{mins}m {rem_secs}s")
    }
}

pub(crate) fn format_buffer_pool_bytes(bytes_str: Option<&String>) -> String {
    let Some(s) = bytes_str else {
        return "—".to_string();
    };
    let Ok(bytes) = s.parse::<u64>() else {
        return s.clone();
    };
    if bytes >= 1024 * 1024 * 1024 {
        format!("{:.2} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    } else if bytes >= 1024 * 1024 {
        format!("{:.2} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{} KB", bytes / 1024)
    }
}

pub fn sanitize_filename(name: &str) -> String {
    let trimmed = name.trim();
    let without_ext = trimmed.strip_suffix(".sql").unwrap_or(trimmed);
    let mut safe = String::new();
    for c in without_ext.chars() {
        if c.is_alphanumeric() || c == '_' || c == '-' {
            safe.push(c);
        } else if (c.is_whitespace()
            || c == '/'
            || c == '\\'
            || c == ':'
            || c == '*'
            || c == '?'
            || c == '"'
            || c == '<'
            || c == '>'
            || c == '|')
            && !safe.ends_with('_')
        {
            safe.push('_');
        }
    }
    let res = safe.trim_matches(|c| c == '_' || c == '.').to_string();
    if res.is_empty() {
        "query".to_string()
    } else {
        res
    }
}

#[cfg(test)]
mod tests {
    use crate::config::AppConfig;
    #[test]
    fn test_sanitize_filename_and_queries_dir() {
        use crate::app::sanitize_filename;

        assert_eq!(sanitize_filename("My Custom Query"), "My_Custom_Query");
        assert_eq!(
            sanitize_filename("test/unsafe*name?.sql"),
            "test_unsafe_name"
        );
        assert_eq!(sanitize_filename(""), "query");
        assert_eq!(
            sanitize_filename("  spaces_and_tabs  .sql"),
            "spaces_and_tabs"
        );

        let qdir = AppConfig::queries_dir();
        assert!(qdir.exists() || qdir.to_str().is_some());
    }
}
