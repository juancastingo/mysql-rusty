use super::*;

impl MysqlRustyApp {
    pub(crate) fn render_connections_dashboard(&mut self, ui: &mut egui::Ui) {
        let mut to_connect = None;
        let mut to_edit = None;
        let mut to_delete = None;

        egui::ScrollArea::vertical()
            .id_salt("connections_dashboard_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.add_space(20.0);
                ui.vertical_centered(|ui| {
                    egui::Frame::new()
                        .fill(Color32::from_rgb(15, 23, 42))
                        .stroke(Stroke::new(1.0, Color32::from_rgb(30, 41, 59)))
                        .corner_radius(egui::CornerRadius::same(12))
                        .inner_margin(egui::Margin::symmetric(14, 4))
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new(rust_i18n::t!("dashboard.badge"))
                                    .size(11.0)
                                    .color(Color32::from_rgb(56, 189, 248))
                                    .strong(),
                            );
                        });

                    ui.add_space(10.0);
                    ui.heading(
                        RichText::new(rust_i18n::t!("dashboard.heading"))
                            .size(24.0)
                            .color(Color32::WHITE)
                            .strong(),
                    );
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(rust_i18n::t!("dashboard.subtitle"))
                            .size(13.5)
                            .color(Color32::from_rgb(148, 163, 184)),
                    );
                    ui.add_space(14.0);

