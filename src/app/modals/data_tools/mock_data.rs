use super::super::super::*;

impl MysqlRustyApp {
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
}
