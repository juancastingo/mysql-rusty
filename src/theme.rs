use egui::Color32;

#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub struct Palette {
    pub keyword: Color32,
    pub function: Color32,
    pub type_: Color32,
    pub string: Color32,
    pub number: Color32,
    pub comment: Color32,
    pub operator: Color32,
    pub identifier: Color32,
    pub punctuation: Color32,
    pub success: Color32,
    pub warning: Color32,
    pub danger: Color32,
    pub muted_text: Color32,
    pub panel_bg: Color32,
    pub card_bg: Color32,
}

pub fn dark_palette() -> Palette {
    Palette {
        keyword: Color32::from_rgb(56, 189, 248),     // Sky blue
        function: Color32::from_rgb(192, 132, 252),   // Purple
        type_: Color32::from_rgb(45, 212, 191),       // Teal
        string: Color32::from_rgb(163, 230, 53),      // Lime green
        number: Color32::from_rgb(251, 146, 60),      // Orange
        comment: Color32::from_rgb(100, 116, 139),    // Slate gray
        operator: Color32::from_rgb(148, 163, 184),   // Muted slate
        identifier: Color32::from_rgb(226, 232, 240), // Light slate
        punctuation: Color32::from_rgb(148, 163, 184),
        success: Color32::from_rgb(34, 197, 94),
        warning: Color32::from_rgb(234, 179, 8),
        danger: Color32::from_rgb(239, 68, 68),
        muted_text: Color32::from_rgb(148, 163, 184),
        panel_bg: Color32::from_rgb(13, 18, 28),
        card_bg: Color32::from_rgb(20, 27, 41),
    }
}

pub fn light_palette() -> Palette {
    Palette {
        keyword: Color32::from_rgb(3, 105, 161), // Deep sky blue (WCAG AA on white)
        function: Color32::from_rgb(109, 40, 217), // Deep violet
        type_: Color32::from_rgb(15, 118, 110),  // Deep teal
        string: Color32::from_rgb(21, 128, 61),  // Forest green
        number: Color32::from_rgb(194, 65, 12),  // Burnt orange
        comment: Color32::from_rgb(100, 116, 139), // Slate 500
        operator: Color32::from_rgb(71, 85, 105), // Slate 600
        identifier: Color32::from_rgb(15, 23, 42), // Slate 900
        punctuation: Color32::from_rgb(71, 85, 105), // Slate 600
        success: Color32::from_rgb(21, 128, 61),
        warning: Color32::from_rgb(161, 98, 7),
        danger: Color32::from_rgb(185, 28, 28),
        muted_text: Color32::from_rgb(71, 85, 105),
        panel_bg: Color32::from_rgb(248, 250, 252),
        card_bg: Color32::from_rgb(241, 245, 249),
    }
}
