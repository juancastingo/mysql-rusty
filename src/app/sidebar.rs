use super::*;

impl MysqlRustyApp {
    pub(crate) fn render_left_sidebar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::left("left_sidebar")
            .default_size(280.0)
            .size_range(220.0..=420.0)
            .show(ui, |ui| {
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    ui.selectable_value(
                        &mut self.sidebar_tab,
                        SidebarTab::Schema,
                        RichText::new(rust_i18n::t!("sidebar.schema")).size(12.0),
                    );
                    ui.selectable_value(
                        &mut self.sidebar_tab,
                        SidebarTab::Connections,
                        RichText::new(rust_i18n::t!("sidebar.servers")).size(12.0),
                    );
                    ui.selectable_value(
                        &mut self.sidebar_tab,
                        SidebarTab::SavedQueries,
                        RichText::new(rust_i18n::t!("sidebar.saved_queries")).size(12.0),
                    );
                    ui.selectable_value(
                        &mut self.sidebar_tab,
                        SidebarTab::History,
                        RichText::new(rust_i18n::t!("sidebar.history")).size(12.0),
                    );
                });
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
        if self.active_session.is_none() {
            ui.vertical_centered(|ui| {
                ui.add_space(30.0);
                ui.label(
                    RichText::new("No database connected").color(Color32::from_rgb(148, 163, 184)),
                );
                ui.add_space(8.0);
                if ui.button("Choose Connection...").clicked() {
                    self.sidebar_tab = SidebarTab::Connections;
                }
            });
            return;
        }

        // Database selector and quick switcher
        ui.horizontal(|ui| {
            ui.label(RichText::new("DB:").strong());
            let current_db = if self.active_database.is_empty() {
                "None"
            } else {
                &self.active_database
            };
            egui::ComboBox::from_id_salt("sidebar_schema_db_selector")
                .selected_text(current_db)
                .width(130.0)
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
                .small_button("Refresh")
                .on_hover_text("Refresh active database schema")
                .clicked()
            {
                self.reload_active_schema();
            }
        });

        if self.databases.len() > 1 {
            ui.horizontal_wrapped(|ui| {
                for db in self.databases.clone() {
                    let is_sel = self.active_database == db;
                    let text = if is_sel {
                        RichText::new(format!("[{db}]"))
                            .strong()
                            .color(Color32::from_rgb(56, 189, 248))
                    } else {
                        RichText::new(db.clone()).weak()
                    };
                    if ui.selectable_label(is_sel, text).clicked() {
                        self.active_database = db;
                        self.reload_active_schema();
                    }
                }
            });
        }
        ui.separator();

