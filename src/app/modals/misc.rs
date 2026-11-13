use super::super::*;

impl MysqlRustyApp {
    pub(crate) fn render_safety_modal(&mut self, ctx: &egui::Context) {
        if !self.show_safety_modal {
            return;
        }
        let mut open = self.show_safety_modal;
        let mut execute_confirmed = false;

        egui::Window::new(rust_i18n::t!("safety_modal.title"))
            .open(&mut open)
            .resizable(false)
            .default_width(460.0)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(Color32::from_rgb(69, 10, 10))
                    .stroke(Stroke::new(1.0, Color32::from_rgb(239, 68, 68)))
                    .corner_radius(egui::CornerRadius::same(8))
                    .inner_margin(12.0)
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(&self.safety_warning_msg)
                                .color(Color32::from_rgb(254, 202, 202))
                                .strong(),
                        );
                    });

                ui.add_space(8.0);
                ui.label(RichText::new(rust_i18n::t!("safety_modal.target_stmt")).strong());
                egui::ScrollArea::vertical()
                    .max_height(110.0)
                    .show(ui, |ui| {
                        ui.code(&self.pending_safe_sql);
                    });

                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui
                        .add(
                            egui::Button::new(
                                RichText::new(rust_i18n::t!("safety_modal.proceed"))
                                    .color(Color32::from_rgb(239, 68, 68))
                                    .strong(),
                            )
                            .fill(Color32::from_rgb(127, 29, 29)),
                        )
                        .clicked()
                    {
                        execute_confirmed = true;
                    }

                    if ui.button(rust_i18n::t!("safety_modal.cancel")).clicked() {
                        self.show_safety_modal = false;
                        self.pending_safe_sql.clear();
                    }
                });
            });

        if execute_confirmed {
            self.show_safety_modal = false;
            let sql = std::mem::take(&mut self.pending_safe_sql);
            self.execute_sql_forced(sql);
        } else if !open {
            self.show_safety_modal = false;
            self.pending_safe_sql.clear();
        }
    }

    pub(crate) fn render_save_snippet_modal(&mut self, ctx: &egui::Context) {
        if !self.show_save_snippet_modal {
            return;
        }
        let mut open = self.show_save_snippet_modal;
        let mut save_to_library = false;
        let mut save_as_file = false;

        egui::Window::new("Save Query")
            .open(&mut open)
            .default_width(480.0)
            .show(ctx, |ui| {
                ui.label(RichText::new("Query Name / Title:").strong());
                ui.add(
                    egui::TextEdit::singleline(&mut self.new_snippet_title)
                        .hint_text("e.g. Monthly Revenue by Category")
                        .desired_width(ui.available_width()),
                );
                ui.add_space(4.0);

                ui.horizontal(|ui| {
                    ui.label("Database Tag:");
                    let current_db = if self.new_snippet_db.is_empty() {
                        "Any / None"
                    } else {
                        &self.new_snippet_db
                    };
                    egui::ComboBox::from_id_salt("save_query_db_combo")
                        .selected_text(current_db)
                        .width(180.0)
                        .show_ui(ui, |ui| {
                            if ui
                                .selectable_label(self.new_snippet_db.is_empty(), "Any / None")
                                .clicked()
                            {
                                self.new_snippet_db.clear();
                            }
                            for db in &self.databases {
                                let is_sel = &self.new_snippet_db == db;
                                if ui.selectable_label(is_sel, db).clicked() {
                                    self.new_snippet_db = db.clone();
                                }
                            }
                        });
                });
                ui.add_space(4.0);

                ui.label("Description / Notes (Optional):");
                ui.add(
                    egui::TextEdit::multiline(&mut self.new_snippet_desc)
                        .desired_rows(2)
                        .hint_text("Add context, filters, or notes for this query...")
                        .desired_width(ui.available_width()),
                );

                if let Some(tab) = self.tabs.get(self.active_tab_idx) {
                    ui.add_space(6.0);
                    ui.label(RichText::new("SQL Preview:").strong().small());
                    egui::ScrollArea::vertical()
                        .max_height(100.0)
                        .show(ui, |ui| {
                            ui.code(&tab.sql);
                        });
                }

                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    let save_btn = egui::Button::new(
                        RichText::new("Save to Library")
                            .color(Color32::WHITE)
                            .strong(),
                    )
                    .fill(Color32::from_rgb(14, 116, 144));
                    if ui.add(save_btn).clicked() {
                        save_to_library = true;
                    }

                    if ui
                        .button("Save as .sql File...")
                        .on_hover_text("Save query as a standalone .sql file on disk")
                        .clicked()
                    {
                        save_as_file = true;
                    }

                    if ui.button("Cancel").clicked() {
                        self.show_save_snippet_modal = false;
                    }
                });
            });

        if save_to_library {
            self.show_save_snippet_modal = false;
            if let Some(tab) = self.tabs.get(self.active_tab_idx) {
                let title = if self.new_snippet_title.trim().is_empty() {
                    "Saved Query".to_string()
                } else {
                    self.new_snippet_title.trim().to_string()
                };
                let now_str = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();
                self.config.snippets.push(SavedSnippet {
                    id: Uuid::new_v4().to_string(),
                    title,
                    sql: tab.sql.clone(),
                    database: if self.new_snippet_db.trim().is_empty() {
                        None
                    } else {
                        Some(self.new_snippet_db.trim().to_string())
                    },
                    description: self.new_snippet_desc.trim().to_string(),
                    created_at: now_str,
                });
                let _ = self.config.save();
                self.status_banner = "Saved query to app library successfully.".to_string();
            }
        } else if save_as_file {
            self.show_save_snippet_modal = false;
            self.save_active_tab_to_file(true);
        } else if !open {
            self.show_save_snippet_modal = false;
        }
    }

    pub(crate) fn render_cell_inspector_modal(&mut self, ctx: &egui::Context) {
        let Some(inspector) = &self.cell_inspector else {
            return;
        };
        let mut close_modal = false;
        let mut copy_val: Option<String> = None;

        egui::Window::new("🔍 Cell Value & JSON Inspector")
            .collapsible(false)
            .resizable(true)
            .default_size([580.0, 420.0])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!("Column: `{}`", inspector.column_name))
                            .strong()
                            .color(Color32::from_rgb(56, 189, 248)),
                    );
                    if let Some(tbl) = &inspector.table_name {
                        ui.label(RichText::new(format!("Table: `{tbl}`")).weak());
                    }
                    ui.label(RichText::new(format!("Row: #{}", inspector.row_index + 1)).weak());
                    if inspector.formatted_json.is_some() {
                        ui.label(
                            RichText::new("[VALID JSON]")
                                .color(Color32::from_rgb(34, 197, 94))
                                .strong()
                                .small(),
                        );
                    }
                });
                ui.separator();

                let is_json = inspector.formatted_json.is_some();
                let display_text = if let Some(pretty) = &inspector.formatted_json {
                    pretty
                } else {
                    &inspector.raw_value
                };

                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!(
                            "Length: {} chars ({} bytes){}",
                            display_text.len(),
                            display_text.len(),
                            if is_json { " (Prettified JSON)" } else { "" }
                        ))
                        .small()
                        .weak(),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("📋 Copy Value").clicked() {
                            copy_val = Some(display_text.clone());
                        }
                    });
                });
                ui.add_space(4.0);

                egui::ScrollArea::both().max_height(280.0).show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut display_text.as_str())
                            .font(egui::TextStyle::Monospace)
                            .code_editor()
                            .desired_width(f32::INFINITY),
                    );
                });

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Close").clicked() {
                        close_modal = true;
                    }
                });
            });

        if let Some(val) = copy_val {
            ctx.copy_text(val);
            self.status_banner = "Copied cell content to clipboard.".to_string();
        }
        if close_modal {
            self.cell_inspector = None;
        }
    }

    pub(crate) fn render_command_palette_modal(&mut self, ctx: &egui::Context) {
        if !self.show_command_palette {
            return;
        }

        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.show_command_palette = false;
            return;
        }

        #[derive(Clone)]
        enum PaletteAction {
            NewTab,
            RunQuery,
            ExplainQuery,
            FormatSql,
            OpenCsvImport,
            NewTableDesigner,
            OpenSchemaDiff,
            OpenServerAdmin,
            StartTransaction,
            ToggleAiPanel,
            OpenTable(String),
            OpenSnippet { title: String, sql: String },
            OpenLogsFolder,
            OpenRunSqlFile,
            ToggleVimMode,
            OpenShortcuts,
        }

        struct PaletteEntry {
            category: &'static str,
            label: String,
            detail: String,
            action: PaletteAction,
        }

        let mut entries = vec![
            PaletteEntry {
                category: "Action",
                label: "New Query Tab".to_string(),
                detail: "Open a blank SQL scratchpad tab".to_string(),
                action: PaletteAction::NewTab,
            },
            PaletteEntry {
                category: "Action",
                label: "Run Query".to_string(),
                detail: format!(
                    "Execute SQL in current tab ({})",
                    self.keymap.label(crate::shortcuts::Action::RunQuery)
                ),
                action: PaletteAction::RunQuery,
            },
            PaletteEntry {
                category: "Action",
                label: "Explain Query".to_string(),
                detail: "Run EXPLAIN on current query".to_string(),
                action: PaletteAction::ExplainQuery,
            },
            PaletteEntry {
                category: "Action",
                label: "Format SQL".to_string(),
                detail: "Pretty-print SQL in current tab".to_string(),
                action: PaletteAction::FormatSql,
            },
            PaletteEntry {
                category: "Action",
                label: "Import CSV...".to_string(),
                detail: "Open CSV Import Wizard".to_string(),
                action: PaletteAction::OpenCsvImport,
            },
            PaletteEntry {
                category: "Action",
                label: "New Table Designer".to_string(),
                detail: "Create a new table visually".to_string(),
                action: PaletteAction::NewTableDesigner,
            },
            PaletteEntry {
                category: "Action",
                label: "Schema Diff".to_string(),
                detail: "Compare schemas between databases".to_string(),
                action: PaletteAction::OpenSchemaDiff,
            },
            PaletteEntry {
                category: "Action",
                label: "Server Admin & Variables".to_string(),
                detail: "Inspect SHOW VARIABLES and status".to_string(),
                action: PaletteAction::OpenServerAdmin,
            },
            PaletteEntry {
                category: "Action",
                label: "Start Transaction".to_string(),
                detail: "BEGIN a manual transaction".to_string(),
                action: PaletteAction::StartTransaction,
            },
            PaletteEntry {
                category: "Action",
                label: "Toggle AI Assistant".to_string(),
                detail: "Show or hide AI SQL assistant panel".to_string(),
                action: PaletteAction::ToggleAiPanel,
            },
            PaletteEntry {
                category: "Action",
                label: "Open logs folder".to_string(),
                detail: "Open application log directory in file manager".to_string(),
                action: PaletteAction::OpenLogsFolder,
            },
            PaletteEntry {
                category: "Action",
                label: "Run SQL file...".to_string(),
                detail: "Stream and execute a large .sql dump file".to_string(),
                action: PaletteAction::OpenRunSqlFile,
            },
            PaletteEntry {
                category: "Action",
                label: "Toggle Vim Mode".to_string(),
                detail: "Enable or disable Vim keybindings in SQL editor".to_string(),
                action: PaletteAction::ToggleVimMode,
            },
            PaletteEntry {
                category: "Help",
                label: "Keyboard shortcuts".to_string(),
                detail: format!(
                    "View and customize keybindings ({})",
                    self.keymap.label(crate::shortcuts::Action::ShortcutsHelp)
                ),
                action: PaletteAction::OpenShortcuts,
            },
        ];

        if let Some(schema) = self.schemas.get(&self.active_database) {
            for t in &schema.tables {
                entries.push(PaletteEntry {
                    category: "Table",
                    label: t.name.clone(),
                    detail: format!("SELECT * FROM `{}` (~{} rows)", t.name, t.approx_rows),
                    action: PaletteAction::OpenTable(t.name.clone()),
                });
            }
        }

        for snip in &self.config.snippets {
            entries.push(PaletteEntry {
                category: "Snippet",
                label: snip.title.clone(),
                detail: snip.sql.lines().next().unwrap_or("").to_string(),
                action: PaletteAction::OpenSnippet {
                    title: snip.title.clone(),
                    sql: snip.sql.clone(),
                },
            });
        }

        let query_lower = self.command_palette_query.trim().to_lowercase();
        let terms: Vec<&str> = query_lower.split_whitespace().collect();
        let filtered: Vec<PaletteEntry> = entries
            .into_iter()
            .filter(|e| {
                if terms.is_empty() {
                    return true;
                }
                let hay = format!("{} {} {}", e.category, e.label, e.detail).to_lowercase();
                terms.iter().all(|t| hay.contains(t))
            })
            .take(50)
            .collect();

        if filtered.is_empty() {
            self.command_palette_selected = 0;
        } else if self.command_palette_selected >= filtered.len() {
            self.command_palette_selected = filtered.len() - 1;
        }

        let (down_pressed, up_pressed, enter_pressed) = ctx.input(|i| {
            (
                i.key_pressed(egui::Key::ArrowDown),
                i.key_pressed(egui::Key::ArrowUp),
                i.key_pressed(egui::Key::Enter),
            )
        });

        if down_pressed && !filtered.is_empty() {
            self.command_palette_selected = (self.command_palette_selected + 1) % filtered.len();
        }
        if up_pressed && !filtered.is_empty() {
            self.command_palette_selected = if self.command_palette_selected == 0 {
                filtered.len() - 1
            } else {
                self.command_palette_selected - 1
            };
        }

        let mut chosen_action: Option<PaletteAction> = if enter_pressed && !filtered.is_empty() {
            Some(filtered[self.command_palette_selected].action.clone())
        } else {
            None
        };

        let mut open = self.show_command_palette;
        egui::Window::new("Command Palette")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .title_bar(false)
            .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 72.0))
            .default_width(540.0)
            .show(ctx, |ui| {
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut self.command_palette_query)
                        .hint_text("Type a command, table, or saved snippet... (Esc to close)")
                        .desired_width(f32::INFINITY),
                );
                resp.request_focus();
                if resp.changed() {
                    self.command_palette_selected = 0;
                }

                ui.add_space(6.0);
                ui.separator();

                if filtered.is_empty() {
                    ui.label(
                        RichText::new("No matching commands, tables, or snippets.")
                            .color(Color32::GRAY)
                            .italics(),
                    );
                } else {
                    egui::ScrollArea::vertical()
                        .max_height(320.0)
                        .show(ui, |ui| {
                            for (idx, item) in filtered.iter().enumerate() {
                                let is_selected = idx == self.command_palette_selected;
                                let bg = if is_selected {
                                    Color32::from_rgb(30, 58, 138)
                                } else {
                                    Color32::TRANSPARENT
                                };
                                let frame = egui::Frame::NONE
                                    .fill(bg)
                                    .corner_radius(egui::CornerRadius::same(4))
                                    .inner_margin(egui::Margin::symmetric(8, 5));

                                let row_resp = frame
                                    .show(ui, |ui| {
                                        ui.set_width(ui.available_width());
                                        ui.horizontal(|ui| {
                                            let badge_color = match item.category {
                                                "Table" => Color32::from_rgb(56, 189, 248),
                                                "Snippet" => Color32::from_rgb(250, 204, 21),
                                                _ => Color32::from_rgb(52, 211, 153),
                                            };
                                            ui.label(
                                                RichText::new(format!("[{}]", item.category))
                                                    .small()
                                                    .strong()
                                                    .color(badge_color),
                                            );
                                            ui.label(RichText::new(&item.label).strong());
                                            ui.with_layout(
                                                egui::Layout::right_to_left(egui::Align::Center),
                                                |ui| {
                                                    ui.label(
                                                        RichText::new(&item.detail).small().color(
                                                            Color32::from_rgb(148, 163, 184),
                                                        ),
                                                    );
                                                },
                                            );
                                        });
                                    })
                                    .response;

                                if row_resp.interact(egui::Sense::click()).clicked() {
                                    chosen_action = Some(item.action.clone());
                                }
                            }
                        });
                }
            });

        self.show_command_palette = open;

        if let Some(action) = chosen_action {
            self.show_command_palette = false;
            match action {
                PaletteAction::NewTab => {
                    let n = self.tabs.len() + 1;
                    self.tabs
                        .push(QueryTab::new(format!("Query {}", n), String::new()));
                    self.active_tab_idx = self.tabs.len() - 1;
                }
                PaletteAction::RunQuery => {
                    self.execute_current_tab_query(None);
                }
                PaletteAction::ExplainQuery => {
                    self.execute_current_tab_query(Some("EXPLAIN"));
                }
                PaletteAction::FormatSql => {
                    if let Some(tab) = self.tabs.get_mut(self.active_tab_idx) {
                        tab.sql = format_sql_pretty(&tab.sql);
                    }
                }
                PaletteAction::OpenCsvImport => {
                    self.open_csv_import_dialog();
                }
                PaletteAction::OpenLogsFolder => {
                    let dir = AppConfig::logs_dir();
                    #[cfg(target_os = "linux")]
                    let cmd = "xdg-open";
                    #[cfg(target_os = "macos")]
                    let cmd = "open";
                    #[cfg(target_os = "windows")]
                    let cmd = "explorer";
                    if let Err(err) = std::process::Command::new(cmd).arg(&dir).spawn() {
                        tracing::warn!(error = %err, dir = %dir.display(), "Failed to open logs folder");
                        self.status_banner = format!("Logs folder: {}", dir.display());
                    }
                }
                PaletteAction::OpenRunSqlFile => {
                    self.restore_modal = Some(RestoreModalState {
                        file_path: None,
                        database: self.active_database.clone(),
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
                PaletteAction::ToggleVimMode => {
                    self.config.ui.vim_mode = !self.config.ui.vim_mode;
                    let _ = self.config.save();
                    self.status_banner = if self.config.ui.vim_mode {
                        "Vim Mode enabled".to_string()
                    } else {
                        "Vim Mode disabled".to_string()
                    };
                }
                PaletteAction::NewTableDesigner => {
                    self.table_designer_draft = Some(TableDraft::new_empty("new_table"));
                }
                PaletteAction::OpenSchemaDiff => {
                    self.show_schema_diff_modal = true;
                }
                PaletteAction::OpenServerAdmin => {
                    self.bottom_tab = BottomTab::ServerAdmin;
                }
                PaletteAction::StartTransaction => {
                    self.start_transaction();
                }
                PaletteAction::ToggleAiPanel => {
                    self.show_ai_panel = !self.show_ai_panel;
                }
                PaletteAction::OpenTable(table_name) => {
                    let sql = format!("SELECT * FROM `{}` LIMIT 100;", table_name);
                    self.tabs.push(QueryTab::new(table_name, sql));
                    self.active_tab_idx = self.tabs.len() - 1;
                    self.execute_current_tab_query(None);
                }
                PaletteAction::OpenSnippet { title, sql } => {
                    self.tabs.push(QueryTab::new(title, sql));
                    self.active_tab_idx = self.tabs.len() - 1;
                }
                PaletteAction::OpenShortcuts => {
                    self.show_shortcuts_modal = true;
                    self.rebinding_action = None;
                    self.rebind_conflict_msg = None;
                }
            }
        }
    }

    pub(crate) fn render_shortcuts_modal(&mut self, ctx: &egui::Context) {
        if !self.show_shortcuts_modal {
            return;
        }

        if let Some(target_action) = self.rebinding_action {
            let captured = ctx.input(crate::shortcuts::capture_shortcut_from_input);
            if let Some(sc) = captured {
                if sc.key == egui::Key::Escape && sc.modifiers == egui::Modifiers::NONE {
                    self.rebinding_action = None;
                    self.rebind_conflict_msg = None;
                } else if let Some(conflict) = self.keymap.find_conflict(sc, target_action) {
                    self.rebind_conflict_msg = Some(format!(
                        "{} is already bound to \"{}\". Press another key or Esc to cancel.",
                        sc.format(),
                        conflict.label()
                    ));
                } else {
                    self.keymap.set(target_action, sc);
                    self.config.keybindings = self.keymap.to_overrides();
                    let _ = self.config.save();
                    self.rebinding_action = None;
                    self.rebind_conflict_msg = None;
                }
            }
        } else if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.show_shortcuts_modal = false;
            return;
        }

        let mut open = self.show_shortcuts_modal;
        egui::Window::new("Keyboard Shortcuts")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(600.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Search:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.shortcuts_filter)
                            .hint_text("Filter actions or shortcuts...")
                            .desired_width(280.0),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("Reset to defaults").clicked() {
                            self.keymap.reset_to_defaults();
                            self.config.keybindings.clear();
                            let _ = self.config.save();
                            self.rebinding_action = None;
                            self.rebind_conflict_msg = None;
                        }
                    });
                });

                if let Some(msg) = &self.rebind_conflict_msg {
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(msg)
                            .small()
                            .color(Color32::from_rgb(248, 113, 113)),
                    );
                }

                ui.separator();

                let filter_lower = self.shortcuts_filter.trim().to_lowercase();
                egui::ScrollArea::vertical()
                    .max_height(380.0)
                    .show(ui, |ui| {
                        egui::Grid::new("shortcuts_help_grid")
                            .num_columns(4)
                            .striped(true)
                            .spacing([14.0, 6.0])
                            .show(ui, |ui| {
                                ui.label(RichText::new("Category").strong());
                                ui.label(RichText::new("Action").strong());
                                ui.label(RichText::new("Binding").strong());
                                ui.label("");
                                ui.end_row();

                                for &action in crate::shortcuts::Action::all() {
                                    let binding_str = self.keymap.label(action);
                                    if !filter_lower.is_empty() {
                                        let hay = format!(
                                            "{} {} {}",
                                            action.category(),
                                            action.label(),
                                            binding_str
                                        )
                                        .to_lowercase();
                                        if !hay.contains(&filter_lower) {
                                            continue;
                                        }
                                    }

                                    ui.label(
                                        RichText::new(action.category())
                                            .small()
                                            .color(Color32::from_rgb(148, 163, 184)),
                                    );
                                    ui.label(action.label());

                                    let is_rebinding = self.rebinding_action == Some(action);
                                    if is_rebinding {
                                        ui.label(
                                            RichText::new("Press new key...")
                                                .strong()
                                                .color(Color32::from_rgb(250, 204, 21)),
                                        );
                                        if ui.button("Cancel").clicked() {
                                            self.rebinding_action = None;
                                            self.rebind_conflict_msg = None;
                                        }
                                    } else {
                                        ui.code(&binding_str);
                                        if ui.button("Edit").clicked() {
                                            self.rebinding_action = Some(action);
                                            self.rebind_conflict_msg = None;
                                        }
                                    }
                                    ui.end_row();
                                }
                            });
                    });
            });
        self.show_shortcuts_modal = open;
        if !self.show_shortcuts_modal {
            self.rebinding_action = None;
            self.rebind_conflict_msg = None;
        }
    }
}
