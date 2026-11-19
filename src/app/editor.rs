use super::*;

impl MysqlRustyApp {
    pub(crate) fn render_central_workspace(&mut self, ui: &mut egui::Ui) {
        if self.active_session.is_none() {
            egui::CentralPanel::default().show(ui, |ui| {
                self.render_connections_dashboard(ui);
            });
            return;
        }

        let trigger_save =
            ui.input_mut(|i| self.keymap.consume(i, crate::shortcuts::Action::SaveTab));
        let trigger_open =
            ui.input_mut(|i| self.keymap.consume(i, crate::shortcuts::Action::OpenFile));
        if trigger_save {
            self.save_active_tab_to_file(false);
        }
        if trigger_open {
            self.open_sql_file_dialog();
        }

        egui::CentralPanel::default().show(ui, |ui| {
            let pal = self.effective_palette(ui.ctx());
            // Query Tabs Bar
            ui.horizontal(|ui| {
                ui.add_space(2.0);
                let mut close_idx = None;
                let mut tab_to_rename: Option<(usize, String)> = None;

                egui::ScrollArea::horizontal()
                    .id_salt("query_tabs_scroll")
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            for idx in 0..self.tabs.len() {
                                let is_active = idx == self.active_tab_idx;
                                let is_renaming = self.renaming_tab_idx == Some(idx);
                                let is_dirty = self.tabs[idx].is_dirty;
                                let title = self.tabs[idx].display_title();

                                let tab_bg = if is_active {
                                    pal.tab_active_bg
                                } else {
                                    pal.tab_inactive_bg
                                };
                                let tab_stroke = if is_active {
                                    Stroke::new(1.0, pal.border_active)
                                } else {
                                    Stroke::new(1.0, pal.border_subtle)
                                };

                                egui::Frame::new()
                                    .fill(tab_bg)
                                    .stroke(tab_stroke)
                                    .corner_radius(egui::CornerRadius {
                                        nw: 6,
                                        ne: 6,
                                        sw: 0,
                                        se: 0,
                                    })
                                    .inner_margin(egui::Margin::symmetric(8, 4))
                                    .show(ui, |ui| {
                                        ui.horizontal(|ui| {
                                            if is_renaming {
                                                let edit_resp = ui.add(
                                                    egui::TextEdit::singleline(
                                                        &mut self.rename_tab_buffer,
                                                    )
                                                    .desired_width(110.0),
                                                );
                                                if edit_resp.lost_focus()
                                                    || ui.input(|i| i.key_pressed(Key::Enter))
                                                {
                                                    tab_to_rename = Some((
                                                        idx,
                                                        self.rename_tab_buffer.clone(),
                                                    ));
                                                }
                                                if ui.input(|i| i.key_pressed(Key::Escape)) {
                                                    self.renaming_tab_idx = None;
                                                }
                                            } else {
                                                if is_dirty {
                                                    ui.label(
                                                        RichText::new("●")
                                                            .size(8.0)
                                                            .color(pal.accent),
                                                    );
                                                }
                                                let label_color = if is_active {
                                                    pal.text_primary
                                                } else {
                                                    pal.muted_text
                                                };
                                                let mut label = RichText::new(&title)
                                                    .size(12.5)
                                                    .color(label_color);
                                                if is_active {
                                                    label = label.strong();
                                                }
                                                let resp = ui.add(
                                                    egui::Label::new(label)
                                                        .sense(egui::Sense::click()),
                                                );
                                                if resp.clicked() {
                                                    self.active_tab_idx = idx;
                                                }
                                                if resp.middle_clicked() && self.tabs.len() > 1 {
                                                    close_idx = Some(idx);
                                                }
                                                if resp.double_clicked() {
                                                    self.renaming_tab_idx = Some(idx);
                                                    self.rename_tab_buffer =
                                                        self.tabs[idx].title.clone();
                                                }
                                                resp.context_menu(|ui| {
                                                    if ui.button("Rename").clicked() {
                                                        self.renaming_tab_idx = Some(idx);
                                                        self.rename_tab_buffer =
                                                            self.tabs[idx].title.clone();
                                                        ui.close();
                                                    }
                                                    if ui.button("Save (Ctrl+S)").clicked() {
                                                        self.active_tab_idx = idx;
                                                        self.save_active_tab_direct();
                                                        ui.close();
                                                    }
                                                    if self.tabs.len() > 1
                                                        && ui.button("Close Tab").clicked()
                                                    {
                                                        close_idx = Some(idx);
                                                        ui.close();
                                                    }
                                                });
                                            }
                                            if ui
                                                .add(
                                                    egui::Button::new(
                                                        RichText::new(if self.tabs[idx].undocked {
                                                            "↙"
                                                        } else {
                                                            "↗"
                                                        })
                                                        .size(11.0)
                                                        .color(pal.muted_text),
                                                    )
                                                    .frame(false),
                                                )
                                                .on_hover_text(if self.tabs[idx].undocked {
                                                    "Redock tab into main window"
                                                } else {
                                                    "Undock tab into separate window"
                                                })
                                                .clicked()
                                            {
                                                self.tabs[idx].undocked = !self.tabs[idx].undocked;
                                            }
                                            if self.tabs.len() > 1
                                                && ui
                                                    .add(
                                                        egui::Button::new(
                                                            RichText::new("×")
                                                                .size(11.5)
                                                                .color(pal.muted_text),
                                                        )
                                                        .frame(false),
                                                    )
                                                    .on_hover_text("Close tab")
                                                    .clicked()
                                            {
                                                close_idx = Some(idx);
                                            }
                                        });
                                    });
                                ui.add_space(2.0);
                            }

                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new("+ New Tab")
                                            .size(12.0)
                                            .color(pal.text_secondary),
                                    )
                                    .fill(pal.tab_inactive_bg)
                                    .stroke(Stroke::new(1.0, pal.border_subtle))
                                    .corner_radius(egui::CornerRadius::same(5)),
                                )
                                .clicked()
                            {
                                let n = self.tabs.len() + 1;
                                self.tabs.push(QueryTab::new(
                                    format!("Query {n}"),
                                    "SELECT * FROM users LIMIT 50;",
                                ));
                                self.active_tab_idx = self.tabs.len() - 1;
                            }
                        });
                    });

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
            let vim_mode_enabled = self.config.ui.vim_mode;
            let keymap = self.keymap.clone();
            let editor_palette = self.effective_palette(ui.ctx());
            let initial_font_size = self.config.ui.editor_font_size.clamp(10.0, 28.0);
            let mut editor_font_size = initial_font_size;
            ui.input_mut(|i| {
                if keymap.consume(i, crate::shortcuts::Action::ZoomIn)
                    || i.consume_key(egui::Modifiers::COMMAND, egui::Key::Plus)
                {
                    editor_font_size = (editor_font_size + 1.0).clamp(10.0, 28.0);
                } else if keymap.consume(i, crate::shortcuts::Action::ZoomOut) {
                    editor_font_size = (editor_font_size - 1.0).clamp(10.0, 28.0);
                } else if keymap.consume(i, crate::shortcuts::Action::ZoomReset) {
                    editor_font_size = 14.0;
                }
            });
            let mut trigger_tx_action: Option<u8> = None;
            let mut editor_anchor_rect: Option<egui::Rect> = None;

            if let Some(tab) = self.tabs.get_mut(self.active_tab_idx) {
                // Quick LIMIT Presets Bar
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new(rust_i18n::t!("editor.limit_label"))
                            .size(12.0)
                            .strong()
                            .color(editor_palette.muted_text),
                    );

                    let active_sql_target = tab.selected_sql.as_deref().unwrap_or(&tab.sql);
                    let current_limit = detect_current_limit(active_sql_target);

                    let presets = [
                        (Some(50), "50"),
                        (Some(100), "100"),
                        (Some(500), "500"),
                        (Some(1000), "1000"),
                        (None, "All"),
                    ];

                    for (preset_val, label) in presets {
                        let is_selected = current_limit == preset_val;
                        let btn = egui::Button::new(
                            RichText::new(label)
                                .size(11.0)
                                .strong()
                                .color(if is_selected {
                                    editor_palette.accent_text
                                } else {
                                    editor_palette.text_secondary
                                }),
                        )
                        .fill(if is_selected {
                            editor_palette.accent_bg
                        } else {
                            editor_palette.card_bg
                        })
                        .stroke(Stroke::new(
                            1.0,
                            if is_selected {
                                editor_palette.border_active
                            } else {
                                editor_palette.card_border
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
                        editor_palette.badge(
                            ui,
                            &format!(
                                "IN TX ({} edit{})",
                                uncommitted_muts,
                                if uncommitted_muts == 1 { "" } else { "s" }
                            ),
                            editor_palette.warning,
                            editor_palette.warning_bg,
                        );
                        let commit_btn = egui::Button::new(
                            RichText::new("Commit")
                                .size(11.0)
                                .strong()
                                .color(editor_palette.success),
                        )
                        .fill(editor_palette.success_bg)
                        .stroke(Stroke::new(1.0, editor_palette.success.gamma_multiply(0.5)))
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
                                .color(editor_palette.danger),
                        )
                        .fill(editor_palette.danger_bg)
                        .stroke(Stroke::new(1.0, editor_palette.danger.gamma_multiply(0.5)))
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
                                .color(editor_palette.text_secondary),
                        )
                        .fill(editor_palette.card_bg)
                        .stroke(Stroke::new(1.0, editor_palette.card_border))
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
                                .color(editor_palette.purple),
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
                let has_error_fix = tab.ai_error_fix.is_some();

                if (has_suggestions_or_ghost || has_error_fix)
                    && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
                {
                    tab.ghost_completion = None;
                    tab.local_suggestions.clear();
                    tab.selected_suggestion_idx = 0;
                    tab.ai_error_fix = None;
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
                    let has_any_tab_completion = tab.ai_error_fix.is_some()
                        || expand_sql_macro_with_cursor(&last_word).is_some()
                        || tab.ghost_completion.is_some()
                        || !tab.local_suggestions.is_empty();

                    if has_any_tab_completion
                        && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Tab))
                    {
                        if let Some(new_cursor) = super::ai_panel::apply_ai_error_fix(tab) {
                            move_cursor_to_char = Some(new_cursor);
                            keep_editor_focus = true;
                        } else if let Some((expanded, cursor_offset)) =
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

                // #18 Find / Replace shortcuts (Ctrl+F / Ctrl+H / F3 / Esc) & #37 NL2SQL (Ctrl+I)
                let mut jump_to_match: Option<usize> = None;
                ui.input_mut(|i| {
                    if keymap.consume(i, crate::shortcuts::Action::Nl2Sql) {
                        tab.nl2sql_open = !tab.nl2sql_open;
                        if tab.nl2sql_open {
                            tab.nl2sql_autofocus = true;
                        }
                    } else if keymap.consume(i, crate::shortcuts::Action::Find) {
                        tab.find_open = true;
                        tab.find_autofocus = true;
                        if let Some(sel) = &tab.selected_sql {
                            tab.find_opts.query = sel.lines().next().unwrap_or(sel).to_string();
                        }
                    } else if keymap.consume(i, crate::shortcuts::Action::Replace) {
                        tab.find_open = true;
                        tab.replace_open = true;
                        tab.find_autofocus = true;
                        if let Some(sel) = &tab.selected_sql {
                            tab.find_opts.query = sel.lines().next().unwrap_or(sel).to_string();
                        }
                    } else if tab.find_open && i.consume_key(egui::Modifiers::NONE, Key::Escape) {
                        tab.find_open = false;
                        tab.replace_open = false;
                        keep_editor_focus = true;
                    } else if tab.find_open
                        && !tab.find_matches.is_empty()
                        && keymap.consume(i, crate::shortcuts::Action::FindPrev)
                    {
                        let len = tab.find_matches.len();
                        tab.find_current = if tab.find_current == 0 {
                            len - 1
                        } else {
                            tab.find_current - 1
                        };
                        jump_to_match = Some(tab.find_current);
                    } else if tab.find_open
                        && !tab.find_matches.is_empty()
                        && keymap.consume(i, crate::shortcuts::Action::FindNext)
                    {
                        tab.find_current = (tab.find_current + 1) % tab.find_matches.len();
                        jump_to_match = Some(tab.find_current);
                    }
                });

                if tab.nl2sql_open {
                    let mut trigger_generate = false;
                    let mut close_nl2sql = false;
                    let mut insert_sql: Option<(String, bool)> = None;

                    egui::Frame::new()
                        .fill(editor_palette.card_bg)
                        .stroke(Stroke::new(1.0, editor_palette.purple))
                        .corner_radius(egui::CornerRadius::same(6))
                        .inner_margin(8.0)
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label(
                                    RichText::new("Prompt to SQL")
                                        .color(editor_palette.purple)
                                        .strong(),
                                );
                                let resp = ui.add(
                                    egui::TextEdit::singleline(&mut tab.nl2sql_prompt)
                                        .hint_text("e.g., Get top 5 customers with highest total orders in the last 30 days")
                                        .desired_width((ui.available_width() - 150.0).max(120.0)),
                                );
                                if tab.nl2sql_autofocus {
                                    resp.request_focus();
                                    tab.nl2sql_autofocus = false;
                                }
                                if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                                    trigger_generate = true;
                                }
                                if ui.input(|i| i.key_pressed(Key::Escape)) {
                                    close_nl2sql = true;
                                }
                                if tab.nl2sql_loading {
                                    ui.spinner();
                                } else if ui
                                    .add_enabled(
                                        !tab.nl2sql_prompt.trim().is_empty(),
                                        editor_palette.primary_button("Generate"),
                                    )
                                    .clicked()
                                {
                                    trigger_generate = true;
                                }
                                if ui.small_button("✕").clicked() {
                                    close_nl2sql = true;
                                }
                            });

                            if let Some(err) = &tab.nl2sql_error {
                                ui.colored_label(editor_palette.danger, err);
                            }

                            if let Some(res) = &tab.nl2sql_result {
                                ui.separator();
                                let job = highlight_sql_with_palette(
                                    &res.sql,
                                    None,
                                    None,
                                    13.0,
                                    &editor_palette,
                                );
                                egui::ScrollArea::vertical()
                                    .id_salt("nl2sql_preview_scroll")
                                    .max_height(120.0)
                                    .show(ui, |ui| {
                                        ui.label(job);
                                    });
                                ui.label(
                                    RichText::new(&res.explanation)
                                        .small()
                                        .color(editor_palette.muted_text),
                                );
                                for bullet in &res.assumptions {
                                    ui.label(
                                        RichText::new(format!("• {bullet}"))
                                            .small()
                                            .color(editor_palette.muted_text),
                                    );
                                }
                                ui.add_space(4.0);
                                ui.horizontal(|ui| {
                                    if ui
                                        .add(editor_palette.primary_button("Insert at Cursor"))
                                        .clicked()
                                    {
                                        insert_sql = Some((res.sql.clone(), false));
                                    }
                                    if ui.button("Insert & Run").clicked() {
                                        insert_sql = Some((res.sql.clone(), true));
                                    }
                                    if ui.button("Regenerate").clicked() {
                                        trigger_generate = true;
                                    }
                                    if ui.button("Discard").clicked() {
                                        close_nl2sql = true;
                                    }
                                });
                            }
                        });

                    if close_nl2sql {
                        tab.nl2sql_open = false;
                        tab.nl2sql_result = None;
                        tab.nl2sql_error = None;
                        keep_editor_focus = true;
                    } else if trigger_generate && !tab.nl2sql_prompt.trim().is_empty() {
                        tab.nl2sql_loading = true;
                        tab.nl2sql_error = None;
                        let prompt = tab.nl2sql_prompt.clone();
                        let schema = self
                            .schemas
                            .get(&self.active_database)
                            .cloned()
                            .unwrap_or_else(|| crate::db::DatabaseSchema {
                                name: self.active_database.clone(),
                                ..Default::default()
                            });
                        let ai_cfg = self.config.ai.clone();
                        let tab_id = tab.id.clone();
                        let tx = self.tx.clone();
                        self.rt.spawn(async move {
                            let res = crate::ai::generate_sql_from_natural_language(
                                &prompt, &schema, &ai_cfg,
                            )
                            .await
                            .map_err(|e| format!("{e:#}"));
                            let _ = tx.send(AsyncEvent::Nl2SqlFinished { tab_id, result: res });
                        });
                    } else if let Some((sql_to_insert, and_run)) = insert_sql {
                        let new_cursor = replace_at_cursor(&mut tab.sql, tab.cursor_index, 0, &sql_to_insert);
                        tab.cursor_index = new_cursor;
                        tab.is_dirty = true;
                        tab.last_edit_instant = Some(std::time::Instant::now());
                        tab.nl2sql_open = false;
                        tab.nl2sql_result = None;
                        if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), editor_id) {
                            let ccursor = egui::text::CCursor::new(new_cursor);
                            state
                                .cursor
                                .set_char_range(Some(egui::text::CCursorRange::one(ccursor)));
                            state.store(ui.ctx(), editor_id);
                        }
                        keep_editor_focus = true;
                        if and_run {
                            editor_run_query = true;
                        }
                    }
                }

                if tab.find_open {
                    let mut recompute = false;
                    let mut do_replace_one = false;
                    let mut do_replace_all = false;
                    let mut close_find = false;

                    egui::Frame::new()
                        .fill(editor_palette.card_bg)
                        .stroke(Stroke::new(1.0, editor_palette.card_border))
                        .corner_radius(egui::CornerRadius::same(6))
                        .inner_margin(6.0)
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                let border_color = if tab.find_error.is_some() {
                                    editor_palette.danger
                                } else {
                                    editor_palette.card_border
                                };
                                let edit_frame = egui::Frame::NONE
                                    .stroke(Stroke::new(1.0, border_color))
                                    .corner_radius(egui::CornerRadius::same(4));
                                let resp = edit_frame
                                    .show(ui, |ui| {
                                        ui.add(
                                            egui::TextEdit::singleline(&mut tab.find_opts.query)
                                                .hint_text("Find...")
                                                .desired_width(220.0),
                                        )
                                    })
                                    .inner;
                                if tab.find_autofocus {
                                    resp.request_focus();
                                    tab.find_autofocus = false;
                                }
                                if resp.changed() {
                                    recompute = true;
                                    tab.find_current = 0;
                                }
                                if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                                    if !tab.find_matches.is_empty() {
                                        if ui.input(|i| i.modifiers.shift) {
                                            let len = tab.find_matches.len();
                                            tab.find_current = if tab.find_current == 0 {
                                                len - 1
                                            } else {
                                                tab.find_current - 1
                                            };
                                        } else {
                                            tab.find_current =
                                                (tab.find_current + 1) % tab.find_matches.len();
                                        }
                                        jump_to_match = Some(tab.find_current);
                                    }
                                    resp.request_focus();
                                }

                                if ui
                                    .selectable_label(tab.find_opts.regex, ".*")
                                    .on_hover_text("Regex")
                                    .clicked()
                                {
                                    tab.find_opts.regex = !tab.find_opts.regex;
                                    recompute = true;
                                }
                                if ui
                                    .selectable_label(tab.find_opts.case_sensitive, "Aa")
                                    .on_hover_text("Match case")
                                    .clicked()
                                {
                                    tab.find_opts.case_sensitive = !tab.find_opts.case_sensitive;
                                    recompute = true;
                                }
                                if ui
                                    .selectable_label(tab.find_opts.whole_word, "ab")
                                    .on_hover_text("Whole word")
                                    .clicked()
                                {
                                    tab.find_opts.whole_word = !tab.find_opts.whole_word;
                                    recompute = true;
                                }

                                let count_label = if tab.find_matches.is_empty() {
                                    "0 of 0".to_string()
                                } else {
                                    format!(
                                        "{} of {}",
                                        tab.find_current + 1,
                                        tab.find_matches.len()
                                    )
                                };
                                ui.label(RichText::new(count_label).small().weak());

                                if ui
                                    .add_enabled(
                                        !tab.find_matches.is_empty(),
                                        egui::Button::new("↑").small(),
                                    )
                                    .clicked()
                                {
                                    let len = tab.find_matches.len();
                                    tab.find_current = if tab.find_current == 0 {
                                        len - 1
                                    } else {
                                        tab.find_current - 1
                                    };
                                    jump_to_match = Some(tab.find_current);
                                }
                                if ui
                                    .add_enabled(
                                        !tab.find_matches.is_empty(),
                                        egui::Button::new("↓").small(),
                                    )
                                    .clicked()
                                {
                                    tab.find_current =
                                        (tab.find_current + 1) % tab.find_matches.len();
                                    jump_to_match = Some(tab.find_current);
                                }
                                if ui
                                    .selectable_label(tab.replace_open, "Replace")
                                    .clicked()
                                {
                                    tab.replace_open = !tab.replace_open;
                                }
                                if ui.small_button("✕").clicked() {
                                    close_find = true;
                                }
                            });

                            if tab.replace_open {
                                ui.horizontal(|ui| {
                                    ui.add(
                                        egui::TextEdit::singleline(&mut tab.replace_text)
                                            .hint_text("Replace with...")
                                            .desired_width(220.0),
                                    );
                                    if ui
                                        .add_enabled(
                                            !tab.find_matches.is_empty(),
                                            egui::Button::new("Replace").small(),
                                        )
                                        .clicked()
                                    {
                                        do_replace_one = true;
                                    }
                                    if ui
                                        .add_enabled(
                                            !tab.find_matches.is_empty(),
                                            egui::Button::new("Replace all").small(),
                                        )
                                        .clicked()
                                    {
                                        do_replace_all = true;
                                    }
                                });
                            }
                            if let Some(err) = &tab.find_error {
                                ui.colored_label(Color32::from_rgb(248, 113, 113), err);
                            }
                        });

                    if close_find {
                        tab.find_open = false;
                        tab.replace_open = false;
                        keep_editor_focus = true;
                    } else {
                        if do_replace_one
                            && let Some(m) = tab.find_matches.get(tab.find_current).cloned()
                        {
                            tab.sql = crate::find::replace_one(
                                &tab.sql,
                                m,
                                &tab.replace_text,
                                &tab.find_opts,
                            );
                            tab.is_dirty = true;
                            tab.last_edit_instant = Some(std::time::Instant::now());
                            recompute = true;
                        } else if do_replace_all {
                            let (new_sql, count) = crate::find::replace_all(
                                &tab.sql,
                                &tab.find_opts,
                                &tab.replace_text,
                            );
                            if count > 0 {
                                tab.sql = new_sql;
                                tab.is_dirty = true;
                                tab.last_edit_instant = Some(std::time::Instant::now());
                            }
                            recompute = true;
                        }

                        if recompute || tab.last_edit_instant.is_some() {
                            match crate::find::find_matches(&tab.sql, &tab.find_opts) {
                                Ok(m) => {
                                    tab.find_matches = m;
                                    tab.find_error = None;
                                    if tab.find_current >= tab.find_matches.len() {
                                        tab.find_current = 0;
                                    }
                                }
                                Err(e) => {
                                    tab.find_matches.clear();
                                    tab.find_current = 0;
                                    tab.find_error = Some(e);
                                }
                            }
                        }
                    }
                }

                if let Some(idx) = jump_to_match
                    && let Some(range) = tab.find_matches.get(idx)
                {
                    let start_c = crate::syntax::byte_idx_to_char_idx(&tab.sql, range.start);
                    let end_c = crate::syntax::byte_idx_to_char_idx(&tab.sql, range.end);
                    if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), editor_id) {
                        state.cursor.set_char_range(Some(egui::text::CCursorRange::two(
                            egui::text::CCursor::new(start_c),
                            egui::text::CCursor::new(end_c),
                        )));
                        state.store(ui.ctx(), editor_id);
                    }
                    tab.cursor_index = end_c;
                }

                let ghost_clone = tab.ghost_completion.clone();
                let ai_error_fix_clone = tab.ai_error_fix.clone();
                let cursor_idx_snapshot = tab.cursor_index;
                let find_ranges_clone = if tab.find_open {
                    tab.find_matches.clone()
                } else {
                    Vec::new()
                };
                let find_current_clone = if tab.find_open && !tab.find_matches.is_empty() {
                    Some(tab.find_current)
                } else {
                    None
                };
                let mut layouter = |ui: &Ui, buf: &dyn egui::TextBuffer, wrap_width: f32| {
                    let ghost_for_syntax = if ai_error_fix_clone.is_some() {
                        None
                    } else {
                        ghost_clone.as_deref()
                    };
                    let mut job = crate::syntax::highlight_sql_with_palette(
                        buf.as_str(),
                        ghost_for_syntax,
                        Some(cursor_idx_snapshot),
                        editor_font_size,
                        &editor_palette,
                    );
                    if !find_ranges_clone.is_empty() {
                        crate::syntax::apply_background_ranges(
                            &mut job,
                            &find_ranges_clone,
                            find_current_clone,
                        );
                    }
                    if let Some(fix) = &ai_error_fix_clone {
                        crate::syntax::apply_error_fix_overlay(
                            &mut job,
                            &fix.strike_ranges,
                            &fix.fixed_sql,
                            &fix.insert_ranges,
                            editor_font_size,
                        );
                    }
                    job.wrap.max_width = wrap_width;
                    ui.fonts_mut(|f| f.layout_job(job))
                };

                if vim_mode_enabled
                    && ui.memory(|m| m.has_focus(editor_id))
                    && !tab.find_open
                    && !tab.nl2sql_open
                {
                    let mut vim_changed = false;
                    let mut cur = tab.cursor_index;
                    ui.input_mut(|i| {
                        let mut remaining = Vec::with_capacity(i.events.len());
                        for ev in i.events.drain(..) {
                            let is_insert = tab.vim_state.mode == crate::vim::Mode::Insert;
                            let vim_key = match &ev {
                                egui::Event::Text(t) if !is_insert => {
                                    t.chars().next().map(crate::vim::VimKey::Char)
                                }
                                egui::Event::Key {
                                    key,
                                    pressed: true,
                                    modifiers,
                                    ..
                                } => match key {
                                    egui::Key::Escape => Some(crate::vim::VimKey::Esc),
                                    egui::Key::Enter if !is_insert && !modifiers.ctrl && !modifiers.command => {
                                        Some(crate::vim::VimKey::Enter)
                                    }
                                    egui::Key::Backspace if !is_insert => {
                                        Some(crate::vim::VimKey::Backspace)
                                    }
                                    egui::Key::Delete if !is_insert => {
                                        Some(crate::vim::VimKey::Delete)
                                    }
                                    egui::Key::ArrowLeft if !is_insert => {
                                        Some(crate::vim::VimKey::Left)
                                    }
                                    egui::Key::ArrowRight if !is_insert => {
                                        Some(crate::vim::VimKey::Right)
                                    }
                                    egui::Key::ArrowUp if !is_insert => {
                                        Some(crate::vim::VimKey::Up)
                                    }
                                    egui::Key::ArrowDown if !is_insert => {
                                        Some(crate::vim::VimKey::Down)
                                    }
                                    egui::Key::D if !is_insert && modifiers.ctrl => {
                                        Some(crate::vim::VimKey::CtrlD)
                                    }
                                    egui::Key::U if !is_insert && modifiers.ctrl => {
                                        Some(crate::vim::VimKey::CtrlU)
                                    }
                                    egui::Key::R if !is_insert && modifiers.ctrl => {
                                        Some(crate::vim::VimKey::CtrlR)
                                    }
                                    _ => None,
                                },
                                _ => None,
                            };

                            if let Some(vk) = vim_key
                                && tab.vim_state.handle_key(vk, &mut tab.sql, &mut cur)
                            {
                                vim_changed = true;
                                continue;
                            }
                            if !is_insert && matches!(ev, egui::Event::Text(_)) {
                                continue;
                            }
                            remaining.push(ev);
                        }
                        i.events = remaining;
                    });
                    if vim_changed {
                        tab.cursor_index = cur;
                        if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), editor_id) {
                            let range = if let Some((s, e)) =
                                tab.vim_state.visual_selection_range(&tab.sql, cur)
                            {
                                egui::text::CCursorRange::two(
                                    egui::text::CCursor::new(s),
                                    egui::text::CCursor::new(e),
                                )
                            } else {
                                egui::text::CCursorRange::one(egui::text::CCursor::new(cur))
                            };
                            state.cursor.set_char_range(Some(range));
                            state.store(ui.ctx(), editor_id);
                        }
                    }
                }

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
                editor_anchor_rect = Some(editor_resp.rect);

                if editor_resp.has_focus() {
                    ui.input_mut(|i| {
                        let is_shift = i.modifiers.shift;
                        if keymap.consume(i, crate::shortcuts::Action::RunStatement)
                            || (is_shift && i.consume_key(Modifiers::NONE, Key::F5))
                        {
                            editor_run_stmt = true;
                        } else if keymap.consume(i, crate::shortcuts::Action::RunQuery)
                            || i.consume_key(Modifiers::NONE, Key::F5)
                        {
                            editor_run_query = true;
                        }
                    });
                }

                if editor_resp.changed() {
                    tab.is_dirty = true;
                    tab.selected_sql = None;
                    tab.ai_error_fix = None;
                    tab.ai_error_fix_loading = false;
                    tab.ai_error_fix_req_id += 1;
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

                // Compact Editor Status Strip (Ln/Col, lines, selection, font size)
                {
                    let (prefix_before_cursor, _) =
                        split_sql_at_cursor(&tab.sql, tab.cursor_index);
                    let cur_line = prefix_before_cursor.chars().filter(|&c| c == '\n').count() + 1;
                    let cur_col = prefix_before_cursor
                        .rsplit('\n')
                        .next()
                        .map(|s| s.chars().count() + 1)
                        .unwrap_or(1);
                    let total_lines = tab.sql.lines().count().max(1);
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!(
                                "Ln {cur_line}, Col {cur_col}  •  {total_lines} lines"
                            ))
                            .size(11.0)
                            .color(editor_palette.muted_text),
                        );
                        if let Some(sel) = &tab.selected_sql {
                            ui.label(
                                RichText::new(format!("•  {} chars selected", sel.chars().count()))
                                    .size(11.0)
                                    .color(editor_palette.accent_text),
                            );
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{:.0}px", editor_font_size))
                                    .size(11.0)
                                    .color(editor_palette.muted_text),
                            );
                        });
                    });
                }

                // Inline Autocomplete Bar (Schema tokens + Ghost completion accept button)
                if !tab.local_suggestions.is_empty() || tab.ghost_completion.is_some() {
                    let mut clicked_new_cursor: Option<usize> = None;
                    ui.horizontal_wrapped(|ui| {
                        ui.label(
                            RichText::new("Suggestions:")
                                .small()
                                .color(editor_palette.accent_text),
                        );
                        if let Some(ghost) = tab.ghost_completion.clone() {
                            let preview = ghost.replace('\n', " ↵ ");
                            if ui
                                .small_button(
                                    RichText::new(format!("[Tab] {preview}"))
                                        .color(editor_palette.accent_text),
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
                                    .color(editor_palette.warning)
                            } else {
                                RichText::new(label).color(editor_palette.text_primary)
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
                                    .color(editor_palette.muted_text),
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
                    let mut apply_fix_clicked = false;
                    let mut dismiss_fix_clicked = false;
                    editor_palette
                        .banner_frame(editor_palette.danger_bg, editor_palette.danger)
                        .show(ui, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                ui.colored_label(
                                    editor_palette.danger,
                                    format!("SQL Error: {err}"),
                                );
                                if tab.ai_error_fix_loading {
                                    ui.spinner();
                                    ui.colored_label(
                                        editor_palette.accent_text,
                                        "AI suggesting fix...",
                                    );
                                } else if let Some(fix) = &tab.ai_error_fix {
                                    if !fix.explanation.is_empty() {
                                        ui.colored_label(
                                            editor_palette.success,
                                            format!("Fix: {}", fix.explanation),
                                        );
                                    }
                                    if ui
                                        .small_button("Apply Fix (Tab)")
                                        .on_hover_text("Apply AI correction to editor (Tab)")
                                        .clicked()
                                    {
                                        apply_fix_clicked = true;
                                    }
                                    if ui
                                        .small_button("Dismiss (Esc)")
                                        .on_hover_text("Dismiss AI suggestion (Esc)")
                                        .clicked()
                                    {
                                        dismiss_fix_clicked = true;
                                    }
                                }
                                if ui.small_button("Fix Query with AI").clicked() {
                                    request_fix_with_ai = Some(err.clone());
                                }
                            });
                        });
                    if apply_fix_clicked {
                        if let Some(new_cursor) = super::ai_panel::apply_ai_error_fix(tab) {
                            if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), editor_id) {
                                let ccursor = egui::text::CCursor::new(new_cursor);
                                state
                                    .cursor
                                    .set_char_range(Some(egui::text::CCursorRange::one(ccursor)));
                                state.store(ui.ctx(), editor_id);
                            }
                            tab.cursor_index = new_cursor;
                        }
                    } else if dismiss_fix_clicked {
                        tab.ai_error_fix = None;
                    }
                }
            }

            match trigger_tx_action {
                Some(1) => self.commit_transaction(),
                Some(2) => self.rollback_transaction(),
                Some(3) => self.start_transaction(),
                _ => {}
            }

            if let Some(rect) = editor_anchor_rect {
                self.tour
                    .set_anchor(crate::tour::TourAnchor::SqlEditor, rect);
            }

            if (editor_font_size - initial_font_size).abs() > 0.01 {
                self.config.ui.editor_font_size = editor_font_size;
                let _ = self.config.save();
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
                if ui
                    .small_button(if self.er_undocked { "↙" } else { "↗" })
                    .on_hover_text(if self.er_undocked {
                        "Redock ER Diagram"
                    } else {
                        "Undock ER Diagram into separate window"
                    })
                    .clicked()
                {
                    self.er_undocked = !self.er_undocked;
                }
                ui.selectable_value(
                    &mut self.bottom_tab,
                    BottomTab::ExecutionPlanAndLog,
                    RichText::new("Query Stats & DDL").size(12.5),
                );
                let is_sqlite = self
                    .active_session
                    .as_ref()
                    .is_some_and(|s| s.dialect() == crate::dialect::Dialect::Sqlite);
                if !is_sqlite
                    && ui
                        .selectable_value(
                            &mut self.bottom_tab,
                            BottomTab::ServerAdmin,
                            RichText::new("Server Monitor").size(12.5),
                        )
                        .clicked()
                    && self.server_processes.is_none()
                    && !self.is_server_admin_loading
                {
                    self.refresh_server_admin();
                }
            });
            ui.separator();
            self.tour.set_anchor(
                crate::tour::TourAnchor::ResultsGrid,
                ui.available_rect_before_wrap(),
            );

            match self.bottom_tab {
                BottomTab::ResultsGrid => self.render_results_grid(ui),
                BottomTab::ErRelationships => self.render_er_relationships(ui),
                BottomTab::ExecutionPlanAndLog => self.render_execution_stats(ui),
                BottomTab::ServerAdmin => self.render_server_admin(ui),
            }
        });
    }

    pub(crate) fn render_detached_viewports(&mut self, ctx: &egui::Context) {
        let palette = self.effective_palette(ctx);
        let editor_font_size = self.config.ui.editor_font_size.clamp(10.0, 28.0);
        let undocked_indices: Vec<usize> = self
            .tabs
            .iter()
            .enumerate()
            .filter_map(|(i, t)| t.undocked.then_some(i))
            .collect();

        for idx in undocked_indices {
            let Some(tab) = self.tabs.get(idx) else {
                continue;
            };
            let vid = tab.viewport_id();
            let title = format!("{} — MySQL-Rusty", tab.display_title());
            let mut redock = false;
            let mut run_now = false;

            ctx.show_viewport_immediate(
                vid,
                egui::ViewportBuilder::default()
                    .with_title(title)
                    .with_inner_size([920.0, 620.0]),
                |ui, _class| {
                    if ui.ctx().input(|i| i.viewport().close_requested()) {
                        redock = true;
                    }
                    egui::CentralPanel::default().show(ui, |ui| {
                        let Some(tab) = self.tabs.get_mut(idx) else {
                            return;
                        };
                        ui.horizontal(|ui| {
                            if ui.button("▶ Run").clicked() {
                                run_now = true;
                            }
                            if ui.button("↙ Redock").clicked() {
                                redock = true;
                            }
                            if tab.is_running {
                                ui.spinner();
                                ui.label("Running...");
                            }
                            if let Some(res) = &tab.last_result {
                                ui.label(
                                    RichText::new(format!(
                                        "{} rows ({} ms)",
                                        res.rows.len(),
                                        res.duration_ms
                                    ))
                                    .small()
                                    .weak(),
                                );
                            }
                        });
                        ui.separator();

                        let mut layouter =
                            |ui: &Ui, buf: &dyn egui::TextBuffer, wrap_width: f32| {
                                let mut job = crate::syntax::highlight_sql_with_palette(
                                    buf.as_str(),
                                    None,
                                    None,
                                    editor_font_size,
                                    &palette,
                                );
                                job.wrap.max_width = wrap_width;
                                ui.fonts_mut(|f| f.layout_job(job))
                            };

                        egui::ScrollArea::vertical()
                            .id_salt(("detached_sql_scroll", &tab.id))
                            .max_height(240.0)
                            .show(ui, |ui| {
                                let resp = ui.add(
                                    egui::TextEdit::multiline(&mut tab.sql)
                                        .code_editor()
                                        .desired_width(f32::INFINITY)
                                        .desired_rows(10)
                                        .layouter(&mut layouter),
                                );
                                if resp.changed() {
                                    tab.is_dirty = true;
                                }
                            });

                        ui.separator();
                        if let Some(err) = &tab.last_error {
                            ui.colored_label(Color32::from_rgb(248, 113, 113), err);
                        } else if let Some(res) = &tab.last_result {
                            egui::ScrollArea::both()
                                .id_salt(("detached_res_scroll", &tab.id))
                                .show(ui, |ui| {
                                    egui::Grid::new(("detached_res_grid", &tab.id))
                                        .striped(true)
                                        .show(ui, |ui| {
                                            for col in &res.columns {
                                                ui.label(RichText::new(col).strong());
                                            }
                                            ui.end_row();
                                            for row in res.rows.iter().take(200) {
                                                for cell in row {
                                                    ui.label(cell.display_text());
                                                }
                                                ui.end_row();
                                            }
                                        });
                                });
                        } else {
                            ui.label(RichText::new("No results yet.").weak());
                        }
                    });
                },
            );

            if redock && let Some(tab) = self.tabs.get_mut(idx) {
                tab.undocked = false;
            }
            if run_now {
                self.active_tab_idx = idx;
                self.execute_current_tab_query(None);
            }
        }

        if self.er_undocked {
            let mut redock_er = false;
            ctx.show_viewport_immediate(
                Self::er_viewport_id(),
                egui::ViewportBuilder::default()
                    .with_title("ER Diagram — MySQL-Rusty")
                    .with_inner_size([1024.0, 720.0]),
                |ui, _class| {
                    if ui.ctx().input(|i| i.viewport().close_requested()) {
                        redock_er = true;
                    }
                    egui::CentralPanel::default().show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.heading("ER Relationships");
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui.button("↙ Redock").clicked() {
                                        redock_er = true;
                                    }
                                },
                            );
                        });
                        ui.separator();
                        self.render_er_relationships_inner(ui);
                    });
                },
            );
            if redock_er {
                self.er_undocked = false;
            }
        }
    }
}
