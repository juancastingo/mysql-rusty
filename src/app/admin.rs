use super::*;

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum ServerAdminSubtab {
    OverviewAndMetrics,
    ServerVariables,
    SlowQueries,
    UsersAndPrivileges,
}

impl MysqlRustyApp {
    pub fn fetch_server_variables(&mut self) {
        let Some(session) = self.active_session.clone() else {
            return;
        };
        self.is_loading_variables = true;
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let res = session
                .list_server_variables()
                .await
                .map_err(|e| format!("{e:#}"));
            let _ = tx.send(AsyncEvent::ServerVariablesLoaded(res));
        });
    }

    pub fn refresh_server_admin(&mut self) {
        let Some(session) = self.active_session.clone() else {
            self.status_banner = "Not connected to any database session.".to_string();
            return;
        };
        self.is_server_admin_loading = true;
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let status_res = session
                .server_status_summary()
                .await
                .map_err(|e| format!("{e:#}"));
            let proc_res = session.list_processes().await.map_err(|e| format!("{e:#}"));
            let _ = tx.send(AsyncEvent::ServerAdminLoaded {
                status: status_res,
                processes: proc_res,
            });
        });
    }

    pub fn kill_server_query(&mut self, conn_id: u64) {
        let Some(session) = self.active_session.clone() else {
            return;
        };
        let tx = self.tx.clone();
        self.status_banner = format!("Killing query for connection #{conn_id}...");
        self.rt.spawn(async move {
            let res = session
                .kill_query(conn_id)
                .await
                .map(|_| conn_id)
                .map_err(|e| format!("{e:#}"));
            let _ = tx.send(AsyncEvent::QueryKilled(res));
        });
    }

    pub(crate) fn render_server_admin(&mut self, ui: &mut Ui) {
        let Some(_session) = self.active_session.as_ref() else {
            ui.label(
                RichText::new(
                    "Connect to a MySQL server or Demo Sandbox to inspect server administration.",
                )
                .weak(),
            );
            return;
        };

        let mut kill_query_id = None;

        ui.horizontal(|ui| {
            ui.heading(
                RichText::new("Server Administration & Process Monitor")
                    .color(Color32::from_rgb(56, 189, 248))
                    .strong(),
            );
            ui.add_space(16.0);
            if ui.button("Refresh").clicked() {
                self.refresh_server_admin();
            }
            if self.is_server_admin_loading {
                ui.spinner();
                ui.label("Refreshing status...");
            }
        });
        ui.separator();

        ui.horizontal(|ui| {
            ui.selectable_value(
                &mut self.server_admin_subtab,
                ServerAdminSubtab::OverviewAndMetrics,
                "Metrics & Processes",
            );
            ui.selectable_value(
                &mut self.server_admin_subtab,
                ServerAdminSubtab::ServerVariables,
                "Server Variables",
            );
            ui.selectable_value(
                &mut self.server_admin_subtab,
                ServerAdminSubtab::SlowQueries,
                "Slow Queries",
            );
            ui.selectable_value(
                &mut self.server_admin_subtab,
                ServerAdminSubtab::UsersAndPrivileges,
                "Users & Privileges",
            );
        });
        ui.separator();

        if self.server_admin_subtab == ServerAdminSubtab::UsersAndPrivileges {
            self.render_users_and_privileges_subtab(ui);
            return;
        }

        if self.server_admin_subtab == ServerAdminSubtab::SlowQueries {
            self.render_slow_queries_subtab(ui);
            return;
        }

        if self.server_admin_subtab == ServerAdminSubtab::ServerVariables {
            if self.server_variables.is_empty() && !self.is_loading_variables {
                self.fetch_server_variables();
            }
            ui.horizontal(|ui| {
                ui.label("Filter variables:");
                ui.add(
                    egui::TextEdit::singleline(&mut self.server_variables_filter)
                        .desired_width(220.0)
                        .hint_text("Search name or value..."),
                );
                if ui.button("Reload").clicked() {
                    self.fetch_server_variables();
                }
                if self.is_loading_variables {
                    ui.spinner();
                    ui.label("Loading variables from server...");
                }
            });
            ui.separator();

            let filter = self.server_variables_filter.to_lowercase();
            let filtered_vars: Vec<&(String, String)> = self
                .server_variables
                .iter()
                .filter(|(k, v)| {
                    filter.is_empty()
                        || k.to_lowercase().contains(&filter)
                        || v.to_lowercase().contains(&filter)
                })
                .collect();

            ui.label(
                RichText::new(format!(
                    "Showing {} of {} server variables",
                    filtered_vars.len(),
                    self.server_variables.len()
                ))
                .small()
                .weak(),
            );

            let mut copy_val: Option<String> = None;
            TableBuilder::new(ui)
                .striped(true)
                .resizable(true)
                .column(Column::initial(240.0).at_least(140.0))
                .column(Column::remainder().at_least(180.0))
                .column(Column::exact(65.0))
                .header(22.0, |mut h| {
                    h.col(|ui| {
                        ui.strong("Variable Name");
                    });
                    h.col(|ui| {
                        ui.strong("Value");
                    });
                    h.col(|ui| {
                        ui.strong("Action");
                    });
                })
                .body(|body| {
                    body.rows(20.0, filtered_vars.len(), |mut row| {
                        let (k, v) = filtered_vars[row.index()];
                        row.col(|ui| {
                            ui.label(
                                RichText::new(k)
                                    .monospace()
                                    .color(Color32::from_rgb(56, 189, 248)),
                            );
                        });
                        row.col(|ui| {
                            ui.label(RichText::new(v).monospace());
                        });
                        row.col(|ui| {
                            if ui
                                .small_button("📋")
                                .on_hover_text("Copy variable value")
                                .clicked()
                            {
                                copy_val = Some(v.clone());
                            }
                        });
                    });
                });

            if let Some(val) = copy_val {
                ui.ctx().copy_text(val);
                self.status_banner = "Copied variable value to clipboard.".to_string();
            }
            return;
        }

        // Server Metrics Cards
        let uptime_str = self.server_status_metrics.get("Uptime");
        let uptime_disp = format_uptime_secs(uptime_str);
        let threads_disp = self
            .server_status_metrics
            .get("Threads_connected")
            .cloned()
            .unwrap_or_else(|| "—".to_string());
        let queries_disp = self
            .server_status_metrics
            .get("Questions")
            .cloned()
            .unwrap_or_else(|| "—".to_string());
        let slow_disp = self
            .server_status_metrics
            .get("Slow_queries")
            .cloned()
            .unwrap_or_else(|| "0".to_string());
        let buffer_disp = format_buffer_pool_bytes(
            self.server_status_metrics
                .get("Innodb_buffer_pool_bytes_data"),
        );

        ui.horizontal_wrapped(|ui| {
            let cards = [
                ("Uptime", &uptime_disp, Color32::from_rgb(56, 189, 248)),
                (
                    "Connections",
                    &threads_disp,
                    Color32::from_rgb(52, 211, 153),
                ),
                (
                    "Total Queries",
                    &queries_disp,
                    Color32::from_rgb(192, 132, 252),
                ),
                ("Slow Queries", &slow_disp, Color32::from_rgb(251, 146, 60)),
                (
                    "InnoDB Buffer",
                    &buffer_disp,
                    Color32::from_rgb(45, 212, 191),
                ),
            ];

            for (title, value, color) in cards {
                egui::Frame::group(ui.style())
                    .fill(Color32::from_rgb(15, 23, 42))
                    .stroke(Stroke::new(1.0, color))
                    .show(ui, |ui| {
                        ui.set_width(140.0);
                        ui.label(RichText::new(title).small().weak());
                        ui.label(RichText::new(value).strong().color(color).size(15.0));
                    });
            }
        });

        if !self.qps_history.is_empty() || !self.threads_history.is_empty() {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new("Live QPS (Queries / Sec)")
                            .strong()
                            .small()
                            .color(Color32::from_rgb(56, 189, 248)),
                    );
                    Plot::new("qps_history_plot")
                        .height(85.0)
                        .width(ui.available_width() * 0.49)
                        .show(ui, |plot_ui| {
                            plot_ui.line(
                                Line::new("QPS", PlotPoints::new(self.qps_history.clone()))
                                    .color(Color32::from_rgb(56, 189, 248))
                                    .width(2.0),
                            );
                        });
                });
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new("Active Connections (Threads)")
                            .strong()
                            .small()
                            .color(Color32::from_rgb(52, 211, 153)),
                    );
                    Plot::new("threads_history_plot")
                        .height(85.0)
                        .width(ui.available_width())
                        .show(ui, |plot_ui| {
                            plot_ui.line(
                                Line::new("Threads", PlotPoints::new(self.threads_history.clone()))
                                    .color(Color32::from_rgb(52, 211, 153))
                                    .width(2.0),
                            );
                        });
                });
            });
        }

        ui.add_space(8.0);
        ui.label(RichText::new("Live Process List (SHOW FULL PROCESSLIST):").strong());
        ui.separator();

        if let Some(result) = &self.server_processes {
            let procs = result.clone();
            let num_rows = procs.rows.len();

            let text_height = egui::TextStyle::Body.resolve(ui.style()).size.max(14.0) + 8.0;

            TableBuilder::new(ui)
                .striped(true)
                .resizable(true)
                .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                .column(Column::initial(60.0).at_least(40.0)) // Id
                .column(Column::initial(90.0).at_least(60.0)) // User
                .column(Column::initial(130.0).at_least(80.0)) // Host
                .column(Column::initial(110.0).at_least(60.0)) // DB
                .column(Column::initial(80.0).at_least(60.0)) // Command
                .column(Column::initial(60.0).at_least(40.0)) // Time
                .column(Column::initial(90.0).at_least(50.0)) // State
                .column(Column::remainder().at_least(180.0)) // Info
                .column(Column::exact(80.0)) // Action
                .header(24.0, |mut header| {
                    header.col(|ui| {
                        ui.strong("Id");
                    });
                    header.col(|ui| {
                        ui.strong("User");
                    });
                    header.col(|ui| {
                        ui.strong("Host");
                    });
                    header.col(|ui| {
                        ui.strong("DB");
                    });
                    header.col(|ui| {
                        ui.strong("Command");
                    });
                    header.col(|ui| {
                        ui.strong("Time (s)");
                    });
                    header.col(|ui| {
                        ui.strong("State");
                    });
                    header.col(|ui| {
                        ui.strong("Info");
                    });
                    header.col(|ui| {
                        ui.strong("Action");
                    });
                })
                .body(|body| {
                    body.rows(text_height, num_rows, |mut row| {
                        let row_idx = row.index();
                        if let Some(r) = procs.rows.get(row_idx) {
                            let id_val = r.first().map(|c| c.display_text()).unwrap_or_default();
                            let user_val = r.get(1).map(|c| c.display_text()).unwrap_or_default();
                            let host_val = r.get(2).map(|c| c.display_text()).unwrap_or_default();
                            let db_val = r.get(3).map(|c| c.display_text()).unwrap_or_default();
                            let cmd_val = r.get(4).map(|c| c.display_text()).unwrap_or_default();
                            let time_val = r.get(5).map(|c| c.display_text()).unwrap_or_default();
                            let state_val = r.get(6).map(|c| c.display_text()).unwrap_or_default();
                            let info_val = r.get(7).map(|c| c.display_text()).unwrap_or_default();

                            row.col(|ui| {
                                ui.monospace(&id_val);
                            });
                            row.col(|ui| {
                                ui.label(&user_val);
                            });
                            row.col(|ui| {
                                ui.label(&host_val);
                            });
                            row.col(|ui| {
                                ui.label(&db_val);
                            });
                            row.col(|ui| {
                                ui.label(&cmd_val);
                            });
                            row.col(|ui| {
                                ui.monospace(&time_val);
                            });
                            row.col(|ui| {
                                ui.label(&state_val);
                            });
                            row.col(|ui| {
                                ui.add(
                                    egui::Label::new(RichText::new(&info_val).monospace().small())
                                        .truncate(),
                                );
                            });
                            row.col(|ui| {
                                if let Ok(conn_id) = id_val.parse::<u64>()
                                    && ui
                                        .small_button(
                                            RichText::new("Kill")
                                                .color(Color32::from_rgb(239, 68, 68)),
                                        )
                                        .clicked()
                                {
                                    kill_query_id = Some(conn_id);
                                }
                            });
                        }
                    });
                });
        } else {
            ui.label(
                RichText::new(
                    "No process list loaded. Click 'Refresh' to inspect active connections.",
                )
                .weak(),
            );
        }

        if let Some(id) = kill_query_id {
            self.kill_server_query(id);
        }
    }

    pub fn fetch_slow_queries(&mut self) {
        let Some(session) = self.active_session.clone() else {
            return;
        };
        self.slow_queries_loading = true;
        let limit = self.slow_queries_limit;
        let schema_filter = if self.slow_queries_only_current_db && !self.active_database.is_empty()
        {
            Some(self.active_database.clone())
        } else {
            None
        };
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let res = async {
                let enabled = session.performance_schema_enabled().await?;
                let qr = if enabled {
                    session
                        .top_statement_digests(limit, schema_filter.as_deref())
                        .await?
                } else {
                    QueryResult {
                        sql: String::new(),
                        database: String::new(),
                        columns: Vec::new(),
                        rows: Vec::new(),
                        affected_rows: 0,
                        last_insert_id: None,
                        duration_ms: 0,
                        status_message: String::new(),
                        editable_table: None,
                        primary_key_column: None,
                        truncated: false,
                    }
                };
                Ok::<_, anyhow::Error>((enabled, qr))
            }
            .await
            .map_err(|e| format!("{e:#}"));
            let _ = tx.send(AsyncEvent::SlowQueriesLoaded(res));
        });
    }

    pub fn reset_slow_queries(&mut self) {
        let Some(session) = self.active_session.clone() else {
            return;
        };
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let res = session
                .reset_statement_digests()
                .await
                .map_err(|e| format!("{e:#}"));
            let _ = tx.send(AsyncEvent::SlowQueriesReset(res));
        });
    }

    fn render_slow_queries_subtab(&mut self, ui: &mut Ui) {
        if self.slow_queries_enabled.is_none() && !self.slow_queries_loading {
            self.fetch_slow_queries();
        }

        let is_read_only = self
            .active_profile_id
            .as_ref()
            .and_then(|id| self.config.connections.iter().find(|p| &p.id == id))
            .is_some_and(|p| p.read_only);

        let mut trigger_refresh = false;
        let mut trigger_reset = false;
        ui.horizontal(|ui| {
            if ui
                .checkbox(
                    &mut self.slow_queries_only_current_db,
                    "Only current database",
                )
                .changed()
            {
                trigger_refresh = true;
            }
            egui::ComboBox::from_id_salt("slow_queries_limit")
                .selected_text(format!("Top {}", self.slow_queries_limit))
                .width(90.0)
                .show_ui(ui, |ui| {
                    for lim in [25usize, 50, 100] {
                        if ui
                            .selectable_value(
                                &mut self.slow_queries_limit,
                                lim,
                                format!("Top {lim}"),
                            )
                            .changed()
                        {
                            trigger_refresh = true;
                        }
                    }
                });
            if ui.button("Refresh").clicked() {
                trigger_refresh = true;
            }
            if self.slow_queries_confirm_reset {
                if ui
                    .button(RichText::new("Confirm Reset").color(Color32::from_rgb(248, 113, 113)))
                    .clicked()
                {
                    self.slow_queries_confirm_reset = false;
                    trigger_reset = true;
                }
                if ui.button("Cancel").clicked() {
                    self.slow_queries_confirm_reset = false;
                }
            } else if ui
                .add_enabled(!is_read_only, egui::Button::new("Reset stats"))
                .clicked()
            {
                self.slow_queries_confirm_reset = true;
            }
            if self.slow_queries_loading {
                ui.spinner();
            }
        });
        ui.separator();

        if trigger_refresh {
            self.fetch_slow_queries();
        }
        if trigger_reset {
            self.reset_slow_queries();
        }

        if let Some(err) = &self.slow_queries_error {
            ui.colored_label(Color32::from_rgb(248, 113, 113), err);
            return;
        }

        if self.slow_queries_enabled == Some(false) {
            let snippet = "[mysqld]\nperformance_schema=ON";
            egui::Frame::new()
                .fill(Color32::from_rgb(30, 41, 59))
                .inner_margin(12.0)
                .corner_radius(egui::CornerRadius::same(6))
                .show(ui, |ui| {
                    ui.label(
                        RichText::new("performance_schema is disabled on this server.")
                            .color(Color32::from_rgb(251, 191, 36))
                            .strong(),
                    );
                    ui.label("Add the following to my.cnf and restart MySQL to enable statement digests:");
                    ui.code(snippet);
                    if ui.small_button("Copy snippet").clicked() {
                        ui.ctx().copy_text(snippet.to_string());
                        self.status_banner = "Copied my.cnf snippet.".to_string();
                    }
                });
            return;
        }

        let Some(res) = self.slow_queries_result.clone() else {
            ui.label(RichText::new("No statement digests loaded yet.").weak());
            return;
        };

        let mut open_sql: Option<String> = None;
        let mut explain_sql: Option<String> = None;

        egui::ScrollArea::both().show(ui, |ui| {
            egui::Grid::new("slow_queries_grid")
                .striped(true)
                .spacing([12.0, 6.0])
                .show(ui, |ui| {
                    for h in [
                        "DB",
                        "Query",
                        "Calls",
                        "Total (s)",
                        "Avg (ms)",
                        "Max (ms)",
                        "Examined/Sent",
                        "No Index",
                        "Actions",
                    ] {
                        ui.label(RichText::new(h).strong());
                    }
                    ui.end_row();

                    for row in &res.rows {
                        let cell_str = |idx: usize| -> String {
                            row.get(idx).map(|c| c.display_text()).unwrap_or_default()
                        };
                        let cell_num =
                            |idx: usize| -> f64 { cell_str(idx).parse::<f64>().unwrap_or(0.0) };
                        let db = cell_str(0);
                        let digest = cell_str(1);
                        let calls = cell_str(2);
                        let total_s = cell_str(3);
                        let avg_ms = cell_str(4);
                        let max_ms = cell_str(5);
                        let examined = cell_num(6);
                        let sent = cell_num(7);
                        let no_idx = cell_num(8) as u64 + cell_num(9) as u64;
                        let sample = row
                            .get(12)
                            .map(|c| c.display_text())
                            .filter(|s| !s.is_empty() && s != "NULL");

                        let ratio = if sent > 0.0 {
                            examined / sent
                        } else {
                            examined
                        };
                        let preview: String = digest.chars().take(72).collect();
                        let target_sql = sample.unwrap_or_else(|| {
                            crate::syntax::digest_placeholders_to_params(&digest)
                        });

                        ui.label(if db.is_empty() || db == "NULL" {
                            "-"
                        } else {
                            &db
                        });
                        ui.label(RichText::new(&preview).monospace().small())
                            .on_hover_text(&digest);
                        ui.label(calls);
                        ui.label(total_s);
                        ui.label(avg_ms);
                        ui.label(max_ms);
                        let ratio_text = format!("{ratio:.1}x");
                        if ratio > 100.0 {
                            ui.colored_label(Color32::from_rgb(248, 113, 113), ratio_text);
                        } else {
                            ui.label(ratio_text);
                        }
                        if no_idx > 0 {
                            ui.colored_label(Color32::from_rgb(248, 113, 113), no_idx.to_string());
                        } else {
                            ui.label("0");
                        }
                        ui.horizontal(|ui| {
                            if ui.small_button("Open in tab").clicked() {
                                open_sql = Some(target_sql.clone());
                            }
                            if ui.small_button("Explain").clicked() {
                                explain_sql = Some(target_sql);
                            }
                        });
                        ui.end_row();
                    }
                });
        });

        if let Some(sql) = open_sql {
            let n = self.tabs.len() + 1;
            self.tabs.push(QueryTab::new(format!("Slow {n}"), sql));
            self.active_tab_idx = self.tabs.len() - 1;
        } else if let Some(sql) = explain_sql {
            let n = self.tabs.len() + 1;
            self.tabs.push(QueryTab::new(format!("Explain {n}"), sql));
            self.active_tab_idx = self.tabs.len() - 1;
            self.execute_current_tab_query(Some("EXPLAIN"));
        }
    }

    pub(crate) fn fetch_users(&mut self) {
        let Some(session) = self.active_session.clone() else {
            return;
        };
        self.users_loading = true;
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let res = session.list_users().await.map_err(|e| format!("{e:#}"));
            let _ = tx.send(AsyncEvent::UsersLoaded(res));
        });
    }

    pub(crate) fn select_user(&mut self, idx: usize) {
        let Some(u) = self.users_list.get(idx).cloned() else {
            return;
        };
        self.selected_user_idx = Some(idx);
        self.user_new_mode = false;
        self.user_form_name = u.user.clone();
        self.user_form_host = u.host.clone();
        self.user_form_plugin = u.plugin.clone();
        self.user_form_password.clear();
        self.user_form_expire = u.password_expired;

        let Some(session) = self.active_session.clone() else {
            return;
        };
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let grants = session
                .fetch_user_grants(&u.user, &u.host)
                .await
                .map_err(|e| format!("{e:#}"));
            let _ = tx.send(AsyncEvent::UserGrantsLoaded {
                user: u.user,
                host: u.host,
                grants,
            });
        });
    }

    fn render_users_and_privileges_subtab(&mut self, ui: &mut Ui) {
        if self.users_list.is_empty() && !self.users_loading && self.users_error.is_none() {
            self.fetch_users();
        }

        let is_read_only = self
            .active_profile_id
            .as_ref()
            .and_then(|id| self.config.connections.iter().find(|p| &p.id == id))
            .is_some_and(|p| p.read_only);

        if is_read_only {
            ui.colored_label(
                Color32::from_rgb(251, 191, 36),
                "Read-Only connection: user creation, deletion, and privilege edits are disabled.",
            );
        }
        if let Some(err) = &self.users_error {
            ui.colored_label(Color32::from_rgb(248, 113, 113), err);
        }

        let mut select_idx: Option<usize> = None;
        let mut do_refresh = false;
        let mut do_delete: Option<(String, String)> = None;
        let mut do_create = false;
        let mut do_update_pw = false;
        let mut do_apply_privs = false;

        ui.columns(2, |cols| {
            // Left pane: Master list
            cols[0].horizontal(|ui| {
                ui.label(RichText::new("Database Accounts").strong());
                if ui.small_button("Refresh").clicked() {
                    do_refresh = true;
                }
                if ui
                    .add_enabled(!is_read_only, egui::Button::new("+ Add Account").small())
                    .clicked()
                {
                    self.user_new_mode = true;
                    self.selected_user_idx = None;
                    self.user_form_name.clear();
                    self.user_form_host = "%".to_string();
                    self.user_form_plugin = "caching_sha2_password".to_string();
                    self.user_form_password.clear();
                    self.user_form_expire = false;
                    self.user_original_privs.clear();
                    self.user_edited_privs.clear();
                }
                if let Some(idx) = self.selected_user_idx
                    && let Some(u) = self.users_list.get(idx)
                    && ui
                        .add_enabled(!is_read_only, egui::Button::new("Delete").small())
                        .clicked()
                {
                    do_delete = Some((u.user.clone(), u.host.clone()));
                }
            });
            cols[0].separator();
            egui::ScrollArea::vertical()
                .id_salt("users_master_list")
                .max_height(380.0)
                .show(&mut cols[0], |ui| {
                    for (idx, u) in self.users_list.iter().enumerate() {
                        let selected = !self.user_new_mode && self.selected_user_idx == Some(idx);
                        let label = if u.account_locked {
                            format!("{}@{} [LOCKED]", u.user, u.host)
                        } else {
                            format!("{}@{}", u.user, u.host)
                        };
                        if ui.selectable_label(selected, label).clicked() {
                            select_idx = Some(idx);
                        }
                    }
                });

            // Right pane: Details & Privileges
            cols[1].heading(if self.user_new_mode {
                "New User Account".to_string()
            } else if !self.user_form_name.is_empty() {
                format!("Account: {}@{}", self.user_form_name, self.user_form_host)
            } else {
                "Select a user account".to_string()
            });
            cols[1].separator();

            egui::Grid::new("user_login_grid")
                .num_columns(2)
                .spacing([10.0, 6.0])
                .show(&mut cols[1], |ui| {
                    ui.label("Username:");
                    ui.add_enabled(
                        self.user_new_mode && !is_read_only,
                        egui::TextEdit::singleline(&mut self.user_form_name).desired_width(200.0),
                    );
                    ui.end_row();

                    ui.label("Host:");
                    ui.add_enabled(
                        self.user_new_mode && !is_read_only,
                        egui::TextEdit::singleline(&mut self.user_form_host).desired_width(200.0),
                    );
                    ui.end_row();

                    ui.label("Auth Plugin:");
                    egui::ComboBox::from_id_salt("user_plugin_combo")
                        .selected_text(&self.user_form_plugin)
                        .show_ui(ui, |ui| {
                            for p in ["caching_sha2_password", "mysql_native_password"] {
                                ui.selectable_value(&mut self.user_form_plugin, p.to_string(), p);
                            }
                        });
                    ui.end_row();

                    ui.label("Password:");
                    ui.add(
                        egui::TextEdit::singleline(&mut self.user_form_password)
                            .password(true)
                            .desired_width(200.0),
                    );
                    ui.end_row();

                    ui.label("Password Status:");
                    ui.checkbox(&mut self.user_form_expire, "Expire password");
                    ui.end_row();
                });

            cols[1].horizontal(|ui| {
                if self.user_new_mode {
                    if ui
                        .add_enabled(
                            !is_read_only && !self.user_form_name.trim().is_empty(),
                            egui::Button::new("Create User"),
                        )
                        .clicked()
                    {
                        do_create = true;
                    }
                } else if self.selected_user_idx.is_some()
                    && ui
                        .add_enabled(
                            !is_read_only && !self.user_form_password.is_empty(),
                            egui::Button::new("Update Password"),
                        )
                        .clicked()
                {
                    do_update_pw = true;
                }
            });

            if !self.user_new_mode && self.selected_user_idx.is_some() {
                cols[1].separator();
                cols[1].label(RichText::new("Administrative & Schema Privileges").strong());
                let groups: &[(&str, &[&str])] = &[
                    ("Data", &["SELECT", "INSERT", "UPDATE", "DELETE"]),
                    (
                        "Structure",
                        &[
                            "CREATE",
                            "ALTER",
                            "DROP",
                            "INDEX",
                            "CREATE VIEW",
                            "SHOW VIEW",
                        ],
                    ),
                    (
                        "Administration",
                        &[
                            "SUPER",
                            "PROCESS",
                            "RELOAD",
                            "REPLICATION CLIENT",
                            "SHOW DATABASES",
                        ],
                    ),
                ];
                for (group_name, privs) in groups {
                    cols[1].label(RichText::new(*group_name).small().weak());
                    cols[1].horizontal_wrapped(|ui| {
                        for p in *privs {
                            let mut checked = self.user_edited_privs.contains(*p);
                            if ui
                                .add_enabled(!is_read_only, egui::Checkbox::new(&mut checked, *p))
                                .changed()
                            {
                                if checked {
                                    self.user_edited_privs.insert((*p).to_string());
                                } else {
                                    self.user_edited_privs.remove(*p);
                                }
                            }
                        }
                    });
                }

                let mut grants: Vec<String> = self
                    .user_edited_privs
                    .difference(&self.user_original_privs)
                    .cloned()
                    .collect();
                let mut revokes: Vec<String> = self
                    .user_original_privs
                    .difference(&self.user_edited_privs)
                    .cloned()
                    .collect();
                grants.sort();
                revokes.sort();

                let preview = crate::db::build_grant_revoke_sql(
                    &self.user_form_name,
                    &self.user_form_host,
                    "*.*",
                    &grants,
                    &revokes,
                );
                if !preview.is_empty() {
                    cols[1].add_space(4.0);
                    cols[1].label(RichText::new(preview.join(";\n")).monospace().small());
                }

                cols[1].horizontal(|ui| {
                    if ui
                        .add_enabled(
                            !is_read_only && (!grants.is_empty() || !revokes.is_empty()),
                            egui::Button::new("Apply Changes"),
                        )
                        .clicked()
                    {
                        do_apply_privs = true;
                    }
                    if ui.button("Revert").clicked() {
                        self.user_edited_privs = self.user_original_privs.clone();
                    }
                });
            }
        });

        if do_refresh {
            self.fetch_users();
        }
        if let Some(idx) = select_idx {
            self.select_user(idx);
        }
        if let Some((user, host)) = do_delete
            && let Some(session) = self.active_session.clone()
        {
            let tx = self.tx.clone();
            self.rt.spawn(async move {
                let res = session
                    .drop_user(&user, &host)
                    .await
                    .map(|()| format!("Dropped user `{user}`@`{host}`."))
                    .map_err(|e| format!("{e:#}"));
                let _ = tx.send(AsyncEvent::UserActionFinished(res));
            });
        }
        if do_create && let Some(session) = self.active_session.clone() {
            let user = self.user_form_name.trim().to_string();
            let host = self.user_form_host.trim().to_string();
            let pw = self.user_form_password.clone();
            let plugin = self.user_form_plugin.clone();
            let tx = self.tx.clone();
            self.rt.spawn(async move {
                let res = session
                    .create_user(&user, &host, &pw, &plugin)
                    .await
                    .map(|()| format!("Created user `{user}`@`{host}`."))
                    .map_err(|e| format!("{e:#}"));
                let _ = tx.send(AsyncEvent::UserActionFinished(res));
            });
        }
        if do_update_pw && let Some(session) = self.active_session.clone() {
            let user = self.user_form_name.clone();
            let host = self.user_form_host.clone();
            let pw = self.user_form_password.clone();
            let tx = self.tx.clone();
            self.rt.spawn(async move {
                let res = session
                    .update_user_password(&user, &host, &pw)
                    .await
                    .map(|()| format!("Updated password for `{user}`@`{host}`."))
                    .map_err(|e| format!("{e:#}"));
                let _ = tx.send(AsyncEvent::UserActionFinished(res));
            });
        }
        if do_apply_privs && let Some(session) = self.active_session.clone() {
            let mut grants: Vec<String> = self
                .user_edited_privs
                .difference(&self.user_original_privs)
                .cloned()
                .collect();
            let mut revokes: Vec<String> = self
                .user_original_privs
                .difference(&self.user_edited_privs)
                .cloned()
                .collect();
            grants.sort();
            revokes.sort();
            let user = self.user_form_name.clone();
            let host = self.user_form_host.clone();
            let tx = self.tx.clone();
            self.rt.spawn(async move {
                let res = session
                    .apply_user_privileges(&user, &host, &grants, &revokes)
                    .await
                    .map(|()| format!("Updated privileges for `{user}`@`{host}`."))
                    .map_err(|e| format!("{e:#}"));
                let _ = tx.send(AsyncEvent::UserActionFinished(res));
            });
        }
    }
}
