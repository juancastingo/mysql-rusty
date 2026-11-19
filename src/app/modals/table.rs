use super::super::*;

impl MysqlRustyApp {
    pub(crate) fn render_table_designer_modal(&mut self, ctx: &egui::Context) {
        let Some(mut draft) = self.table_designer_draft.take() else {
            return;
        };
        let mut keep_open = true;
        let mut action = TableDesignerAction::None;

        egui::Window::new("Visual Table Designer")
            .collapsible(false)
            .resizable(true)
            .default_size([740.0, 580.0])
            .open(&mut keep_open)
            .show(ctx, |ui| {
                action = draft.render_editor(ui);
            });

        if !keep_open || action == TableDesignerAction::Close {
            self.table_designer_draft = None;
        } else {
            match action {
                TableDesignerAction::ApplyDdl(ddl) => {
                    self.table_designer_draft = None;
                    self.execute_sql_in_current_tab(ddl, None);
                    self.reload_active_schema();
                }
                TableDesignerAction::OpenInQueryTab(ddl) => {
                    self.table_designer_draft = None;
                    let n = self.tabs.len() + 1;
                    self.tabs.push(QueryTab::new(format!("DDL {n}"), ddl));
                    self.active_tab_idx = self.tabs.len() - 1;
                }
                TableDesignerAction::None => {
                    self.table_designer_draft = Some(draft);
                }
                TableDesignerAction::Close => {
                    self.table_designer_draft = None;
                }
            }
        }
    }

    pub(crate) fn render_table_action_modal(&mut self, ctx: &egui::Context) {
        let Some((db_name, table_name, mut new_name)) = self.table_action_modal.clone() else {
            return;
        };
        let pal = self.effective_palette(ctx);
        let mut open = true;
        let mut sql_to_run: Option<String> = None;
        let mut close_now = false;

        egui::Window::new(format!("Table Actions: `{table_name}`"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .default_width(400.0)
            .show(ctx, |ui| {
                ui.label(RichText::new("Duplicate Table").strong());
                ui.horizontal(|ui| {
                    ui.label("New name:");
                    ui.text_edit_singleline(&mut new_name);
                });
                ui.add_space(4.0);
                let has_name = !new_name.trim().is_empty();
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(has_name, egui::Button::new("Duplicate Structure Only"))
                        .clicked()
                    {
                        sql_to_run = Some(format!(
                            "CREATE TABLE `{db_name}`.`{}` LIKE `{db_name}`.`{table_name}`;",
                            new_name.trim()
                        ));
                        close_now = true;
                    }
                    if ui
                        .add_enabled(has_name, pal.primary_button("Duplicate Structure + Data"))
                        .clicked()
                    {
                        let target = new_name.trim();
                        sql_to_run = Some(format!(
                            "CREATE TABLE `{db_name}`.`{target}` LIKE `{db_name}`.`{table_name}`;\nINSERT INTO `{db_name}`.`{target}` SELECT * FROM `{db_name}`.`{table_name}`;"
                        ));
                        close_now = true;
                    }
                });
                ui.separator();
                ui.label(RichText::new("Danger Zone").strong().color(pal.danger));
                ui.add_space(2.0);
                if ui.add(pal.danger_button("Truncate Table")).clicked() {
                    sql_to_run = Some(format!("TRUNCATE TABLE `{db_name}`.`{table_name}`;"));
                    close_now = true;
                }
            });

        if close_now || !open {
            self.table_action_modal = None;
        } else {
            self.table_action_modal = Some((db_name, table_name.clone(), new_name));
        }

        if let Some(sql) = sql_to_run {
            self.tabs
                .push(QueryTab::new(format!("Action {table_name}"), sql));
            self.active_tab_idx = self.tabs.len() - 1;
            self.execute_current_tab_query(None);
        }
    }

