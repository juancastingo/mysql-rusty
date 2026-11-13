use super::super::*;

impl MysqlRustyApp {
    pub(crate) fn render_schema_diff_modal(&mut self, ctx: &egui::Context) {
        if !self.show_schema_diff_modal {
            return;
        }
        let mut open = self.show_schema_diff_modal;
        let mut open_in_tab_sql = None;

        egui::Window::new("Schema Diff & DDL Synchronization")
            .open(&mut open)
            .default_width(650.0)
            .default_height(480.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Source DB:").strong());
                    egui::ComboBox::from_id_salt("diff_src_cb")
                        .selected_text(if self.diff_source_db.is_empty() {
                            "Select Source..."
                        } else {
                            &self.diff_source_db
                        })
                        .show_ui(ui, |ui| {
                            for db in &self.databases {
                                ui.selectable_value(&mut self.diff_source_db, db.clone(), db);
                            }
                        });

                    ui.label(RichText::new("Target DB:").strong());
                    egui::ComboBox::from_id_salt("diff_tgt_cb")
                        .selected_text(if self.diff_target_db.is_empty() {
                            "Select Target..."
                        } else {
                            &self.diff_target_db
                        })
                        .show_ui(ui, |ui| {
                            for db in &self.databases {
                                ui.selectable_value(&mut self.diff_target_db, db.clone(), db);
                            }
                        });

                    if ui.button("Compare").clicked() {
                        if let (Some(src), Some(tgt)) = (
                            self.schemas.get(&self.diff_source_db),
                            self.schemas.get(&self.diff_target_db),
                        ) {
                            self.current_diff_report = Some(diff_schemas(src, tgt));
                        } else {
                            self.status_banner =
                                "Please ensure both database schemas are loaded before comparing."
                                    .to_string();
                        }
                    }
                });
                ui.separator();

