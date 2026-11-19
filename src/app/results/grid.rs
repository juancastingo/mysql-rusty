use super::super::*;

impl MysqlRustyApp {
    pub(crate) fn render_results_grid(&mut self, ui: &mut Ui) {
        let pal = self.effective_palette(ui.ctx());
        let (is_running, has_result) = match self.tabs.get(self.active_tab_idx) {
            Some(t) => (t.is_running, t.last_result.is_some()),
            None => return,
        };

        if is_running {
            ui.vertical_centered(|ui| {
                ui.add_space(28.0);
                ui.spinner();
                ui.add_space(6.0);
                ui.label(RichText::new("Executing SQL query...").color(pal.text_secondary));
            });
            return;
        }

        if !has_result {
            ui.vertical_centered(|ui| {
                ui.add_space(32.0);
                ui.label(
                    RichText::new("No query results yet")
                        .size(14.0)
                        .strong()
                        .color(pal.text_secondary),
                );
                ui.add_space(4.0);
                ui.label(
                    RichText::new(
                        "Run a query with Ctrl+Enter to inspect rows, charts, and stats here.",
                    )
                    .size(12.0)
                    .color(pal.muted_text),
                );
            });
            return;
        }

        let mut result = self.tabs[self.active_tab_idx].last_result.clone().unwrap();
        let active_schema_opt = self.schemas.get(&self.active_database).cloned();
        let mut trigger_commit_edits = false;
        let mut trigger_commit_deletes = false;
        let mut trigger_fk_lookup: Option<(String, String, String)> = None; // (key, ref_table, ref_col)
        let mut trigger_ai_prompt: Option<String> = None;
        let mut cell_inspector_to_open: Option<CellInspectorState> = None;
        let mut open_related_query: Option<(String, String)> = None;
        let mut duplicate_insert_tab: Option<(String, String)> = None;
        let mut copy_banner_msg: Option<String> = None;
        let mut load_all_sql: Option<String> = None;
        let fk_cache_snapshot = self.fk_lookup_cache.clone();
        let pending_edits;
        let pending_deletes;

        {
            let tab = &mut self.tabs[self.active_tab_idx];

            // Multi-Result Sub-Tabs (when a script executes multiple statements)
            if tab.multi_results.len() > 1 {
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new("Result Sets:")
                            .small()
                            .strong()
                            .color(pal.accent_text),
                    );
                    if tab.multi_result_layout == MultiResultLayout::Tabs {
                        for (r_idx, r_item) in tab.multi_results.iter().enumerate() {
                            let is_sel = tab.active_result_idx == r_idx;
                            let lbl = format!("Result {} ({} rows)", r_idx + 1, r_item.rows.len());
                            if ui.selectable_label(is_sel, lbl).clicked() {
                                tab.active_result_idx = r_idx;
                                tab.last_result = Some(r_item.clone());
                                tab.current_page = 0;
                                tab.sort_column = None;
                                result = r_item.clone();
                            }
                        }
                    } else {
                        ui.label(
                            RichText::new(format!(
                                "Showing all {} results",
                                tab.multi_results.len()
                            ))
                            .small()
                            .color(pal.muted_text),
                        );
                        if tab.multi_result_layout == MultiResultLayout::SideBySide {
                            ui.separator();
                            ui.label(RichText::new("Cols:").small().color(pal.muted_text));
                            let current_cols_label = match tab.multi_result_columns {
                                None => "Auto".to_string(),
                                Some(c) => format!("{c}"),
                            };
                            egui::ComboBox::from_id_salt("multi_res_cols_combo")
                                .selected_text(current_cols_label)
                                .width(58.0)
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(
                                        &mut tab.multi_result_columns,
                                        None,
                                        "Auto",
                                    );
                                    for c in [1usize, 2, 3, 4] {
                                        ui.selectable_value(
                                            &mut tab.multi_result_columns,
                                            Some(c),
                                            format!("{c}"),
                                        );
                                    }
                                });
                        }
                        ui.separator();
                        if ui.small_button("▾ Expand All").clicked() {
                            tab.multi_result_collapsed.clear();
                        }
                        if ui.small_button("▸ Collapse All").clicked() {
                            for i in 0..tab.multi_results.len() {
                                tab.multi_result_collapsed.insert(i, true);
                            }
                        }
                        if ui
                            .small_button("↺ Reset Size")
                            .on_hover_text("Reset custom card heights")
                            .clicked()
                        {
                            tab.multi_result_heights.clear();
                        }
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.selectable_value(
                            &mut tab.multi_result_layout,
                            MultiResultLayout::SideBySide,
                            "Side-by-Side",
                        );
                        ui.selectable_value(
                            &mut tab.multi_result_layout,
                            MultiResultLayout::Stacked,
                            "Stacked",
                        );
                        ui.selectable_value(
                            &mut tab.multi_result_layout,
                            MultiResultLayout::Tabs,
                            "Tabs",
                        );
                    });
                });
                ui.separator();

                if tab.multi_result_layout != MultiResultLayout::Tabs {
                    Self::render_multi_results_split_view(ui, tab);
                    return;
                }
            }

            if result.truncated {
                pal.banner_frame(pal.warning_bg, pal.warning)
                    .show(ui, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(
                                RichText::new(format!(
                                    "Showing first {} rows (truncated at max_rows = {})",
                                    result.rows.len(),
                                    result.rows.len()
                                ))
                                .color(pal.warning)
                                .strong(),
                            );
                            if tab.confirm_load_all {
                                ui.label(
                                    RichText::new("This may use a lot of memory.")
                                        .color(pal.text_secondary),
                                );
                                if ui.small_button("Confirm Load All").clicked() {
                                    tab.confirm_load_all = false;
                                    load_all_sql = Some(result.sql.clone());
                                }
                                if ui.small_button("Cancel").clicked() {
                                    tab.confirm_load_all = false;
                                }
                            } else if ui.small_button("Load all rows").clicked() {
                                tab.confirm_load_all = true;
                            }
                            ui.label(
                                RichText::new("Tip: add a LIMIT clause")
                                    .color(pal.muted_text)
                                    .size(11.5),
                            );
                        });
                    });
                ui.add_space(4.0);
            }

            // Results Action Bar (Filter, Pagination, Export, Inline Edit Commit)
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("Filter:").color(pal.text_secondary));
                ui.add(
                    egui::TextEdit::singleline(&mut tab.row_filter)
                        .desired_width(130.0)
                        .hint_text("Search rows..."),
                );

                let filter_btn_txt = if tab.show_visual_filter {
                    "Hide Filter ▴"
                } else {
                    "Column Filter ▾"
                };
                if ui.small_button(filter_btn_txt).clicked() {
                    tab.show_visual_filter = !tab.show_visual_filter;
                }

                ui.separator();
                egui::ComboBox::from_id_salt("results_page_size_combo")
                    .selected_text(if tab.page_size == 0 {
                        "All rows".to_string()
                    } else {
                        format!("{} / page", tab.page_size)
                    })
                    .width(85.0)
                    .show_ui(ui, |ui| {
                        for ps in [25, 50, 100, 250, 500, 0] {
                            let label = if ps == 0 {
                                "All".to_string()
                            } else {
                                format!("{ps}")
                            };
                            if ui.selectable_label(tab.page_size == ps, label).clicked() {
                                tab.page_size = ps;
                                tab.current_page = 0;
                            }
                        }
                    });

                ui.separator();
                ui.selectable_value(&mut tab.view_mode, ResultViewMode::Grid, "Table");
                ui.selectable_value(&mut tab.view_mode, ResultViewMode::Chart, "Chart");

                ui.separator();
                if ui
                    .selectable_label(tab.show_row_inspector, "Inspector")
                    .on_hover_text("Toggle vertical single-row inspector & FK lookup drawer")
                    .clicked()
                {
                    tab.show_row_inspector = !tab.show_row_inspector;
                    if tab.show_row_inspector
                        && tab.row_inspector_row.is_none()
                        && !result.rows.is_empty()
                    {
                        tab.row_inspector_row = Some(0);
                        tab.row_inspector_buffers = result.rows[0]
                            .iter()
                            .map(|c| c.display_text())
                            .collect();
                    }
                }

                if ui
                    .small_button("Pin Baseline")
                    .on_hover_text("Save current result set as baseline for comparison")
                    .clicked()
                {
                    tab.baseline_result = Some(result.clone());
                    tab.show_baseline_compare = false;
                    copy_banner_msg = Some("Pinned current result as Baseline.".to_string());
                }
                if tab.baseline_result.is_some() {
                    let cmp_lbl = if tab.show_baseline_compare {
                        "Exit Compare"
                    } else {
                        "Compare"
                    };
                    if ui
                        .selectable_label(tab.show_baseline_compare, cmp_lbl)
                        .clicked()
                    {
                        tab.show_baseline_compare = !tab.show_baseline_compare;
                    }
                }

                ui.separator();
                ui.label(
                    RichText::new(format!(
                        "{} rows ({} ms)",
                        result.rows.len(),
                        result.duration_ms
                    ))
                    .strong()
                    .color(pal.text_primary),
                );

                if let (Some(tbl), Some(pk)) = (&result.editable_table, &result.primary_key_column)
                {
                    pal.badge(ui, &format!("Editable: {tbl} ({pk})"), pal.success, pal.success_bg);
                }

                if !tab.pending_edits.is_empty() {
                    let apply_btn = egui::Button::new(
                        RichText::new(format!("Apply {} Edit(s)", tab.pending_edits.len()))
                            .color(pal.warning)
                            .strong(),
                    )
                    .fill(pal.warning_bg)
                    .stroke(Stroke::new(1.0, pal.warning))
                    .corner_radius(egui::CornerRadius::same(4));
                    if ui.add(apply_btn).clicked() {
                        trigger_commit_edits = true;
                    }
                    if ui.small_button("Discard").clicked() {
                        tab.pending_edits.clear();
                    }
                }

                ui.separator();
                ui.menu_button("Export ▾", |ui| {
                    if ui.button("Export CSV...").clicked()
                        && let Ok(csv_data) = to_csv_string(&result)
                    {
                        if let Some(path) = rfd::FileDialog::new()
                            .set_file_name("query_results.csv")
                            .save_file()
                        {
                            let _ = save_string_to_path(&path, &csv_data);
                        } else {
                            ui.ctx().copy_text(csv_data);
                        }
                        if result.truncated {
                            copy_banner_msg = Some(format!(
                                "Exported CSV. Warning: Export contains only the first {} rows.",
                                result.rows.len()
                            ));
                        }
                        ui.close();
                    }
                    if ui.button("Export JSON...").clicked()
                        && let Ok(json_data) = to_json_string(&result)
                    {
                        if let Some(path) = rfd::FileDialog::new()
                            .set_file_name("query_results.json")
                            .save_file()
                        {
                            let _ = save_string_to_path(&path, &json_data);
                        } else {
                            ui.ctx().copy_text(json_data);
                        }
                        if result.truncated {
                            copy_banner_msg = Some(format!(
                                "Exported JSON. Warning: Export contains only the first {} rows.",
                                result.rows.len()
                            ));
                        }
                        ui.close();
                    }
                    ui.separator();
                    if ui.button("Copy Markdown Table").clicked() {
                        let md = to_markdown_table(&result);
                        ui.ctx().copy_text(md);
                        copy_banner_msg = Some(if result.truncated {
                            format!(
                                "Copied Markdown. Warning: Export contains only the first {} rows.",
                                result.rows.len()
                            )
                        } else {
                            "Copied Markdown table to clipboard.".to_string()
                        });
                        ui.close();
                    }
                    if ui.button("Copy SQL INSERTs").clicked() {
                        let tbl = result
                            .editable_table
                            .as_deref()
                            .unwrap_or("exported_rows");
                        let inserts = to_sql_inserts(&result, tbl);
                        ui.ctx().copy_text(inserts);
                        copy_banner_msg = Some(if result.truncated {
                            format!(
                                "Copied SQL INSERTs. Warning: Export contains only the first {} rows.",
                                result.rows.len()
                            )
                        } else {
                            "Copied SQL INSERT statements to clipboard.".to_string()
                        });
                        ui.close();
                    }
                });

                if ui
                    .small_button(RichText::new("AI Insights").color(pal.purple).strong())
                    .on_hover_text("Analyze these query results with AI")
                    .clicked()
                {
                    let preview_rows: Vec<String> = result
                        .rows
                        .iter()
                        .take(25)
                        .map(|r| {
                            r.iter()
                                .map(|c| c.display_text())
                                .collect::<Vec<_>>()
                                .join(" | ")
                        })
                        .collect();
                    trigger_ai_prompt = Some(format!(
                        "Please analyze these database query results:\nQuery: `{}`\nColumns: {}\nSample Rows (first {}):\n{}\n\nProvide key metrics, patterns, summary statistics, and notable takeaways concisely.",
                        result.sql,
                        result.columns.join(", "),
                        preview_rows.len(),
                        preview_rows.join("\n")
                    ));
                }
            });

            ui.add_space(2.0);

            // Visual Column Filter Bar
            if tab.show_visual_filter {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("Where:")
                            .size(12.0)
                            .strong()
                            .color(Color32::from_rgb(56, 189, 248)),
                    );
                    let selected_col_name = tab
                        .visual_filter_col
                        .and_then(|idx| result.columns.get(idx))
                        .cloned()
                        .unwrap_or_else(|| "Any Column".to_string());

                    egui::ComboBox::from_id_salt("results_vis_filter_col_combo")
                        .selected_text(selected_col_name)
                        .width(130.0)
                        .show_ui(ui, |ui| {
                            if ui
                                .selectable_label(tab.visual_filter_col.is_none(), "Any Column")
                                .clicked()
                            {
                                tab.visual_filter_col = None;
                                tab.current_page = 0;
                            }
                            for (i, col) in result.columns.iter().enumerate() {
                                if ui
                                    .selectable_label(tab.visual_filter_col == Some(i), col)
                                    .clicked()
                                {
                                    tab.visual_filter_col = Some(i);
                                    tab.current_page = 0;
                                }
                            }
                        });

                    egui::ComboBox::from_id_salt("results_vis_filter_op_combo")
                        .selected_text(tab.visual_filter_op.label())
                        .width(115.0)
                        .show_ui(ui, |ui| {
                            for op in [
                                VisualFilterOp::Contains,
                                VisualFilterOp::Equals,
                                VisualFilterOp::NotEquals,
                                VisualFilterOp::StartsWith,
                                VisualFilterOp::GreaterThan,
                                VisualFilterOp::LessThan,
                                VisualFilterOp::IsNull,
                                VisualFilterOp::IsNotNull,
                            ] {
                                if ui
                                    .selectable_label(tab.visual_filter_op == op, op.label())
                                    .clicked()
                                {
                                    tab.visual_filter_op = op;
                                    tab.current_page = 0;
                                }
                            }
                        });

                    if tab.visual_filter_op != VisualFilterOp::IsNull
                        && tab.visual_filter_op != VisualFilterOp::IsNotNull
                    {
                        let resp = ui.add(
                            egui::TextEdit::singleline(&mut tab.visual_filter_val)
                                .desired_width(160.0)
                                .hint_text("Value to match..."),
                        );
                        if resp.changed() {
                            tab.current_page = 0;
                        }
                    }

                    if (!tab.visual_filter_val.is_empty() || tab.visual_filter_col.is_some())
                        && ui.small_button("Clear").clicked()
                    {
                        tab.visual_filter_val.clear();
                        tab.visual_filter_col = None;
                        tab.current_page = 0;
                    }
                });
                ui.add_space(2.0);
            }

            if tab.view_mode == ResultViewMode::Chart {
                Self::draw_results_chart(ui, tab, &result);
                return;
            }

            // #10 Result Snapshot Compare vs Baseline
            if tab.show_baseline_compare
                && let Some(baseline) = &tab.baseline_result
            {
                let key_idx = result
                    .primary_key_column
                    .as_ref()
                    .and_then(|pk| {
                        result
                            .columns
                            .iter()
                            .position(|c| c.eq_ignore_ascii_case(pk))
                    })
                    .unwrap_or(0);
                let report = compute_data_diff(baseline, &result, key_idx);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!(
                            "Baseline Compare: {} modified | {} added | {} removed | {} unchanged",
                            report.modified_count,
                            report.right_only_count,
                            report.left_only_count,
                            report.same_count
                        ))
                        .strong()
                        .color(Color32::from_rgb(56, 189, 248)),
                    );
                });
                ui.separator();
                egui::ScrollArea::both().show(ui, |ui| {
                    for entry in &report.entries {
                        if entry.status == RowDiffStatus::Same {
                            continue;
                        }
                        let (badge, color) = match entry.status {
                            RowDiffStatus::OnlyInRight => {
                                ("+ ADDED", Color32::from_rgb(52, 211, 153))
                            }
                            RowDiffStatus::OnlyInLeft => {
                                ("- REMOVED", Color32::from_rgb(248, 113, 113))
                            }
                            RowDiffStatus::Modified => {
                                ("~ MODIFIED", Color32::from_rgb(250, 204, 21))
                            }
                            RowDiffStatus::Same => ("SAME", Color32::GRAY),
                        };
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(badge).small().strong().color(color));
                            ui.label(RichText::new(format!("Key: {}", entry.key_display)).strong());
                            if let Some(r) = &entry.right_row {
                                ui.label(RichText::new(format!("New: {}", r.join(" | "))).small());
                            }
                            if entry.status == RowDiffStatus::Modified
                                && let Some(l) = &entry.left_row
                            {
                                ui.label(
                                    RichText::new(format!("(Was: {})", l.join(" | ")))
                                        .small()
                                        .weak(),
                                );
                            }
                        });
                    }
                });
                return;
            }

            // #4 & #5 Multi-Row Selection & Pending Deletes Action Bar
            if !tab.selected_rows.is_empty() || !tab.pending_row_deletes.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    if !tab.selected_rows.is_empty() {
                        let mut sel_indices: Vec<usize> =
                            tab.selected_rows.iter().copied().collect();
                        sel_indices.sort_unstable();
                        ui.label(
                            RichText::new(format!("{} row(s) selected:", sel_indices.len()))
                                .small()
                                .strong()
                                .color(Color32::from_rgb(56, 189, 248)),
                        );
                        if ui.small_button("Copy JSON").clicked() {
                            ui.ctx()
                                .copy_text(format_rows_as_json(&result, &sel_indices));
                            copy_banner_msg =
                                Some(format!("Copied {} row(s) as JSON.", sel_indices.len()));
                        }
                        if ui.small_button("Copy INSERT").clicked() {
                            let tbl = result.editable_table.as_deref().unwrap_or("table_name");
                            ui.ctx().copy_text(format_rows_as_sql_insert(
                                tbl,
                                &result,
                                &sel_indices,
                            ));
                            copy_banner_msg = Some(format!(
                                "Copied {} row(s) as SQL INSERT.",
                                sel_indices.len()
                            ));
                        }
                        if let Some(pk_col) = &result.primary_key_column
                            && ui.small_button("Copy UPDATE").clicked()
                        {
                            let tbl = result.editable_table.as_deref().unwrap_or("table_name");
                            ui.ctx().copy_text(format_rows_as_sql_update(
                                tbl,
                                pk_col,
                                &result,
                                &sel_indices,
                            ));
                            copy_banner_msg = Some(format!(
                                "Copied {} row(s) as SQL UPDATE.",
                                sel_indices.len()
                            ));
                        }
                        if ui.small_button("Duplicate Row(s)").clicked() {
                            let tbl = result.editable_table.as_deref().unwrap_or("table_name");
                            let sql = format_rows_as_sql_insert(tbl, &result, &sel_indices);
                            duplicate_insert_tab = Some((format!("Duplicate {tbl}"), sql));
                        }
                        if let (Some(_tbl), Some(pk_col)) =
                            (&result.editable_table, &result.primary_key_column)
                            && let Some(pk_idx) = result
                                .columns
                                .iter()
                                .position(|c| c.eq_ignore_ascii_case(pk_col))
                            && ui.small_button("Mark for Delete").clicked()
                        {
                            for &r_idx in &sel_indices {
                                if !tab
                                    .pending_row_deletes
                                    .iter()
                                    .any(|(idx, _, _)| *idx == r_idx)
                                    && let Some(pk_val) =
                                        result.rows.get(r_idx).and_then(|r| r.get(pk_idx))
                                {
                                    tab.pending_row_deletes.push((
                                        r_idx,
                                        pk_col.clone(),
                                        pk_val.display_text(),
                                    ));
                                }
                            }
                        }
                        if ui.small_button("Clear Sel").clicked() {
                            tab.selected_rows.clear();
                        }
                    }

                    if !tab.pending_row_deletes.is_empty() {
                        ui.separator();
                        let del_btn = egui::Button::new(
                            RichText::new(format!(
                                "Commit Deletes ({})",
                                tab.pending_row_deletes.len()
                            ))
                            .small()
                            .strong()
                            .color(Color32::WHITE),
                        )
                        .fill(Color32::from_rgb(185, 28, 28));
                        if ui.add(del_btn).clicked() {
                            trigger_commit_deletes = true;
                        }
                        if ui.small_button("Discard Deletes").clicked() {
                            tab.pending_row_deletes.clear();
                        }
                    }
                });
                ui.add_space(2.0);
            }

            // Filter & sort rows
            let filter_lower = tab.row_filter.to_lowercase();
            let val_filter_lower = tab.visual_filter_val.to_lowercase();
            let mut filtered_indices: Vec<usize> = (0..result.rows.len())
                .filter(|&r_idx| {
                    let row = &result.rows[r_idx];
                    if !filter_lower.is_empty()
                        && !row
                            .iter()
                            .any(|c| c.display_text().to_lowercase().contains(&filter_lower))
                    {
                        return false;
                    }
                    if let Some(col_idx) = tab.visual_filter_col {
                        if let Some(cell) = row.get(col_idx) {
                            let text = cell.display_text();
                            let text_lower = text.to_lowercase();
                            match tab.visual_filter_op {
                                VisualFilterOp::Contains => text_lower.contains(&val_filter_lower),
                                VisualFilterOp::Equals => {
                                    text.eq_ignore_ascii_case(&tab.visual_filter_val)
                                }
                                VisualFilterOp::NotEquals => {
                                    !text.eq_ignore_ascii_case(&tab.visual_filter_val)
                                }
                                VisualFilterOp::StartsWith => {
                                    text_lower.starts_with(&val_filter_lower)
                                }
                                VisualFilterOp::GreaterThan => {
                                    if let (Ok(n1), Ok(n2)) =
                                        (text.parse::<f64>(), tab.visual_filter_val.parse::<f64>())
                                    {
                                        n1 > n2
                                    } else {
                                        text > tab.visual_filter_val
                                    }
                                }
                                VisualFilterOp::LessThan => {
                                    if let (Ok(n1), Ok(n2)) =
                                        (text.parse::<f64>(), tab.visual_filter_val.parse::<f64>())
                                    {
                                        n1 < n2
                                    } else {
                                        text < tab.visual_filter_val
                                    }
                                }
                                VisualFilterOp::IsNull => text.is_empty() || text == "NULL",
                                VisualFilterOp::IsNotNull => !text.is_empty() && text != "NULL",
                            }
                        } else {
                            false
                        }
                    } else if !val_filter_lower.is_empty() {
                        match tab.visual_filter_op {
                            VisualFilterOp::Contains => row.iter().any(|c| {
                                c.display_text().to_lowercase().contains(&val_filter_lower)
                            }),
                            VisualFilterOp::Equals => row.iter().any(|c| {
                                c.display_text()
                                    .eq_ignore_ascii_case(&tab.visual_filter_val)
                            }),
                            VisualFilterOp::NotEquals => !row.iter().any(|c| {
                                c.display_text()
                                    .eq_ignore_ascii_case(&tab.visual_filter_val)
                            }),
                            VisualFilterOp::StartsWith => row.iter().any(|c| {
                                c.display_text()
                                    .to_lowercase()
                                    .starts_with(&val_filter_lower)
                            }),
                            _ => true,
                        }
                    } else {
                        true
                    }
                })
                .collect();

            if let Some((sort_col, asc)) = tab.sort_column {
                filtered_indices.sort_by(|&a, &b| {
                    let va = result.rows[a]
                        .get(sort_col)
                        .map(|c| c.display_text())
                        .unwrap_or_default();
                    let vb = result.rows[b]
                        .get(sort_col)
                        .map(|c| c.display_text())
                        .unwrap_or_default();
                    if asc { va.cmp(&vb) } else { vb.cmp(&va) }
                });
            }

            let total_matching = filtered_indices.len();
            let total_pages = if tab.page_size == 0 {
                1
            } else {
                total_matching.div_ceil(tab.page_size)
            }
            .max(1);

            if tab.current_page >= total_pages {
                tab.current_page = total_pages.saturating_sub(1);
            }

            let (display_start, display_end) = if tab.page_size == 0 {
                (0, total_matching)
            } else {
                let start = tab.current_page * tab.page_size;
                let end = (start + tab.page_size).min(total_matching);
                (start, end)
            };
            let page_indices: Vec<usize> = if total_matching > 0 && display_start < total_matching {
                filtered_indices[display_start..display_end].to_vec()
            } else {
                Vec::new()
            };

            // Render pagination navigation if pages > 1
            if tab.page_size > 0 && total_pages > 1 {
                ui.horizontal(|ui| {
                    let can_prev = tab.current_page > 0;
                    let can_next = tab.current_page + 1 < total_pages;
                    if ui
                        .add_enabled(can_prev, egui::Button::new("⏮").small())
                        .on_hover_text("First page")
                        .clicked()
                    {
                        tab.current_page = 0;
                    }
                    if ui
                        .add_enabled(can_prev, egui::Button::new("◀ Prev").small())
                        .clicked()
                    {
                        tab.current_page = tab.current_page.saturating_sub(1);
                    }
                    ui.label(
                        RichText::new(format!(
                            "Showing rows {}-{} of {} (Page {} of {})",
                            if total_matching == 0 {
                                0
                            } else {
                                display_start + 1
                            },
                            display_end,
                            total_matching,
                            tab.current_page + 1,
                            total_pages
                        ))
                        .small()
                        .color(pal.muted_text),
                    );
                    if ui
                        .add_enabled(can_next, egui::Button::new("Next ▶").small())
                        .clicked()
                    {
                        tab.current_page += 1;
                    }
                    if ui
                        .add_enabled(can_next, egui::Button::new("⏭").small())
                        .on_hover_text("Last page")
                        .clicked()
                    {
                        tab.current_page = total_pages.saturating_sub(1);
                    }
                });
                ui.add_space(2.0);
            }

            let pk_col_idx = result.primary_key_column.as_ref().and_then(|pk| {
                result
                    .columns
                    .iter()
                    .position(|c| c.eq_ignore_ascii_case(pk))
            });

            // #1 & #2 Vertical Row Inspector Drawer (Right side inside ResultsGrid)
            if tab.show_row_inspector {
                let max_row = result.rows.len().saturating_sub(1);
                let cur_row_idx = tab.row_inspector_row.unwrap_or(0).min(max_row);
                if tab.row_inspector_buffers.len() != result.columns.len()
                    && let Some(row_data) = result.rows.get(cur_row_idx)
                {
                    tab.row_inspector_buffers = row_data.iter().map(|c| c.display_text()).collect();
                    tab.row_inspector_row = Some(cur_row_idx);
                }

                egui::Panel::right("row_inspector_drawer")
                    .resizable(true)
                    .default_size(300.0)
                    .min_size(220.0)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(format!("Row #{}", cur_row_idx + 1))
                                    .strong()
                                    .color(Color32::from_rgb(56, 189, 248)),
                            );
                            if ui.small_button("◀").clicked() && cur_row_idx > 0 {
                                let next_r = cur_row_idx - 1;
                                tab.row_inspector_row = Some(next_r);
                                if let Some(rd) = result.rows.get(next_r) {
                                    tab.row_inspector_buffers =
                                        rd.iter().map(|c| c.display_text()).collect();
                                }
                            }
                            if ui.small_button("▶").clicked() && cur_row_idx < max_row {
                                let next_r = cur_row_idx + 1;
                                tab.row_inspector_row = Some(next_r);
                                if let Some(rd) = result.rows.get(next_r) {
                                    tab.row_inspector_buffers =
                                        rd.iter().map(|c| c.display_text()).collect();
                                }
                            }
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui.small_button("×").clicked() {
                                        tab.show_row_inspector = false;
                                    }
                                },
                            );
                        });
                        ui.separator();

                        let can_edit = pk_col_idx.is_some() && result.editable_table.is_some();
                        let table_fks: Vec<crate::db::ForeignKeyInfo> = result
                            .editable_table
                            .as_ref()
                            .and_then(|tname| {
                                active_schema_opt.as_ref().and_then(|s| {
                                    s.tables
                                        .iter()
                                        .find(|t| t.name.eq_ignore_ascii_case(tname))
                                        .map(|t| t.foreign_keys.clone())
                                })
                            })
                            .unwrap_or_default();

                        egui::ScrollArea::vertical()
                            .id_salt("row_inspector_scroll")
                            .show(ui, |ui| {
                                for (c_idx, col_name) in result.columns.iter().enumerate() {
                                    let is_pk = pk_col_idx == Some(c_idx);
                                    let fk_opt = table_fks
                                        .iter()
                                        .find(|fk| fk.column_name.eq_ignore_ascii_case(col_name));

                                    ui.horizontal(|ui| {
                                        let badge = if is_pk {
                                            " [PK]"
                                        } else if fk_opt.is_some() {
                                            " [FK]"
                                        } else {
                                            ""
                                        };
                                        ui.label(
                                            RichText::new(format!("{col_name}{badge}"))
                                                .small()
                                                .strong(),
                                        );
                                        if let Some(fk) = fk_opt {
                                            let fk_key = format!(
                                                "{}.{}",
                                                fk.referenced_table, fk.referenced_column
                                            );
                                            if let Some(opts) = fk_cache_snapshot.get(&fk_key) {
                                                egui::ComboBox::from_id_salt((
                                                    "fk_lookup_combo",
                                                    c_idx,
                                                ))
                                                .selected_text("FK Lookup")
                                                .width(110.0)
                                                .show_ui(ui, |ui| {
                                                    for (val, lbl) in opts {
                                                        if ui.selectable_label(false, lbl).clicked()
                                                            && let Some(buf) = tab
                                                                .row_inspector_buffers
                                                                .get_mut(c_idx)
                                                        {
                                                            *buf = val.clone();
                                                        }
                                                    }
                                                });
                                            } else if ui
                                                .small_button("Lookup")
                                                .on_hover_text(format!(
                                                    "Load values from `{}`",
                                                    fk.referenced_table
                                                ))
                                                .clicked()
                                            {
                                                trigger_fk_lookup = Some((
                                                    fk_key,
                                                    fk.referenced_table.clone(),
                                                    fk.referenced_column.clone(),
                                                ));
                                            }
                                        }
                                    });

                                    if let Some(buf) = tab.row_inspector_buffers.get_mut(c_idx) {
                                        if can_edit && !is_pk {
                                            ui.add(
                                                egui::TextEdit::singleline(buf)
                                                    .desired_width(f32::INFINITY),
                                            );
                                        } else {
                                            ui.add_enabled(
                                                false,
                                                egui::TextEdit::singleline(buf)
                                                    .desired_width(f32::INFINITY),
                                            );
                                        }
                                    }
                                    ui.add_space(4.0);
                                }
                            });

                        if can_edit {
                            ui.separator();
                            if ui.button("Stage Edits").clicked()
                                && let (Some(pk_idx), Some(pk_name), Some(row_cells)) = (
                                    pk_col_idx,
                                    &result.primary_key_column,
                                    result.rows.get(cur_row_idx),
                                )
                            {
                                let pk_val = row_cells[pk_idx].display_text();
                                for (c_idx, col_name) in result.columns.iter().enumerate() {
                                    if c_idx == pk_idx {
                                        continue;
                                    }
                                    if let Some(new_val) = tab.row_inspector_buffers.get(c_idx) {
                                        let old_val = row_cells
                                            .get(c_idx)
                                            .map(|c| c.display_text())
                                            .unwrap_or_default();
                                        if new_val != &old_val {
                                            tab.pending_edits.push(PendingCellEdit {
                                                row_idx: cur_row_idx,
                                                col_idx: c_idx,
                                                column_name: col_name.clone(),
                                                pk_column: pk_name.clone(),
                                                pk_value: pk_val.clone(),
                                                new_value: new_val.clone(),
                                            });
                                        }
                                    }
                                }
                            }
                        }
                    });
            }

            // Render Virtualized Data Grid inside a horizontal ScrollArea so wide
            // result sets can be scrolled horizontally.
            let hscroll_id = tab.id.clone();
            egui::ScrollArea::horizontal()
                .id_salt(("results_grid_hscroll", hscroll_id))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let num_cols = result.columns.len().max(1);
                    let mut builder = TableBuilder::new(ui)
                        .striped(true)
                        .resizable(true)
                        .cell_layout(egui::Layout::left_to_right(egui::Align::Center));

                    builder = builder.column(Column::exact(28.0)); // Selection checkbox column
                    for _ in 0..num_cols {
                        builder = builder.column(Column::initial(135.0).at_least(65.0).clip(true));
                    }

                    builder
                    .header(24.0, |mut header| {
                header.col(|ui| {
                    let all_selected = !page_indices.is_empty()
                        && page_indices.iter().all(|r| tab.selected_rows.contains(r));
                    let mut checked = all_selected;
                    if ui
                        .checkbox(&mut checked, "")
                        .on_hover_text("Select/Deselect all rows on this page")
                        .changed()
                    {
                        for &r in &page_indices {
                            if checked {
                                tab.selected_rows.insert(r);
                            } else {
                                tab.selected_rows.remove(&r);
                            }
                        }
                    }
                });
                for (col_idx, col_name) in result.columns.iter().enumerate() {
                    header.col(|ui| {
                        let arrow = match tab.sort_column {
                            Some((c, true)) if c == col_idx => " ▲",
                            Some((c, false)) if c == col_idx => " ▼",
                            _ => "",
                        };
                        let hdr_resp = ui
                            .button(RichText::new(format!("{col_name}{arrow}")).strong())
                            .on_hover_text("Click to sort | Right-click to copy column as IN (...) or JS Array");
                        if hdr_resp.clicked() {
                            let next_asc = match tab.sort_column {
                                Some((c, asc)) if c == col_idx => !asc,
                                _ => true,
                            };
                            tab.sort_column = Some((col_idx, next_asc));
                        }
                        hdr_resp.context_menu(|ui| {
                            if ui.button("📋 Copy Column as SQL IN (...)").clicked() {
                                let in_clause = format_column_as_sql_in(&result, col_idx, &filtered_indices);
                                ui.ctx().copy_text(in_clause);
                                copy_banner_msg = Some(format!("Copied `{col_name}` as SQL IN (...) clause."));
                                ui.close();
                            }
                            if ui.button("📋 Copy Column as JS/TS Array").clicked() {
                                let js_arr = format_column_as_js_array(&result, col_idx, &filtered_indices);
                                ui.ctx().copy_text(js_arr);
                                copy_banner_msg = Some(format!("Copied `{col_name}` as JS/TS array."));
                                ui.close();
                            }
                        });
                    });
                }
            })
            .body(|body| {
                body.rows(24.0, page_indices.len(), |mut row| {
                    let actual_row_idx = page_indices[row.index()];
                    let row_cells = &result.rows[actual_row_idx];
                    let is_marked_delete = tab
                        .pending_row_deletes
                        .iter()
                        .any(|(idx, _, _)| *idx == actual_row_idx);
                    let is_selected_row = tab.selected_rows.contains(&actual_row_idx);

                    row.col(|ui| {
                        if is_marked_delete {
                            ui.painter().rect_filled(ui.max_rect(), 0.0, pal.row_delete_bg);
                        } else if is_selected_row {
                            ui.painter().rect_filled(ui.max_rect(), 0.0, pal.row_selected_bg);
                        }
                        let mut sel = is_selected_row;
                        if ui.checkbox(&mut sel, "").changed() {
                            if sel {
                                tab.selected_rows.insert(actual_row_idx);
                            } else {
                                tab.selected_rows.remove(&actual_row_idx);
                            }
                        }
                    });

                    for col_idx in 0..result.columns.len() {
                        row.col(|ui| {
                            let pending = tab.pending_edits.iter().find(|e| {
                                e.row_idx == actual_row_idx && e.col_idx == col_idx
                            });
                            if is_marked_delete {
                                ui.painter().rect_filled(ui.max_rect(), 0.0, pal.row_delete_bg);
                            } else if pending.is_some() {
                                ui.painter().rect_filled(ui.max_rect(), 0.0, pal.cell_edited_bg);
                            } else if is_selected_row {
                                ui.painter().rect_filled(ui.max_rect(), 0.0, pal.row_selected_bg);
                            }

                            let is_editing = matches!(
                                &tab.editing_cell,
                                Some((r, c, _)) if *r == actual_row_idx && *c == col_idx
                            );

                            if is_editing {
                                let (_, _, mut edit_buf) = tab.editing_cell.clone().unwrap();
                                let resp = ui.add(
                                    egui::TextEdit::singleline(&mut edit_buf)
                                        .desired_width(ui.available_width().max(100.0)),
                                );
                                if !resp.has_focus() {
                                    resp.request_focus();
                                }
                                if ui.input(|i| i.key_pressed(Key::Escape)) {
                                    tab.editing_cell = None;
                                } else {
                                    if resp.changed() {
                                        tab.editing_cell =
                                            Some((actual_row_idx, col_idx, edit_buf.clone()));
                                    }
                                    if resp.lost_focus()
                                        || ui.input(|i| i.key_pressed(Key::Enter))
                                    {
                                        if let (Some(pk_idx), Some(pk_name)) =
                                            (pk_col_idx, &result.primary_key_column)
                                        {
                                            let pk_val = row_cells[pk_idx].display_text();
                                            let old_val = row_cells[col_idx].display_text();
                                            if edit_buf != old_val {
                                                tab.pending_edits.push(PendingCellEdit {
                                                    row_idx: actual_row_idx,
                                                    col_idx,
                                                    column_name: result.columns[col_idx].clone(),
                                                    pk_column: pk_name.clone(),
                                                    pk_value: pk_val,
                                                    new_value: edit_buf,
                                                });
                                            }
                                        }
                                        tab.editing_cell = None;
                                    }
                                }
                            } else {
                                let cell = &row_cells[col_idx];
                                let mut text = if let Some(p) = pending {
                                    RichText::new(&p.new_value)
                                        .color(pal.warning)
                                        .strong()
                                } else if cell.is_null() {
                                    RichText::new("NULL")
                                        .italics()
                                        .monospace()
                                        .size(11.0)
                                        .color(pal.muted_text)
                                        .background_color(pal.null_badge_bg)
                                } else {
                                    RichText::new(cell.display_text()).color(pal.text_primary)
                                };
                                if is_marked_delete {
                                    text = text.strikethrough().color(pal.danger);
                                }

                                let label_resp = ui.add(
                                    egui::Label::new(text)
                                        .truncate()
                                        .sense(egui::Sense::click()),
                                );
                                if pending.is_some() {
                                    label_resp.clone().on_hover_text(format!(
                                        "Edited (original: {})",
                                        cell.display_text()
                                    ));
                                }
                                if label_resp.double_clicked() {
                                    if pk_col_idx.is_some() {
                                        tab.editing_cell = Some((
                                            actual_row_idx,
                                            col_idx,
                                            cell.display_text(),
                                        ));
                                    } else {
                                        let raw_val = cell.display_text();
                                        let formatted_json = serde_json::from_str::<serde_json::Value>(&raw_val)
                                            .ok()
                                            .and_then(|v| serde_json::to_string_pretty(&v).ok());
                                        cell_inspector_to_open = Some(CellInspectorState {
                                            table_name: result.editable_table.clone(),
                                            column_name: result.columns[col_idx].clone(),
                                            row_index: actual_row_idx,
                                            raw_value: raw_val,
                                            formatted_json,
                                        });
                                    }
                                }

                                label_resp.context_menu(|ui| {
                                    if ui.button("🔍 Inspect Cell / JSON").clicked() {
                                        let raw_val = cell.display_text();
                                        let formatted_json = serde_json::from_str::<serde_json::Value>(&raw_val)
                                            .ok()
                                            .and_then(|v| serde_json::to_string_pretty(&v).ok());
                                        cell_inspector_to_open = Some(CellInspectorState {
                                            table_name: result.editable_table.clone(),
                                            column_name: result.columns[col_idx].clone(),
                                            row_index: actual_row_idx,
                                            raw_value: raw_val,
                                            formatted_json,
                                        });
                                        ui.close();
                                    }
                                    if ui.button("📋 Copy Value").clicked() {
                                        ui.ctx().copy_text(cell.display_text());
                                        ui.close();
                                    }
                                    if ui.button("Inspect Row (Vertical)").clicked() {
                                        tab.show_row_inspector = true;
                                        tab.row_inspector_row = Some(actual_row_idx);
                                        tab.row_inspector_buffers = row_cells
                                            .iter()
                                            .map(|c| c.display_text())
                                            .collect();
                                        ui.close();
                                    }
                                    ui.separator();
                                    if ui.button("Copy Row as JSON").clicked() {
                                        ui.ctx().copy_text(format_rows_as_json(&result, &[actual_row_idx]));
                                        copy_banner_msg = Some("Copied row as JSON.".to_string());
                                        ui.close();
                                    }
                                    if ui.button("Copy Row as SQL INSERT").clicked() {
                                        let tbl = result.editable_table.as_deref().unwrap_or("table_name");
                                        ui.ctx().copy_text(format_rows_as_sql_insert(tbl, &result, &[actual_row_idx]));
                                        copy_banner_msg = Some("Copied row as SQL INSERT.".to_string());
                                        ui.close();
                                    }
                                    if let Some(pk_col) = &result.primary_key_column
                                        && ui.button("Copy Row as SQL UPDATE").clicked() {
                                            let tbl = result.editable_table.as_deref().unwrap_or("table_name");
                                            ui.ctx().copy_text(format_rows_as_sql_update(tbl, pk_col, &result, &[actual_row_idx]));
                                            copy_banner_msg = Some("Copied row as SQL UPDATE.".to_string());
                                            ui.close();
                                        }
                                    if ui.button("Duplicate Row (SQL INSERT)").clicked() {
                                        let tbl = result.editable_table.as_deref().unwrap_or("table_name");
                                        let sql = format_rows_as_sql_insert(tbl, &result, &[actual_row_idx]);
                                        duplicate_insert_tab = Some((format!("Duplicate {tbl}"), sql));
                                        ui.close();
                                    }
                                    if let (Some(_tbl), Some(pk_col), Some(pk_idx)) = (
                                        &result.editable_table,
                                        &result.primary_key_column,
                                        pk_col_idx,
                                    ) {
                                        let delete_lbl = if is_marked_delete {
                                            "Unmark Row Delete"
                                        } else {
                                            "Mark Row for Delete"
                                        };
                                        if ui.button(delete_lbl).clicked() {
                                            if is_marked_delete {
                                                tab.pending_row_deletes
                                                    .retain(|(idx, _, _)| *idx != actual_row_idx);
                                            } else {
                                                let pk_val = row_cells[pk_idx].display_text();
                                                tab.pending_row_deletes.push((
                                                    actual_row_idx,
                                                    pk_col.clone(),
                                                    pk_val,
                                                ));
                                            }
                                            ui.close();
                                        }
                                    }

                                    // Browse Related FK Rows
                                    if !cell.is_null()
                                        && let (Some(tbl_name), Some(schema)) = (&result.editable_table, &active_schema_opt) {
                                            let col_name = &result.columns[col_idx];
                                            let raw_val = cell.display_text();
                                            let formatted_val = if raw_val.parse::<i64>().is_ok() || raw_val.parse::<f64>().is_ok() {
                                                raw_val.clone()
                                            } else {
                                                format!("'{}'", raw_val.replace('\'', "''"))
                                            };

                                            // Outgoing FKs
                                            if let Some(tbl_schema) = schema.tables.iter().find(|t| t.name.eq_ignore_ascii_case(tbl_name)) {
                                                for fk in &tbl_schema.foreign_keys {
                                                    if fk.column_name.eq_ignore_ascii_case(col_name) {
                                                        let lbl = format!("🔗 Browse `{}` ({} = {})", fk.referenced_table, fk.referenced_column, raw_val);
                                                        if ui.button(lbl).clicked() {
                                                            let sql = format!("SELECT * FROM `{}` WHERE `{}` = {} LIMIT 100;", fk.referenced_table, fk.referenced_column, formatted_val);
                                                            open_related_query = Some((format!("{} ({})", fk.referenced_table, raw_val), sql));
                                                            ui.close();
                                                        }
                                                    }
                                                }
                                            }
                                            // Incoming FKs from other tables referencing this table/column
                                            for other_tbl in &schema.tables {
                                                for fk in &other_tbl.foreign_keys {
                                                    if fk.referenced_table.eq_ignore_ascii_case(tbl_name) && fk.referenced_column.eq_ignore_ascii_case(col_name) {
                                                        let lbl = format!("🔗 Browse `{}` ({} = {})", other_tbl.name, fk.column_name, raw_val);
                                                        if ui.button(lbl).clicked() {
                                                            let sql = format!("SELECT * FROM `{}` WHERE `{}` = {} LIMIT 100;", other_tbl.name, fk.column_name, formatted_val);
                                                            open_related_query = Some((format!("{} ({})", other_tbl.name, raw_val), sql));
                                                            ui.close();
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                });
                            }
                        });
                    }
                });
            });
                });

            pending_edits = tab.pending_edits.clone();
            pending_deletes = tab.pending_row_deletes.clone();
        }

        if let Some(msg) = copy_banner_msg {
            self.status_banner = msg;
        }
        if let Some(sql) = load_all_sql {
            self.execute_sql_with_limit(sql, None);
        }
        if let Some((title, sql)) = duplicate_insert_tab {
            self.tabs.push(QueryTab::new(title, sql));
            self.active_tab_idx = self.tabs.len() - 1;
        }
        if let Some((fk_key, ref_table, ref_col)) = trigger_fk_lookup
            && let Some(session) = self.active_session.clone()
        {
            let db = self.active_database.clone();
            let tx = self.tx.clone();
            let sql = format!("SELECT * FROM `{ref_table}` ORDER BY `{ref_col}` LIMIT 100;");
            self.rt.spawn(async move {
                let res = session
                    .execute_query(&db, &sql, None, None)
                    .await
                    .map_err(|e| format!("{e:#}"));
                let _ = tx.send(AsyncEvent::FkLookupLoaded {
                    key: fk_key,
                    result: res,
                });
            });
        }
        if trigger_commit_deletes
            && let (Some(session), Some(table_name)) =
                (self.active_session.clone(), result.editable_table.clone())
        {
            let tab_id = self.tabs[self.active_tab_idx].id.clone();
            let db = self.active_database.clone();
            let tx = self.tx.clone();
            self.rt.spawn(async move {
                let mut total_deleted = 0u64;
                let mut err_out: Option<String> = None;
                for (_r_idx, pk_col, pk_val) in &pending_deletes {
                    let lit = if pk_val.parse::<i64>().is_ok() || pk_val.parse::<f64>().is_ok() {
                        pk_val.clone()
                    } else {
                        format!("'{}'", pk_val.replace('\'', "''"))
                    };
                    let sql =
                        format!("DELETE FROM `{table_name}` WHERE `{pk_col}` = {lit} LIMIT 1;");
                    match session.execute_query(&db, &sql, None, None).await {
                        Ok(r) => total_deleted += r.affected_rows.max(1),
                        Err(e) => {
                            err_out = Some(format!("{e:#}"));
                            break;
                        }
                    }
                }
                let _ = tx.send(AsyncEvent::RowDeletesApplied {
                    tab_id,
                    deleted_count: match err_out {
                        Some(e) => Err(e),
                        None => Ok(total_deleted),
                    },
                });
            });
        }
        if let Some((title, sql)) = open_related_query {
            self.tabs.push(QueryTab::new(title, sql));
            self.active_tab_idx = self.tabs.len() - 1;
            self.execute_current_tab_query(None);
        }
        if let Some(ci) = cell_inspector_to_open {
            self.cell_inspector = Some(ci);
        }

        if let Some(prompt) = trigger_ai_prompt {
            self.show_ai_panel = true;
            self.send_chat_prompt(Some(prompt));
        }

        if trigger_commit_edits
            && let (Some(session), Some(table_name)) =
                (self.active_session.clone(), result.editable_table.clone())
        {
            let tab_id = self.tabs[self.active_tab_idx].id.clone();
            let db = self.active_database.clone();
            let tx = self.tx.clone();
            self.rt.spawn(async move {
                let res = session
                    .apply_cell_edits(&db, &table_name, &pending_edits)
                    .await
                    .map_err(|e| format!("{e:#}"));
                let _ = tx.send(AsyncEvent::CellEditsApplied {
                    tab_id,
                    updated_count: res,
                });
            });
        }
    }
}
