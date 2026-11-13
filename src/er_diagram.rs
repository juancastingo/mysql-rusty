use crate::db::DatabaseSchema;
use anyhow::{Context, Result, anyhow};
use egui::epaint::CubicBezierShape;
use egui::{Color32, FontId, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Ui, Vec2};
use std::collections::{HashMap, HashSet};

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
    pub export_status: Option<String>,
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
            export_status: None,
        }
    }
}

/// Sugiyama-style layered auto-layout:
/// 1. Builds parent -> child FK edges, breaking cycles via DFS back-edge removal.
/// 2. Assigns each table to its longest-path topological layer.
/// 3. Orders nodes within each layer using a barycenter heuristic to reduce crossings.
pub fn compute_auto_layout(schema: &DatabaseSchema) -> HashMap<String, Pos2> {
    let mut positions = HashMap::new();
    let n = schema.tables.len();
    if n == 0 {
        return positions;
    }

    let name_to_idx: HashMap<String, usize> = schema
        .tables
        .iter()
        .enumerate()
        .map(|(i, t)| (t.name.to_ascii_lowercase(), i))
        .collect();

    // Directed edges: parent -> child (referenced_table -> table)
    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (child_idx, tbl) in schema.tables.iter().enumerate() {
        for fk in &tbl.foreign_keys {
            if let Some(&parent_idx) = name_to_idx.get(&fk.referenced_table.to_ascii_lowercase())
                && parent_idx != child_idx
                && !adj[parent_idx].contains(&child_idx)
            {
                adj[parent_idx].push(child_idx);
            }
        }
    }

    // DFS cycle-breaking: remove back-edges
    let mut state = vec![0u8; n]; // 0 = unvisited, 1 = visiting, 2 = done
    let mut dag_adj: Vec<Vec<usize>> = vec![Vec::new(); n];
    fn dfs_dag(u: usize, adj: &[Vec<usize>], state: &mut [u8], dag_adj: &mut [Vec<usize>]) {
        state[u] = 1;
        for &v in &adj[u] {
            if state[v] == 0 {
                dag_adj[u].push(v);
                dfs_dag(v, adj, state, dag_adj);
            } else if state[v] == 2 {
                dag_adj[u].push(v);
            }
            // state[v] == 1 is a back-edge; skip to break cycle
        }
        state[u] = 2;
    }
    for i in 0..n {
        if state[i] == 0 {
            dfs_dag(i, &adj, &mut state, &mut dag_adj);
        }
    }

    // Longest-path layer assignment in DAG (Kahn's topological order)
    let mut indeg = vec![0usize; n];
    for edges in &dag_adj {
        for &v in edges {
            indeg[v] += 1;
        }
    }
    let mut queue: std::collections::VecDeque<usize> = (0..n).filter(|&i| indeg[i] == 0).collect();
    let mut layer = vec![0usize; n];
    let mut preds: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (u, edges) in dag_adj.iter().enumerate() {
        for &v in edges {
            preds[v].push(u);
        }
    }
    while let Some(u) = queue.pop_front() {
        for &v in &dag_adj[u] {
            layer[v] = layer[v].max(layer[u] + 1);
            indeg[v] -= 1;
            if indeg[v] == 0 {
                queue.push_back(v);
            }
        }
    }

    let max_layer = layer.iter().copied().max().unwrap_or(0);
    let mut layers: Vec<Vec<usize>> = vec![Vec::new(); max_layer + 1];
    for (idx, &l) in layer.iter().enumerate() {
        layers[l].push(idx);
    }

    // Barycenter ordering within each layer based on predecessors in earlier layers
    let mut order_in_layer: HashMap<usize, f32> = HashMap::new();
    for (pos, &node) in layers[0].iter().enumerate() {
        order_in_layer.insert(node, pos as f32);
    }
    for layer_nodes in layers.iter_mut().skip(1) {
        layer_nodes.sort_by(|&a, &b| {
            let bary = |node: usize| -> f32 {
                let ps = &preds[node];
                if ps.is_empty() {
                    return 0.0;
                }
                let sum: f32 = ps
                    .iter()
                    .filter_map(|p| order_in_layer.get(p).copied())
                    .sum();
                sum / (ps.len() as f32)
            };
            bary(a)
                .partial_cmp(&bary(b))
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| schema.tables[a].name.cmp(&schema.tables[b].name))
        });
        for (pos, &node) in layer_nodes.iter().enumerate() {
            order_in_layer.insert(node, pos as f32);
        }
    }

    let x_spacing = 300.0;
    let y_spacing = 240.0;
    let start_x = 40.0;
    let start_y = 40.0;

    for (l_idx, nodes) in layers.iter().enumerate() {
        for (slot_idx, &tbl_idx) in nodes.iter().enumerate() {
            let pos = Pos2::new(
                start_x + (l_idx as f32) * x_spacing,
                start_y + (slot_idx as f32) * y_spacing,
            );
            positions.insert(schema.tables[tbl_idx].name.clone(), pos);
        }
    }
    positions
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