                if let Some(report) = &self.current_diff_report {
                    ui.label(
                        RichText::new(format!(
                            "Diff Summary ({} changes detected):",
                            report.summary.len()
                        ))
                        .strong(),
                    );
                    egui::ScrollArea::vertical()
                        .max_height(130.0)
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
                    egui::ScrollArea::vertical()
                        .max_height(180.0)
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
                    });
                } else {
                    ui.vertical_centered(|ui| {
                        ui.add_space(30.0);
                        ui.label(RichText::new("Select Source and Target databases above, then click 'Compare' to generate a synchronization DDL script.").weak());
                    });
                }
            });

        if let Some(sql) = open_in_tab_sql {
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

        egui::Window::new("Intelligent Mock Data Generator")
            .open(&mut open)
            .default_width(520.0)
            .show(ctx, |ui| {
                if let (Some(tbl_name), Some(schema)) = (table_opt.as_ref(), schema_opt.as_ref()) {
                    if let Some(table) = schema.tables.iter().find(|t| t.name == *tbl_name) {
                        ui.label(
                            RichText::new(format!(
                                "Target Table: `{tbl_name}` ({} columns)",
                                table.columns.len()
                            ))
                            .strong(),
                        );
                        ui.add_space(4.0);

                        ui.horizontal(|ui| {
                            ui.label("Rows to generate:");
                            ui.add(
                                egui::DragValue::new(&mut self.mock_data_row_count).range(1..=1000),
                            );
                            if ui.small_button("10").clicked() {
                                self.mock_data_row_count = 10;
                            }
                            if ui.small_button("25").clicked() {
                                self.mock_data_row_count = 25;
                            }
                            if ui.small_button("50").clicked() {
                                self.mock_data_row_count = 50;
                            }
                            if ui.small_button("100").clicked() {
                                self.mock_data_row_count = 100;
                            }
                        });

                        ui.separator();
                        let preview_sql =
                            generate_mock_data_sql(table, self.mock_data_row_count.min(8));
                        ui.label(RichText::new("Data Preview (Sample rows):").strong());
                        egui::ScrollArea::vertical()
                            .max_height(140.0)
                            .show(ui, |ui| {
                                ui.code(preview_sql);
                            });

                        ui.add_space(8.0);
                        ui.horizontal(|ui| {
                            if ui.button("Open in Query Tab").clicked() {
                                let full_sql =
                                    generate_mock_data_sql(table, self.mock_data_row_count);
                                generate_to_tab = Some((tbl_name.clone(), full_sql));
                            }
                            if ui.button("▶ Insert Directly into DB").clicked() {
                                let full_sql =
                                    generate_mock_data_sql(table, self.mock_data_row_count);
                                insert_directly = Some(full_sql);
                            }
                            if ui.button("Cancel").clicked() {
                                self.show_mock_data_modal = false;
                            }
                        });
                    } else {
                        ui.label("Table not found in active database schema.");
                    }
                } else {
                    ui.label("No active database or table selected.");
                }
            });

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
        let Some(wizard) = &mut self.csv_wizard else {
            return;
        };
        let mut close_modal = false;
        let mut trigger_import = false;

        egui::Window::new("📥 CSV Import Wizard")
            .collapsible(false)
            .resizable(true)
            .default_size([650.0, 520.0])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.heading(
                        RichText::new("Import Data from CSV")
                            .color(Color32::from_rgb(56, 189, 248))
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
                    ui.colored_label(Color32::from_rgb(239, 68, 68), err);
                }
                if let Some(msg) = &wizard.status_message {
                    ui.colored_label(Color32::from_rgb(34, 197, 94), msg);
                }

                ui.separator();
                ui.horizontal(|ui| {
                    let target_name = if wizard.create_new_table {
                        wizard.new_table_name.trim()
                    } else {
                        wizard.target_table.trim()
                    };
                    let can_import = !target_name.is_empty() && !wizard.is_importing;

                    let btn = egui::Button::new(
                        RichText::new("🚀 Import Now")
                            .strong()
                            .color(Color32::from_rgb(16, 185, 129)),
                    )
                    .fill(Color32::from_rgb(6, 78, 59));
                    if ui.add_enabled(can_import, btn).clicked() {
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
        let mut open_sync_in_tab = false;

        egui::Window::new("Data Diff (Row Comparison)")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(780.0)
            .default_height(500.0)
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label("Source:");
                    egui::ComboBox::from_id_salt("dd_src_db")
                        .selected_text(&self.data_diff_source_db)
                        .width(120.0)
                        .show_ui(ui, |ui| {
                            for db in &self.databases {
                                ui.selectable_value(&mut self.data_diff_source_db, db.clone(), db);
                            }
                        });
                    ui.add(
                        egui::TextEdit::singleline(&mut self.data_diff_source_table)
                            .desired_width(120.0)
                            .hint_text("source table"),
                    );

                    ui.label("Target:");
                    egui::ComboBox::from_id_salt("dd_tgt_db")
                        .selected_text(&self.data_diff_target_db)
                        .width(120.0)
                        .show_ui(ui, |ui| {
                            for db in &self.databases {
                                ui.selectable_value(&mut self.data_diff_target_db, db.clone(), db);
                            }
                        });
                    ui.add(
                        egui::TextEdit::singleline(&mut self.data_diff_target_table)
                            .desired_width(120.0)
                            .hint_text("target table"),
                    );

                    ui.label("Key Col #:");
                    ui.add(egui::DragValue::new(&mut self.data_diff_key_col).range(0..=30));

                    if ui
                        .add_enabled(!self.data_diff_loading, egui::Button::new("Compare Rows"))
                        .clicked()
                    {
                        run_diff = true;
                    }
                });

                ui.separator();

                if self.data_diff_loading {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Loading and comparing table rows...");
                    });
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
                            if ui.small_button("Open Sync SQL in Tab").clicked() {
                                open_sync_in_tab = true;
                            }
                            if ui.small_button("Copy Sync SQL").clicked() {
                                ui.ctx().copy_text(self.data_diff_sync_sql.clone());
                            }
                        });
                    });
                    ui.separator();

                    egui::ScrollArea::both()
                        .max_height(260.0)
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
                    egui::ScrollArea::vertical()
                        .max_height(110.0)
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
            });

        self.show_data_diff_modal = open;

        if open_sync_in_tab {
            self.tabs.push(QueryTab::new(
                format!("DataSync {}", self.data_diff_target_table),
                self.data_diff_sync_sql.clone(),
            ));
            self.active_tab_idx = self.tabs.len() - 1;
            self.show_data_diff_modal = false;
        }

        if run_diff
            && !self.data_diff_source_table.trim().is_empty()
            && !self.data_diff_target_table.trim().is_empty()
            && let Some(session) = self.active_session.clone()
        {
            self.data_diff_loading = true;
            let src_db = self.data_diff_source_db.clone();
            let src_tbl = self.data_diff_source_table.clone();
            let tgt_db = self.data_diff_target_db.clone();
            let tgt_tbl = self.data_diff_target_table.clone();
            let key_col = self.data_diff_key_col;
            let tx = self.tx.clone();
            self.rt.spawn(async move {
                let sql_l = format!("SELECT * FROM `{src_db}`.`{src_tbl}` LIMIT 500;");
                let sql_r = format!("SELECT * FROM `{tgt_db}`.`{tgt_tbl}` LIMIT 500;");
                let left = session
                    .execute_query(&src_db, &sql_l, None, None)
                    .await
                    .map_err(|e| format!("{e:#}"));
                let right = session
                    .execute_query(&tgt_db, &sql_r, None, None)
                    .await
                    .map_err(|e| format!("{e:#}"));
                let _ = tx.send(AsyncEvent::DataDiffLoaded {
                    left,
                    right,
                    key_col_idx: key_col,
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
                    egui::ScrollArea::vertical()
                        .id_salt("dump_preview_scroll")
                        .max_height(210.0)
                        .show(ui, |ui| {
                            ui.add(
                                egui::TextEdit::multiline(&mut self.dump_preview_sql)
                                    .font(egui::TextStyle::Monospace)
                                    .desired_width(f32::INFINITY),
                            );
                        });
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
}
