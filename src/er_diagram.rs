use crate::db::DatabaseSchema;
use egui::epaint::CubicBezierShape;
use egui::{Color32, FontId, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2};
use std::collections::HashMap;

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ErCanvasState {
    pub pan_offset: Vec2,
    pub zoom: f32,
    pub node_positions: HashMap<String, Pos2>,
    pub dragged_node: Option<String>,
    pub hovered_node: Option<String>,
    pub hovered_fk: Option<String>,
    pub last_db_name: String,
}

impl Default for ErCanvasState {
    fn default() -> Self {
        Self {
            pan_offset: Vec2::ZERO,
            zoom: 1.0,
            node_positions: HashMap::new(),
            dragged_node: None,
            hovered_node: None,
            hovered_fk: None,
            last_db_name: String::new(),
        }
    }
}

#[allow(dead_code)]
impl ErCanvasState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn auto_layout(&mut self, schema: &DatabaseSchema) {
        let cols_per_row = 3;
        let x_spacing = 290.0;
        let y_spacing = 240.0;
        let start_x = 40.0;
        let start_y = 30.0;

        self.node_positions.clear();
        for (i, table) in schema.tables.iter().enumerate() {
            let col = (i % cols_per_row) as f32;
            let row = (i / cols_per_row) as f32;
            let pos = Pos2::new(start_x + col * x_spacing, start_y + row * y_spacing);
            self.node_positions.insert(table.name.clone(), pos);
        }
    }

    pub fn reset_zoom(&mut self) {
        self.zoom = 1.0;
        self.pan_offset = Vec2::ZERO;
    }

    pub fn show(&mut self, ui: &mut Ui, schema: &DatabaseSchema) -> Response {
        // Auto-layout on database change or when first loaded
        if self.last_db_name != schema.name || self.node_positions.is_empty() {
            self.last_db_name = schema.name.clone();
            self.auto_layout(schema);
        }

        // Top Toolbar
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(format!(
                    "ER Diagram: `{}` ({} tables)",
                    schema.name,
                    schema.tables.len()
                ))
                .strong()
                .color(Color32::from_rgb(56, 189, 248)),
            );
            ui.separator();

            if ui
                .button("Auto-Layout")
                .on_hover_text("Rearrange table nodes into a clean grid")
                .clicked()
            {
                self.auto_layout(schema);
            }
            if ui.button("Reset Zoom").clicked() {
                self.reset_zoom();
            }
            if ui.button("+").on_hover_text("Zoom in").clicked() {
                self.zoom = (self.zoom * 1.15).min(2.5);
            }
            if ui.button("-").on_hover_text("Zoom out").clicked() {
                self.zoom = (self.zoom / 1.15).max(0.35);
            }
            ui.label(
                egui::RichText::new(format!("{:.0}%", self.zoom * 100.0))
                    .small()
                    .color(Color32::from_rgb(148, 163, 184)),
            );

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(
                    egui::RichText::new(
                        "Tip: Drag nodes to move • Drag background to pan • Scroll to zoom",
                    )
                    .weak()
                    .small(),
                );
            });
        });
        ui.separator();

        let available_size = ui.available_size();
        let (response, painter) = ui.allocate_painter(available_size, Sense::click_and_drag());
        let canvas_rect = response.rect;
        let origin = canvas_rect.min;

        // Mouse pan & zoom
        if response.hovered() {
            let scroll_delta = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll_delta != 0.0 {
                let zoom_factor = if scroll_delta > 0.0 { 1.08 } else { 0.92 };
                let new_zoom = (self.zoom * zoom_factor).clamp(0.35, 2.5);
                if let Some(mouse_pos) = response.hover_pos() {
                    let mouse_offset = mouse_pos - origin - self.pan_offset;
                    self.pan_offset += mouse_offset * (1.0 - new_zoom / self.zoom);
                }
                self.zoom = new_zoom;
            }
        }

        // Background canvas dragging (pan)
        if response.dragged() && self.dragged_node.is_none() {
            self.pan_offset += response.drag_delta();
        }

        // Draw background grid dots
        let grid_spacing = 30.0 * self.zoom;
        if grid_spacing > 12.0 {
            let offset_x = (origin.x + self.pan_offset.x) % grid_spacing;
            let offset_y = (origin.y + self.pan_offset.y) % grid_spacing;
            let mut x = origin.x + offset_x;
            while x < canvas_rect.max.x {
                let mut y = origin.y + offset_y;
                while y < canvas_rect.max.y {
                    painter.circle_filled(
                        Pos2::new(x, y),
                        1.0,
                        Color32::from_rgba_unmultiplied(100, 116, 139, 45),
                    );
                    y += grid_spacing;
                }
                x += grid_spacing;
            }
        }

        let node_width = 220.0 * self.zoom;
        let header_height = 28.0 * self.zoom;
        let row_height = 18.0 * self.zoom;
        let font_title = FontId::proportional(12.0 * self.zoom.clamp(0.7, 1.6));
        let font_col = FontId::monospace(10.5 * self.zoom.clamp(0.7, 1.5));

        // Precompute screen rectangles for each table and column anchor points
        let mut table_rects: HashMap<String, Rect> = HashMap::new();
        let mut col_anchor_y: HashMap<(String, String), f32> = HashMap::new();

        for table in &schema.tables {
            let canvas_pos = self
                .node_positions
                .get(&table.name)
                .cloned()
                .unwrap_or(Pos2::new(50.0, 50.0));
            let screen_top_left = origin + self.pan_offset + canvas_pos.to_vec2() * self.zoom;

            let visible_cols = table.columns.len().min(12);
            let node_height = header_height + (visible_cols as f32 * row_height) + 12.0 * self.zoom;
            let rect = Rect::from_min_size(screen_top_left, Vec2::new(node_width, node_height));
            table_rects.insert(table.name.clone(), rect);

            for (idx, col) in table.columns.iter().take(visible_cols).enumerate() {
                let y = screen_top_left.y
                    + header_height
                    + (idx as f32 + 0.5) * row_height
                    + 4.0 * self.zoom;
                col_anchor_y.insert((table.name.clone(), col.name.clone()), y);
            }
        }

        // Handle dragging node boxes
        if response.drag_started()
            && let Some(mouse_pos) = response.hover_pos()
        {
            for (tbl_name, rect) in &table_rects {
                if rect.contains(mouse_pos) {
                    self.dragged_node = Some(tbl_name.clone());
                    break;
                }
            }
        }

        if response.dragged()
            && let Some(dragged) = &self.dragged_node
        {
            let delta = response.drag_delta() / self.zoom;
            if let Some(pos) = self.node_positions.get_mut(dragged) {
                *pos += delta;
            }
        }

        if response.drag_stopped() {
            self.dragged_node = None;
        }

        // Hover tracking
        self.hovered_node = None;
        if let Some(mouse_pos) = response.hover_pos() {
            for (tbl_name, rect) in &table_rects {
                if rect.contains(mouse_pos) {
                    self.hovered_node = Some(tbl_name.clone());
                    break;
                }
            }
        }

        // Draw Foreign Key Bézier Curves
        for table in &schema.tables {
            let Some(src_rect) = table_rects.get(&table.name).cloned() else {
                continue;
            };

            for fk in &table.foreign_keys {
                let Some(target_rect) = table_rects.get(&fk.referenced_table).cloned() else {
                    continue;
                };

                let src_y = col_anchor_y
                    .get(&(table.name.clone(), fk.column_name.clone()))
                    .copied()
                    .unwrap_or_else(|| src_rect.center().y);

                let target_y = col_anchor_y
                    .get(&(fk.referenced_table.clone(), fk.referenced_column.clone()))
                    .copied()
                    .unwrap_or_else(|| target_rect.center().y);

                // Determine whether target is to the right or left of source
                let (p0, p3, p1, p2) = if src_rect.center().x <= target_rect.center().x {
                    let start = Pos2::new(src_rect.max.x, src_y);
                    let end = Pos2::new(target_rect.min.x, target_y);
                    let dx = ((end.x - start.x).abs() * 0.5).max(30.0 * self.zoom);
                    let c1 = Pos2::new(start.x + dx, start.y);
                    let c2 = Pos2::new(end.x - dx, end.y);
                    (start, end, c1, c2)
                } else {
                    let start = Pos2::new(src_rect.min.x, src_y);
                    let end = Pos2::new(target_rect.max.x, target_y);
                    let dx = ((start.x - end.x).abs() * 0.5).max(30.0 * self.zoom);
                    let c1 = Pos2::new(start.x - dx, start.y);
                    let c2 = Pos2::new(end.x + dx, end.y);
                    (start, end, c1, c2)
                };

                let is_hovered = self.hovered_node.as_ref() == Some(&table.name)
                    || self.hovered_node.as_ref() == Some(&fk.referenced_table);

                let (curve_color, stroke_width) = if is_hovered {
                    (
                        Color32::from_rgb(236, 72, 153),
                        2.8 * self.zoom.clamp(0.8, 1.5),
                    )
                } else {
                    (
                        Color32::from_rgba_unmultiplied(168, 85, 247, 180),
                        1.6 * self.zoom.clamp(0.8, 1.5),
                    )
                };

                // Draw Cubic Bézier Curve
                painter.add(CubicBezierShape::from_points_stroke(
                    [p0, p1, p2, p3],
                    false,
                    Color32::TRANSPARENT,
                    Stroke::new(stroke_width, curve_color),
                ));

                // Draw Arrowhead at target point
                let dir = (p3 - p2).normalized();
                let normal = Vec2::new(-dir.y, dir.x);
                let arrow_len = 8.0 * self.zoom.clamp(0.6, 1.4);
                let p_left = p3 - dir * arrow_len + normal * (arrow_len * 0.45);
                let p_right = p3 - dir * arrow_len - normal * (arrow_len * 0.45);
                painter.add(egui::Shape::convex_polygon(
                    vec![p3, p_left, p_right],
                    curve_color,
                    Stroke::NONE,
                ));
            }
        }

        // Draw Table Node Boxes
        for table in &schema.tables {
            let Some(rect) = table_rects.get(&table.name).cloned() else {
                continue;
            };

            let is_hovered = self.hovered_node.as_ref() == Some(&table.name);
            let is_dragged = self.dragged_node.as_ref() == Some(&table.name);

            let border_stroke = if is_dragged {
                Stroke::new(2.0, Color32::from_rgb(236, 72, 153))
            } else if is_hovered {
                Stroke::new(2.0, Color32::from_rgb(56, 189, 248))
            } else {
                Stroke::new(1.0, Color32::from_rgb(71, 85, 105))
            };

            // Node body background
            painter.rect_filled(rect, 6.0 * self.zoom, Color32::from_rgb(15, 23, 42));
            painter.rect_stroke(rect, 6.0 * self.zoom, border_stroke, StrokeKind::Middle);

            // Node header
            let header_rect = Rect::from_min_size(rect.min, Vec2::new(rect.width(), header_height));
            let header_fill = if is_hovered {
                Color32::from_rgb(30, 58, 138) // Blue tint
            } else {
                Color32::from_rgb(30, 41, 59) // Slate 800
            };
            painter.rect_filled(
                header_rect,
                egui::CornerRadius {
                    nw: (6.0 * self.zoom) as u8,
                    ne: (6.0 * self.zoom) as u8,
                    sw: 0,
                    se: 0,
                },
                header_fill,
            );

            // Table icon and title
            let title_text = table.name.clone();
            painter.text(
                Pos2::new(header_rect.min.x + 8.0 * self.zoom, header_rect.center().y),
                egui::Align2::LEFT_CENTER,
                title_text,
                font_title.clone(),
                Color32::from_rgb(241, 245, 249),
            );

            // Column rows
            let visible_cols = table.columns.len().min(12);
            for (idx, col) in table.columns.iter().take(visible_cols).enumerate() {
                let y =
                    rect.min.y + header_height + (idx as f32 + 0.5) * row_height + 4.0 * self.zoom;
                let text_pos = Pos2::new(rect.min.x + 8.0 * self.zoom, y);

                let badge = if col.is_primary_key {
                    "[PK] "
                } else if table.foreign_keys.iter().any(|f| f.column_name == col.name) {
                    "[FK] "
                } else {
                    "     "
                };

                let col_name_color = if col.is_primary_key {
                    Color32::from_rgb(251, 191, 36) // Amber
                } else {
                    Color32::from_rgb(226, 232, 240)
                };

                let line_text = format!("{badge}{}", col.name);
                painter.text(
                    text_pos,
                    egui::Align2::LEFT_CENTER,
                    line_text,
                    font_col.clone(),
                    col_name_color,
                );

                // Right-aligned column type
                let type_pos = Pos2::new(rect.max.x - 8.0 * self.zoom, y);
                painter.text(
                    type_pos,
                    egui::Align2::RIGHT_CENTER,
                    &col.data_type,
                    font_col.clone(),
                    Color32::from_rgb(148, 163, 184),
                );
            }

            if table.columns.len() > 12 {
                let y = rect.min.y + header_height + (12.0 * row_height) + 4.0 * self.zoom;
                painter.text(
                    Pos2::new(rect.center().x, y),
                    egui::Align2::CENTER_CENTER,
                    format!("+ {} more columns", table.columns.len() - 12),
                    font_col.clone(),
                    Color32::from_rgb(100, 116, 139),
                );
            }
        }

        response
    }
}