pub fn export_er_to_svg(schema: &DatabaseSchema, positions: &HashMap<String, Pos2>) -> String {
    let card_w = 220.0_f32;
    let header_h = 30.0_f32;
    let row_h = 18.0_f32;
    let padding = 40.0_f32;

    let mut rects: HashMap<String, (f32, f32, f32, f32)> = HashMap::new();
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;

    for (idx, table) in schema.tables.iter().enumerate() {
        let pos = positions.get(&table.name).copied().unwrap_or_else(|| {
            Pos2::new(
                40.0 + ((idx % 3) as f32) * 300.0,
                40.0 + ((idx / 3) as f32) * 240.0,
            )
        });
        let visible_cols = table.columns.len().min(12) as f32;
        let extra_row = if table.columns.len() > 12 { 18.0 } else { 8.0 };
        let h = header_h + visible_cols * row_h + extra_row;
        rects.insert(table.name.clone(), (pos.x, pos.y, card_w, h));
        min_x = min_x.min(pos.x);
        min_y = min_y.min(pos.y);
        max_x = max_x.max(pos.x + card_w);
        max_y = max_y.max(pos.y + h);
    }

    if !min_x.is_finite() {
        min_x = 0.0;
        min_y = 0.0;
        max_x = 400.0;
        max_y = 200.0;
    }

    let vx = min_x - padding;
    let vy = min_y - padding;
    let vw = (max_x - min_x + padding * 2.0).max(200.0);
    let vh = (max_y - min_y + padding * 2.0).max(160.0);

    let mut svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{vw:.0}" height="{vh:.0}" viewBox="{vx:.1} {vy:.1} {vw:.1} {vh:.1}" font-family="ui-monospace, SFMono-Regular, Menlo, monospace">
  <defs>
    <marker id="fk-arrow" markerWidth="8" markerHeight="8" refX="7" refY="4" orient="auto">
      <path d="M0,1 L7,4 L0,7 Z" fill="#38bdf8" />
    </marker>
  </defs>
  <rect x="{vx:.1}" y="{vy:.1}" width="{vw:.1}" height="{vh:.1}" fill="#0f172a" />
"##
    );

    // Draw FK edges
    for table in &schema.tables {
        let Some(&(cx, cy, cw, ch)) = rects.get(&table.name) else {
            continue;
        };
        for fk in &table.foreign_keys {
            let Some(&(px, py, pw, ph)) = rects.get(&fk.referenced_table) else {
                continue;
            };
            let start_x = if cx >= px + pw { cx } else { cx + cw };
            let start_y = cy + ch * 0.5;
            let end_x = if cx >= px + pw { px + pw } else { px };
            let end_y = py + ph * 0.5;
            let ctrl_dx = ((end_x - start_x).abs() * 0.5).max(40.0);
            let c1x = if start_x < end_x {
                start_x + ctrl_dx
            } else {
                start_x - ctrl_dx
            };
            let c2x = if start_x < end_x {
                end_x - ctrl_dx
            } else {
                end_x + ctrl_dx
            };
            svg.push_str(&format!(
                r##"  <path d="M{start_x:.1},{start_y:.1} C{c1x:.1},{start_y:.1} {c2x:.1},{end_y:.1} {end_x:.1},{end_y:.1}" fill="none" stroke="#38bdf8" stroke-width="1.6" stroke-opacity="0.75" marker-end="url(#fk-arrow)" />
"##
            ));
        }
    }

    // Draw table cards
    for table in &schema.tables {
        let Some(&(x, y, w, h)) = rects.get(&table.name) else {
            continue;
        };
        let fk_cols: HashSet<&str> = table
            .foreign_keys
            .iter()
            .map(|f| f.column_name.as_str())
            .collect();
        let tname = xml_escape(&table.name);
        svg.push_str(&format!(
            r##"  <g>
    <rect x="{x:.1}" y="{y:.1}" width="{w:.1}" height="{h:.1}" rx="6" fill="#1e293b" stroke="#475569" stroke-width="1.2" />
    <rect x="{x:.1}" y="{y:.1}" width="{w:.1}" height="{header_h:.1}" rx="6" fill="#0f172a" />
    <text x="{tx:.1}" y="{ty:.1}" fill="#38bdf8" font-size="12" font-weight="bold">{tname}</text>
"##,
            tx = x + 10.0,
            ty = y + 20.0
        ));

        for (i, col) in table.columns.iter().take(12).enumerate() {
            let cy = y + header_h + 14.0 + (i as f32) * row_h;
            let badge = if col.is_primary_key {
                "PK "
            } else if fk_cols.contains(col.name.as_str()) {
                "FK "
            } else {
                "   "
            };
            let badge_color = if col.is_primary_key {
                "#facc15"
            } else if fk_cols.contains(col.name.as_str()) {
                "#38bdf8"
            } else {
                "#94a3b8"
            };
            let cname = xml_escape(&col.name);
            let ctype = xml_escape(&col.data_type);
            svg.push_str(&format!(
                r##"    <text x="{lx:.1}" y="{cy:.1}" fill="{badge_color}" font-size="10">{badge}</text>
    <text x="{nx:.1}" y="{cy:.1}" fill="#e2e8f0" font-size="10">{cname}</text>
    <text x="{rx:.1}" y="{cy:.1}" fill="#94a3b8" font-size="10" text-anchor="end">{ctype}</text>
"##,
                lx = x + 8.0,
                nx = x + 28.0,
                rx = x + w - 8.0,
            ));
        }

        if table.columns.len() > 12 {
            let cy = y + header_h + 14.0 + 12.0 * row_h;
            svg.push_str(&format!(
                r##"    <text x="{mx:.1}" y="{cy:.1}" fill="#64748b" font-size="10" text-anchor="middle">+ {more} more columns</text>
"##,
                mx = x + w * 0.5,
                more = table.columns.len() - 12
            ));
        }
        svg.push_str("  </g>\n");
    }

    svg.push_str("</svg>\n");
    svg
}

