use super::super::*;

impl MysqlRustyApp {
    pub(crate) fn render_connection_modal(&mut self, ctx: &egui::Context) {
        if !self.show_connection_modal {
            return;
        }
        let mut open = self.show_connection_modal;
        let mut save_and_connect = false;
        let mut save_only = false;

        egui::Window::new("MySQL / MariaDB Connection Profile")
            .open(&mut open)
            .resizable(false)
            .default_width(460.0)
            .show(ctx, |ui| {
                egui::Grid::new("conn_form_grid")
                    .num_columns(2)
                    .spacing([12.0, 8.0])
                    .show(ui, |ui| {
                        ui.label("Connection Name:");
                        ui.text_edit_singleline(&mut self.editing_connection.name);
                        ui.end_row();

                        ui.label("Host / IP:");
                        ui.text_edit_singleline(&mut self.editing_connection.host);
                        ui.end_row();

                        ui.label("Port:");
                        ui.add(egui::DragValue::new(&mut self.editing_connection.port));
                        ui.end_row();

                        ui.label("Username:");
                        ui.text_edit_singleline(&mut self.editing_connection.user);
                        ui.end_row();

                        ui.label("Password:");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.editing_connection.password)
                                .password(true),
                        );
                        ui.end_row();

                        ui.label("Default Database:");
                        ui.text_edit_singleline(&mut self.editing_connection.default_database);
                        ui.end_row();

                        ui.label("Environment Badge:");
                        egui::ComboBox::from_id_salt("env_badge_modal_combo")
                            .selected_text(self.editing_connection.environment.label())
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut self.editing_connection.environment,
                                    Environment::Development,
                                    "Development (Dev)",
                                );
                                ui.selectable_value(
                                    &mut self.editing_connection.environment,
                                    Environment::Staging,
                                    "Staging",
                                );
                                ui.selectable_value(
                                    &mut self.editing_connection.environment,
                                    Environment::Production,
                                    "Production (Strict Guard)",
                                );
                            });
                        ui.end_row();

                        ui.label("Security Mode:");
                        ui.checkbox(
                            &mut self.editing_connection.read_only,
                            "Read-Only (Prevent write queries)",
                        )
                        .on_hover_text("When checked, blocks all INSERT, UPDATE, DELETE, DROP, and ALTER queries.");
                        ui.end_row();

                        ui.label("SSL Mode:");
                        egui::ComboBox::from_id_salt("ssl_mode_combo")
                            .selected_text(format!("{:?}", self.editing_connection.ssl_mode))
                            .show_ui(ui, |ui| {
                                ui.selectable_value(
                                    &mut self.editing_connection.ssl_mode,
                                    SslMode::Disabled,
                                    "Disabled",
                                );
                                ui.selectable_value(
                                    &mut self.editing_connection.ssl_mode,
                                    SslMode::Preferred,
                                    "Preferred",
                                );
                                ui.selectable_value(
                                    &mut self.editing_connection.ssl_mode,
                                    SslMode::Required,
                                    "Required",
                                );
                            });
                        ui.end_row();