    pub(crate) fn render_table_info_modal(&mut self, ctx: &egui::Context) {
        let Some((db_name, tbl, mut subtab)) = self.table_info_modal.clone() else {
            return;
        };
        let pal = self.effective_palette(ctx);
        let mut open = true;

        egui::Window::new(format!("Table Info: `{db_name}`.`{}`", tbl.name))
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .default_width(580.0)
            .default_height(400.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut subtab, 0, format!("Columns ({})", tbl.columns.len()));
                    ui.selectable_value(
                        &mut subtab,
                        1,
                        format!("Foreign Keys ({})", tbl.foreign_keys.len()),
                    );
                    ui.selectable_value(&mut subtab, 2, "DDL");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(format!("~{} rows", tbl.approx_rows))
                                .small()
                                .color(pal.muted_text),
                        );
                    });
                });
                ui.separator();

                egui::ScrollArea::vertical()
                    .id_salt("table_info_scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| match subtab {
                        0 => {
                            for col in &tbl.columns {
                                ui.horizontal(|ui| {
                                    if col.is_primary_key {
                                        pal.badge(ui, "PK", pal.warning_bg, pal.warning);
                                    }
                                    ui.label(RichText::new(&col.name).strong());
                                    ui.label(
                                        RichText::new(&col.data_type)
                                            .monospace()
                                            .color(pal.accent_text),
                                    );
                                    if !col.is_nullable {
                                        ui.label(
                                            RichText::new("NOT NULL")
                                                .small()
                                                .color(pal.muted_text),
                                        );
                                    }
                                    if let Some(def) = &col.default_value {
                                        ui.label(
                                            RichText::new(format!("DEFAULT {def}"))
                                                .small()
                                                .color(pal.muted_text),
                                        );
                                    }
                                });
                            }
                        }
                        1 => {
                            if tbl.foreign_keys.is_empty() {
                                ui.label(
                                    RichText::new("No foreign keys defined.")
                                        .color(pal.muted_text),
                                );
                            } else {
                                for fk in &tbl.foreign_keys {
                                    ui.label(format!(
                                        "{} -> {}.{}",
                                        fk.column_name, fk.referenced_table, fk.referenced_column
                                    ));
                                }
                            }
                        }
                        _ => {
                            let mut ddl = format_table_create_ddl(&tbl);
                            ui.horizontal(|ui| {
                                if ui.small_button("Copy DDL").clicked() {
                                    ui.ctx().copy_text(ddl.clone());
                                }
                            });
                            ui.add(
                                egui::TextEdit::multiline(&mut ddl)
                                    .font(egui::TextStyle::Monospace)
                                    .desired_width(f32::INFINITY),
                            );
                        }
                    });
            });

        if !open {
            self.table_info_modal = None;
        } else {
            self.table_info_modal = Some((db_name, tbl, subtab));
        }
    }

    pub(crate) fn render_routine_call_modal(&mut self, ctx: &egui::Context) {
        let pal = self.effective_palette(ctx);
        let Some(state) = self.routine_call_modal.as_mut() else {
            return;
        };
        let mut open = true;
        let mut cancel_clicked = false;
        let mut execute_sql: Option<String> = None;

        egui::Window::new(format!(
            "Execute Procedure — `{}`.`{}`",
            state.db, state.routine_name
        ))
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
        .default_width(440.0)
        .show(ctx, |ui| {
            if state.params.is_empty() {
                ui.label("This procedure takes no parameters.");
            } else {
                egui::Grid::new("routine_params_grid")
                    .num_columns(3)
                    .spacing([10.0, 6.0])
                    .show(ui, |ui| {
                        for (idx, p) in state.params.iter().enumerate() {
                            ui.label(
                                RichText::new(format!("[{}] {}", p.mode, p.name))
                                    .monospace()
                                    .strong(),
                            );
                            ui.label(RichText::new(&p.data_type).small().color(pal.muted_text));
                            if p.mode.eq_ignore_ascii_case("OUT") {
                                ui.label(
                                    RichText::new("(OUT variable)")
                                        .small()
                                        .color(pal.muted_text),
                                );
                            } else if let Some(buf) = state.arg_values.get_mut(idx) {
                                ui.add(
                                    egui::TextEdit::singleline(buf)
                                        .hint_text("e.g. 42 or 'text' or NULL")
                                        .desired_width(180.0),
                                );
                            }
                            ui.end_row();
                        }
                    });
            }
            ui.separator();
            let preview = crate::db::generate_routine_call_sql(
                &state.routine_name,
                &state.params,
                &state.arg_values,
            );
            ui.label(
                RichText::new(&preview)
                    .monospace()
                    .small()
                    .color(pal.muted_text),
            );
            ui.add_space(6.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add(pal.primary_button("Execute")).clicked() {
                    execute_sql = Some(preview);
                }
                if ui.button("Cancel").clicked() {
                    cancel_clicked = true;
                }
            });
        });

        if let Some(sql) = execute_sql {
            self.routine_call_modal = None;
            let n = self.tabs.len() + 1;
            self.tabs.push(QueryTab::new(format!("Call {n}"), sql));
            self.active_tab_idx = self.tabs.len() - 1;
            self.execute_current_tab_query(None);
        } else if !open || cancel_clicked {
            self.routine_call_modal = None;
        }
    }
}
