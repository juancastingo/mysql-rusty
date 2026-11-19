use super::*;

impl MysqlRustyApp {
    pub(crate) fn render_connections_dashboard(&mut self, ui: &mut egui::Ui) {
        let pal = self.effective_palette(ui.ctx());
        let mut to_connect = None;
        let mut to_edit = None;
        let mut to_delete = None;

        egui::ScrollArea::vertical()
            .id_salt("connections_dashboard_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.add_space(24.0);
                ui.vertical_centered(|ui| {
                    egui::Frame::new()
                        .fill(pal.accent_bg)
                        .stroke(Stroke::new(1.0, pal.accent.gamma_multiply(0.4)))
                        .corner_radius(egui::CornerRadius::same(12))
                        .inner_margin(egui::Margin::symmetric(14, 4))
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new(rust_i18n::t!("dashboard.badge"))
                                    .size(11.0)
                                    .color(pal.accent_text)
                                    .strong(),
                            );
                        });

                    ui.add_space(10.0);
                    ui.heading(
                        RichText::new(rust_i18n::t!("dashboard.heading"))
                            .size(24.0)
                            .color(pal.text_primary)
                            .strong(),
                    );
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(rust_i18n::t!("dashboard.subtitle"))
                            .size(13.5)
                            .color(pal.muted_text),
                    );
                    ui.add_space(16.0);
                });

                // Properly centered action row
                let avail_w = ui.available_width();
                let actions_w = 410.0f32.min(avail_w - 24.0).max(200.0);
                let actions_pad = ((avail_w - actions_w) / 2.0).max(12.0);
                ui.horizontal(|ui| {
                    ui.add_space(actions_pad);
                    ui.horizontal_wrapped(|ui| {
                        if ui
                            .add(pal.primary_button("+ New Connection Profile"))
                            .clicked()
                        {
                            self.editing_connection = ConnectionProfile::default();
                            self.show_connection_modal = true;
                        }
                        ui.add_space(4.0);
                        if ui
                            .add(pal.subtle_button("Import from Workbench"))
                            .on_hover_text(
                                "Import connections from MySQL Workbench (connections.xml)",
                            )
                            .clicked()
                        {
                            self.import_workbench_connections();
                        }
                        if !self.config.trashed_connections.is_empty() {
                            ui.add_space(4.0);
                            if ui
                                .add(pal.subtle_button(format!(
                                    "Recycle Bin ({})",
                                    self.config.trashed_connections.len()
                                )))
                                .on_hover_text("Restore or permanently delete trashed connections")
                                .clicked()
                            {
                                self.show_connection_trash_modal = true;
                            }
                        }
                    });
                });

                ui.add_space(22.0);

                let max_w = 880.0f32.min(ui.available_width() - 48.0).max(300.0);
                let left_pad = ((ui.available_width() - max_w) / 2.0).max(16.0);

                if self.config.connections.is_empty() {
                    ui.horizontal(|ui| {
                        ui.add_space(left_pad);
                        ui.vertical(|ui| {
                            ui.set_width(max_w);
                            pal.card_frame().inner_margin(28.0).show(ui, |ui| {
                                ui.vertical_centered(|ui| {
                                    ui.label(
                                        RichText::new("No connection profiles yet")
                                            .size(16.0)
                                            .strong()
                                            .color(pal.text_primary),
                                    );
                                    ui.add_space(6.0);
                                    ui.label(
                                        RichText::new(
                                            "Create a new MySQL, MariaDB, PostgreSQL, or SQLite connection, or try the built-in sandbox.",
                                        )
                                        .size(12.5)
                                        .color(pal.muted_text),
                                    );
                                    ui.add_space(14.0);
                                    if ui
                                        .add(pal.subtle_button("Restore Sandbox Demo Profile"))
                                        .clicked()
                                    {
                                        self.config
                                            .connections
                                            .push(ConnectionProfile::demo_profile());
                                        let _ = self.config.save();
                                    }
                                });
                            });
                        });
                    });
                } else {
                    let profiles = self.config.connections.clone();
                    for prof in profiles {
                        let is_demo = prof.is_demo;
                        let card_accent = if is_demo {
                            pal.success
                        } else {
                            self.parse_hex_color(&prof.color_hex)
                        };

                        ui.horizontal(|ui| {
                            ui.add_space(left_pad);
                            ui.vertical(|ui| {
                                ui.set_width(max_w);
                                let card_id = ui.make_persistent_id(("dashboard_card", &prof.id));
                                let prev_hovered = ui
                                    .ctx()
                                    .data(|d| d.get_temp::<bool>(card_id))
                                    .unwrap_or(false);

                                let frame_resp = egui::Frame::new()
                                    .fill(if prev_hovered {
                                        pal.card_hover_bg
                                    } else {
                                        pal.card_bg
                                    })
                                    .stroke(Stroke::new(
                                        1.0,
                                        if prev_hovered {
                                            pal.border_active
                                        } else {
                                            pal.card_border
                                        },
                                    ))
                                    .corner_radius(egui::CornerRadius::same(10))
                                    .inner_margin(14.0)
                                    .show(ui, |ui| {
                                        let inner_resp = ui.horizontal(|ui| {
                                            // Reserve space for the left accent bar; we'll paint it after knowing row height
                                            let bar_x = ui.cursor().min.x;
                                            let bar_top = ui.cursor().min.y;
                                            ui.add_space(12.0);

                                            ui.vertical(|ui| {
                                                ui.horizontal_wrapped(|ui| {
                                                    ui.label(
                                                        RichText::new(&prof.name)
                                                            .size(16.0)
                                                            .color(pal.text_primary)
                                                            .strong(),
                                                    );
                                                    if is_demo {
                                                        pal.badge(
                                                            ui,
                                                            "DEMO SANDBOX",
                                                            pal.success,
                                                            pal.success_bg,
                                                        );
                                                    }
                                                    if prof.ssh.enabled {
                                                        pal.badge(
                                                            ui,
                                                            "SSH TUNNEL",
                                                            pal.accent_text,
                                                            pal.accent_bg,
                                                        );
                                                    }
                                                    let (er, eg, eb) = prof.environment.color_rgb();
                                                    let (env_fg, env_bg) =
                                                        pal.env_badge_colors(er, eg, eb);
                                                    pal.badge(
                                                        ui,
                                                        &prof.environment.label().to_uppercase(),
                                                        env_fg,
                                                        env_bg,
                                                    );
                                                    if prof.read_only {
                                                        pal.badge(
                                                            ui,
                                                            "READ-ONLY",
                                                            pal.warning,
                                                            pal.warning_bg,
                                                        );
                                                    }
                                                });
                                                ui.add_space(4.0);
                                                ui.horizontal_wrapped(|ui| {
                                                    ui.label(
                                                        RichText::new(format!(
                                                            "User: {}",
                                                            prof.user
                                                        ))
                                                        .size(12.5)
                                                        .color(pal.text_secondary),
                                                    );
                                                    ui.label(
                                                        RichText::new("•").color(pal.muted_text),
                                                    );
                                                    ui.label(
                                                        RichText::new(format!(
                                                            "Host: {}:{}",
                                                            prof.host, prof.port
                                                        ))
                                                        .size(12.5)
                                                        .color(pal.muted_text),
                                                    );
                                                    ui.label(
                                                        RichText::new("•").color(pal.muted_text),
                                                    );
                                                    ui.label(
                                                        RichText::new(format!(
                                                            "DB: {}",
                                                            if prof.default_database.is_empty() {
                                                                "None"
                                                            } else {
                                                                &prof.default_database
                                                            }
                                                        ))
                                                        .size(12.5)
                                                        .color(pal.muted_text),
                                                    );
                                                });
                                                if prof.ssh.enabled {
                                                    ui.add_space(2.0);
                                                    ui.label(
                                                        RichText::new(format!(
                                                            "SSH Bastion: {}@{} (Port: {})",
                                                            prof.ssh.ssh_user,
                                                            prof.ssh.ssh_host,
                                                            prof.ssh.ssh_port
                                                        ))
                                                        .size(11.5)
                                                        .color(pal.accent_text),
                                                    );
                                                }
                                            });

                                            ui.with_layout(
                                                egui::Layout::right_to_left(egui::Align::Center),
                                                |ui| {
                                                    let connect_btn = if is_demo {
                                                        egui::Button::new(
                                                            RichText::new("Connect")
                                                                .size(13.0)
                                                                .color(Color32::WHITE)
                                                                .strong(),
                                                        )
                                                        .fill(Color32::from_rgb(5, 150, 105))
                                                        .corner_radius(egui::CornerRadius::same(6))
                                                    } else {
                                                        pal.primary_button("Connect")
                                                    };
                                                    if ui.add(connect_btn).clicked() {
                                                        to_connect = Some(prof.clone());
                                                    }
                                                    if ui.button("Edit").clicked() {
                                                        to_edit = Some(prof.clone());
                                                    }
                                                    if ui
                                                        .button(
                                                            RichText::new("Delete")
                                                                .color(pal.danger),
                                                        )
                                                        .clicked()
                                                    {
                                                        to_delete = Some(prof.id.clone());
                                                    }
                                                },
                                            );

                                            (bar_x, bar_top)
                                        });

                                        let (bar_x, bar_top) = inner_resp.inner;
                                        let bar_bottom = inner_resp.response.rect.bottom();
                                        let bar_rect = egui::Rect::from_min_max(
                                            egui::pos2(bar_x, bar_top),
                                            egui::pos2(bar_x + 4.0, bar_bottom.max(bar_top + 40.0)),
                                        );
                                        ui.painter().rect_filled(
                                            bar_rect,
                                            egui::CornerRadius::same(2),
                                            card_accent,
                                        );
                                    });

                                let hovered = frame_resp.response.hovered();
                                ui.ctx().data_mut(|d| d.insert_temp(card_id, hovered));

                                if frame_resp
                                    .response
                                    .interact(egui::Sense::click())
                                    .double_clicked()
                                {
                                    to_connect = Some(prof.clone());
                                }
                            });
                        });
                        ui.add_space(10.0);
                    }
                }
                ui.add_space(30.0);
            });

        if let Some(prof) = to_connect {
            self.connect_to_profile(prof);
        }
        if let Some(prof) = to_edit {
            self.editing_connection = prof;
            self.show_connection_modal = true;
        }
        if let Some(id) = to_delete {
            let name = self
                .config
                .connections
                .iter()
                .find(|c| c.id == id)
                .map(|c| c.name.clone())
                .unwrap_or_else(|| "this connection".to_string());
            self.pending_delete_connection = Some((id, name));
        }
    }
}
