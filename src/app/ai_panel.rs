use super::*;

impl MysqlRustyApp {
    pub(crate) fn trigger_inline_ai_completion(&mut self) {
        let Some(tab) = self.tabs.get_mut(self.active_tab_idx) else {
            return;
        };
        tab.inline_req_id += 1;
        let req_id = tab.inline_req_id;
        let tab_id = tab.id.clone();
        let (sql_prefix, sql_suffix) = {
            let (p, s) = split_sql_at_cursor(&tab.sql, tab.cursor_index);
            (p.to_string(), s.to_string())
        };

        // Opción 1: Local Schema Native runs in 0ms without async network spawning
        if self.config.ai.provider == AiProvider::LocalSchema {
            let schema = self.schemas.get(&self.active_database);
            tab.ghost_completion = compute_heuristic_ghost_completion(&sql_prefix, schema);
            return;
        }

        let ddl = self
            .schemas
            .get(&self.active_database)
            .map(|s| s.to_pruned_ai_ddl_context(&sql_prefix))
            .unwrap_or_default();
        let ai_cfg = self.config.ai.clone();
        let tx = self.tx.clone();

        self.is_inline_ai_loading = true;
        self.status_banner = format!(
            "Requesting inline SQL completion from {}...",
            ai_cfg.provider.label()
        );

        self.rt.spawn(async move {
            let res = request_inline_completion(&ai_cfg, &sql_prefix, &sql_suffix, &ddl)
                .await
                .map_err(|e| format!("{e:#}"));
            let _ = tx.send(AsyncEvent::InlineAiCompletion {
                tab_id,
                req_id,
                result: res,
            });
        });
    }

    pub(crate) fn trigger_ai_error_fix(&mut self, tab_id: &str, err_msg: &str) {
        if !self.config.ai.enable_auto_error_fix {
            return;
        }
        let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) else {
            return;
        };
        let failed_sql = tab
            .last_executed_sql
            .as_deref()
            .filter(|s| !s.trim().is_empty() && tab.sql.contains(s))
            .unwrap_or(&tab.sql)
            .to_string();
        if failed_sql.trim().is_empty() {
            return;
        }

        tab.ai_error_fix_req_id += 1;
        let req_id = tab.ai_error_fix_req_id;
        tab.ai_error_fix = None;

        let schema = self.schemas.get(&self.active_database).cloned();
        if self.config.ai.provider == AiProvider::LocalSchema {
            if let Some(mut fix) =
                crate::ai::compute_heuristic_sql_error_fix(&failed_sql, err_msg, schema.as_ref())
            {
                align_fix_to_tab_sql(&tab.sql, &mut fix);
                tab.ai_error_fix = Some(fix);
                self.status_banner =
                    "AI fix ready — Press Tab to apply or Esc to dismiss.".to_string();
            }
            return;
        }

        tab.ai_error_fix_loading = true;
        let ddl = schema
            .as_ref()
            .map(|s| s.to_pruned_ai_ddl_context(&failed_sql))
            .unwrap_or_default();
        let dialect = self
            .active_session
            .as_ref()
            .map(|s| s.dialect().display_name())
            .unwrap_or("MySQL")
            .to_string();
        let ai_cfg = self.config.ai.clone();
        let tx = self.tx.clone();
        let tab_id_owned = tab_id.to_string();
        let err_owned = err_msg.to_string();

