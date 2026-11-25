use super::super::super::*;

impl MysqlRustyApp {
    pub(crate) fn render_data_diff_modal(&mut self, ctx: &egui::Context) {
        if !self.show_data_diff_modal {
            return;
        }
        let mut open = self.show_data_diff_modal;
        let mut run_diff = false;
        let mut run_batch_diff = false;
        let mut load_src_tables = false;
        let mut open_sync_in_tab = false;
        let mut open_batch_sql_in_tab: Option<(String, String)> = None;
        let mut trigger_sync_confirm: Option<(String, Option<String>, bool)> = None;
        let servers = self.diff_server_options();

        egui::Window::new("Data Diff (Row Comparison)")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .min_width(540.0)
            .min_height(340.0)
            .default_width(860.0)
            .default_height(600.0)
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("Mode:").strong());
                    ui.selectable_value(&mut self.data_diff_batch_mode, false, "Single Table");
                    ui.selectable_value(
                        &mut self.data_diff_batch_mode,
                        true,
                        "All Tables (Schema + Rows)",
                    );
                    ui.separator();

                    ui.label(RichText::new("Saved Syncs:").strong());
                    let selected_preset_label = self
                        .data_diff_selected_preset_id
                        .as_deref()
                        .and_then(|id| {
                            self.config
                                .saved_sync_presets
                                .iter()
                                .find(|p| p.id == id)
                                .map(|p| p.name.as_str())
                        })
                        .unwrap_or("Select saved sync...");
                    let mut load_preset: Option<crate::config::SavedSyncPreset> = None;
                    egui::ComboBox::from_id_salt("dd_saved_presets")
                        .selected_text(selected_preset_label)
                        .width(175.0)
                        .show_ui(ui, |ui| {
                            if self.config.saved_sync_presets.is_empty() {
                                ui.label(RichText::new("No saved syncs yet").small().weak());
                            } else {
                                for preset in &self.config.saved_sync_presets {
                                    let is_sel = self.data_diff_selected_preset_id.as_deref()
                                        == Some(preset.id.as_str());
                                    if ui.selectable_label(is_sel, &preset.name).clicked() {
                                        load_preset = Some(preset.clone());
                                    }
                                }
                            }
                        });
                    if let Some(preset) = load_preset {
                        self.data_diff_selected_preset_id = Some(preset.id.clone());
                        self.data_diff_preset_name = preset.name.clone();
                        self.data_diff_batch_mode = preset.batch_mode;
                        self.data_diff_source_db = preset.source_db.clone();
                        self.data_diff_source_table = preset.source_table.clone();
                        self.data_diff_target_db = preset.target_db.clone();
                        self.data_diff_target_table = preset.target_table.clone();
                        self.data_diff_key_col = preset.key_col;
                        self.data_diff_excluded_tables =
                            preset.excluded_tables.iter().cloned().collect();

                        let src_match = servers
                            .iter()
                            .find(|(_, t)| t.eq_ignore_ascii_case(&preset.source_server_name))
                            .and_then(|(idx, _)| *idx);
                        let tgt_match = servers
                            .iter()
                            .find(|(_, t)| t.eq_ignore_ascii_case(&preset.target_server_name))
                            .and_then(|(idx, _)| *idx);

                        if let Some(s_idx) = src_match {
                            self.data_diff_source_ws_idx = Some(s_idx);
                        }
                        if let Some(t_idx) = tgt_match {
                            self.data_diff_target_ws_idx = Some(t_idx);
                        }

                        if (!self.open_workspaces.is_empty()
                            && (src_match.is_none() || tgt_match.is_none()))
                            || self.active_session.is_none()
                        {
                            self.status_banner = format!(
                                "Loaded preset '{}', but connect to both `{}` and `{}` first.",
                                preset.name,
                                preset.source_server_name,
                                preset.target_server_name
                            );
                        } else if preset.batch_mode {
                            run_batch_diff = true;
                        } else {
                            run_diff = true;
                        }
                    }

                    if let Some(sel_id) = self.data_diff_selected_preset_id.clone()
                        && ui
                            .small_button("🗑")
                            .on_hover_text("Delete selected sync preset")
                            .clicked()
                    {
                        self.config.saved_sync_presets.retain(|p| p.id != sel_id);
                        self.data_diff_selected_preset_id = None;
                        let _ = self.config.save();
                        self.status_banner = "Deleted saved sync preset.".to_string();
                    }