                    ui.horizontal(|ui| {
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new(" + New Connection Profile ")
                                        .size(13.5)
                                        .color(Color32::WHITE)
                                        .strong(),
                                )
                                .fill(Color32::from_rgb(14, 116, 144))
                                .corner_radius(egui::CornerRadius::same(6)),
                            )
                            .clicked()
                        {
                            self.editing_connection = ConnectionProfile::default();
                            self.show_connection_modal = true;
                        }
                        ui.add_space(6.0);
                        if ui
                            .add(
                                egui::Button::new(
                                    RichText::new(" ⬇ Import from Workbench ")
                                        .size(13.0)
                                        .color(Color32::from_rgb(226, 232, 240)),
                                )
                                .fill(Color32::from_rgb(30, 41, 59))
                                .corner_radius(egui::CornerRadius::same(6)),
                            )
                            .on_hover_text(
                                "Import connections from MySQL Workbench (connections.xml)",
                            )
                            .clicked()
                        {
                            self.import_workbench_connections();
                        }
                    });
                    if !self.config.trashed_connections.is_empty() {
                        ui.add_space(8.0);
                        if ui
                            .button(format!(
                                "Recycle Bin ({})",
                                self.config.trashed_connections.len()
                            ))
                            .on_hover_text("Restore or permanently delete trashed connections")
                            .clicked()
                        {
                            self.show_connection_trash_modal = true;
                        }
                    }
                });
                ui.add_space(20.0);

                let max_w = 880.0f32.min(ui.available_width() - 48.0);
                let left_pad = ((ui.available_width() - max_w) / 2.0).max(24.0);

                if self.config.connections.is_empty() {
                    ui.vertical_centered(|ui| {
                        ui.add_space(40.0);
                        ui.label(
                            RichText::new("No connection profiles available.")
                                .color(Color32::from_rgb(148, 163, 184)),
                        );
                        ui.add_space(10.0);
                        if ui.button("Restore Sandbox Demo Profile").clicked() {
                            self.config
                                .connections
                                .push(ConnectionProfile::demo_profile());
                            let _ = self.config.save();
                        }
                    });
                } else {
                    let profiles = self.config.connections.clone();
                    for prof in profiles {
                        let is_demo = prof.is_demo;
                        let card_accent = if is_demo {
                            Color32::from_rgb(16, 185, 129) // Emerald
                        } else {
                            self.parse_hex_color(&prof.color_hex)
                        };

                        ui.horizontal(|ui| {
                            ui.add_space(left_pad);
                            ui.vertical(|ui| {
                                ui.set_width(max_w);
                                egui::Frame::new()
                                    .fill(Color32::from_rgb(17, 24, 39)) // Dark elevated surface
                                    .stroke(Stroke::new(1.0, Color32::from_rgb(31, 41, 55)))
                                    .corner_radius(egui::CornerRadius::same(10))
                                    .inner_margin(16.0)
                                    .show(ui, |ui| {
                                        ui.horizontal(|ui| {
                                            // Left color indicator bar
                                            let (rect, _) = ui.allocate_exact_size(
                                                egui::vec2(4.0, 48.0),
                                                egui::Sense::hover(),
                                            );
                                            ui.painter().rect_filled(
                                                rect,
                                                egui::CornerRadius::same(2),
                                                card_accent,
                                            );
                                            ui.add_space(10.0);

                                            ui.vertical(|ui| {
                                                ui.horizontal(|ui| {
                                                    ui.heading(
                                                        RichText::new(&prof.name)
                                                            .size(17.0)
                                                            .color(Color32::WHITE)
                                                            .strong(),
                                                    );
                                                    if is_demo {
                                                        egui::Frame::new()
                                                            .fill(Color32::from_rgb(6, 78, 59))
                                                            .corner_radius(
                                                                egui::CornerRadius::same(4),
                                                            )
                                                            .inner_margin(egui::Margin::symmetric(
                                                                6, 2,
                                                            ))
                                                            .show(ui, |ui| {
                                                                ui.label(
                                                                    RichText::new("DEMO SANDBOX")
                                                                        .size(10.5)
                                                                        .color(Color32::from_rgb(
                                                                            52, 211, 153,
                                                                        ))
                                                                        .strong(),
                                                                );
                                                            });
                                                    }
                                                    if prof.ssh.enabled {
                                                        egui::Frame::new()
                                                            .fill(Color32::from_rgb(8, 51, 68))
                                                            .corner_radius(
                                                                egui::CornerRadius::same(4),
                                                            )
                                                            .inner_margin(egui::Margin::symmetric(
                                                                6, 2,
                                                            ))
                                                            .show(ui, |ui| {
                                                                ui.label(
                                                                    RichText::new("SSH TUNNEL")
                                                                        .size(10.5)
                                                                        .color(Color32::from_rgb(
                                                                            56, 189, 248,
                                                                        ))
                                                                        .strong(),
                                                                );
                                                            });
                                                    }
                                                    let (er, eg, eb) = prof.environment.color_rgb();
                                                    egui::Frame::new()
                                                        .fill(Color32::from_rgb(
                                                            er / 5,
                                                            eg / 5,
                                                            eb / 5,
                                                        ))
                                                        .stroke(Stroke::new(
                                                            1.0,
                                                            Color32::from_rgb(er, eg, eb),
                                                        ))
                                                        .corner_radius(egui::CornerRadius::same(4))
                                                        .inner_margin(egui::Margin::symmetric(6, 2))
                                                        .show(ui, |ui| {
                                                            ui.label(
                                                                RichText::new(
                                                                    prof.environment
                                                                        .label()
                                                                        .to_uppercase(),
                                                                )
                                                                .size(10.5)
                                                                .color(Color32::from_rgb(
                                                                    er, eg, eb,
                                                                ))
                                                                .strong(),
                                                            );
                                                        });
                                                    if prof.read_only {
                                                        egui::Frame::new()
                                                            .fill(Color32::from_rgb(67, 20, 7))
                                                            .stroke(Stroke::new(
                                                                1.0,
                                                                Color32::from_rgb(251, 146, 60),
                                                            ))
                                                            .corner_radius(
                                                                egui::CornerRadius::same(4),
                                                            )
                                                            .inner_margin(egui::Margin::symmetric(
                                                                6, 2,
                                                            ))
                                                            .show(ui, |ui| {
                                                                ui.label(
                                                                    RichText::new("READ-ONLY")
                                                                        .size(10.5)
                                                                        .color(Color32::from_rgb(
                                                                            251, 146, 60,
                                                                        ))
                                                                        .strong(),
                                                                );
                                                            });
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
                                                        .color(Color32::from_rgb(226, 232, 240)),
                                                    );
                                                    ui.label(RichText::new("•").weak());
                                                    ui.label(
                                                        RichText::new(format!(
                                                            "Host: {}:{}",
                                                            prof.host, prof.port
                                                        ))
                                                        .size(12.5)
                                                        .color(Color32::from_rgb(148, 163, 184)),
                                                    );
                                                    ui.label(RichText::new("•").weak());
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
                                                        .color(Color32::from_rgb(148, 163, 184)),
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
                                                        .color(Color32::from_rgb(56, 189, 248)),
                                                    );
                                                }
                                            });

                                            ui.with_layout(
                                                egui::Layout::right_to_left(egui::Align::Center),
                                                |ui| {
                                                    if ui
                                                        .add(
                                                            egui::Button::new(
                                                                RichText::new(" Connect ")
                                                                    .size(13.5)
                                                                    .color(Color32::WHITE)
                                                                    .strong(),
                                                            )
                                                            .fill(if is_demo {
                                                                Color32::from_rgb(16, 185, 129)
                                                            } else {
                                                                Color32::from_rgb(2, 132, 199)
                                                            })
                                                            .corner_radius(
                                                                egui::CornerRadius::same(6),
                                                            ),
                                                        )
                                                        .clicked()
                                                    {
                                                        to_connect = Some(prof.clone());
                                                    }
                                                    if ui.button("Edit").clicked() {
                                                        to_edit = Some(prof.clone());
                                                    }
                                                    if ui.button("Delete").clicked() {
                                                        to_delete = Some(prof.id.clone());
                                                    }
                                                },
                                            );
                                        });
                                    });
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
