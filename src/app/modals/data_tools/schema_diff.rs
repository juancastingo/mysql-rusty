use super::super::super::*;

impl MysqlRustyApp {
    pub(crate) fn render_schema_diff_modal(&mut self, ctx: &egui::Context) {
        if !self.show_schema_diff_modal {
            return;
        }
        let mut open = self.show_schema_diff_modal;
        let mut open_in_tab_sql = None;
        let mut sync_sql_to_confirm: Option<String> = None;
        let servers = self.diff_server_options();

        egui::Window::new("Schema Diff & DDL Synchronization")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .min_width(520.0)
            .min_height(320.0)
            .default_width(720.0)
            .default_height(480.0)
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("Source:").strong());
                    if servers.len() > 1 {
                        let src_srv_label = self.diff_server_label(self.diff_source_ws_idx);
                        egui::ComboBox::from_id_salt("diff_src_srv_cb")
                            .selected_text(src_srv_label)
                            .width(120.0)
                            .show_ui(ui, |ui| {
                                for (idx, title) in &servers {
                                    if ui
                                        .selectable_value(
                                            &mut self.diff_source_ws_idx,
                                            *idx,
                                            title,
                                        )
                                        .changed()
                                    {
                                        self.diff_source_db = self.diff_server_active_db(*idx);
                                    }
                                }
                            });
                    }
                    let src_dbs = self.diff_server_databases(self.diff_source_ws_idx);
                    egui::ComboBox::from_id_salt("diff_src_cb")
                        .selected_text(if self.diff_source_db.is_empty() {
                            "Select Source..."
                        } else {
                            &self.diff_source_db
                        })
                        .show_ui(ui, |ui| {
                            for db in &src_dbs {
                                ui.selectable_value(&mut self.diff_source_db, db.clone(), db);
                            }
                        });

                    ui.label(RichText::new("Target:").strong());
                    if servers.len() > 1 {
                        let tgt_srv_label = self.diff_server_label(self.diff_target_ws_idx);
                        egui::ComboBox::from_id_salt("diff_tgt_srv_cb")
                            .selected_text(tgt_srv_label)
                            .width(120.0)
                            .show_ui(ui, |ui| {
                                for (idx, title) in &servers {
                                    if ui
                                        .selectable_value(
                                            &mut self.diff_target_ws_idx,
                                            *idx,
                                            title,
                                        )
                                        .changed()
                                    {
                                        self.diff_target_db = self.diff_server_active_db(*idx);
                                    }
                                }
                            });
                    }
                    let tgt_dbs = self.diff_server_databases(self.diff_target_ws_idx);
                    egui::ComboBox::from_id_salt("diff_tgt_cb")
                        .selected_text(if self.diff_target_db.is_empty() {
                            "Select Target..."
                        } else {
                            &self.diff_target_db
                        })
                        .show_ui(ui, |ui| {
                            for db in &tgt_dbs {
                                ui.selectable_value(&mut self.diff_target_db, db.clone(), db);
                            }
                        });

