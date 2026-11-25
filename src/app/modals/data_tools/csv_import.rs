use super::super::super::*;

impl MysqlRustyApp {
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
}
