use super::*;

impl MysqlRustyApp {
    pub(crate) fn render_open_connections_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("open_connections_tab_bar")
            .exact_size(30.0)
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.add_space(4.0);
                    let is_home = self.active_workspace_idx.is_none();
                    let home_btn = egui::Button::new(
                        RichText::new(rust_i18n::t!("connections_bar.home"))
                            .size(12.0)
                            .strong()
                            .color(if is_home {
                                Color32::WHITE
                            } else {
                                Color32::from_rgb(148, 163, 184)
                            }),
                    )
                    .fill(if is_home {
                        Color32::from_rgb(30, 58, 138)
                    } else {
                        Color32::from_rgb(20, 27, 42)
                    })
                    .corner_radius(egui::CornerRadius::same(4));

                    if ui
                        .add(home_btn)
                        .on_hover_text(rust_i18n::t!("connections_bar.home_tooltip"))
                        .clicked()
                    {
                        self.switch_to_workspace(None);
                    }

                    let mut switch_target: Option<usize> = None;
                    let mut close_target: Option<usize> = None;

                    for (idx, ws) in self.open_workspaces.iter().enumerate() {
                        let is_active = self.active_workspace_idx == Some(idx);
                        let bg = if is_active {
                            Color32::from_rgb(15, 39, 68)
                        } else {
                            Color32::from_rgb(19, 25, 38)
                        };
                        let border = if is_active {
                            Stroke::new(1.0, Color32::from_rgb(56, 189, 248))
                        } else {
                            Stroke::new(1.0, Color32::from_rgb(33, 43, 63))
                        };

                        egui::Frame::new()
                            .fill(bg)
                            .stroke(border)
                            .corner_radius(egui::CornerRadius::same(4))
                            .inner_margin(egui::Margin::symmetric(8, 2))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    let (r, g, b) = ws.env_color;
                                    ui.label(
                                        RichText::new("●")
                                            .small()
                                            .color(Color32::from_rgb(r, g, b)),
                                    );
                                    let label_text =
                                        RichText::new(&ws.title).size(12.0).color(if is_active {
                                            Color32::WHITE
                                        } else {
                                            Color32::from_rgb(203, 213, 225)
                                        });
                                    let label_text = if is_active {
                                        label_text.strong()
                                    } else {
                                        label_text
                                    };
                                    if ui.selectable_label(false, label_text).clicked() {
                                        switch_target = Some(idx);
                                    }
                                    if ui
                                        .small_button(
                                            RichText::new("×")
                                                .small()
                                                .color(Color32::from_rgb(248, 113, 113)),
                                        )
                                        .on_hover_text(format!("Close connection `{}`", ws.title))
                                        .clicked()
                                    {
                                        close_target = Some(idx);
                                    }
                                });
                            });
                    }

                    if ui
                        .small_button("+")
                        .on_hover_text(rust_i18n::t!("connections_bar.new_conn_tooltip"))
                        .clicked()
                    {
                        self.switch_to_workspace(None);
                    }

                    if let Some(idx) = close_target {
                        self.close_workspace(idx);
                    } else if let Some(idx) = switch_target {
                        self.switch_to_workspace(Some(idx));
                    }
                });
            });
    }

    pub(crate) fn render_top_toolbar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("top_toolbar")
            .exact_size(48.0)
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.add_space(4.0);
                    // Brand Logo with glowing accent
                    ui.label(
                        RichText::new("MySQL-Rusty")
                            .size(16.0)
                            .color(Color32::from_rgb(56, 189, 248))
                            .strong(),
                    );
                    ui.add_space(4.0);
                    ui.separator();
                    ui.add_space(4.0);

                    // Connection selector & status indicator
                    let is_connected = self.active_session.is_some();
                    let select_conn_default = rust_i18n::t!("toolbar.select_connection");
                    let active_conn_name = self
                        .config
                        .connections
                        .iter()
                        .find(|c| Some(&c.id) == self.active_profile_id.as_ref())
                        .map(|c| c.name.as_str())
                        .unwrap_or(select_conn_default.as_ref());

                    let status_dot = if is_connected {
                        RichText::new("●").color(Color32::from_rgb(16, 185, 129))
                    } else {
                        RichText::new("○").color(Color32::from_rgb(148, 163, 184))
                    };
                    ui.label(status_dot);

                    egui::ComboBox::from_id_salt("conn_selector")
                        .selected_text(RichText::new(active_conn_name).strong())
                        .width(220.0)
                        .show_ui(ui, |ui| {
                            let profiles = self.config.connections.clone();
                            for prof in profiles {
                                let is_sel = Some(&prof.id) == self.active_profile_id.as_ref();
                                if ui.selectable_label(is_sel, &prof.name).clicked() {
                                    self.connect_to_profile(prof);
                                }
                            }
                        });

                    if is_connected {
                        if ui
                            .button(RichText::new(rust_i18n::t!("toolbar.disconnect")).small().color(Color32::from_rgb(248, 113, 113)))
                            .on_hover_text("Disconnect from active database session")
                            .clicked()
                        {
                            self.disconnect();
                        }
                    } else if ui
                        .button(RichText::new(rust_i18n::t!("toolbar.servers_dashboard")).small())
                        .on_hover_text("Open servers and connection profiles")
                        .clicked()
                    {
                        self.sidebar_tab = SidebarTab::Connections;
                    }

                    if ui.button(rust_i18n::t!("toolbar.new_btn")).on_hover_text("Add new connection profile").clicked() {
                        self.editing_connection = ConnectionProfile::default();
                        self.show_connection_modal = true;
                    }

                    ui.add_space(4.0);
                    ui.separator();
                    ui.add_space(4.0);

                    // Database Selector
                    ui.label(RichText::new(rust_i18n::t!("toolbar.db_label")).weak());
                    let none_db = rust_i18n::t!("toolbar.none");
                    let current_db = if self.active_database.is_empty() {
                        none_db.as_ref()
                    } else {
                        &self.active_database
                    };
                    egui::ComboBox::from_id_salt("db_selector")
                        .selected_text(current_db)
                        .width(150.0)
                        .show_ui(ui, |ui| {
                            let dbs = self.databases.clone();
                            for db in dbs {
                                let selected = self.active_database == db;
                                if ui.selectable_label(selected, &db).clicked() {
                                    self.active_database = db;
                                    self.reload_active_schema();
                                }
                            }
                        });

                    if ui
                        .button("↻")
                        .on_hover_text("Refresh database schema metadata")
                        .clicked()
                    {
                        self.reload_active_schema();
                    }

                    if is_connected {
                        if ui
                            .small_button(rust_i18n::t!("toolbar.data_diff"))
                            .on_hover_text("Compare table rows across databases/tables")
                            .clicked()
                        {
                            self.show_data_diff_modal = true;
                            if self.data_diff_source_db.is_empty() {
                                self.data_diff_source_db = self.active_database.clone();
                                self.data_diff_target_db = self.active_database.clone();
                            }
                        }
                        if ui
                            .small_button(rust_i18n::t!("toolbar.sql_dump"))
                            .on_hover_text("Export or import .sql database dumps")
                            .clicked()
                        {
                            self.show_sql_dump_modal = true;
                            if self.dump_selected_tables.is_empty()
                                && let Some(schema) = self.schemas.get(&self.active_database) {
                                    for t in &schema.tables {
                                        self.dump_selected_tables.insert(t.name.clone());
                                    }
                                }
                        }
                    }

                    ui.add_space(4.0);
                    ui.separator();
                    ui.add_space(4.0);

                    // Query execution controls
                    let has_selection = self
                        .tabs
                        .get(self.active_tab_idx)
                        .and_then(|t| t.selected_sql.as_ref())
                        .is_some();
                    let run_key = self.keymap.label(crate::shortcuts::Action::RunQuery);
                    let run_label = if has_selection {
                        rust_i18n::t!("toolbar.run_selection", key = run_key)
                    } else {
                        rust_i18n::t!("toolbar.run", key = run_key)
                    };
                    let is_running = self
                        .tabs
                        .get(self.active_tab_idx)
                        .map(|t| t.is_running)
                        .unwrap_or(false);

                    let run_btn = egui::Button::new(
                        RichText::new(run_label)
                            .color(Color32::WHITE)
                            .strong(),
                    )
                    .fill(Color32::from_rgb(16, 185, 129))
                    .corner_radius(egui::CornerRadius::same(6));

                    if ui.add(run_btn).clicked() {
                        self.auto_retry_attempted = false;
                        self.execute_current_tab_query(None);
                    }

                    if self.connection_lost.is_some() {
                        let reconnect_btn = egui::Button::new(
                            RichText::new(" Reconnect ").color(Color32::WHITE).strong(),
                        )
                        .fill(Color32::from_rgb(220, 38, 38))
                        .corner_radius(egui::CornerRadius::same(6));
                        if ui
                            .add(reconnect_btn)
                            .on_hover_text(
                                self.connection_lost
                                    .as_deref()
                                    .unwrap_or("Connection lost — click to reconnect"),
                            )
                            .clicked()
                        {
                            self.auto_retry_attempted = false;
                            self.reconnect_active_session();
                        }
                    }

                    if is_running {
                        let stop_btn = egui::Button::new(
                            RichText::new(" ■ Stop ")
                                .color(Color32::WHITE)
                                .strong(),
                        )
                        .fill(Color32::from_rgb(225, 29, 72))
                        .corner_radius(egui::CornerRadius::same(6));

                        if ui
                            .add(stop_btn)
                            .on_hover_text("Abort running query execution")
                            .clicked()
                        {
                            if let Some(tab) = self.tabs.get_mut(self.active_tab_idx) {
                                if let Some(handle) = tab.task_abort_handle.take() {
                                    handle.abort();
                                }
                                tab.is_running = false;
                                tab.last_error = Some("Query cancelled by user".to_string());
                            }
                            if let Some(session) = self.active_session.clone()
                                && let Some(conn_id) = session.editor_connection_id()
                            {
                                self.rt.spawn(async move {
                                    if let Err(err) = session.kill_query(conn_id).await {
                                        tracing::warn!(conn_id, error = %err, "Failed to KILL QUERY on server");
                                    } else {
                                        tracing::info!(conn_id, "Cancelled query on server via KILL QUERY");
                                    }
                                });
                            }
                            self.status_banner = "Query cancelled by user.".to_string();
                        }
                    }

                    if !has_selection {
                        let run_stmt_btn = egui::Button::new(
                            RichText::new(" ↳ Stmt ")
                                .color(Color32::from_rgb(209, 250, 229))
                                .size(12.0),
                        )
                        .fill(Color32::from_rgb(6, 78, 59))
                        .stroke(Stroke::new(1.0, Color32::from_rgb(16, 185, 129)))
                        .corner_radius(egui::CornerRadius::same(6));

                        if ui
                            .add(run_stmt_btn)
                            .on_hover_text("Execute the SQL statement currently under cursor (Ctrl+Shift+Enter)")
                            .clicked()
                        {
                            self.execute_current_statement();
                        }
                    }

                    if ui
                        .button("Explain")
                        .on_hover_text("Inspect MySQL execution plan for current query")
                        .clicked()
                    {
                        self.execute_current_tab_query(Some("EXPLAIN"));
                    }

                    if ui
                        .button("Format SQL")
                        .on_hover_text("Format SQL clauses cleanly")
                        .clicked()
                        && let Some(tab) = self.tabs.get_mut(self.active_tab_idx) {
                            tab.sql = format_sql_pretty(&tab.sql);
                        }

                    if ui
                        .button("Save Query")
                        .on_hover_text("Directly save query to library and disk (Ctrl+S)")
                        .clicked()
                    {
                        self.save_active_tab_direct();
                    }

                    if ui
                        .button("Save As...")
                        .on_hover_text("Save query with custom title, tags, and description")
                        .clicked()
                    {
                        self.open_save_query_modal();
                    }

                    if ui
                        .button("Open SQL...")
                        .on_hover_text("Open .sql script from disk (Ctrl+O)")
                        .clicked()
                    {
                        self.open_sql_file_dialog();
                    }

                    let ai_btn = egui::Button::new(
                        RichText::new(" AI Auto ")
                            .color(Color32::from_rgb(233, 213, 255))
                            .size(12.0),
                    )
                    .fill(Color32::from_rgb(88, 28, 135))
                    .stroke(Stroke::new(1.0, Color32::from_rgb(168, 85, 247)))
                    .corner_radius(egui::CornerRadius::same(6));

                    if ui
                        .add(ai_btn)
                        .on_hover_text("Trigger inline AI SQL completion at cursor (Ctrl+Space)")
                        .clicked()
                    {
                        self.trigger_inline_ai_completion();
                    }

                    if ui
                        .button("Prompt to SQL")
                        .on_hover_text("Generate SQL from natural language (Ctrl+I)")
                        .clicked()
                        && let Some(tab) = self.tabs.get_mut(self.active_tab_idx)
                    {
                        tab.nl2sql_open = !tab.nl2sql_open;
                        if tab.nl2sql_open {
                            tab.nl2sql_autofocus = true;
                        }
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let copilot_text = if self.show_ai_panel {
                            RichText::new("AI Copilot").color(Color32::WHITE).strong()
                        } else {
                            RichText::new("AI Copilot").weak()
                        };
                        let copilot_btn = egui::Button::new(copilot_text)
                            .fill(if self.show_ai_panel {
                                Color32::from_rgb(126, 34, 206)
                            } else {
                                Color32::from_rgb(24, 32, 47)
                            })
                            .stroke(Stroke::new(
                                1.0,
                                if self.show_ai_panel {
                                    Color32::from_rgb(168, 85, 247)
                                } else {
                                    Color32::from_rgb(45, 55, 75)
                                },
                            ))
                            .corner_radius(egui::CornerRadius::same(6));

                        if ui.add(copilot_btn).clicked() {
                            self.show_ai_panel = !self.show_ai_panel;
                        }

                        if ui
                            .button(format!("AI: {}", self.config.ai.provider.label()))
                            .on_hover_text("Configure AI autocomplete & chat provider")
                            .clicked()
                        {
                            self.show_ai_settings_modal = true;
                        }
                    });
                });
            });
    }

    pub(crate) fn render_bottom_status_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::bottom("status_bar")
            .exact_size(26.0)
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    let dot_color = if self.active_session.is_some() && self.connection_lost.is_none()
                    {
                        Color32::from_rgb(16, 185, 129)
                    } else {
                        Color32::from_rgb(239, 68, 68)
                    };
                    ui.colored_label(dot_color, "●");
                    ui.label(&self.status_banner);

                    if let Some(err) = self.connection_lost.clone() {
                        ui.separator();
                        ui.colored_label(
                            Color32::from_rgb(248, 113, 113),
                            rust_i18n::t!("status_bar.connection_lost"),
                        )
                        .on_hover_text(err);
                        if ui.small_button(rust_i18n::t!("status_bar.reconnect")).clicked() {
                            self.auto_retry_attempted = false;
                            self.reconnect_active_session();
                        }
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new("Ctrl+K: Palette | Tab: Macro/Ghost | Ctrl+Space: AI | Ctrl+Shift+F: Format | Ctrl+Enter: Run")
                                .weak()
                                .small(),
                        );
                        if self.config.ui.vim_mode
                            && let Some(tab) = self.tabs.get(self.active_tab_idx)
                        {
                            let mode = tab.vim_state.mode;
                            let color = match mode {
                                crate::vim::Mode::Normal => Color32::from_rgb(56, 189, 248),
                                crate::vim::Mode::Insert => Color32::from_rgb(34, 197, 94),
                                crate::vim::Mode::Visual | crate::vim::Mode::VisualLine => {
                                    Color32::from_rgb(250, 204, 21)
                                }
                            };
                            ui.separator();
                            ui.label(RichText::new(mode.status_label()).strong().small().color(color));
                        }
                    });
                });
            });
    }
}
