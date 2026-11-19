use super::super::*;

impl MysqlRustyApp {
    fn diff_server_options(&self) -> Vec<(Option<usize>, String)> {
        if self.open_workspaces.is_empty() {
            if self.active_session.is_some() {
                vec![(None, "Active Server".to_string())]
            } else {
                Vec::new()
            }
        } else {
            self.open_workspaces
                .iter()
                .enumerate()
                .map(|(i, ws)| (Some(i), ws.title.clone()))
                .collect()
        }
    }

    fn diff_server_label(&self, ws_idx: Option<usize>) -> String {
        let resolved = ws_idx.or(self.active_workspace_idx);
        if let Some(idx) = resolved
            && let Some(ws) = self.open_workspaces.get(idx)
        {
            return ws.title.clone();
        }
        "Active Server".to_string()
    }

    fn diff_server_databases(&self, ws_idx: Option<usize>) -> Vec<String> {
        let resolved = ws_idx.or(self.active_workspace_idx);
        if resolved == self.active_workspace_idx {
            return self.databases.clone();
        }
        if let Some(idx) = resolved
            && let Some(ws) = self.open_workspaces.get(idx)
        {
            return ws.databases.clone();
        }
        self.databases.clone()
    }

    fn diff_server_active_db(&self, ws_idx: Option<usize>) -> String {
        let resolved = ws_idx.or(self.active_workspace_idx);
        if resolved == self.active_workspace_idx {
            return self.active_database.clone();
        }
        if let Some(idx) = resolved
            && let Some(ws) = self.open_workspaces.get(idx)
        {
            return ws.active_database.clone();
        }
        self.active_database.clone()
    }

    pub(crate) fn diff_server_schema(
        &self,
        ws_idx: Option<usize>,
        db: &str,
    ) -> Option<&DatabaseSchema> {
        let resolved = ws_idx.or(self.active_workspace_idx);
        if resolved == self.active_workspace_idx {
            return self.schemas.get(db);
        }
        if let Some(idx) = resolved
            && let Some(ws) = self.open_workspaces.get(idx)
        {
            return ws.schemas.get(db);
        }
        None
    }

    fn diff_server_session(&self, ws_idx: Option<usize>) -> Option<Arc<ActiveSession>> {
        let resolved = ws_idx.or(self.active_workspace_idx);
        if resolved == self.active_workspace_idx {
            return self.active_session.clone();
        }
        if let Some(idx) = resolved
            && let Some(ws) = self.open_workspaces.get(idx)
        {
            return Some(ws.session.clone());
        }
        self.active_session.clone()
    }

    pub(crate) fn diff_included_table_set(
        &self,
        src: &DatabaseSchema,
        tgt: &DatabaseSchema,
    ) -> std::collections::HashSet<String> {
        let mut included = std::collections::HashSet::new();
        for t in &src.tables {
            if !self.diff_excluded_tables.contains(&t.name) {
                included.insert(t.name.clone());
            }
        }
        for t in &tgt.tables {
            if !self.diff_excluded_tables.contains(&t.name) {
                included.insert(t.name.clone());
            }
        }
        included
    }

