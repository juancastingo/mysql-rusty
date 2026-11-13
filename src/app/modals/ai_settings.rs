use super::super::*;

impl MysqlRustyApp {
    pub(crate) fn render_ai_settings_modal(&mut self, ctx: &egui::Context) {
        if !self.show_ai_settings_modal {
            return;
        }
        let mut open = self.show_ai_settings_modal;
        let mut should_save = false;

        egui::Window::new("AI Autocomplete & Provider Settings")
            .open(&mut open)
            .resizable(false)
            .default_width(480.0)
            .show(ctx, |ui| {
                ui.label("Select AI Provider for Inline Autocomplete & SQL Chat:");
                egui::ComboBox::from_id_salt("ai_provider_combo")
                    .selected_text(self.config.ai.provider.label())
                    .width(340.0)
                    .show_ui(ui, |ui| {
                        let providers = [
                            AiProvider::LocalSchema,
                            AiProvider::Ollama,
                            AiProvider::GitHubCopilot,
                            AiProvider::OpenAiCompatible,
                            AiProvider::Anthropic,
                            AiProvider::Gemini,
                            AiProvider::CliCommand,
                        ];
                        for p in providers {
                            let label = p.label();
                            ui.selectable_value(&mut self.config.ai.provider, p, label);
                        }
                    });

                ui.checkbox(
                    &mut self.config.ai.enable_inline_autocomplete,
                    "Enable Copilot-style inline ghost SQL suggestions",
                );
                ui.separator();

                match self.config.ai.provider {
                    AiProvider::LocalSchema => {
                        ui.colored_label(
                            Color32::from_rgb(52, 211, 153),
                            "⚡ Option 1: Local Schema Native (0ms Latency, 100% Offline)",
                        );
                        ui.label(
                            "Runs instantly in Rust memory without network calls or cloud APIs. \
                            Generates context-aware ghost continuations for SELECT, JOIN, WHERE, \
                            ORDER BY, GROUP BY, INSERT, and UPDATE based on your database tables and foreign keys."
                        );
                        ui.small("Recommended for max typing speed, zero latency, and private offline schemas.");
                    }
                    AiProvider::Ollama => {
                        ui.colored_label(
                            Color32::from_rgb(167, 139, 250),
                            "💻 Option 2: Ollama FIM (Fast Local Code LLM)",
                        );
                        ui.label("Ollama Endpoint URL:");
                        ui.text_edit_singleline(&mut self.config.ai.ollama_endpoint);
                        ui.label("Model Name (e.g. `qwen2.5-coder:1.5b` [recommended], `0.5b`, `7b`):");
                        ui.text_edit_singleline(&mut self.config.ai.ollama_model);
                        ui.checkbox(
                            &mut self.config.ai.ollama_use_fim,
                            "⚡ Enable native FIM (Fill-In-The-Middle) prompt mode for code models",
                        );
                        ui.small("Tip: `qwen2.5-coder:1.5b` with FIM responds in ~80-150ms on local hardware.");
                    }
                    AiProvider::GitHubCopilot => {
                        ui.colored_label(
                            Color32::from_rgb(56, 189, 248),
                            "✨ Option 3: GitHub Copilot (Official Subscription)",
                        );

                        let is_logged_in = !self.config.ai.copilot_github_token.trim().is_empty();
                        if is_logged_in {
                            ui.horizontal(|ui| {
                                ui.colored_label(Color32::from_rgb(34, 197, 94), "✅ Logged In & Ready");
                                if ui.button("Disconnect").clicked() {
                                    self.config.ai.copilot_github_token.clear();
                                    self.config.ai.copilot_session_token.clear();
                                    self.config.ai.copilot_token_expires_at = 0;
                                    self.copilot_auth_status = "Disconnected from GitHub Copilot.".to_string();
                                }
                            });
                            ui.label("Copilot Model Engine:");
                            egui::ComboBox::from_id_salt("copilot_model_combo")
                                .selected_text(&self.config.ai.copilot_model)
                                .show_ui(ui, |ui| {
                                    ui.selectable_value(&mut self.config.ai.copilot_model, "gpt-4o".to_string(), "gpt-4o (Smartest)");
                                    ui.selectable_value(&mut self.config.ai.copilot_model, "gpt-3.5-turbo".to_string(), "gpt-3.5-turbo (Ultra-fast)");
                                    ui.selectable_value(&mut self.config.ai.copilot_model, "claude-3.5-sonnet".to_string(), "claude-3.5-sonnet");
                                });
                        } else {
                            ui.label("Log in using your GitHub account with active Copilot subscription:");
                            if self.is_copilot_authenticating {
                                ui.horizontal(|ui| {
                                    ui.spinner();
                                    ui.label("Connecting to GitHub...");
                                });
                            } else if let Some(dev) = &self.copilot_device_code_data {
                                ui.group(|ui| {
                                    ui.label(RichText::new("Step 1: Open verification page").strong());
                                    if ui.button("🌐 Open https://github.com/login/device in Browser").clicked() {
                                        ui.ctx().open_url(egui::OpenUrl::new_tab(&dev.verification_uri));
                                    }
                                    ui.add_space(4.0);
                                    ui.label(RichText::new("Step 2: Enter this one-time code:").strong());
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            RichText::new(&dev.user_code)
                                                .size(20.0)
                                                .strong()
                                                .color(Color32::from_rgb(251, 191, 36)),
                                        );
                                        if ui.button("📋 Copy Code").clicked() {
                                            ui.ctx().copy_text(dev.user_code.clone());
                                            self.status_banner = "Copied device code to clipboard!".to_string();
                                        }
                                    });
                                    ui.add_space(4.0);
                                    ui.label(RichText::new("Step 3: Click once approved in browser:").strong());
                                    if ui.button("🔄 Check / Confirm Authorization").clicked() {
                                        let device_code = dev.device_code.clone();
                                        let tx = self.tx.clone();
                                        self.is_copilot_authenticating = true;
                                        self.rt.spawn(async move {
                                            let res = crate::ai::poll_copilot_device_token(&device_code).await;
                                            let outcome = match res {
                                                Ok(Ok(token)) => Ok(token),
                                                Ok(Err(err_msg)) => Err(err_msg),
                                                Err(e) => Err(format!("{e:#}")),
                                            };
                                            let _ = tx.send(AsyncEvent::CopilotPollFinished(outcome));
                                        });
                                    }
                                });
                            } else {
                                if ui.button("🔗 Connect with GitHub (Device Code)").clicked() {
                                    self.is_copilot_authenticating = true;
                                    let tx = self.tx.clone();
                                    self.rt.spawn(async move {
                                        let res = crate::ai::request_copilot_device_code()
                                            .await
                                            .map_err(|e| format!("{e:#}"));
                                        let _ = tx.send(AsyncEvent::CopilotDeviceCodeReady(res));
                                    });
                                }
                            }

                            ui.collapsing("Or paste GitHub Token manually", |ui| {
                                ui.add(
                                    egui::TextEdit::singleline(&mut self.config.ai.copilot_github_token)
                                        .password(true)
                                        .hint_text("ghu_... or personal access token"),
                                );
                                ui.small("Accepts personal token or ghu_ OAuth token with Copilot access.");
                            });

                            if !self.copilot_auth_status.is_empty() {
                                ui.colored_label(Color32::from_rgb(234, 179, 8), &self.copilot_auth_status);
                            }
                        }
                    }
                    AiProvider::OpenAiCompatible => {
                        ui.label("Base URL (OpenAI, Groq, OpenRouter, vLLM):");
                        ui.text_edit_singleline(&mut self.config.ai.openai_base_url);
                        ui.label("API Key:");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.config.ai.openai_api_key)
                                .password(true),
                        );
                        ui.label("Model Name:");
                        ui.text_edit_singleline(&mut self.config.ai.openai_model);
                    }
                    AiProvider::Anthropic => {
                        ui.label("Anthropic API Key:");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.config.ai.anthropic_api_key)
                                .password(true),
                        );
                        ui.label("Claude Model:");
                        ui.text_edit_singleline(&mut self.config.ai.anthropic_model);
                    }
                    AiProvider::Gemini => {
                        ui.label("Google Gemini API Key:");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.config.ai.gemini_api_key)
                                .password(true),
                        );
                        ui.label("Gemini Model:");
                        ui.text_edit_singleline(&mut self.config.ai.gemini_model);
                    }
                    AiProvider::CliCommand => {
                        ui.label(
                            "Local CLI Command (e.g. `agy`, `llm`, `ollama run qwen2.5-coder`):",
                        );
                        ui.text_edit_singleline(&mut self.config.ai.cli_command);
                        ui.checkbox(
                            &mut self.config.ai.cli_pass_via_stdin,
                            "Pass prompt via STDIN (uncheck to pass as final CLI argument)",
                        );
                    }
                }

                ui.separator();
                ui.label(RichText::new("Performance & Inline Latency").strong());
                ui.horizontal(|ui| {
                    ui.label("Debounce Delay:");
                    ui.add(
                        egui::Slider::new(&mut self.config.ai.autocomplete_debounce_ms, 50..=600)
                            .suffix(" ms"),
                    );
                });
                ui.small("150–200 ms recommended for fast, natural response while typing.");

                ui.horizontal(|ui| {
                    ui.label("Max Completion Tokens:");
                    ui.add(
                        egui::Slider::new(&mut self.config.ai.autocomplete_max_tokens, 15..=100)
                            .suffix(" tokens"),
                    );
                });
                ui.small("Capping at 25–40 tokens cuts LLM inference time by ~75%.");

                ui.separator();
                ui.label(RichText::new("Query Execution").strong());
                ui.horizontal(|ui| {
                    ui.label("Query Timeout (seconds, 0 = none):");
                    ui.add(
                        egui::DragValue::new(&mut self.config.query.timeout_secs)
                            .range(0..=3600)
                            .speed(1.0),
                    );
                });
                ui.horizontal(|ui| {
                    ui.label("Max Rows per Result:");
                    ui.add(
                        egui::DragValue::new(&mut self.config.query.max_rows)
                            .range(100..=1_000_000)
                            .speed(100.0),
                    );
                });
                ui.checkbox(
                    &mut self.config.ui.vim_mode,
                    "Enable Vim Mode in SQL Editor",
                );

                ui.add_space(6.0);
                ui.label(egui::RichText::new("Appearance").strong());
                ui.horizontal(|ui| {
                    ui.label("Theme:");
                    egui::ComboBox::from_id_salt("settings_theme_combo")
                        .selected_text(self.config.ui.theme.label())
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.config.ui.theme,
                                crate::config::ThemePreference::Dark,
                                "Dark",
                            );
                            ui.selectable_value(
                                &mut self.config.ui.theme,
                                crate::config::ThemePreference::Light,
                                "Light",
                            );
                            ui.selectable_value(
                                &mut self.config.ui.theme,
                                crate::config::ThemePreference::System,
                                "Follow System",
                            );
                        });
                });
                ui.horizontal(|ui| {
                    ui.label("UI Scale:");
                    ui.add(
                        egui::Slider::new(&mut self.config.ui.ui_scale, 0.75..=2.0)
                            .step_by(0.05)
                            .suffix("x"),
                    );
                });
                ui.horizontal(|ui| {
                    ui.label("Editor Font Size:");
                    ui.add(
                        egui::Slider::new(&mut self.config.ui.editor_font_size, 10.0..=28.0)
                            .step_by(1.0)
                            .suffix(" px"),
                    );
                });

                ui.separator();
                if ui.button("Save Configuration").clicked() {
                    should_save = true;
                }
            });

        self.show_ai_settings_modal = open;
        if should_save {
            let _ = self.config.save();
            self.show_ai_settings_modal = false;
            self.status_banner = format!(
                "Saved AI configuration ({})",
                self.config.ai.provider.label()
            );
        }
    }
}
