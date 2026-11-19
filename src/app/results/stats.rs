use super::super::*;

impl MysqlRustyApp {
    pub(crate) fn er_viewport_id() -> egui::ViewportId {
        egui::ViewportId::from_hash_of("er_diagram_viewport")
    }

    pub(crate) fn render_er_relationships_inner(&mut self, ui: &mut Ui) {
        let Some(schema) = self.schemas.get(&self.active_database).cloned() else {
            ui.label(RichText::new("No active schema loaded.").weak());
            return;
        };
        self.er_canvas_state.show(ui, &schema);
    }

    pub(crate) fn render_er_relationships(&mut self, ui: &mut Ui) {
        if self.er_undocked {
            ui.vertical_centered(|ui| {
                ui.add_space(24.0);
                ui.label(RichText::new("ER Diagram is open in a detached window.").weak());
                if ui.button("Redock ER Diagram").clicked() {
                    self.er_undocked = false;
                }
            });
            return;
        }
        self.render_er_relationships_inner(ui);
    }

    pub(crate) fn render_execution_stats(&mut self, ui: &mut Ui) {
        let Some((tab_id, tab_sql, ai_loading, ai_report, ai_error)) =
            self.tabs.get(self.active_tab_idx).map(|t| {
                (
                    t.id.clone(),
                    t.sql.clone(),
                    t.ai_plan_loading,
                    t.ai_plan_report.clone(),
                    t.ai_plan_error.clone(),
                )
            })
        else {
            return;
        };

        let is_read_only = self
            .active_profile_id
            .as_ref()
            .and_then(|id| self.config.connections.iter().find(|p| &p.id == id))
            .is_some_and(|p| p.read_only);

        ui.horizontal(|ui| {
            let can_analyze =
                self.active_session.is_some() && !tab_sql.trim().is_empty() && !ai_loading;
            let btn_label = if ai_loading {
                "Analyzing Plan..."
            } else {
                "AI Explain & Optimize"
            };
            if ui
                .add_enabled(can_analyze, egui::Button::new(btn_label))
                .clicked()
                && let Some(session) = self.active_session.clone()
            {
                if let Some(tab) = self.tabs.get_mut(self.active_tab_idx) {
                    tab.ai_plan_loading = true;
                    tab.ai_plan_error = None;
                }
                let tx = self.tx.clone();
                let ctx = ui.ctx().clone();
                let db = self.active_database.clone();
                let ai_config = self.config.ai.clone();
                let schema = self.schemas.get(&db).cloned();
                self.rt.spawn(async move {
                    let res = match session.run_explain_analyze(&db, &tab_sql).await {
                        Ok(plan_out) => crate::ai::diagnose_query_plan(
                            &ai_config,
                            &tab_sql,
                            &plan_out,
                            schema.as_ref(),
                        )
                        .await
                        .map_err(|e| format!("{e:#}")),
                        Err(e) => Err(format!("{e:#}")),
                    };
                    let _ = tx.send(AsyncEvent::AiOptimizePlanFinished {
                        tab_id,
                        result: res,
                    });
                    ctx.request_repaint();
                });
            }
        });

        let pal = self.effective_palette(ui.ctx());
        if let Some(err) = &ai_error {
            ui.colored_label(pal.danger, format!("AI Explain error: {err}"));
        }

        if let Some(report) = &ai_report {
            ui.add_space(6.0);
            egui::Frame::new()
                .fill(pal.card_bg)
                .stroke(Stroke::new(1.0, pal.accent))
                .corner_radius(egui::CornerRadius::same(6))
                .inner_margin(10.0)
                .show(ui, |ui| {
                    ui.label(
                        RichText::new("AI Query Optimizer Diagnosis")
                            .strong()
                            .color(pal.accent_text),
                    );
                    ui.add_space(4.0);
                    ui.label(RichText::new(&report.summary).color(pal.text_primary));

                    if !report.bottlenecks.is_empty() {
                        ui.add_space(4.0);
                        ui.horizontal_wrapped(|ui| {
                            ui.label(
                                RichText::new("Bottlenecks:")
                                    .strong()
                                    .small()
                                    .color(pal.text_secondary),
                            );
                            for b in &report.bottlenecks {
                                pal.badge(ui, b, pal.warning, pal.warning_bg);
                            }
                        });
                    }

                    if let Some(rewritten) = &report.rewritten_sql {
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("Rewritten SQL:")
                                    .strong()
                                    .small()
                                    .color(pal.text_secondary),
                            );
                            if ui.small_button("Open in New Tab").clicked() {
                                self.tabs.push(QueryTab::new("Optimized Query", rewritten));
                                self.active_tab_idx = self.tabs.len() - 1;
                            }
                            if ui.small_button("Copy SQL").clicked() {
                                ui.ctx().copy_text(rewritten.clone());
                            }
                        });
                        ui.code(rewritten);
                    }

                    if !report.suggested_indexes.is_empty() {
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new("Suggested Indexes:")
                                .strong()
                                .small()
                                .color(pal.text_secondary),
                        );
                        for ddl in &report.suggested_indexes {
                            ui.horizontal_wrapped(|ui| {
                                ui.monospace(ddl);
                                if ui.small_button("Copy").clicked() {
                                    ui.ctx().copy_text(ddl.clone());
                                }
                                if ui
                                    .add_enabled(
                                        !is_read_only && self.active_session.is_some(),
                                        egui::Button::new("Create Index").small(),
                                    )
                                    .on_disabled_hover_text("Disabled on read-only connections")
                                    .clicked()
                                {
                                    self.tabs.push(QueryTab::new("Create Index", ddl));
                                    self.active_tab_idx = self.tabs.len() - 1;
                                }
                            });
                        }
                    }
                });
            ui.add_space(6.0);
        }

        let Some(tab) = self.tabs.get(self.active_tab_idx) else {
            return;
        };
        if let Some(res) = &tab.last_result {
            ui.label(
                RichText::new("Last Executed SQL Statement")
                    .strong()
                    .color(pal.text_primary),
            );
            ui.code(&res.sql);
            ui.add_space(6.0);

            egui::Grid::new("exec_stats_summary_grid")
                .num_columns(2)
                .spacing([16.0, 4.0])
                .show(ui, |ui| {
                    ui.label(RichText::new("Target Database").color(pal.muted_text));
                    ui.label(RichText::new(&res.database).strong().color(pal.text_primary));
                    ui.end_row();

                    ui.label(RichText::new("Execution Time").color(pal.muted_text));
                    ui.label(
                        RichText::new(format!("{} ms", res.duration_ms))
                            .strong()
                            .color(pal.accent_text),
                    );
                    ui.end_row();

                    ui.label(RichText::new("Rows Returned").color(pal.muted_text));
                    ui.label(
                        RichText::new(res.rows.len().to_string())
                            .strong()
                            .color(pal.text_primary),
                    );
                    ui.end_row();

                    ui.label(RichText::new("Rows Affected").color(pal.muted_text));
                    ui.label(
                        RichText::new(res.affected_rows.to_string())
                            .strong()
                            .color(pal.text_primary),
                    );
                    ui.end_row();

                    if let Some(last_id) = res.last_insert_id {
                        ui.label(RichText::new("Last Insert ID").color(pal.muted_text));
                        ui.label(
                            RichText::new(last_id.to_string())
                                .strong()
                                .color(pal.text_primary),
                        );
                        ui.end_row();
                    }
                });

            // Visual EXPLAIN Tree (MySQL/MariaDB format)
            let is_postgres = self
                .active_session
                .as_ref()
                .is_some_and(|s| s.dialect() == crate::dialect::Dialect::Postgres);
            let is_explain = res.sql.trim().to_uppercase().starts_with("EXPLAIN")
                || res
                    .columns
                    .iter()
                    .any(|c| c.eq_ignore_ascii_case("select_type"));
            if !is_postgres && is_explain && !res.rows.is_empty() {
                ui.separator();
                ui.heading(
                    RichText::new("Visual Execution Plan (EXPLAIN Tree)")
                        .color(pal.accent_text)
                        .size(15.0),
                );
                ui.add_space(4.0);

                let col_table_idx = res
                    .columns
                    .iter()
                    .position(|c| c.eq_ignore_ascii_case("table"));
                let col_type_idx = res
                    .columns
                    .iter()
                    .position(|c| c.eq_ignore_ascii_case("type"));
                let col_key_idx = res
                    .columns
                    .iter()
                    .position(|c| c.eq_ignore_ascii_case("key"));
                let col_rows_idx = res
                    .columns
                    .iter()
                    .position(|c| c.eq_ignore_ascii_case("rows"));
                let col_extra_idx = res
                    .columns
                    .iter()
                    .position(|c| c.eq_ignore_ascii_case("Extra"));

                let mut has_full_scan = false;

                for (idx, row) in res.rows.iter().enumerate() {
                    let table_name = col_table_idx
                        .and_then(|i| row.get(i))
                        .map(|c| c.display_text())
                        .unwrap_or_else(|| format!("Node #{idx}"));
                    let access_type = col_type_idx
                        .and_then(|i| row.get(i))
                        .map(|c| c.display_text())
                        .unwrap_or_else(|| "ALL".to_string());
                    let key_used = col_key_idx
                        .and_then(|i| row.get(i))
                        .map(|c| c.display_text())
                        .unwrap_or_else(|| "NULL".to_string());
                    let rows_est = col_rows_idx
                        .and_then(|i| row.get(i))
                        .map(|c| c.display_text())
                        .unwrap_or_else(|| "—".to_string());
                    let extra = col_extra_idx
                        .and_then(|i| row.get(i))
                        .map(|c| c.display_text())
                        .unwrap_or_default();

                    let (type_color, type_bg, type_badge) = match access_type.to_uppercase().as_str()
                    {
                        "ALL" => {
                            has_full_scan = true;
                            (pal.danger, pal.danger_bg, "FULL TABLE SCAN")
                        }
                        "INDEX" => (pal.warning, pal.warning_bg, "FULL INDEX SCAN"),
                        "RANGE" => (pal.warning, pal.warning_bg, "INDEX RANGE SCAN"),
                        "REF" | "EQ_REF" | "CONST" => (pal.success, pal.success_bg, "INDEX LOOKUP"),
                        _ => (pal.muted_text, pal.badge_bg, "SCAN"),
                    };

                    egui::Frame::new()
                        .fill(pal.card_bg)
                        .stroke(Stroke::new(1.0, type_color))
                        .corner_radius(egui::CornerRadius::same(6))
                        .inner_margin(10.0)
                        .show(ui, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                ui.label(
                                    RichText::new(format!("Step {}: {table_name}", idx + 1))
                                        .strong()
                                        .size(13.0)
                                        .color(pal.text_primary),
                                );
                                pal.badge(ui, type_badge, type_color, type_bg);
                                ui.label(
                                    RichText::new(format!("Est. Rows: {rows_est}"))
                                        .small()
                                        .color(pal.muted_text),
                                );
                            });
                            ui.horizontal_wrapped(|ui| {
                                ui.label(
                                    RichText::new(format!("Index Used: {key_used}"))
                                        .small()
                                        .color(pal.accent_text),
                                );
                                if !extra.is_empty() {
                                    ui.label(
                                        RichText::new(format!("Extra: {extra}"))
                                            .small()
                                            .color(pal.muted_text),
                                    );
                                }
                            });
                        });
                    ui.add_space(4.0);
                }

                if has_full_scan {
                    pal.banner_frame(pal.danger_bg, pal.danger)
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new(
                                    "Optimization Tip: Table performs a FULL TABLE SCAN (ALL). Creating an index on the join or WHERE filter columns can drastically reduce query latency.",
                                )
                                .color(pal.danger)
                                .small(),
                            );
                        });
                }

                // Index Advisor Recommendation
                let mut advise_index_actions: Vec<(String, String)> = Vec::new();
                for row in &res.rows {
                    let table_name = col_table_idx
                        .and_then(|i| row.get(i))
                        .map(|c| c.display_text())
                        .unwrap_or_default();
                    let access_type = col_type_idx
                        .and_then(|i| row.get(i))
                        .map(|c| c.display_text())
                        .unwrap_or_default();
                    let key_used = col_key_idx
                        .and_then(|i| row.get(i))
                        .map(|c| c.display_text())
                        .unwrap_or_default();
                    let extra = col_extra_idx
                        .and_then(|i| row.get(i))
                        .map(|c| c.display_text())
                        .unwrap_or_default();

                    if !table_name.is_empty()
                        && (access_type.eq_ignore_ascii_case("ALL")
                            || key_used == "NULL"
                            || extra.contains("Using filesort"))
                    {
                        let recommended_cols =
                            if let Some(schema) = self.schemas.get(&self.active_database) {
                                if let Some(tbl) = schema
                                    .tables
                                    .iter()
                                    .find(|t| t.name.eq_ignore_ascii_case(&table_name))
                                {
                                    tbl.columns
                                        .iter()
                                        .filter(|c| !c.is_primary_key)
                                        .take(2)
                                        .map(|c| c.name.clone())
                                        .collect::<Vec<_>>()
                                } else {
                                    vec!["created_at".to_string()]
                                }
                            } else {
                                vec!["status".to_string()]
                            };
                        let col_str = recommended_cols.join(", ");
                        let first_col = recommended_cols
                            .first()
                            .cloned()
                            .unwrap_or_else(|| "lookup".into());
                        let idx_name = format!("idx_{}_{}", table_name, first_col);
                        let ddl =
                            format!("CREATE INDEX `{idx_name}` ON `{table_name}` ({col_str});");
                        if !advise_index_actions.iter().any(|(t, _)| t == &table_name) {
                            advise_index_actions.push((table_name, ddl));
                        }
                    }
                }

                if !advise_index_actions.is_empty() {
                    ui.add_space(6.0);
                    egui::Frame::new()
                        .fill(pal.purple_bg)
                        .stroke(Stroke::new(1.0, pal.purple))
                        .corner_radius(egui::CornerRadius::same(6))
                        .inner_margin(10.0)
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new("Index Advisor Recommendations")
                                    .strong()
                                    .color(pal.purple),
                            );
                            ui.add_space(4.0);
                            for (tbl, ddl) in advise_index_actions {
                                ui.horizontal_wrapped(|ui| {
                                    ui.label(
                                        RichText::new(format!("• For {tbl}:"))
                                            .strong()
                                            .small()
                                            .color(pal.text_primary),
                                    );
                                    ui.monospace(&ddl);
                                    if ui.small_button("Copy").clicked() {
                                        ui.ctx().copy_text(ddl.clone());
                                        self.status_banner =
                                            format!("Copied index DDL for `{tbl}` to clipboard.");
                                    }
                                    if ui.small_button("Open in Tab").clicked() {
                                        self.tabs.push(QueryTab::new(format!("Index {tbl}"), ddl));
                                        self.active_tab_idx = self.tabs.len() - 1;
                                    }
                                });
                            }
                        });
                }
            }
        }
        if let Some(schema) = self.schemas.get(&self.active_database) {
            ui.separator();
            egui::CollapsingHeader::new(
                RichText::new("AI Schema DDL Context Preview")
                    .strong()
                    .color(pal.text_secondary),
            )
            .id_salt("stats_ai_ddl_preview")
            .default_open(false)
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("stats_ai_ddl_scroll")
                    .max_height(180.0)
                    .show(ui, |ui| {
                        ui.code(schema.to_ai_ddl_context());
                    });
            });
        }
    }
}
