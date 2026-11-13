use super::*;

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum ServerAdminSubtab {
    OverviewAndMetrics,
    ServerVariables,
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
                "📊 Metrics & Processes",
            );
            ui.selectable_value(
                &mut self.server_admin_subtab,
                ServerAdminSubtab::ServerVariables,
                "⚙ Server Variables (SHOW VARIABLES)",
            );
        });
        ui.separator();

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
}