pub fn export_er_to_png(
    schema: &DatabaseSchema,
    positions: &HashMap<String, Pos2>,
) -> Result<Vec<u8>> {
    let svg = export_er_to_svg(schema, positions);
    let mut opts = resvg::usvg::Options::default();
    opts.fontdb_mut().load_system_fonts();
    let tree = resvg::usvg::Tree::from_str(&svg, &opts)
        .map_err(|e| anyhow!("Failed to parse generated ER SVG: {e}"))?;
    let size = tree.size().to_int_size();
    let mut pixmap = tiny_skia::Pixmap::new(size.width(), size.height())
        .ok_or_else(|| anyhow!("Failed to allocate PNG pixmap"))?;
    resvg::render(
        &tree,
        tiny_skia::Transform::identity(),
        &mut pixmap.as_mut(),
    );
    pixmap
        .encode_png()
        .context("Failed to encode ER diagram as PNG")
}

#[allow(dead_code)]
impl ErCanvasState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn auto_layout(&mut self, schema: &DatabaseSchema) {
        self.node_positions = compute_auto_layout(schema);
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
                .button("Auto-Arrange")
                .on_hover_text("Arrange tables by foreign-key hierarchy")
                .clicked()
            {
                self.auto_layout(schema);
            }
            if ui.button("Export SVG").clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .set_file_name(format!("{}_er.svg", schema.name))
                    .add_filter("SVG", &["svg"])
                    .save_file()
            {
                let svg = export_er_to_svg(schema, &self.node_positions);
                self.export_status = match std::fs::write(&path, svg) {
                    Ok(()) => Some(format!("Exported SVG to {}", path.display())),
                    Err(e) => Some(format!("SVG export failed: {e}")),
                };
            }
            if ui.button("Export PNG").clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .set_file_name(format!("{}_er.png", schema.name))
                    .add_filter("PNG", &["png"])
                    .save_file()
            {
                self.export_status = match export_er_to_png(schema, &self.node_positions)
                    .and_then(|bytes| std::fs::write(&path, bytes).map_err(Into::into))
                {
                    Ok(()) => Some(format!("Exported PNG to {}", path.display())),
                    Err(e) => Some(format!("PNG export failed: {e}")),
                };
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_er_auto_layout_and_svg_png_export() {
        let schema = crate::demo::build_demo_schema("ecommerce_prod");
        let positions = compute_auto_layout(&schema);
        assert_eq!(positions.len(), schema.tables.len());

        // Ensure no two tables overlap at identical coordinates
        let unique_coords: HashSet<(i32, i32)> = positions
            .values()
            .map(|p| (p.x.round() as i32, p.y.round() as i32))
            .collect();
        assert_eq!(unique_coords.len(), schema.tables.len());

        let svg = export_er_to_svg(&schema, &positions);
        assert!(svg.starts_with("<svg "));
        assert!(svg.contains("users"));
        assert!(svg.contains("orders"));

        let png = export_er_to_png(&schema, &positions).unwrap();
        assert!(png.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]));
    }
}