        self.rt.spawn(async move {
            let res = crate::ai::request_sql_error_fix(
                &ai_cfg,
                &dialect,
                &failed_sql,
                &err_owned,
                &ddl,
                schema.as_ref(),
            )
            .await
            .map_err(|e| format!("{e:#}"));
            let _ = tx.send(AsyncEvent::AiErrorFixReady {
                tab_id: tab_id_owned,
                req_id,
                result: res,
            });
        });
    }

    pub(crate) fn send_chat_prompt(&mut self, custom_prompt: Option<String>) {
        let prompt = custom_prompt.unwrap_or_else(|| self.chat_input.trim().to_string());
        if prompt.is_empty() {
            return;
        }
        self.chat_input.clear();
        self.chat_messages.push(ChatMessage {
            role: "user".to_string(),
            content: prompt.clone(),
            extracted_sql: None,
        });
        self.is_chat_loading = true;

        let current_sql = self
            .tabs
            .get(self.active_tab_idx)
            .map(|t| t.sql.clone())
            .unwrap_or_default();
        let ddl = self
            .schemas
            .get(&self.active_database)
            .map(|s| s.to_ai_ddl_context())
            .unwrap_or_default();
        let ai_cfg = self.config.ai.clone();
        let tx = self.tx.clone();

        self.rt.spawn(async move {
            let res = request_chat_assistant(&ai_cfg, &prompt, &current_sql, &ddl)
                .await
                .map_err(|e| format!("{e:#}"));
            let _ = tx.send(AsyncEvent::ChatAiReply(res));
        });
    }

    pub(crate) fn render_right_ai_panel(&mut self, ui: &mut egui::Ui) {
        egui::Panel::right("ai_assistant_panel")
            .default_size(340.0)
            .size_range(260.0..=520.0)
            .show(ui, |ui| {
                self.tour
                    .set_anchor(crate::tour::TourAnchor::AiPanel, ui.max_rect());
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.heading(
                        RichText::new("AI SQL Copilot")
                            .color(Color32::from_rgb(192, 132, 252))
                            .strong(),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("Configure").clicked() {
                            self.show_ai_settings_modal = true;
                        }
                    });
                });
                ui.label(
                    RichText::new(format!("Active Engine: {}", self.config.ai.provider.label()))
                        .small()
                        .weak(),
                );
                ui.checkbox(
                    &mut self.config.ai.include_schema_context,
                    "Include active schema DDL context",
                );
                ui.separator();

                // Quick AI actions
                ui.horizontal_wrapped(|ui| {
                    if ui.small_button("Explain Current Query").clicked() {
                        self.send_chat_prompt(Some(
                            "Explain how the current SQL query works and whether any indexes can speed it up.".to_string(),
                        ));
                    }
                    if ui.small_button("Suggest Indexes").clicked() {
                        self.send_chat_prompt(Some(
                            "Rewrite the current SQL query for maximum performance in MySQL 8.0 and provide CREATE INDEX statements if helpful.".to_string(),
                        ));
                    }
                    if ui.small_button("Top Revenue Report").clicked() {
                        self.send_chat_prompt(Some(
                            "Write a MySQL query joining users, orders, and order_items to find the top customers and most profitable product categories.".to_string(),
                        ));
                    }
                });
                ui.separator();

                let mut sql_to_insert: Option<(String, bool)> = None;
                let available_height = (ui.available_height() - 95.0).max(120.0);

                egui::ScrollArea::vertical()
                    .max_height(available_height)
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        for msg in &self.chat_messages {
                            let is_user = msg.role == "user";
                            let bg = if is_user {
                                Color32::from_rgb(30, 41, 59)
                            } else {
                                Color32::from_rgb(15, 23, 42)
                            };
                            egui::Frame::group(ui.style()).fill(bg).show(ui, |ui| {
                                ui.label(
                                    RichText::new(if is_user { "You" } else { "AI Copilot" })
                                        .strong()
                                        .small()
                                        .color(if is_user {
                                            Color32::from_rgb(56, 189, 248)
                                        } else {
                                            Color32::from_rgb(192, 132, 252)
                                        }),
                                );
                                ui.label(&msg.content);
                                if let Some(sql_block) = &msg.extracted_sql {
                                    ui.horizontal(|ui| {
                                        if ui.small_button("Insert into Editor").clicked() {
                                            sql_to_insert = Some((sql_block.clone(), false));
                                        }
                                        if ui.small_button("Insert & Run").clicked() {
                                            sql_to_insert = Some((sql_block.clone(), true));
                                        }
                                    });
                                }
                            });
                            ui.add_space(4.0);
                        }
                        if self.is_chat_loading {
                            ui.spinner();
                            ui.label(RichText::new("Generating SQL response...").italics());
                        }
                    });

                if let Some((sql, run_now)) = sql_to_insert {
                    if let Some(tab) = self.tabs.get_mut(self.active_tab_idx) {
                        tab.sql = sql;
                    }
                    if run_now {
                        self.execute_current_tab_query(None);
                    }
                }

                ui.separator();
                ui.horizontal(|ui| {
                    let resp = ui.add(
                        egui::TextEdit::multiline(&mut self.chat_input)
                            .desired_rows(2)
                            .desired_width(ui.available_width() - 62.0)
                            .hint_text("Ask in natural language (e.g. 'Monthly revenue by country')..."),
                    );
                    if (resp.lost_focus()
                        && ui.input(|i| i.key_pressed(Key::Enter) && !i.modifiers.shift))
                        || ui.button("Send").clicked()
                    {
                        self.send_chat_prompt(None);
                    }
                });
            });
    }
}

pub(crate) fn align_fix_to_tab_sql(
    tab_sql: &str,
    fix: &mut crate::ai::SqlErrorFixSuggestion,
) -> bool {
    if tab_sql == fix.original_sql {
        fix.replace_byte_range = None;
        return true;
    }
    if let Some(base) = tab_sql.find(&fix.original_sql) {
        let end = base + fix.original_sql.len();
        for r in &mut fix.strike_ranges {
            r.start += base;
            r.end += base;
        }
        fix.replace_byte_range = Some(base..end);
        return true;
    }
    false
}

pub(crate) fn apply_ai_error_fix(tab: &mut QueryTab) -> Option<usize> {
    let fix = tab.ai_error_fix.take()?;
    tab.ai_error_fix_loading = false;
    let new_cursor = if let Some(range) = fix.replace_byte_range {
        if range.end <= tab.sql.len()
            && tab.sql.is_char_boundary(range.start)
            && tab.sql.is_char_boundary(range.end)
        {
            tab.sql.replace_range(range.clone(), &fix.fixed_sql);
            tab.sql[..range.start].chars().count() + fix.fixed_sql.chars().count()
        } else {
            tab.sql = fix.fixed_sql;
            tab.sql.chars().count()
        }
    } else {
        tab.sql = fix.fixed_sql;
        tab.sql.chars().count()
    };
    tab.is_dirty = true;
    tab.ghost_completion = None;
    Some(new_cursor)
}
