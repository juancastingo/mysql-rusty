use super::*;

fn format_sidebar_select_sql(
    dialect: crate::dialect::Dialect,
    db_name: &str,
    table_name: &str,
) -> String {
    let target = match dialect {
        crate::dialect::Dialect::MySql => dialect.quote_qualified(db_name, table_name),
        crate::dialect::Dialect::Postgres => {
            if let Some((schema, tbl)) = table_name.split_once('.') {
                dialect.quote_qualified(schema, tbl)
            } else {
                dialect.quote_ident(table_name)
            }
        }
        crate::dialect::Dialect::Sqlite => dialect.quote_ident(table_name),
    };
    format!("SELECT * FROM {target} LIMIT 100;")
}

fn format_sidebar_ddl_sql(
    dialect: crate::dialect::Dialect,
    db_name: &str,
    table_name: &str,
) -> String {
    let escaped = table_name.replace('\'', "''");
    match dialect {
        crate::dialect::Dialect::MySql => {
            format!(
                "SHOW CREATE TABLE {};",
                dialect.quote_qualified(db_name, table_name)
            )
        }
        crate::dialect::Dialect::Sqlite => {
            format!("SELECT sql FROM sqlite_master WHERE type = 'table' AND name = '{escaped}';")
        }
        crate::dialect::Dialect::Postgres => {
            let (schema, tbl) = table_name.split_once('.').unwrap_or(("public", table_name));
            let s_esc = schema.replace('\'', "''");
            let t_esc = tbl.replace('\'', "''");
            format!(
                "SELECT column_name, data_type, is_nullable, column_default FROM information_schema.columns WHERE table_schema = '{s_esc}' AND table_name = '{t_esc}' ORDER BY ordinal_position;"
            )
        }
    }
}

