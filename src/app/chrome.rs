use super::*;

impl MysqlRustyApp {
    pub(crate) fn render_open_connections_bar(&mut self, ui: &mut egui::Ui) {
        let pal = self.effective_palette(ui.ctx());
        egui::Panel::top("open_connections_tab_bar")
            .exact_size(32.0)
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.add_space(4.0);
                    let is_home = self.active_workspace_idx.is_none();
                    let home_btn = egui::Button::new(
                        RichText::new(rust_i18n::t!("connections_bar.home"))
                            .size(12.0)
                            .strong()
                            .color(if is_home {
                                pal.accent_text
                            } else {
                                pal.muted_text
                            }),
                    )
                    .fill(if is_home {
                        pal.tab_active_bg
                    } else {
                        pal.tab_inactive_bg
                    })
                    .stroke(Stroke::new(
                        1.0,
                        if is_home {
                            pal.border_active
                        } else {
                            pal.border_subtle
                        },
                    ))
                    .corner_radius(egui::CornerRadius::same(6));

                    if ui
                        .add(home_btn)
                        .on_hover_text(rust_i18n::t!("connections_bar.home_tooltip"))
                        .clicked()
                    {
                        self.switch_to_workspace(None);
                    }

                    let mut switch_target: Option<usize> = None;
                    let mut close_target: Option<usize> = None;

                    egui::ScrollArea::horizontal()
                        .id_salt("open_connections_scroll")
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            ui.horizontal_centered(|ui| {
                                for (idx, ws) in self.open_workspaces.iter().enumerate() {
                                    let is_active = self.active_workspace_idx == Some(idx);
                                    let bg = if is_active {
                                        pal.tab_active_bg
                                    } else {
                                        pal.tab_inactive_bg
                                    };
                                    let border = if is_active {
                                        Stroke::new(1.0, pal.border_active)
                                    } else {
                                        Stroke::new(1.0, pal.border_subtle)
                                    };

                                    let frame_resp = egui::Frame::new()
                                        .fill(bg)
                                        .stroke(border)
                                        .corner_radius(egui::CornerRadius::same(6))
                                        .inner_margin(egui::Margin::symmetric(8, 2))
                                        .show(ui, |ui| {
                                            ui.horizontal(|ui| {
                                                let (r, g, b) = ws.env_color;
                                                ui.label(
                                                    RichText::new("●")
                                                        .small()
                                                        .color(Color32::from_rgb(r, g, b)),
                                                );
                                                let mut label_text = RichText::new(&ws.title)
                                                    .size(12.0)
                                                    .color(if is_active {
                                                        pal.text_primary
                                                    } else {
                                                        pal.text_secondary
                                                    });
                                                if is_active {
                                                    label_text = label_text.strong();
                                                }
                                                let label_resp = ui.add(
                                                    egui::Label::new(label_text)
                                                        .sense(egui::Sense::click()),
                                                );
                                                if label_resp.clicked() {
                                                    switch_target = Some(idx);
                                                }
                                                if label_resp.middle_clicked() {
                                                    close_target = Some(idx);
                                                }
                                                if ui
                                                    .add(
                                                        egui::Button::new(
                                                            RichText::new("×")
                                                                .small()
                                                                .color(pal.muted_text),
                                                        )
                                                        .frame(false),
                                                    )
                                                    .on_hover_text(format!(
                                                        "Close connection `{}`",
                                                        ws.title
                                                    ))
                                                    .clicked()
                                                {
                                                    close_target = Some(idx);
                                                }
                                            });
                                        });
                                    let tab_resp =
                                        frame_resp.response.interact(egui::Sense::click());
                                    if tab_resp.clicked() {
                                        switch_target = Some(idx);
                                    }
                                    if tab_resp.middle_clicked() {
                                        close_target = Some(idx);
                                    }
                                }

                                if ui
                                    .small_button("+")
                                    .on_hover_text(rust_i18n::t!(
                                        "connections_bar.new_conn_tooltip"
                                    ))
                                    .clicked()
                                {
                                    self.switch_to_workspace(None);
                                }
                            });
                        });

                    if let Some(idx) = close_target {
                        self.close_workspace(idx);
                    } else if let Some(idx) = switch_target {
                        self.switch_to_workspace(Some(idx));
                    }
                });
            });
    }

    pub(crate) fn render_top_toolbar(&mut self, ui: &mut egui::Ui) {
        let pal = self.effective_palette(ui.ctx());
        egui::Panel::top("top_toolbar")
            .exact_size(46.0)
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.add_space(4.0);
                    let sidebar_visible = self.show_sidebar && !self.zen_mode;
                    if ui
                        .add(pal.icon_toggle_button("◧", sidebar_visible))
                        .on_hover_text("Toggle left sidebar (Ctrl+B)")
                        .clicked()
                    {
                        self.show_sidebar = !self.show_sidebar;
                        if self.show_sidebar {
                            self.zen_mode = false;
                        }
                    }
                    ui.add_space(2.0);
                    // Brand Logo
                    ui.label(
                        RichText::new("MySQL-Rusty")
                            .size(15.5)
                            .color(pal.accent_text)
                            .strong(),
                    );
                    if let Some(session) = &self.active_session {
                        pal.dialect_badge(ui, session.dialect());
                    }
                    ui.add_space(4.0);
                    ui.separator();
                    ui.add_space(4.0);

                    // Connection selector & status indicator
                    let is_connected = self.active_session.is_some();
                    let in_workspace = self.active_workspace_idx.is_some();
                    let select_conn_default = rust_i18n::t!("toolbar.select_connection");
                    let active_conn_name = self
                        .config
                        .connections
                        .iter()
                        .find(|c| Some(&c.id) == self.active_profile_id.as_ref())
                        .map(|c| c.name.as_str())
                        .unwrap_or(select_conn_default.as_ref());

                    let status_dot = if is_connected {
                        RichText::new("●").color(pal.success)
                    } else {
                        RichText::new("○").color(pal.muted_text)
                    };
                    ui.label(status_dot);

                    egui::ComboBox::from_id_salt("conn_selector")
                        .selected_text(RichText::new(active_conn_name).strong())
                        .width(180.0)
                        .show_ui(ui, |ui| {
                            let profiles = self.config.connections.clone();
                            for prof in profiles {
                                let is_sel = Some(&prof.id) == self.active_profile_id.as_ref();
                                let label = if prof.is_favorite {
                                    format!("★ {}", prof.name)
                                } else {
                                    prof.name.clone()
                                };
                                if ui.selectable_label(is_sel, label).clicked() {
                                    self.connect_to_profile(prof);
                                }
                            }
                        });

                    if is_connected {
                        if ui
                            .button(
                                RichText::new(rust_i18n::t!("toolbar.disconnect"))
                                    .small()
                                    .color(pal.danger),
                            )
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

                    if ui
                        .button(rust_i18n::t!("toolbar.new_btn"))
                        .on_hover_text("Add new connection profile")
                        .clicked()
                    {
                        self.editing_connection = ConnectionProfile::default();
                        self.show_connection_modal = true;
                    }

                    if in_workspace || is_connected {
                        ui.add_space(2.0);
                        ui.separator();
                        ui.add_space(2.0);

                        // Database Selector
                        ui.label(RichText::new(rust_i18n::t!("toolbar.db_label")).color(pal.muted_text));
                        let none_db = rust_i18n::t!("toolbar.none");
                        let current_db = if self.active_database.is_empty() {
                            none_db.as_ref()
                        } else {
                            &self.active_database
                        };
                        egui::ComboBox::from_id_salt("db_selector")
                            .selected_text(current_db)
                            .width(140.0)
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

                        ui.add_space(2.0);
                        ui.separator();
                        ui.add_space(2.0);

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

                        let run_fill = if pal.is_dark {
                            Color32::from_rgb(5, 150, 105) // Emerald 600
                        } else {
                            Color32::from_rgb(22, 163, 74) // Green 600
                        };
                        let run_btn = egui::Button::new(
                            RichText::new(run_label)
                                .color(Color32::WHITE)
                                .strong(),
                        )
                        .fill(run_fill)
                        .corner_radius(egui::CornerRadius::same(6));

                        let run_resp = ui.add(run_btn);
                        self.tour
                            .set_anchor(crate::tour::TourAnchor::RunButton, run_resp.rect);
                        if run_resp.clicked() {
                            self.auto_retry_attempted = false;
                            self.execute_current_tab_query(None);
                        }

                        if self.connection_lost.is_some()
                            && ui
                                .add(pal.danger_button(rust_i18n::t!("toolbar.reconnect").trim()))
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

                        if is_running {
                            if let Some(started) = self
                                .tabs
                                .get(self.active_tab_idx)
                                .and_then(|t| t.run_started_at)
                            {
                                let secs = started.elapsed().as_secs_f32();
                                ui.spinner();
                                pal.badge(
                                    ui,
                                    &format!("{secs:.1}s"),
                                    pal.accent_text,
                                    pal.accent_bg,
                                );
                            }
                            if ui
                                .add(pal.danger_button("■ Stop"))
                                .on_hover_text("Abort running query execution")
                                .clicked()
                            {
                                if let Some(tab) = self.tabs.get_mut(self.active_tab_idx) {
                                    if let Some(handle) = tab.task_abort_handle.take() {
                                        handle.abort();
                                    }
                                    tab.is_running = false;
                                    tab.run_started_at = None;
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
                                RichText::new("↳ Stmt")
                                    .color(pal.success)
                                    .size(12.0)
                                    .strong(),
                            )
                            .fill(pal.success_bg)
                            .stroke(Stroke::new(1.0, pal.success.gamma_multiply(0.5)))
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
                            .on_hover_text("Inspect execution plan for current query")
                            .clicked()
                        {
                            self.execute_current_tab_query(Some("EXPLAIN"));
                        }

                        if ui
                            .button("Format")
                            .on_hover_text("Format SQL clauses cleanly (Ctrl+Shift+F)")
                            .clicked()
                            && let Some(tab) = self.tabs.get_mut(self.active_tab_idx)
                        {
                            tab.sql = format_sql_pretty(&tab.sql);
                        }

                        // Compact File dropdown to prevent toolbar overflow
                        ui.menu_button("File ▾", |ui| {
                            if ui
                                .button("Save Query (Ctrl+S)")
                                .on_hover_text("Directly save query to library and disk")
                                .clicked()
                            {
                                self.save_active_tab_direct();
                                ui.close();
                            }
                            if ui
                                .button("Save As...")
                                .on_hover_text("Save query with custom title, tags, and description")
                                .clicked()
                            {
                                self.open_save_query_modal();
                                ui.close();
                            }
                            if ui
                                .button("Open SQL... (Ctrl+O)")
                                .on_hover_text("Open .sql script from disk")
                                .clicked()
                            {
                                self.open_sql_file_dialog();
                                ui.close();
                            }
                            if is_connected {
                                ui.separator();
                                if ui
                                    .button(rust_i18n::t!("toolbar.data_diff"))
                                    .on_hover_text("Compare table rows across databases/tables")
                                    .clicked()
                                {
                                    self.show_data_diff_modal = true;
                                    if self.data_diff_source_db.is_empty() {
                                        self.data_diff_source_db = self.active_database.clone();
                                        self.data_diff_target_db = self.active_database.clone();
                                    }
                                    ui.close();
                                }
                                if ui
                                    .button(rust_i18n::t!("toolbar.sql_dump"))
                                    .on_hover_text("Export or import .sql database dumps")
                                    .clicked()
                                {
                                    self.show_sql_dump_modal = true;
                                    if self.dump_selected_tables.is_empty()
                                        && let Some(schema) = self.schemas.get(&self.active_database)
                                    {
                                        for t in &schema.tables {
                                            self.dump_selected_tables.insert(t.name.clone());
                                        }
                                    }
                                    ui.close();
                                }
                            }
                        });

                        let ai_btn = egui::Button::new(
                            RichText::new("AI Auto")
                                .color(pal.purple)
                                .size(12.0)
                                .strong(),
                        )
                        .fill(pal.purple_bg)
                        .stroke(Stroke::new(1.0, pal.purple.gamma_multiply(0.5)))
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
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // Quick Zen Mode toggle
                        if ui
                            .add(pal.icon_toggle_button("⛶", self.zen_mode))
                            .on_hover_text(if self.zen_mode {
                                "Exit Zen Mode (restore sidebar & panels)"
                            } else {
                                "Zen Mode — hide sidebar and panels for focused querying"
                            })
                            .clicked()
                        {
                            self.zen_mode = !self.zen_mode;
                        }

                        // Quick Dark/Light theme toggle
                        let is_dark = pal.is_dark;
                        let theme_icon = if is_dark { "☀" } else { "☾" };
                        let theme_tip = if is_dark {
                            format!(
                                "Switch to Light theme (current: {})",
                                self.config.ui.dark_preset.label()
                            )
                        } else {
                            format!(
                                "Switch to Dark theme ({})",
                                self.config.ui.dark_preset.label()
                            )
                        };
                        if ui
                            .add(pal.icon_toggle_button(theme_icon, false))
                            .on_hover_text(theme_tip)
                            .clicked()
                        {
                            self.config.ui.theme = if is_dark {
                                crate::config::ThemePreference::Light
                            } else {
                                crate::config::ThemePreference::Dark
                            };
                            let _ = self.config.save();
                        }

                        let copilot_text = if self.show_ai_panel && !self.zen_mode {
                            RichText::new("AI Copilot").color(Color32::WHITE).strong()
                        } else {
                            RichText::new("AI Copilot").color(pal.text_secondary)
                        };
                        let copilot_btn = egui::Button::new(copilot_text)
                            .fill(if self.show_ai_panel && !self.zen_mode {
                                Color32::from_rgb(126, 34, 206)
                            } else {
                                pal.card_bg
                            })
                            .stroke(Stroke::new(
                                1.0,
                                if self.show_ai_panel && !self.zen_mode {
                                    pal.purple
                                } else {
                                    pal.card_border
                                },
                            ))
                            .corner_radius(egui::CornerRadius::same(6));

                        if ui.add(copilot_btn).clicked() {
                            self.show_ai_panel = !self.show_ai_panel;
                            if self.show_ai_panel {
                                self.zen_mode = false;
                            }
                        }

                        if ui
                            .button(format!("⚙ {}", self.config.ai.provider.label()))
                            .on_hover_text("Appearance, theme presets & AI provider settings")
                            .clicked()
                        {
                            self.show_ai_settings_modal = true;
                        }
                    });
                });
            });
    }

    pub(crate) fn render_bottom_status_bar(&mut self, ui: &mut egui::Ui) {
        let pal = self.effective_palette(ui.ctx());
        egui::Panel::bottom("status_bar")
            .exact_size(28.0)
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    let dot_color = if self.connection_lost.is_some() {
                        pal.danger
                    } else if self.active_session.is_some() {
                        pal.success
                    } else {
                        pal.muted_text
                    };
                    ui.colored_label(dot_color, "●");

                    let status_max_w = (ui.available_width() - 560.0).max(110.0);
                    ui.add_sized(
                        [status_max_w, 18.0],
                        egui::Label::new(
                            RichText::new(&self.status_banner)
                                .size(11.5)
                                .color(pal.text_secondary),
                        )
                        .truncate(),
                    )
                    .on_hover_text(&self.status_banner);

                    if let Some(err) = self.connection_lost.clone() {
                        ui.separator();
                        ui.colored_label(pal.danger, rust_i18n::t!("status_bar.connection_lost"))
                            .on_hover_text(err);
                        if ui
                            .small_button(rust_i18n::t!("status_bar.reconnect"))
                            .clicked()
                        {
                            self.auto_retry_attempted = false;
                            self.reconnect_active_session();
                        }
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // Clickable Density switcher chip
                        let density_label = self.config.ui.density.label();
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new(density_label)
                                        .size(10.5)
                                        .color(pal.muted_text),
                                )
                                .frame(false),
                            )
                            .on_hover_text(
                                "Click to cycle UI density (Compact / Comfortable / Spacious)",
                            )
                            .clicked()
                        {
                            self.config.ui.density = self.config.ui.density.next();
                            let _ = self.config.save();
                        }

                        ui.separator();

                        // Clickable Theme Preset switcher chip
                        let preset_label = if pal.is_dark {
                            self.config.ui.dark_preset.label()
                        } else {
                            "Light"
                        };
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new(preset_label)
                                        .size(10.5)
                                        .color(pal.accent_text),
                                )
                                .frame(false),
                            )
                            .on_hover_text("Click to cycle theme preset")
                            .clicked()
                        {
                            if pal.is_dark {
                                let all = crate::config::DarkThemePreset::all();
                                let idx = all
                                    .iter()
                                    .position(|&p| p == self.config.ui.dark_preset)
                                    .unwrap_or(0);
                                self.config.ui.dark_preset = all[(idx + 1) % all.len()];
                            } else {
                                self.config.ui.theme = crate::config::ThemePreference::Dark;
                            }
                            let _ = self.config.save();
                        }

                        if self.active_session.is_some()
                            && let Some(tab) = self.tabs.get(self.active_tab_idx)
                        {
                            let (prefix, _) = split_sql_at_cursor(&tab.sql, tab.cursor_index);
                            let ln = prefix.chars().filter(|&c| c == '\n').count() + 1;
                            let col = prefix
                                .rsplit('\n')
                                .next()
                                .map(|s| s.chars().count() + 1)
                                .unwrap_or(1);
                            ui.separator();
                            ui.label(
                                RichText::new(format!("Ln {ln}, Col {col}"))
                                    .size(11.0)
                                    .monospace()
                                    .color(pal.muted_text),
                            );

                            if let Some(res) = &tab.last_result {
                                ui.separator();
                                ui.label(
                                    RichText::new(format!(
                                        "{} rows • {}ms",
                                        res.rows.len(),
                                        res.duration_ms
                                    ))
                                    .size(11.0)
                                    .color(pal.text_secondary),
                                );
                            }
                        }

                        if let Some(prof) = self
                            .active_profile_id
                            .as_ref()
                            .and_then(|id| self.config.connections.iter().find(|c| &c.id == id))
                        {
                            let (er, eg, eb) = prof.environment.color_rgb();
                            let (fg, bg) = pal.env_badge_colors(er, eg, eb);
                            ui.separator();
                            pal.micro_badge(ui, &prof.environment.label().to_uppercase(), fg, bg);
                        }

                        if self.config.ui.vim_mode
                            && let Some(tab) = self.tabs.get(self.active_tab_idx)
                        {
                            let mode = tab.vim_state.mode;
                            let (fg, bg) = match mode {
                                crate::vim::Mode::Normal => (pal.accent_text, pal.accent_bg),
                                crate::vim::Mode::Insert => (pal.success, pal.success_bg),
                                crate::vim::Mode::Visual | crate::vim::Mode::VisualLine => {
                                    (pal.warning, pal.warning_bg)
                                }
                            };
                            ui.separator();
                            pal.badge(ui, mode.status_label(), fg, bg);
                        }
                    });
                });
            });
    }
}