                        if self.editing_connection.ssl_mode != SslMode::Disabled {
                            ui.label("Trust Certificates:");
                            ui.checkbox(
                                &mut self.editing_connection.ssl_accept_invalid_certs,
                                "Allow Self-Signed / Unknown CA",
                            )
                            .on_hover_text("Accept self-signed or internal CA SSL certificates (prevents UnknownIssuer error).");
                            ui.end_row();
                        }
                    });

                ui.separator();
                ui.checkbox(
                    &mut self.editing_connection.ssh.enabled,
                    "Enable SSH Tunnel Port Forwarding (`ssh -N -L`)",
                );
                if self.editing_connection.ssh.enabled {
                    egui::Grid::new("ssh_form_grid")
                        .num_columns(2)
                        .spacing([12.0, 6.0])
                        .show(ui, |ui| {
                            ui.label("SSH Host:");
                            ui.text_edit_singleline(&mut self.editing_connection.ssh.ssh_host);
                            ui.end_row();

                            ui.label("SSH Port:");
                            ui.add(egui::DragValue::new(
                                &mut self.editing_connection.ssh.ssh_port,
                            ));
                            ui.end_row();

                            ui.label("SSH User:");
                            ui.text_edit_singleline(&mut self.editing_connection.ssh.ssh_user);
                            ui.end_row();

                            ui.label("SSH Private Key:");
                            ui.horizontal(|ui| {
                                ui.text_edit_singleline(&mut self.editing_connection.ssh.ssh_key_path);
                                if ui.small_button("Browse...").on_hover_text("Browse SSH private key file").clicked() {
                                    let default_dir = dirs::home_dir().map(|h| h.join(".ssh")).unwrap_or_else(|| std::path::PathBuf::from("/"));
                                    if let Some(path) = rfd::FileDialog::new().set_directory(default_dir).pick_file() {
                                        self.editing_connection.ssh.ssh_key_path = path.to_string_lossy().to_string();
                                    }
                                }
                            });
                            ui.end_row();
                        });
                }

                // Test Connection Result feedback
                if self.testing_connection {
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(
                            RichText::new("Testing connection (handshake, SSH tunnel & auth)...")
                                .color(Color32::from_rgb(56, 189, 248)),
                        );
                    });
                } else if let Some(res) = &self.test_connection_result {
                    ui.add_space(8.0);
                    match res {
                        Ok(msg) => {
                            egui::Frame::group(ui.style())
                                .fill(Color32::from_rgb(6, 78, 59))
                                .stroke(Stroke::new(1.0, Color32::from_rgb(16, 185, 129)))
                                .inner_margin(8.0)
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.label(
                                            RichText::new("[OK]")
                                                .color(Color32::from_rgb(16, 185, 129))
                                                .strong(),
                                        );
                                        ui.label(
                                            RichText::new(msg)
                                                .color(Color32::from_rgb(209, 250, 229))
                                                .small(),
                                        );
                                    });
                                });
                        }
                        Err(err) => {
                            egui::Frame::group(ui.style())
                                .fill(Color32::from_rgb(69, 10, 10))
                                .stroke(Stroke::new(1.0, Color32::from_rgb(239, 68, 68)))
                                .inner_margin(8.0)
                                .show(ui, |ui| {
                                    ui.vertical(|ui| {
                                        ui.label(
                                            RichText::new("[FAIL] Connection Failed")
                                                .color(Color32::from_rgb(239, 68, 68))
                                                .strong(),
                                        );
                                        ui.label(
                                            RichText::new(err)
                                                .color(Color32::from_rgb(254, 202, 202))
                                                .small(),
                                        );
                                        if err.contains("Permission denied") || err.contains("SSH") {
                                            ui.add_space(2.0);
                                            ui.label(
                                                RichText::new("Tip: For AWS EC2 / Amazon Linux bastion, SSH User is usually 'ec2-user' (not 'ubuntu'). Verify key file path and permissions.")
                                                    .color(Color32::from_rgb(253, 224, 71))
                                                    .small(),
                                            );
                                        }
                                    });
                                });
                        }
                    }
                }

                ui.separator();
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(!self.testing_connection, egui::Button::new("Test Connection"))
                        .on_hover_text("Test network, SSH tunnel, credentials and MySQL ping")
                        .clicked()
                    {
                        self.testing_connection = true;
                        self.test_connection_result = None;
                        let mut test_prof = self.editing_connection.clone();
                        if let Some((h, p)) = test_prof
                            .ssh
                            .ssh_host
                            .trim()
                            .split_once(':')
                            .map(|(h, p)| (h.trim().to_string(), p.trim().to_string()))
                        {
                            test_prof.ssh.ssh_host = h;
                            if let Ok(port) = p.parse::<u16>() {
                                test_prof.ssh.ssh_port = port;
                            }
                        }
                        let tx = self.tx.clone();
                        self.rt.spawn(async move {
                            let res = match ActiveSession::connect(test_prof).await {
                                Ok(session) => {
                                    match session.execute_query("", "SELECT VERSION(), CURRENT_USER();", None).await {
                                        Ok(qr) => {
                                            let v = qr.rows.first().and_then(|r| r.first()).map(|val| val.display_text()).unwrap_or_else(|| "Unknown".to_string());
                                            let u = qr.rows.first().and_then(|r| r.get(1)).map(|val| val.display_text()).unwrap_or_else(|| "Unknown".to_string());
                                            Ok(format!("Connection successful!\n• MySQL Version: {v}\n• Authenticated User: {u}"))
                                        }
                                        Err(_) => Ok("Connection successful (handshake & auth succeeded)!".to_string()),
                                    }
                                }
                                Err(e) => Err(format!("{e:#}")),
                            };
                            let _ = tx.send(AsyncEvent::TestConnectionResult(res));
                        });
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .button(
                                RichText::new("Save & Connect")
                                    .color(Color32::from_rgb(56, 189, 248))
                                    .strong(),
                            )
                            .clicked()
                        {
                            save_and_connect = true;
                        }
                        if ui.button("Save").clicked() {
                            save_only = true;
                        }
                    });
                });
            });

        self.show_connection_modal = open;
        if !open {
            self.test_connection_result = None;
            self.testing_connection = false;
        }

        if save_and_connect || save_only {
            let mut prof = self.editing_connection.clone();
            if let Some((h, p)) = prof
                .ssh
                .ssh_host
                .trim()
                .split_once(':')
                .map(|(h, p)| (h.trim().to_string(), p.trim().to_string()))
            {
                prof.ssh.ssh_host = h;
                if let Ok(port) = p.parse::<u16>() {
                    prof.ssh.ssh_port = port;
                }
            }

            if let Some(existing) = self.config.connections.iter_mut().find(|c| c.id == prof.id) {
                *existing = prof.clone();
            } else {
                self.config.connections.push(prof.clone());
            }
            let _ = self.config.save();
            self.show_connection_modal = false;
            self.test_connection_result = None;

            if save_and_connect {
                self.connect_to_profile(prof);
            }
        }
    }

    pub(crate) fn render_delete_connection_confirm_modal(&mut self, ctx: &egui::Context) {
        let Some((conn_id, conn_name)) = self.pending_delete_connection.clone() else {
            return;
        };

        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.pending_delete_connection = None;
            return;
        }

        let mut open = true;
        let mut confirmed = false;
        let mut cancelled = false;

        egui::Window::new("⚠ Move Connection to Recycle Bin")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .default_width(390.0)
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(format!(
                        "Move connection \"{conn_name}\" to the Recycle Bin?"
                    ))
                    .strong(),
                );
                ui.add_space(4.0);
                ui.label(
                    RichText::new("You can restore it later from the Recycle Bin.")
                        .small()
                        .color(Color32::from_rgb(148, 163, 184)),
                );
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    let del_btn = egui::Button::new(
                        RichText::new("Move to Trash")
                            .strong()
                            .color(Color32::WHITE),
                    )
                    .fill(Color32::from_rgb(185, 28, 28));
                    if ui.add(del_btn).clicked() {
                        confirmed = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancelled = true;
                    }
                });
            });

        if confirmed {
            if let Some(pos) = self.config.connections.iter().position(|c| c.id == conn_id) {
                let profile = self.config.connections.remove(pos);
                self.config.trashed_connections.insert(
                    0,
                    TrashedConnection {
                        profile,
                        deleted_at: Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
                    },
                );
                let _ = self.config.save();
                self.status_banner = format!("Moved connection '{conn_name}' to Recycle Bin.");
            }
            self.pending_delete_connection = None;
        } else if !open || cancelled {
            self.pending_delete_connection = None;
        }
    }

    pub(crate) fn render_connection_trash_modal(&mut self, ctx: &egui::Context) {
        if !self.show_connection_trash_modal {
            return;
        }

        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.show_connection_trash_modal = false;
            return;
        }

        let mut open = self.show_connection_trash_modal;
        let mut restore_idx: Option<usize> = None;
        let mut purge_idx: Option<usize> = None;
        let mut empty_all = false;

        egui::Window::new("Connection Recycle Bin")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .default_width(520.0)
            .show(ctx, |ui| {
                if self.config.trashed_connections.is_empty() {
                    ui.label(
                        RichText::new("Recycle Bin is empty.")
                            .italics()
                            .color(Color32::GRAY),
                    );
                    return;
                }

                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(format!(
                            "{} deleted connection(s)",
                            self.config.trashed_connections.len()
                        ))
                        .small()
                        .color(Color32::from_rgb(148, 163, 184)),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let empty_btn = egui::Button::new(
                            RichText::new("Empty Trash").small().color(Color32::WHITE),
                        )
                        .fill(Color32::from_rgb(153, 27, 27));
                        if ui.add(empty_btn).clicked() {
                            empty_all = true;
                        }
                    });
                });
                ui.separator();

                egui::ScrollArea::vertical()
                    .max_height(320.0)
                    .show(ui, |ui| {
                        for (idx, item) in self.config.trashed_connections.iter().enumerate() {
                            egui::Frame::group(ui.style())
                                .fill(Color32::from_rgb(17, 24, 39))
                                .corner_radius(egui::CornerRadius::same(6))
                                .inner_margin(egui::Margin::same(8))
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.vertical(|ui| {
                                            ui.label(RichText::new(&item.profile.name).strong());
                                            ui.label(
                                                RichText::new(format!(
                                                    "{}@{}:{}/{} • deleted {}",
                                                    item.profile.user,
                                                    item.profile.host,
                                                    item.profile.port,
                                                    item.profile.default_database,
                                                    item.deleted_at
                                                ))
                                                .small()
                                                .color(Color32::from_rgb(148, 163, 184)),
                                            );
                                        });
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                if ui.small_button("Delete Forever").clicked() {
                                                    purge_idx = Some(idx);
                                                }
                                                let restore_btn = egui::Button::new(
                                                    RichText::new("Restore")
                                                        .small()
                                                        .strong()
                                                        .color(Color32::WHITE),
                                                )
                                                .fill(Color32::from_rgb(16, 185, 129));
                                                if ui.add(restore_btn).clicked() {
                                                    restore_idx = Some(idx);
                                                }
                                            },
                                        );
                                    });
                                });
                            ui.add_space(4.0);
                        }
                    });
            });

        self.show_connection_trash_modal = open;

        if empty_all {
            self.config.trashed_connections.clear();
            let _ = self.config.save();
            self.status_banner = "Emptied Connection Recycle Bin.".to_string();
        } else if let Some(idx) = restore_idx {
            if idx < self.config.trashed_connections.len() {
                let item = self.config.trashed_connections.remove(idx);
                let name = item.profile.name.clone();
                self.config.connections.push(item.profile);
                let _ = self.config.save();
                self.status_banner = format!("Restored connection '{name}'.");
            }
        } else if let Some(idx) = purge_idx
            && idx < self.config.trashed_connections.len()
        {
            let item = self.config.trashed_connections.remove(idx);
            let _ = self.config.save();
            self.status_banner = format!("Permanently deleted connection '{}'.", item.profile.name);
        }
    }
}
