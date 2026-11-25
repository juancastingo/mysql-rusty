use super::super::super::*;

impl MysqlRustyApp {
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