                    ui.add(
                        egui::TextEdit::singleline(&mut self.data_diff_preset_name)
                            .desired_width(135.0)
                            .hint_text("Preset name..."),
                    );
                    if ui
                        .small_button("💾 Save Sync")
                        .on_hover_text("Save current Source, Target, and table selection")
                        .clicked()
                    {
                        let src_srv = self.diff_server_label(self.data_diff_source_ws_idx);
                        let tgt_srv = self.diff_server_label(self.data_diff_target_ws_idx);
                        let name = if self.data_diff_preset_name.trim().is_empty() {
                            format!(
                                "{}/{} -> {}/{}",
                                src_srv,
                                self.data_diff_source_db,
                                tgt_srv,
                                self.data_diff_target_db
                            )
                        } else {
                            self.data_diff_preset_name.trim().to_string()
                        };
                        let mut excluded: Vec<String> =
                            self.data_diff_excluded_tables.iter().cloned().collect();
                        excluded.sort_unstable();
                        let preset = crate::config::SavedSyncPreset {
                            id: uuid::Uuid::new_v4().to_string(),
                            name: name.clone(),
                            batch_mode: self.data_diff_batch_mode,
                            source_server_name: src_srv,
                            source_db: self.data_diff_source_db.clone(),
                            source_table: self.data_diff_source_table.clone(),
                            target_server_name: tgt_srv,
                            target_db: self.data_diff_target_db.clone(),
                            target_table: self.data_diff_target_table.clone(),
                            key_col: self.data_diff_key_col,
                            excluded_tables: excluded,
                        };
                        self.data_diff_selected_preset_id = Some(preset.id.clone());
                        self.data_diff_preset_name = name.clone();
                        self.config
                            .saved_sync_presets
                            .retain(|p| !p.name.eq_ignore_ascii_case(&name));
                        self.config.saved_sync_presets.push(preset);
                        let _ = self.config.save();
                        self.status_banner = format!("Saved sync preset `{name}`.");
                    }
                });
                ui.separator();

                ui.horizontal_wrapped(|ui| {
                    ui.label("Source:");
                    if servers.len() > 1 {
                        let src_srv_label = self.diff_server_label(self.data_diff_source_ws_idx);
                        egui::ComboBox::from_id_salt("dd_src_srv")
                            .selected_text(src_srv_label)
                            .width(120.0)
                            .show_ui(ui, |ui| {
                                for (idx, title) in &servers {
                                    if ui
                                        .selectable_value(
                                            &mut self.data_diff_source_ws_idx,
                                            *idx,
                                            title,
                                        )
                                        .changed()
                                    {
                                        self.data_diff_source_db =
                                            self.diff_server_active_db(*idx);
                                    }
                                }
                            });
                    }
                    let src_dbs = self.diff_server_databases(self.data_diff_source_ws_idx);
                    egui::ComboBox::from_id_salt("dd_src_db")
                        .selected_text(&self.data_diff_source_db)
                        .width(120.0)
                        .show_ui(ui, |ui| {
                            for db in &src_dbs {
                                ui.selectable_value(&mut self.data_diff_source_db, db.clone(), db);
                            }
                        });
                    if !self.data_diff_batch_mode {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.data_diff_source_table)
                                .desired_width(120.0)
                                .hint_text("source table"),
                        );
                    }

                    ui.label("Target:");
                    if servers.len() > 1 {
                        let tgt_srv_label = self.diff_server_label(self.data_diff_target_ws_idx);
                        egui::ComboBox::from_id_salt("dd_tgt_srv")
                            .selected_text(tgt_srv_label)
                            .width(120.0)
                            .show_ui(ui, |ui| {
                                for (idx, title) in &servers {
                                    if ui
                                        .selectable_value(
                                            &mut self.data_diff_target_ws_idx,
                                            *idx,
                                            title,
                                        )
                                        .changed()
                                    {
                                        self.data_diff_target_db =
                                            self.diff_server_active_db(*idx);
                                    }
                                }
                            });
                    }
                    let tgt_dbs = self.diff_server_databases(self.data_diff_target_ws_idx);
                    egui::ComboBox::from_id_salt("dd_tgt_db")
                        .selected_text(&self.data_diff_target_db)
                        .width(120.0)
                        .show_ui(ui, |ui| {
                            for db in &tgt_dbs {
                                ui.selectable_value(&mut self.data_diff_target_db, db.clone(), db);
                            }
                        });
                    if !self.data_diff_batch_mode {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.data_diff_target_table)
                                .desired_width(120.0)
                                .hint_text("target table"),
                        );
                    }

                    ui.label("Key Col #:");
                    ui.add(egui::DragValue::new(&mut self.data_diff_key_col).range(0..=30));

                    if !self.data_diff_batch_mode {
                        if ui
                            .add_enabled(!self.data_diff_loading, egui::Button::new("Compare Rows"))
                            .clicked()
                        {
                            run_diff = true;
                        }
                    } else if ui
                        .add_enabled(
                            !self.data_diff_loading,
                            egui::Button::new("Compare Selected Tables"),
                        )
                        .clicked()
                    {
                        run_batch_diff = true;
                    }
                });
                if servers.len() < 2 {
                    ui.label(
                        RichText::new("Tip: Connect to a 2nd server in the Servers tab to compare rows across different servers.")
                            .small()
                            .weak(),
                    );
                }

                if self.data_diff_batch_mode {
                    if let Some(src_schema) = self
                        .diff_server_schema(self.data_diff_source_ws_idx, &self.data_diff_source_db)
                        .cloned()
                    {
                        let mut table_names: Vec<String> =
                            src_schema.tables.iter().map(|t| t.name.clone()).collect();
                        if let Some(tgt_schema) = self.diff_server_schema(
                            self.data_diff_target_ws_idx,
                            &self.data_diff_target_db,
                        ) {
                            for t in &tgt_schema.tables {
                                if !table_names.iter().any(|x| x == &t.name) {
                                    table_names.push(t.name.clone());
                                }
                            }
                        }
                        table_names.sort_unstable();
                        let selected_count = table_names
                            .iter()
                            .filter(|t| !self.data_diff_excluded_tables.contains(*t))
                            .count();

                        egui::CollapsingHeader::new(format!(
                            "Tables to Compare ({selected_count}/{} selected)",
                            table_names.len()
                        ))
                        .default_open(self.data_diff_batch_results.is_empty())
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.data_diff_table_filter)
                                        .desired_width(160.0)
                                        .hint_text("Filter tables..."),
                                );
                                if ui.small_button("Select All").clicked() {
                                    self.data_diff_excluded_tables.clear();
                                }
                                if ui.small_button("Deselect All").clicked() {
                                    for t in &table_names {
                                        self.data_diff_excluded_tables.insert(t.clone());
                                    }
                                }
                            });
                            let filter_lower =
                                self.data_diff_table_filter.trim().to_ascii_lowercase();
                            egui::ScrollArea::vertical()
                                .id_salt("dd_batch_tables_scroll")
                                .auto_shrink([false, false])
                                .max_height(110.0)
                                .show(ui, |ui| {
                                    ui.horizontal_wrapped(|ui| {
                                        for t in &table_names {
                                            if !filter_lower.is_empty()
                                                && !t.to_ascii_lowercase().contains(&filter_lower)
                                            {
                                                continue;
                                            }
                                            let mut checked =
                                                !self.data_diff_excluded_tables.contains(t);
                                            if ui.checkbox(&mut checked, t).changed() {
                                                if checked {
                                                    self.data_diff_excluded_tables.remove(t);
                                                } else {
                                                    self.data_diff_excluded_tables
                                                        .insert(t.clone());
                                                }
                                            }
                                        }
                                    });
                                });
                        });
                    } else if !self.data_diff_source_db.is_empty() {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("Source schema not loaded yet for table list.")
                                    .small()
                                    .weak(),
                            );
                            if ui.small_button("Load Tables").clicked() {
                                load_src_tables = true;
                            }
                        });
                    }
                }
                ui.separator();

                if self.data_diff_loading {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(if self.data_diff_batch_mode {
                            "Comparing schemas and rows across selected tables..."
                        } else {
                            "Loading and comparing table rows..."
                        });
                    });
                } else if self.data_diff_batch_mode {
                    if self.data_diff_batch_results.is_empty() {
                        ui.label(
                            RichText::new("Select Source & Target servers/databases, uncheck any tables you want to skip, and click 'Compare Selected Tables'.")
                                .weak(),
                        );
                    } else {
                        let all_sync_sql: String = self
                            .data_diff_batch_results
                            .iter()
                            .filter(|e| !e.schema_sql.trim().is_empty() || !e.sync_sql.trim().is_empty())
                            .map(|e| {
                                let mut s = format!("-- === Table `{}` ===\n", e.table_name);
                                if !e.schema_sql.trim().is_empty() {
                                    s.push_str(&e.schema_sql);
                                    s.push('\n');
                                }
                                if !e.sync_sql.trim().is_empty() {
                                    s.push_str(&e.sync_sql);
                                    s.push('\n');
                                }
                                s
                            })
                            .collect::<Vec<_>>()
                            .join("\n");

                        ui.horizontal(|ui| {
                            let total = self.data_diff_batch_results.len();
                            let with_diff = self
                                .data_diff_batch_results
                                .iter()
                                .filter(|e| {
                                    e.schema_has_diff
                                        || e.error.is_some()
                                        || e.report.as_ref().is_some_and(|r| {
                                            r.modified_count > 0
                                                || r.left_only_count > 0
                                                || r.right_only_count > 0
                                        })
                                })
                                .count();
                            ui.label(
                                RichText::new(format!(
                                    "Compared {total} table(s) · {with_diff} with differences"
                                ))
                                .strong()
                                .color(Color32::from_rgb(56, 189, 248)),
                            );
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if !all_sync_sql.trim().is_empty() {
                                    if ui
                                        .add(
                                            egui::Button::new(
                                                RichText::new("⚡ Apply All Sync to Target...")
                                                    .small()
                                                    .color(Color32::WHITE)
                                                    .strong(),
                                            )
                                            .fill(Color32::from_rgb(180, 83, 9)),
                                        )
                                        .clicked()
                                    {
                                        let scope = format!(
                                            "{} table(s)",
                                            self.data_diff_batch_results.len()
                                        );
                                        trigger_sync_confirm =
                                            Some((all_sync_sql.clone(), Some(scope), true));
                                    }
                                    if ui.small_button("Open All Sync SQL in Tab").clicked() {
                                        open_batch_sql_in_tab = Some((
                                            "BatchSync".to_string(),
                                            all_sync_sql.clone(),
                                        ));
                                    }
                                    if ui.small_button("Copy All Sync SQL").clicked() {
                                        ui.ctx().copy_text(all_sync_sql.clone());
                                    }
                                }
                            });
                        });
                        ui.separator();

                        let avail_h = ui.available_height();
                        let has_preview = self
                            .data_diff_selected_batch_idx
                            .and_then(|i| self.data_diff_batch_results.get(i))
                            .is_some();
                        let results_h = if has_preview {
                            ((avail_h - 48.0) * 0.55).max(120.0)
                        } else {
                            (avail_h - 12.0).max(160.0)
                        };

                        egui::ScrollArea::vertical()
                            .id_salt("dd_batch_results_scroll")
                            .auto_shrink([false, false])
                            .max_height(results_h)
                            .show(ui, |ui| {
                                ui.set_min_width(ui.available_width());
                                for (idx, entry) in
                                    self.data_diff_batch_results.iter().enumerate()
                                {
                                    let selected =
                                        self.data_diff_selected_batch_idx == Some(idx);
                                    let data_summary = if let Some(err) = &entry.error {
                                        format!("Error: {err}")
                                    } else if let Some(rep) = &entry.report {
                                        if rep.modified_count == 0
                                            && rep.left_only_count == 0
                                            && rep.right_only_count == 0
                                        {
                                            format!("Rows OK ({} same)", rep.same_count)
                                        } else {
                                            format!(
                                                "~{} mod, +{} src, -{} tgt",
                                                rep.modified_count,
                                                rep.left_only_count,
                                                rep.right_only_count
                                            )
                                        }
                                    } else {
                                        "No data".to_string()
                                    };
                                    let row_color = if entry.error.is_some() {
                                        Color32::from_rgb(248, 113, 113)
                                    } else if entry.schema_has_diff
                                        || entry.report.as_ref().is_some_and(|r| {
                                            r.modified_count > 0
                                                || r.left_only_count > 0
                                                || r.right_only_count > 0
                                        })
                                    {
                                        Color32::from_rgb(250, 204, 21)
                                    } else {
                                        Color32::from_rgb(52, 211, 153)
                                    };
                                    let label = format!(
                                        "{}  |  Schema: {}  |  Data: {}",
                                        entry.table_name, entry.schema_status, data_summary
                                    );
                                    if ui
                                        .selectable_label(
                                            selected,
                                            RichText::new(label).small().color(row_color),
                                        )
                                        .clicked()
                                    {
                                        self.data_diff_selected_batch_idx = Some(idx);
                                    }
                                }
                            });

                        if let Some(sel_idx) = self.data_diff_selected_batch_idx
                            && let Some(entry) = self.data_diff_batch_results.get(sel_idx)
                        {
                            ui.separator();
                            let mut preview = String::new();
                            if !entry.schema_sql.trim().is_empty() {
                                preview.push_str(&entry.schema_sql);
                                preview.push('\n');
                            }
                            if !entry.sync_sql.trim().is_empty() {
                                preview.push_str(&entry.sync_sql);
                            }
                            if preview.trim().is_empty() {
                                ui.label(
                                    RichText::new(format!(
                                        "`{}`: No schema or data sync SQL needed.",
                                        entry.table_name
                                    ))
                                    .small()
                                    .weak(),
                                );
                            } else {
                                ui.label(
                                    RichText::new(format!(
                                        "Sync SQL for `{}`:",
                                        entry.table_name
                                    ))
                                    .small()
                                    .strong(),
                                );
                                let mut preview_buf = preview;
                                let preview_h = (ui.available_height() - 8.0).max(100.0);
                                egui::ScrollArea::vertical()
                                    .id_salt("dd_batch_preview_scroll")
                                    .auto_shrink([false, false])
                                    .max_height(preview_h)
                                    .show(ui, |ui| {
                                        ui.add(
                                            egui::TextEdit::multiline(&mut preview_buf)
                                                .font(egui::TextStyle::Monospace)
                                                .desired_width(f32::INFINITY),
                                        );
                                    });
                            }
                        }
                    }
                } else if let Some(report) = &self.data_diff_report {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!(
                                "Same: {} | Modified: {} | Only in Source: {} | Only in Target: {}",
                                report.same_count,
                                report.modified_count,
                                report.left_only_count,
                                report.right_only_count
                            ))
                            .strong()
                            .color(Color32::from_rgb(56, 189, 248)),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if !self.data_diff_sync_sql.trim().is_empty()
                                && ui
                                    .add(
                                        egui::Button::new(
                                            RichText::new("⚡ Apply Sync to Target...")
                                                .small()
                                                .color(Color32::WHITE)
                                                .strong(),
                                        )
                                        .fill(Color32::from_rgb(180, 83, 9)),
                                    )
                                    .clicked()
                            {
                                let scope = format!(
                                    "{} -> {}",
                                    self.data_diff_source_table, self.data_diff_target_table
                                );
                                trigger_sync_confirm = Some((
                                    self.data_diff_sync_sql.clone(),
                                    Some(scope),
                                    false,
                                ));
                            }
                            if ui.small_button("Open Sync SQL in Tab").clicked() {
                                open_sync_in_tab = true;
                            }
                            if ui.small_button("Copy Sync SQL").clicked() {
                                ui.ctx().copy_text(self.data_diff_sync_sql.clone());
                            }
                        });
                    });
                    ui.separator();

                    let avail_h = ui.available_height();
                    let rows_h = ((avail_h - 48.0) * 0.65).max(120.0);
                    egui::ScrollArea::both()
                        .id_salt("dd_single_rows_scroll")
                        .auto_shrink([false, false])
                        .max_height(rows_h)
                        .show(ui, |ui| {
                            for entry in &report.entries {
                                if entry.status == RowDiffStatus::Same {
                                    continue;
                                }
                                let (badge, color) = match entry.status {
                                    RowDiffStatus::OnlyInLeft => {
                                        ("+ SOURCE ONLY", Color32::from_rgb(52, 211, 153))
                                    }
                                    RowDiffStatus::OnlyInRight => {
                                        ("- TARGET ONLY", Color32::from_rgb(248, 113, 113))
                                    }
                                    RowDiffStatus::Modified => {
                                        ("~ MODIFIED", Color32::from_rgb(250, 204, 21))
                                    }
                                    RowDiffStatus::Same => ("SAME", Color32::GRAY),
                                };
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new(badge).small().strong().color(color));
                                    ui.label(RichText::new(&entry.key_display).strong());
                                    if let Some(l) = &entry.left_row {
                                        ui.label(
                                            RichText::new(format!("Src: {}", l.join(" | "))).small(),
                                        );
                                    }
                                    if let Some(r) = &entry.right_row {
                                        ui.label(
                                            RichText::new(format!("Tgt: {}", r.join(" | ")))
                                                .small()
                                                .weak(),
                                        );
                                    }
                                });
                            }
                        });

                    ui.separator();
                    ui.label(RichText::new("Synchronization SQL Preview:").small().strong());
                    let sql_h = (ui.available_height() - 8.0).max(90.0);
                    egui::ScrollArea::vertical()
                        .id_salt("dd_single_sql_scroll")
                        .auto_shrink([false, false])
                        .max_height(sql_h)
                        .show(ui, |ui| {
                            ui.add(
                                egui::TextEdit::multiline(&mut self.data_diff_sync_sql)
                                    .font(egui::TextStyle::Monospace)
                                    .desired_width(f32::INFINITY),
                            );
                        });
                } else {
                    ui.label(
                        RichText::new("Select source & target databases and table names, then click Compare Rows.")
                            .weak(),
                    );
                }
                let remaining_h = ui.available_height();
                if remaining_h > 0.0 {
                    ui.allocate_space(egui::vec2(0.0, remaining_h));
                }
            });

        self.show_data_diff_modal = open;

        if load_src_tables && !self.data_diff_source_db.is_empty() {
            let ws_idx = self.data_diff_source_ws_idx;
            let db = self.data_diff_source_db.clone();
            self.spawn_load_workspace_schema(ws_idx, db);
        }

        if open_sync_in_tab {
            self.tabs.push(QueryTab::new(
                format!("DataSync {}", self.data_diff_target_table),
                self.data_diff_sync_sql.clone(),
            ));
            self.active_tab_idx = self.tabs.len() - 1;
            self.show_data_diff_modal = false;
        }

        if let Some((title, sql)) = open_batch_sql_in_tab {
            self.tabs.push(QueryTab::new(title, sql));
            self.active_tab_idx = self.tabs.len() - 1;
            self.show_data_diff_modal = false;
        }

        if let Some((sql, scope, reload_schema)) = trigger_sync_confirm {
            let src_ws = self.data_diff_source_ws_idx;
            let src_db = self.data_diff_source_db.clone();
            let tgt_ws = self.data_diff_target_ws_idx;
            let tgt_db = self.data_diff_target_db.clone();
            self.open_db_sync_confirm(
                "Confirm Data & Schema Sync",
                src_ws,
                &src_db,
                tgt_ws,
                &tgt_db,
                scope.as_deref(),
                sql,
                reload_schema,
            );
        }

        if run_diff
            && !self.data_diff_source_table.trim().is_empty()
            && !self.data_diff_target_table.trim().is_empty()
            && let (Some(src_session), Some(tgt_session)) = (
                self.diff_server_session(self.data_diff_source_ws_idx),
                self.diff_server_session(self.data_diff_target_ws_idx),
            )
        {
            self.data_diff_loading = true;
            let src_db = self.data_diff_source_db.clone();
            let src_tbl = self.data_diff_source_table.clone();
            let tgt_db = self.data_diff_target_db.clone();
            let tgt_tbl = self.data_diff_target_table.clone();
            let key_col = self.data_diff_key_col;
            let tx = self.tx.clone();
            self.rt.spawn(async move {
                let sql_l = format!("SELECT * FROM `{src_db}`.`{src_tbl}` ORDER BY 1 LIMIT 2000;");
                let sql_r = format!("SELECT * FROM `{tgt_db}`.`{tgt_tbl}` ORDER BY 1 LIMIT 2000;");
                let (left_res, right_res) = tokio::join!(
                    src_session.execute_query(&src_db, &sql_l, None, None),
                    tgt_session.execute_query(&tgt_db, &sql_r, None, None),
                );
                let _ = tx.send(AsyncEvent::DataDiffLoaded {
                    left: left_res.map_err(|e| format!("{e:#}")),
                    right: right_res.map_err(|e| format!("{e:#}")),
                    key_col_idx: key_col,
                });
            });
        }

        if run_batch_diff
            && !self.data_diff_source_db.trim().is_empty()
            && !self.data_diff_target_db.trim().is_empty()
            && let (Some(src_session), Some(tgt_session)) = (
                self.diff_server_session(self.data_diff_source_ws_idx),
                self.diff_server_session(self.data_diff_target_ws_idx),
            )
        {
            self.data_diff_loading = true;
            self.data_diff_batch_results.clear();
            self.data_diff_selected_batch_idx = None;
            let src_db = self.data_diff_source_db.clone();
            let tgt_db = self.data_diff_target_db.clone();
            let cached_src_schema = self
                .diff_server_schema(self.data_diff_source_ws_idx, &src_db)
                .cloned();
            let cached_tgt_schema = self
                .diff_server_schema(self.data_diff_target_ws_idx, &tgt_db)
                .cloned();
            let excluded = self.data_diff_excluded_tables.clone();
            let key_col = self.data_diff_key_col;
            let tx = self.tx.clone();

            self.rt.spawn(async move {
                let src_schema = match cached_src_schema {
                    Some(s) => Ok(s),
                    None => src_session
                        .introspect_database(&src_db)
                        .await
                        .map_err(|e| format!("{e:#}")),
                };
                let tgt_schema = match cached_tgt_schema {
                    Some(s) => Ok(s),
                    None => tgt_session
                        .introspect_database(&tgt_db)
                        .await
                        .map_err(|e| format!("{e:#}")),
                };

                let (Ok(src_s), Ok(tgt_s)) = (src_schema, tgt_schema) else {
                    let _ = tx.send(AsyncEvent::BatchDataDiffLoaded {
                        entries: Vec::new(),
                    });
                    return;
                };

                let mut table_names: Vec<String> =
                    src_s.tables.iter().map(|t| t.name.clone()).collect();
                for t in &tgt_s.tables {
                    if !table_names.iter().any(|x| x == &t.name) {
                        table_names.push(t.name.clone());
                    }
                }
                table_names.sort_unstable();
                table_names.retain(|t| !excluded.contains(t));

                let mut entries = Vec::with_capacity(table_names.len());
                for tbl in table_names {
                    let mut one_set = std::collections::HashSet::new();
                    one_set.insert(tbl.clone());
                    let s_diff =
                        crate::schema_diff::diff_schemas_filtered(&src_s, &tgt_s, Some(&one_set));
                    let in_src = src_s.tables.iter().any(|t| t.name == tbl);
                    let in_tgt = tgt_s.tables.iter().any(|t| t.name == tbl);
                    let schema_has_diff =
                        s_diff.summary.iter().any(|line| !line.starts_with("[OK]"));
                    let schema_status = if !schema_has_diff {
                        "OK".to_string()
                    } else if !in_tgt {
                        "Missing in Target".to_string()
                    } else if !in_src {
                        "Only in Target".to_string()
                    } else {
                        format!("{} change(s)", s_diff.summary.len())
                    };
                    let schema_sql = if schema_has_diff {
                        s_diff.migration_sql
                    } else {
                        String::new()
                    };

                    if !in_src || !in_tgt {
                        entries.push(BatchTableDiffEntry {
                            table_name: tbl,
                            schema_status,
                            schema_has_diff,
                            schema_sql,
                            report: None,
                            sync_sql: String::new(),
                            error: None,
                        });
                        continue;
                    }

                    let tbl_meta = src_s.tables.iter().find(|t| t.name == tbl);
                    let pk_indices: Vec<usize> = tbl_meta
                        .map(|t| {
                            t.columns
                                .iter()
                                .enumerate()
                                .filter(|(_, c)| c.is_primary_key)
                                .map(|(i, _)| i)
                                .collect::<Vec<_>>()
                        })
                        .filter(|v| !v.is_empty())
                        .unwrap_or_else(|| vec![key_col]);
                    let generated_cols: std::collections::HashSet<String> = tbl_meta
                        .map(|t| {
                            t.columns
                                .iter()
                                .filter(|c| c.extra.to_ascii_uppercase().contains("GENERATED"))
                                .map(|c| c.name.clone())
                                .collect()
                        })
                        .unwrap_or_default();
                    let order_by = tbl_meta
                        .map(|t| {
                            let pk_names: Vec<String> = pk_indices
                                .iter()
                                .filter_map(|&i| t.columns.get(i).map(|c| format!("`{}`", c.name)))
                                .collect();
                            if pk_names.is_empty() {
                                "1".to_string()
                            } else {
                                pk_names.join(", ")
                            }
                        })
                        .unwrap_or_else(|| "1".to_string());

                    let sql_l =
                        format!("SELECT * FROM `{src_db}`.`{tbl}` ORDER BY {order_by} LIMIT 2000;");
                    let sql_r =
                        format!("SELECT * FROM `{tgt_db}`.`{tbl}` ORDER BY {order_by} LIMIT 2000;");
                    let (left_res, right_res) = tokio::join!(
                        src_session.execute_query(&src_db, &sql_l, None, None),
                        tgt_session.execute_query(&tgt_db, &sql_r, None, None),
                    );
                    match (left_res, right_res) {
                        (Ok(l), Ok(r)) => {
                            let rep = crate::data_diff::compute_data_diff_with_options(
                                &l,
                                &r,
                                &pk_indices,
                                &generated_cols,
                            );
                            let sync_sql =
                                crate::data_diff::generate_data_diff_upsert_sql_with_keys(
                                    &tbl,
                                    &rep,
                                    &pk_indices,
                                    &generated_cols,
                                );
                            entries.push(BatchTableDiffEntry {
                                table_name: tbl,
                                schema_status,
                                schema_has_diff,
                                schema_sql,
                                report: Some(rep),
                                sync_sql,
                                error: None,
                            });
                        }
                        (Err(e), _) | (_, Err(e)) => {
                            entries.push(BatchTableDiffEntry {
                                table_name: tbl,
                                schema_status,
                                schema_has_diff,
                                schema_sql,
                                report: None,
                                sync_sql: String::new(),
                                error: Some(format!("{e:#}")),
                            });
                        }
                    }
                }

                let _ = tx.send(AsyncEvent::BatchDataDiffLoaded { entries });
            });
        }
    }

    pub(crate) fn render_db_sync_confirm_modal(&mut self, ctx: &egui::Context) {
        let Some(sync) = self.pending_db_sync.clone() else {
            return;
        };
        let pal = self.effective_palette(ctx);
        let mut open = true;
        let mut execute_sync = false;
        let mut cancel_sync = false;

        egui::Window::new(sync.title.clone())
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(640.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                pal.compact_card_frame().show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("SOURCE (FROM):")
                                .small()
                                .strong()
                                .color(pal.accent_text),
                        );
                        ui.label(
                            RichText::new(&sync.source_label)
                                .strong()
                                .color(pal.text_primary),
                        );
                    });
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new("TARGET (TO — WILL BE MODIFIED):")
                                .small()
                                .strong()
                                .color(pal.warning),
                        );
                        ui.label(
                            RichText::new(&sync.target_label)
                                .strong()
                                .color(pal.warning),
                        );
                    });
                    ui.label(
                        RichText::new(format!(
                            "Transactional execution (START TRANSACTION ... COMMIT, {} statement(s))",
                            sync.statement_count
                        ))
                        .small()
                        .color(pal.muted_text),
                    );
                });

                ui.add_space(6.0);
                if !sync.target_is_local {
                    pal.banner_frame(pal.danger_bg, pal.danger).show(ui, |ui| {
                        ui.label(
                            RichText::new("BLOCKED: Target must be a 'local' or 'localhost' server to prevent overwriting dev/uat/prod.")
                                .color(pal.text_primary)
                                .strong(),
                        );
                    });
                    ui.add_space(4.0);
                } else if sync.target_is_read_only {
                    ui.colored_label(
                        pal.danger,
                        "BLOCKED: Target connection is in Read-Only mode.",
                    );
                    ui.add_space(4.0);
                } else if sync.target_is_prod {
                    ui.colored_label(
                        pal.danger,
                        "WARNING: Target connection is marked as PRODUCTION.",
                    );
                    ui.add_space(4.0);
                }

                ui.label(RichText::new("SQL to execute on Target:").small().strong());
                let mut sql_preview = sync.sql.clone();
                let sync_sql_h = (ui.available_height() - 68.0).max(140.0);
                egui::ScrollArea::vertical()
                    .id_salt("db_sync_confirm_sql_scroll")
                    .auto_shrink([false, false])
                    .max_height(sync_sql_h)
                    .show(ui, |ui| {
                        ui.add(
                            egui::TextEdit::multiline(&mut sql_preview)
                                .font(egui::TextStyle::Monospace)
                                .desired_width(f32::INFINITY),
                        );
                    });

                ui.add_space(6.0);
                if self.db_sync_running {
                    let (done, total, current_stmt) = self
                        .db_sync_progress
                        .clone()
                        .unwrap_or((0, sync.statement_count.max(1), "Starting transaction...".to_string()));
                    let progress = if total > 0 {
                        (done as f32 / total as f32).clamp(0.0, 1.0)
                    } else {
                        0.0
                    };
                    ui.add(
                        egui::ProgressBar::new(progress)
                            .show_percentage()
                            .text(format!("Executing {done}/{total}")),
                    );
                    ui.label(
                        RichText::new(current_stmt)
                            .small()
                            .monospace()
                            .color(pal.muted_text),
                    );
                } else {
                    let can_run = sync.target_is_local && !sync.target_is_read_only;
                    ui.add_enabled_ui(can_run, |ui| {
                        ui.checkbox(
                            &mut self.db_sync_confirm_checked,
                            format!("I reviewed Source -> Target and confirm executing on `{}`", sync.target_label),
                        );
                    });
                    ui.add_space(6.0);
                    ui.separator();
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let ready = can_run && self.db_sync_confirm_checked;
                        if ui
                            .add_enabled(
                                ready,
                                pal.danger_button("Confirm & Execute on Target"),
                            )
                            .clicked()
                        {
                            execute_sync = true;
                        }
                        if ui.button("Cancel").clicked() {
                            cancel_sync = true;
                        }
                    });
                }
            });

        if !open || cancel_sync {
            if !self.db_sync_running {
                self.pending_db_sync = None;
                self.db_sync_confirm_checked = false;
            }
            return;
        }

        if execute_sync
            && sync.target_is_local
            && !sync.target_is_read_only
            && let Some(tgt_session) = self.diff_server_session(sync.target_ws_idx)
        {
            self.db_sync_running = true;
            let stmts: Vec<String> = split_sql_statements(&sync.sql)
                .into_iter()
                .filter(|s| {
                    s.lines().any(|line| {
                        let t = line.trim();
                        !t.is_empty() && !t.starts_with("--")
                    })
                })
                .collect();
            let total = stmts.len();
            self.db_sync_progress = Some((0, total, "START TRANSACTION;".to_string()));
            let tgt_ws_idx = sync.target_ws_idx;
            let tgt_db = sync.target_db.clone();
            let reload_schema = sync.reload_target_schema;
            let tx = self.tx.clone();

            self.rt.spawn(async move {
                // Disable FK checks for the duration of the sync transaction so
                // multi-table or self-referencing inserts/updates/deletes don't fail
                // due to table/row ordering, and always restore them afterwards.
                let (fk_off_sql, fk_on_sql): (Option<&str>, Option<&str>) =
                    match tgt_session.as_ref() {
                        crate::db::ActiveSession::RealMysql { .. } => (
                            Some("SET FOREIGN_KEY_CHECKS = 0;"),
                            Some("SET FOREIGN_KEY_CHECKS = 1;"),
                        ),
                        crate::db::ActiveSession::Sqlite { .. } => (
                            Some("PRAGMA foreign_keys = OFF;"),
                            Some("PRAGMA foreign_keys = ON;"),
                        ),
                        _ => (None, None),
                    };

                if let Some(sql) = fk_off_sql
                    && let Err(e) = tgt_session.execute_query(&tgt_db, sql, None, None).await
                {
                    let _ = tx.send(AsyncEvent::DbSyncFinished {
                        target_ws_idx: tgt_ws_idx,
                        target_db: tgt_db,
                        reload_schema,
                        result: Err(format!("Failed to disable foreign key checks: {e:#}")),
                    });
                    return;
                }

                let is_ddl = |stmt: &str| {
                    let upper = stmt
                        .lines()
                        .map(|l| l.trim())
                        .find(|l| !l.is_empty() && !l.starts_with("--"))
                        .unwrap_or("")
                        .to_ascii_uppercase();
                    upper.starts_with("CREATE ")
                        || upper.starts_with("ALTER ")
                        || upper.starts_with("DROP ")
                        || upper.starts_with("TRUNCATE ")
                        || upper.starts_with("RENAME ")
                };

                let (ddl_stmts, dml_stmts): (Vec<String>, Vec<String>) =
                    stmts.into_iter().partition(|s| is_ddl(s));

                let mut executed = 0usize;
                for stmt in ddl_stmts {
                    let preview: String = stmt.chars().take(90).collect();
                    let _ = tx.send(AsyncEvent::DbSyncProgress {
                        done: executed,
                        total,
                        current_stmt: format!("[DDL] {preview}"),
                    });
                    if let Err(e) = tgt_session.execute_query(&tgt_db, &stmt, None, None).await {
                        if let Some(sql) = fk_on_sql {
                            let _ = tgt_session.execute_query(&tgt_db, sql, None, None).await;
                        }
                        let _ = tx.send(AsyncEvent::DbSyncFinished {
                            target_ws_idx: tgt_ws_idx,
                            target_db: tgt_db,
                            reload_schema,
                            result: Err(format!(
                                "DDL failed at statement {}/{total}: {e:#}",
                                executed + 1
                            )),
                        });
                        return;
                    }
                    executed += 1;
                    let _ = tx.send(AsyncEvent::DbSyncProgress {
                        done: executed,
                        total,
                        current_stmt: "Done".to_string(),
                    });
                }

                if !dml_stmts.is_empty() {
                    let begin_sql = match tgt_session.dialect() {
                        crate::dialect::Dialect::Sqlite => "BEGIN TRANSACTION;",
                        _ => "START TRANSACTION;",
                    };
                    if let Err(e) = tgt_session
                        .execute_query(&tgt_db, begin_sql, None, None)
                        .await
                    {
                        if let Some(sql) = fk_on_sql {
                            let _ = tgt_session.execute_query(&tgt_db, sql, None, None).await;
                        }
                        let _ = tx.send(AsyncEvent::DbSyncFinished {
                            target_ws_idx: tgt_ws_idx,
                            target_db: tgt_db,
                            reload_schema,
                            result: Err(format!("Failed to start transaction: {e:#}")),
                        });
                        return;
                    }

                    for stmt in dml_stmts {
                        let preview: String = stmt.chars().take(90).collect();
                        let _ = tx.send(AsyncEvent::DbSyncProgress {
                            done: executed,
                            total,
                            current_stmt: preview,
                        });
                        if let Err(e) = tgt_session.execute_query(&tgt_db, &stmt, None, None).await
                        {
                            let _ = tgt_session
                                .execute_query(&tgt_db, "ROLLBACK;", None, None)
                                .await;
                            if let Some(sql) = fk_on_sql {
                                let _ = tgt_session.execute_query(&tgt_db, sql, None, None).await;
                            }
                            let _ = tx.send(AsyncEvent::DbSyncFinished {
                                target_ws_idx: tgt_ws_idx,
                                target_db: tgt_db,
                                reload_schema,
                                result: Err(format!(
                                    "Rolled back DML at statement {}/{total}: {e:#}",
                                    executed + 1
                                )),
                            });
                            return;
                        }
                        executed += 1;
                        let _ = tx.send(AsyncEvent::DbSyncProgress {
                            done: executed,
                            total,
                            current_stmt: "Done".to_string(),
                        });
                    }

                    let commit_res = tgt_session
                        .execute_query(&tgt_db, "COMMIT;", None, None)
                        .await;
                    if let Some(sql) = fk_on_sql {
                        let _ = tgt_session.execute_query(&tgt_db, sql, None, None).await;
                    }
                    if let Err(e) = commit_res {
                        let _ = tx.send(AsyncEvent::DbSyncFinished {
                            target_ws_idx: tgt_ws_idx,
                            target_db: tgt_db,
                            reload_schema,
                            result: Err(format!("Failed to commit transaction: {e:#}")),
                        });
                        return;
                    }
                } else if let Some(sql) = fk_on_sql {
                    let _ = tgt_session.execute_query(&tgt_db, sql, None, None).await;
                }

                let _ = tx.send(AsyncEvent::DbSyncFinished {
                    target_ws_idx: tgt_ws_idx,
                    target_db: tgt_db,
                    reload_schema,
                    result: Ok(executed),
                });
            });
        }
    }
}