impl MysqlRustyApp {
    pub(crate) fn render_left_sidebar(&mut self, ui: &mut egui::Ui) {
        let pal = self.effective_palette(ui.ctx());
        egui::Panel::left("left_sidebar")
            .default_size(280.0)
            .size_range(220.0..=420.0)
            .show(ui, |ui| {
                self.tour
                    .set_anchor(crate::tour::TourAnchor::SchemaTree, ui.max_rect());
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    for (tab, label) in [
                        (SidebarTab::Schema, rust_i18n::t!("sidebar.schema")),
                        (SidebarTab::Connections, rust_i18n::t!("sidebar.servers")),
                        (SidebarTab::SavedQueries, rust_i18n::t!("sidebar.saved_queries")),
                        (SidebarTab::History, rust_i18n::t!("sidebar.history")),
                    ] {
                        let is_sel = self.sidebar_tab == tab;
                        if ui.add(pal.pill_button(label.as_ref(), is_sel)).clicked() {
                            self.sidebar_tab = tab;
                        }
                    }
                });
                ui.add_space(2.0);
                ui.separator();

                match self.sidebar_tab {
                    SidebarTab::Schema => self.render_schema_tree(ui),
                    SidebarTab::Connections => self.render_connections_list(ui),
                    SidebarTab::SavedQueries => self.render_saved_queries_panel(ui),
                    SidebarTab::History => self.render_query_history_panel(ui),
                }
            });
    }

    pub(crate) fn render_schema_tree(&mut self, ui: &mut Ui) {
        let pal = self.effective_palette(ui.ctx());
        if self.active_session.is_none() {
            ui.vertical_centered(|ui| {
                ui.add_space(30.0);
                ui.label(RichText::new("No database connected").color(pal.muted_text));
                ui.add_space(8.0);
                if ui.add(pal.primary_button("Choose Connection...")).clicked() {
                    self.sidebar_tab = SidebarTab::Connections;
                }
            });
            return;
        }

        // Database selector and refresh
        ui.horizontal(|ui| {
            ui.label(RichText::new("DB:").strong().color(pal.text_secondary));
            let current_db = if self.active_database.is_empty() {
                "None"
            } else {
                &self.active_database
            };
            egui::ComboBox::from_id_salt("sidebar_schema_db_selector")
                .selected_text(current_db)
                .width((ui.available_width() - 62.0).max(100.0))
                .show_ui(ui, |ui| {
                    let dbs = self.databases.clone();
                    for db in dbs {
                        let is_sel = self.active_database == db;
                        if ui.selectable_label(is_sel, &db).clicked() {
                            self.active_database = db;
                            self.reload_active_schema();
                        }
                    }
                });
            if ui
                .small_button("↻")
                .on_hover_text("Refresh active database schema")
                .clicked()
            {
                self.reload_active_schema();
            }
        });
        ui.add_space(2.0);

        // Search filter with clear button
        ui.horizontal(|ui| {
            let clear_w = if self.schema_filter.is_empty() {
                0.0
            } else {
                24.0
            };
            ui.add(
                egui::TextEdit::singleline(&mut self.schema_filter)
                    .hint_text("Filter tables or columns...")
                    .desired_width((ui.available_width() - clear_w).max(80.0)),
            );
            if !self.schema_filter.is_empty()
                && ui.small_button("×").on_hover_text("Clear filter").clicked()
            {
                self.schema_filter.clear();
            }
        });

        // Schema quick actions (wrapped cleanly within 280px sidebar)
        ui.horizontal_wrapped(|ui| {
            if ui
                .small_button("+ New Table")
                .on_hover_text("Visually design and create a new table")
                .clicked()
            {
                self.table_designer_draft = Some(TableDraft::new_empty("new_table"));
            }
            if ui
                .small_button("Import CSV")
                .on_hover_text(
                    "Import a CSV file into a new or existing table with preview and mapping",
                )
                .clicked()
            {
                self.open_csv_import_dialog();
            }
            if ui
                .small_button("Diff & Sync")
                .on_hover_text("Compare two schemas and generate DDL synchronization script")
                .clicked()
            {
                self.show_schema_diff_modal = true;
                self.diff_source_db = self.active_database.clone();
                self.diff_target_db = self
                    .databases
                    .iter()
                    .find(|d| **d != self.active_database)
                    .cloned()
                    .unwrap_or_default();
                if !self.diff_source_db.is_empty()
                    && !self.diff_target_db.is_empty()
                    && let (Some(src), Some(tgt)) = (
                        self.schemas.get(&self.diff_source_db),
                        self.schemas.get(&self.diff_target_db),
                    )
                {
                    self.current_diff_report = Some(diff_schemas(src, tgt));
                }
            }
        });
        ui.separator();

        let dbs_list = self.databases.clone();
        let schemas_snapshot = self.schemas.clone();
        let filter_lower = self.schema_filter.to_lowercase();
        let mut pending_sql_action: Option<(String, bool)> = None; // (sql, auto_run)
        let mut toggle_favorite_table: Option<String> = None;
        let mut pending_ddl_fetch: Option<(String, String, String)> = None;
        let mut pending_routine_call: Option<(String, String)> = None;
        let mut activate_db: Option<String> = None;
        let mut load_db_schema: Option<String> = None;

        let fav_tables: Vec<String> = self
            .config
            .connections
            .iter()
            .find(|c| Some(&c.id) == self.active_profile_id.as_ref())
            .map(|c| c.favorite_tables.clone())
            .unwrap_or_default();

        egui::ScrollArea::vertical().show(ui, |ui| {
            for db_name in &dbs_list {
                let is_active_db = self.active_database == *db_name;
                let db_header_text = if is_active_db {
                    RichText::new(format!("{db_name} (active)"))
                        .strong()
                        .color(pal.accent_text)
                } else {
                    RichText::new(db_name.as_str())
                        .strong()
                        .color(pal.text_primary)
                };

                let db_resp = egui::CollapsingHeader::new(db_header_text)
                    .id_salt(("sidebar_db_tree", db_name))
                    .default_open(is_active_db)
                    .show(ui, |ui| {
                        if !is_active_db
                            && ui
                                .small_button("Set Active DB")
                                .on_hover_text(format!("Switch active database to `{db_name}`"))
                                .clicked()
                        {
                            activate_db = Some(db_name.clone());
                        }

                        let Some(schema) = schemas_snapshot.get(db_name) else {
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new("Schema not loaded")
                                        .small()
                                        .color(pal.muted_text),
                                );
                                if ui.small_button("Load").clicked() {
                                    load_db_schema = Some(db_name.clone());
                                }
                            });
                            return;
                        };

                        let mut sorted_tables: Vec<&crate::db::TableSchema> =
                            schema.tables.iter().collect();
                        sorted_tables.sort_by(|a, b| {
                            let fa = fav_tables.iter().any(|f| f.eq_ignore_ascii_case(&a.name));
                            let fb = fav_tables.iter().any(|f| f.eq_ignore_ascii_case(&b.name));
                            fb.cmp(&fa).then_with(|| a.name.cmp(&b.name))
                        });

                        for table in sorted_tables {
                            let matches_table = filter_lower.is_empty()
                                || table.name.to_lowercase().contains(&filter_lower)
                                || table
                                    .columns
                                    .iter()
                                    .any(|c| c.name.to_lowercase().contains(&filter_lower));

                            if !matches_table {
                                continue;
                            }

                            let is_fav = fav_tables
                                .iter()
                                .any(|f| f.eq_ignore_ascii_case(&table.name));
                            let star_prefix = if is_fav { "★ " } else { "" };
                            let header_label =
                                format!("{star_prefix}{} (~{})", table.name, table.approx_rows);
                            let header_text = if is_fav {
                                RichText::new(header_label).strong().color(pal.warning)
                            } else {
                                RichText::new(header_label)
                                    .strong()
                                    .color(pal.text_primary)
                            };
                            let tbl_resp = egui::CollapsingHeader::new(header_text)
                                .id_salt(("sidebar_tbl_tree", db_name, &table.name))
                                .default_open(!filter_lower.is_empty())
                                .show(ui, |ui| {
                                    ui.horizontal_wrapped(|ui| {
                                        let pin_txt = if is_fav { "★" } else { "☆" };
                                        if ui
                                            .small_button(pin_txt)
                                            .on_hover_text(
                                                "Pin/Unpin favorite table to top of Schema Explorer",
                                            )
                                            .clicked()
                                        {
                                            toggle_favorite_table = Some(table.name.clone());
                                        }
                                        let active_dialect = self
                                            .active_session
                                            .as_ref()
                                            .map(|s| s.dialect())
                                            .unwrap_or(crate::dialect::Dialect::MySql);
                                        if ui
                                            .small_button("SELECT *")
                                            .on_hover_text("Preview first 100 rows")
                                            .clicked()
                                        {
                                            pending_sql_action = Some((
                                                format_sidebar_select_sql(
                                                    active_dialect,
                                                    db_name,
                                                    &table.name,
                                                ),
                                                true,
                                            ));
                                        }
                                        if ui
                                            .small_button("Design")
                                            .on_hover_text("Visual table designer")
                                            .clicked()
                                        {
                                            self.table_designer_draft =
                                                Some(TableDraft::from_table(table));
                                        }
                                        ui.menu_button("More ▾", |ui| {
                                            if ui.button("Show DDL").clicked() {
                                                pending_sql_action = Some((
                                                    format_sidebar_ddl_sql(
                                                        active_dialect,
                                                        db_name,
                                                        &table.name,
                                                    ),
                                                    true,
                                                ));
                                                ui.close();
                                            }
                                            if ui.button("Table Info").clicked() {
                                                self.table_info_modal =
                                                    Some((db_name.clone(), (*table).clone(), 0));
                                                ui.close();
                                            }
                                            if ui.button("Duplicate / Truncate").clicked() {
                                                self.table_action_modal = Some((
                                                    db_name.clone(),
                                                    table.name.clone(),
                                                    format!("{}_copy", table.name),
                                                ));
                                                ui.close();
                                            }
                                            if ui.button("Import CSV...").clicked() {
                                                if let Some(path) = rfd::FileDialog::new()
                                                    .add_filter("CSV", &["csv", "txt"])
                                                    .pick_file()
                                                    && let Ok(content) =
                                                        std::fs::read_to_string(&path)
                                                {
                                                    match parse_csv_to_batch_insert(
                                                        &table.name,
                                                        &content,
                                                    ) {
                                                        Ok(batch_sql) => {
                                                            self.tabs.push(QueryTab::new(
                                                                format!("Import {}", table.name),
                                                                batch_sql,
                                                            ));
                                                            self.active_tab_idx =
                                                                self.tabs.len() - 1;
                                                            self.status_banner = format!(
                                                                "Generated batch INSERT statements from `{}`",
                                                                path.file_name()
                                                                    .unwrap_or_default()
                                                                    .to_string_lossy()
                                                            );
                                                        }
                                                        Err(err) => {
                                                            self.status_banner = format!(
                                                                "Failed to parse CSV: {err}"
                                                            );
                                                        }
                                                    }
                                                }
                                                ui.close();
                                            }
                                            if ui.button("Generate Mock Data").clicked() {
                                                self.mock_data_target_table =
                                                    Some(table.name.clone());
                                                self.show_mock_data_modal = true;
                                                ui.close();
                                            }
                                            if ui.button("Insert Name in Editor").clicked() {
                                                pending_sql_action =
                                                    Some((table.name.clone(), false));
                                                ui.close();
                                            }
                                        });
                                    });

                                    for col in &table.columns {
                                        let is_fk = table
                                            .foreign_keys
                                            .iter()
                                            .any(|fk| fk.column_name.eq_ignore_ascii_case(&col.name));
                                        ui.horizontal(|ui| {
                                            ui.spacing_mut().item_spacing.x = 5.0;
                                            if col.is_primary_key {
                                                pal.micro_badge(
                                                    ui,
                                                    "PK",
                                                    pal.warning,
                                                    pal.warning_bg,
                                                );
                                            } else if is_fk {
                                                pal.micro_badge(
                                                    ui,
                                                    "FK",
                                                    pal.accent_text,
                                                    pal.accent_bg,
                                                );
                                            } else {
                                                ui.label(
                                                    RichText::new("•")
                                                        .size(10.0)
                                                        .color(pal.muted_text),
                                                );
                                            }
                                            ui.label(
                                                RichText::new(&col.name)
                                                    .size(12.0)
                                                    .color(pal.text_primary),
                                            );
                                            ui.label(
                                                RichText::new(&col.data_type)
                                                    .monospace()
                                                    .color(pal.muted_text)
                                                    .size(10.5),
                                            );
                                        });
                                    }

                                    if !table.foreign_keys.is_empty() {
                                        ui.collapsing(
                                            RichText::new(format!(
                                                "[FK] Foreign Keys ({})",
                                                table.foreign_keys.len()
                                            ))
                                            .small()
                                            .color(pal.accent_text),
                                            |ui| {
                                                for fk in &table.foreign_keys {
                                                    ui.label(
                                                        RichText::new(format!(
                                                            "{} -> {}.{}",
                                                            fk.column_name,
                                                            fk.referenced_table,
                                                            fk.referenced_column
                                                        ))
                                                        .small()
                                                        .color(pal.text_secondary),
                                                    );
                                                }
                                            },
                                        );
                                    }
                                });

                            tbl_resp.header_response.context_menu(|ui| {
                                if ui.button("SELECT * LIMIT 100").clicked() {
                                    let active_dialect = self
                                        .active_session
                                        .as_ref()
                                        .map(|s| s.dialect())
                                        .unwrap_or(crate::dialect::Dialect::MySql);
                                    pending_sql_action = Some((
                                        format_sidebar_select_sql(
                                            active_dialect,
                                            db_name,
                                            &table.name,
                                        ),
                                        true,
                                    ));
                                    ui.close();
                                }
                                if ui.button("Show DDL").clicked() {
                                    let active_dialect = self
                                        .active_session
                                        .as_ref()
                                        .map(|s| s.dialect())
                                        .unwrap_or(crate::dialect::Dialect::MySql);
                                    pending_sql_action = Some((
                                        format_sidebar_ddl_sql(
                                            active_dialect,
                                            db_name,
                                            &table.name,
                                        ),
                                        true,
                                    ));
                                    ui.close();
                                }
                                if ui.button("Design Table").clicked() {
                                    self.table_designer_draft =
                                        Some(TableDraft::from_table(table));
                                    ui.close();
                                }
                                if ui.button("Table Info").clicked() {
                                    self.table_info_modal =
                                        Some((db_name.clone(), (*table).clone(), 0));
                                    ui.close();
                                }
                                if ui.button("Duplicate / Truncate").clicked() {
                                    self.table_action_modal = Some((
                                        db_name.clone(),
                                        table.name.clone(),
                                        format!("{}_copy", table.name),
                                    ));
                                    ui.close();
                                }
                                if ui.button("Generate Mock Data").clicked() {
                                    self.mock_data_target_table = Some(table.name.clone());
                                    self.show_mock_data_modal = true;
                                    ui.close();
                                }
                                ui.separator();
                                let pin_label =
                                    if is_fav { "Unpin Favorite" } else { "Pin Favorite" };
                                if ui.button(pin_label).clicked() {
                                    toggle_favorite_table = Some(table.name.clone());
                                    ui.close();
                                }
                            });
                        }

                        if !schema.views.is_empty() {
                            ui.collapsing(format!("Views ({})", schema.views.len()), |ui| {
                                for view in &schema.views {
                                    if !filter_lower.is_empty()
                                        && !view.name.to_lowercase().contains(&filter_lower)
                                    {
                                        continue;
                                    }
                                    ui.horizontal(|ui| {
                                        ui.label(&view.name);
                                        if ui.small_button("SELECT").clicked() {
                                            pending_sql_action = Some((
                                                format!("SELECT * FROM `{}` LIMIT 100;", view.name),
                                                true,
                                            ));
                                        }
                                        if ui.small_button("DDL").clicked() {
                                            pending_ddl_fetch = Some((
                                                db_name.clone(),
                                                "VIEW".to_string(),
                                                view.name.clone(),
                                            ));
                                        }
                                    });
                                }
                            });
                        }

                        if !schema.procedures.is_empty() {
                            ui.collapsing(
                                format!("Stored Procedures ({})", schema.procedures.len()),
                                |ui| {
                                    for proc_info in &schema.procedures {
                                        if !filter_lower.is_empty()
                                            && !proc_info
                                                .name
                                                .to_lowercase()
                                                .contains(&filter_lower)
                                        {
                                            continue;
                                        }
                                        ui.horizontal(|ui| {
                                            ui.label(&proc_info.name);
                                            if ui.small_button("Call").clicked() {
                                                pending_routine_call =
                                                    Some((db_name.clone(), proc_info.name.clone()));
                                            }
                                            if ui.small_button("DDL").clicked() {
                                                pending_ddl_fetch = Some((
                                                    db_name.clone(),
                                                    "PROCEDURE".to_string(),
                                                    proc_info.name.clone(),
                                                ));
                                            }
                                        });
                                    }
                                },
                            );
                        }

                        if !schema.functions.is_empty() {
                            ui.collapsing(
                                format!("Functions ({})", schema.functions.len()),
                                |ui| {
                                    for func_info in &schema.functions {
                                        if !filter_lower.is_empty()
                                            && !func_info
                                                .name
                                                .to_lowercase()
                                                .contains(&filter_lower)
                                        {
                                            continue;
                                        }
                                        ui.horizontal(|ui| {
                                            ui.label(&func_info.name);
                                            if !func_info.data_type.is_empty() {
                                                ui.label(
                                                    RichText::new(&func_info.data_type)
                                                        .small()
                                                        .weak(),
                                                );
                                            }
                                            if ui.small_button("DDL").clicked() {
                                                pending_ddl_fetch = Some((
                                                    db_name.clone(),
                                                    "FUNCTION".to_string(),
                                                    func_info.name.clone(),
                                                ));
                                            }
                                        });
                                    }
                                },
                            );
                        }

                        if !schema.triggers.is_empty() {
                            ui.collapsing(format!("Triggers ({})", schema.triggers.len()), |ui| {
                                for trig in &schema.triggers {
                                    if !filter_lower.is_empty()
                                        && !trig.name.to_lowercase().contains(&filter_lower)
                                        && !trig.table.to_lowercase().contains(&filter_lower)
                                    {
                                        continue;
                                    }
                                    ui.horizontal(|ui| {
                                        ui.label(&trig.name);
                                        ui.label(
                                            RichText::new(format!(
                                                "{} {} ON {}",
                                                trig.timing, trig.event, trig.table
                                            ))
                                            .small()
                                            .weak(),
                                        );
                                        if ui.small_button("DDL").clicked() {
                                            pending_ddl_fetch = Some((
                                                db_name.clone(),
                                                "TRIGGER".to_string(),
                                                trig.name.clone(),
                                            ));
                                        }
                                    });
                                }
                            });
                        }

                        if !schema.events.is_empty() {
                            ui.collapsing(format!("Events ({})", schema.events.len()), |ui| {
                                for ev in &schema.events {
                                    if !filter_lower.is_empty()
                                        && !ev.name.to_lowercase().contains(&filter_lower)
                                    {
                                        continue;
                                    }
                                    ui.horizontal(|ui| {
                                        ui.label(&ev.name);
                                        let badge_color =
                                            if ev.status.eq_ignore_ascii_case("ENABLED") {
                                                pal.success
                                            } else {
                                                pal.muted_text
                                            };
                                        ui.colored_label(badge_color, &ev.status);
                                        if ui.small_button("DDL").clicked() {
                                            pending_ddl_fetch = Some((
                                                db_name.clone(),
                                                "EVENT".to_string(),
                                                ev.name.clone(),
                                            ));
                                        }
                                    });
                                }
                            });
                        }
                    });

                if db_resp.header_response.clicked() && !is_active_db {
                    activate_db = Some(db_name.clone());
                }
                let mut open_backup_for: Option<String> = None;
                let mut open_restore_for: Option<String> = None;
                db_resp.header_response.context_menu(|ui| {
                    if ui.button("Backup database...").clicked() {
                        open_backup_for = Some(db_name.clone());
                        ui.close();
                    }
                    if ui.button("Run SQL file...").clicked() {
                        open_restore_for = Some(db_name.clone());
                        ui.close();
                    }
                });
                if let Some(db) = open_restore_for {
                    self.restore_modal = Some(RestoreModalState {
                        file_path: None,
                        database: db,
                        policy: crate::backup::OnError::Stop,
                        running: false,
                        cancel: None,
                        bytes_done: 0,
                        bytes_total: 0,
                        statements_ok: 0,
                        statements_failed: 0,
                        errors: Vec::new(),
                        status: None,
                        prod_confirm_input: String::new(),
                    });
                }
                if let Some(db) = open_backup_for {
                    let all_tables: Vec<String> = self
                        .schemas
                        .get(&db)
                        .map(|s| s.tables.iter().map(|t| t.name.clone()).collect())
                        .unwrap_or_default();
                    let selected_tables = all_tables.iter().cloned().collect();
                    self.backup_modal = Some(BackupModalState {
                        database: db,
                        all_tables,
                        selected_tables,
                        include_data: true,
                        include_drop: true,
                        include_views: true,
                        include_routines: true,
                        include_triggers: true,
                        batch_rows: 500,
                        running: false,
                        cancel: None,
                        current_table: String::new(),
                        table_index: 0,
                        table_total: 0,
                        rows_done: 0,
                        status: None,
                    });
                }
            }
        });

        if let Some(db) = activate_db {
            self.active_database = db;
            self.reload_active_schema();
        } else if let Some(db) = load_db_schema
            && let Some(session) = self.active_session.clone()
        {
            let tx = self.tx.clone();
            self.rt.spawn(async move {
                if let Ok(schema) = session.introspect_database(&db).await {
                    let _ = tx.send(AsyncEvent::SchemaLoaded(schema));
                }
            });
        }

        if let Some(tbl_name) = toggle_favorite_table
            && let Some(prof) = self
                .config
                .connections
                .iter_mut()
                .find(|c| Some(&c.id) == self.active_profile_id.as_ref())
        {
            if let Some(pos) = prof
                .favorite_tables
                .iter()
                .position(|t| t.eq_ignore_ascii_case(&tbl_name))
            {
                prof.favorite_tables.remove(pos);
            } else {
                prof.favorite_tables.push(tbl_name);
            }
            let _ = self.config.save();
        }

        if let Some((sql_snippet, run_immediately)) = pending_sql_action {
            if let Some(tab) = self.tabs.get_mut(self.active_tab_idx) {
                if run_immediately {
                    tab.sql = sql_snippet;
                } else {
                    if !tab.sql.ends_with(' ') && !tab.sql.is_empty() {
                        tab.sql.push(' ');
                    }
                    tab.sql.push_str(&sql_snippet);
                }
            }
            if run_immediately {
                self.execute_current_tab_query(None);
            }
        }

        if let Some((db, obj_type, name)) = pending_ddl_fetch
            && let Some(session) = self.active_session.clone()
        {
            let tx = self.tx.clone();
            let title = format!("{obj_type}: {name}");
            self.rt.spawn(async move {
                let ddl = session
                    .fetch_object_ddl(&db, &obj_type, &name)
                    .await
                    .map_err(|e| format!("{e:#}"));
                let _ = tx.send(AsyncEvent::ObjectDdlLoaded { title, ddl });
            });
        }

        if let Some((db, routine_name)) = pending_routine_call
            && let Some(session) = self.active_session.clone()
        {
            let tx = self.tx.clone();
            self.rt.spawn(async move {
                let params = session
                    .fetch_routine_parameters(&db, &routine_name)
                    .await
                    .map_err(|e| format!("{e:#}"));
                let _ = tx.send(AsyncEvent::RoutineCallModalReady {
                    db,
                    routine_name,
                    params,
                });
            });
        }
    }

    pub(crate) fn render_connections_list(&mut self, ui: &mut Ui) {
        let pal = self.effective_palette(ui.ctx());
        ui.horizontal_wrapped(|ui| {
            if ui.add(pal.primary_button("+ New Profile")).clicked() {
                self.editing_connection = ConnectionProfile::default();
                self.show_connection_modal = true;
            }
            if ui
                .button("Import Workbench")
                .on_hover_text("Import connections from MySQL Workbench (connections.xml)")
                .clicked()
            {
                self.import_workbench_connections();
            }
        });
        ui.add_space(6.0);

        let profiles = self.config.connections.clone();
        let mut to_connect = None;
        let mut to_edit = None;
        let mut to_delete = None;

        if !self.config.trashed_connections.is_empty() {
            ui.horizontal(|ui| {
                if ui
                    .small_button(format!(
                        "Recycle Bin ({})",
                        self.config.trashed_connections.len()
                    ))
                    .on_hover_text("View and restore deleted connections")
                    .clicked()
                {
                    self.show_connection_trash_modal = true;
                }
            });
            ui.add_space(4.0);
        }

        egui::ScrollArea::vertical().show(ui, |ui| {
            for prof in profiles {
                let is_active = Some(&prof.id) == self.active_profile_id.as_ref();
                egui::Frame::new()
                    .fill(pal.card_bg)
                    .stroke(if is_active {
                        Stroke::new(1.0, pal.success)
                    } else {
                        Stroke::new(1.0, pal.card_border)
                    })
                    .corner_radius(egui::CornerRadius::same(8))
                    .inner_margin(10.0)
                    .show(ui, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(
                                RichText::new(&prof.name)
                                    .strong()
                                    .size(13.0)
                                    .color(pal.text_primary),
                            );
                            let (er, eg, eb) = prof.environment.color_rgb();
                            let (env_fg, env_bg) = pal.env_badge_colors(er, eg, eb);
                            pal.badge(ui, prof.environment.label(), env_fg, env_bg);
                            if prof.read_only {
                                pal.badge(ui, "RO", pal.warning, pal.warning_bg);
                            }
                            if is_active {
                                pal.badge(ui, "ONLINE", pal.success, pal.success_bg);
                            }
                        });
                        ui.add_space(2.0);
                        ui.label(
                            RichText::new(format!("{}@{}:{}", prof.user, prof.host, prof.port))
                                .small()
                                .color(pal.muted_text),
                        );
                        if prof.ssh.enabled {
                            ui.label(
                                RichText::new(format!("SSH: {}", prof.ssh.ssh_host))
                                    .size(10.5)
                                    .color(pal.accent_text),
                            );
                        }
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            let conn_label = if is_active { "Reconnect" } else { "Connect" };
                            let btn = if is_active {
                                egui::Button::new(
                                    RichText::new(conn_label)
                                        .size(11.5)
                                        .strong()
                                        .color(Color32::WHITE),
                                )
                                .fill(Color32::from_rgb(5, 150, 105))
                                .corner_radius(egui::CornerRadius::same(4))
                            } else {
                                egui::Button::new(
                                    RichText::new(conn_label)
                                        .size(11.5)
                                        .strong()
                                        .color(Color32::WHITE),
                                )
                                .fill(Color32::from_rgb(2, 132, 199))
                                .corner_radius(egui::CornerRadius::same(4))
                            };
                            if ui.add(btn).clicked() {
                                to_connect = Some(prof.clone());
                            }
                            if ui.small_button("Edit").clicked() {
                                to_edit = Some(prof.clone());
                            }
                            if ui
                                .small_button(RichText::new("Delete").color(pal.danger))
                                .clicked()
                            {
                                to_delete = Some(prof.id.clone());
                            }
                        });
                    });
                ui.add_space(4.0);
            }
        });

        if let Some(p) = to_connect {
            self.connect_to_profile(p);
        }
        if let Some(p) = to_edit {
            self.editing_connection = p;
            self.show_connection_modal = true;
        }
        if let Some(id) = to_delete {
            let name = self
                .config
                .connections
                .iter()
                .find(|c| c.id == id)
                .map(|c| c.name.clone())
                .unwrap_or_else(|| "this connection".to_string());
            self.pending_delete_connection = Some((id, name));
        }
    }

    pub(crate) fn render_saved_queries_panel(&mut self, ui: &mut Ui) {
        let pal = self.effective_palette(ui.ctx());
        let mut load_sql: Option<String> = None;
        let mut run_sql_now: Option<String> = None;
        let mut new_tab_entry: Option<(String, String)> = None;
        let mut delete_snippet_id: Option<String> = None;
        let mut export_snippet_sql: Option<(String, String)> = None;
        let mut rename_snippet: Option<(String, String)> = None;

        ui.horizontal_wrapped(|ui| {
            ui.label(
                RichText::new("Saved Queries")
                    .strong()
                    .color(pal.text_primary),
            );
            if ui
                .small_button("+ Save Active")
                .on_hover_text("Directly save active query to library and disk (Ctrl+S)")
                .clicked()
            {
                self.save_active_tab_direct();
            }
            if ui
                .small_button("Open File...")
                .on_hover_text("Open .sql file into a tab (Ctrl+O)")
                .clicked()
            {
                self.open_sql_file_dialog();
            }
        });
        ui.add_space(4.0);

        ui.horizontal(|ui| {
            let clear_w = if self.snippet_search_filter.is_empty() {
                0.0
            } else {
                24.0
            };
            ui.add(
                egui::TextEdit::singleline(&mut self.snippet_search_filter)
                    .hint_text("Search queries by name, DB, or SQL...")
                    .desired_width((ui.available_width() - clear_w).max(80.0)),
            );
            if !self.snippet_search_filter.is_empty()
                && ui.small_button("×").on_hover_text("Clear search").clicked()
            {
                self.snippet_search_filter.clear();
            }
        });
        ui.add_space(4.0);

        let filter = self.snippet_search_filter.to_lowercase();
        let total_count = self.config.snippets.len();

        if self.config.snippets.is_empty() {
            ui.vertical_centered(|ui| {
                ui.add_space(20.0);
                ui.label(RichText::new("No saved queries yet.").color(pal.muted_text));
                ui.add_space(6.0);
                if ui.button("Save Current Query Tab").clicked() {
                    self.save_active_tab_direct();
                }
            });
            return;
        }

        let matching_count = self
            .config
            .snippets
            .iter()
            .filter(|snip| {
                filter.is_empty()
                    || snip.title.to_lowercase().contains(&filter)
                    || snip.sql.to_lowercase().contains(&filter)
                    || snip.description.to_lowercase().contains(&filter)
                    || snip
                        .database
                        .as_deref()
                        .unwrap_or("")
                        .to_lowercase()
                        .contains(&filter)
            })
            .count();

        let count_text = if filter.is_empty() {
            format!("{total_count} saved queries")
        } else {
            format!("Showing {matching_count} of {total_count} queries")
        };
        ui.label(RichText::new(count_text).small().color(pal.muted_text));
        ui.add_space(2.0);

        egui::ScrollArea::vertical().show(ui, |ui| {
            for snip in &self.config.snippets {
                if !filter.is_empty()
                    && !snip.title.to_lowercase().contains(&filter)
                    && !snip.sql.to_lowercase().contains(&filter)
                    && !snip.description.to_lowercase().contains(&filter)
                    && !snip
                        .database
                        .as_deref()
                        .unwrap_or("")
                        .to_lowercase()
                        .contains(&filter)
                {
                    continue;
                }

                pal.compact_card_frame().show(ui, |ui| {
                    let is_renaming = self.renaming_snippet_id.as_deref() == Some(&snip.id);
                    ui.horizontal_wrapped(|ui| {
                        if is_renaming {
                            let edit_res = ui.add(
                                egui::TextEdit::singleline(&mut self.rename_snippet_buffer)
                                    .desired_width(120.0),
                            );
                            if edit_res.lost_focus() || ui.input(|i| i.key_pressed(Key::Enter)) {
                                rename_snippet =
                                    Some((snip.id.clone(), self.rename_snippet_buffer.clone()));
                            }
                            if ui.small_button("OK").clicked() {
                                rename_snippet =
                                    Some((snip.id.clone(), self.rename_snippet_buffer.clone()));
                            }
                            if ui.small_button("Cancel").clicked()
                                || ui.input(|i| i.key_pressed(Key::Escape))
                            {
                                self.renaming_snippet_id = None;
                            }
                        } else {
                            let label_res = ui.label(
                                RichText::new(&snip.title)
                                    .strong()
                                    .size(13.0)
                                    .color(pal.text_primary),
                            );
                            if label_res.double_clicked() {
                                self.renaming_snippet_id = Some(snip.id.clone());
                                self.rename_snippet_buffer = snip.title.clone();
                            }
                            if let Some(db) = &snip.database {
                                ui.colored_label(pal.accent_text, format!("[{db}]"));
                            }
                        }
                    });

                    if !snip.description.is_empty() {
                        ui.label(
                            RichText::new(&snip.description)
                                .small()
                                .color(pal.muted_text),
                        );
                    }

                    let preview_lines: Vec<&str> = snip.sql.lines().take(3).collect();
                    let preview = preview_lines.join("\n");
                    ui.label(
                        RichText::new(preview)
                            .monospace()
                            .small()
                            .color(pal.text_secondary),
                    );

                    ui.add_space(4.0);
                    ui.horizontal_wrapped(|ui| {
                        if ui
                            .small_button("Open Tab")
                            .on_hover_text("Open this query in a new editor tab")
                            .clicked()
                        {
                            new_tab_entry = Some((snip.title.clone(), snip.sql.clone()));
                        }
                        if ui
                            .small_button("Run")
                            .on_hover_text("Load and execute immediately")
                            .clicked()
                        {
                            run_sql_now = Some(snip.sql.clone());
                        }
                        if ui
                            .small_button("Paste")
                            .on_hover_text("Paste into active tab")
                            .clicked()
                        {
                            load_sql = Some(snip.sql.clone());
                        }
                        if ui
                            .small_button("Copy")
                            .on_hover_text("Copy SQL to clipboard")
                            .clicked()
                        {
                            ui.ctx().copy_text(snip.sql.clone());
                            self.status_banner = "Copied SQL to clipboard.".to_string();
                        }
                        if ui
                            .small_button("Export")
                            .on_hover_text("Save query as .sql file on disk")
                            .clicked()
                        {
                            export_snippet_sql = Some((snip.title.clone(), snip.sql.clone()));
                        }
                        if ui
                            .small_button("Rename")
                            .on_hover_text("Rename query (or double-click title)")
                            .clicked()
                        {
                            self.renaming_snippet_id = Some(snip.id.clone());
                            self.rename_snippet_buffer = snip.title.clone();
                        }
                        if ui
                            .small_button(RichText::new("Delete").color(pal.danger))
                            .on_hover_text("Delete saved query")
                            .clicked()
                        {
                            delete_snippet_id = Some(snip.id.clone());
                        }
                    });
                });
                ui.add_space(4.0);
            }
        });

        if let Some((id, new_title)) = rename_snippet {
            self.rename_saved_query(&id, &new_title);
            self.renaming_snippet_id = None;
        }

        if let Some(id) = delete_snippet_id {
            self.config.snippets.retain(|s| s.id != id);
            let _ = self.config.save();
            self.status_banner = "Deleted saved query.".to_string();
        }

        if let Some((title, sql)) = new_tab_entry {
            self.tabs.push(QueryTab::new(title, sql));
            self.active_tab_idx = self.tabs.len() - 1;
        }

        if let Some(sql) = load_sql
            && let Some(tab) = self.tabs.get_mut(self.active_tab_idx)
        {
            tab.sql = sql;
            tab.is_dirty = true;
        }

        if let Some(sql) = run_sql_now {
            if let Some(tab) = self.tabs.get_mut(self.active_tab_idx) {
                tab.sql = sql.clone();
            }
            self.execute_sql_in_current_tab(sql, None);
        }

        if let Some((title, sql)) = export_snippet_sql {
            let filename = format!("{}.sql", title.replace([' ', '/'], "_"));
            if let Some(path) = rfd::FileDialog::new()
                .set_file_name(&filename)
                .add_filter("SQL Files", &["sql"])
                .save_file()
            {
                let _ = std::fs::write(&path, sql);
                self.status_banner = format!("Exported query to {}", path.display());
            }
        }
    }

    pub(crate) fn render_query_history_panel(&mut self, ui: &mut Ui) {
        let pal = self.effective_palette(ui.ctx());
        let mut run_sql: Option<String> = None;
        let mut new_tab_sql: Option<String> = None;
        let mut save_snippet_from_history: Option<SavedSnippet> = None;
        let mut trigger_clear = false;

        ui.horizontal(|ui| {
            ui.label(
                RichText::new("Execution History")
                    .strong()
                    .color(pal.text_primary),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if !self.history_records.is_empty() {
                    if self.history_confirm_clear {
                        if ui
                            .small_button(RichText::new("Confirm Clear").color(pal.danger))
                            .clicked()
                        {
                            self.history_confirm_clear = false;
                            trigger_clear = true;
                        }
                        if ui.small_button("Cancel").clicked() {
                            self.history_confirm_clear = false;
                        }
                    } else if ui.small_button("Clear History").clicked() {
                        self.history_confirm_clear = true;
                    }
                }
            });
        });
        ui.add_space(4.0);

        ui.horizontal(|ui| {
            let clear_w = if self.history_search_filter.is_empty() {
                0.0
            } else {
                24.0
            };
            let search_resp = ui.add(
                egui::TextEdit::singleline(&mut self.history_search_filter)
                    .hint_text("Search history by SQL or DB...")
                    .desired_width((ui.available_width() - clear_w).max(80.0)),
            );
            if search_resp.changed() {
                self.history_limit = 100;
                self.history_dirty = true;
                self.history_last_input = Some(std::time::Instant::now());
            }
            if !self.history_search_filter.is_empty()
                && ui.small_button("×").on_hover_text("Clear search").clicked()
            {
                self.history_search_filter.clear();
                self.history_limit = 100;
                self.refresh_history();
            }
        });

        ui.horizontal_wrapped(|ui| {
            if ui
                .checkbox(&mut self.history_filter_current_conn, "Current conn")
                .changed()
            {
                self.history_limit = 100;
                self.refresh_history();
            }
            if ui
                .checkbox(&mut self.history_filter_failed_only, "Failed only")
                .changed()
            {
                self.history_limit = 100;
                self.refresh_history();
            }
            let db_label = if self.history_filter_database.is_empty() {
                "All DBs".to_string()
            } else {
                self.history_filter_database.clone()
            };
            egui::ComboBox::from_id_salt("history_db_filter")
                .selected_text(db_label)
                .width(100.0)
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_value(
                            &mut self.history_filter_database,
                            String::new(),
                            "All DBs",
                        )
                        .changed()
                    {
                        self.history_limit = 100;
                        self.refresh_history();
                    }
                    for db in &self.databases {
                        if ui
                            .selectable_value(&mut self.history_filter_database, db.clone(), db)
                            .changed()
                        {
                            self.history_limit = 100;
                            self.refresh_history();
                        }
                    }
                });
        });
        ui.add_space(4.0);

        if self.history_records.is_empty() {
            ui.vertical_centered(|ui| {
                ui.add_space(20.0);
                ui.label(RichText::new("No matching queries in history.").color(pal.muted_text));
            });
        } else {
            egui::ScrollArea::vertical().show(ui, |ui| {
                for entry in &self.history_records {
                    let short_ts = entry
                        .ts
                        .split('T')
                        .nth(1)
                        .and_then(|t| t.get(..8))
                        .unwrap_or(&entry.ts);
                    let first_line = entry
                        .sql
                        .lines()
                        .next()
                        .unwrap_or("")
                        .trim()
                        .chars()
                        .take(90)
                        .collect::<String>();
                    let hover_text = if let Some(err) = &entry.error {
                        format!("{}\n\nError: {err}", entry.sql)
                    } else {
                        entry.sql.clone()
                    };

                    let frame_resp = pal
                        .compact_card_frame()
                        .show(ui, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                if entry.success {
                                    ui.colored_label(pal.success, "✓");
                                } else {
                                    ui.colored_label(pal.danger, "✗");
                                }
                                ui.label(RichText::new(short_ts).small().color(pal.muted_text));
                                if !entry.database.is_empty() {
                                    ui.colored_label(
                                        pal.accent_text,
                                        format!("[{}]", entry.database),
                                    );
                                }
                                ui.label(
                                    RichText::new(format!(
                                        "{} rows • {} ms",
                                        entry.row_count, entry.duration_ms
                                    ))
                                    .small()
                                    .color(pal.muted_text),
                                );
                            });
                            let sql_resp = ui.add(
                                egui::Label::new(
                                    RichText::new(&first_line)
                                        .monospace()
                                        .small()
                                        .color(pal.text_primary),
                                )
                                .sense(egui::Sense::click()),
                            );
                            if sql_resp.clicked() {
                                new_tab_sql = Some(entry.sql.clone());
                            }
                            sql_resp.on_hover_text(&hover_text);

                            ui.add_space(2.0);
                            ui.horizontal(|ui| {
                                if ui.small_button("Run again").clicked() {
                                    run_sql = Some(entry.sql.clone());
                                }
                                if ui.small_button("Copy").clicked() {
                                    ui.ctx().copy_text(entry.sql.clone());
                                    self.status_banner = "Copied SQL to clipboard.".to_string();
                                }
                                if ui
                                    .small_button("Save")
                                    .on_hover_text("Save query into Library")
                                    .clicked()
                                {
                                    let title = first_line.chars().take(40).collect::<String>();
                                    let clean_title = if title.is_empty() {
                                        "Historic Query".to_string()
                                    } else {
                                        title
                                    };
                                    save_snippet_from_history = Some(SavedSnippet {
                                        id: uuid::Uuid::new_v4().to_string(),
                                        title: clean_title,
                                        sql: entry.sql.clone(),
                                        database: if entry.database.is_empty() {
                                            None
                                        } else {
                                            Some(entry.database.clone())
                                        },
                                        description: format!("Executed at {}", entry.ts),
                                        created_at: entry.ts.clone(),
                                    });
                                }
                            });
                        })
                        .response;
                    frame_resp.on_hover_text(&hover_text);
                    ui.add_space(4.0);
                }

                if self.history_has_more
                    && ui
                        .centered_and_justified(|ui| ui.button("Load more (100)"))
                        .inner
                        .clicked()
                {
                    self.history_limit += 100;
                    self.refresh_history();
                }
            });
        }

        if let Some(snip) = save_snippet_from_history {
            let title = snip.title.clone();
            self.config.snippets.push(snip);
            let _ = self.config.save();
            self.status_banner = format!("Saved '{title}' to library.");
        }

        if trigger_clear {
            self.clear_history();
        }

        if let Some(sql) = run_sql {
            if let Some(tab) = self.tabs.get_mut(self.active_tab_idx) {
                tab.sql = sql;
                tab.is_dirty = true;
            }
            self.execute_current_tab_query(None);
        }

        if let Some(sql) = new_tab_sql {
            let n = self.tabs.len() + 1;
            self.tabs.push(QueryTab::new(format!("Query {n}"), sql));
            self.active_tab_idx = self.tabs.len() - 1;
        }
    }
}