    pub(crate) fn spawn_load_workspace_schema(&mut self, ws_idx: Option<usize>, db: String) {
        let Some(session) = self.diff_server_session(ws_idx) else {
            return;
        };
        if db.trim().is_empty() {
            return;
        }
        self.diff_loading_schemas = true;
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let res = session
                .introspect_database(&db)
                .await
                .map_err(|e| format!("{e:#}"));
            let _ = tx.send(AsyncEvent::WorkspaceSchemaLoaded {
                ws_idx,
                db,
                schema: res,
            });
        });
    }

    fn diff_server_profile(&self, ws_idx: Option<usize>) -> Option<&ConnectionProfile> {
        let resolved = ws_idx.or(self.active_workspace_idx);
        let profile_id = if let Some(idx) = resolved
            && let Some(ws) = self.open_workspaces.get(idx)
        {
            Some(ws.profile_id.as_str())
        } else {
            self.active_profile_id.as_deref()
        }?;
        self.config.connections.iter().find(|c| c.id == profile_id)
    }

    #[allow(clippy::too_many_arguments)]
    fn open_db_sync_confirm(
        &mut self,
        title: impl Into<String>,
        src_ws_idx: Option<usize>,
        src_db: &str,
        tgt_ws_idx: Option<usize>,
        tgt_db: &str,
        extra_scope: Option<&str>,
        sql: String,
        reload_target_schema: bool,
    ) {
        let stmts: Vec<String> = split_sql_statements(&sql)
            .into_iter()
            .filter(|s| {
                s.lines().any(|line| {
                    let t = line.trim();
                    !t.is_empty() && !t.starts_with("--")
                })
            })
            .collect();
        if stmts.is_empty() {
            self.status_banner = "No executable SQL statements to sync.".to_string();
            return;
        }

        let src_srv = self.diff_server_label(src_ws_idx);
        let tgt_srv = self.diff_server_label(tgt_ws_idx);
        let suffix = extra_scope.map(|s| format!(" ({s})")).unwrap_or_default();
        let source_label = format!("{src_srv} / {src_db}{suffix}");
        let target_label = format!("{tgt_srv} / {tgt_db}{suffix}");

        let tgt_prof = self.diff_server_profile(tgt_ws_idx);
        let target_is_prod = tgt_prof.is_some_and(|p| p.environment == Environment::Production);
        let target_is_read_only = tgt_prof.is_some_and(|p| p.read_only);
        let target_is_local = tgt_srv.to_ascii_lowercase().contains("local")
            || tgt_prof.is_some_and(|p| {
                p.is_demo
                    || p.name.to_ascii_lowercase().contains("local")
                    || p.host.to_ascii_lowercase().contains("localhost")
            });

        self.db_sync_confirm_checked = false;
        self.db_sync_running = false;
        self.db_sync_progress = None;
        self.pending_db_sync = Some(PendingDbSync {
            title: title.into(),
            source_label,
            target_label,
            target_ws_idx: tgt_ws_idx,
            target_db: tgt_db.to_string(),
            target_is_prod,
            target_is_read_only,
            target_is_local,
            sql,
            statement_count: stmts.len(),
            reload_target_schema,
        });
    }

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

    pub(crate) fn render_mock_data_modal(&mut self, ctx: &egui::Context) {
        if !self.show_mock_data_modal {
            return;
        }
        let mut open = self.show_mock_data_modal;
        let mut generate_to_tab = None;
        let mut insert_directly = None;

        let table_opt = self.mock_data_target_table.clone();
        let schema_opt = self.schemas.get(&self.active_database).cloned();
        let is_read_only = self
            .active_profile_id
            .as_ref()
            .and_then(|id| self.config.connections.iter().find(|p| &p.id == id))
            .is_some_and(|p| p.read_only);

        if let (Some(tbl_name), Some(schema)) = (table_opt.as_ref(), schema_opt.as_ref())
            && let Some(table) = schema.tables.iter().find(|t| t.name == *tbl_name)
            && self
                .mock_gen_config
                .as_ref()
                .is_none_or(|c| c.table_name != *tbl_name)
        {
            self.mock_gen_config = Some(crate::mock_gen::MockGenConfig::from_table(
                table,
                self.mock_data_row_count,
            ));
        }

        let mut fetch_fks = false;

        egui::Window::new("Intelligent Mock Data Generator")
            .open(&mut open)
            .default_width(600.0)
            .show(ctx, |ui| {
                if let (Some(tbl_name), Some(schema)) = (table_opt.as_ref(), schema_opt.as_ref()) {
                    if let Some(table) = schema.tables.iter().find(|t| t.name == *tbl_name) {
                        ui.label(
                            RichText::new(format!(
                                "Target Table: `{tbl_name}` ({} columns, {} FKs)",
                                table.columns.len(),
                                table.foreign_keys.len()
                            ))
                            .strong(),
                        );
                        ui.add_space(4.0);

                        ui.horizontal(|ui| {
                            ui.label("Rows to generate:");
                            ui.add(
                                egui::DragValue::new(&mut self.mock_data_row_count).range(1..=5000),
                            );
                            for preset in [10usize, 25, 50, 100] {
                                if ui.small_button(preset.to_string()).clicked() {
                                    self.mock_data_row_count = preset;
                                }
                            }
                        });

                        if let Some(cfg) = self.mock_gen_config.as_mut() {
                            cfg.row_count = self.mock_data_row_count;

                            ui.horizontal(|ui| {
                                ui.checkbox(
                                    &mut cfg.truncate_before,
                                    "Truncate table before insert",
                                );
                                if !table.foreign_keys.is_empty() {
                                    let fk_btn = if self.mock_gen_loading_fks {
                                        "Sampling FK Parents..."
                                    } else {
                                        "Sample FK Parent IDs"
                                    };
                                    if ui
                                        .add_enabled(
                                            !self.mock_gen_loading_fks
                                                && self.active_session.is_some(),
                                            egui::Button::new(fk_btn).small(),
                                        )
                                        .clicked()
                                    {
                                        fetch_fks = true;
                                    }
                                    let sampled_cols = cfg.fk_parent_samples.len();
                                    if sampled_cols > 0 {
                                        ui.label(
                                            RichText::new(format!(
                                                "{sampled_cols} FK col(s) sampled"
                                            ))
                                            .small()
                                            .color(Color32::from_rgb(34, 197, 94)),
                                        );
                                    }
                                }
                            });

                            ui.separator();
                            ui.label(RichText::new("Column Strategies:").strong());
                            egui::ScrollArea::vertical()
                                .id_salt("mock_gen_cols")
                                .max_height(150.0)
                                .show(ui, |ui| {
                                    for (idx, (col_name, strategy)) in
                                        cfg.column_strategies.iter_mut().enumerate()
                                    {
                                        ui.horizontal(|ui| {
                                            ui.label(
                                                RichText::new(col_name.as_str())
                                                    .monospace()
                                                    .small(),
                                            );
                                            egui::ComboBox::from_id_salt(format!(
                                                "mock_strat_{idx}"
                                            ))
                                            .selected_text(strategy.label())
                                            .show_ui(
                                                ui,
                                                |ui| {
                                                    for variant in
                                                    crate::mock_gen::MockFieldType::preset_variants(
                                                    )
                                                {
                                                    let lbl = variant.label();
                                                    ui.selectable_value(strategy, variant, lbl);
                                                }
                                                },
                                            );
                                        });
                                    }
                                });

                            ui.separator();
                            let mut preview_cfg = cfg.clone();
                            preview_cfg.row_count = self.mock_data_row_count.min(5);
                            let preview_sql =
                                crate::mock_gen::generate_mock_insert_statements(&preview_cfg, 50)
                                    .join("\n");
                            ui.label(RichText::new("SQL Preview (Sample rows):").strong());
                            egui::ScrollArea::vertical()
                                .id_salt("mock_gen_preview")
                                .max_height(130.0)
                                .show(ui, |ui| {
                                    ui.code(preview_sql);
                                });

                            ui.add_space(8.0);
                            ui.horizontal(|ui| {
                                if ui.button("Open in Query Tab").clicked() {
                                    let full_sql =
                                        crate::mock_gen::generate_mock_insert_statements(cfg, 100)
                                            .join("\n\n");
                                    generate_to_tab = Some((tbl_name.clone(), full_sql));
                                }
                                if ui
                                    .add_enabled(
                                        !is_read_only,
                                        egui::Button::new("Insert Directly into DB"),
                                    )
                                    .on_disabled_hover_text("Disabled on read-only connections")
                                    .clicked()
                                {
                                    let full_sql =
                                        crate::mock_gen::generate_mock_insert_statements(cfg, 100)
                                            .join("\n\n");
                                    insert_directly = Some(full_sql);
                                }
                                if ui.button("Cancel").clicked() {
                                    self.show_mock_data_modal = false;
                                }
                            });
                        }
                    } else {
                        ui.label("Table not found in active database schema.");
                    }
                } else {
                    ui.label("No active database or table selected.");
                }
            });

        if fetch_fks
            && let (Some(tbl_name), Some(schema), Some(session)) = (
                table_opt.as_ref(),
                schema_opt.as_ref(),
                self.active_session.clone(),
            )
            && let Some(table) = schema.tables.iter().find(|t| t.name == *tbl_name).cloned()
        {
            self.mock_gen_loading_fks = true;
            let tx = self.tx.clone();
            let ctx = ctx.clone();
            let db = self.active_database.clone();
            let tbl_name = tbl_name.clone();
            self.rt.spawn(async move {
                let res = crate::mock_gen::fetch_fk_parent_samples(&session, &db, &table)
                    .await
                    .map_err(|e| format!("{e:#}"));
                let _ = tx.send(AsyncEvent::MockFkSamplesLoaded {
                    table_name: tbl_name,
                    samples: res,
                });
                ctx.request_repaint();
            });
        }

        if let Some((tbl, sql)) = generate_to_tab {
            self.show_mock_data_modal = false;
            self.tabs.push(QueryTab::new(format!("Seed {tbl}"), sql));
            self.active_tab_idx = self.tabs.len() - 1;
        } else if let Some(sql) = insert_directly {
            self.show_mock_data_modal = false;
            self.execute_sql_forced(sql);
        } else if !open {
            self.show_mock_data_modal = false;
        }
    }

    pub(crate) fn render_csv_import_modal(&mut self, ctx: &egui::Context) {
        let pal = self.effective_palette(ctx);
        let Some(wizard) = &mut self.csv_wizard else {
            return;
        };
        let mut close_modal = false;
        let mut trigger_import = false;

        egui::Window::new("CSV Import Wizard")
            .collapsible(false)
            .resizable(true)
            .default_size([650.0, 520.0])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.heading(
                        RichText::new("Import Data from CSV")
                            .color(pal.accent_text)
                            .size(16.0),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Close").clicked() {
                            close_modal = true;
                        }
                    });
                });
                ui.separator();

                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("File: `{}`", wizard.file_name)).strong());
                    ui.label(format!("({} total rows detected)", wizard.total_rows));
                });
                ui.add_space(6.0);

                // Target table configuration
                ui.horizontal(|ui| {
                    ui.radio_value(&mut wizard.create_new_table, false, "Existing Table:");
                    let current_target = &mut wizard.target_table;
                    let tables = self
                        .schemas
                        .get(&self.active_database)
                        .map(|s| s.tables.clone())
                        .unwrap_or_default();
                    egui::ComboBox::from_id_salt("csv_target_table_combo")
                        .selected_text(if current_target.is_empty() {
                            "Select a table..."
                        } else {
                            current_target.as_str()
                        })
                        .show_ui(ui, |ui| {
                            for tbl in &tables {
                                ui.selectable_value(current_target, tbl.name.clone(), &tbl.name);
                            }
                        });
                });
                ui.horizontal(|ui| {
                    ui.radio_value(&mut wizard.create_new_table, true, "Create New Table:");
                    ui.add(
                        egui::TextEdit::singleline(&mut wizard.new_table_name)
                            .hint_text("table_name"),
                    );
                });

                ui.separator();
                ui.label(RichText::new("Column Mapping:").strong());
                egui::ScrollArea::vertical()
                    .id_salt("csv_mapping_scroll")
                    .max_height(140.0)
                    .show(ui, |ui| {
                        for (csv_col, db_col) in wizard.column_mappings.iter_mut() {
                            ui.horizontal(|ui| {
                                ui.set_min_width(180.0);
                                ui.label(RichText::new(format!("CSV `{csv_col}`")).monospace());
                                ui.label("➜");
                                ui.add(
                                    egui::TextEdit::singleline(db_col)
                                        .desired_width(160.0)
                                        .hint_text("db_column_name"),
                                );
                            });
                        }
                    });

                ui.separator();
                ui.label(RichText::new("Data Preview (First 5 Rows):").strong());
                egui::ScrollArea::horizontal()
                    .id_salt("csv_preview_scroll")
                    .max_height(100.0)
                    .show(ui, |ui| {
                        let num_cols = wizard.headers.len().max(1);
                        let mut builder = TableBuilder::new(ui)
                            .striped(true)
                            .resizable(true)
                            .cell_layout(egui::Layout::left_to_right(egui::Align::Center));
                        for _ in 0..num_cols {
                            builder = builder.column(Column::initial(100.0).at_least(60.0));
                        }
                        builder
                            .header(20.0, |mut h| {
                                for head in &wizard.headers {
                                    h.col(|ui| {
                                        ui.strong(head);
                                    });
                                }
                            })
                            .body(|mut b| {
                                for row in &wizard.sample_rows {
                                    b.row(18.0, |mut r| {
                                        for cell in row {
                                            r.col(|ui| {
                                                ui.label(cell);
                                            });
                                        }
                                    });
                                }
                            });
                    });

                if let Some(err) = &wizard.error_message {
                    ui.colored_label(pal.danger, err);
                }
                if let Some(msg) = &wizard.status_message {
                    ui.colored_label(pal.success, msg);
                }

                ui.separator();
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let target_name = if wizard.create_new_table {
                        wizard.new_table_name.trim()
                    } else {
                        wizard.target_table.trim()
                    };
                    let can_import = !target_name.is_empty() && !wizard.is_importing;

                    if ui
                        .add_enabled(can_import, pal.primary_button("Import Now"))
                        .clicked()
                    {
                        trigger_import = true;
                    }

                    if ui.button("Cancel").clicked() {
                        close_modal = true;
                    }
                });
            });

        if trigger_import {
            self.execute_csv_import();
        }
        if close_modal {
            self.csv_wizard = None;
        }
    }

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
                    if let Err(e) = tgt_session
                        .execute_query(&tgt_db, "START TRANSACTION;", None, None)
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

    pub(crate) fn render_sql_dump_modal(&mut self, ctx: &egui::Context) {
        if !self.show_sql_dump_modal {
            return;
        }
        let mut open = self.show_sql_dump_modal;
        let mut trigger_export = false;
        let mut trigger_save_file = false;
        let mut trigger_import_file = false;

        egui::Window::new("SQL Dump Export & Import")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(680.0)
            .default_height(460.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.checkbox(&mut self.dump_include_drop, "Include DROP TABLE IF EXISTS");
                    ui.checkbox(&mut self.dump_include_data, "Include Data (INSERTs)");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add_enabled(
                                !self.dump_is_busy,
                                egui::Button::new("Import / Run .sql File..."),
                            )
                            .clicked()
                        {
                            trigger_import_file = true;
                        }
                    });
                });
                ui.separator();

                if let Some(schema) = self.schemas.get(&self.active_database) {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Select Tables:").strong());
                        if ui.small_button("All").clicked() {
                            for t in &schema.tables {
                                self.dump_selected_tables.insert(t.name.clone());
                            }
                        }
                        if ui.small_button("None").clicked() {
                            self.dump_selected_tables.clear();
                        }
                    });
                    egui::ScrollArea::vertical()
                        .id_salt("dump_tables_list")
                        .max_height(120.0)
                        .show(ui, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                for t in &schema.tables {
                                    let mut checked = self.dump_selected_tables.contains(&t.name);
                                    if ui.checkbox(&mut checked, &t.name).changed() {
                                        if checked {
                                            self.dump_selected_tables.insert(t.name.clone());
                                        } else {
                                            self.dump_selected_tables.remove(&t.name);
                                        }
                                    }
                                }
                            });
                        });
                }

                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(!self.dump_is_busy, egui::Button::new("Generate Dump"))
                        .clicked()
                    {
                        trigger_export = true;
                    }
                    if !self.dump_preview_sql.is_empty() {
                        if ui.button("Save to .sql File...").clicked() {
                            trigger_save_file = true;
                        }
                        if ui.button("Copy All").clicked() {
                            ui.ctx().copy_text(self.dump_preview_sql.clone());
                        }
                    }
                    if self.dump_is_busy {
                        ui.spinner();
                        ui.label("Working...");
                    }
                });

                if !self.dump_preview_sql.is_empty() {
                    ui.separator();
                    let dump_h = ui.available_height().max(140.0);
                    egui::ScrollArea::vertical()
                        .id_salt("dump_preview_scroll")
                        .auto_shrink([false, false])
                        .max_height(dump_h)
                        .show(ui, |ui| {
                            ui.add(
                                egui::TextEdit::multiline(&mut self.dump_preview_sql)
                                    .font(egui::TextStyle::Monospace)
                                    .desired_width(f32::INFINITY),
                            );
                        });
                }
                let remaining_h = ui.available_height();
                if remaining_h > 0.0 {
                    ui.allocate_space(egui::vec2(0.0, remaining_h));
                }
            });

        self.show_sql_dump_modal = open;

        if trigger_save_file {
            let default_name = format!("{}_dump.sql", self.active_database);
            if let Some(path) = rfd::FileDialog::new()
                .set_file_name(&default_name)
                .save_file()
            {
                let _ = save_string_to_path(&path, &self.dump_preview_sql);
                self.status_banner = format!("Saved SQL dump to {}", path.display());
            }
        }

        if trigger_export
            && let (Some(session), Some(schema)) = (
                self.active_session.clone(),
                self.schemas.get(&self.active_database).cloned(),
            )
        {
            self.dump_is_busy = true;
            let db = self.active_database.clone();
            let selected = self.dump_selected_tables.clone();
            let inc_drop = self.dump_include_drop;
            let inc_data = self.dump_include_data;
            let tx = self.tx.clone();
            self.rt.spawn(async move {
                let mut full_dump = format!("-- MySQL-Rusty SQL Dump for `{db}`\nUSE `{db}`;\n\n");
                let mut err_msg = None;
                for tbl in &schema.tables {
                    if !selected.contains(&tbl.name) {
                        continue;
                    }
                    let data_res = if inc_data {
                        let sql = format!("SELECT * FROM `{db}`.`{}` LIMIT 1000;", tbl.name);
                        match session.execute_query(&db, &sql, None, None).await {
                            Ok(r) => Some(r),
                            Err(e) => {
                                err_msg = Some(format!("{e:#}"));
                                break;
                            }
                        }
                    } else {
                        None
                    };
                    full_dump.push_str(&generate_table_dump_sql(
                        tbl,
                        data_res.as_ref(),
                        inc_drop,
                        inc_data,
                        100,
                    ));
                    full_dump.push('\n');
                }
                let _ = tx.send(AsyncEvent::SqlDumpLoaded {
                    sql: match err_msg {
                        Some(e) => Err(e),
                        None => Ok(full_dump),
                    },
                    suggested_name: db,
                });
            });
        }

        if trigger_import_file
            && let Some(path) = rfd::FileDialog::new()
                .add_filter("SQL", &["sql", "txt"])
                .pick_file()
            && let Ok(content) = std::fs::read_to_string(&path)
            && let Some(session) = self.active_session.clone()
        {
            self.dump_is_busy = true;
            let db = self.active_database.clone();
            let tx = self.tx.clone();
            self.rt.spawn(async move {
                let stmts = split_sql_statements(&content);
                let mut executed = 0usize;
                let mut err_out = None;
                for stmt in stmts {
                    if let Err(e) = session.execute_query(&db, &stmt, None, None).await {
                        err_out = Some(format!("{e:#}"));
                        break;
                    }
                    executed += 1;
                }
                let _ = tx.send(AsyncEvent::SqlDumpImportFinished(match err_out {
                    Some(e) => Err(e),
                    None => Ok(executed),
                }));
            });
        }
    }

    pub(crate) fn render_backup_modal(&mut self, ctx: &egui::Context) {
        let Some(state) = self.backup_modal.as_mut() else {
            return;
        };
        let mut open = true;
        let mut start_backup = false;
        let mut cancel_backup = false;
        let db = state.database.clone();

        egui::Window::new(format!("Backup Database — `{db}`"))
            .open(&mut open)
            .resizable(true)
            .default_width(480.0)
            .show(ctx, |ui| {
                ui.add_enabled_ui(!state.running, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Tables").strong());
                        if ui.small_button("All").clicked() {
                            state.selected_tables = state.all_tables.iter().cloned().collect();
                        }
                        if ui.small_button("None").clicked() {
                            state.selected_tables.clear();
                        }
                    });
                    if state.all_tables.is_empty() {
                        ui.label(
                            RichText::new(
                                "All tables in database will be discovered at backup time.",
                            )
                            .small()
                            .weak(),
                        );
                    } else {
                        egui::ScrollArea::vertical()
                            .max_height(150.0)
                            .show(ui, |ui| {
                                for tbl in &state.all_tables {
                                    let mut checked = state.selected_tables.contains(tbl);
                                    if ui.checkbox(&mut checked, tbl).changed() {
                                        if checked {
                                            state.selected_tables.insert(tbl.clone());
                                        } else {
                                            state.selected_tables.remove(tbl);
                                        }
                                    }
                                }
                            });
                    }

                    ui.separator();
                    ui.checkbox(&mut state.include_data, "Include row data (INSERT batches)");
                    ui.checkbox(&mut state.include_drop, "Include DROP IF EXISTS");
                    ui.checkbox(&mut state.include_views, "Include views");
                    ui.checkbox(
                        &mut state.include_routines,
                        "Include procedures & functions",
                    );
                    ui.checkbox(&mut state.include_triggers, "Include triggers");
                    ui.horizontal(|ui| {
                        ui.label("Batch rows:");
                        ui.add(
                            egui::DragValue::new(&mut state.batch_rows)
                                .range(1..=5000)
                                .speed(25),
                        );
                    });
                });

                ui.separator();
                if state.running {
                    let frac = if state.table_total > 0 {
                        state.table_index as f32 / state.table_total as f32
                    } else {
                        0.0
                    };
                    ui.add(
                        egui::ProgressBar::new(frac)
                            .text(format!(
                                "Table {}/{}: {} ({} rows)",
                                state.table_index,
                                state.table_total,
                                state.current_table,
                                state.rows_done
                            ))
                            .animate(true),
                    );
                    ui.add_space(6.0);
                    if ui.button("Cancel").clicked() {
                        cancel_backup = true;
                    }
                } else {
                    if let Some(status) = &state.status {
                        ui.label(RichText::new(status).small());
                        ui.add_space(4.0);
                    }
                    if ui.button("Start Backup...").clicked() {
                        start_backup = true;
                    }
                }
            });

        if cancel_backup
            && let Some(st) = self.backup_modal.as_ref()
            && let Some(flag) = &st.cancel
        {
            flag.store(true, std::sync::atomic::Ordering::Relaxed);
        }

        if start_backup
            && let Some(session) = self.active_session.clone()
            && let Some(st) = self.backup_modal.as_mut()
        {
            let default_name = format!(
                "{}_{}.sql",
                st.database,
                chrono::Local::now().format("%Y%m%d_%H%M")
            );
            if let Some(path) = rfd::FileDialog::new()
                .set_file_name(&default_name)
                .add_filter("SQL", &["sql"])
                .save_file()
            {
                let tables: Vec<String> = st
                    .all_tables
                    .iter()
                    .filter(|t| st.selected_tables.contains(*t))
                    .cloned()
                    .collect();
                let opts = crate::backup::BackupOptions {
                    database: st.database.clone(),
                    tables,
                    include_data: st.include_data,
                    include_drop: st.include_drop,
                    include_views: st.include_views,
                    include_routines: st.include_routines,
                    include_triggers: st.include_triggers,
                    batch_rows: st.batch_rows,
                };
                let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
                st.running = true;
                st.cancel = Some(cancel.clone());
                st.status = None;
                st.table_index = 0;
                st.table_total = 0;
                st.rows_done = 0;
                let tx = self.tx.clone();
                self.rt.spawn(async move {
                    let _ = crate::backup::run_backup(session, opts, path, cancel, move |p| {
                        let _ = tx.send(AsyncEvent::BackupProgress(p));
                    })
                    .await;
                });
            }
        }

        if !open && self.backup_modal.as_ref().is_some_and(|m| !m.running) {
            self.backup_modal = None;
        }
    }

    pub(crate) fn render_restore_modal(&mut self, ctx: &egui::Context) {
        let active_profile = self
            .active_profile_id
            .as_ref()
            .and_then(|id| self.config.connections.iter().find(|p| &p.id == id))
            .cloned();
        let is_read_only = active_profile.as_ref().is_some_and(|p| p.read_only);
        let is_production = active_profile
            .as_ref()
            .is_some_and(|p| p.environment == Environment::Production);

        let pal = self.effective_palette(ctx);
        let databases = self.databases.clone();
        let Some(state) = self.restore_modal.as_mut() else {
            return;
        };
        let mut open = true;
        let mut close_clicked = false;
        let mut start_restore = false;
        let mut cancel_restore = false;

        egui::Window::new("Run SQL File (Streaming Restore)")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .default_width(520.0)
            .show(ctx, |ui| {
                ui.add_enabled_ui(!state.running, |ui| {
                    ui.horizontal(|ui| {
                        ui.label("SQL file:");
                        let path_label = state
                            .file_path
                            .as_ref()
                            .map(|p| p.display().to_string())
                            .unwrap_or_else(|| "(none selected)".to_string());
                        ui.label(RichText::new(path_label).monospace().small());
                        if ui.button("Browse...").clicked()
                            && let Some(p) = rfd::FileDialog::new()
                                .add_filter("SQL Files", &["sql"])
                                .pick_file()
                        {
                            state.file_path = Some(p);
                        }
                    });

                    ui.horizontal(|ui| {
                        ui.label("Target database:");
                        egui::ComboBox::from_id_salt("restore_target_db")
                            .selected_text(if state.database.is_empty() {
                                "(none)"
                            } else {
                                &state.database
                            })
                            .show_ui(ui, |ui| {
                                for db in &databases {
                                    ui.selectable_value(&mut state.database, db.clone(), db);
                                }
                            });
                    });

                    ui.horizontal(|ui| {
                        ui.label("On error:");
                        ui.radio_value(&mut state.policy, crate::backup::OnError::Stop, "Stop");
                        ui.radio_value(
                            &mut state.policy,
                            crate::backup::OnError::Continue,
                            "Continue",
                        );
                    });
                });

                if is_read_only {
                    ui.colored_label(
                        pal.danger,
                        "Refused: active connection is marked Read-Only.",
                    );
                } else if is_production && !state.running {
                    ui.colored_label(
                        pal.warning,
                        format!(
                            "Production environment: type '{}' to confirm execution:",
                            state.database
                        ),
                    );
                    ui.text_edit_singleline(&mut state.prod_confirm_input);
                }

                ui.separator();
                let frac = if state.bytes_total > 0 {
                    (state.bytes_done as f32 / state.bytes_total as f32).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                if state.running || state.bytes_done > 0 {
                    ui.add(
                        egui::ProgressBar::new(frac)
                            .text(format!(
                                "{}/{} bytes | {} ok, {} failed",
                                state.bytes_done,
                                state.bytes_total,
                                state.statements_ok,
                                state.statements_failed
                            ))
                            .animate(state.running),
                    );
                }

                if !state.errors.is_empty() {
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(format!("Errors (showing {}):", state.errors.len()))
                            .color(pal.danger)
                            .strong(),
                    );
                    egui::ScrollArea::vertical()
                        .id_salt("restore_errors_scroll")
                        .max_height(150.0)
                        .show(ui, |ui| {
                            for err in &state.errors {
                                ui.label(
                                    RichText::new(format!(
                                        "{} -> {}",
                                        err.statement_preview, err.message
                                    ))
                                    .monospace()
                                    .small()
                                    .color(pal.danger),
                                );
                            }
                        });
                }

                if let Some(status) = &state.status {
                    ui.add_space(4.0);
                    ui.label(RichText::new(status).small());
                }

                ui.add_space(6.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if state.running {
                        if ui.button("Cancel").clicked() {
                            cancel_restore = true;
                        }
                    } else {
                        let prod_ok = !is_production
                            || (!state.database.is_empty()
                                && state.prod_confirm_input == state.database);
                        let can_start = !is_read_only && prod_ok && state.file_path.is_some();
                        if ui
                            .add_enabled(can_start, pal.primary_button("Start"))
                            .clicked()
                        {
                            start_restore = true;
                        }
                        if ui.button("Close").clicked() {
                            close_clicked = true;
                        }
                    }
                });
            });

        if cancel_restore
            && let Some(st) = self.restore_modal.as_ref()
            && let Some(flag) = &st.cancel
        {
            flag.store(true, std::sync::atomic::Ordering::Relaxed);
        }

        if start_restore
            && let Some(session) = self.active_session.clone()
            && let Some(st) = self.restore_modal.as_mut()
            && let Some(path) = st.file_path.clone()
        {
            let db = st.database.clone();
            let policy = st.policy;
            let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
            st.running = true;
            st.cancel = Some(cancel.clone());
            st.bytes_done = 0;
            st.bytes_total = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            st.statements_ok = 0;
            st.statements_failed = 0;
            st.errors.clear();
            st.status = None;
            let tx = self.tx.clone();
            self.rt.spawn(async move {
                let _ = crate::backup::run_sql_file(session, db, path, policy, cancel, move |p| {
                    let _ = tx.send(AsyncEvent::RestoreProgress(p));
                })
                .await;
            });
        }

        if (!open || close_clicked) && self.restore_modal.as_ref().is_some_and(|m| !m.running) {
            self.restore_modal = None;
        }
    }
}
