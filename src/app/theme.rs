use super::*;

impl MysqlRustyApp {
    pub(crate) fn parse_hex_color(&self, hex: &str) -> Color32 {
        let clean = hex.trim_start_matches('#');
        if clean.len() == 6
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

pub(crate) fn configure_modern_dark_theme(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.window_fill = Color32::from_rgb(11, 15, 23); // Obsidian 950
    visuals.panel_fill = Color32::from_rgb(13, 18, 28); // Deep slate surface
    visuals.extreme_bg_color = Color32::from_rgb(8, 11, 17); // Editor background
    visuals.faint_bg_color = Color32::from_rgb(20, 26, 40); // Faint background
    visuals.selection.bg_fill = Color32::from_rgb(14, 116, 144); // Cyan selection
    visuals.selection.stroke = Stroke::new(1.0, Color32::from_rgb(56, 189, 248));

    // Corner radii
    visuals.window_corner_radius = egui::CornerRadius::same(10);
    visuals.menu_corner_radius = egui::CornerRadius::same(8);

    // Widget styling (Buttons, inputs, dropdowns)
    let subtle_border = Stroke::new(1.0, Color32::from_rgb(33, 43, 63));
    let hover_border = Stroke::new(1.0, Color32::from_rgb(56, 189, 248));
    let active_border = Stroke::new(1.0, Color32::from_rgb(14, 165, 233));

    visuals.widgets.noninteractive.corner_radius = egui::CornerRadius::same(6);
    visuals.widgets.noninteractive.bg_fill = Color32::from_rgb(15, 21, 32);
    visuals.widgets.noninteractive.bg_stroke = subtle_border;
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, Color32::from_rgb(148, 163, 184));

    visuals.widgets.inactive.corner_radius = egui::CornerRadius::same(6);
    visuals.widgets.inactive.bg_fill = Color32::from_rgb(20, 27, 41);
    visuals.widgets.inactive.weak_bg_fill = Color32::from_rgb(16, 22, 34);
    visuals.widgets.inactive.bg_stroke = subtle_border;
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, Color32::from_rgb(226, 232, 240));

    visuals.widgets.hovered.corner_radius = egui::CornerRadius::same(6);
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(30, 41, 62);
    visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(25, 34, 52);
    visuals.widgets.hovered.bg_stroke = hover_border;
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, Color32::WHITE);

    visuals.widgets.active.corner_radius = egui::CornerRadius::same(6);
    visuals.widgets.active.bg_fill = Color32::from_rgb(37, 51, 77);
    visuals.widgets.active.weak_bg_fill = Color32::from_rgb(32, 44, 66);
    visuals.widgets.active.bg_stroke = active_border;
    visuals.widgets.active.fg_stroke = Stroke::new(1.0, Color32::WHITE);

    visuals.widgets.open.corner_radius = egui::CornerRadius::same(6);
    visuals.widgets.open.bg_fill = Color32::from_rgb(23, 31, 47);
    visuals.widgets.open.bg_stroke = hover_border;

    ctx.set_visuals(visuals);

    // Spacing and ergonomic button sizes
    ctx.style_mut_of(egui::Theme::Dark, |style| {
        style.spacing.button_padding = egui::vec2(10.0, 5.0);
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.interact_size = egui::vec2(28.0, 24.0);
        style.spacing.combo_width = 180.0;
    });

    let mut fonts = egui::FontDefinitions::default();
    let candidate_fonts = [
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
    ];
    for path in candidate_fonts {
        if let Ok(font_bytes) = std::fs::read(path) {
            fonts.font_data.insert(
                "system_fallback".to_owned(),
                std::sync::Arc::new(egui::FontData::from_owned(font_bytes)),
            );
            fonts
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default()
                .push("system_fallback".to_owned());
            fonts
                .families
                .entry(egui::FontFamily::Monospace)
                .or_default()
                .push("system_fallback".to_owned());
            break;
        }
    }
    ctx.set_fonts(fonts);
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
