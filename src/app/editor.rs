use super::*;

impl MysqlRustyApp {
    pub(crate) fn render_central_workspace(&mut self, ui: &mut egui::Ui) {
        if self.active_session.is_none() {
            egui::CentralPanel::default().show(ui, |ui| {
                self.render_connections_dashboard(ui);
            });
            return;
        }

        let trigger_save = ui.input_mut(|i| {
            i.consume_key(Modifiers::COMMAND, Key::S) || i.consume_key(Modifiers::CTRL, Key::S)
        });
        let trigger_open = ui.input_mut(|i| {
            i.consume_key(Modifiers::COMMAND, Key::O) || i.consume_key(Modifiers::CTRL, Key::O)
        });
        if trigger_save {
            self.save_active_tab_to_file(false);
        }
        if trigger_open {
            self.open_sql_file_dialog();
        }

        egui::CentralPanel::default().show(ui, |ui| {
            // Query Tabs Bar
            ui.horizontal(|ui| {
                ui.add_space(2.0);
                let mut close_idx = None;
                let mut tab_to_rename: Option<(usize, String)> = None;

                for idx in 0..self.tabs.len() {
                    let is_active = idx == self.active_tab_idx;
                    let is_renaming = self.renaming_tab_idx == Some(idx);
                    let title = self.tabs[idx].display_title();

                    let tab_bg = if is_active {
                        Color32::from_rgb(13, 18, 28)
                    } else {
                        Color32::from_rgb(9, 13, 20)
                    };
                    let tab_stroke = if is_active {
                        Stroke::new(1.0, Color32::from_rgb(56, 189, 248))
                    } else {
                        Stroke::new(1.0, Color32::from_rgb(26, 35, 51))
                    };

                    egui::Frame::new()
                        .fill(tab_bg)
                        .stroke(tab_stroke)
                        .corner_radius(egui::CornerRadius { nw: 6, ne: 6, sw: 0, se: 0 })
                        .inner_margin(egui::Margin::symmetric(8, 4))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                if is_renaming {
                                    let edit_resp = ui.add(
                                        egui::TextEdit::singleline(&mut self.rename_tab_buffer)
                                            .desired_width(100.0),
                                    );
                                    if edit_resp.lost_focus() || ui.input(|i| i.key_pressed(Key::Enter)) {
                                        tab_to_rename = Some((idx, self.rename_tab_buffer.clone()));
                                    }
                                    if ui.input(|i| i.key_pressed(Key::Escape)) {
                                        self.renaming_tab_idx = None;
                                    }
                                } else {
                                    let label_color = if is_active {
                                        Color32::WHITE
                                    } else {
                                        Color32::from_rgb(148, 163, 184)
                                    };
                                    let label = RichText::new(&title).size(12.5).color(label_color);
                                    let resp = ui.selectable_label(is_active, label);
                                    if resp.clicked() {
                                        self.active_tab_idx = idx;
                                    }
                                    if resp.double_clicked() {
                                        self.renaming_tab_idx = Some(idx);
                                        self.rename_tab_buffer = self.tabs[idx].title.clone();
                                    }
                                    resp.context_menu(|ui| {
                                        if ui.button("Rename").clicked() {
                                            self.renaming_tab_idx = Some(idx);
                                            self.rename_tab_buffer = self.tabs[idx].title.clone();
                                            ui.close();
                                        }
                                        if ui.button("Save (Direct)").clicked() {
                                            self.active_tab_idx = idx;
                                            self.save_active_tab_direct();
                                            ui.close();
                                        }
                                        if self.tabs.len() > 1 && ui.button("Close Tab").clicked() {
                                            close_idx = Some(idx);
                                            ui.close();
                                        }
                                    });
                                }
                                if self.tabs.len() > 1
                                    && ui.button(RichText::new("×").size(11.0).weak()).clicked() {
                                        close_idx = Some(idx);
                                    }
                            });
                        });
                    ui.add_space(2.0);
                }
                if let Some((idx, new_title)) = tab_to_rename {
                    self.rename_tab(idx, new_title);
                    self.renaming_tab_idx = None;
                }
                if let Some(idx) = close_idx {
                    self.tabs.remove(idx);
                    if self.active_tab_idx >= self.tabs.len() {
                        self.active_tab_idx = self.tabs.len().saturating_sub(1);
                    }
                }
                if ui
                    .add(
                        egui::Button::new(RichText::new("+ New Tab").size(12.0).weak())
                            .fill(Color32::from_rgb(15, 21, 33))
                            .corner_radius(egui::CornerRadius::same(5)),
                    )
                    .clicked()
                {
                    let n = self.tabs.len() + 1;
                    self.tabs
                        .push(QueryTab::new(format!("Query {n}"), "SELECT * FROM users LIMIT 50;"));
                    self.active_tab_idx = self.tabs.len() - 1;
                }

                ui.add_space(4.0);
                if ui
                    .small_button("Rename Tab")
                    .on_hover_text("Rename active query tab (or double-click tab name)")
                    .clicked()
                {
                    self.renaming_tab_idx = Some(self.active_tab_idx);
                    if let Some(tab) = self.tabs.get(self.active_tab_idx) {
                        self.rename_tab_buffer = tab.title.clone();
                    }
                }
                if ui
                    .small_button("Save (Ctrl+S)")
                    .on_hover_text("Directly save query to disk and library")
                    .clicked()
                {
                    self.save_active_tab_direct();
                }
                if ui
                    .small_button("Open (Ctrl+O)")
                    .on_hover_text("Open .sql file from disk into an editor tab")
                    .clicked()
                {
                    self.open_sql_file_dialog();
                }
            });
            ui.separator();

            // SQL Editor Section
            let schema = self.schemas.get(&self.active_database).cloned();
            let enable_ghost = self.config.ai.enable_inline_autocomplete;
            let mut request_fix_with_ai: Option<String> = None;
            let mut editor_run_query = false;
            let mut editor_run_stmt = false;
            let in_tx = self.in_transaction;
            let uncommitted_muts = self.uncommitted_mutations;
            let mut trigger_tx_action: Option<u8> = None;

            if let Some(tab) = self.tabs.get_mut(self.active_tab_idx) {
                // Quick LIMIT Presets Bar
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new("Limit:")
                            .size(12.0)
                            .strong()
                            .color(Color32::from_rgb(148, 163, 184)),
                    );

                    let active_sql_target = tab.selected_sql.as_deref().unwrap_or(&tab.sql);
                    let current_limit = detect_current_limit(active_sql_target);

                    let presets = [
                        (Some(10), "10"),
                        (Some(50), "50"),
                        (Some(100), "100"),
                        (Some(250), "250"),
                        (Some(500), "500"),
                        (Some(1000), "1000"),
                        (Some(5000), "5000"),
                        (None, "All"),
                    ];

                    for (preset_val, label) in presets {
                        let is_selected = current_limit == preset_val;
                        let btn = egui::Button::new(
                            RichText::new(label)
                                .size(11.0)
                                .strong()
                                .color(if is_selected {
                                    Color32::WHITE
                                } else {
                                    Color32::from_rgb(203, 213, 225)
                                }),
                        )
                        .fill(if is_selected {
                            Color32::from_rgb(14, 116, 144)
                        } else {
                            Color32::from_rgb(20, 27, 41)
                        })
                        .stroke(Stroke::new(
                            1.0,
                            if is_selected {
                                Color32::from_rgb(56, 189, 248)
                            } else {
                                Color32::from_rgb(33, 43, 63)
                            },
                        ))
                        .corner_radius(egui::CornerRadius::same(4));

                        if ui
                            .add(btn)
                            .on_hover_text(if let Some(n) = preset_val {
                                format!("Set LIMIT {n} in current SQL query")
                            } else {
                                "Remove LIMIT clause (return all rows)".to_string()
                            })
                            .clicked()
                        {
                            if let Some(sel) = tab.selected_sql.clone() {
                                let updated_sel = apply_limit_to_sql(&sel, preset_val);
                                tab.sql = tab.sql.replace(&sel, &updated_sel);
                                tab.selected_sql = Some(updated_sel);
                            } else {
                                tab.sql = apply_limit_to_sql(&tab.sql, preset_val);
                            }
                            tab.is_dirty = true;
                        }
                    }

                    egui::ComboBox::from_id_salt("limit_more_presets_combo")
                        .selected_text(if let Some(lim) = current_limit {
                            format!("{lim} rows")
                        } else {
                            "No Limit".to_string()
                        })
                        .width(90.0)
                        .show_ui(ui, |ui| {
                            for val in [5, 10, 20, 25, 50, 100, 200, 250, 500, 1000, 2000, 5000, 10000] {
                                let is_sel = current_limit == Some(val);
                                if ui.selectable_label(is_sel, format!("LIMIT {val}")).clicked() {
                                    if let Some(sel) = tab.selected_sql.clone() {
                                        let updated_sel = apply_limit_to_sql(&sel, Some(val));
                                        tab.sql = tab.sql.replace(&sel, &updated_sel);
                                        tab.selected_sql = Some(updated_sel);
                                    } else {
                                        tab.sql = apply_limit_to_sql(&tab.sql, Some(val));
                                    }
                                    tab.is_dirty = true;
                                }
                            }
                            ui.separator();
                            if ui.selectable_label(current_limit.is_none(), "No Limit (All)").clicked() {
                                if let Some(sel) = tab.selected_sql.clone() {
                                    let updated_sel = apply_limit_to_sql(&sel, None);
                                    tab.sql = tab.sql.replace(&sel, &updated_sel);
                                    tab.selected_sql = Some(updated_sel);
                                } else {
                                    tab.sql = apply_limit_to_sql(&tab.sql, None);
                                }
                                tab.is_dirty = true;
                            }
                        });

                    ui.separator();
                    if in_tx {
                        ui.label(
                            RichText::new(format!(
                                "⚠ IN TX ({} edit{})",
                                uncommitted_muts,
                                if uncommitted_muts == 1 { "" } else { "s" }
                            ))
                            .size(11.0)
                            .color(Color32::from_rgb(250, 204, 21))
                            .strong(),
                        );
                        let commit_btn = egui::Button::new(
                            RichText::new("Commit")
                                .size(11.0)
                                .strong()
                                .color(Color32::from_rgb(220, 252, 231)),
                        )
                        .fill(Color32::from_rgb(22, 101, 52))
                        .stroke(Stroke::new(1.0, Color32::from_rgb(34, 197, 94)))
                        .corner_radius(egui::CornerRadius::same(4));

                        if ui
                            .add(commit_btn)
                            .on_hover_text("Commit current open transaction (COMMIT)")
                            .clicked()
                        {
                            trigger_tx_action = Some(1);
                        }

                        let rb_btn = egui::Button::new(
                            RichText::new("Rollback")
                                .size(11.0)
                                .strong()
                                .color(Color32::from_rgb(254, 202, 202)),
                        )
                        .fill(Color32::from_rgb(153, 27, 27))
                        .stroke(Stroke::new(1.0, Color32::from_rgb(239, 68, 68)))
                        .corner_radius(egui::CornerRadius::same(4));

                        if ui
                            .add(rb_btn)
                            .on_hover_text("Rollback current open transaction (ROLLBACK)")
                            .clicked()
                        {
                            trigger_tx_action = Some(2);
                        }
                    } else {
                        let begin_btn = egui::Button::new(
                            RichText::new("Begin Tx")
                                .size(11.0)
                                .color(Color32::from_rgb(203, 213, 225)),
                        )
                        .fill(Color32::from_rgb(20, 27, 41))
                        .stroke(Stroke::new(1.0, Color32::from_rgb(51, 65, 85)))
                        .corner_radius(egui::CornerRadius::same(4));

                        if ui
                            .add(begin_btn)
                            .on_hover_text("Start a manual transaction (START TRANSACTION)")
                            .clicked()
                        {
                            trigger_tx_action = Some(3);
                        }
                    }
                });
                ui.add_space(2.0);

                // Dynamic Template Parameters (:param_name) Bar
                let detected_params = extract_sql_parameters(&tab.sql);
                if !detected_params.is_empty() {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(
                            RichText::new("Params:")
                                .size(12.0)
                                .strong()
                                .color(Color32::from_rgb(192, 132, 252)),
                        );
                        for p in &detected_params {
                            let val_entry = tab.query_params.entry(p.clone()).or_default();
                            ui.label(RichText::new(format!(":{p} =")).monospace().small());
                            ui.add(
                                egui::TextEdit::singleline(val_entry)
                                    .desired_width(95.0)
                                    .hint_text("value or NULL"),
                            );
                        }
                    });
                    ui.add_space(2.0);
                }

                let editor_id = ui.make_persistent_id(("sql_editor", &tab.id));
                let mut move_cursor_to_char: Option<usize> = None;
                let mut keep_editor_focus = false;

                // Accept or dismiss suggestions via keyboard: Esc, Alt+Up/Down, Tab, Alt+1..8
                let mut selected_suggestion: Option<CompletionItem> = None;
                let has_suggestions_or_ghost =
                    tab.ghost_completion.is_some() || !tab.local_suggestions.is_empty();

                if has_suggestions_or_ghost
                    && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
                {
                    tab.ghost_completion = None;
                    tab.local_suggestions.clear();
                    tab.selected_suggestion_idx = 0;
                    keep_editor_focus = true;
                } else if !tab.local_suggestions.is_empty()
                    && ui.input_mut(|i| i.consume_key(Modifiers::ALT, Key::ArrowDown))
                {
                    tab.selected_suggestion_idx =
                        (tab.selected_suggestion_idx + 1) % tab.local_suggestions.len();
                    keep_editor_focus = true;
                } else if !tab.local_suggestions.is_empty()
                    && ui.input_mut(|i| i.consume_key(Modifiers::ALT, Key::ArrowUp))
                {
                    tab.selected_suggestion_idx = tab
                        .selected_suggestion_idx
                        .checked_sub(1)
                        .unwrap_or(tab.local_suggestions.len() - 1);
                    keep_editor_focus = true;
                } else {
                    let last_word = {
                        let (prefix_before_cursor, _) = split_sql_at_cursor(&tab.sql, tab.cursor_index);
                        prefix_before_cursor
                            .split_whitespace()
                            .last()
                            .unwrap_or("")
                            .trim()
                            .to_string()
                    };
                    let has_any_tab_completion = expand_sql_macro_with_cursor(&last_word).is_some()
                        || tab.ghost_completion.is_some()
                        || !tab.local_suggestions.is_empty();

                    if has_any_tab_completion
                        && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Tab))
                    {
                        if let Some((expanded, cursor_offset)) =
                            expand_sql_macro_with_cursor(&last_word)
                        {
                            let new_cursor = replace_at_cursor(
                                &mut tab.sql,
                                tab.cursor_index,
                                last_word.len(),
                                expanded,
                            );
                            let base_chars = new_cursor.saturating_sub(expanded.chars().count());
                            tab.is_dirty = true;
                            move_cursor_to_char = Some(base_chars + cursor_offset);
                            keep_editor_focus = true;
                        } else if let Some(ghost) = tab.ghost_completion.take() {
                            let new_cursor =
                                replace_at_cursor(&mut tab.sql, tab.cursor_index, 0, &ghost);
                            tab.is_dirty = true;
                            move_cursor_to_char = Some(new_cursor);
                            keep_editor_focus = true;
                        } else if !tab.local_suggestions.is_empty() {
                            let idx = tab
                                .selected_suggestion_idx
                                .min(tab.local_suggestions.len() - 1);
                            selected_suggestion = Some(tab.local_suggestions[idx].clone());
                        }
                    } else {
                        for (idx, item) in tab.local_suggestions.iter().enumerate().take(8) {
                            let num_key = match idx {
                                0 => Key::Num1,
                                1 => Key::Num2,
                                2 => Key::Num3,
                                3 => Key::Num4,
                                4 => Key::Num5,
                                5 => Key::Num6,
                                6 => Key::Num7,
                                7 => Key::Num8,
                                _ => break,
                            };
                            if ui.input_mut(|i| i.consume_key(Modifiers::ALT, num_key)) {
                                selected_suggestion = Some(item.clone());
                                break;
                            }
                        }
                    }
                }

                if let Some(item) = selected_suggestion {
                    let new_cursor = replace_at_cursor(
                        &mut tab.sql,
                        tab.cursor_index,
                        tab.current_token_prefix.len(),
                        &item.insert_text,
                    );
                    tab.local_suggestions.clear();
                    tab.selected_suggestion_idx = 0;
                    tab.ghost_completion = None;
                    tab.is_dirty = true;
                    move_cursor_to_char = Some(new_cursor);
                    keep_editor_focus = true;
                }

                if let Some(target_char_idx) = move_cursor_to_char {
                    if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), editor_id) {
                        let ccursor = egui::text::CCursor::new(target_char_idx);
                        state
                            .cursor
                            .set_char_range(Some(egui::text::CCursorRange::one(ccursor)));
                        state.store(ui.ctx(), editor_id);
                    }
                    tab.cursor_index = target_char_idx;
                }

                let ghost_clone = tab.ghost_completion.clone();
                let cursor_idx_snapshot = tab.cursor_index;
                let mut layouter = |ui: &Ui, buf: &dyn egui::TextBuffer, wrap_width: f32| {
                    let mut job = highlight_sql(
                        buf.as_str(),
                        ghost_clone.as_deref(),
                        Some(cursor_idx_snapshot),
                        14.0,
                    );
                    job.wrap.max_width = wrap_width;
                    ui.fonts_mut(|f| f.layout_job(job))
                };

                let output = egui::ScrollArea::vertical()
                    .id_salt("sql_editor_scroll")
                    .max_height(195.0)
                    .show(ui, |ui| {
                        egui::TextEdit::multiline(&mut tab.sql)
                            .id(editor_id)
                            .font(egui::TextStyle::Monospace)
                            .code_editor()
                            .desired_rows(8)
                            .desired_width(f32::INFINITY)
                            .layouter(&mut layouter)
                            .show(ui)
                    })
                    .inner;

                if keep_editor_focus {
                    output.response.request_focus();
                }

                if let Some(cursor_range) = output.state.cursor.char_range() {
                    let sorted = cursor_range.as_sorted_char_range();
                    tab.cursor_index = cursor_range.primary.index.into();
                    let s_start_raw: usize = sorted.start.into();
                    let s_end_raw: usize = sorted.end.into();
                    if s_start_raw != s_end_raw {
                        let chars: Vec<char> = tab.sql.chars().collect();
                        let s_start = s_start_raw.min(chars.len());
                        let s_end = s_end_raw.min(chars.len());
                        let selected: String = chars[s_start..s_end].iter().collect();
                        tab.selected_sql = if selected.trim().is_empty() {
                            None
                        } else {
                            Some(selected)
                        };
                    } else {
                        tab.selected_sql = None;
                    }
                }

                let editor_resp = output.response;

                if editor_resp.has_focus() {
                    ui.input_mut(|i| {
                        let is_ctrl = i.modifiers.ctrl || i.modifiers.command;
                        let is_shift = i.modifiers.shift;

                        let has_enter_press = i.events.iter().any(|e| {
                            matches!(e, egui::Event::Key { key: Key::Enter, pressed: true, .. })
                        });

                        if is_ctrl && is_shift && has_enter_press {
                            i.events.retain(|e| {
                                !matches!(e, egui::Event::Key { key: Key::Enter, pressed: true, .. })
                            });
                            editor_run_stmt = true;
                        } else if is_ctrl && has_enter_press {
                            i.events.retain(|e| {
                                !matches!(e, egui::Event::Key { key: Key::Enter, pressed: true, .. })
                            });
                            editor_run_query = true;
                        } else if is_shift && i.consume_key(Modifiers::NONE, Key::F5) {
                            editor_run_stmt = true;
                        } else if i.consume_key(Modifiers::NONE, Key::F5) {
                            editor_run_query = true;
                        }
                    });
                }

                if editor_resp.changed() {
                    tab.is_dirty = true;
                    tab.selected_sql = None;
                    tab.last_edit_instant = Some(std::time::Instant::now());
                    let (sql_prefix, _) = split_sql_at_cursor(&tab.sql, tab.cursor_index);
                    let (prefix, suggestions) =
                        compute_local_completions(sql_prefix, schema.as_ref(), 8);
                    tab.current_token_prefix = prefix;
                    tab.local_suggestions = suggestions;
                    tab.selected_suggestion_idx = 0;
                    if enable_ghost {
                        tab.ghost_completion =
                            compute_heuristic_ghost_completion(sql_prefix, schema.as_ref());
                    } else {
                        tab.ghost_completion = None;
                    }
                }

                // Inline Autocomplete Bar (Schema tokens + Ghost completion accept button)
                if !tab.local_suggestions.is_empty() || tab.ghost_completion.is_some() {
                    let mut clicked_new_cursor: Option<usize> = None;
                    ui.horizontal_wrapped(|ui| {
                        ui.label(
                            RichText::new("Suggestions:")
                                .small()
                                .color(Color32::from_rgb(56, 189, 248)),
                        );
                        if let Some(ghost) = tab.ghost_completion.clone() {
                            let preview = ghost.replace('\n', " ↵ ");
                            if ui
                                .small_button(
                                    RichText::new(format!("[Tab] {preview}"))
                                        .color(Color32::from_rgb(125, 211, 252)),
                                )
                                .on_hover_text("Insert ghost completion at cursor (Tab)")
                                .clicked()
                            {
                                let new_cursor =
                                    replace_at_cursor(&mut tab.sql, tab.cursor_index, 0, &ghost);
                                tab.ghost_completion = None;
                                tab.is_dirty = true;
                                clicked_new_cursor = Some(new_cursor);
                            }
                        }

                        let suggestions = tab.local_suggestions.clone();
                        let prefix_len = tab.current_token_prefix.len();
                        let has_ghost = tab.ghost_completion.is_some();
                        let active_idx = tab
                            .selected_suggestion_idx
                            .min(suggestions.len().saturating_sub(1));
                        for (idx, item) in suggestions.into_iter().enumerate() {
                            let is_selected = idx == active_idx;
                            let badge = if is_selected && !has_ghost {
                                "[Tab]".to_string()
                            } else if idx < 8 {
                                format!("[Alt+{}]", idx + 1)
                            } else {
                                String::new()
                            };
                            let label = if badge.is_empty() {
                                format!("[{}] {}", item.kind.badge(), item.label)
                            } else {
                                format!("{badge} [{}] {}", item.kind.badge(), item.label)
                            };
                            let text = if is_selected {
                                RichText::new(label)
                                    .strong()
                                    .color(Color32::from_rgb(250, 204, 21))
                            } else {
                                RichText::new(label)
                            };
                            if ui
                                .small_button(text)
                                .on_hover_text(format!(
                                    "{}\nShortcut: {} (Alt+↑/↓ to select, Esc to dismiss)",
                                    item.detail,
                                    if is_selected && !has_ghost {
                                        "Tab"
                                    } else if idx < 8 {
                                        "Alt+1..8"
                                    } else {
                                        "Click"
                                    }
                                ))
                                .clicked()
                            {
                                let new_cursor = replace_at_cursor(
                                    &mut tab.sql,
                                    tab.cursor_index,
                                    prefix_len,
                                    &item.insert_text,
                                );
                                tab.local_suggestions.clear();
                                tab.selected_suggestion_idx = 0;
                                tab.ghost_completion = None;
                                tab.is_dirty = true;
                                clicked_new_cursor = Some(new_cursor);
                            }
                        }

                        if ui
                            .small_button(
                                RichText::new("[Esc] ✕")
                                    .small()
                                    .color(Color32::from_rgb(148, 163, 184)),
                            )
                            .on_hover_text("Dismiss suggestions (Esc)")
                            .clicked()
                        {
                            tab.local_suggestions.clear();
                            tab.selected_suggestion_idx = 0;
                            tab.ghost_completion = None;
                            editor_resp.request_focus();
                        }
                    });
                    if let Some(target_char_idx) = clicked_new_cursor {
                        if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), editor_id) {
                            let ccursor = egui::text::CCursor::new(target_char_idx);
                            state
                                .cursor
                                .set_char_range(Some(egui::text::CCursorRange::one(ccursor)));
                            state.store(ui.ctx(), editor_id);
                        }
                        tab.cursor_index = target_char_idx;
                        editor_resp.request_focus();
                    }
                }

                if let Some(err) = &tab.last_error {
                    egui::Frame::group(ui.style())
                        .fill(Color32::from_rgb(69, 10, 10))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.colored_label(
                                    Color32::from_rgb(248, 113, 113),
                                    format!("SQL Error: {err}"),
                                );
                                if ui.small_button("Fix Query with AI").clicked() {
                                    request_fix_with_ai = Some(err.clone());
                                }
                            });
                        });
                }
            }

            match trigger_tx_action {
                Some(1) => self.commit_transaction(),
                Some(2) => self.rollback_transaction(),
                Some(3) => self.start_transaction(),
                _ => {}
            }

            if editor_run_stmt {
                self.execute_current_statement();
            } else if editor_run_query {
                self.execute_current_tab_query(None);
            }

            if let Some(err_msg) = request_fix_with_ai {
                self.show_ai_panel = true;
                self.send_chat_prompt(Some(format!(
                    "My SQL query failed with the following MySQL error:\n`{err_msg}`\nPlease fix the query and explain the cause."
                )));
            }

            ui.separator();

            // Bottom Workspace Tabs (Results Grid | ER Relationships | Execution Log)
            ui.horizontal(|ui| {
                ui.selectable_value(
                    &mut self.bottom_tab,
                    BottomTab::ResultsGrid,
                    RichText::new("Results Grid").size(12.5),
                );
                ui.selectable_value(
                    &mut self.bottom_tab,
                    BottomTab::ErRelationships,
                    RichText::new("ER Diagram").size(12.5),
                );
                ui.selectable_value(
                    &mut self.bottom_tab,
                    BottomTab::ExecutionPlanAndLog,
                    RichText::new("Query Stats & DDL").size(12.5),
                );
                if ui
                    .selectable_value(
                        &mut self.bottom_tab,
                        BottomTab::ServerAdmin,
                        RichText::new("Server Monitor").size(12.5),
                    )
                    .clicked()
                    && self.server_processes.is_none() && !self.is_server_admin_loading {
                        self.refresh_server_admin();
                    }
            });
            ui.separator();

            match self.bottom_tab {
                BottomTab::ResultsGrid => self.render_results_grid(ui),
                BottomTab::ErRelationships => self.render_er_relationships(ui),
                BottomTab::ExecutionPlanAndLog => self.render_execution_stats(ui),
                BottomTab::ServerAdmin => self.render_server_admin(ui),
            }
        });
    }
}
