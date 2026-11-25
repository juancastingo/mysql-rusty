use super::super::super::*;

impl MysqlRustyApp {
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
}
