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
