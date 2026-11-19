use super::super::*;

impl MysqlRustyApp {
    pub(crate) fn render_multi_results_split_view(ui: &mut Ui, tab: &mut QueryTab) {
        let mut focus_idx: Option<usize> = None;
        let results_snapshot = tab.multi_results.clone();
        let layout = tab.multi_result_layout;
        let default_height = match layout {
            MultiResultLayout::Stacked => 220.0,
            MultiResultLayout::SideBySide => 260.0,
            MultiResultLayout::Tabs => 240.0,
        };

        egui::ScrollArea::vertical()
            .id_salt("multi_results_split_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| match layout {
                MultiResultLayout::Stacked => {
                    for (idx, res) in results_snapshot.iter().enumerate() {
                        let height = tab
                            .multi_result_heights
                            .entry(idx)
                            .or_insert(default_height);
                        let collapsed = tab.multi_result_collapsed.entry(idx).or_insert(false);
                        if let Some(f) =
                            Self::render_compact_result_card(ui, idx, res, height, collapsed)
                        {
                            focus_idx = Some(f);
                        }
                        ui.add_space(6.0);
                    }
                }
                MultiResultLayout::SideBySide => {
                    let cols_per_row = tab
                        .multi_result_columns
                        .unwrap_or_else(|| {
                            if results_snapshot.len() >= 3 && ui.available_width() >= 1100.0 {
                                3
                            } else {
                                2
                            }
                        })
                        .max(1);
                    for chunk_start in (0..results_snapshot.len()).step_by(cols_per_row) {
                        let chunk_end = (chunk_start + cols_per_row).min(results_snapshot.len());
                        let count = chunk_end - chunk_start;
                        ui.columns(count, |columns| {
                            for (offset, col_ui) in columns.iter_mut().enumerate().take(count) {
                                let idx = chunk_start + offset;
                                let res = &results_snapshot[idx];
                                let height = tab
                                    .multi_result_heights
                                    .entry(idx)
                                    .or_insert(default_height);
                                let collapsed =
                                    tab.multi_result_collapsed.entry(idx).or_insert(false);
                                if let Some(f) = Self::render_compact_result_card(
                                    col_ui, idx, res, height, collapsed,
                                ) {
                                    focus_idx = Some(f);
                                }
                            }
                        });
                        ui.add_space(6.0);
                    }
                }
                MultiResultLayout::Tabs => {}
            });