                    let mut load_missing = Vec::new();
                    if ui
                        .add_enabled(!self.diff_loading_schemas, egui::Button::new("Compare"))
                        .clicked()
                    {
                        let src_opt = self
                            .diff_server_schema(self.diff_source_ws_idx, &self.diff_source_db)
                            .cloned();
                        let tgt_opt = self
                            .diff_server_schema(self.diff_target_ws_idx, &self.diff_target_db)
                            .cloned();
                        if let (Some(src), Some(tgt)) = (src_opt, tgt_opt) {
                            let included = self.diff_included_table_set(&src, &tgt);
                            self.current_diff_report = Some(
                                crate::schema_diff::diff_schemas_filtered(
                                    &src,
                                    &tgt,
                                    Some(&included),
                                ),
                            );
                        } else {
                            if self
                                .diff_server_schema(self.diff_source_ws_idx, &self.diff_source_db)
                                .is_none()
                                && !self.diff_source_db.is_empty()
                            {
                                load_missing
                                    .push((self.diff_source_ws_idx, self.diff_source_db.clone()));
                            }
                            if self
                                .diff_server_schema(self.diff_target_ws_idx, &self.diff_target_db)
                                .is_none()
                                && !self.diff_target_db.is_empty()
                            {
                                load_missing
                                    .push((self.diff_target_ws_idx, self.diff_target_db.clone()));
                            }
                        }
                    }
                    if self.diff_loading_schemas {
                        ui.spinner();
                        ui.label(RichText::new("Loading schema...").small().weak());
                    }
                    for (ws_idx, db) in load_missing {
                        self.spawn_load_workspace_schema(ws_idx, db);
                    }
                });

                // Optional table filter checkboxes (all tables included by default)
                if let (Some(src), Some(tgt)) = (
                    self.diff_server_schema(self.diff_source_ws_idx, &self.diff_source_db)
                        .cloned(),
                    self.diff_server_schema(self.diff_target_ws_idx, &self.diff_target_db)
                        .cloned(),
                ) {
                    let mut all_tables: Vec<String> = src
                        .tables
                        .iter()
                        .chain(tgt.tables.iter())
                        .map(|t| t.name.clone())
                        .collect();
                    all_tables.sort_unstable();
                    all_tables.dedup();
                    let selected_count = all_tables
                        .iter()
                        .filter(|t| !self.diff_excluded_tables.contains(*t))
                        .count();
                    egui::CollapsingHeader::new(format!(
                        "Tables to Compare ({selected_count}/{} selected)",
                        all_tables.len()
                    ))
                    .default_open(false)
                    .show(ui, |ui| {
                        let mut changed = false;
                        ui.horizontal(|ui| {
                            ui.add(
                                egui::TextEdit::singleline(&mut self.diff_table_filter)
                                    .desired_width(160.0)
                                    .hint_text("Filter tables..."),
                            );
                            if ui.small_button("Select All").clicked() {
                                self.diff_excluded_tables.clear();
                                changed = true;
                            }
                            if ui.small_button("Deselect All").clicked() {
                                for t in &all_tables {
                                    self.diff_excluded_tables.insert(t.clone());
                                }
                                changed = true;
                            }
                        });
                        let filter_lower = self.diff_table_filter.trim().to_ascii_lowercase();
                        egui::ScrollArea::vertical()
                            .max_height(110.0)
                            .show(ui, |ui| {
                                ui.horizontal_wrapped(|ui| {
                                    for t in &all_tables {
                                        if !filter_lower.is_empty()
                                            && !t.to_ascii_lowercase().contains(&filter_lower)
                                        {
                                            continue;
                                        }
                                        let mut checked = !self.diff_excluded_tables.contains(t);
                                        if ui.checkbox(&mut checked, t).changed() {
                                            if checked {
                                                self.diff_excluded_tables.remove(t);
                                            } else {
                                                self.diff_excluded_tables.insert(t.clone());
                                            }
                                            changed = true;
                                        }
                                    }
                                });
                            });
                        if changed && self.current_diff_report.is_some() {
                            let included = self.diff_included_table_set(&src, &tgt);
                            self.current_diff_report = Some(
                                crate::schema_diff::diff_schemas_filtered(
                                    &src,
                                    &tgt,
                                    Some(&included),
                                ),
                            );
                        }
                    });
                }
                if servers.len() < 2 {
                    ui.label(
                        RichText::new("Tip: Connect to a 2nd server in the Servers tab to compare schemas across servers.")
                            .small()
                            .weak(),
                    );
                }
                ui.separator();

                if let Some(report) = &self.current_diff_report {
                    ui.label(
                        RichText::new(format!(
                            "Diff Summary ({} changes detected):",
                            report.summary.len()
                        ))
                        .strong(),
                    );
                    let avail_h = ui.available_height();
                    let summary_h = ((avail_h - 80.0) * 0.35).max(100.0);
                    egui::ScrollArea::vertical()
                        .id_salt("sd_summary_scroll")
                        .auto_shrink([false, false])
                        .max_height(summary_h)
                        .show(ui, |ui| {
                            for item in &report.summary {
                                let color = if item.starts_with('+') {
                                    Color32::from_rgb(52, 211, 153)
                                } else if item.starts_with('~') {
                                    Color32::from_rgb(250, 204, 21)
                                } else if item.starts_with('-') {
                                    Color32::from_rgb(251, 146, 60)
                                } else {
                                    Color32::from_rgb(148, 163, 184)
                                };
                                ui.colored_label(color, item);
                            }
                        });

                    ui.separator();
                    ui.label(RichText::new("Generated DDL Migration SQL:").strong());
                    let mut sql_view = report.migration_sql.as_str();
                    let sql_h = (ui.available_height() - 42.0).max(120.0);
                    egui::ScrollArea::vertical()
                        .id_salt("sd_sql_scroll")
                        .auto_shrink([false, false])
                        .max_height(sql_h)
                        .show(ui, |ui| {
                            ui.add(
                                egui::TextEdit::multiline(&mut sql_view)
                                    .font(egui::TextStyle::Monospace)
                                    .code_editor()
                                    .desired_width(f32::INFINITY),
                            );
                        });

                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui.button("Copy Migration Script").clicked() {
                            ui.ctx().copy_text(report.migration_sql.clone());
                            self.status_banner = "Copied migration DDL to clipboard.".to_string();
                        }
                        if ui.button("▶ Open in Query Tab").clicked() {
                            open_in_tab_sql = Some(report.migration_sql.clone());
                        }
                        if ui.button("📄 Versioned Migration").clicked() {
                            open_in_tab_sql = Some(report.generate_migration_file_content());
                        }
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new("⚡ Apply Sync to Target...")
                                        .color(Color32::WHITE)
                                        .strong(),
                                )
                                .fill(Color32::from_rgb(180, 83, 9)),
                            )
                            .clicked()
                        {
                            sync_sql_to_confirm = Some(report.migration_sql.clone());
                        }
                    });
                } else {
                    ui.vertical_centered(|ui| {
                        ui.add_space(30.0);
                        ui.label(RichText::new("Select Source and Target databases above, then click 'Compare' to generate a synchronization DDL script.").weak());
                    });
                }
                let remaining_h = ui.available_height();
                if remaining_h > 0.0 {
                    ui.allocate_space(egui::vec2(0.0, remaining_h));
                }
            });

        if let Some(sql) = sync_sql_to_confirm {
            let src_ws = self.diff_source_ws_idx;
            let src_db = self.diff_source_db.clone();
            let tgt_ws = self.diff_target_ws_idx;
            let tgt_db = self.diff_target_db.clone();
            self.open_db_sync_confirm(
                "Confirm Schema Sync",
                src_ws,
                &src_db,
                tgt_ws,
                &tgt_db,
                None,
                sql,
                true,
            );
        } else if let Some(sql) = open_in_tab_sql {
            self.show_schema_diff_modal = false;
            let n = self.tabs.len() + 1;
            self.tabs
                .push(QueryTab::new(format!("Migration Sync {n}"), sql));
            self.active_tab_idx = self.tabs.len() - 1;
        } else if !open {
            self.show_schema_diff_modal = false;
        }
    }
}