        ui.horizontal(|ui| {
            ui.label("Filter:");
            ui.add(
                egui::TextEdit::singleline(&mut self.schema_filter)
                    .hint_text("Search tables or columns..."),
            );
            if ui
                .small_button("+ New Table")
                .on_hover_text("Visually design and create a new table")
                .clicked()
            {
                self.table_designer_draft = Some(TableDraft::new_empty("new_table"));
            }
            if ui
                .small_button("📥 Import CSV")
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
        ui.add_space(4.0);

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
                        .color(Color32::from_rgb(56, 189, 248))
                } else {
                    RichText::new(db_name.as_str()).strong()
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
                                ui.label(RichText::new("Schema not loaded").small().weak());
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

                            let is_fav = fav_tables.iter().any(|f| f.eq_ignore_ascii_case(&table.name));
                            let star_prefix = if is_fav { "★ " } else { "" };
                            let header_label =
                                format!("{star_prefix}{} (~{})", table.name, table.approx_rows);
                            let header_text = if is_fav {
                                RichText::new(header_label)
                                    .strong()
                                    .color(Color32::from_rgb(250, 204, 21))
                            } else {
                                RichText::new(header_label).strong()
                            };
                            egui::CollapsingHeader::new(header_text)
                                .id_salt(("sidebar_tbl_tree", db_name, &table.name))
                                .default_open(is_active_db)
                                .show(ui, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            let pin_txt = if is_fav { "★ Unpin" } else { "☆ Pin" };
                            if ui
                                .small_button(pin_txt)
                                .on_hover_text("Pin/Unpin favorite table to top of Schema Explorer")
                                .clicked()
                            {
                                toggle_favorite_table = Some(table.name.clone());
                            }
                            if ui.small_button("SELECT *").clicked() {
                                pending_sql_action = Some((
                                    format!("SELECT * FROM `{}`.`{}` LIMIT 100;", db_name, table.name),
                                    true,
                                ));
                            }
                            if ui.small_button("DDL").clicked() {
                                pending_sql_action = Some((
                                    format!("SHOW CREATE TABLE `{}`.`{}`;", db_name, table.name),
                                    true,
                                ));
                            }
                            if ui
                                .small_button("Design Table")
                                .on_hover_text("Visual table designer to alter columns, indexes, and FKs")
                                .clicked()
                            {
                                self.table_designer_draft = Some(TableDraft::from_table(table));
                            }
                            if ui
                                .small_button("Info")
                                .on_hover_text("Inspect Columns, Indexes, Foreign Keys, and DDL")
                                .clicked()
                            {
                                self.table_info_modal = Some((db_name.clone(), (*table).clone(), 0));
                            }
                            if ui
                                .small_button("Dup / Trunc")
                                .on_hover_text("Duplicate structure/data or truncate this table")
                                .clicked()
                            {
                                self.table_action_modal = Some((
                                    db_name.clone(),
                                    table.name.clone(),
                                    format!("{}_copy", table.name),
                                ));
                            }
                            if ui
                                .small_button("Import CSV")
                                .on_hover_text("Import CSV file as batch INSERT statements into this table")
                                .clicked()
                                && let Some(path) = rfd::FileDialog::new()
                                    .add_filter("CSV", &["csv", "txt"])
                                    .pick_file()
                                    && let Ok(content) = std::fs::read_to_string(&path) {
                                        match parse_csv_to_batch_insert(&table.name, &content) {
                                            Ok(batch_sql) => {
                                                self.tabs.push(QueryTab::new(
                                                    format!("Import {}", table.name),
                                                    batch_sql,
                                                ));
                                                self.active_tab_idx = self.tabs.len() - 1;
                                                self.status_banner = format!(
                                                    "Generated batch INSERT statements from `{}`",
                                                    path.file_name().unwrap_or_default().to_string_lossy()
                                                );
                                            }
                                            Err(err) => {
                                                self.status_banner = format!("Failed to parse CSV: {err}");
                                            }
                                        }
                                    }
                            if ui
                                .small_button("Mock Data")
                                .on_hover_text("Generate realistic random seed data for this table")
                                .clicked()
                            {
                                self.mock_data_target_table = Some(table.name.clone());
                                self.show_mock_data_modal = true;
                            }
                            if ui.small_button("Insert Name").clicked() {
                                pending_sql_action = Some((table.name.clone(), false));
                            }
                        });

                        for col in &table.columns {
                            let icon = if col.is_primary_key { "[PK] " } else { "     " };
                            ui.horizontal(|ui| {
                                ui.label(format!("{icon}{}", col.name));
                                ui.label(
                                    RichText::new(&col.data_type)
                                        .color(Color32::from_rgb(100, 116, 139))
                                        .small(),
                                );
                            });
                        }

                        if !table.foreign_keys.is_empty() {
                            ui.collapsing(
                                RichText::new(format!(
                                    "[FK] Foreign Keys ({})",
                                    table.foreign_keys.len()
                                ))
                                .small(),
                                |ui| {
                                    for fk in &table.foreign_keys {
                                        ui.label(
                                            RichText::new(format!(
                                                "{} -> {}.{}",
                                                fk.column_name,
                                                fk.referenced_table,
                                                fk.referenced_column
                                            ))
                                            .small(),
                                        );
                                    }
                                },
                            );
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
                                                Color32::from_rgb(52, 211, 153)
                                            } else {
                                                Color32::from_rgb(148, 163, 184)
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
        if ui.button("+ Add MySQL / MariaDB Profile").clicked() {
            self.editing_connection = ConnectionProfile::default();
            self.show_connection_modal = true;
        }
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
                    .fill(Color32::from_rgb(17, 24, 39))
                    .stroke(if is_active {
                        Stroke::new(1.0, Color32::from_rgb(16, 185, 129))
                    } else {
                        Stroke::new(1.0, Color32::from_rgb(31, 41, 55))
                    })
                    .corner_radius(egui::CornerRadius::same(8))
                    .inner_margin(10.0)
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&prof.name).strong().size(13.0));
                            let (er, eg, eb) = prof.environment.color_rgb();
                            ui.colored_label(
                                Color32::from_rgb(er, eg, eb),
                                format!("[{}]", prof.environment.label()),
                            );
                            if prof.read_only {
                                ui.colored_label(Color32::from_rgb(251, 146, 60), "[Read-Only]");
                            }
                            if is_active {
                                egui::Frame::new()
                                    .fill(Color32::from_rgb(6, 78, 59))
                                    .corner_radius(egui::CornerRadius::same(4))
                                    .inner_margin(egui::Margin::symmetric(5, 1))
                                    .show(ui, |ui| {
                                        ui.label(
                                            RichText::new("ONLINE")
                                                .size(9.5)
                                                .color(Color32::from_rgb(52, 211, 153))
                                                .strong(),
                                        );
                                    });
                            }
                        });
                        ui.add_space(2.0);
                        ui.label(
                            RichText::new(format!("{}@{}:{}", prof.user, prof.host, prof.port))
                                .small()
                                .color(Color32::from_rgb(148, 163, 184)),
                        );
                        if prof.ssh.enabled {
                            ui.label(
                                RichText::new(format!("SSH: {}", prof.ssh.ssh_host))
                                    .size(10.5)
                                    .color(Color32::from_rgb(56, 189, 248)),
                            );
                        }
                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            let conn_label = if is_active { "Reconnect" } else { "Connect" };
                            let btn =
                                egui::Button::new(RichText::new(conn_label).size(11.5).strong())
                                    .fill(if is_active {
                                        Color32::from_rgb(6, 78, 59)
                                    } else {
                                        Color32::from_rgb(2, 132, 199)
                                    })
                                    .corner_radius(egui::CornerRadius::same(4));
                            if ui.add(btn).clicked() {
                                to_connect = Some(prof.clone());
                            }
                            if ui.small_button("Edit").clicked() {
                                to_edit = Some(prof.clone());
                            }
                            if ui.small_button("Delete").clicked() {
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
        let mut load_sql: Option<String> = None;
        let mut run_sql_now: Option<String> = None;
        let mut new_tab_entry: Option<(String, String)> = None;
        let mut delete_snippet_id: Option<String> = None;
        let mut export_snippet_sql: Option<(String, String)> = None;
        let mut rename_snippet: Option<(String, String)> = None;

        ui.horizontal(|ui| {
            ui.label(RichText::new("Saved Queries Library").strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .button("+ Save Active")
                    .on_hover_text("Directly save active query to library and disk (Ctrl+S)")
                    .clicked()
                {
                    self.save_active_tab_direct();
                }
                if ui
                    .button("Open File...")
                    .on_hover_text("Open .sql file into a tab (Ctrl+O)")
                    .clicked()
                {
                    self.open_sql_file_dialog();
                }
            });
        });
        ui.add_space(2.0);

        ui.add(
            egui::TextEdit::singleline(&mut self.snippet_search_filter)
                .hint_text("Search queries by name, tag, database, or SQL...")
                .desired_width(ui.available_width()),
        );
        ui.add_space(4.0);

        let filter = self.snippet_search_filter.to_lowercase();
        let total_count = self.config.snippets.len();

        if self.config.snippets.is_empty() {
            ui.vertical_centered(|ui| {
                ui.add_space(20.0);
                ui.label(RichText::new("No saved queries yet.").weak());
                ui.add_space(6.0);
                if ui.button("Save Current Query Tab").clicked() {
                    self.save_active_tab_direct();
                }
            });
            return;
        }

        ui.label(
            RichText::new(format!("Showing queries ({} total)", total_count))
                .small()
                .weak(),
        );
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

                egui::Frame::new()
                    .fill(Color32::from_rgb(17, 24, 39))
                    .stroke(Stroke::new(1.0, Color32::from_rgb(31, 41, 55)))
                    .corner_radius(egui::CornerRadius::same(6))
                    .inner_margin(8.0)
                    .show(ui, |ui| {
                        let is_renaming = self.renaming_snippet_id.as_deref() == Some(&snip.id);
                        ui.horizontal(|ui| {
                            if is_renaming {
                                let edit_res = ui.add(
                                    egui::TextEdit::singleline(&mut self.rename_snippet_buffer)
                                        .desired_width(120.0),
                                );
                                if edit_res.lost_focus() || ui.input(|i| i.key_pressed(Key::Enter))
                                {
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
                                let label_res =
                                    ui.label(RichText::new(&snip.title).strong().size(13.0));
                                if label_res.double_clicked() {
                                    self.renaming_snippet_id = Some(snip.id.clone());
                                    self.rename_snippet_buffer = snip.title.clone();
                                }
                                if let Some(db) = &snip.database {
                                    ui.colored_label(
                                        Color32::from_rgb(56, 189, 248),
                                        format!("[{db}]"),
                                    );
                                }
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if ui
                                            .small_button("Delete")
                                            .on_hover_text("Delete saved query")
                                            .clicked()
                                        {
                                            delete_snippet_id = Some(snip.id.clone());
                                        }
                                        if ui
                                            .small_button("Rename")
                                            .on_hover_text("Rename query (or double-click title)")
                                            .clicked()
                                        {
                                            self.renaming_snippet_id = Some(snip.id.clone());
                                            self.rename_snippet_buffer = snip.title.clone();
                                        }
                                    },
                                );
                            }
                        });

                        if !snip.description.is_empty() {
                            ui.label(
                                RichText::new(&snip.description)
                                    .small()
                                    .color(Color32::from_rgb(148, 163, 184)),
                            );
                        }

                        let preview_lines: Vec<&str> = snip.sql.lines().take(3).collect();
                        let preview = preview_lines.join("\n");
                        ui.label(RichText::new(preview).monospace().small().weak());

                        ui.add_space(2.0);
                        ui.horizontal_wrapped(|ui| {
                            if ui
                                .small_button("Open Tab")
                                .on_hover_text("Open this query in a new editor tab")
                                .clicked()
                            {
                                new_tab_entry = Some((snip.title.clone(), snip.sql.clone()));
                            }
                            if ui
                                .small_button("Run Now")
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
                                .small_button("Export .sql")
                                .on_hover_text("Save query as .sql file on disk")
                                .clicked()
                            {
                                export_snippet_sql = Some((snip.title.clone(), snip.sql.clone()));
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
        let mut run_sql: Option<String> = None;
        let mut new_tab_sql: Option<String> = None;
        let mut save_snippet_from_history: Option<SavedSnippet> = None;
        let mut trigger_clear = false;

        ui.horizontal(|ui| {
            ui.label(RichText::new("Execution History").strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if !self.history_records.is_empty() {
                    if self.history_confirm_clear {
                        if ui
                            .small_button(
                                RichText::new("Confirm Clear")
                                    .color(Color32::from_rgb(248, 113, 113)),
                            )
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
        ui.add_space(2.0);

        let search_resp = ui.add(
            egui::TextEdit::singleline(&mut self.history_search_filter)
                .hint_text("Search SQLite history by SQL or database...")
                .desired_width(ui.available_width()),
        );
        if search_resp.changed() {
            self.history_limit = 100;
            self.history_dirty = true;
            self.history_last_input = Some(std::time::Instant::now());
        }

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
                ui.label(RichText::new("No matching queries in history.").weak());
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

                    let frame_resp = egui::Frame::new()
                        .fill(Color32::from_rgb(15, 23, 42))
                        .stroke(Stroke::new(1.0, Color32::from_rgb(30, 41, 59)))
                        .corner_radius(egui::CornerRadius::same(6))
                        .inner_margin(8.0)
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                if entry.success {
                                    ui.colored_label(Color32::from_rgb(52, 211, 153), "✓");
                                } else {
                                    ui.colored_label(Color32::from_rgb(248, 113, 113), "✗");
                                }
                                ui.label(RichText::new(short_ts).small().weak());
                                if !entry.database.is_empty() {
                                    ui.colored_label(
                                        Color32::from_rgb(56, 189, 248),
                                        format!("[{}]", entry.database),
                                    );
                                }
                                ui.label(
                                    RichText::new(format!(
                                        "{} rows • {} ms",
                                        entry.row_count, entry.duration_ms
                                    ))
                                    .small()
                                    .weak(),
                                );
                            });
                            let sql_resp = ui.add(
                                egui::Label::new(RichText::new(&first_line).monospace().small())
                                    .sense(egui::Sense::click()),
                            );
                            if sql_resp.clicked() {
                                new_tab_sql = Some(entry.sql.clone());
                            }
                            sql_resp.on_hover_text(&hover_text);

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