        if let Some(idx) = focus_idx
            && let Some(res) = tab.multi_results.get(idx).cloned()
        {
            tab.active_result_idx = idx;
            tab.last_result = Some(res);
            tab.current_page = 0;
            tab.sort_column = None;
            tab.multi_result_layout = MultiResultLayout::Tabs;
        }
    }

    pub(crate) fn render_compact_result_card(
        ui: &mut Ui,
        idx: usize,
        res: &QueryResult,
        max_table_height: &mut f32,
        collapsed: &mut bool,
    ) -> Option<usize> {
        let pal = if ui.visuals().dark_mode {
            crate::theme::dark_palette()
        } else {
            crate::theme::light_palette()
        };
        let mut focus_clicked = false;
        let frame = egui::Frame::group(ui.style())
            .fill(pal.card_bg)
            .stroke(Stroke::new(1.0, pal.card_border))
            .corner_radius(egui::CornerRadius::same(6))
            .inner_margin(egui::Margin::same(8));

        frame.show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let toggle_icon = if *collapsed { "▸" } else { "▾" };
                if ui
                    .small_button(toggle_icon)
                    .on_hover_text(if *collapsed {
                        "Expand result"
                    } else {
                        "Collapse result"
                    })
                    .clicked()
                {
                    *collapsed = !*collapsed;
                }
                ui.label(
                    RichText::new(format!("Result #{}", idx + 1))
                        .strong()
                        .color(pal.accent_text),
                );
                ui.label(
                    RichText::new(format!("{} rows • {} ms", res.rows.len(), res.duration_ms))
                        .small()
                        .color(pal.muted_text),
                );
                let sql_preview: String = res.sql.split_whitespace().collect::<Vec<_>>().join(" ");
                let short_sql = if sql_preview.chars().count() > 36 {
                    let prefix: String = sql_preview.chars().take(36).collect();
                    format!("{prefix}...")
                } else {
                    sql_preview.clone()
                };
                if !short_sql.is_empty() {
                    ui.label(
                        RichText::new(short_sql)
                            .small()
                            .monospace()
                            .color(pal.muted_text),
                    )
                    .on_hover_text(&res.sql);
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .small_button("Focus Tab")
                        .on_hover_text("Focus this result in single-tab view")
                        .clicked()
                    {
                        focus_clicked = true;
                    }
                    if !*collapsed {
                        let expand_label = if *max_table_height >= 420.0 {
                            "Compact"
                        } else {
                            "Expand"
                        };
                        if ui
                            .small_button(expand_label)
                            .on_hover_text("Toggle taller / compact height (or drag bottom edge)")
                            .clicked()
                        {
                            *max_table_height = if *max_table_height >= 420.0 {
                                240.0
                            } else {
                                500.0
                            };
                        }
                    }
                    if ui
                        .small_button("Copy CSV")
                        .on_hover_text("Copy this result as CSV")
                        .clicked()
                        && let Ok(csv_text) = to_csv_string(res)
                    {
                        ui.ctx().copy_text(csv_text);
                    }
                });
            });

            if *collapsed {
                return;
            }

            ui.separator();

            if res.columns.is_empty() {
                ui.label(
                    RichText::new(&res.status_message)
                        .small()
                        .color(pal.success),
                );
                return;
            }

            let num_cols = res.columns.len();
            let col_width = (ui.available_width() / num_cols.max(1) as f32).clamp(90.0, 220.0);

            ui.push_id(("multi_res_card", idx), |ui| {
                egui::ScrollArea::horizontal()
                    .id_salt(("multi_res_hscroll", idx))
                    .show(ui, |ui| {
                        let table = TableBuilder::new(ui)
                            .striped(true)
                            .resizable(true)
                            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                            .max_scroll_height(*max_table_height)
                            .columns(Column::initial(col_width).at_least(60.0), num_cols);

                        table
                            .header(22.0, |mut header| {
                                for col_name in &res.columns {
                                    header.col(|ui| {
                                        ui.label(
                                            RichText::new(col_name)
                                                .small()
                                                .strong()
                                                .color(pal.text_secondary),
                                        );
                                    });
                                }
                            })
                            .body(|body| {
                                body.rows(20.0, res.rows.len(), |mut row| {
                                    let r_idx = row.index();
                                    let row_data = &res.rows[r_idx];
                                    for c_idx in 0..num_cols {
                                        row.col(|ui| {
                                            let txt = row_data
                                                .get(c_idx)
                                                .map(|c| c.display_text())
                                                .unwrap_or_default();
                                            let is_null = row_data
                                                .get(c_idx)
                                                .map(|c| matches!(c, crate::db::CellValue::Null))
                                                .unwrap_or(false);
                                            let rich = if is_null {
                                                RichText::new("NULL")
                                                    .small()
                                                    .italics()
                                                    .monospace()
                                                    .color(pal.muted_text)
                                                    .background_color(pal.null_badge_bg)
                                            } else {
                                                RichText::new(&txt).small().color(pal.text_primary)
                                            };
                                            if ui
                                                .add(
                                                    egui::Label::new(rich)
                                                        .truncate()
                                                        .sense(egui::Sense::click()),
                                                )
                                                .on_hover_text("Click to copy value")
                                                .clicked()
                                            {
                                                ui.ctx().copy_text(txt);
                                            }
                                        });
                                    }
                                });
                            });
                    });
            });

            // Vertical resize drag handle at the bottom edge of the card
            let (handle_rect, handle_resp) = ui.allocate_exact_size(
                egui::vec2(ui.available_width(), 7.0),
                egui::Sense::click_and_drag(),
            );
            let line_color = if handle_resp.hovered() || handle_resp.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
                Color32::from_rgb(56, 189, 248)
            } else {
                Color32::from_rgb(51, 65, 85)
            };
            let center_y = handle_rect.center().y;
            let bar_half_w = 24.0_f32.min(handle_rect.width() * 0.25);
            ui.painter().line_segment(
                [
                    egui::pos2(handle_rect.center().x - bar_half_w, center_y),
                    egui::pos2(handle_rect.center().x + bar_half_w, center_y),
                ],
                Stroke::new(2.0, line_color),
            );
            if handle_resp.dragged() {
                *max_table_height =
                    (*max_table_height + handle_resp.drag_delta().y).clamp(80.0, 900.0);
            }
        });

        if focus_clicked { Some(idx) } else { None }
    }
}
